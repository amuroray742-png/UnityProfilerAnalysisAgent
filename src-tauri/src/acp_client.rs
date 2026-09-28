//! ACP v1 session and MCP lifetime. No plain-text protocol fallback.
pub mod agents;
pub mod client;
pub mod protocol;
pub mod stream_relay;
use crate::{
    extractor::MetricsSnapshot,
    mcp::{bridge::BridgeServer, server::ProfilerServer, MetricsStore},
    parser::detail::FrameStore,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct DiagnoseRequest {
    pub file_id: String,
    pub agent_id: String,
    pub snapshot: MetricsSnapshot,
    pub details: Option<Arc<FrameStore>>,
    pub bridge_executable: PathBuf,
    pub event_tx: mpsc::UnboundedSender<DiagnoseEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum DiagnoseEvent {
    Started {
        agent_id: String,
    },
    Chunk {
        text: String,
    },
    McpCall {
        tool: String,
        args: serde_json::Value,
    },
    McpResult {
        tool: String,
        result: serde_json::Value,
    },
    Log {
        message: String,
    },
    Finished {
        total_chunks: u64,
        stop_reason: String,
    },
    Cancelled,
    Error {
        message: String,
    },
}
impl DiagnoseEvent {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished { .. } | Self::Cancelled | Self::Error { .. }
        )
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub session_id: String,
    pub file_id: String,
    #[serde(flatten)]
    pub event: DiagnoseEvent,
}
#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("Agent 未安装: {0}")]
    AgentNotInstalled(String),
    #[error("进程/IO 错误: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("ACP 协议错误: {0}")]
    Protocol(String),
    #[error("会话错误: {0}")]
    Session(String),
    #[error("诊断已取消")]
    Cancelled,
    #[error("其他错误: {0}")]
    Other(String),
}
impl From<serde_json::Error> for AcpError {
    fn from(e: serde_json::Error) -> Self {
        Self::Protocol(e.to_string())
    }
}

struct WorkDir(PathBuf);
impl WorkDir {
    fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("upaa-agent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub async fn start_diagnose(
    preset: agents::AgentPreset,
    req: DiagnoseRequest,
) -> Result<client::SessionHandle, AcpError> {
    if !agents::probe_available(&preset.command) {
        return Err(AcpError::AgentNotInstalled(preset.command));
    }
    let (handle, cancel, done) = client::SessionHandle::channel();
    tokio::spawn(async move {
        let events = req.event_tx.clone();
        let _ = events.send(DiagnoseEvent::Started {
            agent_id: req.agent_id.clone(),
        });
        let result = run_session(preset, req, cancel).await;
        let terminal = match result {
            Ok((chunks, reason)) => DiagnoseEvent::Finished {
                total_chunks: chunks,
                stop_reason: reason,
            },
            Err(AcpError::Cancelled) => DiagnoseEvent::Cancelled,
            Err(error) => DiagnoseEvent::Error {
                message: error.to_string(),
            },
        };
        let _ = events.send(terminal);
        done.send_replace(true);
    });
    Ok(handle)
}
async fn run_session(
    preset: agents::AgentPreset,
    req: DiagnoseRequest,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<(u64, String), AcpError> {
    if *cancel.borrow() {
        return Err(AcpError::Cancelled);
    }
    let workspace = WorkDir::new()?;
    let store = MetricsStore::new();
    store.set_capture(req.snapshot, req.details).await;
    let (audit_tx, mut audit_rx) = mpsc::channel(64);
    let bridge = BridgeServer::start(ProfilerServer {
        store,
        audit: Some(audit_tx),
    })
    .await?;
    let config = bridge.acp_config(&req.bridge_executable);
    let mut child = client::spawn_agent(&preset, &workspace.0).await?;
    let tree = match client::ProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill().await;
            return Err(error.into());
        }
    };
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AcpError::Session("stdin missing".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AcpError::Session("stdout missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AcpError::Session("stderr missing".into()))?;
    let events = req.event_tx.clone();
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut stderr = stderr;
        let mut bytes = [0; 4096];
        while let Ok(n) = stderr.read(&mut bytes).await {
            if n == 0 {
                break;
            }
            let _ = events.send(DiagnoseEvent::Log {
                message: String::from_utf8_lossy(&bytes[..n]).into_owned(),
            });
        }
    });
    let events = req.event_tx.clone();
    workers.spawn(async move {
        while let Some(audit) = audit_rx.recv().await {
            let _ = events.send(DiagnoseEvent::McpCall {
                tool: audit.tool.clone(),
                args: audit.arguments,
            });
            let _ = events.send(DiagnoseEvent::McpResult {
                tool: audit.tool,
                result: serde_json::json!({"isError":audit.is_error}),
            });
        }
    });
    let prompt="请只通过 unity-profiler MCP 工具分析当前录制，先调用 performance_session_summary、performance_frames 和 performance_analysis，再选择有证据的帧调用 performance_frame / performance_cpu_hierarchy。用中文给出简洁结论、具体帧号和数据依据。遵守摘要中的 metricSemantics：分位数不是平均值，不能把小样本 p50 判为口径冲突；没有已验证的 exclusive/self 时间时不推断未解释或剩余 CPU 时间。performance_analysis 的默认阈值仅用于筛查，不是用户预算；核对 evidenceFrames 的原始样本后再解释，isolated-peak 是少量峰值而不是持续超限。issues 为空不代表没有性能问题。仅 quality.status=available 可作确定性结论；partial 必须说明覆盖率，estimated/unavailable 不作确定性诊断。CPU 为 inclusive，父子耗时不可相加；GC 单位字节。调用树必须检查 queryWarnings、depthTruncated 和 nextStart；深度截断时增大 max_depth 并从 start=0 重查，有 nextStart 时续页；线程列表同样检查 nextStart。若未读取完整，明确说明只查看部分样本，不声称完整归因。GC.Alloc 可嵌套，每个样本的字节单独计入；按线程及最近的非 GC.Alloc 父样本归因。结论中的分配明细必须与已校验的线程/帧 GC 总量核对；不一致时说明尚未解释的差额，不猜测原因，不丢弃嵌套分配。缺失数据明确说明。不要读取或修改工作目录文件，不执行命令，不访问网络。".to_owned();
    let mut peer = protocol::Peer::new(stdout, stdin, cancel, req.event_tx);
    let result = peer.run(&workspace.0, config, prompt).await;
    let chunks = peer.chunks;
    drop(peer);
    bridge.shutdown().await;
    drop(tree); // terminates the Windows adapter and all job descendants
    let _ = child.kill().await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while workers.join_next().await.is_some() {}
    })
    .await;
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    result.map(|reason| (chunks, reason))
}
