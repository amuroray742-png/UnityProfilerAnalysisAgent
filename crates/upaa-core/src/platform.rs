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
