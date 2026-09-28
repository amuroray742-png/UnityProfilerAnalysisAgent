use bytes::Bytes;
use serde_json::{json, Value};
use unity_profiler_analysis_agent_lib::{extractor, parser};
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/editor-dump.json")).unwrap()
}
async fn parse(v: Value) -> Result<parser::ParsedProfile, parser::ParseError> {
    let bytes = Bytes::from(serde_json::to_vec(&v).unwrap());
    parser::json::parse(&bytes, "fixture.json", bytes.len() as u64).await
}
#[tokio::test]
async fn dump_import_preserves_scope_units_and_zero() {
    let p = parse(fixture()).await.unwrap();
    assert_eq!(p.meta.frame_count, 20);
    assert_eq!(p.frames.len(), 2);
    assert_eq!(p.frames[0].index, 10);
    assert_eq!(p.frames[0].cpu_ms, 12.0);
    assert_eq!(p.frames[0].gc_alloc_bytes, 32);
    let s = extractor::extract(&p);
    assert_eq!(s.meta.frame_count, 2);
    assert_eq!(s.meta.declared_frame_count, 20);
    assert_eq!(s.gc.total_alloc_bytes, Some(32));
    assert_eq!(s.gc.alloc_per_frame_bytes.quality.status, "available");
    assert_eq!(s.gc.gen_collections.gen0, None);
    assert_eq!(s.rendering.draw_calls.p95, None);
    let update =
        s.gc.top_alloc_sites
            .iter()
            .find(|s| s.name == "Update")
            .unwrap();
    assert_eq!(update.total_bytes, 24);
    assert_eq!(update.call_count, 2);
    assert_eq!(s.cpu.frame_timeline[1].frame_index, 12);
    assert_eq!(s.cpu.frame_timeline[0].gc_alloc_bytes, Some(32));
    assert_eq!(s.cpu.frame_timeline[1].gc_alloc_bytes, Some(0));
    assert_eq!(s.cpu.frame_timeline[0].frame_time_ms, Some(16.0));
}
#[tokio::test]
async fn compact_summaries_preserve_all_calls_and_raw_tree() {
    let p = parse(fixture()).await.unwrap();
    let frame = &p.frames[0];
    assert_eq!(frame.main_thread_samples.len(), 4);
    let alloc = frame
        .main_thread_samples
        .iter()
        .find(|s| s.name == "GC.Alloc")
        .unwrap();
    assert_eq!(
        (alloc.total_ms, alloc.call_count, alloc.max_ms),
        (2.0, 2, 1.0)
    );
    assert_eq!(frame.gc_alloc_sites.len(), 2);
    let update = frame
        .gc_alloc_sites
        .iter()
        .find(|s| s.name == "Update")
        .unwrap();
    assert_eq!(
        (update.total_bytes, update.call_count, update.max_bytes),
        (24, 2, 20)
    );
    let tree = p
        .details
        .as_ref()
        .unwrap()
        .hierarchy(10, None, 0, 500, 64)
        .unwrap();
    assert_eq!(tree.samples.len(), 5);
    assert_eq!(tree.samples[2].marker_id, 126);
    assert_eq!(tree.samples[3].marker_id, 139);
    assert_eq!(tree.samples[3].parent_index, Some(2));
    assert_eq!(tree.samples[2].gc_alloc_bytes, Some(20));
    assert_eq!(tree.samples[3].gc_alloc_bytes, Some(4));

    // Reconstruct ungrouped rows independently from the public fixture and
    // compare the complete outward metrics, not only their grand totals.
    let mut ungrouped = p.clone();
    ungrouped.frames[0].main_thread_samples = tree
        .samples
        .iter()
        .map(|s| parser::Sample {
            name: s.name.clone(),
            total_ms: s.total_ms,
            max_ms: s.total_ms,
            call_count: 1,
        })
        .collect();
    ungrouped.frames[0].gc_alloc_sites = vec![
        parser::AllocSite {
            name: "Update".into(),
            thread: update.thread.clone(),
            total_bytes: 20,
            max_bytes: 20,
            call_count: 1,
        },
        parser::AllocSite {
            name: "Update".into(),
            thread: update.thread.clone(),
            total_bytes: 4,
            max_bytes: 4,
            call_count: 1,
        },
        frame
            .gc_alloc_sites
            .iter()
            .find(|s| s.name == "Worker")
            .unwrap()
            .clone(),
    ];
    assert_eq!(
        serde_json::to_value(extractor::extract(&p)).unwrap(),
        serde_json::to_value(extractor::extract(&ungrouped)).unwrap()
    );
}

#[tokio::test]
async fn compaction_keeps_threads_separate_and_rejects_overflow() {
    let p = parse(fixture()).await.unwrap();
    let mut f = p.frames[0].clone();
    f.gc_alloc_sites[1].name = f.gc_alloc_sites[0].name.clone();
    parser::compact::compact(&mut f).unwrap();
    assert_eq!(f.gc_alloc_sites.len(), 2);
    for field in 0..3 {
        let mut f = p.frames[0].clone();
        match field {
            0 => {
                f.gc_alloc_sites[0].total_bytes = u64::MAX;
                f.gc_alloc_sites.push(f.gc_alloc_sites[0].clone());
            }
            1 => {
                f.main_thread_samples[0].call_count = u64::MAX;
                f.main_thread_samples.push(f.main_thread_samples[0].clone());
            }
            _ => {
                f.main_thread_samples[0].total_ms = f64::MAX;
                f.main_thread_samples.push(f.main_thread_samples[0].clone());
            }
        }
        assert!(parser::compact::compact(&mut f)
            .unwrap_err()
            .to_string()
            .contains("frame[10]"));
    }
}
#[tokio::test]
async fn gc_missing_or_inconsistent_is_partial_not_zero() {
    for change in 0..3 {
        let mut v = fixture();
        if change == 0 {
            v["frames"][0]["threads"][0]["samples"][2]["metadata_count"] = json!(0);
        }
        if change == 1 {
            v["frames"][0]["gc_alloc_bytes_total"] = json!(999);
        }
        if change == 2 {
            v["frames"][0]["threads"][1]["gc_alloc_total_bytes"] = json!(999);
        }
        let p = parse(v).await.unwrap();
        let s = extractor::extract(&p);
        assert!(!p.frames[0].quality.gc);
        assert_eq!(s.cpu.frame_timeline[0].gc_alloc_bytes, None);
        assert_eq!(s.cpu.frame_timeline[1].gc_alloc_bytes, Some(0));
        assert!(p.frames[0].gc_alloc_sites.is_empty());
        assert_eq!(s.gc.total_alloc_bytes, Some(0)); // second frame is a real zero
        assert_eq!(s.gc.alloc_per_frame_bytes.quality.status, "partial");
        assert_eq!(s.gc.alloc_per_frame_bytes.quality.valid_frames, 1);
        assert!(!s.gc.alloc_per_frame_bytes.quality.can_diagnose());
        assert_eq!(s.cpu.main_thread_ms.quality.status, "available");
    }
}
#[tokio::test]
async fn malformed_dump_does_not_fall_back() {
    for change in 0..7 {
        let mut v = fixture();
        match change {
            0 => v["frames"][1]["frame_index"] = json!(10),
            1 => v["frames"][0]["threads"][1]["thread_index"] = json!(0),
            2 => v["frames"][0]["threads"][0]["samples"][2]["sample_index"] = json!(9),
            3 => v["frames"][0]["threads"][0]["samples"][0]["children_count"] = json!(100),
            4 => v["frames"][0]["threads"][0]["samples"][0]["time_ms"] = json!(-1),
            5 => v["frames"][0]["sample_count_total"] = json!(1),
            _ => v["frames"][0]["threads"] = json!("broken"),
        }
        assert!(parse(v).await.is_err(), "case {change}");
    }
    assert!(parse(json!({"input_file":"bad","frames":[]}))
        .await
        .is_err());
}
#[tokio::test]
async fn missing_main_and_changed_marker_ids() {
    let mut v = fixture();
    v["frames"][0]["threads"][0]["thread_name"] = json!("Other");
    v["frames"][1]["threads"][0]["samples"][0]["marker_id"] = json!(999999);
    let s = extractor::extract(&parse(v).await.unwrap());
    assert_eq!(s.cpu.main_thread_ms.quality.status, "partial");
    assert_eq!(s.gc.total_alloc_bytes, Some(32));
    assert_eq!(s.cpu.frame_timeline[0].ms, None);
}
#[tokio::test]
async fn legacy_json_dispatch_and_absent_values() {
    let v2 = parse(json!({"header":{},"samples":[{"name":"A","totalMs":5}]}))
        .await
        .unwrap();
    let s = extractor::extract(&v2);
    assert_eq!(s.cpu.main_thread_ms.p95, Some(5.0));
    assert_eq!(s.cpu.main_thread_ms.quality.status, "estimated");
    assert_eq!(s.gc.total_alloc_bytes, None);
    for v in [
        json!({"frames":[{"cpuMs":0,"gcAllocBytes":0}]}),
        json!([{"cpuMs":0,"gcAllocBytes":0}]),
    ] {
        let s = extractor::extract(&parse(v).await.unwrap());
        assert_eq!(s.cpu.main_thread_ms.p50, Some(0.0));
        assert_eq!(s.gc.total_alloc_bytes, Some(0));
        assert_eq!(s.rendering.draw_calls.p95, None);
    }
    assert!(parse(json!({"unrelated":true})).await.is_err());
}
#[tokio::test]
#[ignore = "requires UNITY_PROFILER_DUMP_PATH pointing to the 64-frame reference dump"]
async fn real_dump_production_path() {
    let path = std::env::var("UNITY_PROFILER_DUMP_PATH")
        .expect("set UNITY_PROFILER_DUMP_PATH; missing input is a failure");
    let started = std::time::Instant::now();
    let p = parser::parse_file(std::path::Path::new(&path))
        .await
        .expect("production import");
    assert_eq!(p.frames.len(), 64);
    assert_eq!(p.meta.frame_count, 2000);
    assert_eq!(
        p.frames[0]
            .main_thread_samples
            .iter()
            .map(|s| s.call_count)
            .sum::<u64>(),
        2076
    );
    let tree = p.details.as_ref().unwrap().load(p.frames[0].index).unwrap();
    assert_eq!(
        tree.threads
            .iter()
            .find(|t| t.info.name == "Main Thread")
            .unwrap()
            .samples
            .len(),
        2076
    );
    assert_eq!(p.frames[0].gc_alloc_bytes, 136);
    assert!((p.frames[0].cpu_ms - 49.33438491821289).abs() < 0.000001);
    // Independent typed expected totals; unknown fields are discarded.
    #[derive(serde::Deserialize)]
    struct Expected {
        frames: Vec<ExpectedFrame>,
    }
    #[derive(serde::Deserialize)]
    struct ExpectedFrame {
        frame_index: usize,
        gc_alloc_bytes_total: u64,
        frame_time_ms: f64,
        threads: Vec<ExpectedThread>,
    }
    #[derive(serde::Deserialize)]
    struct ExpectedThread {
        thread_name: String,
        samples: Vec<ExpectedSample>,
    }
    #[derive(serde::Deserialize)]
    struct ExpectedSample {
        time_ms: f64,
    }
    let expected: Expected =
        serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
            .unwrap();
    for (a, e) in p.frames.iter().zip(&expected.frames) {
        assert!(a.quality.gc, "frame {}: {:?}", a.index, a.quality.reasons);
        assert!(a.quality.cpu, "frame {}: {:?}", a.index, a.quality.reasons);
        assert_eq!(a.index, e.frame_index);
        assert_eq!(a.gc_alloc_bytes, e.gc_alloc_bytes_total);
        let main = e
            .threads
            .iter()
            .find(|t| t.thread_name == "Main Thread")
            .unwrap();
        assert_eq!(
            a.main_thread_samples
                .iter()
                .map(|s| s.call_count)
                .sum::<u64>(),
            main.samples.len() as u64
        );
        let tree = p.details.as_ref().unwrap().load(a.index).unwrap();
        assert_eq!(
            tree.threads
                .iter()
                .find(|t| t.info.name == "Main Thread")
                .unwrap()
                .samples
                .len(),
            main.samples.len()
        );
        assert!((a.cpu_ms - main.samples[0].time_ms).abs() < 0.000001);
        assert!((a.duration_ms - e.frame_time_ms).abs() < 0.000001);
        assert_eq!(
            a.gc_alloc_sites.iter().map(|s| s.total_bytes).sum::<u64>(),
            a.gc_alloc_bytes
        );
    }
    let s = extractor::extract(&p);
    assert_eq!(
        s.gc.total_alloc_bytes,
        Some(expected.frames.iter().map(|f| f.gc_alloc_bytes_total).sum())
    );
    assert_eq!(s.gc.alloc_per_frame_bytes.quality.valid_frames, 64);
    for (actual, reference) in s.cpu.frame_timeline.iter().zip(&expected.frames) {
        assert_eq!(actual.gc_alloc_bytes, Some(reference.gc_alloc_bytes_total));
    }
    println!(
        "dump bytes={}, imported={}, declared={}, elapsed={:?}",
        p.meta.file_size_bytes,
        p.frames.len(),
        p.meta.frame_count,
        started.elapsed()
    );
}

#[tokio::test]
async fn missing_all_gc_serializes_null_and_analysis_abstains() {
    let mut v = fixture();
    for f in v["frames"].as_array_mut().unwrap() {
        f["gc_alloc_bytes_total"] = json!(999);
    }
    let s = extractor::extract(&parse(v).await.unwrap());
    let wire = serde_json::to_value(&s).unwrap();
    assert!(wire["gc"]["totalAllocBytes"].is_null());
    assert!(wire["gc"]["allocPerFrameBytes"]["p95"].is_null());
    let store = unity_profiler_analysis_agent_lib::mcp::MetricsStore::new();
    store.set(s).await;
    let analysis = unity_profiler_analysis_agent_lib::mcp::transport::run_analysis(&store, "gc")
        .await
        .unwrap();
    assert_eq!(analysis["issues"], json!([]));
    assert!(
        unity_profiler_analysis_agent_lib::mcp::transport::run_frames(&store, 999, 1)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn roots_metadata_negative_and_missing_fields() {
    let mut v = fixture();
    v["frames"][0]["threads"][0]["samples"][2]["gc_alloc_bytes"] = json!(-1);
    let error = parse(v).await.unwrap_err().to_string();
    assert!(
        error.contains("frames[0]") && error.contains("threads[0]") && error.contains("samples[2]"),
        "{error}"
    );
    let mut v = fixture();
    v["frames"][0]["threads"] = Value::Null;
    assert!(parse(v).await.is_err());
    let mut v = fixture();
    v["frames"][0]["threads"][0]["samples"][2]
        .as_object_mut()
        .unwrap()
        .remove("gc_alloc_bytes");
    assert!(!parse(v).await.unwrap().frames[0].quality.gc);
    let mut v = fixture();
    v["frames"][0]["threads"][0]["samples"][4]["children_count"] = json!(1);
    assert!(parse(v).await.is_err());
    let mut v = fixture();
    v["frames"][0]["threads"][1]["thread_name"] = json!("Main Thread");
    assert!(!parse(v).await.unwrap().frames[0].quality.cpu);
}
#[tokio::test]
async fn percentile_and_hotspot_aggregation_respect_valid_frames() {
    let p=parse(json!({"frames":[
        {"cpuMs":0,"gcAllocBytes":0,"mainThreadSamples":[{"name":"Update","totalMs":2,"callCount":2,"maxMs":1}]},
        {"cpuMs":10,"gcAllocBytes":1024,"mainThreadSamples":[{"name":"Update","totalMs":3,"callCount":1,"maxMs":3}]},
        {"gcAllocBytes":2048}
    ]})).await.unwrap();
    let s = extractor::extract(&p);
    assert_eq!(s.cpu.main_thread_ms.p95, Some(10.0));
    assert_eq!(s.cpu.main_thread_ms.quality.valid_frames, 2);
    assert_eq!(s.cpu.top_hotspots[0].total_ms, 5.0);
    assert_eq!(s.cpu.top_hotspots[0].call_count, 3);
    assert_eq!(s.cpu.top_hotspots[0].max_ms, 3.0);
    assert_eq!(s.gc.total_alloc_bytes, Some(3072));
    assert_eq!(s.gc.alloc_per_frame_bytes.p50, Some(1024.0));
    assert_eq!(s.rendering.batches_saved_by_srp_batcher, None);
}

#[tokio::test]
async fn experimental_binary_outputs_never_become_real_zero_metrics() {
    let mut bytes = Vec::new();
    use parser::data::constants;
    for n in [constants::UNITY_DATA_MAGIC, 32, 6000, 4, 0, 2, 1] {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&306i32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&16_000i32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&constants::FRAME_END_MARKER.to_le_bytes());
    bytes.extend_from_slice(&constants::FILE_END_MARKER.to_le_bytes());
    let profile = parser::data::parse(&Bytes::from(bytes), "synthetic.data", 64)
        .await
        .unwrap();
    let snapshot = extractor::extract(&profile);
    assert_eq!(snapshot.cpu.main_thread_ms.p95, Some(16.0));
    assert_eq!(snapshot.cpu.main_thread_ms.quality.status, "estimated");
    assert_eq!(snapshot.gc.total_alloc_bytes, None);
    assert_eq!(snapshot.gc.site_quality.status, "unavailable");
    assert_eq!(snapshot.cpu.hotspot_quality.status, "unavailable");
    assert_eq!(snapshot.rendering.draw_calls.p95, None);
    let raw = parser::raw::parse(&Bytes::from_static(b"UNITY\x01\x00\x00\x00"), "test.raw", 9)
        .await
        .unwrap();
    assert_eq!(extractor::extract(&raw).gc.total_alloc_bytes, None);
}

#[tokio::test]
async fn analysis_traces_isolated_peaks_and_preserves_missing_data_boundary() {
    use unity_profiler_analysis_agent_lib::mcp::{MetricsStore, transport::run_analysis};
    let mut frames: Vec<Value> = (0..21).map(|i| json!({"frameIndex":100+i*3,"cpuMs":1.0,"gcAllocBytes":0})).collect();
    frames[20]["cpuMs"] = json!(40.0);
    frames[20]["gcAllocBytes"] = json!(8388608);
    let store = MetricsStore::new();
    store.set(extractor::extract(&parse(json!({"frames":frames})).await.unwrap())).await;
    let analysis = run_analysis(&store, "all").await.unwrap();
    assert_eq!(analysis["thresholdPolicy"]["userConfigured"], false);
    let issues = analysis["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2);
    for issue in issues {
        assert_eq!(issue["trigger"], "isolated-peak");
        assert_eq!(issue["affectedFrames"], 1);
        assert_eq!(issue["validFrames"], 21);
        assert_eq!(issue["evidenceFrames"][0]["frameIndex"], 160);
    }
    assert_eq!(issues[0]["unit"], "ms");
    assert_eq!(issues[0]["p95"], 1.0);
    assert_eq!(issues[0]["evidenceFrames"][0]["value"], 40.0);
    assert_eq!(issues[1]["unit"], "bytes");
    frames[0].as_object_mut().unwrap().remove("gcAllocBytes");
    store.set(extractor::extract(&parse(json!({"frames":frames})).await.unwrap())).await;
    let partial = run_analysis(&store, "gc").await.unwrap();
    assert_eq!(partial["quality"]["gc"]["status"], "partial");
    assert_eq!(partial["issues"], json!([]));
}

#[tokio::test]
async fn analysis_evidence_is_bounded_sorted_and_focus_specific() {
    use unity_profiler_analysis_agent_lib::mcp::{MetricsStore, transport::run_analysis};
    let frames: Vec<Value> = (0..9).map(|i| json!({"frameIndex":200+i*5,"cpuMs":20+i,"gcAllocBytes":0})).collect();
    let store = MetricsStore::new();
    store.set(extractor::extract(&parse(json!({"frames":frames})).await.unwrap())).await;
    let analysis = run_analysis(&store, "cpu").await.unwrap();
    let issues = analysis["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["affectedFrames"], 9);
    assert_eq!(issues[0]["trigger"], "p95");
    assert_eq!(issues[0]["evidenceFrames"].as_array().unwrap().len(), 5);
    assert_eq!(issues[0]["evidenceFrames"][0]["frameIndex"], 240);
    assert_eq!(run_analysis(&store, "gc").await.unwrap()["issues"], json!([]));
}

#[tokio::test]
async fn public_isolated_peak_fixture_reconciles_metrics_and_original_threads() {
    let p = parse(serde_json::from_str(include_str!("fixtures/isolated-peak.json")).unwrap()).await.unwrap();
    let s = extractor::extract(&p);
    assert_eq!(p.frames.len(), 21);
    assert_eq!(s.cpu.main_thread_ms.p95, Some(1.0));
    assert_eq!(s.cpu.main_thread_ms.max, Some(40.0));
    assert_eq!(s.gc.alloc_per_frame_bytes.p95, Some(0.0));
    assert_eq!(s.gc.total_alloc_bytes, Some(8388608));
    let peak = p.details.as_ref().unwrap().load(160).unwrap();
    assert_eq!(peak.threads.iter().flat_map(|t| &t.samples).filter_map(|s| s.gc_alloc_bytes).sum::<u64>(), 8388608);
}
