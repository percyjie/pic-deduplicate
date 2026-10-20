CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS merchants (
    id INTEGER PRIMARY KEY,
    folder_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS batches (
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

CREATE TABLE IF NOT EXISTS images (
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

CREATE TABLE IF NOT EXISTS batch_items (
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

CREATE INDEX IF NOT EXISTS idx_batches_merchant_started
    ON batches (merchant_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_batch_items_batch_outcome
    ON batch_items (batch_id, outcome);

CREATE INDEX IF NOT EXISTS idx_batch_items_batch_error
    ON batch_items (batch_id, error_code, warning_code);
