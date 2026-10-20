import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  App as AntdApp,
  Button,
  Card,
  Descriptions,
  Drawer,
  Dropdown,
  Select,
  Space,
  Spin,
  Table,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import type { MenuProps, TableColumnsType } from "antd";
import {
  FileTextOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  MoreOutlined,
  ReloadOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";

import {
  getBatchDetail,
  listBatches,
  openLibrary,
  openReport,
  openResultDir,
  regenerateReport,
} from "../api/commands";
import type { BatchDetail, BatchPage, BatchSummary } from "../api/types";
import { useAppStore } from "../store/appStore";
import {
  batchKindLabel,
  batchStatusMeta,
  errorMessage,
  formatDateTime,
  formatNumber,
  outcomeLabel,
} from "../utils/format";

const { Text } = Typography;

function HistoryView() {
  const { message } = AntdApp.useApp();
  const snapshot = useAppStore((state) => state.snapshot);
  const selectedMerchantId = useAppStore((state) => state.selectedMerchantId);
  const activeJob = useAppStore((state) => state.activeJob);
  const setView = useAppStore((state) => state.setView);
  const beginBatch = useAppStore((state) => state.beginBatch);
  const [filter, setFilter] = useState<number | "all">(
    selectedMerchantId ?? "all",
  );
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [data, setData] = useState<BatchPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [detail, setDetail] = useState<BatchDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const result = await listBatches({
        merchantId: filter === "all" ? null : filter,
        page,
        pageSize,
      });
      setData(result);
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [filter, message, page, pageSize]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    setFilter(selectedMerchantId ?? "all");
    setPage(1);
  }, [selectedMerchantId]);

  function openDetail(batchId: number) {
    setDrawerOpen(true);
    setDetail(null);
    setDetailLoading(true);
    void getBatchDetail(batchId)
      .then((result) => setDetail(result))
      .catch((error) => {
        message.error(errorMessage(error));
        setDrawerOpen(false);
      })
      .finally(() => setDetailLoading(false));
  }

  async function handleOpenResult(batchId: number) {
    try {
      await openResultDir(batchId);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function handleOpenReport(batchId: number) {
    try {
      await openReport(batchId);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function handleRegenerate(batchId: number) {
    if (activeJob) {
      message.warning("任务执行中，暂不能重新生成报告。");
      return;
    }
    const key = `regen-${batchId}`;
    message.open({
      key,
      type: "loading",
      content: "正在重新生成报告…",
      duration: 0,
    });
    try {
      await regenerateReport(batchId);
      message.open({
        key,
        type: "success",
        content: "报告已重新生成。",
        duration: 2,
      });
      await load();
      if (detail?.batch.id === batchId) {
        openDetail(batchId);
      }
    } catch (error) {
      message.open({
        key,
        type: "error",
        content: errorMessage(error),
        duration: 4,
      });
    }
  }

  function handleRerun(batch: BatchSummary) {
    if (activeJob) {
      message.warning("已有任务执行中，请等待完成或先取消。");
      return;
    }
    beginBatch({
      merchantId: batch.merchantId,
      kind: batch.batchType,
      sourceDir: batch.sourceAbsPath,
    });
  }

  async function handleOpenLibrary(merchantId: number) {
    try {
      await openLibrary(merchantId);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  function handleAction(key: string, batch: BatchSummary) {
    switch (key) {
      case "detail":
        openDetail(batch.id);
        break;
      case "result":
        void handleOpenResult(batch.id);
        break;
      case "report":
        void handleOpenReport(batch.id);
        break;
      case "regenerate":
        void handleRegenerate(batch.id);
        break;
      case "rerun":
        handleRerun(batch);
        break;
      case "library":
        void handleOpenLibrary(batch.merchantId);
        break;
      default:
        break;
    }
  }

  const moreItems: MenuProps["items"] = [
    { key: "regenerate", label: "重新生成报告" },
    { key: "rerun", label: "重新运行此批次", disabled: Boolean(activeJob) },
    { type: "divider" },
    { key: "library", label: "打开商户主库" },
  ];

  const columns: TableColumnsType<BatchSummary> = [
    {
      title: "开始时间",
      dataIndex: "startedAt",
      width: 168,
      render: (value: string) => (
        <span className="nowrap">{formatDateTime(value)}</span>
      ),
    },
    {
      title: "商户",
      dataIndex: "merchantName",
      width: 120,
      render: (value: string) => <Text strong>{value}</Text>,
    },
    {
      title: "类型",
      dataIndex: "batchType",
      width: 118,
      render: (value: BatchSummary["batchType"]) => (
        <Tag color={value === "initial_import" ? "purple" : "blue"}>
          {batchKindLabel(value)}
        </Tag>
      ),
    },
    {
      title: "状态",
      dataIndex: "status",
      width: 128,
      render: (value: BatchSummary["status"]) => {
        const meta = batchStatusMeta(value);
        return <Tag color={meta.color}>{meta.label}</Tag>;
      },
    },
    {
      title: "来源目录",
      dataIndex: "sourceAbsPath",
      ellipsis: true,
      render: (value: string) => (
        <Tooltip title={value}>
          <span className="cell-ellipsis">{value}</span>
        </Tooltip>
      ),
    },
    {
      title: "文件数",
      dataIndex: "totalFiles",
      width: 84,
      align: "right",
      render: (value: number) => formatNumber(value),
    },
    {
      title: "不重复 / 入库",
      key: "unique",
      width: 130,
      align: "right",
      render: (_, record) => (
        <Tooltip title="不重复判定数 / 实际成功写入主库数">
          <span>
            <strong>{formatNumber(record.uniqueCount)}</strong>
            <span className="muted"> / {formatNumber(record.importedCount)}</span>
          </span>
        </Tooltip>
      ),
    },
    {
      title: "重复",
      key: "duplicates",
      width: 100,
      align: "right",
      render: (_, record) =>
        formatNumber(
          record.duplicateExistingCount + record.duplicateInBatchCount,
        ),
    },
    {
      title: "异常 / 警告",
      key: "issues",
      width: 108,
      align: "right",
      render: (_, record) => (
        <span>
          <span className={record.errorCount ? "red" : "muted"}>
            {formatNumber(record.errorCount)}
          </span>
          <span className="muted"> / </span>
          <span className={record.warningCount ? "orange" : "muted"}>
            {formatNumber(record.warningCount)}
          </span>
        </span>
      ),
    },
    {
      title: "操作",
      key: "actions",
      width: 250,
      fixed: "right",
      render: (_, record) => (
        <Space size={4}>
          <Button type="link" size="small" onClick={() => openDetail(record.id)}>
            详情
          </Button>
          <Button
            type="link"
            size="small"
            icon={<FolderOpenOutlined />}
            onClick={() => void handleOpenResult(record.id)}
          >
            结果
          </Button>
          <Button
            type="link"
            size="small"
            icon={<FileTextOutlined />}
            disabled={!record.reportPath}
            onClick={() => void handleOpenReport(record.id)}
          >
            报告
          </Button>
          <Dropdown
            menu={{
              items: moreItems,
              onClick: ({ key }) => handleAction(key, record),
            }}
            trigger={["click"]}
          >
            <Button type="text" size="small" icon={<MoreOutlined />} />
          </Dropdown>
        </Space>
      ),
    },
  ];

  if (!snapshot) return null;

  const merchantOptions = [
    { value: "all" as const, label: "全部商户" },
    ...snapshot.merchants.map((merchant) => ({
      value: merchant.id,
      label: merchant.displayName,
    })),
  ];

  return (
    <div className="page-stack">
      {activeJob && (
        <Alert
          type="info"
          showIcon
          message="已有批次任务正在执行"
          description="历史记录可以正常查看；任务完成前不能重新运行批次或重新生成报告。"
        />
      )}

      <Card className="surface table-card" bordered={false}>
        <div className="table-toolbar">
          <div>
            <h3>批次历史</h3>
            <p>每次处理都会留下独立的目录、计数和报告。</p>
          </div>
          <Space>
            <Select
              value={filter}
              style={{ width: 180 }}
              options={merchantOptions}
              onChange={(value) => {
                setFilter(value);
                setPage(1);
              }}
            />
            <Button
              icon={<ReloadOutlined />}
              loading={loading}
              onClick={() => void load()}
            >
              刷新
            </Button>
            <Button
              icon={<HistoryOutlined />}
              onClick={() => setView("overview")}
            >
              返回概览
            </Button>
          </Space>
        </div>

        <Table<BatchSummary>
          rowKey="id"
          size="middle"
          loading={loading}
          columns={columns}
          dataSource={data?.items ?? []}
          scroll={{ x: 1320 }}
          locale={{ emptyText: "还没有批次记录。选择商户并开始一次导入吧。" }}
          pagination={{
            current: page,
            pageSize,
            total: data?.total ?? 0,
            showSizeChanger: true,
            pageSizeOptions: [10, 20, 50, 100],
            showTotal: (total) => `共 ${formatNumber(total)} 条批次`,
            onChange: (nextPage, nextPageSize) => {
              setPage(nextPage);
              setPageSize(nextPageSize);
            },
          }}
        />
      </Card>

      <div className="note-strip">
        <ThunderboltOutlined />
        <span>
          同一来源重复运行时，主库不会新增；已入库的图片会全部判定为“主库已有重复”。
        </span>
      </div>

      <Drawer
        width={760}
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        title={detail ? `批次 #${detail.batch.id}` : "批次详情"}
        destroyOnHidden
      >
        <Spin spinning={detailLoading}>
          {detail && (
            <div className="detail-stack">
              <div className="detail-head">
                <Tag color={batchStatusMeta(detail.batch.status).color}>
                  {batchStatusMeta(detail.batch.status).label}
                </Tag>
                <Tag
                  color={
                    detail.batch.batchType === "initial_import" ? "purple" : "blue"
                  }
                >
                  {batchKindLabel(detail.batch.batchType)}
                </Tag>
                <span className="muted">
                  {detail.batch.merchantName} ·{" "}
                  {formatDateTime(detail.batch.finishedAt)}
                </span>
              </div>

              {detail.batch.note && (
                <Alert
                  type={
                    detail.batch.status === "failed" ? "error" : "warning"
                  }
                  showIcon
                  message={detail.batch.note}
                />
              )}

              <Descriptions
                size="small"
                column={1}
                bordered
                items={[
                  {
                    key: "source",
                    label: "来源目录",
                    children: <code className="wrap-code">{detail.batch.sourceAbsPath}</code>,
                  },
                  {
                    key: "result",
                    label: "结果目录",
                    children: (
                      <code className="wrap-code">{detail.batch.resultAbsPath}</code>
                    ),
                  },
                  {
                    key: "started",
                    label: "开始时间",
                    children: formatDateTime(detail.batch.startedAt),
                  },
                  {
                    key: "finished",
                    label: "结束时间",
                    children: formatDateTime(detail.batch.finishedAt),
                  },
                ]}
              />

              <div className="result-grid detail-counts">
                <div>
                  <span>总文件</span>
                  <strong>{formatNumber(detail.batch.totalFiles)}</strong>
                </div>
                <div>
                  <span>不重复判定</span>
                  <strong className="green">
                    {formatNumber(detail.batch.uniqueCount)}
                  </strong>
                </div>
                <div>
                  <span>成功入库</span>
                  <strong>{formatNumber(detail.batch.importedCount)}</strong>
                </div>
                <div>
                  <span>主库重复</span>
                  <strong>
                    {formatNumber(detail.batch.duplicateExistingCount)}
                  </strong>
                </div>
                <div>
                  <span>批内重复</span>
                  <strong>
                    {formatNumber(detail.batch.duplicateInBatchCount)}
                  </strong>
                </div>
                <div>
                  <span>跳过</span>
                  <strong>{formatNumber(detail.batch.skippedCount)}</strong>
                </div>
                <div>
                  <span>异常</span>
                  <strong className={detail.batch.errorCount ? "red" : ""}>
                    {formatNumber(detail.batch.errorCount)}
                  </strong>
                </div>
                <div>
                  <span>警告</span>
                  <strong className={detail.batch.warningCount ? "orange" : ""}>
                    {formatNumber(detail.batch.warningCount)}
                  </strong>
                </div>
              </div>

              <div className="detail-actions">
                <Button
                  icon={<FolderOpenOutlined />}
                  onClick={() => void handleOpenResult(detail.batch.id)}
                >
                  打开结果目录
                </Button>
                <Button
                  icon={<FileTextOutlined />}
                  disabled={!detail.batch.reportPath}
                  onClick={() => void handleOpenReport(detail.batch.id)}
                >
                  打开报告
                </Button>
                <Button
                  icon={<ReloadOutlined />}
                  onClick={() => void handleRegenerate(detail.batch.id)}
                >
                  重新生成报告
                </Button>
                <Button
                  type="primary"
                  icon={<ThunderboltOutlined />}
                  disabled={Boolean(activeJob)}
                  onClick={() => handleRerun(detail.batch)}
                >
                  重新运行
                </Button>
              </div>

              <div>
                <h4 className="detail-section-title">
                  异常与警告（最多显示 300 条）
                </h4>
                {detail.issues.length === 0 ? (
                  <Alert
                    type="success"
                    showIcon
                    message="没有异常或警告记录。"
                  />
                ) : (
                  <Table
                    rowKey="seq"
                    size="small"
                    pagination={false}
                    scroll={{ y: 280 }}
                    dataSource={detail.issues}
                    columns={[
                      {
                        title: "序号",
                        dataIndex: "seq",
                        width: 64,
                      },
                      {
                        title: "文件",
                        dataIndex: "fileName",
                        ellipsis: true,
                        render: (value: string, record) => (
                          <Tooltip title={record.sourceAbsPath}>
                            <span>{value}</span>
                          </Tooltip>
                        ),
                      },
                      {
                        title: "结果",
                        dataIndex: "outcome",
                        width: 130,
                        render: (value: string) => (
                          <Tag
                            color={
                              value === "error"
                                ? "red"
                                : value === "skipped"
                                  ? "default"
                                  : "orange"
                            }
                          >
                            {outcomeLabel(value)}
                          </Tag>
                        ),
                      },
                      {
                        title: "说明",
                        key: "message",
                        render: (_, record) => (
                          <span className="issue-message">
                            {record.errorMessage ?? record.warningMessage ?? "—"}
                          </span>
                        ),
                      },
                    ]}
                  />
                )}
              </div>
            </div>
          )}
        </Spin>
      </Drawer>
    </div>
  );
}

export default HistoryView;
