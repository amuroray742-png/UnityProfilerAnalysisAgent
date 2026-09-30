//! Automatic runs can investigate and edit code, never arbitrary project files.
use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path},
};

pub fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let rel = Path::new(relative);
    if relative.contains(['\\', ':'])
        || relative
            .split('/')
            .any(|p| p.is_empty() || p.starts_with('.') || p.ends_with(['.', ' ']))
        || rel.components().any(|p| !matches!(p, Component::Normal(_)))
        || !relative.starts_with("Assets/")
    {
        return Err("新增路径必须位于 Assets 内，不能是链接或特殊路径".into());
    }
    let mut path = root.to_path_buf();
    if crate::project::files::linked(&fs::symlink_metadata(&path).map_err(|e| e.to_string())?) {
        return Err("工程根目录是链接".into());
    }
    for part in rel.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(m) if crate::project::files::linked(&m) => {
                return Err("路径包含链接/junction".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}
// Keep every existing ancestor in place while committing on Windows.
pub fn lock_parents(root: &Path, relative: &str) -> Result<Vec<fs::File>, String> {
    checked_path(root, relative)?;
    let mut result = vec![];
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let mut path = root.to_path_buf();
        for part in Path::new(relative).components() {
            let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if crate::project::files::linked(&m) || !m.is_dir() {
                return Err("父目录已变化".into());
            }
            result.push(
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x02000000)
                    .open(&path)
                    .map_err(|e| e.to_string())?,
            );
            path.push(part);
        }
    }
    Ok(result)
}
pub fn record_bytes(root: &Path, c: &Change) -> Result<Vec<u8>, String> {
    if c.kind == "directory" {
        let path = checked_path(root, &c.path)?;
        if !path.is_dir() {
            return Err("目录不存在".into());
        }
        return Ok(vec![]);
    }
    let mut bytes = vec![];
    crate::project::files::open(root, &c.path)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("文件超过上限".into());
    }
    Ok(bytes)
}
fn meta(path: &str, directory: bool) -> Vec<u8> {
    let importer = if directory {
        "DefaultImporter"
    } else {
        match Path::new(path).extension().and_then(|s| s.to_str()) {
            Some("cs") => "MonoImporter",
            Some("shader") => "ShaderImporter",
            Some("compute") => "ComputeShaderImporter",
            _ => "ShaderIncludeImporter",
        }
    };
    let fields = if importer == "MonoImporter" {
        "  serializedVersion: 2\n  defaultReferences: []\n  executionOrder: 0\n  icon: {instanceID: 0}\n"
    } else {
        ""
    };
    format!("fileFormatVersion: 2\nguid: {}\n{}{}:\n  externalObjects: {{}}\n{}  userData: \n  assetBundleName: \n  assetBundleVariant: \n", uuid::Uuid::new_v4().simple(), if directory {"folderAsset: yes\n"}else{""}, importer, fields).into_bytes()
}
fn created(path: String, bytes: Vec<u8>, kind: &str) -> Change {
    Change {
        task_id: None,
        kind: kind.into(),
        path,
        before_hash: storage::hash(&[]),
        after_hash: storage::hash(&bytes),
        before: vec![],
        after: bytes,
        state: "prepared".into(),
    }
}
impl Workspace {
    pub fn automatic_query(&self, run_id: &str, name: &str, args: Value) -> Result<Value, String> {
        self.check()?;
        let mut data = self.data.lock().unwrap();
        let root = data.root.clone();
        let run = data
            .rounds
            .iter_mut()
            .flat_map(|r| &mut r.runs)
            .find(|r| r.id == run_id)
            .ok_or("运行不存在")?;
        if !run.automatic || run.status != "running" {
            return Err("当前会话没有自动优化权限".into());
        }
        if name == "optimization_task" {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Request {
                id: String,
                kind: String,
                title: String,
                evidence: String,
                instructions: String,
                acceptance: String,
            }
            let task: Request = serde_json::from_value(args).map_err(|e| e.to_string())?;
            if !["optimize", "marker", "investigate"].contains(&task.kind.as_str())
                || [
                    &task.id,
                    &task.title,
                    &task.evidence,
                    &task.instructions,
                    &task.acceptance,
                ]
                .iter()
                .any(|v| v.trim().is_empty())
            {
                return Err("任务需要目标、证据和验收方式".into());
            }
            if [
                &task.id,
                &task.title,
                &task.evidence,
                &task.instructions,
                &task.acceptance,
            ]
            .iter()
            .map(|v| v.len())
            .sum::<usize>()
                > 16000
            {
                return Err("任务过大".into());
            }
            let next = Task {
                id: task.id.clone(),
                kind: task.kind.clone(),
                title: task.title,
                evidence: task.evidence,
                files: BTreeMap::new(),
                instructions: task.instructions,
                acceptance: task.acceptance,
                constraints: run.requirements.clone(),
                selected: task.kind != "investigate",
            };
            if let Some(old) = run.tasks.iter_mut().find(|t| t.id == task.id) {
                *old = next;
            } else {
                if run.tasks.len() >= 20 {
                    return Err("本轮最多 20 项内部任务".into());
                }
                run.tasks.push(next);
            }
            self.save(&data)?;
            return Ok(json!({"recorded":true,"taskId":task.id}));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Create {
            task_id: String,
            path: String,
            content: String,
        }
        let request: Create = serde_json::from_value(args).map_err(|e| e.to_string())?;
        if run.checks.len() >= 3 {
            return Err("已达检查上限".into());
        }
        if !run
            .tasks
            .iter()
            .any(|t| t.id == request.task_id && ["optimize", "marker"].contains(&t.kind.as_str()))
        {
            return Err("请先记录有依据的优化或 Marker 任务".into());
        }
        if !editing::allowed(&request.path)
            || request.content.is_empty()
            || request.content.len() > 2 * 1024 * 1024
        {
            return Err("仅允许新增不超过 2 MiB 的代码文件".into());
        }
        let path = checked_path(&root, &request.path)?;
        if fs::symlink_metadata(&path).is_ok()
            || fs::symlink_metadata(path.with_file_name(format!(
                "{}.meta",
                path.file_name().unwrap().to_string_lossy()
            )))
            .is_ok()
        {
            return Err("目标或 meta 已存在，禁止覆盖".into());
        }
        let mut additions = vec![];
        let parts: Vec<_> = request.path.split('/').collect();
        for n in 2..parts.len() {
            let relative = parts[..n].join("/");
            let dir = checked_path(&root, &relative)?;
            if !dir.exists() {
                if root.join(format!("{relative}.meta")).exists() {
                    return Err("新目录 meta 已存在".into());
                }
                additions.push(created(
                    format!("{relative}.meta"),
                    meta(&relative, true),
                    "create",
                ));
                additions.push(created(relative, vec![], "directory"));
            } else if !dir.is_dir() {
                return Err("父路径不是目录".into());
            }
        }
        additions.push(created(
            format!("{}.meta", request.path),
            meta(&request.path, false),
            "create",
        ));
        additions.push(created(
            request.path.clone(),
            request.content.into_bytes(),
            "create",
        ));
        for c in &mut additions {
            c.task_id = Some(request.task_id.clone());
        }
        let count = additions.len();
        let start = run.changes.len();
        run.changes.extend(additions.clone());
        if let Err(e) = self.save(&data) {
            data.rounds
                .iter_mut()
                .flat_map(|r| &mut r.runs)
                .find(|r| r.id == run_id)
                .unwrap()
                .changes
                .truncate(start);
            return Err(e);
        }
        for (offset, c) in additions.iter().enumerate() {
            self.check()?;
            let _parents = lock_parents(&root, &c.path)?;
            let target = checked_path(&root, &c.path)?;
            let outcome = (|| -> Result<(), String> {
                if c.kind == "directory" {
                    fs::create_dir(target).map_err(|e| e.to_string())?;
                } else {
                    publish_new(&target, &c.after)?;
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                data.rounds
                    .iter_mut()
                    .flat_map(|r| &mut r.runs)
                    .find(|r| r.id == run_id)
                    .unwrap()
                    .changes[start + offset]
                    .state = "conflict".into();
                self.save(&data)?;
                return Err(format!("创建未完成，已保存冲突记录：{e}"));
            }
            data.rounds
                .iter_mut()
                .flat_map(|r| &mut r.runs)
                .find(|r| r.id == run_id)
                .unwrap()
                .changes[start + offset]
                .state = "applied".into();
            self.save(&data)?;
        }
        Ok(
            json!({"created":request.path,"records":count,"status":"代码已新增，尚需编译与重录验证"}),
        )
    }
}
pub fn rollback_created(root: &Path, c: &Change, _runs: &[Run]) -> Result<(), String> {
    if c.state == "conflict" {
        return Err(format!("{} 创建存在冲突，需要人工核查", c.path));
    }
    let path = checked_path(root, &c.path)?;
    if !path.exists() {
        return Ok(());
    }
    let _parents = lock_parents(root, &c.path)?;
    if !path.exists() {
        return Ok(());
    }
    if c.kind == "directory" {
        return fs::remove_dir(&path).map_err(|e| format!("新目录非空或发生变化，停止回退：{e}"));
    }
    let current = storage::hash(&record_bytes(root, c)?);
    if c.kind == "metadata" && current == c.before_hash {
        return Ok(());
    }
    if current != c.after_hash {
        return Err(format!("{} 有外部改动，停止回退", c.path));
    }
    if c.kind == "metadata" {
        storage::atomic(&path, &c.before)
    } else {
        fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// Conservative dependency scan before deleting new assets. Incomplete scans block deletion.
pub fn preflight_rollback(root: &Path, runs: &[Run]) -> Result<(), String> {
    let changes: Vec<_> = runs
        .iter()
        .flat_map(|r| &r.changes)
        .filter(|c| !["rolled_back", "not_applied"].contains(&c.state.as_str()))
        .collect();
    let created: Vec<_> = changes.iter().filter(|c| c.kind == "create").collect();
    if created.is_empty() {
        return Ok(());
    }
    let mut tokens = vec![];
    for c in created {
        if c.state == "conflict" {
            return Err(format!("{} 创建存在冲突", c.path));
        }
        if c.path.ends_with(".meta") {
            if let Some(guid) = std::str::from_utf8(&c.after)
                .ok()
                .and_then(|s| s.lines().find_map(|l| l.strip_prefix("guid: ")))
            {
                tokens.push(guid.to_owned());
            }
        } else if let Some(stem) = Path::new(&c.path).file_stem().and_then(|s| s.to_str()) {
            tokens.push(stem.to_owned());
            if c.path.ends_with(".cs") {
                let text = editing::decode(&c.after)?;
                let words: Vec<_> = text
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .filter(|s| !s.is_empty())
                    .collect();
                for pair in words.windows(2) {
                    if ["class", "struct", "interface", "enum", "record"].contains(&pair[0]) {
                        tokens.push(pair[1].into());
                    }
                }
            }
        }
    }
    let mut latest = BTreeMap::new();
    for c in &changes {
        latest.insert(c.path.clone(), *c);
    }
    for c in latest.values() {
        if c.kind != "directory" && root.join(&c.path).exists() && {
            let current = storage::hash(&record_bytes(root, c)?);
            current != c.after_hash && !(c.kind == "metadata" && current == c.before_hash)
        } {
            return Err(format!("{} 有外部改动，停止回退", c.path));
        }
    }
    let mut pending = vec![
        root.join("Assets"),
        root.join("Packages"),
        root.join("ProjectSettings"),
    ];
    let mut count = 0;
    while let Some(path) = pending.pop() {
        count += 1;
        if count > 200000 {
            return Err("引用检查超过上限，无法安全移除新文件".into());
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if crate::project::files::linked(&metadata) {
            return Err("引用检查遇到链接，覆盖不完整".into());
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if latest.contains_key(&relative) {
            continue;
        }
        if !matches!(
            path.extension().and_then(|s| s.to_str()),
            Some(
                "cs" | "shader"
                    | "hlsl"
                    | "cginc"
                    | "compute"
                    | "meta"
                    | "prefab"
                    | "unity"
                    | "asset"
                    | "mat"
                    | "json"
                    | "asmdef"
                    | "asmref"
            )
        ) {
            continue;
        }
        if metadata.len() > 64 * 1024 * 1024 {
            return Err("引用检查遇到超大文本，覆盖不完整".into());
        }
        let reader = BufReader::new(crate::project::files::open(root, &relative)?);
        for line in reader.lines() {
            let line = line.map_err(|_| format!("{relative} 无法读取，引用检查不完整"))?;
            if tokens.iter().any(|t| line.contains(t)) {
                return Err(format!(
                    "{relative} 可能引用本轮新增代码或资源，请先处理依赖后回退"
                ));
            }
        }
    }
    Ok(())
}

/// Only formatting and documented empty/default importer fields are accepted automatically.
/// A changed GUID, importer, reference or non-default setting remains an external conflict.
pub fn reconcile_meta(root: &Path, run: &mut Run) -> Result<Vec<String>, String> {
    fn canonical(bytes: &[u8]) -> Result<Vec<String>, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        Ok(text
            .lines()
            .map(str::trim_end)
            .filter(|l| {
                !l.is_empty()
                    && !matches!(
                        *l,
                        "  defaultTextures: []"
                            | "  nonModifiableTextures: []"
                            | "  preprocessorOverride: 0"
                    )
            })
            .map(str::to_owned)
            .collect())
    }
    let mut latest = BTreeMap::new();
    for c in &run.changes {
        if c.path.ends_with(".meta") && c.state == "applied" {
            latest.insert(c.path.clone(), c.clone());
        }
    }
    let mut normalized = vec![];
    for (path, c) in latest {
        let after = record_bytes(root, &c)?;
        if storage::hash(&after) == c.after_hash {
            continue;
        }
        if canonical(&after)? != canonical(&c.after)? {
            return Err(format!("{path} 导入设置或 GUID 发生未知变化，停止自动推进"));
        }
        run.changes.push(Change {
            task_id: c.task_id.clone(),
            kind: "metadata".into(),
            path: path.clone(),
            before_hash: c.after_hash,
            after_hash: storage::hash(&after),
            before: c.after,
            after,
            state: "applied".into(),
        });
        normalized.push(path);
    }
    Ok(normalized)
}

/// Publish complete bytes without ever replacing an existing target.
fn publish_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".upaa-new-{}.tmp", id()));
    let result = (|| -> Result<(), String> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let src: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    src.as_ptr(),
                    dst.as_ptr(),
                    0x8,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        #[cfg(not(windows))]
        {
            fs::hard_link(&temporary, path).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}
