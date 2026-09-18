//! 把扁平的 DiskSample 数组构建成 Unity-style 层级树
//!
//! 复刻 `librashuai/UnityPerfAgent/internal/capture/capture.go::buildForest`
//! 每个节点的 SelfMs = TotalMs - sum(children.TotalMs)；GC.Alloc KB 向父节点上卷。

use std::collections::HashMap;

use super::categories::name as cat_name;
use super::constants::BYTES_PER_KB;
use super::markers::MarkerInfo;
use super::samples::DiskSample;

#[derive(Debug, Clone)]
pub struct SampleNode {
    pub name: String,
    pub category: String,
    pub total_ms: f32,
    pub self_ms: f32,
    pub start_ms: f32,
    pub gc_alloc_kb: f32,
    pub children: Vec<SampleNode>,
}

/// 把 GC.Alloc KB 上卷到父节点。
pub fn roll_up_gc_alloc(nodes: &mut [SampleNode]) -> f32 {
    let mut total = 0.0f32;
    for node in nodes.iter_mut() {
        let child_total = roll_up_gc_alloc(&mut node.children);
        node.gc_alloc_kb += child_total;
        total += node.gc_alloc_kb;
    }
    total
}

/// 把 self_ms 按 category 累加到 FrameCpuBreakdown 各字段。
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuBreakdown {
    pub rendering_ms: f32,
    pub scripts_ms: f32,
    pub physics_ms: f32,
    pub animation_ms: f32,
    pub gc_ms: f32,
    pub vsync_ms: f32,
    pub ui_ms: f32,
    pub others_ms: f32,
}

pub fn accumulate_cpu(nodes: &[SampleNode], out: &mut CpuBreakdown) {
    for n in nodes {
        match n.category.as_str() {
            "Render" => out.rendering_ms += n.self_ms,
            "Scripts" => out.scripts_ms += n.self_ms,
            "Physics" | "Physics2D" => out.physics_ms += n.self_ms,
            "Animation" => out.animation_ms += n.self_ms,
            "GC" => out.gc_ms += n.self_ms,
            "VSync" => out.vsync_ms += n.self_ms,
            "Gui" | "UI Layout" | "UI Render" | "UI Details" => out.ui_ms += n.self_ms,
            _ => {}
        }
        accumulate_cpu(&n.children, out);
    }
}

/// 构建 forest。`samples` 来自主线程采样表。
pub fn build_forest(
    samples: &[DiskSample],
    markers: &HashMap<u32, MarkerInfo>,
    frame_start_ns: u64,
) -> Vec<SampleNode> {
    let mut next = 0usize;
    let mut roots = Vec::new();
    while next < samples.len() {
        match build_node(samples, &mut next, markers, frame_start_ns) {
            Ok(n) => roots.push(n),
            Err(_) => break,
        }
    }
    roots
}

fn build_node(
    samples: &[DiskSample],
    next: &mut usize,
    markers: &HashMap<u32, MarkerInfo>,
    frame_start_ns: u64,
) -> Result<SampleNode, &'static str> {
    if *next >= samples.len() {
        return Err("EOF in build_node");
    }
    let raw = samples[*next];
    *next += 1;
    if raw.children < 0 || (raw.children as usize) > samples.len().saturating_sub(*next) {
        return Err("invalid children count");
    }
    let info = markers.get(&raw.marker_id);
    let name = info.map(|m| m.name.clone()).unwrap_or_else(|| format!("marker#{}", raw.marker_id));
    let category = info
        .and_then(|m| cat_name(m.category_id))
        .unwrap_or("")
        .to_string();
    let total = raw.total_ns / 1_000_000.0;
    let start = if raw.start_ns >= frame_start_ns {
        (raw.start_ns - frame_start_ns) as f32 / 1_000_000.0
    } else {
        0.0
    };
    let mut node = SampleNode {
        name,
        category,
        total_ms: total,
        self_ms: 0.0,
        start_ms: start,
        gc_alloc_kb: raw.gc_alloc_bytes as f32 / BYTES_PER_KB as f32,
        children: Vec::with_capacity(raw.children as usize),
    };
    let mut child_total = 0.0f32;
    for _ in 0..raw.children {
        let child = build_node(samples, next, markers, frame_start_ns)?;
        child_total += child.total_ms;
        node.children.push(child);
    }
    if node.total_ms > child_total {
        node.self_ms = node.total_ms - child_total;
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, cat: u16) -> MarkerInfo {
        MarkerInfo {
            name: name.to_string(),
            category_id: cat,
        }
    }

    fn disk(marker_id: u32, total_ns: f32, start_ns: u64, children: i32) -> DiskSample {
        DiskSample {
            marker_id,
            total_ns,
            start_ns,
            children,
            gc_alloc_bytes: 0,
        }
    }

    #[test]
    fn builds_hierarchy_with_self_time() {
        let markers: HashMap<u32, MarkerInfo> = [
            (1, m("PlayerLoop", 0)),
            (2, m("Update", 1)),
            (3, m("Render", 0)),
        ]
        .into_iter()
        .collect();

        // root (PlayerLoop, total=20ms) -> [Update (5ms), Render (3ms)]
        // self should be 20 - 8 = 12ms
        let samples = vec![
            disk(1, 20_000_000.0, 0, 2),    // PlayerLoop root
            disk(2, 5_000_000.0, 0, 0),     // Update leaf
            disk(3, 3_000_000.0, 0, 0),     // Render leaf
        ];
        let forest = build_forest(&samples, &markers, 0);
        assert_eq!(forest.len(), 1);
        let root = &forest[0];
        assert_eq!(root.name, "PlayerLoop");
        assert_eq!(root.total_ms, 20.0);
        assert!((root.self_ms - 12.0).abs() < 1e-3);
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn rolls_up_gc_alloc_to_parents() {
        let mut nodes = vec![SampleNode {
            name: "Parent".into(),
            category: "Scripts".into(),
            total_ms: 10.0,
            self_ms: 0.0,
            start_ms: 0.0,
            gc_alloc_kb: 0.0,
            children: vec![SampleNode {
                name: "Child".into(),
                category: "GC".into(),
                total_ms: 1.0,
                self_ms: 1.0,
                start_ms: 0.0,
                gc_alloc_kb: 64.0,
                children: vec![],
            }],
        }];
        let total = roll_up_gc_alloc(&mut nodes);
        assert_eq!(nodes[0].gc_alloc_kb, 64.0);
        assert_eq!(total, 64.0);
    }

    #[test]
    fn accumulates_cpu_by_category() {
        let nodes = vec![
            SampleNode {
                name: "R".into(),
                category: "Render".into(),
                total_ms: 5.0,
                self_ms: 5.0,
                start_ms: 0.0,
                gc_alloc_kb: 0.0,
                children: vec![],
            },
            SampleNode {
                name: "S".into(),
                category: "Scripts".into(),
                total_ms: 3.0,
                self_ms: 3.0,
                start_ms: 0.0,
                gc_alloc_kb: 0.0,
                children: vec![],
            },
            SampleNode {
                name: "Unknown".into(),
                category: "".into(),
                total_ms: 2.0,
                self_ms: 2.0,
                start_ms: 0.0,
                gc_alloc_kb: 0.0,
                children: vec![],
            },
        ];
        let mut b = CpuBreakdown::default();
        accumulate_cpu(&nodes, &mut b);
        assert_eq!(b.rendering_ms, 5.0);
        assert_eq!(b.scripts_ms, 3.0);
        assert_eq!(b.others_ms, 0.0);
    }
}