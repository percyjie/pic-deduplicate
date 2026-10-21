import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ActiveJobSnapshot,
  BatchDetail,
  BatchKind,
  BatchPage,
  BatchSummary,
  JobFailedPayload,
  JobFinishedPayload,
  JobProgress,
  MatchMode,
  MerchantSummary,
  RootInspection,
  RootSnapshot,
  StartBatchResult,
} from "./types";

export function inspectRoot(path: string) {
  return invoke<RootInspection>("inspect_root", { path });
}

export function initializeRoot(path: string) {
  return invoke<RootSnapshot>("initialize_root", { path });
}

export function openRoot(path: string) {
  return invoke<RootSnapshot>("open_root", { path });
}

export function getRootSnapshot() {
  return invoke<RootSnapshot>("get_root_snapshot");
}

export function refreshMerchants() {
  return invoke<RootSnapshot>("refresh_merchants");
}

export function closeRoot() {
  return invoke<void>("close_root");
}

export function createMerchant(name: string) {
  return invoke<MerchantSummary>("create_merchant", { name });
}

export function startBatch(params: {
  merchantId: number;
  sourceDir: string;
  kind: BatchKind;
  matchMode: MatchMode;
  visualThreshold: number;
}) {
  return invoke<StartBatchResult>("start_batch", params);
}

export function cancelJob(jobId: string) {
  return invoke<void>("cancel_job", { jobId });
}

export function getActiveJob() {
  return invoke<ActiveJobSnapshot | null>("get_active_job");
}

export function listBatches(params: {
  merchantId?: number | null;
  page?: number;
  pageSize?: number;
}) {
  return invoke<BatchPage>("list_batches", {
    merchantId: params.merchantId ?? null,
    page: params.page ?? 1,
    pageSize: params.pageSize ?? 20,
  });
}

export function getBatchDetail(batchId: number) {
  return invoke<BatchDetail>("get_batch_detail", { batchId });
}

export function regenerateReport(batchId: number) {
  return invoke<BatchSummary>("regenerate_report", { batchId });
}

export function openResultDir(batchId: number) {
  return invoke<void>("open_result_dir", { batchId });
}

export function openReport(batchId: number) {
  return invoke<void>("open_report", { batchId });
}

export function openLibrary(merchantId: number) {
  return invoke<void>("open_library", { merchantId });
}

export function onJobProgress(handler: (payload: JobProgress) => void): Promise<UnlistenFn> {
  return listen<JobProgress>("job-progress", (event) => handler(event.payload));
}

export function onJobFinished(
  handler: (payload: JobFinishedPayload) => void,
): Promise<UnlistenFn> {
  return listen<JobFinishedPayload>("job-finished", (event) => handler(event.payload));
}

export function onJobFailed(
  handler: (payload: JobFailedPayload) => void,
): Promise<UnlistenFn> {
  return listen<JobFailedPayload>("job-failed", (event) => handler(event.payload));
}
