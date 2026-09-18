//! Unity 2022.3 marker 定义表解析
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::readMarkers`
//! ```
//! int32 markerCount
//! for each marker:
//!     u32 markerID
//!     string name (4B-aligned, NUL-terminated)
//!     u32 groupFlags           // categoryID = groupFlags >> 16
//!     int32 metaCount
//!     metaCount × (u32 + string)
//! ```

use std::collections::HashMap;

use super::constants::MAX_MARKER_DEFS;
use super::reader::Reader;
use crate::parser::ParseError;

#[derive(Debug, Clone)]
pub struct MarkerInfo {
    pub name: String,
    pub category_id: u16,
}

pub fn read_markers(r: &mut Reader, markers: &mut HashMap<u32, MarkerInfo>) -> Result<(), ParseError> {
    let n = r.i32();
    if n < 0 || n > MAX_MARKER_DEFS {
        return Err(ParseError::Other(format!("marker count {} invalid", n)));
    }
    for _ in 0..n {
        let id = r.u32();
        let name = r.str();
        let group_flags = r.u32();
        let meta = r.i32();
        if meta < 0 || meta > MAX_MARKER_DEFS {
            return Err(ParseError::Other(format!("marker meta count {} invalid", meta)));
        }
        markers.insert(
            id,
            MarkerInfo {
                name: name.clone(),
                category_id: (group_flags >> 16) as u16,
            },
        );
        for _ in 0..meta {
            r.u32();
            r.str();
        }
        if let Some(e) = r.err() {
            return Err(ParseError::Other(format!("marker #{}: {}", id, e)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::data::reader::Reader as R;

    fn buf_with_markers(names: &[(&str, u16)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(names.len() as i32).to_le_bytes());
        for (i, (name, cat)) in names.iter().enumerate() {
            let id = (i + 1) as u32;
            v.extend_from_slice(&id.to_le_bytes());
            // string: name + NUL + pad to 4-byte boundary
            let name_bytes = name.as_bytes();
            v.extend_from_slice(name_bytes);
            v.push(0);
            let pad = (4 - ((name_bytes.len() + 1) % 4)) % 4;
            v.extend(std::iter::repeat(0u8).take(pad));
            // groupFlags: lower 16 bits arbitrary, upper 16 bits = category
            let group_flags: u32 = ((*cat as u32) << 16) | 0x1234;
            v.extend_from_slice(&group_flags.to_le_bytes());
            // meta count = 0
            v.extend_from_slice(&0i32.to_le_bytes());
        }
        v
    }

    #[test]
    fn reads_markers_table() {
        let buf = buf_with_markers(&[
            ("GC.Alloc", 17),
            ("PlayerLoop", 0),
            ("BehaviourUpdate", 1),
        ]);
        let mut r = R::new(&buf);
        let mut markers = HashMap::new();
        read_markers(&mut r, &mut markers).unwrap();

        assert_eq!(markers.len(), 3);
        assert_eq!(markers.get(&1).unwrap().name, "GC.Alloc");
        assert_eq!(markers.get(&1).unwrap().category_id, 17);
        assert_eq!(markers.get(&2).unwrap().name, "PlayerLoop");
        assert_eq!(markers.get(&3).unwrap().name, "BehaviourUpdate");
    }

    #[test]
    fn rejects_negative_count() {
        let buf = (-5i32).to_le_bytes().to_vec();
        let mut r = R::new(&buf);
        let mut markers = HashMap::new();
        assert!(read_markers(&mut r, &mut markers).is_err());
    }
}