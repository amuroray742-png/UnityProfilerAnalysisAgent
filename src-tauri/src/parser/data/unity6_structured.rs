//! Sequential Unity 6000.3 decoding. No offset search or reference dump input.
//! Marker definitions are capture state and must survive empty per-frame tables.
//! Unknown post-sample sections fail explicitly. The frame trailer is not decoded.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{
    frame_header::{read_frame_header, FrameHeader},
    markers::MarkerInfo,
    reader::Reader,
    stats::read_stats,
};
use crate::parser::{AllocSite, Frame, FrameQuality, ParseError, Sample as SummarySample};

#[derive(Debug, Default, Clone)]
pub struct Decoder {
    markers: Arc<HashMap<u32, MarkerInfo>>,
    counter_markers: Arc<HashSet<u32>>,
}

pub const RENDER_COUNTER_NAMES: [&str; 5] = [
    "Draw Calls Count",
    "SetPass Calls Count",
    "Batches Count",
    "Triangles Count",
    "Vertices Count",
];

impl Decoder {
    pub(crate) fn marker_storage(&self) -> (usize, usize, usize) {
        (
            Arc::as_ptr(&self.markers) as usize,
            self.markers.len(),
            self.markers.values().map(|m| m.name.len()).sum(),
        )
    }
}

#[derive(Debug)]
pub struct DecodedFrame {
    pub header: FrameHeader,
    pub threads: Vec<Thread>,
    pub thread_section_offset: usize,
    pub trailer_offset: usize,
}

#[derive(Debug)]
pub struct Thread {
    pub id: u64,
    pub group: String,
    pub name: String,
    pub samples: Vec<Sample>,
    pub gc_bytes: u64,
    /// Counted sample indices between GC and general metadata; meaning unknown.
    pub post_gc_sample_indices: Vec<u32>,
}

#[derive(Debug)]
pub struct Sample {
    pub marker_id: u32,
    pub name: String,
    pub category: u16,
    pub duration_ns: f32,
    pub start_ns: u64,
    pub children: u32,
    pub parent: Option<usize>,
    pub metadata_count: u32,
    pub gc_bytes: Option<u64>,
    pub counter_value: Option<u64>,
    pub is_counter: bool,
}

impl Sample {
    pub fn editor_time_ms(&self) -> f64 {
        (self.duration_ns * 1e-6_f32) as f64
    }
    pub fn editor_start_ms(&self) -> f64 {
        (self.start_ns as f64 / 1e6) as f32 as f64
    }
}

impl DecodedFrame {
    /// Adapt verified samples to the current summary contract. Keep the raw
    /// header/threads in DecodedFrame for structural queries and verification.
    pub fn summary(&self, index: usize) -> Frame {
        let mut quality = FrameQuality::missing("unity6000.3-data-structured");
        quality.gc = true;
        quality.sites = true;
        let mut render_counters = std::collections::BTreeMap::new();
        for counter in RENDER_COUNTER_NAMES {
            let values: Vec<_> = self
                .threads
                .iter()
                .flat_map(|t| &t.samples)
                .filter(|s| s.name == counter)
                .map(|s| s.counter_value)
                .collect();
            if let Some(value) = values
                .first()
                .copied()
                .flatten()
                .filter(|v| values.iter().all(|n| *n == Some(*v)))
            {
                render_counters.insert(counter.to_owned(), value);
            } else {
                let reason = if values.is_empty() {
                    "未记录该计数"
                } else if values.iter().any(Option::is_none) {
                    "计数 metadata 缺失或未识别"
                } else {
                    "同帧多个观测值冲突"
                };
                quality.reasons.push(format!("{counter}: {reason}"));
            }
        }
        let draw = render_counters
            .get("Draw Calls Count")
            .and_then(|v| u32::try_from(*v).ok());
        let set_pass = render_counters
            .get("SetPass Calls Count")
            .and_then(|v| u32::try_from(*v).ok());
        quality.draw = draw.is_some();
        quality.set_pass = set_pass.is_some();
        for name in ["Draw Calls Count", "SetPass Calls Count"] {
            if render_counters
                .get(name)
                .is_some_and(|v| *v > u32::MAX as u64)
            {
                quality
                    .reasons
                    .push(format!("{name}: 超出当前聚合字段范围"));
            }
        }
        quality.render = true;
        let render_events = self
            .threads
            .iter()
            .enumerate()
            .flat_map(|(i, t)| {
                t.samples
                    .iter()
                    .filter(|s| s.category == 0 && !s.is_counter && s.duration_ns > 0.0)
                    .map(move |s| SummarySample {
                        name: format!("{} #{} / {}", t.name, i, s.name),
                        total_ms: s.editor_time_ms(),
                        max_ms: s.editor_time_ms(),
                        call_count: 1,
                    })
            })
            .collect();
        quality
            .reasons
            .push("录制帧时间缺少下一帧起始时间戳".into());
        let main: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.name == "Main Thread")
            .collect();
        let mut cpu_ms = 0.0;
        let mut main_thread_samples = Vec::new();
        if main.len() == 1
            && main[0]
                .samples
                .iter()
                .filter(|s| s.parent.is_none())
                .count()
                == 1
        {
            cpu_ms = main[0].samples[0].editor_time_ms();
            quality.cpu = true;
            quality.samples = true;
            main_thread_samples = main[0]
                .samples
                .iter()
                .map(|s| SummarySample {
                    name: s.name.clone(),
                    total_ms: s.editor_time_ms(),
                    max_ms: s.editor_time_ms(),
                    call_count: 1,
                })
                .collect();
        } else {
            quality
                .reasons
                .push("Main Thread 缺失、重复或根样本不唯一".into());
        }
        let mut gc_alloc_sites = Vec::new();
        for (thread_index, thread) in self.threads.iter().enumerate() {
            for sample in &thread.samples {
                if let Some(bytes) = sample.gc_bytes {
                    let mut parent = sample.parent;
                    while let Some(i) = parent {
                        if thread.samples[i].name != "GC.Alloc" {
                            break;
                        }
                        parent = thread.samples[i].parent;
                    }
                    gc_alloc_sites.push(AllocSite {
                        name: parent
                            .map(|i| thread.samples[i].name.clone())
                            .unwrap_or_else(|| "未归因".into()),
                        thread: format!("{} #{}", thread.name, thread_index),
                        total_bytes: bytes,
                        max_bytes: bytes,
                        call_count: 1,
                    });
                }
            }
        }
        Frame {
            quality,
            index,
            duration_ms: 0.0,
            cpu_ms,
            gc_alloc_bytes: self.threads.iter().map(|t| t.gc_bytes).sum(),
            draw_calls: draw.unwrap_or(0),
            set_pass_calls: set_pass.unwrap_or(0),
            render_counters,
            main_thread_samples,
            gc_alloc_sites,
            render_events,
        }
    }
}

fn error(r: &Reader<'_>, field: &str) -> ParseError {
    ParseError::Other(format!(
        "Unity 6000.3 body offset {}: {}{}",
        r.pos(),
        field,
        r.err().map(|e| format!(": {e}")).unwrap_or_default()
    ))
}

fn check(r: &Reader<'_>, field: &str) -> Result<(), ParseError> {
    if r.err().is_some() {
        Err(error(r, field))
    } else {
        Ok(())
    }
}

fn count(
    r: &mut Reader<'_>,
    stride: usize,
    limit: usize,
    field: &str,
) -> Result<usize, ParseError> {
    let n = r.u32() as usize;
    check(r, field)?;
    if n > limit || n > r.remaining() / stride.max(1) {
        return Err(error(r, &format!("{field} count {n} exceeds bounds")));
    }
    Ok(n)
}

impl Decoder {
    pub fn decode(&mut self, body: &[u8]) -> Result<DecodedFrame, ParseError> {
        if body.len() < 32 || body[body.len() - 4..] != 0xAFAFAFAFu32.to_le_bytes() {
            return Err(ParseError::Truncated(
                "Unity 6000.3 frame/end marker".into(),
            ));
        }
        let mut r = Reader::new(&body[..body.len() - 4]);
        let header = read_frame_header(&mut r);
        if header.cpu_us < 0 || header.gpu_us < 0 {
            return Err(error(&r, "negative frame duration"));
        }
        read_stats(&mut r)?;
        check(&r, "stats")?;
        let n = count(&mut r, 16, 100_000, "marker definitions")?;
        let mut updated = HashSet::new();
        for _ in 0..n {
            let id = r.u32();
            let name = r.str();
            let flags = r.u32();
            if flags & 0x80 != 0 {
                Arc::make_mut(&mut self.counter_markers).insert(id);
            } else {
                Arc::make_mut(&mut self.counter_markers).remove(&id);
            }
            let metadata = count(&mut r, 8, 100_000, "marker metadata")?;
            if !updated.insert(id) {
                return Err(error(&r, "duplicate marker definition"));
            }
            for _ in 0..metadata {
                r.u32();
                r.str();
            }
            check(&r, "marker definition")?;
            Arc::make_mut(&mut self.markers).insert(
                id,
                MarkerInfo {
                    name,
                    category_id: (flags >> 16) as u16,
                },
            );
            if self.markers.len() > 100_000 {
                return Err(error(&r, "capture marker limit exceeded"));
            }
        }
        let thread_section_offset = r.pos() as usize;
        let n = count(&mut r, 24, 512, "threads")?;
        if n == 0 {
            return Err(error(&r, "no exported threads"));
        }
        let mut threads = Vec::with_capacity(n);
        let mut ids = HashSet::new();
        for index in 0..n {
            let thread = self
                .thread(&mut r)
                .map_err(|e| ParseError::Other(format!("thread[{index}]: {e}")))?;
            if !ids.insert(thread.id) {
                return Err(error(&r, "duplicate thread ID"));
            }
            threads.push(thread);
        }
        Ok(DecodedFrame {
            header,
            threads,
            thread_section_offset,
            trailer_offset: r.pos() as usize,
        })
    }

    fn thread(&self, r: &mut Reader<'_>) -> Result<Thread, ParseError> {
        let id = r.u64();
        let group = r.str();
        let name = r.str();
        let n = count(r, 20, 1_000_000, "samples")?;
        let mut samples = Vec::with_capacity(n);
        let mut stack: Vec<(usize, u32)> = Vec::new();
        for index in 0..n {
            while stack.last().is_some_and(|(_, left)| *left == 0) {
                stack.pop();
            }
            let parent = stack.last().map(|(i, _)| *i);
            if let Some((_, left)) = stack.last_mut() {
                *left -= 1;
            }
            let marker_id = r.u32();
            let duration_ns = r.f32();
            let start_ns = r.u64();
            let children = r.u32();
            if !duration_ns.is_finite() || duration_ns < 0.0 || children as usize > n - index - 1 {
                return Err(error(
                    r,
                    &format!("sample[{index}] invalid duration/children"),
                ));
            }
            let (name, category) = if marker_id == u32::MAX && parent.is_none() {
                (String::new(), 16)
            } else {
                let marker = self.markers.get(&marker_id).ok_or_else(|| {
                    error(r, &format!("sample[{index}] undefined marker {marker_id}"))
                })?;
                (marker.name.clone(), marker.category_id)
            };
            samples.push(Sample {
                marker_id,
                name,
                category,
                duration_ns,
                start_ns,
                children,
                parent,
                metadata_count: 0,
                gc_bytes: None,
                counter_value: None,
                is_counter: self.counter_markers.contains(&marker_id),
            });
            if children > 0 {
                stack.push((index, children));
            }
        }
        if stack.iter().any(|(_, left)| *left != 0) {
            return Err(error(r, "sample tree not closed"));
        }
        // Counted 12-byte auxiliary records precede indexed/GC metadata in
        // 6000.3.9f1. Semantics are not exposed as performance metrics.
        let auxiliary_count = count(r, 12, 1_000_000, "sample auxiliary records")?;
        for _ in 0..auxiliary_count {
            r.u32();
            let index = r.u32() as usize;
            r.u32();
            if index >= n {
                return Err(error(r, "auxiliary sample index"));
            }
        }
        let indexed_count = count(r, 8, n, "indexed records")?;
        let mut indexed = HashSet::new();
        for _ in 0..indexed_count {
            let index = r.u32() as usize;
            r.u32();
            if index >= n || !indexed.insert(index) {
                return Err(error(r, "indexed record sample index"));
            }
        }
        let gc_count = count(r, 8, n, "GC records")?;
        for _ in 0..gc_count {
            let index = r.u32() as usize;
            let bytes = r.u32() as u64;
            let sample = samples
                .get_mut(index)
                .ok_or_else(|| error(r, "GC sample index"))?;
            if sample.name != "GC.Alloc" || sample.gc_bytes.replace(bytes).is_some() {
                return Err(error(r, "GC record marker mismatch or duplicate"));
            }
        }
        if samples
            .iter()
            .any(|s| s.name == "GC.Alloc" && s.gc_bytes.is_none())
        {
            return Err(error(r, "GC record missing"));
        }
        let post_gc_count = count(r, 4, n, "post-GC sample indices")?;
        let mut post_gc_sample_indices = Vec::with_capacity(post_gc_count);
        for _ in 0..post_gc_count {
            let index = r.u32();
            if index as usize >= n {
                return Err(error(r, "post-GC sample index boundary"));
            }
            post_gc_sample_indices.push(index);
        }
        let metadata_count = count(r, 8, 1_000_000, "general metadata")?;
        let mut seen = HashSet::new();
        for _ in 0..metadata_count {
            let index = r.u32() as usize;
            let fields = count(r, 8, 100_000, "metadata fields")?;
            let sample = samples
                .get_mut(index)
                .ok_or_else(|| error(r, "metadata sample index"))?;
            if fields == 0 {
                if index != 0 {
                    return Err(error(r, "unknown empty metadata record"));
                }
                continue;
            }
            if !seen.insert(index) {
                return Err(error(r, "duplicate general metadata"));
            }
            sample.metadata_count = fields as u32;
            for _ in 0..fields {
                let tag = r.u32();
                let size = count(r, 1, r.remaining(), "metadata payload")?;
                if let Some(gc_bytes) = sample.gc_bytes {
                    if fields != 1 || tag != 3 || size != 4 {
                        return Err(error(r, "unsupported GC metadata payload"));
                    }
                    if r.u32() as u64 != gc_bytes {
                        return Err(error(r, "GC metadata payload differs"));
                    }
                } else {
                    if RENDER_COUNTER_NAMES.contains(&sample.name.as_str())
                        && self.counter_markers.contains(&sample.marker_id)
                    {
                        if fields != 1 || sample.counter_value.is_some() {
                            return Err(error(r, "render counter metadata duplicate/arity"));
                        }
                        sample.counter_value = match (tag, size) {
                            (2, 4) => Some(r.u32() as u64),
                            (4, 8) => Some(r.u64()),
                            _ => return Err(error(r, "unsupported render counter metadata type")),
                        };
                    } else {
                        r.skip(size);
                    }
                }
                r.skip((4 - size % 4) % 4);
                check(r, "metadata payload")?;
            }
        }
        if samples
            .iter()
            .any(|s| s.gc_bytes.is_some() && s.metadata_count != 1)
        {
            return Err(error(r, "GC general metadata missing"));
        }
        let index_count = count(r, 4, n, "sample index list")?;
        for _ in 0..index_count {
            if r.u32() as usize >= n {
                return Err(error(r, "sample index list boundary"));
            }
        }
        r.u32();
        r.u32();
        let trailing_count = count(r, 12, 1_000_000, "thread trailing records")?;
        r.skip(trailing_count * 12);
        check(r, "thread end")?;
        let gc_bytes = samples.iter().filter_map(|s| s.gc_bytes).sum();
        Ok(Thread {
            id,
            group,
            name,
            samples,
            gc_bytes,
            post_gc_sample_indices,
        })
    }
}
