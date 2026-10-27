use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use upaa_core::{
    capture::Conditions,
    engine::{Engine, Request},
    extractor, parser,
    queries::transport::run_session_summary,
    storage,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("upaa-core-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn conditions() -> Conditions {
    Conditions {
        device: "fixture".into(),
        platform: "Windows".into(),
        scenario: "fixture".into(),
        operation: "same".into(),
        build: "Development".into(),
        quality: "high".into(),
        resolution: "1080p".into(),
        profiling: "normal".into(),
        code_version: "test".into(),
    }
}
async fn load(e: &mut Engine, t: &Temp, id: &str) -> PathBuf {
    let path = t.0.join(format!("{id}.json"));
    std::fs::write(&path, include_bytes!("fixtures/editor-dump.json")).unwrap();
    let hash = storage::file_hash(&path, &AtomicBool::new(false)).unwrap();
    e.handle(Request::Open {
        id: id.into(),
        path: path.clone(),
        expected_hash: hash,
        conditions: conditions(),
    })
    .await
    .unwrap();
    path
}
#[tokio::test]
async fn snapshot_matches_desktop_shared_core() {
    let t = Temp::new();
    let mut e = Engine::new(Arc::new(AtomicBool::new(false)));
    let path = load(&mut e, &t, "a").await;
    let profile = parser::parse_file(&path).await.unwrap();
    let store = upaa_core::queries::MetricsStore::new();
    store
        .set_capture(extractor::extract(&profile), profile.details)
        .await;
    let got = e
        .handle(Request::Query {
            id: "a".into(),
            tool: "performance_session_summary".into(),
            arguments: json!({}),
            editor_evidence: None,
        })
        .await
        .unwrap();
    assert_eq!(got, run_session_summary(&store).await.unwrap());
}
#[tokio::test]
async fn changed_recording_rejected() {
    let t = Temp::new();
    let mut e = Engine::new(Arc::new(AtomicBool::new(false)));
    let path = load(&mut e, &t, "a").await;
    std::fs::write(path, "{}").unwrap();
    assert!(e
        .handle(Request::Query {
            id: "a".into(),
            tool: "performance_session_summary".into(),
            arguments: json!({}),
            editor_evidence: None
        })
        .await
        .is_err());
}
#[tokio::test]
async fn cannot_open_placeholder_or_edit() {
    let t = Temp::new();
    let flag = Arc::new(AtomicBool::new(false));
    let mut e = Engine::new(flag.clone());
    let path = t.0.join("f.raw");
    std::fs::write(&path, b"fixture").unwrap();
    assert!(e
        .handle(Request::Open {
            id: "raw".into(),
            path,
            expected_hash: "".into(),
            conditions: conditions()
        })
        .await
        .is_err());
    load(&mut e, &t, "a").await;
    for name in ["optimization_replace", "project_propose_tasks", "shell"] {
        assert!(e
            .handle(Request::Query {
                id: "a".into(),
                tool: name.into(),
                arguments: json!({}),
                editor_evidence: None
            })
            .await
            .is_err());
    }
    flag.store(true, Ordering::SeqCst);
    assert!(e.handle(Request::Info).await.is_err());
}
#[tokio::test]
async fn comparison_conditions_and_missing_are_not_passes() {
    let t = Temp::new();
    let mut e = Engine::new(Arc::new(AtomicBool::new(false)));
    load(&mut e, &t, "a").await;
    load(&mut e, &t, "b").await;
    let v = e
        .handle(Request::Compare {
            a: "a".into(),
            b: "b".into(),
            range_a: None,
            range_b: None,
            confirmed: false,
            budgets: BTreeMap::from([("cpuMs".into(), 1.)]),
        })
        .await
        .unwrap();
    assert_eq!(v["metrics"][0]["verdict"], "不可判定");
    assert_eq!(v["statisticMethod"], "upaa-round-index-v1");
}
