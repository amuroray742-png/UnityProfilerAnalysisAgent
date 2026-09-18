//! ACP 兼容 Agent 预设
//!
//! 用户可配置任意 ACP 兼容 Agent：
//! - Claude Code ACP（`claude-code-acp`）
//! - Google Gemini CLI（`gemini --experimental-acp`）
//! - 其他 ACP Registry 中的 Agent

use serde::{Deserialize, Serialize};

/// 单个 Agent 预设
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPreset {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub description: String,
    /// 启动时检测 command 是否可用
    pub available: bool,
}

/// 内置预设
pub fn builtin_presets() -> Vec<AgentPreset> {
    vec![
        AgentPreset {
            id: "claude-code".to_string(),
            label: "Claude Code".to_string(),
            command: "claude-code-acp".to_string(),
            args: vec![],
            description: "Claude Code 的 ACP 适配器（订阅 / API key 都可）".to_string(),
            available: false,
        },
        AgentPreset {
            id: "gemini".to_string(),
            label: "Gemini CLI".to_string(),
            command: "gemini".to_string(),
            args: vec!["--experimental-acp".to_string()],
            description: "Google Gemini CLI 的实验性 ACP 模式".to_string(),
            available: false,
        },
        AgentPreset {
            id: "codex".to_string(),
            label: "Codex CLI".to_string(),
            command: "codex-acp".to_string(),
            args: vec![],
            description: "OpenAI Codex CLI（ACP 适配器）".to_string(),
            available: false,
        },
    ]
}

/// 检测 command 是否在 PATH 中
pub fn probe_available(command: &str) -> bool {
    let command_name = command.split_whitespace().next().unwrap_or(command);
    which(command_name).is_some()
}

/// 把 (command, args) 解析成实际可 spawn 的 (program, args)。
///
/// 处理 Windows 上 npm 全局装的 PowerShell shim（`.ps1`）：
/// `CreateProcessW` 无法直接执行 `.ps1` 文件（默认关联到 Notepad），
/// 必须通过 `powershell -File` 间接启动。
/// `.cmd` / `.bat` 走 `cmd /C`。
pub fn resolve_command(
    command: &str,
    args: &[String],
) -> Option<(String, Vec<String>)> {
    let path = which(command)?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    match ext.as_deref() {
        Some("ps1") | Some("psm1") => {
            let mut wrapped = vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                path.to_string_lossy().into_owned(),
            ];
            wrapped.extend_from_slice(args);
            Some(("powershell".to_string(), wrapped))
        }
        Some("cmd") | Some("bat") => {
            let mut wrapped = vec!["/C".to_string(), path.to_string_lossy().into_owned()];
            wrapped.extend_from_slice(args);
            Some(("cmd".to_string(), wrapped))
        }
        _ => Some((path.to_string_lossy().into_owned(), args.to_vec())),
    }
}

/// 简易 `which` 实现（Windows 兼容 PATHEXT 所有后缀，包括 .ps1 shim）
fn which(cmd: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| {
                ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WS;.WSF;.WSC;.WSH;.MSC;.PS1;.PSM1"
                    .to_string()
            })
            .split(';')
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![String::new()]
    };

    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{}{}", cmd, ext));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        let direct = dir.join(cmd);
        if direct.is_file() {
            return Some(direct);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// `std::env::set_var` 不是线程安全的，多个测试并发跑会相互污染。
    /// 这里用一个全局 Mutex 串行化所有会改 env 的用例。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn probe_returns_false_for_missing() {
        assert!(!probe_available("definitely-not-a-real-command-12345"));
    }

    #[test]
    fn builtin_presets_has_three() {
        let p = builtin_presets();
        assert!(p.len() >= 2);
        assert!(p.iter().any(|p| p.id == "claude-code"));
        assert!(p.iter().any(|p| p.id == "gemini"));
    }

    /// 用 RAII 守护临时目录：测试结束自动清理；env var 还原
    struct TestEnv {
        tmp: std::path::PathBuf,
        original_path: std::ffi::OsString,
        original_pathext: Option<std::ffi::OsString>,
    }

    impl TestEnv {
        fn new(name: &str, pathext: &str) -> Self {
            let tmp = std::env::temp_dir().join(format!(
                "test-probe-{}-{}",
                name,
                std::process::id()
            ));
            let _ = fs::create_dir_all(&tmp);
            let original_path = std::env::var_os("PATH").unwrap_or_default();
            let original_pathext = std::env::var_os("PATHEXT");

            // PATH 临时目录优先
            let mut new_path = tmp.clone().into_os_string();
            new_path.push(";");
            new_path.push(&original_path);
            std::env::set_var("PATH", &new_path);
            // PATHEXT 用测试自带的（覆盖父进程的设置，保证 .ps1 也能命中）
            std::env::set_var("PATHEXT", pathext);

            Self {
                tmp,
                original_path,
                original_pathext,
            }
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            std::env::set_var("PATH", &self.original_path);
            match &self.original_pathext {
                Some(v) => std::env::set_var("PATHEXT", v),
                None => std::env::remove_var("PATHEXT"),
            }
            let _ = fs::remove_dir_all(&self.tmp);
        }
    }

    #[test]
    fn resolve_command_wraps_ps1_with_powershell() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let env = TestEnv::new("ps1", ".COM;.EXE;.BAT;.CMD;.PS1");
        fs::write(env.tmp.join("fake-agent-ps1.ps1"), "# fake\n").unwrap();

        let (program, args) =
            resolve_command("fake-agent-ps1", &["--flag".to_string()]).expect("should resolve");

        assert_eq!(program, "powershell");
        assert!(args.windows(2).any(|w| w[0] == "-NoProfile"), "missing -NoProfile");
        assert!(
            args.windows(2).any(|w| w[0] == "-ExecutionPolicy" && w[1] == "Bypass"),
            "missing -ExecutionPolicy Bypass"
        );
        assert!(args.windows(2).any(|w| w[0] == "-File"), "missing -File");
        assert_eq!(args.last().map(String::as_str), Some("--flag"));
    }

    #[test]
    fn resolve_command_wraps_cmd_with_cmd_exe() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let env = TestEnv::new("cmd", ".COM;.EXE;.BAT;.CMD");
        fs::write(env.tmp.join("fake-agent-cmd.cmd"), "@echo off\n").unwrap();

        let (program, args) = resolve_command("fake-agent-cmd", &[]).expect("should resolve");

        assert_eq!(program, "cmd");
        assert_eq!(args[0], "/C");
    }

    #[test]
    fn resolve_command_returns_none_for_missing() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let env = TestEnv::new("missing", ".COM;.EXE;.BAT;.CMD;.PS1");
        let result = resolve_command("completely-fake-zzz-9999", &[]);
        assert!(result.is_none());
    }
}