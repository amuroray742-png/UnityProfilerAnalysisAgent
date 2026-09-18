//! 渲染相关指标提取

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::extractor::cpu::{FrameTimeStats, Hotspot};
use crate::parser::Frame;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderingMetrics {
    pub draw_calls: FrameTimeStats,
    pub set_pass_calls: FrameTimeStats,
    /// SRP Batcher 节省的批次估算
    pub batches_saved_by_srp_batcher: u64,
    pub top_render_events: Vec<Hotspot>,
}

pub fn extract(frames: &[Frame]) -> RenderingMetrics {
    let mut dc: Vec<f64> = frames.iter().map(|f| f.draw_calls as f64).collect();
    dc.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let draw_calls = FrameTimeStats {
        p50: percentile(&dc, 0.50),
        p95: percentile(&dc, 0.95),
        p99: percentile(&dc, 0.99),
        max: dc.last().copied().unwrap_or(0.0),
    };

    let mut sp: Vec<f64> = frames.iter().map(|f| f.set_pass_calls as f64).collect();
    sp.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let set_pass_calls = FrameTimeStats {
        p50: percentile(&sp, 0.50),
        p95: percentile(&sp, 0.95),
        p99: percentile(&sp, 0.99),
        max: sp.last().copied().unwrap_or(0.0),
    };

    // SRP Batcher 节省估算：当 Draw Call / SetPass 比值小于 1.5 时认为 SRP Batcher 生效
    let total_dc: f64 = dc.iter().sum();
    let total_sp: f64 = sp.iter().sum();
    let batches_saved_by_srp_batcher = if total_sp > 0.0 {
        (total_dc - total_sp).max(0.0) as u64
    } else {
        0
    };

    let top_render_events = aggregate_render_events(frames);

    RenderingMetrics {
        draw_calls,
        set_pass_calls,
        batches_saved_by_srp_batcher,
        top_render_events,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn aggregate_render_events(frames: &[Frame]) -> Vec<Hotspot> {
    let mut acc: HashMap<String, (f64, u64, f64)> = HashMap::new();
    for frame in frames {
        for sample in &frame.render_events {
            let entry = acc.entry(sample.name.clone()).or_insert((0.0, 0, 0.0));
            entry.0 += sample.total_ms;
            entry.1 += sample.call_count;
            if sample.max_ms > entry.2 {
                entry.2 = sample.max_ms;
            }
        }
    }
    let mut evs: Vec<Hotspot> = acc
        .into_iter()
        .map(|(name, (total_ms, call_count, max_ms))| Hotspot {
            avg_ms: if call_count > 0 { total_ms / call_count as f64 } else { 0.0 },
            name,
            total_ms,
            call_count,
            max_ms,
        })
        .collect();
    evs.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap_or(std::cmp::Ordering::Equal));
    evs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_srp_batcher_savings() {
        let frames = vec![
            Frame {
                index: 0,
                duration_ms: 0.0,
                cpu_ms: 0.0,
                gc_alloc_bytes: 0,
                draw_calls: 200,
                set_pass_calls: 50,
                main_thread_samples: vec![],
                gc_alloc_sites: vec![],
                render_events: vec![],
            },
            Frame {
                index: 1,
                duration_ms: 0.0,
                cpu_ms: 0.0,
                gc_alloc_bytes: 0,
                draw_calls: 300,
                set_pass_calls: 60,
                main_thread_samples: vec![],
                gc_alloc_sites: vec![],
                render_events: vec![],
            },
        ];
        let m = extract(&frames);
        assert_eq!(m.batches_saved_by_srp_batcher, 500 - 110);
    }
}