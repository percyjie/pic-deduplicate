import { useEffect } from "react";
import { App as AntdApp } from "antd";

import {
  getActiveJob,
  onJobFailed,
  onJobFinished,
  onJobProgress,
} from "./api/commands";
import Workspace from "./components/Workspace";
import { useAppStore } from "./store/appStore";
import WelcomeView from "./views/WelcomeView";
import "./App.css";

function App() {
  const { message, notification } = AntdApp.useApp();
  const snapshot = useAppStore((state) => state.snapshot);
  const rootPath = snapshot?.rootPath ?? null;

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    async function register() {
      const listeners = await Promise.all([
        onJobProgress((payload) => {
          useAppStore.getState().applyProgress(payload);
        }),
        onJobFinished((payload) => {
          const store = useAppStore.getState();
          store.finishJob(payload);
          void store.refresh().catch(() => undefined);
          if (payload.error) {
            notification.error({
              message: "批次处理失败",
              description: payload.error,
            });
          } else if (payload.batch.status === "cancelled") {
            message.info("任务已取消，已处理的文件已保留。");
          } else if (payload.batch.status === "completed_with_errors") {
            notification.warning({
              message: "批次完成，但有文件处理异常",
              description: `成功入库 ${payload.batch.importedCount} 张，异常 ${payload.batch.errorCount} 个，警告 ${payload.batch.warningCount} 个。可在结果报告中查看详情。`,
            });
          } else {
            message.success(
              `批次处理完成：入库 ${payload.batch.importedCount} 张图片。`,
            );
          }
        }),
        onJobFailed((payload) => {
          useAppStore.getState().failJob();
          notification.error({
            message: "任务中断",
            description: payload.error.message,
          });
        }),
      ]);
      if (disposed) {
        listeners.forEach((unlisten) => unlisten());
      } else {
        unlisteners.push(...listeners);
      }
    }

    void register();
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [message, notification]);

  useEffect(() => {
    if (!rootPath) return;
    let cancelled = false;
    void getActiveJob()
      .then((job) => {
        if (cancelled || !job) return;
        useAppStore.getState().resumeJob(job);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [rootPath]);

  return snapshot ? <Workspace /> : <WelcomeView />;
}

export default App;
