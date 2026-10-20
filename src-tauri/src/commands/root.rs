use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Local;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::core::paths::{
    is_same_or_child, is_unc_path, normalize_path, remove_stale_temp_files, set_hidden,
    RootPaths, CONTROL_DIR, LIBRARY_DIR, RESULTS_DIR,
};
use crate::db::{self, queries};
use crate::error::{AppError, AppResult};
use crate::models::{MerchantSummary, RootInspection, RootSnapshot};
use crate::state::{AppState, RootContext, RootSession};

const ROOT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct RootMarker {
    app: String,
    format_version: u32,
    root_id: String,
    created_at: String,
}

#[tauri::command]
pub fn inspect_root(path: String) -> AppResult<RootInspection> {
    let requested = PathBuf::from(path);
    if !requested.exists() {
        return Err(AppError::user("PATH_NOT_FOUND", "所选主文件夹不存在。"));
    }
    if !requested.is_dir() {
        return Err(AppError::user("PATH_NOT_DIRECTORY", "所选路径不是文件夹。"));
    }
    if is_unc_path(&requested) {
        return Err(AppError::user(
            "NETWORK_PATH_UNSUPPORTED",
            "暂不支持网络共享路径，请选择本地磁盘文件夹。",
        ));
    }
    let root = normalize_path(&requested)?;
    let paths = RootPaths::from_root(&root);
    let looks_like_merchant = root.join(LIBRARY_DIR).is_dir() || root.join(RESULTS_DIR).is_dir();
    let initialized = paths.marker.is_file() && paths.db.is_file();
    let merchant_dir_count = top_level_merchant_dirs(&root)
        .map(|items| items.len())
        .unwrap_or(0);

    let message = if looks_like_merchant {
        Some("这个目录看起来是商户目录，请选择包含商户文件夹的上级主文件夹。".to_string())
    } else if initialized {
        None
    } else if paths.control.exists() {
        Some("检测到未完成的控制目录，请确认后重新初始化或选择其他文件夹。".to_string())
    } else {
        Some("首次使用会在该目录创建隐藏索引库，不会修改现有图片。".to_string())
    };

    Ok(RootInspection {
        path: root.to_string_lossy().to_string(),
        initialized,
        looks_like_merchant,
        merchant_dir_count,
        message,
    })
}

#[tauri::command]
pub fn initialize_root(state: State<'_, AppState>, path: String) -> AppResult<RootSnapshot> {
    open_or_initialize(&state, &path, true)
}

#[tauri::command]
pub fn open_root(state: State<'_, AppState>, path: String) -> AppResult<RootSnapshot> {
    open_or_initialize(&state, &path, false)
}

#[tauri::command]
pub fn get_root_snapshot(state: State<'_, AppState>) -> AppResult<RootSnapshot> {
    let context = state.session_context()?;
    build_snapshot(&context)
}

#[tauri::command]
pub fn refresh_merchants(state: State<'_, AppState>) -> AppResult<RootSnapshot> {
    get_root_snapshot(state)
}

#[tauri::command]
pub fn close_root(state: State<'_, AppState>) -> AppResult<()> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    if inner.active_job.is_some() || inner.starting {
        return Err(AppError::user(
            "JOB_ALREADY_RUNNING",
            "任务执行中，不能关闭主文件夹。",
        ));
    }
    inner.session = None;
    Ok(())
}

#[tauri::command]
pub fn create_merchant(state: State<'_, AppState>, name: String) -> AppResult<MerchantSummary> {
    let context = state.session_context()?;
    let inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    if inner.active_job.is_some() || inner.starting {
        return Err(AppError::user(
            "JOB_ALREADY_RUNNING",
            "任务执行中，不能创建商户。",
        ));
    }
    drop(inner);

    crate::core::naming::validate_folder_name(&name)?;
    if name.starts_with('.') {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "商户名称不能以点开头。",
        ));
    }
    if [CONTROL_DIR, LIBRARY_DIR, RESULTS_DIR].contains(&name.as_str()) {
        return Err(AppError::user(
            "INVALID_MERCHANT_NAME",
            "该名称是程序保留名称，请换一个名称。",
        ));
    }

    let merchant_path = context.root.join(&name);
    if merchant_path.exists() {
        return Err(AppError::user(
            "MERCHANT_EXISTS",
            "该商户文件夹已经存在。",
        ));
    }
    fs::create_dir(&merchant_path)?;
    let now = Local::now().to_rfc3339();
    let db = context
        .db
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
    let id = match queries::insert_merchant(&db, &name, &now) {
        Ok(id) => id,
        Err(error) => {
            let _ = fs::remove_dir(&merchant_path);
            return Err(error);
        }
    };
    Ok(MerchantSummary {
        id,
        folder_name: name.clone(),
        display_name: name,
        path: merchant_path.to_string_lossy().to_string(),
        exists: true,
        image_count: 0,
        total_size: 0,
        last_batch_at: None,
    })
}

pub fn build_snapshot(context: &RootContext) -> AppResult<RootSnapshot> {
    let merchants = sync_merchants(context)?;
    let total_images = merchants.iter().map(|merchant| merchant.image_count).sum();
    let total_size = merchants.iter().map(|merchant| merchant.total_size).sum();
    Ok(RootSnapshot {
        root_path: context.root.to_string_lossy().to_string(),
        merchants,
        total_images,
        total_size,
    })
}

pub fn sync_merchants(context: &RootContext) -> AppResult<Vec<MerchantSummary>> {
    let now = Local::now().to_rfc3339();
    let db = context
        .db
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
    for folder in top_level_merchant_dirs(&context.root)? {
        queries::insert_merchant_if_missing(&db, &folder, &now)?;
    }
    queries::merchant_summaries(&db, &context.root)
}

fn open_or_initialize(
    state: &State<'_, AppState>,
    requested: &str,
    allow_create: bool,
) -> AppResult<RootSnapshot> {
    let context = prepare_root_path(Path::new(requested))?;
    let paths = RootPaths::from_root(&context);
    let initialized = paths.marker.is_file() && paths.db.is_file();

    let mut inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    if inner.active_job.is_some() || inner.starting {
        return Err(AppError::user(
            "JOB_ALREADY_RUNNING",
            "任务执行中，不能切换主文件夹。",
        ));
    }

    if let Some(existing) = &inner.session {
        if existing.context.root == context {
            return build_snapshot(&existing.context);
        }
    }

    let session = if initialized {
        open_existing_root(&context, &paths)?
    } else {
        if !allow_create {
            return Err(AppError::user(
                "ROOT_NOT_INITIALIZED",
                "该主文件夹尚未初始化，请先执行初始化。",
            ));
        }
        if paths.control.exists() {
            return Err(AppError::user(
                "ROOT_PARTIAL_STATE",
                format!(
                    "控制目录 {} 已存在但不完整。请检查该目录，或选择其他主文件夹。",
                    paths.control.display()
                ),
            ));
        }
        initialize_new_root(&context, &paths)?
    };
    let snapshot = build_snapshot(&session.context)?;
    inner.session = Some(session);
    Ok(snapshot)
}

fn prepare_root_path(requested: &Path) -> AppResult<PathBuf> {
    if !requested.exists() {
        return Err(AppError::user("PATH_NOT_FOUND", "所选主文件夹不存在。"));
    }
    if !requested.is_dir() {
        return Err(AppError::user("PATH_NOT_DIRECTORY", "所选路径不是文件夹。"));
    }
    if is_unc_path(requested) {
        return Err(AppError::user(
            "NETWORK_PATH_UNSUPPORTED",
            "暂不支持网络共享路径，请选择本地磁盘文件夹。",
        ));
    }
    let root = normalize_path(requested)?;
    if root.parent().is_none() {
        return Err(AppError::user(
            "ROOT_IS_DRIVE",
            "不能把驱动器根目录作为主文件夹，请选择专用文件夹。",
        ));
    }
    if root.join(LIBRARY_DIR).is_dir() || root.join(RESULTS_DIR).is_dir() {
        return Err(AppError::user(
            "ROOT_LOOKS_LIKE_MERCHANT",
            "该目录看起来是商户目录，请选择包含商户文件夹的上级主文件夹。",
        ));
    }
    Ok(root)
}

fn initialize_new_root(root: &Path, paths: &RootPaths) -> AppResult<RootSession> {
    fs::create_dir(&paths.control)?;
    set_hidden(&paths.control);
    fs::create_dir_all(&paths.backups)?;

    let now = Local::now().to_rfc3339();
    let root_id = Uuid::new_v4().to_string();
    let marker = RootMarker {
        app: "pic-deduplicate".to_string(),
        format_version: ROOT_FORMAT_VERSION,
        root_id: root_id.clone(),
        created_at: now.clone(),
    };
    write_marker(paths, &marker)?;
    let lock_file = acquire_root_lock(paths)?;
    let mut conn = db::open_database(&paths.db)?;
    let tx = conn.transaction()?;
    queries::set_meta(&tx, "root_id", &root_id)?;
    queries::set_meta(&tx, "created_at", &now)?;
    queries::set_meta(&tx, "app_format_version", &ROOT_FORMAT_VERSION.to_string())?;
    tx.commit()?;

    let context = RootContext {
        root: root.to_path_buf(),
        control_dir: paths.control.clone(),
        db: Arc::new(Mutex::new(conn)),
    };
    sync_merchants(&context)?;
    Ok(RootSession {
        context,
        _lock_file: Arc::new(lock_file),
    })
}

fn open_existing_root(root: &Path, paths: &RootPaths) -> AppResult<RootSession> {
    if !paths.db.is_file() {
        return Err(AppError::user(
            "DB_MISSING",
            format!(
                "索引数据库不存在：{}。请不要删除 .picdedupe 目录。",
                paths.db.display()
            ),
        ));
    }
    let marker_text = fs::read_to_string(&paths.marker).map_err(|error| {
        AppError::user(
            "ROOT_MARKER_INVALID",
            format!("无法读取主文件夹标识：{error}"),
        )
    })?;
    let marker: RootMarker = serde_json::from_str(&marker_text).map_err(|error| {
        AppError::user(
            "ROOT_MARKER_INVALID",
            format!("主文件夹标识已损坏：{error}"),
        )
    })?;
    if marker.app != "pic-deduplicate" || marker.format_version != ROOT_FORMAT_VERSION {
        return Err(AppError::user(
            "ROOT_FORMAT_UNSUPPORTED",
            "该主文件夹由不兼容的程序版本创建。",
        ));
    }

    let lock_file = acquire_root_lock(paths)?;
    let db = db::open_database(&paths.db)?;
    let stored_root_id = queries::get_meta(&db, "root_id")?;
    if stored_root_id.as_deref() != Some(marker.root_id.as_str()) {
        return Err(AppError::user(
            "ROOT_ID_MISMATCH",
            "主文件夹标识与索引数据库不匹配，请勿混用不同主文件夹的 .picdedupe 目录。",
        ));
    }
    queries::interrupt_running_batches(&db)?;
    fs::create_dir_all(&paths.backups)?;

    let context = RootContext {
        root: root.to_path_buf(),
        control_dir: paths.control.clone(),
        db: Arc::new(Mutex::new(db)),
    };
    sync_merchants(&context)?;
    cleanup_stale_temp_files(root);
    Ok(RootSession {
        context,
        _lock_file: Arc::new(lock_file),
    })
}

fn cleanup_stale_temp_files(root: &Path) {
    let Ok(merchants) = top_level_merchant_dirs(root) else {
        return;
    };
    for merchant in merchants {
        for reserved in [LIBRARY_DIR, RESULTS_DIR] {
            let _ = remove_stale_temp_files(&root.join(&merchant).join(reserved));
        }
    }
}

fn acquire_root_lock(paths: &RootPaths) -> AppResult<File> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock)?;
    file.try_lock_exclusive().map_err(|_| {
        AppError::user(
            "ROOT_BUSY",
            "该主文件夹正在被另一个程序窗口使用。",
        )
    })?;
    Ok(file)
}

fn write_marker(paths: &RootPaths, marker: &RootMarker) -> AppResult<()> {
    let data = serde_json::to_vec_pretty(marker)?;
    let temp = paths.control.join(".marker.tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    fs::rename(temp, &paths.marker)?;
    Ok(())
}

fn top_level_merchant_dirs(root: &Path) -> AppResult<Vec<String>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|value| value.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        if name.starts_with('.') || name == LIBRARY_DIR || name == RESULTS_DIR {
            continue;
        }
        if is_hidden_or_reparse(&path) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        result.push(name);
    }
    result.sort_by_key(|name| name.to_lowercase());
    Ok(result)
}

#[cfg(windows)]
fn is_hidden_or_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(path)
        .map(|metadata| {
            metadata.file_attributes()
                & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden_or_reparse(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

#[allow(dead_code)]
fn is_inside_root(root: &Path, path: &Path) -> bool {
    is_same_or_child(root, path)
}
