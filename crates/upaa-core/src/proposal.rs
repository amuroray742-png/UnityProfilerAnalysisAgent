use serde::{Serialize,Deserialize};
use std::{collections::BTreeMap,path::Path,io::Read};
use crate::storage;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub evidence: String,
    pub files: BTreeMap<String, String>,
    pub instructions: String,
    pub acceptance: String,
    pub constraints: String,
    pub selected: bool,
}
pub fn allowed(path: &str) -> bool {
    !path.contains('\\')
        && !path
            .split('/')
            .any(|s| s.starts_with('.') || s.eq_ignore_ascii_case("com.upaa.inspector"))
        && (path.starts_with("Assets/") || path.starts_with("Packages/"))
        && matches!(
            Path::new(path).extension().and_then(|s| s.to_str()),
            Some("cs" | "shader" | "hlsl" | "cginc" | "compute")
        )
}
pub fn read(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    if !allowed(path) {
        return Err("只允许已有工程代码文件，禁止新增/资源/设置/插件".into());
    }
    let mut f = crate::project::files::open(root, path)?;
    let mut b = vec![];
    f.by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > 2 * 1024 * 1024 {
        return Err("单个代码文件超过 2 MiB".into());
    }
    Ok(b)
}
pub fn decode(b: &[u8]) -> Result<String, String> {
    if b.starts_with(&[255, 254]) || b.starts_with(&[254, 255]) {
        if b.len() % 2 != 0 {
            return Err("UTF-16 长度无效".into());
        }
        let le = b[0] == 255;
        String::from_utf16(
            &b[2..]
                .chunks_exact(2)
                .map(|v| {
                    if le {
                        u16::from_le_bytes([v[0], v[1]])
                    } else {
                        u16::from_be_bytes([v[0], v[1]])
                    }
                })
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())
    } else {
        String::from_utf8(b.strip_prefix(&[239, 187, 191]).unwrap_or(b).to_vec())
            .map_err(|e| e.to_string())
    }
}
pub fn validate_tasks(tasks: &[Task], root: &Path) -> Result<(), String> {
    if tasks.is_empty() || tasks.len() > 20 {
        return Err("请选择 1..20 项任务".into());
    }
    let mut ids = std::collections::HashSet::new();
    for t in tasks {
        if !ids.insert(&t.id)
            || !["optimize", "marker"].contains(&t.kind.as_str())
            || t.title.is_empty()
            || t.evidence.is_empty()
            || t.instructions.is_empty()
            || t.acceptance.is_empty()
            || t.files.is_empty()
        {
            return Err("任务缺少证据、验收或已有代码范围；调查任务不可执行".into());
        }
        if serde_json::to_vec(t).unwrap().len() > 16 * 1024 || t.files.len() > 20 {
            return Err("任务过大，请拆分".into());
        }
        for (p, h) in &t.files {
            if storage::hash(&read(root, p)?) != *h {
                return Err(format!("{p} 已变化，请重新读取并确认任务"));
            }
        }
    }
    Ok(())
}
