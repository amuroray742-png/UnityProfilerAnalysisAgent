//! Tauri commands（前端 invoke 入口）

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use thiserror::Error;

use crate::acp_client::agents::{builtin_presets, probe_available, AgentPreset};

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

/// 登记原文件路径与 fileId；不复制录制文件。
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
        tokio::task::spawn_blocking(move || {
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
        })
        .await
        .map_err(|e| CommandError::Parse(e.to_string()))?
        .map_err(|e| CommandError::Parse(e.to_string()))?
    } else {
        parser::parse_file(&entry.file_path)
            .await
            .map_err(|e| CommandError::Parse(e.to_string()))?
    };

    let total_bytes = profile.meta.file_size_bytes;
    let frame_count = profile.frames.len();
    let details = profile.details.clone();
    let snapshot = tokio::task::spawn_blocking(move || extract(&profile))
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?;
    if !state
        .put_analysis(file_id.clone(), snapshot.clone(), details)
        .await
    {
        return Err(CommandError::UnknownFileId(file_id));
    }

    // 完成后发 100% 事件
    let _ = app.emit(
        "parse-progress",
        ParseProgress {
            file_id: file_id.clone(),
            done_bytes: total_bytes,
            total_bytes,
            current_frame: frame_count,
        },
    );

    Ok(snapshot)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn frame_details(
    file_id: String,
    frame_index: usize,
    start: usize,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<parser::detail::FramePage, CommandError> {
    let source = state.get_details(&file_id).await.ok_or_else(|| {
        CommandError::Other("该 fileId 没有可用调用树，请先导入支持的 data 或 Editor dump".into())
    })?;
    tokio::task::spawn_blocking(move || source.frame(frame_index, start, limit))
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
        .map_err(|e| CommandError::Other(e.to_string()))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn cpu_hierarchy(
    file_id: String,
    frame_index: usize,
    thread_index: Option<usize>,
    start: usize,
    limit: usize,
    max_depth: usize,
    state: State<'_, AppState>,
) -> Result<parser::detail::HierarchyPage, CommandError> {
    let source = state.get_details(&file_id).await.ok_or_else(|| {
        CommandError::Other("该 fileId 没有可用调用树，请先导入支持的 data 或 Editor dump".into())
    })?;
    tokio::task::spawn_blocking(move || {
        source.hierarchy(frame_index, thread_index, start, limit, max_depth)
    })
    .await
    .map_err(|e| CommandError::Other(e.to_string()))?
    .map_err(|e| CommandError::Other(e.to_string()))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn release_file(file_id: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    state.release_file(&file_id).await;
    Ok(())
}

/// 启动会话：原子绑定快照、详情与取消句柄；握手在后台进行。
#[tauri::command(rename_all = "camelCase")]
pub async fn diagnose(
    file_id: String,
    agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<DiagnoseSession, CommandError> {
    let preset = builtin_presets()
        .into_iter()
        .find(|p| p.id == agent_id)
        .ok_or_else(|| CommandError::AgentNotInstalled(agent_id.clone()))?;
    let executable = std::env::current_exe().map_err(|e| CommandError::Other(e.to_string()))?;
    let (session_id, session, mut event_rx) = state
        .start_session(&file_id, preset, executable)
        .await
        .map_err(|e| CommandError::Acp(e.to_string()))?;
    let relay_state = state.inner().clone();
    let relay_id = session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let terminal = event.terminal();
            let scoped = crate::acp_client::SessionEvent {
                session_id: relay_id.clone(),
                file_id: file_id.clone(),
                event,
            };
            if app.emit("diagnose-event", scoped).is_err() {
                session.cancel().await;
                break;
            }
            if terminal {
                break;
            }
        }
        relay_state.finish_session(&relay_id).await;
    });
    Ok(DiagnoseSession {
        session_id,
        agent_id,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn cancel_diagnose(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state.cancel_session(&session_id).await;
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
