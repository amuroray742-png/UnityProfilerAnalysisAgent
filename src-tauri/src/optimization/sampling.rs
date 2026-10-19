//! Re-recording observations, deliberately separate from manual task acceptance.
use super::Task;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub fn validate_names(kind: &str, names: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    if names.len() > 3
        || (!names.is_empty() && kind != "marker")
        || names.iter().any(|name| {
            name.trim().is_empty()
                || name.len() > 200
                || name.chars().any(char::is_control)
                || !seen.insert(name)
        })
    {
        return Err(
            "仅 Marker 任务可记录最多 3 个不同的固定采样名称（每个 1..200 字节，无控制字符）"
                .into(),
        );
    }
    Ok(())
}

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
