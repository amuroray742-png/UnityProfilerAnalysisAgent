//! ACP Agent 子进程管理
//!
//! 极简 stdio 协议（不依赖 agent-client-protocol SDK）：
//! - prompt 通过 stdin 写入，EOF 由 tokio::process::Child 的 drop 触发
//! - stdout 按行切 chunk 推到前端
//! - stderr 推到 [error] 事件
//! - 子进程退出时 emit Finished

use std::process::Stdio;
use std::sync::Arc;

use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::agents::{resolve_command, AgentPreset};
use super::AcpError;

/// 会话句柄：内部 Arc<Mutex<Option<JoinHandle>>>，Clone 之后多个 owner 都能 cancel
///
/// cancel() = abort supervisor。supervisor 在 future drop 时会带着
/// `kill_on_drop=true` 的 Child 一起 drop，进程被强杀。
#[derive(Clone)]
pub struct SessionHandle {
    inner: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl SessionHandle {
    pub fn new(supervisor: JoinHandle<()>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(supervisor))),
        }
    }

    /// abort supervisor task（kill_on_drop 会级联杀掉 child 进程）。
    /// 多次 cancel 安全：第一次会 abort，后续 no-op。
    pub async fn cancel(&self) {
        if let Some(h) = self.inner.lock().await.take() {
            h.abort();
        }
    }
}

/// spawn Agent 子进程
///
/// 通过 `resolve_command` 处理 .ps1 / .cmd / .bat 等 Windows shim 脚本：
/// - `.ps1` → `powershell -NoProfile -ExecutionPolicy Bypass -File <path> <args>`
/// - `.cmd` / `.bat` → `cmd /C <path> <args>`
/// - 其他 → 直接 spawn
pub async fn spawn_agent(preset: &AgentPreset) -> Result<Child, AcpError> {
    let (program, args) = resolve_command(&preset.command, &preset.args)
        .ok_or_else(|| AcpError::AgentNotInstalled(preset.command.clone()))?;

    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = cmd.spawn()?;
    Ok(child)
}