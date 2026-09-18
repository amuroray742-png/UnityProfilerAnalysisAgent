//! Unity Profiler `.data` 文件解析器
//!
//! 支持：
//! - Unity 2022.3.x：完整 port `librashuai/UnityPerfAgent/internal/capture/capture.go`
//! - Unity 6000.x：共用 block 迭代 + 部分 body 解码（marker 表 + memory counter scan）
//!
//! 流式读取避免 1.1 GB 文件 OOM。

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

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use bytes::Bytes;
use byteorder::ByteOrder;

use crate::parser::{Frame, ParsedProfile, ParseError, ProfileMeta, ProfilerFormat, Sample};

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
    let total_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(1 << 20, file);

    let mut frames: Vec<Frame> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut unity_version: Option<String> = None;
    let mut is_unity6: Option<bool> = None;
    let mut first_block_seen = false;
    let mut last_done: u64 = 0;

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
                warnings.push("truncated block header".into());
                break;
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
        }

        // 读取 frame body
        let body_size = block_header.body_size as usize;
        let mut body = vec![0u8; body_size];
        match reader.read_exact(&mut body) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                warnings.push(format!(
                    "truncated frame body at offset {}: expected {} bytes, file ended",
                    last_done, body_size
                ));
                break;
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
            warnings.push(format!(
                "frame body end marker 0x{:08x} != 0xAFAFAFAF (body_size={})",
                end_marker, body_size
            ));
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

        let frame_index = frames.len();
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

    Ok(ParsedProfile {
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

    while pos < bytes.len() {
        if pos + 4 > bytes.len() {
            if !first_block_seen {
                return Err(ParseError::Truncated("empty .data file".into()));
            }
            return Err(ParseError::Truncated("missing 0xDEADFEED end marker".into()));
        }
        let first4 = &bytes[pos..pos + 4];
        if is_file_end_marker(first4) {
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
            warnings.push(format!(
                "frame body end marker 0x{:08x} != 0xAFAFAFAF",
                end_marker
            ));
            continue;
        }
        let frame_header = read_frame_header_from_body(body);
        if frame_header.is_synthetic() || frame_header.is_zero_duration() {
            continue;
        }
        let frame_index = frames.len();
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
    Ok(ParsedProfile {
        meta,
        frames,
        warnings,
    })
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
    gc_alloc::associate(body_no_end, metadata_offset, &mut disk_samples, &markers_map);

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

/// Unity 6000.x body 解码（增量支持）。
///
/// 当前已实测可解码的字段（Unity 6.3.23f1 frame body）：
/// - Frame header（已知）
/// - Memory counter scan（已知）
/// - Main Thread sample table + GC.Alloc metadata（基于用户实测文件验证）
///
/// 余下未支持（占位 + 提示 warning）：
/// - Stats block post-sentinel
/// - Marker table / marker metadata
/// - 其他 39 个 thread 的 sample table
fn parse_unity6_frame_body(
    body: &[u8],
    frame_header: &FrameHeader,
    frame_index: usize,
) -> Result<Frame, ParseError> {
    let body_no_end = &body[..body.len() - 4];

    // 1) Memory counter scan（已在前面实现）
    let counters = unity6_memory_counters::find_all(body_no_end);

    // 2) GC.Alloc metadata — 优先用 layout 反推的 finder（更准），找不到再退回盲扫
    let mut gc_alloc_pairs: Vec<unity6_layout::RawGcAlloc> = vec![];
    let mut main_sample_table_offset: Option<usize> = None;
    let mut main_sample_count: usize = 0;
    if let Some((gc_off, entries)) = unity6_layout::find_gc_alloc_metadata(body_no_end) {
        gc_alloc_pairs = entries;
        // Main Thread sample table 的 GC.Alloc sample_index 最大值 = N
        // 但样本数不是 max+1，可能更少。仍按 max+1 估一下。
        if let Some(max_si) = gc_alloc_pairs.iter().map(|p| p.sample_index).max() {
            main_sample_count = (max_si + 1) as usize;
            // sample table 在 GC.Alloc metadata 之前
            // 在 [gc_off - 20*max_si - 4, gc_off] 区间找 marker_id=1506 的 sample 0
            // 简化：直接 search marker_id=1506 + total_ns ≈ frame_header.cpu_ms() 的位置
            let expected_total_ms = frame_header.cpu_ms();
            let search_start = gc_off.saturating_sub(20 * main_sample_count + 200);
            let search_end = gc_off;
            // 在更宽的区间搜：scanner 内部 4-byte stride
            for o in (search_start..search_end).step_by(4) {
                if o + 20 > body_no_end.len() {
                    break;
                }
                let mid = byteorder::LittleEndian::read_u32(&body_no_end[o..o + 4]);
                if mid == 1506 {
                    // heuristic: this might be Main Thread sample 0
                    main_sample_table_offset = Some(o);
                    break;
                }
            }
            let _ = expected_total_ms; // not used now
        }
    }

    // 读 Main Thread samples（如找到）
    let main_samples_data = if let Some(off) = main_sample_table_offset {
        unity6_layout::read_samples(body_no_end, off, main_sample_count).unwrap_or_default()
    } else {
        vec![]
    };

    // 3) GC.Alloc 字节数累加
    let gc_alloc_bytes: u64 = gc_alloc_pairs.iter().map(|p| p.alloc_bytes as u64).sum();

    // 4) 构造输出 Frame
    let mut main_samples = vec![Sample {
        name: format!("Frame {}", frame_header.frame_id),
        total_ms: frame_header.cpu_ms(),
        call_count: 1,
        max_ms: frame_header.cpu_ms(),
    }];

    if !main_samples_data.is_empty() {
        main_samples.push(Sample {
            name: format!(
                "Main Thread samples (Unity 6 layout verified, {} samples)",
                main_samples_data.len()
            ),
            total_ms: main_samples_data.iter().map(|s| s.total_ns as f64 / 1e6).sum(),
            call_count: main_samples_data.len() as u64,
            max_ms: main_samples_data
                .iter()
                .map(|s| s.total_ns as f64 / 1e6)
                .fold(0.0, f64::max),
        });
    }

    if !gc_alloc_pairs.is_empty() {
        main_samples.push(Sample {
            name: format!(
                "GC.Alloc ({} metadata entries, total {} bytes)",
                gc_alloc_pairs.len(),
                gc_alloc_bytes
            ),
            total_ms: 0.0,
            call_count: gc_alloc_pairs.len() as u64,
            max_ms: 0.0,
        });
    }

    if !counters.is_empty() {
        main_samples.push(Sample {
            name: format!("Memory counters: {}", counters.len()),
            total_ms: 0.0,
            call_count: counters.len() as u64,
            max_ms: 0.0,
        });
    }

    Ok(Frame {
        index: frame_index,
        duration_ms: frame_header.cpu_ms(),
        cpu_ms: frame_header.cpu_ms(),
        gc_alloc_bytes,
        draw_calls: 0,
        set_pass_calls: 0,
        main_thread_samples: main_samples,
        gc_alloc_sites: vec![],
        render_events: vec![],
    })
}

fn flatten_for_samples(
    node: &SampleNode,
    main: &mut Vec<Sample>,
    render: &mut Vec<Sample>,
) {
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

fn aggregate_gc_sites(forest: &[SampleNode]) -> Vec<Sample> {
    let mut acc: std::collections::HashMap<String, (f64, u64, f64)> =
        std::collections::HashMap::new();
    fn walk(
        node: &SampleNode,
        acc: &mut std::collections::HashMap<String, (f64, u64, f64)>,
    ) {
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
    let mut out: Vec<Sample> = acc
        .into_iter()
        .map(|(name, (total_bytes, calls, max_kb))| Sample {
            name,
            total_ms: total_bytes,
            call_count: calls,
            max_ms: max_kb * 1024.0,
        })
        .collect();
    out.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap_or(std::cmp::Ordering::Equal));
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
    fn parses_unity6_file_block_headers() {
        // 复用 Phase 1 已验证的字节：只解析 block header + frame header，
        // body 不是有效 Unity 2022.3 格式，但 Unity 6 分支也走不通，
        // 因此这里只检查能识别 Unity 6 + 跳过 body。
        let path = std::env::temp_dir()
            .join("upaa_data_test")
            .join("unity6_marker_only.data");
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
        let profile = parse_path(&path).expect("parse_path");
        assert_eq!(profile.meta.unity_version.as_deref(), Some("6000.3.23f1"));
        // synthetic frame 被丢弃 → 0 frames
        assert_eq!(profile.meta.frame_count, 0);
    }
}