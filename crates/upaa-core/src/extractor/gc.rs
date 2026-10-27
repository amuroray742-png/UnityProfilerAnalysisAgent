use super::{cpu::FrameTimeStats, Quality};
use crate::parser::Frame;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllocHotspot {
    pub name: String,
    pub thread: String,
    pub total_bytes: u64,
    pub call_count: u64,
    pub avg_bytes: f64,
    pub max_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenCollections {
    pub gen0: Option<u64>,
    pub gen1: Option<u64>,
    pub gen2: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GcMetrics {
    pub total_alloc_bytes: Option<u64>,
    pub alloc_per_frame_bytes: FrameTimeStats,
    pub gen_collections: GenCollections,
    pub top_alloc_sites: Vec<AllocHotspot>,
    pub site_quality: Quality,
}
pub fn extract(frames: &[Frame]) -> GcMetrics {
    let valid: Vec<_> = frames.iter().filter(|f| f.quality.gc).collect();
    let mut acc: BTreeMap<(String, String), (u64, u64, u64)> = BTreeMap::new();
    for s in frames
        .iter()
        .filter(|f| f.quality.sites)
        .flat_map(|f| &f.gc_alloc_sites)
    {
        let a = acc.entry((s.thread.clone(), s.name.clone())).or_default();
        a.0 = a.0.saturating_add(s.total_bytes);
        a.1 = a.1.saturating_add(s.call_count);
        a.2 = a.2.max(s.max_bytes);
    }
    let mut sites: Vec<_> = acc
        .into_iter()
        .map(
            |((thread, name), (total_bytes, call_count, max_bytes))| AllocHotspot {
                thread,
                name,
                total_bytes,
                call_count,
                max_bytes,
                avg_bytes: if call_count == 0 {
                    0.0
                } else {
                    total_bytes as f64 / call_count as f64
                },
            },
        )
        .collect();
    sites.sort_by(|a, b| {
        b.total_bytes
            .cmp(&a.total_bytes)
            .then(a.thread.cmp(&b.thread))
            .then(a.name.cmp(&b.name))
    });
    GcMetrics {
        total_alloc_bytes: if valid.is_empty() {
            None
        } else {
            valid
                .iter()
                .try_fold(0u64, |sum, f| sum.checked_add(f.gc_alloc_bytes))
        },
        alloc_per_frame_bytes: FrameTimeStats::new(
            valid.iter().map(|f| f.gc_alloc_bytes as f64).collect(),
            Quality::from_frames(frames, |f| f.quality.gc),
        ),
        gen_collections: GenCollections {
            gen0: None,
            gen1: None,
            gen2: None,
        },
        top_alloc_sites: sites,
        site_quality: Quality::from_frames(frames, |f| f.quality.sites),
    }
}
