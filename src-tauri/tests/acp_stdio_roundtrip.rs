//! ACP sessions against an independent Node fixture and the real MCP bridge.
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use unity_profiler_analysis_agent_lib::{
    acp_client::{self, agents::AgentPreset, DiagnoseEvent, DiagnoseRequest},
    extractor, parser,
};

async fn launch(
    command: String,
    args: Vec<String>,
) -> (
    acp_client::client::SessionHandle,
    mpsc::UnboundedReceiver<DiagnoseEvent>,
) {
    let bytes = bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json"));
    let p = parser::json::parse(&bytes, "fixture.json", bytes.len() as u64)
        .await
        .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let preset = AgentPreset {
        id: "test".into(),
        label: "test".into(),
        command,
        args,
        description: "protocol test".into(),
        available: true,
    };
    let handle = acp_client::start_diagnose(
        preset,
        DiagnoseRequest {
            file_id: "fixture".into(),
            agent_id: "test".into(),
            snapshot: extractor::extract(&p),
            details: p.details,
            bridge_executable: PathBuf::from(
                std::env::var_os("UPAA_TEST_APP_EXE")
                    .unwrap_or_else(|| env!("CARGO_BIN_EXE_unity-profiler-analysis-agent").into()),
            ),
            event_tx: tx,
        },
    )
    .await
    .unwrap();
    (handle, rx)
}
async fn fixture(
    mode: &str,
) -> (
    acp_client::client::SessionHandle,
    mpsc::UnboundedReceiver<DiagnoseEvent>,
) {
    launch(
        "node".into(),
        vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/acp-agent.cjs")
                .to_string_lossy()
                .into_owned(),
            mode.into(),
        ],
    )
    .await
}
async fn collect(rx: &mut mpsc::UnboundedReceiver<DiagnoseEvent>) -> Vec<DiagnoseEvent> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    })
    .await
    .expect("session did not terminate")
}
#[tokio::test]
async fn actual_acp_handshake_mcp_query_stream_and_single_terminal() {
    let (handle, mut rx) = fixture("success").await;
    let events = collect(&mut rx).await;
    handle.wait().await;
    assert_eq!(
        events.iter().filter(|e| e.terminal()).count(),
        1,
        "{events:?}"
    );
    assert!(
        matches!(events.last(),Some(DiagnoseEvent::Finished{total_chunks:1,stop_reason}) if stop_reason=="end_turn"),
        "{events:?}"
    );
    assert!(events
        .iter()
        .any(|e| matches!(e,DiagnoseEvent::Chunk{text} if text.contains("20 B"))));
    assert!(!events
        .iter()
        .any(|e| matches!(e,DiagnoseEvent::Chunk{text} if text.contains("STALE"))));
    assert!(events
        .iter()
        .any(|e| matches!(e,DiagnoseEvent::Log{message} if message.contains("ordinary"))));
    assert!(events
        .iter()
        .any(|e| matches!(e,DiagnoseEvent::McpCall{tool,..} if tool=="performance_cpu_hierarchy")));
}
#[tokio::test]
async fn errors_never_emit_success_terminal() {
    for mode in ["error", "malformed", "version", "limit"] {
        let (handle, mut rx) = fixture(mode).await;
        let events = collect(&mut rx).await;
        handle.wait().await;
        assert_eq!(events.iter().filter(|e| e.terminal()).count(), 1);
        assert!(
            matches!(events.last(), Some(DiagnoseEvent::Error { .. })),
            "{mode}: {events:?}"
        );
    }
}
#[tokio::test]
async fn cancellation_during_initialization_is_bounded_and_idempotent() {
    let (handle, mut rx) = fixture("hang-initialize").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    tokio::time::timeout(Duration::from_secs(5), handle.cancel())
        .await
        .unwrap();
    handle.cancel().await;
    let events = collect(&mut rx).await;
    assert!(
        matches!(events.last(), Some(DiagnoseEvent::Cancelled)),
        "{events:?}"
    );
}

#[tokio::test]
async fn app_state_rejects_duplicate_sessions_and_release_cancels_owned_work() {
    use unity_profiler_analysis_agent_lib::state::AppState;
    let bytes = bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json"));
    let profile = parser::json::parse(&bytes, "fixture", bytes.len() as u64)
        .await
        .unwrap();
    let state = AppState::new();
    state
        .put_snapshot("a".into(), extractor::extract(&profile))
        .await;
    let preset = AgentPreset {
        id: "test".into(),
        label: "test".into(),
        command: "node".into(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/acp-agent.cjs")
                .to_string_lossy()
                .into_owned(),
            "hang-initialize".into(),
        ],
        description: "fixture".into(),
        available: true,
    };
    let exe = PathBuf::from(
        std::env::var_os("UPAA_TEST_APP_EXE")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_unity-profiler-analysis-agent").into()),
    );
    let (id, handle, mut rx) = state
        .start_session("a", preset.clone(), exe.clone())
        .await
        .unwrap();
    assert!(state.start_session("a", preset, exe).await.is_err());
    state.cancel_session("not-this-session").await;
    assert!(state.0.lock().await.active_sessions.contains_key(&id));
    tokio::time::timeout(Duration::from_secs(5), state.release_file("a"))
        .await
        .unwrap();
    handle.wait().await;
    assert!(state.0.lock().await.active_sessions.is_empty());
    assert!(state.get_snapshot("a").await.is_none());
    assert!(matches!(
        collect(&mut rx).await.last(),
        Some(DiagnoseEvent::Cancelled)
    ));
}

#[tokio::test]
#[ignore = "requires UPAA_REAL_AGENT; cancels after the first real MCP query on the public fixture"]
async fn real_agent_cancellation_closes_session() {
    let command = std::env::var("UPAA_REAL_AGENT").expect("set UPAA_REAL_AGENT");
    let (handle, mut rx) = launch(command, vec![]).await;
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match rx.recv().await {
                Some(DiagnoseEvent::McpCall { tool, .. }) => {
                    println!("cancel after MCP {tool}");
                    break;
                }
                Some(e) if e.terminal() => panic!("terminated before MCP: {e:?}"),
                None => panic!("no MCP call"),
                _ => {}
            }
        }
    })
    .await
    .expect("no real MCP query before deadline");
    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(10), handle.cancel())
        .await
        .unwrap();
    let events = collect(&mut rx).await;
    let confirmed = events
        .iter()
        .any(|e| matches!(e,DiagnoseEvent::Log{message} if message=="ACP 取消已确认"));
    println!(
        "cancel_elapsed={:?}, protocol_confirmed={confirmed}",
        started.elapsed()
    );
    assert!(
        matches!(events.last(), Some(DiagnoseEvent::Cancelled)),
        "{events:?}"
    );
}
#[tokio::test]
#[cfg(windows)]
async fn cancellation_reaps_adapter_descendants_even_without_cancel_response() {
    for mode in ["cancel", "uncooperative"] {
        let (handle, mut rx) = fixture(mode).await;
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = rx.recv().await {
                if let DiagnoseEvent::Log { message } = event {
                    if let Some(pid) = message.split("DESCENDANT_PID=").nth(1) {
                        return pid.trim().parse::<u32>().unwrap();
                    }
                }
            }
            panic!("no descendant PID")
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(6), handle.cancel())
            .await
            .unwrap();
        let events = collect(&mut rx).await;
        assert!(
            matches!(events.last(), Some(DiagnoseEvent::Cancelled)),
            "{events:?}"
        );
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !process.is_null() {
                assert_eq!(
                    WaitForSingleObject(process, 2000),
                    0,
                    "descendant survived cancellation"
                );
                CloseHandle(process);
            }
        }
        #[cfg(not(windows))]
        let _ = pid;
    }
}
#[tokio::test]
#[ignore = "requires UPAA_REAL_AGENT; sends only the public synthetic fixture to an authenticated Agent"]
async fn real_agent_queries_profiler_over_mcp() {
    let command = std::env::var("UPAA_REAL_AGENT").expect("set UPAA_REAL_AGENT");
    let (handle, mut rx) = launch(command, vec![]).await;
    let result=tokio::time::timeout(Duration::from_secs(330),async {
        let mut queried=false;let mut text=String::new();let mut terminal=None;
        while let Some(event)=rx.recv().await {
            match event {
                DiagnoseEvent::McpCall{tool,..}=>{println!("MCP {tool}");queried=true;},
                DiagnoseEvent::Chunk{text:chunk}=>text.push_str(&chunk),
                event if event.terminal()=>terminal=Some(event),
                _=>{},
            }
        }
        println!("answer_chars={}, terminal={terminal:?}",text.len());
        assert!(queried,"Agent never queried MCP");
        assert!(!text.is_empty());
        assert!(matches!(terminal,Some(DiagnoseEvent::Finished{stop_reason,..}) if stop_reason=="end_turn"));
    }).await;
    handle.cancel().await;
    result.expect("real Agent timed out");
}
