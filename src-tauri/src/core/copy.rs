use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::core::paths::TEMP_PREFIX;
use crate::error::{AppError, AppResult};

const BUFFER_SIZE: usize = 1024 * 1024;

pub fn copy_verified(
    source: &Path,
    target: &Path,
    expected_hash: &str,
    cancel: &AtomicBool,
) -> AppResult<()> {
    if target.exists() {
        return Err(AppError::user(
            "TARGET_EXISTS",
            format!("目标文件已存在，拒绝覆盖：{}", target.display()),
        ));
    }
    let parent = target.parent().ok_or_else(|| {
        AppError::user(
            "INVALID_TARGET_PATH",
            format!("无法确定目标目录：{}", target.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let temp_path = parent.join(format!("{TEMP_PREFIX}{}", Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut source_file = File::open(source)?;
        let mut target_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; BUFFER_SIZE];

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::Cancelled);
            }
            let read = source_file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            target_file.write_all(&buffer[..read])?;
        }
        target_file.flush()?;
        target_file.sync_all()?;
        drop(target_file);

        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != expected_hash {
            return Err(AppError::user(
                "COPY_HASH_MISMATCH",
                "复制后的文件哈希与源文件不一致，已取消写入。",
            ));
        }
        fs::rename(&temp_path, target)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}
