//! 从 ParsedProfile 提取核心指标
//!
//! 输出 `MetricsSnapshot` 给前端 / MCP Server 共享。
//! 控制 JSON 大小在 ~20-50KB，便于 Agent 通过 MCP 工具快速返回。

use serde::{Deserialize, Serialize};

use crate::parser::ParsedProfile;

pub mod cpu;
pub mod gc;
pub mod rendering;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSnapshot {
    pub meta: SnapshotMeta,
    pub cpu: cpu::CpuMetrics,
    pub gc: gc::GcMetrics,
    pub rendering: rendering::RenderingMetrics,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    pub file_name: String,
    pub duration_ms: f64,
    pub frame_count: usize,
    pub platform: Option<String>,
    pub unity_version: Option<String>,
}

/// 提取入口
pub fn extract(profile: &ParsedProfile) -> MetricsSnapshot {
    MetricsSnapshot {
        meta: SnapshotMeta {
            file_name: profile.meta.file_name.clone(),
            duration_ms: profile.meta.duration_ms,
            frame_count: profile.frames.len().max(profile.meta.frame_count),
            platform: profile.meta.platform.clone(),
            unity_version: profile.meta.unity_version.clone(),
        },
        cpu: cpu::extract(&profile.frames),
        gc: gc::extract(&profile.frames),
        rendering: rendering::extract(&profile.frames),
        warnings: profile.warnings.clone(),
    }
}