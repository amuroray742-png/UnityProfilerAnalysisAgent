//! Only fixed UPAA commands are invoked; the Agent cannot supply CLI commands or C#.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorStatus {
    pub status: String,
    pub reason: Option<String>,
    pub unity_version: Option<String>,
    pub target_platform: Option<String>,
    pub sampled_at: Option<String>,
    pub details: Option<Value>,
}
impl EditorStatus {
    pub fn unavailable(reason: &str) -> Self {
        Self {
            status: "unavailable".into(),
            reason: Some(reason.into()),
            unity_version: None,
            target_platform: None,
            sampled_at: None,
            details: None,
        }
    }
}
fn executable() -> Result<PathBuf, String> {
    let name = if cfg!(windows) { "unity.exe" } else { "unity" };
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            let path = p.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        let path = local.join("Unity/bin").join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("未安装 Unity CLI；仍可使用离线工程分析".into())
}
async fn read_capped(mut input: impl AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    (&mut input)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        Err("Unity CLI 响应过大".into())
    } else {
        Ok(bytes)
    }
}
async fn invoke(
    root: &Path,
    command: &str,
    mut request: Value,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if !["upaa_context", "upaa_asset"].contains(&command) {
        return Err("Editor 命令未授权".into());
    }
    super::files::check(cancel)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    request["requestId"] = json!(request_id);
    let mut cmd = Command::new(executable()?);
    cmd.args(["command", command, "--request"])
        .arg(serde_json::to_string(&request).unwrap())
        .arg("--project-path")
        .arg(root)
        .args([
            "--timeout",
            "15",
            "--format",
            "json",
            "--non-interactive",
            "--no-pager",
        ]);
    cmd.env_remove("UNITY_PROJECT_PATH")
        .env("UNITY_NO_UPDATE_CHECK", "1")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("无法启动 Unity CLI：{e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let cancelled = async {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if super::files::check(cancel).is_err() {
                break;
            }
        }
    };
    let result = tokio::select! {
        _=cancelled=>Err("Editor 采集已取消".into()),
        result=tokio::time::timeout(Duration::from_secs(20),async{let(status,out,_)=tokio::join!(child.wait(),read_capped(stdout),read_capped(stderr));let out=out?;let data:Value=serde_json::from_slice(&out).map_err(|_|"Unity CLI 返回非 JSON")?;if !status.map_err(|e|e.to_string())?.success()||data["success"]!=true{let codes=data["errors"].as_array().map(|es|es.iter().filter_map(|e|e["code"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();return Err(format!("Editor 未就绪或采集插件不可用（{codes}）；请确认所选工程已打开并安装插件"));}Ok(data)})=>result.unwrap_or_else(|_|Err("Editor 采集超时；没有关闭或改变 Unity".into()))
    };
    if result.is_err() {
        let _ = child.kill().await;
        if command == "upaa_asset" {
            if let Ok(exe) = executable() {
                let mut stop = Command::new(exe);
                stop.args(["command", "upaa_cancel", "--request"])
                    .arg(json!({"requestId":request_id}).to_string())
                    .arg("--project-path")
                    .arg(root)
                    .args([
                        "--timeout",
                        "2",
                        "--format",
                        "json",
                        "--non-interactive",
                        "--no-pager",
                    ]);
                stop.env_remove("UNITY_PROJECT_PATH")
                    .kill_on_drop(true)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                stop.creation_flags(0x08000000);
                let _ = tokio::time::timeout(Duration::from_secs(3), stop.status()).await;
            }
        }
    }
    let envelope = result?;
    // CLI versions wrap custom command values in data.result (sometimes a JSON string).
    let mut data = envelope.get("data").cloned().ok_or("CLI 缺少 data")?;
    for _ in 0..5 {
        if let Some(s) = data.as_str() {
            data = serde_json::from_str(s).map_err(|_| "插件响应不是 JSON")?;
        } else if data.get("protocolVersion").is_some() {
            break;
        } else if let Some(v) = data.get("result").or_else(|| data.get("data")) {
            data = v.clone();
        } else {
            break;
        }
    }
    validate(&data, root)?;
    Ok(data)
}
pub fn validate(data: &Value, root: &Path) -> Result<(), String> {
    if data["protocolVersion"] != 1 {
        return Err("Editor 采集插件协议不兼容，需要 UPAA inspector v1".into());
    }
    let path = PathBuf::from(data["projectRoot"].as_str().ok_or("插件未返回工程身份")?)
        .canonicalize()
        .map_err(|_| "Editor 工程路径不可验证")?;
    if path != root.canonicalize().map_err(|e| e.to_string())? {
        return Err("Editor 工程身份不匹配，拒绝混合证据".into());
    }
    if data["status"] != "ready" {
        return Err(data["reason"]
            .as_str()
            .unwrap_or("Editor 编译/导入/资源状态不可用")
            .into());
    }
    Ok(())
}
pub async fn status(root: &Path, cancel: &AtomicBool) -> EditorStatus {
    match invoke(root, "upaa_context", json!({}), cancel).await {
        Ok(v) => EditorStatus {
            status: "ready".into(),
            reason: None,
            unity_version: v["unityVersion"].as_str().map(Into::into),
            target_platform: v["targetPlatform"].as_str().map(Into::into),
            sampled_at: v["sampledAt"].as_str().map(Into::into),
            details: Some(v),
        },
        Err(e) => EditorStatus::unavailable(&e),
    }
}
pub async fn inspect(
    root: &Path,
    path: &str,
    start: usize,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if !super::valid_asset_path(path) {
        return Err("非法资源路径".into());
    }
    let v = invoke(
        root,
        "upaa_asset",
        json!({"path":path,"start":start,"limit":limit}),
        cancel,
    )
    .await?;
    if v["path"] != path {
        return Err("Editor 返回了其他资源，拒绝使用".into());
    }
    super::bounded(v)
}
