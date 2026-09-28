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
    value["cpu"].as_object_mut().unwrap().remove("topHotspots");
    value["gc"].as_object_mut().unwrap().remove("topAllocSites");
    value["rendering"].as_object_mut().unwrap().remove("topRenderEvents");
    compact_descriptions(&mut value);
    value["hotspotCounts"] = json!({"cpu":snapshot.cpu.top_hotspots.len(),"gc":snapshot.gc.top_alloc_sites.len()});
    value["detailTools"] = json!({"hotspots":"performance_hotspots","semantics":"performance_metric_semantics","frames":"performance_frames"});
    value["metricSemantics"] = metric_semantics();
    Ok(value)
}

/// Keep frame-specific reason lists from overwhelming the diagnostic overview.
/// Full per-frame quality remains available from the detail tools.
fn compact_descriptions(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in ["reasons", "warnings"] {
                if let Some(Value::Array(rows)) = map.get_mut(key) {
                    let total = rows.len();
                    rows.truncate(5);
                    map.insert(format!("{key}Total"), json!(total));
                    map.insert(format!("{key}Truncated"), json!(total > 5));
                }
            }
            for item in map.values_mut() { compact_descriptions(item); }
        }
        Value::Array(rows) => for item in rows { compact_descriptions(item); },
        Value::String(text) if text.chars().count() > 256 => {
            *text = text.chars().take(256).collect::<String>() + "…[摘要省略，请查原始帧]";
        }
        _ => {}
    }
}

pub fn metric_semantics() -> Value {
    json!({
        "percentiles": {
            "population": "仅该指标的有效帧；真实零值参与排序，缺失值不参与",
            "method": "按升序排序，取零起始索引 round((n-1)*q)，半整数向上取整，不插值",
            "smallSampleExample": {"values": [0,32], "p50":32,"p95":32,"p99":32,"max":32},
            "frequency": "p99=max 或其他分位数相等不能推出只有一帧尖峰；异常帧数量必须依据逐帧值或 affectedFrames，不由分位数相等关系推断",
            "interpretation": "p50 不是平均值；偶数样本不取中间两数的平均，不能仅凭 p50 与部分帧数值不同判定数据矛盾"
        },
        "cpu": {
            "sampleTime": "inclusive：包含子样本；可信 dump/data 主路径的帧 CPU 取唯一主线程根样本，其他 JSON 显式值或估算必须结合 source 和 quality 解读",
            "exclusiveTime": "未提供已验证的 self/exclusive CPU 耗时；不能把父子样本相加或用热点列表相减来推断剩余、未解释或未采样 CPU 时间",
            "frameTimeDifference": "录制帧时间与主线程根样本耗时是不同观测口径；相减不能证明差额属于未采样 CPU、GPU、等待或任何具体工作。需要独立线程、时间区间和相关计数证据，不要给差额归因",
            "coverage": "调用树无分页/深度截断仅代表已导出样本读取完整，不证明所有运行工作都被 instrumentation 覆盖"
        },
        "gc": "每个 GC.Alloc 样本的字节独立计入（含嵌套分配）；按线程和最近非 GC.Alloc 父样本归因，和已校验的帧总量核对"
    })
}

/// Rankings are investigation candidates, not threshold-confirmed bottlenecks.
pub async fn run_hotspots(store: &MetricsStore, area: &str, start: usize, limit: usize) -> Result<Value, McpToolError> {
    if !["cpu", "gc"].contains(&area) || limit == 0 || limit > 50 {
        return Err(McpToolError::BadArg("area 必须为 cpu/gc，limit 必须为 1..=50".into()));
    }
    let snapshot = store.get().await.ok_or(McpToolError::NoSnapshot)?;
    let (total, quality) = if area == "cpu" {
        (snapshot.cpu.top_hotspots.len(), &snapshot.cpu.hotspot_quality)
    } else { (snapshot.gc.top_alloc_sites.len(), &snapshot.gc.site_quality) };
    if start > total { return Err(McpToolError::BadArg("start 超出热点列表".into())); }
    let end = start.saturating_add(limit).min(total);
    let rows = if area == "cpu" {
        json!(&snapshot.cpu.top_hotspots[start..end])
    } else { json!(&snapshot.gc.top_alloc_sites[start..end]) };
    let mut result = json!({"area":area,"start":start,"total":total,"returned":end-start,
        "nextStart":if end < total {Some(end)} else {None},"quality":quality,"rows":rows,
        "scope":if area == "cpu" {"主线程 marker，按累计 inclusive 毫秒降序；调用次数和单次最大耗时；父子与递归不可相加为总 CPU，须查帧树确认具体路径"} else {"已导出线程，按累计分配字节降序；按线程和最近非 GC.Alloc 父样本归因；分配不等于 GC 回收停顿"},
        "interpretation":"排名用于确定调查顺序，不证明瓶颈；partial 仅覆盖有效样本，estimated 不作确定性结论；marker 名不能单独证明源码实现"});
    compact_descriptions(&mut result["quality"]);
    Ok(result)
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
            let trigger_value = if area == "rendering" { stats.p95 } else { stats.max };
            if trigger_value.is_some_and(|v| v > budget) {
                let mut frames: Vec<_> = snapshot.cpu.frame_timeline.iter().filter_map(|frame| {
                    let value = match area {
                        "cpu" => frame.ms,
                        "gc" => frame.gc_alloc_bytes.map(|v| v as f64),
                        _ => None,
                    }?;
                    (value > budget).then_some((frame.frame_index, value))
                }).collect();
                frames.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let affected = if area == "rendering" { None } else { Some(frames.len()) };
                let evidence: Vec<_> = frames.iter().take(5).map(|(index, value)| {
                    json!({"frameIndex": index, "value": value})
                }).collect();
                issues.push(json!({
                    "area":area,"severity":"medium","kind":"threshold-exceeded",
                    "p95":stats.p95,"max":stats.max,"budget":budget,"source":stats.quality.source,
                    "unit":if area == "cpu" {"ms"} else if area == "gc" {"bytes"} else {"count"},
                    "trigger":if stats.p95.is_some_and(|v| v > budget) {"p95"} else {"isolated-peak"},
                    "affectedFrames":affected,"validFrames":stats.quality.valid_frames,
                    "evidenceFrames":evidence,"evidenceLimit":5,
                    "interpretation":"默认筛查阈值超限，需结合目标帧率、平台和原始样本核对；不能直接判定为已确认瓶颈"
                }));
            }
        }
    }
    let mut investigation_frames = Vec::new();
    for area in ["cpu", "gc"] {
        if focus != "all" && focus != area { continue; }
        let mut ranked: Vec<_> = snapshot.cpu.frame_timeline.iter().filter_map(|f| {
            let value = if area == "cpu" { f.ms } else { f.gc_alloc_bytes.map(|v| v as f64) }?;
            Some((f.frame_index, value))
        }).collect();
        ranked.sort_by(|a,b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        investigation_frames.push(json!({"area":area,"selection":"最高观测值候选，不等于超预算；质量见 quality",
            "frames":ranked.iter().take(3).map(|(id,v)|json!({"frameIndex":id,"value":v})).collect::<Vec<_>>() }));
    }
    let mut result = json!({"investigationFrames":investigation_frames,"issues":issues,"thresholdPolicy":{"source":"application-default-heuristic","userConfigured":false,"emptyIssuesMeaning":"未发现可用完整观测指标超过默认阈值，不等于没有性能问题；缺失、部分或估算指标不作确定性诊断"},"quality":{"cpu":snapshot.cpu.main_thread_ms.quality,"gc":snapshot.gc.alloc_per_frame_bytes.quality,"rendering":snapshot.rendering.draw_calls.quality},"warnings":snapshot.warnings});
    compact_descriptions(&mut result);
    result
}
