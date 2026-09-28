use super::{
    cpu::{aggregate, FrameTimeStats, Hotspot},
    Quality,
};
use crate::parser::Frame;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderingMetrics {
    pub draw_calls: FrameTimeStats,
    pub set_pass_calls: FrameTimeStats,
    pub batches: FrameTimeStats,
    pub triangles: FrameTimeStats,
    pub vertices: FrameTimeStats,
    pub batches_saved_by_srp_batcher: Option<u64>,
    pub top_render_events: Vec<Hotspot>,
    pub event_quality: Quality,
}
pub fn extract(frames: &[Frame]) -> RenderingMetrics {
    let counter = |name: &str| {
        FrameTimeStats::new(
            frames
                .iter()
                .filter_map(|f| f.render_counters.get(name).map(|v| *v as f64))
                .collect(),
            Quality::from_frames(frames, |f| f.render_counters.contains_key(name)),
        )
    };
    RenderingMetrics {
        batches: counter("Batches Count"),
        triangles: counter("Triangles Count"),
        vertices: counter("Vertices Count"),
        draw_calls: FrameTimeStats::new(
            frames
                .iter()
                .filter(|f| f.quality.draw)
                .map(|f| f.draw_calls as f64)
                .collect(),
            Quality::from_frames(frames, |f| f.quality.draw),
        ),
        set_pass_calls: FrameTimeStats::new(
            frames
                .iter()
                .filter(|f| f.quality.set_pass)
                .map(|f| f.set_pass_calls as f64)
                .collect(),
            Quality::from_frames(frames, |f| f.quality.set_pass),
        ),
        batches_saved_by_srp_batcher: None,
        top_render_events: aggregate(
            frames
                .iter()
                .filter(|f| f.quality.render)
                .flat_map(|f| &f.render_events),
        ),
        event_quality: Quality::from_frames(frames, |f| f.quality.render),
    }
}
