//! User-only, project-bound embedded plugin installation. No MCP entry point.
use super::{storage, workflow, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
const PACKAGE: &str = "com.upaa.inspector";
const TARGET: &str = "Packages/com.upaa.inspector";
const RECORD: &str = "plugin-install.json";
const FILES: &[(&str, &[u8], &str)] = include!(concat!(env!("OUT_DIR"), "/plugin_bundle.rs"));
const MAX: u64 = 4 * 1024 * 1024;
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRecord {
    project_id: String,
    root: PathBuf,
    id: String,
    bundle_hash: String,
    before: Vec<u8>,
    after: Vec<u8>,
    status: String,
    reason: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub state: String,
    pub reason: String,
    pub version: String,
    pub bundle_hash: String,
    pub lock_warning: Option<String>,
    pub record: Option<Value>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn bundle() -> Result<(String, String), String> {
    if FILES.is_empty() {
        return Err("程序内插件资源缺失，请重新安装工具".into());
    }
    let mut fingerprints = vec![];
    for (name, bytes, hash) in FILES {
        if storage::hash(bytes) != *hash {
            return Err("程序内插件资源校验失败".into());
        }
        fingerprints.push(format!("{name}:{hash}"));
    }
    let p: Value = serde_json::from_slice(
        FILES
            .iter()
            .find(|f| f.0 == "package.json")
            .ok_or("缺少插件清单")?
            .1,
    )
    .map_err(err)?;
    if p["name"] != PACKAGE {
        return Err("插件身份错误".into());
    }
    Ok((
        p["version"].as_str().ok_or("插件版本缺失")?.into(),
        storage::hash(fingerprints.join("\n").as_bytes()),
    ))
}
fn safe(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if crate::project::files::linked(&m) => {
                return Err(format!("拒绝链接或 junction：{}", p.display()))
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    safe(path)?;
    let mut b = vec![];
    File::open(path)
        .map_err(err)?
        .take(MAX + 1)
        .read_to_end(&mut b)
        .map_err(err)?;
    if b.len() as u64 > MAX {
        return Err("安装文件超过读取上限".into());
    }
    Ok(b)
}
// Hold directories against rename/junction replacement throughout the transaction on Windows.
fn pin(path: &Path) -> Result<File, String> {
    safe(path)?;
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(3).custom_flags(0x02000000);
    }
    let f = opts.open(path).map_err(err)?;
    safe(path)?;
    Ok(f)
}
fn manifest(root: &Path) -> Result<Vec<u8>, String> {
    let b = read(&root.join("Packages/manifest.json"))?;
    if b.len() > 128 * 1024 {
        return Err("manifest 超过 128 KiB，停止自动安装".into());
    }
    Ok(b)
}
fn json_bytes(b: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(b.strip_prefix(&[239, 187, 191]).unwrap_or(b)).map_err(err)
}
fn dependency(b: &[u8]) -> Result<Option<String>, String> {
    let v = json_bytes(b)?;
    let d = v["dependencies"]
        .as_object()
        .ok_or("manifest 缺少 dependencies 对象")?;
    d.get(PACKAGE)
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("插件依赖必须是字符串".into())
        })
        .transpose()
}
// Locate JSON member spans without reserializing any unrelated fields or formatting.
fn ws(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p].is_ascii_whitespace() {
        p += 1;
    }
    p
}
fn end(b: &[u8], p: usize) -> usize {
    if b[p] == b'"' {
        let mut i = p + 1;
        while i < b.len() {
            if b[i] == b'\\' {
                i += 2;
            } else if b[i] == b'"' {
                return i + 1;
            } else {
                i += 1;
            }
        }
        return i;
    }
    if b[p] == b'{' || b[p] == b'[' {
        let close = if b[p] == b'{' { b'}' } else { b']' };
        let mut i = p + 1;
        while i < b.len() {
            i = ws(b, i);
            if b[i] == close {
                return i + 1;
            }
            if b[i] == b',' || b[i] == b':' {
                i += 1;
            } else {
                i = end(b, i);
            }
        }
        return i;
    }
    let mut i = p;
    while i < b.len() && !b[i].is_ascii_whitespace() && !b",]}:".contains(&b[i]) {
        i += 1;
    }
    i
}
fn members(b: &[u8], p: usize) -> Result<Vec<(String, usize, usize, usize)>, String> {
    if b.get(p) != Some(&b'{') {
        return Err("不是 JSON 对象".into());
    }
    let mut i = ws(b, p + 1);
    let mut rows = vec![];
    let mut keys = std::collections::HashSet::new();
    while b[i] != b'}' {
        let start = i;
        let k = end(b, i);
        let name: String = serde_json::from_slice(&b[i..k]).map_err(err)?;
        if !keys.insert(name.clone()) {
            return Err("manifest 存在重复字段".into());
        }
        let v = ws(b, ws(b, k) + 1);
        let e = end(b, v);
        rows.push((name, start, v, e));
        i = ws(b, e);
        if b[i] == b',' {
            i = ws(b, i + 1);
        }
    }
    Ok(rows)
}
fn without_dependency(b: &[u8]) -> Result<Vec<u8>, String> {
    json_bytes(b)?;
    let start = ws(
        b,
        if b.starts_with(&[239, 187, 191]) {
            3
        } else {
            0
        },
    );
    let root = members(b, start)?;
    let d = root
        .iter()
        .find(|r| r.0 == "dependencies")
        .ok_or("缺少 dependencies")?;
    let rows = members(b, d.2)?;
    let Some(i) = rows.iter().position(|r| r.0 == PACKAGE) else {
        return Ok(b.to_vec());
    };
    let (_, s, _, e) = rows[i];
    let (a, z) = if i + 1 < rows.len() {
        (s, ws(b, e) + 1)
    } else if i > 0 {
        (ws(b, rows[i - 1].3), e)
    } else {
        (s, e)
    };
    let mut out = b[..a].to_vec();
    out.extend_from_slice(&b[z..]);
    json_bytes(&out)?;
    Ok(out)
}
fn inventory(root: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(base: &Path, path: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
        safe(path)?;
        for e in fs::read_dir(path).map_err(err)? {
            let p = e.map_err(err)?.path();
            safe(&p)?;
            if p.is_dir() {
                walk(base, &p, out)?;
            } else {
                if out.len() > 512 {
                    return Err("插件目录包含过多文件".into());
                }
                out.insert(
                    p.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    storage::hash(&read(&p)?),
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}
fn expected() -> BTreeMap<String, String> {
    FILES
        .iter()
        .map(|(n, _, h)| (n.to_string(), h.to_string()))
        .collect()
}
fn same(root: &Path) -> Result<bool, String> {
    Ok(inventory(root)? == expected())
}
fn load(w: &Workspace) -> Result<Option<InstallRecord>, String> {
    let path = w.directory.join(RECORD);
    safe(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let r: InstallRecord = serde_json::from_slice(&read(&path)?).map_err(err)?;
    let d = w.data.lock().unwrap();
    if r.project_id != d.id || r.root != d.root || uuid::Uuid::parse_str(&r.id).is_err() {
        return Err("安装记录工程身份不匹配".into());
    }
    Ok(Some(r))
}
fn record_view(r: &InstallRecord) -> Value {
    json!({"id":r.id,"status":r.status,"reason":r.reason,"bundleHash":r.bundle_hash})
}
fn save(w: &Workspace, r: &InstallRecord) -> Result<(), String> {
    safe(&w.directory.join(RECORD))?;
    storage::atomic(
        &w.directory.join(RECORD),
        &serde_json::to_vec(r).map_err(err)?,
    )
}
fn old_source(root: &Path, dep: &str) -> Result<(), String> {
    let p = dep
        .strip_prefix("file:")
        .ok_or("Git／registry 来源不自动迁移")?;
    let p = PathBuf::from(p);
    let p = if p.is_absolute() {
        p
    } else {
        root.join("Packages").join(p)
    };
    safe(&p)?;
    match fs::symlink_metadata(&p) {
        Ok(_) => {
            if !p.is_dir() || !same(&p)? {
                return Err(
                    "旧插件版本或内容与程序内置版本不同，请保留并核对本地修改；不会覆盖".into(),
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(err(e)),
    }
    Ok(())
}
pub fn status(w: &Workspace) -> Result<PluginStatus, String> {
    let (version, bundle_hash) = bundle()?;
    let root = w.data.lock().unwrap().root.clone();
    workflow::project_available(&root)?;
    safe(&root.join("Packages"))?;
    let b = manifest(&root)?;
    let dep = dependency(&b)?;
    // Also validate duplicate manifest members before offering installation.
    without_dependency(&b)?;
    let record = load(w)?;
    let (state, reason) = if record.as_ref().is_some_and(|r| r.status != "completed") {
        (
            "interrupted",
            "上次安装未完成。关闭目标 Editor 后点击继续安装；记录与指纹不符时会停止。".to_owned(),
        )
    } else if root.join(TARGET).try_exists().map_err(err)? {
        if same(&root.join(TARGET))? {
            if dep.is_some() {
                (
                    "conflict",
                    "内嵌包已存在但 manifest 仍声明其他来源，请先核对；不自动覆盖".into(),
                )
            } else {
                (
                    "installed",
                    "插件文件已安装；仍需打开 Unity 完成依赖解析和编译".into(),
                )
            }
        } else {
            (
                "conflict",
                "工程内插件版本或内容不同，不自动覆盖或升级".into(),
            )
        }
    } else if let Some(d) = dep {
        match old_source(&root, &d) {
            Ok(()) => (
                "migrate",
                "检测到本地路径依赖，可迁移为随工程提交的内嵌包".into(),
            ),
            Err(e) => ("conflict", e),
        }
    } else {
        ("missing", "尚未安装，可继续离线分析".into())
    };
    let lock = root.join("Packages/packages-lock.json");
    let lock_warning = if lock.try_exists().map_err(err)? {
        let v = json_bytes(&read(&lock)?)?;
        let d = &v["dependencies"][PACKAGE];
        (d["source"] != "embedded"
            && (d["source"] == "local"
                || d["version"]
                    .as_str()
                    .is_some_and(|v| v.starts_with("file:"))))
        .then(|| {
            "lock 中仍有本地路径记录，请打开 Unity 重新解析；确认不再引用个人目录后提交 lock 文件"
                .into()
        })
    } else {
        None
    };
    Ok(PluginStatus {
        state: state.into(),
        reason,
        version,
        bundle_hash,
        lock_warning,
        record: record.as_ref().map(record_view),
    })
}
struct EditorLease {
    file: Option<File>,
    path: PathBuf,
    created: bool,
    _temp: File,
}
impl Drop for EditorLease {
    fn drop(&mut self) {
        self.file.take();
        if self.created {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn editor_lease(root: &Path) -> Result<EditorLease, String> {
    #[cfg(not(windows))]
    {
        let _ = root;
        return Err("插件安装首版仅支持 Windows".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = root.join("Temp");
        safe(&temp)?;
        if !temp.exists() {
            fs::create_dir(&temp).map_err(err)?;
        }
        let temp_pin = pin(&temp)?;
        let path = temp.join("UnityLockfile");
        safe(&path)?;
        let created = !path.exists();
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
            .map_err(|_| {
                "请先关闭目标工程的 Unity Editor，或检查 Temp/UnityLockfile 访问权限".to_owned()
            })?;
        Ok(EditorLease {
            file: Some(f),
            path,
            created,
            _temp: temp_pin,
        })
    }
}
/// No caller-provided root or bundle. Recovery resumes only the recorded transaction.
pub fn install(w: &Workspace) -> Result<PluginStatus, String> {
    if w.busy.swap(true, Ordering::SeqCst) {
        return Err("请先停止当前任务".into());
    }
    struct Busy<'a>(&'a Workspace);
    impl Drop for Busy<'_> {
        fn drop(&mut self) {
            self.0.busy.store(false, Ordering::SeqCst);
        }
    }
    let _busy = Busy(w);
    let root = w.data.lock().unwrap().root.clone();
    workflow::project_available(&root)?;
    let _root = pin(&root)?;
    let _packages = pin(&root.join("Packages"))?;
    let _records = pin(&w.directory)?;
    let _editor = editor_lease(&root)?;
    let s = status(w)?;
    if s.state == "installed" {
        return Ok(s);
    }
    if s.state == "conflict" {
        return Err(s.reason);
    }
    let before = manifest(&root)?;
    let mut r = if let Some(r) = load(w)?.filter(|r| r.status != "completed") {
        if r.bundle_hash != s.bundle_hash || r.after != without_dependency(&r.before)? {
            return Err("恢复记录与当前插件不匹配，请核对安装记录".into());
        }
        r
    } else {
        if let Some(d) = dependency(&before)? {
            old_source(&root, &d)?;
        }
        let d = w.data.lock().unwrap();
        InstallRecord {
            project_id: d.id.clone(),
            root: root.clone(),
            id: super::id(),
            bundle_hash: s.bundle_hash,
            before: before.clone(),
            after: without_dependency(&before)?,
            status: "prepared".into(),
            reason: None,
        }
    };
    if before != r.before && before != r.after {
        return Err("manifest 已被外部修改，停止安装恢复".into());
    }
    save(w, &r)?;
    let outcome = (|| -> Result<(), String> {
        let target = root.join(TARGET);
        let stage = w.directory.join(format!("plugin-stage-{}", r.id));
        // Staging is outside Unity's Packages; incomplete packages are never exposed to Editor.
        safe(&stage)?;
        if target.exists() {
            if !same(&target)? {
                return Err("目标插件发生变化，停止恢复".into());
            }
        } else {
            if !stage.exists() {
                fs::create_dir(&stage).map_err(err)?;
            }
            let _stage = pin(&stage)?;
            let mut directories = vec![];
            for (name, bytes, _) in FILES {
                let path = stage.join(name);
                safe(&path)?;
                fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
                directories.push(pin(path.parent().unwrap())?);
                if path.exists() {
                    if read(&path)? != *bytes {
                        return Err("暂存文件发生变化或写入不完整，请核对安装记录".into());
                    }
                } else {
                    let mut f = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(err)?;
                    f.write_all(bytes).map_err(err)?;
                    f.sync_all().map_err(err)?;
                }
            }
            if !same(&stage)? {
                return Err("暂存插件文件不完整或包含未知文件".into());
            }
            if manifest(&root)? != r.before {
                return Err("manifest 已变化，未发布插件".into());
            }
            // Directory rename across volumes is not supported: publish through a verified sibling stage.
            let publish = root
                .join("Packages")
                .join(format!(".upaa-install-{}", r.id));
            safe(&publish)?;
            if !publish.exists() {
                fs::create_dir(&publish).map_err(err)?;
            }
            let mut publish_directories = vec![pin(&publish)?];
            for (name, bytes, _) in FILES {
                let path = publish.join(name);
                safe(&path)?;
                fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
                publish_directories.push(pin(path.parent().unwrap())?);
                if path.exists() {
                    if read(&path)? != *bytes {
                        return Err("工程暂存文件冲突".into());
                    }
                } else {
                    let mut f = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&path)
                        .map_err(err)?;
                    f.write_all(bytes).map_err(err)?;
                    f.sync_all().map_err(err)?;
                }
            }
            if !same(&publish)? || manifest(&root)? != r.before {
                return Err("发布前指纹冲突".into());
            }
            if target.try_exists().map_err(err)? {
                return Err("插件目标已存在，不覆盖".into());
            }
            drop(publish_directories);
            safe(&publish)?;
            fs::rename(&publish, &target).map_err(err)?;
        }
        if !same(&target)? {
            return Err("发布后插件指纹变化".into());
        }
        let current = manifest(&root)?;
        if current != r.after {
            if current != r.before {
                return Err("插件已复制，但 manifest 被外部修改；请核对后恢复".into());
            }
            safe(&root.join("Packages/manifest.json"))?;
            storage::atomic(&root.join("Packages/manifest.json"), &r.after)?;
        }
        Ok(())
    })();
    r.status = if outcome.is_ok() {
        "completed"
    } else {
        "interrupted"
    }
    .into();
    r.reason = outcome.as_ref().err().cloned();
    save(w, &r)?;
    outcome?;
    status(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (Temp, Workspace) {
        let p = std::env::temp_dir().join(format!("upaa-plugin-{}", super::super::id()));
        let root = p.join("中文工程");
        let records = p.join("records");
        fs::create_dir_all(&records).unwrap();
        for d in ["Assets", "Packages", "ProjectSettings"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::write(
            root.join("Packages/manifest.json"),
            b"{\n  \"dependencies\": {\"keep\": \"1.2\"},\n  \"scopedRegistries\": []\n}\n",
        )
        .unwrap();
        let w = Workspace::create(records, root, "plugin test".into()).unwrap();
        (Temp(p), w)
    }
    fn root(w: &Workspace) -> PathBuf {
        w.data.lock().unwrap().root.clone()
    }
    fn local(w: &Workspace, p: &str) {
        fs::write(root(w).join("Packages/manifest.json"),serde_json::to_vec(&json!({"dependencies":{PACKAGE:format!("file:{p}"),"keep":"1.2"},"scopedRegistries":[]})).unwrap()).unwrap();
    }
    #[test]
    fn resource_integrity() {
        let (v, h) = bundle().unwrap();
        assert_eq!(v, "0.1.0");
        assert_eq!(h.len(), 64);
        assert!(FILES.iter().any(|f| f.0.ends_with(".cs.meta")));
    }
    #[test]
    fn remove_only_dependency_preserves_bytes() {
        for content in [
   "{\"dependencies\": {\"com.upaa.inspector\":\"file:x\"}}",
   "{\"dependencies\": {\"com.upaa.inspector\":\"file:x\", \"a\":\"1\"}, \"other\": [1,2]}",
   "{\"dependencies\": {\"a\":\"1\", \"com.upaa.inspector\":\"file:x\"}, \"other\": {\"a\":true}}",
   "{\"dependencies\": {\"a\":\"1\", \"com.upaa.inspector\":\"file:x\", \"b\":\"2\"}}"
  ]{let b=without_dependency(content.as_bytes()).unwrap();let mut v:Value=serde_json::from_str(content).unwrap();v["dependencies"].as_object_mut().unwrap().remove(PACKAGE);assert_eq!(json_bytes(&b).unwrap(),v);assert!(String::from_utf8(b).unwrap().contains("\"dependencies\": {"));}
        assert!(without_dependency(b"{\"dependencies\":{},\"dependencies\":{}}").is_err());
    }
    #[test]
    fn first_install_idempotent_and_metadata() {
        let (_t, w) = setup();
        let b = manifest(&root(&w)).unwrap();
        assert_eq!(status(&w).unwrap().state, "missing");
        assert_eq!(install(&w).unwrap().state, "installed");
        assert_eq!(manifest(&root(&w)).unwrap(), b);
        assert!(same(&root(&w).join(TARGET)).unwrap());
        let id = load(&w).unwrap().unwrap().id;
        install(&w).unwrap();
        assert_eq!(load(&w).unwrap().unwrap().id, id);
        assert!(!root(&w).join("Packages/packages-lock.json").exists());
    }
    #[test]
    fn missing_external_path_migrates_and_backup_preserved() {
        let (_t, w) = setup();
        local(&w, "../removed-plugin");
        let b = manifest(&root(&w)).unwrap();
        assert_eq!(status(&w).unwrap().state, "migrate");
        install(&w).unwrap();
        assert_eq!(load(&w).unwrap().unwrap().before, b);
        assert!(dependency(&manifest(&root(&w)).unwrap()).unwrap().is_none());
    }
    #[test]
    fn different_existing_package_and_remote_source_refused() {
        let (_t, w) = setup();
        fs::write(
            root(&w).join("Packages/manifest.json"),
            serde_json::to_vec(&json!({"dependencies":{PACKAGE:"https://example.com/a.git"}}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status(&w).unwrap().state, "conflict");
        assert!(install(&w).is_err());
    }
    #[test]
    fn external_changed_package_refused() {
        let (t, w) = setup();
        let old = t.0.join("old");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("package.json"), "{}").unwrap();
        local(&w, old.to_str().unwrap());
        assert_eq!(status(&w).unwrap().state, "conflict");
        assert!(install(&w).is_err());
        assert!(!root(&w).join(TARGET).exists());
    }
    #[test]
    fn matching_external_package_migrates() {
        let (t, w) = setup();
        let old = t.0.join("old");
        for (n, b, _) in FILES {
            let p = old.join(n);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b).unwrap();
        }
        local(&w, old.to_str().unwrap());
        install(&w).unwrap();
        assert!(same(&old).unwrap());
    }
    #[test]
    fn edited_embedded_is_not_overwritten() {
        let (_t, w) = setup();
        install(&w).unwrap();
        fs::write(root(&w).join(TARGET).join("package.json"), "{}").unwrap();
        assert_eq!(status(&w).unwrap().state, "conflict");
        assert!(install(&w).is_err());
    }
    #[test]
    fn busy_denied() {
        let (_t, w) = setup();
        w.busy.store(true, Ordering::SeqCst);
        assert!(install(&w).is_err());
        assert!(!root(&w).join(TARGET).exists());
    }
    #[test]
    fn interrupted_after_publish_can_resume() {
        let (_t, w) = setup();
        local(&w, "../missing");
        install(&w).unwrap();
        let mut r = load(&w).unwrap().unwrap();
        fs::write(root(&w).join("Packages/manifest.json"), &r.before).unwrap();
        r.status = "prepared".into();
        save(&w, &r).unwrap();
        assert_eq!(status(&w).unwrap().state, "interrupted");
        install(&w).unwrap();
        assert_eq!(manifest(&root(&w)).unwrap(), r.after);
    }
    #[test]
    fn external_manifest_change_blocks_recovery() {
        let (_t, w) = setup();
        install(&w).unwrap();
        let mut r = load(&w).unwrap().unwrap();
        r.status = "prepared".into();
        save(&w, &r).unwrap();
        fs::write(
            root(&w).join("Packages/manifest.json"),
            b"{\"dependencies\":{\"new\":\"1\"}}",
        )
        .unwrap();
        assert!(install(&w).is_err());
        assert!(String::from_utf8(manifest(&root(&w)).unwrap())
            .unwrap()
            .contains("new"));
    }
    #[test]
    fn readonly_manifest_keeps_recoverable_partial_state() {
        let (_t, w) = setup();
        local(&w, "../missing");
        let path = root(&w).join("Packages/manifest.json");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(install(&w).is_err());
        assert_eq!(load(&w).unwrap().unwrap().status, "interrupted");
        let mut p = fs::metadata(&path).unwrap().permissions();
        p.set_readonly(false);
        fs::set_permissions(&path, p).unwrap();
        install(&w).unwrap();
    }
    #[test]
    fn lock_file_only_warns_not_rewritten() {
        let (_t, w) = setup();
        let path = root(&w).join("Packages/packages-lock.json");
        let b = serde_json::to_vec(
            &json!({"dependencies":{PACKAGE:{"source":"local","version":"file:C:/someone"}}}),
        )
        .unwrap();
        fs::write(&path, &b).unwrap();
        assert!(install(&w).unwrap().lock_warning.is_some());
        assert_eq!(fs::read(&path).unwrap(), b);
        fs::write(&path, serde_json::to_vec(&json!({"dependencies":{PACKAGE:{"source":"embedded","version":"file:com.upaa.inspector"}}})).unwrap()).unwrap();
        assert!(status(&w).unwrap().lock_warning.is_none());
    }
    #[cfg(windows)]
    #[test]
    fn editor_open_denied() {
        let (_t, w) = setup();
        let _lease = editor_lease(&root(&w)).unwrap();
        assert!(install(&w).is_err());
        assert!(!root(&w).join(TARGET).exists());
    }
    #[cfg(windows)]
    #[test]
    fn packages_junction_denied() {
        let (t, w) = setup();
        let packages = root(&w).join("Packages");
        let outside = t.0.join("outside");
        fs::rename(&packages, &outside).unwrap();
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&packages)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(install(&w).is_err());
        fs::remove_dir(&packages).unwrap();
    }
    #[test]
    fn project_binding_in_record_checked() {
        let (_t, w) = setup();
        install(&w).unwrap();
        let mut r = load(&w).unwrap().unwrap();
        r.project_id = "other".into();
        save(&w, &r).unwrap();
        assert!(status(&w).is_err());
    }
}
