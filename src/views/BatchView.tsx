import { useEffect, useState } from "react";
import {
  Alert,
  App as AntdApp,
  Button,
  Card,
  Progress,
  Segmented,
  Steps,
  Tag,
  Tooltip,
} from "antd";
import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircleOutlined,
  CloseCircleOutlined,
  ExclamationCircleOutlined,
  FileTextOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  ImportOutlined,
  InboxOutlined,
  LoadingOutlined,
  ReloadOutlined,
  StopOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";

import {
  cancelJob,
  openReport,
  openResultDir,
  startBatch,
} from "../api/commands";
import type { BatchKind } from "../api/types";
import { useAppStore } from "../store/appStore";
import {
  batchKindLabel,
  batchStatusMeta,
  errorMessage,
  formatDateTime,
  formatNumber,
  phaseLabel,
  phaseOrder,
} from "../utils/format";

function BatchView() {
  const { message, modal } = AntdApp.useApp();
  const snapshot = useAppStore((state) => state.snapshot);
  const pendingBatch = useAppStore((state) => state.pendingBatch);
  const activeJob = useAppStore((state) => state.activeJob);
  const progress = useAppStore((state) => state.progress);
  const lastFinished = useAppStore((state) => state.lastFinished);
  const beginBatch = useAppStore((state) => state.beginBatch);
  const setView = useAppStore((state) => state.setView);
  const [sourceDir, setSourceDir] = useState(pendingBatch?.sourceDir ?? "");
  const [kind, setKind] = useState<BatchKind>(pendingBatch?.kind ?? "daily");
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  useEffect(() => {
    setSourceDir(pendingBatch?.sourceDir ?? "");
    setKind(pendingBatch?.kind ?? "daily");
  }, [pendingBatch?.merchantId, pendingBatch?.kind, pendingBatch?.sourceDir]);

  if (!snapshot) return null;

  const jobMerchant = activeJob
    ? (snapshot.merchants.find(
        (merchant) => merchant.id === activeJob.merchantId,
      ) ?? null)
    : null;
  const setupMerchant = pendingBatch
    ? (snapshot.merchants.find(
        (merchant) => merchant.id === pendingBatch.merchantId,
      ) ?? null)
    : null;

  async function chooseSource() {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择待处理的图片来源文件夹",
        buttonLabel: "选择此文件夹",
      });
      if (typeof picked === "string") {
        setSourceDir(picked);
      }
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  function confirmStart() {
    if (!pendingBatch || !setupMerchant) return;
    if (!sourceDir.trim()) {
      message.warning("请先选择来源文件夹。");
      return;
    }
    modal.confirm({
      title: "开始批量处理？",
      width: 540,
      content: (
        <div className="start-confirm">
          <p>
            商户：<strong>{setupMerchant.displayName}</strong>
          </p>
          <p>
            模式：<strong>{batchKindLabel(kind)}</strong>
          </p>
          <p className="start-confirm-path">{sourceDir}</p>
          <p>
            源文件不会被修改。唯一图片会复制到主库，同时本次的重复和不重复图片会复制到新的批次结果目录。
          </p>
          {kind === "initial_import" && setupMerchant.imageCount > 0 && (
            <Alert
              type="warning"
              showIcon
              message={`该商户主库已有 ${formatNumber(
                setupMerchant.imageCount,
              )} 张图片`}
              description="本次依然只新增唯一图片；与主库重复的文件会进入“重复”目录。"
            />
          )}
        </div>
      ),
      okText: "开始处理",
      cancelText: "取消",
      onOk: () => handleStart(pendingBatch.merchantId),
    });
  }

  async function handleStart(merchantId: number) {
    setStarting(true);
    try {
      const result = await startBatch({
        merchantId,
        sourceDir,
        kind,
      });
      const store = useAppStore.getState();
      if (!store.activeJob || store.activeJob.jobId !== result.jobId) {
        store.setActiveJob({
          jobId: result.jobId,
          batchId: result.batchId,
          merchantId,
          startedAt: new Date().toISOString(),
          progress: null,
        });
        store.setProgress(null);
      }
      message.success("批次任务已开始。");
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setStarting(false);
    }
  }

  function confirmCancel() {
    if (!activeJob) return;
    modal.confirm({
      title: "取消当前任务？",
      content:
        "已成功写入主库的图片会保留，当前批次会标记为“已取消”，并生成部分报告。之后可以用同一来源重新运行，已入库图片会被判定为主库重复。",
      okText: "取消任务",
      cancelText: "继续处理",
      okButtonProps: { danger: true },
      onOk: async () => {
        setCancelling(true);
        try {
          await cancelJob(activeJob.jobId);
          message.info("正在取消任务，请稍候…");
        } catch (error) {
          message.error(errorMessage(error));
        } finally {
          setCancelling(false);
        }
      },
    });
  }

  async function openBatchPath(kindOfPath: "result" | "report", batchId: number) {
    try {
      if (kindOfPath === "result") {
        await openResultDir(batchId);
      } else {
        await openReport(batchId);
      }
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  const phaseIndex = progress ? phaseOrder.indexOf(progress.phase) : 0;
  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.processed / progress.total) * 100))
      : 0;

  return (
    <div className="page-stack">
      {activeJob && (
        <Card className="surface job-card" bordered={false}>
          <div className="job-head">
            <div>
              <div className="job-eyebrow">
                <LoadingOutlined spin /> 批次执行中
              </div>
              <h2>
                {jobMerchant?.displayName ?? `商户 #${activeJob.merchantId}`}
              </h2>
              <div className="job-sub">
                批次 #{activeJob.batchId}
                {progress?.message ? ` · ${progress.message}` : ""}
              </div>
            </div>
            <Button
              danger
              icon={<StopOutlined />}
              loading={cancelling}
              onClick={confirmCancel}
            >
              取消任务
            </Button>
          </div>

          <Steps
            className="job-steps"
            size="small"
            current={phaseIndex}
            items={phaseOrder.map((phase) => ({ title: phaseLabel(phase) }))}
          />

          <div className="job-progress">
            <Progress
              percent={percent}
              status="active"
              strokeColor={{ from: "#2563eb", to: "#38bdf8" }}
            />
            <div className="job-file">
              {progress?.currentFile ? (
                <>
                  <span>当前文件</span>
                  <code>{progress.currentFile}</code>
                </>
              ) : (
                <span>{progress?.message ?? "正在准备…"}</span>
              )}
            </div>
          </div>

          <div className="job-stats">
            <div className="job-stat">
              <span>不重复（判定）</span>
              <strong className="green">
                {formatNumber(progress?.uniqueCount ?? 0)}
              </strong>
            </div>
            <div className="job-stat">
              <span>主库已有重复</span>
              <strong>{formatNumber(progress?.duplicateExistingCount ?? 0)}</strong>
            </div>
            <div className="job-stat">
              <span>同批次重复</span>
              <strong>{formatNumber(progress?.duplicateInBatchCount ?? 0)}</strong>
            </div>
            <div className="job-stat">
              <span>跳过</span>
              <strong>{formatNumber(progress?.skippedCount ?? 0)}</strong>
            </div>
            <div className="job-stat">
              <span>异常</span>
              <strong className={progress?.errorCount ? "red" : ""}>
                {formatNumber(progress?.errorCount ?? 0)}
              </strong>
            </div>
            <div className="job-stat">
              <span>警告</span>
              <strong className={progress?.warningCount ? "orange" : ""}>
                {formatNumber(progress?.warningCount ?? 0)}
              </strong>
            </div>
          </div>

          <Alert
            className="job-note"
            type="info"
            showIcon
            message="源文件只读"
            description="任务执行期间请勿关闭程序或拔出磁盘。取消后可安全地使用同一来源重新运行。"
          />
        </Card>
      )}

      {!activeJob && lastFinished && (
        <Card className="surface result-card" bordered={false}>
          <div className="result-head">
            <div className={`result-icon ${lastFinished.batch.status}`}>
              {lastFinished.batch.status === "completed" ? (
                <CheckCircleOutlined />
              ) : lastFinished.batch.status === "cancelled" ? (
                <StopOutlined />
              ) : (
                <ExclamationCircleOutlined />
              )}
            </div>
            <div className="result-head-text">
              <h2>
                批次
                {batchStatusMeta(lastFinished.batch.status).label}
                <Tag
                  color={batchStatusMeta(lastFinished.batch.status).color}
                >
                  {batchKindLabel(lastFinished.batch.batchType)}
                </Tag>
              </h2>
              <div className="job-sub">
                {lastFinished.batch.merchantName} · 批次 #
                {lastFinished.batch.id} ·{" "}
                {formatDateTime(lastFinished.batch.finishedAt)}
              </div>
            </div>
            <div className="result-actions">
              <Button
                icon={<FolderOpenOutlined />}
                onClick={() =>
                  void openBatchPath("result", lastFinished.batch.id)
                }
              >
                打开结果目录
              </Button>
              <Button
                icon={<FileTextOutlined />}
                disabled={!lastFinished.batch.reportPath}
                onClick={() =>
                  void openBatchPath("report", lastFinished.batch.id)
                }
              >
                打开报告
              </Button>
            </div>
          </div>

          {lastFinished.error && (
            <Alert
              type="error"
              showIcon
              className="result-alert"
              message="批次异常中止"
              description={lastFinished.error}
            />
          )}
          {lastFinished.batch.note && !lastFinished.error && (
            <Alert
              type="warning"
              showIcon
              className="result-alert"
              message={lastFinished.batch.note}
            />
          )}

          <div className="result-highlight">
            <div>
              <span>成功入库</span>
              <strong>{formatNumber(lastFinished.batch.importedCount)}</strong>
              <small>张唯一图片已复制到主库</small>
            </div>
            <div>
              <span>判定不重复</span>
              <strong className="green">
                {formatNumber(lastFinished.batch.uniqueCount)}
              </strong>
              <small>复制失败时可能大于实际入库数</small>
            </div>
          </div>

          <div className="result-grid">
            <div>
              <span>总文件</span>
              <strong>{formatNumber(lastFinished.batch.totalFiles)}</strong>
            </div>
            <div>
              <span>主库已有重复</span>
              <strong>
                {formatNumber(lastFinished.batch.duplicateExistingCount)}
              </strong>
            </div>
            <div>
              <span>同批次重复</span>
              <strong>
                {formatNumber(lastFinished.batch.duplicateInBatchCount)}
              </strong>
            </div>
            <div>
              <span>跳过</span>
              <strong>{formatNumber(lastFinished.batch.skippedCount)}</strong>
            </div>
            <div>
              <span>异常</span>
              <strong className={lastFinished.batch.errorCount ? "red" : ""}>
                {formatNumber(lastFinished.batch.errorCount)}
              </strong>
            </div>
            <div>
              <span>警告</span>
              <strong
                className={lastFinished.batch.warningCount ? "orange" : ""}
              >
                {formatNumber(lastFinished.batch.warningCount)}
              </strong>
            </div>
          </div>

          <div className="result-paths">
            <div>
              <span>来源</span>
              <Tooltip title={lastFinished.batch.sourceAbsPath}>
                <code>{lastFinished.batch.sourceAbsPath}</code>
              </Tooltip>
            </div>
            <div>
              <span>结果</span>
              <Tooltip title={lastFinished.batch.resultAbsPath}>
                <code>{lastFinished.batch.resultAbsPath}</code>
              </Tooltip>
            </div>
          </div>

          <div className="result-footer">
            <Button
              icon={<HistoryOutlined />}
              onClick={() => setView("history")}
            >
              查看历史记录
            </Button>
            <Button
              type="primary"
              onClick={() => setView("overview")}
            >
              返回商户概览
            </Button>
          </div>
        </Card>
      )}

      {!activeJob && !lastFinished && pendingBatch && setupMerchant && (
        <div className="batch-layout">
          <Card className="surface setup-card" bordered={false}>
            <div className="setup-eyebrow">本次任务</div>
            <div className="setup-merchant">
              <span className="merchant-avatar large">
                {setupMerchant.displayName.slice(0, 1).toUpperCase()}
              </span>
              <div>
                <div className="setup-merchant-name">
                  {setupMerchant.displayName}
                </div>
                <div className="setup-merchant-meta">
                  主库 {formatNumber(setupMerchant.imageCount)} 张 ·{" "}
                  {formatDateTime(setupMerchant.lastBatchAt)}
                </div>
              </div>
            </div>

            <Segmented
              block
              className="mode-segment"
              value={kind}
              onChange={(value) => setKind(value as BatchKind)}
              options={[
                {
                  value: "daily",
                  label: (
                    <span className="segment-label">
                      <ThunderboltOutlined /> 日常批量去重
                    </span>
                  ),
                },
                {
                  value: "initial_import",
                  label: (
                    <span className="segment-label">
                      <ImportOutlined /> 首次导入历史
                    </span>
                  ),
                },
              ]}
            />

            <p className="mode-description">
              {kind === "daily"
                ? "逐张与主库所有图片比对。新的唯一图片会被加入主库，重复图片只进入本次结果目录。"
                : "用于第一次导入历史图片。除了与主库比对，还会对本次历史图片内部去重，只把唯一图片写入主库。"}
            </p>

            {kind === "initial_import" && setupMerchant.imageCount > 0 && (
              <Alert
                type="warning"
                showIcon
                message="主库已有图片"
                description="本次仍会先与主库比对，只新增唯一图片。"
              />
            )}

            <div className="source-block">
              <div className="source-label">图片来源文件夹</div>
              <div className="source-input">
                <Tooltip title={sourceDir || undefined}>
                  <div className={sourceDir ? "source-value" : "source-value empty"}>
                    {sourceDir || "尚未选择来源文件夹"}
                  </div>
                </Tooltip>
                <Button
                  icon={<FolderOpenOutlined />}
                  onClick={() => void chooseSource()}
                >
                  浏览…
                </Button>
              </div>
              <div className="source-hint">
                支持 jpg / png / webp / heic 等常见图片格式，会递归扫描子文件夹。
              </div>
            </div>

            <Button
              type="primary"
              size="large"
              block
              icon={<ThunderboltOutlined />}
              loading={starting}
              onClick={confirmStart}
            >
              开始处理
            </Button>

            <div className="setup-foot">
              <span>
                <InboxOutlined /> 源文件只读
              </span>
              <span>
                <ReloadOutlined /> 可随时取消
              </span>
            </div>
          </Card>

          <div className="batch-side">
            <Card className="surface" bordered={false}>
              <h3 className="side-title">处理流程</h3>
              <Steps
                direction="vertical"
                size="small"
                current={0}
                items={[
                  {
                    title: "扫描来源",
                    description: "递归收集图片，跳过程序生成目录与隐藏目录。",
                  },
                  {
                    title: "计算 SHA-256",
                    description: "流式读取文件，完成后校验文件未被改动。",
                  },
                  {
                    title: "与主库比对",
                    description: "识别主库已有重复、同批次重复与唯一图片。",
                  },
                  {
                    title: "安全复制",
                    description:
                      "唯一图写入主库，全部文件按结果复制到本批次目录。",
                  },
                  {
                    title: "生成报告",
                    description:
                      "输出 CSV 与批次摘要，并记录数据库批次历史。",
                  },
                ]}
              />
            </Card>

            <Alert
              type="info"
              showIcon
              icon={<ExclamationCircleOutlined />}
              message="关于磁盘空间"
              description="唯一图片会在主库和“不重复”目录各保留一份，重复图片也会复制到“重复”目录。开始前请预留足够空间。"
            />

            <Alert
              type="success"
              showIcon
              icon={<CheckCircleOutlined />}
              message="精确去重，不做视觉相似"
              description="只有文件内容完全一致才会判定为重复。重新压缩或缩放后的图片不会被误判。"
            />
          </div>
        </div>
      )}

      {!activeJob && !lastFinished && !pendingBatch && (
        <Card className="surface" bordered={false}>
          <div className="empty-block">
            <CloseCircleOutlined className="empty-icon" />
            <h3>尚未选择任务</h3>
            <p>请先在商户概览中选择“日常批量去重”或“首次导入历史图片”。</p>
            <Button type="primary" onClick={() => setView("overview")}>
              返回商户概览
            </Button>
          </div>
        </Card>
      )}

      {!activeJob && lastFinished && (
        <div className="batch-again">
          <Button
            icon={<ThunderboltOutlined />}
            onClick={() =>
              beginBatch({
                merchantId: lastFinished.batch.merchantId,
                kind: lastFinished.batch.batchType,
                sourceDir: lastFinished.batch.sourceAbsPath,
              })
            }
          >
            用同一来源再运行一次
          </Button>
        </div>
      )}
    </div>
  );
}

export default BatchView;
