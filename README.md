# 图片去重（PicDeduplicate）

Windows 本地桌面图片去重工具。选择一个主文件夹，其一级子目录视为商户；把历史图片或每日新图片批量导入后，与当前商户主库做 SHA-256 精确比对，并按批次输出“重复 / 不重复”目录与 CSV 报告。源文件始终只读，不上传网络。

## 技术栈

- 前端：React 19 + TypeScript + Vite + Ant Design + Zustand
- 桌面壳：Tauri 2
- 后端：Rust、SQLite（rusqlite bundled）、SHA-256 流式哈希、Rayon

## 目录约定

```text
<主文件夹>/
├─ .picdedupe/                 # 索引数据库、锁文件、备份
├─ 商户A/
│  ├─ 图片主库/YYYY-MM/        # 唯一图片
│  └─ 去重结果/时间戳批次/
│     ├─ 重复/
│     ├─ 不重复/
│     ├─ 批次报告.csv
│     └─ 批次摘要.json
└─ 商户B/
```

`图片主库`、`去重结果` 由程序托管，内部有 `.picdedupe_generated` 标记，扫描来源时会整体跳过。

## 开发

```bash
npm install
npm run tauri dev
```

## 测试

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

## 构建安装包

```bash
npm run tauri build -- --bundles nsis
```

产物位于 `src-tauri/target/release/bundle/nsis/`，使用离线 WebView2 安装包，按当前用户安装。

首次执行安装包构建时需要联网下载 NSIS 3.13 工具链和 WebView2 离线安装器（缓存在 `%LOCALAPPDATA%\tauri`）。网络受限时可先手动准备这些文件，或直接分发 `src-tauri/target/release/pic-deduplicate.exe` 便携版。

## 说明

- 重复判定仅使用完整文件 SHA-256，不做视觉相似或感知哈希。
- 唯一图片会同时写入主库与本批次“不重复”目录，需预留双份磁盘空间。
- 同一时间只执行一个批次任务；取消或异常中断后可以安全重跑。
- 删除 `.picdedupe` 会丢失快速索引，V1 不会自动从主库全量重建。
