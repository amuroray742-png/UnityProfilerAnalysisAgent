use super::*;
use crate::acp_client::{self, client::SessionHandle, DiagnoseEvent, DiagnoseRequest};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
use tauri::Manager;
#[derive(Default)]
pub struct OptimizationState {
    pub operation: tokio::sync::Mutex<()>,
    pub workspace: tokio::sync::Mutex<Option<Arc<Workspace>>>,
    pub active: tokio::sync::Mutex<Option<SessionHandle>>,
}
pub(super) static ACTIVE: AtomicBool = AtomicBool::new(false);
pub static START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}
struct StartingRun {
    workspace: Arc<Workspace>,
    run: String,
    armed: bool,
}
impl Drop for StartingRun {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.workspace.event(
                &self.run,
                &DiagnoseEvent::Error {
                    message: "修改会话启动中断，已保留记录，可重试".into(),
                },
            );
            self.workspace.busy.store(false, Ordering::SeqCst);
            ACTIVE.store(false, Ordering::SeqCst);
        }
    }
}
#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    PrepareAutomatic {
        file_id: String,
    },
    StartAutomatic {
        round_id: String,
        agent_id: String,
        requirements: String,
    },
    Create {
        directory: PathBuf,
        root: PathBuf,
        name: String,
    },
    Open {
        directory: PathBuf,
    },
    Get,
    Close,
    Adopt {
        file_id: String,
    },
    Import {
        path: PathBuf,
        conditions: Conditions,
    },
    Update {
        round_id: String,
        tasks: Vec<Task>,
        budgets: BTreeMap<String, f64>,
        conditions: BTreeMap<String, Conditions>,
        tests: Vec<String>,
    },
    Start {
        round_id: String,
        agent_id: String,
    },
    Cancel,
    Check {
        round_id: String,
    },
    Rollback {
        round_id: String,
    },
    Compare {
        round_id: String,
        candidate_id: String,
        range_a: Option<[usize; 2]>,
        range_b: Option<[usize; 2]>,
        confirmed: bool,
    },
    Decide {
        round_id: String,
        decision: String,
        correctness: String,
    },
    Next {
        baseline_id: String,
    },
    Detail {
        round_id: String,
        run_id: String,
        start: usize,
    },
    VerifyTask {
        round_id: String,
        task_id: String,
        status: String,
    },
    Hotspots {
        round_id: String,
        start: usize,
    },
    Export {
        path: PathBuf,
        format: String,
    },
}
fn compact_comparison(value: &Option<Value>) -> Value {
    let Some(mut v) = value.clone() else {
        return Value::Null;
    };
    if let Some(rows) = v["hotspots"]["rows"].as_array_mut() {
        let total = rows.len();
        rows.truncate(20);
        v["hotspots"]["total"] = json!(total);
        v["hotspots"]["nextStart"] = json!((total > 20).then_some(20));
    }
    v
}
fn run_view(s: &Run) -> Value {
    json!({"id":s.id,"automatic":s.automatic,"tasks":s.tasks,"text":s.text.chars().take(4000).collect::<String>(),"textPartial":s.text.chars().count()>4000,"requirements":s.requirements,"taskVersion":s.task_version,"agentId":s.agent_id,"sessionId":s.session_id,"status":s.status,"reason":s.reason,"createdAt":s.created_at,"checks":s.checks,"baselineCheck":s.baseline_check,"changes":s.changes.iter().map(|c|json!({"path":c.path,"kind":c.kind,"taskId":c.task_id,"beforeHash":c.before_hash,"afterHash":c.after_hash,"state":c.state})).collect::<Vec<_>>()})
}
pub(super) fn view(w: &Workspace) -> Value {
    let d = w.data.lock().unwrap();
    json!({"saveError":*w.save_error.lock().unwrap(),"version":d.version,"rootAvailable":super::workflow::project_available(&d.root).is_ok(),"id":d.id,"name":d.name,"root":d.root,"directory":w.directory,"budgets":d.budgets,"busy":w.busy.load(Ordering::SeqCst),
    "captures":d.captures.iter().map(|c|json!({"id":c.id,"path":c.path,"hash":c.hash,"conditions":c.conditions,"snapshot":c.snapshot.meta})).collect::<Vec<_>>(),
    "rounds":d.rounds.iter().map(|r|json!({"workflow":r.workflow,"performanceStatus":r.performance_status(),"id":r.id,"baseline":r.baseline,"candidate":r.candidate,"tasks":r.tasks,"taskVersion":r.task_version,"taskVerifications":r.task_verifications,"tests":r.tests,"comparison":compact_comparison(&r.comparison),"correctness":r.correctness,"decision":r.decision,
        "reports":r.reports.iter().enumerate().map(|(i,p)|json!({"attempt":r.reports[..=i].iter().filter(|q|q.stage==p.stage).count(),"reportId":p.report_id,"sessionId":p.session_id,"stage":p.stage,"agentId":p.agent_id,"status":p.status,"createdAt":p.created_at,"parentReportId":p.parent_report_id,"reason":p.incomplete_reason})).collect::<Vec<_>>(),
        "runs":r.runs.iter().map(run_view).collect::<Vec<_>>() })).collect::<Vec<_>>()})
}
pub(super) async fn capture(path: PathBuf, conditions: Conditions) -> Result<Capture, String> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let p = path.clone();
    let hash = tokio::task::spawn_blocking(move || storage::file_hash(&p, &AtomicBool::new(false)))
        .await
        .map_err(|e| e.to_string())??;
    let profile = crate::parser::parse_file(&path)
        .await
        .map_err(|e| e.to_string())?;
    let snapshot = crate::extractor::extract(&profile);
    let mut frames = profile.frames;
    for f in &mut frames {
        f.main_thread_samples.clear();
        f.gc_alloc_sites.clear();
        f.render_events.clear();
    }
    let p = path.clone();
    let after =
        tokio::task::spawn_blocking(move || storage::file_hash(&p, &AtomicBool::new(false)))
            .await
            .map_err(|e| e.to_string())??;
    if hash != after {
        return Err("解析期间录制文件变化".into());
    }
    Ok(Capture {
        id: id(),
        path,
        hash,
        conditions,
        snapshot,
        frames,
    })
}
#[tauri::command(rename_all = "camelCase")]
pub async fn optimization_command(
    mut action: Action,
    app: tauri::AppHandle,
    state: tauri::State<'_, OptimizationState>,
    app_state: tauri::State<'_, crate::state::AppState>,
) -> Result<Value, String> {
    let _operation = if matches!(action, Action::Get | Action::Cancel) {
        None
    } else {
        Some(state.operation.lock().await)
    };
    if let Action::PrepareAutomatic { file_id } = &action {
        let file_id = file_id.clone();
        let (root, report_id) = {
            let inner = app_state.0.lock().await;
            if !inner.active_sessions.is_empty() {
                return Err("请等待定位结束".into());
            }
            let report = inner
                .reports
                .values()
                .filter(|r| r.file_id == file_id && r.stage == "project" && r.status == "completed")
                .max_by_key(|r| &r.created_at)
                .ok_or("请先完成工程定位")?;
            let root = report
                .project_context
                .as_ref()
                .and_then(|c| c["project"]["root"].as_str())
                .ok_or("工程身份缺失")?;
            (
                PathBuf::from(root)
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
                report.report_id.clone(),
            )
        };
        let mut current = state.workspace.lock().await;
        if current
            .as_ref()
            .is_some_and(|w| w.busy.load(Ordering::SeqCst))
        {
            return Err("优化正在运行".into());
        }
        if current
            .as_ref()
            .is_some_and(|w| w.data.lock().unwrap().root != root)
        {
            *current = None;
        }
        if current.is_none() {
            let directory = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("optimization-projects")
                .join(storage::hash(
                    root.to_string_lossy().to_lowercase().as_bytes(),
                ));
            let w = tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                if directory.join("optimization.json").exists() {
                    Workspace::open(directory)
                } else {
                    let name = root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    Workspace::create(directory, root, name)
                }
            })
            .await
            .map_err(|e| e.to_string())??;
            *current = Some(Arc::new(w));
        }
        let w = current.as_ref().unwrap();
        if w.data
            .lock()
            .unwrap()
            .rounds
            .last()
            .is_some_and(|r| r.reports.iter().any(|p| p.report_id == report_id))
        {
            return Ok(view(w));
        }
        drop(current);
        action = Action::Adopt { file_id };
    }
    match action {
        Action::Create {
            directory,
            root,
            name,
        } => {
            let mut current = state.workspace.lock().await;
            if current.is_some() {
                return Err("请先关闭当前优化项目".into());
            }
            let w = tokio::task::spawn_blocking(move || Workspace::create(directory, root, name))
                .await
                .map_err(|e| e.to_string())??;
            let v = view(&w);
            *current = Some(Arc::new(w));
            return Ok(v);
        }
        Action::Open { directory } => {
            let mut current = state.workspace.lock().await;
            if current.is_some() {
                return Err("请先关闭当前优化项目".into());
            }
            let w = tokio::task::spawn_blocking(move || Workspace::open(directory))
                .await
                .map_err(|e| e.to_string())??;
            let v = view(&w);
            *current = Some(Arc::new(w));
            return Ok(v);
        }
        _ => {}
    }
    let w = state.workspace.lock().await.clone();
    if matches!(action, Action::Get) && w.is_none() {
        return Ok(Value::Null);
    }
    let w = w.ok_or("请创建或打开优化项目")?;
    if matches!(action, Action::Cancel) {
        w.cancel_epoch.fetch_add(1, Ordering::SeqCst);
        w.cancelled.store(true, Ordering::SeqCst);
        if let Some(h) = state.active.lock().await.clone() {
            h.cancel().await;
        }
        return Ok(view(&w));
    }
    if matches!(action, Action::Get) {
        return Ok(view(&w));
    }
    if w.busy.load(Ordering::SeqCst) {
        return Err("修改中只能查看或取消".into());
    }
    match action {
        Action::Close => {
            *state.workspace.lock().await = None;
            return Ok(Value::Null);
        }
        Action::Adopt { file_id } => {
            let (path, reports, capture_hash) = {
                let s = app_state.0.lock().await;
                if !s.active_sessions.is_empty() {
                    return Err("请等待当前诊断结束".into());
                }
                let path = s
                    .uploads
                    .get(&file_id)
                    .ok_or("当前录制已释放")?
                    .file_path
                    .clone();
                let reports: Vec<_> = s
                    .reports
                    .values()
                    .filter(|r| r.file_id == file_id)
                    .cloned()
                    .collect();
                (
                    path,
                    reports,
                    s.capture_hashes
                        .get(&file_id)
                        .cloned()
                        .ok_or("导入指纹缺失，请重新导入并诊断")?,
                )
            };
            let localization = reports
                .iter()
                .filter(|r| r.stage == "project" && r.status == "completed")
                .max_by_key(|r| &r.created_at)
                .ok_or("需要完成工程定位；旧报告需重新定位生成任务")?;
            let context = localization
                .project_context
                .as_ref()
                .ok_or("工程证据缺失")?;
            let root = PathBuf::from(context["project"]["root"].as_str().ok_or("工程身份缺失")?)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if root != w.data.lock().unwrap().root {
                return Err("定位报告属于其他工程".into());
            }
            let tasks: Vec<Task> =
                serde_json::from_value(context["taskDrafts"].clone()).unwrap_or_default();
            let c = capture(path, Conditions::default()).await?;
            if c.hash != capture_hash {
                return Err("录制已变化，拒绝将旧报告绑定新数据".into());
            }
            let mut d = w.data.lock().unwrap();
            let round = Round {
                workflow: Default::default(),
                task_version: 1,
                task_verifications: BTreeMap::new(),
                id: id(),
                baseline: c.id.clone(),
                candidate: None,
                reports,
                tasks,
                runs: vec![],
                tests: vec![],
                comparison: None,
                correctness: "pending".into(),
                decision: "pending".into(),
            };
            let pending = d
                .rounds
                .last()
                .filter(|r| r.reports.is_empty() && r.runs.is_empty())
                .and_then(|r| d.captures.iter().find(|a| a.id == r.baseline))
                .is_some_and(|a| a.hash == c.hash);
            if pending {
                let last = d.rounds.last_mut().unwrap();
                last.reports = round.reports;
                last.tasks = round.tasks;
            } else {
                d.captures.push(c);
                d.rounds.push(round);
            }
            w.save(&d)?;
        }
        Action::Import { path, conditions } => {
            let c = capture(path, conditions).await?;
            let mut d = w.data.lock().unwrap();
            d.captures.push(c);
            w.save(&d)?;
        }
        Action::Update {
            round_id,
            tasks,
            budgets,
            conditions,
            tests,
        } => {
            if tasks.len() > 20
                || tests.len() > 100
                || tests.iter().any(|s| s.is_empty() || s.len() > 300)
                || budgets.iter().any(|(k, v)| {
                    !comparison::METRICS.contains(&k.as_str()) || !v.is_finite() || *v < 0.
                })
            {
                return Err("任务/测试/预算无效".into());
            }
            let mut d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter_mut()
                .find(|r| r.id == round_id)
                .ok_or("轮次不存在")?;
            // Users may refine goals, not silently broaden a diagnosed file set.
            for t in &tasks {
                let original = r
                    .tasks
                    .iter()
                    .find(|old| old.id == t.id)
                    .ok_or("新任务需要重新定位")?;
                if t.files
                    .iter()
                    .any(|(path, hash)| original.files.get(path) != Some(hash))
                {
                    return Err("文件范围变化需要重新定位".into());
                }
            }
            if r.tasks
                .iter()
                .map(|t| serde_json::to_string(t).unwrap())
                .collect::<Vec<_>>()
                != tasks
                    .iter()
                    .map(|t| serde_json::to_string(t).unwrap())
                    .collect::<Vec<_>>()
            {
                r.task_version += 1;
                r.task_verifications.clear();
            }
            r.tasks = tasks;
            r.tests = tests;
            let condition_changes = conditions.iter().any(|(id, c)| {
                d.captures
                    .iter()
                    .find(|v| v.id == *id)
                    .is_some_and(|v| v.conditions != *c)
            });
            if condition_changes || d.budgets != budgets {
                // Older rounds keep their frozen comparison conditions and budgets.
                d.rounds
                    .iter_mut()
                    .find(|r| r.id == round_id)
                    .unwrap()
                    .comparison = None;
            }
            d.budgets = budgets;
            for (id, condition) in conditions {
                d.captures
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or("录制不存在")?
                    .conditions = condition;
            }
            w.save(&d)?;
        }
        Action::Start { .. } | Action::StartAutomatic { .. } => {
            let cancel_epoch = w.cancel_epoch.load(Ordering::SeqCst);
            super::workflow::project_available(&w.data.lock().unwrap().root)?;
            let (round_id, agent_id, requirements) = match action {
                Action::Start { round_id, agent_id } => (round_id, agent_id, None),
                Action::StartAutomatic {
                    round_id,
                    agent_id,
                    requirements,
                } => {
                    if requirements.len() > 16000 {
                        return Err("补充要求过长".into());
                    }
                    (round_id, agent_id, Some(requirements))
                }
                _ => unreachable!(),
            };
            let automatic = requirements.is_some();
            let _start = START_LOCK.lock().await;
            if !app_state.0.lock().await.active_sessions.is_empty() {
                return Err("请先结束诊断".into());
            }
            if !["codex", "claude-code"].contains(&agent_id.as_str()) {
                return Err("该适配器尚未验证受限修改能力，不自动替换 Agent".into());
            }
            let preset = acp_client::agents::builtin_presets()
                .into_iter()
                .find(|a| a.id == agent_id)
                .ok_or("Agent 不存在")?;
            if !acp_client::agents::probe_available(&preset.command) {
                return Err("所选修改 Agent 不可用".into());
            }
            let (snapshot, path, expected) = {
                let d = w.data.lock().unwrap();
                let r = d
                    .rounds
                    .iter()
                    .find(|r| r.id == round_id)
                    .ok_or("轮次不存在")?;
                if d.rounds.last().is_none_or(|last| last.id != round_id) || r.decision != "pending"
                {
                    return Err("只能优化当前未结束轮次；请先开始下一轮".into());
                }
                let c = d
                    .captures
                    .iter()
                    .find(|c| c.id == r.baseline)
                    .ok_or("基线不存在")?;
                (c.snapshot.clone(), c.path.clone(), c.hash.clone())
            };
            if storage::file_hash(&path, &AtomicBool::new(false))? != expected {
                return Err("基线文件发生变化".into());
            }
            let details = crate::parser::parse_file(&path)
                .await
                .map_err(|e| e.to_string())?
                .details;
            if w.cancel_epoch.load(Ordering::SeqCst) != cancel_epoch {
                return Err("已停止启动优化，尚未修改代码".into());
            }
            if ACTIVE.swap(true, Ordering::SeqCst) {
                return Err("已有修改运行".into());
            }
            let run = match w.begin_mode(&round_id, agent_id.clone(), requirements) {
                Ok(id) => id,
                Err(e) => {
                    ACTIVE.store(false, Ordering::SeqCst);
                    return Err(e);
                }
            };
            if w.cancel_epoch.load(Ordering::SeqCst) != cancel_epoch {
                w.cancelled.store(true, Ordering::SeqCst);
            }
            let mut startup = StartingRun {
                workspace: w.clone(),
                run: run.clone(),
                armed: true,
            };
            let scope = Arc::new(super::session::EditScope {
                operation: tokio::sync::Mutex::new(()),
                workspace: w.clone(),
                run_id: run.clone(),
            });
            let root = w.data.lock().unwrap().root.clone();
            let baseline =
                crate::project::editor::check_changes(&root, vec![], vec![], &w.cancelled)
                    .await
                    .unwrap_or_else(|e| json!({"status":"unavailable","reason":e}));
            {
                let mut d = w.data.lock().unwrap();
                let r = d
                    .rounds
                    .iter_mut()
                    .flat_map(|r| &mut r.runs)
                    .find(|r| r.id == run)
                    .unwrap();
                r.baseline_check = Some(baseline);
                w.save(&d)?;
            }
            if w.cancelled.load(Ordering::SeqCst) {
                w.event(&run, &DiagnoseEvent::Cancelled)?;
                ACTIVE.store(false, Ordering::SeqCst);
                return Ok(view(&w));
            }
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let project = if automatic {
                let root = root.clone();
                let file_id = run.clone();
                let cancelled = w.cancelled.clone();
                let prepared = tokio::task::spawn_blocking(move || {
                    crate::project::ProjectScope::prepare(file_id, root, cancelled)
                })
                .await
                .map_err(|e| e.to_string())?;
                if w.cancelled.load(Ordering::SeqCst) {
                    w.event(&run, &DiagnoseEvent::Cancelled)?;
                    return Ok(view(&w));
                }
                let mut project = prepared?;
                // Scanning follows the user's cancellation; closing the read-only ACP scope
                // must not cancel the workspace or block subsequent manual checks/rollback.
                project.cancelled = Arc::new(AtomicBool::new(false));
                Some(Arc::new(project))
            } else {
                None
            };
            let request = DiagnoseRequest {
                project,
                source: None,
                parent_report: None,
                file_id: run.clone(),
                agent_id,
                snapshot,
                details,
                bridge_executable: std::env::current_exe().map_err(|e| e.to_string())?,
                event_tx: tx,
            };
            let handle =
                match acp_client::start_with_scope(preset, request, Some(scope.clone())).await {
                    Ok(h) => h,
                    Err(e) => {
                        w.event(
                            &run,
                            &DiagnoseEvent::Error {
                                message: e.to_string(),
                            },
                        )?;
                        ACTIVE.store(false, Ordering::SeqCst);
                        return Err(e.to_string());
                    }
                };
            {
                let mut active = state.active.lock().await;
                *active = Some(handle.clone());
            }
            if w.cancelled.load(Ordering::SeqCst) {
                handle.cancel().await;
            }
            let work = w.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let terminal = event.terminal();
                    if terminal && !work.cancelled.load(Ordering::SeqCst) {
                        let need = {
                            let d = work.data.lock().unwrap();
                            let r = d
                                .rounds
                                .iter()
                                .flat_map(|r| &r.runs)
                                .find(|r| r.id == run)
                                .unwrap();
                            !r.changes.is_empty()
                                && r.checks.last().and_then(|c| c["checkedRevision"].as_u64())
                                    != Some(r.changes.len() as u64)
                                && r.checks.len() < 3
                        };
                        if need {
                            let _ = scope.check_editor().await;
                        }
                    }
                    if let Err(error) = work.event(&run, &event) {
                        work.cancelled.store(true, Ordering::SeqCst);
                        {
                            let mut d = work.data.lock().unwrap();
                            if let Some(r) = d
                                .rounds
                                .iter_mut()
                                .flat_map(|r| &mut r.runs)
                                .find(|r| r.id == run)
                            {
                                r.reason=Some(format!("保存优化记录失败，已停止：{error}。已有磁盘备份保留，请检查保存目录。"));
                                r.status = if r.changes.iter().any(|c| c.state == "applied") {
                                    "partial"
                                } else {
                                    "failed"
                                }
                                .into();
                            }
                        }
                        handle.cancel().await;
                        break;
                    }
                    if terminal {
                        break;
                    }
                    if work.cancelled.load(Ordering::SeqCst) {
                        handle.cancel().await;
                    }
                }
                work.busy.store(false, Ordering::SeqCst);
                ACTIVE.store(false, Ordering::SeqCst);
            });
            startup.armed = false;
        }
        Action::Check { round_id } => {
            super::workflow::project_available(&w.data.lock().unwrap().root)?;
            let _start = START_LOCK.lock().await;
            if !app_state.0.lock().await.active_sessions.is_empty()
                || ACTIVE.swap(true, Ordering::SeqCst)
            {
                return Err("请先结束当前诊断或修改".into());
            }
            struct CheckGuard(Arc<Workspace>);
            impl Drop for CheckGuard {
                fn drop(&mut self) {
                    self.0.busy.store(false, Ordering::SeqCst);
                    ACTIVE.store(false, Ordering::SeqCst);
                }
            }
            let _guard = CheckGuard(w.clone());
            w.cancelled.store(false, Ordering::SeqCst);
            w.busy.store(true, Ordering::SeqCst);
            let run_id = {
                let d = w.data.lock().unwrap();
                let round = d
                    .rounds
                    .iter()
                    .find(|r| r.id == round_id)
                    .ok_or("轮次不存在")?;
                let run = round.runs.last().ok_or("还没有修改记录")?;
                if run.status == "rolled_back" {
                    return Err("本轮已回退，请对下一次修改执行检查".into());
                }
                run.id.clone()
            };
            let scope = super::session::EditScope {
                operation: tokio::sync::Mutex::new(()),
                workspace: w.clone(),
                run_id,
            };
            scope.check_editor().await?;
        }
        Action::Rollback { round_id } => {
            super::workflow::project_available(&w.data.lock().unwrap().root)?;
            let work = w.clone();
            tokio::task::spawn_blocking(move || {
                work.rollback(&round_id)?;
                let mut d = work.data.lock().unwrap();
                let r = d
                    .rounds
                    .iter_mut()
                    .find(|r| r.id == round_id)
                    .ok_or("轮次不存在")?;
                r.decision = "rolled_back".into();
                work.save(&d)
            })
            .await
            .map_err(|e| e.to_string())??;
        }
        Action::Compare {
            round_id,
            candidate_id,
            range_a,
            range_b,
            confirmed,
        } => {
            let (a, b, budgets) = {
                let d = w.data.lock().unwrap();
                let r = d
                    .rounds
                    .iter()
                    .find(|r| r.id == round_id)
                    .ok_or("轮次不存在")?;
                let a = d
                    .captures
                    .iter()
                    .find(|c| c.id == r.baseline)
                    .ok_or("基线不存在")?;
                let b = d
                    .captures
                    .iter()
                    .find(|c| c.id == candidate_id)
                    .ok_or("B 不存在")?;
                if a.id == b.id {
                    return Err("请选择另一份录制".into());
                }
                (a.clone(), b.clone(), d.budgets.clone())
            };
            let mut result = comparison::compare(&a, &b, range_a, range_b, confirmed, &budgets)?;
            result["hotspots"] = match (
                comparison::paths(&a, range_a).await,
                comparison::paths(&b, range_b).await,
            ) {
                (Ok(a), Ok(b)) => comparison::associate(a, b),
                (a, b) => {
                    json!({"status":"unavailable","reason":format!("{:?}; {:?}",a.err(),b.err())})
                }
            };
            let mut d = w.data.lock().unwrap();
            let r = d.rounds.iter_mut().find(|r| r.id == round_id).unwrap();
            r.candidate = Some(candidate_id);
            if r.tasks.iter().any(|t| t.selected)
                && r.tasks
                    .iter()
                    .filter(|t| t.selected)
                    .all(|t| t.kind == "marker")
            {
                result["taskOutcome"] =
                    json!("纯 Marker 轮次：只验收新采样及调查问题，不按性能预算宣称优化完成");
            }
            r.comparison = Some(result);
            w.save(&d)?;
        }
        Action::Decide {
            round_id,
            decision,
            correctness,
        } => {
            if !["accepted", "continue", "pending"].contains(&decision.as_str())
                || !["pending", "passed", "problem"].contains(&correctness.as_str())
            {
                return Err("状态无效".into());
            }
            let mut d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter_mut()
                .find(|r| r.id == round_id)
                .ok_or("轮次不存在")?;
            if decision == "accepted"
                && (r.candidate.is_none()
                    || r.comparison.is_none()
                    || r.runs.is_empty()
                    || r.runs.iter().all(|s| s.status == "rolled_back"))
            {
                return Err("请先完成修改并导入 B 对比；已回退不能接受".into());
            }
            r.decision = decision;
            r.correctness = correctness;
            w.save(&d)?;
        }
        Action::Next { baseline_id } => {
            let mut d = w.data.lock().unwrap();
            if !d.captures.iter().any(|c| c.id == baseline_id) {
                return Err("基线不存在".into());
            }
            d.rounds.push(Round {
                workflow: Default::default(),
                task_version: 1,
                task_verifications: BTreeMap::new(),
                id: id(),
                baseline: baseline_id,
                candidate: None,
                reports: vec![],
                tasks: vec![],
                runs: vec![],
                tests: vec![],
                comparison: None,
                correctness: "pending".into(),
                decision: "pending".into(),
            });
            w.save(&d)?;
        }
        Action::VerifyTask {
            round_id,
            task_id,
            status,
        } => {
            if !["pending", "passed", "problem"].contains(&status.as_str()) {
                return Err("任务验收状态无效".into());
            }
            let mut d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter_mut()
                .find(|r| r.id == round_id)
                .ok_or("轮次不存在")?;
            if !r.tasks.iter().any(|t| t.id == task_id) {
                return Err("任务不存在".into());
            }
            r.task_verifications.insert(task_id, status);
            w.save(&d)?;
        }
        Action::Hotspots { round_id, start } => {
            let d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter()
                .find(|r| r.id == round_id)
                .ok_or("轮次不存在")?;
            let rows = r
                .comparison
                .as_ref()
                .and_then(|v| v["hotspots"]["rows"].as_array())
                .ok_or("热点不可用")?;
            if start > rows.len() {
                return Err("分页越界".into());
            }
            let end = (start + 20).min(rows.len());
            return Ok(
                json!({"rows":rows[start..end],"total":rows.len(),"nextStart":(end<rows.len()).then_some(end)}),
            );
        }
        Action::Detail {
            round_id,
            run_id,
            start,
        } => {
            let d = w.data.lock().unwrap();
            let r = d
                .rounds
                .iter()
                .find(|r| r.id == round_id)
                .ok_or("轮次不存在")?;
            let run = r.runs.iter().find(|r| r.id == run_id).ok_or("运行不存在")?;
            let mut data = serde_json::to_value(run).map_err(|e| e.to_string())?;
            for (row, change) in data["changes"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .zip(&run.changes)
            {
                row["before"] = json!(editing::decode(&change.before)?);
                row["after"] = json!(editing::decode(&change.after)?);
            }
            let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
            let chars: Vec<_> = text.chars().collect();
            if start > chars.len() {
                return Err("分页越界".into());
            }
            let end = (start + 12000).min(chars.len());
            return Ok(
                json!({"text":chars[start..end].iter().collect::<String>(),"nextStart":(end<chars.len()).then_some(end)}),
            );
        }
        Action::Export { path, format } => {
            let extension = if format == "html" { "html" } else { "md" };
            if path.extension().and_then(|e| e.to_str()) != Some(extension) {
                return Err("导出文件必须使用 .md / .html 扩展名".into());
            }
            if !["markdown", "html"].contains(&format.as_str()) {
                return Err("导出格式无效".into());
            }
            let d = w.data.lock().unwrap();
            let mut text = format!("# {} 优化记录\n\n工程：{}\n\n", d.name, d.root.display());
            for capture in &d.captures {
                text+=&format!("## 录制 {}\n\n文件：{} · SHA-256 {}\n\nUnity {} · {} 帧\n\n复现条件：\n```json\n{}\n```\n",capture.id,capture.path.display(),capture.hash,capture.snapshot.meta.unity_version.as_deref().unwrap_or("未知"),capture.frames.len(),serde_json::to_string_pretty(&capture.conditions).unwrap());
            }
            for r in &d.rounds {
                text += &format!(
                    "## 轮次 {}\n\n正确性：{}；用户决策：{}\n\n",
                    r.id, r.correctness, r.decision
                );
                text += &format!("性能状态：{}\n\n", r.performance_status());
                for p in &r.reports {
                    text += &p.markdown();
                }
                text += &format!("\n任务人工验收：{:?}\n", r.task_verifications);
                for run in &r.runs {
                    text += &format!(
                        "\n任务版本 {}：\n```json\n{}\n```\n",
                        run.task_version,
                        serde_json::to_string_pretty(&run.tasks).unwrap()
                    );
                    text += &format!(
                        "\n### 修改 Agent {} · 会话 {}\n\n状态：{}；原因：{}\n\n{}\n\n",
                        run.agent_id,
                        run.session_id,
                        run.status,
                        run.reason.as_deref().unwrap_or("无"),
                        run.text
                    );
                    for c in &run.changes {
                        text += &format!(
                            "\n变更类型：{} · {} · 任务 {}\n",
                            c.kind,
                            c.path,
                            c.task_id.as_deref().unwrap_or("旧记录未关联")
                        );
                        let diff = editing::diff(c)?;
                        let lines: Vec<_> = diff.lines().collect();
                        let preview = lines
                            .iter()
                            .take(200)
                            .copied()
                            .collect::<Vec<_>>()
                            .join("\n");
                        let fence = "`".repeat(
                            preview
                                .split(|c| c != '`')
                                .map(str::len)
                                .max()
                                .unwrap_or(0)
                                .max(3)
                                + 1,
                        );
                        text += &format!("\n{fence}diff\n{preview}\n{fence}\n");
                        if lines.len() > 200 {
                            text+="差异超过200行，导出仅预览；完整字节与逐次修改记录保存在优化项目中。\n";
                        }

                        text += &format!(
                            "- {}：{} → {}（{}）\n",
                            c.path, c.before_hash, c.after_hash, c.state
                        );
                    }
                    text += &format!(
                        "\n检查：\n```json\n{}\n```\n",
                        serde_json::to_string_pretty(&run.checks).unwrap()
                    );
                }
                text += &format!(
                    "\n复验：\n```json\n{}\n```\n",
                    serde_json::to_string_pretty(&r.comparison).unwrap()
                );
            }
            text+="\n编译通过不等于性能改善，性能差异不证明修改因果；游戏正确性及重录由用户确认。未默认附带完整源码或录制。\n";
            let bytes = if format == "html" {
                crate::reports::document(&text)
            } else {
                text
            };
            drop(d);
            storage::atomic(&path, bytes.as_bytes())?;
        }
        _ => return Err("操作无效".into()),
    }
    Ok(view(&w))
}
