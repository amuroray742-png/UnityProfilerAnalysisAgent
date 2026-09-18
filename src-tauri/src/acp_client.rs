//! ACP Client —— 极简 stdio 协议
//!
//! 不依赖 agent-client-protocol SDK；直接把 prompt 写进 stdin，
//! 把 stdout 按行切 chunk 推到前端，stderr 作为 [error] event。
//!
//! 适用 agent：
//! - claude-code-acp：实测吃 stdin prompt（legacy 模式），stdout 流式输出
//! - gemini --experimental-acp：可能需要 JSON-RPC framing，先按行试
//! - codex-acp：同上
//!
//! 若 agent 需要严格 JSON-RPC 握手，此实现会卡在「Started 已发但 stdout 无响应」。

pub mod agents;
pub mod client;
pub mod stream_relay;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::extractor::MetricsSnapshot;

#[derive(Debug)]
pub struct DiagnoseRequest {
    pub file_id: String,
    pub agent_id: String,
    pub snapshot: MetricsSnapshot,
    pub event_tx: mpsc::UnboundedSender<DiagnoseEvent>,
    pub cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DiagnoseEvent {
    Started { agent_id: String },
    Chunk { text: String },
    McpCall { tool: String, args: serde_json::Value },
    McpResult { tool: String, result: serde_json::Value },
    Finished { total_chunks: u64 },
    Error { message: String },
}

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("Agent 未安装: {0}")]
    AgentNotInstalled(String),

    #[error("spawn 子进程失败: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("ACP 协议错误: {0}")]
    Protocol(String),

    #[error("session 启动失败: {0}")]
    Session(String),

    #[error("其他错误: {0}")]
    Other(String),
}

impl From<serde_json::Error> for AcpError {
    fn from(err: serde_json::Error) -> Self {
        AcpError::Protocol(err.to_string())
    }
}

/// 极简 stdio 协议启动诊断：
/// 1. spawn 子进程（已 resolve_command 处理 .ps1 / .bat shim）
/// 2. Started event 立即发出
/// 3. 4 个并发 task：
///    - stdin writer：写入 prompt，然后 EOF（drop）
///    - stdout reader：每行 → Chunk
///    - stderr reader：每行 → Error
///    - child.wait()：进程退出后 emit Finished
/// 4. abort supervisor task = cancel（kill_on_drop 会强杀 child）
pub async fn start_diagnose(
    preset: agents::AgentPreset,
    req: DiagnoseRequest,
) -> Result<client::SessionHandle, AcpError> {
    if !agents::probe_available(&preset.command) {
        return Err(AcpError::AgentNotInstalled(preset.command));
    }

    let mut child = client::spawn_agent(&preset).await?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AcpError::Session("stdin not piped".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AcpError::Session("stdout not piped".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AcpError::Session("stderr not piped".into()))?;

    let event_tx = req.event_tx.clone();
    let agent_id = req.agent_id.clone();
    let snapshot_json = serde_json::to_string_pretty(&req.snapshot).unwrap_or_default();
    let prompt = format!(
        "你是 Unity 性能优化专家。分析下面这份 Profiler 数据：\n\n```json\n{}\n```\n\n请输出 TOP 3 性能问题、根因、可执行建议。\n",
        snapshot_json
    );

    let supervisor = tokio::spawn(async move {
        // 1) Started
        let _ = event_tx.send(DiagnoseEvent::Started { agent_id });

        // 2) stdin writer：写 prompt → EOF
        let tx_stdin_err = event_tx.clone();
        let stdin_task = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut stdin = stdin;
            if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
                let _ = tx_stdin_err.send(DiagnoseEvent::Error {
                    message: format!("写入 stdin 失败: {}", e),
                });
            }
            // stdin 在此 drop → EOF 发给 agent
        });

        // 3) stdout reader：每行 → Chunk
        let tx_out = event_tx.clone();
        let stdout_task = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            let mut count: u64 = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                count += 1;
                if tx_out
                    .send(DiagnoseEvent::Chunk {
                        text: format!("{}\n", line),
                    })
                    .is_err()
                {
                    break;
                }
            }
            tracing::debug!(target: "agent_stdout", "EOF after {} lines", count);
        });

        // 4) stderr reader：每行 → Error
        let tx_err = event_tx.clone();
        let stderr_task = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx_err
                    .send(DiagnoseEvent::Error {
                        message: format!("[stderr] {}", line),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        // 5) 等待子进程退出
        let status = child.wait().await;

        // 等子 task 收尾（EOF → reader 退出）
        let _ = stdin_task.await;
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        match status {
            Ok(s) => {
                tracing::info!(target: "agent_exit", "agent exited: {:?}", s);
                if !s.success() {
                    let code = s.code();
                    let _ = event_tx.send(DiagnoseEvent::Error {
                        message: format!("agent 退出码: {:?}", code),
                    });
                }
            }
            Err(e) => {
                let _ = event_tx.send(DiagnoseEvent::Error {
                    message: format!("等待子进程退出失败: {}", e),
                });
            }
        }

        let _ = event_tx.send(DiagnoseEvent::Finished { total_chunks: 0 });
    });

    Ok(client::SessionHandle::new(supervisor))
}