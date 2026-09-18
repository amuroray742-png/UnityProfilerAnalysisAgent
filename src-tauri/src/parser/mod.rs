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

pub mod data;
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
    pub index: usize,
    pub duration_ms: f64,
    pub cpu_ms: f64,
    pub gc_alloc_bytes: u64,
    pub draw_calls: u32,
    pub set_pass_calls: u32,
    /// 主线程采样（按耗时聚合）
    pub main_thread_samples: Vec<Sample>,
    /// GC 分配站点（按字节聚合）
    pub gc_alloc_sites: Vec<Sample>,
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

    // `.data` 文件可能很大（GB 级），走流式路径避免 OOM
    let mut profile = match format {
        ProfilerFormat::Json => {
            let bytes = Bytes::from(tokio::fs::read(path).await?);
            let file_size_bytes = bytes.len() as u64;
            json::parse(&bytes, &file_name, file_size_bytes).await?
        }
        ProfilerFormat::Raw => {
            let bytes = Bytes::from(tokio::fs::read(path).await?);
            let file_size_bytes = bytes.len() as u64;
            raw::parse(&bytes, &file_name, file_size_bytes).await?
        }
        ProfilerFormat::Data => data::parse_path(path)?,
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