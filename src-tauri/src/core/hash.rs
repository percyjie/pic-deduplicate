use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

const BUFFER_SIZE: usize = 1024 * 1024;

pub fn hash_file(path: &Path, cancel: &AtomicBool) -> AppResult<String> {
    let before = fs::metadata(path)?;
    let before_modified = before.modified().ok();
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let after = fs::metadata(path)?;
    if before.len() != after.len() || before_modified != after.modified().ok() {
        return Err(AppError::user(
            "SOURCE_CHANGED_DURING_HASH",
            "文件在计算哈希期间发生变化，已跳过。",
        ));
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_existing_file(path: &Path) -> AppResult<(String, u64)> {
    let cancel = AtomicBool::new(false);
    let hash = hash_file(path, &cancel)?;
    let size = fs::metadata(path)?.len();
    Ok((hash, size))
}

pub fn metadata_timestamp(time: Option<SystemTime>) -> Option<i64> {
    time.and_then(|value| {
        value
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as i64)
    })
}
