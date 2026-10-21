use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, MutexGuard};

use chrono::{DateTime, Local};
use rayon::prelude::*;
use rusqlite::Connection;

use crate::core::copy::copy_verified;
use crate::core::guard::validate_source;
use crate::core::hash::{hash_existing_file, hash_file};
use crate::core::naming::{master_file_name, pick_result_name};
use crate::core::paths::{
    absolute_string, is_generated_dir, new_generated_dir, path_to_rel, prepare_generated_root,
    rel_to_path, write_generated_marker, RootPaths, DUPLICATES_DIR, LIBRARY_DIR, RESULTS_DIR,
    UNIQUES_DIR,
};
use crate::core::report::{write_batch_reports, ReportMeta};
use crate::core::scan::{scan_source, ScannedFile};
use crate::core::visual::{self, VisualHashes, HASH_VERSION};
use crate::db::queries::{
    self, BatchCounts, BatchReportItem, LibraryImage, LibraryVisual, NewBatchItem,
    NewImageVisual, NewLibraryImage,
};
use crate::error::{AppError, AppResult};
use crate::models::{BatchFinish, BatchKind, JobPhase, MatchMode, ProgressUpdate};
use crate::state::RootContext;

#[derive(Debug, Clone, Copy)]
pub struct BatchOptions {
    pub match_mode: MatchMode,
    pub visual_threshold: u32,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            match_mode: MatchMode::Exact,
            visual_threshold: visual::DEFAULT_THRESHOLD,
        }
    }
}

#[derive(Debug)]
pub struct PreparedBatch {
    pub batch_id: i64,
    pub merchant_id: i64,
    pub merchant_name: String,
    pub kind: BatchKind,
    pub match_mode: MatchMode,
    pub visual_threshold: u32,
    pub source: PathBuf,
    pub result_dir: PathBuf,
    pub result_rel_path: String,
    pub library_dir: PathBuf,
    pub import_month: String,
    pub started_at: String,
}

pub fn prepare_batch(
    ctx: &RootContext,
    merchant_id: i64,
    source: &Path,
    kind: BatchKind,
) -> AppResult<PreparedBatch> {
    prepare_batch_with_options(ctx, merchant_id, source, kind, BatchOptions::default())
}

pub fn prepare_batch_with_options(
    ctx: &RootContext,
    merchant_id: i64,
    source: &Path,
    kind: BatchKind,
    options: BatchOptions,
) -> AppResult<PreparedBatch> {
    let options = BatchOptions {
        match_mode: options.match_mode,
        visual_threshold: options.visual_threshold.min(visual::MAX_THRESHOLD),
    };
    let merchant = {
        let db = lock_db(ctx)?;
        queries::find_merchant(&db, merchant_id)?
    };
    let merchant_dir = ctx.root.join(&merchant.folder_name);
    if !merchant_dir.is_dir() {
        return Err(AppError::user(
            "MERCHANT_FOLDER_MISSING",
            format!(
                "商户文件夹不存在：{}。请恢复文件夹后刷新商户列表。",
                merchant_dir.display()
            ),
        ));
    }

    let validation = validate_source(&ctx.root, &merchant_dir, source)?;
    let source = validation.source;

    let library_dir = merchant_dir.join(LIBRARY_DIR);
    prepare_generated_root(&library_dir)?;
    let results_dir = merchant_dir.join(RESULTS_DIR);
    prepare_generated_root(&results_dir)?;

    let now: DateTime<Local> = Local::now();
    let import_month = now.format("%Y-%m").to_string();
    let month_dir = library_dir.join(&import_month);
    if month_dir.exists() {
        if !month_dir.is_dir() || !is_generated_dir(&month_dir) {
            return Err(AppError::user(
                "RESERVED_FOLDER_CONFLICT",
                format!("主库年月目录 {} 被非程序文件占用。", month_dir.display()),
            ));
        }
    } else {
        new_generated_dir(&month_dir)?;
    }

    let (batch_dir, _batch_name) = create_batch_dir(&results_dir, &now)?;
    let duplicate_dir = batch_dir.join(DUPLICATES_DIR);
    let unique_dir = batch_dir.join(UNIQUES_DIR);
    if let Err(error) = new_generated_dir(&duplicate_dir).and_then(|_| new_generated_dir(&unique_dir)) {
        let _ = fs::remove_dir_all(&batch_dir);
        return Err(error);
    }

    let result_rel_path = path_to_rel(&ctx.root, &batch_dir)?;
    let started_at = now.to_rfc3339();
    let batch_id = {
        let db = lock_db(ctx)?;
        queries::insert_running_batch(
            &db,
            merchant_id,
            kind.as_str(),
            options.match_mode.as_str(),
            options.visual_threshold,
            &absolute_string(&source),
            &result_rel_path,
            &import_month,
            &started_at,
        )?
    };

    Ok(PreparedBatch {
        batch_id,
        merchant_id,
        merchant_name: merchant.folder_name,
        kind,
        match_mode: options.match_mode,
        visual_threshold: options.visual_threshold,
        source,
        result_dir: batch_dir,
        result_rel_path,
        library_dir,
        import_month,
        started_at,
    })
}

pub fn run_prepared(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    cancel: Arc<AtomicBool>,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> AppResult<BatchFinish> {
    let mut state = RunState::default();
    let result = execute(ctx, prepared, &cancel, progress, &mut state);
    let status = match result {
        Ok(()) => RunStatus::Completed,
        Err(AppError::Cancelled) => RunStatus::Cancelled,
        Err(error) => RunStatus::Failed(error.to_string()),
    };

    progress(ProgressUpdate {
        phase: JobPhase::Finalizing,
        total: state.items.len() as u64,
        processed: state.items.len() as u64,
        message: Some("正在生成批次报告…".to_string()),
        ..Default::default()
    });

    finalize(ctx, prepared, &state, status, progress)
}

fn execute(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
    state: &mut RunState,
) -> AppResult<()> {
    progress(ProgressUpdate {
        phase: JobPhase::Scanning,
        message: Some("正在扫描来源目录…".to_string()),
        ..Default::default()
    });
    let scan = scan_source(&prepared.source, cancel)?;
    state.scan_warnings = scan.warnings;
    state.scanned_total = scan.files.len() as u64;
    state.items = scan
        .files
        .into_iter()
        .enumerate()
        .map(|(index, file)| WorkItem::new(index as i64 + 1, file))
        .collect();

    progress(ProgressUpdate {
        phase: JobPhase::Scanning,
        processed: state.scanned_total,
        total: state.scanned_total,
        message: Some(format!("扫描完成，共 {} 个图片文件。", state.scanned_total)),
        ..Default::default()
    });

    let pool = build_worker_pool()?;
    hash_all(&mut state.items, &pool, cancel, progress)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::Cancelled);
    }

    if prepared.match_mode == MatchMode::Visual {
        ensure_library_visuals(ctx, prepared, &pool, cancel, progress)?;
        visual_hash_items(&mut state.items, &pool, cancel, progress)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
    }

    let mut counts = BatchCounts {
        total_files: state.items.len() as u64,
        ..Default::default()
    };
    classify(ctx, prepared, &mut state.items, cancel, progress, &mut counts)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::Cancelled);
    }

    let new_items: Vec<NewBatchItem> = state.items.iter().map(new_batch_item).collect();
    let ids = {
        let mut db = lock_db(ctx)?;
        queries::insert_batch_items(&mut db, prepared.batch_id, &new_items)?
    };
    if ids.len() != state.items.len() {
        return Err(AppError::user(
            "INTERNAL_ERROR",
            "批次条目写入数量不一致。",
        ));
    }
    for (item, id) in state.items.iter_mut().zip(ids) {
        item.item_id = Some(id);
    }
    state.inserted = true;

    ensure_disk_space(ctx, &state.items)?;

    copy_all(
        ctx,
        prepared,
        &mut state.items,
        cancel,
        progress,
        &mut counts,
    )?;

    Ok(())
}

fn build_worker_pool() -> AppResult<rayon::ThreadPool> {
    let threads = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .min(4);
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|error| {
            AppError::user("THREAD_POOL_FAILED", format!("无法启动处理线程：{error}"))
        })
}

fn hash_all(
    items: &mut [WorkItem],
    pool: &rayon::ThreadPool,
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> AppResult<()> {
    let total = items
        .iter()
        .filter(|item| item.file.size_bytes > 0)
        .count() as u64;
    if total == 0 {
        return Ok(());
    }
    progress(ProgressUpdate {
        phase: JobPhase::Hashing,
        total,
        message: Some("正在计算 SHA-256…".to_string()),
        ..Default::default()
    });

    let processed = AtomicU64::new(0);

    pool.install(|| {
        items.par_iter_mut().for_each(|item| {
            if item.file.size_bytes <= 0 {
                return;
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match hash_file(&item.file.abs_path, cancel) {
                Ok(hash) => item.hash = Some(hash),
                Err(AppError::Cancelled) => {}
                Err(error) => {
                    item.outcome = Some("error".to_string());
                    item.error_code = Some(error.code());
                    item.error_message = Some(error.to_string());
                }
            }
            let done = processed.fetch_add(1, Ordering::Relaxed) + 1;
            if done == total || done % 64 == 0 {
                progress(ProgressUpdate {
                    phase: JobPhase::Hashing,
                    processed: done,
                    total,
                    message: Some(format!("已计算 {done}/{total} 个文件的哈希")),
                    ..Default::default()
                });
            }
        });
    });

    Ok(())
}

fn ensure_library_visuals(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    pool: &rayon::ThreadPool,
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> AppResult<()> {
    let pending = {
        let db = lock_db(ctx)?;
        queries::load_pending_visuals(&db, prepared.merchant_id, HASH_VERSION)?
    };
    if pending.is_empty() {
        return Ok(());
    }

    let total = pending.len() as u64;
    progress(ProgressUpdate {
        phase: JobPhase::Fingerprint,
        total,
        message: Some(format!("正在为主库 {total} 张图片建立视觉指纹…")),
        ..Default::default()
    });

    let processed = AtomicU64::new(0);
    let results: Vec<NewImageVisual> = pool.install(|| {
        pending
            .par_iter()
            .filter_map(|entry| {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                let path = rel_to_path(&ctx.root, &entry.library_rel_path);
                let hashes = visual::compute(&path).ok();
                let done = processed.fetch_add(1, Ordering::Relaxed) + 1;
                if done == total || done % 64 == 0 {
                    progress(ProgressUpdate {
                        phase: JobPhase::Fingerprint,
                        processed: done,
                        total,
                        message: Some(format!("主库视觉指纹 {done}/{total}")),
                        ..Default::default()
                    });
                }
                hashes.map(|hashes| NewImageVisual {
                    image_id: entry.image_id,
                    hashes,
                })
            })
            .collect()
    });

    {
        let mut db = lock_db(ctx)?;
        queries::save_image_visuals(&mut db, &results, &Local::now().to_rfc3339())?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::Cancelled);
    }
    Ok(())
}

fn visual_hash_items(
    items: &mut [WorkItem],
    pool: &rayon::ThreadPool,
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> AppResult<()> {
    let total = items
        .iter()
        .filter(|item| item.file.size_bytes > 0 && item.outcome.as_deref() != Some("error"))
        .count() as u64;
    if total == 0 {
        return Ok(());
    }
    progress(ProgressUpdate {
        phase: JobPhase::Fingerprint,
        total,
        message: Some("正在生成来源图片的视觉指纹…".to_string()),
        ..Default::default()
    });

    let processed = AtomicU64::new(0);
    pool.install(|| {
        items.par_iter_mut().for_each(|item| {
            if item.file.size_bytes <= 0 || item.outcome.as_deref() == Some("error") {
                return;
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match visual::compute(&item.file.abs_path) {
                Ok(hashes) => item.visual = Some(hashes),
                Err(AppError::Cancelled) => {}
                Err(error) => {
                    item.warning_code = Some("VISUAL_UNAVAILABLE".to_string());
                    item.warning_message =
                        Some(format!("{error}本文件仅参与精确比对。"));
                }
            }
            let done = processed.fetch_add(1, Ordering::Relaxed) + 1;
            if done == total || done % 64 == 0 {
                progress(ProgressUpdate {
                    phase: JobPhase::Fingerprint,
                    processed: done,
                    total,
                    message: Some(format!("来源视觉指纹 {done}/{total}")),
                    ..Default::default()
                });
            }
        });
    });
    Ok(())
}

fn classify(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    items: &mut [WorkItem],
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
    counts: &mut BatchCounts,
) -> AppResult<()> {
    let library = {
        let db = lock_db(ctx)?;
        queries::load_library_images(&db, prepared.merchant_id)?
    };
    let visual_library = if prepared.match_mode == MatchMode::Visual {
        let db = lock_db(ctx)?;
        queries::load_library_visuals(&db, prepared.merchant_id, HASH_VERSION)?
    } else {
        Vec::new()
    };
    let total = items.len() as u64;
    progress(ProgressUpdate {
        phase: JobPhase::Comparing,
        total,
        message: Some("正在与主库比对…".to_string()),
        unique_count: counts.unique_count,
        ..Default::default()
    });

    let mut visual_matches: Vec<Option<VisualLibraryMatch>> = vec![None; items.len()];
    if prepared.match_mode == MatchMode::Visual && !visual_library.is_empty() {
        progress(ProgressUpdate {
            phase: JobPhase::Comparing,
            total,
            message: Some("正在执行主库视觉相似比对…".to_string()),
            unique_count: counts.unique_count,
            ..Default::default()
        });
        visual_matches = find_library_visual_matches(
            items,
            &library,
            &visual_library,
            prepared.visual_threshold,
            cancel,
        );
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
    }

    let mut seen_in_batch: HashMap<String, (i64, i64)> = HashMap::new();
    let mut seen_visual: Vec<SeenVisual> = Vec::new();
    let mut processed = 0u64;

    for (index, item) in items.iter_mut().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
        processed += 1;
        if processed % 256 == 0 || processed == total {
            progress(ProgressUpdate {
                phase: JobPhase::Comparing,
                processed,
                total,
                unique_count: counts.unique_count,
                duplicate_existing_count: counts.duplicate_existing_count,
                duplicate_in_batch_count: counts.duplicate_in_batch_count,
                skipped_count: counts.skipped_count,
                error_count: counts.error_count,
                warning_count: counts.warning_count,
                message: Some(format!("已比对 {processed}/{total} 个文件")),
                current_file: None,
            });
        }

        if item.warning_code.as_deref() == Some("VISUAL_UNAVAILABLE") {
            counts.warning_count += 1;
        }

        if item.file.size_bytes == 0 {
            item.outcome = Some("skipped".to_string());
            item.warning_code = Some("EMPTY_FILE".to_string());
            item.warning_message = Some("文件大小为 0，已跳过。".to_string());
            counts.skipped_count += 1;
            counts.warning_count += 1;
            continue;
        }
        if item.outcome.as_deref() == Some("error") {
            counts.error_count += 1;
            continue;
        }
        let Some(hash) = item.hash.clone() else {
            set_item_error(
                item,
                "INTERNAL_ERROR",
                "文件哈希缺失，已按异常跳过。".to_string(),
            );
            counts.error_count += 1;
            continue;
        };

        if let Some(existing) = library.get(&hash) {
            if existing.size_bytes != item.file.size_bytes {
                set_item_error(
                    item,
                    "HASH_COLLISION",
                    "哈希与主库记录相同但文件大小不同，已按异常处理。".to_string(),
                );
                counts.error_count += 1;
                continue;
            }
            let path = rel_to_path(&ctx.root, &existing.library_rel_path);
            match fs::metadata(&path) {
                Ok(metadata) if metadata.len() == existing.size_bytes as u64 => {
                    item.outcome = Some("duplicate_existing".to_string());
                    item.duplicate_of_image_id = Some(existing.id);
                    item.duplicate_kind = Some("exact".to_string());
                    counts.duplicate_existing_count += 1;
                }
                Ok(_) => {
                    set_item_error(
                        item,
                        "LIBRARY_FILE_SIZE_MISMATCH",
                        format!("主库文件大小异常：{}。", path.display()),
                    );
                    counts.error_count += 1;
                }
                Err(_) => {
                    set_item_error(
                        item,
                        "LIBRARY_FILE_MISSING",
                        format!(
                            "主库中的重复文件不存在：{}。请恢复主库文件，勿删除索引。",
                            path.display()
                        ),
                    );
                    counts.error_count += 1;
                }
            }
            continue;
        }

        if let Some(candidate) = visual_matches.get(index).and_then(Option::as_ref) {
            let existing = &visual_library[candidate.library_index];
            let path = rel_to_path(&ctx.root, &existing.library_rel_path);
            if fs::metadata(&path).is_ok() {
                item.outcome = Some("duplicate_existing".to_string());
                item.duplicate_of_image_id = Some(existing.image_id);
                item.duplicate_kind = Some("visual".to_string());
                item.visual_distance = Some(candidate.distance);
                counts.duplicate_existing_count += 1;
            } else {
                set_item_error(
                    item,
                    "LIBRARY_FILE_MISSING",
                    format!(
                        "主库中的相似文件不存在：{}。请恢复主库文件，勿删除索引。",
                        path.display()
                    ),
                );
                counts.error_count += 1;
            }
            continue;
        }

        if let Some((first_seq, first_size)) = seen_in_batch.get(&hash) {
            if *first_size != item.file.size_bytes {
                set_item_error(
                    item,
                    "HASH_COLLISION",
                    "同批次内哈希相同但文件大小不同，已按异常处理。".to_string(),
                );
                counts.error_count += 1;
            } else {
                item.outcome = Some("duplicate_in_batch".to_string());
                item.duplicate_of_seq = Some(*first_seq);
                item.duplicate_kind = Some("exact".to_string());
                counts.duplicate_in_batch_count += 1;
            }
            continue;
        }

        if prepared.match_mode == MatchMode::Visual {
            if let Some(hashes) = &item.visual {
                if let Some((first_seq, distance)) =
                    find_batch_visual(&seen_visual, hashes, prepared.visual_threshold)
                {
                    item.outcome = Some("duplicate_in_batch".to_string());
                    item.duplicate_of_seq = Some(first_seq);
                    item.duplicate_kind = Some("visual".to_string());
                    item.visual_distance = Some(distance);
                    counts.duplicate_in_batch_count += 1;
                    continue;
                }
            }
        }

        seen_in_batch.insert(hash, (item.seq, item.file.size_bytes));
        if let Some(hashes) = &item.visual {
            seen_visual.push(SeenVisual {
                seq: item.seq,
                hash0: hashes.rotations[0],
                aspect_ratio: hashes.aspect_ratio(),
            });
        }
        item.outcome = Some("unique".to_string());
        counts.unique_count += 1;
    }

    progress(ProgressUpdate {
        phase: JobPhase::Comparing,
        processed: total,
        total,
        unique_count: counts.unique_count,
        duplicate_existing_count: counts.duplicate_existing_count,
        duplicate_in_batch_count: counts.duplicate_in_batch_count,
        skipped_count: counts.skipped_count,
        error_count: counts.error_count,
        warning_count: counts.warning_count,
        message: Some("比对完成。".to_string()),
        current_file: None,
    });
    Ok(())
}

#[derive(Clone, Copy)]
struct VisualLibraryMatch {
    library_index: usize,
    image_id: i64,
    distance: u32,
}

struct SeenVisual {
    seq: i64,
    hash0: u64,
    aspect_ratio: f32,
}

fn find_library_visual_matches(
    items: &[WorkItem],
    library: &HashMap<String, LibraryImage>,
    visuals: &[LibraryVisual],
    threshold: u32,
    cancel: &AtomicBool,
) -> Vec<Option<VisualLibraryMatch>> {
    items
        .par_iter()
        .map(|item| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if item.file.size_bytes <= 0 || item.outcome.as_deref() == Some("error") {
                return None;
            }
            let hashes = item.visual.as_ref()?;
            if let Some(hash) = &item.hash {
                if library.contains_key(hash) {
                    return None;
                }
            }

            let aspect = hashes.aspect_ratio();
            let mut best: Option<VisualLibraryMatch> = None;
            for (index, entry) in visuals.iter().enumerate() {
                if !visual::aspect_compatible(aspect, entry.aspect_ratio) {
                    continue;
                }
                let distance = visual::best_distance(&hashes.rotations, entry.rotations[0]);
                if distance > threshold {
                    continue;
                }
                let better = best.as_ref().map_or(true, |current| {
                    distance < current.distance
                        || (distance == current.distance && entry.image_id < current.image_id)
                });
                if better {
                    best = Some(VisualLibraryMatch {
                        library_index: index,
                        image_id: entry.image_id,
                        distance,
                    });
                }
            }
            best
        })
        .collect()
}

fn find_batch_visual(
    seen: &[SeenVisual],
    hashes: &VisualHashes,
    threshold: u32,
) -> Option<(i64, u32)> {
    let aspect = hashes.aspect_ratio();
    let mut best: Option<(i64, u32)> = None;
    for candidate in seen {
        if !visual::aspect_compatible(aspect, candidate.aspect_ratio) {
            continue;
        }
        let distance = visual::best_distance(&hashes.rotations, candidate.hash0);
        if distance > threshold {
            continue;
        }
        if best.map_or(true, |(_, current)| distance < current) {
            best = Some((candidate.seq, distance));
        }
    }
    best
}

fn copy_all(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    items: &mut [WorkItem],
    cancel: &AtomicBool,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
    counts: &mut BatchCounts,
) -> AppResult<()> {
    let unique_dir = prepared.result_dir.join(UNIQUES_DIR);
    let duplicate_dir = prepared.result_dir.join(DUPLICATES_DIR);
    let mut used_unique = HashSet::new();
    let mut used_duplicate = HashSet::new();
    let total = items.len() as u64;

    progress(ProgressUpdate {
        phase: JobPhase::Copying,
        total,
        unique_count: counts.unique_count,
        duplicate_existing_count: counts.duplicate_existing_count,
        duplicate_in_batch_count: counts.duplicate_in_batch_count,
        skipped_count: counts.skipped_count,
        error_count: counts.error_count,
        warning_count: counts.warning_count,
        message: Some("正在复制图片…".to_string()),
        ..Default::default()
    });

    for (index, item) in items.iter_mut().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Cancelled);
        }
        let before_outcome = item.outcome.clone();
        let before_warning = item.warning_code.is_some();

        match item.outcome.as_deref().unwrap_or_default() {
            "unique" => copy_unique(
                ctx,
                prepared,
                item,
                &unique_dir,
                &mut used_unique,
                cancel,
            )?,
            "duplicate_existing" | "duplicate_in_batch" => copy_duplicate(
                ctx,
                prepared,
                item,
                &duplicate_dir,
                &mut used_duplicate,
                cancel,
            )?,
            _ => {}
        }

        if before_outcome != item.outcome {
            decrement_outcome(counts, before_outcome.as_deref());
            increment_outcome(counts, item.outcome.as_deref());
        }
        if !before_warning && item.warning_code.is_some() {
            counts.warning_count += 1;
        }

        let processed = index as u64 + 1;
        if processed % 16 == 0 || processed == total {
            progress(ProgressUpdate {
                phase: JobPhase::Copying,
                processed,
                total,
                current_file: Some(item.file.file_name.clone()),
                unique_count: counts.unique_count,
                duplicate_existing_count: counts.duplicate_existing_count,
                duplicate_in_batch_count: counts.duplicate_in_batch_count,
                skipped_count: counts.skipped_count,
                error_count: counts.error_count,
                warning_count: counts.warning_count,
                message: Some(format!("已处理 {processed}/{total} 个文件")),
            });
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_unique(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    item: &mut WorkItem,
    unique_dir: &Path,
    used_unique: &mut HashSet<String>,
    cancel: &AtomicBool,
) -> AppResult<()> {
    let Some(hash) = item.hash.clone() else {
        mark_item_error(
            ctx,
            item,
            &AppError::user("INTERNAL_ERROR", "唯一图片缺少哈希，无法入库。"),
        )?;
        return Ok(());
    };
    let month_dir = prepared.library_dir.join(&prepared.import_month);
    let target = match resolve_library_target(
        &month_dir,
        &item.file.file_name,
        &hash,
        item.file.size_bytes,
    ) {
        Ok(target) => target,
        Err(error) => {
            mark_item_error(ctx, item, &error)?;
            return Ok(());
        }
    };

    let mut created = false;
    if !target.adopted {
        match copy_verified(&item.file.abs_path, &target.path, &hash, cancel) {
            Ok(()) => created = true,
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                mark_item_error(ctx, item, &error)?;
                return Ok(());
            }
        }
    }

    let library_rel_path = path_to_rel(&ctx.root, &target.path)?;
    let item_id = item.item_id.ok_or_else(|| {
        AppError::user("INTERNAL_ERROR", "批次条目缺少数据库标识。")
    })?;
    let new_image = NewLibraryImage {
        merchant_id: prepared.merchant_id,
        sha256: hash.clone(),
        size_bytes: item.file.size_bytes,
        library_rel_path,
        original_name: item.file.file_name.clone(),
        original_source_path: absolute_string(&item.file.abs_path),
        imported_at: Local::now().to_rfc3339(),
        batch_id: prepared.batch_id,
        visual: item.visual,
    };

    let commit = {
        let mut db = lock_db(ctx)?;
        queries::commit_library_image(&mut db, &new_image, item_id)
    };
    if let Err(error) = commit {
        if created {
            let _ = fs::remove_file(&target.path);
        }
        return Err(error);
    }
    copy_to_result(ctx, item, unique_dir, used_unique, cancel)
}

fn copy_duplicate(
    ctx: &RootContext,
    _prepared: &PreparedBatch,
    item: &mut WorkItem,
    duplicate_dir: &Path,
    used_duplicate: &mut HashSet<String>,
    cancel: &AtomicBool,
) -> AppResult<()> {
    copy_to_result(ctx, item, duplicate_dir, used_duplicate, cancel)
}

fn copy_to_result(
    ctx: &RootContext,
    item: &mut WorkItem,
    target_dir: &Path,
    used: &mut HashSet<String>,
    cancel: &AtomicBool,
) -> AppResult<()> {
    let file_name = match pick_result_name(target_dir, &item.file.file_name, used) {
        Ok(name) => name,
        Err(error) => {
            mark_item_warning(ctx, item, "RESULT_NAME_CONFLICT", &error.to_string())?;
            return Ok(());
        }
    };
    let target = target_dir.join(&file_name);
    let hash = item.hash.clone().unwrap_or_default();
    match copy_verified(&item.file.abs_path, &target, &hash, cancel) {
        Ok(()) => {
            let relative = path_to_rel(&ctx.root, &target)?;
            let item_id = item.item_id.ok_or_else(|| {
                AppError::user("INTERNAL_ERROR", "批次条目缺少数据库标识。")
            })?;
            {
                let db = lock_db(ctx)?;
                queries::set_item_result(&db, item_id, &relative)?;
            }
        }
        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
        Err(error) => {
            let message = format!("结果文件复制失败：{error}");
            mark_item_warning(ctx, item, "RESULT_COPY_FAILED", &message)?;
        }
    }
    Ok(())
}

fn resolve_library_target(
    month_dir: &Path,
    original_name: &str,
    hash: &str,
    size_bytes: i64,
) -> AppResult<LibraryTarget> {
    for prefix_len in [16usize, 24, 32, 40, 48, 56, 64] {
        let file_name = master_file_name(original_name, hash, prefix_len);
        let path = month_dir.join(&file_name);
        if !path.exists() {
            return Ok(LibraryTarget {
                path,
                adopted: false,
            });
        }
        let metadata = fs::metadata(&path)?;
        if metadata.len() != size_bytes as u64 {
            continue;
        }
        if let Ok((existing_hash, _)) = hash_existing_file(&path) {
            if existing_hash == hash {
                return Ok(LibraryTarget {
                    path,
                    adopted: true,
                });
            }
        }
    }
    Err(AppError::user(
        "LIBRARY_NAME_CONFLICT",
        "主库中同名文件过多，无法生成安全文件名。",
    ))
}

fn finalize(
    ctx: &RootContext,
    prepared: &PreparedBatch,
    state: &RunState,
    status: RunStatus,
    progress: &(dyn Fn(ProgressUpdate) + Send + Sync),
) -> AppResult<BatchFinish> {
    let (counts, report_items) = if state.inserted {
        let db = lock_db(ctx)?;
        (
            queries::compute_batch_counts(&db, prepared.batch_id)?,
            queries::report_items(&db, prepared.batch_id)?,
        )
    } else {
        let items = report_items_from_state(&state.items, &status);
        (counts_from_report_items(&items), items)
    };

    let finished_at = Local::now().to_rfc3339();
    let (status_text, error_text) = match &status {
        RunStatus::Completed => {
            if counts.error_count > 0 {
                ("completed_with_errors", None)
            } else {
                ("completed", None)
            }
        }
        RunStatus::Cancelled => ("cancelled", None),
        RunStatus::Failed(message) => ("failed", Some(message.clone())),
    };
    let mut note = match &status {
        RunStatus::Cancelled => Some("任务已取消，已处理的文件已保留。".to_string()),
        RunStatus::Failed(message) => Some(format!("批次失败：{message}")),
        RunStatus::Completed => None,
    };
    if !state.inserted && !state.items.is_empty() {
        note = Some(merge_note(
            note,
            "任务在写入索引前中止，未处理文件在报告中标记为已跳过。",
        ));
    }

    let meta = ReportMeta {
        batch_id: prepared.batch_id,
        batch_type: prepared.kind.as_str(),
        merchant_name: &prepared.merchant_name,
        source_dir: &prepared.source,
        result_dir: &prepared.result_dir,
        started_at: &prepared.started_at,
        finished_at: &finished_at,
        status: status_text,
        match_mode: prepared.match_mode.as_str(),
        visual_threshold: prepared.visual_threshold,
    };
    let report_paths = write_batch_reports(
        &ctx.root,
        &meta,
        &counts,
        &report_items,
        &state.scan_warnings,
        error_text.as_deref(),
    );
    let (report_rel_path, summary_rel_path) = match report_paths {
        Ok((csv, summary)) => (Some(csv), Some(summary)),
        Err(error) => {
            note = Some(merge_note(
                note,
                &format!("报告生成失败：{error}。可在历史记录中重新生成。"),
            ));
            (None, None)
        }
    };

    {
        let db = lock_db(ctx)?;
        queries::finalize_batch(
            &db,
            prepared.batch_id,
            status_text,
            &finished_at,
            &counts,
            report_rel_path.as_deref(),
            summary_rel_path.as_deref(),
            note.as_deref(),
        )?;
        queries::checkpoint(&db);
    }

    if matches!(status, RunStatus::Completed) {
        backup_database(ctx);
    }

    progress(ProgressUpdate {
        phase: JobPhase::Finalizing,
        processed: counts.total_files,
        total: counts.total_files,
        unique_count: counts.unique_count,
        duplicate_existing_count: counts.duplicate_existing_count,
        duplicate_in_batch_count: counts.duplicate_in_batch_count,
        skipped_count: counts.skipped_count,
        error_count: counts.error_count,
        warning_count: counts.warning_count,
        message: Some("批次处理结束。".to_string()),
        current_file: None,
    });

    let batch = {
        let db = lock_db(ctx)?;
        queries::get_batch_summary(&db, &ctx.root, prepared.batch_id)?
    }
    .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录丢失。"))?;

    Ok(BatchFinish {
        batch,
        error: error_text,
    })
}

pub fn regenerate_report(ctx: &RootContext, batch_id: i64) -> AppResult<crate::models::BatchSummary> {
    let (batch, counts, items) = {
        let db = lock_db(ctx)?;
        let batch = queries::get_batch_summary(&db, &ctx.root, batch_id)?
            .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录不存在。"))?;
        let counts = queries::compute_batch_counts(&db, batch_id)?;
        let items = queries::report_items(&db, batch_id)?;
        (batch, counts, items)
    };

    let finished_at = batch
        .finished_at
        .clone()
        .unwrap_or_else(|| batch.started_at.clone());
    let meta = ReportMeta {
        batch_id,
        batch_type: &batch.batch_type,
        merchant_name: &batch.merchant_name,
        source_dir: Path::new(&batch.source_abs_path),
        result_dir: Path::new(&batch.result_abs_path),
        started_at: &batch.started_at,
        finished_at: &finished_at,
        status: &batch.status,
        match_mode: &batch.match_mode,
        visual_threshold: batch.visual_threshold,
    };
    let (report_rel_path, summary_rel_path) =
        write_batch_reports(&ctx.root, &meta, &counts, &items, &[], None)?;

    let db = lock_db(ctx)?;
    queries::set_batch_report_paths(
        &db,
        batch_id,
        Some(&report_rel_path),
        Some(&summary_rel_path),
    )?;
    queries::get_batch_summary(&db, &ctx.root, batch_id)?
        .ok_or_else(|| AppError::user("BATCH_NOT_FOUND", "批次记录不存在。"))
}

fn create_batch_dir(results_dir: &Path, now: &DateTime<Local>) -> AppResult<(PathBuf, String)> {
    let base = now.format("%Y-%m-%d_%H-%M-%S").to_string();
    for attempt in 1..=200u32 {
        let name = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}_{attempt:02}")
        };
        let path = results_dir.join(&name);
        match fs::create_dir(&path) {
            Ok(()) => {
                if let Err(error) = write_generated_marker(&path) {
                    let _ = fs::remove_dir(&path);
                    return Err(error);
                }
                return Ok((path, name));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::user(
        "BATCH_DIR_CONFLICT",
        "同一秒内的批次目录过多，请稍后重试。",
    ))
}

fn ensure_disk_space(ctx: &RootContext, items: &[WorkItem]) -> AppResult<()> {
    let needed: u128 = items
        .iter()
        .filter_map(|item| match item.outcome.as_deref() {
            Some("unique") => Some(item.file.size_bytes.max(0) as u128 * 2),
            Some("duplicate_existing") | Some("duplicate_in_batch") => {
                Some(item.file.size_bytes.max(0) as u128)
            }
            _ => None,
        })
        .sum();
    if needed == 0 {
        return Ok(());
    }
    let free = available_disk_space(&ctx.root)? as u128;
    if needed > free {
        return Err(AppError::user(
            "INSUFFICIENT_DISK_SPACE",
            format!(
                "磁盘可用空间不足：本批次预计需要 {}，当前可用 {}。请清理磁盘后重试。",
                format_bytes(needed),
                format_bytes(free)
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn available_disk_space(path: &Path) -> AppResult<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(AppError::user(
            "DISK_SPACE_CHECK_FAILED",
            "无法读取磁盘可用空间，请检查磁盘状态。",
        ));
    }
    Ok(free)
}

#[cfg(not(windows))]
fn available_disk_space(_path: &Path) -> AppResult<u64> {
    Ok(u64::MAX)
}

fn format_bytes(value: u128) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value, UNITS[unit])
    } else {
        format!("{size:.2} {}", UNITS[unit])
    }
}

fn backup_database(ctx: &RootContext) {
    let paths = RootPaths::from_root(&ctx.root);
    if fs::create_dir_all(&paths.backups).is_err() {
        return;
    }
    let file_name = format!(
        "index-{}.db",
        Local::now().format("%Y%m%d-%H%M%S-%3f")
    );
    let target = paths.backups.join(file_name);
    if let Ok(db) = ctx.db.lock() {
        let escaped = target.to_string_lossy().replace('\'', "''");
        if db
            .execute_batch(&format!("VACUUM INTO '{escaped}';"))
            .is_err()
        {
            return;
        }
    } else {
        return;
    }

    let mut files: Vec<(String, PathBuf)> = fs::read_dir(&paths.backups)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("index-") && name.ends_with(".db") {
                        Some((name, entry.path()))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in files.into_iter().skip(3) {
        let _ = fs::remove_file(path);
    }
}

fn new_batch_item(item: &WorkItem) -> NewBatchItem {
    NewBatchItem {
        seq: item.seq,
        source_abs_path: absolute_string(&item.file.abs_path),
        source_rel_path: item.file.rel_path.clone(),
        file_name: item.file.file_name.clone(),
        size_bytes: item.file.size_bytes,
        mtime_ms: item.file.mtime_ms,
        sha256: item.hash.clone(),
        outcome: item
            .outcome
            .clone()
            .unwrap_or_else(|| "skipped".to_string()),
        duplicate_kind: item.duplicate_kind.clone(),
        visual_distance: item.visual_distance,
        duplicate_of_image_id: item.duplicate_of_image_id,
        duplicate_of_seq: item.duplicate_of_seq,
        error_code: item.error_code.clone(),
        error_message: item.error_message.clone(),
        warning_code: item.warning_code.clone(),
        warning_message: item.warning_message.clone(),
    }
}

fn report_items_from_state(items: &[WorkItem], status: &RunStatus) -> Vec<BatchReportItem> {
    let by_seq: HashMap<i64, &WorkItem> = items.iter().map(|item| (item.seq, item)).collect();
    let unprocessed_message = match status {
        RunStatus::Cancelled => "任务取消前未处理该文件。",
        _ => "任务中止前未处理该文件。",
    };

    items
        .iter()
        .map(|item| {
            let mut outcome = item
                .outcome
                .clone()
                .unwrap_or_else(|| "skipped".to_string());
            let mut warning_code = item.warning_code.clone();
            let mut warning_message = item.warning_message.clone();
            if item.outcome.is_none() {
                outcome = "skipped".to_string();
                warning_code = Some("NOT_PROCESSED".to_string());
                warning_message = Some(unprocessed_message.to_string());
            }
            let (duplicate_item_name, duplicate_item_source) = item
                .duplicate_of_seq
                .and_then(|seq| by_seq.get(&seq))
                .map(|first| {
                    (
                        Some(first.file.file_name.clone()),
                        Some(absolute_string(&first.file.abs_path)),
                    )
                })
                .unwrap_or((None, None));

            BatchReportItem {
                seq: item.seq,
                file_name: item.file.file_name.clone(),
                source_abs_path: absolute_string(&item.file.abs_path),
                source_rel_path: item.file.rel_path.clone(),
                size_bytes: Some(item.file.size_bytes),
                sha256: item.hash.clone(),
                outcome,
                duplicate_kind: item.duplicate_kind.clone(),
                visual_distance: item.visual_distance,
                library_rel_path: None,
                result_rel_path: None,
                error_code: item.error_code.clone(),
                error_message: item.error_message.clone(),
                warning_code,
                warning_message,
                duplicate_image_name: None,
                duplicate_image_path: None,
                duplicate_item_name,
                duplicate_item_source,
            }
        })
        .collect()
}

fn counts_from_report_items(items: &[BatchReportItem]) -> BatchCounts {
    let mut counts = BatchCounts {
        total_files: items.len() as u64,
        ..Default::default()
    };
    for item in items {
        match item.outcome.as_str() {
            "unique" => counts.unique_count += 1,
            "duplicate_existing" => counts.duplicate_existing_count += 1,
            "duplicate_in_batch" => counts.duplicate_in_batch_count += 1,
            "skipped" => counts.skipped_count += 1,
            "error" => counts.error_count += 1,
            _ => {}
        }
        if item.warning_code.is_some() {
            counts.warning_count += 1;
        }
    }
    counts
}

fn lock_db(ctx: &RootContext) -> AppResult<MutexGuard<'_, Connection>> {
    ctx.db.lock().map_err(|_| {
        AppError::user("DB_LOCK_POISONED", "索引库状态异常，请重启程序。")
    })
}

fn set_item_error(item: &mut WorkItem, code: &str, message: String) {
    item.outcome = Some("error".to_string());
    item.error_code = Some(code.to_string());
    item.error_message = Some(message);
}

fn mark_item_error(ctx: &RootContext, item: &mut WorkItem, error: &AppError) -> AppResult<()> {
    let item_id = item.item_id.ok_or_else(|| {
        AppError::user("INTERNAL_ERROR", "批次条目缺少数据库标识。")
    })?;
    {
        let db = lock_db(ctx)?;
        queries::set_item_error(&db, item_id, &error.code(), &error.to_string())?;
    }
    set_item_error(item, &error.code(), error.to_string());
    Ok(())
}

fn mark_item_warning(
    ctx: &RootContext,
    item: &mut WorkItem,
    code: &str,
    message: &str,
) -> AppResult<()> {
    let item_id = item.item_id.ok_or_else(|| {
        AppError::user("INTERNAL_ERROR", "批次条目缺少数据库标识。")
    })?;
    {
        let db = lock_db(ctx)?;
        queries::set_item_warning(&db, item_id, code, message)?;
    }
    item.warning_code = Some(code.to_string());
    item.warning_message = Some(message.to_string());
    Ok(())
}

fn increment_outcome(counts: &mut BatchCounts, outcome: Option<&str>) {
    match outcome {
        Some("unique") => counts.unique_count += 1,
        Some("duplicate_existing") => counts.duplicate_existing_count += 1,
        Some("duplicate_in_batch") => counts.duplicate_in_batch_count += 1,
        Some("skipped") => counts.skipped_count += 1,
        Some("error") => counts.error_count += 1,
        _ => {}
    }
}

fn decrement_outcome(counts: &mut BatchCounts, outcome: Option<&str>) {
    match outcome {
        Some("unique") => counts.unique_count = counts.unique_count.saturating_sub(1),
        Some("duplicate_existing") => {
            counts.duplicate_existing_count = counts.duplicate_existing_count.saturating_sub(1)
        }
        Some("duplicate_in_batch") => {
            counts.duplicate_in_batch_count = counts.duplicate_in_batch_count.saturating_sub(1)
        }
        Some("skipped") => counts.skipped_count = counts.skipped_count.saturating_sub(1),
        Some("error") => counts.error_count = counts.error_count.saturating_sub(1),
        _ => {}
    }
}

fn merge_note(existing: Option<String>, extra: &str) -> String {
    match existing {
        Some(note) if !note.is_empty() => format!("{note} {extra}"),
        _ => extra.to_string(),
    }
}

struct LibraryTarget {
    path: PathBuf,
    adopted: bool,
}

enum RunStatus {
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Default)]
struct RunState {
    items: Vec<WorkItem>,
    scan_warnings: Vec<String>,
    scanned_total: u64,
    inserted: bool,
}

struct WorkItem {
    seq: i64,
    file: ScannedFile,
    hash: Option<String>,
    visual: Option<VisualHashes>,
    outcome: Option<String>,
    duplicate_kind: Option<String>,
    visual_distance: Option<u32>,
    duplicate_of_image_id: Option<i64>,
    duplicate_of_seq: Option<i64>,
    error_code: Option<String>,
    error_message: Option<String>,
    warning_code: Option<String>,
    warning_message: Option<String>,
    item_id: Option<i64>,
}

impl WorkItem {
    fn new(seq: i64, file: ScannedFile) -> Self {
        Self {
            seq,
            file,
            hash: None,
            visual: None,
            outcome: None,
            duplicate_kind: None,
            visual_distance: None,
            duplicate_of_image_id: None,
            duplicate_of_seq: None,
            error_code: None,
            error_message: None,
            warning_code: None,
            warning_message: None,
            item_id: None,
        }
    }
}
