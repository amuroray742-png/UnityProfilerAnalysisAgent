//! Re-recording observations, deliberately separate from manual task acceptance.
use super::Task;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub use upaa_core::proposal::validate_names;

pub fn observe(tasks: &[Task], paths: &Result<BTreeMap<String, Value>, String>) -> Value {
    let mut rows = Vec::new();
    for task in tasks.iter().filter(|t| t.selected && t.kind == "marker") {
        if task.marker_names.is_empty() {
            rows.push(
                json!({"taskId":task.id,"name":null,"status":"manual","matches":[],
                "reason":"此任务未记录固定名称，请按任务方案在 B 调用树中人工核对"}),
            );
        }
        for name in &task.marker_names {
            match paths {
                Err(reason) => rows.push(json!({"taskId":task.id,"name":name,"status":"unavailable","matches":[],"reason":reason})),
                Ok(paths) => {
                    // Match the leaf, never a parent or a substring. Preserve distinct paths.
                    let mut matches = Vec::new();
                    let mut total = 0;
                    for (key, value) in paths {
                        let Ok((group, role, path)) = serde_json::from_str::<(Option<String>, String, Vec<String>)>(key) else { continue };
                        if path.last() != Some(name) { continue; }
                        total += 1;
                        if matches.len() < 5 {
                            matches.push(json!({"roleAndPath":[group,role,path],"observation":value}));
                        }
                    }
                    // Deep/long names must not make the persistent view unbounded.
                    while serde_json::to_vec(&matches).unwrap().len() > 8 * 1024 {
                        matches.pop();
                    }
                    let truncated = total > matches.len();
                    rows.push(json!({"taskId":task.id,"name":name,"status":if total>0 {"observed"}else{"notObserved"},
                        "matches":matches,"totalPaths":total,"truncated":truncated}));
                }
            }
        }
    }
    json!({"rows":rows,"scope":"仅检查 B 所选范围内已导出的采样，名称相同不证明来自本轮代码。未观察到不等于未执行；出现也不证明调查问题已回答或性能改善。每个名称最多展示 5 条路径，完整证据请查看 B 调用树；GC 缺失不是零。"})
}

impl super::Workspace {
    pub fn accept_sampling(&self, rid: &str) -> Result<(), String> {
        if self.busy.load(std::sync::atomic::Ordering::SeqCst) { return Err("请先结束运行".into()); }
        let mut saved=self.data.lock().unwrap();let mut d=saved.clone();
        super::scenes::round_access(&d,rid,false)?;
        let round=d.rounds.iter_mut().find(|r|r.id==rid).ok_or("轮次不存在")?;
        if !round.marker_only() { return Err("只有实际改动全部属于补点任务且无冲突时，才能跳过性能对比".into()); }
        round.decision="accepted".into();
        round.comparison=None;
        // A previously selected B has not been accepted as evidence by this action.
        round.candidate=None;
        self.save(&d)?;*saved=d;
        Ok(())
    }
}
