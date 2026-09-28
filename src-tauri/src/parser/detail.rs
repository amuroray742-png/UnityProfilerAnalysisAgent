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
    sync::Mutex,
};

const MAX_FRAME_BYTES: usize = 128 << 20;
pub const MAX_NODES: usize = 500;
pub const MAX_DEPTH: usize = 64;
pub const MAX_THREADS: usize = 128;

#[derive(Debug, thiserror::Error)]
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetailThread {
    pub info: ThreadInfo,
    pub samples: Vec<DetailSample>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct DetailFrame {
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
        info.raw_frame_id = Some(decoded.header.frame_id);
        info.raw_duplicate_id = Some(decoded.header.duplicate_id);
        info.start_ns = Some(decoded.header.start_ns.to_string());
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
                    });
                }
                DetailThread { info, samples }
            })
            .collect();
        Self { info, threads }
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
pub struct FrameStore {
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
        })
    }
    pub fn binary_bytes(bytes: Bytes) -> Self {
        Self {
            backing: Backing::Memory(bytes),
            frames: BTreeMap::new(),
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
                before,
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
    pub fn load(&self, index: usize) -> Result<DetailFrame, QueryError> {
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
        match location {
            Location::Spool { .. } => {
                serde_json::from_slice(&bytes).map_err(|e| QueryError::Read(e.to_string()))
            }
            Location::Binary {
                hash,
                before,
                duration,
                ..
            } => {
                if fingerprint(&bytes) != *hash {
                    return Err(QueryError::SourceChanged);
                }
                let decoded = before
                    .clone()
                    .decode(&bytes)
                    .map_err(|e| QueryError::Read(e.to_string()))?;
                Ok(DetailFrame::binary(decoded, index, *duration))
            }
        }
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
            info: frame.info,
            thread_count: count,
            threads: frame
                .threads
                .into_iter()
                .skip(start)
                .take(limit)
                .map(|t| t.info)
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
            .into_iter()
            .find(|t| t.info.thread_index == chosen)
            .ok_or(QueryError::ThreadNotFound(chosen))?;
        if start > thread.samples.len() {
            return Err(QueryError::BadArg("样本起点超出范围".into()));
        }
        let depth_truncated = thread.samples.iter().any(|s| s.depth >= max_depth);
        let mut samples: Vec<_> = thread
            .samples
            .into_iter()
            .filter(|s| s.sample_index >= start && s.depth < max_depth)
            .take(limit + 1)
            .collect();
        let next_start = if samples.len() > limit {
            samples.pop().map(|s| s.sample_index)
        } else {
            None
        };
        Ok(HierarchyPage {
            info: frame.info,
            thread: thread.info,
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
