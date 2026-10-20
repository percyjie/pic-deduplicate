use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootInspection {
    pub path: String,
    pub initialized: bool,
    pub looks_like_merchant: bool,
    pub merchant_dir_count: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MerchantSummary {
    pub id: i64,
    pub folder_name: String,
    pub display_name: String,
    pub path: String,
    pub exists: bool,
    pub image_count: u64,
    pub total_size: u64,
    pub last_batch_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootSnapshot {
    pub root_path: String,
    pub merchants: Vec<MerchantSummary>,
    pub total_images: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    InitialImport,
    Daily,
}

impl BatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InitialImport => "initial_import",
            Self::Daily => "daily",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobPhase {
    Scanning,
    Hashing,
    Comparing,
    Copying,
    Finalizing,
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub phase: JobPhase,
    pub processed: u64,
    pub total: u64,
    pub current_file: Option<String>,
    pub unique_count: u64,
    pub duplicate_existing_count: u64,
    pub duplicate_in_batch_count: u64,
    pub skipped_count: u64,
    pub error_count: u64,
    pub warning_count: u64,
    pub message: Option<String>,
}

impl Default for ProgressUpdate {
    fn default() -> Self {
        Self {
            phase: JobPhase::Scanning,
            processed: 0,
            total: 0,
            current_file: None,
            unique_count: 0,
            duplicate_existing_count: 0,
            duplicate_in_batch_count: 0,
            skipped_count: 0,
            error_count: 0,
            warning_count: 0,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: String,
    pub batch_id: i64,
    pub merchant_id: i64,
    pub phase: JobPhase,
    pub processed: u64,
    pub total: u64,
    pub current_file: Option<String>,
    pub unique_count: u64,
    pub duplicate_existing_count: u64,
    pub duplicate_in_batch_count: u64,
    pub skipped_count: u64,
    pub error_count: u64,
    pub warning_count: u64,
    pub message: Option<String>,
}

impl JobProgress {
    pub fn from_update(
        job_id: &str,
        batch_id: i64,
        merchant_id: i64,
        update: ProgressUpdate,
    ) -> Self {
        Self {
            job_id: job_id.to_string(),
            batch_id,
            merchant_id,
            phase: update.phase,
            processed: update.processed,
            total: update.total,
            current_file: update.current_file,
            unique_count: update.unique_count,
            duplicate_existing_count: update.duplicate_existing_count,
            duplicate_in_batch_count: update.duplicate_in_batch_count,
            skipped_count: update.skipped_count,
            error_count: update.error_count,
            warning_count: update.warning_count,
            message: update.message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveJobSnapshot {
    pub job_id: String,
    pub batch_id: i64,
    pub merchant_id: i64,
    pub started_at: String,
    pub progress: Option<JobProgress>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBatchResult {
    pub job_id: String,
    pub batch_id: i64,
    pub result_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub id: i64,
    pub merchant_id: i64,
    pub merchant_name: String,
    pub batch_type: String,
    pub source_abs_path: String,
    pub result_abs_path: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total_files: u64,
    pub unique_count: u64,
    pub imported_count: u64,
    pub duplicate_existing_count: u64,
    pub duplicate_in_batch_count: u64,
    pub skipped_count: u64,
    pub error_count: u64,
    pub warning_count: u64,
    pub report_path: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPage {
    pub items: Vec<BatchSummary>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchIssue {
    pub seq: i64,
    pub file_name: String,
    pub source_abs_path: String,
    pub outcome: String,
    pub error_message: Option<String>,
    pub warning_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchDetail {
    pub batch: BatchSummary,
    pub issues: Vec<BatchIssue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFinishedPayload {
    pub job_id: String,
    pub batch: BatchSummary,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFinish {
    pub batch: BatchSummary,
    pub error: Option<String>,
}
