use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Local;
use pic_deduplicate_lib::core::paths::{
    is_supported_image, CONTROL_DIR, DB_FILE, DUPLICATES_DIR, GENERATED_MARKER, UNIQUES_DIR,
};
use pic_deduplicate_lib::core::pipeline::{
    prepare_batch, prepare_batch_with_options, run_prepared, BatchOptions, PreparedBatch,
};
use pic_deduplicate_lib::core::scan::scan_source;
use pic_deduplicate_lib::core::visual::DEFAULT_THRESHOLD;
use pic_deduplicate_lib::db::{self, queries};
use pic_deduplicate_lib::error::AppError;
use pic_deduplicate_lib::models::{BatchFinish, BatchKind, MatchMode, ProgressUpdate};
use pic_deduplicate_lib::state::RootContext;
use tempfile::TempDir;
use walkdir::WalkDir;

struct Fixture {
    _root_dir: TempDir,
    _source_dir: TempDir,
    root: PathBuf,
    source: PathBuf,
    context: RootContext,
    merchant_id: i64,
}

impl Fixture {
    fn new(merchant: &str) -> Self {
        let root_dir = tempfile::tempdir().expect("root tempdir");
        let root = root_dir.path().to_path_buf();
        let control = root.join(CONTROL_DIR);
        fs::create_dir_all(&control).expect("control dir");
        let conn = db::open_database(&control.join(DB_FILE)).expect("database");
        let now = Local::now().to_rfc3339();
        let merchant_id = queries::insert_merchant(&conn, merchant, &now).expect("merchant");
        fs::create_dir_all(root.join(merchant)).expect("merchant dir");
        let context = RootContext {
            root: root.clone(),
            control_dir: control,
            db: Arc::new(Mutex::new(conn)),
        };
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let source = source_dir.path().to_path_buf();
        Self {
            _root_dir: root_dir,
            _source_dir: source_dir,
            root,
            source,
            context,
            merchant_id,
        }
    }

    fn add_merchant(&self, name: &str) -> i64 {
        fs::create_dir_all(self.root.join(name)).expect("merchant dir");
        let db = self.context.db.lock().expect("db lock");
        queries::insert_merchant(&db, name, &Local::now().to_rfc3339()).expect("insert merchant")
    }

    fn image_count(&self, merchant_id: i64) -> i64 {
        let db = self.context.db.lock().expect("db lock");
        db.query_row(
            "SELECT COUNT(*) FROM images WHERE merchant_id = ?1",
            [merchant_id],
            |row| row.get(0),
        )
        .expect("image count")
    }
}

fn write_file(dir: &Path, relative: &str, contents: &[u8]) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(path, contents).expect("write file");
}

fn clear_dir(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("entry");
        if entry.file_type().expect("file type").is_dir() {
            fs::remove_dir_all(entry.path()).expect("remove dir");
        } else {
            fs::remove_file(entry.path()).expect("remove file");
        }
    }
}

fn image_files(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_supported_image(entry.path()))
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = image_files(dir)
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn execute(
    fixture: &Fixture,
    merchant_id: i64,
    source: &Path,
    kind: BatchKind,
    cancel: Arc<AtomicBool>,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> (PreparedBatch, BatchFinish) {
    let prepared = prepare_batch(&fixture.context, merchant_id, source, kind).expect("prepare");
    let finish = run_prepared(&fixture.context, &prepared, cancel, progress).expect("run");
    (prepared, finish)
}

fn execute_ok(
    fixture: &Fixture,
    merchant_id: i64,
    source: &Path,
    kind: BatchKind,
) -> (PreparedBatch, BatchFinish) {
    let (prepared, finish) = execute(
        fixture,
        merchant_id,
        source,
        kind,
        Arc::new(AtomicBool::new(false)),
        &|_| {},
    );
    assert!(
        finish.error.is_none(),
        "unexpected batch error: {:?}",
        finish.error
    );
    (prepared, finish)
}

fn execute_visual_ok(
    fixture: &Fixture,
    merchant_id: i64,
    source: &Path,
    kind: BatchKind,
) -> (PreparedBatch, BatchFinish) {
    let options = BatchOptions {
        match_mode: MatchMode::Visual,
        visual_threshold: DEFAULT_THRESHOLD,
    };
    let prepared =
        prepare_batch_with_options(&fixture.context, merchant_id, source, kind, options)
            .expect("prepare visual batch");
    let finish = run_prepared(
        &fixture.context,
        &prepared,
        Arc::new(AtomicBool::new(false)),
        &|_| {},
    )
    .expect("run visual batch");
    assert!(
        finish.error.is_none(),
        "unexpected batch error: {:?}",
        finish.error
    );
    (prepared, finish)
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/visual")
        .join(name)
}

fn copy_fixture(name: &str, dir: &Path, target: &str) {
    let target_path = dir.join(target);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).expect("target parent");
    }
    fs::copy(fixture_path(name), &target_path).expect("copy fixture");
}

fn batch_item_rows(
    fixture: &Fixture,
    batch_id: i64,
) -> Vec<(String, String, Option<String>, Option<i64>)> {
    let db = fixture.context.db.lock().expect("db lock");
    let mut stmt = db
        .prepare(
            "SELECT file_name, outcome, duplicate_kind, visual_distance
             FROM batch_items WHERE batch_id = ?1 ORDER BY file_name",
        )
        .expect("prepare item rows");
    let rows = stmt
        .query_map([batch_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query item rows");
    rows.map(|row| row.expect("item row")).collect()
}

#[test]
fn initial_import_dedupes_inside_batch_and_writes_results() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "a.jpg", b"same-content");
    write_file(&fixture.source, "sub/b.jpg", b"same-content");
    write_file(&fixture.source, "c.JPG", b"other-content");
    write_file(&fixture.source, "empty.jpg", b"");

    let (prepared, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    let batch = &finish.batch;

    assert_eq!(batch.status, "completed");
    assert_eq!(batch.total_files, 4);
    assert_eq!(batch.unique_count, 2);
    assert_eq!(batch.imported_count, 2);
    assert_eq!(batch.duplicate_in_batch_count, 1);
    assert_eq!(batch.duplicate_existing_count, 0);
    assert_eq!(batch.skipped_count, 1);
    assert_eq!(batch.error_count, 0);
    assert_eq!(batch.warning_count, 1);
    assert_eq!(fixture.image_count(fixture.merchant_id), 2);

    assert_eq!(image_files(&prepared.library_dir).len(), 2);
    assert_eq!(image_files(&prepared.result_dir.join(DUPLICATES_DIR)).len(), 1);
    assert_eq!(image_files(&prepared.result_dir.join(UNIQUES_DIR)).len(), 2);
    assert!(prepared.result_dir.join("批次报告.csv").is_file());
    assert!(prepared.result_dir.join("批次摘要.json").is_file());

    let csv = fs::read(prepared.result_dir.join("批次报告.csv")).expect("read csv");
    assert_eq!(&csv[..3], &[0xEF, 0xBB, 0xBF]);

    assert_eq!(fs::read(fixture.source.join("a.jpg")).unwrap(), b"same-content");
    assert_eq!(fs::read(fixture.source.join("sub/b.jpg")).unwrap(), b"same-content");
    assert_eq!(fs::read(fixture.source.join("empty.jpg")).unwrap(), b"");
}

#[test]
fn daily_batch_finds_library_duplicates_and_imports_only_new_images() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "a.jpg", b"content-a");
    write_file(&fixture.source, "c.jpg", b"content-c");
    execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );

    clear_dir(&fixture.source);
    write_file(&fixture.source, "copy-a.jpg", b"content-a");
    write_file(&fixture.source, "copy-c.jpg", b"content-c");
    write_file(&fixture.source, "new.jpg", b"content-new");

    let (prepared, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::Daily,
    );
    let batch = &finish.batch;
    assert_eq!(batch.status, "completed");
    assert_eq!(batch.batch_type, "daily");
    assert_eq!(batch.total_files, 3);
    assert_eq!(batch.unique_count, 1);
    assert_eq!(batch.imported_count, 1);
    assert_eq!(batch.duplicate_existing_count, 2);
    assert_eq!(batch.duplicate_in_batch_count, 0);
    assert_eq!(fixture.image_count(fixture.merchant_id), 3);
    assert_eq!(image_files(&prepared.library_dir).len(), 3);
    assert_eq!(image_files(&prepared.result_dir.join(DUPLICATES_DIR)).len(), 2);
    assert_eq!(image_files(&prepared.result_dir.join(UNIQUES_DIR)).len(), 1);
}

#[test]
fn reimporting_same_source_adds_nothing_and_keeps_old_result() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "a.jpg", b"content-a");
    write_file(&fixture.source, "b.jpg", b"content-b");
    write_file(&fixture.source, "empty.jpg", b"");
    let (first_prepared, _) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    let before = fixture.image_count(fixture.merchant_id);

    let (_, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::Daily,
    );
    let batch = &finish.batch;
    assert_eq!(batch.unique_count, 0);
    assert_eq!(batch.imported_count, 0);
    assert_eq!(batch.duplicate_existing_count, 2);
    assert_eq!(batch.skipped_count, 1);
    assert_eq!(fixture.image_count(fixture.merchant_id), before);
    assert!(first_prepared.result_dir.is_dir());
    assert_eq!(image_files(&first_prepared.result_dir.join(DUPLICATES_DIR)).len(), 0);
}

#[test]
fn same_file_name_with_different_content_keeps_both_and_does_not_overwrite_results() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "x/photo.jpg", b"photo-content-x");
    write_file(&fixture.source, "y/photo.jpg", b"photo-content-y");

    let (prepared, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    assert_eq!(finish.batch.unique_count, 2);
    assert_eq!(finish.batch.imported_count, 2);
    assert_eq!(fixture.image_count(fixture.merchant_id), 2);
    assert_eq!(
        file_names(&prepared.result_dir.join(UNIQUES_DIR)),
        vec!["photo (2).jpg".to_string(), "photo.jpg".to_string()]
    );
    let library_names = file_names(&prepared.library_dir);
    assert_eq!(library_names.len(), 2);
    assert!(library_names.iter().all(|name| name.starts_with("photo__")));
}

#[test]
fn merchants_have_isolated_libraries_for_the_same_image() {
    let fixture = Fixture::new("商户A");
    let merchant_b = fixture.add_merchant("商户B");
    write_file(&fixture.source, "shared.jpg", b"shared-content");

    let (_, finish_a) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    let (_, finish_b) = execute_ok(
        &fixture,
        merchant_b,
        &fixture.source,
        BatchKind::InitialImport,
    );

    assert_eq!(finish_a.batch.unique_count, 1);
    assert_eq!(finish_b.batch.unique_count, 1);
    assert_eq!(fixture.image_count(fixture.merchant_id), 1);
    assert_eq!(fixture.image_count(merchant_b), 1);
    assert_eq!(
        image_files(&fixture.root.join("商户A/图片主库")).len(),
        1
    );
    assert_eq!(
        image_files(&fixture.root.join("商户B/图片主库")).len(),
        1
    );
}

#[test]
fn generated_and_reserved_directories_are_never_scanned() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "good.jpg", b"good-content");
    write_file(&fixture.source, "图片主库/bad1.jpg", b"bad-1");
    write_file(&fixture.source, "去重结果/bad2.jpg", b"bad-2");
    write_file(&fixture.source, "custom/bad3.jpg", b"bad-3");
    fs::write(
        fixture.source.join("custom").join(GENERATED_MARKER),
        b"generated",
    )
    .expect("marker");

    let scan = scan_source(&fixture.source, &AtomicBool::new(false)).expect("scan");
    assert_eq!(scan.files.len(), 1);
    assert_eq!(scan.files[0].file_name, "good.jpg");

    let prepared = prepare_batch(
        &fixture.context,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    )
    .expect("prepare");
    let finish = run_prepared(
        &fixture.context,
        &prepared,
        Arc::new(AtomicBool::new(false)),
        &|_| {},
    )
    .expect("run");
    assert_eq!(finish.batch.total_files, 1);

    let library_source = prepared.library_dir.clone();
    let error = prepare_batch(
        &fixture.context,
        fixture.merchant_id,
        &library_source,
        BatchKind::Daily,
    )
    .expect_err("library must not be a source");
    assert_eq!(error.code(), "SOURCE_INSIDE_GENERATED");

    let error = prepare_batch(
        &fixture.context,
        fixture.merchant_id,
        &fixture.root,
        BatchKind::Daily,
    )
    .expect_err("root must not be a source");
    assert_eq!(error.code(), "SOURCE_IS_ROOT_OR_ANCESTOR");
}

#[test]
fn cancel_during_copy_leaves_no_temporary_files_and_keeps_partial_report() {
    let fixture = Fixture::new("商户A");
    for index in 0..40 {
        write_file(
            &fixture.source,
            &format!("image-{index:02}.jpg"),
            format!("content-{index}").as_bytes(),
        );
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let (prepared, finish) = execute(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
        cancel,
        &move |update| {
            if update.phase == pic_deduplicate_lib::models::JobPhase::Copying
                && update.processed > 0
            {
                trigger.store(true, Ordering::SeqCst);
            }
        },
    );

    assert_eq!(finish.batch.status, "cancelled");
    assert!(finish.batch.imported_count < 40);
    assert!(prepared.result_dir.join("批次报告.csv").is_file());
    let leftovers: Vec<PathBuf> = WalkDir::new(&prepared.result_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".picdedupe-tmp-")
        })
        .map(|entry| entry.path().to_path_buf())
        .collect();
    assert!(leftovers.is_empty(), "temporary files remain: {leftovers:?}");
    for index in 0..40 {
        assert_eq!(
            fs::read(fixture.source.join(format!("image-{index:02}.jpg"))).unwrap(),
            format!("content-{index}").as_bytes()
        );
    }
}

#[test]
fn cancel_during_hashing_still_reports_every_scanned_file() {
    let fixture = Fixture::new("商户A");
    for index in 0..200 {
        write_file(
            &fixture.source,
            &format!("bulk-{index:03}.jpg"),
            format!("bulk-content-{index}").as_bytes(),
        );
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let (prepared, finish) = execute(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
        cancel,
        &move |update| {
            if update.phase == pic_deduplicate_lib::models::JobPhase::Hashing
                && update.processed >= 64
            {
                trigger.store(true, Ordering::SeqCst);
            }
        },
    );

    assert_eq!(finish.batch.status, "cancelled");
    assert_eq!(finish.batch.total_files, 200);
    assert_eq!(finish.batch.imported_count, 0);
    assert_eq!(finish.batch.skipped_count, 200);
    assert_eq!(finish.batch.warning_count, 200);
    assert_eq!(image_files(&prepared.library_dir).len(), 0);

    let csv_path = prepared.result_dir.join("批次报告.csv");
    let bytes = fs::read(&csv_path).expect("report exists");
    let text = std::str::from_utf8(&bytes[3..]).expect("utf-8 csv");
    let mut reader = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let records: Vec<csv::StringRecord> = reader.records().map(|r| r.unwrap()).collect();
    assert_eq!(records.len(), 200);
    assert_eq!(&records[0][6], "已跳过");
    assert_eq!(&records[0][13], "NOT_PROCESSED");

    let summary: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(prepared.result_dir.join("批次摘要.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(summary["status"], "cancelled");
    assert_eq!(summary["counts"]["totalFiles"], 200);
    assert_eq!(summary["counts"]["skipped"], 200);
    assert_eq!(summary["counts"]["warnings"], 200);
}

#[test]
fn zero_byte_files_are_skipped_without_error() {
    let fixture = Fixture::new("商户A");
    write_file(&fixture.source, "empty.png", b"");
    let (prepared, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    assert_eq!(finish.batch.status, "completed");
    assert_eq!(finish.batch.total_files, 1);
    assert_eq!(finish.batch.skipped_count, 1);
    assert_eq!(finish.batch.error_count, 0);
    assert_eq!(finish.batch.imported_count, 0);
    assert_eq!(image_files(&prepared.library_dir).len(), 0);
}

#[test]
fn inspect_source_inside_control_directory_is_rejected() {
    let fixture = Fixture::new("商户A");
    let error = prepare_batch(
        &fixture.context,
        fixture.merchant_id,
        &fixture.context.control_dir,
        BatchKind::Daily,
    )
    .expect_err("control dir must be rejected");
    assert!(matches!(error, AppError::User { .. }));
    assert_eq!(error.code(), "SOURCE_INSIDE_CONTROL");
}

#[test]
fn visual_mode_matches_compressed_resized_and_rotated_library_images() {
    let fixture = Fixture::new("商户A");
    copy_fixture("base.png", &fixture.source, "base.png");
    execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    {
        let db = fixture.context.db.lock().expect("db lock");
        let visuals: i64 = db
            .query_row("SELECT COUNT(*) FROM image_visuals", [], |row| row.get(0))
            .expect("visual count");
        assert_eq!(visuals, 0, "精确模式不应生成视觉指纹");
    }

    clear_dir(&fixture.source);
    copy_fixture("base-q60.jpg", &fixture.source, "compressed.jpg");
    copy_fixture("base-resized-q55.jpg", &fixture.source, "resized.jpg");
    copy_fixture("base-rot90.png", &fixture.source, "rotated.png");
    copy_fixture("other.png", &fixture.source, "other.png");

    let (prepared, finish) = execute_visual_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::Daily,
    );
    let batch = &finish.batch;
    assert_eq!(batch.status, "completed");
    assert_eq!(batch.match_mode, "visual");
    assert_eq!(batch.visual_threshold, DEFAULT_THRESHOLD);
    assert_eq!(batch.total_files, 4);
    assert_eq!(batch.unique_count, 1);
    assert_eq!(batch.imported_count, 1);
    assert_eq!(batch.duplicate_existing_count, 3);
    assert_eq!(batch.duplicate_in_batch_count, 0);
    assert_eq!(batch.visual_duplicate_count, 3);
    assert_eq!(batch.error_count, 0);
    assert_eq!(fixture.image_count(fixture.merchant_id), 2);
    assert_eq!(image_files(&prepared.library_dir).len(), 2);

    let rows = batch_item_rows(&fixture, prepared.batch_id);
    assert_eq!(rows.len(), 4);
    for (name, outcome, kind, distance) in &rows {
        match name.as_str() {
            "compressed.jpg" | "resized.jpg" => {
                assert_eq!(outcome, "duplicate_existing", "{name}");
                assert_eq!(kind.as_deref(), Some("visual"), "{name}");
                assert!(
                    distance.expect("distance") <= DEFAULT_THRESHOLD as i64,
                    "{name}: {distance:?}"
                );
            }
            "rotated.png" => {
                assert_eq!(outcome, "duplicate_existing");
                assert_eq!(kind.as_deref(), Some("visual"));
                assert_eq!(*distance, Some(0));
            }
            "other.png" => {
                assert_eq!(outcome, "unique");
                assert!(kind.is_none());
                assert!(distance.is_none());
            }
            other => panic!("unexpected file {other}"),
        }
    }

    let db = fixture.context.db.lock().expect("db lock");
    let visual_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM image_visuals", [], |row| row.get(0))
        .expect("visual count");
    assert_eq!(visual_rows, 2, "主库退回应补齐视觉指纹，新图片也应入库指纹");
    drop(db);

    let bytes = fs::read(prepared.result_dir.join("批次报告.csv")).expect("read csv");
    let text = std::str::from_utf8(&bytes[3..]).expect("utf-8 csv");
    let mut reader = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let headers = reader.headers().expect("headers").clone();
    assert_eq!(&headers[15], "匹配方式");
    assert_eq!(&headers[16], "视觉差异");
    let records: Vec<csv::StringRecord> = reader.records().map(|r| r.unwrap()).collect();
    let compressed = records
        .iter()
        .find(|record| &record[1] == "compressed.jpg")
        .expect("compressed row");
    assert_eq!(&compressed[7], "主库视觉相似");
    assert_eq!(&compressed[15], "视觉相似");
    let distance: u32 = compressed[16].parse().expect("visual distance");
    assert!(distance <= DEFAULT_THRESHOLD);

    let summary: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(prepared.result_dir.join("批次摘要.json")).expect("summary file"),
    )
    .expect("summary json");
    assert_eq!(summary["matchMode"], "visual");
    assert_eq!(summary["visualThreshold"], DEFAULT_THRESHOLD);
}

#[test]
fn visual_mode_dedupes_rotated_and_compressed_files_inside_batch() {
    let fixture = Fixture::new("商户A");
    copy_fixture("base.png", &fixture.source, "a.png");
    copy_fixture("base-rot90.png", &fixture.source, "b.png");
    copy_fixture("base-q60.jpg", &fixture.source, "c.jpg");
    copy_fixture("other.png", &fixture.source, "d.png");

    let (prepared, finish) = execute_visual_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    let batch = &finish.batch;
    assert_eq!(batch.status, "completed");
    assert_eq!(batch.total_files, 4);
    assert_eq!(batch.unique_count, 2);
    assert_eq!(batch.imported_count, 2);
    assert_eq!(batch.duplicate_in_batch_count, 2);
    assert_eq!(batch.visual_duplicate_count, 2);
    assert_eq!(batch.error_count, 0);
    assert_eq!(fixture.image_count(fixture.merchant_id), 2);
    assert_eq!(image_files(&prepared.library_dir).len(), 2);

    let rows = batch_item_rows(&fixture, prepared.batch_id);
    let visual_duplicates: Vec<_> = rows
        .iter()
        .filter(|row| row.1 == "duplicate_in_batch")
        .collect();
    assert_eq!(visual_duplicates.len(), 2);
    assert!(visual_duplicates
        .iter()
        .all(|row| row.2.as_deref() == Some("visual")));
    assert!(visual_duplicates
        .iter()
        .all(|row| row.3.expect("distance") <= DEFAULT_THRESHOLD as i64));
}

#[test]
fn exact_mode_treats_visually_similar_images_as_new() {
    let fixture = Fixture::new("商户A");
    copy_fixture("base.png", &fixture.source, "base.png");
    execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );

    clear_dir(&fixture.source);
    copy_fixture("base-q60.jpg", &fixture.source, "compressed.jpg");
    copy_fixture("base-rot90.png", &fixture.source, "rotated.png");

    let (_, finish) = execute_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::Daily,
    );
    let batch = &finish.batch;
    assert_eq!(batch.match_mode, "exact");
    assert_eq!(batch.unique_count, 2);
    assert_eq!(batch.imported_count, 2);
    assert_eq!(batch.duplicate_existing_count, 0);
    assert_eq!(batch.visual_duplicate_count, 0);
    assert_eq!(fixture.image_count(fixture.merchant_id), 3);
}

#[test]
fn visual_mode_imports_low_detail_images_with_warning() {
    let fixture = Fixture::new("商户A");
    copy_fixture("solid.png", &fixture.source, "solid.png");

    let (prepared, finish) = execute_visual_ok(
        &fixture,
        fixture.merchant_id,
        &fixture.source,
        BatchKind::InitialImport,
    );
    let batch = &finish.batch;
    assert_eq!(batch.status, "completed");
    assert_eq!(batch.unique_count, 1);
    assert_eq!(batch.imported_count, 1);
    assert_eq!(batch.warning_count, 1);
    assert_eq!(batch.error_count, 0);
    assert_eq!(batch.visual_duplicate_count, 0);

    let db = fixture.context.db.lock().expect("db lock");
    let (code, message): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT warning_code, warning_message FROM batch_items WHERE batch_id = ?1",
            [prepared.batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("warning row");
    assert_eq!(code.as_deref(), Some("VISUAL_UNAVAILABLE"));
    assert!(message.unwrap_or_default().contains("仅参与精确比对"));
}
