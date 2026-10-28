//! Public synthetic boundary fixtures; real Editor reference is opt-in below.
use bytes::Bytes;
use upaa_core::parser::data::{self, unity6_structured::Decoder};

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

// Frame 0 ends with a real 104 B allocation at t=2000; frame 1 repeats
// its metadata at t=2000 but only indexes the new 17 B allocation.
fn body(frame: i32, continuation: bool) -> Vec<u8> {
    let start = if frame == 0 { 1000u64 } else { 2000u64 };
    let mut b = vec![0; 28 + 136];
    b[..4].copy_from_slice(&frame.to_le_bytes());
    b[4..8].copy_from_slice(&frame.to_le_bytes());
    b[8..16].copy_from_slice(&start.to_le_bytes());
    b[16..20].copy_from_slice(&1u32.to_le_bytes());
    b[24..28].copy_from_slice(&1u32.to_le_bytes());
    word(&mut b, u32::MAX);
    b.resize(b.len() + 64 + 1060 + 32, 0);
    word(&mut b, 1);
    word(&mut b, 718);
    string(&mut b, "GC.Alloc");
    word(&mut b, 17 << 16);
    word(&mut b, 1);
    word(&mut b, 0x203);
    string(&mut b, "Size");
    word(&mut b, 1);
    b.extend(42u64.to_le_bytes());
    string(&mut b, "Worker");
    string(&mut b, "Main Thread");
    word(&mut b, if continuation { 3 } else { 2 });
    for (marker, duration, time, children) in [
        (u32::MAX, 1000f32, start, if continuation { 2 } else { 1 }),
        (
            718,
            if continuation { 0f32 } else { 100f32 },
            if continuation { start } else { 1900 },
            0,
        ),
    ] {
        word(&mut b, marker);
        b.extend(duration.to_le_bytes());
        b.extend(time.to_le_bytes());
        word(&mut b, children);
    }
    if continuation {
        word(&mut b, 718);
        b.extend(0f32.to_le_bytes());
        b.extend(2100u64.to_le_bytes());
        word(&mut b, 0);
    }
    for v in [
        0,
        0,
        1,
        if continuation { 2 } else { 1 },
        if continuation { 17 } else { 104 },
        0,
        if continuation { 2 } else { 1 },
        1,
        1,
        3,
        4,
        104,
    ] {
        word(&mut b, v);
    }
    if continuation {
        for v in [2, 1, 3, 4, 17] {
            word(&mut b, v);
        }
    }
    for v in [0, 0, 0, 0, 0xAFAFAFAF] {
        word(&mut b, v);
    }
    b
}
fn capture(bodies: &[Vec<u8>]) -> Bytes {
    let mut b = vec![];
    for body in bodies {
        for v in [0x20220328, body.len() as u32, 6000, 3, 23, 2, 1] {
            word(&mut b, v);
        }
        b.extend(body);
    }
    word(&mut b, 0xDEADFEED);
    b.into()
}

#[tokio::test]
async fn continuation_preserves_evidence_without_counting_bytes_or_calls_twice() {
    let b = capture(&[body(0, false), body(1, true)]);
    let profile = data::parse(&b, "boundary.data", b.len() as u64)
        .await
        .unwrap();
    assert_eq!(
        profile
            .frames
            .iter()
            .map(|f| f.gc_alloc_bytes)
            .collect::<Vec<_>>(),
        [104, 17]
    );
    assert!(profile
        .frames
        .iter()
        .all(|f| f.quality.gc && f.quality.sites));
    assert_eq!(
        profile.frames[1]
            .gc_alloc_sites
            .iter()
            .map(|s| s.call_count)
            .sum::<u64>(),
        1
    );
    assert_eq!(
        upaa_core::extractor::extract(&profile).gc.total_alloc_bytes,
        Some(121)
    );
    // Query frame 1 first: the on-demand checkpoint must retain previous-frame
    // evidence, rather than relying on the order in which callers query frames.
    let store = profile.details.unwrap();
    for index in [1, 0, 1] {
        let f = store.load(index).unwrap();
        if index == 1 {
            let s = &f.threads[0].samples[1];
            assert!(s.gc_continuation);
            assert_eq!(s.gc_alloc_bytes, None);
            assert_eq!(s.metadata[0].value.as_deref(), Some("104"));
            assert_eq!(s.raw_duration_ns, Some(0.0));
            assert!(!f.threads[0].samples[2].gc_continuation);
            assert_eq!(f.threads[0].samples[2].gc_alloc_bytes, Some(17));
        }
    }
}

#[test]
fn continuation_requires_adjacent_frame_same_thread_marker_time_and_metadata() {
    let first = body(0, false);
    let next = body(1, true);
    let decoded = Decoder::default().decode(&first).unwrap();
    let thread = decoded.thread_section_offset + 4;
    let sample = thread + 8 + 8 + 12 + 4;
    let metadata_payload = next.len() - 44;
    assert_eq!(
        u32::from_le_bytes(
            next[metadata_payload..metadata_payload + 4]
                .try_into()
                .unwrap()
        ),
        104
    );
    let edits = [
        (0, 2),                            // non-adjacent raw frame ID
        (thread, 43),                      // different raw thread identity
        (sample + 20 + 4, 1f32.to_bits()), // nonzero duration
        (sample + 20 + 8, 2001),           // not at frame start
        (metadata_payload, 105),           // metadata differs from previous allocation
        (metadata_payload - 8, 2),         // wrong payload type
    ];
    for (offset, value) in edits {
        let mut b = next.clone();
        b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let mut decoder = Decoder::default();
        decoder.decode(&first).unwrap();
        assert!(decoder.decode(&b).is_err(), "accepted mutation at {offset}");
    }
    assert!(
        Decoder::default().decode(&next).is_err(),
        "no preceding allocation evidence"
    );
    let mut wrong_end = first.clone();
    wrong_end[sample + 20 + 4..sample + 20 + 8].copy_from_slice(&99f32.to_le_bytes());
    let mut decoder = Decoder::default();
    decoder.decode(&wrong_end).unwrap();
    assert!(
        decoder.decode(&next).is_err(),
        "previous allocation does not end at boundary"
    );
    let mut legacy = Decoder::configured(true, false, Default::default());
    legacy.decode(&first).unwrap();
    assert!(
        legacy.decode(&next).is_err(),
        "not validated for Unity 2022"
    );

    // Remove the continuation's entire metadata record without corrupting the
    // surrounding layout: the cross-frame match alone must not be accepted.
    let mut no_metadata = next.clone();
    no_metadata[metadata_payload - 20..metadata_payload - 16].copy_from_slice(&1u32.to_le_bytes());
    no_metadata.drain(metadata_payload - 16..metadata_payload + 4);
    let mut decoder = Decoder::default();
    decoder.decode(&first).unwrap();
    assert!(decoder.decode(&no_metadata).is_err());

    // An ordinary allocation later in the same frame still needs an index.
    let gc_count = sample + 3 * 20 + 8;
    let mut missing_real = next.clone();
    missing_real[gc_count..gc_count + 4].copy_from_slice(&0u32.to_le_bytes());
    missing_real.drain(gc_count + 4..gc_count + 12);
    let mut decoder = Decoder::default();
    decoder.decode(&first).unwrap();
    assert!(decoder.decode(&missing_real).is_err());

    // A successful continuation must not retain a stale allocation for a later
    // frame; its last sample is a new zero-duration allocation, not a boundary.
    let mut decoder = Decoder::default();
    decoder.decode(&first).unwrap();
    decoder.decode(&next).unwrap();
    let mut later = next.clone();
    later[..4].copy_from_slice(&2i32.to_le_bytes());
    assert!(decoder.decode(&later).is_err());
}

#[test]
fn genuine_zero_duration_allocation_at_boundary_still_counts() {
    let first = body(0, false);
    let mut next = body(1, true);
    let decoded = Decoder::default().decode(&first).unwrap();
    let sample = decoded.thread_section_offset + 4 + 8 + 8 + 12 + 4;
    let gc_count = sample + 3 * 20 + 8;
    next[gc_count..gc_count + 4].copy_from_slice(&2u32.to_le_bytes());
    next.splice(
        gc_count + 4..gc_count + 4,
        [1u32, 104].into_iter().flat_map(u32::to_le_bytes),
    );
    let mut decoder = Decoder::default();
    decoder.decode(&first).unwrap();
    let frame = decoder.decode(&next).unwrap();
    assert_eq!(frame.threads[0].gc_bytes, 121);
    assert!(!frame.threads[0].samples[1].gc_continuation);
}

#[test]
fn continuation_follows_thread_identity_when_thread_indices_change() {
    let first = body(0, false);
    let decoded = Decoder::default().decode(&first).unwrap();
    let section = decoded.thread_section_offset;
    let mut two_threads = first.clone();
    two_threads[section..section + 4].copy_from_slice(&2u32.to_le_bytes());
    let mut other = first[section + 4..first.len() - 4].to_vec();
    other[..8].copy_from_slice(&99u64.to_le_bytes());
    // Only thread 99 ends at this boundary. Thread 42 ends one ns earlier.
    let duration = section + 4 + 8 + 8 + 12 + 4 + 20 + 4;
    two_threads[duration..duration + 4].copy_from_slice(&99f32.to_le_bytes());
    two_threads.splice(first.len() - 4..first.len() - 4, other);
    let mut next = body(1, true);
    next[section + 4..section + 12].copy_from_slice(&99u64.to_le_bytes());
    let mut decoder = Decoder::default();
    decoder.decode(&two_threads).unwrap();
    let frame = decoder.decode(&next).unwrap();
    assert!(frame.threads[0].samples[1].gc_continuation);
    assert_eq!(frame.threads[0].id, 99);
    assert_eq!(frame.threads[0].gc_bytes, 17);
}

#[tokio::test]
#[ignore = "requires UNITY_GC_DATA_PATH and independent UNITY_GC_REFERENCE_PATH from Editor"]
async fn editor_reference_full_capture_and_lazy_queries() {
    use serde_json::Value;
    use std::{
        fs::File,
        io::{BufReader, Read},
        path::PathBuf,
    };
    let path = PathBuf::from(std::env::var("UNITY_GC_DATA_PATH").expect("set UNITY_GC_DATA_PATH"));
    let reference_path =
        std::env::var("UNITY_GC_REFERENCE_PATH").expect("set UNITY_GC_REFERENCE_PATH");
    let reference: Value =
        serde_json::from_reader(BufReader::new(File::open(reference_path).unwrap())).unwrap();
    let number = |v: &Value| v.as_str().unwrap().parse::<u64>().unwrap();
    let expected_frames = reference["frames"].as_array().unwrap();
    assert!(!expected_frames.is_empty());
    let profile = upaa_core::parser::parse_file(&path)
        .await
        .expect("full production import");
    assert_eq!(
        profile.meta.unity_version.as_deref(),
        reference["editor_version"].as_str()
    );
    assert_eq!(profile.frames.len(), expected_frames.len());
    let snapshot = upaa_core::extractor::extract(&profile);
    assert_eq!(
        snapshot.gc.total_alloc_bytes,
        Some(number(&reference["gc_bytes"]))
    );
    let coverage = snapshot.meta.parsing.unwrap();
    assert_eq!(coverage.decoded_frames, expected_frames.len());
    assert_eq!((coverage.skipped_frames, coverage.failed_frames), (0, 0));
    for (index, (frame, expected)) in profile.frames.iter().zip(expected_frames).enumerate() {
        assert_eq!(expected["frame_index"].as_u64(), Some(index as u64));
        assert!(
            frame.quality.gc && frame.quality.sites,
            "GC coverage frame {index}"
        );
        assert_eq!(
            frame.gc_alloc_bytes,
            number(&expected["gc_bytes"]),
            "frame {index}"
        );
        assert_eq!(
            frame
                .gc_alloc_sites
                .iter()
                .map(|s| s.total_bytes)
                .sum::<u64>(),
            frame.gc_alloc_bytes
        );
    }
    let mut reader = BufReader::new(File::open(path).unwrap());
    let mut decoder = Decoder::default();
    let mut continuations = vec![];
    let mut metadata_total = 0u64;
    let mut excluded = 0u64;
    let mut thread_count = 0usize;
    for (index, expected) in expected_frames.iter().enumerate() {
        let mut header = [0; 28];
        reader.read_exact(&mut header).unwrap();
        let header = data::header::read_block_header(&header).unwrap();
        let mut bytes = vec![0; header.body_size as usize];
        reader.read_exact(&mut bytes).unwrap();
        let frame = decoder.decode(&bytes).unwrap();
        let threads = expected["threads"].as_array().unwrap();
        assert_eq!(
            frame.threads.len(),
            threads.len(),
            "thread count frame {index}"
        );
        for (ti, (thread, expected)) in frame.threads.iter().zip(threads).enumerate() {
            thread_count += 1;
            assert_eq!(expected["thread_index"].as_u64(), Some(ti as u64));
            let editor_id = number(&expected["thread_id"]);
            assert!(thread.id == editor_id || (thread.id as u32 as i32 as i64 as u64) == editor_id);
            assert_eq!(Some(thread.name.as_str()), expected["name"].as_str());
            assert_eq!(Some(thread.group.as_str()), expected["group"].as_str());
            assert_eq!(
                Some(thread.samples.len() as u64),
                expected["sample_count"].as_u64()
            );
            assert_eq!(
                thread.gc_bytes,
                number(&expected["gc_bytes"]),
                "frame {index} thread {ti}"
            );
            let mut metadata_bytes = 0;
            for (si, sample) in thread
                .samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.name == "GC.Alloc")
            {
                let bytes: u64 = sample.metadata[0]
                    .value
                    .as_deref()
                    .unwrap()
                    .parse()
                    .unwrap();
                metadata_bytes += bytes;
                if sample.gc_continuation {
                    assert_eq!(sample.gc_bytes, None);
                    let boundary = reference["boundaries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|b| {
                            b["frame_index"].as_u64() == Some(index as u64)
                                && b["thread_index"].as_u64() == Some(ti as u64)
                        })
                        .unwrap();
                    let raw = boundary["samples"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|s| s["sample_index"].as_u64() == Some(si as u64))
                        .unwrap();
                    assert_eq!(number(&raw["metadata_bytes"]), bytes);
                    assert_eq!(number(&raw["start_ns"]), sample.start_ns);
                    assert_eq!(number(&raw["duration_ns"]), 0);
                    let previous = reference["boundaries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|b| {
                            b["frame_index"].as_u64() == Some(index as u64 - 1)
                                && number(&b["thread_id"]) == editor_id
                        })
                        .unwrap();
                    let previous_sample = previous["samples"].as_array().unwrap().last().unwrap();
                    assert_eq!(
                        number(&previous_sample["start_ns"])
                            + number(&previous_sample["duration_ns"]),
                        sample.start_ns
                    );
                    assert_eq!(number(&previous_sample["metadata_bytes"]), bytes);
                    let previous_column = previous["columns"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| {
                            c["indices"]
                                .as_array()
                                .unwrap()
                                .contains(&previous_sample["sample_index"])
                        })
                        .unwrap();
                    assert_eq!(number(&previous_column["gc_bytes"]), bytes);
                    let column = boundary["columns"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| {
                            c["indices"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|i| i.as_u64() == Some(si as u64))
                        })
                        .unwrap();
                    assert_eq!(
                        number(&column["gc_bytes"]),
                        0,
                        "Editor must exclude the continuation"
                    );
                    excluded += bytes;
                    continuations.push((index, ti, si, bytes));
                }
            }
            assert_eq!(metadata_bytes, number(&expected["metadata_bytes"]));
            metadata_total += metadata_bytes;
        }
    }
    assert!(
        !continuations.is_empty(),
        "reference must exercise boundary continuations"
    );
    assert_eq!(metadata_total, number(&reference["metadata_bytes"]));
    assert_eq!(
        metadata_total - excluded,
        snapshot.gc.total_alloc_bytes.unwrap()
    );
    let store = profile.details.unwrap();
    for &(index, ti, si, bytes) in continuations.iter().rev() {
        let frame = store.load(index).unwrap();
        let sample = &frame.threads[ti].samples[si];
        assert!(sample.gc_continuation);
        assert_eq!(sample.gc_alloc_bytes, None);
        assert_eq!(
            sample.metadata[0].value.as_deref(),
            Some(bytes.to_string().as_str())
        );
        assert_eq!(
            frame.info.gc_alloc_bytes,
            Some(number(&expected_frames[index]["gc_bytes"]))
        );
    }
    println!("frames={} threads={thread_count} gc={} raw_metadata={metadata_total} excluded={excluded} continuations={continuations:?}", expected_frames.len(), snapshot.gc.total_alloc_bytes.unwrap());
}
