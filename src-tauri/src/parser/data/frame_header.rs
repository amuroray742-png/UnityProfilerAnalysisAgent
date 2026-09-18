//! Frame body header（24 字节，两版本共用）
//!
//! ```
//! frameID          i32   // Unity 2022.3: == duplicateID; Unity 6.x: session counter
//! duplicateID      i32   // 6.x = frameID + 306（实测）
//! startNS          u64   // 单调时间戳（ns）
//! cpuUS            i32   // 主线程 CPU 时间（µs）
//! gpuUS            i32   // GPU 时间（µs；Unity 6 实测常为 0）
//! gatheredData     u32   // ==0 表示合成帧（librashuai 不变量）
//! ```

use super::reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub frame_id: i32,
    pub duplicate_id: i32,
    pub start_ns: u64,
    pub cpu_us: i32,
    pub gpu_us: i32,
    pub gathered_data: u32,
}

impl FrameHeader {
    /// `cpu_us != 0 && gathered_data == 0` 是 librashuai 识别的「合成帧」信号。
    ///
    /// 这种帧是 Unity Editor 的占位帧，不包含真实采样数据，必须丢弃。
    pub fn is_synthetic(&self) -> bool {
        self.cpu_us != 0 && self.gathered_data == 0
    }

    /// `cpu_us == 0` 表示该帧没有 CPU 时间数据，librashuai 视为 zeroDuration 跳过。
    pub fn is_zero_duration(&self) -> bool {
        self.cpu_us == 0
    }

    pub fn cpu_ms(&self) -> f64 {
        self.cpu_us as f64 / 1000.0
    }

    pub fn gpu_ms(&self) -> f64 {
        self.gpu_us as f64 / 1000.0
    }
}

pub fn read_frame_header(r: &mut Reader) -> FrameHeader {
    FrameHeader {
        frame_id: r.i32(),
        duplicate_id: r.i32(),
        start_ns: r.u64(),
        cpu_us: r.i32(),
        gpu_us: r.i32(),
        gathered_data: r.u32(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_with_frame(frame_id: i32, duplicate_id: i32, cpu_us: i32, gathered: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&frame_id.to_le_bytes());
        v.extend_from_slice(&duplicate_id.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes()); // start_ns
        v.extend_from_slice(&cpu_us.to_le_bytes());
        v.extend_from_slice(&0i32.to_le_bytes()); // gpu_us
        v.extend_from_slice(&gathered.to_le_bytes());
        v
    }

    #[test]
    fn parses_real_frame() {
        let buf = buf_with_frame(1, 307, 43_135, 15_613);
        let mut r = Reader::new(&buf);
        let h = read_frame_header(&mut r);
        assert_eq!(h.frame_id, 1);
        assert_eq!(h.duplicate_id, 307);
        assert_eq!(h.cpu_us, 43_135);
        assert_eq!(h.gathered_data, 15_613);
        assert!(!h.is_synthetic());
        assert!(!h.is_zero_duration());
        assert!((h.cpu_ms() - 43.135).abs() < 1e-9);
    }

    /// 实测 Unity 6.3.23f1 文件：frameID=0, duplicateID=306, cpuUS>0, gathered>0
    #[test]
    fn parses_unity6_real_frame() {
        let buf = buf_with_frame(0, 306, 36_178, 15_613);
        let mut r = Reader::new(&buf);
        let h = read_frame_header(&mut r);
        assert_eq!(h.frame_id, 0);
        assert_eq!(h.duplicate_id, 306);
        // duplicateID 不等于 frameID 但不算 synthetic（librashuai 的检查在 6.x 不适用）
        assert!(!h.is_synthetic());
    }

    #[test]
    fn detects_synthetic_frame() {
        let buf = buf_with_frame(1, 1, 16_000, 0);
        let mut r = Reader::new(&buf);
        let h = read_frame_header(&mut r);
        assert!(h.is_synthetic());
    }

    #[test]
    fn detects_zero_duration_frame() {
        let buf = buf_with_frame(2, 2, 0, 100);
        let mut r = Reader::new(&buf);
        let h = read_frame_header(&mut r);
        assert!(h.is_zero_duration());
        // 0 + gathered>0 不算 synthetic
        assert!(!h.is_synthetic());
    }
}