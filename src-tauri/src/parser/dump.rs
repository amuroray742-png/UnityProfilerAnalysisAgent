//! Typed Unity Editor dump import. Unknown fields are skipped, never retained as a Value tree.
use super::detail::{DetailFrame, DetailSample, DetailThread, FrameInfo, FrameStore, ThreadInfo};
use super::{
    AllocSite, Frame, FrameQuality, ParseError, ParsedProfile, ProfileMeta, ProfilerFormat, Sample,
};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct ProbeFrame {
    #[serde(default, deserialize_with = "present")]
    threads: bool,
}
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(d)?;
    Ok(true)
}
#[derive(Deserialize)]
struct Probe {
    #[serde(default, deserialize_with = "present")]
    input_file: bool,
    #[serde(default, deserialize_with = "present")]
    unity_version: bool,
    frames: Option<Vec<ProbeFrame>>,
}
pub fn is_dump(bytes: &[u8]) -> Result<bool, ParseError> {
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') {
        return Ok(false);
    }
    let p: Probe = serde_json::from_slice(bytes)?;
    Ok(p.input_file
        || p.unity_version
        || p.frames
            .as_ref()
            .is_some_and(|f| f.iter().any(|f| f.threads)))
}
#[derive(Deserialize)]
struct Dump {
    unity_version: String,
    frame_count: usize,
    #[serde(deserialize_with = "read_frames")]
    frames: Vec<DumpFrame>,
}
#[derive(Deserialize)]
struct DumpFrame {
    frame_index: usize,
    frame_time_ms: Option<f64>,
    sample_count_total: usize,
    gc_alloc_bytes_total: Option<u64>,
    #[serde(deserialize_with = "read_threads")]
    threads: Vec<Thread>,
}
#[derive(Deserialize)]
struct Thread {
    thread_index: usize,
    thread_id: u64,
    thread_name: String,
    #[serde(default)]
    thread_group_name: Option<String>,
    gc_alloc_total_bytes: Option<u64>,
    #[serde(deserialize_with = "read_samples")]
    samples: Vec<DumpSample>,
}
#[derive(Deserialize)]
struct DumpSample {
    sample_index: usize,
    marker_id: i32,
    marker_name: String,
    #[serde(default)]
    category_index: Option<u16>,
    time_ms: f64,
    start_time_ms: f64,
    children_count: usize,
    metadata_count: usize,
    gc_alloc_bytes: Option<u64>,
}
fn invalid(context: &str, field: &str) -> ParseError {
    ParseError::Other(format!("{context}: {field} 无效"))
}
pub fn parse(bytes: &[u8], name: &str, size: u64) -> Result<ParsedProfile, ParseError> {
    let dump: Dump = serde_json::from_slice(bytes)?;
    let mut details = FrameStore::spool()?;
    let mut indices = HashSet::new();
    let mut frames = Vec::with_capacity(dump.frames.len());
    let mut warnings = vec![];
    for df in dump.frames {
        let ctx = format!("frame {}", df.frame_index);
        if !indices.insert(df.frame_index) {
            return Err(invalid(&ctx, "重复 frame_index"));
        }
        if df.frame_time_ms.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err(invalid(&ctx, "frame_time_ms"));
        }
        let mut quality = FrameQuality::missing("unity-editor-dump");
        quality.duration = df.frame_time_ms.is_some();
        let main_count = df
            .threads
            .iter()
            .filter(|t| t.thread_name == "Main Thread")
            .count();
        let mut cpu = 0.0;
        let mut main_samples = vec![];
        let mut sites = vec![];
        let mut gc = 0u64;
        let mut gc_valid = true;
        let mut sample_count = 0usize;
        let mut thread_indices = HashSet::new();
        let mut thread_ids = HashSet::new();
        for thread in &df.threads {
            let tc = format!(
                "{ctx}, thread {} ({})",
                thread.thread_index, thread.thread_name
            );
            if !thread_indices.insert(thread.thread_index) || !thread_ids.insert(thread.thread_id) {
                return Err(invalid(&tc, "重复线程标识"));
            }
            // Stack stores direct children still to be consumed, not descendant counts.
            let mut stack: Vec<(usize, usize)> = vec![];
            let mut roots = 0;
            let mut thread_gc = 0u64;
            let mut thread_valid = true;
            for (i, s) in thread.samples.iter().enumerate() {
                let sc = format!("{tc}, sample {i}");
                if s.sample_index != i {
                    return Err(invalid(&sc, "sample_index"));
                }
                if !s.time_ms.is_finite()
                    || s.time_ms < 0.0
                    || !s.start_time_ms.is_finite()
                    || s.start_time_ms < 0.0
                {
                    return Err(invalid(&sc, "time_ms/start_time_ms"));
                }
                while stack.last().is_some_and(|p| p.1 == 0) {
                    stack.pop();
                }
                if stack.is_empty() {
                    roots += 1;
                }
                let parent = stack
                    .iter()
                    .rev()
                    .find(|(j, _)| thread.samples[*j].marker_name != "GC.Alloc")
                    .map(|(j, _)| *j);
                if let Some(p) = stack.last_mut() {
                    p.1 -= 1;
                }
                if s.children_count > thread.samples.len() - i - 1 {
                    return Err(invalid(&sc, "children_count 越界"));
                }
                if s.marker_name == "GC.Alloc" {
                    if s.metadata_count == 0 || s.gc_alloc_bytes.is_none() {
                        thread_valid = false;
                        quality.reasons.push(format!("{sc}: GC metadata 缺失"));
                    } else {
                        let bytes = s.gc_alloc_bytes.unwrap();
                        thread_gc = thread_gc
                            .checked_add(bytes)
                            .ok_or_else(|| invalid(&sc, "GC 总量溢出"))?;
                        sites.push(AllocSite {
                            name: parent
                                .map(|j| thread.samples[j].marker_name.clone())
                                .unwrap_or_else(|| "未归因".into()),
                            thread: format!("{} #{}", thread.thread_name, thread.thread_index),
                            total_bytes: bytes,
                            max_bytes: bytes,
                            call_count: 1,
                        });
                    }
                }
                if s.children_count > 0 {
                    stack.push((i, s.children_count));
                }
                let _ = s.marker_id; // Identity is capture-specific; names carry semantic roles.
            }
            if stack.iter().any(|p| p.1 != 0) {
                return Err(invalid(&tc, "children_count 未闭合"));
            }
            sample_count += thread.samples.len();
            if thread.gc_alloc_total_bytes != Some(thread_gc) {
                thread_valid = false;
                quality.reasons.push(format!("{tc}: GC 总量缺失或不一致"));
            }
            gc_valid &= thread_valid;
            gc = gc
                .checked_add(thread_gc)
                .ok_or_else(|| invalid(&tc, "GC 总量溢出"))?;
            if main_count == 1 && thread.thread_name == "Main Thread" && roots == 1 {
                if let Some(root) = thread.samples.first() {
                    cpu = root.time_ms;
                    quality.cpu = true;
                    quality.samples = true;
                    main_samples = thread
                        .samples
                        .iter()
                        .map(|s| Sample {
                            name: s.marker_name.clone(),
                            total_ms: s.time_ms,
                            max_ms: s.time_ms,
                            call_count: 1,
                        })
                        .collect();
                }
            }
        }
        if sample_count != df.sample_count_total {
            return Err(invalid(&ctx, "sample_count_total 不一致"));
        }
        if df.gc_alloc_bytes_total != Some(gc) || df.threads.is_empty() {
            gc_valid = false;
            quality
                .reasons
                .push(format!("{ctx}: GC 帧总量缺失或不一致"));
        }
        if !quality.cpu {
            quality
                .reasons
                .push(format!("{ctx}: Main Thread 缺失、重复或根样本不唯一"));
        }
        quality.gc = gc_valid;
        quality.sites = gc_valid;
        if !gc_valid {
            sites.clear();
            gc = 0;
        }
        warnings.extend(quality.reasons.clone());
        let mut frame = Frame {
            quality,
            index: df.frame_index,
            duration_ms: df.frame_time_ms.unwrap_or(0.0),
            cpu_ms: cpu,
            gc_alloc_bytes: gc,
            draw_calls: 0,
            set_pass_calls: 0,
            render_counters: Default::default(),
            main_thread_samples: main_samples,
            gc_alloc_sites: sites,
            render_events: vec![],
        };
        super::compact::compact(&mut frame)?;
        let mut threads = Vec::with_capacity(df.threads.len());
        for thread in df.threads {
            let info = ThreadInfo {
                thread_index: thread.thread_index,
                thread_id: thread.thread_id.to_string(),
                name: thread.thread_name,
                group: thread.thread_group_name,
                sample_count: thread.samples.len(),
            };
            let mut stack: Vec<(usize, usize)> = Vec::new();
            let mut samples = Vec::with_capacity(thread.samples.len());
            for s in thread.samples {
                while stack.last().is_some_and(|(_, count)| *count == 0) {
                    stack.pop();
                }
                let parent_index = stack.last().map(|(index, _)| *index);
                let depth = stack.len();
                if let Some((_, left)) = stack.last_mut() {
                    *left -= 1;
                }
                if s.children_count > 0 {
                    stack.push((s.sample_index, s.children_count));
                }
                let gc_alloc_bytes = if gc_valid && s.marker_name == "GC.Alloc" {
                    s.gc_alloc_bytes
                } else {
                    None
                };
                samples.push(DetailSample {
                    sample_index: s.sample_index,
                    parent_index,
                    depth,
                    marker_id: s.marker_id as i64,
                    name: s.marker_name,
                    category_index: s.category_index,
                    total_ms: s.time_ms,
                    start_ms: s.start_time_ms,
                    raw_start_ns: None,
                    raw_duration_ns: None,
                    children_count: s.children_count,
                    metadata_count: s.metadata_count,
                    gc_alloc_bytes,
                    self_ms: None,
                    self_reason: None,
                    is_counter: false,
                    metadata: vec![],
                });
            }
            threads.push(DetailThread { info, samples });
        }
        details.write_frame(&DetailFrame {
            info: FrameInfo::summary(&frame),
            threads,
        })?;
        frames.push(frame);
    }
    if frames.len() != dump.frame_count {
        warnings.push(format!(
            "部分导出：分析 {} 帧，录制声明 {} 帧",
            frames.len(),
            dump.frame_count
        ));
    }
    Ok(ParsedProfile {
        details: Some(std::sync::Arc::new(details)),
        meta: ProfileMeta {
            file_name: name.into(),
            format: ProfilerFormat::Json,
            duration_ms: frames.iter().map(|f| f.duration_ms).sum(),
            frame_count: dump.frame_count,
            platform: None,
            unity_version: Some(dump.unity_version),
            file_size_bytes: size,
        },
        frames,
        warnings,
    })
}

fn read_list<'de, D, T>(d: D, label: &'static str) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct List<T> {
        label: &'static str,
        marker: std::marker::PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for List<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "{} array", self.label)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            loop {
                match seq.next_element::<T>().map_err(|e| {
                    serde::de::Error::custom(format!("{}[{}]: {}", self.label, out.len(), e))
                })? {
                    Some(v) => out.push(v),
                    None => return Ok(out),
                }
            }
        }
    }
    d.deserialize_seq(List {
        label,
        marker: std::marker::PhantomData,
    })
}
fn read_frames<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<DumpFrame>, D::Error> {
    read_list(d, "frames")
}
fn read_threads<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Thread>, D::Error> {
    read_list(d, "threads")
}
fn read_samples<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<DumpSample>, D::Error> {
    read_list(d, "samples")
}
