//! Block header 解析
//!
//! 28 字节小端：
//!   [0..4]   formatVersion  (u32) — 必须 = `UNITY_DATA_MAGIC` (0x20220328)
//!   [4..8]   bodySize       (u32)
//!   [8..12]  unityMajor     (u32)
//!   [12..16] unityMinor     (u32)
//!   [16..20] unityPatch     (u32)
//!   [20..24] releaseType    (u32) — 0=a, 1=b, 2=f, 3=c
//!   [24..28] releaseNumber  (u32)

use byteorder::{LittleEndian, ReadBytesExt};

use super::constants::{
    BLOCK_HEADER_SIZE, FILE_END_MARKER, MAX_FRAME_BODY_BYTES, UNITY_DATA_MAGIC,
};
use crate::parser::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub format_version: u32,
    pub body_size: u32,
    pub unity_major: u32,
    pub unity_minor: u32,
    pub unity_patch: u32,
    pub release_type: u32,
    pub release_number: u32,
}

impl BlockHeader {
    /// 校验 magic + body_size 合法性。
    pub fn validate(&self) -> Result<(), ParseError> {
        if self.format_version != UNITY_DATA_MAGIC {
            return Err(ParseError::Other(format!(
                "formatVersion 0x{:08x} != expected 0x{:08x} (Unity .data magic)",
                self.format_version, UNITY_DATA_MAGIC
            )));
        }
        if self.body_size < 4 || self.body_size > MAX_FRAME_BODY_BYTES {
            return Err(ParseError::Other(format!(
                "frame body size {} not in [4, {}]",
                self.body_size, MAX_FRAME_BODY_BYTES
            )));
        }
        Ok(())
    }

    pub fn unity_version_string(&self) -> String {
        let release = match self.release_type {
            0 => "a",
            1 => "b",
            2 => "f",
            3 => "c",
            _ => "?",
        };
        format!(
            "{}.{}.{}{}{}",
            self.unity_major, self.unity_minor, self.unity_patch, release, self.release_number
        )
    }

    pub fn is_unity_6_or_later(&self) -> bool {
        self.unity_major >= 6000
    }
}

/// 从 `&[u8]`（长度必须 ≥ 28）解析 BlockHeader。
pub fn read_block_header(buf: &[u8]) -> Result<BlockHeader, ParseError> {
    if buf.len() < BLOCK_HEADER_SIZE {
        return Err(ParseError::Truncated(format!(
            "block header needs {} bytes, got {}",
            BLOCK_HEADER_SIZE,
            buf.len()
        )));
    }
    let mut c = std::io::Cursor::new(buf);
    let format_version = c
        .read_u32::<LittleEndian>()
        .map_err(|e| ParseError::Other(format!("header.format_version: {}", e)))?;
    let body_size = c
        .read_u32::<LittleEndian>()
        .map_err(|e| ParseError::Other(format!("header.body_size: {}", e)))?;
    let unity_major = c.read_u32::<LittleEndian>().unwrap_or(0);
    let unity_minor = c.read_u32::<LittleEndian>().unwrap_or(0);
    let unity_patch = c.read_u32::<LittleEndian>().unwrap_or(0);
    let release_type = c.read_u32::<LittleEndian>().unwrap_or(0);
    let release_number = c.read_u32::<LittleEndian>().unwrap_or(0);
    Ok(BlockHeader {
        format_version,
        body_size,
        unity_major,
        unity_minor,
        unity_patch,
        release_type,
        release_number,
    })
}

/// 文件结束标记判别。
pub fn is_file_end_marker(first4: &[u8]) -> bool {
    first4.len() >= 4
        && u32::from_le_bytes([first4[0], first4[1], first4[2], first4[3]]) == FILE_END_MARKER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_header(major: u32, body_size: u32) -> Vec<u8> {
        let mut v = vec![0u8; BLOCK_HEADER_SIZE];
        v[0..4].copy_from_slice(&UNITY_DATA_MAGIC.to_le_bytes());
        v[4..8].copy_from_slice(&body_size.to_le_bytes());
        v[8..12].copy_from_slice(&major.to_le_bytes());
        v[12..16].copy_from_slice(&3u32.to_le_bytes());
        v[16..20].copy_from_slice(&23u32.to_le_bytes());
        v[20..24].copy_from_slice(&2u32.to_le_bytes()); // f
        v[24..28].copy_from_slice(&1u32.to_le_bytes());
        v
    }

    #[test]
    fn parses_unity_2022_header() {
        let buf = synth_header(2022, 4096);
        let h = read_block_header(&buf).unwrap();
        assert_eq!(h.unity_major, 2022);
        assert_eq!(h.body_size, 4096);
        assert_eq!(h.unity_version_string(), "2022.3.23f1");
        assert!(h.validate().is_ok());
        assert!(!h.is_unity_6_or_later());
    }

    #[test]
    fn parses_unity_6000_header() {
        let buf = synth_header(6000, 5_493_756);
        let h = read_block_header(&buf).unwrap();
        assert_eq!(h.unity_major, 6000);
        assert_eq!(h.unity_version_string(), "6000.3.23f1");
        assert!(h.is_unity_6_or_later());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut buf = synth_header(2022, 4096);
        buf[0..4].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        let h = read_block_header(&buf).unwrap();
        assert!(h.validate().is_err());
    }

    #[test]
    fn rejects_zero_body_size() {
        let buf = synth_header(2022, 0);
        let h = read_block_header(&buf).unwrap();
        assert!(h.validate().is_err());
    }

    #[test]
    fn rejects_oversized_body() {
        let buf = synth_header(2022, MAX_FRAME_BODY_BYTES + 1);
        let h = read_block_header(&buf).unwrap();
        assert!(h.validate().is_err());
    }

    #[test]
    fn detects_file_end_marker() {
        let mut buf = vec![0u8; 4];
        buf.copy_from_slice(&FILE_END_MARKER.to_le_bytes());
        assert!(is_file_end_marker(&buf));
        assert!(!is_file_end_marker(&[0u8; 4]));
    }
}