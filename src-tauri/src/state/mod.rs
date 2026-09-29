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
    pub reports: HashMap<String, crate::reports::Report>,
    pub sources: HashMap<String, Arc<crate::source::SourceScope>>,
    pub projects: HashMap<String, Arc<crate::project::ProjectScope>>,
    pub preparations: HashMap<String, Arc<std::sync::atomic::AtomicBool>>,
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
            reports: HashMap::new(),
            sources: HashMap::new(),
            projects: HashMap::new(),
            preparations: HashMap::new(),
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
        inner.projects.retain(|_, p| {
            if p.info.file_id == file_id {
                p.cancelled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                false
            } else {
                true
            }
        });
        inner.reports.retain(|_, r| r.file_id != file_id);
        if let Some(flag) = inner.preparations.remove(file_id) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        inner.sources.retain(|_, s| {
            if s.info.file_id == file_id {
                s.cancelled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                false
            } else {
                true
            }
        });
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
        self.start_report_session(file_id, preset, executable, None)
            .await
    }
    pub async fn start_report_session(
        &self,
        file_id: &str,
        preset: crate::acp_client::agents::AgentPreset,
        executable: PathBuf,
        source_request: Option<(String, String)>,
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
        if inner.preparations.contains_key(file_id) {
            return Err(AcpError::Session("源码目录正在准备".into()));
        }
        let (parent, source, project) = if let Some((report_id, scope_id)) = source_request {
            let report = inner
                .reports
                .get(&report_id)
                .filter(|r| {
                    r.file_id == file_id && r.stage == "performance" && r.status == "completed"
                })
                .cloned()
                .ok_or_else(|| AcpError::Session("需要当前录制的完整首轮报告".into()))?;
            let source = inner
                .sources
                .get(&scope_id)
                .filter(|s| {
                    s.info.file_id == file_id
                        && !s.cancelled.load(std::sync::atomic::Ordering::Relaxed)
                })
                .cloned();
            let project = inner
                .projects
                .get(&scope_id)
                .filter(|p| {
                    p.info.file_id == file_id
                        && !p.cancelled.load(std::sync::atomic::Ordering::Relaxed)
                })
                .cloned();
            if source.is_none() && project.is_none() {
                return Err(AcpError::Session(
                    "源码/工程范围已失效，请重新准备目录".into(),
                ));
            }
            (Some(report), source, project)
        } else {
            (None, None, None)
        };
        let (event_tx, mut agent_rx) = tokio::sync::mpsc::unbounded_channel();
        let (relay_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let id = Uuid::new_v4().to_string();
        let mut report = crate::reports::Report::new(
            id.clone(),
            file_id.into(),
            preset.id.clone(),
            parent.as_ref().map(|r| r.report_id.clone()),
            &snapshot,
        );
        if let Some(p) = &project {
            report.stage = "project".into();
            report.project_context = Some(p.context());
        }
        let report_project = project.clone();
        let req = DiagnoseRequest {
            project,
            source,
            parent_report: parent.as_ref().map(|r| r.markdown()),
            file_id: file_id.into(),
            agent_id: preset.id.clone(),
            snapshot,
            details: inner.details.get(file_id).cloned(),
            bridge_executable: executable,
            event_tx,
        };
        let handle = start_diagnose(preset, req).await?;
        inner
            .reports
            .retain(|_, r| r.file_id != file_id || (parent.is_some() && r.stage == "performance"));
        inner.reports.insert(id.clone(), report);
        let relay_state = self.clone();
        let report_id = id.clone();
        tokio::spawn(async move {
            while let Some(event) = agent_rx.recv().await {
                let terminal = event.terminal();
                {
                    let mut inner = relay_state.0.lock().await;
                    if let Some(r) = inner.reports.get_mut(&report_id) {
                        r.apply(&event);
                        if terminal {
                            if let Some(p) = &report_project {
                                r.project_context = Some(p.context());
                            }
                        }
                    }
                }
                let _ = relay_tx.send(event);
                if terminal {
                    break;
                }
            }
            let mut inner = relay_state.0.lock().await;
            if let Some(r) = inner.reports.get_mut(&report_id) {
                if r.status == "running" {
                    r.apply(&crate::acp_client::DiagnoseEvent::Error {
                        message: "诊断流意外结束".into(),
                    });
                }
            }
        });
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
        {
            let inner = self.0.lock().await;
            if let Some(session) = inner.active_sessions.get(session_id) {
                for project in inner
                    .projects
                    .values()
                    .filter(|p| p.info.file_id == session.file_id)
                {
                    project
                        .cancelled
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                for scope in inner
                    .sources
                    .values()
                    .filter(|s| s.info.file_id == session.file_id)
                {
                    scope
                        .cancelled
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
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
        {
            let mut inner = self.0.lock().await;
            if let Some(report) = inner.reports.get_mut(session_id) {
                report.apply(&crate::acp_client::DiagnoseEvent::Cancelled);
            }
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
