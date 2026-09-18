//! CPU 帧时间 + 热点提取

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::parser::Frame;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    /// 主线程耗时分位数（ms）
    pub main_thread_ms: FrameTimeStats,
    /// 主线程 Top 热点
    pub top_hotspots: Vec<Hotspot>,
    /// 帧时间序列（用于绘图）
    pub frame_timeline: Vec<FrameSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameTimeStats {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
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
    pub ms: f64,
}

pub fn extract(frames: &[Frame]) -> CpuMetrics {
    let mut durations: Vec<f64> = frames
        .iter()
        .map(|f| if f.cpu_ms > 0.0 { f.cpu_ms } else { f.duration_ms })
        .filter(|d| *d > 0.0)
        .collect();

    durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let main_thread_ms = FrameTimeStats {
        p50: percentile(&durations, 0.50),
        p95: percentile(&durations, 0.95),
        p99: percentile(&durations, 0.99),
        max: durations.last().copied().unwrap_or(0.0),
    };

    let top_hotspots = aggregate_hotspots(frames);
    let frame_timeline: Vec<FrameSample> = frames
        .iter()
        .map(|f| FrameSample {
            frame_index: f.index,
            ms: if f.cpu_ms > 0.0 { f.cpu_ms } else { f.duration_ms },
        })
        .filter(|s| s.ms > 0.0)
        .collect();

    CpuMetrics {
        main_thread_ms,
        top_hotspots,
        frame_timeline,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn aggregate_hotspots(frames: &[Frame]) -> Vec<Hotspot> {
    let mut acc: HashMap<String, (f64, u64, f64)> = HashMap::new();
    for frame in frames {
        for sample in &frame.main_thread_samples {
            let entry = acc.entry(sample.name.clone()).or_insert((0.0, 0, 0.0));
            entry.0 += sample.total_ms;
            entry.1 += sample.call_count;
            if sample.max_ms > entry.2 {
                entry.2 = sample.max_ms;
            }
        }
    }

    let mut hotspots: Vec<Hotspot> = acc
        .into_iter()
        .map(|(name, (total_ms, call_count, max_ms))| Hotspot {
            avg_ms: if call_count > 0 { total_ms / call_count as f64 } else { 0.0 },
            name,
            total_ms,
            call_count,
            max_ms,
        })
        .collect();

    hotspots.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap_or(std::cmp::Ordering::Equal));
    hotspots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Sample;

    fn make_frame(idx: usize, ms: f64, hotspots: Vec<(&str, f64)>) -> Frame {
        Frame {
            index: idx,
            duration_ms: ms,
            cpu_ms: ms,
            gc_alloc_bytes: 0,
            draw_calls: 0,
            set_pass_calls: 0,
            main_thread_samples: hotspots
                .into_iter()
                .map(|(n, total)| Sample {
                    name: n.to_string(),
                    total_ms: total,
                    call_count: 1,
                    max_ms: total,
                })
                .collect(),
            gc_alloc_sites: vec![],
            render_events: vec![],
        }
    }

    #[test]
    fn percentile_basic() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 0.5), 3.0);
        assert_eq!(percentile(&v, 0.95), 5.0);
    }

    #[test]
    fn aggregates_hotspots() {
        let frames = vec![
            make_frame(0, 14.0, vec![("Update", 8.0), ("Render", 6.0)]),
            make_frame(1, 16.0, vec![("Update", 9.0), ("Render", 7.0)]),
            make_frame(2, 17.0, vec![("Update", 10.0), ("Render", 7.0)]),
            make_frame(3, 18.0, vec![("Update", 11.0), ("Render", 7.0)]),
            make_frame(4, 20.0, vec![("Update", 12.0), ("Render", 8.0)]),
        ];
        let m = extract(&frames);
        assert_eq!(m.top_hotspots[0].name, "Update");
        assert!((m.top_hotspots[0].total_ms - 50.0).abs() < 0.01);
        assert_eq!(m.main_thread_ms.p50, 17.0);
        assert_eq!(m.main_thread_ms.max, 20.0);
    }
}