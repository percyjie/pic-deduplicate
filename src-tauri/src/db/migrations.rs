use rusqlite::Connection;

use crate::error::{AppError, AppResult};

const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = include_str!("schema.sql");

pub fn initialize(conn: &mut Connection) -> AppResult<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -32768;",
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(AppError::user(
            "DB_VERSION_UNSUPPORTED",
            format!("索引数据库版本 {version} 高于当前程序支持的版本 {SCHEMA_VERSION}。"),
        ));
    }

    conn.execute_batch(SCHEMA)?;
    if version == 0 {
        conn.execute_batch(
            "PRAGMA application_id = 1346978628;
             PRAGMA user_version = 1;",
        )?;
    }

    let integrity: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(AppError::user(
            "DB_CORRUPTED",
            format!("索引数据库完整性检查失败：{integrity}"),
        ));
    }

    Ok(())
}
