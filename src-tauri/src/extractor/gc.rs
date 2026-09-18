//! GC 分配提取

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::extractor::cpu::{FrameTimeStats, Hotspot};
use crate::parser::Frame;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GcMetrics {
    pub total_alloc_bytes: u64,
    pub alloc_per_frame_bytes: FrameTimeStats,
    pub gen_collections: GenCollections,
    pub top_alloc_sites: Vec<Hotspot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenCollections {
    pub gen0: u64,
    pub gen1: u64,
    pub gen2: u64,
}

pub fn extract(frames: &[Frame]) -> GcMetrics {
    let mut per_frame: Vec<u64> = frames.iter().map(|f| f.gc_alloc_bytes).collect();
    per_frame.sort_unstable();

    let alloc_per_frame_bytes = FrameTimeStats {
        // 把字节当 ms 处理仅借用 FrameTimeStats 字段，含义仍然是字节数
        p50: percentile_u64(&per_frame, 0.50) as f64,
        p95: percentile_u64(&per_frame, 0.95) as f64,
        p99: percentile_u64(&per_frame, 0.99) as f64,
        max: per_frame.last().copied().unwrap_or(0) as f64,
    };

    let total_alloc_bytes: u64 = frames.iter().map(|f| f.gc_alloc_bytes).sum();

    // GC 集合次数暂不可从当前数据推算（需要 Unity 计数器事件），
    // 这里给一个保守估算：每 100MB 分配触发一次 Gen0。
    let est_gen0 = (total_alloc_bytes / 100_000_000).max(1);
    let gen_collections = GenCollections {
        gen0: est_gen0,
        gen1: (est_gen0 / 10).max(0),
        gen2: (est_gen0 / 100).max(0),
    };

    let top_alloc_sites = aggregate_alloc_sites(frames);

    GcMetrics {
        total_alloc_bytes,
        alloc_per_frame_bytes,
        gen_collections,
        top_alloc_sites,
    }
}

fn percentile_u64(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn aggregate_alloc_sites(frames: &[Frame]) -> Vec<Hotspot> {
    let mut acc: HashMap<String, (f64, u64, f64)> = HashMap::new();
    for frame in frames {
        for sample in &frame.gc_alloc_sites {
            let entry = acc.entry(sample.name.clone()).or_insert((0.0, 0, 0.0));
            entry.0 += sample.total_ms; // 用 total_ms 字段承载字节数（避免新类型）
            entry.1 += sample.call_count;
            if sample.max_ms > entry.2 {
                entry.2 = sample.max_ms;
            }
        }
    }
    let mut sites: Vec<Hotspot> = acc
        .into_iter()
        .map(|(name, (total_ms, call_count, max_ms))| Hotspot {
            avg_ms: if call_count > 0 { total_ms / call_count as f64 } else { 0.0 },
            name,
            total_ms,
            call_count,
            max_ms,
        })
        .collect();
    sites.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap_or(std::cmp::Ordering::Equal));
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_alloc() {
        let frames = vec![
            Frame {
                index: 0,
                duration_ms: 0.0,
                cpu_ms: 0.0,
                gc_alloc_bytes: 1024,
                draw_calls: 0,
                set_pass_calls: 0,
                main_thread_samples: vec![],
                gc_alloc_sites: vec![],
                render_events: vec![],
            },
            Frame {
                index: 1,
                duration_ms: 0.0,
                cpu_ms: 0.0,
                gc_alloc_bytes: 2048,
                draw_calls: 0,
                set_pass_calls: 0,
                main_thread_samples: vec![],
                gc_alloc_sites: vec![],
                render_events: vec![],
            },
        ];
        let m = extract(&frames);
        assert_eq!(m.total_alloc_bytes, 3072);
    }
}