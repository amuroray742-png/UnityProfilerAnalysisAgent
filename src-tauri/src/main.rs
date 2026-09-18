// Tauri 入口：禁止在 Windows 上显示控制台
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    unity_profiler_analysis_agent_lib::run()
}