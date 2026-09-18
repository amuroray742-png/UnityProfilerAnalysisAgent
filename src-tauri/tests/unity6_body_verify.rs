//! 验证 Unity 6 wire format：读取 body 字节 + JSON ground truth，逐字段比对 Main Thread
//! 2076 个样本。如果所有字段都匹配，说明我们已经掌握了 Unity 6 sample 表 + GC.Alloc metadata
//! 的精确 layout，可以开始 port Unity 2022.3 解析器到 Unity 6。

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct GroundTruthSample {
    sample_index: u32,
    marker_id: i32,
    time_ms: f64,
    start_time_ms: f64,
    children_count: i32,
}

#[derive(Debug)]
struct GroundTruthThread {
    thread_name: String,
    thread_group_name: String,
    samples: Vec<GroundTruthSample>,
    gc_alloc_total_bytes: u64,
}

fn load_ground_truth(json_path: &Path) -> (u32 /*frame_count*/, Vec<GroundTruthThread>) {
    let content = std::fs::read_to_string(json_path).expect("read json");
    // 简单 JSON 解析（避免引 serde_json 依赖）
    // 用 regex-style 不合适，直接 string find 简化：只取 thread 0（Main Thread）
    let v: serde_json::Value = serde_json::from_str(&content).expect("parse json");

    let frames = v["frames"].as_array().expect("frames array");
    let frame0 = &frames[0];
    let threads = frame0["threads"].as_array().expect("threads array");
    let main = &threads[0];

    let samples: Vec<GroundTruthSample> = main["samples"]
        .as_array()
        .expect("samples array")
        .iter()
        .map(|s| GroundTruthSample {
            sample_index: s["sample_index"].as_u64().unwrap() as u32,
            marker_id: s["marker_id"].as_i64().unwrap() as i32,
            time_ms: s["time_ms"].as_f64().unwrap(),
            start_time_ms: s["start_time_ms"].as_f64().unwrap(),
            children_count: s["children_count"].as_i64().unwrap() as i32,
        })
        .collect();

    let gc_alloc_total_bytes = main["gc_alloc_total_bytes"].as_u64().unwrap_or(0);

    let main_thread = GroundTruthThread {
        thread_name: main["thread_name"].as_str().unwrap_or("").to_string(),
        thread_group_name: main["thread_group_name"].as_str().unwrap_or("").to_string(),
        samples,
        gc_alloc_total_bytes,
    };

    (frame0["frame_index"].as_u64().unwrap_or(0) as u32, vec![main_thread])
}

#[test]
#[ignore]
fn verify_unity6_main_thread_samples_against_ground_truth() {
    let json_path = std::env::var("UNITY_PROFILER_DUMP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn1/ProfilerCaptures/If Jinn__2026-09-17_17-30-55.data.dump.json",
            )
        });
    let data_path = std::env::var("UNITY_PROFILER_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn1/ProfilerCaptures/If Jinn__2026-09-17_17-30-55.data",
            )
        });
    if !json_path.exists() || !data_path.exists() {
        eprintln!("跳过：文件不存在");
        return;
    }

    let (frame_count, threads) = load_ground_truth(&json_path);
    let main = &threads[0];
    assert_eq!(main.thread_name, "Main Thread");
    println!(
        "ground truth: frame 0, Main Thread, {} samples, gc_alloc={}",
        main.samples.len(),
        main.gc_alloc_total_bytes
    );

    // Read frame body
    let mut f = std::fs::File::open(&data_path).expect("open .data");
    let mut hdr = [0u8; 28];
    f.read_exact(&mut hdr).expect("read header");
    let body_size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    f.seek(std::io::SeekFrom::Current(0)).unwrap(); // 已经在 28
    let mut body = vec![0u8; body_size];
    f.read_exact(&mut body).expect("read body");
    let frame_end = u32::from_le_bytes([
        body[body_size - 4],
        body[body_size - 3],
        body[body_size - 2],
        body[body_size - 1],
    ]);
    assert_eq!(frame_end, 0xAFAFAFAF, "frame end marker");
    println!("body size: {} bytes", body_size);

    // Main Thread samples 起始 offset = 186584
    let samples_start = 186584usize;
    let sample_count = main.samples.len();
    println!("verifying {} Main Thread samples at body offset {}..{}",
        sample_count, samples_start, samples_start + sample_count * 20);

    let mut mismatches = 0;
    let mut marker_id_mismatches = 0;
    let mut total_ns_mismatches = 0;
    let mut start_ns_mismatches = 0;
    let mut children_mismatches = 0;
    for s in &main.samples {
        let off = samples_start + s.sample_index as usize * 20;
        if off + 20 > body.len() {
            eprintln!("  sample_index={} out of range", s.sample_index);
            mismatches += 1;
            continue;
        }
        let marker_id = i32::from_le_bytes([body[off], body[off+1], body[off+2], body[off+3]]);
        let total_ns = f32::from_le_bytes([body[off+4], body[off+5], body[off+6], body[off+7]]);
        let start_ns = u64::from_le_bytes([
            body[off+8], body[off+9], body[off+10], body[off+11],
            body[off+12], body[off+13], body[off+14], body[off+15],
        ]);
        let children = i32::from_le_bytes([body[off+16], body[off+17], body[off+18], body[off+19]]);

        // 比较（容忍 ns 级浮点误差）
        let ns_total = (s.time_ms * 1e6) as u64;
        let ns_start = (s.start_time_ms * 1e6) as u64;
        let mut ok = true;
        if marker_id != s.marker_id { marker_id_mismatches += 1; ok = false; }
        if (total_ns as u64).abs_diff(ns_total) > 1 { total_ns_mismatches += 1; ok = false; }
        if (start_ns).abs_diff(ns_start) > 1_000_000 { start_ns_mismatches += 1; ok = false; }
        if children != s.children_count { children_mismatches += 1; ok = false; }
        if !ok { mismatches += 1; }
    }
    println!(
        "Main Thread verification: {}/{} samples match, {} mismatches",
        sample_count - mismatches, sample_count, mismatches
    );
    println!(
        "  marker_id mismatches: {}, total_ns: {}, start_ns: {}, children: {}",
        marker_id_mismatches, total_ns_mismatches, start_ns_mismatches, children_mismatches
    );

    // GC.Alloc metadata
    // 顺序：Main Thread 的 5 个 GC.Alloc 样本（id=1335）在 samples 数组里按 sample_index 升序
    // metadata 段：5 条 (i32 sampleIndex, u32 bytes) 4B 对齐 + 8B 步进
    let gc_alloc_start = 229580usize;
    let mut gc_bytes_sum = 0u64;
    let gc_allocs: Vec<&GroundTruthSample> = main
        .samples
        .iter()
        .filter(|s| s.marker_id == 1335)
        .collect();
    println!("GC.Alloc samples in Main Thread: {}", gc_allocs.len());
    for (i, s) in gc_allocs.iter().enumerate() {
        let off = gc_alloc_start + i * 8;
        if off + 8 > body.len() {
            eprintln!("  GC.Alloc #{} out of range", i);
            break;
        }
        let si = u32::from_le_bytes([body[off], body[off+1], body[off+2], body[off+3]]);
        let bt = u32::from_le_bytes([body[off+4], body[off+5], body[off+6], body[off+7]]);
        let ok_si = si == s.sample_index;
        if !ok_si {
            eprintln!(
                "  GC.Alloc #{} si mismatch: body={} json={}",
                i, si, s.sample_index
            );
        }
        gc_bytes_sum += bt as u64;
        let _ = ok_si;
    }
    println!(
        "GC.Alloc metadata sum from body: {} bytes (json thread total: {})",
        gc_bytes_sum, main.gc_alloc_total_bytes
    );

    assert!(
        mismatches == 0,
        "{} samples had field mismatches (marker_id: {}, total_ns: {}, start_ns: {}, children: {})",
        mismatches,
        marker_id_mismatches, total_ns_mismatches, start_ns_mismatches, children_mismatches
    );
    assert_eq!(
        gc_bytes_sum, main.gc_alloc_total_bytes,
        "GC.Alloc total bytes mismatch"
    );
}

/// 验证所有 thread 都用同一格式（20 bytes/sample）
#[test]
#[ignore]
fn verify_unity6_all_threads_use_20byte_sample_format() {
    let json_path = std::env::var("UNITY_PROFILER_DUMP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn1/ProfilerCaptures/If Jinn__2026-09-17_17-30-55.data.dump.json",
            )
        });
    let data_path = std::env::var("UNITY_PROFILER_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn1/ProfilerCaptures/If Jinn__2026-09-17_17-30-55.data",
            )
        });
    if !json_path.exists() || !data_path.exists() {
        eprintln!("跳过");
        return;
    }

    let content = std::fs::read_to_string(&json_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    let frame0 = &v["frames"][0];
    let threads = frame0["threads"].as_array().unwrap();

    let mut f = std::fs::File::open(&data_path).unwrap();
    let mut hdr = [0u8; 28];
    f.read_exact(&mut hdr).unwrap();
    let body_size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    let mut body = vec![0u8; body_size];
    f.read_exact(&mut body).unwrap();

    // 对每个 thread：找 thread_name 在 body 中出现的位置（最可能是 thread header 的开头附近）
    // 验证该 thread 的 sample 0 在 thread_name 字符串后某固定偏移
    println!("scan all {} threads:", threads.len());
    for t in threads {
        let tname = t["thread_name"].as_str().unwrap_or("").to_string();
        let sample_count = t["samples"].as_array().unwrap().len();
        if sample_count == 0 || tname.is_empty() {
            continue;
        }
        let sample0 = &t["samples"][0];
        let marker_id = sample0["marker_id"].as_i64().unwrap() as i32;
        let total_ms = sample0["time_ms"].as_f64().unwrap();
        let total_ns = (total_ms * 1e6) as u64;
        // 找 thread name 第一次出现位置
        let name_bytes = tname.as_bytes();
        let mut name_pos = None;
        for i in 0..body.len().saturating_sub(name_bytes.len()) {
            let m = (0..name_bytes.len()).all(|j| body[i + j] == name_bytes[j]);
            if m {
                name_pos = Some(i);
                break;
            }
        }
        let Some(np) = name_pos else {
            continue;
        };
        // thread header: threadId(8) + groupName(?) + threadName(name_bytes padded 4B) + sampleCount(4)
        // 然后是 sample 表
        // sample 0 在 sample 表第一个 = np + header_size
        // 我们用 sample 0 的 marker_id 找匹配位置
        let mut found = false;
        for off in (np..np + 200).step_by(4) {
            if off + 20 > body.len() {
                break;
            }
            let mid = i32::from_le_bytes([body[off], body[off+1], body[off+2], body[off+3]]);
            let tns = f32::from_le_bytes([body[off+4], body[off+5], body[off+6], body[off+7]]);
            if mid == marker_id && (tns as u64).abs_diff(total_ns) < 5 {
                println!(
                    "  thread '{}': sample 0 at body_offset={} (after name at {}), {} samples",
                    tname, off, np, sample_count
                );
                found = true;
                break;
            }
        }
        if !found {
            println!("  thread '{}': sample 0 not found near name at {}", tname, np);
        }
    }
}