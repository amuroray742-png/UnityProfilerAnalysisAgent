//! Unity Profiler 格式解析器
//!
//! 支持格式：
//! - `.json`：Unity Profiler 窗口 → Export → JSON（结构化，MVP 主路径）
//! - `.raw`：Unity 老版本二进制格式（尽力解析）
//! - `.data`：Unity 2021+ 私有离线格式（部分支持）
//! - `.pd3u`：PlayerConnection Data Unity 实时流（基于 protobuf）

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

pub mod compact;
pub mod data;
pub mod detail;
pub mod dump;
pub mod json;
pub mod pd3u;
pub mod raw;

/// Profiler 文件来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfilerFormat {
    Json,
    Raw,
    Data,
    Pd3u,
}

impl ProfilerFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "json" => Some(Self::Json),
            "raw" => Some(Self::Raw),
            "data" => Some(Self::Data),
            "pd3u" | "uppc" => Some(Self::Pd3u),
            _ => None,
        }
    }
}

/// 解析后的 Profiler 数据（统一内部表示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedProfile {
    /// Local, transient query source. Never serialized into a snapshot or export.
    #[serde(skip)]
    pub details: Option<std::sync::Arc<detail::FrameStore>>,
    pub meta: ProfileMeta,
    pub frames: Vec<Frame>,
    /// 解析过程中的警告（如 magic 不匹配、截断等）
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub file_name: String,
    pub format: ProfilerFormat,
    pub duration_ms: f64,
    pub frame_count: usize,
    pub platform: Option<String>,
    pub unity_version: Option<String>,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default)]
    pub quality: FrameQuality,
    pub index: usize,
    pub duration_ms: f64,
    pub cpu_ms: f64,
    pub gc_alloc_bytes: u64,
    pub draw_calls: u32,
    pub set_pass_calls: u32,
    #[serde(default)]
    pub render_counters: std::collections::BTreeMap<String, u64>,
    /// 主线程 inclusive 摘要；dump / 结构化 data 按帧和名称合并。
    /// 原始样本数是 call_count 之和，原始树通过 details 查询。
    pub main_thread_samples: Vec<Sample>,
    /// GC 分配站点；dump / 结构化 data 按帧、线程和归因名称合并。
    pub gc_alloc_sites: Vec<AllocSite>,
    /// 渲染事件
    pub render_events: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub name: String,
    pub total_ms: f64,
    pub call_count: u64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameQuality {
    pub duration: bool,
    pub cpu: bool,
    pub gc: bool,
    pub draw: bool,
    pub set_pass: bool,
    pub samples: bool,
    pub sites: bool,
    pub render: bool,
    pub estimated: bool,
    pub source: String,
    pub reasons: Vec<String>,
}
impl Default for FrameQuality {
    fn default() -> Self {
        Self::missing("unknown")
    }
}
impl FrameQuality {
    pub fn missing(source: &str) -> Self {
        Self {
            duration: false,
            cpu: false,
            gc: false,
            draw: false,
            set_pass: false,
            samples: false,
            sites: false,
            render: false,
            estimated: false,
            source: source.into(),
            reasons: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllocSite {
    pub name: String,
    pub thread: String,
    pub total_bytes: u64,
    pub call_count: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("不支持的文件格式: {0}")]
    UnsupportedFormat(String),

    #[error("文件读取失败: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("protobuf 解码失败: {0}")]
    Protobuf(String),

    #[error("二进制格式 magic 不匹配（可能文件损坏或格式未知）")]
    BadMagic,

    #[error("数据截断：{0}")]
    Truncated(String),

    #[error("其他错误: {0}")]
    Other(String),
}

impl From<prost::DecodeError> for ParseError {
    fn from(err: prost::DecodeError) -> Self {
        ParseError::Protobuf(err.to_string())
    }
}

/// 按扩展名自动分派
pub async fn parse_file(path: &Path) -> Result<ParsedProfile, ParseError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| ParseError::UnsupportedFormat("(无扩展名)".to_string()))?;

    let format = ProfilerFormat::from_extension(ext)
        .ok_or_else(|| ParseError::UnsupportedFormat(ext.to_string()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unknown)")
        .to_string();

    // `.data` 分块读取；完整结果仍累积在内存，容量上限尚未验证。
    let mut profile = match format {
        ProfilerFormat::Json => {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || json::parse_path(&path))
                .await
                .map_err(|e| ParseError::Other(e.to_string()))??
        }
        ProfilerFormat::Raw => {
            let bytes = Bytes::from(tokio::fs::read(path).await?);
            let file_size_bytes = bytes.len() as u64;
            raw::parse(&bytes, &file_name, file_size_bytes).await?
        }
        ProfilerFormat::Data => {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || data::parse_path(&path))
                .await
                .map_err(|e| ParseError::Other(e.to_string()))??
        }
        ProfilerFormat::Pd3u => {
            let bytes = Bytes::from(tokio::fs::read(path).await?);
            let file_size_bytes = bytes.len() as u64;
            pd3u::parse(&bytes, &file_name, file_size_bytes).await?
        }
    };

    // 通用元信息填充
    profile.meta.format = format;
    if profile.meta.file_name.is_empty() {
        profile.meta.file_name = file_name.clone();
    }
    if profile.meta.file_size_bytes == 0 {
        profile.meta.file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }

    Ok(profile)
}
