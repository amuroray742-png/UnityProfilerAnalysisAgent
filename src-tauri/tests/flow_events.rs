use serde_json::{json, Value};
use unity_profiler_analysis_agent_lib::parser;

#[tokio::test]
async fn dump_flow_is_unavailable_and_window_arguments_are_checked() {
    let b = bytes::Bytes::from_static(include_bytes!("fixtures/editor-dump.json"));
    let p = parser::json::parse(&b, "fixture.json", b.len() as u64)
        .await
        .unwrap();
    let s = p.details.unwrap();
    let page = s.flows(10, 10, None, 0, 10).unwrap();
    assert_eq!(page["available"], false);
    assert_eq!(page["total"], 0);
    assert!(s.flows(10, 9, None, 0, 10).is_err());
    assert!(s.flows(10, 18, None, 0, 10).is_err());
    assert!(s.flows(10, 10, None, 0, 51).is_err());
    assert!(s.flows(10, 10, None, 1, 10).is_err());
    let mut value: Value = serde_json::from_slice(&b).unwrap();
    value["frames"][0]["threads"] = json!([]);
    value["frames"][0]["sample_count_total"] = json!(0);
    value["frames"][0]["gc_alloc_bytes_total"] = json!(0);
    let b = bytes::Bytes::from(serde_json::to_vec(&value).unwrap());
    let p = parser::json::parse(&b, "empty.json", b.len() as u64)
        .await
        .unwrap();
    assert_eq!(
        p.details.unwrap().flows(10, 10, None, 0, 10).unwrap()["available"],
        false
    );
}

#[tokio::test]
#[ignore = "requires UNITY_FLOW_DATA_PATH and UNITY_FLOW_REFERENCE_PATH; explicit missing input fails"]
async fn editor_flow_matches_all_decoded_records_and_production_queries() {
    use std::io::Read;
    let path = std::env::var("UNITY_FLOW_DATA_PATH").expect("set UNITY_FLOW_DATA_PATH");
    let truth: Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("UNITY_FLOW_REFERENCE_PATH").expect("set UNITY_FLOW_REFERENCE_PATH"),
        )
        .unwrap(),
    )
    .unwrap();
    let timer = std::time::Instant::now();
    let mut f = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let mut decoder = parser::data::unity6_structured::Decoder::default();
    let mut count = 0usize;
    let frames = truth["frames"].as_array().unwrap();
    assert!(!frames.is_empty());
    for frame in frames {
        let mut h = [0; 28];
        f.read_exact(&mut h).unwrap();
        let h = parser::data::header::read_block_header(&h).unwrap();
        h.validate().unwrap();
        assert_eq!(h.unity_version_string(), truth["editorVersion"]);
        let mut b = vec![0; h.body_size as usize];
        f.read_exact(&mut b).unwrap();
        let d = decoder.decode(&b).unwrap();
        let expected = frame["threads"].as_array().unwrap();
        assert_eq!(
            d.threads
                .iter()
                .filter(|t| !t.flow_events.is_empty())
                .count(),
            expected.len()
        );
        for (ti, t) in d.threads.iter().enumerate() {
            let actual:Vec<_>=t.flow_events.iter().map(|e|json!({"ParentSampleIndex":e.sample_index,"FlowId":e.flow_id,"FlowEventType":e.event_type})).collect();
            let e = expected.iter().find(|t| t["threadIndex"] == ti);
            assert_eq!(
                json!(actual),
                e.map(|t| t["events"].clone()).unwrap_or(json!([])),
                "frame {} thread {ti}",
                frame["frameIndex"]
            );
            count += actual.len();
        }
    }
    let mut end = [0; 4];
    f.read_exact(&mut end).unwrap();
    assert_eq!(u32::from_le_bytes(end), 0xDEADFEED);
    assert_eq!(f.read(&mut [0]).unwrap(), 0);
    assert_eq!(count as u64, truth["total"].as_u64().unwrap());
    let p = parser::parse_file(std::path::Path::new(&path))
        .await
        .unwrap();
    assert_eq!(p.frames.len(), frames.len());
    let s = p.details.unwrap();
    for fi in [0, 1, 127, 511, 999, 1500, 1999]
        .into_iter()
        .filter(|i| *i < frames.len())
    {
        let mut rows = Vec::new();
        let mut start = 0;
        loop {
            let page = s.flows(fi, fi, None, start, 50).unwrap();
            assert_eq!(page["available"], true);
            assert!(serde_json::to_vec_pretty(&page).unwrap().len() < 24 * 1024);
            rows.extend(page["rows"].as_array().unwrap().clone());
            if let Some(next) = page["nextStart"].as_u64() {
                start = next as usize;
            } else {
                break;
            }
        }
        let expected: Vec<_> = frames[fi]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| {
                t["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(move |e| (t["threadIndex"].clone(), e))
            })
            .collect();
        assert_eq!(rows.len(), expected.len());
        for (r, (ti, e)) in rows.iter().zip(expected) {
            assert_eq!(r["threadIndex"], ti);
            assert_eq!(r["sampleIndex"], e["ParentSampleIndex"]);
            assert_eq!(r["flowId"], e["FlowId"]);
            assert_eq!(r["eventType"], e["FlowEventType"]);
        }
    }
    println!(
        "Flow: frames={} events={count} bytes={} elapsed={:?}",
        frames.len(),
        std::fs::metadata(path).unwrap().len(),
        timer.elapsed()
    );
}
