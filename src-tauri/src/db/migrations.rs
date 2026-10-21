use rusqlite::Connection;

use crate::error::{AppError, AppResult};

const SCHEMA_VERSION: i64 = 2;
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
    if version < 2 {
        migrate_to_v2(conn)?;
    }
    if version == 0 {
        conn.execute_batch("PRAGMA application_id = 1346978628;")?;
    }
    if version < SCHEMA_VERSION {
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
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

fn migrate_to_v2(conn: &mut Connection) -> AppResult<()> {
    let tx = conn.transaction()?;
    add_column_if_missing(
        &tx,
        "batches",
        "match_mode",
        "TEXT NOT NULL DEFAULT 'exact' CHECK (match_mode IN ('exact', 'visual'))",
    )?;
    add_column_if_missing(
        &tx,
        "batches",
        "visual_threshold",
        "INTEGER NOT NULL DEFAULT 8 CHECK (visual_threshold BETWEEN 0 AND 24)",
    )?;
    add_column_if_missing(
        &tx,
        "batch_items",
        "duplicate_kind",
        "TEXT CHECK (duplicate_kind IS NULL OR duplicate_kind IN ('exact', 'visual'))",
    )?;
    add_column_if_missing(
        &tx,
        "batch_items",
        "visual_distance",
        "INTEGER CHECK (visual_distance IS NULL OR (visual_distance BETWEEN 0 AND 64))",
    )?;
    tx.commit()?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> AppResult<()> {
    if column_exists(conn, table, column)? {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
