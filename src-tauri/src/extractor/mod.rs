//! 从 ParsedProfile 提取核心指标
//!
//! 输出 `MetricsSnapshot` 给前端 / MCP Server 共享。
//! 快照包含指标可用性和有效帧范围，不将缺失数据编码为零。

use serde::{Deserialize, Serialize};

use crate::parser::ParsedProfile;

pub mod cpu;
pub mod gc;
pub mod memory;
pub mod rendering;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSnapshot {
    #[serde(default)]
    pub memory: memory::MemoryMetrics,
    pub meta: SnapshotMeta,
    pub cpu: cpu::CpuMetrics,
    pub gc: gc::GcMetrics,
    pub rendering: rendering::RenderingMetrics,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    #[serde(default)]
    pub parsing: Option<ParsingCoverage>,
    pub duration_quality: Quality,
    pub declared_frame_count: usize,
    pub source: String,
    pub file_name: String,
    pub duration_ms: Option<f64>,
    pub frame_count: usize,
    pub platform: Option<String>,
    pub unity_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsingCoverage {
    pub raw_blocks: usize,
    pub decoded_frames: usize,
    pub skipped_frames: usize,
    pub failed_frames: usize,
    pub validation: String,
}
/// 提取入口
pub fn extract(profile: &ParsedProfile) -> MetricsSnapshot {
    MetricsSnapshot {
        memory: memory::extract(&profile.frames),
        meta: SnapshotMeta {
            parsing: (profile.meta.format == crate::parser::ProfilerFormat::Data).then(|| {
                ParsingCoverage {
                    raw_blocks: profile.frames.len(),
                    decoded_frames: profile
                        .frames
                        .iter()
                        .filter(|f| !f.quality.source.ends_with("-skipped"))
                        .count(),
                    skipped_frames: profile
                        .frames
                        .iter()
                        .filter(|f| f.quality.source.ends_with("-skipped"))
                        .count(),
                    failed_frames: 0,
                    validation: if profile
                        .frames
                        .iter()
                        .any(|f| f.quality.version_verified == Some(false))
                    {
                        "pending-editor-comparison"
                    } else {
                        "limited-capture-comparison"
                    }
                    .into(),
                }
            }),
            duration_quality: Quality::from_frames(&profile.frames, |f| f.quality.duration),
            declared_frame_count: profile.meta.frame_count,
            source: profile
                .frames
                .first()
                .map(|f| f.quality.source.clone())
                .unwrap_or_else(|| "unknown".into()),
            file_name: profile.meta.file_name.clone(),
            duration_ms: profile.frames.iter().any(|f| f.quality.duration).then(|| {
                profile
                    .frames
                    .iter()
                    .filter(|f| f.quality.duration)
                    .map(|f| f.duration_ms)
                    .sum()
            }),
            frame_count: profile.frames.len(),
            platform: profile.meta.platform.clone(),
            unity_version: profile.meta.unity_version.clone(),
        },
        cpu: cpu::extract(&profile.frames),
        gc: gc::extract(&profile.frames),
        rendering: rendering::extract(&profile.frames),
        warnings: profile.warnings.clone(),
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quality {
    pub status: String,
    pub source: String,
    pub reasons: Vec<String>,
    pub valid_frames: usize,
    pub total_frames: usize,
}
impl Quality {
    pub fn from_frames(
        frames: &[crate::parser::Frame],
        valid: impl Fn(&crate::parser::Frame) -> bool,
    ) -> Self {
        let count = frames.iter().filter(|f| valid(f)).count();
        let estimated = frames.iter().any(|f| valid(f) && f.quality.estimated);
        let unverified = frames
            .iter()
            .any(|f| f.quality.version_verified == Some(false));
        let status = if count == 0 {
            "unavailable"
        } else if count < frames.len() {
            "partial"
        } else if unverified {
            "unverified"
        } else if estimated {
            "estimated"
        } else {
            "available"
        };
        let mut reasons: Vec<String> = frames
            .iter()
            .filter(|f| !valid(f))
            .flat_map(|f| f.quality.reasons.clone())
            .collect();
        if count < frames.len() && reasons.is_empty() {
            reasons.push("输入未提供或解析器尚未验证该指标".into());
        }
        if unverified {
            reasons.push("版本待验证；不能作确定性达标结论".into());
        }
        if estimated {
            reasons.push("包含估算值，不能视为观测结果".into());
        }
        reasons.sort();
        reasons.dedup();
        Self {
            status: status.into(),
            source: frames
                .first()
                .map(|f| f.quality.source.clone())
                .unwrap_or_else(|| "unknown".into()),
            reasons,
            valid_frames: count,
            total_frames: frames.len(),
        }
    }
    pub fn can_diagnose(&self) -> bool {
        self.status == "available"
    }
}
