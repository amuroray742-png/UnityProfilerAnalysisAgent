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

    pub async fn memory_page(
        &self,
        name: &str,
        start: usize,
        limit: usize,
    ) -> Result<serde_json::Value, transport::McpToolError> {
        let inner = self.inner.lock().await;
        let snapshot = inner
            .snapshot
            .as_ref()
            .ok_or(transport::McpToolError::NoSnapshot)?;
        Ok(snapshot.memory.page(name, start, limit)?)
    }
    pub async fn get(&self) -> Option<MetricsSnapshot> {
        self.inner.lock().await.snapshot.clone()
    }
}

pub mod transport;
