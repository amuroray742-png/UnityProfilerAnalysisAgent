//! Transient, per-import frame queries. Binary inputs use verified byte ranges
//! and copy-on-write marker checkpoints; dump frames use an anonymous spool.
//! A query materializes at most one frame and returns a bounded page.
use super::{
    data::unity6_structured::{DecodedFrame, Decoder},
    Frame, ParseError,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex, OnceLock, Weak},
};

const MAX_FRAME_BYTES: usize = 128 << 20;
pub const MAX_NODES: usize = 500;
pub const MAX_DEPTH: usize = 64;
pub const MAX_THREADS: usize = 128;

#[derive(Debug, Clone, thiserror::Error)]
pub enum QueryError {
    #[error("该输入没有可用的原始调用树")]
    Unavailable,
    #[error("帧 {0} 不存在")]
    FrameNotFound(usize),
    #[error("线程 {0} 不存在")]
    ThreadNotFound(usize),
    #[error("Main Thread 缺失或不唯一，请明确指定 threadIndex")]
    AmbiguousMainThread,
    #[error("参数错误: {0}")]
    BadArg(String),
    #[error("源文件已变化，请重新导入")]
    SourceChanged,
    #[error("单帧读取超过 128 MiB 查询限制")]
    FrameTooLarge,
    #[error("帧读取失败: {0}")]
    Read(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    #[serde(default)]
    pub version_verified: Option<bool>,
    #[serde(default)]
    pub unknown_section_count: usize,
    pub frame_index: usize,
    pub raw_frame_id: Option<i32>,
    pub raw_duplicate_id: Option<i32>,
    pub start_ns: Option<String>,
    pub source: String,
    pub cpu_ms: Option<f64>,
    pub frame_time_ms: Option<f64>,
    pub gc_alloc_bytes: Option<u64>,
    pub render_counters: BTreeMap<String, u64>,
    pub warnings: Vec<String>,
}
impl FrameInfo {
    pub fn summary(frame: &Frame) -> Self {
        let mut render_counters = frame.render_counters.clone();
        if frame.quality.draw {
            render_counters.insert("Draw Calls Count".into(), frame.draw_calls as u64);
        }
        if frame.quality.set_pass {
            render_counters.insert("SetPass Calls Count".into(), frame.set_pass_calls as u64);
        }
        Self {
            version_verified: frame.quality.version_verified,
            unknown_section_count: 0,
            frame_index: frame.index,
            raw_frame_id: None,
            raw_duplicate_id: None,
            start_ns: None,
            source: frame.quality.source.clone(),
            cpu_ms: frame.quality.cpu.then_some(frame.cpu_ms),
            frame_time_ms: frame.quality.duration.then_some(frame.duration_ms),
            gc_alloc_bytes: frame.quality.gc.then_some(frame.gc_alloc_bytes),
            render_counters,
            warnings: frame.quality.reasons.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub thread_index: usize,
    /// Decimal string: Editor IDs can exceed JavaScript's safe integer range.
    pub thread_id: String,
    pub name: String,
    pub group: Option<String>,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailSample {
    pub sample_index: usize,
    pub parent_index: Option<usize>,
    pub depth: usize,
    pub marker_id: i64,
    pub name: String,
    pub category_index: Option<u16>,
    pub total_ms: f64,
    pub start_ms: f64,
    pub raw_start_ns: Option<String>,
    pub raw_duration_ns: Option<f32>,
    pub children_count: usize,
    pub metadata_count: usize,
    /// None for non-allocation samples or invalid/missing GC metadata.
    pub gc_alloc_bytes: Option<u64>,
    #[serde(default)]
    pub self_ms: Option<f64>,
    #[serde(default)]
    pub self_reason: Option<String>,
    #[serde(default)]
    pub is_counter: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<super::data::unity6_structured::MetadataValue>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetailThread {
    pub info: ThreadInfo,
    pub samples: Vec<DetailSample>,
    #[serde(default)]
    pub flow_events: Option<Vec<super::data::unity6_structured::FlowEvent>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct DetailFrame {
    #[serde(default)]
    pub sections: Vec<super::data::unity6_structured::OpaqueSection>,
    pub info: FrameInfo,
    pub threads: Vec<DetailThread>,
}
impl DetailFrame {
    fn binary(decoded: DecodedFrame, index: usize, duration: Option<f64>) -> Self {
        let mut summary = decoded.summary(index);
        if let Some(ms) = duration {
            summary.duration_ms = ms;
            summary.quality.duration = true;
            summary
                .quality
                .reasons
                .retain(|r| r != "录制帧时间缺少下一帧起始时间戳");
        }
        let mut info = FrameInfo::summary(&summary);
        info.unknown_section_count = decoded.sections.len();
        info.raw_frame_id = Some(decoded.header.frame_id);
        info.raw_duplicate_id = Some(decoded.header.duplicate_id);
        info.start_ns = Some(decoded.header.start_ns.to_string());
        let legacy = decoded.legacy;
        let sections = decoded.sections;
        let threads = decoded
            .threads
            .into_iter()
            .enumerate()
            .map(|(thread_index, t)| {
                let info = ThreadInfo {
                    thread_index,
                    thread_id: t.id.to_string(),
                    name: t.name,
                    group: Some(t.group),
                    sample_count: t.samples.len(),
                };
                let mut samples: Vec<DetailSample> = Vec::with_capacity(t.samples.len());
                for (sample_index, s) in t.samples.into_iter().enumerate() {
                    let depth = s.parent.map(|p| samples[p].depth + 1).unwrap_or(0);
                    let total_ms = s.editor_time_ms();
                    let start_ms = s.editor_start_ms();
                    samples.push(DetailSample {
                        sample_index,
                        parent_index: s.parent,
                        depth,
                        marker_id: s.marker_id as i64,
                        name: s.name,
                        category_index: Some(s.category),
                        total_ms,
                        start_ms,
                        raw_start_ns: Some(s.start_ns.to_string()),
                        raw_duration_ns: Some(s.duration_ns),
                        children_count: s.children as usize,
                        metadata_count: s.metadata_count as usize,
                        gc_alloc_bytes: s.gc_bytes,
                        self_ms: None,
                        self_reason: None,
                        is_counter: s.is_counter,
                        metadata: s.metadata,
                    });
                }
                DetailThread {
                    info,
                    samples,
                    flow_events: if legacy { None } else { Some(t.flow_events) },
                }
            })
            .collect();
        Self {
            info,
            threads,
            sections,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePage {
    pub info: FrameInfo,
    pub thread_count: usize,
    pub threads: Vec<ThreadInfo>,
    pub next_start: Option<usize>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyPage {
    pub info: FrameInfo,
    pub thread: ThreadInfo,
    pub samples: Vec<DetailSample>,
    pub next_start: Option<usize>,
    pub max_depth: usize,
    pub depth_truncated: bool,
}

#[derive(Debug)]
enum Backing {
    File { file: Mutex<File>, size: u64 },
    Memory(Bytes),
}
#[derive(Debug)]
enum Location {
    Binary {
        offset: u64,
        size: usize,
        hash: u64,
        before: Decoder,
        duration: Option<f64>,
    },
    Spool {
        offset: u64,
        size: usize,
    },
}
#[derive(Debug)]
struct FrameCache {
    rows: std::collections::VecDeque<(usize, usize, Arc<DetailFrame>)>,
    bytes: usize,
    decodes: usize,
    budget: usize,
}
impl Default for FrameCache {
    fn default() -> Self {
        Self {
            rows: Default::default(),
            bytes: 0,
            decodes: 0,
            budget: 64 * 1024 * 1024,
        }
    }
}
type InFlight = OnceLock<Result<Arc<DetailFrame>, QueryError>>;

#[derive(Debug)]
pub struct FrameStore {
    inflight: Mutex<BTreeMap<usize, Weak<InFlight>>>,
    cache: Mutex<FrameCache>,
    backing: Backing,
    frames: BTreeMap<usize, Location>,
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
fn temporary_file() -> std::io::Result<File> {
    let path = std::env::temp_dir().join(format!("upaa-frames-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_DELETE_ON_CLOSE | FILE_ATTRIBUTE_TEMPORARY.
        options.custom_flags(0x04000100);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    #[cfg(unix)]
    std::fs::remove_file(path)?;
    Ok(file)
}

impl FrameStore {
    /// Diagnostic counts, not an allocator-accurate byte estimate.
    pub fn storage_counts(&self) -> serde_json::Value {
        let mut seen = std::collections::HashSet::new();
        let mut entries = 0usize;
        let mut names = 0usize;
        for location in self.frames.values() {
            if let Location::Binary { before, .. } = location {
                let (identity, count, bytes) = before.marker_storage();
                if seen.insert(identity) {
                    entries += count;
                    names += bytes;
                }
            }
        }
        serde_json::json!({"indexedFrames":self.frames.len(),"markerTableVersions":seen.len(),"markerEntries":entries,"markerNameBytes":names})
    }
    pub fn binary_file(file: File) -> std::io::Result<Self> {
        let size = file.metadata()?.len();
        Ok(Self {
            backing: Backing::File {
                file: Mutex::new(file),
                size,
            },
            frames: BTreeMap::new(),
            cache: Mutex::new(FrameCache::default()),
            inflight: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn binary_bytes(bytes: Bytes) -> Self {
        Self {
            backing: Backing::Memory(bytes),
            frames: BTreeMap::new(),
            cache: Mutex::new(FrameCache::default()),
            inflight: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn spool() -> std::io::Result<Self> {
        Self::binary_file(temporary_file()?)
    }
    pub fn index_binary(&mut self, index: usize, offset: u64, body: &[u8], before: Decoder) {
        self.frames.insert(
            index,
            Location::Binary {
                offset,
                size: body.len(),
                hash: fingerprint(body),
                before: before.checkpoint(),
                duration: None,
            },
        );
    }
    pub fn set_durations(&mut self, frames: &[Frame]) {
        for frame in frames {
            if let Some(Location::Binary { duration, .. }) = self.frames.get_mut(&frame.index) {
                *duration = frame.quality.duration.then_some(frame.duration_ms);
            }
        }
    }
    pub fn write_frame(&mut self, frame: &DetailFrame) -> Result<(), ParseError> {
        let bytes = serde_json::to_vec(frame)?;
        let Backing::File { file, size } = &mut self.backing else {
            unreachable!()
        };
        let file = file
            .get_mut()
            .map_err(|e| ParseError::Other(e.to_string()))?;
        let offset = file.seek(SeekFrom::End(0))?;
        file.write_all(&bytes)?;
        *size = offset + bytes.len() as u64;
        self.frames.insert(
            frame.info.frame_index,
            Location::Spool {
                offset,
                size: bytes.len(),
            },
        );
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    pub fn load(&self, index: usize) -> Result<Arc<DetailFrame>, QueryError> {
        // Only active callers retain the single-flight result. This also shares
        // oversized frames without keeping them in the bounded LRU cache.
        let flight = {
            let mut active = self
                .inflight
                .lock()
                .map_err(|e| QueryError::Read(e.to_string()))?;
            active.retain(|_, value| value.strong_count() > 0);
            if let Some(value) = active.get(&index).and_then(Weak::upgrade) {
                value
            } else {
                let value = Arc::new(OnceLock::new());
                active.insert(index, Arc::downgrade(&value));
                value
            }
        };
        // Every caller verifies source bytes, even when joining an active decode.
        let location = self
            .frames
            .get(&index)
            .ok_or(QueryError::FrameNotFound(index))?;
        let (offset, count) = match location {
            Location::Binary { offset, size, .. } | Location::Spool { offset, size } => {
                (*offset, *size)
            }
        };
        if count > MAX_FRAME_BYTES {
            return Err(QueryError::FrameTooLarge);
        }
        let bytes = match &self.backing {
            Backing::File { file, size } => {
                let mut file = file.lock().map_err(|e| QueryError::Read(e.to_string()))?;
                if file
                    .metadata()
                    .map_err(|e| QueryError::Read(e.to_string()))?
                    .len()
                    != *size
                {
                    return Err(QueryError::SourceChanged);
                }
                let mut bytes = vec![0; count];
                file.seek(SeekFrom::Start(offset))
                    .and_then(|_| file.read_exact(&mut bytes))
                    .map_err(|e| QueryError::Read(e.to_string()))?;
                bytes
            }
            Backing::Memory(bytes) => bytes
                .get(
                    offset as usize
                        ..(offset as usize)
                            .checked_add(count)
                            .ok_or(QueryError::SourceChanged)?,
                )
                .ok_or(QueryError::SourceChanged)?
                .to_vec(),
        };
        if let Location::Binary { hash, .. } = location {
            if fingerprint(&bytes) != *hash {
                return Err(QueryError::SourceChanged);
            }
        }
        flight
            .get_or_init(|| self.materialize(index, location, &bytes))
            .clone()
    }
    fn materialize(
        &self,
        index: usize,
        location: &Location,
        bytes: &[u8],
    ) -> Result<Arc<DetailFrame>, QueryError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| QueryError::Read(e.to_string()))?;
        if let Some(pos) = cache.rows.iter().position(|r| r.0 == index) {
            let row = cache.rows.remove(pos).unwrap();
            let frame = row.2.clone();
            cache.rows.push_back(row);
            return Ok(frame);
        }
        let mut frame = match location {
            Location::Spool { .. } => serde_json::from_slice::<DetailFrame>(bytes)
                .map_err(|e| QueryError::Read(e.to_string()))?,
            Location::Binary {
                before, duration, ..
            } => DetailFrame::binary(
                before
                    .clone()
                    .decode(bytes)
                    .map_err(|e| QueryError::Read(e.to_string()))?,
                index,
                *duration,
            ),
        };
        for t in &mut frame.threads {
            super::evidence::calculate_self(&mut t.samples);
        }
        let cost = frame_cost(&frame);
        let frame = Arc::new(frame);
        cache.decodes += 1;
        if cost <= cache.budget {
            while cache.rows.len() >= 4 || cache.bytes + cost > cache.budget {
                if let Some((_, n, _)) = cache.rows.pop_front() {
                    cache.bytes -= n;
                } else {
                    break;
                }
            }
            cache.bytes += cost;
            cache.rows.push_back((index, cost, frame.clone()));
        }
        Ok(frame)
    }
    pub fn cache_counts(&self) -> serde_json::Value {
        let c = self.cache.lock().unwrap();
        serde_json::json!({"frames":c.rows.len(),"bytes":c.bytes,"decodes":c.decodes})
    }
    pub fn sections(
        &self,
        index: usize,
        start: usize,
        limit: usize,
    ) -> Result<serde_json::Value, QueryError> {
        validate_limit(limit, 50)?;
        let f = self.load(index)?;
        if start > f.sections.len() {
            return Err(QueryError::BadArg("区段起点超出范围".into()));
        }
        let end = start.saturating_add(limit).min(f.sections.len());
        Ok(
            serde_json::json!({"frameIndex":index,"rows":&f.sections[start..end],"total":f.sections.len(),"nextStart":(end<f.sections.len()).then_some(end),"scope":"offset 相对帧体；unknown 仅验证结构或保留字节，不解释业务含义。帧尾允许 opaque 数据。"}),
        )
    }
    pub fn frame(&self, index: usize, start: usize, limit: usize) -> Result<FramePage, QueryError> {
        validate_limit(limit, MAX_THREADS)?;
        let frame = self.load(index)?;
        if start > frame.threads.len() {
            return Err(QueryError::BadArg("线程起点超出范围".into()));
        }
        let count = frame.threads.len();
        let end = start.saturating_add(limit).min(count);
        Ok(FramePage {
            info: frame.info.clone(),
            thread_count: count,
            threads: frame
                .threads
                .iter()
                .skip(start)
                .take(limit)
                .map(|t| t.info.clone())
                .collect(),
            next_start: (end < count).then_some(end),
        })
    }
    pub fn hierarchy(
        &self,
        index: usize,
        thread_index: Option<usize>,
        start: usize,
        limit: usize,
        max_depth: usize,
    ) -> Result<HierarchyPage, QueryError> {
        validate_limit(limit, MAX_NODES)?;
        validate_limit(max_depth, MAX_DEPTH)?;
        let frame = self.load(index)?;
        let chosen = if let Some(index) = thread_index {
            index
        } else {
            let main: Vec<_> = frame
                .threads
                .iter()
                .filter(|t| t.info.name == "Main Thread")
                .collect();
            if main.len() != 1 {
                return Err(QueryError::AmbiguousMainThread);
            }
            main[0].info.thread_index
        };
        let thread = frame
            .threads
            .iter()
            .find(|t| t.info.thread_index == chosen)
            .ok_or(QueryError::ThreadNotFound(chosen))?;
        if start > thread.samples.len() {
            return Err(QueryError::BadArg("样本起点超出范围".into()));
        }
        let depth_truncated = thread.samples.iter().any(|s| s.depth >= max_depth);
        let mut samples: Vec<_> = thread
            .samples
            .iter()
            .filter(|s| s.sample_index >= start && s.depth < max_depth)
            .take(limit + 1)
            .map(|s| {
                let mut s = s.clone();
                s.metadata.clear();
                s
            })
            .collect();
        let next_start = if samples.len() > limit {
            samples.pop().map(|s| s.sample_index)
        } else {
            None
        };
        Ok(HierarchyPage {
            info: frame.info.clone(),
            thread: thread.info.clone(),
            samples,
            next_start,
            max_depth,
            depth_truncated,
        })
    }
}
fn validate_limit(value: usize, max: usize) -> Result<(), QueryError> {
    if value == 0 || value > max {
        Err(QueryError::BadArg(format!("数量/深度必须在 1..={max}")))
    } else {
        Ok(())
    }
}

// Capacity-based retained heap accounting, including nested metadata buffers.
fn frame_cost(f: &DetailFrame) -> usize {
    use std::mem::size_of;
    fn text(s: &String) -> usize {
        s.capacity()
    }
    let mut n = size_of::<DetailFrame>()
        + f.threads.capacity() * size_of::<DetailThread>()
        + f.sections.capacity() * size_of::<super::data::unity6_structured::OpaqueSection>();
    n += f.info.source.capacity()
        + f.info.start_ns.as_ref().map_or(0, text)
        + f.info.warnings.capacity() * size_of::<String>()
        + f.info.warnings.iter().map(text).sum::<usize>();
    n += f
        .info
        .render_counters
        .iter()
        .map(|(k, _)| k.capacity() + 96)
        .sum::<usize>();
    for s in &f.sections {
        n += text(&s.name) + text(&s.raw_hex) + text(&s.semantic_status);
    }
    for t in &f.threads {
        n += text(&t.info.thread_id)
            + text(&t.info.name)
            + t.info.group.as_ref().map_or(0, text)
            + t.samples.capacity() * size_of::<DetailSample>();
        n += t.flow_events.as_ref().map_or(0, |v| {
            v.capacity() * size_of::<super::data::unity6_structured::FlowEvent>()
        });
        for s in &t.samples {
            n += text(&s.name)
                + s.raw_start_ns.as_ref().map_or(0, text)
                + s.self_reason.as_ref().map_or(0, text)
                + s.metadata.capacity()
                    * size_of::<super::data::unity6_structured::MetadataValue>();
            for m in &s.metadata {
                n += text(&m.status)
                    + text(&m.raw_hex)
                    + m.value.as_ref().map_or(0, text)
                    + m.unit.as_ref().map_or(0, text)
                    + m.reason.as_ref().map_or(0, text)
                    + m.definition.as_ref().map_or(0, |d| text(&d.name));
            }
        }
    }
    n
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    fn frame() -> DetailFrame {
        serde_json::from_value(serde_json::json!({"info":{"frameIndex":0,"rawFrameId":null,"rawDuplicateId":null,"startNs":null,"source":"fixture","cpuMs":null,"frameTimeMs":null,"gcAllocBytes":null,"renderCounters":{},"warnings":[]},"threads":[]})).unwrap()
    }
    #[test]
    fn oversize_bypasses_cache_and_capacities_count() {
        let mut f = frame();
        let initial = frame_cost(&f);
        f.info.source.reserve(4096);
        assert!(frame_cost(&f) >= initial + 4000);
        let mut s = FrameStore::spool().unwrap();
        s.write_frame(&f).unwrap();
        s.cache.lock().unwrap().budget = 1;
        s.load(0).unwrap();
        s.load(0).unwrap();
        assert_eq!(s.cache_counts()["frames"], 0);
        assert_eq!(s.cache_counts()["decodes"], 2);
    }
    #[test]
    fn active_oversize_queries_share_without_retaining_result() {
        let mut store = FrameStore::spool().unwrap();
        store.write_frame(&frame()).unwrap();
        store.cache.lock().unwrap().budget = 1;
        let store = Arc::new(store);
        // Hold the request group open deterministically, including callers that
        // the scheduler starts after the tiny fixture has already decoded.
        let flight = Arc::new(OnceLock::new());
        store
            .inflight
            .lock()
            .unwrap()
            .insert(0, Arc::downgrade(&flight));
        let callers: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.load(0).unwrap())
            })
            .collect();
        let results: Vec<_> = callers.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(results.iter().all(|v| Arc::ptr_eq(v, &results[0])));
        assert_eq!(store.cache_counts()["decodes"], 1);
        assert_eq!(store.cache_counts()["frames"], 0);
        let weak = Arc::downgrade(&results[0]);
        drop(results);
        drop(flight);
        assert!(weak.upgrade().is_none());
        store.load(0).unwrap();
        assert_eq!(store.cache_counts()["decodes"], 2);
    }
    #[test]
    fn budget_eviction_and_release() {
        let f = frame();
        let cost = frame_cost(&f);
        let mut s = FrameStore::spool().unwrap();
        s.write_frame(&f).unwrap();
        let mut f = frame();
        f.info.frame_index = 1;
        s.write_frame(&f).unwrap();
        s.cache.lock().unwrap().budget = cost + 32;
        let f = s.load(0).unwrap();
        let weak = Arc::downgrade(&f);
        drop(f);
        s.load(1).unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(s.cache_counts()["frames"], 1);
        let f = s.load(1).unwrap();
        let weak = Arc::downgrade(&f);
        drop(f);
        drop(s);
        assert!(weak.upgrade().is_none());
    }
}
