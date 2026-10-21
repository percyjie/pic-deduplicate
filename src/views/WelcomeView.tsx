import { useState } from "react";
import { App as AntdApp, Button, Tag } from "antd";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowRightOutlined,
  CheckCircleFilled,
  DatabaseOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  LockOutlined,
  SafetyCertificateOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";

import { initializeRoot, inspectRoot, openRoot } from "../api/commands";
import { useAppStore } from "../store/appStore";
import { errorMessage } from "../utils/format";

const LAST_ROOT_KEY = "picdeduplicate.lastRoot";

function WelcomeView() {
  const { message, modal } = AntdApp.useApp();
  const setSnapshot = useAppStore((state) => state.setSnapshot);
  const [busy, setBusy] = useState(false);
  const [lastRoot] = useState(() => localStorage.getItem(LAST_ROOT_KEY));

  async function activateRoot(path: string, initialized: boolean) {
    setBusy(true);
    try {
      const snapshot = initialized
        ? await openRoot(path)
        : await initializeRoot(path);
      setSnapshot(snapshot);
      localStorage.setItem(LAST_ROOT_KEY, snapshot.rootPath);
      message.success(
        initialized ? "主文件夹已打开。" : "主文件夹初始化完成。",
      );
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function inspectAndContinue(path: string) {
    setBusy(true);
    try {
      const info = await inspectRoot(path);
      if (info.looksLikeMerchant) {
        modal.warning({
          title: "请选择上级主文件夹",
          content:
            "这个目录本身包含“图片主库”或“去重结果”，看起来是商户目录。请选择包含所有商户文件夹的上级主文件夹。",
        });
        return;
      }
      if (info.initialized) {
        await activateRoot(path, true);
        return;
      }
      modal.confirm({
        title: "初始化这个主文件夹？",
        width: 520,
        content: (
          <div className="welcome-confirm">
            <p>
              程序将在该目录创建隐藏的 <code>.picdedupe</code>{" "}
              索引库，并自动把一级子文件夹识别为商户。
            </p>
            <p>现有图片不会被移动、改名或删除。主库与结果目录均为额外副本。</p>
            <p className="welcome-confirm-path">{info.path}</p>
          </div>
        ),
        okText: "初始化并打开",
        cancelText: "取消",
        onOk: () => activateRoot(path, false),
      });
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  async function chooseFolder() {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择主文件夹",
        buttonLabel: "选择此文件夹",
      });
      if (typeof picked === "string") {
        await inspectAndContinue(picked);
      }
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  return (
    <div className="welcome">
      <div className="welcome-glow welcome-glow-one" />
      <div className="welcome-glow welcome-glow-two" />

      <div className="welcome-card">
        <section className="welcome-main">
          <div className="welcome-brand">
            <div className="brand-mark">
              <DatabaseOutlined />
            </div>
            <span>PicDeduplicate</span>
            <Tag color="blue">Windows 本地版</Tag>
          </div>

          <h1>
            本地图片
            <br />
            <span>智能去重</span>
          </h1>
          <p className="welcome-lead">
            按商户维护唯一图片主库。每次导入与主库逐一比对，重复与不重复图片自动按批次归档。
          </p>

          <div className="welcome-actions">
            <Button
              type="primary"
              size="large"
              icon={<FolderOpenOutlined />}
              loading={busy}
              onClick={() => void chooseFolder()}
            >
              选择主文件夹
            </Button>
            {lastRoot && (
              <Button
                size="large"
                icon={<HistoryOutlined />}
                disabled={busy}
                onClick={() => void inspectAndContinue(lastRoot)}
              >
                打开上次文件夹
              </Button>
            )}
          </div>

          {lastRoot && (
            <div className="welcome-last-path">
              上次使用：<span>{lastRoot}</span>
            </div>
          )}

          <div className="welcome-features">
            <div>
              <SafetyCertificateOutlined />
              <span>
                <strong>精确 + 视觉比对</strong>
                SHA-256 判重，可选识别压缩、缩放与 90° 旋转
              </span>
            </div>
            <div>
              <LockOutlined />
              <span>
                <strong>纯本地、无网络</strong>
                不上传任何图片或数据
              </span>
            </div>
            <div>
              <ThunderboltOutlined />
              <span>
                <strong>商户独立主库</strong>
                索引加速，批次可追溯
              </span>
            </div>
          </div>
        </section>

        <aside className="welcome-aside">
          <div className="welcome-aside-title">工作方式</div>
          <ol className="welcome-steps">
            <li>
              <span className="step-index">01</span>
              <div>
                <strong>选择主文件夹</strong>
                <p>每个一级子文件夹代表一个商户。</p>
              </div>
            </li>
            <li>
              <span className="step-index">02</span>
              <div>
                <strong>选择商户与来源</strong>
                <p>首次导入历史图片，之后按天导入新图片。</p>
              </div>
            </li>
            <li>
              <span className="step-index">03</span>
              <div>
                <strong>获得批次结果</strong>
                <p>每次生成独立的重复 / 不重复目录与 CSV 报告。</p>
              </div>
            </li>
          </ol>

          <div className="folder-preview">
            <div className="folder-preview-title">
              <FolderOpenOutlined /> 主文件夹结构
            </div>
            <pre>{`主文件夹/
├─ 商户A/
│  ├─ 图片主库/       唯一图片
│  ├─ 去重结果/       按批次归档
│  └─ 其他目录/       源文件只读
└─ 商户B/`}</pre>
          </div>

          <div className="welcome-note">
            <CheckCircleFilled />
            源文件始终不会被修改、移动或删除。
            <ArrowRightOutlined />
          </div>
        </aside>
      </div>
    </div>
  );
}

export default WelcomeView;
