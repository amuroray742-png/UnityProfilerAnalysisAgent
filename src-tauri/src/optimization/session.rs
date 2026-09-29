use super::*;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
#[derive(Debug)]
pub struct EditScope {
    pub operation: tokio::sync::Mutex<()>,
    pub workspace: Arc<Workspace>,
    pub run_id: String,
}
impl EditScope {
    pub async fn query(&self, name: &str, args: Value) -> Result<Value, String> {
        let _operation = self.operation.lock().await;
        if name == "optimization_check" {
            if args != json!({}) {
                return Err("检查参数必须为空，测试由用户选择".into());
            }
            let result = self.check_editor_inner().await?;
            if serde_json::to_vec(&result).unwrap().len() > 16000 {
                return Ok(
                    json!({"status":result["status"],"checkedRevision":result["checkedRevision"],"partial":true,"reason":"检查明细超过单次响应限制，完整结果已持久化；请分页读取 optimization_context 的 checks"}),
                );
            }
            return Ok(result);
        }
        if name == "optimization_replace" {
            let root = self.workspace.data.lock().unwrap().root.clone();
            let status = crate::project::editor::status(&root, &self.workspace.cancelled).await;
            if status.status == "busy" {
                return Err("Editor 正在编译、导入或 Play Mode，暂不可写入".into());
            }
        }
        let workspace = self.workspace.clone();
        let run = self.run_id.clone();
        let name = name.to_owned();
        let result = tokio::task::spawn_blocking(move || workspace.edit_query(&run, &name, args))
            .await
            .map_err(|e| e.to_string())??;
        if serde_json::to_string_pretty(&result).unwrap().len() > 20000 {
            return Err("响应超过限制，请缩小分页".into());
        }
        Ok(result)
    }
    pub async fn check_editor(&self) -> Result<Value, String> {
        let _operation = self.operation.lock().await;
        self.check_editor_inner().await
    }
    async fn check_editor_inner(&self) -> Result<Value, String> {
        self.workspace.check()?;
        let (root, paths, tests, revision, expected) = {
            let mut d = self.workspace.data.lock().unwrap();
            let root = d.root.clone();
            let round = d
                .rounds
                .iter_mut()
                .find(|r| r.runs.iter().any(|r| r.id == self.run_id))
                .ok_or("运行不存在")?;
            let tests = round.tests.clone();
            let run = round.runs.iter_mut().find(|r| r.id == self.run_id).unwrap();
            if run.checks.len() >= 3 {
                return Err("已达到 3 次检查上限，停止自动修复".into());
            }
            let paths: Vec<_> = run.changes.iter().map(|c| c.path.clone()).collect();
            let expected: BTreeMap<_, _> = run
                .changes
                .iter()
                .filter(|c| c.state == "applied")
                .map(|c| (c.path.clone(), c.after_hash.clone()))
                .collect();
            let revision = run.changes.len();
            run.checks
                .push(json!({"status":"running","startedAt":chrono::Utc::now().to_rfc3339()}));
            self.workspace.save(&d)?;
            (root, paths, tests, revision, expected)
        };
        let before = {
            let root = root.clone();
            let work = self.workspace.clone();
            tokio::task::spawn_blocking(move || inventory(&root, &work.cancelled))
                .await
                .map_err(|e| e.to_string())?
        };
        let current = {
            let root = root.clone();
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                for (path, hash) in expected {
                    if storage::hash(&editing::read(&root, &path)?) != hash {
                        return Err(format!(
                            "{path} 与本次修改记录不一致，不能把当前工程检查归属到旧修改"
                        ));
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())?
        };
        let result = match current {
            Ok(()) => {
                crate::project::editor::check_changes(
                    &root,
                    paths.clone(),
                    tests,
                    &self.workspace.cancelled,
                )
                .await
            }
            Err(e) => Err(e),
        };
        let after = {
            let root = root.clone();
            let work = self.workspace.clone();
            tokio::task::spawn_blocking(move || inventory(&root, &work.cancelled))
                .await
                .map_err(|e| e.to_string())?
        };
        let mut value =
            result.unwrap_or_else(|reason| json!({"status":"unavailable","reason":reason}));
        let mut d = self.workspace.data.lock().unwrap();
        let run = d
            .rounds
            .iter_mut()
            .flat_map(|r| &mut r.runs)
            .find(|r| r.id == self.run_id)
            .ok_or("运行不存在")?;
        value["checkedRevision"] = json!(revision);
        match (before, after) {
            (Ok(before), Ok(after)) => {
                let keys: std::collections::BTreeSet<_> =
                    before.keys().chain(after.keys()).collect();
                let changed: Vec<_> = keys
                    .into_iter()
                    .filter(|p| before.get(*p) != after.get(*p))
                    .cloned()
                    .collect();
                if !changed.is_empty() {
                    value["status"] = json!("unavailable");
                    value["unexpectedFiles"] = json!(changed);
                    value["reason"] =
                        json!("检查引发或同时出现业务文件变化，暂停自动修改，请人工核查");
                    self.workspace.cancelled.store(true, Ordering::SeqCst);
                }
            }
            (before, after) => {
                value["status"] = json!("unavailable");
                value["reason"] = json!(format!(
                    "文件变化检查未完成：{:?} {:?}",
                    before.err(),
                    after.err()
                ));
            }
        }
        value["fileWatchScope"] = json!(
            "Assets、ProjectSettings、内嵌 Packages；代码指纹及其他资源的大小/修改时间，不遍历链接"
        );
        if run.changes.len() != revision {
            value["status"] = json!("unavailable");
            value["reason"] = json!("检查期间代码再次修改");
        }
        let baseline_errors = run
            .baseline_check
            .as_ref()
            .map(|v| v["editor"]["check"]["errors"].clone())
            .unwrap_or(Value::Null);
        value["baselineErrors"] = baseline_errors.clone();
        if let Some(errors) = value["editor"]["check"]["errors"].as_array() {
            value["newErrors"] = json!(errors
                .iter()
                .filter(|e| !baseline_errors.as_array().is_some_and(|a| a.contains(e)))
                .collect::<Vec<_>>());
        }
        *run.checks.last_mut().unwrap() = value.clone();
        self.workspace.save(&d)?;
        Ok(value)
    }
}
impl Workspace {
    pub fn event(
        &self,
        run_id: &str,
        event: &crate::acp_client::DiagnoseEvent,
    ) -> Result<(), String> {
        use crate::acp_client::DiagnoseEvent::*;
        let mut d = self.data.lock().unwrap();
        let run = d
            .rounds
            .iter_mut()
            .flat_map(|r| &mut r.runs)
            .find(|r| r.id == run_id)
            .ok_or("运行不存在")?;
        if run.status != "running" {
            return Ok(());
        }
        match event {
            SessionCreated { acp_session_id } => run.session_id = acp_session_id.clone(),
            Chunk { text } => {
                let mut n = text.len().min(crate::reports::MAX_REPORT - run.text.len());
                while !text.is_char_boundary(n) {
                    n -= 1;
                }
                run.text.push_str(&text[..n]);
                if n < text.len() {
                    run.reason = Some("正文达到 2 MiB，已停止".into());
                    self.cancelled.store(true, Ordering::SeqCst);
                }
            }
            Finished { .. } => {
                run.status = if !run.changes.iter().any(|c| c.state == "applied") {
                    "failed"
                } else {
                    "modified"
                }
                .into();
            }
            Cancelled => {
                run.status = if !run.changes.iter().any(|c| c.state == "applied") {
                    "failed"
                } else {
                    "partial"
                }
                .into();
                run.reason = Some("已取消，保留落盘改动".into());
            }
            Error { message } => {
                run.status = if !run.changes.iter().any(|c| c.state == "applied") {
                    "failed"
                } else {
                    "partial"
                }
                .into();
                run.reason = Some(message.clone());
            }
            _ => return Ok(()),
        }
        if event.terminal() {
            self.busy.store(false, Ordering::SeqCst);
        }
        // Backups and terminal states always flush; avoid rewriting all backup bytes
        // on every streamed token. A crash can lose at most this unflushed text tail.
        let mut saved = self.last_report_save.lock().unwrap();
        if matches!(event, Chunk { .. }) && saved.elapsed() < std::time::Duration::from_millis(250)
        {
            return Ok(());
        }
        *saved = std::time::Instant::now();
        self.save(&d)
    }
}
pub fn schemas() -> Value {
    json!({"tools":[
        {"name":"optimization_context","description":"默认分页读取本轮持久化任务、限制、检查及既有修改。编辑前读完任务页面。报告正文及资源证据用 report_id 按需分页读取，不续接聊天历史。","inputSchema":{"type":"object","properties":{"start":{"type":"integer","minimum":0},"report_id":{"type":"string"}},"additionalProperties":false}},
        {"name":"optimization_read","description":"只读取用户批准的已有代码，返回行号和当前指纹，按 nextStart 续页。","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}},
        {"name":"optimization_replace","description":"在批准文件中唯一原文替换，保留编码与换行，持久化备份后原子写入。必须先读取最新指纹；不允许新增/删除/改名。取消保留已写改动。","inputSchema":{"type":"object","properties":{"task_id":{"type":"string"},"path":{"type":"string"},"expected_hash":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["task_id","path","expected_hash","old_text","new_text"],"additionalProperties":false}},
        {"name":"optimization_check","description":"请求匹配工程的固定编译/Shader 检查与用户选择的 EditMode 测试，每个新会话最多 3 次。检查未完成不能宣称通过。","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
    ]})
}
/// Detect business-file changes caused by compilation/import/test callbacks.
/// Metadata for binary assets, hashes for code. Never follows links or opens scenes.
pub fn inventory(
    root: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, String>, String> {
    let mut pending = vec![];
    for name in ["Assets", "Packages", "ProjectSettings"] {
        pending.push(root.join(name));
    }
    let mut result = BTreeMap::new();
    while let Some(path) = pending.pop() {
        crate::project::files::check(cancel)?;
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if crate::project::files::linked(&meta) {
            result.insert(relative, "link-excluded".into());
            continue;
        }
        if meta.is_dir() {
            if path.file_name().is_some_and(|s| {
                [".git", "node_modules", "bin", "obj"]
                    .contains(&s.to_string_lossy().to_lowercase().as_str())
            }) {
                continue;
            }
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        } else if meta.is_file() {
            if result.len() >= 200000 {
                return Err("检查文件记录超过200000，检查未完成".into());
            }
            let fingerprint = if editing::allowed(&relative) && meta.len() <= 2 * 1024 * 1024 {
                storage::hash(&editing::read(root, &relative)?)
            } else {
                format!(
                    "{}:{:?}",
                    meta.len(),
                    meta.modified().map_err(|e| e.to_string())?
                )
            };
            result.insert(relative, fingerprint);
        }
    }
    Ok(result)
}
