//! `.raw` 格式尽力解析器
//!
//! Unity 老版本 Profiler 窗口 "Save to file" 出来的二进制格式。
//! 该格式未公开文档，社区有部分反向工程（基于 PackedFile 流）。
//!
//! 本解析器采用"尽力而为"策略：
//! - 验证 magic number
//! - 解析文件头
//! - 尝试解析 frame index
//! - 任何失败都不报错，而是返回部分结果 + warnings
//!
//! 主要目的：让 JSON 不可用时仍能给用户一些信息。

use bytes::{Buf, Bytes};

use super::{Frame, ParsedProfile, ParseError, ProfileMeta, Sample};

/// 已知的 Unity Profiler .raw magic（部分版本）
const MAGIC_CANDIDATES: &[&[u8]] = &[
    b"UNITY",
    b"\x50\x52\x4F\x46", // "PROF"
    b"PROFILER",
];

pub async fn parse(
    bytes: &Bytes,
    file_name: &str,
    file_size_bytes: u64,
) -> Result<ParsedProfile, ParseError> {
    let mut warnings = Vec::new();
    let mut buf = bytes.clone();

    // 1. 尝试匹配 magic
    let mut matched_magic = None;
    for magic in MAGIC_CANDIDATES {
        if buf.len() >= magic.len() && &buf[..magic.len()] == *magic {
            matched_magic = Some(magic.to_vec());
            break;
        }
    }

    if matched_magic.is_none() {
        warnings.push(format!(
            ".raw 文件 magic 不匹配（期望 {:?} 之一）。文件可能不是 Unity Profiler .raw 或格式未知。",
            MAGIC_CANDIDATES
                .iter()
                .map(|m| String::from_utf8_lossy(m).to_string())
                .collect::<Vec<_>>()
        ));
        // 不直接报错：构造空 profile + warning
        return Ok(empty_profile(file_name, file_size_bytes, warnings));
    }

    // 2. 跳过 magic
    let magic_len = matched_magic.unwrap().len();
    buf.advance(magic_len);

    // 3. 尝试解析版本号（u32 LE）
    if buf.remaining() < 4 {
        warnings.push("文件过短，无法解析版本号".to_string());
        return Ok(empty_profile(file_name, file_size_bytes, warnings));
    }
    let version = buf.get_u32_le();
    warnings.push(format!("检测到 Unity Profiler .raw，版本号: {}", version));

    // 4. 尝试估算 frame 数量（启发式：按 4KB 平均 frame 大小）
    let estimated_frames = (file_size_bytes / 4096).max(1) as usize;
    warnings.push(format!(
        "无法精确解析 .raw frame 结构（私有格式未公开），估算约 {} 帧",
        estimated_frames
    ));

    Ok(ParsedProfile {
        meta: ProfileMeta {
            file_name: file_name.to_string(),
            format: super::ProfilerFormat::Raw,
            duration_ms: 0.0,
            frame_count: estimated_frames,
            platform: None,
            unity_version: Some(format!("raw-v{}", version)),
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
                name: "(.raw 格式未完整解析)".to_string(),
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

fn empty_profile(file_name: &str, file_size_bytes: u64, warnings: Vec<String>) -> ParsedProfile {
    ParsedProfile {
        meta: ProfileMeta {
            file_name: file_name.to_string(),
            format: super::ProfilerFormat::Raw,
            duration_ms: 0.0,
            frame_count: 0,
            platform: None,
            unity_version: None,
            file_size_bytes,
        },
        frames: vec![],
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_bad_magic() {
        let bytes = Bytes::from_static(b"NOPE\x00\x00\x00");
        let profile = parse(&bytes, "fake.raw", bytes.len() as u64).await.unwrap();
        assert!(profile.warnings.iter().any(|w| w.contains("magic")));
        assert_eq!(profile.frames.len(), 0);
    }

    #[tokio::test]
    async fn parses_matching_magic() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"UNITY");
        bytes.extend_from_slice(&42u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 100]);

        let profile = parse(&Bytes::from(bytes.clone()), "ok.raw", bytes.len() as u64)
            .await
            .unwrap();
        assert!(profile.warnings.iter().any(|w| w.contains("版本号: 42")));
    }
}