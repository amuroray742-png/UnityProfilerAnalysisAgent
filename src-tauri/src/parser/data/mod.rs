//! Unity Profiler `.data` 文件解析器
//!
//! 支持：
//! - Unity 2022.3.x：完整 port `librashuai/UnityPerfAgent/internal/capture/capture.go`
//! - Unity 6000.3.23f1：顺序结构解析，跨帧 marker 状态，CPU / GC 有限样本验证
//!
//! 分块读取输入；结果仍保留于内存，尚无容量上限承诺。

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

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use bytes::Bytes;

use crate::parser::{Frame, ParseError, ParsedProfile, ProfileMeta, ProfilerFormat, Sample};

use self::forest::{accumulate_cpu, build_forest, roll_up_gc_alloc, CpuBreakdown, SampleNode};
use self::frame_header::{read_frame_header, FrameHeader};
use self::header::{is_file_end_marker, read_block_header};
use self::markers::read_markers;
use self::samples::{read_main_thread_samples, DiskSample};
use self::stats::{read_stats, StatsResult};

/// 进度回调：每解析完一帧调用一次，参数是 `(已完成字节数, 文件总字节数)`。
pub type ProgressCallback<'a> = &'a mut dyn FnMut(u64, u64);

/// 解析 `.data` 文件（流式）。
pub fn parse_path(path: &Path) -> Result<ParsedProfile, ParseError> {
    parse_path_with_progress(path, &mut |_, _| {})
}

/// 解析 `.data` 文件（流式，带进度回调）。
pub fn parse_path_with_progress(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<ParsedProfile, ParseError> {
    let file = File::open(path)?;
    let mut details = super::detail::FrameStore::binary_file(file.try_clone()?)?;
    let total_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(1 << 20, file);

    let mut frames: Vec<Frame> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut unity_version: Option<String> = None;
    let mut is_unity6: Option<bool> = None;
    let mut first_block_seen = false;
    let mut last_done: u64 = 0;
    let mut unity6_decoder = unity6_structured::Decoder::default();
    let mut previous_start_ns = None;
    let mut block_index = 0;

    loop {
        // Peek 4 字节判定 EOF marker 或 block header magic
        let mut first4 = [0u8; 4];
        match reader.read_exact(&mut first4) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                if !first_block_seen {
                    return Err(ParseError::Truncated("empty .data file".into()));
                }
                return Err(ParseError::Truncated(
                    "missing 0xDEADFEED end marker".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        }
        last_done += 4;
        if is_file_end_marker(&first4) {
            // 文件结束：再读 1 byte（应 EOF），然后返回
            let mut one = [0u8; 1];
            match reader.read_exact(&mut one) {
                Ok(()) => {
                    warnings.push(format!(
                        "file ends with extra byte 0x{:02x} after 0xDEADFEED",
                        one[0]
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(e) => return Err(e.into()),
            }
            break;
        }

        if first4 != constants::UNITY_DATA_MAGIC.to_le_bytes() {
            return Err(ParseError::Other(format!(
                "expected formatVersion 0x{:08x} or 0x{:08x}, got 0x{:08x}{}",
                constants::UNITY_DATA_MAGIC,
                constants::FILE_END_MARKER,
                u32::from_le_bytes(first4),
                if first_block_seen {
                    " (Unity version mismatch within capture?)"
                } else {
                    ""
                }
            )));
        }

        // 读完剩余 24 字节 header
        let mut rest = [0u8; 24];
        match reader.read_exact(&mut rest) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(ParseError::Truncated("incomplete block header".into()));
            }
            Err(e) => return Err(e.into()),
        }
        last_done += 24;

        let mut header_buf = [0u8; 28];
        header_buf[..4].copy_from_slice(&first4);
        header_buf[4..].copy_from_slice(&rest);
        let block_header = read_block_header(&header_buf)?;
        block_header.validate()?;

        if !first_block_seen {
            unity_version = Some(block_header.unity_version_string());
            is_unity6 = Some(block_header.is_unity_6_or_later());
            first_block_seen = true;
        } else if unity_version.as_deref() != Some(block_header.unity_version_string().as_str()) {
            return Err(ParseError::Other(
                "Unity version changed within capture".into(),
            ));
        }

        // 读取 frame body
        let body_size = block_header.body_size as usize;
        let mut body = vec![0u8; body_size];
        match reader.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(ParseError::Truncated(format!(
                    "truncated frame body at offset {}: expected {} bytes, file ended",
                    last_done, body_size
                )));
            }
            Err(e) => return Err(e.into()),
        }
        last_done += body_size as u64;

        // 校验 frameEndMarker
        if body_size < 4 {
            warnings.push(format!("frame body too small: {}", body_size));
            continue;
        }
        let end_marker = u32::from_le_bytes([
            body[body_size - 4],
            body[body_size - 3],
            body[body_size - 2],
            body[body_size - 1],
        ]);
        if end_marker != constants::FRAME_END_MARKER {
            return Err(ParseError::Other(format!(
                "frame body end marker 0x{:08x} != 0xAFAFAFAF (body_size={})",
                end_marker, body_size
            )));
        }

        let frame_index = block_index;
        block_index += 1;
        if block_header.unity_version_string() == "6000.3.23f1" {
            details.index_binary(
                frame_index,
                last_done - body_size as u64,
                &body,
                unity6_decoder.clone(),
            );
            let decoded = unity6_decoder
                .decode(&body)
                .map_err(|e| ParseError::Other(format!("frame[{frame_index}]: {e}")))?;
            append_structured_frame(&mut frames, &mut previous_start_ns, decoded, frame_index)?;
            progress(last_done, total_size);
            continue;
        }
        let frame_header = read_frame_header_from_body(&body);
        if frame_header.is_synthetic() {
            warnings.push("dropped synthetic frame".into());
            progress(last_done, total_size);
            continue;
        }
        if frame_header.is_zero_duration() {
            // zeroDuration 跳过（librashuai 不变量）
            progress(last_done, total_size);
            continue;
        }

        let unity6 = is_unity6.unwrap_or(false);
        match if unity6 {
            parse_unity6_frame_body(&body, &frame_header, frame_index)
        } else {
            parse_unity2022_frame_body(&body, &frame_header, frame_index)
        } {
            Ok(frame) => frames.push(frame),
            Err(e) => warnings.push(format!("frame {} decode partial: {}", frame_index, e)),
        }

        progress(last_done, total_size);
    }

    let frame_count = frames.len();
    let duration_ms: f64 = frames.iter().map(|f| f.duration_ms).sum();
    let meta = ProfileMeta {
        file_name: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unknown)")
            .to_string(),
        format: ProfilerFormat::Data,
        duration_ms,
        frame_count,
        platform: None,
        unity_version,
        file_size_bytes: total_size,
    };

    details.set_durations(&frames);
    Ok(ParsedProfile {
        details: (!details.is_empty()).then(|| std::sync::Arc::new(details)),
        meta,
        frames,
        warnings,
    })
}

/// 旧 API（bytes-based，用于单元测试和向后兼容）。
pub async fn parse(
    bytes: &Bytes,
    file_name: &str,
    file_size_bytes: u64,
) -> Result<ParsedProfile, ParseError> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut unity_version: Option<String> = None;
    let mut is_unity6: Option<bool> = None;
    let mut first_block_seen = false;
    let mut pos = 0usize;
    let mut details = super::detail::FrameStore::binary_bytes(bytes.clone());
    let mut saw_end = false;
    let mut unity6_decoder = unity6_structured::Decoder::default();
    let mut previous_start_ns = None;
    let mut block_index = 0;

    while pos < bytes.len() {
        if pos + 4 > bytes.len() {
            if !first_block_seen {
                return Err(ParseError::Truncated("empty .data file".into()));
            }
            return Err(ParseError::Truncated(
                "missing 0xDEADFEED end marker".into(),
            ));
        }
        let first4 = &bytes[pos..pos + 4];
        if is_file_end_marker(first4) {
            saw_end = true;
            break;
        }
        if pos + 28 > bytes.len() {
            return Err(ParseError::Truncated("incomplete block header".into()));
        }
        let block_header = read_block_header(&bytes[pos..pos + 28])?;
        block_header.validate()?;
        pos += 28;

        if pos + block_header.body_size as usize > bytes.len() {
            return Err(ParseError::Truncated("incomplete block body".into()));
        }
        let body_end = pos + block_header.body_size as usize;
        let body = &bytes[pos..body_end];
        pos = body_end;

        if !first_block_seen {
            unity_version = Some(block_header.unity_version_string());
            is_unity6 = Some(block_header.is_unity_6_or_later());
            first_block_seen = true;
        } else if unity_version.as_deref() != Some(block_header.unity_version_string().as_str()) {
            return Err(ParseError::Other(
                "Unity version changed within capture".into(),
            ));
        }
        if block_header.body_size < 4 {
            warnings.push(format!("frame body too small: {}", block_header.body_size));
            continue;
        }
        let end_marker = u32::from_le_bytes([
            body[block_header.body_size as usize - 4],
            body[block_header.body_size as usize - 3],
            body[block_header.body_size as usize - 2],
            body[block_header.body_size as usize - 1],
        ]);
        if end_marker != constants::FRAME_END_MARKER {
            return Err(ParseError::Other(format!(
                "frame body end marker 0x{:08x} != 0xAFAFAFAF",
                end_marker
            )));
        }
        let frame_index = block_index;
        block_index += 1;
        if block_header.unity_version_string() == "6000.3.23f1" {
            details.index_binary(
                frame_index,
                (body_end - body.len()) as u64,
                body,
                unity6_decoder.clone(),
            );
            let decoded = unity6_decoder
                .decode(body)
                .map_err(|e| ParseError::Other(format!("frame[{frame_index}]: {e}")))?;
            append_structured_frame(&mut frames, &mut previous_start_ns, decoded, frame_index)?;
            continue;
        }
        let frame_header = read_frame_header_from_body(body);
        if frame_header.is_synthetic() || frame_header.is_zero_duration() {
            continue;
        }
        let unity6 = is_unity6.unwrap_or(false);
        match if unity6 {
            parse_unity6_frame_body(body, &frame_header, frame_index)
        } else {
            parse_unity2022_frame_body(body, &frame_header, frame_index)
        } {
            Ok(frame) => frames.push(frame),
            Err(e) => warnings.push(format!("frame {} decode partial: {}", frame_index, e)),
        }
    }

    if !saw_end {
        return Err(ParseError::Truncated(
            "missing 0xDEADFEED end marker".into(),
        ));
    }
    let frame_count = frames.len();
    let duration_ms: f64 = frames.iter().map(|f| f.duration_ms).sum();
    let meta = ProfileMeta {
        file_name: file_name.to_string(),
        format: ProfilerFormat::Data,
        duration_ms,
        frame_count,
        platform: None,
        unity_version,
        file_size_bytes,
    };
    details.set_durations(&frames);
    Ok(ParsedProfile {
        details: (!details.is_empty()).then(|| std::sync::Arc::new(details)),
        meta,
        frames,
        warnings,
    })
}

fn append_structured_frame(
    frames: &mut Vec<Frame>,
    previous_start_ns: &mut Option<u64>,
    decoded: unity6_structured::DecodedFrame,
    index: usize,
) -> Result<(), ParseError> {
    if let (Some(previous), Some(start)) = (frames.last_mut(), *previous_start_ns) {
        if let Some(interval) = decoded.header.start_ns.checked_sub(start) {
            // Editor converts its integer interval via float nanoseconds to milliseconds.
            previous.duration_ms = (interval as f32 * 1e-6_f32) as f64;
            previous.quality.duration = true;
            previous
                .quality
                .reasons
                .retain(|r| r != "录制帧时间缺少下一帧起始时间戳");
        } else {
            previous
                .quality
                .reasons
                .push("帧起始时间戳倒退，录制帧时间不可用".into());
        }
    }
    *previous_start_ns = Some(decoded.header.start_ns);
    let mut frame = decoded.summary(index);
    super::compact::compact(&mut frame)?;
    frames.push(frame);
    Ok(())
}

fn read_frame_header_from_body(body: &[u8]) -> FrameHeader {
    if body.len() < 28 {
        return FrameHeader {
            frame_id: 0,
            duplicate_id: 0,
            start_ns: 0,
            cpu_us: 0,
            gpu_us: 0,
            gathered_data: 0,
        };
    }
    let mut r = reader::Reader::new(&body[..28]);
    read_frame_header(&mut r)
}

/// Unity 2022.3 完整 body 解码。
fn parse_unity2022_frame_body(
    body: &[u8],
    frame_header: &FrameHeader,
    frame_index: usize,
) -> Result<Frame, ParseError> {
    let body_no_end = &body[..body.len() - 4];
    let mut r = reader::Reader::new(body_no_end);

    let _ = read_frame_header(&mut r); // 已读过
    let _stats: StatsResult = read_stats(&mut r)?;
    if let Some(e) = r.err() {
        return Err(ParseError::Other(format!("stats: {}", e)));
    }

    let mut markers_map = std::collections::HashMap::new();
    read_markers(&mut r, &mut markers_map)?;
    if let Some(e) = r.err() {
        return Err(ParseError::Other(format!("markers: {}", e)));
    }

    let mut disk_samples: Vec<DiskSample> = read_main_thread_samples(&mut r)?;
    if let Some(e) = r.err() {
        return Err(ParseError::Other(format!("samples: {}", e)));
    }

    let metadata_offset = r.pos() as usize;
    gc_alloc::associate(
        body_no_end,
        metadata_offset,
        &mut disk_samples,
        &markers_map,
    );

    let _counters = memory_counters::find(body_no_end);
    let frame_start_ns = frame_header.start_ns;
    let forest = build_forest(&disk_samples, &markers_map, frame_start_ns);

    let mut forest = forest;
    roll_up_gc_alloc(&mut forest);

    let mut cpu = CpuBreakdown::default();
    accumulate_cpu(&forest, &mut cpu);

    let (main_thread_ms, total_gc_alloc_bytes) = if let Some(root) = forest.first() {
        (root.total_ms as f64, root.gc_alloc_kb as u64 * 1024)
    } else {
        (frame_header.cpu_ms(), 0u64)
    };

    let mut render_events: Vec<Sample> = Vec::new();
    let mut main_samples_flat: Vec<Sample> = Vec::new();
    for node in &forest {
        flatten_for_samples(node, &mut main_samples_flat, &mut render_events);
    }

    Ok(Frame {
        quality: super::FrameQuality {
            duration: true,
            cpu: true,
            gc: true,
            samples: true,
            render: true,
            draw: false,
            set_pass: false,
            sites: false,
            source: "unity2022-data".into(),
            ..Default::default()
        },
        index: frame_index,
        duration_ms: main_thread_ms,
        cpu_ms: main_thread_ms,
        gc_alloc_bytes: total_gc_alloc_bytes,
        draw_calls: 0,
        set_pass_calls: 0,
        main_thread_samples: main_samples_flat,
        gc_alloc_sites: aggregate_gc_sites(&forest),
        render_events,
    })
}

/// 未验证 Unity 6 版本的帧头 CPU 估算。旧 GC 扫描与合成热点不参与输出。
fn parse_unity6_frame_body(
    body: &[u8],
    frame_header: &FrameHeader,
    frame_index: usize,
) -> Result<Frame, ParseError> {
    let _ = body;
    let mut quality = super::FrameQuality::missing("unity6-data-experimental");
    quality.cpu = true;
    quality.estimated = true;
    quality.reasons.push(
        "该 Unity 6 版本未通过结构验证；仅展示帧头 CPU 估算，录制帧时间、GC、热点与渲染计数不可用"
            .into(),
    );
    Ok(Frame {
        quality,
        index: frame_index,
        duration_ms: frame_header.cpu_ms(),
        cpu_ms: frame_header.cpu_ms(),
        gc_alloc_bytes: 0,
        draw_calls: 0,
        set_pass_calls: 0,
        main_thread_samples: vec![],
        gc_alloc_sites: vec![],
        render_events: vec![],
    })
}

fn flatten_for_samples(node: &SampleNode, main: &mut Vec<Sample>, render: &mut Vec<Sample>) {
    main.push(Sample {
        name: node.name.clone(),
        total_ms: node.total_ms as f64,
        call_count: 1,
        max_ms: node.total_ms as f64,
    });
    if node.category == "Render" {
        render.push(Sample {
            name: node.name.clone(),
            total_ms: node.total_ms as f64,
            call_count: 1,
            max_ms: node.total_ms as f64,
        });
    }
    for c in &node.children {
        flatten_for_samples(c, main, render);
    }
}

fn aggregate_gc_sites(forest: &[SampleNode]) -> Vec<super::AllocSite> {
    let mut acc: std::collections::HashMap<String, (f64, u64, f64)> =
        std::collections::HashMap::new();
    fn walk(node: &SampleNode, acc: &mut std::collections::HashMap<String, (f64, u64, f64)>) {
        if node.gc_alloc_kb > 0.0 {
            let e = acc.entry(node.name.clone()).or_insert((0.0, 0, 0.0));
            e.0 += node.gc_alloc_kb as f64 * 1024.0;
            e.1 += 1;
            if node.gc_alloc_kb as f64 > e.2 {
                e.2 = node.gc_alloc_kb as f64;
            }
        }
        for c in &node.children {
            walk(c, acc);
        }
    }
    for n in forest {
        walk(n, &mut acc);
    }
    let mut out: Vec<super::AllocSite> = acc
        .into_iter()
        .map(|(name, (total_bytes, calls, max_kb))| super::AllocSite {
            name,
            thread: "Main Thread".into(),
            total_bytes: total_bytes as u64,
            call_count: calls,
            max_bytes: (max_kb * 1024.0) as u64,
        })
        .collect();
    out.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));
    out
}

#[allow(dead_code)]
fn _force_use(s: &StatsResult) {
    let _ = s.audio_used_bytes;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_synth_2022_file(path: &Path) {
        let mut buf = Vec::new();

        // Block header: 28 bytes
        let body_size: u32 = constants::FRAME_HEADER_SIZE as u32 + 4; // 28B frame header + 4B frame end marker
        buf.extend_from_slice(&constants::UNITY_DATA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&body_size.to_le_bytes());
        buf.extend_from_slice(&2022u32.to_le_bytes());
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&47u32.to_le_bytes());
        buf.extend_from_slice(&2u32.to_le_bytes()); // f
        buf.extend_from_slice(&1u32.to_le_bytes());

        // Frame header: 28 bytes
        buf.extend_from_slice(&1i32.to_le_bytes()); // frameID
        buf.extend_from_slice(&1i32.to_le_bytes()); // duplicateID (== frameID for Unity 2022)
        buf.extend_from_slice(&0u64.to_le_bytes()); // startNS
        buf.extend_from_slice(&16_000i32.to_le_bytes()); // cpuUS = 16ms
        buf.extend_from_slice(&0i32.to_le_bytes()); // gpuUS
        buf.extend_from_slice(&1u32.to_le_bytes()); // gatheredData (non-zero)
                                                    // Frame end marker
        buf.extend_from_slice(&constants::FRAME_END_MARKER.to_le_bytes());

        // File end marker
        buf.extend_from_slice(&constants::FILE_END_MARKER.to_le_bytes());

        let mut f = File::create(path).unwrap();
        f.write_all(&buf).unwrap();
    }

    #[test]
    fn parses_synthetic_2022_file_via_path() {
        let dir = std::env::temp_dir().join("upaa_data_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synth_2022.data");
        write_synth_2022_file(&path);

        let profile = parse_path(&path).expect("parse_path");
        assert_eq!(profile.meta.unity_version.as_deref(), Some("2022.3.47f1"));
        // body 只有 24B frame header + 4B end marker，没有 markers/samples，
        // body decode 会失败并落到 warnings，但 frame header 已经被读取过
        assert_eq!(profile.meta.frame_count, 0);
    }

    #[test]
    fn rejects_unity6_header_without_structured_body() {
        // A supported version must not silently skip an undecodable body.
        let path = std::env::temp_dir()
            .join("upaa_data_test")
            .join("unity6_marker_only.data");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut buf = Vec::new();
        // Block header
        let body_size: u32 = constants::FRAME_HEADER_SIZE as u32 + 4;
        buf.extend_from_slice(&constants::UNITY_DATA_MAGIC.to_le_bytes());
        buf.extend_from_slice(&body_size.to_le_bytes());
        buf.extend_from_slice(&6000u32.to_le_bytes());
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&23u32.to_le_bytes());
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        // Frame header (gathered=0 + cpu!=0 → synthetic, 应该跳过)
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&306i32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&36_178i32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // synthetic: cpu!=0 but gathered=0
        buf.extend_from_slice(&constants::FRAME_END_MARKER.to_le_bytes());
        buf.extend_from_slice(&constants::FILE_END_MARKER.to_le_bytes());

        std::fs::write(&path, &buf).unwrap();
        let error = parse_path(&path).unwrap_err().to_string();
        assert!(error.contains("frame[0]"), "{error}");
    }
}
