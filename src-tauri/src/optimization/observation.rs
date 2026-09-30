//! Durable, bounded public activity feed. Never records private reasoning or tool results.
use super::*;
use crate::acp_client::DiagnoseEvent;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant},
};

pub const ACTIVITY_LIMIT: u64 = 10 * 1024 * 1024;
#[derive(Default)]
pub struct Observation {
    pub notify: Option<Arc<dyn Fn(&str, Value) + Send + Sync>>,
    pub progress: Option<Value>,
}
impl std::fmt::Debug for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observation")
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}
fn plain(path: &Path) -> Result<(), String> {
    if crate::project::files::linked(&fs::symlink_metadata(path).map_err(|e| e.to_string())?) {
        return Err("活动目录不可包含链接".into());
    }
    Ok(())
}
fn path(dir: &Path, run: &str) -> Result<PathBuf, String> {
    plain(dir)?;
    if run.is_empty() || !run.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("活动身份无效".into());
    }
    let folder = dir.join("activity");
    if folder.exists() {
        plain(&folder)?;
    }
    let path = folder.join(format!("{run}.jsonl"));
    if path.exists() {
        plain(&path)?;
    }
    Ok(path)
}
fn brief(v: &Value) -> Value {
    let Some(o) = v.as_object() else {
        return Value::Null;
    };
    let mut out = serde_json::Map::new();
    // No replacement bodies, original code or arbitrary tool outputs.
    for key in [
        "path",
        "frame_index",
        "thread_index",
        "start",
        "start_line",
        "query",
        "report_id",
        "task_id",
    ] {
        if let Some(v) = o.get(key) {
            out.insert(
                key.into(),
                if let Some(s) = v.as_str() {
                    json!(s.chars().take(500).collect::<String>())
                } else if v.is_number() {
                    v.clone()
                } else {
                    Value::Null
                },
            );
        }
    }
    Value::Object(out)
}
pub fn public_event(event: &DiagnoseEvent) -> Option<Value> {
    Some(match event {
        DiagnoseEvent::ToolActivity {
            call_id,
            tool,
            status,
            args,
            error,
        } => {
            json!({"kind":"tool","callId":call_id,"tool":tool,"status":status,"args":brief(args),"error":error.as_ref().map(|s|s.chars().take(1000).collect::<String>())})
        }
        DiagnoseEvent::Chunk { text } => json!({"kind":"chunk","text":text}),
        DiagnoseEvent::SessionCreated { acp_session_id } => {
            json!({"kind":"session","sessionId":acp_session_id})
        }
        DiagnoseEvent::Started { agent_id } => json!({"kind":"started","agentId":agent_id}),
        DiagnoseEvent::Finished { .. } => json!({"kind":"finished"}),
        DiagnoseEvent::Cancelled => json!({"kind":"cancelled"}),
        DiagnoseEvent::Error { message } => {
            json!({"kind":"error","message":message.chars().take(2000).collect::<String>()})
        }
        _ => return None,
    })
}
impl Workspace {
    pub fn activity(
        &self,
        project: &str,
        round: &str,
        run: &str,
        event: &DiagnoseEvent,
    ) -> Result<(), String> {
        if let DiagnoseEvent::Chunk { text } = event {
            if text.len() > 16384 {
                let mut rest = text.as_str();
                while !rest.is_empty() {
                    let mut end = rest.len().min(16384);
                    while !rest.is_char_boundary(end) {
                        end -= 1;
                    }
                    self.activity(
                        project,
                        round,
                        run,
                        &DiagnoseEvent::Chunk {
                            text: rest[..end].into(),
                        },
                    )?;
                    rest = &rest[end..];
                }
                return Ok(());
            }
        }
        let Some(event) = public_event(event) else {
            return Ok(());
        };
        let result: Result<(), String> = (|| {
            let obs = self.observation.lock().unwrap();
            let p = path(&self.directory, run)?;
            fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
            plain(p.parent().unwrap())?;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)
                .map_err(|e| e.to_string())?;
            let offset = f.metadata().map_err(|e| e.to_string())?.len();
            // Leave room for an explicit cap marker; subsequent activity is deliberately not retained.
            if offset >= ACTIVITY_LIMIT - 4096 {
                return Ok(());
            }
            let mut row = json!({"projectId":project,"roundId":round,"runId":run,"sequence":offset,"time":chrono::Utc::now().to_rfc3339(),"event":event});
            let mut bytes = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
            if offset + bytes.len() as u64 + 1 >= ACTIVITY_LIMIT - 4096 {
                row["event"] = json!({"kind":"limited","message":"工作过程达到 10 MiB，后续活动未记录；完整报告和实际修改仍单独保存。"});
                bytes = serde_json::to_vec(&row).unwrap();
                // Pad the committed cap entry so its next cursor reaches the cap sentinel.
                bytes.resize((ACTIVITY_LIMIT - 4096 - offset) as usize, b' ');
            }
            bytes.push(b'\n');
            f.write_all(&bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| format!("活动存档失败，已停止：{e}"))?;
            row["nextCursor"] = json!(offset + bytes.len() as u64);
            if let Some(notify) = &obs.notify {
                notify("workflow-activity", row);
            }
            Ok(())
        })();
        if let Err(ref e) = result {
            *self.save_error.lock().unwrap() = Some(e.clone());
        }
        result
    }
}
pub fn read(dir: &Path, run: &str, cursor: u64) -> Result<Value, String> {
    let p = path(dir, run)?;
    if !p.exists() {
        return Ok(json!({"available":false,"rows":[],"nextCursor":0,"hasMore":false}));
    }
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if cursor > len || len > ACTIVITY_LIMIT {
        return Err("活动游标或文件长度无效".into());
    }
    if cursor > 0 {
        f.seek(SeekFrom::Start(cursor - 1))
            .map_err(|e| e.to_string())?;
        let mut b = [0];
        f.read_exact(&mut b).map_err(|e| e.to_string())?;
        if b[0] != b'\n' {
            return Err("活动游标不在记录边界".into());
        }
    }
    f.seek(SeekFrom::Start(cursor)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(f);
    let mut rows = vec![];
    let mut next = cursor;
    let mut size = 0;
    while rows.len() < 100 && size < 128 * 1024 {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 || !line.ends_with('\n') {
            break;
        }
        let mut row: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if row["sequence"] != next {
            return Err("活动顺序损坏".into());
        }
        next += n as u64;
        row["nextCursor"] = json!(next);
        size += n;
        rows.push(row);
    }
    Ok(
        json!({"available":true,"rows":rows,"nextCursor":next,"hasMore":next<len&&size>0,"limited":len>=ACTIVITY_LIMIT-4096}),
    )
}

#[derive(Clone)]
pub struct ParseProgress {
    workspace: Arc<Workspace>,
    project: String,
    round: String,
    operation: String,
    last: Arc<Mutex<Option<Instant>>>,
}
impl ParseProgress {
    pub fn new(w: Arc<Workspace>, round: String, operation: String) -> Self {
        let project = w.data.lock().unwrap().id.clone();
        Self {
            workspace: w,
            project,
            round,
            operation,
            last: Arc::new(Mutex::new(None)),
        }
    }
    pub fn update(
        &self,
        stage: &str,
        done: Option<u64>,
        total: Option<u64>,
        status: &str,
        reason: Option<String>,
    ) {
        if status=="running" && self.workspace.cancelled.load(std::sync::atomic::Ordering::SeqCst){return;}
        let mut last = self.last.lock().unwrap();
        let mut obs = self.workspace.observation.lock().unwrap();
        let same = obs.progress.as_ref().is_some_and(|v| {
            v["stage"] == stage && v["operationId"] == self.operation && v["total"] == json!(total)
        });
        if same
            && status == "running"
            && done != total
            && last.is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        *last = Some(Instant::now());
        let row = json!({"projectId":self.project,"roundId":self.round,"operationId":self.operation,"stage":stage,"done":done,"total":total,"status":status,"reason":reason});
        obs.progress = Some(row.clone());
        if let Some(notify) = &obs.notify {
            notify("workflow-progress", row);
        }
    }
    pub fn fail(&self, error: &str) {
        let stage = self
            .workspace
            .observation
            .lock()
            .unwrap()
            .progress
            .as_ref()
            .filter(|v| v["operationId"] == self.operation)
            .and_then(|v| v["stage"].as_str())
            .unwrap_or("verify")
            .to_owned();
        self.update(
            &stage,
            None,
            None,
            if self
                .workspace
                .cancelled
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                "cancelled"
            } else {
                "failed"
            },
            Some(error.into()),
        );
    }
    pub async fn hash(&self, path: PathBuf) -> Result<String, String> {
        let p = self.clone();
        self.update("verify", None, None, "running", None);
        tokio::task::spawn_blocking(move || {
            use sha2::{Digest, Sha256};
            let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
            let total = f.metadata().map_err(|e| e.to_string())?.len();
            let mut h = Sha256::new();
            let mut b = [0; 128 * 1024];
            let mut done = 0;
            p.update("verify", Some(0), Some(total), "running", None);
            loop {
                crate::parser::data::cancelled(&p.workspace.cancelled)
                    .map_err(|e| e.to_string())?;
                let n = f.read(&mut b).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                h.update(&b[..n]);
                done += n as u64;
                p.update("verify", Some(done), Some(total), "running", None);
            }
            Ok(format!("{:x}", h.finalize()))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub async fn parse(&self, path: PathBuf) -> Result<crate::parser::ParsedProfile, String> {
        self.update("parse", None, None, "running", None);
        let p = self.clone();
        let parsed = crate::parser::parse_file_cancel(
            &path,
            move |done, total| {
                if !p
                    .workspace
                    .cancelled
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    p.update("parse", Some(done), Some(total), "running", None)
                }
            },
            self.workspace.cancelled.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        if parsed.meta.format == crate::parser::ProfilerFormat::Data {
            self.update(
                "parse",
                Some(parsed.meta.file_size_bytes),
                Some(parsed.meta.file_size_bytes),
                "running",
                None,
            );
        }
        Ok(parsed)
    }
}
