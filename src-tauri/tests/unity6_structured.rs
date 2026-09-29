use serde::Deserialize;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
};
use unity_profiler_analysis_agent_lib::parser::data::{
    header::read_block_header, unity6_structured::Decoder,
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

#[test]
fn sequential_samples_gc_and_persistent_markers() {
    let mut decoder = Decoder::default();
    let first = decoder.decode(&body(true)).unwrap();
    assert_eq!(first.threads[0].gc_bytes, 136);
    assert_eq!(first.threads[0].samples[1].parent, Some(0));
    assert_eq!(first.threads[0].samples[0].editor_time_ms(), 1.0);
    assert_eq!(first.trailer_offset, body(true).len() - 4);
    let summary = first.summary(17);
    assert_eq!(summary.index, 17);
    assert!(summary.quality.cpu && summary.quality.gc && summary.quality.sites);
    assert!(!summary.quality.duration && !summary.quality.draw);
    assert_eq!(summary.cpu_ms, 1.0);
    assert_eq!(summary.gc_alloc_sites[0].name, "Main Thread");
    assert_eq!(summary.gc_alloc_sites[0].total_bytes, 136);
    let second = decoder.decode(&body(false)).unwrap();
    assert_eq!(second.threads[0].samples[1].name, "GC.Alloc");
    assert!(Decoder::default().decode(&body(false)).is_err());
}

#[test]
fn rejects_truncation_at_every_boundary() {
    let bytes = body(true);
    for end in 0..bytes.len() - 4 {
        let mut truncated = bytes[..end].to_vec();
        word(&mut truncated, 0xAFAFAFAF);
        assert!(
            Decoder::default().decode(&truncated).is_err(),
            "accepted truncation at {end}"
        );
    }
}

#[test]
fn corrupt_structure_never_falls_back_to_search() {
    let bytes = body(true);
    let decoded = Decoder::default().decode(&bytes).unwrap();
    let sample_start = decoded.thread_section_offset + 4 + 8 + 4 + 12 + 4;
    let edits = [
        (sample_start, 999999u32),
        (sample_start + 4, f32::NAN.to_bits()),
        (sample_start + 16, 5),
        (sample_start + 40, 1),
        (sample_start + 40 + 8, 0), // GC count removed
        (bytes.len() - 24, 137),
    ]; // general GC payload differs
    for (offset, value) in edits {
        let mut damaged = bytes.clone();
        damaged[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            Decoder::default().decode(&damaged).is_err(),
            "accepted corruption at {offset}"
        );
    }
}

#[test]
fn counted_post_gc_indices_are_bounded_and_preserved() {
    let mut bytes = body(true);
    let section = bytes.len() - 4 - 11 * 4;
    assert_eq!(&bytes[section..section + 4], &0u32.to_le_bytes());
    bytes.splice(
        section..section + 4,
        [2u32, 0, 1].into_iter().flat_map(u32::to_le_bytes),
    );
    let decoded = Decoder::default().decode(&bytes).unwrap();
    assert_eq!(decoded.threads[0].post_gc_sample_indices, vec![0, 1]);
    bytes[section + 8..section + 12].copy_from_slice(&2u32.to_le_bytes());
    assert!(Decoder::default().decode(&bytes).is_err());
}

#[tokio::test]
async fn flows_preserve_types_order_cross_frame_ids_and_validate_boundaries() {
    use unity_profiler_analysis_agent_lib::parser::data;
    let mut b = body(true);
    let tail = b.len() - 8;
    let mut records = vec![63u32];
    for i in 0..63 {
        records.extend([
            if i == 62 { u32::MAX } else { i % 2 },
            4294967295,
            if i == 62 { 99 } else { i % 4 },
        ]);
    }
    b.splice(tail..tail + 4, records.iter().flat_map(|v| v.to_le_bytes()));
    let decoded = Decoder::default().decode(&b).unwrap();
    assert_eq!(decoded.threads[0].flow_events.len(), 63);
    assert_eq!(decoded.threads[0].flow_events[62].sample_index, -1);
    let mut damaged = b.clone();
    damaged[tail + 4..tail + 8].copy_from_slice(&2u32.to_le_bytes());
    assert!(Decoder::default().decode(&damaged).is_err());
    damaged = b.clone();
    damaged[tail + 4..tail + 8].copy_from_slice(&(-2i32).to_le_bytes());
    assert!(Decoder::default().decode(&damaged).is_err());
    assert!(Decoder::default().decode(&b[..b.len() - 9]).is_err());
    let mut bytes = Vec::new();
    for _ in 0..2 {
        for v in [0x20220328, b.len() as u32, 6000, 3, 23, 2, 1] {
            word(&mut bytes, v);
        }
        bytes.extend(&b);
    }
    word(&mut bytes, 0xDEADFEED);
    let bytes = bytes::Bytes::from(bytes);
    let p = data::parse(&bytes, "flows.data", bytes.len() as u64)
        .await
        .unwrap();
    let s = p.details.clone().unwrap();
    let page = s.flows(0, 1, Some(u32::MAX), 0, 50).unwrap();
    assert_eq!(page["available"], true);
    assert_eq!(page["total"], 126);
    assert_eq!(page["nextStart"], 50);
    let next = s.flows(0, 1, Some(u32::MAX), 50, 50).unwrap();
    assert_eq!(next["rows"][12]["kind"], "Unknown");
    assert!(next["rows"][12]["marker"].is_null());
    assert_eq!(next["rows"][13]["frameIndex"], 1);
    assert_eq!(next["unknownTypes"], 2);
    assert_eq!(s.flows(0, 1, Some(0), 0, 10).unwrap()["total"], 0);
    let metrics = unity_profiler_analysis_agent_lib::mcp::MetricsStore::new();
    metrics
        .set_capture(
            unity_profiler_analysis_agent_lib::extractor::extract(&p),
            Some(s),
        )
        .await;
    let result = unity_profiler_analysis_agent_lib::mcp::tools::dispatch(
        &metrics,
        "performance_flow_events",
        serde_json::json!({"frame_index":0,"end_frame_index":1,"flow_id":4294967295u64}),
    )
    .await
    .unwrap();
    assert_eq!(result["total"], 126);

    // Same ID on another thread remains a distinct observation, not deduplicated.
    let section = decoded.thread_section_offset;
    let mut second_thread = b[section + 4..b.len() - 4].to_vec();
    second_thread[..8].copy_from_slice(&43u64.to_le_bytes());
    b[section..section + 4].copy_from_slice(&2u32.to_le_bytes());
    b.splice(b.len() - 4..b.len() - 4, second_thread);
    let mut bytes = Vec::new();
    for v in [0x20220328, b.len() as u32, 6000, 3, 23, 2, 1] {
        word(&mut bytes, v);
    }
    bytes.extend(b);
    word(&mut bytes, 0xDEADFEED);
    let bytes = bytes::Bytes::from(bytes);
    let p = data::parse(&bytes, "threads.data", bytes.len() as u64)
        .await
        .unwrap();
    let page = p
        .details
        .unwrap()
        .flows(0, 0, Some(u32::MAX), 62, 2)
        .unwrap();
    assert_eq!(page["total"], 126);
    assert_eq!(page["rows"][0]["threadId"], "42");
    assert_eq!(page["rows"][1]["threadId"], "43");
}

#[tokio::test]
async fn production_file_and_bytes_paths_share_capture_state_and_reject_truncation() {
    use unity_profiler_analysis_agent_lib::{extractor, parser::data};
    let mut bytes = Vec::new();
    for definitions in [true, false] {
        let mut body = body(definitions);
        if !definitions {
            body[8..16].copy_from_slice(&1_250_000u64.to_le_bytes());
        }
        for value in [0x20220328, body.len() as u32, 6000, 3, 23, 2, 1] {
            word(&mut bytes, value);
        }
        bytes.extend(body);
    }
    word(&mut bytes, 0xDEADFEED);
    let path = std::env::temp_dir().join(format!("upaa_structured_{}.data", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let from_file = data::parse_path(&path).unwrap();
    let from_bytes = data::parse(
        &bytes::Bytes::copy_from_slice(&bytes),
        "test.data",
        bytes.len() as u64,
    )
    .await
    .unwrap();
    for profile in [&from_file, &from_bytes] {
        assert_eq!(profile.frames.len(), 2);
        assert_eq!(profile.frames[1].index, 1);
        let snapshot = extractor::extract(profile);
        assert_eq!(snapshot.cpu.main_thread_ms.p95, Some(1.0));
        assert_eq!(snapshot.cpu.main_thread_ms.quality.valid_frames, 2);
        assert_eq!(snapshot.gc.total_alloc_bytes, Some(272));
        assert_eq!(snapshot.gc.top_alloc_sites[0].total_bytes, 272);
        assert_eq!(snapshot.meta.duration_ms, Some(1.25));
        assert_eq!(snapshot.meta.duration_quality.status, "partial");
        assert_eq!(snapshot.meta.duration_quality.valid_frames, 1);
        assert!(!profile.frames[1].quality.duration);
        assert_eq!(snapshot.rendering.draw_calls.p95, None);
        let details = profile.details.as_ref().unwrap();
        let page = details.hierarchy(1, None, 1, 1, 64).unwrap();
        assert_eq!(page.samples[0].name, "GC.Alloc");
        assert_eq!(page.samples[0].parent_index, Some(0));
        assert_eq!(page.samples[0].gc_alloc_bytes, Some(136));
        assert_eq!(page.samples[0].raw_start_ns.as_deref(), Some("3000000"));
        assert_eq!(page.info.frame_time_ms, None);
        assert_eq!(
            details.frame(0, 0, 1).unwrap().info.frame_time_ms,
            Some(1.25)
        );
    }
    let mut changed = bytes.clone();
    changed[28] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(
        from_file.details.as_ref().unwrap().load(0),
        Err(unity_profiler_analysis_agent_lib::parser::detail::QueryError::SourceChanged)
    ));
    for cut in [bytes.len() - 1, bytes.len() - 4, 30] {
        std::fs::write(&path, &bytes[..cut]).unwrap();
        assert!(
            data::parse_path(&path).is_err(),
            "file accepted truncation at {cut}"
        );
        assert!(data::parse(
            &bytes::Bytes::copy_from_slice(&bytes[..cut]),
            "cut.data",
            cut as u64
        )
        .await
        .is_err());
    }
    let mut reversed = bytes.clone();
    reversed[36..44].copy_from_slice(&2_500_000u64.to_le_bytes());
    let profile = data::parse(&bytes::Bytes::from(reversed), "reset.data", 0)
        .await
        .unwrap();
    assert!(!profile.frames[0].quality.duration);
    assert!(profile.frames[0].quality.cpu && profile.frames[0].quality.gc);
    assert_eq!(extractor::extract(&profile).meta.duration_ms, None);
    let mut zero_interval = bytes.clone();
    zero_interval[36..44].copy_from_slice(&1_250_000u64.to_le_bytes());
    let profile = data::parse(&bytes::Bytes::from(zero_interval), "zero.data", 0)
        .await
        .unwrap();
    assert!(profile.frames[0].quality.duration);
    assert_eq!(profile.frames[0].duration_ms, 0.0);
    std::fs::remove_file(path).unwrap();
}

#[derive(Deserialize)]
struct Dump {
    unity_version: String,
    frame_count: usize,
    frames: Vec<DumpFrame>,
}
#[derive(Deserialize)]
struct DumpFrame {
    frame_index: usize,
    frame_time_ms: f64,
    sample_count_total: usize,
    gc_alloc_bytes_total: u64,
    threads: Vec<DumpThread>,
}
#[derive(Deserialize)]
struct DumpThread {
    thread_id: u64,
    thread_name: String,
    thread_group_name: String,
    gc_alloc_total_bytes: u64,
    samples: Vec<DumpSample>,
}
#[derive(Deserialize)]
struct DumpSample {
    sample_index: usize,
    marker_id: i64,
    marker_name: String,
    category_index: u16,
    time_ms: f64,
    start_time_ms: f64,
    children_count: u32,
    metadata_count: u32,
    gc_alloc_bytes: u64,
}

#[tokio::test]
#[ignore = "requires UNITY_PROFILER_DATA_PATH and UNITY_PROFILER_DUMP_PATH"]
async fn real_capture_without_reference_assisted_location() {
    let started = std::time::Instant::now();
    let data_path = PathBuf::from(
        std::env::var("UNITY_PROFILER_DATA_PATH").expect("set UNITY_PROFILER_DATA_PATH"),
    );
    let dump_path = PathBuf::from(
        std::env::var("UNITY_PROFILER_DUMP_PATH").expect("set UNITY_PROFILER_DUMP_PATH"),
    );
    let dump: Dump =
        serde_json::from_reader(BufReader::new(File::open(&dump_path).expect("open dump")))
            .expect("deserialize dump");
    assert!(matches!(
        dump.unity_version.as_str(),
        "6000.3.23f1" | "6000.3.9f1"
    ));
    assert!(!dump.frames.is_empty(), "reference exports no frames");
    assert!(dump.frame_count >= dump.frames.len());
    let expected_samples: usize = dump.frames.iter().map(|f| f.sample_count_total).sum();
    assert!(expected_samples > 0, "reference exports no samples");
    let reference_indices: std::collections::HashSet<_> =
        dump.frames.iter().map(|f| f.frame_index).collect();
    assert_eq!(
        reference_indices.len(),
        dump.frames.len(),
        "duplicate reference indices"
    );
    assert!(reference_indices.iter().all(|i| *i < dump.frame_count));
    let mut file = File::open(&data_path).expect("open data");
    let mut decoder = Decoder::default();
    let mut frames = 0;
    let mut verified_samples = 0;
    loop {
        let mut header = [0u8; 28];
        file.read_exact(&mut header[..4])
            .expect("block or end marker");
        if u32::from_le_bytes(header[..4].try_into().unwrap()) == 0xDEADFEED {
            break;
        }
        file.read_exact(&mut header[4..]).unwrap();
        let header = read_block_header(&header).unwrap();
        header.validate().unwrap();
        assert_eq!(header.unity_version_string(), dump.unity_version);
        let mut body = vec![0; header.body_size as usize];
        file.read_exact(&mut body).unwrap();
        // The decoder sees only binary bytes and capture state, never dump fields.
        let actual = decoder
            .decode(&body)
            .unwrap_or_else(|e| panic!("block[{frames}]: {e}"));
        if let Some(expected) = dump.frames.iter().find(|f| f.frame_index == frames) {
            assert_eq!(
                actual.threads.len(),
                expected.threads.len(),
                "frame[{frames}] thread count"
            );
            assert_eq!(
                actual
                    .threads
                    .iter()
                    .map(|t| t.samples.len())
                    .sum::<usize>(),
                expected.sample_count_total
            );
            assert_eq!(
                actual.threads.iter().map(|t| t.gc_bytes).sum::<u64>(),
                expected.gc_alloc_bytes_total
            );
            for (thread_index, (thread, reference)) in
                actual.threads.iter().zip(&expected.threads).enumerate()
            {
                assert_eq!(thread.name, reference.thread_name);
                assert_eq!(thread.group, reference.thread_group_name);
                assert!(
                    reference.thread_id == thread.id
                        || reference.thread_id == thread.id as i32 as i64 as u64
                );
                assert_eq!(thread.gc_bytes, reference.gc_alloc_total_bytes);
                assert_eq!(thread.samples.len(), reference.samples.len());
                for (index, (sample, reference)) in
                    thread.samples.iter().zip(&reference.samples).enumerate()
                {
                    let context = format!("frame[{frames}] thread[{thread_index}] sample[{index}]");
                    assert_eq!(index, reference.sample_index, "{context}");
                    assert!(
                        sample.marker_id == reference.marker_id as u32
                            || sample.marker_id == u32::MAX
                                && sample.parent.is_none()
                                && reference.marker_id == 0,
                        "{context}"
                    );
                    assert_eq!(sample.name, reference.marker_name, "{context}");
                    assert_eq!(sample.category, reference.category_index, "{context}");
                    // serde_json's decimal-to-f64 conversion can differ by a double ULP;
                    // both values still must round to the exact Editor float32 value.
                    assert_eq!(
                        sample.editor_time_ms() as f32,
                        reference.time_ms as f32,
                        "{context}"
                    );
                    assert_eq!(
                        sample.editor_start_ms() as f32,
                        reference.start_time_ms as f32,
                        "{context}"
                    );
                    assert_eq!(sample.children, reference.children_count, "{context}");
                    assert_eq!(sample.metadata_count, reference.metadata_count, "{context}");
                    assert_eq!(
                        sample.gc_bytes.unwrap_or(0),
                        reference.gc_alloc_bytes,
                        "{context}"
                    );
                    verified_samples += 1;
                }
            }
        }
        frames += 1;
    }
    let mut trailing = [0];
    assert_eq!(file.read(&mut trailing).unwrap(), 0);
    assert_eq!(frames, dump.frame_count);
    assert_eq!(verified_samples, expected_samples);
    let imported_dump = unity_profiler_analysis_agent_lib::parser::parse_file(&dump_path)
        .await
        .expect("production dump parse_file");
    assert_eq!(imported_dump.frames.len(), dump.frames.len());
    let profile = unity_profiler_analysis_agent_lib::parser::parse_file(&data_path)
        .await
        .expect("production parse_file");
    assert_eq!(profile.frames.len(), frames);
    for reference in &dump.frames {
        // Query storage is checked independently of the aggregate snapshot.
        // Full one-frame loads verify preservation; bounded pages verify IPC selection.
        let binary_tree = profile
            .details
            .as_ref()
            .unwrap()
            .load(reference.frame_index)
            .unwrap();
        let dump_tree = imported_dump
            .details
            .as_ref()
            .unwrap()
            .load(reference.frame_index)
            .unwrap();
        assert_eq!(binary_tree.threads.len(), reference.threads.len());
        assert_eq!(dump_tree.threads.len(), reference.threads.len());
        for ((binary, dumped), expected) in binary_tree
            .threads
            .iter()
            .zip(&dump_tree.threads)
            .zip(&reference.threads)
        {
            assert_eq!(binary.info.name, expected.thread_name);
            assert_eq!(binary.samples.len(), expected.samples.len());
            assert_eq!(dumped.samples.len(), expected.samples.len());
            for ((a, b), e) in binary
                .samples
                .iter()
                .zip(&dumped.samples)
                .zip(&expected.samples)
            {
                assert_eq!(a.sample_index, e.sample_index);
                assert_eq!(a.name, e.marker_name);
                assert_eq!(a.parent_index, b.parent_index);
                assert_eq!(a.depth, b.depth);
                assert_eq!(a.total_ms as f32, e.time_ms as f32);
                assert_eq!(a.gc_alloc_bytes, b.gc_alloc_bytes);
            }
        }
        if reference.frame_index == dump.frames[0].frame_index {
            for source in [
                profile.details.as_ref().unwrap(),
                imported_dump.details.as_ref().unwrap(),
            ] {
                let page = source
                    .hierarchy(reference.frame_index, None, 1, 5, 64)
                    .unwrap();
                assert!(page.samples.len() <= 5);
                assert_eq!(page.samples[0].sample_index, 1);
                assert_eq!(
                    page.info.gc_alloc_bytes,
                    Some(reference.gc_alloc_bytes_total)
                );
            }
        }
        let frame = &profile.frames[reference.frame_index];
        assert_eq!(frame.index, reference.frame_index);
        assert!(
            frame.quality.cpu && frame.quality.gc && frame.quality.samples && frame.quality.sites
        );
        // Render counter presence is independently checked against Editor counters.
        if frame.index + 1 < profile.frames.len() {
            assert!(frame.quality.duration);
            assert_eq!(frame.duration_ms as f32, reference.frame_time_ms as f32);
        } else {
            assert!(!frame.quality.duration);
        }
        let main = reference
            .threads
            .iter()
            .find(|t| t.thread_name == "Main Thread")
            .unwrap();
        assert_eq!(frame.cpu_ms as f32, main.samples[0].time_ms as f32);
        assert_eq!(
            frame
                .main_thread_samples
                .iter()
                .map(|s| s.call_count)
                .sum::<u64>(),
            main.samples.len() as u64
        );
        let mut expected_cpu = std::collections::BTreeMap::<String, (f64, u64, f64)>::new();
        for sample in &main.samples {
            let row = expected_cpu.entry(sample.marker_name.clone()).or_default();
            row.0 += sample.time_ms;
            row.1 += 1;
            row.2 = row.2.max(sample.time_ms);
        }
        for summaries in [
            &frame.main_thread_samples,
            &imported_dump
                .frames
                .iter()
                .find(|f| f.index == frame.index)
                .unwrap()
                .main_thread_samples,
        ] {
            assert_eq!(summaries.len(), expected_cpu.len());
            for row in summaries {
                let expected = expected_cpu.get(&row.name).unwrap();
                assert_eq!(row.call_count, expected.1);
                assert!(
                    (row.total_ms - expected.0).abs() <= 1e-6 * expected.0.max(1.0),
                    "{} total",
                    row.name
                );
                assert!(
                    (row.max_ms - expected.2).abs() <= 1e-6 * expected.2.max(1.0),
                    "{} max",
                    row.name
                );
            }
        }
        assert_eq!(frame.gc_alloc_bytes, reference.gc_alloc_bytes_total);
        let imported = imported_dump
            .frames
            .iter()
            .find(|f| f.index == frame.index)
            .unwrap();
        assert!(imported.quality.cpu && imported.quality.gc);
        assert_eq!(frame.cpu_ms as f32, imported.cpu_ms as f32);
        assert_eq!(frame.gc_alloc_bytes, imported.gc_alloc_bytes);
        assert_eq!(
            frame
                .gc_alloc_sites
                .iter()
                .map(|s| s.total_bytes)
                .sum::<u64>(),
            reference.gc_alloc_bytes_total
        );
        let mut expected_sites =
            std::collections::BTreeMap::<(String, String), (u64, u64, u64)>::new();
        for (thread_index, thread) in reference.threads.iter().enumerate() {
            let mut parents: Vec<(usize, u32)> = Vec::new();
            for (i, sample) in thread.samples.iter().enumerate() {
                while parents.last().is_some_and(|(_, n)| *n == 0) {
                    parents.pop();
                }
                if sample.marker_name == "GC.Alloc" {
                    let name = parents
                        .iter()
                        .rev()
                        .map(|(j, _)| thread.samples[*j].marker_name.as_str())
                        .find(|name| *name != "GC.Alloc")
                        .unwrap_or("未归因");
                    let row = expected_sites
                        .entry((
                            name.to_owned(),
                            format!("{} #{}", thread.thread_name, thread_index),
                        ))
                        .or_default();
                    row.0 += sample.gc_alloc_bytes;
                    row.1 += 1;
                    row.2 = row.2.max(sample.gc_alloc_bytes);
                }
                if let Some((_, n)) = parents.last_mut() {
                    *n -= 1;
                }
                if sample.children_count > 0 {
                    parents.push((i, sample.children_count));
                }
            }
        }
        let actual_sites: std::collections::BTreeMap<_, _> = frame
            .gc_alloc_sites
            .iter()
            .map(|s| {
                (
                    (s.name.clone(), s.thread.clone()),
                    (s.total_bytes, s.call_count, s.max_bytes),
                )
            })
            .collect();
        assert_eq!(actual_sites.len(), frame.gc_alloc_sites.len());
        assert_eq!(actual_sites, expected_sites);
        let dump_sites: std::collections::BTreeMap<_, _> = imported
            .gc_alloc_sites
            .iter()
            .map(|s| {
                (
                    (s.name.clone(), s.thread.clone()),
                    (s.total_bytes, s.call_count, s.max_bytes),
                )
            })
            .collect();
        assert_eq!(dump_sites.len(), imported.gc_alloc_sites.len());
        assert_eq!(dump_sites, expected_sites);
    }
    println!("decoded={frames}, production={frames}, compared={}, samples={verified_samples}, elapsed={:?}", dump.frames.len(), started.elapsed());
}

#[test]
#[ignore = "requires UNITY_PROFILER_ADDITIONAL_DATA_PATH; structure only, no Editor comparison"]
fn additional_capture_structure_only() {
    let path = std::env::var("UNITY_PROFILER_ADDITIONAL_DATA_PATH")
        .expect("set UNITY_PROFILER_ADDITIONAL_DATA_PATH");
    let mut file = BufReader::new(File::open(path).expect("open additional recording"));
    let mut decoder = Decoder::default();
    let started = std::time::Instant::now();
    let mut frames = 0;
    let mut samples = 0;
    loop {
        let mut bytes = [0; 28];
        file.read_exact(&mut bytes[..4]).expect("block/end marker");
        if u32::from_le_bytes(bytes[..4].try_into().unwrap()) == 0xDEADFEED {
            break;
        }
        file.read_exact(&mut bytes[4..]).unwrap();
        let header = read_block_header(&bytes).unwrap();
        header.validate().unwrap();
        assert_eq!(header.unity_version_string(), "6000.3.23f1");
        let mut body = vec![0; header.body_size as usize];
        file.read_exact(&mut body).unwrap();
        let decoded = decoder
            .decode(&body)
            .unwrap_or_else(|e| panic!("block[{frames}]: {e}"));
        samples += decoded
            .threads
            .iter()
            .map(|t| t.samples.len())
            .sum::<usize>();
        frames += 1;
    }
    assert_eq!(file.read(&mut [0]).unwrap(), 0);
    assert_eq!(frames, 2000);
    println!("additional capture: structured frames={frames}, samples={samples}, elapsed={:?}; no metric ground truth", started.elapsed());
}
