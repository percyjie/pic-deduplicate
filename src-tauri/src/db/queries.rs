use std::collections::HashMap;
use std::path::Path;

use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{AppError, AppResult};
use crate::models::{BatchIssue, BatchPage, BatchSummary, MerchantSummary};

#[derive(Debug, Clone)]
pub struct StoredMerchant {
    pub id: i64,
    pub folder_name: String,
}

#[derive(Debug, Clone)]
pub struct LibraryImage {
    pub id: i64,
    pub size_bytes: i64,
    pub library_rel_path: String,
    pub original_name: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct BatchCounts {
    pub total_files: u64,
    pub unique_count: u64,
    pub imported_count: u64,
    pub duplicate_existing_count: u64,
    pub duplicate_in_batch_count: u64,
    pub skipped_count: u64,
    pub error_count: u64,
    pub warning_count: u64,
}

impl BatchCounts {
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }
}

pub struct NewBatchItem {
    pub seq: i64,
    pub source_abs_path: String,
    pub source_rel_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
    pub sha256: Option<String>,
    pub outcome: String,
    pub duplicate_of_image_id: Option<i64>,
    pub duplicate_of_seq: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub warning_code: Option<String>,
    pub warning_message: Option<String>,
}

pub struct NewLibraryImage {
    pub merchant_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub library_rel_path: String,
    pub original_name: String,
    pub original_source_path: String,
    pub imported_at: String,
    pub batch_id: i64,
}

#[derive(Debug, Clone)]
pub struct BatchReportItem {
    pub seq: i64,
    pub file_name: String,
    pub source_abs_path: String,
    pub source_rel_path: String,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub outcome: String,
    pub library_rel_path: Option<String>,
    pub result_rel_path: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub warning_code: Option<String>,
    pub warning_message: Option<String>,
    pub duplicate_image_name: Option<String>,
    pub duplicate_image_path: Option<String>,
    pub duplicate_item_name: Option<String>,
    pub duplicate_item_source: Option<String>,
}

const BATCH_SELECT: &str = "
SELECT b.id, b.merchant_id, m.folder_name, b.batch_type, b.source_abs_path,
       b.result_rel_path, b.status, b.started_at, b.finished_at, b.total_files,
       b.unique_count, b.imported_count, b.duplicate_existing_count,
       b.duplicate_in_batch_count, b.skipped_count, b.error_count,
       b.warning_count, b.report_rel_path, b.note
FROM batches b
JOIN merchants m ON m.id = b.merchant_id";

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
        .optional()?)
}

pub fn insert_merchant(conn: &Connection, folder_name: &str, now: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO merchants (folder_name, display_name, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?2)",
        params![folder_name, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_merchant_if_missing(
    conn: &Connection,
    folder_name: &str,
    now: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO merchants (folder_name, display_name, created_at, updated_at)
         VALUES (?1, ?1, ?2, ?2)",
        params![folder_name, now],
    )?;
    Ok(())
}

pub fn find_merchant(conn: &Connection, id: i64) -> AppResult<StoredMerchant> {
    conn.query_row(
        "SELECT id, folder_name FROM merchants WHERE id = ?1",
        params![id],
        |row| {
            Ok(StoredMerchant {
                id: row.get(0)?,
                folder_name: row.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AppError::user("MERCHANT_NOT_FOUND", "所选商户不存在。"))
}

pub fn merchant_summaries(conn: &Connection, root: &Path) -> AppResult<Vec<MerchantSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name,
                COUNT(i.id) AS image_count,
                COALESCE(SUM(i.size_bytes), 0) AS total_size,
                (SELECT MAX(b.started_at) FROM batches b WHERE b.merchant_id = m.id) AS last_batch
         FROM merchants m
         LEFT JOIN images i ON i.merchant_id = m.id
         GROUP BY m.id, m.folder_name
         ORDER BY m.folder_name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |row| {
        let folder_name: String = row.get(1)?;
        let path = root.join(&folder_name);
        Ok(MerchantSummary {
            id: row.get(0)?,
            folder_name: folder_name.clone(),
            display_name: folder_name,
            path: path.to_string_lossy().to_string(),
            exists: path.is_dir(),
            image_count: to_u64(row.get::<_, i64>(2)?),
            total_size: to_u64(row.get::<_, i64>(3)?),
            last_batch_at: row.get(4)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn insert_running_batch(
    conn: &Connection,
    merchant_id: i64,
    batch_type: &str,
    source_abs_path: &str,
    result_rel_path: &str,
    import_month: &str,
    started_at: &str,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO batches (
            merchant_id, batch_type, source_abs_path, result_rel_path, status,
            import_month, started_at
         ) VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6)",
        params![
            merchant_id,
            batch_type,
            source_abs_path,
            result_rel_path,
            import_month,
            started_at
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn interrupt_running_batches(conn: &Connection) -> AppResult<()> {
    let finished_at = Local::now().to_rfc3339();
    conn.execute(
        "UPDATE batches
         SET status = 'interrupted',
             finished_at = ?1,
             note = TRIM(COALESCE(note, '') || ' 上次运行未正常结束，批次已标记为中断。'),
             total_files = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id),
             unique_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND outcome = 'unique'),
             duplicate_existing_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND outcome = 'duplicate_existing'),
             duplicate_in_batch_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND outcome = 'duplicate_in_batch'),
             skipped_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND outcome = 'skipped'),
             error_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND (outcome = 'error' OR error_code IS NOT NULL)),
             warning_count = (SELECT COUNT(*) FROM batch_items WHERE batch_id = batches.id AND warning_code IS NOT NULL),
             imported_count = (SELECT COUNT(*) FROM images WHERE first_batch_id = batches.id)
         WHERE status = 'running'",
        params![finished_at],
    )?;
    Ok(())
}

pub fn get_batch_summary(
    conn: &Connection,
    root: &Path,
    batch_id: i64,
) -> AppResult<Option<BatchSummary>> {
    let sql = format!("{BATCH_SELECT} WHERE b.id = ?1");
    conn.query_row(&sql, params![batch_id], |row| {
        row_to_batch_summary(row, root)
    })
    .optional()
    .map_err(AppError::from)
}

pub fn list_batches(
    conn: &Connection,
    root: &Path,
    merchant_id: Option<i64>,
    page: u32,
    page_size: u32,
) -> AppResult<BatchPage> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let offset = (page - 1) as i64 * page_size as i64;

    let (total, sql) = match merchant_id {
        Some(id) => (
            conn.query_row(
                "SELECT COUNT(*) FROM batches WHERE merchant_id = ?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )?,
            format!(
                "{BATCH_SELECT} WHERE b.merchant_id = ?1 ORDER BY b.started_at DESC LIMIT ?2 OFFSET ?3"
            ),
        ),
        None => (
            conn.query_row("SELECT COUNT(*) FROM batches", [], |row| row.get::<_, i64>(0))?,
            format!("{BATCH_SELECT} ORDER BY b.started_at DESC LIMIT ?1 OFFSET ?2"),
        ),
    };

    let mut items = Vec::new();
    match merchant_id {
        Some(id) => {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![id, page_size, offset], |row| {
                row_to_batch_summary(row, root)
            })?;
            for row in rows {
                items.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![page_size, offset], |row| {
                row_to_batch_summary(row, root)
            })?;
            for row in rows {
                items.push(row?);
            }
        }
    }

    Ok(BatchPage {
        items,
        total: to_u64(total),
        page,
        page_size,
    })
}

pub fn batch_issues(conn: &Connection, batch_id: i64) -> AppResult<Vec<BatchIssue>> {
    let mut stmt = conn.prepare(
        "SELECT seq, file_name, source_abs_path, outcome, error_message, warning_message
         FROM batch_items
         WHERE batch_id = ?1
           AND (outcome = 'error' OR error_message IS NOT NULL OR warning_message IS NOT NULL)
         ORDER BY seq
         LIMIT 300",
    )?;
    let rows = stmt.query_map(params![batch_id], |row| {
        Ok(BatchIssue {
            seq: row.get(0)?,
            file_name: row.get(1)?,
            source_abs_path: row.get(2)?,
            outcome: row.get(3)?,
            error_message: row.get(4)?,
            warning_message: row.get(5)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn load_library_images(
    conn: &Connection,
    merchant_id: i64,
) -> AppResult<HashMap<String, LibraryImage>> {
    let mut stmt = conn.prepare(
        "SELECT id, sha256, size_bytes, library_rel_path, original_name, imported_at
         FROM images WHERE merchant_id = ?1",
    )?;
    let rows = stmt.query_map(params![merchant_id], |row| {
        Ok((
            row.get::<_, String>(1)?,
            LibraryImage {
                id: row.get(0)?,
                size_bytes: row.get(2)?,
                library_rel_path: row.get(3)?,
                original_name: row.get(4)?,
                imported_at: row.get(5)?,
            },
        ))
    })?;
    let mut result = HashMap::new();
    for row in rows {
        let (hash, image) = row?;
        result.insert(hash, image);
    }
    Ok(result)
}

pub fn insert_batch_items(
    conn: &mut Connection,
    batch_id: i64,
    items: &[NewBatchItem],
) -> AppResult<Vec<i64>> {
    let tx = conn.transaction()?;
    let mut ids = Vec::with_capacity(items.len());
    let mut id_by_seq = HashMap::<i64, i64>::new();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO batch_items (
                batch_id, seq, source_abs_path, source_rel_path, file_name, size_bytes,
                mtime_ms, sha256, outcome, duplicate_of_image_id, duplicate_of_item_id,
                error_code, error_message, warning_code, warning_message
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;
        for item in items {
            let duplicate_item_id = match item.duplicate_of_seq {
                Some(seq) => Some(id_by_seq.get(&seq).copied().ok_or_else(|| {
                    AppError::user("INTERNAL_ERROR", "批内重复条目关联失败。")
                })?),
                None => None,
            };
            stmt.execute(params![
                batch_id,
                item.seq,
                item.source_abs_path,
                item.source_rel_path,
                item.file_name,
                item.size_bytes,
                item.mtime_ms,
                item.sha256,
                item.outcome,
                item.duplicate_of_image_id,
                duplicate_item_id,
                item.error_code,
                item.error_message,
                item.warning_code,
                item.warning_message,
            ])?;
            let id = tx.last_insert_rowid();
            id_by_seq.insert(item.seq, id);
            ids.push(id);
        }
    }
    tx.commit()?;
    Ok(ids)
}

pub fn commit_library_image(
    conn: &mut Connection,
    image: &NewLibraryImage,
    batch_item_id: i64,
) -> AppResult<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO images (
            merchant_id, sha256, size_bytes, library_rel_path, original_name,
            original_source_path, imported_at, first_batch_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            image.merchant_id,
            image.sha256,
            image.size_bytes,
            image.library_rel_path,
            image.original_name,
            image.original_source_path,
            image.imported_at,
            image.batch_id,
        ],
    )?;
    let image_id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE batch_items SET library_rel_path = ?2 WHERE id = ?1",
        params![batch_item_id, image.library_rel_path],
    )?;
    tx.commit()?;
    Ok(image_id)
}

pub fn set_item_result(
    conn: &Connection,
    batch_item_id: i64,
    result_rel_path: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE batch_items SET result_rel_path = ?2 WHERE id = ?1",
        params![batch_item_id, result_rel_path],
    )?;
    Ok(())
}

pub fn set_item_warning(
    conn: &Connection,
    batch_item_id: i64,
    code: &str,
    message: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE batch_items
         SET warning_code = ?2, warning_message = ?3
         WHERE id = ?1",
        params![batch_item_id, code, message],
    )?;
    Ok(())
}

pub fn set_item_error(
    conn: &Connection,
    batch_item_id: i64,
    code: &str,
    message: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE batch_items
         SET outcome = 'error', error_code = ?2, error_message = ?3
         WHERE id = ?1",
        params![batch_item_id, code, message],
    )?;
    Ok(())
}

pub fn set_batch_report_paths(
    conn: &Connection,
    batch_id: i64,
    report_rel_path: Option<&str>,
    summary_rel_path: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE batches SET report_rel_path = ?2, summary_rel_path = ?3 WHERE id = ?1",
        params![batch_id, report_rel_path, summary_rel_path],
    )?;
    Ok(())
}

pub fn compute_batch_counts(conn: &Connection, batch_id: i64) -> AppResult<BatchCounts> {
    let (total, unique, duplicate_existing, duplicate_in_batch, skipped, error, warning) =
        conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN outcome = 'unique' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN outcome = 'duplicate_existing' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN outcome = 'duplicate_in_batch' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN outcome = 'skipped' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN outcome = 'error' OR error_code IS NOT NULL THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN warning_code IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM batch_items WHERE batch_id = ?1",
            params![batch_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
    let imported: i64 = conn.query_row(
        "SELECT COUNT(*) FROM images WHERE first_batch_id = ?1",
        params![batch_id],
        |row| row.get(0),
    )?;

    Ok(BatchCounts {
        total_files: to_u64(total),
        unique_count: to_u64(unique),
        imported_count: to_u64(imported),
        duplicate_existing_count: to_u64(duplicate_existing),
        duplicate_in_batch_count: to_u64(duplicate_in_batch),
        skipped_count: to_u64(skipped),
        error_count: to_u64(error),
        warning_count: to_u64(warning),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_batch(
    conn: &Connection,
    batch_id: i64,
    status: &str,
    finished_at: &str,
    counts: &BatchCounts,
    report_rel_path: Option<&str>,
    summary_rel_path: Option<&str>,
    note: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE batches SET
            status = ?2,
            finished_at = ?3,
            total_files = ?4,
            unique_count = ?5,
            imported_count = ?6,
            duplicate_existing_count = ?7,
            duplicate_in_batch_count = ?8,
            skipped_count = ?9,
            error_count = ?10,
            warning_count = ?11,
            report_rel_path = ?12,
            summary_rel_path = ?13,
            note = ?14
         WHERE id = ?1",
        params![
            batch_id,
            status,
            finished_at,
            counts.total_files as i64,
            counts.unique_count as i64,
            counts.imported_count as i64,
            counts.duplicate_existing_count as i64,
            counts.duplicate_in_batch_count as i64,
            counts.skipped_count as i64,
            counts.error_count as i64,
            counts.warning_count as i64,
            report_rel_path,
            summary_rel_path,
            note,
        ],
    )?;
    Ok(())
}

pub fn report_items(conn: &Connection, batch_id: i64) -> AppResult<Vec<BatchReportItem>> {
    let mut stmt = conn.prepare(
        "SELECT bi.seq, bi.file_name, bi.source_abs_path, bi.source_rel_path,
                bi.size_bytes, bi.sha256, bi.outcome, bi.library_rel_path,
                bi.result_rel_path, bi.error_code, bi.error_message, bi.warning_code,
                bi.warning_message, img.original_name, img.library_rel_path,
                dup.file_name, dup.source_abs_path
         FROM batch_items bi
         LEFT JOIN images img ON img.id = bi.duplicate_of_image_id
         LEFT JOIN batch_items dup ON dup.id = bi.duplicate_of_item_id
         WHERE bi.batch_id = ?1
         ORDER BY bi.seq",
    )?;
    let rows = stmt.query_map(params![batch_id], |row| {
        Ok(BatchReportItem {
            seq: row.get(0)?,
            file_name: row.get(1)?,
            source_abs_path: row.get(2)?,
            source_rel_path: row.get(3)?,
            size_bytes: row.get(4)?,
            sha256: row.get(5)?,
            outcome: row.get(6)?,
            library_rel_path: row.get(7)?,
            result_rel_path: row.get(8)?,
            error_code: row.get(9)?,
            error_message: row.get(10)?,
            warning_code: row.get(11)?,
            warning_message: row.get(12)?,
            duplicate_image_name: row.get(13)?,
            duplicate_image_path: row.get(14)?,
            duplicate_item_name: row.get(15)?,
            duplicate_item_source: row.get(16)?,
        })
    })?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn checkpoint(conn: &Connection) {
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}

fn row_to_batch_summary(row: &Row<'_>, root: &Path) -> rusqlite::Result<BatchSummary> {
    let result_rel_path: String = row.get(5)?;
    let report_rel_path: Option<String> = row.get(17)?;
    Ok(BatchSummary {
        id: row.get(0)?,
        merchant_id: row.get(1)?,
        merchant_name: row.get(2)?,
        batch_type: row.get(3)?,
        source_abs_path: row.get(4)?,
        result_abs_path: root.join(&result_rel_path).to_string_lossy().to_string(),
        status: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        total_files: to_u64(row.get::<_, i64>(9)?),
        unique_count: to_u64(row.get::<_, i64>(10)?),
        imported_count: to_u64(row.get::<_, i64>(11)?),
        duplicate_existing_count: to_u64(row.get::<_, i64>(12)?),
        duplicate_in_batch_count: to_u64(row.get::<_, i64>(13)?),
        skipped_count: to_u64(row.get::<_, i64>(14)?),
        error_count: to_u64(row.get::<_, i64>(15)?),
        warning_count: to_u64(row.get::<_, i64>(16)?),
        report_path: report_rel_path
            .map(|path| root.join(path).to_string_lossy().to_string()),
        note: row.get(18)?,
    })
}

fn to_u64(value: i64) -> u64 {
    value.max(0) as u64
}
