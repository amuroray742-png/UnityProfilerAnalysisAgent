use bytes::Bytes;
use serde_json::{json, Value};
use unity_profiler_analysis_agent_lib::{extractor, mcp, parser};

async fn fixture() -> parser::ParsedProfile {
    let b = Bytes::from_static(include_bytes!("fixtures/editor-dump.json"));
    parser::json::parse(&b, "fixture.json", b.len() as u64)
        .await
        .unwrap()
}

#[tokio::test]
async fn identical_marker_names_keep_distinct_parent_paths_and_missing_gc_is_not_zero() {
    fn sample(i: usize, parent_name: &str, time: f64, start: f64, children: usize) -> Value {
        json!({"sample_index":i,"marker_id":i,"marker_name":parent_name,"time_ms":time,
            "start_time_ms":start,"children_count":children,"metadata_count":0})
    }
    let frames:Vec<_>=[(10,8.,2.,3.),(12,2.,1.,1.)].into_iter().map(|(index,parent,a,b)| json!({
        "frame_index":index,"frame_time_ms":20,"sample_count_total":5,"gc_alloc_bytes_total":0,
        "threads":[{"thread_index":0,"thread_id":42,"thread_name":"Main Thread","gc_alloc_total_bytes":0,
            "samples":[sample(0,"Main Thread",20.,0.,2),sample(1,"P",parent,0.,1),sample(2,"Same",a,0.,0),
                sample(3,"Q",parent,10.,1),sample(4,"Same",b,10.,0)]}]})).collect();
    let mut value = json!({"input_file":"public.data","unity_version":"6000.3.23f1","frame_count":2,"frames":frames});
    let parse = |v: &Value| Bytes::from(serde_json::to_vec(v).unwrap());
    let b = parse(&value);
    let p = parser::json::parse(&b, "paths.json", b.len() as u64)
        .await
        .unwrap();
    let result = p.details.unwrap().compare(10, 12, None, 0, 50).unwrap();
    let same: Vec<_> = result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["path"].as_array().unwrap().last() == Some(&json!("Same")))
        .collect();
    assert_eq!(same.len(), 2);
    assert_ne!(same[0]["path"], same[1]["path"]);
    assert_eq!(same[0]["inclusiveDeltaMs"], 2.);
    assert_eq!(same[1]["inclusiveDeltaMs"], 1.);
    value["frames"][0]["gc_alloc_bytes_total"] = Value::Null;
    let b = parse(&value);
    let p = parser::json::parse(&b, "paths.json", b.len() as u64)
        .await
        .unwrap();
    let result = p.details.unwrap().compare(10, 12, None, 0, 50).unwrap();
    assert!(result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["gcDeltaBytes"].is_null() && r["current"]["gcBytes"].is_null()));
}
#[tokio::test]
async fn self_uses_full_children_before_depth_filter_and_comparison_retains_paths() {
    let p = fixture().await;
    let store = p.details.as_ref().unwrap();
    let full = store.hierarchy(10, None, 0, 500, 64).unwrap();
    let shallow = store.hierarchy(10, None, 0, 500, 1).unwrap();
    assert_eq!(shallow.samples[0].self_ms, full.samples[0].self_ms);
    let zero = store.hierarchy(12, None, 0, 500, 64).unwrap();
    assert_eq!(zero.samples[0].self_ms, Some(zero.samples[0].total_ms));
    let comparison = store.compare(10, 12, None, 0, 50).unwrap();
    assert_eq!(comparison["baselineFrameIndex"], 12);
    assert!(comparison["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"].as_array().unwrap().len() > 2));
    assert!(store.compare(10, 10, None, 0, 10).is_err());
    assert!(store.compare(10, 12, Some(1), 0, 10).is_err()); // Worker absent in baseline.
    assert!(store.compare(10, 12, None, 0, 51).is_err());
    let first = store.compare(10, 12, None, 0, 1).unwrap();
    assert_eq!(first["nextStart"], 1);
    let evidence = store.evidence(10, 0, 50, false).unwrap();
    assert!(evidence["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["metadataTruncated"] == true)); // dump lacks general payload.
    let metrics = mcp::MetricsStore::new();
    metrics
        .set_capture(extractor::extract(&p), p.details.clone())
        .await;
    let wire = mcp::tools::dispatch(
        &metrics,
        "performance_compare_frames",
        json!({"frame_index":10,"baseline_frame_index":12}),
    )
    .await
    .unwrap();
    assert!(wire["rows"].is_array());
    metrics.clear().await;
    assert!(mcp::tools::dispatch(
        &metrics,
        "performance_frame_evidence",
        json!({"frame_index":10})
    )
    .await
    .is_err());
}

#[tokio::test]
#[ignore = "requires UNITY_EVIDENCE_DATA_PATH and UNITY_EVIDENCE_REFERENCE_PATH; explicit missing input fails"]
async fn editor_metadata_matches_production() {
    let path = std::env::var("UNITY_EVIDENCE_DATA_PATH").expect("set UNITY_EVIDENCE_DATA_PATH");
    let reference =
        std::env::var("UNITY_EVIDENCE_REFERENCE_PATH").expect("set UNITY_EVIDENCE_REFERENCE_PATH");
    let truth: Value = serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
    let timer = std::time::Instant::now();
    let p = parser::parse_file(std::path::Path::new(&path))
        .await
        .unwrap();
    let store = p.details.unwrap();
    let mut fields = 0;
    for f in truth["frames"].as_array().unwrap() {
        let index = f["frameIndex"].as_u64().unwrap() as usize;
        let mut start = 0;
        let mut rows = Vec::new();
        loop {
            let page = store.evidence(index, start, 50, false).unwrap();
            assert!(serde_json::to_vec_pretty(&page).unwrap().len() < 24 * 1024);
            rows.extend(page["rows"].as_array().unwrap().clone());
            if let Some(next) = page["nextStart"].as_u64() {
                start = next as usize;
            } else {
                break;
            }
        }
        let expected: usize = f["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["rows"].as_array().unwrap().len())
            .sum();
        assert_eq!(rows.len(), expected);
        for t in f["threads"].as_array().unwrap() {
            for r in t["rows"].as_array().unwrap() {
                let actual = rows
                    .iter()
                    .find(|a| {
                        a["threadIndex"] == t["threadIndex"] && a["sampleIndex"] == r["sampleIndex"]
                    })
                    .unwrap();
                assert_eq!(actual["marker"], r["name"]);
                assert_eq!(actual["isCounter"], r["counter"]);
                assert_eq!(
                    actual["metadataCount"].as_u64().unwrap() as usize,
                    r["fields"].as_array().unwrap().len()
                );
                for (a, e) in actual["metadata"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(r["fields"].as_array().unwrap())
                {
                    fields += 1;
                    let hex = e["rawHex"].as_str().unwrap();
                    assert_eq!(a["byteLength"].as_u64().unwrap() as usize, hex.len() / 2);
                    assert_eq!(
                        a["rawHex"].as_str().unwrap(),
                        &hex[..hex.len().min(128)],
                        "frame {index} marker {}",
                        r["name"]
                    );
                    if let Some(df) = e["definition"].as_object() {
                        let descriptor = a["definition"]["descriptor"].as_u64().unwrap();
                        assert_eq!(descriptor & 255, df["type"].as_u64().unwrap());
                        assert_eq!((descriptor >> 8) & 255, df["unit"].as_u64().unwrap());
                        assert_eq!(
                            a["definition"]["name"].as_str().unwrap(),
                            df["name"].as_str().unwrap_or("")
                        );
                    }
                    if let Some(n) = e["numeric"].as_str() {
                        let actual = a["value"]
                            .as_str()
                            .unwrap_or_else(|| panic!("numeric missing: {a}"));
                        let av = actual.parse::<f64>().unwrap();
                        let ev = n.parse::<f64>().unwrap();
                        assert!(
                            (av - ev).abs() <= 1e-6_f64.max(ev.abs() * 1e-6),
                            "frame {index} {}: {a} expected {n}",
                            r["name"]
                        );
                    }
                }
            }
        }
    }
    assert!(fields > 0);
    println!(
        "verified {fields} fields; {} frames; {:?}",
        truth["frames"].as_array().unwrap().len(),
        timer.elapsed()
    );
}
