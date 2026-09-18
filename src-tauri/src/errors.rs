//! 统一错误类型

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 序列化: {0}")]
    Json(#[from] serde_json::Error),

    #[error("解析错误: {0}")]
    Parse(#[from] crate::parser::ParseError),

    #[error("ACP 错误: {0}")]
    Acp(#[from] crate::acp_client::AcpError),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("其他: {0}")]
    Other(String),
}