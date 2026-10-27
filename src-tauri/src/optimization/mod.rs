//! Persistent optimization workspaces. No Agent owns the edit/rollback ledger.
pub mod archive;
pub mod automatic;
pub mod commands;
pub mod comparison;
pub mod editing;
pub mod observation;
pub mod plugin;
pub mod sampling;
pub mod scenes;
pub mod session;
pub mod storage;
pub mod workflow;
use crate::reports::Report;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

pub use upaa_core::proposal::Task;

pub use upaa_core::capture::{Conditions, Capture};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default = "modify_kind")]
    pub kind: String,
    pub path: String,
    pub before_hash: String,
    pub after_hash: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub state: String,
}
fn modify_kind() -> String {
    "modify".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    #[serde(default)]
    pub sequence: u64,
    #[serde(default)]
    pub automatic: bool,
    #[serde(default)]
    pub requirements: String,
    #[serde(skip)]
    pub read_receipts: BTreeMap<String, String>,
    #[serde(default)]
    pub task_version: u64,
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub tasks: Vec<Task>,
    pub status: String,
    pub text: String,
    pub changes: Vec<Change>,
    pub checks: Vec<serde_json::Value>,
    pub created_at: String,
    pub reason: Option<String>,
    #[serde(default)]
    pub baseline_check: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round {
    #[serde(default)]
    pub scene_id: String,
    #[serde(default)]
    pub workflow: workflow::Progress,
    #[serde(default)]
    pub task_version: u64,
    #[serde(default)]
    pub task_verifications: BTreeMap<String, String>,
    pub id: String,
    pub baseline: String,
    pub candidate: Option<String>,
    pub reports: Vec<Report>,
    pub tasks: Vec<Task>,
    pub runs: Vec<Run>,
    #[serde(default)]
    pub tests: Vec<String>,
    pub comparison: Option<serde_json::Value>,
    pub correctness: String,
    pub decision: String,
}
impl Round {
    pub fn performance_status(&self) -> &'static str {
        if !self.runs.is_empty() && self.runs.iter().all(|r| r.status == "rolled_back") {
            return "不可判定（本轮已回退）";
        }
        let Some(comparison) = &self.comparison else {
            return "待录制";
        };
        if self.tasks.iter().any(|t| t.selected)
            && self
                .tasks
                .iter()
                .filter(|t| t.selected)
                .all(|t| t.kind == "marker")
        {
            return "不可判定（Marker 任务单独验收）";
        }
        let Some(metrics) = comparison["metrics"].as_array() else {
            return "不可判定";
        };
        let targets: Vec<_> = metrics
            .iter()
            .filter(|m| m["budgetP95"].is_number())
            .collect();
        if targets.is_empty() || targets.iter().any(|m| m["verdict"] == "不可判定") {
            "不可判定"
        } else if targets.iter().any(|m| m["verdict"] == "未达到目标") {
            "未达到目标"
        } else {
            "达到目标"
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default)]
    pub scenes: Vec<scenes::Scene>,
    #[serde(default)]
    pub active_scene_id: String,
    #[serde(default)]
    pub code_revision: u64,
    #[serde(default)]
    pub run_sequence: u64,
    pub version: u32,
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub budgets: BTreeMap<String, f64>,
    pub captures: Vec<Capture>,
    pub rounds: Vec<Round>,
}
#[derive(Debug)]
pub struct Workspace {
    pub observation: Mutex<observation::Observation>,
    pub directory: PathBuf,
    pub save_error: Mutex<Option<String>>,
    pub data: Mutex<Project>,
    pub cancelled: Arc<AtomicBool>,
    pub cancel_epoch: std::sync::atomic::AtomicU64,
    /// A process-wide workspace lease; editor actions also take `data`.
    pub busy: AtomicBool,
    pub(crate) _lease: std::fs::File,
    pub(crate) _root_lease: std::fs::File,
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
