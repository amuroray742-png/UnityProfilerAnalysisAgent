use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::Ordering,
};
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn file_hash(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    crate::project::files::hash(&mut f, u64::MAX, cancel)
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_file_name(format!(".upaa-{}.tmp", id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        if path.exists() {
            let meta = fs::metadata(path)?;
            if meta.permissions().readonly() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "目标文件只读",
                ));
            }
            f.set_permissions(meta.permissions())?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let src: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // Same-volume replacement, flushed before replacing the existing name.
            for attempt in 0..7 {
                if unsafe {
                    windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                        src.as_ptr(),
                        dst.as_ptr(),
                        0x1 | 0x8,
                    )
                } != 0
                {
                    break;
                }
                let error = std::io::Error::last_os_error();
                // AV/indexers/readers may briefly hold a destination without delete sharing.
                // MoveFileEx also reports access-denied for a reader without delete sharing.
                // A real permissions error remains an error after this bounded retry.
                if attempt == 6 || !matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
                    return Err(error);
                }
                std::thread::sleep(std::time::Duration::from_millis(10 << attempt));
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|e| e.to_string())
}
fn lease(directory: &Path) -> Result<File, String> {
    let path = directory.join(".optimization.lock");
    if path.exists()
        && crate::project::files::linked(&fs::symlink_metadata(&path).map_err(|e| e.to_string())?)
    {
        return Err("锁文件不可为链接".into());
    }
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.try_lock()
        .map_err(|_| "该优化项目已由其他窗口或进程打开")?;
    Ok(f)
}
fn root_lease(root: &Path) -> Result<File, String> {
    let directory = std::env::temp_dir().join("upaa-optimization-locks");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let key = hash(root.to_string_lossy().to_lowercase().as_bytes());
    let path = directory.join(key);
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.try_lock()
        .map_err(|_| "另一个优化项目已打开此 Unity 工程，请先关闭")?;
    Ok(f)
}
impl Workspace {
    pub fn create(directory: PathBuf, root: PathBuf, name: String) -> Result<Self, String> {
        for path in [&root, &directory] {
            if crate::project::files::linked(
                &fs::symlink_metadata(path).map_err(|e| e.to_string())?,
            ) {
                return Err("工程或保存目录不可为链接/junction".into());
            }
        }
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for d in ["Assets", "Packages", "ProjectSettings"] {
            if !root.join(d).is_dir() {
                return Err("请选择 Unity 工程根目录".into());
            }
        }
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        if directory.starts_with(&root) {
            return Err("优化项目保存目录必须位于 Unity 工程之外".into());
        }
        let lease = lease(&directory)?;
        if directory.join("optimization.json").exists() {
            return Err("已有优化项目，请打开而非覆盖".into());
        }
        let root_lease = root_lease(&root)?;
        let data = Project {
            version: 2,
            id: id(),
            name,
            root,
            budgets: BTreeMap::new(),
            captures: vec![],
            rounds: vec![],
        };
        let s = Self {
            directory,
            _lease: lease,
            _root_lease: root_lease,
            data: Mutex::new(data),
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: AtomicBool::new(false),
            last_report_save: Mutex::new(std::time::Instant::now()),
        };
        s.save(&s.data.lock().unwrap())?;
        Ok(s)
    }
    pub fn open(directory: PathBuf) -> Result<Self, String> {
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        let lease = lease(&directory)?;
        let mut bytes = vec![];
        File::open(directory.join("optimization.json"))
            .map_err(|e| e.to_string())?
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("优化项目超过读取上限".into());
        }
        let mut data: Project = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        data.root = data.root.canonicalize().map_err(|e| e.to_string())?;
        let root_lease = root_lease(&data.root)?;
        if data.version != 1 && data.version != 2 {
            return Err("不支持的优化项目版本".into());
        }
        if data.version == 1 {
            let backup = directory.join("optimization.v1.backup.json");
            if !backup.exists() {
                let mut f = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&backup)
                    .map_err(|e| e.to_string())?;
                f.write_all(&bytes)
                    .and_then(|_| f.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            data.version = 2;
        }
        for round in &mut data.rounds {
            for run in &mut round.runs {
                for c in &mut run.changes {
                    if hash(&c.before) != c.before_hash || hash(&c.after) != c.after_hash {
                        return Err("变更备份校验失败".into());
                    }
                    if c.state == "prepared" {
                        match super::automatic::record_bytes(&data.root, c).map(|b| hash(&b)) {
                            Ok(h) if h == c.after_hash => c.state = "applied".into(),
                            Ok(h) if c.kind == "modify" && h == c.before_hash => {
                                c.state = "not_applied".into()
                            }
                            Err(_)
                                if c.kind != "modify"
                                    && super::automatic::checked_path(&data.root, &c.path)
                                        .is_ok_and(|p| {
                                            std::fs::symlink_metadata(p).is_err_and(|e| {
                                                e.kind() == std::io::ErrorKind::NotFound
                                            })
                                        }) =>
                            {
                                c.state = "not_applied".into()
                            }
                            _ => {
                                c.state = "conflict".into();
                                run.reason = Some("中断写入后文件变化，需人工核查".into());
                            }
                        }
                    }
                }
                for check in &mut run.checks {
                    if check["status"] == "running" {
                        check["status"] = serde_json::json!("unavailable");
                        check["reason"] = serde_json::json!("应用中断，未取得完整检查结果");
                    }
                }
                if run.status == "running" {
                    run.status = "interrupted".into();
                    run.reason = Some("应用意外退出；需检查已落盘文件后在新会话重试".into());
                }
            }
        }
        let s = Self {
            directory,
            _lease: lease,
            _root_lease: root_lease,
            data: Mutex::new(data),
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: AtomicBool::new(false),
            last_report_save: Mutex::new(std::time::Instant::now()),
        };
        s.save(&s.data.lock().unwrap())?;
        Ok(s)
    }
    pub fn save(&self, data: &Project) -> Result<(), String> {
        if crate::project::files::linked(
            &fs::symlink_metadata(&self.directory).map_err(|e| e.to_string())?,
        ) {
            return Err("保存目录不可是链接".into());
        }
        let bytes = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("项目数据超过 128 MiB，请新建优化项目；现有备份保留".into());
        }
        atomic(&self.directory.join("optimization.json"), &bytes)
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err("本轮已取消".into())
        } else {
            Ok(())
        }
    }
}
