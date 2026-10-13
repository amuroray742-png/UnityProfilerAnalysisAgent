//! Unity 6 counter ID → 字段名映射（可扩展）
//!
//! Unity 6 的 Memory counter schema 未公开文档；当前只暴露 raw value。
//! 待 UnityCsReference Unity 6 源码核实后再补字段映射。
//!
//! 调用方应使用 [`classify`] 拿到「已知分类」或回退到「RawCounter」。

use super::unity6_memory_counters::RawCounter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterKind {
    /// 总内存占用（占用 / 保留）
    TotalUsed,
    TotalReserved,
    /// GC 堆
    GcUsed,
    GcReserved,
    /// Texture / Mesh / AnimationClip 等资源
    TextureMemory,
    MeshMemory,
    MaterialCount,
    AnimationClipMemory,
    /// 帧内 GC.Alloc
    GcAllocBytes,
    /// 未知 counter，原样保留
    Unknown,
}

pub fn classify(c: &RawCounter) -> (CounterKind, &'static str) {
    // Unity 6 实测 counterId 范围 0x64..（100+）—— 这里用启发式区间。
    // 注意：当前未确认区间含义；初版只识别 0x64（首个 type-4 record）作为 TotalUsed。
    match (c.counter_id, c.value_type) {
        (0x64, 4) => (CounterKind::TotalUsed, "TotalUsed(64)"),
        (id, _) if (0x68..=0x6D).contains(&id) => (CounterKind::GcUsed, "GC_Used(68..6D)"),
        (id, _) if (0x6E..=0x73).contains(&id) => (CounterKind::GcReserved, "GC_Reserved(6E..73)"),
        (id, _) if (0x74..=0x79).contains(&id) => (CounterKind::TextureMemory, "Texture(74..79)"),
        _ => (CounterKind::Unknown, "Unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_total_used() {
        let c = RawCounter {
            counter_id: 0x64,
            value_type: 4,
            value: 4_510_029,
        };
        let (kind, label) = classify(&c);
        assert_eq!(kind, CounterKind::TotalUsed);
        assert!(label.contains("TotalUsed"));
    }

    #[test]
    fn classifies_unknown() {
        let c = RawCounter {
            counter_id: 999,
            value_type: 2,
            value: 42,
        };
        let (kind, _) = classify(&c);
        assert_eq!(kind, CounterKind::Unknown);
    }
}