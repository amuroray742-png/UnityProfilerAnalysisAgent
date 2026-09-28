//! Manual production-path benchmark. Each process imports one supplied capture
//! repeatedly, exercises a bounded query, then drops every capture owner.
use std::{path::PathBuf, time::Instant};
use unity_profiler_analysis_agent_lib::{extractor, parser};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let process_start = Instant::now();
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("provide input path")?);
    let repeats: usize = args.next().unwrap_or_else(|| "3".into()).parse()?;
    if !(1..=10).contains(&repeats) {
        return Err("repeats must be 1..=10".into());
    }
    let size = std::fs::metadata(&path)?.len();
    println!(
        "{}",
        serde_json::json!({"phase":"start","inputBytes":size,"repeats":repeats})
    );
    for iteration in 0..repeats {
        let start = Instant::now();
        let profile = parser::parse_file(&path).await?;
        let first_index = profile
            .frames
            .first()
            .ok_or("input has no frames to benchmark")?
            .index;
        let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
        let cpu_rows: usize = profile
            .frames
            .iter()
            .map(|f| f.main_thread_samples.len())
            .sum();
        let gc_rows: usize = profile.frames.iter().map(|f| f.gc_alloc_sites.len()).sum();
        let summary_name_bytes: usize = profile
            .frames
            .iter()
            .map(|f| {
                f.main_thread_samples
                    .iter()
                    .map(|s| s.name.len())
                    .sum::<usize>()
                    + f.gc_alloc_sites
                        .iter()
                        .map(|s| s.name.len() + s.thread.len())
                        .sum::<usize>()
            })
            .sum();
        let storage = profile.details.as_ref().map(|s| s.storage_counts());
        let extract_start = Instant::now();
        let snapshot = extractor::extract(&profile);
        let extract_ms = extract_start.elapsed().as_secs_f64() * 1000.0;
        let query_start = Instant::now();
        let query = profile
            .details
            .as_ref()
            .map(|source| source.hierarchy(first_index, None, 0, 200, 64))
            .transpose()?;
        let query_ms = query_start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{}",
            serde_json::json!({"phase":"loaded","iteration":iteration,"elapsedMs":process_start.elapsed().as_secs_f64()*1000.0,"parseMs":parse_ms,"extractMs":extract_ms,"queryMs":query_ms,
            "frames":snapshot.meta.frame_count,"queriedNodes":query.as_ref().map(|p|p.samples.len()),"cpuSummaryRows":cpu_rows,"gcSummaryRows":gc_rows,"summaryNameBytes":summary_name_bytes,"queryStorage":storage})
        );
        let weak = profile.details.as_ref().map(std::sync::Arc::downgrade);
        drop(query);
        drop(snapshot);
        drop(profile);
        if weak.is_some_and(|p| p.upgrade().is_some()) {
            return Err("capture query source retained after release".into());
        }
        println!(
            "{}",
            serde_json::json!({"phase":"released","iteration":iteration,"elapsedMs":process_start.elapsed().as_secs_f64()*1000.0})
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
}
