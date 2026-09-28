//! MCP 工具实现（业务逻辑）
//!
//! 这层独立于 rmcp SDK 的 transport 细节，方便测试和复用。

use serde_json::{json, Value};

use super::MetricsStore;
use crate::extractor::MetricsSnapshot;

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum McpToolError {
    #[error(transparent)]
    Query(#[from] crate::parser::detail::QueryError),
    #[error("无已加载的 Profiler 数据")]
    NoSnapshot,
    #[error("帧 {0} 不存在")]
    FrameNotFound(usize),
    #[error("参数错误: {0}")]
    BadArg(String),
}

/// Session summary
pub async fn run_session_summary(store: &MetricsStore) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    let mut value =
        serde_json::to_value(&snapshot).map_err(|e| McpToolError::BadArg(e.to_string()))?;
    // Full timelines are delivered by performance_frames, not duplicated here.
    value["cpu"]
        .as_object_mut()
        .unwrap()
        .remove("frameTimeline");
    value["metricSemantics"] = json!({
        "percentiles": {
            "population": "仅该指标的有效帧；真实零值参与排序，缺失值不参与",
            "method": "按升序排序，取零起始索引 round((n-1)*q)，半整数向上取整，不插值",
            "smallSampleExample": {"values": [0,32], "p50":32,"p95":32,"p99":32,"max":32},
            "interpretation": "p50 不是平均值；偶数样本不取中间两数的平均，不能仅凭 p50 与部分帧数值不同判定数据矛盾"
        },
        "cpu": {
            "sampleTime": "inclusive：包含子样本；可信 dump/data 主路径的帧 CPU 取唯一主线程根样本，其他 JSON 显式值或估算必须结合 source 和 quality 解读",
            "exclusiveTime": "未提供已验证的 self/exclusive CPU 耗时；不能把父子样本相加或用热点列表相减来推断剩余、未解释或未采样 CPU 时间",
            "coverage": "调用树无分页/深度截断仅代表已导出样本读取完整，不证明所有运行工作都被 instrumentation 覆盖"
        },
        "gc": "每个 GC.Alloc 样本的字节独立计入（含嵌套分配）；按线程和最近非 GC.Alloc 父样本归因，和已校验的帧总量核对"
    });
    Ok(value)
}

/// Frame 时间序列
pub async fn run_frames(
    store: &MetricsStore,
    start: usize,
    limit: usize,
) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    if limit == 0 || limit > 500 {
        return Err(McpToolError::BadArg("limit 必须在 1..=500".into()));
    }
    let timeline = &snapshot.cpu.frame_timeline;
    if start > timeline.len() {
        return Err(McpToolError::BadArg("start 超出时间线范围".into()));
    }
    let end = start.saturating_add(limit).min(timeline.len());
    let slice = &timeline[start..end];

    Ok(json!({
        "start": start,
        "limit": limit,
        "returned": slice.len(),
        "frames": slice,
    }))
}

/// 单帧查询
pub async fn run_frame(store: &MetricsStore, frame_index: usize) -> Result<Value, McpToolError> {
    run_frame_page(store, frame_index, 0, 128).await
}

pub async fn run_frame_page(
    store: &MetricsStore,
    frame_index: usize,
    start: usize,
    limit: usize,
) -> Result<Value, McpToolError> {
    let source = store.query_source().await?;
    let frame = tokio::task::spawn_blocking(move || source.frame(frame_index, start, limit))
        .await
        .map_err(|e| McpToolError::BadArg(e.to_string()))??;
    serde_json::to_value(frame).map_err(|e| McpToolError::BadArg(e.to_string()))
}

/// 原始单帧调用树的有界分页；不再使用全局热点。
pub async fn run_cpu_hierarchy(
    store: &MetricsStore,
    frame_index: usize,
    max_depth: usize,
) -> Result<Value, McpToolError> {
    run_cpu_hierarchy_page(store, frame_index, None, 0, 200, max_depth).await
}

pub async fn run_cpu_hierarchy_page(
    store: &MetricsStore,
    frame_index: usize,
    thread_index: Option<usize>,
    start: usize,
    limit: usize,
    max_depth: usize,
) -> Result<Value, McpToolError> {
    let source = store.query_source().await?;
    let page = tokio::task::spawn_blocking(move || {
        source.hierarchy(frame_index, thread_index, start, limit, max_depth)
    })
    .await
    .map_err(|e| McpToolError::BadArg(e.to_string()))??;
    let mut warnings = Vec::new();
    if page.depth_truncated {
        warnings.push("depthTruncated=true：当前深度隐藏了样本，可能包括嵌套 GC.Alloc。增大 max_depth 并从 start=0 重新查询；若仍截断，明确说明未完整归因，不得用可见样本之和替代帧或线程 GC 总量。");
    }
    if page.next_start.is_some() {
        warnings.push("nextStart 非空：还有后续样本页。使用 nextStart 续查；当前页的分配不是线程或帧 GC 总量。");
    }
    let mut value = serde_json::to_value(page).map_err(|e| McpToolError::BadArg(e.to_string()))?;
    value["queryWarnings"] = json!(warnings);
    Ok(value)
}

/// 综合分析
pub async fn run_analysis(store: &MetricsStore, focus: &str) -> Result<Value, McpToolError> {
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    Ok(build_analysis(&snapshot, focus))
}

fn build_analysis(snapshot: &MetricsSnapshot, focus: &str) -> Value {
    let mut issues = Vec::new();
    for (area, stats, budget) in [
        ("cpu", &snapshot.cpu.main_thread_ms, 16.67),
        (
            "gc",
            &snapshot.gc.alloc_per_frame_bytes,
            4.0 * 1024.0 * 1024.0,
        ),
        ("rendering", &snapshot.rendering.draw_calls, 1500.0),
    ] {
        if (focus == area || focus == "all") && stats.quality.can_diagnose() {
            if let Some(p95) = stats.p95.filter(|v| *v > budget) {
                issues.push(json!({"area":area,"severity":"medium","p95":p95,"budget":budget,"source":stats.quality.source}));
            }
        }
    }
    json!({"issues":issues,"quality":{"cpu":snapshot.cpu.main_thread_ms.quality,"gc":snapshot.gc.alloc_per_frame_bytes.quality,"rendering":snapshot.rendering.draw_calls.quality},"warnings":snapshot.warnings})
}
