import type { BatchKind, BatchStatus, JobPhase } from "../api/types";

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? size : size.toFixed(2)} ${units[unit]}`;
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat("zh-CN").format(value);
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.length > 0) return message;
  }
  return "操作失败，请稍后重试。";
}

export function batchKindLabel(kind: BatchKind): string {
  return kind === "initial_import" ? "首次导入" : "日常批量去重";
}

export function batchStatusMeta(status: BatchStatus): {
  label: string;
  color: string;
} {
  switch (status) {
    case "completed":
      return { label: "已完成", color: "green" };
    case "completed_with_errors":
      return { label: "完成（有异常）", color: "orange" };
    case "cancelled":
      return { label: "已取消", color: "default" };
    case "interrupted":
      return { label: "已中断", color: "volcano" };
    case "failed":
      return { label: "失败", color: "red" };
    case "running":
      return { label: "执行中", color: "blue" };
    default:
      return { label: status, color: "default" };
  }
}

export function outcomeLabel(outcome: string): string {
  switch (outcome) {
    case "unique":
      return "不重复";
    case "duplicate_existing":
      return "重复（主库已有）";
    case "duplicate_in_batch":
      return "重复（同批次）";
    case "skipped":
      return "已跳过";
    case "error":
      return "处理失败";
    default:
      return outcome;
  }
}

export const phaseOrder: JobPhase[] = [
  "scanning",
  "hashing",
  "comparing",
  "copying",
  "finalizing",
];

export function phaseLabel(phase: JobPhase): string {
  switch (phase) {
    case "scanning":
      return "扫描文件";
    case "hashing":
      return "计算哈希";
    case "comparing":
      return "比对主库";
    case "copying":
      return "复制入库";
    case "finalizing":
      return "生成报告";
    default:
      return phase;
  }
}
