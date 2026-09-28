use serde_json::Value;
use unity_profiler_analysis_agent_lib::{extractor, parser};
#[tokio::test]
#[ignore = "requires UNITY_RENDER_DATA_PATH and UNITY_RENDER_REFERENCE_PATH; explicit missing input fails"]
async fn editor_render_counters_match_production_import() {
    let path = std::env::var("UNITY_RENDER_DATA_PATH").expect("set UNITY_RENDER_DATA_PATH");
    let reference =
        std::env::var("UNITY_RENDER_REFERENCE_PATH").expect("set UNITY_RENDER_REFERENCE_PATH");
    let expected: Value = serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
    let start = std::time::Instant::now();
    let profile = parser::parse_file(std::path::Path::new(&path))
        .await
        .unwrap();
    let frames = expected["frames"].as_array().unwrap();
    assert!(!frames.is_empty());
    assert_eq!(profile.frames.len(), frames.len());
    for (f, truth) in profile.frames.iter().zip(frames) {
        assert_eq!(f.index as u64, truth["frame_index"].as_u64().unwrap());
        for c in truth["counters"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let value = if c["available"] == true {
                c["value"].as_u64()
            } else {
                None
            };
            assert_eq!(
                f.render_counters.get(name).copied(),
                value,
                "frame {} counter {}",
                f.index,
                name
            );
        }
        assert_eq!(
            f.quality.draw,
            f.render_counters.contains_key("Draw Calls Count")
        );
        assert_eq!(
            f.quality.set_pass,
            f.render_counters.contains_key("SetPass Calls Count")
        );
    }
    let snapshot = extractor::extract(&profile);
    for (name, stats) in [
        ("Draw Calls Count", &snapshot.rendering.draw_calls),
        ("SetPass Calls Count", &snapshot.rendering.set_pass_calls),
        ("Batches Count", &snapshot.rendering.batches),
        ("Triangles Count", &snapshot.rendering.triangles),
        ("Vertices Count", &snapshot.rendering.vertices),
    ] {
        let mut values: Vec<u64> = frames
            .iter()
            .filter_map(|f| {
                let counters = f["counters"].as_array().unwrap();
                assert_eq!(counters.len(), 5);
                let c = counters
                    .iter()
                    .find(|c| c["name"] == name)
                    .expect("reference counter entry");
                (c["available"] == true).then(|| c["value"].as_u64().expect("integer counter"))
            })
            .collect();
        values.sort();
        assert_eq!(stats.quality.valid_frames, values.len(), "{name}");
        let percentile = |q: f64| {
            if values.is_empty() {
                None
            } else {
                Some(values[((values.len() - 1) as f64 * q).round() as usize] as f64)
            }
        };
        assert_eq!(stats.p50, percentile(0.5));
        assert_eq!(stats.p95, percentile(0.95));
        assert_eq!(stats.p99, percentile(0.99));
        assert_eq!(stats.max, values.last().map(|v| *v as f64));
        println!(
            "{name}: valid={} p95={:?} max={:?}",
            values.len(),
            stats.p95,
            stats.max
        );
    }
    for i in [0, profile.frames.len() - 1] {
        let info = profile
            .details
            .as_ref()
            .unwrap()
            .frame(i, 0, 1)
            .unwrap()
            .info;
        assert_eq!(info.render_counters, profile.frames[i].render_counters);
    }
    println!(
        "frames={} bytes={} elapsed={:?}",
        profile.frames.len(),
        std::fs::metadata(path).unwrap().len(),
        start.elapsed()
    );
}

// Public synthetic bytes: no data or marker IDs copied from a private capture.
fn word(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}
fn synthetic(
    definitions: bool,
    base: u32,
    values: &[Option<u64>],
    tag: u32,
    counter_flag: bool,
    auxiliary_index: u32,
) -> Vec<u8> {
    synthetic_named(
        definitions,
        base,
        values,
        tag,
        counter_flag,
        auxiliary_index,
        "Draw Calls Count",
    )
}
fn synthetic_named(
    definitions: bool,
    base: u32,
    values: &[Option<u64>],
    tag: u32,
    counter_flag: bool,
    auxiliary_index: u32,
    counter_name: &str,
) -> Vec<u8> {
    let mut out = vec![0; 28 + 136];
    word(&mut out, u32::MAX);
    out.resize(out.len() + 64 + 1060 + 32, 0);
    word(
        &mut out,
        if definitions {
            2 + values.len() as u32
        } else {
            0
        },
    );
    if definitions {
        for (id, name, flags) in [
            (base, "Main Thread", 16 << 16),
            (base + 1, "Render.Work", 0),
        ]
        .into_iter()
        .chain(values.iter().enumerate().map(|(i, _)| {
            (
                base + 2 + i as u32,
                counter_name,
                if counter_flag { 0x80 } else { 0 },
            )
        })) {
            word(&mut out, id);
            string(&mut out, name);
            word(&mut out, flags);
            word(&mut out, 0);
        }
    }
    word(&mut out, 2);
    for ti in 0..2 {
        out.extend((ti as u64 + 1).to_le_bytes());
        string(&mut out, "");
        string(
            &mut out,
            if ti == 0 {
                "Main Thread"
            } else {
                "Render Thread"
            },
        );
        let n = if ti == 0 { 1 } else { 1 + values.len() };
        word(&mut out, n as u32);
        for i in 0..n {
            word(&mut out, if ti == 0 { base } else { base + 1 + i as u32 });
            out.extend((if i == 0 { 2_000_000f32 } else { 0f32 }).to_le_bytes());
            out.extend(0u64.to_le_bytes());
            word(&mut out, 0);
        }
        word(&mut out, 1);
        for v in [19, auxiliary_index, 73] {
            word(&mut out, v);
        }
        for _ in 0..3 {
            word(&mut out, 0);
        }
        word(
            &mut out,
            if ti == 0 {
                0
            } else {
                values.iter().flatten().count() as u32
            },
        );
        if ti == 1 {
            for (i, value) in values.iter().enumerate() {
                if let Some(v) = value {
                    for v in [i as u32 + 1, 1, tag, if tag == 4 { 8 } else { 4 }] {
                        word(&mut out, v);
                    }
                    if tag == 4 {
                        out.extend(v.to_le_bytes());
                    } else {
                        word(&mut out, *v as u32);
                    }
                }
            }
        }
        for _ in 0..4 {
            word(&mut out, 0);
        }
    }
    word(&mut out, 0xAFAFAFAF);
    out
}
#[test]
fn counter_metadata_is_typed_thread_independent_and_marker_ids_are_capture_state() {
    use parser::data::unity6_structured::Decoder;
    for base in [19, 8041] {
        let mut decoder = Decoder::default();
        let first = decoder
            .decode(&synthetic(true, base, &[Some(327)], 2, true, 0))
            .unwrap()
            .summary(10);
        assert_eq!(first.draw_calls, 327);
        assert!(first.quality.draw);
        assert_eq!(first.render_events.len(), 1);
        assert!(first.render_events[0]
            .name
            .contains("Render Thread #1 / Render.Work"));
        let zero = decoder
            .decode(&synthetic(false, base, &[Some(0)], 4, true, 0))
            .unwrap()
            .summary(12);
        assert!(zero.quality.draw);
        assert_eq!(zero.render_counters["Draw Calls Count"], 0);
        let missing = decoder
            .decode(&synthetic(false, base, &[None], 2, true, 0))
            .unwrap()
            .summary(14);
        assert!(!missing.quality.draw);
        assert!(!missing.render_counters.contains_key("Draw Calls Count"));
        let stats = extractor::rendering::extract(&[first, zero, missing]);
        assert_eq!(stats.draw_calls.quality.status, "partial");
        assert_eq!(stats.draw_calls.quality.valid_frames, 2);
        assert!(stats
            .draw_calls
            .quality
            .reasons
            .iter()
            .all(|r| r.starts_with("Draw Calls Count:")));
        assert!(stats
            .batches
            .quality
            .reasons
            .iter()
            .all(|r| r.starts_with("Batches Count:")));
        assert_eq!(stats.draw_calls.p95, Some(327.0));
        assert_eq!(stats.batches.p95, None);
        let wide = decoder
            .decode(&synthetic(
                false,
                base,
                &[Some(u32::MAX as u64 + 1)],
                4,
                true,
                0,
            ))
            .unwrap()
            .summary(16);
        assert!(!wide.quality.draw); // no truncation into the legacy u32 field
        assert_eq!(
            wide.render_counters["Draw Calls Count"],
            u32::MAX as u64 + 1
        );
    }
}
#[test]
fn missing_flags_conflicting_or_incomplete_observations_are_not_valid_counts() {
    use parser::data::unity6_structured::Decoder;
    for (values, flag) in [
        (vec![Some(12)], false),
        (vec![Some(12), Some(13)], true),
        (vec![Some(12), None], true),
    ] {
        let frame = Decoder::default()
            .decode(&synthetic(true, 44, &values, 2, flag, 0))
            .unwrap()
            .summary(0);
        assert!(!frame.quality.draw);
        assert!(frame.render_counters.is_empty());
    }
    let frame = Decoder::default()
        .decode(&synthetic(true, 44, &[Some(12), Some(12)], 2, true, 0))
        .unwrap()
        .summary(0);
    assert_eq!(frame.draw_calls, 12); // equal observations are not summed
}
#[test]
fn invalid_counter_types_and_auxiliary_boundaries_fail_closed() {
    use parser::data::unity6_structured::Decoder;
    assert!(Decoder::default()
        .decode(&synthetic(true, 44, &[Some(12)], 3, true, 0))
        .is_err());
    assert!(Decoder::default()
        .decode(&synthetic(true, 44, &[Some(12)], 2, true, 1))
        .is_err());
    let bytes = synthetic(true, 44, &[Some(12)], 2, true, 0);
    for end in 0..bytes.len() - 4 {
        let mut truncated = bytes[..end].to_vec();
        word(&mut truncated, 0xAFAFAFAF);
        assert!(
            Decoder::default().decode(&truncated).is_err(),
            "accepted boundary {end}"
        );
    }
}

#[test]
fn all_five_counter_names_accept_observed_zero_and_large_u64_values() {
    use parser::data::unity6_structured::{Decoder, RENDER_COUNTER_NAMES};
    for name in RENDER_COUNTER_NAMES {
        let tag = if name == "Triangles Count" || name == "Vertices Count" {
            4
        } else {
            2
        };
        for value in [0, if tag == 4 { u32::MAX as u64 + 17 } else { 888 }] {
            let f = Decoder::default()
                .decode(&synthetic_named(
                    true,
                    614,
                    &[Some(value)],
                    tag,
                    true,
                    0,
                    name,
                ))
                .unwrap()
                .summary(0);
            assert_eq!(f.render_counters[name], value);
        }
    }
}

#[tokio::test]
async fn render_counters_reach_production_snapshot_frame_queries_and_mcp() {
    use unity_profiler_analysis_agent_lib::mcp::{tools::dispatch, MetricsStore};
    let mut bytes = Vec::new();
    for (index, value) in [2000, 0].into_iter().enumerate() {
        let mut body = synthetic(index == 0, 314, &[Some(value)], 2, true, 0);
        body[8..16].copy_from_slice(&(index as u64 * 16_000_000).to_le_bytes());
        for v in [0x20220328, body.len() as u32, 6000, 3, 9, 2, 1] {
            word(&mut bytes, v);
        }
        bytes.extend(body);
    }
    word(&mut bytes, 0xDEADFEED);
    let profile = parser::data::parse(&bytes::Bytes::from(bytes), "public-render.data", 0)
        .await
        .unwrap();
    let store = MetricsStore::new();
    store
        .set_capture(extractor::extract(&profile), profile.details)
        .await;
    let hotspots = dispatch(
        &store,
        "performance_hotspots",
        serde_json::json!({"area":"rendering","limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(hotspots["rows"][0]["callCount"], 2);
    assert_eq!(hotspots["rows"][0]["totalMs"], 4.0);
    let frame = dispatch(
        &store,
        "performance_frame",
        serde_json::json!({"frame_index":1}),
    )
    .await
    .unwrap();
    assert_eq!(frame["info"]["renderCounters"]["Draw Calls Count"], 0);
    let timeline = dispatch(&store, "performance_frames", serde_json::json!({"start":0}))
        .await
        .unwrap();
    assert_eq!(timeline["frames"][0]["drawCalls"], 2000);
    assert_eq!(timeline["frames"][1]["drawCalls"], 0);
    assert!(timeline["frames"][1]["setPassCalls"].is_null());
    let analysis = dispatch(
        &store,
        "performance_analysis",
        serde_json::json!({"focus":"rendering"}),
    )
    .await
    .unwrap();
    assert_eq!(analysis["issues"][0]["affectedFrames"], 1);
    assert_eq!(analysis["issues"][0]["evidenceFrames"][0]["frameIndex"], 0);
    let summary = dispatch(&store, "performance_session_summary", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(summary["hotspotCounts"]["rendering"], 1);
    assert!(summary["rendering"].get("topRenderEvents").is_none());
}
