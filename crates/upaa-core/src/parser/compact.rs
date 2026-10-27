//! Compact aggregation rows only. Original samples remain in FrameStore.
use super::{AllocSite, Frame, ParseError, Sample};
use std::collections::HashMap;

pub fn compact(frame: &mut Frame) -> Result<(), ParseError> {
    let index = frame.index;
    let invalid = || ParseError::Other(format!("frame[{index}]: aggregate overflow"));
    for samples in [&mut frame.main_thread_samples, &mut frame.render_events] {
        let mut cpu: HashMap<String, (f64, u64, f64)> = HashMap::new();
        for sample in std::mem::take(samples) {
            let entry = cpu.entry(sample.name).or_default();
            entry.0 += sample.total_ms;
            if !entry.0.is_finite() {
                return Err(invalid());
            }
            entry.1 = entry
                .1
                .checked_add(sample.call_count)
                .ok_or_else(&invalid)?;
            entry.2 = entry.2.max(sample.max_ms);
        }
        *samples = cpu
            .into_iter()
            .map(|(name, (total_ms, call_count, max_ms))| Sample {
                name,
                total_ms,
                call_count,
                max_ms,
            })
            .collect();
        samples.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let mut gc: HashMap<(String, String), (u64, u64, u64)> = HashMap::new();
    for site in std::mem::take(&mut frame.gc_alloc_sites) {
        let entry = gc.entry((site.thread, site.name)).or_default();
        entry.0 = entry.0.checked_add(site.total_bytes).ok_or_else(&invalid)?;
        entry.1 = entry.1.checked_add(site.call_count).ok_or_else(&invalid)?;
        entry.2 = entry.2.max(site.max_bytes);
    }
    frame.gc_alloc_sites = gc
        .into_iter()
        .map(
            |((thread, name), (total_bytes, call_count, max_bytes))| AllocSite {
                thread,
                name,
                total_bytes,
                call_count,
                max_bytes,
            },
        )
        .collect();
    frame
        .gc_alloc_sites
        .sort_by(|a, b| a.thread.cmp(&b.thread).then(a.name.cmp(&b.name)));
    Ok(())
}
