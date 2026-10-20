use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub const CONTROL_DIR: &str = ".picdedupe";
pub const DB_FILE: &str = "index.db";
pub const MARKER_FILE: &str = "marker.json";
pub const LOCK_FILE: &str = "app.lock";
pub const BACKUPS_DIR: &str = "backups";
pub const LIBRARY_DIR: &str = "图片主库";
pub const RESULTS_DIR: &str = "去重结果";
pub const DUPLICATES_DIR: &str = "重复";
pub const UNIQUES_DIR: &str = "不重复";
pub const REPORT_FILE: &str = "批次报告.csv";
pub const SUMMARY_FILE: &str = "批次摘要.json";
pub const GENERATED_MARKER: &str = ".picdedupe_generated";
pub const TEMP_PREFIX: &str = ".picdedupe-tmp-";
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "webp", "tif", "tiff", "heic",
    "heif", "avif", "ico",
];

#[derive(Debug, Clone)]
pub struct RootPaths {
    pub root: PathBuf,
    pub control: PathBuf,
    pub db: PathBuf,
    pub marker: PathBuf,
    pub lock: PathBuf,
    pub backups: PathBuf,
}

impl RootPaths {
    pub fn from_root(root: &Path) -> Self {
        let control = root.join(CONTROL_DIR);
        Self {
            root: root.to_path_buf(),
            db: control.join(DB_FILE),
            marker: control.join(MARKER_FILE),
            lock: control.join(LOCK_FILE),
            backups: control.join(BACKUPS_DIR),
            control,
        }
    }
}

pub fn normalize_path(path: &Path) -> AppResult<PathBuf> {
    dunce::canonicalize(path).map_err(|error| {
        AppError::user(
            "PATH_UNAVAILABLE",
            format!("无法访问路径 {}：{error}", path.display()),
        )
    })
}

pub fn is_unc_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with(r"\\") || text.starts_with("//")
}

pub fn normalize_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

pub fn is_same_or_child(parent: &Path, child: &Path) -> bool {
    let parent = normalize_for_compare(parent);
    let child = normalize_for_compare(child);
    child == parent || child.starts_with(&format!("{parent}/"))
}

pub fn absolute_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn path_to_rel(root: &Path, path: &Path) -> AppResult<String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        AppError::user(
            "PATH_OUTSIDE_ROOT",
            format!("路径 {} 不在主文件夹内。", path.display()),
        )
    })?;
    Ok(relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

pub fn rel_to_path(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub fn is_generated_dir(path: &Path) -> bool {
    path.join(GENERATED_MARKER).is_file()
}

pub fn has_generated_ancestor(path: &Path) -> bool {
    path.ancestors().any(is_generated_dir)
}

pub fn write_generated_marker(dir: &Path) -> AppResult<()> {
    let marker = dir.join(GENERATED_MARKER);
    if !marker.exists() {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker)?;
        file.write_all(b"pic-deduplicate generated directory\n")?;
    }
    Ok(())
}

pub fn prepare_generated_root(path: &Path) -> AppResult<()> {
    if path.exists() {
        if !path.is_dir() {
            return Err(AppError::user(
                "RESERVED_FOLDER_CONFLICT",
                format!("保留路径 {} 已被同名文件占用。", path.display()),
            ));
        }
        if !is_generated_dir(path) {
            return Err(AppError::user(
                "RESERVED_FOLDER_CONFLICT",
                format!(
                    "目录 {} 已存在但不属于本程序。请先移走或改名该目录。",
                    path.display()
                ),
            ));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    write_generated_marker(path)
}

pub fn new_generated_dir(path: &Path) -> AppResult<()> {
    fs::create_dir(path)?;
    write_generated_marker(path)
}

pub fn remove_stale_temp_files(dir: &Path) -> AppResult<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(TEMP_PREFIX)
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let ext = extension.to_ascii_lowercase();
            SUPPORTED_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn set_hidden(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, INVALID_FILE_ATTRIBUTES,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let attributes = GetFileAttributesW(wide.as_ptr());
        if attributes != INVALID_FILE_ATTRIBUTES {
            let _ = SetFileAttributesW(wide.as_ptr(), attributes | FILE_ATTRIBUTE_HIDDEN);
        }
    }
}

#[cfg(not(windows))]
pub fn set_hidden(_path: &Path) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn same_or_child_is_case_insensitive_and_separator_agnostic() {
        assert!(is_same_or_child(
            Path::new(r"E:\Root"),
            Path::new(r"e:\root\商户")
        ));
        assert!(is_same_or_child(Path::new("E:/Root"), Path::new(r"E:\Root")));
        assert!(!is_same_or_child(
            Path::new(r"E:\Root"),
            Path::new(r"E:\Root2")
        ));
        assert!(!is_same_or_child(
            Path::new(r"E:\Root"),
            Path::new(r"E:\Other")
        ));
    }

    #[test]
    fn unc_paths_are_detected() {
        assert!(is_unc_path(Path::new(r"\\server\share")));
        assert!(is_unc_path(Path::new("//server/share")));
        assert!(!is_unc_path(Path::new(r"C:\photos")));
    }

    #[test]
    fn relative_paths_roundtrip_with_forward_slashes() {
        let root = Path::new(r"E:\Root");
        let path = root.join("商户A").join(LIBRARY_DIR).join("2026-10").join("a.jpg");
        let relative = path_to_rel(root, &path).unwrap();
        assert_eq!(relative, "商户A/图片主库/2026-10/a.jpg");
        assert_eq!(rel_to_path(root, &relative), path);
        assert!(path_to_rel(root, Path::new(r"E:\Other\a.jpg")).is_err());
    }

    #[test]
    fn supported_image_extensions_are_case_insensitive() {
        assert!(is_supported_image(Path::new("a.JPG")));
        assert!(is_supported_image(Path::new("a.webp")));
        assert!(is_supported_image(Path::new("a.HeIf")));
        assert!(!is_supported_image(Path::new("a.raw")));
        assert!(!is_supported_image(Path::new("noext")));
    }

    #[test]
    fn stale_temp_files_are_removed_and_normal_files_kept() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("2026-10");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(format!("{TEMP_PREFIX}abc")), b"partial").unwrap();
        fs::write(nested.join("photo.jpg"), b"image").unwrap();

        let removed = remove_stale_temp_files(dir.path()).unwrap();
        assert_eq!(removed, 1);
        assert!(!nested.join(format!("{TEMP_PREFIX}abc")).exists());
        assert!(nested.join("photo.jpg").exists());
    }
}
