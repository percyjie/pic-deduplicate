use rusqlite::{params, Connection};

use pic_deduplicate_lib::db::{self, queries};

const V1_SCHEMA: &str = r#"
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE merchants (
    id INTEGER PRIMARY KEY,
    folder_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE batches (
    id INTEGER PRIMARY KEY,
    merchant_id INTEGER NOT NULL REFERENCES merchants(id) ON DELETE RESTRICT,
    batch_type TEXT NOT NULL CHECK (batch_type IN ('initial_import', 'daily')),
    source_abs_path TEXT NOT NULL,
    result_rel_path TEXT NOT NULL COLLATE NOCASE UNIQUE,
    status TEXT NOT NULL CHECK (status IN (
        'running', 'completed', 'completed_with_errors',
        'cancelled', 'interrupted', 'failed'
    )),
    import_month TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    total_files INTEGER NOT NULL DEFAULT 0,
    unique_count INTEGER NOT NULL DEFAULT 0,
    imported_count INTEGER NOT NULL DEFAULT 0,
    duplicate_existing_count INTEGER NOT NULL DEFAULT 0,
    duplicate_in_batch_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    error_count INTEGER NOT NULL DEFAULT 0,
    warning_count INTEGER NOT NULL DEFAULT 0,
    report_rel_path TEXT,
    summary_rel_path TEXT,
    note TEXT
);
CREATE TABLE images (
    id INTEGER PRIMARY KEY,
    merchant_id INTEGER NOT NULL REFERENCES merchants(id) ON DELETE RESTRICT,
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    library_rel_path TEXT NOT NULL COLLATE NOCASE,
    original_name TEXT NOT NULL,
    original_source_path TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    first_batch_id INTEGER REFERENCES batches(id) ON DELETE SET NULL,
    UNIQUE (merchant_id, sha256),
    UNIQUE (merchant_id, library_rel_path)
);
CREATE TABLE batch_items (
    id INTEGER PRIMARY KEY,
    batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    source_abs_path TEXT NOT NULL,
    source_rel_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    size_bytes INTEGER,
    mtime_ms INTEGER,
    sha256 TEXT CHECK (sha256 IS NULL OR length(sha256) = 64),
    outcome TEXT NOT NULL CHECK (outcome IN (
        'unique', 'duplicate_existing', 'duplicate_in_batch', 'skipped', 'error'
    )),
    duplicate_of_image_id INTEGER REFERENCES images(id) ON DELETE SET NULL,
    duplicate_of_item_id INTEGER REFERENCES batch_items(id) ON DELETE SET NULL,
    library_rel_path TEXT,
    result_rel_path TEXT,
    error_code TEXT,
    error_message TEXT,
    warning_code TEXT,
    warning_message TEXT,
    UNIQUE (batch_id, seq)
);
PRAGMA user_version = 1;
"#;

fn column_names(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("table info");
    stmt.query_map([], |row| row.get::<_, String>(1))
        .expect("columns")
        .map(|row| row.expect("column name"))
        .collect()
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, i64>(0),
    )
    .expect("sqlite_master")
        > 0
}

#[test]
fn version_one_database_is_migrated_without_losing_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("index.db");

    {
        let conn = Connection::open(&path).expect("open v1 db");
        conn.execute_batch(V1_SCHEMA).expect("create v1 schema");
        conn.execute(
            "INSERT INTO merchants (id, folder_name, display_name, created_at, updated_at)
             VALUES (1, '商户A', '商户A', '2026-10-20T10:00:00+08:00', '2026-10-20T10:00:00+08:00')",
            [],
        )
        .expect("merchant");
        conn.execute(
            "INSERT INTO batches (
                id, merchant_id, batch_type, source_abs_path, result_rel_path, status,
                import_month, started_at, total_files, unique_count
             ) VALUES (1, 1, 'initial_import', 'E:/来源', '商户A/去重结果/批次1',
                       'completed', '2026-10', '2026-10-20T10:00:00+08:00', 2, 2)",
            [],
        )
        .expect("batch");
    }

    let mut conn = db::open_database(&path).expect("migrate db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
    assert!(table_exists(&conn, "image_visuals"));
    assert!(column_names(&conn, "batches").contains(&"match_mode".to_string()));
    assert!(column_names(&conn, "batches").contains(&"visual_threshold".to_string()));
    assert!(column_names(&conn, "batch_items").contains(&"duplicate_kind".to_string()));
    assert!(column_names(&conn, "batch_items").contains(&"visual_distance".to_string()));

    let batch = queries::get_batch_summary(&conn, dir.path(), 1)
        .expect("summary")
        .expect("batch exists");
    assert_eq!(batch.merchant_name, "商户A");
    assert_eq!(batch.match_mode, "exact");
    assert_eq!(batch.visual_threshold, 8);
    assert_eq!(batch.visual_duplicate_count, 0);

    let batch_id = queries::insert_running_batch(
        &conn,
        1,
        "daily",
        "visual",
        8,
        "E:/来源2",
        "商户A/去重结果/批次2",
        "2026-10",
        "2026-10-21T10:00:00+08:00",
    )
    .expect("insert visual batch");
    let items = queries::insert_batch_items(
        &mut conn,
        batch_id,
        &[queries::NewBatchItem {
            seq: 1,
            source_abs_path: "E:/来源2/a.png".to_string(),
            source_rel_path: "a.png".to_string(),
            file_name: "a.png".to_string(),
            size_bytes: 32,
            mtime_ms: None,
            sha256: Some("d".repeat(64)),
            outcome: "unique".to_string(),
            duplicate_kind: None,
            visual_distance: None,
            duplicate_of_image_id: None,
            duplicate_of_seq: None,
            error_code: None,
            error_message: None,
            warning_code: None,
            warning_message: None,
        }],
    )
    .expect("items");
    let image = queries::NewLibraryImage {
        merchant_id: 1,
        sha256: "d".repeat(64),
        size_bytes: 32,
        library_rel_path: "商户A/图片主库/2026-10/a__dddd.png".to_string(),
        original_name: "a.png".to_string(),
        original_source_path: "E:/来源2/a.png".to_string(),
        imported_at: "2026-10-21T10:00:00+08:00".to_string(),
        batch_id,
        visual: Some(pic_deduplicate_lib::core::visual::VisualHashes {
            rotations: [1, 2, 3, 4],
            width: 64,
            height: 64,
        }),
    };
    let image_id =
        queries::commit_library_image(&mut conn, &image, items[0]).expect("library image");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM image_visuals WHERE image_id = ?1 AND version = 1",
            params![image_id],
            |row| row.get(0),
        )
        .expect("visual row");
    assert_eq!(count, 1);
}

#[test]
fn migration_is_idempotent_on_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("index.db");
    drop(db::open_database(&path).expect("create db"));
    drop(db::open_database(&path).expect("reopen db"));

    let conn = db::open_database(&path).expect("reopen again");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
}
