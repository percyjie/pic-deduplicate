use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{message}")]
    User { code: String, message: String },
    #[error("任务已取消")]
    Cancelled,
    #[error("文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据库操作失败：{0}")]
    Db(#[from] rusqlite::Error),
    #[error("数据格式错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("报告写入失败：{0}")]
    Csv(#[from] csv::Error),
}

impl AppError {
    pub fn user(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::User {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> String {
        match self {
            Self::User { code, .. } => code.clone(),
            Self::Cancelled => "CANCELLED".into(),
            Self::Io(_) => "IO_ERROR".into(),
            Self::Db(_) => "DB_ERROR".into(),
            Self::Json(_) => "JSON_ERROR".into(),
            Self::Csv(_) => "REPORT_ERROR".into(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", &self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;
