//! GC.Alloc metadata strict-match
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::readGCAllocMetadata`。
//! 主线程采样表后有一段 `(sampleIndex i32, bytes u32)` 对，每个 GC.Alloc
//! 采样必须恰有一条 size 记录与之对应（"strict full-match"）。
//!
//! 对齐偏移在 0..7 之间；依次尝试每个对齐以应对不同的写入器实现。
//! 不完整匹配静默丢弃（不修改 samples，与 Go 行为一致）。

use std::collections::HashMap;

use byteorder::{ByteOrder, LittleEndian};

use super::constants::MAX_GC_ALLOC_BYTES;
use super::samples::DiskSample;
use super::markers::MarkerInfo;

/// 把 GC.Alloc metadata 归因到对应 sample。
///
/// - `body`: 整个 frame body（除末尾 4 字节 frameEndMarker 外）  
/// - `metadata_start`: 当前 reader 游标位置相对于 body 的字节偏移  
/// - `samples`: 主线程采样（mut；`gc_alloc_bytes` 字段会被设置）
/// - `markers`: marker ID → MarkerInfo，用于识别 GC.Alloc 采样
pub fn associate(body: &[u8], metadata_start: usize, samples: &mut [DiskSample], markers: &HashMap<u32, MarkerInfo>) {
    // 1. 收集所有 GC.Alloc 采样索引
    let mut gc_indexes: HashMap<i32, ()> = HashMap::new();
    for (i, s) in samples.iter().enumerate() {
        if let Some(info) = markers.get(&s.marker_id) {
            if info.name == "GC.Alloc" {
                gc_indexes.insert(i as i32, ());
            }
        }
    }
    if gc_indexes.is_empty() || metadata_start + 8 > body.len() {
        return;
    }

    let mut best: HashMap<i32, u32> = HashMap::new();
    // 2. 尝试每个 8 字节对齐偏移（0..8）
    for alignment in 0..8usize {
        let mut run: HashMap<i32, u32> = HashMap::new();
        let mut pos = metadata_start + alignment;
        while pos + 8 <= body.len() {
            let index = i32::from_le_bytes([
                body[pos],
                body[pos + 1],
                body[pos + 2],
                body[pos + 3],
            ]);
            let size = u32::from_le_bytes([
                body[pos + 4],
                body[pos + 5],
                body[pos + 6],
                body[pos + 7],
            ]);
            pos += 8;
            let wanted = gc_indexes.contains_key(&index);
            let duplicate = run.contains_key(&index);
            if wanted && !duplicate && size <= MAX_GC_ALLOC_BYTES {
                run.insert(index, size);
                continue;
            }
            // 不匹配时结算当前 run
            if run.len() > best.len() {
                best = run.clone();
            }
            run.clear();
        }
        if run.len() > best.len() {
            best = run;
        }
    }

    // 3. 严格一对一：所有 GC.Alloc 必须匹配上才应用
    if best.len() != gc_indexes.len() {
        return;
    }
    for (index, size) in best {
        if let Some(s) = samples.get_mut(index as usize) {
            s.gc_alloc_bytes = size;
        }
    }
    let _ = LittleEndian::read_u32; // 抑制未使用 import 警告
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::data::markers::MarkerInfo;

    fn synth_metadata_body() -> Vec<u8> {
        let mut body = vec![0u8; 64];
        // metadataStart = 8
        // alignment = 0 (already 8-aligned)
        // entry at pos 8: sampleIndex=0, size=64
        body[8..12].copy_from_slice(&0i32.to_le_bytes());
        body[12..16].copy_from_slice(&64u32.to_le_bytes());
        // entry at pos 16: sampleIndex=2, size=512
        body[16..20].copy_from_slice(&2i32.to_le_bytes());
        body[20..24].copy_from_slice(&512u32.to_le_bytes());
        // entry at pos 24: bogus
        body[24..28].copy_from_slice(&(-1i32).to_le_bytes());
        body
    }

    fn gc_marker_map() -> HashMap<u32, MarkerInfo> {
        let mut m = HashMap::new();
        m.insert(7, MarkerInfo { name: "GC.Alloc".into(), category_id: 17 });
        m
    }

    #[test]
    fn associates_strict_match() {
        let body = synth_metadata_body();
        let mut samples = vec![
            DiskSample { marker_id: 7, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
            DiskSample { marker_id: 9, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
            DiskSample { marker_id: 7, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
        ];
        let markers = gc_marker_map();
        associate(&body, 8, &mut samples, &markers);
        assert_eq!(samples[0].gc_alloc_bytes, 64);
        assert_eq!(samples[1].gc_alloc_bytes, 0);
        assert_eq!(samples[2].gc_alloc_bytes, 512);
    }

    #[test]
    fn incomplete_match_silently_dropped() {
        let body = synth_metadata_body();
        let mut samples = vec![
            DiskSample { marker_id: 7, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
            DiskSample { marker_id: 9, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
            DiskSample { marker_id: 7, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
            // 额外的 GC.Alloc 没有 metadata
            DiskSample { marker_id: 7, total_ns: 0.0, start_ns: 0, children: 0, gc_alloc_bytes: 0 },
        ];
        let markers = gc_marker_map();
        associate(&body, 8, &mut samples, &markers);
        // strict-match 失败：全部归 0
        assert_eq!(samples[0].gc_alloc_bytes, 0);
        assert_eq!(samples[2].gc_alloc_bytes, 0);
    }
}