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
    {
        let mut d = w.data.lock().unwrap();
        scenes::create(&mut d, "测试场景".into(), String::new()).unwrap();
        d.scenes[0].id = "test-scene".into(); d.active_scene_id = "test-scene".into();
        let c = std::thread::spawn(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let path = public_capture("editor-dump.json");
            let p = rt.block_on(parser::parse_file(&path)).unwrap();
            Capture { scene_id: "test-scene".into(), code_revision: Some(0), id: "a".into(),
                hash: storage::hash(&std::fs::read(&path).unwrap()), path, conditions: Default::default(),
                snapshot: extractor::extract(&p), frames: p.frames }
        }).join().unwrap();
        d.captures.push(c);
    }
    let rid = id();
    w.data.lock().unwrap().rounds.push(Round {
        scene_id: "test-scene".into(),
        workflow: Default::default(),
        task_version: 1,
        task_verifications: BTreeMap::new(),
        id: rid.clone(),
        baseline: "a".into(),
        candidate: None,
        reports: vec![],
        tasks: vec![Task {
            marker_names: vec![],
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
    assert!(w.rollback(&r).unwrap_err().contains("历史"));
    w.rollback(&second).unwrap();
    assert!(w.rollback(&r).unwrap_err().contains("历史"));
    assert_eq!(editing::read(&w.data.lock().unwrap().root, "Assets/Work.cs").unwrap(), b"class Work { int number = 2; }");
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
        investigation_context: None,
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
    let mut cursor=0;let mut activity=vec![];
    loop {let p=observation::read(&w.directory,&run,cursor).unwrap();activity.extend(p["rows"].as_array().unwrap().iter().cloned());cursor=p["nextCursor"].as_u64().unwrap();if p["hasMore"]!=true{break;}}
    assert!(activity.iter().any(|e|e["event"]["kind"]=="chunk"));
    assert!(activity.iter().any(|e|e["event"]["tool"]=="optimization_replace"&&e["event"]["status"]=="running"));
    assert!(activity.iter().any(|e|e["event"]["tool"]=="optimization_replace"&&e["event"]["status"]=="completed"));
    assert!(activity.iter().all(|e|e["event"]["args"].get("new_text").is_none()));
    println!("Public modification activity: {} committed events",activity.len());
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
    let mut marker = scope.context()["taskDrafts"][0].clone();
    marker["kind"] = json!("marker");
    marker["markerNames"] = json!(["Work.Fixed"]);
    scope.query("project_propose_tasks", json!({"tasks":[marker.clone()]})).await.unwrap();
    assert_eq!(scope.context()["taskDrafts"][0]["markerNames"], json!(["Work.Fixed"]));
    for kind in ["optimize", "investigate"] {
        let mut invalid = marker.clone();
        invalid["kind"] = json!(kind);
        assert!(scope.query("project_propose_tasks", json!({"tasks":[invalid]})).await.is_err());
    }
    marker["markerNames"] = json!(["Work.Fixed", "Work.Fixed"]);
    assert!(scope.query("project_propose_tasks", json!({"tasks":[marker]})).await.is_err());
    assert_eq!(scope.context()["taskDrafts"][0]["markerNames"], json!(["Work.Fixed"]));

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
    first_marker_review(&w,&run);
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
        Capture { scene_id: "test-scene".into(), code_revision: Some(0),
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
    assert!(w.begin(&r, "b".into()).unwrap_err().contains("历史"));
    // A still-pending round may retry; a finalized round must remain immutable.
    w.data.lock().unwrap().rounds[0].decision = "pending".into();
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
    let saved = archive::load(
        &w.directory,
        &std::fs::read(w.directory.join("optimization.json")).unwrap(),
    )
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
    if kind=="marker" {
        let mut review=history_review("marker");review["focus"]=json!(null);review["markers"]=json!([]);
        w.edit_query(run,"optimization_review",review).unwrap();
    }
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
        storage::atomic(
            &directory.join("optimization.json"),
            &serde_json::to_vec(&*d).unwrap(),
        )
        .unwrap();
    }
    let original = std::fs::read(directory.join("optimization.json")).unwrap();
    drop(w);
    let w = Workspace::open(directory.clone()).unwrap();
    assert_eq!(w.data.lock().unwrap().version, 4);
    assert_eq!(
        std::fs::read(directory.join("optimization.v1.backup.json")).unwrap(),
        original
    );
}

#[test]
fn workflow_archive_split_recovery_and_missing_root() {
    let (t, w, r, run) = automatic_setup();
    w.event(
        &run,
        &DiagnoseEvent::Chunk {
            text: "本轮已调查".into(),
        },
    )
    .unwrap();
    // This tail is durable in the journal even if no manifest checkpoint follows.
    archive::append(
        &w.directory,
        &run,
        &json!({"offset":"本轮已调查".len(),"event":{"kind":"chunk","text":"，继续定位"}}),
    )
    .unwrap();
    let directory = w.directory.clone();
    let root = w.data.lock().unwrap().root.clone();
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("optimization.json")).unwrap())
            .unwrap();
    assert!(index["rounds"][0]["object"].is_string());
    assert!(!serde_json::to_string(&index)
        .unwrap()
        .contains("本轮已调查"));
    drop(w);
    let moved = t.0.join("temporarily-missing");
    std::fs::rename(&root, &moved).unwrap();
    let reopened = Workspace::open(directory).unwrap();
    let d = reopened.data.lock().unwrap();
    let round = d.rounds.iter().find(|x| x.id == r).unwrap();
    assert_eq!(round.runs.last().unwrap().text, "本轮已调查，继续定位");
    assert_eq!(round.runs.last().unwrap().status, "interrupted");
    assert_eq!(d.root, root);
    assert!(workflow::project_available(&d.root).is_err());
}

#[test]
fn workflow_report_wal_replays_once_and_marks_interruption() {
    let (_t, w, r, _run) = automatic_setup();
    let directory = w.directory.clone();
    let mut p = w.data.lock().unwrap().rounds[0].reports[0].clone();
    p.report_id = id();
    p.text = "已经保存".into();
    p.status = "running".into();
    let pid = p.report_id.clone();
    {
        let mut d = w.data.lock().unwrap();
        d.rounds[0].reports = vec![p];
        d.rounds[0].workflow.status = "running".into();
        w.save(&d).unwrap();
    }
    archive::append(
        &directory,
        &pid,
        &json!({"offset":0,"event":{"kind":"chunk","text":"已经保存"}}),
    )
    .unwrap();
    archive::append(
        &directory,
        &pid,
        &json!({"offset":"已经保存".len(),"event":{"kind":"chunk","text":"和未提交尾部"}}),
    )
    .unwrap();
    drop(w);
    let w = Workspace::open(directory.clone()).unwrap();
    assert_eq!(
        w.data.lock().unwrap().rounds[0].reports[0].text,
        "已经保存和未提交尾部"
    );
    assert_eq!(
        w.data.lock().unwrap().rounds[0].workflow.status,
        "interrupted"
    );
    assert!(workflow::export_round(&w, &r)
        .unwrap()
        .contains("未提交尾部"));
    drop(w);
    let w = Workspace::open(directory).unwrap();
    assert_eq!(
        w.data.lock().unwrap().rounds[0].reports[0].text,
        "已经保存和未提交尾部"
    );
}

#[tokio::test]
async fn workflow_next_requires_decision_and_uses_correct_baseline() {
    let (_t, w, _r) = setup(b"class Work {}");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-dump.json");
    let profile = parser::parse_file(&path).await.unwrap();
    w.data.lock().unwrap().captures.push(Capture { scene_id: "test-scene".into(), code_revision: Some(0),
id: "b".into(),
        path,
        hash: "test".into(),
        conditions: Conditions::default(),
        snapshot: extractor::extract(&profile),
        frames: profile.frames,
    });
    assert!(workflow::next_round(&w).is_err());
    {
        let mut d = w.data.lock().unwrap();
        d.rounds[0].candidate = Some("b".into());
        d.rounds[0].decision = "accepted".into();
    }
    workflow::next_round(&w).unwrap();
    {
        let mut d = w.data.lock().unwrap();
        assert_eq!(d.rounds[1].baseline, "b");
        assert!(d.rounds[1].reports.is_empty());
        d.rounds[1].decision = "rolled_back".into();
    }
    workflow::next_round(&w).unwrap();
    assert_eq!(w.data.lock().unwrap().rounds[2].baseline, "b");
}

fn public_capture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn seed_diagnosis(w: &Workspace, status: &str) -> Vec<String> {
    let mut d = w.data.lock().unwrap();
    let snapshot = &d.captures.last().unwrap().snapshot;
    let mut performance = unity_profiler_analysis_agent_lib::reports::Report::new(
        id(), "old-file".into(), "claude-code".into(), None, snapshot,
    );
    performance.status = "completed".into();
    performance.session_id = "old-performance-session".into();
    performance.text = "OLD_DIAGNOSIS_SENTINEL".into();
    let mut project = unity_profiler_analysis_agent_lib::reports::Report::new(
        id(), "old-file".into(), "codex".into(), Some(performance.report_id.clone()), snapshot,
    );
    project.stage = "project".into();
    project.status = status.into();
    project.session_id = "old-project-session".into();
    project.text = "OLD_LOCALIZATION_SENTINEL".into();
    let ids = vec![performance.report_id.clone(), project.report_id.clone()];
    let r = d.rounds.last_mut().unwrap();
    r.reports = vec![performance, project];
    r.tasks = vec![Task {
        marker_names: vec![],
        id: "old-task".into(), kind: "investigate".into(), title: "旧诊断任务".into(),
        evidence: "旧录制".into(), files: BTreeMap::new(), instructions: "调查".into(),
        acceptance: "核对".into(), constraints: String::new(), selected: true,
    }];
    r.task_verifications.insert("old-task".into(), "passed".into());
    r.task_version = 7;
    r.tests = vec!["Public.EditMode".into()];
    r.workflow = workflow::Progress {
        stage: "project".into(), status: status.into(), reason: Some("旧诊断原因".into()),
        analysis_agent: "claude-code".into(), localization_agent: "codex".into(),
    };
    d.budgets.insert("cpuMs".into(), 16.0);
    w.save(&d).unwrap();
    ids
}

#[tokio::test]
async fn workflow_replacing_diagnosed_a_resets_same_round_and_survives_reopen() {
    for status in ["completed", "failed", "cancelled", "interrupted"] {
        let (temp, w, rid) = setup(b"class Work {}");
        workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
        seed_diagnosis(&w, status);
        let (prior, old_capture, old_baseline) = {
            let mut d = w.data.lock().unwrap();
            let mut previous = d.rounds[0].clone();
            previous.id = id();
            previous.decision = "accepted".into();
            d.rounds.insert(0, previous.clone());
            // Stale comparison/confirmation must not survive a baseline replacement.
            let r = d.rounds.last_mut().unwrap();
            r.candidate = Some(r.baseline.clone());
            r.comparison = Some(json!({"old":"comparison"}));
            r.correctness = "passed".into();
            w.save(&d).unwrap();
            (serde_json::to_value(previous).unwrap(), serde_json::to_value(&d.captures[0]).unwrap(), d.rounds[1].baseline.clone())
        };
        workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap();
        let baseline = {
            let d = w.data.lock().unwrap();
            assert_eq!(d.rounds.len(), 2);
            assert_eq!(serde_json::to_value(&d.rounds[0]).unwrap(), prior);
            assert_eq!(serde_json::to_value(&d.captures[0]).unwrap(), old_capture);
            let r = &d.rounds[1];
            assert_eq!(r.id, rid);
            assert_ne!(r.baseline, old_baseline);
            assert!(r.reports.is_empty() && r.tasks.is_empty() && r.runs.is_empty());
            assert!(r.task_verifications.is_empty() && r.candidate.is_none() && r.comparison.is_none());
            assert_eq!(r.task_version, 8);
            assert_eq!(r.tests, vec!["Public.EditMode"]);
            assert_eq!(r.correctness, "pending");
            assert_eq!(r.decision, "pending");
            assert!(r.workflow.stage.is_empty() && r.workflow.status.is_empty() && r.workflow.reason.is_none());
            assert_eq!(r.workflow.analysis_agent, "claude-code");
            assert_eq!(r.workflow.localization_agent, "codex");
            assert_eq!(d.budgets["cpuMs"], 16.0);
            assert!(!workflow::ready_report(r));
            r.baseline.clone()
        };
        assert!(w.begin_mode(&rid, "codex".into(), Some(String::new())).unwrap_err().contains("需要完整工程定位报告"));
        assert!(!workflow::export_round(&w, &rid).unwrap().contains("SENTINEL"));
        assert_eq!(std::fs::read(temp.0.join("project/Assets/Work.cs")).unwrap(), b"class Work {}");
        let directory = w.directory.clone();
        drop(w);
        let reopened = Workspace::open(directory).unwrap();
        let d = reopened.data.lock().unwrap();
        assert_eq!(d.rounds.len(), 2);
        assert_eq!(serde_json::to_value(&d.rounds[0]).unwrap(), prior);
        assert_eq!(d.rounds[1].id, rid);
        assert_eq!(d.rounds[1].baseline, baseline);
        assert!(d.rounds[1].reports.is_empty() && d.rounds[1].tasks.is_empty());
        assert_eq!(d.rounds[1].tests, vec!["Public.EditMode"]);
    }
}

#[tokio::test]
async fn workflow_replacing_inherited_a_preserves_history_and_survives_reopen() {
    let (temp, w, rid) = setup(b"class Work { int number = 1; }");
    let tasks = w.data.lock().unwrap().rounds[0].tasks.clone();
    workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
    w.data.lock().unwrap().rounds[0].tasks = tasks;
    let run = w.begin(&rid, "public-test".into()).unwrap();
    replace(&w, &run, "number = 1", "number = 2").unwrap();
    finish(&w, &run);
    workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "b".into(), None).await.unwrap();
    let (prior, inherited) = {
        let mut d = w.data.lock().unwrap();
        let mut report = unity_profiler_analysis_agent_lib::reports::Report::new(id(), "fixture".into(), "public-test".into(), None, &d.captures[0].snapshot);
        report.status = "completed".into();
        report.text = "公开夹具历史报告".into();
        d.rounds[0].reports.push(report);
        d.rounds[0].decision = "accepted".into();
        w.save(&d).unwrap();
        (serde_json::to_value(&d.rounds[0]).unwrap(), d.rounds[0].candidate.clone().unwrap())
    };
    workflow::next_round(&w).unwrap();
    assert_eq!(w.data.lock().unwrap().rounds[1].baseline, inherited);
    let fresh = temp.0.join("新 A.json");
    std::fs::copy(public_capture("editor-dump.json"), &fresh).unwrap();
    workflow::bind_capture(w.clone(), fresh.clone(), "a".into(), None).await.unwrap();
    let (directory, baseline, next_id) = {
        let d = w.data.lock().unwrap();
        assert_eq!(serde_json::to_value(&d.rounds[0]).unwrap(), prior);
        let r = &d.rounds[1];
        assert_ne!(r.baseline, inherited);
        assert!(r.reports.is_empty() && r.runs.is_empty() && r.candidate.is_none());
        let a = d.captures.iter().find(|c| c.id == r.baseline).unwrap();
        let b = d.captures.iter().find(|c| c.id == inherited).unwrap();
        assert_eq!(a.path, fresh.canonicalize().unwrap());
        let compared = comparison::compare(a, b, None, None, false, &d.budgets).unwrap();
        assert_eq!(compared["baseline"], r.baseline);
        assert_eq!(compared["metrics"][0]["a"]["totalFrames"], 2);
        assert_eq!(compared["metrics"][0]["b"]["totalFrames"], 21);
        (w.directory.clone(), r.baseline.clone(), r.id.clone())
    };
    assert!(workflow::export_round(&w, &next_id).unwrap().contains("新 A.json"));
    drop(w);
    let reopened = Workspace::open(directory).unwrap();
    let d = reopened.data.lock().unwrap();
    assert_eq!(d.rounds[1].baseline, baseline);
    assert_eq!(serde_json::to_value(&d.rounds[0]).unwrap(), prior);
    assert!(d.rounds[1].reports.is_empty() && d.rounds[1].runs.is_empty());
}

#[tokio::test]
async fn workflow_failed_or_cancelled_a_import_preserves_memory_and_manifest() {
    let (temp, w, _rid) = setup(b"class Work {}");
    workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
    seed_diagnosis(&w, "interrupted");
    let before = serde_json::to_value(&*w.data.lock().unwrap()).unwrap();
    let manifest = w.directory.join("optimization.json");
    let bytes = std::fs::read(&manifest).unwrap();
    let invalid = temp.0.join("invalid.json");
    std::fs::write(&invalid, b"invalid profiler input").unwrap();
    assert!(workflow::bind_capture(w.clone(), invalid, "a".into(), None).await.is_err());
    assert_eq!(serde_json::to_value(&*w.data.lock().unwrap()).unwrap(), before);
    assert_eq!(std::fs::read(&manifest).unwrap(), bytes);
    let cancelled = w.cancelled.clone();
    w.observation.lock().unwrap().notify = Some(Arc::new(move |_, event| {
        if event["stage"] == "save" && event["status"] == "running" {
            cancelled.store(true, Ordering::SeqCst);
        }
    }));
    assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap_err().contains("取消"));
    assert_eq!(serde_json::to_value(&*w.data.lock().unwrap()).unwrap(), before);
    assert_eq!(std::fs::read(&manifest).unwrap(), bytes);
    assert!(!w.busy.load(Ordering::SeqCst));
    w.observation.lock().unwrap().notify = None;
    let permissions = std::fs::metadata(&manifest).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&manifest, readonly).unwrap();
    assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.is_err());
    assert_eq!(serde_json::to_value(&*w.data.lock().unwrap()).unwrap(), before);
    assert_eq!(std::fs::read(&manifest).unwrap(), bytes);
    assert!(w.save_error.lock().unwrap().is_some());
    std::fs::set_permissions(&manifest, permissions).unwrap();
    workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap();
    assert_ne!(serde_json::to_value(&*w.data.lock().unwrap()).unwrap(), before);
    assert!(w.data.lock().unwrap().rounds[0].reports.is_empty());
    assert!(!w.busy.load(Ordering::SeqCst));
}

#[tokio::test]
async fn workflow_a_replacement_rejects_started_finished_and_busy_rounds() {
    let (_temp, w, rid) = setup(b"class Work { int number = 1; }");
    workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
    let before = w.data.lock().unwrap().rounds[0].baseline.clone();
    w.busy.store(true, Ordering::SeqCst);
    assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap_err().contains("任务正在运行"));
    assert!(w.busy.load(Ordering::SeqCst));
    w.busy.store(false, Ordering::SeqCst);
    for decision in ["accepted", "rolled_back"] {
        w.data.lock().unwrap().rounds[0].decision = decision.into();
        assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap_err().contains("已结束"));
    }
    {
        let mut d = w.data.lock().unwrap();
        d.rounds[0].decision = "pending".into();
        let report = unity_profiler_analysis_agent_lib::reports::Report::new(id(), "fixture".into(), "public-test".into(), None, &d.captures[0].snapshot);
        d.rounds[0].reports.push(report);
    }
    assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap_err().contains("任务正在运行"));
    w.data.lock().unwrap().rounds[0].reports.clear();
    seed_diagnosis(&w, "completed");
    let run = w.begin_mode(&rid, "public-test".into(), Some(String::new())).unwrap();
    finish(&w, &run);
    for status in ["investigated", "failed", "cancelled", "interrupted"] {
        w.data.lock().unwrap().rounds[0].runs[0].status = status.into();
        assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap_err().contains("已开始优化"));
    }
    assert_eq!(w.data.lock().unwrap().rounds[0].baseline, before);
}

#[tokio::test]
async fn workflow_a_replacement_revalidates_round_before_commit() {
    for change in ["identity", "decision", "running", "runs"] {
        let (_temp, w, rid) = setup(b"class Work {}");
        workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
        seed_diagnosis(&w, "completed");
        let run_id = w.begin_mode(&rid, "public-test".into(), Some(String::new())).unwrap();
        finish(&w, &run_id);
        let run = w.data.lock().unwrap().rounds[0].runs.pop().unwrap();
        w.save(&w.data.lock().unwrap()).unwrap();
        let baseline = w.data.lock().unwrap().rounds[0].baseline.clone();
        let bytes = std::fs::read(w.directory.join("optimization.json")).unwrap();
        let weak = Arc::downgrade(&w);
        w.observation.lock().unwrap().notify = Some(Arc::new(move |_, event| {
            if event["stage"] == "save" && event["status"] == "running" {
                let w = weak.upgrade().unwrap();
                let mut d = w.data.lock().unwrap();
                let r = d.rounds.last_mut().unwrap();
                match change {
                    "identity" => r.id = id(),
                    "decision" => r.decision = "accepted".into(),
                    "running" => r.reports[0].status = "running".into(),
                    "runs" => r.runs.push(run.clone()),
                    _ => unreachable!(),
                }
            }
        }));
        assert!(workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.is_err());
        let d = w.data.lock().unwrap();
        assert_eq!(d.rounds[0].baseline, baseline);
        assert_eq!(d.captures.len(), 2);
        assert_eq!(d.rounds[0].reports.len(), 2);
        assert_eq!(std::fs::read(w.directory.join("optimization.json")).unwrap(), bytes);
        assert!(!w.busy.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn workflow_replacing_rolled_back_baseline_clears_inheritance_note() {
    let (_temp, w, _rid) = setup(b"class Work {}");
    workflow::bind_capture(w.clone(), public_capture("editor-dump.json"), "a".into(), None).await.unwrap();
    w.data.lock().unwrap().rounds[0].decision = "rolled_back".into();
    workflow::next_round(&w).unwrap();
    assert!(w.data.lock().unwrap().rounds[1].workflow.reason.is_some());
    workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), None).await.unwrap();
    assert!(w.data.lock().unwrap().rounds[1].workflow.reason.is_none());
    assert!(w.data.lock().unwrap().rounds[1].reports.is_empty());
}

#[test]
fn workflow_objects_are_verified_and_failed_save_preserves_manifest() {
    let (_t, w, _r) = setup(b"class Work {}");
    let directory = w.directory.clone();
    let before = std::fs::read(directory.join("optimization.json")).unwrap();
    let index: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let hash = index["rounds"][0]["object"].as_str().unwrap();
    let object = directory.join("objects").join(format!("{hash}.json"));
    std::fs::write(&object, b"{}").unwrap();
    assert!(archive::load(&directory, &before)
        .unwrap_err()
        .contains("损坏"));
    assert_eq!(
        std::fs::read(directory.join("optimization.json")).unwrap(),
        before
    );
}

#[test]
fn workflow_save_failure_is_visible_and_does_not_overwrite_manifest() {
    let (_t, w, _r) = setup(b"class Work {}");
    let manifest = w.directory.join("optimization.json");
    let before = std::fs::read(&manifest).unwrap();
    let mut perm = std::fs::metadata(&manifest).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&manifest, perm).unwrap();
    assert!(w.save(&w.data.lock().unwrap()).is_err());
    assert!(w.save_error.lock().unwrap().is_some());
    assert_eq!(std::fs::read(&manifest).unwrap(), before);
    let mut perm = std::fs::metadata(&manifest).unwrap().permissions();
    perm.set_readonly(false);
    std::fs::set_permissions(&manifest, perm).unwrap();
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
    scenes::create(&mut w.data.lock().unwrap(), "自动优化场景".into(), String::new()).unwrap();
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
        investigation_context: None,
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

#[test]
fn activity_feed_is_durable_paged_scoped_and_does_not_keep_code_arguments() {
    let (_temp,w,rid)=setup(b"public class Work {}");
    let pid=w.data.lock().unwrap().id.clone();let run=id();
    assert_eq!(observation::read(&w.directory,&run,0).unwrap()["available"],false);
    w.activity(&pid,&rid,&run,&DiagnoseEvent::ToolActivity{call_id:"call".into(),tool:"optimization_replace".into(),status:"running".into(),args:json!({"path":"Assets/中文.cs","new_text":"secret code","old_text":"old code"}),error:None}).unwrap();
    for _ in 0..120{w.activity(&pid,&rid,&run,&DiagnoseEvent::Chunk{text:"公开说明中文\n".into()}).unwrap();}
    w.activity(&pid,&rid,&run,&DiagnoseEvent::ToolActivity{call_id:"call".into(),tool:"optimization_replace".into(),status:"completed".into(),args:json!({}),error:None}).unwrap();
    let page=observation::read(&w.directory,&run,0).unwrap();assert_eq!(page["rows"].as_array().unwrap().len(),100);assert_eq!(page["hasMore"],true);
    assert_eq!(page["rows"][0]["event"]["args"]["path"],"Assets/中文.cs");assert!(page.to_string().find("secret code").is_none());
    let cursor=page["nextCursor"].as_u64().unwrap();let rest=observation::read(&w.directory,&run,cursor).unwrap();assert_eq!(rest["rows"].as_array().unwrap().len(),22);assert_eq!(rest["hasMore"],false);
    assert!(observation::read(&w.directory,&run,1).is_err());assert!(observation::read(&w.directory,"../escape",0).is_err());
    let dir=w.directory.clone();drop(w);let reopened=Workspace::open(dir).unwrap();assert_eq!(observation::read(&reopened.directory,&run,cursor).unwrap(),rest);
}

#[test]
fn activity_limit_preserves_report_semantics_and_torn_tail_is_not_published(){
    use std::io::Write;
    let (_temp,w,rid)=setup(b"class Work {}");let pid=w.data.lock().unwrap().id.clone();let run=id();
    let text="中".repeat(16000);
    for _ in 0..240{w.activity(&pid,&rid,&run,&DiagnoseEvent::Chunk{text:text.clone()}).unwrap();}
    let p=w.directory.join("activity").join(format!("{run}.jsonl"));assert!(std::fs::metadata(&p).unwrap().len()<=observation::ACTIVITY_LIMIT);
    let mut cursor=0;let mut limited=false;loop{let v=observation::read(&w.directory,&run,cursor).unwrap();for row in v["rows"].as_array().unwrap(){limited|=row["event"]["kind"]=="limited";assert!(row["event"]["text"].as_str().is_none_or(|s|s.len()<=16384));}cursor=v["nextCursor"].as_u64().unwrap();if v["hasMore"]!=true{break;}}
    assert!(limited);
    let other=id();w.activity(&pid,&rid,&other,&DiagnoseEvent::Chunk{text:"正文".into()}).unwrap();let v=observation::read(&w.directory,&other,0).unwrap();let cursor=v["nextCursor"].as_u64().unwrap();
    std::fs::OpenOptions::new().append(true).open(w.directory.join("activity").join(format!("{other}.jsonl"))).unwrap().write_all(b"{torn").unwrap();
    let v=observation::read(&w.directory,&other,cursor).unwrap();assert!(v["rows"].as_array().unwrap().is_empty());assert_eq!(v["hasMore"],false);
}

#[tokio::test]
async fn observed_capture_hash_and_parse_use_real_bytes(){
    let (_temp,w,rid)=setup(b"class Work {}");let p=observation::ParseProgress::new(w.clone(),rid,"operation".into());
    let path=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-dump.json");
    let bytes=std::fs::read(&path).unwrap();assert_eq!(p.hash(path.clone()).await.unwrap(),storage::hash(&bytes));
    let v=w.observation.lock().unwrap().progress.clone().unwrap();assert_eq!(v["done"],bytes.len());assert_eq!(v["total"],bytes.len());
    let profile=p.parse(path).await.unwrap();assert!(!profile.frames.is_empty());let v=w.observation.lock().unwrap().progress.clone().unwrap();assert!(v["total"].is_null());assert_eq!(v["stage"],"parse");
    p.fail("损坏结构");assert_eq!(w.observation.lock().unwrap().progress.as_ref().unwrap()["status"],"failed");
}

#[test]
fn activity_write_failure_sets_unsaved_state(){
    let (_temp,w,rid)=setup(b"class Work {}");let pid=w.data.lock().unwrap().id.clone();let run=id();
    w.activity(&pid,&rid,&run,&DiagnoseEvent::Chunk{text:"before".into()}).unwrap();let path=w.directory.join("activity").join(format!("{run}.jsonl"));
    let mut permissions=std::fs::metadata(&path).unwrap().permissions();permissions.set_readonly(true);std::fs::set_permissions(&path,permissions).unwrap();
    assert!(w.activity(&pid,&rid,&run,&DiagnoseEvent::Chunk{text:"after".into()}).is_err());assert!(w.save_error.lock().unwrap().is_some());
    let mut permissions=std::fs::metadata(&path).unwrap().permissions();permissions.set_readonly(false);std::fs::set_permissions(path,permissions).unwrap();
}

#[tokio::test]
async fn marker_names_survive_storage_and_match_only_exact_leaf_paths() {
    use unity_profiler_analysis_agent_lib::optimization::sampling;
    let (_temp, w, _round, run) = automatic_setup();
    let request = json!({"id":"t", "kind":"marker", "title":"观察真实路径", "evidence":"公开录制",
        "instructions":"固定同步范围", "acceptance":"重录并核对", "marker_names":["Work", "Absent"]});
    first_marker_review(&w,&run);
    w.edit_query(&run,"optimization_task", request.clone()).unwrap();
    let tasks = w.data.lock().unwrap().rounds[0].runs[0].tasks.clone();
    assert_eq!(tasks[0].marker_names, ["Work", "Absent"]);
    for names in [json!(["Work","Work"]), json!([" "]), json!((0..17).map(|i|format!("Point.{i}")).collect::<Vec<_>>())] {
        let mut bad=request.clone();bad["marker_names"]=names;
        assert!(w.edit_query(&run,"optimization_task",bad).is_err());
    }
    let mut old = serde_json::to_value(&tasks[0]).unwrap();
    old.as_object_mut().unwrap().remove("markerNames");
    let old: Task=serde_json::from_value(old).unwrap();
    assert!(old.marker_names.is_empty());
    assert_eq!(sampling::observe(&[old], &Ok(BTreeMap::new()))["rows"][0]["status"], "manual");
    let file=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-dump.json");
    let p=unity_profiler_analysis_agent_lib::parser::parse_file(&file).await.unwrap();
    let capture=Capture { scene_id: "test-scene".into(), code_revision: Some(0),
id:"b".into(),path:file.clone(),hash:storage::hash(&std::fs::read(&file).unwrap()),conditions:Default::default(),snapshot:unity_profiler_analysis_agent_lib::extractor::extract(&p),frames:p.frames};
    let mut paths=comparison::paths(&capture,None).await.unwrap();
    // A child under Work must not count as another Work observation.
    paths.insert(serde_json::to_string(&("", "Main Thread", vec!["Work", "Child"])).unwrap(),json!({}));
    paths.insert(serde_json::to_string(&("", "Main Thread", vec!["Work.Extra"])).unwrap(),json!({}));
    let result=sampling::observe(&tasks,&Ok(paths));
    assert_eq!(result["rows"][0]["status"],"observed");
    assert_eq!(result["rows"][0]["totalPaths"],1);
    assert_eq!(result["rows"][1]["status"],"notObserved");
    assert_eq!(sampling::observe(&tasks,&Err("录制已变化".into()))["rows"][0]["status"],"unavailable");
    finish(&w,&run);
    let directory=w.directory.clone();drop(w);
    let restored=Workspace::open(directory).unwrap();
    assert_eq!(restored.data.lock().unwrap().rounds[0].tasks[0].marker_names,["Work","Absent"]);
}

#[tokio::test]
async fn replacing_b_discards_old_sampling_acceptance_only_after_success() {
    let (_temp,w,rid)=setup(b"class Work {}");
    workflow::bind_capture(w.clone(),public_capture("editor-dump.json"),"a".into(),None).await.unwrap();
    seed_diagnosis(&w,"completed");
    let run=w.begin_mode(&rid,"public-test".into(),Some(String::new())).unwrap();
    finish(&w,&run);
    workflow::bind_capture(w.clone(),public_capture("editor-dump.json"),"b".into(),None).await.unwrap();
    { let mut d=w.data.lock().unwrap(); d.rounds[0].task_verifications.insert("marker".into(),"passed".into()); }
    let old=w.data.lock().unwrap().rounds[0].candidate.clone();
    assert!(workflow::bind_capture(w.clone(),w.directory.join("missing.json"),"b".into(),None).await.is_err());
    assert_eq!(w.data.lock().unwrap().rounds[0].task_verifications["marker"],"passed");
    workflow::bind_capture(w.clone(),public_capture("isolated-peak.json"),"b".into(),None).await.unwrap();
    let d=w.data.lock().unwrap();
    assert_ne!(d.rounds[0].candidate,old);
    assert!(d.rounds[0].task_verifications.is_empty());
}

#[tokio::test]
async fn scenes_keep_rounds_baselines_budgets_and_history_separate() {
    let (_t, w, first) = setup(b"class Work {}");
    {
        let mut d = w.data.lock().unwrap();
        scenes::edit(
            &mut d,
            "test-scene",
            "主城跑图".into(),
            "进入主城后绕广场一圈".into(),
        )
        .unwrap();
        d.scenes[0].budgets.insert("cpuMs".into(), 16.0);
        d.rounds[0].decision = "rolled_back".into();
    }
    workflow::next_round(&w).unwrap();
    {
        let mut d = w.data.lock().unwrap();
        assert_eq!(d.rounds[1].baseline, "a");
        d.rounds[1].decision = "rolled_back".into();
        scenes::complete(&mut d, "test-scene", true).unwrap();
        assert!(scenes::access(&d, "test-scene").is_err());
        scenes::create(&mut d, "百人战斗".into(), "固定角色进入战斗".into()).unwrap();
    }
    workflow::bind_capture(
        w.clone(),
        public_capture("isolated-peak.json"),
        "a".into(),
        None,
    )
    .await
    .unwrap();
    let (directory, y) = {
        let d = w.data.lock().unwrap();
        let y = d.active_scene_id.clone();
        assert_eq!(d.rounds.iter().filter(|r| r.scene_id == y).count(), 1);
        assert_eq!(
            d.rounds
                .iter()
                .filter(|r| r.scene_id == "test-scene")
                .count(),
            2
        );
        assert_eq!(d.rounds[0].id, first);
        assert!(d.scenes[1].budgets.is_empty());
        assert_ne!(d.rounds[2].baseline, "a");
        let c = d
            .captures
            .iter()
            .find(|c| c.id == d.rounds[2].baseline)
            .unwrap();
        assert_eq!(c.conditions.scenario, "百人战斗");
        assert_eq!(c.conditions.operation, "固定角色进入战斗");
        (w.directory.clone(), y)
    };
    assert!(workflow::export_round(&w, &first)
        .unwrap()
        .contains("主城跑图 · 第 1 轮"));
    drop(w);
    let w = Workspace::open(directory).unwrap();
    let mut d = w.data.lock().unwrap();
    assert_eq!(d.active_scene_id, y);
    assert!(d.scenes[0].completed);
    scenes::select(&mut d, "test-scene").unwrap();
    scenes::complete(&mut d, "test-scene", false).unwrap();
    assert!(scenes::access(&d, "test-scene").is_ok());
    w.save(&d).unwrap();
}

#[tokio::test]
async fn scenes_block_unsettled_edits_and_require_new_recording_after_other_scene_edits() {
    let (_t, w, x_round) = setup(b"class Work { int number = 1; }");
    let y = {
        let mut d = w.data.lock().unwrap();
        let y = scenes::create(&mut d, "Y".into(), String::new()).unwrap();
        scenes::select(&mut d, "test-scene").unwrap();
        y
    };
    let run = w.begin(&x_round, "fixture".into()).unwrap();
    replace(&w, &run, "number = 1", "number = 2").unwrap();
    finish(&w, &run);
    {
        let mut d = w.data.lock().unwrap();
        assert_eq!(scenes::blocker(&d, &y).as_deref(), Some("test-scene"));
        assert!(scenes::complete(&mut d, "test-scene", true).is_err());
        scenes::edit(&mut d, &y, "战斗".into(), "说明仍可编辑".into()).unwrap();
        scenes::select(&mut d, &y).unwrap();
    }
    assert!(workflow::bind_capture(
        w.clone(),
        public_capture("editor-dump.json"),
        "a".into(),
        None
    )
    .await
    .unwrap_err()
    .contains("接受或回退"));
    w.rollback(&x_round).unwrap();
    w.data.lock().unwrap().rounds[0].decision = "rolled_back".into();
    workflow::bind_capture(
        w.clone(),
        public_capture("editor-dump.json"),
        "a".into(),
        None,
    )
    .await
    .unwrap();
    let y_round = {
        let mut d = w.data.lock().unwrap();
        let tasks = d.rounds[0].tasks.clone();
        let r = d.rounds.last_mut().unwrap();
        r.tasks = tasks;
        r.id.clone()
    };
    let run = w.begin(&y_round, "fixture".into()).unwrap();
    replace(&w, &run, "number = 1", "number = 3").unwrap();
    finish(&w, &run);
    w.rollback(&y_round).unwrap();
    {
        let mut d = w.data.lock().unwrap();
        d.rounds.last_mut().unwrap().decision = "rolled_back".into();
        scenes::select(&mut d, "test-scene").unwrap();
        assert!(scenes::stale(&d, &d.captures[0])); // rollback does not restore freshness
    }
    workflow::next_round(&w).unwrap();
    let rid = w.data.lock().unwrap().rounds.last().unwrap().id.clone();
    assert!(w
        .begin(&rid, "fixture".into())
        .unwrap_err()
        .contains("重新录制"));
    assert!(scenes::round_access(&w.data.lock().unwrap(), &rid, true).is_err());
    let old = w
        .data
        .lock()
        .unwrap()
        .rounds
        .last()
        .unwrap()
        .baseline
        .clone();
    assert!(workflow::bind_capture(
        w.clone(),
        w.directory.join("missing.json"),
        "a".into(),
        None
    )
    .await
    .is_err());
    assert_eq!(w.data.lock().unwrap().rounds.last().unwrap().baseline, old);
    workflow::bind_capture(
        w.clone(),
        public_capture("editor-dump.json"),
        "a".into(),
        None,
    )
    .await
    .unwrap();
    let d = w.data.lock().unwrap();
    assert!(scenes::round_access(&d, &rid, true).is_ok());
    assert_eq!(d.rounds[0].baseline, "a");
    assert_eq!(d.rounds[0].runs[0].status, "rolled_back");
}

#[test]
fn scene_names_ownership_and_global_rollback_order_are_checked() {
    let (_t, w, x) = setup(b"class Work { int number = 1; }");
    {
        let mut d = w.data.lock().unwrap();
        assert!(scenes::create(&mut d, "  ".into(), String::new()).is_err());
        assert!(scenes::create(&mut d, "测试场景".into(), String::new()).is_err());
        assert!(scenes::create(&mut d, "场".repeat(81), String::new()).is_err());
        assert!(scenes::create(&mut d, "Y".into(), "字".repeat(4001)).is_err());
        assert!(scenes::select(&mut d, "missing").is_err());
    }
    let first = w.begin(&x, "fixture".into()).unwrap();
    replace(&w, &first, "1", "2").unwrap();
    finish(&w, &first);
    {
        let mut d = w.data.lock().unwrap();
        d.rounds[0].decision = "accepted".into();
        let y = scenes::create(&mut d, "Y".into(), String::new()).unwrap();
        let mut r = d.rounds[0].clone();
        r.id = id();
        r.scene_id = y.clone();
        r.decision = "pending".into();
        r.runs.clear();
        let mut c = d.captures[0].clone();
        c.id = id();
        c.scene_id = y.clone();
        c.code_revision = Some(d.code_revision);
        r.baseline = c.id.clone();
        r.tasks[0].files.insert(
            "Assets/Work.cs".into(),
            storage::hash(b"class Work { int number = 2; }"),
        );
        d.captures.push(c);
        d.rounds.push(r);
        assert!(scenes::validate(&d).is_ok());
        d.rounds[1].candidate = Some("a".into());
        assert!(scenes::validate(&d).is_err());
        d.rounds[1].candidate = None;
    }
    let y_round = w.data.lock().unwrap().rounds[1].id.clone();
    let second = w.begin(&y_round, "fixture".into()).unwrap();
    replace(&w, &second, "2", "3").unwrap();
    finish(&w, &second);
    {
        let mut d = w.data.lock().unwrap();
        assert!(d.rounds[1].runs[0].sequence > d.rounds[0].runs[0].sequence);
        // Simulate an interleaved storage order; dependency checks must use execution sequence.
        d.rounds[1].decision = "accepted".into();
        d.rounds[0].decision = "pending".into();
        d.rounds.swap(0, 1);
    }
    assert!(w.rollback(&x).unwrap_err().contains("后续"));
    assert_eq!(
        editing::read(&w.data.lock().unwrap().root, "Assets/Work.cs").unwrap(),
        b"class Work { int number = 3; }"
    );
}

#[test]
fn all_legacy_versions_migrate_once_with_original_manifest_and_unknown_freshness() {
    for version in [1, 2, 3] {
        let (_t, w, rid) = setup(b"class Work {}");
        let dir = w.directory.clone();
        {
            let mut d = w.data.lock().unwrap();
            d.budgets.insert("cpuMs".into(), 20.0);
            w.save(&d).unwrap();
            let mut v: serde_json::Value = if version == 3 {
                serde_json::from_slice(&std::fs::read(dir.join("optimization.json")).unwrap())
                    .unwrap()
            } else {
                serde_json::to_value(&*d).unwrap()
            };
            v["version"] = json!(version);
            for key in ["scenes", "activeSceneId", "codeRevision", "runSequence"] {
                v.as_object_mut().unwrap().remove(key);
            }
            storage::atomic(
                &dir.join("optimization.json"),
                &serde_json::to_vec(&v).unwrap(),
            )
            .unwrap();
        }
        let old = std::fs::read(dir.join("optimization.json")).unwrap();
        drop(w);
        let w = Workspace::open(dir.clone()).unwrap();
        let sid = {
            let d = w.data.lock().unwrap();
            assert_eq!(d.version, 4);
            assert_eq!(d.scenes.len(), 1);
            assert_eq!(d.scenes[0].name, "默认场景");
            assert_eq!(d.scenes[0].budgets["cpuMs"], 20.0);
            assert_eq!(d.rounds[0].id, rid);
            assert!(d.captures[0].code_revision.is_none());
            assert!(scenes::round_access(&d, &rid, true).is_err());
            assert_eq!(d.captures[0].scene_id, d.scenes[0].id);
            d.scenes[0].id.clone()
        };
        assert_eq!(
            std::fs::read(dir.join(format!("optimization.v{version}.backup.json"))).unwrap(),
            old
        );
        assert!(workflow::export_round(&w, &rid)
            .unwrap()
            .contains("默认场景"));
        drop(w);
        let w = Workspace::open(dir).unwrap();
        let d = w.data.lock().unwrap();
        assert_eq!(d.scenes.len(), 1);
        assert_eq!(d.scenes[0].id, sid);
    }
}

#[test]
fn failed_scene_migration_keeps_old_manifest_and_empty_projects_get_one_scene() {
    let (_t, w, _) = setup(b"class Work {}");
    let dir = w.directory.clone();
    let mut v = serde_json::to_value(&*w.data.lock().unwrap()).unwrap();
    v["version"] = json!(2);
    v["rounds"] = json!([]);
    v["captures"] = json!([]);
    let original = serde_json::to_vec(&v).unwrap();
    storage::atomic(&dir.join("optimization.json"), &original).unwrap();
    drop(w);
    // A non-file at the object destination makes the new v4 save fail.
    let objects = dir.join("objects");
    let saved_objects = dir.join("original-objects");
    std::fs::rename(&objects, &saved_objects).unwrap();
    std::fs::write(&objects, b"not a directory").unwrap();
    assert!(Workspace::open(dir.clone()).is_err());
    assert_eq!(
        std::fs::read(dir.join("optimization.json")).unwrap(),
        original
    );
    std::fs::remove_file(&objects).unwrap();
    std::fs::rename(&saved_objects, &objects).unwrap();
    let w = Workspace::open(dir.clone()).unwrap();
    assert_eq!(w.data.lock().unwrap().scenes.len(), 1);
    drop(w);
    let w = Workspace::open(dir).unwrap();
    let d = w.data.lock().unwrap();
    assert_eq!(d.scenes.len(), 1);
    assert!(d.rounds.is_empty());
}


#[tokio::test]
async fn selecting_another_scene_during_import_does_not_redirect_capture_or_progress() {
    let (_t, w, rid) = setup(b"class Work {}");
    let y = {
        let mut d = w.data.lock().unwrap();
        let y = scenes::create(&mut d, "Y".into(), String::new()).unwrap();
        scenes::select(&mut d, "test-scene").unwrap();
        y
    };
    let weak = Arc::downgrade(&w);
    let selected = y.clone();
    w.observation.lock().unwrap().notify = Some(Arc::new(move |_, event| {
        if event["stage"] == "save" && event["status"] == "running" {
            let w = weak.upgrade().unwrap();
            scenes::select(&mut w.data.lock().unwrap(), &selected).unwrap();
        }
    }));
    workflow::bind_capture(w.clone(), public_capture("isolated-peak.json"), "a".into(), Some("scene-import".into())).await.unwrap();
    let d = w.data.lock().unwrap();
    assert_eq!(d.active_scene_id, y);
    assert!(scenes::current(&d, &y).is_none());
    let r = d.rounds.iter().find(|r|r.id==rid).unwrap();
    assert_eq!(r.scene_id, "test-scene");
    assert_eq!(d.captures.iter().find(|c|c.id==r.baseline).unwrap().scene_id, "test-scene");
    assert_eq!(w.observation.lock().unwrap().progress.as_ref().unwrap()["sceneId"], "test-scene");
}


#[test]
fn check_inventory_ignores_identical_settings_rewrite_but_detects_content_changes() {
    let (_temp, w, _) = setup(b"class Work {}");
    let root = w.data.lock().unwrap().root.clone();
    let key = "ProjectSettings/PluginSettings.xml";
    let path = root.join(key);
    std::fs::write(&path, b"<enabled>true </enabled>").unwrap();
    let before = session::inventory(&root, &w.cancelled).unwrap();
    let changed_time = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    std::fs::write(&path, b"<enabled>true </enabled>").unwrap();
    std::fs::File::options().write(true).open(&path).unwrap()
        .set_modified(changed_time).unwrap();
    assert_eq!(before, session::inventory(&root, &w.cancelled).unwrap());
    // Same byte count and timestamp still must not hide a real settings mutation.
    std::fs::write(&path, b"<enabled>false</enabled>").unwrap();
    std::fs::File::options().write(true).open(&path).unwrap()
        .set_modified(changed_time).unwrap();
    assert_ne!(before[key], session::inventory(&root, &w.cancelled).unwrap()[key]);
    std::fs::remove_file(path).unwrap();
    assert!(!session::inventory(&root, &w.cancelled).unwrap().contains_key(key));
}

#[test]
fn check_inventory_bounds_settings_reads_and_keeps_resource_time_checks() {
    let (_temp, w, _) = setup(b"class Work {}");
    let root = w.data.lock().unwrap().root.clone();
    for key in ["ProjectSettings/Large.bin", "Assets/Texture.bin"] {
        let path = root.join(key);
        std::fs::write(&path, vec![0; 2 * 1024 * 1024 + 1]).unwrap();
        let before = session::inventory(&root, &w.cancelled).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60)).unwrap();
        assert_ne!(before[key], session::inventory(&root, &w.cancelled).unwrap()[key]);
    }
    w.cancelled.store(true, Ordering::SeqCst);
    assert!(session::inventory(&root, &w.cancelled).is_err());
}

// History regressions use disposable archives; no user code is modified.
fn add_marker_history(w: &Workspace) {
    let mut d = w.data.lock().unwrap();
    let mut old = d.rounds[0].clone();
    old.id = "previous".into();
    old.decision = "accepted".into();
    old.candidate = Some("a".into());
    old.reports[0].report_id = "previous-report".into();
    let run = &mut old.runs[0];
    run.id = "previous-run".into();
    run.status = "modified".into();
    run.tasks[0].kind = "marker".into();
    run.tasks[0].marker_names = vec!["Public.Existing".into()];
    run.changes.push(Change { task_id:Some("t".into()),kind:"modify".into(),path:"Assets/Work.cs".into(),before_hash:"old".into(),after_hash:"new".into(),before:vec![],after:vec![],state:"applied".into() });
    old.tasks = run.tasks.clone();
    old.comparison = Some(json!({"sampling":{"rows":[{"name":"Public.Existing","status":"observed"}]}}));
    d.rounds.insert(0,old);
}
fn history_review(action: &str) -> serde_json::Value {
    json!({"coverage":{"names":["Public.Existing"],"complete":true,"evidence":"精确查询全量300条，nextStart=null，代表帧树无截断","reproduction":"reproduced"},"evidencePlan":{"existingSufficient":false,"sourceAudit":"Assets/Work.cs getter分支","rawInputs":"输入集合长度","filteredOutputs":"分支输出数量","cacheState":"集合容量","instanceIdentity":"稳定owner元数据","lifecycle":"创建销毁计数","decisionRules":"输入增长与缓存重建分别判定","captureSteps":"首次和重复调用同一owner","overhead":"静态marker和无GC数值元数据","remainingGaps":[]},"focus":{"roundId":"previous","taskId":"t","mode":"continue","targetTaskId":"t","target":"原分配问题","disposition":"continuing","priorityReason":"继续原每帧分配问题，其他热点尚未证实收益","targetPlatform":"Android","targetEnvironment":"player","benefitScope":"player","platformEvidence":"实际调用在Runtime，无Editor条件编译；收益需Player复测","resumeCondition":"沿 getter 继续定位"},"markers":[{"roundId":"previous","taskId":"t","status":"unresolved","evidence":"当前 A 主线程 Public.Existing 每帧 1 次，12 次 GC.Alloc，出现不等于原因已定位","nextStep":"读取分配路径的 getter","unobservedNames":[]}],"nextAction":action,"reason":"旧采样已定位到 getter，但尚需区分两个实现分支","newEvidence":"已读 getter 两个同步分支，旧采样没有区分","newMarkerQuestion":"分支 A 有分配则追踪集合创建，分支 B 有分配则追踪格式化"})
}
#[test]
fn investigation_handoff_scopes_history_and_preserves_observation_provenance() {
    let (_t,w,r,run)=automatic_setup(); add_marker_history(&w);
    {
        let mut d=w.data.lock().unwrap();
        let mut other=d.rounds[0].clone();other.id="other".into();other.scene_id="other-scene".into();d.rounds.insert(0,other);
        let mut future=d.rounds.last().unwrap().clone();future.id="future".into();d.rounds.push(future);
        let c=investigation::context(&d,&r);
        assert_eq!(c["rounds"].as_array().unwrap().len(),1);
        assert_eq!(c["consecutiveMarkerOnlyRounds"],1);
        assert_eq!(c["rounds"][0]["candidateIsCurrentBaseline"],true);
        d.rounds[1].candidate=Some("different-capture".into());
        assert_eq!(investigation::context(&d,&r)["rounds"][0]["candidateIsCurrentBaseline"],false);
        d.rounds[1].runs[0].changes[0].state="rolled_back".into();
        assert!(investigation::context(&d,&r)["requiredMarkerReviews"].as_array().unwrap().is_empty());
    }
    let report=w.edit_query(&run,"optimization_context",json!({"report_id":"previous-report"})).unwrap();
    assert!(report["text"].as_str().unwrap().contains("Public evidence"));
    assert!(w.edit_query(&run,"optimization_context",json!({"report_id":"not-in-history"})).is_err());
}
#[test]
fn investigation_requires_review_before_existing_and_new_file_writes() {
    let (_t,w,r,run)=automatic_setup(); add_marker_history(&w);
    assert!(replace(&w,&run,"number = 1","number = 2").unwrap_err().contains("optimization_review"));
    assert!(create(&w,&run,"Assets/Helper.cs").unwrap_err().contains("optimization_review"));
    let mut missing=history_review("optimize");missing["markers"]=json!([]);
    assert!(w.edit_query(&run,"optimization_review",missing).is_err());
    let mut duplicate=history_review("optimize");let entry=duplicate["markers"][0].clone();duplicate["markers"].as_array_mut().unwrap().push(entry);
    assert!(w.edit_query(&run,"optimization_review",duplicate).is_err());
    w.edit_query(&run,"optimization_review",history_review("investigate")).unwrap();
    assert!(replace(&w,&run,"number = 1","number = 2").is_err());
    w.edit_query(&run,"optimization_review",history_review("optimize")).unwrap();
    replace(&w,&run,"number = 1","number = 2").unwrap();
    let data=archive::load(&w.directory,&std::fs::read(w.directory.join("optimization.json")).unwrap()).unwrap();
    assert_eq!(data.rounds.iter().find(|x|x.id==r).unwrap().runs[0].investigation_review.as_ref().unwrap().next_action,"optimize");
}
#[test]
fn investigation_rejects_repeat_markers_and_unsupported_continued_sampling() {
    let (_t,w,_r,run)=automatic_setup(); add_marker_history(&w);
    let mut unsupported=history_review("marker");unsupported["newEvidence"]=json!("");
    assert!(w.edit_query(&run,"optimization_review",unsupported).is_err());
    let mut review=history_review("marker");review["focus"]["targetTaskId"]=json!("sample");w.edit_query(&run,"optimization_review",review).unwrap();
    let mut task=json!({"id":"sample","kind":"marker","title":"采样","evidence":"已读取两个分支","instructions":"同步分支","acceptance":"区分两个原因","marker_names":["Public.Existing"]});
    assert!(w.edit_query(&run,"optimization_task",task.clone()).unwrap_err().contains("重复"));
    task["marker_names"]=json!(["Public.Getter.Branch"]);
    w.edit_query(&run,"optimization_task",task).unwrap();
    // Old archives without the optional review field remain readable.
    let mut old=serde_json::to_value(&w.data.lock().unwrap().rounds[1].runs[0]).unwrap();
    old.as_object_mut().unwrap().remove("investigationReview");
    assert!(serde_json::from_value::<Run>(old).unwrap().investigation_review.is_none());
}
#[tokio::test]
#[ignore = "Explicit read-only private archive verification via UPAA_INVESTIGATION_ARCHIVE"]
async fn real_investigation_archive_handoff_read_only() {
    let path=PathBuf::from(std::env::var("UPAA_INVESTIGATION_ARCHIVE").expect("explicit archive required"));
    let manifest=std::fs::read(path.join("optimization.json")).unwrap();
    let data=archive::load(&path,&manifest).unwrap();
    let current=data.rounds.last().unwrap();
    let context=investigation::context(&data,&current.id);
    assert!(context["rounds"].as_array().unwrap().len()>=2);
    assert!(context["requiredMarkerReviews"].as_array().unwrap().iter().any(|r|r["markerNames"].as_array().unwrap().iter().any(|s|s=="UI.Time.Petra")));
    assert!(current.marker_only(),"第五轮实际改动全部属于补点，可跳过性能对比");
    assert!(investigation::guard(&context,None,"marker",&[],"test").is_err());
    assert!(data.rounds.len()>=5,"此验收针对ProjectJinn第五轮及以后现场");
    assert_eq!(context["focus"]["taskId"],"minimap-nav-churn");
    let old_review=current.runs.last().unwrap().investigation_review.as_ref().unwrap();
    assert!(old_review.coverage.is_none());
    assert!(investigation::validate(&context,old_review).is_err(),"旧版无完整取证记录必须补充后才能修改");
    let capture=data.captures.iter().find(|c|c.id==current.baseline).unwrap();
    let store=mcp::MetricsStore::new();store.set(capture.snapshot.clone()).await;
    let target=mcp::tools::dispatch(&store,"performance_hotspots",json!({"area":"cpu","names":["UI.MiniMap.NavPath","UI.MiniMap.NavDisplay"],"limit":50})).await.unwrap();
    assert_eq!(target["targetLookup"]["lookupComplete"],true);
    assert!(target["targetLookup"]["scannedEntries"].as_u64().unwrap()>250);
    assert_eq!(target["rows"].as_array().unwrap().len(),2);
    assert!(target["rows"].as_array().unwrap().iter().all(|r|r["callCount"]==74));
    eprintln!("ProjectJinn target lookup: {target}");
    let gc=mcp::tools::dispatch(&store,"performance_hotspots",json!({"area":"gc","limit":50})).await.unwrap();
    assert!(gc["rows"].as_array().unwrap().iter().all(|r|r["allocationCount"].is_number()&&r.get("callCount").is_none()));
    let tm_gc=gc["rows"].as_array().unwrap().iter().find(|r|r["name"]=="TM.OnUpdate").unwrap();
    let tm_cpu=capture.snapshot.cpu.top_hotspots.iter().find(|r|r.name=="TM.OnUpdate").unwrap();
    assert_eq!(tm_cpu.call_count,capture.frames.len() as u64);
    let source_gc=capture.snapshot.gc.top_alloc_sites.iter().find(|r|r.name=="TM.OnUpdate").unwrap();
    assert_eq!(tm_gc["allocationCount"],source_gc.call_count);
    assert_ne!(tm_gc["allocationCount"].as_u64(),Some(tm_cpu.call_count));
    assert_eq!(std::fs::read(path.join("optimization.json")).unwrap(),manifest);
    if let Ok(output)=std::env::var("UPAA_INVESTIGATION_OUTPUT") {
        std::fs::write(output,serde_json::to_vec_pretty(&json!({"archiveUnchanged":true,"roundCount":data.rounds.len(),"handoff":context,"gc":gc,"tmBusinessCalls":tm_cpu.call_count,"oldUnscopedReviewRejected":true,"targetLookup":target})).unwrap()).unwrap();
    }
}

#[test]
fn investigation_focus_requires_switch_priority_platform_and_task_binding() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    let context=investigation::context(&w.data.lock().unwrap(),&r);
    assert_eq!(context["focus"]["taskId"],"t");
    for field in ["priorityReason","targetPlatform","benefitScope","platformEvidence","resumeCondition"] {
        let mut review=history_review("optimize");review["focus"]["mode"]=json!("switch");review["focus"][field]=json!("");
        assert!(w.edit_query(&run,"optimization_review",review).is_err(),"{field}");
    }
    let mut wrong_platform=history_review("optimize");wrong_platform["focus"]["benefitScope"]=json!("editor");
    assert!(w.edit_query(&run,"optimization_review",wrong_platform).unwrap_err().contains("Player"));
    let mut review=history_review("optimize");review["focus"]["mode"]=json!("switch");
    review["focus"]["targetTaskId"]=json!("editor-only");review["focus"]["benefitScope"]=json!("editor");
    review["focus"]["platformEvidence"]=json!("UNITY_EDITOR限定，只改善编辑器，不解决Android原目标");
    review["focus"]["priorityReason"]=json!("原尖峰未复现，暂缓；编辑器重复订阅是低风险次要问题，不能替代主目标");
    w.edit_query(&run,"optimization_review",review).unwrap();
    assert!(replace(&w,&run,"number = 1","number = 2").unwrap_err().contains("目标决策"));
}
#[test]
fn investigation_unobserved_cannot_resolve_or_withdraw_even_partial_task() {
    let (_t,w,_r,run)=automatic_setup();add_marker_history(&w);
    for status in ["not_observed","not_comparable","unresolved"] {
        let mut review=history_review("investigate");review["markers"][0]["status"]=json!(status);review["focus"]["disposition"]=json!("resolved");
        assert!(w.edit_query(&run,"optimization_review",review).is_err());
    }
    let mut partial=history_review("investigate");partial["markers"][0]["status"]=json!("answered");partial["markers"][0]["unobservedNames"]=json!(["Public.Existing"]);
    assert!(w.edit_query(&run,"optimization_review",partial).is_err());
    let mut waiting=history_review("investigate");waiting["markers"][0]["status"]=json!("not_observed");waiting["markers"][0]["unobservedNames"]=json!(["Public.Existing"]);waiting["focus"]["disposition"]=json!("awaiting_reproduction");
    w.edit_query(&run,"optimization_review",waiting.clone()).unwrap();
    waiting["focus"]["disposition"]=json!("withdrawn");assert!(w.edit_query(&run,"optimization_review",waiting).is_err());
}
#[test]
fn investigation_switch_preserves_original_focus_in_next_round() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    let mut review=history_review("optimize");review["focus"]["mode"]=json!("switch");review["focus"]["disposition"]=json!("awaiting_reproduction");
    w.edit_query(&run,"optimization_review",review).unwrap();
    let mut d=w.data.lock().unwrap();let mut next=d.rounds.iter().find(|x|x.id==r).unwrap().clone();next.id="next-focus".into();next.runs.clear();d.rounds.push(next);
    let c=investigation::context(&d,"next-focus");assert_eq!(c["focus"]["roundId"],"previous");assert_eq!(c["focus"]["taskId"],"t");
}
#[test]
fn investigation_necessary_marker_allowed_after_many_sampling_rounds() {
    let (_t,w,r,_run)=automatic_setup();add_marker_history(&w);
    let mut c=investigation::context(&w.data.lock().unwrap(),&r);c["consecutiveMarkerOnlyRounds"]=json!(20);
    let review:investigation::Review=serde_json::from_value(history_review("marker")).unwrap();
    investigation::guard(&c,Some(&review),"marker",&["Public.Getter.Branch".into()],"t").unwrap();
    assert!(investigation::guard(&c,Some(&review),"marker",&["Public.Existing".into()],"t").is_err());
}

#[test]
fn marker_preflight_requires_complete_plan_and_target_coverage() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    let c=investigation::context(&w.data.lock().unwrap(),&r);
    for (field,value) in [("coverage",json!(null)),("evidencePlan",json!(null))] {
        let mut v=history_review("marker");v[field]=value;
        assert!(w.edit_query(&run,"optimization_review",v).is_err());
    }
    let mut v=history_review("marker");v["coverage"]["names"]=json!([]);
    assert!(w.edit_query(&run,"optimization_review",v).is_err());
    let mut v=history_review("marker");v["coverage"]["complete"]=json!(false);
    assert!(w.edit_query(&run,"optimization_review",v).is_err());
    for field in ["rawInputs","filteredOutputs","cacheState","instanceIdentity","lifecycle","decisionRules","captureSteps","overhead","sourceAudit"] {
        let mut v=history_review("marker");v["evidencePlan"][field]=json!("");
        assert!(w.edit_query(&run,"optimization_review",v).is_err(),"{field}");
    }
    let mut v=history_review("marker");v["evidencePlan"]["remainingGaps"]=json!(["下轮再补实例身份和过滤后数量"]);
    assert!(w.edit_query(&run,"optimization_review",v).is_err());
    let v:investigation::Review=serde_json::from_value(history_review("marker")).unwrap();
    investigation::guard(&c,Some(&v),"marker",&["New.Branch".into()],"t").unwrap();
}
#[test]
fn complete_instrumentation_waits_for_reproduction_and_survives_handoff() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    let mut v=history_review("marker");v["coverage"]["reproduction"]=json!("not_reproduced");
    assert!(w.edit_query(&run,"optimization_review",v.clone()).is_err());
    v["focus"]["disposition"]=json!("awaiting_reproduction");v["evidencePlan"]["existingSufficient"]=json!(true);
    assert!(w.edit_query(&run,"optimization_review",v.clone()).is_err());
    v["nextAction"]=json!("investigate");w.edit_query(&run,"optimization_review",v.clone()).unwrap();
    assert!(create(&w,&run,"Assets/ShouldNotExist.cs").is_err());
    finish(&w,&run);
    let mut d=w.data.lock().unwrap().clone();
    let mut next=d.rounds.last().unwrap().clone();next.id="next".into();next.runs.clear();d.rounds.push(next);
    let context=investigation::context(&d,"next");assert_eq!(context["focus"]["taskId"],"t");
    assert_eq!(context["rounds"][1]["reviews"][0]["review"]["focus"]["disposition"],"awaiting_reproduction");
    assert_eq!(context["rounds"][1]["reviews"][0]["review"]["evidencePlan"]["existingSufficient"],true);
    assert_eq!(context["focus"]["latestReview"]["evidencePlan"]["existingSufficient"],true);
    let repeated=d.rounds[0].runs[0].changes[0].clone();
    d.rounds[0].runs[0].changes.extend(std::iter::repeat_n(repeated,1000));
    let compact:serde_json::Value=serde_json::from_str(&investigation::prompt(&d,"next")).unwrap();
    assert_eq!(compact["incomplete"],true);
    assert_eq!(compact["focus"]["latestReview"]["focus"]["disposition"],"awaiting_reproduction");
    assert_eq!(compact["focus"]["latestReview"]["evidencePlan"]["existingSufficient"],true);
    let _=r;
}
#[test]
fn first_marker_requires_preflight_even_without_history() {
    let (_t,w,r,_run)=automatic_setup();let c=investigation::context(&w.data.lock().unwrap(),&r);
    assert!(investigation::guard(&c,None,"marker",&["New".into()],"t").is_err());
    assert!(investigation::guard(&c,None,"optimize",&[],"t").is_ok());
}
#[tokio::test]
async fn marker_only_skip_requires_b_and_reuses_it_as_next_baseline() {
    let (_t,w,r,run)=automatic_setup();task(&w,&run,"marker");create(&w,&run,"Assets/Helper.cs").unwrap();finish(&w,&run);
    assert!(w.data.lock().unwrap().rounds[0].marker_only());
    assert!(w.accept_sampling(&r).unwrap_err().contains("导入重录 B"));
    assert_eq!(w.data.lock().unwrap().rounds[0].decision,"pending");
    workflow::bind_capture(w.clone(),public_capture("editor-dump.json"),"b".into(),None).await.unwrap();
    let bid=w.data.lock().unwrap().rounds[0].candidate.clone().unwrap();
    let revision=w.data.lock().unwrap().code_revision;
    {let mut d=w.data.lock().unwrap();d.captures.iter_mut().find(|c|c.id==bid).unwrap().code_revision=None;}
    assert!(w.accept_sampling(&r).is_err());
    {let mut d=w.data.lock().unwrap();let c=d.captures.iter_mut().find(|c|c.id==bid).unwrap();c.code_revision=Some(revision);c.scene_id="other-scene".into();}
    assert!(w.accept_sampling(&r).is_err());
    {let mut d=w.data.lock().unwrap();let sid=d.active_scene_id.clone();d.captures.iter_mut().find(|c|c.id==bid).unwrap().scene_id=sid;}
    w.accept_sampling(&r).unwrap();
    { let d=w.data.lock().unwrap();assert!(d.rounds[0].sampling_accepted());assert!(!scenes::unresolved(&d.rounds[0]));assert!(d.rounds[0].comparison.is_none()); }
    workflow::next_round(&w).unwrap();
    let d=w.data.lock().unwrap();let next=d.rounds.last().unwrap();
    assert_ne!(next.baseline,d.rounds[0].baseline);
    assert_eq!(next.baseline,bid);
    assert_eq!(d.rounds[0].candidate.as_ref(),Some(&bid));
    scenes::round_access(&d,&next.id,true).unwrap();
    assert!(d.captures.iter().find(|c|c.id==d.rounds[0].baseline).unwrap().code_revision.is_some());
    assert!(d.root.join("Assets/Helper.cs").exists());
    drop(d);
    let loaded=archive::load(&w.directory,&std::fs::read(w.directory.join("optimization.json")).unwrap()).unwrap();
    assert!(loaded.rounds[0].sampling_accepted());
}
#[test]
fn skip_rejects_mixed_unattributed_conflicting_and_empty_changes() {
    let (_t,w,r,run)=automatic_setup();assert!(w.accept_sampling(&r).is_err());
    create(&w,&run,"Assets/Helper.cs").unwrap();finish(&w,&run);
    assert!(w.accept_sampling(&r).is_err());
    {let mut d=w.data.lock().unwrap();d.rounds[0].tasks[0].kind="marker".into();} // proposals cannot reclassify applied optimization
    assert!(w.accept_sampling(&r).is_err());
    {let mut d=w.data.lock().unwrap();d.rounds[0].runs[0].tasks[0].kind="marker".into();d.rounds[0].runs[0].changes[0].state="conflict".into();}
    assert!(w.accept_sampling(&r).is_err());
    {let mut d=w.data.lock().unwrap();d.rounds[0].runs[0].changes[0].state="applied".into();d.rounds[0].runs[0].changes[0].task_id=None;}
    assert!(w.accept_sampling(&r).is_err());
}

fn first_marker_review(w:&Workspace,run:&str) {
    let mut review=history_review("marker");review["focus"]=json!(null);review["markers"]=json!([]);
    w.edit_query(run,"optimization_review",review).unwrap();
}
#[test]
fn marker_names_allow_a_complete_bounded_evidence_plan() {
    let names:Vec<_>=(0..16).map(|i|format!("Point.{i}")).collect();
    sampling::validate_names("marker",&names).unwrap();
    let mut too_many=names;too_many.push("Point.16".into());
    assert!(sampling::validate_names("marker",&too_many).is_err());
}
#[test]
fn applied_optimization_cannot_be_relabelled_to_skip_comparison() {
    let (_t,w,_r,run)=automatic_setup();create(&w,&run,"Assets/Helper.cs").unwrap();first_marker_review(&w,&run);
    let err=w.edit_query(&run,"optimization_task",json!({"id":"t","kind":"marker","title":"重分类","evidence":"public","instructions":"test","acceptance":"test"})).unwrap_err();
    assert!(err.contains("类型"));assert!(!w.data.lock().unwrap().rounds[0].marker_only());
}

fn waiting_investigation(w: &Workspace, run: &str) {
    let mut review=history_review("investigate");
    review["coverage"]["complete"]=json!(false);
    review["coverage"]["reproduction"]=json!("not_reproduced");
    review["focus"]["disposition"]=json!("awaiting_reproduction");
    review["evidencePlan"]=json!(null);
    w.edit_query(run,"optimization_review",review).unwrap();
    finish(w,run);
}
#[tokio::test]
async fn no_change_investigation_can_close_and_continue_with_new_recording() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    let root=w.data.lock().unwrap().root.clone();let before=std::fs::read(root.join("Assets/Work.cs")).unwrap();
    waiting_investigation(&w,&run);
    assert!(w.data.lock().unwrap().rounds.last().unwrap().investigation_only());
    w.finish_investigation(&r).unwrap();
    assert_eq!(w.data.lock().unwrap().rounds.last().unwrap().decision,"deferred");
    assert!(w.finish_investigation(&r).is_err());
    let loaded=archive::load(&w.directory,&std::fs::read(w.directory.join("optimization.json")).unwrap()).unwrap();
    assert_eq!(loaded.rounds.last().unwrap().decision,"deferred");
    workflow::next_round(&w).unwrap();
    let d=w.data.lock().unwrap();let next=d.rounds.last().unwrap();
    assert_ne!(next.baseline,d.rounds[d.rounds.len()-2].baseline);
    assert!(scenes::round_access(&d,&next.id,true).is_err());
    let context=investigation::context(&d,&next.id);
    assert_eq!(context["focus"]["taskId"],"t");
    assert_eq!(context["focus"]["latestReview"]["focus"]["disposition"],"awaiting_reproduction");
    assert_eq!(context["focus"]["latestReview"]["coverage"]["complete"],false);
    assert_eq!(std::fs::read(root.join("Assets/Work.cs")).unwrap(),before);
    assert!(d.rounds[d.rounds.len()-2].comparison.is_none());
    let next_id=next.id.clone();drop(d);
    workflow::bind_capture(w.clone(),public_capture("editor-dump.json"),"a".into(),None).await.unwrap();
    let d=w.data.lock().unwrap();
    scenes::round_access(&d,&next_id,true).unwrap();
    assert_eq!(d.rounds[d.rounds.len()-2].decision,"deferred");
    assert_eq!(investigation::context(&d,&next_id)["focus"]["latestReview"]["focus"]["disposition"],"awaiting_reproduction");
}
#[test]
fn no_change_completion_rejects_running_failed_unreviewed_and_changed_rounds() {
    let (_t,w,r,run)=automatic_setup();add_marker_history(&w);
    assert!(w.finish_investigation(&r).is_err());
    finish(&w,&run);assert!(w.finish_investigation(&r).is_err());
    {let mut d=w.data.lock().unwrap();d.rounds.last_mut().unwrap().runs[0].status="running".into();}
    waiting_investigation(&w,&run);
    for state in ["running","failed","interrupted","partial"] {
        w.data.lock().unwrap().rounds.last_mut().unwrap().runs[0].status=state.into();
        assert!(w.finish_investigation(&r).is_err(),"{state}");
    }
    {let mut d=w.data.lock().unwrap();d.rounds.last_mut().unwrap().runs[0].status="investigated".into();let change=d.rounds[0].runs[0].changes[0].clone();d.rounds.last_mut().unwrap().runs[0].changes.push(change);}
    assert!(w.finish_investigation(&r).is_err());
    w.data.lock().unwrap().rounds.last_mut().unwrap().decision="deferred".into();
    assert!(workflow::next_round(&w).is_err());
}
#[tokio::test]
#[ignore="Explicit read-only latest ProjectJinn investigation verification"]
async fn real_no_change_investigation_read_only() {
    let path=PathBuf::from(std::env::var("UPAA_INVESTIGATION_ARCHIVE").unwrap());
    let manifest=std::fs::read(path.join("optimization.json")).unwrap();
    let data=archive::load(&path,&manifest).unwrap();let r=data.rounds.last().unwrap();
    assert_eq!(r.id,"4291ed41-e909-4540-a2af-121a17296171");
    assert!(r.investigation_only());assert_eq!(r.decision,"pending");
    assert!(r.runs.len()>=2);assert!(r.runs.iter().all(|s|s.changes.is_empty() && s.status=="investigated"));
    let c=investigation::context(&data,&r.id);let review=r.runs.last().unwrap().investigation_review.as_ref().unwrap();
    investigation::validate(&c,review).unwrap();
    assert!(!review.coverage.as_ref().unwrap().complete);
    assert_eq!(review.focus.as_ref().unwrap().disposition,"awaiting_reproduction");
    assert_eq!(std::fs::read(path.join("optimization.json")).unwrap(),manifest);
    eprintln!("ProjectJinn: repeated zero-change investigations; saved review valid; can finish investigation; archive unchanged");
}

#[test]
fn legacy_zero_change_report_can_close_without_new_write_preflight() {
    let (_t,w,r,run)=automatic_setup();
    w.event(&run,&DiagnoseEvent::Chunk{text:"已调查，当前证据不足；未修改代码，等待新的复现录制".into()}).unwrap();
    finish(&w,&run);
    assert!(w.data.lock().unwrap().rounds[0].runs[0].investigation_review.is_none());
    w.finish_investigation(&r).unwrap();
    workflow::next_round(&w).unwrap();
    assert_eq!(w.data.lock().unwrap().rounds[0].decision,"deferred");
}

#[test]
fn legacy_marker_only_acceptance_without_b_still_requires_fresh_recording() {
    let (_t,w,_r,run)=automatic_setup();task(&w,&run,"marker");create(&w,&run,"Assets/Helper.cs").unwrap();finish(&w,&run);
    w.data.lock().unwrap().rounds[0].decision="accepted".into();
    workflow::next_round(&w).unwrap();
    let d=w.data.lock().unwrap();let next=d.rounds.last().unwrap();
    assert!(scenes::round_access(&d,&next.id,true).is_err());
    assert!(next.workflow.reason.as_ref().unwrap().contains("旧版"));
}
