import { create } from "zustand";

import { getRootSnapshot } from "../api/commands";
import type {
  ActiveJobSnapshot,
  BatchKind,
  JobFinishedPayload,
  JobProgress,
  RootSnapshot,
} from "../api/types";

export type View = "overview" | "batch" | "history";

export interface PendingBatch {
  merchantId: number;
  kind: BatchKind;
  sourceDir?: string;
}

interface AppStore {
  snapshot: RootSnapshot | null;
  selectedMerchantId: number | null;
  view: View;
  activeJob: ActiveJobSnapshot | null;
  progress: JobProgress | null;
  lastFinished: JobFinishedPayload | null;
  pendingBatch: PendingBatch | null;
  setSnapshot: (snapshot: RootSnapshot) => void;
  refresh: () => Promise<RootSnapshot>;
  setView: (view: View) => void;
  selectMerchant: (merchantId: number) => void;
  beginBatch: (pending: PendingBatch) => void;
  setActiveJob: (job: ActiveJobSnapshot | null) => void;
  setProgress: (progress: JobProgress | null) => void;
  resumeJob: (job: ActiveJobSnapshot) => void;
  applyProgress: (progress: JobProgress) => void;
  finishJob: (payload: JobFinishedPayload) => void;
  failJob: () => void;
  resetWorkspace: () => void;
}

export const useAppStore = create<AppStore>((set, get) => ({
  snapshot: null,
  selectedMerchantId: null,
  view: "overview",
  activeJob: null,
  progress: null,
  lastFinished: null,
  pendingBatch: null,

  setSnapshot: (snapshot) =>
    set((state) => {
      const selectedExists = snapshot.merchants.some(
        (merchant) => merchant.id === state.selectedMerchantId,
      );
      return {
        snapshot,
        selectedMerchantId: selectedExists
          ? state.selectedMerchantId
          : (snapshot.merchants[0]?.id ?? null),
      };
    }),

  refresh: async () => {
    const snapshot = await getRootSnapshot();
    get().setSnapshot(snapshot);
    return snapshot;
  },

  setView: (view) => set({ view }),

  selectMerchant: (merchantId) =>
    set({ selectedMerchantId: merchantId, view: "overview" }),

  beginBatch: (pendingBatch) =>
    set({ pendingBatch, view: "batch", lastFinished: null }),

  setActiveJob: (activeJob) => set({ activeJob }),

  setProgress: (progress) => set({ progress }),

  resumeJob: (job) =>
    set({
      activeJob: job,
      progress: job.progress,
      view: "batch",
      selectedMerchantId: job.merchantId,
    }),

  applyProgress: (progress) =>
    set((state) => ({
      progress,
      activeJob:
        state.activeJob && state.activeJob.jobId === progress.jobId
          ? { ...state.activeJob, progress }
          : {
              jobId: progress.jobId,
              batchId: progress.batchId,
              merchantId: progress.merchantId,
              startedAt: "",
              progress,
            },
    })),

  finishJob: (lastFinished) =>
    set({ activeJob: null, progress: null, lastFinished }),

  failJob: () => set({ activeJob: null, progress: null }),

  resetWorkspace: () =>
    set({
      snapshot: null,
      selectedMerchantId: null,
      view: "overview",
      activeJob: null,
      progress: null,
      lastFinished: null,
      pendingBatch: null,
    }),
}));
