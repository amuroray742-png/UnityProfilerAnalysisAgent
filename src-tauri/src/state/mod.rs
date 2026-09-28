//! 全局应用状态：上传文件、解析结果、诊断会话

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::extractor::MetricsSnapshot;
use crate::parser::detail::FrameStore;

#[derive(Debug, Clone)]
pub struct UploadEntry {
    pub file_id: String,
    pub file_path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub extension: String,
}

#[derive(Debug)]
pub struct AppStateInner {
    /// file_id → UploadEntry
    pub uploads: HashMap<String, UploadEntry>,
    /// file_id → MetricsSnapshot
    pub snapshots: HashMap<String, MetricsSnapshot>,
    pub details: HashMap<String, Arc<FrameStore>>,
    /// active session_id（用于 cancel）
    pub active_sessions: HashMap<String, ActiveSession>,
}

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub file_id: String,
    pub handle: crate::acp_client::client::SessionHandle,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Mutex<AppStateInner>>);

impl AppState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(AppStateInner {
            uploads: HashMap::new(),
            snapshots: HashMap::new(),
            details: HashMap::new(),
            active_sessions: HashMap::new(),
        })))
    }

    pub async fn put_upload(&self, entry: UploadEntry) -> String {
        let file_id = entry.file_id.clone();
        self.0.lock().await.uploads.insert(file_id.clone(), entry);
        file_id
    }

    pub async fn put_snapshot(&self, file_id: String, snapshot: MetricsSnapshot) {
        let mut inner = self.0.lock().await;
        inner.details.remove(&file_id);
        inner.snapshots.insert(file_id, snapshot);
    }

    pub async fn put_analysis(
        &self,
        file_id: String,
        snapshot: MetricsSnapshot,
        details: Option<Arc<FrameStore>>,
    ) -> bool {
        let mut inner = self.0.lock().await;
        if !inner.uploads.contains_key(&file_id) {
            return false;
        }
        inner.details.remove(&file_id);
        if let Some(details) = details {
            inner.details.insert(file_id.clone(), details);
        }
        inner.snapshots.insert(file_id, snapshot);
        true
    }
    pub async fn get_details(&self, file_id: &str) -> Option<Arc<FrameStore>> {
        self.0.lock().await.details.get(file_id).cloned()
    }
    pub async fn release_file(&self, file_id: &str) {
        let mut inner = self.0.lock().await;
        inner.uploads.remove(file_id);
        inner.snapshots.remove(file_id);
        inner.details.remove(file_id);
        let sessions: Vec<_> = inner
            .active_sessions
            .iter()
            .filter(|(_, s)| s.file_id == file_id)
            .map(|(id, _)| id.clone())
            .collect();
        let handles: Vec<_> = sessions
            .into_iter()
            .filter_map(|id| inner.active_sessions.remove(&id))
            .map(|s| s.handle)
            .collect();
        drop(inner);
        for handle in handles {
            handle.cancel().await;
        }
    }

    pub async fn get_snapshot(&self, file_id: &str) -> Option<MetricsSnapshot> {
        self.0.lock().await.snapshots.get(file_id).cloned()
    }

    pub async fn get_upload(&self, file_id: &str) -> Option<UploadEntry> {
        self.0.lock().await.uploads.get(file_id).cloned()
    }

    pub async fn finish_session(&self, session_id: &str) {
        self.0.lock().await.active_sessions.remove(session_id);
    }
    pub async fn start_session(
        &self,
        file_id: &str,
        preset: crate::acp_client::agents::AgentPreset,
        executable: PathBuf,
    ) -> Result<
        (
            String,
            crate::acp_client::client::SessionHandle,
            tokio::sync::mpsc::UnboundedReceiver<crate::acp_client::DiagnoseEvent>,
        ),
        crate::acp_client::AcpError,
    > {
        use crate::acp_client::{start_diagnose, AcpError, DiagnoseRequest};
        let mut inner = self.0.lock().await;
        if inner.active_sessions.values().any(|s| s.file_id == file_id) {
            return Err(AcpError::Session("该文件已有诊断会话，请先取消".into()));
        }
        let snapshot = inner
            .snapshots
            .get(file_id)
            .cloned()
            .ok_or_else(|| AcpError::Session("未找到该文件的分析结果".into()))?;
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let req = DiagnoseRequest {
            file_id: file_id.into(),
            agent_id: preset.id.clone(),
            snapshot,
            details: inner.details.get(file_id).cloned(),
            bridge_executable: executable,
            event_tx,
        };
        let handle = start_diagnose(preset, req).await?;
        let id = Uuid::new_v4().to_string();
        inner.active_sessions.insert(
            id.clone(),
            ActiveSession {
                file_id: file_id.into(),
                handle: handle.clone(),
            },
        );
        Ok((id, handle, event_rx))
    }
    pub async fn cancel_session(&self, session_id: &str) {
        let handle = self
            .0
            .lock()
            .await
            .active_sessions
            .get(session_id)
            .map(|s| s.handle.clone());
        if let Some(handle) = handle {
            handle.cancel().await;
        }
        self.finish_session(session_id).await;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn generate_file_id() -> String {
    Uuid::new_v4().to_string()
}
