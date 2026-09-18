//! Tauri commands（前端 invoke 入口）

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::acp_client::agents::{builtin_presets, probe_available, AgentPreset};
use crate::acp_client::{start_diagnose, DiagnoseEvent};
use crate::extractor::{extract, MetricsSnapshot};
use crate::parser;
use crate::parser::data::parse_path_with_progress;
use crate::state::{generate_file_id, AppState, UploadEntry};

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("文件不存在: {0}")]
    FileNotFound(String),

    #[error("解析失败: {0}")]
    Parse(String),

    #[error("未找到 fileId: {0}")]
    UnknownFileId(String),

    #[error("Agent 未安装: {0}")]
    AgentNotInstalled(String),

    #[error("ACP 错误: {0}")]
    Acp(String),

    #[error("其他: {0}")]
    Other(String),
}

// 让 CommandError 可以被 Tauri 自动序列化为前端可见错误
impl serde::Serialize for CommandError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub file_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub extension: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseSession {
    pub session_id: String,
    pub agent_id: String,
}

/// 解析进度事件 payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseProgress {
    pub file_id: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub current_frame: usize,
}

/// 上传（实际是复制文件到 uploads/ 目录并登记 fileId）
#[tauri::command(rename_all = "camelCase")]
pub async fn upload(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<UploadResult, CommandError> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(CommandError::FileNotFound(file_path));
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unknown)")
        .to_string();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let size_bytes = tokio::fs::metadata(&path)
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
        .len();

    let file_id = generate_file_id();
    let entry = UploadEntry {
        file_id: file_id.clone(),
        file_path: path,
        file_name: file_name.clone(),
        size_bytes,
        extension: extension.clone(),
    };

    state.put_upload(entry).await;

    Ok(UploadResult {
        file_id,
        filename: file_name,
        size_bytes,
        extension,
    })
}

/// 解析与提取（带前端进度推送）
#[tauri::command(rename_all = "camelCase")]
pub async fn analyze(
    file_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<MetricsSnapshot, CommandError> {
    let entry = state
        .get_upload(&file_id)
        .await
        .ok_or_else(|| CommandError::UnknownFileId(file_id.clone()))?;

    // 解析：根据扩展名分发；`.data` 走流式路径带进度
    let file_ext = entry
        .file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let profile = if file_ext == "data" {
        let app_for_progress = app.clone();
        let file_id_for_progress = file_id.clone();
        parse_path_with_progress(&entry.file_path, &mut |done, total| {
            // 当前 frame 数 = done_bytes / avg_body_size（粗略估计）
            // 简化：只发 done/total，让前端算百分比
            let _ = app_for_progress.emit(
                "parse-progress",
                ParseProgress {
                    file_id: file_id_for_progress.clone(),
                    done_bytes: done,
                    total_bytes: total,
                    current_frame: 0, // parser 暂不报具体 frame index
                },
            );
        })
        .map_err(|e| CommandError::Parse(e.to_string()))?
    } else {
        parser::parse_file(&entry.file_path)
            .await
            .map_err(|e| CommandError::Parse(e.to_string()))?
    };

    let snapshot = extract(&profile);
    state.put_snapshot(file_id.clone(), snapshot.clone()).await;

    // 完成后发 100% 事件
    let _ = app.emit(
        "parse-progress",
        ParseProgress {
            file_id: file_id.clone(),
            done_bytes: profile.meta.file_size_bytes,
            total_bytes: profile.meta.file_size_bytes,
            current_frame: profile.meta.frame_count,
        },
    );

    Ok(snapshot)
}

/// 启动 AI 诊断
#[tauri::command(rename_all = "camelCase")]
pub async fn diagnose(
    file_id: String,
    agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DiagnoseSession, CommandError> {
    let snapshot = state
        .get_snapshot(&file_id)
        .await
        .ok_or_else(|| CommandError::UnknownFileId(file_id.clone()))?;

    let mut presets = builtin_presets();
    for preset in &mut presets {
        preset.available = probe_available(&preset.command);
    }
    let preset = presets
        .into_iter()
        .find(|p| p.id == agent_id)
        .ok_or_else(|| CommandError::AgentNotInstalled(agent_id.clone()))?;

    if !probe_available(&preset.command) {
        return Err(CommandError::AgentNotInstalled(preset.command.clone()));
    }

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<DiagnoseEvent>();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

    let req = crate::acp_client::DiagnoseRequest {
        file_id: file_id.clone(),
        agent_id: agent_id.clone(),
        snapshot,
        event_tx,
        cancel_rx,
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    state.register_session(file_id.clone()).await;

    let session = start_diagnose(preset, req)
        .await
        .map_err(|e| CommandError::Acp(e.to_string()))?;

    // 后台把 DiagnoseEvent 通过 Tauri emit 推到前端
    // webview 关闭 / 重载时 emit 会失败：log 一次、杀 agent、break，
    // 避免 ERROR 刷屏 + 防止 agent 在没人监听的 webview 上空转。
    let app_for_relay = app.clone();
    let relay_kill_handle = session.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let Err(err) = app_for_relay.emit("diagnose-event", event) {
                tracing::warn!(
                    target: "diagnose_relay",
                    "webview disconnected, stopping relay + killing agent: {}",
                    err
                );
                // 杀 agent child（supervisor abort → kill_on_drop）
                relay_kill_handle.cancel().await;
                break;
            }
        }
    });

    // 防止 session 被 drop（应在 cancel 时被使用）
    tokio::spawn(async move {
        let _session = session; // 保持
        drop(cancel_tx);
    });

    Ok(DiagnoseSession {
        session_id,
        agent_id,
    })
}

/// 取消诊断
#[tauri::command(rename_all = "camelCase")]
pub async fn cancel_diagnose(
    _file_id: String,
    _state: State<'_, AppState>,
) -> Result<(), CommandError> {
    // MVP 简化：实际 cancel 通过 sessionHandle 实现，这里先占位
    Ok(())
}

/// 列出可用 Agent 预设
#[tauri::command]
pub async fn list_agents() -> Result<Vec<AgentPreset>, CommandError> {
    let mut presets = builtin_presets();
    for preset in &mut presets {
        preset.available = probe_available(&preset.command);
    }
    Ok(presets)
}

#[allow(dead_code)]
fn _unused_arc() -> Arc<()> {
    Arc::new(())
}