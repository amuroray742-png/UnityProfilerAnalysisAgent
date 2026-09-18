//! PD3U (PlayerConnection Data Unity) 格式解析器
//!
//! Unity Editor 通过 PlayerConnection 接收的实时 Profiler 数据流协议。
//! 协议基于 protobuf，schema 部分公开。
//!
//! MVP 实现：识别文件头 + 解析基本 frame 数估算。
//! 完整 PD3U 解析需要 Unity 内部 protobuf 定义，参考实现
//! `librashuai/UnityPerfAgent` 的 PD3U Decoder（Go 实现）。

use bytes::{Buf, Bytes};

use super::{Frame, ParsedProfile, ParseError, ProfileMeta, Sample};

/// PD3U 文件常见开头（前 4 字节为标识）
const PD3U_HEADER_MARKERS: &[&[u8]] = &[
    b"PD3U",
    b"UPPC",
    b"\xAB\xCD\xEF\x00", // 试探性 magic
];

pub async fn parse(
    bytes: &Bytes,
    file_name: &str,
    file_size_bytes: u64,
) -> Result<ParsedProfile, ParseError> {
    let mut warnings = Vec::new();
    let mut buf = bytes.clone();

    // 检测 magic
    let mut matched = None;
    for marker in PD3U_HEADER_MARKERS {
        if buf.len() >= marker.len() && &buf[..marker.len()] == *marker {
            matched = Some(marker.len());
            break;
        }
    }

    if matched.is_none() {
        warnings.push("PD3U 文件头识别失败，按帧流估算".to_string());
    } else {
        let skip = matched.unwrap();
        buf.advance(skip);
        warnings.push(format!("识别到 PD3U 头部（{} bytes），按帧流估算", skip));
    }

    // 估算 frame 数（启发式：按 8KB 平均 frame 流）
    let estimated_frames = (file_size_bytes / 8192).max(1) as usize;

    Ok(ParsedProfile {
        meta: ProfileMeta {
            file_name: file_name.to_string(),
            format: super::ProfilerFormat::Pd3u,
            duration_ms: 0.0,
            frame_count: estimated_frames,
            platform: None,
            unity_version: None,
            file_size_bytes,
        },
        frames: vec![Frame {
            index: 0,
            duration_ms: 0.0,
            cpu_ms: 0.0,
            gc_alloc_bytes: 0,
            draw_calls: 0,
            set_pass_calls: 0,
            main_thread_samples: vec![Sample {
                name: "(PD3U 实时流估算)".to_string(),
                total_ms: 0.0,
                call_count: 0,
                max_ms: 0.0,
            }],
            gc_alloc_sites: vec![],
            render_events: vec![],
        }],
        warnings,
    })
}