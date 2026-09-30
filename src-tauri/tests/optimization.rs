//! Public, disposable projects only. Never point mutation tests at a user project.
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use unity_profiler_analysis_agent_lib::{
    acp_client::{self, DiagnoseEvent},
    extractor, mcp,
    optimization::*,
    parser,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("upaa-optimization-test-{}", id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn setup(bytes: &[u8]) -> (Temp, Arc<Workspace>, String) {
    let temp = Temp::new();
    let root = temp.0.join("project");
    let save = temp.0.join("records");
    std::fs::create_dir(&save).unwrap();
    for dir in ["Assets", "Packages", "ProjectSettings"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("Assets/Work.cs"), bytes).unwrap();
    let w = Arc::new(Workspace::create(save, root, "公开测试".into()).unwrap());
    let rid = id();
    w.data.lock().unwrap().rounds.push(Round {
        task_version: 1,
        task_verifications: BTreeMap::new(),
        id: rid.clone(),
        baseline: "a".into(),
        candidate: None,
        reports: vec![],
        tasks: vec![Task {
            id: "t".into(),
            kind: "optimize".into(),
            title: "公开已有代码替换".into(),
            evidence: "公开测试文件证据".into(),
            files: BTreeMap::from([("Assets/Work.cs".into(), storage::hash(bytes))]),
            instructions: "将 number = 1 改为 number = 2，不改其他行为".into(),
            acceptance: "按固定工具读取并替换；检查不可用时明确说明".into(),
            constraints: "不得创建或删除任何文件，仅修改已批准 Work.cs".into(),
            selected: true,
        }],
        runs: vec![],
        tests: vec![],
        comparison: None,
        correctness: "pending".into(),
        decision: "pending".into(),
    });
    w.save(&w.data.lock().unwrap()).unwrap();
    (temp, w, rid)
}
fn replace(w: &Workspace, run: &str, old: &str, new: &str) -> Result<serde_json::Value, String> {
    let current = w.edit_query(run, "optimization_read", json!({"path":"Assets/Work.cs"}))?;
    w.edit_query(run,"optimization_replace",json!({"task_id":"t","path":"Assets/Work.cs","expected_hash":current["hash"],"old_text":old,"new_text":new}))
}
fn finish(w: &Workspace, run: &str) {
    w.event(
        run,
        &DiagnoseEvent::Finished {
            total_chunks: 0,
            stop_reason: "end_turn".into(),
        },
    )
    .unwrap();
}
#[test]
fn durable_cancel_restart_and_conflict_safe_rollback() {
    let original = b"// user uncommitted\r\nclass Work { int number = 1; }\r\n";
    let (_temp, w, r) = setup(original);
    let run = w.begin(&r, "one".into()).unwrap();
    replace(&w, &run, "number = 1", "number = 2").unwrap();
    w.cancelled.store(true, Ordering::SeqCst);
    assert!(replace(&w, &run, "number = 2", "number = 3").is_err());
    let save = w.directory.clone();
    let root = w.data.lock().unwrap().root.clone();
    drop(w);
    let reopened = Workspace::open(save).unwrap();
    assert_eq!(
        reopened.data.lock().unwrap().rounds[0].runs[0].status,
        "interrupted"
    );
    std::fs::write(root.join("Assets/Work.cs"), "external edit").unwrap();
    assert!(reopened.rollback(&r).unwrap_err().contains("外部"));
    assert_eq!(
        std::fs::read_to_string(root.join("Assets/Work.cs")).unwrap(),
        "external edit"
    );
    let after = reopened.data.lock().unwrap().rounds[0].runs[0].changes[0]
        .after
        .clone();
    std::fs::write(root.join("Assets/Work.cs"), after).unwrap();
    reopened.rollback(&r).unwrap();
    assert_eq!(
        std::fs::read(root.join("Assets/Work.cs")).unwrap(),
        original
    );
}
#[test]
fn utf16_and_new_sessions_preserve_authorization_and_current_edits() {
    let mut original = vec![255, 254];
    original.extend(
        "// 中文\r\nclass Work { int number = 1; }\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let (_t, w, r) = setup(&original);
    let first = w.begin(&r, "same-agent".into()).unwrap();
    replace(&w, &first, "number = 1", "number = 2").unwrap();
    finish(&w, &first);
    let second = w.begin(&r, "same-agent".into()).unwrap();
    assert_ne!(first, second);
    replace(&w, &second, "number = 2", "number = 3").unwrap();
    finish(&w, &second);
    w.rollback(&r).unwrap();
    assert_eq!(
        editing::read(&w.data.lock().unwrap().root, "Assets/Work.cs").unwrap(),
        original
    );
}
#[test]
fn unauthorized_paths_hashes_and_late_events_are_rejected() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    assert!(Workspace::open(w.directory.clone()).is_err());
    let run = w.begin(&r, "a".into()).unwrap();
    for path in [
        "../Outside.cs",
        "Assets/New.cs",
        "Assets/../Work.cs",
        "Assets/Work.cs.meta",
        "ProjectSettings/Work.cs",
    ] {
        assert!(w
            .edit_query(
                &run,
                "optimization_replace",
                json!({"path":path,"expected_hash":"x","old_text":"1","new_text":"2"})
            )
            .is_err());
    }
    assert!(w
        .edit_query(
            &run,
            "optimization_read",
            json!({"path":"Assets/Work.cs","unexpected":true})
        )
        .is_err());
    assert!(w.edit_query(&run,"optimization_replace",json!({"task_id":"t","path":"Assets/Work.cs","expected_hash":"wrong","old_text":"1","new_text":"2"})).is_err());
    finish(&w, &run);
    w.event(
        &run,
        &DiagnoseEvent::Chunk {
            text: "late".into(),
        },
    )
    .unwrap();
    assert!(w.data.lock().unwrap().rounds[0].runs[0].text.is_empty());
    assert!(replace(&w, &run, "1", "2").is_err());
}
#[test]
fn later_round_dependency_prevents_old_backup_overwrite() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    let run = w.begin(&r, "a".into()).unwrap();
    replace(&w, &run, "1", "2").unwrap();
    finish(&w, &run);
    let second = {
        let mut d = w.data.lock().unwrap();
        let mut next = d.rounds[0].clone();
        next.id = id();
        next.runs.clear();
        next.tasks[0].files.insert(
            "Assets/Work.cs".into(),
            storage::hash(b"class Work { int number = 2; }"),
        );
        let id = next.id.clone();
        d.rounds.push(next);
        id
    };
    let run = w.begin(&second, "b".into()).unwrap();
    replace(&w, &run, "2", "3").unwrap();
    finish(&w, &run);
    assert!(w.rollback(&r).unwrap_err().contains("后续"));
    w.rollback(&second).unwrap();
    w.rollback(&r).unwrap();
}
#[tokio::test]
async fn diagnostic_store_cannot_access_edit_tools() {
    let s = mcp::MetricsStore::new();
    assert!(mcp::tools::dispatch(&s, "optimization_context", json!({}))
        .await
        .is_err());
}
async fn session(real: Option<&str>) {
    let (_t, w, r) = setup(b"class Work { int number = 1; }\n");
    let profile = parser::parse_file(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-dump.json"),
    )
    .await
    .unwrap();
    let preset = if let Some(id) = real {
        acp_client::agents::builtin_presets()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    } else {
        acp_client::agents::AgentPreset {
            id: "fixture".into(),
            label: "fixture".into(),
            command: "node".into(),
            args: vec![
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/acp-agent.cjs")
                    .to_string_lossy()
                    .into(),
                "modification".into(),
            ],
            description: "public".into(),
            available: true,
        }
    };
    let run = w.begin(&r, preset.id.clone()).unwrap();
    let scope = Arc::new(session::EditScope {
        operation: tokio::sync::Mutex::new(()),
        workspace: w.clone(),
        run_id: run.clone(),
    });
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let request = acp_client::DiagnoseRequest {
        project: None,
        source: None,
        parent_report: None,
        file_id: "a".into(),
        agent_id: preset.id.clone(),
        snapshot: extractor::extract(&profile),
        details: profile.details,
        bridge_executable: std::env::var_os("UPAA_TEST_APP_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_unity-profiler-analysis-agent"))),
        event_tx: tx,
    };
    let handle = acp_client::start_with_scope(preset, request, Some(scope))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(300), async {
        while let Some(event) = rx.recv().await {
            if let DiagnoseEvent::Log { message } = &event {
                eprintln!("{}", message.chars().take(500).collect::<String>());
            }
            let end = event.terminal();
            w.event(&run, &event).unwrap();
            if end {
                assert!(matches!(event, DiagnoseEvent::Finished { .. }), "{event:?}");
                break;
            }
        }
    })
    .await
    .unwrap();
    handle.wait().await;
    let root = w.data.lock().unwrap().root.clone();
    assert!(std::fs::read_to_string(root.join("Assets/Work.cs"))
        .unwrap()
        .contains("number = 2"));
    assert!(!w.data.lock().unwrap().rounds[0].runs[0].changes.is_empty());
    w.rollback(&r).unwrap();
    assert!(std::fs::read_to_string(root.join("Assets/Work.cs"))
        .unwrap()
        .contains("number = 1"));
}
#[tokio::test]
async fn modification_acp_mcp_uses_scoped_tools_and_real_file_ledger() {
    session(None).await;
}
#[tokio::test]
#[ignore = "explicit real Codex public mutation smoke"]
async fn real_codex_modification() {
    session(Some("codex")).await;
}
#[tokio::test]
#[ignore = "explicit real Claude public mutation smoke"]
async fn real_claude_modification() {
    session(Some("claude-code")).await;
}

#[tokio::test]
async fn project_tasks_require_actual_code_read_and_are_not_authorizations() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    let root = w.data.lock().unwrap().root.clone();
    std::fs::write(
        root.join("ProjectSettings/ProjectVersion.txt"),
        "m_EditorVersion: 6000.3.23f1\n",
    )
    .unwrap();
    let scope = Arc::new(
        unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
            "capture".into(),
            root,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let task = w.data.lock().unwrap().rounds[0].tasks[0].clone();
    assert!(scope
        .query("project_propose_tasks", json!({"tasks":[task.clone()]}))
        .await
        .is_err());
    scope
        .query("project_read", json!({"path":"Assets/Work.cs"}))
        .await
        .unwrap();
    scope
        .query("project_propose_tasks", json!({"tasks":[task]}))
        .await
        .unwrap();
    assert_eq!(scope.context()["taskDrafts"][0]["selected"], false);
    assert!(scope
        .query("optimization_replace", json!({}))
        .await
        .is_err());
    assert_eq!(w.data.lock().unwrap().rounds[0].id, r);
}
#[test]
fn marker_task_records_new_sampling_not_performance_success() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    w.data.lock().unwrap().rounds[0].tasks[0].kind = "marker".into();
    let run = w.begin(&r, "agent".into()).unwrap();
    replace(&w, &run, "number = 1", "number = 1; // Marker candidate").unwrap();
    finish(&w, &run);
    let d = w.data.lock().unwrap();
    assert_eq!(d.rounds[0].runs[0].tasks[0].kind, "marker");
    assert!(d.rounds[0].comparison.is_none());
    assert_eq!(d.rounds[0].correctness, "pending");
    assert_eq!(d.rounds[0].decision, "pending");
}
#[test]
fn contexts_page_without_losing_constraints_and_bound_revision_checks() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    w.data.lock().unwrap().rounds[0].tasks[0].constraints = "禁止改变".repeat(1000);
    let run = w.begin(&r, "agent".into()).unwrap();
    let mut start = 0;
    let mut text = String::new();
    loop {
        let page = w
            .edit_query(&run, "optimization_context", json!({"start":start}))
            .unwrap();
        text.push_str(page["text"].as_str().unwrap());
        if let Some(n) = page["nextStart"].as_u64() {
            start = n;
        } else {
            break;
        }
    }
    let all: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(all["runTasks"][0]["constraints"], "禁止改变".repeat(1000));
    assert!(start > 0);
    w.data.lock().unwrap().rounds[0].runs[0].checks = vec![json!({"status":"failed"}); 3];
    assert!(replace(&w, &run, "1", "2").unwrap_err().contains("3 次"));
}
#[tokio::test]
async fn cross_capture_paths_ignore_numeric_ids_and_do_not_pair_renamed_markers() {
    let t = Temp::new();
    let path_a = t.0.join("a.json");
    let path_b = t.0.join("b.json");
    let raw = include_str!("fixtures/editor-dump.json");
    std::fs::write(&path_a, raw).unwrap();
    let mut input: serde_json::Value = serde_json::from_str(raw).unwrap();
    for frame in input["frames"].as_array_mut().unwrap() {
        frame["frame_index"] = json!(frame["frame_index"].as_u64().unwrap() + 100);
        for thread in frame["threads"].as_array_mut().unwrap() {
            thread["thread_id"] = json!(thread["thread_id"].as_u64().unwrap() + 500);
            for sample in thread["samples"].as_array_mut().unwrap() {
                sample["marker_id"] = json!(sample["marker_id"].as_u64().unwrap() + 999);
                if sample["marker_name"] == "Work" {
                    sample["marker_name"] = json!("New.Marker");
                }
            }
        }
    }
    std::fs::write(&path_b, input.to_string()).unwrap();
    async fn capture(path: PathBuf) -> Capture {
        let p = parser::parse_file(&path).await.unwrap();
        Capture {
            id: id(),
            hash: storage::file_hash(&path, &std::sync::atomic::AtomicBool::new(false)).unwrap(),
            path,
            conditions: Conditions::default(),
            snapshot: extractor::extract(&p),
            frames: p.frames,
        }
    }
    let a = capture(path_a).await;
    let b = capture(path_b).await;
    let matches = comparison::associate(
        comparison::paths(&a, Some([10, 10])).await.unwrap(),
        comparison::paths(&b, Some([110, 110])).await.unwrap(),
    );
    let rows = matches["rows"].as_array().unwrap();
    let root = rows
        .iter()
        .find(|r| {
            r["roleAndPath"][1] == "Main Thread"
                && r["roleAndPath"][2].as_array().unwrap().len() == 1
        })
        .unwrap();
    assert_eq!(root["deltaInclusiveMsPerFrame"], 0.);
    let new = rows
        .iter()
        .find(|r| r["roleAndPath"].to_string().contains("New.Marker"))
        .unwrap();
    assert!(new["a"].is_null());
    assert!(new["deltaInclusiveMsPerFrame"].is_null());
    let old = rows
        .iter()
        .find(|r| r["roleAndPath"].to_string().contains("\"Work\""))
        .unwrap();
    assert!(old["b"].is_null());
}

#[test]
fn restart_reconciles_prepared_writes_and_new_runs_invalidate_old_verdicts() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    let run = w.begin(&r, "a".into()).unwrap();
    replace(&w, &run, "1", "2").unwrap();
    finish(&w, &run);
    {
        let mut d = w.data.lock().unwrap();
        let round = &mut d.rounds[0];
        round.runs[0].changes[0].state = "prepared".into();
        round.decision = "accepted".into();
        round.correctness = "passed".into();
        round.comparison = Some(json!({"result":"old"}));
        round.task_verifications.insert("t".into(), "passed".into());
        w.save(&d).unwrap();
    }
    let directory = w.directory.clone();
    drop(w);
    let w = Workspace::open(directory).unwrap();
    assert_eq!(
        w.data.lock().unwrap().rounds[0].runs[0].changes[0].state,
        "applied"
    );
    let retry = w.begin(&r, "b".into()).unwrap();
    assert_ne!(retry, run);
    let d = w.data.lock().unwrap();
    let round = &d.rounds[0];
    assert!(round.comparison.is_none());
    assert!(round.task_verifications.is_empty());
    assert_eq!(round.correctness, "pending");
    assert_eq!(round.decision, "pending");
}

#[cfg(windows)]
#[test]
fn junction_added_after_authorization_cannot_write_outside_project() {
    let (temp, w, r) = setup(b"class Work { int number = 1; }");
    let root = w.data.lock().unwrap().root.clone();
    let inside = root.join("Assets/Sub");
    std::fs::create_dir(&inside).unwrap();
    let original = b"class Nested { int number = 1; }";
    std::fs::write(inside.join("Nested.cs"), original).unwrap();
    w.data.lock().unwrap().rounds[0].tasks[0].files =
        BTreeMap::from([("Assets/Sub/Nested.cs".into(), storage::hash(original))]);
    let run = w.begin(&r, "a".into()).unwrap();
    let outside = temp.0.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("Nested.cs"), original).unwrap();
    std::fs::rename(&inside, root.join("Assets/OriginalSub")).unwrap();
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "New-Item -ItemType Junction -Path $env:UPAA_LINK -Target $env:UPAA_TARGET | Out-Null",
        ])
        .env("UPAA_LINK", &inside)
        .env("UPAA_TARGET", &outside)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(w.edit_query(&run,"optimization_replace",json!({"task_id":"t","path":"Assets/Sub/Nested.cs","expected_hash":storage::hash(original),"old_text":"1","new_text":"2"})).is_err());
    assert_eq!(std::fs::read(outside.join("Nested.cs")).unwrap(), original);
    // Remove only the verified temporary junction itself, never recurse through it.
    std::fs::remove_dir(&inside).unwrap();
}

#[cfg(windows)]
#[test]
fn durable_save_tolerates_short_reader_sharing_lock() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_t, w, _r) = setup(b"class Work { int number = 1; }");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(w.directory.join("optimization.json"))
        .unwrap();
    let reader = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(80));
        drop(file);
    });
    let d = w.data.lock().unwrap();
    w.save(&d).unwrap();
    reader.join().unwrap();
    let saved: Project =
        serde_json::from_slice(&std::fs::read(w.directory.join("optimization.json")).unwrap())
            .unwrap();
    assert_eq!(saved.id, d.id);
}

#[tokio::test]
async fn check_cannot_attribute_external_or_later_edits_to_old_run() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    let run = w.begin(&r, "a".into()).unwrap();
    replace(&w, &run, "1", "2").unwrap();
    finish(&w, &run);
    let root = w.data.lock().unwrap().root.clone();
    std::fs::write(root.join("Assets/Work.cs"), b"external later code").unwrap();
    let scope = session::EditScope {
        operation: tokio::sync::Mutex::new(()),
        workspace: w.clone(),
        run_id: run,
    };
    let result = scope.check_editor().await.unwrap();
    assert_eq!(result["status"], "unavailable");
    assert!(result["reason"].as_str().unwrap().contains("旧修改"));
    assert_eq!(
        std::fs::read(root.join("Assets/Work.cs")).unwrap(),
        b"external later code"
    );
}

#[test]
fn reports_are_loaded_on_demand_and_cannot_cross_rounds() {
    let (_t, w, r) = setup(b"class Work { int number = 1; }");
    let report:unity_profiler_analysis_agent_lib::reports::Report=serde_json::from_value(json!({"reportId":"report","fileId":"a","sessionId":"diagnostic","stage":"project","parentReportId":null,"text":"REPORT_BODY_SENTINEL中文".repeat(1000),"createdAt":"test","agentId":"other-agent","status":"completed","incompleteReason":null,"fileName":"public.json","unityVersion":null,"frameCount":1,"coverage":"test"})).unwrap();
    w.data.lock().unwrap().rounds[0]
        .reports
        .push(report.clone());
    let run = w.begin(&r, "new-agent".into()).unwrap();
    let index = w
        .edit_query(&run, "optimization_context", json!({}))
        .unwrap();
    assert!(!index["text"]
        .as_str()
        .unwrap()
        .contains("REPORT_BODY_SENTINEL"));
    assert!(index["text"].as_str().unwrap().contains("reportId"));
    assert!(w
        .edit_query(
            &run,
            "optimization_context",
            json!({"report_id":"other-round-report"})
        )
        .is_err());
    let mut text = String::new();
    let mut start = 0;
    loop {
        let page = w
            .edit_query(
                &run,
                "optimization_context",
                json!({"report_id":"report","start":start}),
            )
            .unwrap();
        text.push_str(page["text"].as_str().unwrap());
        if let Some(n) = page["nextStart"].as_u64() {
            start = n;
        } else {
            break;
        }
    }
    assert!(start > 0);
    let restored: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(restored["text"], report.text);
}

fn automatic_setup() -> (Temp, Arc<Workspace>, String, String) {
    let (t, w, r) = setup(b"class Work { int number = 1; }");
    let report=serde_json::from_value(json!({"reportId":"report","fileId":"a","sessionId":"diagnostic","stage":"project","parentReportId":null,"text":"Public evidence: investigate Work and its downstream code before optimizing","createdAt":"test","agentId":"other","status":"completed","incompleteReason":null,"fileName":"public.json","unityVersion":null,"frameCount":1,"coverage":"test"})).unwrap();
    w.data.lock().unwrap().rounds[0].reports.push(report);
    w.data.lock().unwrap().rounds[0].tasks.clear();
    let run = w
        .begin_mode(&r, "codex".into(), Some("保持玩法".into()))
        .unwrap();
    task(&w, &run, "optimize");
    (t, w, r, run)
}
fn task(w: &Workspace, run: &str, kind: &str) {
    w.edit_query(run,"optimization_task",json!({"id":"t","kind":kind,"title":"公开热点","evidence":"已读公开 Work.cs；GC 分配候选","instructions":"继续调查并减少分配","acceptance":"编译和重录"})).unwrap();
}
fn create(w: &Workspace, run: &str, path: &str) -> Result<serde_json::Value, String> {
    w.edit_query(run,"optimization_create",json!({"task_id":"t","path":path,"content":"public static class Helper { public const int Value = 2; }\n"}))
}
#[test]
fn automatic_without_drafts_investigates_and_edits_newly_read_code() {
    let (_t, w, r, run) = automatic_setup();
    let root = w.data.lock().unwrap().root.clone();
    std::fs::write(
        root.join("Assets/Downstream.cs"),
        b"class Downstream { int value = 1; }",
    )
    .unwrap();
    let args = json!({"task_id":"t","path":"Assets/Downstream.cs","expected_hash":storage::hash(b"class Downstream { int value = 1; }"),"old_text":"value = 1","new_text":"value = 2"});
    assert!(w
        .edit_query(&run, "optimization_replace", args.clone())
        .is_err());
    w.edit_query(
        &run,
        "optimization_read",
        json!({"path":"Assets/Downstream.cs"}),
    )
    .unwrap();
    task(&w, &run, "investigate");
    assert!(w
        .edit_query(&run, "optimization_replace", args.clone())
        .is_err());
    task(&w, &run, "optimize");
    w.edit_query(&run, "optimization_replace", args).unwrap();
    finish(&w, &run);
    assert_eq!(w.data.lock().unwrap().rounds[0].tasks[0].title, "公开热点");
    w.rollback(&r).unwrap();
    assert!(std::fs::read_to_string(root.join("Assets/Downstream.cs"))
        .unwrap()
        .contains("value = 1"));
}
#[test]
fn new_code_meta_directories_survive_restart_and_rollback_exactly() {
    let (_t, w, r, run) = automatic_setup();
    let root = w.data.lock().unwrap().root.clone();
    create(&w, &run, "Assets/中文/Generated/Helper.cs").unwrap();
    assert!(root.join("Assets/中文/Generated/Helper.cs.meta").exists());
    assert!(create(&w, &run, "Assets/中文/Generated/Helper.cs").is_err());
    let directory = w.directory.clone();
    drop(w);
    let w = Workspace::open(directory).unwrap();
    assert_eq!(
        w.data.lock().unwrap().rounds[0].runs[0].status,
        "interrupted"
    );
    w.rollback(&r).unwrap();
    assert!(!root.join("Assets/中文").exists());
    assert!(!root.join("Assets/中文.meta").exists());
    assert!(root.join("Assets/Work.cs").exists());
}
#[test]
fn new_file_external_change_and_external_reference_block_rollback() {
    let (_t, w, r, run) = automatic_setup();
    let root = w.data.lock().unwrap().root.clone();
    create(&w, &run, "Assets/CacheFile.cs").unwrap();
    finish(&w, &run);
    std::fs::write(
        root.join("Assets/Other.cs"),
        "class Other { int v = Helper.Value; }",
    )
    .unwrap();
    assert!(w.rollback(&r).unwrap_err().contains("引用"));
    assert!(root.join("Assets/CacheFile.cs").exists());
    std::fs::write(root.join("Assets/Other.cs"), "class Other {}").unwrap();
    std::fs::write(root.join("Assets/CacheFile.cs"), "external edit").unwrap();
    assert!(w.rollback(&r).unwrap_err().contains("外部"));
    assert_eq!(
        std::fs::read_to_string(root.join("Assets/CacheFile.cs")).unwrap(),
        "external edit"
    );
}
#[test]
fn restricted_and_automatic_permissions_do_not_expand_into_resources() {
    let (_t, w, r) = setup(b"class Work {}");
    let run = w.begin(&r, "a".into()).unwrap();
    assert!(create(&w, &run, "Assets/Helper.cs").is_err());
    assert!(w.edit_query(&run, "optimization_task", json!({})).is_err());
    let (_t, w, _r, run) = automatic_setup();
    for path in [
        "../Outside.cs",
        "Assets/../Outside.cs",
        "Packages/Helper.cs",
        "Assets/Test.prefab",
        "ProjectSettings/Test.cs",
        "Assets/a.cs:stream",
        "Assets/com.upaa.inspector/Test.cs",
    ] {
        assert!(create(&w, &run, path).is_err(), "{path}");
    }
    task(&w, &run, "investigate");
    assert!(create(&w, &run, "Assets/Helper.cs").is_err());
    task(&w, &run, "marker");
    w.cancelled.store(true, Ordering::SeqCst);
    assert!(create(&w, &run, "Assets/Helper.cs").is_err());
}
#[test]
fn version_one_migrates_with_original_backup() {
    let (_t, w, _r) = setup(b"class Work {}");
    let directory = w.directory.clone();
    {
        let mut d = w.data.lock().unwrap();
        d.version = 1;
        w.save(&d).unwrap();
    }
    let original = std::fs::read(directory.join("optimization.json")).unwrap();
    drop(w);
    let w = Workspace::open(directory.clone()).unwrap();
    assert_eq!(w.data.lock().unwrap().version, 2);
    assert_eq!(
        std::fs::read(directory.join("optimization.v1.backup.json")).unwrap(),
        original
    );
}
#[test]
fn automatic_marker_task_and_retry_keep_separate_sessions() {
    let (_t, w, r, run) = automatic_setup();
    task(&w, &run, "marker");
    create(&w, &run, "Assets/Helper.cs").unwrap();
    finish(&w, &run);
    assert_ne!(
        w.data.lock().unwrap().rounds[0].performance_status(),
        "达到目标"
    );
    let retry = w
        .begin_mode(&r, "codex".into(), Some(String::new()))
        .unwrap();
    assert_ne!(retry, run);
    assert!(w.data.lock().unwrap().rounds[0].runs[1]
        .read_receipts
        .is_empty());
    assert_eq!(w.data.lock().unwrap().rounds[0].runs[0].agent_id, "codex");
}

async fn automatic_real_agent(agent: &str) {
    let public = PathBuf::from(
        std::env::var_os("UPAA_PUBLIC_UNITY_PROJECT").expect("set explicit public Unity project"),
    );
    assert!(
        public.join(".upaa-public-fixture").is_file(),
        "public fixture marker required"
    );
    let root = public.canonicalize().unwrap();
    let original = std::fs::read(root.join("Assets/AutomaticHotspots.cs"))
        .expect("copy public AutomaticHotspots.cs fixture first");
    assert_eq!(
        original,
        include_bytes!("fixtures/unity-project/Assets/AutomaticHotspots.cs")
    );
    assert!(!root.join("Assets/AutomaticBufferCache.cs").exists());
    let records = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../.cache/automatic-records-{}", id()));
    std::fs::create_dir(&records).unwrap();
    let w = Arc::new(Workspace::create(records, root.clone(), "公开自动优化验收".into()).unwrap());
    let (_fixture, template, r, _) = automatic_setup();
    let mut round = template.data.lock().unwrap().rounds[0].clone();
    round.runs.clear();
    round.tasks.clear();
    round.reports[0].text="Public synthetic evidence: AutomaticCaller.Invoke calls AutomaticHotspots.Update. Its byte[4096] allocation is a GC candidate; inspect actual code before changing. There is also an Unmapped.Native marker with no source evidence: record that it cannot be located, do not invent code. A new helper is explicitly required for this acceptance fixture. Returned contents must stay 4096 bytes with byte 0 equal to 1; callers may reuse the buffer.".into();
    round.tests = vec![];
    w.data.lock().unwrap().rounds.push(round);
    let baseline = unity_profiler_analysis_agent_lib::project::editor::check_changes(
        &root,
        vec!["Assets/AutomaticHotspots.cs".into()],
        vec![],
        &w.cancelled,
    )
    .await
    .unwrap();
    assert_eq!(baseline["status"], "passed");
    let run=w.begin_mode(&r,agent.into(),Some("仅优化公开 AutomaticHotspots 的每次分配：读取 AutomaticCaller 调用链，新增 Assets/AutomaticBufferCache.cs 用于缓存，再修改已有 AutomaticHotspots.cs 使用缓存；不得改其他现有文件。保留返回内容，可复用实例。未定位的 Unmapped.Native 只记录调查不足。完成后执行编译检查。".into())).unwrap();
    let scope = Arc::new(session::EditScope {
        operation: tokio::sync::Mutex::new(()),
        workspace: w.clone(),
        run_id: run.clone(),
    });
    let project = Arc::new(
        unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
            run.clone(),
            root.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let profile = parser::parse_file(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-dump.json"),
    )
    .await
    .unwrap();
    let preset = acp_client::agents::builtin_presets()
        .into_iter()
        .find(|p| p.id == agent)
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let req = acp_client::DiagnoseRequest {
        project: Some(project),
        source: None,
        parent_report: None,
        file_id: run.clone(),
        agent_id: agent.into(),
        snapshot: extractor::extract(&profile),
        details: profile.details,
        bridge_executable: std::env::var_os("UPAA_TEST_APP_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_unity-profiler-analysis-agent"))),
        event_tx: tx,
    };
    let handle = acp_client::start_with_scope(preset, req, Some(scope.clone()))
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(600), async {
        while let Some(event) = rx.recv().await {
            if let DiagnoseEvent::Log { message } = &event {
                eprintln!("{}", message.chars().take(250).collect::<String>());
            }
            let end = event.terminal();
            w.event(&run, &event).unwrap();
            if end {
                return matches!(event, DiagnoseEvent::Finished { .. });
            }
        }
        false
    })
    .await;
    if outcome.is_err() {
        handle.cancel().await;
    } else {
        handle.wait().await;
    }
    let check = if w.cancelled.load(Ordering::SeqCst) {
        w.data.lock().unwrap().rounds[0].runs[0]
            .checks
            .last()
            .cloned()
            .unwrap_or(json!({"status":"cancelled"}))
    } else {
        scope.check_editor().await.unwrap()
    };
    let changed = std::fs::read(root.join("Assets/AutomaticHotspots.cs")).unwrap() != original;
    let created = root.join("Assets/AutomaticBufferCache.cs").exists();
    let evidence = serde_json::to_string_pretty(&*w.data.lock().unwrap()).unwrap();
    let evidence_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../.cache/automatic-{agent}.json"));
    std::fs::write(evidence_path, evidence).unwrap();
    w.busy.store(false, Ordering::SeqCst);
    w.rollback(&r).unwrap();
    assert_eq!(
        std::fs::read(root.join("Assets/AutomaticHotspots.cs")).unwrap(),
        original
    );
    assert!(!root.join("Assets/AutomaticBufferCache.cs").exists());
    assert!(!root.join("Assets/AutomaticBufferCache.cs.meta").exists());
    assert!(outcome.unwrap(), "Agent failed");
    assert!(changed && created, "must edit and create actual files");
    assert_eq!(check["status"], "passed", "{check}");
    assert_ne!(
        w.data.lock().unwrap().rounds[0].runs[0].session_id,
        "diagnostic"
    );
}
#[tokio::test]
#[ignore = "explicit public Unity project, real Codex automatic edit/create/check/rollback"]
async fn real_automatic_codex() {
    automatic_real_agent("codex").await;
}
#[tokio::test]
#[ignore = "explicit public Unity project, real Claude automatic edit/create/check/rollback"]
async fn real_automatic_claude() {
    automatic_real_agent("claude-code").await;
}

#[test]
fn metadata_normalization_is_recorded_but_changed_guid_is_not_adopted() {
    let (_t, w, r, run) = automatic_setup();
    create(&w, &run, "Assets/Helper.cs").unwrap();
    let root = w.data.lock().unwrap().root.clone();
    let path = root.join("Assets/Helper.cs.meta");
    let before = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, before.replace("\n", "\r\n")).unwrap();
    {
        let mut d = w.data.lock().unwrap();
        let paths = automatic::reconcile_meta(&root, &mut d.rounds[0].runs[0]).unwrap();
        assert_eq!(paths, vec!["Assets/Helper.cs.meta"]);
        assert_eq!(d.rounds[0].runs[0].changes.last().unwrap().kind, "metadata");
        w.save(&d).unwrap();
    }
    finish(&w, &run);
    // Simulate interruption after restoring normalized metadata but before recording that step.
    std::fs::write(&path, &before).unwrap();
    w.rollback(&r).unwrap();
    assert!(!path.exists());
    let (_t, w, _r, run) = automatic_setup();
    create(&w, &run, "Assets/Helper.cs").unwrap();
    let root = w.data.lock().unwrap().root.clone();
    let path = root.join("Assets/Helper.cs.meta");
    let before = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, before.replace("guid:", "changedGuid:")).unwrap();
    assert!(
        automatic::reconcile_meta(&root, &mut w.data.lock().unwrap().rounds[0].runs[0]).is_err()
    );
}
#[tokio::test]
async fn diagnosis_cannot_create_files_or_record_automatic_tasks() {
    let s = mcp::MetricsStore::new();
    for name in [
        "optimization_create",
        "optimization_task",
        "optimization_replace",
    ] {
        assert!(mcp::tools::dispatch(&s, name, json!({})).await.is_err());
    }
}

#[test]
fn automatic_project_scan_observes_workspace_cancellation() {
    let (_t, w, _r, _run) = automatic_setup();
    let root = w.data.lock().unwrap().root.clone();
    std::fs::write(
        root.join("ProjectSettings/ProjectVersion.txt"),
        "m_EditorVersion: 6000.3.23f1\n",
    )
    .unwrap();
    w.cancelled.store(true, Ordering::SeqCst);
    let error = unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
        "run".into(),
        root.clone(),
        w.cancelled.clone(),
    )
    .unwrap_err();
    assert!(error.contains("取消"));
    w.cancelled.store(false, Ordering::SeqCst);
    let mut scope = unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
        "run".into(),
        root,
        w.cancelled.clone(),
    )
    .unwrap();
    scope.cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    scope.cancelled.store(true, Ordering::SeqCst);
    assert!(
        !w.cancelled.load(Ordering::SeqCst),
        "closing read scope must not disable checks or rollback"
    );
}
