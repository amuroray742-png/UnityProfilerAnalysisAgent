use super::*;
use crate::acp_client::{self, client::SessionHandle, DiagnoseEvent, DiagnoseRequest};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
#[derive(Default)]
pub struct OptimizationState {
    pub operation: tokio::sync::Mutex<()>,
    pub workspace: tokio::sync::Mutex<Option<Arc<Workspace>>>,
    pub active: tokio::sync::Mutex<Option<SessionHandle>>,
}
static ACTIVE: AtomicBool = AtomicBool::new(false);
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
fn view(w: &Workspace) -> Value {
    let d = w.data.lock().unwrap();
    json!({"id":d.id,"name":d.name,"root":d.root,"directory":w.directory,"budgets":d.budgets,"busy":w.busy.load(Ordering::SeqCst),
    "captures":d.captures.iter().map(|c|json!({"id":c.id,"path":c.path,"hash":c.hash,"conditions":c.conditions,"snapshot":c.snapshot.meta})).collect::<Vec<_>>(),
    "rounds":d.rounds.iter().map(|r|json!({"performanceStatus":r.performance_status(),"id":r.id,"baseline":r.baseline,"candidate":r.candidate,"tasks":r.tasks,"taskVersion":r.task_version,"taskVerifications":r.task_verifications,"tests":r.tests,"comparison":compact_comparison(&r.comparison),"correctness":r.correctness,"decision":r.decision,
        "reports":r.reports.iter().map(|p|json!({"reportId":p.report_id,"stage":p.stage,"agentId":p.agent_id,"status":p.status})).collect::<Vec<_>>(),
        "runs":r.runs.iter().map(|s|json!({"id":s.id,"taskVersion":s.task_version,"agentId":s.agent_id,"sessionId":s.session_id,"status":s.status,"reason":s.reason,"createdAt":s.created_at,"checks":s.checks,"baselineCheck":s.baseline_check,"changes":s.changes.iter().map(|c|json!({"path":c.path,"beforeHash":c.before_hash,"afterHash":c.after_hash,"state":c.state})).collect::<Vec<_>>()})).collect::<Vec<_>>() })).collect::<Vec<_>>()})
}
async fn capture(path: PathBuf, conditions: Conditions) -> Result<Capture, String> {
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
    action: Action,
    state: tauri::State<'_, OptimizationState>,
    app_state: tauri::State<'_, crate::state::AppState>,
) -> Result<Value, String> {
    let _operation = if matches!(action, Action::Get | Action::Cancel) {
        None
    } else {
        Some(state.operation.lock().await)
    };
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
                .find(|r| r.stage == "project" && r.status == "completed")
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
            let tasks: Vec<Task> = serde_json::from_value(context["taskDrafts"].clone())
                .map_err(|_| "旧定位报告没有结构化任务，请重新定位")?;
            let c = capture(path, Conditions::default()).await?;
            if c.hash != capture_hash {
                return Err("录制已变化，拒绝将旧报告绑定新数据".into());
            }
            let mut d = w.data.lock().unwrap();
            let round = Round {
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
        Action::Start { round_id, agent_id } => {
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
            if ACTIVE.swap(true, Ordering::SeqCst) {
                return Err("已有修改运行".into());
            }
            let run = match w.begin(&round_id, agent_id.clone()) {
                Ok(id) => id,
                Err(e) => {
                    ACTIVE.store(false, Ordering::SeqCst);
                    return Err(e);
                }
            };
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
            let request = DiagnoseRequest {
                project: None,
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
            *state.active.lock().await = Some(handle.clone());
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
            let work = w.clone();
            tokio::task::spawn_blocking(move || work.rollback(&round_id))
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
