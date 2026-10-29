//! Typed tool dispatch shared by the wire server and protocol tests.
use super::{transport::*, MetricsStore};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frames {
    start: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    frame_index: usize,
    #[serde(default)]
    start: usize,
    #[serde(default = "thread_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hierarchy {
    frame_index: usize,
    #[serde(default)]
    thread_index: Option<usize>,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_depth")]
    max_depth: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Analysis {
    #[serde(default = "default_focus")]
    focus: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hotspots {
    #[serde(default)]
    names: Option<Vec<String>>,
    area: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    frame_index: usize,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
    #[serde(default)]
    counters_only: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Flows {
    frame_index: usize,
    end_frame_index: Option<usize>,
    flow_id: Option<u32>,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compare {
    frame_index: usize,
    baseline_frame_index: usize,
    #[serde(default)]
    thread_index: Option<usize>,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Memory {
    name: String,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sections {
    frame_index: usize,
    #[serde(default)]
    start: usize,
    #[serde(default = "hotspot_limit")]
    limit: usize,
}
fn hotspot_limit() -> usize {
    10
}
fn default_limit() -> usize {
    200
}
fn thread_limit() -> usize {
    128
}
fn default_depth() -> usize {
    3
}
fn default_focus() -> String {
    "all".into()
}
fn args<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, McpToolError> {
    serde_json::from_value(value).map_err(|e| McpToolError::BadArg(e.to_string()))
}

// Read the published bounds rather than maintaining a separate MCP limit table.
// Keep core validation as defense in depth for non-MCP callers.
fn validate_query_bounds(name: &str, arguments: &Value) -> Result<(), McpToolError> {
    static SCHEMAS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let schemas = SCHEMAS.get_or_init(super::list_tool_schemas);
    let Some(tool) = schemas["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
    else {
        return Ok(());
    };
    for field in ["limit", "max_depth"] {
        let rule = &tool["inputSchema"]["properties"][field];
        if let (Some(max), Some(value)) = (rule["maximum"].as_u64(), arguments.get(field)) {
            let min = rule["minimum"].as_u64().unwrap_or(0);
            if !value.as_u64().is_some_and(|n| n >= min && n <= max) {
                let hint = if field == "limit" {
                    "省略 limit 使用默认值，或改为范围内的值；保持本次 start，后续使用返回的 nextStart 分页。不会自动截断或扩大上限。"
                } else {
                    "改为范围内的深度；depthTruncated=true 时在上限内增加 max_depth 并从 start=0 重查。"
                };
                return Err(McpToolError::BadArg(format!(
                    "{name}.{field}={value} 无效：必须为 {min}..={max} 的整数，默认 {}。{hint}",
                    rule["default"]
                )));
            }
        }
    }
    if name == "performance_flow_events" {
        if let (Some(start), Some(end)) = (
            arguments["frame_index"].as_u64(),
            arguments["end_frame_index"].as_u64(),
        ) {
            if !end.checked_sub(start).is_some_and(|span| span < 8) {
                return Err(McpToolError::BadArg(format!("{name}.end_frame_index={end} 无效：必须 >= frame_index={start} 且差值 <= 7（含首尾最多 8 帧）；省略 end_frame_index 查询单帧，更大范围分窗口查询。")));
            }
        }
    }
    Ok(())
}
pub async fn dispatch(
    store: &MetricsStore,
    name: &str,
    arguments: Value,
) -> Result<Value, McpToolError> {
    validate_query_bounds(name, &arguments)?;
    if name == "performance_memory" {
        let a: Memory = args(arguments)?;
        return store.memory_page(&a.name, a.start, a.limit).await;
    }
    if name == "performance_frame_sections" {
        let a: Sections = args(arguments)?;
        let source = store.query_source().await?;
        return tokio::task::spawn_blocking(move || {
            source.sections(a.frame_index, a.start, a.limit)
        })
        .await
        .map_err(|e| McpToolError::BadArg(e.to_string()))?
        .map_err(Into::into);
    }
    if name.starts_with("optimization_") {
        let scope = store
            .modification()
            .await
            .ok_or_else(|| McpToolError::BadArg("未授权修改会话".into()))?;
        return scope
            .query(name, arguments)
            .await
            .map_err(McpToolError::BadArg);
    }
    if name.starts_with("project_") {
        let scope = store
            .project()
            .await
            .ok_or_else(|| McpToolError::BadArg("当前会话未授权工程范围".into()))?;
        return scope
            .query(name, arguments)
            .await
            .map_err(McpToolError::BadArg);
    }
    if name.starts_with("source_") {
        let scope = store
            .source()
            .await
            .ok_or_else(|| McpToolError::BadArg("当前会话未授权源码范围".into()))?;
        let name = name.to_owned();
        return tokio::task::spawn_blocking(move || scope.query(&name, arguments))
            .await
            .map_err(|e| McpToolError::BadArg(e.to_string()))?
            .map_err(McpToolError::BadArg);
    }
    match name {
        "performance_session_summary" => {
            let _: Empty = args(arguments)?;
            run_session_summary(store).await
        }
        "performance_metric_semantics" => {
            let _: Empty = args(arguments)?;
            Ok(metric_semantics())
        }
        "performance_hotspots" => {
            let a: Hotspots = args(arguments)?;
            if let Some(names)=a.names { run_target_hotspots(store, &a.area, &names, a.start, a.limit).await } else {run_hotspots(store, &a.area, a.start, a.limit).await}
        }
        "performance_frames" => {
            let a: Frames = args(arguments)?;
            run_frames(store, a.start, a.limit).await
        }
        "performance_frame" => {
            let a: Frame = args(arguments)?;
            run_frame_page(store, a.frame_index, a.start, a.limit).await
        }
        "performance_cpu_hierarchy" => {
            let a: Hierarchy = args(arguments)?;
            run_cpu_hierarchy_page(
                store,
                a.frame_index,
                a.thread_index,
                a.start,
                a.limit,
                a.max_depth,
            )
            .await
        }
        "performance_flow_events" => {
            let a: Flows = args(arguments)?;
            let source = store.query_source().await?;
            tokio::task::spawn_blocking(move || {
                source.flows(
                    a.frame_index,
                    a.end_frame_index.unwrap_or(a.frame_index),
                    a.flow_id,
                    a.start,
                    a.limit,
                )
            })
            .await
            .map_err(|e| McpToolError::BadArg(e.to_string()))?
            .map_err(McpToolError::from)
        }
        "performance_frame_evidence" => {
            let a: Evidence = args(arguments)?;
            let source = store.query_source().await?;
            tokio::task::spawn_blocking(move || {
                source.evidence(a.frame_index, a.start, a.limit, a.counters_only)
            })
            .await
            .map_err(|e| McpToolError::BadArg(e.to_string()))?
            .map_err(McpToolError::from)
        }
        "performance_compare_frames" => {
            let a: Compare = args(arguments)?;
            let source = store.query_source().await?;
            tokio::task::spawn_blocking(move || {
                source.compare(
                    a.frame_index,
                    a.baseline_frame_index,
                    a.thread_index,
                    a.start,
                    a.limit,
                )
            })
            .await
            .map_err(|e| McpToolError::BadArg(e.to_string()))?
            .map_err(McpToolError::from)
        }
        "performance_analysis" => {
            let a: Analysis = args(arguments)?;
            if !["cpu", "gc", "rendering", "all"].contains(&a.focus.as_str()) {
                return Err(McpToolError::BadArg("未知 focus".into()));
            }
            run_analysis(store, &a.focus).await
        }
        _ => Err(McpToolError::BadArg(format!("未知工具: {name}"))),
    }
}
