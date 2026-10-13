//! Sequential Unity 6000.3 decoding. No offset search or reference dump input.
//! Marker definitions are capture state and must survive empty per-frame tables.
//! Counted unknown sections and opaque frame trailers retain bounded byte evidence.

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
    legacy: bool,
    verification: Option<bool>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    markers: Arc<HashMap<u32, MarkerInfo>>,
    counter_markers: Arc<HashSet<u32>>,
    metadata_definitions: Arc<HashMap<u32, Vec<MetadataDefinition>>>,
}

pub const RENDER_COUNTER_NAMES: [&str; 5] = [
    "Draw Calls Count",
    "SetPass Calls Count",
    "Batches Count",
    "Triangles Count",
    "Vertices Count",
];

impl Decoder {
    pub fn configured(
        legacy: bool,
        verified: bool,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            legacy,
            verification: Some(verified),
            cancel,
            ..Self::default()
        }
    }
    pub fn checkpoint(&self) -> Self {
        let mut copy = self.clone();
        copy.cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        copy
    }
    pub fn verified(&self) -> bool {
        self.verification.unwrap_or(true)
    }
    pub(crate) fn marker_storage(&self) -> (usize, usize, usize) {
        (
            Arc::as_ptr(&self.markers) as usize,
            self.markers.len(),
            self.markers.values().map(|m| m.name.len()).sum(),
        )
    }
}

/// Capture metadata definitions, independent of the host Editor's enums.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataDefinition {
    pub descriptor: u32,
    pub name: String,
    pub name_truncated: bool,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataValue {
    pub field_index: usize,
    pub definition: Option<MetadataDefinition>,
    pub payload_type: u32,
    pub byte_length: usize,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub raw_hex: String,
    pub raw_truncated: bool,
}
fn metadata_value(
    r: &mut Reader<'_>,
    tag: u32,
    size: usize,
    field_index: usize,
    definition: Option<MetadataDefinition>,
    _gc: bool,
) -> MetadataValue {
    let start = r.pos() as usize;
    let bytes = &r.inner.get_ref()[start..start + size];
    // Decimal strings preserve UInt64 precision. GC's compact payload is
    // independently validated against the indexed allocation record.
    let value = match (tag, size) {
        (1, 4) => Some(i32::from_le_bytes(bytes.try_into().unwrap()).to_string()),
        (2, 4) => Some(i32::from_le_bytes(bytes.try_into().unwrap()).to_string()),
        (3, 4) => Some(u32::from_le_bytes(bytes.try_into().unwrap()).to_string()),
        (4, 8) => Some(i64::from_le_bytes(bytes.try_into().unwrap()).to_string()),
        (5, 8) => Some(u64::from_le_bytes(bytes.try_into().unwrap()).to_string()),
        (6, 4) => {
            let v = f32::from_le_bytes(bytes.try_into().unwrap());
            v.is_finite().then(|| v.to_string())
        }
        (7, 8) => {
            let v = f64::from_le_bytes(bytes.try_into().unwrap());
            v.is_finite().then(|| v.to_string())
        }
        (8, _) if size <= 512 => std::str::from_utf8(bytes)
            .ok()
            .filter(|s| {
                !s.chars()
                    .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
            })
            .map(str::to_owned),
        (9, _) if size <= 512 && size % 2 == 0 => String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .ok(),
        _ => None,
    };
    let unit = definition
        .as_ref()
        .and_then(|d| match (d.descriptor >> 8) & 255 {
            1 => Some("nanoseconds"),
            2 => Some("bytes"),
            3 => Some("count"),
            4 => Some("percent"),
            5 => Some("hertz"),
            _ => None,
        })
        .map(str::to_owned);
    let reason = value
        .is_none()
        .then(|| "尚未验证该 payload 类型/尺寸；保留原始字节，不作指标解释".to_owned());
    let raw_hex = bytes.iter().take(64).map(|b| format!("{b:02x}")).collect();
    r.skip(size);
    MetadataValue {
        field_index,
        definition,
        payload_type: tag,
        byte_length: size,
        status: if value.is_some() {
            "available"
        } else {
            "unavailable"
        }
        .into(),
        value,
        unit,
        reason,
        raw_hex,
        raw_truncated: size > 64,
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn values_preserve_precision_sign_units_and_unknown_bytes() {
        let cases = [
            (2, (-7i32).to_le_bytes().to_vec(), Some("-7")),
            (3, u32::MAX.to_le_bytes().to_vec(), Some("4294967295")),
            (4, (-9i64).to_le_bytes().to_vec(), Some("-9")),
            (
                5,
                u64::MAX.to_le_bytes().to_vec(),
                Some("18446744073709551615"),
            ),
            (6, f32::NAN.to_le_bytes().to_vec(), None),
            (6, 1.5f32.to_le_bytes().to_vec(), Some("1.5")),
            (7, 2.5f64.to_le_bytes().to_vec(), Some("2.5")),
            (8, "资源".as_bytes().to_vec(), Some("资源")),
            (8, vec![0, 1, 2, 3], None),
            (88, vec![7; 80], None),
        ];
        for (tag, b, expected) in cases {
            let v = metadata_value(
                &mut Reader::new(&b),
                tag,
                b.len(),
                0,
                Some(MetadataDefinition {
                    descriptor: 0x204,
                    name: "Size".into(),
                    name_truncated: false,
                }),
                false,
            );
            assert_eq!(v.value.as_deref(), expected);
            assert_eq!(v.unit.as_deref(), Some("bytes"));
            assert_eq!(v.raw_hex.len(), b.len().min(64) * 2);
            assert_eq!(v.reason.is_some(), expected.is_none());
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpaqueSection {
    pub name: String,
    pub thread_index: Option<usize>,
    pub offset: usize,
    pub byte_length: usize,
    pub count: Option<usize>,
    pub raw_hex: String,
    pub raw_truncated: bool,
    pub semantic_status: String,
}
fn section(
    r: &Reader<'_>,
    start: usize,
    name: &str,
    thread_index: Option<usize>,
    count: Option<usize>,
) -> OpaqueSection {
    let end = r.pos() as usize;
    let bytes = &r.inner.get_ref()[start..end];
    OpaqueSection {
        name: name.into(),
        thread_index,
        offset: start,
        byte_length: bytes.len(),
        count,
        raw_hex: bytes.iter().take(64).map(|b| format!("{b:02x}")).collect(),
        raw_truncated: bytes.len() > 64,
        semantic_status: "unknown".into(),
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryObservation {
    pub value: Option<String>,
    pub reason: Option<String>,
    pub sources: Vec<MemorySource>,
    pub source_count: usize,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySource {
    pub thread_id: String,
    pub sample_index: usize,
    pub marker_id: u32,
}
pub const MEMORY_COUNTER_NAMES: [&str; 8] = [
    "Total Used Memory",
    "Total Reserved Memory",
    "GC Used Memory",
    "GC Reserved Memory",
    "Gfx Used Memory",
    "Profiler Used Memory",
    "Profiler Reserved Memory",
    "System Used Memory",
];
#[derive(Debug)]
pub struct DecodedFrame {
    pub legacy: bool,
    pub verified: bool,
    pub skipped: bool,
    pub sections: Vec<OpaqueSection>,
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
    pub flow_events: Vec<FlowEvent>,
}

/// Unity 6000.3 thread-tail records, verified against RawFrameDataView.GetFlowEvents.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowEvent {
    pub sample_index: i32,
    pub flow_id: u32,
    pub event_type: u32,
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
    pub metadata: Vec<MetadataValue>,
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
        let mut quality = FrameQuality::missing(if self.legacy {
            "unity2022.3-data-structured"
        } else {
            "unity6000.3-data-structured"
        });
        quality.version_verified = Some(self.verified);
        if !self.verified {
            quality
                .reasons
                .push("版本待验证；禁止确定性达标结论".into());
        }
        if self.skipped {
            quality.source.push_str("-skipped");
            quality
                .reasons
                .push("未采集帧：保留原始块位置，指标不可用".into());
        }
        let mut memory = std::collections::BTreeMap::new();
        for name in MEMORY_COUNTER_NAMES {
            let mut values = Vec::new();
            let mut sources = Vec::new();
            let mut reason = None;
            for t in &self.threads {
                for (sample_index, s) in
                    t.samples.iter().enumerate().filter(|(_, s)| s.name == name)
                {
                    let value = if !s.is_counter {
                        reason = Some("缺少 Counter 标志");
                        None
                    } else if s.metadata_count != 1 || s.metadata.len() != 1 {
                        reason = Some("metadata 缺失或字段数量不符");
                        None
                    } else {
                        let m = &s.metadata[0];
                        if m.unit.as_deref() != Some("bytes") {
                            reason = Some("单位不是 bytes");
                            None
                        } else if !matches!(m.payload_type, 2 | 3 | 4 | 5) {
                            reason = Some("非整数内存 payload");
                            None
                        } else {
                            let v = m.value.as_ref().and_then(|v| v.parse::<u64>().ok());
                            if v.is_none() {
                                reason = Some("负值或无效整数");
                            }
                            v
                        }
                    };
                    values.push(value);
                    if sources.len() < 16 {
                        sources.push(MemorySource {
                            thread_id: t.id.to_string(),
                            sample_index,
                            marker_id: s.marker_id,
                        });
                    }
                }
            }
            let value = values
                .first()
                .copied()
                .flatten()
                .filter(|v| values.iter().all(|x| *x == Some(*v)));
            if values.is_empty() {
                reason = Some("未记录该 Counter");
            } else if value.is_none() && reason.is_none() {
                reason = Some("同帧观测冲突");
            }
            memory.insert(
                name.into(),
                MemoryObservation {
                    value: value.map(|v| v.to_string()),
                    reason: reason.map(str::to_owned),
                    sources,
                    source_count: values.len(),
                },
            );
        }
        quality.gc = !self.skipped;
        quality.sites = !self.skipped;
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
                    "计数 metadata 缺失、未识别或为负值"
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
            memory,
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
        "counted .data body offset {}: {}{}",
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
                "counted .data frame/end marker".into(),
            ));
        }
        let mut r = Reader::new(&body[..body.len() - 4]);
        r.cancel = Some(self.cancel.clone());
        let header = read_frame_header(&mut r);
        if header.cpu_us < 0 || header.gpu_us < 0 {
            return Err(error(&r, "negative frame duration"));
        }
        super::cancelled(&self.cancel)?;
        if self.legacy && header.gathered_data == 0 && header.cpu_us == 0 && body.len() == 32 {
            return Ok(DecodedFrame {
                header,
                threads: vec![],
                thread_section_offset: 28,
                trailer_offset: 28,
                legacy: true,
                verified: false,
                skipped: true,
                sections: vec![],
            });
        }
        read_stats(&mut r)?;
        check(&r, "stats")?;
        let n = count(&mut r, 16, 100_000, "marker definitions")?;
        let mut updated = HashSet::new();
        for _ in 0..n {
            super::cancelled(&self.cancel)?;
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
            let mut definitions = Vec::with_capacity(metadata);
            for _ in 0..metadata {
                let descriptor = r.u32();
                let name = r.str();
                definitions.push(MetadataDefinition {
                    descriptor,
                    name_truncated: name.chars().count() > 128,
                    name: name.chars().take(128).collect(),
                });
            }
            Arc::make_mut(&mut self.metadata_definitions).insert(id, definitions);
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
        let mut metadata_budget = 100_000usize;
        let mut allocation_budget = 256usize * 1024 * 1024;
        let mut sections = Vec::new();
        for index in 0..n {
            let thread = self
                .thread(
                    &mut r,
                    &mut metadata_budget,
                    index,
                    &mut sections,
                    &mut allocation_budget,
                )
                .map_err(|e| ParseError::Other(format!("thread[{index}]: {e}")))?;
            if !ids.insert(thread.id) {
                return Err(error(&r, "duplicate thread ID"));
            }
            threads.push(thread);
        }
        let trailer_offset = r.pos() as usize;
        if r.remaining() > 0 {
            let n = r.remaining();
            r.skip(n);
            sections.push(section(&r, trailer_offset, "frame trailer", None, None));
        }
        Ok(DecodedFrame {
            legacy: self.legacy,
            verified: self.verified(),
            skipped: false,
            sections,
            header,
            threads,
            thread_section_offset,
            trailer_offset,
        })
    }

    fn thread(
        &self,
        r: &mut Reader<'_>,
        metadata_budget: &mut usize,
        thread_index: usize,
        sections: &mut Vec<OpaqueSection>,
        allocation_budget: &mut usize,
    ) -> Result<Thread, ParseError> {
        let id = r.u64();
        let group = r.str();
        let name = r.str();
        let n = count(r, 20, 1_000_000, "samples")?;
        let cost = n
            .checked_mul(std::mem::size_of::<Sample>() + 32)
            .ok_or_else(|| error(r, "sample allocation overflow"))?;
        *allocation_budget = allocation_budget
            .checked_sub(cost)
            .ok_or_else(|| error(r, "decoded frame exceeds 256 MiB allocation budget"))?;
        let mut samples = Vec::with_capacity(n);
        let mut stack: Vec<(usize, u32)> = Vec::new();
        for index in 0..n {
            if index % 1024 == 0 {
                super::cancelled(&self.cancel)?;
            }
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
                *allocation_budget = allocation_budget
                    .checked_sub(marker.name.len())
                    .ok_or_else(|| error(r, "sample names exceed decoded frame budget"))?;
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
                metadata: Vec::new(),
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
        let section_start = r.pos() as usize;
        let auxiliary_count = count(r, 12, 1_000_000, "sample auxiliary records")?;
        for _ in 0..auxiliary_count {
            r.u32();
            let index = r.u32() as usize;
            r.u32();
            if index >= n {
                return Err(error(r, "auxiliary sample index"));
            }
        }
        if auxiliary_count > 0 {
            sections.push(section(
                r,
                section_start,
                "sample auxiliary records",
                Some(thread_index),
                Some(auxiliary_count),
            ));
        }
        let section_start = r.pos() as usize;
        let indexed_count = count(r, 8, n, "indexed records")?;
        let mut indexed = HashSet::new();
        for _ in 0..indexed_count {
            let index = r.u32() as usize;
            r.u32();
            if index >= n || !indexed.insert(index) {
                return Err(error(r, "indexed record sample index"));
            }
        }
        if indexed_count > 0 {
            sections.push(section(
                r,
                section_start,
                "indexed records",
                Some(thread_index),
                Some(indexed_count),
            ));
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
        let section_start = r.pos() as usize;
        let post_gc_count = count(r, 4, n, "post-GC sample indices")?;
        let mut post_gc_sample_indices = Vec::with_capacity(post_gc_count);
        for _ in 0..post_gc_count {
            let index = r.u32();
            if index as usize >= n {
                return Err(error(r, "post-GC sample index boundary"));
            }
            post_gc_sample_indices.push(index);
        }
        if post_gc_count > 0 {
            sections.push(section(
                r,
                section_start,
                "post-GC sample indices",
                Some(thread_index),
                Some(post_gc_count),
            ));
        }
        let metadata_count = count(r, 8, 1_000_000, "general metadata")?;
        let mut seen = HashSet::new();
        for _ in 0..metadata_count {
            super::cancelled(&self.cancel)?;
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
            for field_index in 0..fields {
                let tag = r.u32();
                let size = count(r, 1, r.remaining(), "metadata payload")?;
                let payload_start = r.pos();
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
                            (2, 4) => u64::try_from(r.i32()).ok(),
                            (4, 8) => u64::try_from(r.i64()).ok(),
                            _ => return Err(error(r, "unsupported render counter metadata type")),
                        };
                    } else {
                        r.skip(size);
                    }
                }
                if field_index < 16 && *metadata_budget > 0 {
                    *allocation_budget = allocation_budget
                        .checked_sub(1536)
                        .ok_or_else(|| error(r, "metadata exceeds decoded frame budget"))?;
                    *metadata_budget -= 1;
                    r.inner.set_position(payload_start);
                    let definition = self
                        .metadata_definitions
                        .get(&sample.marker_id)
                        .and_then(|d| d.get(field_index))
                        .cloned();
                    sample.metadata.push(metadata_value(
                        r,
                        tag,
                        size,
                        field_index,
                        definition,
                        sample.gc_bytes.is_some(),
                    ));
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
        let section_start = r.pos() as usize;
        let index_count = count(r, 4, n, "sample index list")?;
        for _ in 0..index_count {
            if r.u32() as usize >= n {
                return Err(error(r, "sample index list boundary"));
            }
        }
        if index_count > 0 {
            sections.push(section(
                r,
                section_start,
                "sample index list",
                Some(thread_index),
                Some(index_count),
            ));
        }
        let scalar_start = r.pos() as usize;
        r.u32();
        r.u32();
        check(r, "thread scalars")?;
        sections.push(section(
            r,
            scalar_start,
            "thread scalars",
            Some(thread_index),
            Some(2),
        ));
        let trailing_start = r.pos() as usize;
        let trailing_count = count(r, 12, 1_000_000, "thread trailing records")?;
        *allocation_budget = allocation_budget
            .checked_sub(trailing_count * std::mem::size_of::<FlowEvent>())
            .ok_or_else(|| error(r, "Flow exceeds decoded frame budget"))?;
        let mut flow_events = Vec::with_capacity(trailing_count);
        for _ in 0..trailing_count {
            let sample_index = r.i32();
            let flow_id = r.u32();
            let event_type = r.u32();
            if sample_index < -1 || sample_index >= n as i32 {
                return Err(error(r, "Flow sample index boundary"));
            }
            flow_events.push(FlowEvent {
                sample_index,
                flow_id,
                event_type,
            });
        }
        check(r, "thread end")?;
        if self.legacy && trailing_count > 0 {
            sections.push(section(
                r,
                trailing_start,
                "legacy thread trailing records",
                Some(thread_index),
                Some(trailing_count),
            ));
        }
        let gc_bytes = samples.iter().filter_map(|s| s.gc_bytes).sum();
        Ok(Thread {
            id,
            group,
            name,
            samples,
            gc_bytes,
            post_gc_sample_indices,
            flow_events,
        })
    }
}
