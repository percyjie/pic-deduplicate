import { useState } from "react";
import { App as AntdApp, Alert, Button, Card, Input, Modal, Tag, Tooltip } from "antd";
import {
  ClockCircleOutlined,
  DatabaseOutlined,
  ExclamationCircleOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  ImportOutlined,
  PlusOutlined,
  ShopOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";

import { createMerchant, openLibrary } from "../api/commands";
import { useAppStore } from "../store/appStore";
import {
  errorMessage,
  formatBytes,
  formatDateTime,
  formatNumber,
} from "../utils/format";

function MerchantOverview() {
  const { message } = AntdApp.useApp();
  const snapshot = useAppStore((state) => state.snapshot);
  const selectedMerchantId = useAppStore((state) => state.selectedMerchantId);
  const activeJob = useAppStore((state) => state.activeJob);
  const beginBatch = useAppStore((state) => state.beginBatch);
  const selectMerchant = useAppStore((state) => state.selectMerchant);
  const setView = useAppStore((state) => state.setView);
  const refresh = useAppStore((state) => state.refresh);
  const [newMerchantOpen, setNewMerchantOpen] = useState(false);
  const [newMerchantName, setNewMerchantName] = useState("");
  const [creating, setCreating] = useState(false);

  if (!snapshot) return null;

  const selected =
    snapshot.merchants.find((merchant) => merchant.id === selectedMerchantId) ??
    snapshot.merchants[0] ??
    null;
  const jobRunning = Boolean(activeJob);

  async function handleOpenLibrary(merchantId: number) {
    try {
      await openLibrary(merchantId);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function handleCreateMerchant() {
    const name = newMerchantName.trim();
    if (!name) {
      message.warning("请输入商户名称。");
      return;
    }
    setCreating(true);
    try {
      const merchant = await createMerchant(name);
      const nextSnapshot = await refresh();
      const store = useAppStore.getState();
      store.setSnapshot(nextSnapshot);
      store.selectMerchant(merchant.id);
      setNewMerchantOpen(false);
      setNewMerchantName("");
      message.success(`商户“${merchant.folderName}”已创建。`);
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="page-stack">
      <div className="stat-row">
        <div className="surface stat-card">
          <div className="stat-icon">
            <ShopOutlined />
          </div>
          <div>
            <div className="stat-label">商户数量</div>
            <div className="stat-value">
              {formatNumber(snapshot.merchants.length)}
            </div>
          </div>
        </div>
        <div className="surface stat-card">
          <div className="stat-icon blue">
            <DatabaseOutlined />
          </div>
          <div>
            <div className="stat-label">主库唯一图片</div>
            <div className="stat-value">
              {formatNumber(snapshot.totalImages)}
            </div>
          </div>
        </div>
        <div className="surface stat-card">
          <div className="stat-icon green">
            <ClockCircleOutlined />
          </div>
          <div>
            <div className="stat-label">主库占用空间</div>
            <div className="stat-value">{formatBytes(snapshot.totalSize)}</div>
          </div>
        </div>
      </div>

      {jobRunning && (
        <Alert
          type="warning"
          showIcon
          icon={<ExclamationCircleOutlined />}
          message="已有批次任务正在执行"
          description="同一时间只允许一个任务。完成或取消前不能开始新的导入，也不能切换主文件夹。"
        />
      )}

      {!selected && (
        <Card className="surface" bordered={false}>
          <div className="empty-block">
            <h3>还没有商户</h3>
            <p>
              在选定的主文件夹中新建一级子文件夹，或点击下方按钮创建第一个商户。
            </p>
            <Button
              type="primary"
              icon={<PlusOutlined />}
              onClick={() => setNewMerchantOpen(true)}
            >
              新建商户
            </Button>
          </div>
        </Card>
      )}

      {selected && (
        <Card className="surface hero-card" bordered={false}>
          <div className="hero-main">
            <div className="hero-avatar">
              {selected.displayName.slice(0, 1).toUpperCase()}
            </div>
            <div className="hero-info">
              <div className="hero-name">
                {selected.displayName}
                {!selected.exists && <Tag color="red">文件夹缺失</Tag>}
                {selected.imageCount === 0 && <Tag>主库为空</Tag>}
              </div>
              <Tooltip title={selected.path}>
                <div className="hero-path">{selected.path}</div>
              </Tooltip>
              <div className="hero-meta">
                <span>
                  主库 <strong>{formatNumber(selected.imageCount)}</strong> 张
                </span>
                <span className="hero-meta-divider" />
                <span>{formatBytes(selected.totalSize)}</span>
                <span className="hero-meta-divider" />
                <span>
                  最近批次 {formatDateTime(selected.lastBatchAt)}
                </span>
              </div>
            </div>
            <div className="hero-actions">
              <Button
                type="primary"
                size="large"
                icon={<ThunderboltOutlined />}
                disabled={!selected.exists || jobRunning}
                onClick={() =>
                  beginBatch({
                    merchantId: selected.id,
                    kind: "daily",
                  })
                }
              >
                日常批量去重
              </Button>
              <Button
                size="large"
                icon={<ImportOutlined />}
                disabled={!selected.exists || jobRunning}
                onClick={() =>
                  beginBatch({
                    merchantId: selected.id,
                    kind: "initial_import",
                  })
                }
              >
                首次导入历史图片
              </Button>
            </div>
          </div>

          <div className="hero-secondary">
            <Button
              icon={<FolderOpenOutlined />}
              disabled={!selected.exists}
              onClick={() => void handleOpenLibrary(selected.id)}
            >
              打开主库
            </Button>
            <Button icon={<HistoryOutlined />} onClick={() => setView("history")}>
              查看历史记录
            </Button>
            <span className="hero-tip">
              首次导入会对历史图片内部去重；日常批次会与整个主库比对。
            </span>
          </div>

          {selected.imageCount === 0 && selected.exists && (
            <Alert
              className="hero-alert"
              type="info"
              showIcon
              message="该商户主库还没有图片"
              description="如果这是第一次使用，请选择“首次导入历史图片”，系统会把历史图片内部去重后写入主库。"
            />
          )}
          {!selected.exists && (
            <Alert
              className="hero-alert"
              type="error"
              showIcon
              message="商户文件夹已不存在"
              description="请把文件夹恢复到原位置后点击左侧“刷新商户”，或关闭主文件夹后重新打开。"
            />
          )}
        </Card>
      )}

      <div className="section-head">
        <div>
          <h3>全部商户</h3>
          <p>主文件夹的一级子文件夹会自动识别为商户。</p>
        </div>
        <Button
          icon={<PlusOutlined />}
          onClick={() => setNewMerchantOpen(true)}
          disabled={jobRunning}
        >
          新建商户
        </Button>
      </div>

      <div className="merchant-grid">
        {snapshot.merchants.map((merchant) => (
          <button
            type="button"
            key={merchant.id}
            className={`merchant-card ${
              merchant.id === selected?.id ? "active" : ""
            }`}
            onClick={() => selectMerchant(merchant.id)}
          >
            <div className="merchant-card-head">
              <span className="merchant-avatar">
                {merchant.displayName.slice(0, 1).toUpperCase()}
              </span>
              <span className="merchant-card-name">{merchant.displayName}</span>
              {!merchant.exists && <Tag color="red">缺失</Tag>}
            </div>
            <div className="merchant-card-stats">
              <span>
                <strong>{formatNumber(merchant.imageCount)}</strong> 张唯一图
              </span>
              <span>{formatBytes(merchant.totalSize)}</span>
            </div>
            <div className="merchant-card-foot">
              最近批次 {formatDateTime(merchant.lastBatchAt)}
            </div>
          </button>
        ))}

        <button
          type="button"
          className="merchant-card add"
          onClick={() => setNewMerchantOpen(true)}
        >
          <PlusOutlined />
          <span>新建商户</span>
          <small>在主文件夹中创建新的商户目录</small>
        </button>
      </div>

      <div className="note-strip">
        <DatabaseOutlined />
        <span>
          “图片主库”和“去重结果”由程序管理，请勿手工修改其中的文件或目录名。
        </span>
      </div>

      <Modal
        title="新建商户"
        open={newMerchantOpen}
        onCancel={() => setNewMerchantOpen(false)}
        onOk={() => void handleCreateMerchant()}
        okText="创建"
        cancelText="取消"
        confirmLoading={creating}
        destroyOnHidden
      >
        <p className="modal-hint">
          将在主文件夹下创建同名文件夹，并加入商户列表。
        </p>
        <Input
          autoFocus
          maxLength={80}
          placeholder="例如：张三水果店"
          value={newMerchantName}
          onChange={(event) => setNewMerchantName(event.target.value)}
          onPressEnter={() => void handleCreateMerchant()}
        />
      </Modal>
    </div>
  );
}

export default MerchantOverview;
