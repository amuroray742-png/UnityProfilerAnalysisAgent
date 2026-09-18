//! Unity 6 .data body 解码（reverse-engineered 真实文件验证）
//!
//! 通过用户提供的 Unity Editor ExtractProfilerDump.cs 输出 JSON 作为 ground truth，
//! 反推出来的 wire format（与 Unity 2022.3 主要差异在 marker 表，sample 表完全相同）：
//!
//! ```
//! frame_body = [
//!   frame_header(28B),                    // 已知，与 Unity 2022.3 一致
//!   stats_block(N),                        // prefix 已知，post-sentinel 待逆向
//!   ...
//!   main_thread_sample_table(M*20B),      // u32 marker_id + f32 total_ns + u64 start_ns + i32 children
//!   main_thread_gc_alloc_metadata(K*8B),   // i32 sample_index + u32 alloc_bytes
//!   ...
//! ]
//! ```

use byteorder::{ByteOrder, LittleEndian};
use crate::parser::ParseError;

/// 20 字节采样
#[derive(Debug, Clone, Copy)]
pub struct RawSample {
    pub marker_id: u32,
    pub total_ns: f32,
    pub start_ns: u64,
    pub children: i32,
}

/// 8 字节 GC.Alloc metadata
#[derive(Debug, Clone, Copy)]
pub struct RawGcAlloc {
    pub sample_index: i32,
    pub alloc_bytes: u32,
}

/// 尝试在 body 任意 4 字节对齐位置搜索 20 字节 sequence，要求 marker_id 匹配且 total_ns 在合理范围。
pub fn find_main_thread_samples(body: &[u8], expected_marker: u32, expected_total_ms: f32) -> Option<usize> {
    let expected_total_ns = expected_total_ms * 1e6;
    let mut i = 0;
    while i + 20 <= body.len() {
        let mid = LittleEndian::read_u32(&body[i..i + 4]);
        if mid == expected_marker {
            let tns = LittleEndian::read_f32(&body[i + 4..i + 8]);
            if (tns - expected_total_ns).abs() < 1.0 {
                return Some(i);
            }
        }
        i += 4;
    }
    None
}

/// 在 body 中找首个 GC.Alloc metadata 段（连续 5+ 条 (i32 sampleIndex, u32 bytes) 4B 对齐）
pub fn find_gc_alloc_metadata(body: &[u8]) -> Option<(usize, Vec<RawGcAlloc>)> {
    // 找一个 candidate pair 后，连续读取 5+ 条相同 stride 的对
    let mut i = 0;
    while i + 8 <= body.len() {
        let si = LittleEndian::read_i32(&body[i..i + 4]);
        let bt = LittleEndian::read_u32(&body[i + 4..i + 8]);
        if si >= 0 && si < 100_000 && bt > 0 && bt < 100_000 {
            // 尝试读后续 5 个 stride-4 或 stride-8 条
            for stride in [4usize, 8] {
                let mut entries = vec![RawGcAlloc { sample_index: si, alloc_bytes: bt }];
                let mut pos = i + stride;
                let mut ok = true;
                for _ in 0..4 {
                    if pos + 8 > body.len() {
                        ok = false;
                        break;
                    }
                    let next_si = LittleEndian::read_i32(&body[pos..pos + 4]);
                    let next_bt = LittleEndian::read_u32(&body[pos + 4..pos + 8]);
                    // 必须 sampleIndex 严格 +1（允许缺号但不允许倒退）
                    if next_si <= entries[entries.len() - 1].sample_index {
                        ok = false;
                        break;
                    }
                    if next_bt == 0 || next_bt > 1_000_000 {
                        ok = false;
                        break;
                    }
                    entries.push(RawGcAlloc { sample_index: next_si, alloc_bytes: next_bt });
                    pos += stride;
                }
                if ok && entries.len() >= 5 {
                    return Some((i, entries));
                }
            }
        }
        i += 4;
    }
    None
}

/// 读取从 sample_table_offset 开始的 N 个 sample。
pub fn read_samples(body: &[u8], offset: usize, count: usize) -> Result<Vec<RawSample>, ParseError> {
    if offset + count * 20 > body.len() {
        return Err(ParseError::Truncated(format!(
            "need {} bytes for {} samples at {}, have {}",
            count * 20,
            count,
            offset,
            body.len() - offset
        )));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = offset + i * 20;
        out.push(RawSample {
            marker_id: LittleEndian::read_u32(&body[o..o + 4]),
            total_ns: LittleEndian::read_f32(&body[o + 4..o + 8]),
            start_ns: LittleEndian::read_u64(&body[o + 8..o + 16]),
            children: LittleEndian::read_i32(&body[o + 16..o + 20]),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_main_thread_sample_0() {
        let mut body = vec![0u8; 1024];
        let o = 500;
        // sample = [marker_id=1506 u32][total_ns=49.334e6 f32][start_ns=any u64][children=2 i32]
        LittleEndian::write_u32(&mut body[o..o + 4], 1506);
        LittleEndian::write_f32(&mut body[o + 4..o + 8], 49_334_000.0);
        LittleEndian::write_u64(&mut body[o + 8..o + 16], 16_281_460_000_000);
        LittleEndian::write_i32(&mut body[o + 16..o + 20], 2);
        let found = find_main_thread_samples(&body, 1506, 49.334);
        assert_eq!(found, Some(o));
    }

    #[test]
    fn finds_gc_alloc_metadata() {
        let mut body = vec![0u8; 1024];
        let start = 100;
        let entries = [
            (459_i32, 20_u32),
            (460, 44),
            (757, 20),
            (758, 20),
            (759, 32),
        ];
        for (i, (si, bt)) in entries.iter().enumerate() {
            let o = start + i * 8;
            LittleEndian::write_i32(&mut body[o..o + 4], *si);
            LittleEndian::write_u32(&mut body[o + 4..o + 8], *bt);
        }
        let (off, parsed) = find_gc_alloc_metadata(&body).unwrap();
        assert_eq!(off, start);
        assert_eq!(parsed.len(), 5);
        for (i, (si, bt)) in entries.iter().enumerate() {
            assert_eq!(parsed[i].sample_index, *si);
            assert_eq!(parsed[i].alloc_bytes, *bt);
        }
    }

    #[test]
    fn reads_samples_in_order() {
        let mut body = vec![0u8; 20 * 3];
        LittleEndian::write_u32(&mut body[0..4], 100);
        LittleEndian::write_f32(&mut body[4..8], 5.5);
        LittleEndian::write_u64(&mut body[8..16], 999);
        LittleEndian::write_i32(&mut body[16..20], 2);
        let samples = read_samples(&body, 0, 3).unwrap();
        assert_eq!(samples[0].marker_id, 100);
        assert_eq!(samples[0].total_ns, 5.5);
        assert_eq!(samples[0].start_ns, 999);
        assert_eq!(samples[0].children, 2);
    }
}