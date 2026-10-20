use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::core::pipeline::{self, PreparedBatch};
use crate::db::queries;
use crate::error::{AppError, AppResult};
use crate::models::{
    ActiveJobSnapshot, BatchDetail, BatchKind, BatchPage, BatchSummary, JobFinishedPayload,
    JobProgress, ProgressUpdate, StartBatchResult,
};
use crate::state::{ActiveJob, AppState, RootContext};

#[tauri::command]
pub async fn start_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    merchant_id: i64,
    source_dir: String,
    kind: BatchKind,
) -> AppResult<StartBatchResult> {
    let context = reserve_start(&state)?;
    let source = PathBuf::from(source_dir);
    let prepare_context = context.clone();
    let prepared = match tauri::async_runtime::spawn_blocking(move || {
        pipeline::prepare_batch(&prepare_context, merchant_id, &source, kind)
    })
    .await
    {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            release_start(&state);
            return Err(error);
        }
        Err(error) => {
            release_start(&state);
            return Err(AppError::user(
                "TASK_PANIC",
                format!("启动任务失败：{error}"),
            ));
        }
    };

    let job_id = Uuid::new_v4().to_string();
    let active = ActiveJob {
        job_id: job_id.clone(),
        batch_id: prepared.batch_id,
        merchant_id: prepared.merchant_id,
        started_at: Local::now().to_rfc3339(),
        cancel: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(Mutex::new(None)),
    };
    {
        let mut inner = state
            .inner
            .lock()
            .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
        inner.starting = false;
        inner.active_job = Some(active.clone());
    }

    let initial = JobProgress::from_update(
        &job_id,
        prepared.batch_id,
        prepared.merchant_id,
        ProgressUpdate {
            message: Some("任务已创建，正在启动…".to_string()),
            ..Default::default()
        },
    );
    if let Ok(mut slot) = active.progress.lock() {
        *slot = Some(initial.clone());
    }
    let _ = app.emit("job-progress", initial);

    let result = StartBatchResult {
        job_id,
        batch_id: prepared.batch_id,
        result_dir: prepared.result_dir.to_string_lossy().to_string(),
    };
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_job(worker_app, context, prepared, active);
    });
    Ok(result)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> AppResult<()> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    match &inner.active_job {
        Some(job) if job.job_id == job_id => {
            job.cancel.store(true, Ordering::SeqCst);
            Ok(())
        }
        _ => Err(AppError::user(
            "JOB_NOT_RUNNING",
            "当前没有可取消的任务。",
        )),
    }
}

#[tauri::command]
pub fn get_active_job(state: State<'_, AppState>) -> AppResult<Option<ActiveJobSnapshot>> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    Ok(inner.active_job.as_ref().map(ActiveJob::snapshot))
}

#[tauri::command]
pub fn list_batches(
    state: State<'_, AppState>,
    merchant_id: Option<i64>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> AppResult<BatchPage> {
    let context = state.session_context()?;
    let db = context
        .db
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
    queries::list_batches(
        &db,
        &context.root,
        merchant_id,
        page.unwrap_or(1),
        page_size.unwrap_or(20),
    )
}

#[tauri::command]
pub fn get_batch_detail(
    state: State<'_, AppState>,
    batch_id: i64,
) -> AppResult<BatchDetail> {
    let context = state.session_context()?;
    let db = context
        .db
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "索引库状态异常。"))?;
    let batch = queries::get_batch_summary(&db, &context.root, batch_id)?
        .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录不存在。"))?;
    let issues = queries::batch_issues(&db, batch_id)?;
    Ok(BatchDetail { batch, issues })
}

#[tauri::command]
pub async fn regenerate_report(
    state: State<'_, AppState>,
    batch_id: i64,
) -> AppResult<BatchSummary> {
    ensure_idle(&state)?;
    let context = state.session_context()?;
    tauri::async_runtime::spawn_blocking(move || {
        pipeline::regenerate_report(&context, batch_id)
    })
    .await
    .map_err(|error| {
        AppError::user("TASK_PANIC", format!("重新生成报告失败：{error}"))
    })?
}

fn reserve_start(state: &State<'_, AppState>) -> AppResult<RootContext> {
    let mut inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    if inner.starting || inner.active_job.is_some() {
        return Err(AppError::user(
            "JOB_ALREADY_RUNNING",
            "已有任务正在执行，请等待完成或先取消。",
        ));
    }
    let context = inner
        .session
        .as_ref()
        .map(|session| session.context.clone())
        .ok_or_else(|| AppError::user("ROOT_NOT_OPEN", "请先选择并打开主文件夹。"))?;
    inner.starting = true;
    Ok(context)
}

fn release_start(state: &State<'_, AppState>) {
    if let Ok(mut inner) = state.inner.lock() {
        inner.starting = false;
    }
}

fn ensure_idle(state: &State<'_, AppState>) -> AppResult<()> {
    let inner = state
        .inner
        .lock()
        .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
    if inner.starting || inner.active_job.is_some() {
        return Err(AppError::user(
            "JOB_ALREADY_RUNNING",
            "任务执行中，暂不能执行该操作。",
        ));
    }
    Ok(())
}

fn run_job(
    app: AppHandle,
    context: RootContext,
    prepared: PreparedBatch,
    active: ActiveJob,
) {
    let job_id = active.job_id.clone();
    let _guard = ActiveJobGuard {
        app: app.clone(),
        job_id: job_id.clone(),
    };

    let last_emit = Mutex::new(Instant::now() - Duration::from_secs(1));
    let progress = |update: ProgressUpdate| {
        let snapshot = JobProgress::from_update(
            &active.job_id,
            active.batch_id,
            active.merchant_id,
            update,
        );
        if let Ok(mut slot) = active.progress.lock() {
            *slot = Some(snapshot.clone());
        }
        let should_emit = last_emit
            .lock()
            .map(|mut at| {
                if at.elapsed() >= Duration::from_millis(100) {
                    *at = Instant::now();
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if should_emit {
            let _ = app.emit("job-progress", snapshot);
        }
    };

    let outcome = pipeline::run_prepared(&context, &prepared, active.cancel.clone(), &progress);

    clear_active_job(&app, &job_id);
    match outcome {
        Ok(finish) => {
            let payload = JobFinishedPayload {
                job_id: job_id.clone(),
                batch: finish.batch,
                error: finish.error,
            };
            let _ = app.emit("job-finished", payload);
        }
        Err(error) => {
            let payload = serde_json::json!({
                "jobId": job_id,
                "error": {
                    "code": error.code(),
                    "message": error.to_string(),
                }
            });
            let _ = app.emit("job-failed", payload);
        }
    }
}

fn clear_active_job(app: &AppHandle, job_id: &str) {
    let state = app.state::<AppState>();
    if let Ok(mut inner) = state.inner.lock() {
        let is_current = inner
            .active_job
            .as_ref()
            .map(|job| job.job_id == job_id)
            .unwrap_or(false);
        if is_current {
            inner.active_job = None;
        }
    }
}

struct ActiveJobGuard {
    app: AppHandle,
    job_id: String,
}

impl Drop for ActiveJobGuard {
    fn drop(&mut self) {
        clear_active_job(&self.app, &self.job_id);
    }
}
