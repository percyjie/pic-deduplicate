use std::fs;
use std::path::Path;

use pic_deduplicate_lib::core::paths::{REPORT_FILE, SUMMARY_FILE};
use pic_deduplicate_lib::core::report::{write_batch_reports, ReportMeta};
use pic_deduplicate_lib::db::open_database;
use pic_deduplicate_lib::db::queries::{
    self, BatchCounts, BatchReportItem, NewBatchItem, NewLibraryImage,
};

fn sample_item(seq: i64, file_name: &str, outcome: &str) -> BatchReportItem {
    BatchReportItem {
        seq,
        file_name: file_name.to_string(),
        source_abs_path: format!(r"E:\来源\{file_name}"),
        source_rel_path: format!("子目录/{file_name}"),
        size_bytes: Some(16),
        sha256: Some("a".repeat(64)),
        outcome: outcome.to_string(),
        duplicate_kind: None,
        visual_distance: None,
        library_rel_path: None,
        result_rel_path: None,
        error_code: None,
        error_message: None,
        warning_code: None,
        warning_message: None,
        duplicate_image_name: None,
        duplicate_image_path: None,
        duplicate_item_name: None,
        duplicate_item_source: None,
    }
}

fn sample_counts() -> BatchCounts {
    BatchCounts {
        total_files: 3,
        unique_count: 1,
        imported_count: 1,
        duplicate_existing_count: 1,
        duplicate_in_batch_count: 0,
        skipped_count: 0,
        error_count: 1,
        warning_count: 0,
    }
}

#[test]
fn csv_report_has_bom_crlf_escaping_and_formula_guard() {
    let root = tempfile::tempdir().unwrap();
    let result_dir = root
        .path()
        .join("商户A")
        .join("去重结果")
        .join("2026-10-20_10-00-00");

    let mut formula = sample_item(1, "=SUM(A1:A2).jpg", "duplicate_existing");
    formula.source_rel_path = "商户,一/照片 \"引号\".jpg".to_string();
    formula.duplicate_image_name = Some("主库图.jpg".to_string());
    formula.duplicate_image_path =
        Some("商户A/图片主库/2026-10/主库图__abc.jpg".to_string());
    formula.duplicate_kind = Some("exact".to_string());

    let mut broken = sample_item(2, "损坏图片.png", "error");
    broken.sha256 = None;
    broken.error_code = Some("READ_FAILED".to_string());
    broken.error_message = Some("读取失败，\n原因：\"文件被占用\"".to_string());

    let mut unique = sample_item(3, "中文 照片.webp", "unique");
    unique.library_rel_path = Some("商户A/图片主库/2026-10/中文 照片__def.webp".to_string());
    unique.result_rel_path = Some("商户A/去重结果/批次/不重复/中文 照片.webp".to_string());

    let items = vec![formula, broken, unique];
    let counts = sample_counts();
    let meta = ReportMeta {
        batch_id: 7,
        batch_type: "daily",
        merchant_name: "商户A",
        source_dir: Path::new(r"E:\来源"),
        result_dir: &result_dir,
        started_at: "2026-10-20T10:00:00+08:00",
        finished_at: "2026-10-20T10:05:00+08:00",
        status: "completed_with_errors",
        match_mode: "visual",
        visual_threshold: 8,
    };

    let (report_rel, summary_rel) =
        write_batch_reports(root.path(), &meta, &counts, &items, &["扫描提示".to_string()], None)
            .unwrap();
    assert!(report_rel.replace('\\', "/").ends_with(REPORT_FILE));
    assert!(summary_rel.replace('\\', "/").ends_with(SUMMARY_FILE));

    let csv_path = result_dir.join(REPORT_FILE);
    let bytes = fs::read(&csv_path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "CSV 必须带 UTF-8 BOM");
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    assert!(text.contains("\r\n"), "CSV 必须使用 CRLF 行尾");
    assert!(!text.starts_with('\u{feff}'));

    let mut reader = csv::ReaderBuilder::new().from_reader(text.as_bytes());
    let headers = reader.headers().unwrap().clone();
    assert_eq!(headers.len(), 17);
    assert_eq!(&headers[0], "序号");
    assert_eq!(&headers[14], "警告信息");
    assert_eq!(&headers[15], "匹配方式");
    assert_eq!(&headers[16], "视觉差异");

    let records: Vec<csv::StringRecord> = reader.records().map(|r| r.unwrap()).collect();
    assert_eq!(records.len(), 3);

    assert_eq!(&records[0][1], "'=SUM(A1:A2).jpg", "公式开头必须被转义");
    assert_eq!(&records[0][2], "商户,一/照片 \"引号\".jpg");
    assert_eq!(&records[0][6], "重复（主库已有）");
    assert_eq!(&records[0][7], "已有主库");
    assert!(records[0][8].contains("主库图.jpg"));
    assert_eq!(&records[0][15], "精确哈希");
    assert_eq!(&records[0][16], "");

    assert_eq!(&records[1][12], "读取失败，\n原因：\"文件被占用\"");
    assert_eq!(&records[1][6], "处理失败");

    assert_eq!(&records[2][1], "中文 照片.webp");
    assert_eq!(&records[2][6], "不重复");

    let summary: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(result_dir.join(SUMMARY_FILE)).unwrap()).unwrap();
    assert_eq!(summary["batchId"], 7);
    assert_eq!(summary["status"], "completed_with_errors");
    assert_eq!(summary["matchMode"], "visual");
    assert_eq!(summary["visualThreshold"], 8);
    assert_eq!(summary["counts"]["errors"], 1);
    assert_eq!(summary["scanWarnings"][0], "扫描提示");
    assert!(summary["fatalError"].is_null());
}

fn new_batch_item(seq: i64, file_name: &str, hash: &str) -> NewBatchItem {
    NewBatchItem {
        seq,
        source_abs_path: format!(r"E:\来源\{file_name}"),
        source_rel_path: file_name.to_string(),
        file_name: file_name.to_string(),
        size_bytes: 16,
        mtime_ms: None,
        sha256: Some(hash.to_string()),
        outcome: "unique".to_string(),
        duplicate_kind: None,
        visual_distance: None,
        duplicate_of_image_id: None,
        duplicate_of_seq: None,
        error_code: None,
        error_message: None,
        warning_code: None,
        warning_message: None,
    }
}

fn new_image(merchant_id: i64, hash: &str, library_rel_path: &str) -> NewLibraryImage {
    NewLibraryImage {
        merchant_id,
        sha256: hash.to_string(),
        size_bytes: 16,
        library_rel_path: library_rel_path.to_string(),
        original_name: "a.jpg".to_string(),
        original_source_path: r"E:\来源\a.jpg".to_string(),
        imported_at: "2026-10-20T10:00:00+08:00".to_string(),
        batch_id: 1,
        visual: None,
    }
}

#[test]
fn database_enforces_unique_hash_per_merchant_and_unique_result_path() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("index.db");
    let mut conn = open_database(&db_path).unwrap();

    let merchant_a = queries::insert_merchant(&conn, "商户A", "2026-10-20T10:00:00+08:00").unwrap();
    let merchant_b = queries::insert_merchant(&conn, "商户B", "2026-10-20T10:00:00+08:00").unwrap();

    let batch_a = queries::insert_running_batch(
        &conn,
        merchant_a,
        "initial_import",
        "exact",
        8,
        r"E:\来源",
        "商户A/去重结果/批次1",
        "2026-10",
        "2026-10-20T10:00:00+08:00",
    )
    .unwrap();

    assert!(
        queries::insert_running_batch(
            &conn,
            merchant_a,
            "daily",
            "exact",
            8,
            r"E:\来源2",
            "商户a/去重结果/批次1",
            "2026-10",
            "2026-10-20T11:00:00+08:00",
        )
        .is_err(),
        "结果目录相对路径必须大小写不敏感唯一"
    );

    let hash = "b".repeat(64);
    let ids = queries::insert_batch_items(
        &mut conn,
        batch_a,
        &[new_batch_item(1, "a.jpg", &hash)],
    )
    .unwrap();
    queries::commit_library_image(
        &mut conn,
        &new_image(merchant_a, &hash, "商户A/图片主库/2026-10/a__b.jpg"),
        ids[0],
    )
    .unwrap();

    let batch_a2 = queries::insert_running_batch(
        &conn,
        merchant_a,
        "daily",
        "exact",
        8,
        r"E:\来源2",
        "商户A/去重结果/批次2",
        "2026-10",
        "2026-10-21T10:00:00+08:00",
    )
    .unwrap();
    let ids2 = queries::insert_batch_items(
        &mut conn,
        batch_a2,
        &[new_batch_item(1, "a2.jpg", &hash)],
    )
    .unwrap();
    assert!(
        queries::commit_library_image(
            &mut conn,
            &new_image(merchant_a, &hash, "商户A/图片主库/2026-10/a2__b.jpg"),
            ids2[0],
        )
        .is_err(),
        "同一商户相同哈希只能入库一次"
    );

    let batch_b = queries::insert_running_batch(
        &conn,
        merchant_b,
        "initial_import",
        "exact",
        8,
        r"E:\来源",
        "商户B/去重结果/批次1",
        "2026-10",
        "2026-10-20T10:00:00+08:00",
    )
    .unwrap();
    let ids_b = queries::insert_batch_items(
        &mut conn,
        batch_b,
        &[new_batch_item(1, "a.jpg", &hash)],
    )
    .unwrap();
    queries::commit_library_image(
        &mut conn,
        &new_image(merchant_b, &hash, "商户B/图片主库/2026-10/a__b.jpg"),
        ids_b[0],
    )
    .unwrap();

    let other_hash = "c".repeat(64);
    assert!(
        queries::commit_library_image(
            &mut conn,
            &new_image(
                merchant_a,
                &other_hash,
                "商户A/图片主库/2026-10/a__b.jpg"
            ),
            ids2[0],
        )
        .is_err(),
        "同一商户的主库相对路径必须唯一"
    );
}

#[test]
fn database_reopen_is_idempotent_and_interrupts_running_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("index.db");
    let (merchant_id, batch_id) = {
        let conn = open_database(&db_path).unwrap();
        let merchant_id =
            queries::insert_merchant(&conn, "商户A", "2026-10-20T10:00:00+08:00").unwrap();
        let batch_id = queries::insert_running_batch(
            &conn,
            merchant_id,
            "daily",
            "exact",
            8,
            r"E:\来源",
            "商户A/去重结果/批次1",
            "2026-10",
            "2026-10-20T10:00:00+08:00",
        )
        .unwrap();
        (merchant_id, batch_id)
    };

    let conn = open_database(&db_path).unwrap();
    let merchant = queries::find_merchant(&conn, merchant_id).unwrap();
    assert_eq!(merchant.folder_name, "商户A");

    queries::interrupt_running_batches(&conn).unwrap();
    let batch = queries::get_batch_summary(&conn, dir.path(), batch_id)
        .unwrap()
        .unwrap();
    assert_eq!(batch.status, "interrupted");
    assert!(batch.finished_at.is_some());

    let page = queries::list_batches(&conn, dir.path(), Some(merchant_id), 1, 10).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].status, "interrupted");
}
