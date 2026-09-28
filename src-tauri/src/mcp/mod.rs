//! MCP 查询、会话数据和 stdio 桥。桌面诊断接线状态见项目台账。

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::extractor::MetricsSnapshot;

/// 共享给 MCP 工具的 metrics 数据
#[derive(Clone, Default)]
pub struct MetricsStore {
    inner: Arc<Mutex<StoreData>>,
}
#[derive(Default)]
struct StoreData {
    snapshot: Option<MetricsSnapshot>,
    details: Option<Arc<crate::parser::detail::FrameStore>>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set(&self, snapshot: MetricsSnapshot) {
        self.set_capture(snapshot, None).await;
    }

    pub async fn set_capture(
        &self,
        snapshot: MetricsSnapshot,
        details: Option<Arc<crate::parser::detail::FrameStore>>,
    ) {
        *self.inner.lock().await = StoreData {
            snapshot: Some(snapshot),
            details,
        };
    }
    pub async fn clear(&self) {
        *self.inner.lock().await = StoreData::default();
    }
    pub async fn query_source(
        &self,
    ) -> Result<Arc<crate::parser::detail::FrameStore>, transport::McpToolError> {
        let inner = self.inner.lock().await;
        if inner.snapshot.is_none() {
            return Err(transport::McpToolError::NoSnapshot);
        }
        inner.details.clone().ok_or_else(|| {
            transport::McpToolError::Query(crate::parser::detail::QueryError::Unavailable)
        })
    }

    pub async fn get(&self) -> Option<MetricsSnapshot> {
        self.inner.lock().await.snapshot.clone()
    }
}

/// tools/list 的实际 schema；与类型化参数分派保持一致。
pub fn list_tool_schemas() -> serde_json::Value {
    let mut schemas = json!({
        "tools": [
            {
                "name": "performance_session_summary",
                "description": "返回当前 Profiler 会话的总体指标摘要及 metricSemantics；解读分位数、inclusive CPU 和 GC 时必须遵守其统计与覆盖范围说明",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            },
            {
                "name": "performance_frames",
                "description": "列出帧范围（默认每批 ≤200 帧）",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer", "minimum": 0, "description": "时间线数组偏移，不是原始帧号" },
                        "limit": { "type": "integer", "default": 200, "minimum": 1, "maximum": 500 }
                    },
                    "required": ["start"]
                }
            },
            {
                "name": "performance_frame",
                "description": "按原始帧号查询指标和线程分页",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "frame_index": { "type": "integer", "minimum": 0 },
                        "start": { "type": "integer", "minimum": 0, "default": 0 },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 128, "default": 128 }
                    },
                    "required": ["frame_index"]
                }
            },
            {
                "name": "performance_cpu_hierarchy",
                "description": "查询原始帧/线程的前序样本分页，保留父索引；inclusive 父子耗时不可相加。必须检查 queryWarnings、depthTruncated 和 nextStart；截断或分页结果不能当作完整 GC 归因，线程列表也可能分页。GC 总量以已校验的帧/线程指标为准。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "frame_index": { "type": "integer", "minimum": 0 },
                        "thread_index": { "type": "integer", "minimum": 0, "description": "原始线程索引；省略时必须存在唯一 Main Thread" },
                        "start": { "type": "integer", "minimum": 0, "default": 0, "description": "原始样本索引；续页使用 nextStart" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 200 },
                        "max_depth": { "type": "integer", "minimum": 1, "maximum": 64, "default": 3, "description": "只返回 depth < max_depth 的样本；depthTruncated=true 时增大深度并从 start=0 重查。默认深度可能隐藏嵌套分配。" }
                    },
                    "required": ["frame_index"]
                }
            },
            {
                "name": "performance_analysis",
                "description": "返回默认阈值筛查与最多 5 个原始帧证据；区分 P95 超限和孤立峰值。阈值不是用户预算，不能直接确认瓶颈，空 issues 不等于无问题",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "focus": { "type": "string", "enum": ["cpu", "gc", "rendering", "all"], "default": "all" }
                    }
                }
            }
        ]
    });
    for tool in schemas["tools"].as_array_mut().unwrap() {
        tool["inputSchema"]["additionalProperties"] = json!(false);
    }
    schemas
}

pub mod bridge;
pub mod server;
pub mod tools;
pub mod transport;
