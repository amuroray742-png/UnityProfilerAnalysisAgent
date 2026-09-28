use bytes::Bytes;
use serde_json::{json, Value};
use unity_profiler_analysis_agent_lib::{extractor, mcp, parser, state};
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/editor-dump.json")).unwrap()
}
async fn parse(v: Value) -> parser::ParsedProfile {
    let b = Bytes::from(serde_json::to_vec(&v).unwrap());
    parser::json::parse(&b, "fixture.json", b.len() as u64)
        .await
        .unwrap()
}
#[tokio::test]
async fn real_frame_tree_pagination_and_identifiers() {
    let mut v = fixture();
    v["frames"][0]["threads"][0]["thread_id"] = json!(u64::MAX);
    v["frames"][0]["threads"][0]["thread_index"] = json!(17);
    let p = parse(v).await;
    let store = p.details.unwrap();
    let f = store.frame(10, 0, 1).unwrap();
    assert_eq!(f.info.frame_index, 10);
    assert_eq!(f.threads[0].thread_id, u64::MAX.to_string());
    assert_eq!(f.next_start, Some(1));
    assert_eq!(store.frame(10, 1, 1).unwrap().threads[0].name, "Worker");
    let first = store.hierarchy(10, None, 0, 2, 64).unwrap();
    assert_eq!(first.thread.thread_index, 17);
    assert_eq!(first.samples[1].parent_index, Some(0));
    assert_eq!(first.next_start, Some(2));
    let second = store.hierarchy(10, Some(17), 2, 2, 64).unwrap();
    assert_eq!(second.samples[0].parent_index, Some(1));
    assert_eq!(second.samples[1].depth, 3);
    assert_eq!(second.samples[0].gc_alloc_bytes, Some(20));
    assert_eq!(second.samples[1].gc_alloc_bytes, Some(4));
    assert_eq!(second.next_start, Some(4));
    let shallow = store.hierarchy(10, None, 0, 500, 1).unwrap();
    assert_eq!(shallow.samples.len(), 1);
    assert!(shallow.depth_truncated);
    assert_eq!(shallow.next_start, None);
    let other = store.hierarchy(12, None, 0, 500, 64).unwrap();
    assert_eq!(other.samples.len(), 1);
    assert_eq!(other.info.gc_alloc_bytes, Some(0));
    for (limit, depth) in [(0, 1), (501, 1), (1, 0), (1, 65), (usize::MAX, 1)] {
        assert!(store.hierarchy(10, None, 0, limit, depth).is_err());
    }
    assert!(store.hierarchy(10, None, 6, 10, 8).is_err());
    assert!(store.hierarchy(10, Some(0), 0, 10, 8).is_err());
    assert!(store.frame(0, 0, 10).is_err());
    assert!(store.frame(10, 3, 10).is_err());
    assert!(store.frame(10, 0, 129).is_err());
}
#[tokio::test]
async fn invalid_gc_does_not_leak_through_details() {
    let mut v = fixture();
    v["frames"][0]["gc_alloc_bytes_total"] = json!(999);
    let p = parse(v).await;
    let page = p.details.unwrap().hierarchy(10, None, 0, 500, 64).unwrap();
    assert_eq!(page.info.gc_alloc_bytes, None);
    assert!(page.samples.iter().all(|s| s.gc_alloc_bytes.is_none()));
    assert!(!page.info.warnings.is_empty());
}
#[tokio::test]
async fn stores_release_and_never_reuse_other_capture_details() {
    let p = parse(fixture()).await;
    let snapshot = extractor::extract(&p);
    let details = p.details.clone().unwrap();
    let weak = std::sync::Arc::downgrade(&details);
    let mcp = mcp::MetricsStore::new();
    mcp.set_capture(snapshot.clone(), Some(details.clone()))
        .await;
    let result = mcp::transport::run_cpu_hierarchy(&mcp, 12, 8)
        .await
        .unwrap();
    assert_eq!(result["info"]["frameIndex"], 12);
    assert_eq!(result["samples"].as_array().unwrap().len(), 1);
    mcp.set(snapshot.clone()).await;
    assert!(mcp::transport::run_cpu_hierarchy(&mcp, 12, 8)
        .await
        .is_err());
    let app = state::AppState::new();
    app.put_upload(state::UploadEntry {
        file_id: "a".into(),
        file_path: "fixture".into(),
        file_name: "fixture".into(),
        size_bytes: 0,
        extension: "json".into(),
    })
    .await;
    assert!(
        app.put_analysis("a".into(), snapshot.clone(), Some(details.clone()))
            .await
    );
    app.release_file("a").await;
    assert!(app.get_details("a").await.is_none());
    assert!(app.get_snapshot("a").await.is_none());
    assert!(
        !app.put_analysis("a".into(), snapshot, Some(details.clone()))
            .await
    );
    drop(details);
    drop(p);
    assert!(weak.upgrade().is_none());
}
