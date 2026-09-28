//! JSON 格式解析器
//!
//! 目标 Unity 版本：**Unity 6000.3**（Unity 6 系列）。
//! 也兼容 Unity 2022 LTS 导出（尽力解析）。
//!
//! 不同 Unity 版本的 JSON schema 略有差异，本解析器做宽容处理。
//!
//! 已知字段：
//! - `meta` / `header` / `profiler` 文件头（platform, unityVersion, frameCount 等）
//! - `frames` / `samples` 主线程采样数组
//! - 每个 sample 含 `name` / `durationMs` / `callCount`
//! - GC 分配通常在 frame 级别以 `gcAlloc` 字段暴露
//! - Unity 6 新增：`category`（ProfilerCategory 枚举）、`stacktrace`（CallStacks 启用时）
//!
//! 关键 Unity 6 markers（用于诊断）：
//! - `PlayerLoop`：主循环基线
//! - `BehaviourUpdate` / `Update.ScriptRunBehaviourUpdate`：MonoBehaviour.Update
//! - `FixedBehaviourUpdate`：FixedUpdate
//! - `GC.Alloc` / `GC.Collect`：GC 分配与回收
//! - `Camera.Render`：相机渲染
//! - `RenderGraph.Execute` / `RenderGraph.Compile`：Unity 6 SRP RenderGraph
//! - `WaitForTargetFPS` / `Gfx.PresentFrame` / `Gfx.WaitForPresentOnGfxThread`：VSync / GPU 等待
//! - `Physics.FetchResults` / `Physics.Simulate`：物理
//! - `JobHandle.Complete` / `Semaphore.WaitForSignal`：Job System 同步点

use bytes::Bytes;
use serde::Deserialize;

use super::{Frame, ParseError, ParsedProfile, ProfileMeta, Sample};

/// Unity Profiler JSON 导出的根结构（宽松 schema）。
/// 实际 Unity 输出字段远多于这些，本结构只取关心的部分。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonRoot {
    /// 形式 1：`{ "meta": {...}, "frames": [...] }`
    V1 {
        #[serde(default)]
        meta: JsonMeta,
        frames: Vec<JsonFrame>,
    },
    /// 形式 2：`{ "header": {...}, "samples": [...] }`
    V2 {
        #[serde(default)]
        header: JsonMeta,
        samples: Vec<JsonSample>,
    },
    /// 兜底：直接是个数组（每元素是一个 frame）
    Frames(Vec<JsonFrame>),
}

impl Default for JsonRoot {
    fn default() -> Self {
        JsonRoot::Frames(Vec::new())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonMeta {
    #[serde(default)]
    platform: Option<String>,
    #[serde(default, alias = "unityVersion", alias = "unity")]
    unity_version: Option<String>,
    #[serde(default, alias = "frameCount", alias = "frame_count")]
    frame_count: Option<usize>,
    #[serde(
        default,
        alias = "durationMs",
        alias = "duration_ms",
        alias = "duration"
    )]
    duration_ms: Option<f64>,
    #[serde(default, alias = "totalTime", alias = "total_time_ms")]
    total_time_ms: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonFrame {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default, alias = "frameIndex", alias = "frame_index")]
    frame_index: Option<usize>,
    #[serde(default, alias = "durationMs", alias = "duration_ms", alias = "ms")]
    duration_ms: Option<f64>,
    #[serde(default, alias = "cpuMs", alias = "cpu_ms")]
    cpu_ms: Option<f64>,
    #[serde(
        default,
        alias = "gcAlloc",
        alias = "gc_alloc_bytes",
        alias = "gcAllocBytes"
    )]
    gc_alloc_bytes: Option<u64>,
    #[serde(default, alias = "drawCalls", alias = "draw_calls")]
    draw_calls: Option<u32>,
    #[serde(default, alias = "setPassCalls", alias = "set_pass_calls")]
    set_pass_calls: Option<u32>,
    #[serde(default, alias = "mainThreadSamples", alias = "main_thread_samples")]
    main_thread_samples: Vec<JsonSample>,
    #[serde(default, alias = "gcAllocSites", alias = "gc_alloc_sites")]
    gc_alloc_sites: Vec<JsonSample>,
    #[serde(default, alias = "renderEvents", alias = "render_events")]
    render_events: Vec<JsonSample>,
    /// 兼容：frame 直接内联一个 samples 数组
    #[serde(default)]
    samples: Vec<JsonSample>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonSample {
    #[serde(default, alias = "name", alias = "functionName", alias = "markerName")]
    name: Option<String>,
    #[serde(
        default,
        alias = "durationMs",
        alias = "duration_ms",
        alias = "ms",
        alias = "totalMs"
    )]
    total_ms: Option<f64>,
    #[serde(default, alias = "callCount", alias = "call_count")]
    call_count: Option<u64>,
    #[serde(default, alias = "maxMs", alias = "max_ms")]
    max_ms: Option<f64>,
    /// Unity 6 新增：ProfilerCategory（解析但不透传，仅用于元数据校验）
    #[serde(default, alias = "category", alias = "categoryName")]
    #[allow(dead_code)]
    category: Option<String>,
    /// Unity 6 新增：marker metadata（GC.Alloc 用 Int64 表示分配字节数）
    #[serde(default, alias = "meta", alias = "metadata", alias = "payload")]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    total_bytes: Option<u64>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[allow(dead_code)] // 公开 API：未来 extractor 会按 marker 类别给 Agent 出建议
fn sample_category(name: &str) -> &'static str {
    // 归类 Unity 6 marker 到诊断类别，方便 Agent 按类别给出建议
    match name {
        "GC.Alloc" | "GC.Collect" | "GC.Alloc.Sample" | "UnsafeUtility.Malloc" => "Memory",
        "PlayerLoop"
        | "BehaviourUpdate"
        | "Update.ScriptRunBehaviourUpdate"
        | "FixedBehaviourUpdate"
        | "PreLateUpdate.ScriptRunBehaviourLateUpdate"
        | "CoroutinesDelayedCalls" => "Scripting",
        "Camera.Render"
        | "Render.Mesh"
        | "Render.OpaqueGeometry"
        | "Render.TransparentGeometry"
        | "Render.UI" => "Rendering",
        "Gfx.WaitForPresentOnGfxThread"
        | "Gfx.WaitForRenderThread"
        | "Gfx.PresentFrame"
        | "Gfx.WaitForCommands"
        | "Gfx.ProcessCommands"
        | "WaitForTargetFPS" => "GfxWait",
        "RenderGraph.Execute"
        | "RenderGraph.Compile"
        | "RenderGraph.Prepare"
        | "RenderGraph.Dispatch"
        | "RenderGraph.Reset" => "RenderGraph",
        "Physics.FetchResults"
        | "Physics.Processing"
        | "Physics.Simulate"
        | "Physics.UpdateBodies"
        | "Physics.SimulateCloth" => "Physics",
        "JobHandle.Complete" | "Semaphore.WaitForSignal" | "WaitForJobGroupID" => "Jobs",
        "EditorLoop" | "Profiler.CollectEditorStats" | "Profiler.CollectGlobalStats" => "Editor",
        _ => "Other",
    }
}

pub async fn parse(
    bytes: &Bytes,
    file_name: &str,
    file_size_bytes: u64,
) -> Result<ParsedProfile, ParseError> {
    if super::dump::is_dump(bytes)? {
        return super::dump::parse(bytes, file_name, file_size_bytes);
    }
    let root: JsonRoot = serde_json::from_slice(bytes)?;
    let mut warnings = Vec::new();

    let mut profile = match root {
        JsonRoot::V1 { meta, frames } => {
            build_from_frames_v1(meta, frames, file_name, file_size_bytes, &mut warnings)
        }
        JsonRoot::V2 { header, samples } => {
            build_from_samples_v2(header, samples, file_name, file_size_bytes, &mut warnings)
        }
        JsonRoot::Frames(frames) => {
            let meta = JsonMeta::default();
            build_from_frames_v1(meta, frames, file_name, file_size_bytes, &mut warnings)
        }
    };

    for f in &profile.frames {
        if f.cpu_ms < 0.0
            || f.duration_ms < 0.0
            || f.main_thread_samples
                .iter()
                .chain(&f.render_events)
                .any(|s| s.total_ms < 0.0 || s.max_ms < 0.0)
        {
            return Err(ParseError::Other(format!(
                "frame {}: 时间字段不能为负值",
                f.index
            )));
        }
    }
    if let Some(dur) = estimate_duration_from_frames(&profile.frames) {
        profile.meta.duration_ms = dur;
    }

    if profile.frames.is_empty() && warnings.is_empty() {
        warnings.push("JSON 文件中未解析到任何 frame 数据".to_string());
    }

    profile.warnings.extend(warnings);
    Ok(profile)
}

fn build_from_frames_v1(
    meta: JsonMeta,
    frames: Vec<JsonFrame>,
    file_name: &str,
    file_size_bytes: u64,
    warnings: &mut Vec<String>,
) -> ParsedProfile {
    let parsed_frames: Vec<Frame> = frames
        .into_iter()
        .enumerate()
        .map(|(i, jf)| convert_frame(i, jf, warnings))
        .collect();

    ParsedProfile {
        details: None,
        meta: ProfileMeta {
            file_name: file_name.to_string(),
            format: super::ProfilerFormat::Json,
            duration_ms: meta.duration_ms.unwrap_or(0.0),
            frame_count: meta.frame_count.unwrap_or(parsed_frames.len()),
            platform: meta.platform,
            unity_version: meta.unity_version,
            file_size_bytes,
        },
        frames: parsed_frames,
        warnings: vec![],
    }
}

fn build_from_samples_v2(
    header: JsonMeta,
    samples: Vec<JsonSample>,
    file_name: &str,
    file_size_bytes: u64,
    warnings: &mut Vec<String>,
) -> ParsedProfile {
    // V2 形式没有显式 frame，把所有 sample 当作单帧（用于估算）
    warnings.push("JSON 格式为扁平 sample 数组，按单帧聚合（精度有限）".to_string());

    let frame = Frame {
        quality: super::FrameQuality {
            duration: header.duration_ms.or(header.total_time_ms).is_some(),
            cpu: !samples.is_empty() && samples.iter().all(|s| s.total_ms.is_some()),
            samples: !samples.is_empty() && samples.iter().all(|s| s.total_ms.is_some()),
            estimated: true,
            ..super::FrameQuality::missing("json-v2")
        },
        index: 0,
        duration_ms: header.duration_ms.or(header.total_time_ms).unwrap_or(0.0),
        cpu_ms: samples
            .iter()
            .filter_map(|s| s.total_ms)
            .sum::<f64>()
            .max(0.0),
        gc_alloc_bytes: 0,
        draw_calls: 0,
        set_pass_calls: 0,
        main_thread_samples: samples
            .iter()
            .map(|s| convert_sample(s, "sample"))
            .collect(),
        gc_alloc_sites: vec![],
        render_events: vec![],
    };

    ParsedProfile {
        details: None,
        meta: ProfileMeta {
            file_name: file_name.to_string(),
            format: super::ProfilerFormat::Json,
            duration_ms: header.duration_ms.or(header.total_time_ms).unwrap_or(0.0),
            frame_count: header.frame_count.unwrap_or(1),
            platform: header.platform,
            unity_version: header.unity_version,
            file_size_bytes,
        },
        frames: vec![frame],
        warnings: vec![],
    }
}

fn convert_frame(index: usize, jf: JsonFrame, _warnings: &mut Vec<String>) -> Frame {
    let idx = jf.index.or(jf.frame_index).unwrap_or(index);
    let main: Vec<Sample> = if jf.main_thread_samples.is_empty() {
        jf.samples
            .iter()
            .map(|s| convert_sample(s, "sample"))
            .collect()
    } else {
        jf.main_thread_samples
            .iter()
            .map(|s| convert_sample(s, "sample"))
            .collect()
    };

    if jf.main_thread_samples.is_empty() && !jf.samples.is_empty() {
        // 兼容：sample 在 frame 根级时复用为 main_thread_samples
    }

    Frame {
        quality: super::FrameQuality {
            duration: jf.duration_ms.is_some(),
            cpu: jf.cpu_ms.is_some(),
            gc: jf.gc_alloc_bytes.is_some(),
            draw: jf.draw_calls.is_some(),
            set_pass: jf.set_pass_calls.is_some(),
            samples: !main.is_empty()
                && (if jf.main_thread_samples.is_empty() {
                    &jf.samples
                } else {
                    &jf.main_thread_samples
                })
                .iter()
                .all(|s| s.total_ms.is_some()),
            sites: !jf.gc_alloc_sites.is_empty()
                && jf.gc_alloc_sites.iter().all(|s| {
                    s.total_bytes.is_some()
                        || s.total_ms.is_some_and(|v| v >= 0.0)
                        || extract_alloc_bytes(s.metadata.as_ref()).is_some_and(|v| v >= 0.0)
                }),
            render: !jf.render_events.is_empty()
                && jf.render_events.iter().all(|s| s.total_ms.is_some()),
            ..super::FrameQuality::missing("json-v1")
        },
        index: idx,
        duration_ms: jf.duration_ms.unwrap_or(0.0),
        cpu_ms: jf.cpu_ms.unwrap_or(0.0),
        gc_alloc_bytes: jf.gc_alloc_bytes.unwrap_or(0),
        draw_calls: jf.draw_calls.unwrap_or(0),
        set_pass_calls: jf.set_pass_calls.unwrap_or(0),
        main_thread_samples: main,
        gc_alloc_sites: jf
            .gc_alloc_sites
            .iter()
            .map(|s| {
                let bytes = s.total_bytes.unwrap_or_else(|| {
                    extract_alloc_bytes(s.metadata.as_ref())
                        .or(s.total_ms)
                        .unwrap_or(0.0) as u64
                });
                super::AllocSite {
                    name: s.name.clone().unwrap_or_else(|| "未归因".into()),
                    thread: "未提供".into(),
                    total_bytes: bytes,
                    max_bytes: s
                        .max_bytes
                        .or_else(|| s.max_ms.map(|v| v as u64))
                        .unwrap_or(bytes),
                    call_count: s.call_count.unwrap_or(1),
                }
            })
            .collect(),
        render_events: jf
            .render_events
            .iter()
            .map(|s| convert_sample(s, "render"))
            .collect(),
    }
}

fn convert_sample(js: &JsonSample, fallback_name: &str) -> Sample {
    // CPU / rendering 样本始终使用耗时单位；字节只在 AllocSite 转换中读取。
    let total = js.total_ms.unwrap_or(0.0);

    Sample {
        name: js.name.clone().unwrap_or_else(|| fallback_name.to_string()),
        total_ms: total,
        call_count: js.call_count.unwrap_or(1),
        max_ms: js.max_ms.unwrap_or_else(|| js.total_ms.unwrap_or(0.0)),
    }
}

fn extract_alloc_bytes(metadata: Option<&serde_json::Value>) -> Option<f64> {
    let m = metadata?;
    // Unity 6 GC.Alloc metadata 可能是数组（Int32 instanceID, UInt16[] name, UInt32 category, Int64 bytes）
    // 或对象 { bytes: number } / { size: number }
    if let Some(arr) = m.as_array() {
        // 取最后一个 Int64 字段作为字节数
        for v in arr.iter().rev() {
            if let Some(n) = v.as_f64() {
                if n > 0.0 && n < 1e12 {
                    return Some(n);
                }
            }
        }
    }
    if let Some(obj) = m.as_object() {
        if let Some(n) = obj.get("bytes").and_then(|v| v.as_f64()) {
            return Some(n);
        }
        if let Some(n) = obj.get("size").and_then(|v| v.as_f64()) {
            return Some(n);
        }
    }
    None
}

fn estimate_duration_from_frames(frames: &[Frame]) -> Option<f64> {
    let sum: f64 = frames.iter().map(|f| f.duration_ms).sum();
    if sum > 0.0 {
        Some(sum)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_v1_format() {
        let json = r#"{
            "meta": { "platform": "Android", "unityVersion": "2022.3.47f1", "frameCount": 2 },
            "frames": [
                {
                    "index": 0,
                    "durationMs": 16.7,
                    "gcAllocBytes": 1024,
                    "drawCalls": 100,
                    "mainThreadSamples": [
                        { "name": "Update", "totalMs": 8.0, "callCount": 1, "maxMs": 8.0 }
                    ]
                },
                {
                    "index": 1,
                    "durationMs": 17.1,
                    "gcAllocBytes": 2048,
                    "drawCalls": 110,
                    "mainThreadSamples": [
                        { "name": "Update", "totalMs": 9.5, "callCount": 1, "maxMs": 9.5 }
                    ]
                }
            ]
        }"#;
        let bytes = Bytes::from(json);
        let profile = parse(&bytes, "test.json", json.len() as u64).await.unwrap();

        assert_eq!(profile.frames.len(), 2);
        assert_eq!(profile.frames[0].draw_calls, 100);
        assert_eq!(profile.frames[1].gc_alloc_bytes, 2048);
        assert_eq!(profile.meta.platform, Some("Android".to_string()));
        assert_eq!(profile.meta.unity_version, Some("2022.3.47f1".to_string()));
    }

    #[tokio::test]
    async fn parses_flat_array() {
        let json = r#"[
            { "index": 0, "durationMs": 16.0, "mainThreadSamples": [] },
            { "index": 1, "durationMs": 17.0, "mainThreadSamples": [] }
        ]"#;
        let bytes = Bytes::from(json);
        let profile = parse(&bytes, "flat.json", json.len() as u64).await.unwrap();
        assert_eq!(profile.frames.len(), 2);
    }

    #[tokio::test]
    async fn warns_on_empty() {
        let json = r#"{ "meta": {}, "frames": [] }"#;
        let bytes = Bytes::from(json);
        let profile = parse(&bytes, "empty.json", json.len() as u64)
            .await
            .unwrap();
        assert!(profile.warnings.iter().any(|w| w.contains("未解析到")));
    }

    #[tokio::test]
    async fn parses_unity_6_format() {
        // Unity 6000.3 导出：含 GC.Alloc metadata + RenderGraph markers
        let json = r#"{
            "meta": {
                "platform": "WindowsPlayer",
                "unityVersion": "6000.3.0f1",
                "frameCount": 2
            },
            "frames": [
                {
                    "index": 0,
                    "durationMs": 16.7,
                    "gcAllocBytes": 0,
                    "drawCalls": 220,
                    "setPassCalls": 35,
                    "mainThreadSamples": [
                        { "name": "PlayerLoop", "totalMs": 16.5, "callCount": 1 },
                        { "name": "BehaviourUpdate", "totalMs": 4.2, "callCount": 24 },
                        { "name": "GC.Alloc", "totalMs": 0.0, "callCount": 8, "metadata": [0, [], 6, 4096] },
                        { "name": "RenderGraph.Execute", "totalMs": 3.1, "callCount": 1 }
                    ]
                }
            ]
        }"#;
        let bytes = Bytes::from(json);
        let profile = parse(&bytes, "unity6.json", json.len() as u64)
            .await
            .unwrap();
        assert_eq!(profile.meta.unity_version.as_deref(), Some("6000.3.0f1"));
        assert_eq!(profile.frames.len(), 1);

        // CPU 样本耗时不应被 GC 字节 metadata 覆盖
        let gc_alloc = profile.frames[0]
            .main_thread_samples
            .iter()
            .find(|s| s.name == "GC.Alloc")
            .unwrap();
        assert_eq!(gc_alloc.total_ms, 0.0);

        // RenderGraph markers 应被识别
        let rg = profile.frames[0]
            .main_thread_samples
            .iter()
            .find(|s| s.name == "RenderGraph.Execute")
            .unwrap();
        assert!((rg.total_ms - 3.1).abs() < 0.01);
    }

    #[test]
    fn sample_category_classification() {
        assert_eq!(sample_category("GC.Alloc"), "Memory");
        assert_eq!(sample_category("BehaviourUpdate"), "Scripting");
        assert_eq!(sample_category("Camera.Render"), "Rendering");
        assert_eq!(sample_category("RenderGraph.Execute"), "RenderGraph");
        assert_eq!(sample_category("Gfx.WaitForPresentOnGfxThread"), "GfxWait");
        assert_eq!(sample_category("Physics.Simulate"), "Physics");
        assert_eq!(sample_category("JobHandle.Complete"), "Jobs");
        assert_eq!(sample_category("SomeRandom.Marker"), "Other");
    }
}
/// File IO and decoding run on the caller's blocking worker.
pub fn parse_path(path: &std::path::Path) -> Result<ParsedProfile, ParseError> {
    let bytes = Bytes::from(std::fs::read(path)?);
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("unknown.json");
    futures::executor::block_on(parse(&bytes, name, bytes.len() as u64))
}
