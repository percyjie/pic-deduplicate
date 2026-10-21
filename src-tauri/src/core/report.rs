use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde_json::json;

use crate::core::paths::{path_to_rel, rel_to_path, REPORT_FILE, SUMMARY_FILE};
use crate::db::queries::{BatchCounts, BatchReportItem};
use crate::error::{AppError, AppResult};

pub struct ReportMeta<'a> {
    pub batch_id: i64,
    pub batch_type: &'a str,
    pub merchant_name: &'a str,
    pub source_dir: &'a Path,
    pub result_dir: &'a Path,
    pub started_at: &'a str,
    pub finished_at: &'a str,
    pub status: &'a str,
    pub match_mode: &'a str,
    pub visual_threshold: u32,
}

pub fn write_batch_reports(
    root: &Path,
    meta: &ReportMeta<'_>,
    counts: &BatchCounts,
    items: &[BatchReportItem],
    scan_warnings: &[String],
    fatal_error: Option<&str>,
) -> AppResult<(String, String)> {
    fs::create_dir_all(meta.result_dir)?;
    let csv_path = meta.result_dir.join(REPORT_FILE);
    let summary_path = meta.result_dir.join(SUMMARY_FILE);

    write_csv(&csv_path, root, items)?;
    write_summary(
        &summary_path,
        meta,
        counts,
        scan_warnings,
        fatal_error,
    )?;

    Ok((
        path_to_rel(root, &csv_path)?,
        path_to_rel(root, &summary_path)?,
    ))
}

fn write_csv(path: &Path, root: &Path, items: &[BatchReportItem]) -> AppResult<()> {
    let temp = temp_path(path);
    let result = (|| -> AppResult<()> {
        let mut file = File::create(&temp)?;
        file.write_all(&[0xEF, 0xBB, 0xBF])?;
        {
            let mut writer = csv::WriterBuilder::new()
                .terminator(csv::Terminator::CRLF)
                .from_writer(&mut file);
            writer.write_record([
                "序号",
                "文件名",
                "相对源路径",
                "源绝对路径",
                "大小(字节)",
                "SHA-256",
                "处理结果",
                "重复来源",
                "重复对象",
                "主库路径",
                "结果路径",
                "错误码",
                "错误信息",
                "警告码",
                "警告信息",
                "匹配方式",
                "视觉差异",
            ])?;

            for item in items {
                let (duplicate_source, duplicate_object) =
                    duplicate_description(item);
                let library_path = item
                    .library_rel_path
                    .as_deref()
                    .map(|value| rel_to_path(root, value).to_string_lossy().to_string())
                    .unwrap_or_default();
                let result_path = item
                    .result_rel_path
                    .as_deref()
                    .map(|value| rel_to_path(root, value).to_string_lossy().to_string())
                    .unwrap_or_default();
                writer.write_record([
                    item.seq.to_string(),
                    excel_safe(&item.file_name),
                    excel_safe(&item.source_rel_path),
                    excel_safe(&item.source_abs_path),
                    item.size_bytes.unwrap_or(0).to_string(),
                    item.sha256.clone().unwrap_or_default(),
                    outcome_label(&item.outcome).to_string(),
                    duplicate_source,
                    excel_safe(&duplicate_object),
                    excel_safe(&library_path),
                    excel_safe(&result_path),
                    item.error_code.clone().unwrap_or_default(),
                    excel_safe(item.error_message.as_deref().unwrap_or_default()),
                    item.warning_code.clone().unwrap_or_default(),
                    excel_safe(item.warning_message.as_deref().unwrap_or_default()),
                    match_label(item.duplicate_kind.as_deref()).to_string(),
                    item.visual_distance
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ])?;
            }
            writer.flush()?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_summary(
    path: &Path,
    meta: &ReportMeta<'_>,
    counts: &BatchCounts,
    scan_warnings: &[String],
    fatal_error: Option<&str>,
) -> AppResult<()> {
    let temp = temp_path(path);
    let warnings: Vec<&String> = scan_warnings.iter().take(100).collect();
    let value = json!({
        "batchId": meta.batch_id,
        "batchType": meta.batch_type,
        "merchantName": meta.merchant_name,
        "sourceDir": meta.source_dir.to_string_lossy(),
        "resultDir": meta.result_dir.to_string_lossy(),
        "startedAt": meta.started_at,
        "finishedAt": meta.finished_at,
        "status": meta.status,
        "matchMode": meta.match_mode,
        "visualThreshold": meta.visual_threshold,
        "generatedAt": Local::now().to_rfc3339(),
        "counts": {
            "totalFiles": counts.total_files,
            "unique": counts.unique_count,
            "imported": counts.imported_count,
            "duplicateExisting": counts.duplicate_existing_count,
            "duplicateInBatch": counts.duplicate_in_batch_count,
            "skipped": counts.skipped_count,
            "errors": counts.error_count,
            "warnings": counts.warning_count,
        },
        "fatalError": fatal_error,
        "scanWarnings": warnings,
    });
    let data = serde_json::to_vec_pretty(&value)?;
    let result = (|| -> AppResult<()> {
        let mut file = File::create(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn duplicate_description(item: &BatchReportItem) -> (String, String) {
    let visual = item.duplicate_kind.as_deref() == Some("visual");
    if let Some(name) = &item.duplicate_image_name {
        let path = item.duplicate_image_path.as_deref().unwrap_or_default();
        let source = if visual { "主库视觉相似" } else { "已有主库" };
        return (source.to_string(), format!("{name}（{path}）"));
    }
    if let Some(name) = &item.duplicate_item_name {
        let path = item.duplicate_item_source.as_deref().unwrap_or_default();
        let source = if visual { "同批次视觉相似" } else { "同批次" };
        return (source.to_string(), format!("{name}（{path}）"));
    }
    (String::new(), String::new())
}

fn match_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some("exact") => "精确哈希",
        Some("visual") => "视觉相似",
        _ => "",
    }
}

fn outcome_label(outcome: &str) -> &'static str {
    match outcome {
        "unique" => "不重复",
        "duplicate_existing" => "重复（主库已有）",
        "duplicate_in_batch" => "重复（同批次）",
        "skipped" => "已跳过",
        "error" => "处理失败",
        _ => "未知",
    }
}

fn excel_safe(value: &str) -> String {
    match value.chars().next() {
        Some('=') | Some('+') | Some('-') | Some('@') => format!("'{value}"),
        _ => value.to_string(),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "report".to_string());
    path.with_file_name(format!(".{file_name}.tmp"))
}

#[allow(dead_code)]
fn report_error(error_code: Option<&str>, message: Option<&str>) -> AppError {
    AppError::user(
        "REPORT_WRITE_FAILED",
        format!(
            "报告写入失败：{}{}",
            error_code.unwrap_or_default(),
            message.unwrap_or_default()
        ),
    )
}
