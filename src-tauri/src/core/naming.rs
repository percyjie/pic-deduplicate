use std::collections::HashSet;
use std::path::Path;

use crate::error::{AppError, AppResult};

const MAX_STEM_CHARS: usize = 80;
const MAX_RESULT_STEM_CHARS: usize = 140;
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9",
];

pub fn validate_folder_name(name: &str) -> AppResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::user("INVALID_MERCHANT_NAME", "商户名称不能为空。"));
    }
    if trimmed != name {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "商户名称首尾不能有空格。",
        ));
    }
    if name.chars().count() > 80 {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "商户名称不能超过 80 个字符。",
        ));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "商户名称不能以点或空格结尾。",
        ));
    }
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "商户名称不能包含 Windows 文件名禁用字符。",
        ));
    }
    let upper = name.to_ascii_uppercase();
    if RESERVED_NAMES.contains(&upper.as_str()) {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "该名称是 Windows 保留名称，请换一个名称。",
        ));
    }
    Ok(())
}

pub fn split_file_name(file_name: &str) -> (String, String) {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    (stem, extension)
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

pub fn sanitize_stem(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            result.push('_');
        } else {
            result.push(ch);
        }
    }
    let result = result.trim().trim_end_matches(['.', ' ']).trim().to_string();
    let result = truncate_chars(&result, MAX_STEM_CHARS);
    let upper = result.to_ascii_uppercase();
    if result.is_empty() {
        "image".to_string()
    } else if RESERVED_NAMES.contains(&upper.as_str()) {
        format!("_{result}")
    } else {
        result
    }
}

pub fn master_file_name(file_name: &str, hash: &str, prefix_len: usize) -> String {
    let (stem, extension) = split_file_name(file_name);
    let prefix = hash.chars().take(prefix_len).collect::<String>();
    let stem = sanitize_stem(&stem);
    if extension.is_empty() {
        format!("{stem}__{prefix}")
    } else {
        format!("{stem}__{prefix}.{extension}")
    }
}

pub fn result_candidate_name(original: &str, attempt: u32) -> (String, String) {
    let (stem, extension) = split_file_name(original);
    let stem = truncate_chars(&stem, MAX_RESULT_STEM_CHARS);
    let suffix = if attempt <= 1 {
        String::new()
    } else {
        format!(" ({attempt})")
    };
    if extension.is_empty() {
        (format!("{stem}{suffix}"), String::new())
    } else {
        (format!("{stem}{suffix}"), extension)
    }
}

pub fn pick_result_name(
    dir: &Path,
    original: &str,
    used: &mut HashSet<String>,
) -> AppResult<String> {
    for attempt in 1..=10_000u32 {
        let (stem, extension) = result_candidate_name(original, attempt);
        let file_name = if extension.is_empty() {
            stem
        } else {
            format!("{stem}.{extension}")
        };
        let key = file_name.to_lowercase();
        if used.contains(&key) {
            continue;
        }
        if dir.join(&file_name).exists() {
            continue;
        }
        used.insert(key);
        return Ok(file_name);
    }
    Err(AppError::user(
        "RESULT_NAME_CONFLICT",
        "结果文件重名过多，无法生成安全文件名。",
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn folder_name_validation_accepts_chinese_and_rejects_unsafe_names() {
        assert!(validate_folder_name("商户A").is_ok());
        assert!(validate_folder_name("New Shop 01").is_ok());
        for name in [
            "", " ", "商户.", "商户 ", "a/b", "CON", "nul", "a:b", "a\nb", "a*b",
        ] {
            assert!(validate_folder_name(name).is_err(), "应拒绝 {name:?}");
        }
    }

    #[test]
    fn split_file_name_lowercases_extension_and_handles_no_extension() {
        assert_eq!(
            split_file_name("Photo.JPG"),
            ("Photo".to_string(), "jpg".to_string())
        );
        assert_eq!(
            split_file_name("archive.tar.gz"),
            ("archive.tar".to_string(), "gz".to_string())
        );
        assert_eq!(
            split_file_name("README"),
            ("README".to_string(), String::new())
        );
    }

    #[test]
    fn master_name_sanitizes_stem_and_appends_hash_prefix() {
        assert_eq!(
            master_file_name("春游 01.jpg", "a1b2c3d4e5f60718ff", 16),
            "春游 01__a1b2c3d4e5f60718.jpg"
        );
        assert_eq!(
            master_file_name("a?b.png", "abcdef0123456789aa", 16),
            "a_b__abcdef0123456789.png"
        );
        assert_eq!(
            master_file_name("CON.jpg", "abcdef0123456789aa", 16),
            "_CON__abcdef0123456789.jpg"
        );
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("图片去重工具", 3), "图片去");
        assert_eq!(truncate_chars("abc", 5), "abc");
    }

    #[test]
    fn result_candidates_add_deterministic_suffix() {
        assert_eq!(
            result_candidate_name("Photo.JPG", 1),
            ("Photo".to_string(), "jpg".to_string())
        );
        assert_eq!(
            result_candidate_name("Photo.JPG", 2),
            ("Photo (2)".to_string(), "jpg".to_string())
        );
    }

    #[test]
    fn pick_result_name_skips_existing_and_used_names_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join("photo.jpg"), b"x").unwrap();

        let mut used = HashSet::new();
        used.insert("photo (2).jpg".to_string());
        let name = pick_result_name(dir.path(), "photo.jpg", &mut used).unwrap();
        assert_eq!(name, "photo (3).jpg");
        assert!(used.contains("photo (3).jpg"));

        let next = pick_result_name(dir.path(), "photo.jpg", &mut used).unwrap();
        assert_eq!(next, "photo (4).jpg");
    }
}
