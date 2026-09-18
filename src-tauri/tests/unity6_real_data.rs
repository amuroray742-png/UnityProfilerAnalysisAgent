//! 集成测试：解析用户提供的真实 Unity 6000.3.23f1 .data 文件
//!
//! 运行方式：`cargo test --test unity6_real_data -- --ignored --nocapture`
//! 文件默认位置：`C:/work/unity_projects/ProjectJinn/ProfilerCaptures/If Jinn__2026-09-17_14-16-24.data`
//! 可通过环境变量 `UNITY_PROFILER_DATA_PATH` 覆盖。

use std::io::{Read, Seek};
use std::path::PathBuf;

use unity_profiler_analysis_agent_lib::parser::data::parse_path_with_progress;
use unity_profiler_analysis_agent_lib::parser::data::unity6_gc_alloc_scan::{
    longest_run, scan, scan_all_alignments,
};

#[test]
#[ignore]
fn parses_real_unity6_capture() {
    let path: PathBuf = std::env::var("UNITY_PROFILER_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn/ProfilerCaptures/If Jinn__2026-09-17_14-16-24.data",
            )
        });

    if !path.exists() {
        eprintln!("跳过：真实 .data 文件不存在 {:?}", path);
        return;
    }

    let mut last_pct = 0u64;
    let total_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let profile = parse_path_with_progress(&path, &mut |done, total| {
        let pct = if total > 0 { (done * 100) / total } else { 0 };
        if pct > last_pct && pct % 10 == 0 {
            eprintln!("progress: {}% ({} / {} bytes)", pct, done, total);
            last_pct = pct;
        }
    })
    .expect("parse_path_with_progress");

    println!(
        "Unity version: {:?}, frame_count: {}, file_size_bytes: {}",
        profile.meta.unity_version, profile.meta.frame_count, profile.meta.file_size_bytes
    );
    println!("warnings: {}", profile.warnings.len());
    for w in profile.warnings.iter().take(5) {
        println!("  - {}", w);
    }

    if !profile.frames.is_empty() {
        let first = &profile.frames[0];
        println!(
            "frame[0]: cpu_ms={:.3}, main_thread_samples={}, gc_alloc_bytes={}",
            first.cpu_ms,
            first.main_thread_samples.len(),
            first.gc_alloc_bytes
        );
    }

    // 用 sanity checks 验证解析结果合理
    assert_eq!(
        profile.meta.unity_version.as_deref(),
        Some("6000.3.23f1"),
        "expected Unity 6000.3.23f1 capture"
    );
    assert!(
        profile.meta.frame_count >= 1,
        "expected at least one non-synthetic frame, got {}",
        profile.meta.frame_count
    );
    let _ = total_size;
}

#[test]
#[ignore]
fn parses_first_30mb_synth_unity6() {
    // 截取真实文件前 30 MB（含约 20 个 frame），避免 1.1 GB 全量加载
    let path: PathBuf = std::env::var("UNITY_PROFILER_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn/ProfilerCaptures/If Jinn__2026-09-17_14-16-24.data",
            )
        });

    if !path.exists() {
        eprintln!("跳过：真实 .data 文件不存在 {:?}", path);
        return;
    }

    let truncated = std::env::temp_dir().join("upaa_unity6_first_30mb.data");
    {
        let mut src = std::fs::File::open(&path).expect("open src");
        let mut dst = std::fs::File::create(&truncated).expect("create dst");
        let copied = std::io::copy(
            &mut Read::by_ref(&mut src).take(30 * 1024 * 1024),
            &mut dst,
        )
        .expect("copy 30MB");
        // 追加 0xDEADFEED 文件结束标记（截断文件没有它，parser 会拒绝）
        use std::io::Write;
        dst.write_all(&0xDEADFEEDu32.to_le_bytes())
            .expect("write EOF marker");
        println!("copied {} bytes + EOF marker to {:?}", copied, truncated);
    }

    let profile = match parse_path_with_progress(&truncated, &mut |done, total| {
        if done == total {
            eprintln!("完成：{} / {} bytes", done, total);
        }
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse_path 返回 {}（截断文件正常，提前结束）", e);
            std::fs::remove_file(&truncated).ok();
            return;
        }
    };

    println!(
        "Unity version: {:?}, frame_count: {}",
        profile.meta.unity_version, profile.meta.frame_count
    );
    println!("warnings ({}):", profile.warnings.len());
    for w in profile.warnings.iter().take(10) {
        println!("  - {}", w);
    }
    if let Some(f) = profile.frames.first() {
        println!(
            "first frame: cpu_ms={:.3} samples={} gc_bytes={}",
            f.cpu_ms,
            f.main_thread_samples.len(),
            f.gc_alloc_bytes
        );
    }

    std::fs::remove_file(&truncated).ok();
}

/// 在用户真实文件第一个 frame body 上盲扫 GC.Alloc metadata
#[test]
#[ignore]
fn scans_frame1_for_gc_alloc_metadata() {
    use unity_profiler_analysis_agent_lib::parser::data::constants::{
        BLOCK_HEADER_SIZE, FRAME_END_MARKER, UNITY_DATA_MAGIC,
    };

    let path: PathBuf = std::env::var("UNITY_PROFILER_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(
                "C:/work/unity_projects/ProjectJinn/ProfilerCaptures/If Jinn__2026-09-17_14-16-24.data",
            )
        });
    if !path.exists() {
        eprintln!("跳过：真实 .data 文件不存在 {:?}", path);
        return;
    }

    let mut src = std::fs::File::open(&path).expect("open src");
    // 读 28 字节 block header
    let mut header = [0u8; BLOCK_HEADER_SIZE];
    src.read_exact(&mut header).expect("read header");
    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let body_size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    assert_eq!(magic, UNITY_DATA_MAGIC);
    println!("frame 0 body_size: {}", body_size);
    src.seek(std::io::SeekFrom::Start(BLOCK_HEADER_SIZE as u64))
        .expect("rewind");

    // 读 frame body（去掉末尾 4B frame end marker）
    let body_len = body_size as usize;
    let mut body = vec![0u8; body_len];
    src.read_exact(&mut body).expect("read body");
    let end = u32::from_le_bytes([
        body[body_len - 4],
        body[body_len - 3],
        body[body_len - 2],
        body[body_len - 1],
    ]);
    assert_eq!(end, FRAME_END_MARKER);

    // 跑盲扫
    let pairs = scan(&body);
    println!(
        "scan only at body[0] alignment: found {} candidate (i32,u32) pairs",
        pairs.len()
    );
    let run0 = longest_run(&pairs);
    println!(
        "longest contiguous ascending run (alignment=0): len={}, first 5: {:?}",
        run0.len(),
        run0.iter().take(5).collect::<Vec<_>>()
    );

    let all = scan_all_alignments(&body);
    let mut best_alignment = 0usize;
    let mut best_run = 0usize;
    for r in &all {
        if r.pairs.len() > best_run {
            best_run = r.pairs.len();
            best_alignment = r.alignment;
        }
    }
    println!(
        "best alignment: {} (run length {})",
        best_alignment, best_run
    );
    if let Some(best) = all.iter().find(|r| r.alignment == best_alignment) {
        let preview: Vec<_> = best.pairs.iter().take(8).collect();
        println!(
            "best run first 8 pairs: offsets=[{}], indices=[{}], bytes=[{}]",
            preview
                .iter()
                .map(|p| p.body_offset.to_string())
                .collect::<Vec<_>>()
                .join(","),
            preview
                .iter()
                .map(|p| p.sample_index.to_string())
                .collect::<Vec<_>>()
                .join(","),
            preview
                .iter()
                .map(|p| p.alloc_bytes.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        if best.pairs.len() > 8 {
            println!("... ({} more pairs)", best.pairs.len() - 8);
        }
    }

    // 也扫 frame 1 (更小，更典型)
    println!("\n=== frame 1 (body_size = 441,680 bytes) ===");
    // 跳过 frame 0 body + frame 1 header
    src.seek(std::io::SeekFrom::Start(
        (BLOCK_HEADER_SIZE + body_size as usize + BLOCK_HEADER_SIZE) as u64,
    ))
    .expect("seek to frame 1 body");
    let mut body1 = vec![0u8; 441680];
    src.read_exact(&mut body1).expect("read body 1");
    let end1 = u32::from_le_bytes([
        body1[441680 - 4],
        body1[441680 - 3],
        body1[441680 - 2],
        body1[441680 - 1],
    ]);
    assert_eq!(end1, FRAME_END_MARKER);
    let pairs1 = scan(&body1);
    println!("scan[0]: {} candidates", pairs1.len());
    println!("longest run[0]: len={}", longest_run(&pairs1).len());
    let all1 = scan_all_alignments(&body1);
    for r in &all1 {
        if r.pairs.len() >= 3 {
            let preview: Vec<_> = r.pairs.iter().take(6).collect();
            // 计算连续 +1 的连续段长度
            let mut max_consec = 1usize;
            let mut cur_consec = 1usize;
            for w in r.pairs.windows(2) {
                if w[1].body_offset == w[0].body_offset + 8 && w[1].sample_index == w[0].sample_index + 1
                {
                    cur_consec += 1;
                    if cur_consec > max_consec {
                        max_consec = cur_consec;
                    }
                } else {
                    cur_consec = 1;
                }
            }
            println!(
                "alignment={}, len={}, max_consecutive_+1={}, first 6: offsets=[{}], indices=[{}], bytes=[{}]",
                r.alignment,
                r.pairs.len(),
                max_consec,
                preview
                    .iter()
                    .map(|p| p.body_offset.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                preview
                    .iter()
                    .map(|p| p.sample_index.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                preview
                    .iter()
                    .map(|p| p.alloc_bytes.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
}