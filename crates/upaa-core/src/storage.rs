use sha2::{Digest,Sha256};
use std::{path::Path,fs::File,sync::atomic::AtomicBool};
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn file_hash(path: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    crate::project::files::hash(&mut f, u64::MAX, cancel)
}
