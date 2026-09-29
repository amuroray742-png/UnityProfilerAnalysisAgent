//! Real MCP initialization and tools/call through the shipped stdio entry point.
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};
use unity_profiler_analysis_agent_lib::{
    extractor,
    mcp::{bridge::BridgeServer, server::ProfilerServer, MetricsStore},
    parser,
};

async fn server(first_index: usize) -> BridgeServer {
    let mut value: Value = serde_json::from_str(include_str!("fixtures/editor-dump.json")).unwrap();
    value["frames"][0]["frame_index"] = json!(first_index);
    let bytes = bytes::Bytes::from(serde_json::to_vec(&value).unwrap());
    let profile = parser::json::parse(&bytes, "fixture.json", bytes.len() as u64)
        .await
        .unwrap();
    let store = MetricsStore::new();
    store
        .set_capture(extractor::extract(&profile), profile.details)
        .await;
    BridgeServer::start(ProfilerServer { store, audit: None })
        .await
        .unwrap()
}
struct Client {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    id: u64,
}
impl Client {
    async fn connect(server: &BridgeServer, token: &str) -> Self {
        let mut child = Command::new(
            std::env::var_os("UPAA_TEST_APP_EXE")
                .unwrap_or_else(|| env!("CARGO_BIN_EXE_unity-profiler-analysis-agent").into()),
        )
        .args(["--mcp-bridge", &server.address().to_string()])
        .env("UPAA_MCP_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            input,
            output,
            id: 0,
        }
    }
    async fn send(&mut self, value: Value) {
        self.input
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
        self.input.flush().await.unwrap();
    }
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}))
            .await;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let line = self
                    .output
                    .next_line()
                    .await
                    .unwrap()
                    .expect("MCP stdout ended before response");
                let value: Value =
                    serde_json::from_str(&line).expect("stdout must contain only JSON-RPC");
                if value["id"] == self.id {
                    return value;
                }
            }
        })
        .await
        .expect("MCP response timeout")
    }
    async fn initialize(&mut self) {
        let init=self.request("initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"wire-test","version":"1"}})).await;
        assert!(
            init["result"]["capabilities"]["tools"].is_object(),
            "{init}"
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "unity-profiler");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
    }
    async fn call(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":args}))
            .await
    }
}
#[tokio::test]
async fn shipped_stdio_binary_serves_real_tree_and_validates_arguments() {
    let server = server(10).await;
    let mut client = Client::connect(&server, server.token()).await;
    client.initialize().await;
    let list = client.request("tools/list", json!({})).await;
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);
    let summary = client.call("performance_session_summary", json!({})).await;
    assert_eq!(
        summary["result"]["structuredContent"]["meta"]["frameCount"],
        2
    );
    assert!(summary["result"]["structuredContent"]["cpu"]
        .get("frameTimeline")
        .is_none());
    let semantics = client.call("performance_metric_semantics", json!({})).await;
    assert_eq!(semantics["result"]["structuredContent"], summary["result"]["structuredContent"]["metricSemantics"]);
    let hotspots = client.call("performance_hotspots", json!({"area":"gc","limit":1})).await;
    let rows = &hotspots["result"]["structuredContent"];
    assert_eq!(rows["rows"][0]["totalBytes"], 24);
    assert_eq!(rows["nextStart"], 1);
    let next = client.call("performance_hotspots", json!({"area":"gc","start":1})).await;
    assert_eq!(next["result"]["structuredContent"]["rows"][0]["totalBytes"], 8);
    assert!(client.call("performance_hotspots",json!({"area":"gc","limit":51})).await["error"].is_object());
    assert!(summary["result"]["content"][0]["text"].as_str().unwrap().contains('\n'));
    let frames = client.call("performance_frames", json!({"start":0,"limit":2})).await;
    let frames = &frames["result"]["structuredContent"]["frames"];
    assert_eq!(frames[0]["frameIndex"], 10);
    assert_eq!(frames[0]["gcAllocBytes"], 32);
    assert_eq!(frames[1]["frameIndex"], 12);
    assert_eq!(frames[1]["gcAllocBytes"], 0);
    // Zero is retained in the population: rounded-index p50 of [0,32] is 32,
    // not an average. The tool must explain this alongside the actual metric.
    let summary_data = &summary["result"]["structuredContent"];
    assert_eq!(summary_data["gc"]["allocPerFrameBytes"]["p50"], 32.0);
    assert_eq!(summary_data["gc"]["allocPerFrameBytes"]["quality"]["validFrames"], 2);
    assert_eq!(summary_data["metricSemantics"]["percentiles"]["smallSampleExample"]["p50"], 32);
    assert!(summary_data["metricSemantics"]["percentiles"]["frequency"].as_str().unwrap().contains("affectedFrames"));
    assert!(summary_data["metricSemantics"]["cpu"]["exclusiveTime"].as_str().unwrap().contains("未提供"));
    assert!(summary_data["metricSemantics"]["cpu"]["frameTimeDifference"].as_str().unwrap().contains("相减不能证明"));
    assert!(list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["annotations"]["readOnlyHint"] == true));
    // The default depth hides a nested 4 B allocation even without a next page.
    // The wire response must explicitly warn against treating visible bytes as totals.
    let shallow = client
        .call("performance_cpu_hierarchy", json!({"frame_index":10}))
        .await;
    let shallow = &shallow["result"]["structuredContent"];
    assert_eq!(shallow["depthTruncated"], true);
    assert!(shallow["nextStart"].is_null());
    assert!(!shallow["queryWarnings"].as_array().unwrap().is_empty());
    assert_eq!(
        shallow["samples"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["gcAllocBytes"].as_u64())
            .sum::<u64>(),
        20
    );
    let full = client
        .call(
            "performance_cpu_hierarchy",
            json!({"frame_index":10,"max_depth":64}),
        )
        .await;
    let full = &full["result"]["structuredContent"];
    assert_eq!(full["depthTruncated"], false);
    assert!(full["nextStart"].is_null());
    assert!(full["queryWarnings"].as_array().unwrap().is_empty());
    let main_gc = full["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["gcAllocBytes"].as_u64())
        .sum::<u64>();
    assert_eq!(main_gc, 24);
    let worker = client
        .call(
            "performance_cpu_hierarchy",
            json!({"frame_index":10,"thread_index":1,"max_depth":64}),
        )
        .await;
    let worker_gc = worker["result"]["structuredContent"]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["gcAllocBytes"].as_u64())
        .sum::<u64>();
    assert_eq!(worker_gc, 8);
    assert_eq!(
        main_gc + worker_gc,
        full["info"]["gcAllocBytes"].as_u64().unwrap()
    );
    let result = client
        .call(
            "performance_cpu_hierarchy",
            json!({"frame_index":10,"start":2,"limit":2,"max_depth":64}),
        )
        .await;
    let page = &result["result"]["structuredContent"];
    assert_eq!(page["info"]["frameIndex"], 10);
    assert_eq!(page["samples"][0]["parentIndex"], 1);
    assert_eq!(page["samples"][0]["gcAllocBytes"], 20);
    assert_eq!(page["samples"][1]["gcAllocBytes"], 4);
    assert_eq!(page["nextStart"], 4);
    assert!(!page["queryWarnings"].as_array().unwrap().is_empty());
    assert!(result["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("GC.Alloc"));
    for args in [
        json!({"start":0,"limit":0}),
        json!({"start":-1}),
        json!({"start":0,"unknown":true}),
        json!({"start":0,"limit":501}),
    ] {
        let error = client.call("performance_frames", args).await;
        assert_eq!(error["error"]["code"], -32602, "{error}");
    }
    assert!(client.call("nonexistent", json!({})).await["error"].is_object());
    let missing = client
        .call("performance_frame", json!({"frame_index":11}))
        .await;
    assert_eq!(missing["result"]["isError"], true);
    let zero = client
        .call("performance_frame", json!({"frame_index":12}))
        .await;
    assert_eq!(
        zero["result"]["structuredContent"]["info"]["gcAllocBytes"],
        0
    );
    server.shutdown().await;
    let status = tokio::time::timeout(Duration::from_secs(5), client.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "stdio proxy must exit on parent shutdown");
}
#[tokio::test]
async fn capabilities_isolate_sessions_and_shutdown_does_not_affect_other_session() {
    let a = server(10).await;
    let b = server(20).await;
    let mut wrong = Client::connect(&a, b.token()).await;
    let status = tokio::time::timeout(Duration::from_secs(5), wrong.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
    let mut client_a = Client::connect(&a, a.token()).await;
    let mut client_b = Client::connect(&b, b.token()).await;
    client_a.initialize().await;
    client_b.initialize().await;
    assert_eq!(
        client_a
            .call("performance_frame", json!({"frame_index":20}))
            .await["result"]["isError"],
        true
    );
    assert_eq!(
        client_b
            .call("performance_frame", json!({"frame_index":10}))
            .await["result"]["isError"],
        true
    );
    a.shutdown().await;
    let result = client_b
        .call("performance_frame", json!({"frame_index":20}))
        .await;
    assert_eq!(
        result["result"]["structuredContent"]["info"]["frameIndex"],
        20
    );
    b.shutdown().await;
    for client in [&mut client_a, &mut client_b] {
        assert!(
            tokio::time::timeout(Duration::from_secs(5), client.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }
}

#[tokio::test]
async fn oversized_request_is_disconnected_without_retaining_the_session() {
    use tokio::io::AsyncReadExt;
    let server = server(10).await;
    let mut socket = tokio::net::TcpStream::connect(server.address())
        .await
        .unwrap();
    socket.write_all(server.token().as_bytes()).await.unwrap();
    let mut ack = [0; 2];
    socket.read_exact(&mut ack).await.unwrap();
    assert_eq!(&ack, b"OK");
    socket.write_all(&vec![b'x'; 65537]).await.unwrap();
    let mut byte = [0];
    let outcome = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut byte))
        .await
        .expect("oversized request must close connection");
    assert!(matches!(outcome, Ok(0) | Err(_)));
    let mut valid = Client::connect(&server, server.token()).await;
    valid.initialize().await;
    assert!(valid.request("tools/list", json!({})).await["result"].is_object());
    server.shutdown().await;
    tokio::time::timeout(Duration::from_secs(5), valid.child.wait())
        .await
        .unwrap()
        .unwrap();
}
