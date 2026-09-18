//! MCP 工具实现（业务逻辑）
//!
//! 这层独立于 rmcp SDK 的 transport 细节，方便测试和复用。

use serde_json::{json, Value};

use super::MetricsStore;
use crate::extractor::MetricsSnapshot;

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum McpToolError {
    #[error("无已加载的 Profiler 数据")]
    NoSnapshot,
    #[error("帧 {0} 不存在")]
    FrameNotFound(usize),
    #[error("参数错误: {0}")]
    BadArg(String),
}

/// Session summary
pub async fn run_session_summary(store: &MetricsStore) -> Result<Value, McpToolError> {
    let snapshot = store
        .get()
        .await
        .ok_or(McpToolError::NoSnapshot)?;
    Ok(serde_json::to_value(&snapshot).unwrap_or_else(|_| json!({})))
}

/// Frame 时间序列
pub async fn run_frames(
    store: &MetricsStore,
    start: usize,
    limit: usize,
) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    let limit = limit.min(500);
    let timeline = &snapshot.cpu.frame_timeline;
    let end = (start + limit).min(timeline.len());
    let slice = &timeline[start..end];

    Ok(json!({
        "start": start,
        "limit": limit,
        "returned": slice.len(),
        "frames": slice,
    }))
}

/// 单帧查询
pub async fn run_frame(
    store: &MetricsStore,
    frame_index: usize,
) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    let frame = snapshot
        .cpu
        .frame_timeline
        .iter()
        .find(|f| f.frame_index == frame_index)
        .ok_or(McpToolError::FrameNotFound(frame_index))?;
    Ok(json!({
        "frame_index": frame.frame_index,
        "ms": frame.ms,
        "meta": snapshot.meta,
    }))
}

/// CPU 层级（简化）
pub async fn run_cpu_hierarchy(
    store: &MetricsStore,
    _frame_index: usize,
    max_depth: usize,
) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    let hotspots: Vec<&_> = snapshot
        .cpu
        .top_hotspots
        .iter()
        .take(max_depth * 5)
        .collect();
    Ok(serde_json::to_value(&hotspots).unwrap_or_else(|_| json!([])))
}

/// 综合分析
pub async fn run_analysis(
    store: &MetricsStore,
    focus: &str,
) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    Ok(build_analysis(&snapshot, focus))
}

fn build_analysis(snapshot: &MetricsSnapshot, focus: &str) -> Value {
    let mut issues = Vec::new();

    if focus == "cpu" || focus == "all" {
        if snapshot.cpu.main_thread_ms.p95 > 16.67 {
            issues.push(json!({
                "severity": if snapshot.cpu.main_thread_ms.p95 > 33.33 { "high" } else { "medium" },
                "area": "cpu",
                "summary": format!("主线程 p95 = {:.2} ms，超出 60FPS 预算", snapshot.cpu.main_thread_ms.p95),
                "top_hotspots": snapshot.cpu.top_hotspots.iter().take(5).collect::<Vec<_>>(),
            }));
        }
    }

    if focus == "gc" || focus == "all" {
        if snapshot.gc.alloc_per_frame_bytes.p95 > 4.0 * 1024.0 * 1024.0 {
            issues.push(json!({
                "severity": "high",
                "area": "gc",
                "summary": format!("每帧 GC 分配 p95 = {:.2} MB", snapshot.gc.alloc_per_frame_bytes.p95 / 1024.0 / 1024.0),
                "top_sites": snapshot.gc.top_alloc_sites.iter().take(5).collect::<Vec<_>>(),
            }));
        }
    }

    if focus == "rendering" || focus == "all" {
        if snapshot.rendering.draw_calls.p95 > 1500.0 {
            issues.push(json!({
                "severity": if snapshot.rendering.draw_calls.p95 > 3000.0 { "high" } else { "medium" },
                "area": "rendering",
                "summary": format!("Draw Call p95 = {:.0}", snapshot.rendering.draw_calls.p95),
                "top_events": snapshot.rendering.top_render_events.iter().take(5).collect::<Vec<_>>(),
            }));
        }
    }

    json!({ "issues": issues })
}