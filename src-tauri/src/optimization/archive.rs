//! Version 3: immutable, hash-checked objects and a last-written manifest.
//! Orphan objects after a failed commit are harmless; old manifests remain readable.
use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
};

fn plain(path: &Path) -> Result<(), String> {
    if crate::project::files::linked(&fs::symlink_metadata(path).map_err(|e| e.to_string())?) {
        return Err("存档内部不可包含链接/junction".into());
    }
    Ok(())
}

fn object(dir: &Path, value: &Value) -> Result<Value, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err("单个存档对象超过 128 MiB，未提交索引，原记录保留".into());
    }
    let hash = storage::hash(&bytes);
    let path = dir.join("objects").join(format!("{hash}.json"));
    if path.exists() {
        plain(&path)?;
        if storage::hash(&fs::read(&path).map_err(|e| e.to_string())?) != hash {
            return Err("已有存档对象损坏，停止保存".into());
        }
    } else {
        storage::atomic(&path, &bytes)?;
    }
    Ok(json!({"object":hash}))
}
fn resolve(dir: &Path, value: &Value) -> Result<Value, String> {
    let hash = value["object"].as_str().ok_or("存档对象引用无效")?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("存档对象指纹无效".into());
    }
    let path = dir.join("objects").join(format!("{hash}.json"));
    plain(&dir.join("objects"))?;
    plain(&path)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
        return Err("单个存档对象过大".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if storage::hash(&bytes) != hash {
        return Err("存档对象损坏，拒绝恢复".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
pub fn save(dir: &Path, data: &Project) -> Result<(), String> {
    for folder in ["objects", "journals"] {
        fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
        plain(&dir.join(folder))?;
    }
    let mut value = serde_json::to_value(data).map_err(|e| e.to_string())?;
    value["version"] = json!(3);
    for capture in value["captures"].as_array_mut().unwrap() {
        *capture = object(dir, capture)?;
    }
    for round in value["rounds"].as_array_mut().unwrap() {
        for report in round["reports"].as_array_mut().unwrap() {
            *report = object(dir, report)?;
        }
        for run in round["runs"].as_array_mut().unwrap() {
            for change in run["changes"].as_array_mut().unwrap() {
                *change = object(dir, change)?;
            }
            *run = object(dir, run)?;
        }
        *round = object(dir, round)?;
    }
    storage::atomic(
        &dir.join("optimization.json"),
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )
}
pub fn load(dir: &Path, bytes: &[u8]) -> Result<Project, String> {
    let mut v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if v["version"] == 3 {
        for c in v["captures"].as_array_mut().ok_or("缺少录制索引")? {
            *c = resolve(dir, c)?;
        }
        for r in v["rounds"].as_array_mut().ok_or("缺少轮次索引")? {
            *r = resolve(dir, r)?;
            for p in r["reports"].as_array_mut().ok_or("缺少报告索引")? {
                *p = resolve(dir, p)?;
            }
            for run in r["runs"].as_array_mut().ok_or("缺少修改索引")? {
                *run = resolve(dir, run)?;
                for c in run["changes"].as_array_mut().ok_or("缺少恢复索引")? {
                    *c = resolve(dir, c)?;
                }
            }
        }
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}
fn journal_path(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("日志身份无效".into());
    }
    Ok(dir.join("journals").join(format!("{id}.jsonl")))
}
pub fn append(dir: &Path, id: &str, value: &Value) -> Result<(), String> {
    fs::create_dir_all(dir.join("journals")).map_err(|e| e.to_string())?;
    plain(&dir.join("journals"))?;
    let path = journal_path(dir, id)?;
    if path.exists() {
        plain(&path)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("存档写入失败，已停止：{e}"))
}
pub fn replay(dir: &Path, id: &str) -> Result<Vec<Value>, String> {
    let path = journal_path(dir, id)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    plain(&dir.join("journals"))?;
    plain(&path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut out = vec![];
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        // An interrupted last write is not a committed event.
        if !line.ends_with('\n') {
            break;
        }
        out.push(serde_json::from_str(&line).map_err(|e| format!("存档日志损坏：{e}"))?);
    }
    Ok(out)
}
