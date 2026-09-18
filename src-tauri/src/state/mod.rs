//! 全局应用状态：上传文件、解析结果、诊断会话

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::extractor::MetricsSnapshot;

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
    /// active session_id（用于 cancel）
    pub active_sessions: HashMap<String, Uuid>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Mutex<AppStateInner>>);

impl AppState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(AppStateInner {
            uploads: HashMap::new(),
            snapshots: HashMap::new(),
            active_sessions: HashMap::new(),
        })))
    }

    pub async fn put_upload(&self, entry: UploadEntry) -> String {
        let file_id = entry.file_id.clone();
        self.0.lock().await.uploads.insert(file_id.clone(), entry);
        file_id
    }

    pub async fn put_snapshot(&self, file_id: String, snapshot: MetricsSnapshot) {
        self.0.lock().await.snapshots.insert(file_id, snapshot);
    }

    pub async fn get_snapshot(&self, file_id: &str) -> Option<MetricsSnapshot> {
        self.0.lock().await.snapshots.get(file_id).cloned()
    }

    pub async fn get_upload(&self, file_id: &str) -> Option<UploadEntry> {
        self.0.lock().await.uploads.get(file_id).cloned()
    }

    pub async fn register_session(&self, file_id: String) -> Uuid {
        let id = Uuid::new_v4();
        self.0.lock().await.active_sessions.insert(file_id, id);
        id
    }

    pub async fn cancel_session(&self, file_id: &str) -> Option<Uuid> {
        self.0.lock().await.active_sessions.remove(file_id)
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