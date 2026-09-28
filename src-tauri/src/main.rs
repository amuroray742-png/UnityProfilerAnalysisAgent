// Tauri 入口：禁止在 Windows 上显示控制台
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--mcp-bridge") {
        let result = args
            .get(2)
            .ok_or_else(|| "missing loopback address".into())
            .and_then(|address| unity_profiler_analysis_agent_lib::mcp::bridge::run_stdio(address));
        if let Err(error) = result {
            eprintln!("MCP bridge: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    unity_profiler_analysis_agent_lib::run()
}
