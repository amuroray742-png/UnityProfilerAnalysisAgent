//! Desktop command discovery and macOS Finder launch environment.
use std::path::{Path, PathBuf};

pub fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn find_command(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 || path.is_absolute() {
        return executable(path).then(|| path.to_path_buf());
    }
    let directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let directories = {
        let mut directories = directories;
        directories.extend(mac_command_directories());
        directories
    };
    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| {
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WS;.WSF;.WSC;.WSH;.MSC;.PS1;.PSM1".into()
        })
        .split(';')
        .map(str::to_owned)
        .collect();
    #[cfg(not(windows))]
    let extensions: Vec<String> = vec![];
    for directory in directories {
        for extension in &extensions {
            let candidate = directory.join(format!("{command}{extension}"));
            if executable(&candidate) {
                return Some(candidate);
            }
        }
        let candidate = directory.join(command);
        if executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn mac_command_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = dirs::home_dir() {
        directories.extend([home.join(".unity/bin"), home.join(".cargo/bin")]);
    }
    directories
}

/// Run before Tauri starts any workers. MCP bridge subprocesses skip this entirely.
#[cfg(target_os = "macos")]
pub fn initialize_environment() {
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && executable(p))
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
        .and_then(|runtime| runtime.block_on(shell_path(&shell)));
    let path = match result {
        Ok(path) => path.into(),
        Err(_) => {
            eprintln!("无法读取登录 shell 的 PATH，使用当前环境和 macOS 常用命令目录");
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into())
        }
    };
    let mut directories: Vec<_> = std::env::split_paths(&path).collect();
    for directory in mac_command_directories() {
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    if let Ok(path) = std::env::join_paths(directories) {
        std::env::set_var("PATH", path);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn initialize_environment() {}

#[cfg(target_os = "macos")]
async fn shell_path(shell: &Path) -> Result<String, String> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(shell);
    command
        .args(["-ilc", "printf '\\036%s\\036' \"$PATH\""])
        .env("DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    if let Some(home) = dirs::home_dir() {
        command.current_dir(home);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let tree = crate::acp_client::client::ProcessTree::attach(&child).map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("missing shell stdout")?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        stdout
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 {
            return Err("shell output too large".into());
        }
        if !child.wait().await.map_err(|e| e.to_string())?.success() {
            return Err("shell failed".into());
        }
        parse_shell_path(&bytes)
    })
    .await;
    drop(tree);
    if !matches!(&result, Ok(Ok(_))) {
        let _ = child.kill().await;
    }
    result.map_err(|_| "shell timeout".to_owned())?
}

#[cfg(target_os = "macos")]
fn parse_shell_path(bytes: &[u8]) -> Result<String, String> {
    let mut parts = bytes.split(|b| *b == 0x1e);
    parts.next();
    let path = parts.next().ok_or("missing PATH delimiter")?;
    parts.next().ok_or("missing final PATH delimiter")?;
    let path = std::str::from_utf8(path).map_err(|_| "invalid PATH encoding")?;
    if path.is_empty() || path.contains(['\0', '\n', '\r']) {
        return Err("invalid PATH".into());
    }
    Ok(path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn unix_command_must_be_an_executable_file() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("upaa-command-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("中文 agent");
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(find_command(path.to_str().unwrap()).is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_command(path.to_str().unwrap()), Some(path));
        assert!(!executable(&directory));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn shell_banner_does_not_corrupt_path() {
        assert_eq!(
            parse_shell_path(b"welcome\n\x1e/opt/homebrew/bin:/usr/bin\x1egoodbye").unwrap(),
            "/opt/homebrew/bin:/usr/bin"
        );
        assert!(parse_shell_path(b"no delimiter").is_err());
        assert!(parse_shell_path(b"\x1e/usr/bin").is_err());
        assert!(parse_shell_path(b"\x1e\x1e").is_err());
        assert!(parse_shell_path(b"\x1e/usr/bin\nnoise\x1e").is_err());
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn shell_path_handles_startup_output_and_bounds_hanging_shells() {
        use std::{os::unix::fs::PermissionsExt, time::Duration};
        let directory = std::env::temp_dir().join(format!("upaa-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let shell = directory.join("shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf 'banner\\036/usr/bin:/bin\\036'\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(shell_path(&shell).await.unwrap(), "/usr/bin:/bin");
        std::fs::write(&shell, "#!/bin/sh\nsleep 30\n").unwrap();
        let start = std::time::Instant::now();
        assert!(shell_path(&shell).await.unwrap_err().contains("timeout"));
        assert!(start.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
