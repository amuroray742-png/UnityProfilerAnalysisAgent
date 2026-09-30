//! Synthetic public contracts, not real-recording version validation.
use bytes::Bytes;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use unity_profiler_analysis_agent_lib::{
    extractor, mcp,
    parser::{
        self,
        data::{self, unity6_structured::Decoder},
    },
};
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
fn body(definitions: bool) -> Vec<u8> {
    let mut out = vec![0; 28 + 136];
    word(&mut out, u32::MAX);
    out.resize(out.len() + 64 + 1060 + 32, 0);
    word(&mut out, if definitions { 2 } else { 0 });
    if definitions {
        for (id, name) in [(913, "Main Thread"), (718, "GC.Alloc")] {
            word(&mut out, id);
            string(&mut out, name);
            word(&mut out, 17 << 16);
            word(&mut out, 0);
        }
    }
    word(&mut out, 1);
    out.extend(42u64.to_le_bytes());
    string(&mut out, "");
    string(&mut out, "Main Thread");
    word(&mut out, 2);
    for (id, ns, children) in [(913, 1000000f32, 1), (718, 100f32, 0)] {
        word(&mut out, id);
        out.extend(ns.to_le_bytes());
        out.extend(3_000_000u64.to_le_bytes());
        word(&mut out, children);
    }
    for value in [0, 0, 1, 1, 136, 0, 1, 1, 1, 3, 4, 136, 0, 1, 0, 0] {
        word(&mut out, value);
    }
    word(&mut out, 0xAFAFAFAF);
    out
}

fn memory_body(
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
            if id >= base + 2 {
                word(&mut out, 1);
                word(&mut out, 0x204);
                string(&mut out, "Bytes");
            } else {
                word(&mut out, 0);
            }
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
                    for v in [
                        i as u32 + 1,
                        1,
                        tag,
                        if tag == 4 || tag == 5 { 8 } else { 4 },
                    ] {
                        word(&mut out, v);
                    }
                    if tag == 4 || tag == 5 {
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

fn capture(version: [u32; 5], bodies: Vec<Vec<u8>>) -> Bytes {
    let mut out = Vec::new();
    for (i, mut body) in bodies.into_iter().enumerate() {
        body[..4].copy_from_slice(&(i as u32).to_le_bytes());
        body[8..16].copy_from_slice(&(i as u64 * 1_000_000).to_le_bytes());
        for v in [0x20220328, body.len() as u32] {
            word(&mut out, v);
        }
        for v in version {
            word(&mut out, v);
        }
        out.extend(body);
    }
    word(&mut out, 0xDEADFEED);
    Bytes::from(out)
}
async fn parse(b: &Bytes) -> parser::ParsedProfile {
    data::parse(b, "test.data", b.len() as u64).await.unwrap()
}
#[tokio::test]
async fn versions_bytes_gc_precision_and_missing_are_not_zero() {
    for version in [
        [2022, 3, 47, 2, 1],
        [6000, 3, 99, 2, 1],
        [6000, 3, 23, 2, 1],
    ] {
        let b = capture(version, vec![body(true), body(false)]);
        let p = parse(&b).await;
        assert_eq!(p.frames[0].gc_alloc_bytes, 136);
        assert_eq!(p.frames[1].gc_alloc_bytes, 136);
        assert_eq!(p.frames[0].duration_ms, 1.0);
        assert!(!p.frames[1].quality.duration);
        assert_eq!(
            p.frames[0].quality.version_verified,
            Some(version[0] == 6000 && version[2] == 23)
        );
        assert_eq!(
            extractor::extract(&p)
                .gc
                .alloc_per_frame_bytes
                .quality
                .can_diagnose(),
            version[0] == 6000 && version[2] == 23
        );
    }
    for v in [
        [2022, 2, 1, 2, 1],
        [2021, 3, 1, 2, 1],
        [6000, 2, 23, 2, 1],
        [6001, 3, 23, 2, 1],
    ] {
        assert!(data::parse(&capture(v, vec![body(true)]), "bad.data", 0)
            .await
            .is_err());
    }
    let mut b = body(true);
    let d = Decoder::default().decode(&b).unwrap();
    let start = d.thread_section_offset + 4 + 8 + 4 + 12 + 4;
    b[start + 40 + 16..start + 40 + 20].copy_from_slice(&1536u32.to_le_bytes());
    let len = b.len();
    b[len - 24..len - 20].copy_from_slice(&1536u32.to_le_bytes());
    let p = parse(&capture([2022, 3, 47, 2, 1], vec![b.clone()])).await;
    assert_eq!(p.frames[0].gc_alloc_bytes, 1536);
    b[len - 24..len - 20].copy_from_slice(&1537u32.to_le_bytes());
    assert!(
        data::parse(&capture([2022, 3, 47, 2, 1], vec![b]), "bad.data", 0)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn all_threads_and_missing_main_preserve_gc() {
    let mut b = body(true);
    let d = Decoder::default().decode(&b).unwrap();
    let section = d.thread_section_offset;
    let mut worker = b[section + 4..b.len() - 4].to_vec();
    worker[..8].copy_from_slice(&43u64.to_le_bytes());
    worker[12..24].copy_from_slice(b"Worker     \0");
    b[section..section + 4].copy_from_slice(&2u32.to_le_bytes());
    b.splice(b.len() - 4..b.len() - 4, worker);
    let p = parse(&capture([2022, 3, 1, 2, 1], vec![b.clone()])).await;
    assert_eq!(p.frames[0].gc_alloc_bytes, 272);
    assert_eq!(p.details.unwrap().frame(0, 0, 10).unwrap().thread_count, 2);
    b[section + 16..section + 28].copy_from_slice(b"Worker     \0");
    let p = parse(&capture([2022, 3, 1, 2, 1], vec![b])).await;
    assert!(!p.frames[0].quality.cpu);
    assert_eq!(p.frames[0].gc_alloc_bytes, 272);
}
#[tokio::test]
async fn strict_file_bytes_endings_unknown_sections_and_cache() {
    let mut b = body(true);
    b.splice(b.len() - 4..b.len() - 4, [7u8; 100]);
    let bytes = capture(
        [6000, 3, 23, 2, 1],
        vec![b, body(false), body(false), body(false), body(false)],
    );
    let path = std::env::temp_dir().join(format!("upaa-repairs-{}.data", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).unwrap();
    let file = data::parse_path(&path).unwrap();
    let p = parse(&bytes).await;
    assert_eq!(
        serde_json::to_value(&file.frames).unwrap(),
        serde_json::to_value(&p.frames).unwrap()
    );
    let store = file.details.unwrap();
    let section = store.sections(0, 0, 50).unwrap();
    let rows = section["rows"].as_array().unwrap();
    let trailer = rows.iter().find(|r| r["name"] == "frame trailer").unwrap();
    assert_eq!(trailer["byteLength"], 100);
    assert_eq!(trailer["rawHex"].as_str().unwrap().len(), 128);
    assert_eq!(trailer["rawTruncated"], true);
    let mut joins = vec![];
    for _ in 0..8 {
        let s = store.clone();
        joins.push(std::thread::spawn(move || {
            s.hierarchy(0, None, 0, 1, 64).unwrap()
        }));
    }
    for t in joins {
        t.join().unwrap();
    }
    assert_eq!(store.cache_counts()["decodes"], 1);
    for i in 1..5 {
        store.load(i).unwrap();
    }
    assert_eq!(store.cache_counts()["frames"], 4);
    store.load(0).unwrap();
    assert_eq!(store.cache_counts()["decodes"], 6);
    let mut changed = bytes.to_vec();
    changed[28] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert!(store.load(0).is_err());
    for data in [
        Bytes::from([bytes.as_ref(), &[1]].concat()),
        bytes.slice(..bytes.len() - 1),
    ] {
        std::fs::write(&path, &data).unwrap();
        assert!(data::parse_path(&path).is_err());
        assert!(data::parse(&data, "bad.data", 0).await.is_err());
    }
    drop(store);
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn memory_units_conflicts_precision_coverage_and_protocol() {
    let name = "Total Used Memory";
    let b = |values: &[Option<u64>], tag, flag| memory_body(true, 50, values, tag, flag, 0, name);
    let p = parse(&capture(
        [6000, 3, 23, 2, 1],
        vec![
            b(&[Some(u64::MAX)], 5, true),
            b(&[None], 5, true),
            b(&[Some(0)], 5, true),
        ],
    ))
    .await;
    assert_eq!(
        p.frames[0].memory[name].value.as_deref(),
        Some("18446744073709551615")
    );
    assert_eq!(p.frames[2].memory[name].value.as_deref(), Some("0"));
    let snapshot = extractor::extract(&p);
    let m = &snapshot.memory.counters[name];
    assert_eq!(m.valid_frames, 2);
    assert_eq!(m.delta.as_deref(), Some("-18446744073709551615"));
    assert_eq!(m.peak_frame, Some(0));
    let metrics = mcp::MetricsStore::new();
    metrics.set_capture(snapshot, p.details).await;
    let page = mcp::tools::dispatch(
        &metrics,
        "performance_memory",
        serde_json::json!({"name":name,"limit":2}),
    )
    .await
    .unwrap();
    assert_eq!(page["nextStart"], 2);
    assert!(page["rows"][1]["observation"]["value"].is_null());
    assert!(mcp::tools::dispatch(
        &metrics,
        "performance_memory",
        serde_json::json!({"name":"bad"})
    )
    .await
    .is_err());
    let sec = mcp::tools::dispatch(
        &metrics,
        "performance_frame_sections",
        serde_json::json!({"frame_index":0}),
    )
    .await
    .unwrap();
    assert!(sec["total"].as_u64().unwrap() > 0);
    for values in [
        vec![Some(1), Some(2)],
        vec![Some(1), None],
        vec![Some(u64::MAX)],
    ] {
        let f = Decoder::default()
            .decode(&b(&values, 4, true))
            .unwrap()
            .summary(0);
        assert!(f.memory[name].value.is_none());
    }
    let mut no_units = b(&[Some(5)], 4, true);
    let offset = no_units
        .windows(4)
        .position(|w| w == 0x204u32.to_le_bytes())
        .unwrap();
    no_units[offset..offset + 4].copy_from_slice(&4u32.to_le_bytes());
    let f = Decoder::default().decode(&no_units).unwrap().summary(0);
    assert!(f.memory[name].reason.as_ref().unwrap().contains("单位"));
    assert!(Decoder::default()
        .decode(&b(&[Some(1)], 4, false))
        .unwrap()
        .summary(0)
        .memory[name]
        .value
        .is_none());
}
#[tokio::test]
async fn cancellation_during_import_and_checkpoint_lifetime() {
    let bytes = capture([6000, 3, 23, 2, 1], vec![body(true), body(false)]);
    let path = std::env::temp_dir().join(format!("upaa-cancel-{}.data", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let flag2 = flag.clone();
    let result = data::parse_path_cancel(
        &path,
        &mut move |_, _| flag2.store(true, Ordering::SeqCst),
        flag.clone(),
    );
    assert!(matches!(result, Err(parser::ParseError::Cancelled)));
    flag.store(false, Ordering::SeqCst);
    let p = data::parse_path_cancel(&path, &mut |_, _| {}, flag.clone()).unwrap();
    flag.store(true, Ordering::SeqCst);
    assert!(p.details.unwrap().load(1).is_ok());
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn skipped_blocks_and_frame_gaps_do_not_inflate_coverage() {
    let mut skip = vec![0u8; 28];
    word(&mut skip, 0xAFAFAFAF);
    let p = parse(&capture(
        [2022, 3, 47, 2, 1],
        vec![body(true), skip, body(false)],
    ))
    .await;
    let s = extractor::extract(&p);
    let c = s.meta.parsing.unwrap();
    assert_eq!(c.raw_blocks, 3);
    assert_eq!(c.skipped_frames, 1);
    assert_eq!(s.gc.alloc_per_frame_bytes.quality.valid_frames, 2);
    assert_eq!(s.gc.alloc_per_frame_bytes.quality.total_frames, 3);
    assert!(!p.frames[0].quality.duration);
    let mut bytes = capture([6000, 3, 23, 2, 1], vec![body(true), body(false)]).to_vec();
    let offset = 28 + body(true).len() + 28;
    bytes[offset..offset + 4].copy_from_slice(&5u32.to_le_bytes());
    let p = parse(&Bytes::from(bytes)).await;
    assert!(!p.frames[0].quality.duration);
}
#[test]
fn bounded_random_damage_never_panics() {
    let good = body(true);
    let mut seed = 71u64;
    for _ in 0..512 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = good.clone();
        let offset = (seed as usize) % (bytes.len() - 4);
        bytes[offset] ^= (seed >> 32) as u8;
        let result = std::panic::catch_unwind(|| Decoder::default().decode(&bytes));
        assert!(result.is_ok());
    }
}

#[tokio::test]
#[ignore = "requires UNITY_MEMORY_DATA_PATH and UNITY_MEMORY_REFERENCE_PATH; explicit missing inputs fail"]
async fn editor_memory_counter_reference() {
    let path = std::env::var("UNITY_MEMORY_DATA_PATH").expect("set UNITY_MEMORY_DATA_PATH");
    let reference: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("UNITY_MEMORY_REFERENCE_PATH").expect("set UNITY_MEMORY_REFERENCE_PATH"),
        )
        .unwrap(),
    )
    .unwrap();
    let p = parser::parse_file(std::path::Path::new(&path))
        .await
        .unwrap();
    let truth = reference["frames"].as_array().expect("frames");
    assert!(!truth.is_empty());
    assert_eq!(truth.len(), p.frames.len());
    for f in truth {
        let i = f["frame_index"].as_u64().unwrap() as usize;
        for c in f["counters"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let expected = if c["available"] == true {
                Some(c["value"].as_str().unwrap())
            } else {
                None
            };
            assert_eq!(
                p.frames[i].memory[name].value.as_deref(),
                expected,
                "frame {i} counter {name}"
            );
        }
    }
}
