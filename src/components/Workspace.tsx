import { useState } from "react";
import { App as AntdApp, Button, Layout, Tag, Tooltip } from "antd";
import {
  AppstoreOutlined,
  DatabaseOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  LoadingOutlined,
  ReloadOutlined,
  SwapOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";

import { closeRoot } from "../api/commands";
import { useAppStore } from "../store/appStore";
import { errorMessage, formatNumber } from "../utils/format";
import BatchView from "../views/BatchView";
import HistoryView from "../views/HistoryView";
import MerchantOverview from "../views/MerchantOverview";

const { Header, Sider, Content } = Layout;

function Workspace() {
  const { message, modal } = AntdApp.useApp();
  const snapshot = useAppStore((state) => state.snapshot);
  const selectedMerchantId = useAppStore((state) => state.selectedMerchantId);
  const view = useAppStore((state) => state.view);
  const activeJob = useAppStore((state) => state.activeJob);
  const setView = useAppStore((state) => state.setView);
  const selectMerchant = useAppStore((state) => state.selectMerchant);
  const resetWorkspace = useAppStore((state) => state.resetWorkspace);
  const refresh = useAppStore((state) => state.refresh);
  const [refreshing, setRefreshing] = useState(false);
  const [closing, setClosing] = useState(false);

  if (!snapshot) return null;

  const selectedMerchant =
    snapshot.merchants.find((merchant) => merchant.id === selectedMerchantId) ??
    snapshot.merchants[0] ??
    null;

  const pageMeta = {
    overview: {
      title: selectedMerchant?.displayName ?? "商户概览",
      subtitle: "选择商户查看主库，或开始一次新的图片导入。",
    },
    batch: {
      title: "批量去重",
      subtitle: activeJob
        ? "任务正在本机后台处理，请保持程序运行。"
        : "选择来源目录后，与当前商户的主库图片逐一比对。",
    },
    history: {
      title: "历史记录",
      subtitle: "追溯每一次批次的来源、计数、异常与报告。",
    },
  }[view];

  async function handleRefresh() {
    setRefreshing(true);
    try {
      await refresh();
      message.success("商户列表已刷新。");
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setRefreshing(false);
    }
  }

  function handleSwitchRoot() {
    modal.confirm({
      title: "切换主文件夹",
      content:
        "切换后当前会话会关闭，索引与批次记录仍保留在主文件夹内。任务执行中无法切换。",
      okText: "关闭并切换",
      cancelText: "取消",
      onOk: async () => {
        setClosing(true);
        try {
          await closeRoot();
          resetWorkspace();
        } catch (error) {
          message.error(errorMessage(error));
        } finally {
          setClosing(false);
        }
      },
    });
  }

  return (
    <Layout className="workspace">
      <Sider width={278} className="app-sider" theme="dark">
        <div className="brand">
          <div className="brand-mark">
            <DatabaseOutlined />
          </div>
          <div>
            <div className="brand-title">图片去重</div>
            <div className="brand-subtitle">PicDeduplicate · 本地版</div>
          </div>
        </div>

        <Tooltip title={snapshot.rootPath} placement="right">
          <div className="root-chip">
            <FolderOpenOutlined />
            <span className="root-chip-text">{snapshot.rootPath}</span>
          </div>
        </Tooltip>

        <div className="sider-nav">
          <button
            type="button"
            className={`nav-item ${view !== "history" ? "active" : ""}`}
            onClick={() => setView("overview")}
          >
            <AppstoreOutlined />
            <span>商户概览</span>
          </button>
          <button
            type="button"
            className={`nav-item ${view === "history" ? "active" : ""}`}
            onClick={() => setView("history")}
          >
            <HistoryOutlined />
            <span>历史记录</span>
          </button>
        </div>

        <div className="sider-section">
          <div className="sider-section-title">
            商户
            <span>{formatNumber(snapshot.merchants.length)}</span>
          </div>
          <div className="merchant-list">
            {snapshot.merchants.map((merchant) => (
              <button
                key={merchant.id}
                type="button"
                className={`merchant-item ${
                  merchant.id === selectedMerchant?.id ? "active" : ""
                }`}
                onClick={() => selectMerchant(merchant.id)}
              >
                <span className="merchant-avatar">
                  {merchant.displayName.slice(0, 1).toUpperCase()}
                </span>
                <span className="merchant-meta">
                  <span className="merchant-name">{merchant.displayName}</span>
                  <span className="merchant-count">
                    {merchant.exists
                      ? `${formatNumber(merchant.imageCount)} 张`
                      : "文件夹缺失"}
                  </span>
                </span>
                {!merchant.exists && <span className="merchant-warning">!</span>}
              </button>
            ))}
            {snapshot.merchants.length === 0 && (
              <div className="merchant-empty">
                暂无商户。一级子文件夹会自动识别为商户。
              </div>
            )}
          </div>
        </div>

        <div className="sider-footer">
          {activeJob && (
            <div className="job-indicator">
              <LoadingOutlined spin />
              <span>批次任务执行中</span>
            </div>
          )}
          <div className="sider-footer-actions">
            <Button
              type="text"
              ghost
              icon={<ReloadOutlined />}
              loading={refreshing}
              onClick={() => void handleRefresh()}
            >
              刷新商户
            </Button>
            <Button
              type="text"
              ghost
              icon={<SwapOutlined />}
              loading={closing}
              disabled={Boolean(activeJob)}
              onClick={handleSwitchRoot}
            >
              切换文件夹
            </Button>
          </div>
        </div>
      </Sider>

      <Layout>
        <Header className="app-header">
          <div>
            <div className="page-title">{pageMeta.title}</div>
            <div className="page-subtitle">{pageMeta.subtitle}</div>
          </div>
          <div className="header-tags">
            {activeJob && (
              <Tag color="processing" icon={<ThunderboltOutlined />}>
                任务进行中
              </Tag>
            )}
            <Tag color="geekblue">纯本地处理</Tag>
            <Tag>源文件只读</Tag>
          </div>
        </Header>
        <Content className="app-content">
          <div className="page-container">
            {view === "overview" && <MerchantOverview />}
            {view === "batch" && <BatchView />}
            {view === "history" && <HistoryView />}
          </div>
        </Content>
      </Layout>
    </Layout>
  );
}

export default Workspace;
