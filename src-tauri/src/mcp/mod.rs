//! 内置 MCP Server（占位）
//!
//! ⚠️ 实现状态：MVP 桩（API 占位编译通过）
//!
//! `rmcp` v0.5 API 仍在演变，本模块先把骨架搭起来。
//! 未来 fork / SDK 稳定后，这里替换为真实的 rmcp Server 实现。

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::extractor::MetricsSnapshot;

/// 共享给 MCP 工具的 metrics 数据
#[derive(Clone, Default)]
pub struct MetricsStore {
    pub snapshot: Arc<Mutex<Option<MetricsSnapshot>>>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set(&self, snapshot: MetricsSnapshot) {
        *self.snapshot.lock().await = Some(snapshot);
    }

    pub async fn get(&self) -> Option<MetricsSnapshot> {
        self.snapshot.lock().await.clone()
    }
}

/// 列出可用工具的 schema（静态描述，供 ACP session/new.mcpServers 注入）
pub fn list_tool_schemas() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "performance_session_summary",
                "description": "返回当前 Profiler 会话的总体指标摘要",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            },
            {
                "name": "performance_frames",
                "description": "列出帧范围（默认每批 ≤200 帧）",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer", "description": "起始帧 index" },
                        "limit": { "type": "integer", "default": 200, "maximum": 500 }
                    },
                    "required": ["start"]
                }
            },
            {
                "name": "performance_frame",
                "description": "查询单帧的完整指标",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "frame_index": { "type": "integer" }
                    },
                    "required": ["frame_index"]
                }
            },
            {
                "name": "performance_cpu_hierarchy",
                "description": "查询某帧的 CPU 调用层级（按耗时聚合）",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "frame_index": { "type": "integer" },
                        "max_depth": { "type": "integer", "default": 3 }
                    },
                    "required": ["frame_index"]
                }
            },
            {
                "name": "performance_analysis",
                "description": "返回已计算的性能分析建议",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "focus": { "type": "string", "enum": ["cpu", "gc", "rendering", "all"], "default": "all" }
                    }
                }
            }
        ]
    })
}

pub mod tools;
pub mod transport;