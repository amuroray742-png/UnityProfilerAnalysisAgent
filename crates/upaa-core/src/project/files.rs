//! Bounded, session-scoped disk reads. No links, junctions, or arbitrary paths.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek},
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};
pub fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("工程任务已取消或失效".into())
    } else {
        Ok(())
    }
}
pub fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}
pub fn open(root: &Path, relative: &str) -> Result<File, String> {
    let rel = Path::new(relative);
    if rel.as_os_str().is_empty()
        || rel.is_absolute()
        || relative.contains(':')
        || rel.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("工程路径越界".into());
    }
    if linked(&fs::symlink_metadata(root).map_err(|e| e.to_string())?) {
        return Err("工程根目录已变成链接".into());
    }
    let mut path = root.to_path_buf();
    for component in rel.components() {
        path.push(component);
        if linked(&fs::symlink_metadata(&path).map_err(|e| e.to_string())?) {
            return Err("不读取符号链接或 junction".into());
        }
    }
    if !path
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(root)
    {
        return Err("工程路径越界".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("不是普通文件".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
        let mut buffer = vec![0u16; 32768];
        // SAFETY: File owns a valid live handle and the buffer has the supplied capacity.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle() as _,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        };
        if n == 0 || n as usize >= buffer.len() {
            return Err("无法验证工程文件句柄".into());
        }
        if !std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..n as usize]))
            .starts_with(root)
        {
            return Err("工程文件句柄越界".into());
        }
    }
    Ok(file)
}
pub fn hash(file: &mut File, max: u64, cancel: &AtomicBool) -> Result<String, String> {
    file.rewind().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 65536];
    loop {
        check(cancel)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max {
            return Err("文件超过允许的大小".into());
        }
        hash.update(&buffer[..n]);
    }
    file.rewind().map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hash.finalize()))
}
/// UTF-8 and BOM UTF-16, bounded per line and file. `false` stops traversal early.
pub fn lines(
    file: &mut File,
    max: u64,
    cancel: &AtomicBool,
    mut visit: impl FnMut(usize, &str) -> Result<bool, String>,
) -> Result<(), String> {
    file.rewind().map_err(|e| e.to_string())?;
    let mut prefix = [0; 3];
    let n = file.read(&mut prefix).map_err(|e| e.to_string())?;
    let utf16 = n >= 2 && (prefix[..2] == [0xff, 0xfe] || prefix[..2] == [0xfe, 0xff]);
    let offset = if utf16 {
        2
    } else if n == 3 && prefix == [0xef, 0xbb, 0xbf] {
        3
    } else {
        0
    };
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut total = offset;
    let mut number = 0;
    loop {
        check(cancel)?;
        let line = if utf16 {
            let mut words = Vec::new();
            loop {
                let mut word = [0; 2];
                let n = reader.read(&mut word[..1]).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                reader
                    .read_exact(&mut word[1..])
                    .map_err(|_| "UTF-16 长度错误")?;
                total += 2;
                if total > max || words.len() > 512 * 1024 {
                    return Err("文件或单行超过读取上限".into());
                }
                let w = if prefix[0] == 0xff {
                    u16::from_le_bytes(word)
                } else {
                    u16::from_be_bytes(word)
                };
                words.push(w);
                if w == 10 {
                    break;
                }
            }
            if words.is_empty() {
                break;
            }
            String::from_utf16(&words).map_err(|_| "UTF-16 编码错误")?
        } else {
            let mut bytes = Vec::new();
            let n = reader
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut bytes)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if n > 1024 * 1024 || total > max {
                return Err("文件或单行超过读取上限".into());
            }
            String::from_utf8(bytes).map_err(|_| "不是 UTF-8 / BOM UTF-16 文本")?
        };
        if line.contains('\0') {
            return Err("二进制内容，不作为文本分析".into());
        }
        number += 1;
        if !visit(number, line.trim_end_matches(['\r', '\n']))? {
            break;
        }
    }
    check(cancel)
}
