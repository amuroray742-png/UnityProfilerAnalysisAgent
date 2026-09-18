//! Stream relay: 把内部 DiagnoseEvent 流转化为 Tauri event 推到前端
//!
//! MVP 桩：在 commands/mod.rs 里直接调用 app.emit。

use tauri::{AppHandle, Emitter};

use super::DiagnoseEvent;

/// 把内部事件 emit 到前端，前端 `listen('diagnose-event', ...)` 接收
pub fn emit_event(app: &AppHandle, event: &DiagnoseEvent) {
    if let Err(err) = app.emit("diagnose-event", event.clone()) {
        tracing::error!("Failed to emit diagnose event: {}", err);
    }
}