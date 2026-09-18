//! End-to-end Unity 6 .data full-file parse sanity check.
//! Reads user's actual 445 MB capture and prints summary.

use std::path::Path;
use std::time::Instant;

use unity_profiler_analysis_agent_lib::parser::data::parse_path;

#[test]
#[ignore]
fn full_parse_user_unity6_capture() {
    let data_path = r"C:\work\unity_projects\ProjectJinn1\ProfilerCaptures\If Jinn__2026-09-17_17-30-55.data";
    if !Path::new(data_path).exists() {
        eprintln!("skip: {} not found", data_path);
        return;
    }

    let t0 = Instant::now();
    let parsed = parse_path(Path::new(data_path)).expect("parse must succeed");
    let elapsed = t0.elapsed();

    let total_frames = parsed.frames.len();
    let total_main_samples: usize = parsed.frames.iter().map(|f| f.main_thread_samples.len()).sum();
    let total_gc_alloc: u64 = parsed.frames.iter().map(|f| f.gc_alloc_bytes).sum();

    println!("===== Unity 6 full-file parse =====");
    println!("file:       {}", data_path);
    println!("elapsed:    {:?}", elapsed);
    println!("unity_ver:  {:?}", parsed.meta.unity_version);
    println!("format:     {:?}", parsed.meta.format);
    println!("frames:     {}", total_frames);
    println!("main_samples (rows per frame sum): {}", total_main_samples);
    println!("gc_alloc_bytes total: {} ({} KB)", total_gc_alloc, total_gc_alloc / 1024);
    if let Some(f0) = parsed.frames.first() {
        println!("---- frame 0 detail ----");
        println!("  duration_ms:    {}", f0.duration_ms);
        println!("  cpu_ms:         {}", f0.cpu_ms);
        println!("  gc_alloc_bytes: {}", f0.gc_alloc_bytes);
        println!("  draw_calls:     {}", f0.draw_calls);
        println!("  set_pass_calls: {}", f0.set_pass_calls);
        println!("  main_thread_samples (rows): {}", f0.main_thread_samples.len());
        for s in &f0.main_thread_samples {
            println!("    [sample] name={} total_ms={:.3} calls={} max_ms={:.3}", s.name, s.total_ms, s.call_count, s.max_ms);
        }
        println!("  gc_alloc_sites: {}", f0.gc_alloc_sites.len());
    }
    println!("==================================");
}