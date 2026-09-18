//! Unity 2022.3 Memory counter table signature scan
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::findRawMemoryCounters`
//! 34 条连续 record：
//! ```
//! record = [counterId u32, valueCount u32=1, type u32, size u32, value bytes]
//! type 2 = 4 bytes (u32 value)
//! type 4 = 8 bytes (u64 value)
//! ```
//!
//! 严格条件：counterStoreId 严格升序 + 9 个值不变量。
//! 命中后写入 [`MemoryCounters`] 各字段。

use byteorder::{ByteOrder, LittleEndian};

use super::constants::{
    MEMORY_COUNTER_COUNT, RAW_MEMORY_COUNTER_SCHEMA, raw_memory_counter_value_size,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryCounters {
    pub system_used_bytes: u64,
    pub committed_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
    pub gc_used_bytes: u64,
    pub gc_reserved_bytes: u64,
    pub profiler_used_bytes: u64,
    pub profiler_reserved_bytes: u64,
    pub audio_used_bytes: u64,
    pub video_used_bytes: u64,
    pub gfx_used_bytes: u64,
    pub gfx_reserved_bytes: u64,
    pub texture_mem_bytes: u64,
    pub texture_count: u32,
    pub mesh_mem_bytes: u64,
    pub mesh_count: u32,
    pub material_count: u32,
    pub material_mem_bytes: u64,
    pub animation_clip_mem_bytes: u64,
    pub animation_clip_count: u32,
    pub asset_count: u32,
    pub gc_alloc_frame_bytes: u64,
    pub gc_alloc_frame_count: u32,
    pub object_count: u32,
    pub game_object_count: u32,
    pub scene_object_count: u32,
}

pub fn find(body: &[u8]) -> Option<MemoryCounters> {
    let records = RAW_MEMORY_COUNTER_SCHEMA.len();
    if records == 0 || body.len() < 16 + records * 8 {
        return None;
    }
    for start in 0..body.len().saturating_sub(16) {
        let mut pos = start;
        let mut values = [0u64; 34];
        let mut previous_id: u32 = 0;
        let mut valid = true;
        for (i, &value_type) in RAW_MEMORY_COUNTER_SCHEMA.iter().enumerate() {
            let size = raw_memory_counter_value_size(value_type);
            if pos + 16 + size > body.len() {
                valid = false;
                break;
            }
            let count = u32::from_le_bytes([
                body[pos + 4],
                body[pos + 5],
                body[pos + 6],
                body[pos + 7],
            ]);
            let typ = u32::from_le_bytes([
                body[pos + 8],
                body[pos + 9],
                body[pos + 10],
                body[pos + 11],
            ]);
            let sz = u32::from_le_bytes([
                body[pos + 12],
                body[pos + 13],
                body[pos + 14],
                body[pos + 15],
            ]);
            if count != MEMORY_COUNTER_COUNT || typ != value_type || sz != size as u32 {
                valid = false;
                break;
            }
            let id = u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            if i > 0 && id <= previous_id {
                valid = false;
                break;
            }
            previous_id = id;
            values[i] = if size == 4 {
                u32::from_le_bytes([
                    body[pos + 16],
                    body[pos + 17],
                    body[pos + 18],
                    body[pos + 19],
                ]) as u64
            } else {
                u64::from_le_bytes([
                    body[pos + 16],
                    body[pos + 17],
                    body[pos + 18],
                    body[pos + 19],
                    body[pos + 20],
                    body[pos + 21],
                    body[pos + 22],
                    body[pos + 23],
                ])
            };
            pos += 16 + size;
        }
        if !valid {
            continue;
        }
        if !valid_values(&values) {
            continue;
        }
        return Some(apply(&values));
    }
    None
}

fn valid_values(v: &[u64; 34]) -> bool {
    // Unity Memory module invariants (librashuai)
    v[4] > 0
        && v[4] <= v[5]
        && v[6] <= v[7]
        && v[16] >= v[18]
        && v[17] >= v[18]
        && v[23] >= v[25]
        && v[24] > 0
        && v[26] > 0
        && v[28] > 0
        && v[30] > 0
        && v[33] > 0
}

fn apply(v: &[u64; 34]) -> MemoryCounters {
    MemoryCounters {
        system_used_bytes: v[0],
        committed_bytes: v[2],
        used_bytes: v[4],
        reserved_bytes: v[5],
        gc_used_bytes: v[6],
        gc_reserved_bytes: v[7],
        profiler_used_bytes: v[8],
        profiler_reserved_bytes: v[9],
        audio_used_bytes: v[10],
        video_used_bytes: v[12],
        gfx_used_bytes: v[21],
        gfx_reserved_bytes: v[22],
        texture_mem_bytes: v[23],
        texture_count: v[24] as u32,
        mesh_mem_bytes: v[25],
        mesh_count: v[26] as u32,
        material_count: v[28] as u32,
        material_mem_bytes: v[27],
        animation_clip_mem_bytes: v[29],
        animation_clip_count: v[30] as u32,
        asset_count: v[16] as u32,
        gc_alloc_frame_bytes: v[20],
        gc_alloc_frame_count: v[19] as u32,
        object_count: v[33] as u32,
        game_object_count: v[18] as u32,
        scene_object_count: v[17] as u32,
    }
}

/// 简短构造函数（避免警告抑制 hack）。
#[allow(dead_code)]
fn _suppress_unused() {
    let _ = LittleEndian::read_u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_table(start_id: u32) -> Vec<u8> {
        // 构造一个满足 schema 与 invariants 的 34-record 序列
        // values:
        //   v[4]  = 100 MB (used)
        //   v[5]  = 200 MB (reserved, > used)
        //   v[6]  = 10 MB  (gc_used)
        //   v[7]  = 20 MB  (gc_reserved, >= gc_used)
        //   v[16] = 50     (asset_count)
        //   v[17] = 30     (scene_object_count)
        //   v[18] = 20     (game_object_count, <= scene_object_count)
        //   v[19] = 5      (gc_alloc_frame_count)
        //   v[20] = 4096   (gc_alloc_frame_bytes)
        //   v[21] = 50 MB  (gfx_used)
        //   v[22] = 100 MB (gfx_reserved)
        //   v[23] = 30 MB  (texture_mem)
        //   v[24] = 100    (texture_count, > 0)
        //   v[25] = 5 MB   (mesh_mem, <= texture_mem)
        //   v[26] = 50     (mesh_count, > 0)
        //   v[27] = 1 MB   (material_mem)
        //   v[28] = 30     (material_count, > 0)
        //   v[29] = 2 MB   (animation_clip_mem)
        //   v[30] = 10     (animation_clip_count, > 0)
        //   v[33] = 1000   (object_count, > 0)
        let v: [u64; 34] = [
            50_000_000, // 0  system_used
            0,
            60_000_000, // 2  committed
            0,
            100_000_000, // 4  used (must be > 0)
            200_000_000, // 5  reserved
            10_000_000,  // 6  gc_used
            20_000_000,  // 7  gc_reserved
            1_000_000,   // 8
            2_000_000,   // 9
            3_000_000,   // 10 audio_used
            0,
            4_000_000,   // 12 video_used
            0,
            0,
            0,
            50, // 16 asset_count
            30, // 17 scene_object_count
            20, // 18 game_object_count (≤ v[17])
            5,  // 19 gc_alloc_frame_count
            4096, // 20 gc_alloc_frame_bytes
            50_000_000, // 21 gfx_used
            100_000_000, // 22 gfx_reserved
            30_000_000, // 23 texture_mem (≥ v[25])
            100, // 24 texture_count
            5_000_000, // 25 mesh_mem
            50, // 26 mesh_count
            1_000_000, // 27 material_mem
            30, // 28 material_count
            2_000_000, // 29 animation_clip_mem
            10, // 30 animation_clip_count
            0,
            0,
            1000, // 33 object_count
        ];
        let mut body = Vec::new();
        for (i, &value_type) in RAW_MEMORY_COUNTER_SCHEMA.iter().enumerate() {
            let id = start_id + i as u32;
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&MEMORY_COUNTER_COUNT.to_le_bytes());
            body.extend_from_slice(&value_type.to_le_bytes());
            let size = raw_memory_counter_value_size(value_type) as u32;
            body.extend_from_slice(&size.to_le_bytes());
            if size == 4 {
                body.extend_from_slice(&(v[i] as u32).to_le_bytes());
            } else {
                body.extend_from_slice(&v[i].to_le_bytes());
            }
        }
        body
    }

    #[test]
    fn finds_valid_signature() {
        let table = synth_table(1000);
        let counters = find(&table).expect("should find");
        assert_eq!(counters.used_bytes, 100_000_000);
        assert_eq!(counters.reserved_bytes, 200_000_000);
        assert_eq!(counters.texture_count, 100);
        assert_eq!(counters.object_count, 1000);
    }

    #[test]
    fn signature_absent_returns_none() {
        // 全部 0 不可能命中（v[4] > 0 校验失败）
        let body = vec![0u8; 4096];
        assert!(find(&body).is_none());
    }
}