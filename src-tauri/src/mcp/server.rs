//! rmcp performs MCP initialization, request dispatch and cancellation framing.
use super::{list_tool_schemas, tools::dispatch, MetricsStore};
use rmcp::{
    model::*,
    service::{RequestContext, RoleServer},
    ErrorData, ServerHandler,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ToolAudit {
    pub tool: String,
    pub arguments: Value,
    pub is_error: bool,
}
#[derive(Clone)]
pub struct ProfilerServer {
    pub store: MetricsStore,
    pub audit: Option<mpsc::Sender<ToolAudit>>,
}
impl ServerHandler for ProfilerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo { capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation { name: "unity-profiler".into(), version: env!("CARGO_PKG_VERSION").into() },
            instructions: Some("只读 Profiler 查询。仅 available 指标可作确定性结论；partial 必须注明覆盖率。GC 单位字节，CPU inclusive 父子耗时不可相加。frame_index 是原始帧号。".into()),
            ..Default::default() }
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params("该工具列表没有续页", None));
        }
        let mut tools: Vec<Tool> = serde_json::from_value(list_tool_schemas()["tools"].clone())
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        if self.store.source().await.is_some() {
            let source_tools: Vec<Tool> =
                serde_json::from_value(crate::source::schemas()["tools"].clone())
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            tools.extend(source_tools);
        }
        if self.store.project().await.is_some() {
            let project_tools: Vec<Tool> =
                serde_json::from_value(crate::project::schemas()["tools"].clone())
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            tools.extend(project_tools);
        }
        for tool in &mut tools {
            tool.annotations = Some(serde_json::from_value(json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})).unwrap());
        }
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = dispatch(&self.store, &request.name, arguments.clone()).await;
        if let Some(tx) = &self.audit {
            let _ = tx.try_send(ToolAudit {
                tool: request.name.to_string(),
                arguments,
                is_error: result.is_err(),
            });
        }
        match result {
            Ok(value) => {
                let mut result = CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&value).expect("JSON value serializes"),
                )]);
                result.structured_content = Some(value);
                Ok(result)
            }
            Err(super::transport::McpToolError::BadArg(message)) => {
                Err(ErrorData::invalid_params(message, None))
            }
            Err(error) => Ok(CallToolResult::error(vec![Content::text(
                error.to_string(),
            )])),
        }
    }
}
