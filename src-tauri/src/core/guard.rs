use std::path::{Path, PathBuf};

use crate::core::paths::{
    has_generated_ancestor, is_same_or_child, is_unc_path, normalize_path, LIBRARY_DIR, RESULTS_DIR,
};
use crate::error::{AppError, AppResult};

pub struct SourceValidation {
    pub source: PathBuf,
}

pub fn validate_source(root: &Path, merchant_folder: &Path, source: &Path) -> AppResult<SourceValidation> {
    if is_unc_path(source) {
        return Err(AppError::user(
            "NETWORK_PATH_UNSUPPORTED",
            "暂不支持网络共享路径，请选择本地磁盘中的文件夹。",
        ));
    }
    if !source.is_dir() {
        return Err(AppError::user(
            "SOURCE_NOT_FOUND",
            format!("来源目录不存在或不是文件夹：{}", source.display()),
        ));
    }
    let source = normalize_path(source)?;
    let root = normalize_path(root)?;
    let merchant_folder = normalize_path(merchant_folder)?;

    if source.parent().is_none() {
        return Err(AppError::user(
            "SOURCE_IS_DRIVE_ROOT",
            "不能直接选择驱动器根目录作为来源。",
        ));
    }
    if is_same_or_child(&source, &root) {
        return Err(AppError::user(
            "SOURCE_IS_ROOT_OR_ANCESTOR",
            "来源不能是主文件夹本身或其上级目录。",
        ));
    }
    if is_same_or_child(&root.join(".picdedupe"), &source) {
        return Err(AppError::user(
            "SOURCE_INSIDE_CONTROL",
            "不能从程序的索引控制目录导入图片。",
        ));
    }
    if has_generated_ancestor(&source) {
        return Err(AppError::user(
            "SOURCE_INSIDE_GENERATED",
            "来源目录属于程序生成的主库或去重结果，不能再次导入。",
        ));
    }
    for generated in [LIBRARY_DIR, RESULTS_DIR] {
        let path = merchant_folder.join(generated);
        if path.exists() && is_same_or_child(&normalize_path(&path)?, &source) {
            return Err(AppError::user(
                "SOURCE_INSIDE_GENERATED",
                "来源目录不能位于当前商户的图片主库或去重结果中。",
            ));
        }
    }

    Ok(SourceValidation { source })
}
