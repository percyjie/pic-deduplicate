# 图片去重（PicDeduplicate）

Windows 本地桌面图片去重工具。选择一个主文件夹，其一级子目录视为商户；把历史图片或每日新图片批量导入后，与当前商户主库比对，并按批次输出“重复 / 不重复”目录与 CSV 报告。源文件始终只读，不上传网络。

v0.2 在 SHA-256 精确比对之外增加可选的**视觉相似比对**：重新压缩、缩放、旋转 90° 的图片可以被判定为重复。

## 技术栈

- 前端：React 19 + TypeScript + Vite + Ant Design + Zustand
- 桌面壳：Tauri 2
- 后端：Rust、SQLite（rusqlite bundled）、SHA-256 流式哈希、WIC 解码、64 位 dHash、Rayon

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

## 比对方式

每个批次都可以单独选择比对方式：

| 方式 | 判定依据 | 适用场景 |
| --- | --- | --- |
| 精确哈希 | 文件 SHA-256 完全一致 | 默认；需要零误判 |
| 视觉相似 | 精确哈希 + 视觉指纹 | 需要识别压缩、缩放、90° 旋转后的同一张图 |

视觉模式的处理规则：

- 解码图片并生成 64 位 dHash，同时计算 0° / 90° / 180° / 270° 四个方向的指纹，因此 90° 旋转不影响判定。
- 先用宽高比做预筛，再计算汉明距离；距离越小越相似。
- 阈值预设：严格 4、标准 8、宽松 12（可接受范围 0–24）。推荐先用“标准”并抽查结果。
- 视觉判定是概率性的，可能把构图相近的不同图片判为重复；报告中会单独标记“视觉相似”并记录视觉差异值。
- 低细节图片（例如纯色图）可能无法生成有效指纹，会记录警告并只参与精确比对。
- 精确匹配始终优先于视觉匹配，因此视觉模式不会把完全相同文件误标为“视觉相似”。
- 视觉模式不处理裁剪、镜像、加水印或任意角度旋转等大幅改动。

每个批次的报告新增 `匹配方式` 和 `视觉差异` 两列；批次摘要 JSON 包含 `matchMode` 与 `visualThreshold`。历史界面可以按批次查看比对方式、阈值和视觉相似重复数。

## 数据与迁移

- 数据库结构版本为 2；打开 v1 数据库时会自动、幂等迁移，原批次记录保留为“精确哈希 / 阈值 8”。
- 视觉指纹保存在 `image_visuals` 表中。主库已有图片会在首次运行视觉模式批次时按需补算，不需要重新导入。
- 删除 `.picdedupe` 会丢失快速索引，程序不会自动从主库全量重建。

## 开发

```bash
npm install
npm run tauri dev
```

## 测试

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

测试覆盖精确去重、视觉匹配（压缩 / 缩放 / 旋转 / 不同图片）、批内视觉去重、低细节图片警告、报告字段以及 v1 → v2 数据库迁移。

## 按 tag 打包

### 1. 给当前版本打 tag

```bash
git tag -a v0.2.0 -m "v0.2.0: 视觉相似去重"
```

tag 只是本地引用；需要共享时再执行 `git push origin v0.2.0`。

### 2. 用脚本从任意 tag 构建便携版

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 v0.2.0
```

脚本会：

1. 在 `.build/worktrees/<tag>` 创建一个指向该 tag 的临时 `git worktree`；
2. 在其中执行 `npm ci` 与 `npm run tauri build -- --no-bundle`；
3. 把 exe 打包为 `release\PicDeduplicate-v0.2.0-x64-portable.zip`；
4. 删除临时 worktree。

当前工作区、分支和未提交修改不会被改动。不同 tag 共用 `.build/cargo-target` 缓存，重复构建会更快；首次构建仍需要联网安装前端依赖并下载 Rust 依赖。

也可以构建历史版本，例如：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 v0.1.0
```

### 3. 可选：同时构建 NSIS 安装包

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 v0.2.0 -WithInstaller
```

安装包输出为 `release\PicDeduplicate-v0.2.0-x64-setup.exe`。NSIS 工具链和 WebView2 离线安装器会在首次构建时下载并缓存到 `%LOCALAPPDATA%\tauri`；网络受限时可能失败，此时使用便携版 ZIP 即可。

### 4. 便携版运行要求

便携版 exe 需要系统已安装 WebView2 Runtime。Windows 11 及大多数更新过的 Windows 10 已自带；未安装的机器需要先安装微软的 WebView2 Runtime。

## 说明

- 源文件只读：程序不会移动、删除、重命名或修改来源图片。
- 唯一图片会同时写入主库与本批次“不重复”目录，需预留双份磁盘空间。
- 同一时间只执行一个批次任务；取消或异常中断后可以安全重跑。
- 所有处理均在本机完成，不上传任何图片或数据。
