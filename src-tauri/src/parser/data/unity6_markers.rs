//! Unity 6000.x marker 定义表解析（实测格式）
//!
//! 与 Unity 2022.3 不同：实测 marker entry 布局：
//! ```
//! [markerId u32][1 u32][unknown u32][name_size u32][name bytes (no NUL)]
//! ```
//!
//! 中间两个 u32 字段的含义待 UnityCsReference Unity 6 源码确认；
//! 当前暂记为 `kind=1` 和 `flags_u32`，避免破坏数据。

use std::collections::HashMap;

use super::constants::MAX_MARKER_DEFS;
use super::markers::MarkerInfo;
use super::reader::Reader;
use crate::parser::ParseError;

#[derive(Debug, Clone)]
pub struct Unity6MarkerEntry {
    pub info: MarkerInfo,
    pub unknown_field: u32,
}

pub fn read_markers(r: &mut Reader) -> Result<HashMap<u32, Unity6MarkerEntry>, ParseError> {
    let mut out = HashMap::new();
    let n = r.i32();
    if n < 0 || n > MAX_MARKER_DEFS {
        return Err(ParseError::Other(format!("unity6 marker count {} invalid", n)));
    }
    for _ in 0..n {
        let id = r.u32();
        let kind = r.u32(); // 实测常 = 1
        let unknown = r.u32();
        let name_size = r.u32();
        if name_size > 4096 {
            return Err(ParseError::Other(format!(
                "unity6 marker #{} name_size {} too large",
                id, name_size
            )));
        }
        // 实测 Unity 6 不做 4 字节对齐：name 直接 name_size 字节，无 padding
        let name_size_u = name_size as usize;
        let advance = name_size_u;
        if let Some(e) = r.err.as_ref() {
            return Err(ParseError::Other(format!("unity6 marker #{} header: {}", id, e)));
        }
        let _ = kind;
        let name_bytes = read_aligned_bytes(r, advance)?;
        let name = String::from_utf8_lossy(&name_bytes[..name_size_u]).into_owned();
        out.insert(
            id,
            Unity6MarkerEntry {
                info: MarkerInfo { name, category_id: 0 },
                unknown_field: unknown,
            },
        );
        if let Some(e) = r.err.as_ref() {
            return Err(ParseError::Other(format!("unity6 marker #{} body: {}", id, e)));
        }
    }
    Ok(out)
}

fn read_aligned_bytes(r: &mut Reader, advance: usize) -> Result<Vec<u8>, ParseError> {
    if r.remaining() < advance {
        return Err(ParseError::Truncated(format!(
            "need {} bytes, have {}",
            advance,
            r.remaining()
        )));
    }
    let mut v = Vec::with_capacity(advance);
    let mut i = 0;
    while i < advance {
        let take = (advance - i).min(4);
        let word = r.u32();
        let bytes = word.to_le_bytes();
        v.extend_from_slice(&bytes[..take]);
        i += take;
    }
    Ok(v)
}

impl<'a> Reader<'a> {
    /// 读取 1 个字节（同步推进游标）。
    pub fn u8(&mut self) -> u8 {
        if !self.need(1) {
            return 0;
        }
        let pos = self.inner.position() as usize;
        let b = self.inner.get_ref()[pos];
        self.inner.set_position(self.inner.position() + 1);
        b
    }

    /// 在当前位置向前看 N 字节，返回 4 字节 LE u32（前缀）。
    pub fn peek_u32_at(&self, offset: usize) -> Option<u32> {
        let pos = self.inner.position() as usize + offset;
        let bytes = self.inner.get_ref().get(pos..pos + 4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_unity6_marker() {
        // 实测格式 (用户 Unity 6 文件):
        //   entry = [id u32][1 u32][?? u32][name_size u32][name bytes (no padding)]
        let name = b"GetAndClearChangedTransforms"; // 28 chars
        let mut buf = Vec::new();
        // count = 2
        buf.extend_from_slice(&2i32.to_le_bytes());
        // marker 0x9D
        buf.extend_from_slice(&0x9Du32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name);
        // marker 0x9E
        let name2 = b"SBOJ";
        buf.extend_from_slice(&0x9Eu32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&0xBu32.to_le_bytes());
        buf.extend_from_slice(&(name2.len() as u32).to_le_bytes());
        buf.extend_from_slice(name2);

        let mut reader = R::new(&buf);
        let markers = read_markers(&mut reader).unwrap();
        assert_eq!(markers.len(), 2);
        let m = &markers.get(&0x9D).unwrap().info;
        assert_eq!(m.name, "GetAndClearChangedTransforms");
        let m2 = &markers.get(&0x9E).unwrap().info;
        assert_eq!(m2.name, "SBOJ");
    }

    use crate::parser::data::reader::Reader as R;
}