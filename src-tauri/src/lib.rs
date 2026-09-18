//! Tauri 入口：注册 commands、plugins、state

pub mod acp_client;
pub mod commands;
pub mod errors;
pub mod extractor;
pub mod mcp;
pub mod parser;
pub mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::upload,
            commands::analyze,
            commands::diagnose,
            commands::cancel_diagnose,
            commands::list_agents,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}