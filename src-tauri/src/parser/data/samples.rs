//! Unity 2022.3 主线程采样表解析
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::readMainThreadSamples`
//! ```
//! int32 threadCount
//! for each thread:
//!     u64 threadId
//!     string groupName (4B-aligned)
//!     string threadName (4B-aligned)   // thread[0] 必须是 "Main Thread"
//!     int32 sampleCount
//!     sampleCount × [u32 markerID, f32 totalNS, u64 startNS, i32 children]
//! ```
//!
//! 返回的是扁平的 disk sample 数组（带 marker ID 与 GC.Alloc 字节占位）。
//! 后续由 `forest` 模块构建层级。

use super::constants::{MAX_SAMPLES_PER_THREAD, MAX_THREADS_PER_FRAME};
use super::reader::Reader;
use crate::parser::ParseError;

#[derive(Debug, Clone, Copy)]
pub struct DiskSample {
    pub marker_id: u32,
    pub total_ns: f32,
    pub start_ns: u64,
    pub children: i32,
    pub gc_alloc_bytes: u32,
}

pub fn read_main_thread_samples(r: &mut Reader) -> Result<Vec<DiskSample>, ParseError> {
    let threads = r.i32();
    if threads < 0 || threads > MAX_THREADS_PER_FRAME {
        return Err(ParseError::Other(format!("thread count {} invalid", threads)));
    }
    for _ in 0..threads {
        r.u64(); // threadId
        r.str(); // groupName
        let name = r.str(); // threadName
        let count = r.i32();
        if count < 0 || count > MAX_SAMPLES_PER_THREAD {
            return Err(ParseError::Other(format!("sample count {} invalid", count)));
        }
        if name != "Main Thread" {
            return Err(ParseError::Other(format!(
                "first thread must be 'Main Thread', got '{}'",
                name
            )));
        }
        let mut samples = Vec::with_capacity(count as usize);
        for _ in 0..count {
            samples.push(DiskSample {
                marker_id: r.u32(),
                total_ns: r.f32(),
                start_ns: r.u64(),
                children: r.i32(),
                gc_alloc_bytes: 0,
            });
        }
        if let Some(e) = r.err() {
            return Err(ParseError::Other(format!("samples: {}", e)));
        }
        return Ok(samples);
    }
    Err(ParseError::Other("no Main Thread found".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::data::reader::Reader as R;

    fn buf_with_samples() -> Vec<u8> {
        let mut v = Vec::new();
        // threadCount = 1
        v.extend_from_slice(&1i32.to_le_bytes());
        // thread[0]
        v.extend_from_slice(&42u64.to_le_bytes()); // threadId
        // groupName "" + NUL + pad
        v.push(0);
        v.extend_from_slice(&[0, 0, 0]); // pad to 4-byte
        // threadName "Main Thread\0\0\0" (11 chars + NUL = 12 bytes already 4-aligned)
        let main = b"Main Thread";
        v.extend_from_slice(main);
        v.push(0);
        let pad = (4 - ((main.len() + 1) % 4)) % 4;
        v.extend(std::iter::repeat(0u8).take(pad));
        // sampleCount = 2
        v.extend_from_slice(&2i32.to_le_bytes());
        // sample[0]: marker=1, totalNS=10.5, startNS=100, children=0
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&10.5f32.to_le_bytes());
        v.extend_from_slice(&100u64.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes());
        // sample[1]: marker=2, totalNS=20.0, startNS=200, children=1
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&20.0f32.to_le_bytes());
        v.extend_from_slice(&200u64.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes());
        v
    }

    #[test]
    fn reads_main_thread_samples() {
        let buf = buf_with_samples();
        let mut r = R::new(&buf);
        let samples = read_main_thread_samples(&mut r).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].marker_id, 1);
        assert!((samples[0].total_ns - 10.5).abs() < 1e-3);
        assert_eq!(samples[1].children, 1);
    }

    #[test]
    fn rejects_non_main_thread_first() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&1i32.to_le_bytes()); // threadCount
        buf.extend_from_slice(&1u64.to_le_bytes()); // threadId
        // groupName ""
        buf.push(0);
        buf.extend_from_slice(&[0, 0, 0]);
        // threadName "Worker"
        buf.extend_from_slice(b"Worker");
        buf.push(0);
        buf.extend_from_slice(&[0]); // pad

        let mut r = R::new(&buf);
        assert!(read_main_thread_samples(&mut r).is_err());
    }
}