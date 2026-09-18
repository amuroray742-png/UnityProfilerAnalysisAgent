//! Cursor-based byte reader for Unity Profiler `.data` frame bodies.
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go` 的 `reader` 结构：
//! - 任何读失败只设置 `err`，不 panic，调用方按 `err.is_some()` 判断
//! - 字符串读取 NUL-terminated + 4 字节对齐（与 Unity 私有布局一致）
//! - `f32` / `f64` 都按位重解释；不假设 NaN 不会出现

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};

use super::constants::{MAX_STRING_BYTES, STRING_ALIGN};

#[derive(Debug)]
pub struct Reader<'a> {
    pub(crate) inner: Cursor<&'a [u8]>,
    pub(crate) err: Option<String>,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            inner: Cursor::new(buf),
            err: None,
        }
    }

    pub fn err(&self) -> Option<&str> {
        self.err.as_deref()
    }

    pub fn pos(&self) -> u64 {
        self.inner.position()
    }

    pub fn remaining(&self) -> usize {
        (self.inner.get_ref().len() as u64).saturating_sub(self.inner.position()) as usize
    }

    pub fn need(&mut self, n: usize) -> bool {
        if self.err.is_some() {
            return false;
        }
        if self.remaining() < n {
            self.err = Some(format!("unexpected EOF: want {} bytes, have {}", n, self.remaining()));
            return false;
        }
        true
    }

    pub fn skip(&mut self, n: usize) {
        if self.err.is_some() {
            return;
        }
        if !self.need(n) {
            return;
        }
        self.inner.set_position(self.inner.position() + n as u64);
    }

    pub fn u32(&mut self) -> u32 {
        if !self.need(4) {
            return 0;
        }
        match self.inner.read_u32::<LittleEndian>() {
            Ok(v) => v,
            Err(e) => {
                self.err = Some(format!("u32: {}", e));
                0
            }
        }
    }

    pub fn i32(&mut self) -> i32 {
        self.u32() as i32
    }

    pub fn u64(&mut self) -> u64 {
        if !self.need(8) {
            return 0;
        }
        match self.inner.read_u64::<LittleEndian>() {
            Ok(v) => v,
            Err(e) => {
                self.err = Some(format!("u64: {}", e));
                0
            }
        }
    }

    pub fn i64(&mut self) -> i64 {
        self.u64() as i64
    }

    pub fn f32(&mut self) -> f32 {
        let bits = self.u32();
        f32::from_bits(bits)
    }

    pub fn f64(&mut self) -> f64 {
        let bits = self.u64();
        f64::from_bits(bits)
    }

    /// NUL-terminated 字符串，按 4 字节对齐（与 Unity 私有格式一致）。
    ///
    /// Go 版本先扫 NUL 终止符，然后按 4 字节向上对齐；Rust 版本保持完全一致。
    pub fn str(&mut self) -> String {
        if self.err.is_some() {
            return String::new();
        }
        let start = self.inner.position() as usize;
        let max_end = (start + MAX_STRING_BYTES).min(self.inner.get_ref().len());
        // 复制需要的字节到本地 Vec，释放 self 的借用
        let nul_and_string: Option<(usize, Vec<u8>)> = {
            let bytes = self.inner.get_ref();
            let mut nul_pos: Option<usize> = None;
            for i in start..max_end {
                if bytes[i] == 0 {
                    nul_pos = Some(i);
                    break;
                }
            }
            nul_pos.map(|nul| {
                let s = bytes[start..nul].to_vec();
                (nul, s)
            })
        };
        let Some((nul, string_bytes)) = nul_and_string else {
            self.err = Some(format!("unterminated string starting at offset {}", start));
            return String::new();
        };
        let n = nul - start;
        let advance = ((n / STRING_ALIGN) + 1) * STRING_ALIGN;
        if !self.need(advance) {
            return String::new();
        }
        let result = match std::str::from_utf8(&string_bytes) {
            Ok(s) => s.to_string(),
            Err(e) => {
                self.err = Some(format!("utf8: {}", e));
                String::new()
            }
        };
        self.inner.set_position(self.inner.position() + advance as u64);
        result
    }

    /// 读取一段连续字节，不推进游标（用于内存 counter scan 等需要 peek 的场景）。
    pub fn peek_bytes(&self, n: usize) -> Option<&[u8]> {
        let pos = self.inner.position() as usize;
        let end = pos.checked_add(n)?;
        if end > self.inner.get_ref().len() {
            return None;
        }
        Some(&self.inner.get_ref()[pos..end])
    }

    /// 强制在游标处读取固定字节数（不要求 NUL）。
    pub fn read_bytes(&mut self, n: usize) -> Vec<u8> {
        if !self.need(n) {
            return Vec::new();
        }
        let mut out = vec![0u8; n];
        if self.inner.read_exact(&mut out).is_err() {
            self.err = Some("read_exact failed".to_string());
            return Vec::new();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of(values: &[u8]) -> Vec<u8> {
        values.to_vec()
    }

    #[test]
    fn reads_primitives_in_little_endian() {
        let buf = bytes_of(&[
            0x78, 0x56, 0x34, 0x12, // u32 = 0x12345678
            0xef, 0xcd, 0xab, 0x90, 0x78, 0x56, 0x34, 0x12, // u64 = 0x1234567890abcdef
            0x00, 0x00, 0x80, 0x3f, // f32 = 1.0
        ]);
        let mut r = Reader::new(&buf);
        assert_eq!(r.u32(), 0x12345678);
        assert_eq!(r.u64(), 0x1234567890abcdef);
        assert_eq!(r.f32(), 1.0);
        assert!(r.err().is_none());
    }

    #[test]
    fn truncates_gracefully() {
        let buf = bytes_of(&[1, 2, 3]); // < 4 bytes
        let mut r = Reader::new(&buf);
        let _ = r.u32();
        assert!(r.err().is_some());
        // 后续读取返回 0 不 panic
        assert_eq!(r.u64(), 0);
    }

    #[test]
    fn reads_nul_terminated_aligned_string() {
        // "ab\0" + 1 byte pad to 4-byte boundary = 4 bytes total
        let buf = bytes_of(&[b'a', b'b', 0x00, 0x00]);
        let mut r = Reader::new(&buf);
        assert_eq!(r.str(), "ab");
        assert_eq!(r.pos(), 4);
        assert!(r.err().is_none());
    }

    #[test]
    fn reads_string_without_trailing_nul_padding() {
        // "abc" + 1 byte pad to 4-byte boundary = 4 bytes
        let buf = bytes_of(&[b'a', b'b', b'c', 0x00]);
        let mut r = Reader::new(&buf);
        assert_eq!(r.str(), "abc");
        assert_eq!(r.pos(), 4);
    }

    #[test]
    fn reads_string_with_padding_bytes() {
        // "a" needs 4 bytes aligned (3 bytes after NUL)
        let buf = bytes_of(&[b'a', 0x00, b'X', b'Y']);
        let mut r = Reader::new(&buf);
        assert_eq!(r.str(), "a");
        assert_eq!(r.pos(), 4);
    }

    #[test]
    fn errors_on_unterminated_string() {
        // 16 bytes of 'a' with no NUL inside MAX_STRING_BYTES window
        let buf = bytes_of(&[b'a'; 16]);
        let mut r = Reader::new(&buf);
        let s = r.str();
        assert_eq!(s, "");
        assert!(r.err().is_some());
    }
}