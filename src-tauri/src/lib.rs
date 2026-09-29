//! Tauri 入口：注册 commands、plugins、state

pub mod acp_client;
pub mod commands;
pub mod errors;
pub mod extractor;
pub mod mcp;
pub mod parser;
pub mod project;
pub mod reports;
pub mod source;
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
            commands::frame_details,
            commands::cpu_hierarchy,
            commands::frame_evidence,
            commands::compare_frames,
            commands::release_file,
            commands::diagnose,
            commands::cancel_diagnose,
            commands::list_agents,
            commands::list_reports,
            commands::export_reports,
            commands::render_report_markdown,
            commands::prepare_source,
            commands::cancel_source_preparation,
            commands::diagnose_source,
            commands::prepare_project,
            commands::project_editor_status,
            commands::diagnose_project,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
