use super::Quality;
use crate::parser::{Frame, Sample};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameTimeStats {
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
    pub quality: Quality,
}
impl FrameTimeStats {
    pub fn new(mut values: Vec<f64>, quality: Quality) -> Self {
        values.sort_by(f64::total_cmp);
        let p = |q: f64| {
            if values.is_empty() {
                None
            } else {
                Some(values[((values.len() - 1) as f64 * q).round() as usize])
            }
        };
        Self {
            p50: p(0.5),
            p95: p(0.95),
            p99: p(0.99),
            max: values.last().copied(),
            quality,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotspot {
    pub name: String,
    pub total_ms: f64,
    pub call_count: u64,
    pub avg_ms: f64,
    pub max_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSample {
    pub frame_index: usize,
    pub ms: Option<f64>,
    pub frame_time_ms: Option<f64>,
    pub gc_alloc_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    pub main_thread_ms: FrameTimeStats,
    pub top_hotspots: Vec<Hotspot>,
    pub hotspot_quality: Quality,
    pub frame_timeline: Vec<FrameSample>,
}
pub fn aggregate<'a>(samples: impl Iterator<Item = &'a Sample>) -> Vec<Hotspot> {
    let mut acc: BTreeMap<String, (f64, u64, f64)> = BTreeMap::new();
    for s in samples {
        let a = acc.entry(s.name.clone()).or_default();
        a.0 += s.total_ms;
        a.1 += s.call_count;
        a.2 = a.2.max(s.max_ms);
    }
    let mut rows: Vec<_> = acc
        .into_iter()
        .map(|(name, (total_ms, call_count, max_ms))| Hotspot {
            name,
            total_ms,
            call_count,
            max_ms,
            avg_ms: if call_count == 0 {
                0.0
            } else {
                total_ms / call_count as f64
            },
        })
        .collect();
    rows.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms).then(a.name.cmp(&b.name)));
    rows
}
pub fn extract(frames: &[Frame]) -> CpuMetrics {
    CpuMetrics {
        main_thread_ms: FrameTimeStats::new(
            frames
                .iter()
                .filter(|f| f.quality.cpu)
                .map(|f| f.cpu_ms)
                .collect(),
            Quality::from_frames(frames, |f| f.quality.cpu),
        ),
        top_hotspots: aggregate(
            frames
                .iter()
                .filter(|f| f.quality.samples)
                .flat_map(|f| &f.main_thread_samples),
        ),
        hotspot_quality: Quality::from_frames(frames, |f| f.quality.samples),
        frame_timeline: frames
            .iter()
            .map(|f| FrameSample {
                frame_index: f.index,
                ms: f.quality.cpu.then_some(f.cpu_ms),
                frame_time_ms: f.quality.duration.then_some(f.duration_ms),
                gc_alloc_bytes: f.quality.gc.then_some(f.gc_alloc_bytes),
            })
            .collect(),
    }
}
