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
pub async fn dispatch(
    store: &MetricsStore,
    name: &str,
    arguments: Value,
) -> Result<Value, McpToolError> {
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
            run_hotspots(store, &a.area, a.start, a.limit).await
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
