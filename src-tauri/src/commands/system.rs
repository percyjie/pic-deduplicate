use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::core::paths::{is_same_or_child, LIBRARY_DIR};
use crate::db::queries;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[tauri::command]
pub fn open_result_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    batch_id: i64,
) -> AppResult<()> {
    let context = state.session_context()?;
    let path = {
        let db = context
            .db
            .lock()
            .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
        let batch = queries::get_batch_summary(&db, &context.root, batch_id)?
            .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录不存在。"))?;
        PathBuf::from(batch.result_abs_path)
    };
    open_inside_root(&app, &context.root, &path, true)
}

#[tauri::command]
pub fn open_report(
    app: AppHandle,
    state: State<'_, AppState>,
    batch_id: i64,
) -> AppResult<()> {
    let context = state.session_context()?;
    let path = {
        let db = context
            .db
            .lock()
            .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
        let batch = queries::get_batch_summary(&db, &context.root, batch_id)?
            .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录不存在。"))?;
        batch.report_path.map(PathBuf::from).ok_or_else(|| {
            AppError::user(
                "REPORT_NOT_FOUND",
                "该批次还没有报告，请在历史记录中重新生成。",
            )
        })?
    };
    open_inside_root(&app, &context.root, &path, false)
}

#[tauri::command]
pub fn open_library(
    app: AppHandle,
    state: State<'_, AppState>,
    merchant_id: i64,
) -> AppResult<()> {
    let context = state.session_context()?;
    let path = {
        let db = context
            .db
            .lock()
            .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
        let merchant = queries::find_merchant(&db, merchant_id)?;
        context.root.join(merchant.folder_name).join(LIBRARY_DIR)
    };
    if !path.is_dir() {
        return Err(AppError::user(
            "LIBRARY_NOT_FOUND",
            "该商户还没有图片主库，请先执行一次导入。",
        ));
    }
    open_inside_root(&app, &context.root, &path, true)
}

fn open_inside_root(
    app: &AppHandle,
    root: &Path,
    path: &Path,
    expect_dir: bool,
) -> AppResult<()> {
    if !is_same_or_child(root, path) {
        return Err(AppError::user(
            "PATH_OUTSIDE_ROOT",
            "目标路径不在当前主文件夹内，已拒绝打开。",
        ));
    }
    if expect_dir && !path.is_dir() {
        return Err(AppError::user(
            "PATH_NOT_FOUND",
            format!("目录不存在：{}", path.display()),
        ));
    }
    if !expect_dir && !path.is_file() {
        return Err(AppError::user(
            "PATH_NOT_FOUND",
            format!("文件不存在：{}", path.display()),
        ));
    }
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| AppError::user("OPEN_FAILED", format!("无法打开路径：{error}")))
}
