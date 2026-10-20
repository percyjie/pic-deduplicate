use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use walkdir::{DirEntry, WalkDir};

use crate::core::paths::{
    has_generated_ancestor, is_generated_dir, is_supported_image, path_to_rel, GENERATED_MARKER,
    LIBRARY_DIR, RESULTS_DIR,
};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub abs_path: PathBuf,
    pub rel_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
}

pub fn scan_source(source: &Path, cancel: &AtomicBool) -> AppResult<ScanResult> {
    if has_generated_ancestor(source) {
        return Err(AppError::user(
            "SOURCE_INSIDE_GENERATED",
            "来源目录属于程序生成目录，不能再次导入。",
        ));
    }

    let mut result = ScanResult::default();
    let walker = WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !should_prune_dir(entry, source));

    for (index, entry) in walker.enumerate() {
        if index % 256 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_warning(
                    &mut result.warnings,
                    format!("无法读取目录项：{error}"),
                );
                continue;
            }
        };
        if entry.depth() == 0 || !entry.file_type().is_file() {
            continue;
        }
        if entry.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        if !is_supported_image(path) {
            continue;
        }
        if is_windows_link_or_reparse(path) {
            continue;
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                push_warning(
                    &mut result.warnings,
                    format!("无法读取文件属性 {}：{error}", path.display()),
                );
                continue;
            }
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            push_warning(
                &mut result.warnings,
                format!("跳过文件名不是有效 Unicode 的文件：{}", path.display()),
            );
            continue;
        };
        let Ok(rel_path) = path_to_rel(source, path) else {
            continue;
        };
        let mtime_ms = metadata.modified().ok().and_then(|time| {
            time.duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_millis() as i64)
        });
        result.files.push(ScannedFile {
            abs_path: path.to_path_buf(),
            rel_path,
            file_name: file_name.to_string(),
            size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            mtime_ms,
        });
    }

    result.files.sort_by(|left, right| {
        left.rel_path
            .to_lowercase()
            .cmp(&right.rel_path.to_lowercase())
            .then_with(|| left.rel_path.cmp(&right.rel_path))
    });

    Ok(result)
}

fn should_prune_dir(entry: &DirEntry, source: &Path) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if !entry.file_type().is_dir() {
        return false;
    }
    if entry.file_type().is_symlink() {
        return true;
    }
    let path = entry.path();
    if path == source {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if name.starts_with(".picdedupe") || name == LIBRARY_DIR || name == RESULTS_DIR {
        return true;
    }
    if is_generated_dir(path) || path.join(GENERATED_MARKER).exists() {
        return true;
    }
    is_hidden_system_or_reparse(path)
}

fn push_warning(warnings: &mut Vec<String>, message: String) {
    if warnings.len() < 200 {
        warnings.push(message);
    }
}

#[cfg(windows)]
fn is_hidden_system_or_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    fs::symlink_metadata(path)
        .map(|metadata| {
            let attributes = metadata.file_attributes();
            attributes
                & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_windows_link_or_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden_system_or_reparse(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_windows_link_or_reparse(_path: &Path) -> bool {
    false
}
