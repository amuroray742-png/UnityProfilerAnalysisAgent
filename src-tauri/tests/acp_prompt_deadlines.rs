//! Exercise public Peer::run so regressions in the production timeout policy are caught.
use serde_json::{json, Value};
use tokio::{
    io::{duplex, split, AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, watch},
    time::{Duration, Instant},
};
use unity_profiler_analysis_agent_lib::acp_client::protocol::Peer;

#[tokio::test(start_paused = true)]
async fn production_prompt_accepts_activity_past_five_minutes_and_requires_end_turn() {
    let (client, server) = duplex(16384);
    let (read, write) = split(client);
    let (read_server, mut write_server) = split(server);
    let (_cancel, cancel_rx) = watch::channel(false);
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut peer = Peer::new(read, write, cancel_rx, tx);
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(read_server);
        for expected in ["initialize", "session/new", "session/prompt"] {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], expected);
            let result = match expected {
                "initialize" => json!({"protocolVersion":1}),
                "session/new" => json!({"sessionId":"production"}),
                _ => {
                    for kind in [
                        "agent_message_chunk",
                        "agent_thought_chunk",
                        "tool_call_update",
                    ] {
                        tokio::time::sleep(Duration::from_secs(240)).await;
                        let update = json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"production","update":{"sessionUpdate":kind,"content":{"type":"text","text":"progress"}}}});
                        write_server
                            .write_all(format!("{update}\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    tokio::time::sleep(Duration::from_secs(240)).await;
                    json!({"stopReason":"end_turn"})
                }
            };
            let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
            write_server
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let start = Instant::now();
    let reason = peer
        .run(std::path::Path::new("."), json!({}), "diagnose".into())
        .await
        .unwrap();
    assert_eq!(reason, "end_turn");
    assert_eq!(start.elapsed(), Duration::from_secs(960));
    server.await.unwrap();
}
