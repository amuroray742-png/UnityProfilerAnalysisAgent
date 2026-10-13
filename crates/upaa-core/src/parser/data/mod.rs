//! Sequential bounded import; 2022.3 and 6000.3 only.
pub mod categories;
pub mod constants;
pub mod forest;
pub mod frame_header;
pub mod gc_alloc;
pub mod header;
pub mod markers;
pub mod memory_counters;
pub mod reader;
pub mod samples;
pub mod stats;
pub mod unity6_counters_known;
pub mod unity6_gc_alloc_scan;
pub mod unity6_layout;
pub mod unity6_markers;
pub mod unity6_memory_counters;
pub mod unity6_structured;

pub mod unity2022;
use crate::parser::{Frame, ParseError, ParsedProfile, ProfileMeta, ProfilerFormat};
use bytes::Bytes;
use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub type ProgressCallback<'a> = &'a mut dyn FnMut(u64, u64);
pub fn cancelled(flag: &AtomicBool) -> Result<(), ParseError> {
    if flag.load(Ordering::Relaxed) {
        Err(ParseError::Cancelled)
    } else {
        Ok(())
    }
}
pub fn parse_path(path: &Path) -> Result<ParsedProfile, ParseError> {
    parse_path_with_progress(path, &mut |_, _| {})
}
pub fn parse_path_with_progress(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<ParsedProfile, ParseError> {
    parse_path_cancel(path, progress, Arc::new(AtomicBool::new(false)))
}
pub fn parse_path_cancel(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: Arc<AtomicBool>,
) -> Result<ParsedProfile, ParseError> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let details = super::detail::FrameStore::binary_file(file.try_clone()?)?;
    decode(
        BufReader::with_capacity(1 << 20, file),
        details,
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("(unknown)"),
        size,
        progress,
        cancel,
    )
}
pub async fn parse(bytes: &Bytes, name: &str, size: u64) -> Result<ParsedProfile, ParseError> {
    decode(
        Cursor::new(bytes.as_ref()),
        super::detail::FrameStore::binary_bytes(bytes.clone()),
        name,
        size,
        &mut |_, _| {},
        Arc::new(AtomicBool::new(false)),
    )
}
fn exact(r: &mut impl Read, out: &mut [u8], cancel: &AtomicBool) -> Result<(), ParseError> {
    for chunk in out.chunks_mut(128 * 1024) {
        cancelled(cancel)?;
        r.read_exact(chunk).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                ParseError::Truncated("incomplete data block or missing end marker".into())
            } else {
                e.into()
            }
        })?;
    }
    Ok(())
}
fn decode(
    mut r: impl Read,
    mut details: super::detail::FrameStore,
    name: &str,
    size: u64,
    progress: &mut dyn FnMut(u64, u64),
    cancel: Arc<AtomicBool>,
) -> Result<ParsedProfile, ParseError> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut version = None;
    let mut decoder = unity6_structured::Decoder::default();
    let mut done = 0u64;
    let mut previous: Option<(u64, i32)> = None;
    let mut warnings = Vec::new();
    loop {
        let mut h = [0; 28];
        exact(&mut r, &mut h[..4], &cancel)?;
        done += 4;
        if header::is_file_end_marker(&h[..4]) {
            let mut extra = [0];
            if r.read(&mut extra)? != 0 {
                return Err(ParseError::Other(
                    "extra bytes after file end marker".into(),
                ));
            }
            if frames.is_empty() {
                return Err(ParseError::Truncated("data file has no frames".into()));
            }
            break;
        }
        exact(&mut r, &mut h[4..], &cancel)?;
        done += 24;
        let h = header::read_block_header(&h)?;
        h.validate()?;
        if !matches!((h.unity_major, h.unity_minor), (2022, 3) | (6000, 3)) {
            return Err(ParseError::UnsupportedFormat(format!(
                "Unity {}；仅支持 2022.3 / 6000.3.x",
                h.unity_version_string()
            )));
        }
        let v = h.unity_version_string();
        if let Some(old) = &version {
            if old != &v {
                return Err(ParseError::Other(
                    "Unity version changed within capture".into(),
                ));
            }
        } else {
            decoder = if h.unity_major == 2022 {
                unity2022::adapter(cancel.clone())
            } else {
                unity6_structured::Decoder::configured(
                    false,
                    matches!(v.as_str(), "6000.3.9f1" | "6000.3.23f1"),
                    cancel.clone(),
                )
            };
            if !decoder.verified() {
                warnings.push(format!(
                    "Unity {v} 版本待验证：结构解析不代表真实录制数值已对照，不能作确定性达标结论"
                ));
            }
            version = Some(v);
        }
        let mut body = vec![0; h.body_size as usize];
        exact(&mut r, &mut body, &cancel)?;
        let offset = done;
        done += body.len() as u64;
        let index = frames.len();
        details.index_binary(index, offset, &body, decoder.clone());
        let decoded = decoder.decode(&body).map_err(|e| {
            if cancel.load(Ordering::Relaxed) {
                ParseError::Cancelled
            } else {
                ParseError::Other(format!("frame[{index}]: {e}"))
            }
        })?;
        let current = (decoded.header.start_ns, decoded.header.frame_id);
        let mut frame = decoded.summary(index);
        let skipped = frame.quality.source.ends_with("-skipped");
        if !skipped {
            if let (Some((start, id)), Some(prev)) = (previous, frames.last_mut()) {
                if id.checked_add(1) == Some(current.1) {
                    if let Some(interval) = current.0.checked_sub(start) {
                        prev.duration_ms = (interval as f32 * 1e-6_f32) as f64;
                        prev.quality.duration = true;
                        prev.quality
                            .reasons
                            .retain(|s| s != "录制帧时间缺少下一帧起始时间戳");
                    } else {
                        prev.quality
                            .reasons
                            .push("帧起始时间戳倒退，录制帧时间不可用".into());
                    }
                } else {
                    prev.quality
                        .reasons
                        .push("原始帧 ID 不连续，录制帧时间不可用".into());
                }
            }
            previous = Some(current);
        } else {
            previous = None;
        }
        super::compact::compact(&mut frame)?;
        frames.push(frame);
        progress(done, size);
    }
    cancelled(&cancel)?;
    details.set_durations(&frames);
    progress(done, size);
    Ok(ParsedProfile {
        meta: ProfileMeta {
            file_name: name.into(),
            format: ProfilerFormat::Data,
            duration_ms: frames
                .iter()
                .filter(|f| f.quality.duration)
                .map(|f| f.duration_ms)
                .sum(),
            frame_count: frames.len(),
            platform: None,
            unity_version: version,
            file_size_bytes: size,
        },
        frames,
        warnings,
        details: Some(Arc::new(details)),
    })
}
