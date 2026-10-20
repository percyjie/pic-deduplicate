export type BatchKind = "initial_import" | "daily";

export type JobPhase =
  | "scanning"
  | "hashing"
  | "comparing"
  | "copying"
  | "finalizing";

export type BatchStatus =
  | "running"
  | "completed"
  | "completed_with_errors"
  | "cancelled"
  | "interrupted"
  | "failed";

export type BatchOutcome =
  | "unique"
  | "duplicate_existing"
  | "duplicate_in_batch"
  | "skipped"
  | "error";

export interface RootInspection {
  path: string;
  initialized: boolean;
  looksLikeMerchant: boolean;
  merchantDirCount: number;
  message: string | null;
}

export interface MerchantSummary {
  id: number;
  folderName: string;
  displayName: string;
  path: string;
  exists: boolean;
  imageCount: number;
  totalSize: number;
  lastBatchAt: string | null;
}

export interface RootSnapshot {
  rootPath: string;
  merchants: MerchantSummary[];
  totalImages: number;
  totalSize: number;
}

export interface JobProgress {
  jobId: string;
  batchId: number;
  merchantId: number;
  phase: JobPhase;
  processed: number;
  total: number;
  currentFile: string | null;
  uniqueCount: number;
  duplicateExistingCount: number;
  duplicateInBatchCount: number;
  skippedCount: number;
  errorCount: number;
  warningCount: number;
  message: string | null;
}

export interface ActiveJobSnapshot {
  jobId: string;
  batchId: number;
  merchantId: number;
  startedAt: string;
  progress: JobProgress | null;
}

export interface StartBatchResult {
  jobId: string;
  batchId: number;
  resultDir: string;
}

export interface BatchSummary {
  id: number;
  merchantId: number;
  merchantName: string;
  batchType: BatchKind;
  sourceAbsPath: string;
  resultAbsPath: string;
  status: BatchStatus;
  startedAt: string;
  finishedAt: string | null;
  totalFiles: number;
  uniqueCount: number;
  importedCount: number;
  duplicateExistingCount: number;
  duplicateInBatchCount: number;
  skippedCount: number;
  errorCount: number;
  warningCount: number;
  reportPath: string | null;
  note: string | null;
}

export interface BatchPage {
  items: BatchSummary[];
  total: number;
  page: number;
  pageSize: number;
}

export interface BatchIssue {
  seq: number;
  fileName: string;
  sourceAbsPath: string;
  outcome: BatchOutcome;
  errorMessage: string | null;
  warningMessage: string | null;
}

export interface BatchDetail {
  batch: BatchSummary;
  issues: BatchIssue[];
}

export interface JobFinishedPayload {
  jobId: string;
  batch: BatchSummary;
  error: string | null;
}

export interface JobFailedPayload {
  jobId: string;
  error: { code: string; message: string };
}

export interface AppErrorPayload {
  code?: string;
  message?: string;
}
