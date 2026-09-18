//! Unity Profiler `.data` 格式常量
//!
//! 来自 `librashuai/UnityPerfAgent/internal/capture/capture.go` 的完整逆向
//! （Unity 2022.3）+ 用户实测 Unity 6000.3.23f1 文件验证。

/// Block header 的 magic / formatVersion。Unity 2022.3 与 6000.3 共用。
pub const UNITY_DATA_MAGIC: u32 = 0x20220328;

/// Block body 末尾的帧结束标记。
pub const FRAME_END_MARKER: u32 = 0xAFAFAFAF;

/// 文件结束标记（4 字节 LE u32）。
pub const FILE_END_MARKER: u32 = 0xDEADFEED;

/// Block header 长度。
pub const BLOCK_HEADER_SIZE: usize = 28;

/// Frame header 长度（28 字节：frameID i32 + duplicateID i32 + startNS u64
/// + cpuUS i32 + gpuUS i32 + gatheredData u32 = 4+4+8+4+4+4）。
pub const FRAME_HEADER_SIZE: usize = 28;

/// 单帧 body 最大尺寸（librashuai 上限）。
pub const MAX_FRAME_BODY_BYTES: u32 = 128 << 20; // 128 MiB

/// marker 数量上限。
pub const MAX_MARKER_DEFS: i32 = 100_000;

/// 每帧线程数量上限。
pub const MAX_THREADS_PER_FRAME: i32 = 512;

/// 每线程采样数量上限。
pub const MAX_SAMPLES_PER_THREAD: i32 = 1_000_000;

/// 字符串长度上限（防止错误数据导致无限读取）。
pub const MAX_STRING_BYTES: usize = 1 << 20; // 1 MiB

/// GC.Alloc metadata 中单条 Size 记录上限（1 GiB）。
pub const MAX_GC_ALLOC_BYTES: u32 = 1 << 30;

/// AllProfilerStats 固定长度（Unity 2022.3）。
pub const ALL_PROFILER_STATS_SIZE: usize = 1060;

/// AllProfilerStats 内 Audio Used Memory 偏移（Unity 2022.3）。
pub const ALL_PROFILER_STATS_AUDIO_OFFSET: usize = 656;

/// 字符串 4 字节对齐时的字节单位。
pub const STRING_ALIGN: usize = 4;

/// 把字节转 KB 的换算（GC.Alloc KB 用 uint64 / 1024 近似）。
pub const BYTES_PER_KB: u32 = 1024;

/// Unity Marker Category ID → 分类名（来自 librashuai）
///
/// Unity 6 沿用此映射（待进一步验证）。
pub fn category_name(id: u16) -> &'static str {
    match id {
        0 => "Render",
        1 => "Scripts",
        4 => "Gui",
        5 => "Physics",
        6 => "Animation",
        17 => "GC",
        18 => "VSync",
        26 => "UI Layout",
        27 => "UI Render",
        33 => "Physics2D",
        35 => "UI Details",
        _ => "",
    }
}

/// Unity Memory 模块固定 34-record counter 类型 schema（Unity 2022.3）。
///
/// `type=2` = 4 字节 u32 value；`type=4` = 8 字节 u64 value。
pub const RAW_MEMORY_COUNTER_SCHEMA: [u32; 34] = [
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 2, 4, 2, 2, 2, 2, 4, 4, 4, 4, 2, 4, 2, 4, 2, 4, 2, 4,
    2, 2,
];

/// 反向 type → value bytes 长度。
pub const fn raw_memory_counter_value_size(value_type: u32) -> usize {
    match value_type {
        2 => 4,
        4 => 8,
        _ => 0,
    }
}

/// `gatheredData == 0` 表示合成帧（librashuai 不变量）。
pub const GATHERED_DATA_SYNTHETIC: u32 = 0;

/// 内存计数器单条 record 中 count 字段期望值（始终 = 1）。
pub const MEMORY_COUNTER_COUNT: u32 = 1;

/// 把 Unity releaseType 编码转成 release suffix。
pub fn release_suffix(t: u32) -> &'static str {
    match t {
        0 => "a",
        1 => "b",
        2 => "f",
        3 => "c",
        _ => "?",
    }
}