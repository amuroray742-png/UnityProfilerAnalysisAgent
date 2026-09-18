//! Unity 2022.3 stats block 解析
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::readStatsAndAuxiliary`
//! ```
//! MemoryStats    33 × u32 + 1 pad u32
//! id/value pairs until id == -1
//! 16 platform values (16 × u32)
//! AllProfilerStats  1060 bytes (Audio Used Memory @ offset 656)
//! aux array #1   int32 count, count × 64 bytes
//! aux array #2   int32 count, count × 44 bytes
//! audio names    int32 len, len bytes
//! UI Canvas      int32 count, count × 52 bytes
//! names blob     int32 len, len bytes (4-byte aligned)
//! event markers  int32 count, count × 12 bytes
//! names blob     int32 len, len bytes (4-byte aligned)
//! UI Batch       int32 count, count × 4 bytes
//! ```
//!
//! 本模块只解析 Audio Used Memory（来自 AllProfilerStats 固定偏移）。
//! 其余字段只是按字节 skip 推进游标。

use super::constants::{
    ALL_PROFILER_STATS_AUDIO_OFFSET, ALL_PROFILER_STATS_SIZE, MAX_THREADS_PER_FRAME,
};
use super::reader::Reader;
use byteorder::{ByteOrder, LittleEndian};
use crate::parser::ParseError;

#[derive(Debug, Default, Clone)]
pub struct StatsResult {
    pub audio_used_bytes: u32,
}

pub fn read_stats(r: &mut Reader) -> Result<StatsResult, ParseError> {
    if let Some(e) = r.err() {
        return Err(ParseError::Other(format!("before stats: {}", e)));
    }
    // MemoryStats: 33 u32 + 1 pad u32
    r.skip(33 * 4);
    r.skip(4);

    // id/value pairs until id == -1
    loop {
        let id = r.i32();
        if let Some(e) = r.err.as_ref() {
            return Err(ParseError::Other(format!("stats id/value pairs: {}", e)));
        }
        if id == -1 {
            break;
        }
        r.skip(4); // value
    }

    // 16 platform values
    r.skip(16 * 4);

    // AllProfilerStats fixed 1060 bytes; read Audio Used Memory at offset 656
    let pos = pos_remaining(r);
    if pos < ALL_PROFILER_STATS_SIZE {
        return Err(ParseError::Truncated(format!(
            "AllProfilerStats needs {} bytes, have {}",
            ALL_PROFILER_STATS_SIZE, pos
        )));
    }
    // Need to peek into the buffer at pos + audio_offset
    let audio_bytes = peek_u32_at_offset(r, ALL_PROFILER_STATS_AUDIO_OFFSET).unwrap_or(0);
    r.skip(ALL_PROFILER_STATS_SIZE);

    // aux array #1 (64-byte stride)
    skip_aux_array(r, 64)?;
    // aux array #2 (44-byte stride)
    skip_aux_array(r, 44)?;
    // audio names: int32 len + bytes
    skip_blob(r)?;
    // UI Canvas
    skip_threaded_array(r, 52)?;
    // names blob
    skip_names_blob(r)?;
    // event markers: count × 12 bytes
    skip_threaded_array(r, 12)?;
    // names blob again
    skip_names_blob(r)?;
    // UI Batch: int32 count, count × 4 bytes
    skip_threaded_array(r, 4)?;

    Ok(StatsResult {
        audio_used_bytes: audio_bytes,
    })
}

fn skip_aux_array(r: &mut Reader, stride: usize) -> Result<(), ParseError> {
    let n = r.i32();
    if n < 0 || n > MAX_THREADS_PER_FRAME {
        return Err(ParseError::Other(format!("aux array count {} invalid", n)));
    }
    r.skip(n as usize * stride);
    Ok(())
}

fn skip_threaded_array(r: &mut Reader, stride: usize) -> Result<(), ParseError> {
    let n = r.i32();
    if n < 0 || n > MAX_THREADS_PER_FRAME {
        return Err(ParseError::Other(format!("aux array count {} invalid", n)));
    }
    r.skip(n as usize * stride);
    Ok(())
}

fn skip_blob(r: &mut Reader) -> Result<(), ParseError> {
    let n = r.i32();
    if n < 0 {
        return Err(ParseError::Other(format!("blob length {} invalid", n)));
    }
    r.skip(n as usize);
    Ok(())
}

fn skip_names_blob(r: &mut Reader) -> Result<(), ParseError> {
    let n = r.i32();
    if n < 0 {
        return Err(ParseError::Other(format!("names blob length {} invalid", n)));
    }
    r.skip(n as usize);
    // 4-byte align
    if let Some(rem) = (r.pos() as usize).checked_rem(4) {
        if rem != 0 {
            r.skip(4 - rem);
        }
    }
    Ok(())
}

fn pos_remaining(r: &Reader) -> usize {
    r.remaining()
}

fn peek_u32_at_offset(r: &Reader, offset: usize) -> Option<u32> {
    let bytes = r.peek_bytes(offset + 4)?;
    Some(LittleEndian::read_u32(&bytes[offset..offset + 4]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::data::reader::Reader as R;

    #[test]
    fn reads_audio_memory_from_all_profiler_stats() {
        // MemoryStats 33*4 + pad 4 + 1 id(-1) + 16*4 platform + 1060 AllProfilerStats + 0 aux + ...
        let mut buf = vec![0u8; 33 * 4 + 4 + 4 + 16 * 4 + ALL_PROFILER_STATS_SIZE + 4 * 8];

        // id = -1 sentinel
        let pos = 33 * 4 + 4;
        buf[pos..pos + 4].copy_from_slice(&(-1i32).to_le_bytes());

        // Audio bytes = 3.5 MB at offset 656 inside AllProfilerStats
        let audio_offset = 33 * 4 + 4 + 4 + 16 * 4 + ALL_PROFILER_STATS_AUDIO_OFFSET;
        let audio_bytes: u32 = 3 * 1024 * 1024 + 512 * 1024;
        buf[audio_offset..audio_offset + 4].copy_from_slice(&audio_bytes.to_le_bytes());

        // UI Batch count = 0
        let ui_batch_pos = 33 * 4 + 4 + 4 + 16 * 4 + ALL_PROFILER_STATS_SIZE + 4 * 6;
        buf[ui_batch_pos..ui_batch_pos + 4].copy_from_slice(&0i32.to_le_bytes());

        let mut r = R::new(&buf);
        let stats = read_stats(&mut r).unwrap();
        assert_eq!(stats.audio_used_bytes, audio_bytes);
        assert!(r.err().is_none());
    }
}