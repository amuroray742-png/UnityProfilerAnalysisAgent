//! Offline Unity project index and narrowly scoped Editor enrichment.
pub mod editor;
mod files;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};
const MAX_FILES: usize = 200_000;
const MAX_ITEMS: usize = 1_000_000;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub scope_id: String,
    pub file_id: String,
    pub root: String,
    pub unity_version: String,
    pub file_count: usize,
    pub warnings: Vec<String>,
    pub editor: editor::EditorStatus,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Object {
    file_id: String,
    class_id: String,
    line: usize,
    name: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reference {
    source: String,
    object_id: Option<String>,
    line: usize,
    property: String,
    guid: Option<String>,
    file_id: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    path: String,
    size: u64,
    hash: Option<String>,
    text: bool,
    reason: Option<String>,
    objects: Vec<Object>,
    #[serde(skip)]
    object_ids: HashSet<String>,
}
#[derive(Debug)]
pub struct ProjectScope {
    pub info: ProjectInfo,
    pub cancelled: Arc<AtomicBool>,
    root: PathBuf,
    entries: BTreeMap<String, Entry>,
    guids: HashMap<String, Vec<String>>,
    references: Vec<Reference>,
    editor_hashes: Mutex<HashMap<String, String>>,
    editor_state: Mutex<Option<Value>>,
    editor_evidence: Mutex<BTreeMap<String, Value>>,
    pub editor_status: Mutex<editor::EditorStatus>,
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
fn text_kind(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "cs" | "shader"
            | "hlsl"
            | "cginc"
            | "compute"
            | "unity"
            | "prefab"
            | "mat"
            | "asset"
            | "meta"
            | "json"
            | "asmdef"
            | "asmref"
            | "txt"
            | "controller"
            | "overridecontroller"
            | "anim"
            | "rendertexture"
            | "shadervariants"
            | "shadergraph"
            | "shadersubgraph"
    )
}
fn limit(path: &str) -> u64 {
    if path.to_ascii_lowercase().ends_with(".cs") {
        2 * 1024 * 1024
    } else {
        64 * 1024 * 1024
    }
}
fn guid(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn builtin(value: &str) -> bool {
    value == "00000000000000000000000000000000" || value.starts_with("0000000000000000")
}
fn warning(w: &mut Vec<String>, value: String) {
    if w.len() < 100 {
        w.push(value);
    }
}
impl ProjectScope {
    pub fn prepare(
        file_id: String,
        root: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if files::linked(&fs::symlink_metadata(&root).map_err(|e| e.to_string())?) {
            return Err("工程根目录不能是链接或 junction".into());
        }
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for name in ["Assets", "Packages", "ProjectSettings"] {
            let p = root.join(name);
            if !p.is_dir() || files::linked(&fs::symlink_metadata(&p).map_err(|e| e.to_string())?) {
                return Err(format!("不是完整 Unity 工程：缺少普通 {name} 目录"));
            }
        }
        let mut version = String::new();
        let mut version_file = files::open(&root, "ProjectSettings/ProjectVersion.txt")?;
        files::lines(&mut version_file, 64 * 1024, &cancelled, |_, line| {
            if let Some(v) = line.strip_prefix("m_EditorVersion:") {
                version = v.trim().into();
            }
            Ok(true)
        })?;
        if version.is_empty() {
            return Err("缺少 Unity 工程版本".into());
        }
        let mut scope = Self {
            info: ProjectInfo {
                scope_id: uuid::Uuid::new_v4().to_string(),
                file_id,
                root: root.to_string_lossy().into(),
                unity_version: version,
                file_count: 0,
                warnings: vec![],
                editor: editor::EditorStatus::unavailable("尚未检查 Editor"),
            },
            cancelled,
            root: root.clone(),
            entries: BTreeMap::new(),
            guids: HashMap::new(),
            references: vec![],
            editor_hashes: Mutex::new(HashMap::new()),
            editor_state: Mutex::new(None),
            editor_evidence: Mutex::new(BTreeMap::new()),
            editor_status: Mutex::new(editor::EditorStatus::unavailable("尚未检查 Editor")),
        };
        let mut pending = vec![
            root.join("Assets"),
            root.join("Packages"),
            root.join("ProjectSettings"),
        ];
        let mut objects = 0;
        let mut skipped = 0;
        while let Some(dir) = pending.pop() {
            files::check(&scope.cancelled)?;
            if files::linked(&fs::symlink_metadata(&dir).map_err(|e| e.to_string())?)
                || !dir
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&root)
            {
                skipped += 1;
                continue;
            }
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) => {
                    warning(&mut scope.info.warnings, format!("{}: {e}", dir.display()));
                    skipped += 1;
                    continue;
                }
            };
            for item in entries {
                files::check(&scope.cancelled)?;
                let item = match item {
                    Ok(e) => e,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let path = item.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                if files::linked(&meta) {
                    skipped += 1;
                    continue;
                }
                if meta.is_dir() {
                    if ![
                        ".git",
                        "library",
                        "temp",
                        "obj",
                        "logs",
                        "build",
                        "builds",
                        "bin",
                        "node_modules",
                    ]
                    .contains(
                        &item
                            .file_name()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .as_str(),
                    ) {
                        pending.push(path);
                    }
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                if scope.entries.len() >= MAX_FILES {
                    return Err("工程超过 200,000 条文件记录，无法建立完整索引".into());
                }
                let rel = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let mut entry = Entry {
                    path: rel.clone(),
                    size: meta.len(),
                    hash: None,
                    text: false,
                    reason: None,
                    objects: vec![],
                    object_ids: HashSet::new(),
                };
                if text_kind(&path) {
                    let mut local_refs = vec![];
                    let mut own_guid = None;
                    let mut current = None;
                    let result = (|| -> Result<(), String> {
                        if meta.len() > limit(&rel) {
                            return Err("文本超过读取上限".into());
                        }
                        let mut f = files::open(&root, &rel)?;
                        let hash = files::hash(&mut f, limit(&rel), &scope.cancelled)?;
                        files::lines(&mut f, limit(&rel), &scope.cancelled, |line_no, line| {
                            let t = line.trim();
                            if rel.ends_with(".meta") {
                                if let Some(v) = line.strip_prefix("guid:") {
                                    let v = v.trim();
                                    if guid(v) {
                                        own_guid = Some(v.to_ascii_lowercase());
                                    }
                                }
                            }
                            if let Some(doc) = t.strip_prefix("--- !u!") {
                                let parts = doc.split_whitespace().collect::<Vec<_>>();
                                if parts.len() >= 2 {
                                    let id = parts[1].trim_start_matches('&');
                                    if id.parse::<i64>().is_ok() && parts[0].parse::<u32>().is_ok()
                                    {
                                        if !entry.object_ids.insert(id.to_owned()) {
                                            return Err(format!("第 {line_no} 行重复 fileID {id}"));
                                        }
                                        current = Some(id.to_owned());
                                        if objects + entry.objects.len() < MAX_ITEMS {
                                            entry.objects.push(Object {
                                                file_id: id.into(),
                                                class_id: parts[0].into(),
                                                line: line_no,
                                                name: None,
                                            });
                                        } else {
                                            return Err("序列化对象索引超过 1,000,000 条".into());
                                        }
                                    } else {
                                        return Err(format!("第 {line_no} 行对象头无法解析"));
                                    }
                                } else {
                                    return Err(format!("第 {line_no} 行对象头不完整"));
                                }
                            }
                            if let Some(name) = t.strip_prefix("m_Name:") {
                                if let Some(obj) = entry.objects.last_mut() {
                                    if obj.name.is_none() {
                                        obj.name = Some(name.trim().chars().take(256).collect());
                                    }
                                }
                            }
                            // Unity serialized PPtr is an inline map. Strings/comments are not parsed as maps.
                            if let Some((property, body)) = t.split_once(':') {
                                let body = body.trim();
                                if body.starts_with('{') && body.ends_with('}') {
                                    let mut id = None;
                                    let mut target = None;
                                    for field in body[1..body.len() - 1].split(',') {
                                        if let Some((k, v)) = field.split_once(':') {
                                            match k.trim() {
                                                "fileID" if v.trim().parse::<i64>().is_ok() => {
                                                    id = Some(v.trim().to_owned())
                                                }
                                                "guid" if guid(v.trim()) => {
                                                    target = Some(v.trim().to_ascii_lowercase())
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    if let Some(id) = id {
                                        if id != "0" {
                                            if scope.references.len() + local_refs.len()
                                                >= MAX_ITEMS
                                            {
                                                return Err("资源引用索引超过 1,000,000 条".into());
                                            }
                                            local_refs.push(Reference {
                                                source: rel.clone(),
                                                object_id: current.clone(),
                                                line: line_no,
                                                property: property
                                                    .trim_start_matches("- ")
                                                    .chars()
                                                    .take(256)
                                                    .collect(),
                                                guid: target,
                                                file_id: id,
                                            });
                                        }
                                    }
                                }
                            }
                            Ok(true)
                        })?;
                        if files::hash(&mut f, limit(&rel), &scope.cancelled)? != hash {
                            return Err("索引期间文件发生变化".into());
                        }
                        entry.hash = Some(hash);
                        entry.text = true;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        entry.objects.clear();
                        entry.object_ids.clear();
                        entry.reason = Some(e.clone());
                        skipped += 1;
                        warning(&mut scope.info.warnings, format!("{rel}: {e}"));
                    } else {
                        objects += entry.objects.len();
                        scope.references.extend(local_refs);
                        if let Some(g) = own_guid {
                            scope
                                .guids
                                .entry(g)
                                .or_default()
                                .push(rel.trim_end_matches(".meta").into());
                        }
                    }
                } else {
                    entry.reason =
                        Some("二进制/非允许文本：仅文件元数据，内部信息需要 Editor".into());
                }
                scope.entries.insert(rel, entry);
            }
        }
        for (g, paths) in &scope.guids {
            if paths.len() != 1 {
                warning(
                    &mut scope.info.warnings,
                    format!("重复 GUID {g}：{} 个候选，不能唯一定位", paths.len()),
                );
            }
            for p in paths {
                if !scope.entries.contains_key(p) && !root.join(p).is_dir() {
                    warning(&mut scope.info.warnings, format!("孤立 meta：{p}"));
                }
            }
        }
        let missing = scope
            .references
            .iter()
            .filter(|r| {
                r.guid
                    .as_ref()
                    .is_some_and(|g| !builtin(g) && !scope.guids.contains_key(g))
            })
            .count();
        if missing > 0 {
            warning(
                &mut scope.info.warnings,
                format!("{missing} 个引用在离线索引中未解析，可能缺失或来自外部包"),
            );
        }
        scope.info.file_count = scope.entries.len();
        scope.info.warnings.push(format!("跳过/无法读取 {skipped} 项；最多展示 100 条明细。引用索引仅覆盖已解析的 Unity 文本 PPtr，不含运行时动态加载、二进制内部、外部包；Prefab 覆盖为原始声明，不代表已合成的有效值。"));
        files::check(&scope.cancelled)?;
        Ok(scope)
    }
    pub fn context(&self) -> Value {
        json!({"project":self.info,"editor":*self.editor_status.lock().unwrap(),"editorAssetsRead":self.editor_hashes.lock().unwrap().len(),"editorEvidence":*self.editor_evidence.lock().unwrap(),"scope":"Assets、ProjectSettings、工程内嵌 Packages；名称/引用/Editor 当前状态不是录制当帧的因果证据"})
    }
    fn verified(&self, path: &str) -> Result<(std::fs::File, &Entry), String> {
        files::check(&self.cancelled)?;
        let entry = self.entries.get(path).ok_or("文件不在工程索引内")?;
        let expected = entry
            .hash
            .as_ref()
            .ok_or("不是可读取的文本；请查询 project_asset")?;
        let mut file = files::open(&self.root, path)?;
        if &files::hash(&mut file, limit(path), &self.cancelled)? != expected {
            return Err(format!("工程文件已变化：{path}，请重新准备并分析"));
        }
        Ok((file, entry))
    }
    fn checked_index(&self) -> Result<(), String> {
        for (p, e) in &self.entries {
            files::check(&self.cancelled)?;
            if e.hash.is_some() {
                self.verified(p)?;
            }
        }
        Ok(())
    }
    fn verify_asset(&self, path: &str) -> Result<(), String> {
        for candidate in [path.to_owned(), format!("{path}.meta")] {
            if self
                .entries
                .get(&candidate)
                .is_some_and(|e| e.hash.is_some())
            {
                self.verified(&candidate)?;
            }
        }
        Ok(())
    }
    pub async fn query(self: &Arc<Self>, name: &str, args: Value) -> Result<Value, String> {
        files::check(&self.cancelled)?;
        let scope = self.clone();
        let tool = name.to_owned();
        let query = args.clone();
        let mut result = tokio::task::spawn_blocking(move || scope.offline(&tool, query))
            .await
            .map_err(|e| e.to_string())??;
        if name == "project_asset" {
            let path = args["path"].as_str().ok_or("缺少 path")?;
            let sampled = editor::inspect(
                &self.root,
                path,
                args["start"].as_u64().unwrap_or(0) as usize,
                args["limit"].as_u64().unwrap_or(20) as usize,
                &self.cancelled,
            )
            .await;
            // Do not combine an older offline object/GUID with a newer Editor observation.
            let check = self.clone();
            let target = path.to_owned();
            tokio::task::spawn_blocking(move || check.verify_asset(&target))
                .await
                .map_err(|e| e.to_string())??;
            match sampled {
                Ok(data) => {
                    let mut baseline = self.editor_state.lock().unwrap();
                    let changed = baseline.as_ref().is_some_and(|v| {
                        v["unityVersion"] != data["unityVersion"]
                            || v["targetPlatform"] != data["targetPlatform"]
                    }) || self
                        .info
                        .editor
                        .unity_version
                        .as_ref()
                        .is_some_and(|v| data["unityVersion"] != *v)
                        || self
                            .info
                            .editor
                            .target_platform
                            .as_ref()
                            .is_some_and(|v| data["targetPlatform"] != *v);
                    if changed {
                        self.editor_evidence.lock().unwrap().insert(path.into(),json!({"status":"invalidated","reason":"Editor 版本或平台发生变化，必须重新准备工程"}));
                        return Err(
                            "Editor 版本或平台发生变化，拒绝混合证据，请重新准备工程".into()
                        );
                    }
                    if baseline.is_none() {
                        *baseline = Some(
                            json!({"unityVersion":data["unityVersion"],"targetPlatform":data["targetPlatform"]}),
                        );
                    }
                    drop(baseline);
                    let fingerprint = data["fingerprint"]
                        .as_str()
                        .ok_or("Editor 未返回资源指纹")?
                        .to_owned();
                    let mut cache = self.editor_hashes.lock().unwrap();
                    if cache.get(path).is_some_and(|h| h != &fingerprint) {
                        self.editor_evidence.lock().unwrap().insert(path.into(),json!({"status":"invalidated","reason":"资源指纹发生变化，先前证据已失效"}));
                        return Err("Editor 资源已变化，请重新准备工程并分析".into());
                    }
                    if cache.len() >= 128 && !cache.contains_key(path) {
                        return Err("本次会话已查询128个资源，请聚焦热点后重试".into());
                    }
                    cache.insert(path.into(), fingerprint);
                    self.editor_evidence.lock().unwrap().insert(path.into(),json!({"status":"ready","fingerprint":data["fingerprint"],"sampledAt":data["sampledAt"],"targetPlatform":data["targetPlatform"],"unityVersion":data["unityVersion"],"origin":data["origin"],"coverage":"partial"}));
                    if let Some(next) = data["nextStart"].as_u64() {
                        if result["nextStart"].is_null() {
                            result["nextStart"] = json!(next);
                        }
                    }
                    result["editorEvidence"] = data;
                }
                Err(e) => {
                    let evidence = json!({"status":"unavailable","reason":e});
                    let mut log = self.editor_evidence.lock().unwrap();
                    if log.len() < 128 || log.contains_key(path) {
                        log.insert(path.into(), evidence.clone());
                    }
                    result["editorEvidence"] = evidence;
                }
            }
        }
        files::check(&self.cancelled)?;
        bounded(result)
    }
    fn offline(&self, name: &str, args: Value) -> Result<Value, String> {
        let allowed = match name {
            "project_summary" => vec![],
            "project_files" => vec!["start", "limit"],
            "project_search" => vec!["query", "start", "limit"],
            "project_read" => vec!["path", "start_line", "limit"],
            "project_asset" => vec!["path", "start", "limit"],
            "project_references" => vec!["path", "direction", "start", "limit"],
            _ => return Err("未知工程工具".into()),
        };
        let obj = args.as_object().ok_or("参数必须为对象")?;
        if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err("未知工程参数".into());
        }
        if name == "project_summary" {
            let mut c = self.context();
            c["editorEvidence"] = json!(self
                .editor_evidence
                .lock()
                .unwrap()
                .iter()
                .take(8)
                .collect::<BTreeMap<_, _>>());
            c["evidenceListCoverage"] =
                json!("最多显示8个资源摘要；各资源的具体信息以 project_asset 返回为准");
            c["project"]["warnings"] = json!(self
                .info
                .warnings
                .iter()
                .take(12)
                .map(|w| preview(w, 600))
                .collect::<Vec<_>>());
            c["warningDetailsTruncated"] = json!(
                self.info.warnings.len() > 12 || self.info.warnings.iter().any(|w| w.len() > 600)
            );
            c["indexCoverage"] = json!(self.info.warnings.last());
            return bounded(c);
        }
        let num = |k: &str, d: usize| -> Result<usize, String> {
            match args.get(k) {
                None => Ok(d),
                Some(v) => v
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or(format!("{k} 必须为非负整数")),
            }
        };
        let start = if name == "project_read" {
            num("start_line", 1)?
                .checked_sub(1)
                .ok_or("行号从 1 开始")?
        } else {
            num("start", 0)?
        };
        let count = num("limit", if name == "project_asset" { 20 } else { 100 })?;
        if count == 0 || count > if name == "project_read" { 400 } else { 100 } {
            return Err("分页数量越界".into());
        }
        let mut rows = vec![];
        let mut seen = 0usize;
        let more = std::cell::Cell::new(false);
        let mut add = |row: Value| -> Result<bool, String> {
            files::check(&self.cancelled)?;
            seen += 1;
            if seen <= start {
                return Ok(true);
            }
            if rows.len() == count || serde_json::to_vec(&rows).unwrap().len() > 12 * 1024 {
                more.set(true);
                return Ok(false);
            }
            rows.push(row);
            Ok(true)
        };
        let mut extra = json!({});
        match name {
            "project_files" => {
                for e in self.entries.values() {
                    if !add(
                        json!({"path":e.path,"size":e.size,"text":e.text,"hash":e.hash,"reason":e.reason}),
                    )? {
                        break;
                    }
                }
            }
            "project_read" => {
                let path = args["path"].as_str().ok_or("缺少 path")?;
                let (mut f, e) = self.verified(path)?;
                extra = json!({"path":path,"hash":e.hash});
                files::lines(&mut f, limit(path), &self.cancelled, |line, text| {
                    add(
                        json!({"line":line,"text":preview(text,2000),"lineTruncated":text.len()>2000}),
                    )
                })?;
                if files::hash(&mut f, limit(path), &self.cancelled)? != e.hash.clone().unwrap() {
                    return Err("读取期间文件已变化".into());
                }
            }
            "project_search" => {
                let q = args["query"]
                    .as_str()
                    .filter(|q| !q.is_empty() && q.len() <= 256)
                    .ok_or("query 需要 1..256 字节字面量")?;
                for (p, e) in &self.entries {
                    if !e.text {
                        continue;
                    }
                    let (mut f, _) = self.verified(p)?;
                    files::lines(&mut f, limit(p), &self.cancelled, |line, text| {
                        if text.contains(q) {
                            add(
                                json!({"path":p,"line":line,"text":preview(text,1500),"hash":e.hash,"lineTruncated":text.len()>1500}),
                            )
                        } else {
                            Ok(true)
                        }
                    })?;
                    if files::hash(&mut f, limit(p), &self.cancelled)? != e.hash.clone().unwrap() {
                        return Err("搜索期间文件已变化".into());
                    }
                    if more.get() {
                        break;
                    }
                }
            }
            "project_asset" => {
                let p = args["path"].as_str().ok_or("缺少 path")?;
                if !valid_asset_path(p) {
                    return Err("资源必须为 Assets/ 或 Packages/ 相对路径".into());
                }
                if let Some(e) = self.entries.get(p) {
                    if e.hash.is_some() {
                        self.verified(p)?;
                    }
                    let meta = format!("{p}.meta");
                    if self.entries.get(&meta).is_some_and(|e| e.hash.is_some()) {
                        self.verified(&meta)?;
                    }
                    extra = json!({"path":p,"size":e.size,"hash":e.hash,"reason":e.reason,"guidCandidates":self.guids.iter().filter(|(_,ps)|ps.iter().any(|v|v==p)).map(|(g,_)|g).collect::<Vec<_>>()});
                    for o in &e.objects {
                        if !add(serde_json::to_value(o).unwrap())? {
                            break;
                        }
                    }
                } else if !p.starts_with("Packages/") {
                    return Err("资源不在索引内".into());
                } else {
                    extra = json!({"path":p,"reason":"外部包，仅允许 Editor 解析的资源摘要，不开放磁盘源码"});
                }
            }
            "project_references" => {
                let p = args["path"].as_str().ok_or("缺少 path")?;
                if !self.entries.contains_key(p) {
                    return Err("资源不在离线索引内".into());
                }
                self.checked_index()?;
                let direction = args["direction"].as_str().unwrap_or("outgoing");
                if !["incoming", "outgoing"].contains(&direction) {
                    return Err("direction 无效".into());
                }
                for r in &self.references {
                    let targets = if let Some(g) = &r.guid {
                        self.guids.get(g).cloned().unwrap_or_default()
                    } else {
                        vec![r.source.clone()]
                    };
                    if (direction == "outgoing" && r.source == p)
                        || (direction == "incoming" && targets.iter().any(|t| t == p))
                    {
                        let local_resolved = targets.len() == 1
                            && self
                                .entries
                                .get(&targets[0])
                                .is_some_and(|e| e.object_ids.contains(&r.file_id));
                        if !add(
                            json!({"reference":r,"targetPaths":targets,"objectResolved":local_resolved,"evidence":"原始序列化 PPtr，不是实际运行依赖或 Prefab 有效覆盖值"}),
                        )? {
                            break;
                        }
                    }
                }
                extra = json!({"coverage":"partial","reason":"仅已索引 Unity 文本引用；不完整反向引用不能视为完整搜索"});
            }
            _ => unreachable!(),
        }
        drop(add);
        extra["rows"] = json!(rows);
        extra["nextStart"] = if more.get() {
            json!(
                start
                    + extra["rows"].as_array().unwrap().len()
                    + if name == "project_read" { 1 } else { 0 }
            )
        } else {
            Value::Null
        };
        extra["warnings"] = json!(self.info.warnings);
        extra["scopeId"] = json!(self.info.scope_id);
        bounded(extra)
    }
}
pub fn valid_asset_path(p: &str) -> bool {
    (p.starts_with("Assets/") || p.starts_with("Packages/"))
        && !p.contains('\\')
        && !p.contains(':')
        && p.split('/').all(|s| !s.is_empty() && s != ".." && s != ".")
}
fn preview(text: &str, max: usize) -> &str {
    let mut n = text.len().min(max);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    &text[..n]
}
pub fn bounded(mut value: Value) -> Result<Value, String> {
    if serde_json::to_string_pretty(&value).unwrap().len() > 20 * 1024 {
        value["warnings"] = json!(["响应包含较多信息，请查 project_summary 的覆盖范围"]);
    }
    if serde_json::to_string_pretty(&value).unwrap().len() > 20 * 1024 {
        return Err("响应超过安全分页预算，请减小 limit 或缩小查询范围".into());
    }
    Ok(value)
}
pub fn schemas() -> Value {
    let page = json!({"start":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}});
    let tool = |name: &str, description: &str, mut properties: Value, required: Vec<&str>| {
        if name != "project_summary" && name != "project_read" {
            for (k, v) in page.as_object().unwrap() {
                properties[k] = v.clone();
            }
        }
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    };
    json!({"tools":[tool("project_summary","工程/Editor 身份、版本、采集范围与覆盖警告",json!({}),vec![]),tool("project_files","分页列出工程文件，不扫描外部包磁盘",json!({}),vec![]),tool("project_search","只读字面量搜索；名称匹配不是性能因果证明",json!({"query":{"type":"string"}}),vec!["query"]),tool("project_read","带行号、哈希读取文本；行从 1 开始，最多400行；检查nextStart和lineTruncated",json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":400}}),vec!["path"]),tool("project_asset","资源序列化对象分页及 Editor 当前资源摘要；与录制当帧不同，检查缺失与变化",json!({"path":{"type":"string"}}),vec!["path"]),tool("project_references","离线原始引用分页，incoming/outgoing；覆盖始终 partial，不证明运行中加载",json!({"path":{"type":"string"},"direction":{"type":"string","enum":["incoming","outgoing"]}}),vec!["path"])]})
}
