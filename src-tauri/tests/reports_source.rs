//! Public fixtures only. No private project is sent to an Agent.
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
use unity_profiler_analysis_agent_lib::{
    acp_client::DiagnoseEvent,
    extractor, parser,
    reports::{self, Report},
    source::SourceScope,
};
fn bridge_executable() -> PathBuf {
    std::env::var_os("UPAA_TEST_APP_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_unity-profiler-analysis-agent")))
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("upaa-source-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    fn scope(&self) -> SourceScope {
        SourceScope::prepare(
            "fixture".into(),
            self.0.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn report() -> Report {
    let profile = parser::json::parse(
        &bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json")),
        "public.json",
        0,
    )
    .await
    .unwrap();
    Report::new(
        "r".into(),
        "fixture".into(),
        "fixture-agent".into(),
        None,
        &extractor::extract(&profile),
    )
}
#[test]
fn source_search_read_and_hash_cover_encoding_paging_and_changes() {
    let root = Temp::new();
    root.write("Assets/中文.cs", "class Work {\n void Update() {}\n}\n");
    root.write("Packages/Local/Work.cs", "class Work {}\n");
    root.write("Library/Cache.cs", "class Work {}");
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend("class 中文 {}".encode_utf16().flat_map(u16::to_le_bytes));
    root.write("Utf16.cs", utf16);
    root.write("bad.cs", [0xff, 0]);
    root.write("large.cs", vec![0; reports::MAX_REPORT + 1]);
    let scope = root.scope();
    assert_eq!(scope.info.file_count, 3);
    assert!(!scope.info.warnings.is_empty());
    let found = scope
        .query("source_search", json!({"query":"Work","limit":1}))
        .unwrap();
    assert_eq!(found["rows"][0]["line"], 1);
    assert_eq!(found["nextStart"], 1);
    let second = scope
        .query("source_search", json!({"query":"Work","start":1}))
        .unwrap();
    assert_eq!(second["rows"].as_array().unwrap().len(), 1);
    let page = scope
        .query(
            "source_read",
            json!({"path":"Assets/中文.cs","start_line":2,"limit":1}),
        )
        .unwrap();
    assert_eq!(page["rows"][0]["line"], 2);
    assert!(page["rows"][0]["text"].as_str().unwrap().contains("Update"));
    assert_eq!(page["nextStart"], 3);
    assert!(scope.text("../outside.cs").is_err());
    assert!(scope.text("Library/Cache.cs").is_err());
    assert!(scope.query("source_files", json!({"limit":101})).is_err());
    root.write("Assets/中文.cs", "class Changed {}");
    assert!(scope.text("Assets/中文.cs").unwrap_err().contains("已变化"));
    scope
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(scope.query("source_files", json!({})).is_err());
}
#[test]
fn source_responses_are_bounded_and_explicitly_truncated() {
    let root = Temp::new();
    root.write(
        "Long.cs",
        format!("{}\n{}", "中".repeat(20_000), "line\n".repeat(600)),
    );
    let scope = root.scope();
    let page = scope
        .query("source_read", json!({"path":"Long.cs","limit":400}))
        .unwrap();
    assert_eq!(page["lineTruncated"], true);
    assert!(page["nextStart"].is_number());
    assert!(serde_json::to_vec(&page).unwrap().len() < 65536);
    let mut wire = rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&page).unwrap(),
    )]);
    wire.structured_content = Some(page.clone());
    assert!(serde_json::to_vec(&wire).unwrap().len() + 256 < 65536);
    let search = scope.query("source_search", json!({"query":"中"})).unwrap();
    assert_eq!(search["lineTruncated"], true);
    assert_eq!(search["rows"][0]["lineTruncated"], true);
}
#[cfg(windows)]
#[test]
fn windows_junctions_are_not_followed_even_when_added_after_indexing() {
    let root = Temp::new();
    let outside = Temp::new();
    root.write("Assets/Work.cs", "class Safe {}");
    outside.write("Secret.cs", "class Secret {}");
    let scope = root.scope();
    let junction = root.0.join("Linked");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside.0)
        .creation_flags(0x08000000)
        .status()
        .unwrap();
    assert!(status.success());
    let new_scope = root.scope();
    assert_eq!(new_scope.info.file_count, 1);
    assert!(new_scope.text("Linked/Secret.cs").is_err());
    assert!(scope.text("Linked/Secret.cs").is_err());
    std::fs::remove_dir(&junction).unwrap();
}
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[tokio::test]
async fn report_limit_preserves_prefix_and_terminal_cannot_be_overwritten() {
    let mut empty = report().await;
    empty.apply(&DiagnoseEvent::Finished {
        total_chunks: 0,
        stop_reason: "end_turn".into(),
    });
    assert_eq!(empty.status, "incomplete");
    let mut r = report().await;
    r.apply(&DiagnoseEvent::Chunk {
        text: "中".repeat(reports::MAX_REPORT / 3 + 1),
    });
    assert!(r.text.len() <= reports::MAX_REPORT);
    assert!(r.text.starts_with('中'));
    assert_eq!(r.status, "incomplete");
    r.apply(&DiagnoseEvent::Finished {
        total_chunks: 1,
        stop_reason: "end_turn".into(),
    });
    assert_eq!(r.status, "incomplete");
    assert!(r.markdown().contains("不完整") || r.markdown().contains("incomplete"));
    let mut limited = report().await;
    limited.apply(&DiagnoseEvent::Error {
        message: "其他错误: REPORT_LIMIT: 上限".into(),
    });
    assert_eq!(limited.status, "incomplete");
}
#[test]
fn markdown_tables_code_and_html_are_safe_and_offline() {
    let text="# 中文\n\n|a|b|\n|-|-|\n|1|2|\n\n```cs\nvar s = \"<script>\";\n```\n\n<script>alert(1)</script>\n\n![remote](https://evil.test/x.png)\n\n[x](javascript:alert%281%29)";
    let html = reports::document(text);
    assert!(html.contains("<table>"));
    assert!(html.contains("<pre><code"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("href=\"javascript:"));
    assert!(html.contains("default-src 'none'"));
}
#[tokio::test]
async fn exports_validate_recording_parentage_empty_status_and_retry_after_failed_write() {
    let root = Temp::new();
    let mut first = report().await;
    let mut store = std::collections::HashMap::from([("r".into(), first.clone())]);
    assert!(reports::select_reports(&store, "fixture", &["r".into()]).is_err());
    first.apply(&DiagnoseEvent::Chunk {
        text: "## 中文\n\n|列|值|\n|-|-|\n|GC|0 B|\n\n```cs\nnew byte[16];\n```".into(),
    });
    first.apply(&DiagnoseEvent::Cancelled);
    let mut source = first.clone();
    source.report_id = "s".into();
    source.session_id = "s".into();
    source.stage = "source".into();
    source.parent_report_id = Some("r".into());
    source.status = "failed".into();
    source.incomplete_reason = Some("测试失败".into());
    store.insert("r".into(), first);
    store.insert("s".into(), source.clone());
    assert!(reports::select_reports(&store, "other-recording", &["r".into()]).is_err());
    assert!(reports::select_reports(&store, "fixture", &["r".into(), "r".into()]).is_err());
    source.parent_report_id = Some("unrelated".into());
    store.insert("s".into(), source.clone());
    assert!(reports::select_reports(&store, "fixture", &["r".into(), "s".into()]).is_err());
    source.parent_report_id = Some("r".into());
    store.insert("s".into(), source);
    let selected = reports::select_reports(&store, "fixture", &["r".into(), "s".into()]).unwrap();
    assert!(reports::save_reports(&selected, "html", &root.0.join("missing/report.html")).is_err());
    for (format, name) in [("markdown", "报告.md"), ("html", "报告.html")] {
        let target = root.0.join(name);
        std::fs::write(&target, "replace after confirmed save").unwrap();
        reports::save_reports(&selected, format, &target).unwrap();
        let body = std::fs::read_to_string(target).unwrap();
        assert!(body.contains("cancelled") && body.contains("failed"));
        assert!(body.contains("中文") && body.contains("测试失败"));
        assert!(body.contains("性能诊断报告") && body.contains("C# 源码定位报告"));
    }
    assert_eq!(store["r"].text, selected[0].text);
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 2);
}
#[tokio::test]
async fn source_tools_require_a_scoped_session() {
    use unity_profiler_analysis_agent_lib::mcp::{tools::dispatch, MetricsStore};
    let store = MetricsStore::new();
    assert!(dispatch(&store, "source_files", json!({})).await.is_err());
    let root = Temp::new();
    root.write("Work.cs", "class Work {}");
    store.set_source(Some(Arc::new(root.scope()))).await;
    let other_store = MetricsStore::new();
    let other_root = Temp::new();
    other_root.write("Other.cs", "class Other {}");
    other_store
        .set_source(Some(Arc::new(other_root.scope())))
        .await;
    assert!(
        dispatch(&other_store, "source_read", json!({"path":"Work.cs"}))
            .await
            .is_err()
    );
    assert!(dispatch(&store, "source_read", json!({"path":"Other.cs"}))
        .await
        .is_err());
    assert_eq!(
        dispatch(&store, "source_files", json!({})).await.unwrap()["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    store.clear().await;
    assert!(dispatch(&store, "source_read", json!({"path":"Work.cs"}))
        .await
        .is_err());
}

async fn stored_session(
    mode: &str,
) -> (
    unity_profiler_analysis_agent_lib::state::AppState,
    String,
    tokio::sync::mpsc::UnboundedReceiver<DiagnoseEvent>,
) {
    use unity_profiler_analysis_agent_lib::{acp_client::agents::AgentPreset, state::AppState};
    let state = AppState::new();
    let profile = parser::json::parse(
        &bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json")),
        "public.json",
        0,
    )
    .await
    .unwrap();
    state
        .put_snapshot("fixture".into(), extractor::extract(&profile))
        .await;
    let preset = AgentPreset {
        id: "public-fixture".into(),
        label: "fixture".into(),
        command: "node".into(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/acp-agent.cjs")
                .to_string_lossy()
                .into(),
            mode.into(),
        ],
        description: "public test".into(),
        available: true,
    };
    let (id, _, rx) = state
        .start_session("fixture", preset, bridge_executable())
        .await
        .unwrap();
    (state, id, rx)
}
#[tokio::test]
async fn actual_acp_overflow_stops_and_preserves_a_backend_report() {
    let (state, id, mut rx) = stored_session("overflow").await;
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        while let Some(e) = rx.recv().await {
            if e.terminal() {
                assert!(matches!(e, DiagnoseEvent::Error { .. }));
                break;
            }
        }
    })
    .await
    .unwrap();
    let inner = state.0.lock().await;
    let report = &inner.reports[&id];
    assert_eq!(report.status, "incomplete");
    assert!(report.text.starts_with("保留开头"));
    assert_eq!(report.text.len(), reports::MAX_REPORT);
    drop(inner);
    state.release_file("fixture").await;
    assert!(state.0.lock().await.reports.is_empty());
}
#[tokio::test]
async fn source_session_uses_new_protocol_context_and_preserves_parent_report() {
    use unity_profiler_analysis_agent_lib::{acp_client::agents::AgentPreset, state::AppState};
    let state = AppState::new();
    let profile = parser::json::parse(
        &bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json")),
        "public.json",
        0,
    )
    .await
    .unwrap();
    state
        .put_snapshot("fixture".into(), extractor::extract(&profile))
        .await;
    {
        let mut inner = state.0.lock().await;
        inner
            .details
            .insert("fixture".into(), profile.details.unwrap());
        let mut parent = report().await;
        parent.apply(&DiagnoseEvent::Chunk {
            text: "首轮报告：帧 10 的分配需要调查。".into(),
        });
        parent.apply(&DiagnoseEvent::Finished {
            total_chunks: 1,
            stop_reason: "end_turn".into(),
        });
        inner.reports.insert("r".into(), parent);
    }
    let scope = Arc::new(
        SourceScope::prepare(
            "fixture".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-project"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let scope_id = scope.info.scope_id.clone();
    state.0.lock().await.sources.insert(scope_id.clone(), scope);
    let preset = AgentPreset {
        id: "fixture".into(),
        label: "fixture".into(),
        command: "node".into(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/acp-agent.cjs")
                .to_string_lossy()
                .into(),
            "source".into(),
        ],
        description: "public".into(),
        available: true,
    };
    let (id, _, mut rx) = state
        .start_report_session(
            "fixture",
            preset.clone(),
            bridge_executable(),
            Some(("r".into(), scope_id)),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        while let Some(e) = rx.recv().await {
            if e.terminal() {
                assert!(matches!(e, DiagnoseEvent::Finished { .. }), "{e:?}");
                break;
            }
        }
    })
    .await
    .unwrap();
    let inner = state.0.lock().await;
    assert_eq!(inner.reports["r"].status, "completed");
    assert_eq!(inner.reports[&id].stage, "source");
    assert!(inner.reports[&id].text.contains("AllocationWork.cs"));
    assert_eq!(inner.reports.len(), 2);
    drop(inner);
    state.finish_session(&id).await;
    let new_scope = Arc::new(
        SourceScope::prepare(
            "fixture".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-project"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let new_scope_id = new_scope.info.scope_id.clone();
    state
        .0
        .lock()
        .await
        .sources
        .insert(new_scope_id.clone(), new_scope.clone());
    let mut cancelling = preset;
    *cancelling.args.last_mut().unwrap() = "cancel".into();
    let (cancel_id, _, mut cancel_rx) = state
        .start_report_session(
            "fixture",
            cancelling,
            bridge_executable(),
            Some(("r".into(), new_scope_id)),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if matches!(cancel_rx.recv().await.unwrap(), DiagnoseEvent::Chunk { .. }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    state.cancel_session(&cancel_id).await;
    let inner = state.0.lock().await;
    assert_eq!(inner.reports["r"].status, "completed");
    assert_eq!(inner.reports[&cancel_id].status, "cancelled");
    assert!(!inner.reports.contains_key(&id));
    assert!(new_scope.text("Assets/AllocationWork.cs").is_err());
    drop(inner);
    state.release_file("fixture").await;
}

#[tokio::test]
#[ignore = "requires UPAA_REAL_AGENT; sends only public synthetic profiler data and public C# fixtures"]
async fn real_agent_locates_public_csharp_with_evidence() {
    use unity_profiler_analysis_agent_lib::acp_client::{
        self, agents::AgentPreset, DiagnoseRequest,
    };
    let command = std::env::var("UPAA_REAL_AGENT").expect("set UPAA_REAL_AGENT");
    let mut capture: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/isolated-peak.json")).unwrap();
    for frame in capture["frames"].as_array_mut().unwrap() {
        for thread in frame["threads"].as_array_mut().unwrap() {
            for sample in thread["samples"].as_array_mut().unwrap() {
                if sample["marker_name"] == "Update" {
                    sample["marker_name"] = json!("AllocationWork.Update");
                } else if sample["marker_name"] == "Work" {
                    sample["marker_name"] = json!("Unmapped.Native");
                }
            }
        }
    }
    let data = bytes::Bytes::from(serde_json::to_vec(&capture).unwrap());
    let profile = parser::json::parse(&data, "public-source-peak.json", data.len() as u64)
        .await
        .unwrap();
    let source = Arc::new(
        SourceScope::prepare(
            "public".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source-project"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let before = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/source-project/Assets/AllocationWork.cs"),
    )
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle=acp_client::start_diagnose(AgentPreset{id:"real-source-test".into(),label:"real".into(),command,args:vec![],description:"public C# validation".into(),available:true},DiagnoseRequest{file_id:"public".into(),agent_id:"real-source-test".into(),snapshot:extractor::extract(&profile),details:profile.details,bridge_executable:std::env::var_os("UPAA_TEST_APP_EXE").map(PathBuf::from).unwrap_or_else(||bridge_executable()),event_tx:tx,source:Some(source),project:None,parent_report:Some("首轮诊断：公开合成录制帧 160 有 40 ms 主线程峰值与 8 MiB GC 分配，需要定位 AllocationWork.Update 的分配；Unmapped.Native 也需核对。帧 100 可作普通帧参考。不能从名称推断具体源码或版本一致性。".into())}).await.unwrap();
    let result=tokio::time::timeout(std::time::Duration::from_secs(420),async{
  let mut read=false;let mut performance=false;let mut text=String::new();let mut terminal=None;
  while let Some(event)=rx.recv().await{match event{DiagnoseEvent::McpCall{tool,args}=>{println!("MCP {tool} {args}");read|=tool=="source_read" && args["path"]=="Assets/AllocationWork.cs";performance|=tool.starts_with("performance_");},DiagnoseEvent::Chunk{text:t}=>text.push_str(&t),e if e.terminal()=>terminal=Some(e),_=>{}}}
  println!("PUBLIC_SOURCE_ANSWER_BEGIN\n{text}\nPUBLIC_SOURCE_ANSWER_END\n{terminal:?}");
  assert!(read && performance,"must read original code and performance evidence");assert!(text.contains("AllocationWork.cs"));assert!(matches!(terminal,Some(DiagnoseEvent::Finished{stop_reason,..}) if stop_reason=="end_turn"));
 }).await;
    handle.cancel().await;
    result.expect("real source diagnosis timed out");
    assert_eq!(
        before,
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/source-project/Assets/AllocationWork.cs")
        )
        .unwrap()
    );
}

#[tokio::test]
async fn project_session_preserves_reports_and_isolates_tools() {
    use unity_profiler_analysis_agent_lib::{acp_client::agents::AgentPreset, state::AppState};
    let state = AppState::new();
    let profile = parser::json::parse(
        &bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json")),
        "public.json",
        0,
    )
    .await
    .unwrap();
    state
        .put_snapshot("fixture".into(), extractor::extract(&profile))
        .await;
    {
        let mut inner = state.0.lock().await;
        inner
            .details
            .insert("fixture".into(), profile.details.unwrap());
        let mut parent = report().await;
        parent.apply(&DiagnoseEvent::Chunk {
            text: "首轮报告：帧 10 的分配需要调查。".into(),
        });
        parent.apply(&DiagnoseEvent::Finished {
            total_chunks: 1,
            stop_reason: "end_turn".into(),
        });
        inner.reports.insert("r".into(), parent);
    }
    let scope = Arc::new(
        unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
            "fixture".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/unity-project"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let scope_id = scope.info.scope_id.clone();
    state
        .0
        .lock()
        .await
        .projects
        .insert(scope_id.clone(), scope);
    let preset = AgentPreset {
        id: "fixture".into(),
        label: "fixture".into(),
        command: "node".into(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/acp-agent.cjs")
                .to_string_lossy()
                .into(),
            "project".into(),
        ],
        description: "public".into(),
        available: true,
    };
    let (id, _, mut rx) = state
        .start_report_session(
            "fixture",
            preset.clone(),
            bridge_executable(),
            Some(("r".into(), scope_id)),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        while let Some(e) = rx.recv().await {
            if e.terminal() {
                assert!(matches!(e, DiagnoseEvent::Finished { .. }), "{e:?}");
                break;
            }
        }
    })
    .await
    .unwrap();
    let inner = state.0.lock().await;
    assert_eq!(inner.reports["r"].status, "completed");
    assert_eq!(inner.reports[&id].stage, "project");
    assert!(inner.reports[&id].text.contains("AllocationWork.cs"));
    assert_eq!(inner.reports.len(), 2);
    drop(inner);
    state.finish_session(&id).await;
    let new_scope = Arc::new(
        unity_profiler_analysis_agent_lib::project::ProjectScope::prepare(
            "fixture".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/unity-project"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap(),
    );
    let new_scope_id = new_scope.info.scope_id.clone();
    state
        .0
        .lock()
        .await
        .projects
        .insert(new_scope_id.clone(), new_scope.clone());
    let mut cancelling = preset;
    *cancelling.args.last_mut().unwrap() = "cancel".into();
    let (cancel_id, _, mut cancel_rx) = state
        .start_report_session(
            "fixture",
            cancelling,
            bridge_executable(),
            Some(("r".into(), new_scope_id)),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if matches!(cancel_rx.recv().await.unwrap(), DiagnoseEvent::Chunk { .. }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    state.cancel_session(&cancel_id).await;
    let inner = state.0.lock().await;
    assert_eq!(inner.reports["r"].status, "completed");
    assert_eq!(inner.reports[&cancel_id].status, "cancelled");
    assert!(!inner.reports.contains_key(&id));
    assert!(new_scope
        .query("project_read", json!({"path":"Assets/AllocationWork.cs"}))
        .await
        .is_err());
    drop(inner);
    state.release_file("fixture").await;
}
