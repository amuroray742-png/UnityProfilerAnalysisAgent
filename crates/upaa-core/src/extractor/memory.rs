//! Byte counters are observations, not a leak diagnosis. Integers stay strings.
use crate::parser::{
    data::unity6_structured::{MemoryObservation, MEMORY_COUNTER_NAMES},
    Frame,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub counters: BTreeMap<String, MemorySummary>,
    pub frames: Vec<MemoryFrame>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummary {
    pub peak: Option<String>,
    pub peak_frame: Option<usize>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub delta: Option<String>,
    pub valid_frames: usize,
    pub total_frames: usize,
    pub status: String,
    pub validation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFrame {
    pub frame_index: usize,
    pub counters: BTreeMap<String, MemoryObservation>,
    pub version_verified: Option<bool>,
}
pub fn extract(frames: &[Frame]) -> MemoryMetrics {
    let mut counters = BTreeMap::new();
    for name in MEMORY_COUNTER_NAMES {
        let values: Vec<_> = frames
            .iter()
            .filter_map(|f| {
                f.memory
                    .get(name)
                    .and_then(|v| v.value.as_ref())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|v| (f.index, v))
            })
            .collect();
        let peak = values.iter().max_by_key(|(_, v)| *v);
        let first = values.first();
        let last = values.last();
        counters.insert(
            name.into(),
            MemorySummary {
                peak: peak.map(|(_, v)| v.to_string()),
                peak_frame: peak.map(|(i, _)| *i),
                first: first.map(|(_, v)| v.to_string()),
                last: last.map(|(_, v)| v.to_string()),
                delta: first
                    .zip(last)
                    .map(|(a, b)| (b.1 as i128 - a.1 as i128).to_string()),
                valid_frames: values.len(),
                total_frames: frames.len(),
                status: if values.is_empty() {
                    "unavailable"
                } else if values.len() < frames.len() {
                    "partial"
                } else {
                    "unverified"
                }
                .into(),
                validation: "pending-editor-comparison".into(),
            },
        );
    }
    MemoryMetrics {
        counters,
        frames: frames
            .iter()
            .map(|f| MemoryFrame {
                frame_index: f.index,
                counters: f.memory.clone(),
                version_verified: f.quality.version_verified,
            })
            .collect(),
    }
}
impl MemoryMetrics {
    pub fn page(
        &self,
        name: &str,
        start: usize,
        limit: usize,
    ) -> Result<serde_json::Value, crate::parser::detail::QueryError> {
        use crate::parser::detail::QueryError;
        if !MEMORY_COUNTER_NAMES.contains(&name)
            || limit == 0
            || limit > 50
            || start > self.frames.len()
        {
            return Err(QueryError::BadArg(
                "未知内存 Counter、分页起点或 limit（1..50）无效".into(),
            ));
        }
        let end = start.saturating_add(limit).min(self.frames.len());
        let mut rows:Vec<_>=self.frames[start..end].iter().map(|f|serde_json::json!({"frameIndex":f.frame_index,"observation":f.counters.get(name),"versionVerified":f.version_verified})).collect();
        while serde_json::to_vec_pretty(&rows)
            .map_err(|e| QueryError::Read(e.to_string()))?
            .len()
            > 20 * 1024
        {
            rows.pop();
            if rows.is_empty() {
                return Err(QueryError::BadArg("单条内存观测超过分页响应预算".into()));
            }
        }
        let end = start + rows.len();
        Ok(
            serde_json::json!({"name":name,"summary":self.counters.get(name),"rows":rows,"total":self.frames.len(),"nextStart":(end<self.frames.len()).then_some(end),"scope":"bytes 十进制字符串；缺失不是零。变化量只比较首末有效点，不证明内存泄漏；内存指标等待 Editor 数值对照，不能用于确定性达标结论。sources 最多 16 项，完整证据查 frame_evidence。"}),
        )
    }
}
