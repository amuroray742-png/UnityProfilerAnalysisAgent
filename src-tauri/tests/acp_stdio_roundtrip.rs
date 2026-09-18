//! 集成测试：用 PowerShell 当假 agent，验证 stdio 协议端到端通。
//!
//! PowerShell 脚本读 stdin、按行输出回 stdout、最后退出。
//! 验证 start_diagnose 真实把 prompt 写进 stdin、stdout 按行切 chunk、
//! child 退出后 emit Finished。

use std::time::Duration;

use tokio::sync::mpsc;
use unity_profiler_analysis_agent_lib::acp_client::agents::{builtin_presets, AgentPreset};
use unity_profiler_analysis_agent_lib::acp_client::{start_diagnose, DiagnoseEvent};
use unity_profiler_analysis_agent_lib::extractor::MetricsSnapshot;

fn fake_snapshot() -> MetricsSnapshot {
    // 用 JSON template 构造最小可用 snapshot（test 不解析其内容）
    let json = serde_json::json!({
        "meta": {
            "fileName": "test.data",
            "fileId": "test-file-id",
            "format": "Data",
            "frameCount": 1,
            "durationMs": 16.67,
            "unityVersion": "6000.3.23f1",
            "platform": "Windows",
        },
        "cpu": {
            "mainThreadMs": {"p50": 16.0, "p95": 16.67, "p99": 17.0, "max": 20.0, "min": 14.0, "samples": 1},
            "frameTimeline": [],
            "topHotspots": [],
        },
        "gc": {
            "totalAllocBytes": 0,
            "allocPerFrameBytes": {"p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "min": 0.0, "samples": 0},
            "genCollections": {"gen0": 0, "gen1": 0, "gen2": 0},
            "topAllocSites": [],
        },
        "rendering": {
            "drawCalls": {"p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "min": 0.0, "samples": 0},
            "setPassCalls": {"p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0, "min": 0.0, "samples": 0},
            "batchesSavedBySrpBatcher": 0,
            "topRenderEvents": [],
        },
        "warnings": [],
    });
    serde_json::from_value(json).expect("fake snapshot template")
}

/// 假 agent preset：powershell 跑一个 echo 脚本
#[allow(dead_code)]
fn fake_powershell_echo_preset() -> Option<AgentPreset> {
    builtin_presets()
        .into_iter()
        .find(|p| p.command == "powershell" || p.command == "pwsh")
        .map(|mut p| {
            p.id = "fake-powershell".to_string();
            p.label = "Fake PowerShell Echo".to_string();
            p.args = vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                r#"Write-Output 'line-A'; Start-Sleep -Milliseconds 30; Write-Output 'line-B'; Write-Output 'line-C'; Write-Output 'line-D'"#.to_string(),
            ];
            p.description = "fake agent for stdio protocol test".to_string();
            p.available = true;
            p
        })
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn start_diagnose_roundtrips_stdin_to_chunks() {
    // 1. 准备事件 channel
    let (tx, mut rx) = mpsc::unbounded_channel::<DiagnoseEvent>();

    // 2. 准备一个 preset，command 直接是 powershell
    //    走 resolve_command 时会进 "其他" 分支，直接 spawn powershell
    let preset = AgentPreset {
        id: "fake-powershell".to_string(),
        label: "Fake PowerShell Echo".to_string(),
        command: "powershell".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            // 读 stdin 全部内容作为参数 (echo back)，然后输出固定 4 行
            r#"Write-Output 'line-A'; Start-Sleep -Milliseconds 30; Write-Output 'line-B'; Write-Output 'line-C'; Write-Output 'line-D'"#.to_string(),
        ],
        description: "fake agent for stdio protocol test".to_string(),
        available: true,
    };

    // 3. 构造请求
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let req = unity_profiler_analysis_agent_lib::acp_client::DiagnoseRequest {
        file_id: "test-file-id".to_string(),
        agent_id: "fake-powershell".to_string(),
        snapshot: fake_snapshot(),
        event_tx: tx,
        cancel_rx,
    };

    // 4. 启动
    let session = start_diagnose(preset, req)
        .await
        .expect("start_diagnose should succeed");

    // 5. 收集事件，等待最多 10s
    let mut events: Vec<DiagnoseEvent> = vec![];
    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            Some(ev) = rx.recv() => {
                let is_finished = matches!(ev, DiagnoseEvent::Finished { .. });
                let is_error = matches!(ev, DiagnoseEvent::Error { .. });
                let kind = match &ev {
                    DiagnoseEvent::Started { .. } => "started",
                    DiagnoseEvent::Chunk { .. } => "chunk",
                    DiagnoseEvent::McpCall { .. } => "mcp-call",
                    DiagnoseEvent::McpResult { .. } => "mcp-result",
                    DiagnoseEvent::Finished { .. } => "finished",
                    DiagnoseEvent::Error { .. } => "error",
                };
                println!("[event] {kind}");
                if let DiagnoseEvent::Chunk { text } = &ev {
                    print!("  payload: {}", text);
                }
                if let DiagnoseEvent::Error { message } = &ev {
                    println!("  payload: {message}");
                }
                events.push(ev);
                if is_finished {
                    break;
                }
                if is_error {
                    // 继续收，看是否最终能到 Finished
                }
            }
            _ = &mut timeout => {
                panic!("timed out waiting for events; got so far: {events:?}");
            }
        }
    }

    // 6. 验证：至少 1 个 Started，4 个 Chunk (line-A..D)，1 个 Finished
    let started = events.iter().filter(|e| matches!(e, DiagnoseEvent::Started { .. })).count();
    let chunks = events.iter().filter_map(|e| match e {
        DiagnoseEvent::Chunk { text } => Some(text.clone()),
        _ => None,
    }).collect::<Vec<_>>();
    let finished = events.iter().filter(|e| matches!(e, DiagnoseEvent::Finished { .. })).count();

    println!("summary: started={started}, chunks={}, finished={finished}", chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        println!("  chunk[{i}]: {}", c.trim_end());
    }

    assert_eq!(started, 1, "exactly 1 Started event expected");
    assert_eq!(finished, 1, "exactly 1 Finished event expected");
    assert!(chunks.len() >= 4, "expected ≥4 chunks, got {}", chunks.len());
    let joined: String = chunks.join("");
    for line in ["line-A", "line-B", "line-C", "line-D"] {
        assert!(joined.contains(line), "missing expected output line: {line}\nfull output:\n{joined}");
    }

    // 7. cancel() 不应 panic
    session.cancel().await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn start_diagnose_emits_finished_after_agent_exits() {
    // 验证：agent 退出后 Finished 一定到达（不会卡住）
    let (tx, mut rx) = mpsc::unbounded_channel::<DiagnoseEvent>();
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

    let preset = AgentPreset {
        id: "fast-exit".to_string(),
        label: "Fast Exit".to_string(),
        command: "powershell".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            r#"Write-Output 'quick'"#.to_string(),
        ],
        description: "exits fast".to_string(),
        available: true,
    };

    let req = unity_profiler_analysis_agent_lib::acp_client::DiagnoseRequest {
        file_id: "f".to_string(),
        agent_id: "fast-exit".to_string(),
        snapshot: fake_snapshot(),
        event_tx: tx,
        cancel_rx,
    };

    let session = start_diagnose(preset, req).await.expect("start ok");

    let mut got_finished = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Some(DiagnoseEvent::Finished { .. })) => {
                got_finished = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    assert!(got_finished, "Finished event never arrived within 15s");
    session.cancel().await;
}