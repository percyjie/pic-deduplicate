use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};
use crate::models::{ActiveJobSnapshot, JobProgress};

#[derive(Clone)]
pub struct RootContext {
    pub root: PathBuf,
    pub control_dir: PathBuf,
    pub db: Arc<Mutex<Connection>>,
}

pub struct RootSession {
    pub context: RootContext,
    pub _lock_file: Arc<File>,
}

#[derive(Clone)]
pub struct ActiveJob {
    pub job_id: String,
    pub batch_id: i64,
    pub merchant_id: i64,
    pub started_at: String,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<Mutex<Option<JobProgress>>>,
}

impl ActiveJob {
    pub fn snapshot(&self) -> ActiveJobSnapshot {
        ActiveJobSnapshot {
            job_id: self.job_id.clone(),
            batch_id: self.batch_id,
            merchant_id: self.merchant_id,
            started_at: self.started_at.clone(),
            progress: self.progress.lock().ok().and_then(|value| value.clone()),
        }
    }
}

#[derive(Default)]
pub struct AppInner {
    pub session: Option<RootSession>,
    pub active_job: Option<ActiveJob>,
    pub starting: bool,
}

#[derive(Default)]
pub struct AppState {
    pub inner: Mutex<AppInner>,
}

impl AppState {
    pub fn session_context(&self) -> AppResult<RootContext> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| AppError::user("STATE_POISONED", "应用状态异常，请重启程序。"))?;
        inner
            .session
            .as_ref()
            .map(|session| session.context.clone())
            .ok_or_else(|| AppError::user("ROOT_NOT_OPEN", "请先选择并打开主文件夹。"))
    }
}
