//! Persistent optimization workspaces. No Agent owns the edit/rollback ledger.
pub mod plugin;
pub mod archive;
pub mod observation;
pub mod automatic;
pub mod commands;
pub mod comparison;
pub mod editing;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub evidence: String,
    pub files: BTreeMap<String, String>,
    pub instructions: String,
    pub acceptance: String,
    pub constraints: String,
    pub selected: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conditions {
    pub device: String,
    pub platform: String,
    pub scenario: String,
    pub operation: String,
    pub build: String,
    pub quality: String,
    pub resolution: String,
    pub profiling: String,
    pub code_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub id: String,
    pub path: PathBuf,
    pub hash: String,
    pub conditions: Conditions,
    pub snapshot: crate::extractor::MetricsSnapshot,
    pub frames: Vec<crate::parser::Frame>,
}
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
