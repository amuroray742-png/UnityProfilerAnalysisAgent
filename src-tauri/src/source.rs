//! Session-scoped, bounded C# inspection. No shell or file mutation tools.
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub const MAX_FILE: usize = 2 * 1024 * 1024;
const EXCLUDED: &[&str] = &[
    ".git",
    "library",
    "temp",
    "obj",
    "logs",
    "build",
    "builds",
    "bin",
    "node_modules",
];
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub scope_id: String,
    pub file_id: String,
    pub root: String,
    pub file_count: usize,
    pub warnings: Vec<String>,
}
#[derive(Debug)]
pub struct SourceScope {
    pub info: SourceInfo,
    root: PathBuf,
    files: BTreeMap<String, String>,
    pub cancelled: Arc<AtomicBool>,
}
fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn decode(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err("UTF-16 长度不正确".into());
        }
        let le = bytes[0] == 0xff;
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|b| {
                if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        String::from_utf16(&words).map_err(|_| "不支持的源码编码".into())
    } else {
        String::from_utf8(
            bytes
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(bytes)
                .to_vec(),
        )
        .map_err(|_| "不支持的源码编码（需要 UTF-8 或带 BOM 的 UTF-16）".into())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl SourceScope {
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("源码范围已取消或释放".into())
        } else {
            Ok(())
        }
    }
    fn bytes(&self, relative: &str) -> Result<Vec<u8>, String> {
        self.check()?;
        let rel = Path::new(relative);
        if rel.is_absolute()
            || rel.components().any(|c| !matches!(c, Component::Normal(_)))
            || relative.contains(':')
        {
            return Err("源码路径越界".into());
        }
        let path = self.root.join(rel);
        let mut check = self.root.clone();
        for c in rel.components() {
            check.push(c);
            if linked(&fs::symlink_metadata(&check).map_err(|e| e.to_string())?) {
                return Err("不读取链接或 junction".into());
            }
        }
        if !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&self.root)
        {
            return Err("源码路径越界".into());
        }
        let file = fs::File::open(&path).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
            let mut buffer = vec![0u16; 32768];
            // SAFETY: the live File owns the handle and the output buffer has the stated capacity.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle() as _,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    0,
                )
            };
            if n == 0 || n as usize >= buffer.len() {
                return Err("无法验证源码文件句柄路径".into());
            }
            let final_path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..n as usize]));
            if !final_path.starts_with(&self.root) {
                return Err("源码文件句柄位于授权目录外".into());
            }
        }

        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE as u64 {
            return Err("文件超过 2 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FILE {
            return Err("文件超过 2 MiB".into());
        }
        self.check()?;
        Ok(bytes)
    }
    pub fn prepare(
        file_id: String,
        root: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if linked(&fs::symlink_metadata(&root).map_err(|e| e.to_string())?) {
            return Err("请选择普通目录，不接受链接或 junction".into());
        }
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if !root.is_dir() {
            return Err("源码位置必须是目录".into());
        }
        let mut scope = Self {
            info: SourceInfo {
                scope_id: uuid::Uuid::new_v4().to_string(),
                file_id,
                root: root.to_string_lossy().into(),
                file_count: 0,
                warnings: vec![],
            },
            root: root.clone(),
            files: BTreeMap::new(),
            cancelled,
        };
        let mut pending = vec![root];
        let mut candidates = 0;
        let mut skipped = 0;
        while let Some(dir) = pending.pop() {
            scope.check()?;
            // Recheck queued directories: a directory may have been replaced after enumeration.
            if fs::symlink_metadata(&dir)
                .map(|m| linked(&m))
                .unwrap_or(true)
                || !dir
                    .canonicalize()
                    .map(|p| p.starts_with(&scope.root))
                    .unwrap_or(false)
            {
                skipped += 1;
                continue;
            }
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            for entry in entries {
                scope.check()?;
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let path = entry.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                if linked(&meta) {
                    skipped += 1;
                    continue;
                }
                if meta.is_dir() {
                    if !EXCLUDED
                        .contains(&entry.file_name().to_string_lossy().to_lowercase().as_str())
                    {
                        pending.push(path);
                    }
                    continue;
                }
                if !meta.is_file()
                    || !path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("cs"))
                {
                    continue;
                }
                candidates += 1;
                if candidates > 50_000 {
                    return Err("超过 50,000 个 C# 文件，请缩小目录".into());
                }
                let rel = path
                    .strip_prefix(&scope.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                match scope.bytes(&rel).and_then(|b| decode(&b).map(|_| b)) {
                    Ok(b) => {
                        scope.files.insert(rel, hash(&b));
                    }
                    Err(e) => {
                        skipped += 1;
                        if scope.info.warnings.len() < 20 {
                            scope.info.warnings.push(format!("{rel}: {e}"));
                        }
                    }
                }
            }
        }
        scope.check()?;
        scope.info.file_count = scope.files.len();
        if skipped > 0 {
            scope.info.warnings.push(format!("跳过 {skipped} 个文件/目录（大小、编码、访问权限或链接）；最多列出 20 条明细，搜索不覆盖这些内容"));
        }
        if scope.files.is_empty() {
            return Err(format!(
                "目录中没有可读取的 C# 文件，请调整源码目录。{}",
                scope.info.warnings.join("；")
            ));
        }
        Ok(scope)
    }
    pub fn text(&self, path: &str) -> Result<(String, String), String> {
        let expected = self.files.get(path).ok_or("文件不在本次源码索引内")?;
        let bytes = self.bytes(path)?;
        let actual = hash(&bytes);
        if &actual != expected {
            return Err(format!("源码已变化：{path}，请重新准备目录并分析"));
        }
        Ok((decode(&bytes)?, actual))
    }
    pub fn query(&self, name: &str, args: Value) -> Result<Value, String> {
        self.check()?;
        let allowed = match name {
            "source_files" => vec!["start", "limit"],
            "source_search" => vec!["query", "start", "limit"],
            "source_read" => vec!["path", "start_line", "limit"],
            _ => return Err("未知源码工具".into()),
        };
        let object = args.as_object().ok_or("参数必须为对象")?;
        if object.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err("未知源码参数".into());
        }
        let number = |key: &str, default: usize| -> Result<usize, String> {
            match args.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| format!("{key} 必须是非负整数")),
            }
        };
        let limit = number("limit", 100)?;
        if limit == 0 || limit > if name == "source_read" { 400 } else { 100 } {
            return Err("分页数量越界".into());
        }
        let start = number("start", 0)?;
        let mut rows = vec![];
        let mut more = false;
        let mut line_truncated = false;
        let mut file_hash = None;
        let next;
        if name == "source_read" {
            let path = args["path"].as_str().ok_or("缺少 path")?;
            let first = number("start_line", 1)?;
            if first == 0 {
                return Err("行号从 1 开始".into());
            }
            let (text, h) = self.text(path)?;
            file_hash = Some(h);
            let lines: Vec<_> = text.lines().collect();
            if first > lines.len() + 1 {
                return Err("起始行越界".into());
            }
            for (i, line) in lines.iter().enumerate().skip(first - 1).take(limit + 1) {
                if rows.len() == limit {
                    more = true;
                    break;
                }
                let mut end = line.len().min(4000);
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                line_truncated |= end < line.len();
                let row = json!({"line":i+1,"text":&line[..end],"lineTruncated":end<line.len()});
                if serde_json::to_vec(&rows).unwrap().len()
                    + serde_json::to_vec(&row).unwrap().len()
                    > 48 * 1024
                {
                    more = true;
                    break;
                }
                rows.push(row);
            }
            next = if more { Some(first + rows.len()) } else { None };
        } else if name == "source_files" {
            if start > self.files.len() {
                return Err("分页起点越界".into());
            }
            for (path, hash) in self.files.iter().skip(start).take(limit + 1) {
                if rows.len() == limit {
                    more = true;
                    break;
                }
                rows.push(json!({"path":path,"hash":hash}));
            }
            next = if more { Some(start + rows.len()) } else { None };
        } else {
            let query = args["query"]
                .as_str()
                .filter(|q| !q.is_empty() && q.len() <= 256)
                .ok_or("query 必须是 1..256 字节的字面量")?;
            let mut matched = 0;
            'files: for path in self.files.keys() {
                let (text, h) = self.text(path)?;
                for (i, line) in text.lines().enumerate() {
                    self.check()?;
                    if !line.contains(query) {
                        continue;
                    }
                    matched += 1;
                    if matched <= start {
                        continue;
                    }
                    if rows.len() == limit {
                        more = true;
                        break 'files;
                    }
                    let mut end = line.len().min(2000);
                    while !line.is_char_boundary(end) {
                        end -= 1;
                    }
                    line_truncated |= end < line.len();
                    rows.push(json!({"path":path,"line":i+1,"text":&line[..end],"hash":h,"lineTruncated":end<line.len()}));
                    if serde_json::to_vec(&rows).unwrap().len() > 40 * 1024 {
                        more = true;
                        break 'files;
                    }
                }
            }
            next = if more { Some(start + rows.len()) } else { None };
        }
        let mut value = json!({"rows":rows,"nextStart":next,"hash":file_hash,"lineTruncated":line_truncated,"warnings":self.info.warnings,"scope":"仅选定目录中已索引的 C#；名称匹配不是性能因果证明，源码不一定对应录制版本"});
        // MCP sends both pretty JSON text and structuredContent. A 20 KiB pretty
        // value bounds both copies plus JSON string escaping below 64 KiB.
        const PAYLOAD_BUDGET: usize = 20 * 1024;
        if serde_json::to_string_pretty(&value).unwrap().len() > PAYLOAD_BUDGET
            && serde_json::to_vec(&value["warnings"]).unwrap().len() > 1024
        {
            value["warnings"] = json!(["覆盖警告过长，查看源码目录准备结果"]);
        }
        while serde_json::to_string_pretty(&value).unwrap().len() > PAYLOAD_BUDGET {
            let rows = value["rows"].as_array_mut().unwrap();
            rows.pop();
            if rows.is_empty() {
                return Err("单项响应过长，无法在 64 KiB 内返回".into());
            }
            let count = rows.len();
            let base = if name == "source_read" {
                args["start_line"].as_u64().unwrap_or(1) as usize
            } else {
                start
            };
            value["nextStart"] = json!(base + count);
        }
        Ok(value)
    }
}
pub fn schemas() -> Value {
    json!({"tools":[
     {"name":"source_files","description":"列出已选目录的 C# 文件；nextStart 非空需续页","inputSchema":{"type":"object","properties":{"start":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}},
     {"name":"source_search","description":"C# 字面量搜索，大小写敏感；返回路径、行号与内容哈希，名称匹配只是候选","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"start":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}},
     {"name":"source_read","description":"按行读取 C#；行号从 1 开始，最多 400 行/64 KiB；检查截断与 nextStart","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":400}},"required":["path"],"additionalProperties":false}}
    ]})
}
