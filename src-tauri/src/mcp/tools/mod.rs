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
    match name {
        "performance_session_summary" => {
            let _: Empty = args(arguments)?;
            run_session_summary(store).await
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
