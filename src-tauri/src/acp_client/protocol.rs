//! ACP v1 JSON-RPC conversation; no legacy plain-text fallback.
use super::{AcpError, DiagnoseEvent};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, watch},
    time::{Duration, Instant},
};

pub struct Peer<R, W> {
    reader: BufReader<R>,
    writer: W,
    cancel: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<DiagnoseEvent>,
    session: Option<String>,
    next_id: u64,
    pub chunks: u64,
    cancelled: bool,
    pub allow_source: bool,
    pub allow_project: bool,
    pub allow_modification: bool,
    report_bytes: usize,
    pending_mcp: std::collections::HashMap<String, String>,
}
impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Peer<R, W> {
    pub fn new(
        read: R,
        write: W,
        cancel: watch::Receiver<bool>,
        events: mpsc::UnboundedSender<DiagnoseEvent>,
    ) -> Self {
        Self {
            reader: BufReader::new(read),
            writer: write,
            cancel,
            events,
            session: None,
            next_id: 0,
            chunks: 0,
            cancelled: false,
            allow_source: false,
            allow_project: false,
            allow_modification: false,
            report_bytes: 0,
            pending_mcp: std::collections::HashMap::new(),
        }
    }
    async fn send(&mut self, value: Value) -> Result<(), AcpError> {
        tokio::time::timeout(Duration::from_secs(5), async {
            self.writer.write_all(value.to_string().as_bytes()).await?;
            self.writer.write_all(b"\n").await?;
            self.writer.flush().await
        })
        .await
        .map_err(|_| AcpError::Protocol("ACP 写入超时".into()))??;
        Ok(())
    }
    pub async fn run(
        &mut self,
        cwd: &std::path::Path,
        mcp: Value,
        prompt: String,
    ) -> Result<String, AcpError> {
        let init=self.request("initialize",json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"unity-profiler-analysis-agent","version":env!("CARGO_PKG_VERSION")}}),Duration::from_secs(30)).await?;
        if init["protocolVersion"] != 1 {
            return Err(AcpError::Protocol("Agent 不支持 ACP v1".into()));
        }
        let created = self
            .request(
                "session/new",
                json!({"cwd":cwd,"mcpServers":[mcp],"_meta":{"disableBuiltInTools":self.allow_modification,"claudeCode":{"options":if self.allow_modification {json!({"settingSources":["user"]})}else{json!({})}}}}),
                Duration::from_secs(60),
            )
            .await?;
        let session = created["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AcpError::Protocol("缺少 sessionId".into()))?
            .to_owned();
        self.session = Some(session.clone());
        let _ = self.events.send(DiagnoseEvent::SessionCreated {
            acp_session_id: session.clone(),
        });
        let result = self
            .request_with_limits(
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":prompt}]}),
                Duration::from_secs(if self.allow_project || self.allow_modification {
                    3600
                } else {
                    1800
                }),
                Some(Duration::from_secs(300)),
            )
            .await?;
        let reason = result["stopReason"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("缺少 stopReason".into()))?;
        match reason {
            "end_turn" => Ok(reason.into()),
            "max_tokens" | "max_turn_requests" | "refusal" => Err(AcpError::Session(format!(
                "诊断未完成，Agent 停止原因：{reason}"
            ))),
            "cancelled" => Err(AcpError::Cancelled),
            _ => Err(AcpError::Protocol(format!("未知 stopReason: {reason}"))),
        }
    }
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AcpError> {
        self.request_with_limits(method, params, timeout, None)
            .await
    }
    async fn request_with_limits(
        &mut self,
        method: &str,
        params: Value,
        total: Duration,
        idle: Option<Duration>,
    ) -> Result<Value, AcpError> {
        if *self.cancel.borrow() {
            return Err(AcpError::Cancelled);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        let started = Instant::now();
        let total_deadline = started + total;
        let mut idle_deadline = started + idle.unwrap_or(total);
        let mut cancel_deadline = None;
        // Buffer remains outside select: cancellation cannot discard a partial line.
        let mut bytes = Vec::new();
        loop {
            let deadline = cancel_deadline.unwrap_or(total_deadline.min(idle_deadline));
            tokio::select! {
                biased;
                _=self.cancel.changed(), if !self.cancelled => {
                    self.cancelled=true;
                    if let Some(session)=&self.session {
                        self.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})).await?;
                        cancel_deadline=Some(Instant::now()+Duration::from_secs(2));
                    } else {return Err(AcpError::Cancelled)}
                },
                _=tokio::time::sleep_until(deadline)=>{
                    return if self.cancelled {Err(AcpError::Cancelled)} else if idle.is_some() {
                        let reason = if total_deadline <= idle_deadline {
                            format!("总时限超时（{} 秒）", total.as_secs())
                        } else {
                            format!("活动超时（连续 {} 秒未收到当前会话活动）", idle.unwrap().as_secs())
                        };
                        Err(AcpError::Protocol(format!("{method} {reason}；报告不完整，已保留收到的正文")))
                    } else {Err(AcpError::Protocol(format!("{method} 超时")))};
                },
                read=read_message(&mut self.reader,&mut bytes)=>{
                    let message=match read {Ok(message)=>message,Err(_) if self.cancelled=>return Err(AcpError::Cancelled),Err(error)=>return Err(error)};
                    bytes.clear();
                    if message["jsonrpc"]!="2.0" {return Err(AcpError::Protocol("stdout 不是 JSON-RPC 2.0".into()))}
                    // Only complete messages for this session count as activity. Partial
                    // bytes, unrelated responses and other sessions cannot keep it alive.
                    if !self.cancelled && idle.is_some()
                        && self.session.as_deref().is_some_and(|s| message["params"]["sessionId"].as_str()==Some(s))
                        && matches!(message["method"].as_str(), Some("session/update" | "session/request_permission")) {
                        idle_deadline=Instant::now()+idle.unwrap();
                    }
                    if let Some(method)=message["method"].as_str() {
                        self.inbound(method,&message).await?;
                    } else if message["id"]==id {
                        if self.cancelled {
                            if message["result"]["stopReason"]=="cancelled" {let _=self.events.send(DiagnoseEvent::Log{message:"ACP 取消已确认".into()});}
                            return Err(AcpError::Cancelled)
                        }
                        if let Some(error)=message.get("error") {return Err(AcpError::Protocol(format!("Agent: {error}")))}
                        return message.get("result").cloned().ok_or_else(||AcpError::Protocol("响应缺少 result".into()));
                    }
                }
            }
        }
    }
    async fn inbound(&mut self, method: &str, message: &Value) -> Result<(), AcpError> {
        let params = &message["params"];
        let matching = self
            .session
            .as_deref()
            .is_some_and(|id| params["sessionId"].as_str() == Some(id));
        if let Some(id) = message.get("id") {
            let response = if method == "session/request_permission" {
                let correlated = if matching && params["_meta"]["is_mcp_tool_approval"] == true {
                    params["toolCall"]["toolCallId"]
                        .as_str()
                        .and_then(|id| self.pending_mcp.remove(id))
                } else {
                    None
                };
                let title = correlated
                    .as_deref()
                    .unwrap_or_else(|| params["toolCall"]["title"].as_str().unwrap_or(""));
                let trusted = (self.allow_modification
                    && crate::optimization::session::schemas()["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|t| {
                            title == format!("mcp__unity-profiler__{}", t["name"].as_str().unwrap())
                        }))
                    || (self.allow_project
                        && crate::project::schemas()["tools"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|t| {
                                title
                                    == format!(
                                        "mcp__unity-profiler__{}",
                                        t["name"].as_str().unwrap()
                                    )
                            }))
                    || (self.allow_source
                        && ["source_files", "source_search", "source_read"]
                            .iter()
                            .any(|n| title == format!("mcp__unity-profiler__{n}")))
                    || crate::mcp::list_tool_schemas()["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|tool| {
                            title
                                == format!(
                                    "mcp__unity-profiler__{}",
                                    tool["name"].as_str().unwrap()
                                )
                        });
                let choice = if matching && !self.cancelled && trusted {
                    params["options"]
                        .as_array()
                        .and_then(|a| a.iter().find(|p| p["kind"] == "allow_once"))
                        .and_then(|p| p["optionId"].as_str())
                } else {
                    None
                };
                if choice.is_none() {
                    let _ = self.events.send(DiagnoseEvent::Log {
                        message: format!("未授权工具请求：{title}"),
                    });
                }
                let outcome = choice
                    .map(|id| json!({"outcome":"selected","optionId":id}))
                    .unwrap_or_else(|| json!({"outcome":"cancelled"}));
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}})
            } else {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"客户端不支持该方法"}})
            };
            self.send(response).await?;
        } else if method == "session/update" && matching && !self.cancelled {
            let update = &params["update"];
            if let Some(id) = update["toolCallId"].as_str() {
                if matches!(update["status"].as_str(), Some("completed" | "failed")) {
                    self.pending_mcp.remove(id);
                } else if update["sessionUpdate"] == "tool_call"
                    && update["rawInput"]["server"] == "unity-profiler"
                {
                    if let Some(name) = update["rawInput"]["tool"].as_str() {
                        if self.pending_mcp.len() >= 1024 {
                            return Err(AcpError::Protocol("未完成工具调用过多".into()));
                        }
                        self.pending_mcp
                            .insert(id.into(), format!("mcp__unity-profiler__{name}"));
                    }
                }
            }
            if update["sessionUpdate"] == "agent_message_chunk"
                && update["content"]["type"] == "text"
            {
                if let Some(text) = update["content"]["text"].as_str() {
                    let remaining = crate::reports::MAX_REPORT - self.report_bytes;
                    let mut end = text.len().min(remaining);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    let overflow = end < text.len();
                    let text = &text[..end];
                    self.report_bytes += text.len();
                    self.chunks += 1;
                    self.events
                        .send(DiagnoseEvent::Chunk { text: text.into() })
                        .map_err(|_| AcpError::Cancelled)?;
                    if overflow {
                        return Err(AcpError::Other(
                            "REPORT_LIMIT: 正文达到 2 MiB，报告不完整".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    bytes: &mut Vec<u8>,
) -> Result<Value, AcpError> {
    loop {
        let data = reader.fill_buf().await?;
        if data.is_empty() {
            return Err(AcpError::Protocol("Agent 在响应结束前关闭 stdout".into()));
        }
        let end = data.iter().position(|b| *b == b'\n');
        let count = end.map(|i| i + 1).unwrap_or(data.len());
        if bytes.len() + count > 1024 * 1024 {
            return Err(AcpError::Protocol("ACP 单条消息超过 1 MiB".into()));
        }
        bytes.extend_from_slice(&data[..count]);
        reader.consume(count);
        if end.is_some() {
            return serde_json::from_slice(bytes).map_err(AcpError::from);
        }
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use tokio::io::{duplex, split};

    fn update(session: &str, kind: &str) -> Value {
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":{"sessionUpdate":kind,"content":{"type":"text","text":"progress"}}}})
    }

    // Virtual time exercises the production request loop without waiting minutes.
    async fn scenario(
        messages: Vec<(u64, Value)>,
        total: u64,
        idle: u64,
        cancel_at: Option<u64>,
    ) -> (Result<Value, AcpError>, Vec<DiagnoseEvent>, u64) {
        let (client, server) = duplex(16384);
        let (read, write) = split(client);
        let (server_read, mut server_write) = split(server);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (events, mut rx) = mpsc::unbounded_channel();
        let mut peer = Peer::new(read, write, cancel_rx, events);
        peer.session = Some("current".into());
        let start = Instant::now();
        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_read);
            read_message(&mut reader, &mut Vec::new()).await.unwrap();
            for (at, message) in messages {
                tokio::time::sleep_until(start + Duration::from_secs(at)).await;
                if server_write
                    .write_all(format!("{message}\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // Keep stdout open: silence must time out, not turn into EOF.
            std::future::pending::<()>().await;
        });
        let cancel = tokio::spawn(async move {
            if let Some(at) = cancel_at {
                tokio::time::sleep_until(start + Duration::from_secs(at)).await;
                cancel_tx.send(true).unwrap();
            }
            std::future::pending::<()>().await;
        });
        let result = peer
            .request_with_limits(
                "session/prompt",
                json!({}),
                Duration::from_secs(total),
                Some(Duration::from_secs(idle)),
            )
            .await;
        let elapsed = start.elapsed().as_secs();
        let mut collected = Vec::new();
        while let Ok(event) = rx.try_recv() {
            collected.push(event);
        }
        server.abort();
        cancel.abort();
        (result, collected, elapsed)
    }

    #[tokio::test(start_paused = true)]
    async fn active_prompt_completes_beyond_old_five_minute_limit() {
        let (result, events, elapsed) = scenario(
            vec![
                (240, update("current", "agent_message_chunk")),
                (480, update("current", "agent_thought_chunk")),
                (720, update("current", "tool_call_update")),
                (
                    960,
                    json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}),
                ),
            ],
            1800,
            300,
            None,
        )
        .await;
        assert_eq!(result.unwrap()["stopReason"], "end_turn");
        assert_eq!(elapsed, 960);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, DiagnoseEvent::Chunk { .. }))
                .count(),
            1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silence_after_output_times_out_and_preserves_text() {
        let (result, events, elapsed) = scenario(
            vec![(200, update("current", "agent_message_chunk"))],
            1800,
            300,
            None,
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("活动超时"));
        assert_eq!(elapsed, 500);
        assert!(matches!(&events[0], DiagnoseEvent::Chunk { text } if text=="progress"));
    }

    #[tokio::test(start_paused = true)]
    async fn unrelated_messages_do_not_extend_idle_timeout() {
        let (result, _, elapsed) = scenario(
            vec![
                (200, update("other", "agent_message_chunk")),
                (250, json!({"jsonrpc":"2.0","id":99,"result":{}})),
            ],
            1800,
            300,
            None,
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("活动超时"));
        assert_eq!(elapsed, 300);
    }

    #[tokio::test(start_paused = true)]
    async fn continuing_activity_cannot_extend_total_deadline() {
        let messages = (1..=10)
            .map(|n| (n * 200, update("current", "tool_call_update")))
            .collect();
        let (result, _, elapsed) = scenario(messages, 1800, 300, None).await;
        assert!(result.unwrap_err().to_string().contains("总时限超时"));
        assert_eq!(elapsed, 1800);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_is_not_extended_by_activity() {
        let (result, events, elapsed) = scenario(
            vec![(101, update("current", "agent_message_chunk"))],
            1800,
            300,
            Some(100),
        )
        .await;
        assert!(matches!(result, Err(AcpError::Cancelled)));
        assert_eq!(elapsed, 102);
        assert!(events.is_empty());
    }
}
