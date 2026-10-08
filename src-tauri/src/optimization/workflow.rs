//! Project-first orchestration. Persisted IDs are independent of transient upload/session IDs.
use super::*;
use crate::acp_client::{self, DiagnoseEvent, DiagnoseRequest};
use serde_json::{json, Value};
use std::{fs, path::Path, sync::atomic::Ordering};
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub status: String,
    pub reason: Option<String>,
    pub analysis_agent: String,
    pub localization_agent: String,
}
pub fn new_round(baseline: String) -> Round {
    Round {
        workflow: Progress::default(),
        task_version: 1,
        task_verifications: BTreeMap::new(),
        id: id(),
        baseline,
        candidate: None,
        reports: vec![],
        tasks: vec![],
        runs: vec![],
        tests: vec![],
        comparison: None,
        correctness: "pending".into(),
        decision: "pending".into(),
    }
}
fn round_mut<'a>(d: &'a mut Project, id: &str) -> Result<&'a mut Round, String> {
    d.rounds
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or("轮次不存在".into())
}
fn live_round(d: &Project, id: &str) -> Result<(), String> {
    if d.rounds.last().is_none_or(|r| r.id != id) {
        return Err("历史轮次只读，请继续当前轮次".into());
    }
    Ok(())
}
pub fn project_available(root: &Path) -> Result<(), String> {
    if root.canonicalize().ok().as_deref() != Some(root)
        || fs::symlink_metadata(root)
            .ok()
            .is_none_or(|m| crate::project::files::linked(&m))
    {
        return Err("工程目录已缺失或被链接替换，不能绑定其他工程；仍可查看存档".into());
    }

    if !["Assets", "Packages", "ProjectSettings"]
        .iter()
        .all(|p| root.join(p).is_dir())
    {
        return Err("Unity 工程不可用；可以查看存档，请恢复原工程后继续".into());
    }
    Ok(())
}
pub fn ready_report(r: &Round) -> bool {
    let latest = r.reports.iter().rev().find(|p| p.stage == "performance");
    r.reports.iter().rev().any(|p| {
        p.stage == "project"
            && p.status == "completed"
            && latest.is_none_or(|a| {
                a.status == "completed"
                    && p.parent_report_id.as_deref() == Some(a.report_id.as_str())
            })
    })
}
fn progress(
    w: &Workspace,
    rid: &str,
    stage: &str,
    status: &str,
    reason: Option<String>,
) -> Result<(), String> {
    let mut d = w.data.lock().unwrap();
    let r = round_mut(&mut d, rid)?;
    r.workflow.stage = stage.into();
    r.workflow.status = status.into();
    r.workflow.reason = reason;
    w.save(&d)
}
pub fn next_round(w: &Workspace) -> Result<(), String> {
    let mut d = w.data.lock().unwrap();
    let r = d.rounds.last().ok_or("尚无轮次")?;
    let baseline = match r.decision.as_str() {
        "accepted" => r.candidate.clone().ok_or("接受前需要导入 B 并完成对比")?,
        "rolled_back" => r.baseline.clone(),
        _ => return Err("请先接受或成功回退当前轮次".into()),
    };
    if !d.captures.iter().any(|c| c.id == baseline) {
        return Err("基线录制索引不存在，不能开始下一轮".into());
    }
    let rolled = r.decision == "rolled_back";
    let mut next = new_round(baseline);
    if rolled {
        next.workflow.reason = Some("已沿用原 A；若当前工程与原录制不同，请导入新的 A".into());
    }
    d.rounds.push(next);
    w.save(&d)
}

fn validate_capture_binding(d: &Project, role: &str) -> Result<(), String> {
    if let Some(r) = d.rounds.last() {
        if r.decision != "pending" {
            return Err("本轮已结束，请开始下一轮".into());
        }
        if role == "a" {
            if r.workflow.status == "running" || r.reports.iter().any(|p| p.status == "running") {
                return Err("任务正在运行，请先停止".into());
            }
            if !r.runs.is_empty() {
                return Err("本轮已开始优化，不能替换 A".into());
            }
        }
        if role == "b" && r.runs.is_empty() {
            return Err("请先完成代码优化，再导入 B".into());
        }
    } else if role == "b" {
        return Err("请先导入 A".into());
    }
    Ok(())
}

/// Bind a capture through the same validation and durable commit used by desktop IPC.
pub async fn bind_capture(
    w: Arc<Workspace>,
    path: PathBuf,
    role: String,
    operation_id: Option<String>,
) -> Result<(), String> {
    if w.busy.load(Ordering::SeqCst) {
        return Err("任务正在运行，请先停止".into());
    }
    if !["a", "b"].contains(&role.as_str()) {
        return Err("录制角色无效".into());
    }
    let rid = {
        let d = w.data.lock().unwrap();
        validate_capture_binding(&d, &role)?;
        w.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "任务正在运行，请先停止")?;
        w.cancelled.store(false, Ordering::SeqCst);
        d.rounds.last().map(|r| r.id.clone())
    };
    struct ImportGuard(Arc<Workspace>);
    impl Drop for ImportGuard {
        fn drop(&mut self) {
            self.0.busy.store(false, Ordering::SeqCst);
        }
    }
    let _import_guard = ImportGuard(w.clone());
    let progress =
        observation::ParseProgress::new(w.clone(), rid.clone().unwrap_or_default(), operation_id.unwrap_or_else(id));
    let c = match commands::capture_observed(
        path,
        Conditions::default(),
        Some(progress.clone()),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            progress.fail(&e);
            return Err(e);
        }
    };
    progress.update("save", None, None, "running", None);
    let mut saved = w.data.lock().unwrap();
    if w.cancelled.load(Ordering::SeqCst) {
        progress.fail("导入已取消");
        return Err("导入已取消".into());
    }
    let validation = if saved.rounds.last().map(|r| &r.id) != rid.as_ref() {
        Err("当前轮次已变化，请重新导入".into())
    } else {
        validate_capture_binding(&saved, &role)
    };
    if let Err(e) = validation {
        progress.fail(&e);
        return Err(e);
    }
    let mut d = saved.clone();
    let cid = c.id.clone();
    d.captures.push(c);
    if role == "a" {
        if let Some(r) = d.rounds.last_mut() {
            r.baseline = cid;
            r.reports.clear();
            r.tasks.clear();
            r.task_verifications.clear();
            r.task_version += 1;
            r.candidate = None;
            r.comparison = None;
            r.correctness = "pending".into();
            r.workflow = Progress {
                analysis_agent: r.workflow.analysis_agent.clone(),
                localization_agent: r.workflow.localization_agent.clone(),
                ..Progress::default()
            };
        } else {
            d.rounds.push(new_round(cid));
        }
    } else {
        let r = d.rounds.last_mut().unwrap();
        r.candidate = Some(cid);
        r.comparison = None;
    }
    if let Err(e) = w.save(&d) {
        progress.fail(&e);
        return Err(e);
    }
    *saved = d;
    progress.update("save", Some(1), Some(1), "completed", None);
    Ok(())
}

#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    Recent,
    PluginStatus { project_id: String },
    PluginInstall { project_id: String },
    PluginRecords { project_id: String },
    Activity {
        round_id: String,
        run_id: String,
        cursor: u64,
    },
    RunText {
        round_id: String,
        run_id: String,
        start: usize,
    },
    Change {
        round_id: String,
        run_id: String,
        index: usize,
        start: usize,
    },
    Create {
        root: PathBuf,
        name: String,
        directory: Option<PathBuf>,
    },
    Open {
        directory: PathBuf,
    },
    Bind {
        path: PathBuf,
        role: String,
        operation_id: Option<String>,
    },
    Analyze {
        round_id: String,
        analysis_agent: String,
        localization_agent: String,
        restart: bool,
    },
    Report {
        round_id: String,
        report_id: String,
        start: usize,
    },
    Snapshot {
        capture_id: String,
    },
    Inspect {
        capture_id: String,
    },
    Export {
        round_id: String,
        path: PathBuf,
        format: String,
    },
    Next,
    Relocate {
        capture_id: String,
        path: PathBuf,
    },
}
fn recent_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recent-projects.json"))
}
fn recent(app: &tauri::AppHandle) -> Result<Vec<Value>, String> {
    let path = recent_path(app)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn remember(app: &tauri::AppHandle, w: &Workspace) -> Result<(), String> {
    fs::create_dir_all(app.path().app_data_dir().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut rows = recent(app)?;
    let d = w.data.lock().unwrap();
    rows.retain(|r| r["directory"] != json!(w.directory));
    rows.insert(0,json!({"id":d.id,"name":d.name,"root":d.root,"directory":w.directory,"updatedAt":chrono::Utc::now().to_rfc3339()}));
    rows.truncate(30);
    storage::atomic(&recent_path(app)?, &serde_json::to_vec(&rows).unwrap())
}
#[tauri::command(rename_all = "camelCase")]
pub async fn workflow_command(
    action: Action,
    app: tauri::AppHandle,
    state: tauri::State<'_, commands::OptimizationState>,
    app_state: tauri::State<'_, crate::state::AppState>,
) -> Result<Value, String> {
    let _operation = state.operation.lock().await;
    if matches!(action, Action::Recent) {
        return Ok(json!(recent(&app)?));
    }
    if matches!(action, Action::Create { .. } | Action::Open { .. }) {
        let mut slot = state.workspace.lock().await;
        if slot.is_some() {
            return Err("请先返回项目首页".into());
        }
        let w = match action {
            Action::Create {
                root,
                name,
                directory,
            } => {
                let directory = directory.unwrap_or(
                    app.path()
                        .app_data_dir()
                        .map_err(|e| e.to_string())?
                        .join("optimization-projects")
                        .join(id()),
                );
                tokio::task::spawn_blocking(move || {
                    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                    Workspace::create(directory, root, name)
                })
                .await
                .map_err(|e| e.to_string())??
            }
            Action::Open { directory } => {
                tokio::task::spawn_blocking(move || Workspace::open(directory))
                    .await
                    .map_err(|e| e.to_string())??
            }
            _ => unreachable!(),
        };
        remember(&app, &w)?;
        let view = commands::view(&w);
        *slot = Some(Arc::new(w));
        return Ok(view);
    }
    let w = state
        .workspace
        .lock()
        .await
        .clone()
        .ok_or("请先新建或打开优化项目")?;
    if let Action::PluginStatus { project_id } | Action::PluginInstall { project_id } | Action::PluginRecords { project_id } = &action {
        if w.data.lock().unwrap().id != *project_id { return Err("工程身份已变化，请重新打开状态".into()); }
        if w.busy.load(Ordering::SeqCst) { return Err("请先停止当前任务，再检查或安装插件".into()); }
        if !app_state.0.lock().await.active_sessions.is_empty() { return Err("请先停止诊断会话".into()); }
        let install = matches!(action, Action::PluginInstall { .. });
        let records = matches!(action, Action::PluginRecords { .. });
        let work = w.clone();
        let result = tokio::task::spawn_blocking(move || if install { plugin::install(&work) } else { plugin::status(&work) }).await.map_err(|e|e.to_string())??;
        if records { return Ok(json!(result.record)); }
        let root=w.data.lock().unwrap().root.clone();
        let mut value=json!(result);
        value["editor"]=json!(crate::project::editor::status(&root, &std::sync::atomic::AtomicBool::new(false)).await);
        return Ok(value);
    }
    {
        let app = app.clone();
        w.observation.lock().unwrap().notify = Some(Arc::new(move |name, value| {
            let _ = app.emit(name, value);
        }));
    }
    if let Action::Activity {
        round_id,
        run_id,
        cursor,
    } = &action
    {
        {
            let d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter()
                .find(|r| r.id == *round_id)
                .ok_or("轮次不存在")?;
            if !r.reports.iter().any(|p| p.report_id == *run_id)
                && !r.runs.iter().any(|p| p.id == *run_id)
            {
                return Err("工作记录不属于此轮次".into());
            }
        }
        let page = observation::read(&w.directory, run_id, *cursor)?;
        let pid = w.data.lock().unwrap().id.clone();
        if page["rows"].as_array().is_some_and(|rows| {
            rows.iter().any(|r| {
                r["projectId"] != pid || r["roundId"] != *round_id || r["runId"] != *run_id
            })
        }) {
            return Err("活动日志身份不匹配".into());
        }
        return Ok(page);
    }
    if let Action::RunText {
        round_id,
        run_id,
        start,
    } = &action
    {
        let d = w.data.lock().unwrap();
        let r = d
            .rounds
            .iter()
            .find(|r| r.id == *round_id)
            .ok_or("轮次不存在")?;
        let run = r
            .runs
            .iter()
            .find(|r| r.id == *run_id)
            .ok_or("修改运行不存在")?;
        let chars: Vec<_> = run.text.chars().collect();
        if *start > chars.len() {
            return Err("分页越界".into());
        }
        let end = (*start + 12000).min(chars.len());
        return Ok(
            json!({"text":chars[*start..end].iter().collect::<String>(),"nextStart":(end<chars.len()).then_some(end),"total":chars.len()}),
        );
    }
    if let Action::Change {
        round_id,
        run_id,
        index,
        start,
    } = &action
    {
        let d = w.data.lock().unwrap();
        let r = d
            .rounds
            .iter()
            .find(|r| r.id == *round_id)
            .ok_or("轮次不存在")?;
        let run = r
            .runs
            .iter()
            .find(|r| r.id == *run_id)
            .ok_or("修改运行不存在")?;
        let c = run.changes.get(*index).ok_or("文件变更不存在")?;
        let text = editing::diff(c)?;
        let chars: Vec<_> = text.chars().collect();
        if *start > chars.len() {
            return Err("分页越界".into());
        }
        let end = (*start + 12000).min(chars.len());
        return Ok(
            json!({"text":chars[*start..end].iter().collect::<String>(),"nextStart":(end<chars.len()).then_some(end),"total":chars.len()}),
        );
    }
    if let Action::Snapshot { capture_id } = action {
        let d = w.data.lock().unwrap();
        return Ok(json!(
            d.captures
                .iter()
                .find(|c| c.id == capture_id)
                .ok_or("录制不存在")?
                .snapshot
        ));
    }
    if let Action::Inspect { capture_id } = action {
        let (path, hash) = {
            let d = w.data.lock().unwrap();
            let c = d
                .captures
                .iter()
                .find(|c| c.id == capture_id)
                .ok_or("录制不存在")?;
            (c.path.clone(), c.hash.clone())
        };
        let p = path.clone();
        let before =
            tokio::task::spawn_blocking(move || storage::file_hash(&p, &AtomicBool::new(false)))
                .await
                .map_err(|e| e.to_string())??;
        if before != hash {
            return Err("原录制指纹不一致，请重新定位文件".into());
        }
        let profile = crate::parser::parse_file(&path)
            .await
            .map_err(|e| e.to_string())?;
        let p = path.clone();
        if tokio::task::spawn_blocking(move || storage::file_hash(&p, &AtomicBool::new(false)))
            .await
            .map_err(|e| e.to_string())??
            != hash
        {
            return Err("解析期间录制变化".into());
        }
        let snapshot = crate::extractor::extract(&profile);
        let file_id = id();
        app_state
            .put_upload(crate::state::UploadEntry {
                file_id: file_id.clone(),
                file_path: path.clone(),
                file_name: snapshot.meta.file_name.clone(),
                size_bytes: profile.meta.file_size_bytes,
                extension: path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .into(),
            })
            .await;
        app_state
            .put_analysis(file_id.clone(), snapshot.clone(), profile.details)
            .await;
        return Ok(json!({"fileId":file_id,"snapshot":snapshot}));
    }
    if let Action::Report {
        round_id,
        report_id,
        start,
    } = action
    {
        let d = w.data.lock().unwrap();
        let r = d
            .rounds
            .iter()
            .find(|r| r.id == round_id)
            .ok_or("轮次不存在")?;
        let p = r
            .reports
            .iter()
            .find(|p| p.report_id == report_id)
            .ok_or("报告不属于此轮次")?;
        let chars: Vec<_> = p.markdown().chars().collect();
        if start > chars.len() {
            return Err("分页越界".into());
        }
        let end = (start + 12000).min(chars.len());
        return Ok(
            json!({"text":chars[start..end].iter().collect::<String>(),"nextStart":(end<chars.len()).then_some(end),"total":chars.len()}),
        );
    }
    if let Action::Export {
        round_id,
        path,
        format,
    } = action
    {
        let text = export_round(&w, &round_id)?;
        let text = match format.as_str() {
            "markdown" => text,
            "html" => crate::reports::document(&text),
            _ => return Err("不支持的格式".into()),
        };
        tokio::task::spawn_blocking(move || storage::atomic(&path, text.as_bytes()))
            .await
            .map_err(|e| e.to_string())??;
        return Ok(commands::view(&w));
    }
    if w.busy.load(Ordering::SeqCst) {
        return Err("任务正在运行，请先停止".into());
    }
    match action {
        Action::Bind {
            path,
            role,
            operation_id,
        } => {
            bind_capture(w.clone(), path, role, operation_id).await?;
        }
        Action::Analyze {
            round_id,
            analysis_agent,
            localization_agent,
            restart,
        } => {
            let _start = commands::START_LOCK.lock().await;
            if !app_state.0.lock().await.active_sessions.is_empty() || commands::active() {
                return Err("请先结束当前 AI 任务".into());
            }
            for aid in [&analysis_agent, &localization_agent] {
                preset(aid)?;
            }
            {
                let mut d = w.data.lock().unwrap();
                project_available(&d.root)?;
                live_round(&d, &round_id)?;
                let r = round_mut(&mut d, &round_id)?;
                if r.decision != "pending" || !r.runs.is_empty() {
                    return Err("本轮已修改或结束，请继续复测或进入下一轮".into());
                }
                r.workflow = Progress {
                    stage: "performance".into(),
                    status: "running".into(),
                    reason: None,
                    analysis_agent: analysis_agent.clone(),
                    localization_agent: localization_agent.clone(),
                };
                w.save(&d)?;
            }
            w.cancelled.store(false, Ordering::SeqCst);
            w.busy.store(true, Ordering::SeqCst);
            commands::ACTIVE.store(true, Ordering::SeqCst);
            let work = w.clone();
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let result = execute(
                    work.clone(),
                    round_id.clone(),
                    analysis_agent,
                    localization_agent,
                    restart,
                    executable,
                )
                .await;
                if let Err(error) = result {
                    let mut d = work.data.lock().unwrap();
                    if let Ok(r) = round_mut(&mut d, &round_id) {
                        r.workflow.status = if work.cancelled.load(Ordering::SeqCst) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        r.workflow.reason = Some(error.clone());
                        for report in r.reports.iter_mut().filter(|p| p.status == "running") {
                            let event = DiagnoseEvent::Error {
                                message: error.clone(),
                            };
                            let _ = archive::append(
                                &work.directory,
                                &report.report_id,
                                &json!({"event":event,"offset":report.text.len()}),
                            );
                            report.apply(&event);
                        }
                        // Keep the in-memory error visible even when the disk is full.
                    }
                    let _ = work.save(&d);
                }
                work.busy.store(false, Ordering::SeqCst);
                commands::ACTIVE.store(false, Ordering::SeqCst);
            });
        }
        Action::Next => next_round(&w)?,
        Action::Relocate { capture_id, path } => {
            let p = path.canonicalize().map_err(|e| e.to_string())?;
            let check = p.clone();
            let h = tokio::task::spawn_blocking(move || {
                storage::file_hash(&check, &AtomicBool::new(false))
            })
            .await
            .map_err(|e| e.to_string())??;
            let mut d = w.data.lock().unwrap();
            let c = d
                .captures
                .iter_mut()
                .find(|c| c.id == capture_id)
                .ok_or("录制不存在")?;
            if h != c.hash {
                return Err("文件指纹不同，不能替代原录制".into());
            }
            c.path = p;
            w.save(&d)?;
        }
        _ => return Err("流程操作无效".into()),
    }
    Ok(commands::view(&w))
}
fn preset(id: &str) -> Result<acp_client::agents::AgentPreset, String> {
    let p = acp_client::agents::builtin_presets()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("AI 不存在")?;
    if !acp_client::agents::probe_available(&p.command) {
        return Err(format!("所选 AI {id} 不可用，不会自动替换"));
    }
    Ok(p)
}

pub async fn execute(
    w: Arc<Workspace>,
    rid: String,
    analysis: String,
    localization: String,
    restart: bool,
    executable: PathBuf,
) -> Result<(), String> {
    let (path, expected, root, snapshot, parent) = {
        let d = w.data.lock().unwrap();
        let r = d.rounds.iter().find(|r| r.id == rid).ok_or("轮次不存在")?;
        let c = d
            .captures
            .iter()
            .find(|c| c.id == r.baseline)
            .ok_or("基线不存在")?;
        let parent = if restart {
            None
        } else {
            r.reports
                .iter()
                .rev()
                .find(|p| p.stage == "performance")
                .filter(|p| p.status == "completed")
                .cloned()
        };
        (
            c.path.clone(),
            c.hash.clone(),
            d.root.clone(),
            c.snapshot.clone(),
            parent,
        )
    };
    let parse_progress = observation::ParseProgress::new(w.clone(), rid.clone(), id());
    let parsed: Result<crate::parser::ParsedProfile, String> = async {
        if parse_progress.hash(path.clone()).await? != expected {
            return Err("原录制已变化，请重新定位同指纹文件".into());
        }
        w.check()?;
        let p = parse_progress.parse(path.clone()).await?;
        if parse_progress.hash(path).await? != expected {
            return Err("解析期间录制发生变化".into());
        }
        Ok(p)
    }
    .await;
    let profile = match parsed {
        Ok(p) => p,
        Err(e) => {
            parse_progress.fail(&e);
            return Err(e);
        }
    };
    parse_progress.update("parse", None, None, "completed", None);
    let file_id = id();
    let details = profile.details;
    let parent = match parent {
        Some(p) => p,
        None => {
            run_report(
                w.clone(),
                &rid,
                &file_id,
                &analysis,
                &snapshot,
                details.clone(),
                None,
                None,
                executable.clone(),
            )
            .await?
        }
    };
    w.check()?;
    progress(&w, &rid, "project", "running", None)?;
    let flag = w.cancelled.clone();
    let fid = file_id.clone();
    let mut project =
        tokio::task::spawn_blocking(move || crate::project::ProjectScope::prepare(fid, root, flag))
            .await
            .map_err(|e| e.to_string())??;
    w.check()?;
    let status = crate::project::editor::status(Path::new(&project.info.root), &w.cancelled).await;
    *project.editor_status.lock().unwrap() = status.clone();
    let offline_reason = (status.status != "ready").then(|| {
        "已完成离线工程定位；未取得 Editor 实时场景、资源和导入信息，详见报告中的证据范围。"
            .to_string()
    });
    project.info.editor = status;
    // ACP closing its read scope must not cancel the workflow's own flag.
    project.cancelled = Arc::new(AtomicBool::new(false));
    run_report(
        w.clone(),
        &rid,
        &file_id,
        &localization,
        &snapshot,
        details,
        Some(parent),
        Some(Arc::new(project)),
        executable,
    )
    .await?;
    progress(&w, &rid, "ready", "completed", offline_reason)
}
async fn run_report(
    w: Arc<Workspace>,
    rid: &str,
    fid: &str,
    agent: &str,
    snapshot: &crate::extractor::MetricsSnapshot,
    details: Option<Arc<crate::parser::detail::FrameStore>>,
    parent: Option<Report>,
    project: Option<Arc<crate::project::ProjectScope>>,
    executable: PathBuf,
) -> Result<Report, String> {
    w.check()?;
    let mut report = Report::new(
        id(),
        fid.into(),
        agent.into(),
        parent.as_ref().map(|p| p.report_id.clone()),
        snapshot,
    );
    if let Some(p) = &project {
        report.stage = "project".into();
        report.project_context = Some(p.context());
    }
    {
        let mut d = w.data.lock().unwrap();
        let r = round_mut(&mut d, rid)?;
        r.workflow.stage = report.stage.clone();
        r.reports.push(report.clone());
        w.save(&d)?;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let request = DiagnoseRequest {
        project: project.clone(),
        source: None,
        parent_report: parent.map(|p| p.markdown()),
        file_id: fid.into(),
        agent_id: agent.into(),
        snapshot: snapshot.clone(),
        details,
        bridge_executable: executable,
        event_tx: tx,
    };
    let handle = match acp_client::start_diagnose(preset(agent)?, request).await {
        Ok(h) => h,
        Err(e) => {
            record_event(
                &w,
                rid,
                &mut report,
                DiagnoseEvent::Error {
                    message: e.to_string(),
                },
                None,
            )?;
            return Err(e.to_string());
        }
    };
    loop {
        let event = tokio::select! {
            event=rx.recv()=>event.unwrap_or(DiagnoseEvent::Error{message:"AI 流中断，请继续此阶段".into()}),
            _=tokio::time::sleep(std::time::Duration::from_millis(150))=>{
                if w.cancelled.load(Ordering::SeqCst){handle.cancel().await;DiagnoseEvent::Cancelled}else{continue}
            }
        };
        let terminal = event.terminal();
        let context = if terminal {
            project.as_ref().map(|p| p.context())
        } else {
            None
        };
        if let Err(e) = record_event(&w, rid, &mut report, event, context) {
            handle.cancel().await;
            return Err(e);
        }
        if report.status != "running" || terminal {
            if report.status != "completed" {
                handle.cancel().await;
                return Err(report
                    .incomplete_reason
                    .clone()
                    .unwrap_or("报告不完整".into()));
            }
            break;
        }
    }
    Ok(report)
}
fn record_event(
    w: &Workspace,
    rid: &str,
    report: &mut Report,
    event: DiagnoseEvent,
    context: Option<Value>,
) -> Result<(), String> {
    let pid = w.data.lock().unwrap().id.clone();
    if !matches!(
        event,
        DiagnoseEvent::Chunk { .. }
            | DiagnoseEvent::SessionCreated { .. }
            | DiagnoseEvent::Finished { .. }
            | DiagnoseEvent::Cancelled
            | DiagnoseEvent::Error { .. }
    ) {
        return w.activity(&pid, rid, &report.report_id, &event);
    }
    // Commit the event before exposing its contents to any polling client.
    let mut entry = json!({"event":event,"offset":report.text.len()});
    if let Some(c) = context {
        entry["context"] = c;
    }
    if let Err(e) = archive::append(&w.directory, &report.report_id, &entry) {
        *w.save_error.lock().unwrap() = Some(e.clone());
        return Err(e);
    }
    w.activity(&pid, rid, &report.report_id, &event)?;
    report.apply(&event);
    if let DiagnoseEvent::SessionCreated { acp_session_id } = &event {
        report.session_id = acp_session_id.clone();
    }
    if let Some(c) = entry.get("context") {
        report.project_context = Some(c.clone());
    }
    let mut d = w.data.lock().unwrap();
    let r = round_mut(&mut d, rid)?;
    let saved = r
        .reports
        .iter_mut()
        .find(|p| p.report_id == report.report_id)
        .ok_or("报告身份不匹配")?;
    *saved = report.clone();
    if report.status != "running" {
        if report.stage == "project" && report.status == "completed" {
            r.tasks = report
                .project_context
                .as_ref()
                .and_then(|c| serde_json::from_value(c["taskDrafts"].clone()).ok())
                .unwrap_or_default();
        }
        w.save(&d)?;
    }
    Ok(())
}
pub fn export_round(w: &Workspace, rid: &str) -> Result<String, String> {
    let d = w.data.lock().unwrap();
    let r = d.rounds.iter().find(|r| r.id == rid).ok_or("轮次不存在")?;
    let mut text = format!(
        "# {} · 本轮优化记录\n\n工程：{}\n\n玩法确认：{} · 决定：{}\n\n",
        d.name,
        d.root.display(),
        r.correctness,
        r.decision
    );
    for c in d
        .captures
        .iter()
        .filter(|c| c.id == r.baseline || Some(&c.id) == r.candidate.as_ref())
    {
        text+=&format!("## 录制 {}\n\n文件：{} · Unity {} · {} 帧\n\n指纹：{}\n\n复现条件：\n```json\n{}\n```\n",if c.id==r.baseline{"A"}else{"B"},c.path.display(),c.snapshot.meta.unity_version.as_deref().unwrap_or("未知"),c.snapshot.meta.frame_count,c.hash,serde_json::to_string_pretty(&c.conditions).unwrap());
    }
    for p in &r.reports {
        text.push_str(&p.markdown());
        text.push_str("\n\n");
    }
    for run in &r.runs {
        text += &format!(
            "## 优化 AI {}\n\n状态：{} · {}\n\n{}\n\n### 调查与任务\n\n",
            run.agent_id,
            run.status,
            run.reason.as_deref().unwrap_or(""),
            run.text
        );
        for t in &run.tasks {
            text += &format!(
                "- {}（{}）：{}；验收：{}\n",
                t.title, t.kind, t.evidence, t.acceptance
            );
        }
        text += "\n### 实际文件变化\n\n";
        for c in &run.changes {
            text += &format!(
                "- {} · {} · {} · {} → {}\n",
                c.path, c.kind, c.state, c.before_hash, c.after_hash
            );
        }
        text += &format!(
            "\n### 检查\n\n```json\n{}\n```\n",
            serde_json::to_string_pretty(&run.checks).unwrap()
        );
    }
    text+=&format!("\n## A/B 复验\n\n```json\n{}\n```\n\n编译通过不等于性能改善；游戏操作、Profiler 重录与玩法确认由用户完成。未附带完整源码和原始录制。\n",serde_json::to_string_pretty(&r.comparison).unwrap());
    Ok(text)
}
