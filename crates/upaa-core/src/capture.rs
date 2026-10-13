use serde::{Serialize,Deserialize};
use std::path::PathBuf;
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
