# 更新日志

本文件记录「框选 AI（Snip Ask）」的版本变更，格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 计划中

- 发布工程：代码签名（EV/OV 证书）、默认安装目录改为 `%LOCALAPPDATA%\Programs\SnipAsk`（避免与数据目录同名）、
  卸载时可选删除本地数据。**自动更新不再列入计划**（v1.0.0 已按用户决策彻底移除，见 [ADR-0008](docs/adr/0008-remove-auto-update.md)）。
- 遗留验证：**AMD / Intel 实机 GPU**（Vulkan 路径已论证厂商无关、本机未实测）、8GB 内存机降档提示（S-13）、
  DashScope Qwen-MT 真实翻译（本机无 Key）、受保护内容（DRM）下的覆盖翻译、真实流式 TTFT 补测。
- 跨环境验收：Win10 干净机、国产杀软（火绒/360 等）、SmartScreen 交互 UI、S-19 真人可用性走查。
- UI 打磨：结果窗 Markdown 渲染、多语言（`en-US` 预留）、无障碍键盘路径、主题自定义导入导出。

## [1.0.2] - 2026-11-06

> 修复版：仅处理 3 个用户反馈问题（源码已冻结）；**无契约变更**（配置 Schema 6 / 43 命令 / 15 事件 /
> `IPC_CONTRACT_VERSION = 1` 均不变），不改数据收集与存储方式。

### 修复
- **结果窗透明外框/固定空白**：结果窗改为不透明面板并铺满窗口，高度随回答内容自适应
  （内容少时窗口收紧、内容多时到屏幕 85% 后内部滚动）；去掉窗口外圈透明边、系统阴影与固定 420px 高度造成的空白。
- **引擎已就绪仍提示「本地模型缺失」**：修复启动时未下载模型留下的旧失败提示在模型安装、引擎就绪后仍显示的问题；
  引擎就绪时自动清空旧提示，仅「未启动」时才展示可读原因（引擎自身原因优先）。
- **自绘标题栏按钮无响应**：补齐窗口权限（最小化/最大化/还原/起始缩放/`set-size`），
  修复主窗口最小化、最大化按钮点击无效（结果窗自动高度也依赖 `set-size` 权限）。

### 双安装包（重建，最终归档）

- 由 T5 基于 v1.0.2 源码重建并归档到 `release/`：
  - **在线安装版** `SnipAsk_1.0.2_x64-setup.exe` **7,778,687 B（7.42 MiB）**，
    SHA256 `d0e2df6b19fc964f243f9f80ca7a330f47cd94e87fa6b9d4d1f23dd9ec3ec5b2`；
  - **内置 OCR 模型版** `SnipAsk_1.0.2_x64-full-setup.exe` **22,564,280 B（21.52 MiB）**，
    SHA256 `67af3437f682cc13e51491d3ac62e39714efb4ebf7d71d548cbed475fa99c901`；
  - 体积门禁 PASS（≤25 MiB / ≤60 MiB）。
- 安装冒烟 PASS：两版静默安装 `/S` 退出码 0（3.3s / 4.2s），安装目录分别 9 文件/27,380,321 B（无模型）与
  13 文件/43,595,613 B（+det/rec/cls/dict）；启动 6s 主窗口「框选 AI」可见、响应正常；
  **关闭主窗口 = 隐藏到托盘（`closeBehavior=hidden-to-tray`，进程存活）**；静默卸载 `/S` 退出码 0（1.3s / 1.7s），
  用户数据恢复。两版均 `NotSigned`。证据 `.verify/v1/installer/`、`release/SHA256SUMS.txt`。

### 验证
- **最终门禁（全绿）**：`cargo fmt --check` PASS（0）；`cargo clippy --all-targets -- -D warnings` PASS（0 warning）；
  `cargo test --all-targets` **484 passed / 0 failed / 20 ignored**（lib 345 + 新增 4 个引擎提示测试）；
  `pnpm typecheck` 0、`pnpm test` **48/48**、`pnpm build:web` 0。
- 真机 E2E（cloud）：点击最小化/最大化真实生效；结果窗卡片与窗口逐像素一致（**460×312**，不再有透明边/固定空白）。
- 引擎提示合并逻辑新增 **4 个单元测试**（`ready_clears_stale_missing_model_note` /
  `stopped_attaches_note_when_engine_has_none` / `stopped_keeps_engine_own_reason` /
  `starting_does_not_attach_old_note`）。
- 最终门禁与双安装包体积/SHA256 以 `.verify/verify-v1.0.2/SUMMARY.md` 与 `release/SHA256SUMS.txt` 为准。

## [1.0.1] - 2026-11-06（用户反馈修复）

> 本版本针对 v1.0.0 的用户反馈做集中修复，并明确产品隐私口径：**不收集任何用户信息**
> （无遥测、无崩溃上报、无使用统计、无更新检查）。功能集合不变，配置自动从 v5 迁移到 **v6**。

### 契约 v6（配置 Schema / 下载状态）

- **配置 Schema v5 → v6**：**删除 `advanced.telemetry`**（v5 配置升级时主动移除旧键）；
  `advanced.skeletonMode` 保留为**开发者配置**（默认 `false`，不再出现在用户界面）；迁移链 **1→2→3→4→5→6**。
- **`DownloadState` 新增 `"idle"`**：未下载默认态，替代「默认 `queued`」；模型/组件列表在未开始下载时显示未下载，
  只有明确开始下载后才进入 `queued → downloading → verifying`。
- 命令仍 **43**、事件仍 **15**，`IPC_CONTRACT_VERSION` 保持 **1**（纯增量 + 配置迁移）。

### 窗口与托盘

- 主窗口去掉 Windows 系统标题栏，改为**自绘标题栏**：拖拽移动、最小化、最大化/还原、关闭，以及八向缩放手柄
  （最大化时隐藏）；浏览器预览下禁用窗口 API。
- **关闭按钮 / `Alt+F4` = 隐藏到托盘（不退出）**；真正退出走托盘「退出」。
- **托盘左键单击直接打开设置**（右键显示菜单）；「重新运行引导」保留；引导完成/跳过后左侧不再显示引导入口。

### 设置与运行信息

- 新增独立 **搜索** 分组：AI 搜索 / 浏览器搜索、搜索引擎与自定义 URL 模板（从「动作与热键」移出）。
- 新增独立 **隐私** 分组：允许上传识别文字、允许上传截图、仅本机处理（写 `privacy.*`），并说明数据只在本机。
- 「动作与热键」补齐四个动作默认模式（搜索 / 图片搜索 / 翻译 / 解答，写 `modes.*`）。
- 高级页：显示**真实内存占用**（进程 WorkingSetSize）、本地引擎状态与「启动/停止引擎」按钮；
  删除遥测开关与调试/骨架模式面板；「安装包类型」改为中性表述（内置 OCR 模型 / 在线下载）。
- **模型下载**：未下载默认 `idle`（不显示进度/取消）；下载中显示进度与取消、取消保留断点可恢复；
  修复「所有模型默认排队中」的观感。

### 隐私与文案

- **不收集任何用户信息**：无遥测、无崩溃上报、无使用统计、无更新检查；删除配置中的遥测字段与界面开关。
- 面向用户的 UI 文案重写：移除「完整版 / 真实能力 / 骨架模式 / 契约 §x / 降级链」等工程词，
  改为自然、简短、可操作的中文说明（设置、引导、覆盖翻译工具条、结果窗、历史、OCR 卡片、错误提示）。
- 安装包描述统一为「内置 OCR 模型版 / 在线安装版」「内置模型 / 在线下载模型」。

### 验收与门禁（v1.0.1，2026-11-06）

- **最终门禁（全绿）**：`cargo fmt --check` PASS（0）；`cargo clippy --all-targets -- -D warnings` PASS（0 warning）；
  `cargo test --all-targets` **480 passed / 0 failed / 15 ignored**（20 个测试目标；lib 341；真实 OCR 集成 24s；
  local engine；cloud mock；model_download 12；history；contract 17 等）；`pnpm typecheck` 0；`pnpm test` **48/48**；
  `pnpm build:web` 0。
- **真机修复验证（PASS，`.verify/v1/fix/form-check.json`）**：界面文案无「完整版/真实能力/骨架模式/契约 §」等词
  （`banned=[]`）；引导完成后左侧导航仅「设置 / 历史」；自绘标题栏存在、8 向缩放手柄=8；`WM_CLOSE` 后主窗口
  `visible=false` 且进程存活、`window_show` 后恢复可见；`app_stats.version=1.0.1`、内存占用 **46MB**（真实值）；
  无模型时启动本地引擎返回可读错误「本地模型缺失」；`models_list` 默认全部 `idle`，`ocr-dict` 真实下载
  `installed`、`qwen3-4b-q4` 取消后 `cancelled`；搜索方式 browser→ai、隐私 `allowImageUpload` true→false、
  默认模式 translate quick→deep 均保存生效。
- v1.0.1 真机 E2E（cloud/local，`ok=true`）：cloud 翻译 0.75s / AI 搜索 3.58s / 图片搜索 10.02s /
  `translate_cover` 5/5（4.09s）/ 历史 3 轮；local Vulkan offload 29 层、本地翻译 1.46s（1.5B，含提示词残留，
  S-14 已知问题）。双安装包最终体积/SHA256 以 `.verify/verify-v1.0.1/SUMMARY.md` 与 `release/SHA256SUMS.txt` 为准。

### 双安装包（重建，最终归档）

- 由 T5 基于 v1.0.1 源码重建并归档到 `release/`：
  - **在线安装版** `SnipAsk_1.0.1_x64-setup.exe` **7,778,389 B（7.42 MiB）**，
    SHA256 `95b241c2d13ad31e38a7dbab4f9c70d874171e292ced5d080a6018c4436a4986`；
  - **内置 OCR 模型版** `SnipAsk_1.0.1_x64-full-setup.exe` **22,561,123 B（21.52 MiB）**，
    SHA256 `fb1728839c481b649c1bd763284d5c07ddabd8469447c29b10e1cc848a23940b`；
  - 体积门禁 PASS（≤25 MiB / ≤60 MiB）；两版 `snip-ask.exe` 不同（edition 编译期注入生效）。
- 安装冒烟 PASS：两版静默安装 `/S` 退出码 0（3.4s / 4.4s），安装目录分别 9 文件/27,375,802 B（无模型）与
  13 文件/43,591,094 B（+det/rec/cls/dict，SHA256 与源一致）；启动 6s 主窗口「框选 AI」可见、响应正常；
  **关闭窗口隐藏到托盘（`closeBehavior=hidden-to-tray`，进程存活）**；静默卸载 `/S` 退出码 0（1.4s），
  用户数据恢复。两版均 `NotSigned`。证据 `.verify/v1/installer/`、`release/SHA256SUMS.txt`。

### 已知限制（沿用，如实标注）

- AMD/Intel 真机 GPU 未测（Vulkan 厂商无关论证）；安装包未签名（SmartScreen 提示属预期）；
  本机无 DashScope Key（Qwen-MT 真实翻译未验收）；本地小模型翻译格式保持不完美；
  8GB 机型降档提示缺失；Win10/国产杀软/真人可用性（S-19）未覆盖。

## [1.0.0] - 2026-11-06（真实可用版本）

> v1.0.0 把 v0.1.0 的「骨架模式 MVP」升级为完整可用的真实产品：真实捕获、真实本地 OCR、云端与本地推理、
> 凭据、历史持久化、双安装包，并**彻底移除自动更新**、新增《自定义指南》。核心决策见
> [ADR-0007 双安装包](docs/adr/0007-dual-installer-editions.md)、[ADR-0008 移除自动更新](docs/adr/0008-remove-auto-update.md)、
> [ADR-0009 真实捕获与 OCR](docs/adr/0009-real-capture-ocr.md)。

### 契约 v5（配置 / 命令 / 事件 / 类型）

- **配置 Schema v4 → v5**：删除 `UpdateChannel` 与 `advanced.updateChannel`（迁移主动移除旧键）；
  `advanced.skeletonMode` 迁移为 **`false`**（默认走真实能力，仍可手动改回调试）；新增
  `providers[].kind`（`"openAiCompatible"` / `"qwenMt"`）；默认 providers 改为三条
  （`deepseek` 启用 / `dashscope`（qwenMt）禁用 / `generic-openai` 禁用）；迁移链 **1→2→3→4→5**。
- **新增 5 命令（39–43）**：`open_path`（白名单目录）、`ocr_status`（OCR 就绪状态）、
  `history_list` / `history_thread` / `history_clear`（历史持久化）；命令总数 **43**。
- **删除事件 `update:status`**，事件总数 **15**；`Stats` 追加 `edition`（`full`/`slim`/`dev`）与 `ocrReady`。
- 新增结构 `OcrStatus` / `OcrAssetStatus` / `HistorySession` / `HistoryTurn` / `HistoryThread`；
  `models.manifest.json` 增加 4 条 OCR 资产（`kind:"onnx"`、`role:"ocr"`、真实 sha256/size/镜像）。
- `IPC_CONTRACT_VERSION` 保持 **1**（纯增量 + 纯删除 + 配置迁移）。

### 真实捕获（Win32 BitBlt）

- `platform/capture.rs` 真实捕获虚拟桌面物理像素外接矩形（`GetDC(NULL)` + `CreateDIBSection` +
  `BitBlt(SRCCOPY|CAPTUREBLT)`，支持负原点与混合 DPI），产出原始 BGRA（供 OCR）与 PNG data URL（供 overlay）。
- 黑屏/空帧判定按**目标区域**（选区切片/显示器切片）计算，DRM / 受保护内容返回 `SA-1001`
  （retryable，fallback `["wgc","retry"]`）并给出可操作提示，不崩溃；WGC 不作主路径
  （S-09 已证 BitBlt 与 WGC 像素级一致、且 WGC 不能绕过 DRM）。

### 真实本地 OCR（RapidOCR PP-OCRv4 / ONNX Runtime）

- 新增真实 ONNX 引擎（`ort` `load-dynamic`）：det/rec/cls 三模型 + `ppocr_keys_v1.txt` 字典；
  启动时把解析到的 `onnxruntime.dll` 写入 `ORT_DYLIB_PATH`（必须在创建 session 前）。
- 整屏 det 采用**双约束像素预算**（长边 ≤ `ocr.detMaxEdge`(3200) 且 ≤3.0Mpx，绝不放大）；选区采用
  **整屏 det 缓存 → 过滤 + rec-only**（P95 596ms ≤ 800ms），缓存缺失/无命中回退 det+rec；
  文字存在判定按 `det≥0.6 + 面积≥400px² + 行高≤70px + 行数≥1`（S-11：误判 2.0%/漏判 0%）。
- 资产解析顺序：随包 `<resource_dir>\ocr\` → `%LOCALAPPDATA%\SnipAsk\models\ocr\` → `runtime\` →
  开发钩子 `SNIP_ASK_OCR_DIR`；`ocr_status` 暴露运行时/模型/缺失项，设置页与「自定义与关于」展示。

### 云端推理 / 搜索 / 翻译

- **DeepSeek**：真实 OpenAI 兼容流式 SSE（Chat / 视觉，短任务显式关闭 thinking）；官方
  `POST /v1/tools/search` + `POST /v1/tools/fetch` 真实检索/抓取并映射为 `SourceRef`；图片搜索四阶段
  （视觉识别 → 检索词 → Search → deep 并发 Fetch → 摘要流式输出）。
- **DashScope Qwen-MT**：`kind="qwenMt"` 走原生 `translation_options`（含术语干预 `terms`）；
  本机无 Key，真实翻译未验收（协议级/单测覆盖）。
- **通用 OpenAI 兼容**：自定义 `baseUrl`（https 或 `http://127.0.0.1[:port]`）+ 模型；`provider_test`
  真实连通性与契约 §8.2 错误映射。
- 翻译回退链（云端 provider 之间、云端↔本地）与 `engine` 如实标注；覆盖翻译真实链路统一经
  `AppState::gateway_for`（骨架 → 本地 → 云端）。

### 凭据与历史持久化

- **Windows Credential Manager**（`keyring` crate，`windows-native`，service `SnipAsk`，user = provider id）
  替换内存模拟；`InMemoryCredentialStore` 仅保留给单测/骨架；密钥不落盘、不进日志/事件。
- 新增 `migrations/002_messages.sql`（`messages` 表）与 `services/history_repo.rs` 真实读写：
  成功任务记录 user/assistant 消息；`history_list`（按 `startedAt` 倒序）/`history_thread`/`history_clear`；
  `history.enabled=false` 不写；启动按 `maxEntries`/`retentionDays` 清理；重启后可浏览历史会话与完整线程。

### 双安装包与去自动更新

- 新增 `scripts/fetch-ocr-assets.ps1`（OCR 资产准备：本机缓存 → manifest 镜像，SHA256 + 大小强校验）
  与 `scripts/build-installers.ps1`（slim/full 构建：`SNIP_ASK_EDITION` 编译期注入、体积门禁、真实安装/启动/卸载冒烟、归档与 SHA256）。
- 新增 `src-tauri/tauri.slim.conf.json` / `tauri.full.conf.json`；产物命名
  `SnipAsk_1.0.0_x64-setup.exe`（slim）/`SnipAsk_1.0.0_x64-full-setup.exe`（full）；
  体积门禁 **slim ≤ 25 MiB、full ≤ 60 MiB**；安装包内随附 `docs/`。
  最终归档产物（2026-11-06，`release/`）：
  - slim **7,772,978 B（7.41 MiB）**，SHA256 `180dc72d5821ec2b1618e1322ca0ef6ea23d72d02b3896ede3748fbbda68c7c5`；
  - full **22,564,103 B（21.52 MiB）**，SHA256 `3097ed9d3226587a04a0e0382757dd52a9d8aec5faa4cf62d3534f8827034db6`（含 4 个 OCR 模型/字典）。
  两版安装包 `NotSigned`、版本 `1.0.0`；构建时间 2026-11-06 01:40:28→01:46:39。
  终版安装冒烟 PASS：slim 安装目录 9 文件 / 27,367,523 B（exe+uninstall+`ocr\onnxruntime.dll`+docs×6，无模型），
  启动 6s 存活可见（WS ≈38.0MB、显示「精简版·真实能力 / 缺少 4 个 OCR 模型」），卸载干净；
  full 安装目录 13 文件 / 43,582,815 B（+det/rec/cls/dict，SHA256 与源一致），显示「完整版·真实能力 / OCR 已就绪」，
  卸载干净；用户真实数据冒烟后 `restored=true`。证据见 `.verify/v1/installer/` 与 `SUMMARY.md`。
- **彻底移除自动更新**：不再有 updater 插件/服务、`advanced.updateChannel`、更新通道 UI、`update:status` 事件；
  升级方式改为覆盖安装或源码构建（[ADR-0008](docs/adr/0008-remove-auto-update.md)）。

### 自定义与文档

- 新增 [`docs/自定义指南.md`](docs/自定义指南.md)（核心交付）：不自动更新与升级方式、提示词覆盖（6 个模板 + 变量规则）、
  术语表（`translate.glossary`）、配置 Schema v5 字段速查、本地模型/后端替换与调试钩子、Ollama/OpenAI 兼容接入、
  full/slim 差异与 OCR 资产手动放置、5+ 可照抄示例、从源码构建双安装包、升级/回滚/备份。
- 新增 `docs/README.md`（文档索引，随安装包分发）；设置页新增「自定义与关于」（版本/版本类型/OCR 状态/
  `open_path` 目录入口/无自动更新说明）。
- 新增 ADR-0007/0008/0009 并更新 ADR 索引；契约文档升 v5；实施架构 / 开发指南 / 技术验证清单 / README /
  SECURITY / PRIVACY / NOTICE 同步到 v1.0.0。

### 修复

- **本地回环请求不再被系统代理拦截**（产品修复）：统一 HTTP 客户端构建（`platform/http.rs`）对 `127.0.0.1`
  显式直连（不继承系统代理），本地 llama-server、自定义 `http://127.0.0.1[:port]` 供应商与回环测试不再被代理影响；
  修复了「本机系统代理不绕过 127.0.0.1」导致的下载/云端 mock loopback 测试假失败（含单测覆盖）。
- `model_download_spike` / `cloud_*` 等回环测试随之恢复全绿（`cargo test --all-targets`）。

### 验收与门禁（2026-11-06）

- **最终门禁（Lead 统一执行，全绿）**：`cargo fmt --check` PASS（0）；`cargo clippy --all-targets -- -D warnings`
  PASS（0 warning）；`cargo test --all-targets` **479 passed / 0 failed / 20 ignored**（340 lib 单测；真实 OCR
  推理集成 22.4s；local engine 真机 Vulkan/CPU 2 项；cloud mock/协议；model_download 12 项 Range/断点/杀进程；
  history 重启恢复 4 项；capture 2 项仅 ignored 需真实桌面）；`pnpm typecheck` 0；`pnpm test` **48/48**
  （含 OCR status 8 + history format 8）；`pnpm build:web` 0。
  双安装包最终体积/SHA256 与冒烟结论见 `release/README.md` 与 `.verify/verify-v1.0.0/SUMMARY.md`。
- **真机 E2E**：真实框选 → 真实 OCR → 云端 DeepSeek 回答 / 本地 GGUF 回答 / 覆盖翻译；
  双安装包静默安装/启动/卸载冒烟、体积与 SHA256 归档。
- **app 级 E2E（cloud/local 两阶段，`ok=true`，已验收）**：
  - cloud：真实捕获 → 真实选区 OCR（hasText=true、5 行、5402ms）→ 翻译（deepseek 1470ms）/
    AI 搜索（deepseek 2892ms）/ 图片搜索（deepseek 10036ms）→ `translate_cover`（全部块成功、4326ms）→
    `history_list`（3 会话）/ `history_thread`（3 轮）；`provider_test` 200、`ocr_status` ready、`app_stats` v1.0.0。
  - local：`SNIP_ASK_BACKENDS_DIR` + `SNIP_ASK_MODEL_PATH`（qwen2.5-1.5b-instruct-q4）→ 引擎 ready /
    device gpu / backend **vulkan** / offload 29 层 / RTX 4060 Laptop；本地翻译 engine=local、747ms。
  - **如实标注**：local 译文含 1 行提示词残留（`Term List…`），属 S-14 本地小模型格式保持问题，
    **不作为翻译质量验收通过**；云端翻译无此问题。
  - E2E 为 debug 构建（applab 隔离）；release 安装包由 T5 冒烟覆盖（启动/OCR 就绪/卸载）。
- **真实 OCR 推理证据**（`.verify/v1/ocr/`）：7 个真实用例（5 张真实渲染截图 + 真实桌面面板 + 真实整虚拟桌面），
  2890 字符字符准确率 **98.13%**（≥95% 目标达成；分用例最低代码页 95.75%，真实桌面面板 100%，
  整虚拟桌面已知行召回 7/8）；4 条 OCR 资产双镜像 SHA256 + Range 206 核验；局限（近似 unclip 导致
  较 Python rapidocr 低 ~1.2pp）如实记录。
- **真实捕获证据**（`.verify/v1/capture/`）：真实 BitBlt 冻结帧 3640×1920（混合 DPI 双屏），
  BGRA 27,955,200 B / PNG 1,911,402 B，帧几何与 Win32 虚拟桌面一致（副屏无关内容已脱敏）。
- **如实标注的遗留**：AMD / Intel 真机 GPU 未测（Vulkan 厂商无关论证）；安装包无代码签名（SmartScreen 提示属预期）；
  本机无 DashScope Key（Qwen-MT 真实翻译未验收）；8GB 降档提示缺失（S-13）；S-19 真人走查 BLOCKED；
  覆盖翻译真实 OCR/翻译质量与 S-12/S-14 质量缺口按既有记录保留。

### 行为变更

- 升级用户（schema v4）首次启动后 `advanced.skeletonMode` 自动变为 `false`（走真实能力）；如需联调可手动改回 `true`。
- 应用不再提供任何更新检查行为；升级需手动覆盖安装新版包。
- 新增 `Stats.edition` / `Stats.ocrReady` 字段；新增 5 命令、删除 1 事件（详见契约 v5）。

## [0.1.0] - 2026-11-05（收尾发布）

> 版本说明：仓库早期脚手架提交（2026-11-04）的内部版本号为 0.1.0，在本变更日志中改记为
> `0.0.1（2026-11-04，脚手架）`；本条目是**首个可安装的收尾版本**，包含此前各批次（P0/P1 技术验证、
> 追加需求可行性研究与契约 v4 落地）的全部内容，并首次归档发行安装包。

### 发布收尾：安装包入库与文档定稿（2026-11-05）

- **发行产物**：由 `pnpm build` 重新构建 NSIS 安装包并归档到 [`release/`](release/README.md)：
  `SnipAsk_0.1.0_x64-setup.exe`（2,810,494 B / 2.68 MiB，SHA256 `066347985d081db5…eed7de25`），
  附 `release/SHA256SUMS.txt` 与 `release/README.md`（产物清单、校验方法、安装/卸载、数据目录、
  未签名与 SmartScreen 说明、构建信息、已知限制）。
- **安装包冒烟（Windows 11 x64，本次发行包）**：静默安装 `/S` → 退出码 0，安装目录出现主程序与卸载器并注册 HKCU 卸载项；
  启动 6s → 进程存活、主窗口「框选 AI」、响应正常、工作集 40.1 MiB；静默卸载 `/S` → 退出码 0，
  程序文件/卸载项/开始菜单与桌面快捷方式全部清除；签名 `NotSigned`、版本资源 `0.1.0`。
- **README 发行化**：新增「下载与安装」章节（产物表、SHA256 校验、安装/卸载、数据目录、SmartScreen 提示），
  更新当前状态、快速开始、仓库结构、文档索引与最终验证事实；`SECURITY.md` 支持范围同步为
  「0.1.x：安装包随仓库分发（未签名）」。
- **工程门禁补强**：新增 `pnpm test` 统一前端纯逻辑单测（覆盖布局 25 + 线程模型 7），`scripts/verify.ps1` 与
  CI web job 增加单测步骤；`.gitignore` 增加 `release/` 例外（安装包与校验和入库）。
  另修复 `local_gateway_sse::server_errors_exhaust_retries` 在连接复用竞态下的偶发失败：
  「请求数」断言增加「退避耗时」兜底（5xx 重试最多等待 3×BASE_BACKOFF），测试不再受服务端连接关闭时序影响。
- **最终门禁**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`（0 warning）、`cargo test`
  （**376 passed / 0 failed / 6 ignored**）、`pnpm typecheck`、`pnpm test`、`pnpm build:web`、`pnpm build` 全部通过。

### 追加需求落地：本地 GPU / 覆盖翻译 / 连续提问（契约 v4，2026-11-05）

按 [`docs/追加需求可行性研究.md`](docs/追加需求可行性研究.md) 与实施规范实现三项追加需求。
**GPU 推理以 Vulkan 为主体、CUDA 为可选组件**，确保 AMD / Intel / NVIDIA 用户都能用（详见
[ADR-0006](docs/adr/0006-vulkan-first-gpu-inference.md)）。

#### 契约 v4（配置 / 命令 / 事件 / 结构）

- **配置 Schema v3 → v4**：`local.device`（`auto` / `cpu` / `gpu`）、`local.gpuBackend`
  （`auto` / `vulkan` / `cuda`，**`auto` = Vulkan → CUDA → CPU**）、`local.gpuLayers`（默认 99，`[0,512]`）；
  迁移链 **1 → 2 → 3 → 4**；`local.idleUnloadSec ∈ [0,2592000]`。`resources/models.manifest.json` 每个条目增加
  `vramMB`（0.5B=700 / 1.8B=1600 / 4B=3200；1.5B=1300 为研究估算，清单暂无该条目）。
- **新增 9 命令**（30–38）：`local_gpu_probe` / `local_engine_status` / `local_engine_start` /
  `local_engine_stop` / `backends_list` / `backends_download` / `backends_cancel` / `backends_remove` /
  `translate_cover`；**新增 2 事件**：`translate:progress` / `translate:ready`（事件总数 14 → 16）。
- `engine:status` 载荷扩展 `device` / `backend` / `fallbackReason` / `offloadLayers` / `deviceName` / `port`
  （旧字段 `local` 保留）；`TaskStartedPayload.parent`（首轮/重答为 `null`）；`RunActionOptions.cover`。
- 新增结构 `BackendInfo` / `GpuDevice` / `GpuBackendStatus` / `GpuBudget` / `GpuProbeResult` /
  `CoverBlock` / `CoveredBlock` / `CoverResult`；新增 `resources/backends.manifest.json`。
- `translate_cover` 定为**新命令**而非复用 `menu_execute`：覆盖翻译需要「块 id ↔ 译文」结构化映射，
  `task:delta` 事件流无法承载。`IPC_CONTRACT_VERSION` 保持 `1`（全部为纯增量）。

#### 需求一 · 本地模型 GPU 推理（Vulkan 优先，CUDA 可选）

- **GPU 探测**：真实执行 `llama-server --list-devices` / `--version` 并解析设备与显存，过滤虚拟/软件适配器
  （Microsoft Basic Render Driver / RDP / 虚拟化等），厂商归类，显存预算 `need = model.vramMB + 512 MiB`，
  推荐后端 **Vulkan 优先**，60s 缓存 + `refresh` 强制重探；`nvidia-smi` 仅作补充展示、失败静默。
- **后端组件**：`backends_list` 从清单 + 安装状态生成；`backends_download` 复用 Range 断点续传 / SHA256 /
  镜像 / `tar.exe` 解压；主安装包只带 CPU 后端，**Vulkan 为默认 GPU 组件按需下载**（AMD/Intel/NVIDIA 通用），
  CUDA（含 runtime）为 NVIDIA 可选组件。清单实际体积（官方 `b12128`）：Vulkan 37,684,327 B /
  CUDA 288,918,265 B + cudart 391,443,627 B / CPU 24,730,477 B（研究期估算 ≈55.8 MB 与 ≈0.9 GB，
  以 `resources/backends.manifest.json` 为准）。
- **引擎监督者**：`plan_attempts` 按配置与探测结果生成 **Vulkan → CUDA → CPU** 回退链；显式传
  `-ngl` / `--device Vulkan0|CUDA0`（研究 §1.2 踩坑）；端口 `39281..=39289` 探测；`/health` 200 → ready（20s 超时）；
  日志解析 offload 层数与 `using device`；空闲 `idleUnloadSec` 卸载；**按 PID/进程树**清理（禁止按镜像名 taskkill）。
- **网关**：`HttpLlmGateway` 实现 OpenAI 兼容流式 SSE（`/v1/chat/completions`），本地 llama-server 与云端共用；
  429/5xx 退避、取消令牌、usage。
- **设置页**：计算设备（自动/CPU/GPU）、GPU 后端（自动=Vulkan 优先/CUDA 可选）、GPU 层数（0–512）、
  设备状态行（如「已检测：AMD Radeon RX 7800 XT 16GB（空闲 15.2GB，Vulkan 可用，推荐 Vulkan）」/
  「未检测到可用 GPU，将使用 CPU」）、后端组件下载入口、显存不足提示与「改用 CPU」；文案明确
  「Vulkan 支持 AMD / Intel / NVIDIA」。
- **真机与遗留**：本机 **NVIDIA RTX 4060 Laptop** 上完成 Vulkan 真机验证（探测、真实推理、offload 日志、
  停止后无残留）；**AMD 真机未测**——兼容性按「Vulkan 路径无 NVIDIA 专属依赖」论证，不能等同实测；
  8GB 机型降档提示（S-13）仍为产品缺口。原始证据见 `.verify/evidence/`（gitignored）。

#### 需求二 · 覆盖式翻译（冻结帧 Canvas 覆盖层）

- 前端 `src/windows/overlay/translationLayout.ts`（**零依赖纯函数，`node --test` 可跑**）：
  OCR 行→段块合并（同行合并 + 垂直间距阈值 + 允许吞行间空白）、自适应字号（0.60–1.00×源行高，5% 步长）、
  自动换行（CJK/拉丁混排）、溢出降级小气泡（不截断，<12px 直接气泡）、深/浅底字色与对比度衬底、`clampRect`。
- `TranslationOverlay.tsx`：冻结帧上叠 Canvas，对原文块高斯模糊 + 半透明遮罩后原位绘制译文；
  三态 `cover | bilingual | original`；ESC 第一次回原文、第二次退出；resize/dpr 自适应；失败块保留原文并标红。
- 后端 `translate_cover`：10 块/批、会话级 `sha1(text+lang+model)` LRU 缓存、失败块保留原文 `failed=true`、
  `translate:progress`（节流）/ `translate:ready`；`engine` 如实标注（`skeleton` / 供应商 id / `local`）。
- 入口：动作菜单翻译项新增「覆盖原句」（仅 `hasText` 且 boxes 非空可用）；overlay 前端子阶段
  `translating → covered`（**后端会话保持 `showingResult`**，有意偏差，避免破坏已冻结契约）。
- **边界**：只做冻结帧覆盖，不实时穿透、不修改目标应用；DRM/无 `dataUrl` 明确降级提示（S-09 结论）。

#### 需求三 · 连续提问（多轮追问）

- **修复 F1（P0）**：`task_followup` 改为按当前活动会话 + `session.task_ids.last()` 定位；任务完成后从
  `TaskStore` 移除不再必然返回 `SA-1099`；增加「任务已完成后追问成功」回归测试。
- **多轮上下文**：`TurnStore` 记录每轮（原始问题 + 完整回答 + engine），追问时注入「选区 + 最近 N 轮问答（截断
  4000–6000 字）」到 `followup.md` 的 `{context}`；取消/失败不记录正文。
- **父子链路**：`TaskStartedPayload.parent` 指向上一轮任务（首轮/重答为 `null`），结果窗据此区分线程首轮/追问。
- **状态机**：任务失败改为 `executing → showingResult`（错误经 `task:error` 内联展示），失败后仍可追问/重答。
- **结果窗线程化**：`turns[]` 模型；历史轮次折叠摘要（可展开）、仅最后一条流式渲染、错误内联、流式期间禁用输入、
  重答清空当前线程并重跑、清空线程按钮；`task:started` 不再清空历史。
- **mock 对齐**：mock 追问守卫改为与真实后端一致（`showingResult` 且最近任务存在），不再出现「预览看着能用」的假象。
- 完整版（`messages` 表持久化 / 历史恢复 / token 预算）不在本批范围。

#### 工程与文档

- `package.json` 新增 `test:layout`（`node --test src/windows/overlay/translationLayout.test.ts`）。
- 新增 `docs/adr/0006-vulkan-first-gpu-inference.md`；更新 `docs/接口契约.md`（契约 v4）、`docs/实施架构.md`
  （§4.1 本地 GPU 模块 / §9.3 追加需求边界）、`docs/开发指南.md`（§8 GPU / Vulkan 与验证钩子）、
  `README.md`、`docs/技术验证清单.md`。
- **门禁与真机验收（2026-11-05 执行）**：
  - 全量门禁全绿：`cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings`（0 warning）、
    `cargo test --offline`（**376 passed / 0 failed / 6 ignored**，16 套件）、`pnpm typecheck`、
    `node --test src/windows/overlay/translationLayout.test.ts`（**25/25**）、
    `node --test src/windows/result/thread.test.ts`（**7/7**）、`pnpm build:web`。
  - 真机 GPU（NVIDIA RTX 4060 Laptop）：`local_gpu_probe` → `Vulkan0` 识别、`useable=true`、
    `recommended="vulkan"`、预算 `{requiredMb:2112, availableMb:7180, fits:true}`；真实 Vulkan 引擎
    （`-ngl 99 --device Vulkan0`，1.8B Q4）→ `/health` 就绪 **2188 ms**、日志 `offloaded 33/33 layers to GPU` +
    `using device Vulkan0`、真实 `/v1/chat/completions` SSE 输出 **90 ms**（CPU `-ngl 0` 同请求 **482 ms**，约 5.4×）、
    `nvidia-smi` 佐证 3477 MiB、按 PID 树停止后无残留；Vulkan 后端组件真实下载 **37,684,327 B** + SHA256 校验一致 +
    跨进程 Range 续传 + 解压后官方 exe 复测 `Vulkan0`。
  - 覆盖翻译：浏览器 mock **真实位图**演示（`.verify/evidence/overlay-cover/`）覆盖模式 Canvas 采样
    `opaque=46964/50400`、原文模式 `opaque=0`，三态/ESC 正确、0 控制台异常；真实 OCR/翻译质量未验收。
  - 连续提问：`followup_regression` **9/9**（含 F1「任务完成后追问成功」、上下文含上一轮回答、失败后可追问）。
  - **遗留（如实标注）**：**AMD/Intel 实机未测**（Vulkan 无 NVIDIA 专属依赖为论证依据）；CUDA 真实推理未实测
    （本机缺 CUDA runtime，探测如实 `available=false`）；真实捕获/OCR 与覆盖翻译生产链路、4B/8GB 边界、真实流式 TTFT 未验收。
  - 通过/失败矩阵、复跑命令与原始输出见 `.verify/verify-20261105-2311/SUMMARY.md`（gitignored，本地证据）。

### 技术验证（P0 Spike，2026-11-04/05）

按 `docs/技术验证清单.md` 完成 **P0（Go 前提条件）S-01 ~ S-08** 首轮 spike。逐项记录与原始数据索引见
[`docs/spikes/README.md`](docs/spikes/README.md)。

- **结论**：S-01（混合 DPI 多屏坐标）、S-02（整屏 OCR det）、S-04（全局热键/冲突/暂停释放）、
  S-07（离线无 Key 受限模式）、S-08（模型下载断点续传与校验）= **DONE**；
  S-03 选区 OCR = **FAILED**（det+rec P95 968ms > 800ms；「整屏 det 缓存 → 选区过滤 + rec-only」P95 596ms 达标，
  需架构/阈值决策）；S-05 本地翻译 = **FAILED**（CPU P50 11.76s > 8s），云端 = **BLOCKED**（无 API Key）；
  S-06 图片搜索 = **BLOCKED**（无 vision/检索 Key）。FAILED/BLOCKED 项不得跳过门禁进入 MVP。
- **新增验证基础设施**：`scripts/spikes/`（`applab` 便携隔离 + WebView2 CDP 驱动真实应用、DPI 感知截图、
  S-01/S-02/S-03/S-04/S-05/S-06/S-07/S-08 复跑脚本与资源纪律）；`docs/spikes/`（7 份 §4 模板验证记录 + 索引）。
- **修复（真实验证发现）**：
  - overlay 由「仅主屏」改为**覆盖整个虚拟桌面的物理矩形**（`PhysicalSize/PhysicalPosition` + client 原点
    迭代校正 + `.shadow(false)`），并修正下发 `dpr` 为窗口实际 `scale_factor()`；修复无边框阴影 inset 导致的
    整体偏移 (11,2)px，跨屏框选误差实测 0.01px（原实现跨屏不可用）。
  - `hotkey_probe` 增加 **OS 级**占用探测（无冲突时临时注册后立即注销，无泄漏）；设置页/引导页把
    `conflictWith="system"` 映射为可操作改键文案。
  - **暂停语义修正**：原 `set_paused(true)` 会注销包括 `togglePause` 在内的全部热键，导致暂停后无法用热键恢复；
    现暂停时保留 `togglePause`、其余释放（真机验证「暂停后辅助进程可注册 Ctrl+Q」）。
  - **受限模式能力门禁**：无可用云端 Key / 无本地模型时，AI 搜索/解答/图片搜索/云端翻译 disabled 并给出
    「去设置配置 Key 或下载本地模型」的可操作原因；跳过引导/引导完成但未配置均进入同一受限状态。
  - 修复 onboarding 两个真实缺陷：无 Key 用户第 1/2 步提交 `{ai:null}` 被判非法；第 3/4/5 步嵌套 payload
    被后端静默丢弃（隐私同意/上传开关/模式选择不生效），新增 `normalize_step_payload` 兼容嵌套与扁平。
  - **模型下载真实实现**：`reqwest` HTTP Range 断点续传 + `.partial` 保留 + 流式 SHA256 + 大小校验 +
    多镜像回退 + 取消/恢复；骨架模式（`skeletonMode=true`）行为逐字保留，开关经 `config_patch` 热同步。
  - **模型清单修正**：`resources/models.manifest.json` 原三个 URL 实测 404；hy-mt / qwen3 已改为官方文件名 +
    真实大小 + SHA256（`verified` 仍为 false，因 opus-mt 官方暂无 Ct2 产物，`urls` 置空并在 `params.pending` 说明）。
- **行为变更**：骨架模式**不豁免**能力门禁——默认配置（无 Key）下云动作会 disabled 并提示配置路径；
  本地翻译需已安装本地模型才可用。这是「无 Key/离线可用」目标的必要行为。
- **依赖**：新增 `reqwest 0.13`（`native-tls`/`stream`/`http2`/`charset`/`system-proxy`，Windows 走 Schannel）；
  `tokio` 增加 `fs`/`io-util`。
- **门禁**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（**200 passed**：
  160 lib + 16 契约 + 12 S-08（2 ignored 真实网络/子进程助手）+ 6 S-01 + 6 S-04）、`pnpm typecheck`、
  `pnpm build:web` 全部通过。

### 技术验证第二轮 · P0 门禁处置落地（2026-11-05）

按用户对首轮 P0 结论的 4 项决策完成落地（逐项记录见 [`docs/spikes/`](docs/spikes/README.md)，实施边界见
[`docs/实施架构.md`](docs/实施架构.md) §9.1）：

- **S-03 选区 OCR 口径变更并落地**：新增「整屏 det 缓存 → 选区过滤 + rec-only」链路（实测 P95 596ms，目标 800ms）：
  `ocr_service` 新增 `det_input_size` / `OcrEngine::{detect, recognize, detect_and_recognize}` / `OcrPipeline`
  （缓存按 `ocr_session_id`、帧局部↔绝对物理坐标平移、缓存缺失/无命中回退 det+rec、新会话失效、空选区零推理），
  `app/state.rs` + `commands/{capture,selection}.rs` 接线；新增 19 项单测 + 9 项集成测试 + 真机冒烟（6 组跨屏拖选 → 菜单正常）。
- **S-02 OCR 像素预算落地**：det 输入改为「长边 ≤ `detMaxEdge`（默认 3200）且 宽×高 ≤ 3.0Mpx」双约束、绝不放大
  （`DET_PIXEL_BUDGET`）。真实 ONNX 复核（干净窗口）：双 4K 7680×2160→3200×900 P95 **743ms**；单 4K→2309×1299
  P95 **794ms**（旧长边口径 3200×1800 为 1682ms，超 1.2s）；主屏 2560×1440→P95 761ms；低配建议 2.5Mpx（P95 632ms）。
- **引导页三档 AI 服务（用户决策 item3）**：翻译方式改为 **云端 / 极小模型（本地）/ 普通模型（本地）** 三选一，
  并显式提示性能与隐私/成本（部分用户为保密/成本选本地）：
  - 云端：实测（DeepSeek，关闭 thinking）首字 P50 0.61s / P95 0.87s；默认 thinking 首字 P95 4.59s（不达标）；
  - 极小模型：Qwen2.5-0.5B-Instruct Q4 + 精简提示词，200 字 P95 **3.21s**、常驻 ≈0.55GiB，质量一般（长文建议云端）；
  - 普通模型：HY-MT1.5-1.8B Q4，实测 P50 11.76s / P95 21.84s（质量优先、纯 CPU 慢）。
  选择写入 `translate.backend` + `local.model`；模型下载列表按档位聚焦；DeepSeek 供应商说明修正（支持视觉、
  无联网搜索 API）。
- **极小模型提示词路由**：新增 `resources/prompts/translate_minimal.md`；`action_router::translate_prompt_key`
  在「本地 + 极小模型 id」时路由到精简提示词（S-05 实测：0.5B 用产品长提示词会复述原文 10/10）。
  同时修复既有契约承诺未实现的问题：`prompt_engine` 现会在渲染前**剔除提示词维护注释头**（此前注释会发给模型）。
- **S-05 云端实测（用户提供 DeepSeek Key）**：`deepseek-flash` 关闭 thinking 首字达标；**默认 thinking FAILED**
  （reasoning 384–736 token）→ 真实网关接入时**必须显式关闭 thinking**（`llm_gateway` TODO 第 3 条）。
  总耗时 P95 1.75s 略超 1.5s（建议验收口径明确为「首字」）。
- **S-06 解阻并完成真实验收（2026-11-05 续）**：用户提供 DeepSeek 官方 **Search/Fetch 工具 API**
  （`POST /v1/tools/search`、`POST /v1/tools/fetch`，与 Chat 同一 Bearer 鉴权）后，基准脚本补齐真实检索/抓取分支并跑
  完整图片搜索四阶段流水线（视觉识别 → 检索词 → 真实 Search → 摘要；deep 模式并发 Fetch 2 条）：
  quick **P50 5.28s / P95 6.01s ≤ 10s**；deep **P50 6.83s / P95 8.29s ≤ 25s**，各 20 次、0 流水线失败。
  Search 单独 P95 413ms、Fetch 稳定源 P95 3.2s；长尾主因是检索结果失效 URL（建议 Fetch 5–8s 超时、404 不重试并回退
  Search 摘要）。**S-06 由 BLOCKED 改为 DONE**；产品网关接入仍需补 Search/Fetch 客户端、`thinking` 关闭开关与费用计量
  （已写入实施架构 TODO #3 与 §9.1）。
- **模型清单扩充**：`resources/models.manifest.json` 新增第 4 条 `qwen2.5-0.5b-instruct-q4`（491,400,032 B，
  sha256 `74a4da8c…`，ModelScope 文件列表 API + Range 206/416 双复核），`params.profile=tiny-local` /
  `promptProfile=translate-minimal`；opus-mt 仍缺 Ct2 产物（`urls` 空）。
- **供应商连通性真实化（S-17 预置）**：`provider_test` 在 `skeletonMode=false` 时真实请求 `GET {baseUrl}/models`
  （Bearer + UA、connect 5s/总 15s），错误映射契约 §8.2（401/403→SA-3002、402→SA-3003、429→SA-3004、超时→SA-3005、
  DNS/拒绝/5xx→SA-3001、其他 4xx→SA-5001）；15 个 mock 用例 + 真实 DeepSeek 冒烟 200 / 208ms。
- **安全**：用户提供的 API Key 仅用于运行时环境变量与脱敏冒烟，**未写入仓库/文档/日志**（全仓扫描无泄漏）。
- **行为变更**：引导三档选择会直接写 `translate.backend`/`local.model`；`provider_test` 真实模式下不再返回占位成功；
  提示词渲染不再包含维护注释头。
- **门禁**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（**242 passed**：178 lib +
  16 契约 + 12 S-08（2 ignored）+ 9 OCR pipeline + 15 provider（1 ignored）+ 6 S-01 + 6 S-04）、`pnpm typecheck`、
  `pnpm build:web` 全部通过。

### 追加需求可行性研究（2026-11-05）

按用户提出的 3 项追加需求完成可行性研究与真实证据采集，结论见
[`docs/追加需求可行性研究.md`](docs/追加需求可行性研究.md)，配套证据（3 份证据报告 + 汇总 JSON + 原型图）见
[`docs/research/`](docs/research/README.md)：

- **本地模型 GPU（可行，强烈推荐）**：本机 RTX 4060 Laptop 8 GB 实测 CUDA `-ngl 99`：1.8B 产品提示词
  P50 1 256 / P95 3 747 ms（CPU 同窗 9 767 / 10 137 ms；S-05 历史 P95 21 837 ms），0.5B/1.5B P95 591/1 144 ms；
  显存峰值 3 712 MiB（45%），prefill 58 ms（流式首字预期 <0.2 s）；Vulkan 同档、CUDA 略优；给出
  `local.device` 配置/探测/回退/打包（Vulkan 55.8 MB 默认组件、CUDA ≈0.9 GB 可选）与人日（MVP-1 ≈4.5）。
- **覆盖式翻译（可行，冻结帧覆盖层）**：100 张真实语料（2 288 行）量化 + 6 组原型（含真实 Win32 记事本）：
  中→英字符扩张 P50 2.46×、行合并 −32.2%、推荐配置下溢出 0%；明确 A（冻结帧覆盖）/B（实时穿透）/C（改写应用）
  中仅 A 可行，给出 Canvas 布局算法、阶段机与人日（≈7.5，含加固 ≈10）。
- **连续提问（已部分实现，存在 P0 缺陷）**：真实应用 IPC 动态复现 `task_followup` 在任务完成后**必然 SA-1099**
  （任务已移出 TaskStore）；上一轮回答未进 prompt、结果窗覆盖而非线程、无 messages 持久化；给出修复与
  MVP/完整版设计及人日（MVP ≈6.5–10.5，最短 4.5–6）。
- **前置依赖**：真实捕获/OCR/LLM 网关/本地 llama.cpp 引擎仍为 `TODO(real)`；三项新需求的排期与依赖关系见报告 §4。

### 技术验证（P1 Spike：S-09 ~ S-20，2026-11-05）

按 [`docs/技术验证清单.md`](docs/技术验证清单.md) 完成 **P1（技术难点/验收标准）S-09 ~ S-20** 共 12 项验证：
逐项结论见 [`docs/spikes/README.md`](docs/spikes/README.md) §6，记录见 `docs/spikes/S-09…S-20*.md`，
复跑脚本见 [`scripts/spikes/`](scripts/spikes/README.md)。本轮由 AI 助手团队（Agent Teams）执行，
除 S-19 真人走查外均为真实应用/真实模型/真实 HTTP 证据，mock 仅作契约自检且明确标注。

- **DONE**：S-09（WGC 实机可用且与 BitBlt 像素级一致；WDA_MONITOR 保护面 BitBlt/WGC 均纯黑、
  WDA_EXCLUDEFROMCAPTURE 下 WGC 3/3 无帧 → 黑帧/空帧都判失败；产品新增纯 safe `is_black_frame`/
  `resolve_capture` 降级链 + SA-1001 16 项单测）；S-10（release 冷启动 P95 858ms ≤1.5s、空闲主进程 WS max
  36.8MiB ≤120MB、空闲 CPU 均值 0.0022% <0.5%）；S-11（推荐 `det≥0.6+面积≥400px²+行高≤70px` 误判 2.0%/漏判 0%）；
  S-15（字符准确率 native 99.38% / budget 99.29%，100/100 张 ≥95%）；S-16（未签名安装/启动 840.4ms/卸载通过、
  Defender 无威胁）；S-17（真实 app IPC 20/20 错误码映射 + 断网降级链 + 真实 DeepSeek 200/350ms）；
  S-18（NSIS 2.57 MiB ≤25MB，CI 体积门禁落地）；S-20（第二实例 10/10、退出后 Ctrl+Q 无残留 5/5、暂停/恢复 3/3）。
- **FAILED/部分通过**：S-12 来源可访问率 59.6%（乐观上界 94.1% <95%；四段 100%、不确定标注 100%）；
  S-14 云端术语/格式 100% 通过，但本地最佳 1.5B 格式 68.1%、0.5B 不达标。
- **IN PROGRESS / BLOCKED**：S-13（常驻 0.5B/1.5B/1.8B = 520.8/1715.3/2102.4 MB；Job Object 本机未强制 →
  OOM 未测；8GB 降档提示缺失）；S-19（真人 3 人不可得 → BLOCKED，已交付走查协议 + 自动走查 4.24s）。
- **代码/工程**：`platform/capture.rs` + `services/capture_service.rs` 黑屏检测与降级决策（16 项单测）；
  `.github/workflows/ci.yml` 新增 `bundle` job（`pnpm tauri build` + ≤25MiB 门禁 + 体积基线归档，
  `SKIP_BUNDLE_GATE=true` 可跳过）；新增 P1 脚本包 `scripts/spikes/{s09,s10_s16_s18,s11_s15,s12_s14,s13_s17_s19_s20}/`
  与 12 份记录、聚合数据。
- **安全**：用户提供的 DeepSeek Key 仅存 gitignored `.verify/p1/key.txt`，运行时读取；仓库/日志/文档无泄露。
- **门槛**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（**258 passed / 3 ignored**）、
  `pnpm typecheck`、`pnpm build:web` 全部通过。

### 修复（真实编译与运行验证，2026-11-04）

- **事件名合法化**：Tauri 2 的 `Emitter::emit` 不允许事件名包含 `.`（原 `overlay.ready` 等 14 个事件在真实运行中全部发射失败），统一改为 `域:动作` 形式（`overlay:ready`、`task:delta` …），并同步契约文档、TS/Rust 常量、mock 与契约测试。
- **overlay 窗口尺寸**：`WebviewWindowBuilder::{inner_size, position}` 接受**逻辑像素**，原实现误传主显示器**物理尺寸**，在 150% 缩放下窗口被放大到 3840×2160 而越界；改为按 `dpr` 换算，并在复用窗口时重新对齐几何。
- **`overlay.ready` 冷启动竞态**：overlay 窗口首次创建时 WebView 尚未完成加载，广播的 `overlay:ready` 会丢失（骨架/真实捕获均必现）；`capture_freeze` 增加**幂等语义**（已有冻结/选区会话则复用并补发快照），配合 `Builder::on_page_load` 补发与前端挂载后主动拉取，三条路径兜底。
- **日志降噪**：捕获/事件诊断日志降为 `debug` 级别（默认 `info` 不刷屏），保留生命周期与失败告警。
- **验证结果**：`cargo build`（debug / release / `--features custom-protocol`）、`cargo test`（132 单测 + 16 契约测试）、`cargo clippy -D warnings`、`cargo fmt --check` 全部通过；release 应用实机启动并完成「`Ctrl+Q` → 框选 → 动作菜单 → 流式结果窗」端到端闭环（骨架模式），配置/历史库/日志落盘正常。

## [0.0.1] - 2026-11-04（脚手架）

首个**基础架构（脚手架）**版本：可运行的空壳闭环，真实外部能力均为 `TODO(real)` 占位实现。

### 新增

- **分层结构**：Rust 侧 `contract / app / commands / domain / services / platform / events / resources` 单向依赖分层；前端 `app / shared(contract·ipc·theme·util) / components/ui / windows(main·overlay·result)`。
- **IPC 契约 v1**（`docs/接口契约.md`）：29 个命令、14 个事件、`AppError` 错误码表（SA-10xx–SA-90xx）、会话状态机、配置 Schema **v3**（含迁移链与原子写约束）、凭据存储约定、物理像素坐标约定；`src/shared/contract/` 与 `src-tauri/src/contract/` 双端同构。
- **配置中心**：默认配置、深度合并、校验、迁移、原子持久化与 `config:changed` 热生效分类。
- **骨架模式（skeleton）**：`advanced.skeletonMode = true` 时捕获/OCR/LLM/搜索/翻译/下载/更新均按契约 §12 的确定性占位实现，事件流与真实实现一致，无网络、无密钥、无模型也能完整走通 UI 闭环。
- **前端三窗口**：`main`（设置页 + 6 步首启引导）、`overlay`（冻结层 + 框选 + 放大镜 + 动作菜单）、`result`（流式输出 + 来源 + 追问），浏览器 mock 后端支持 `/?window=` 独立预览。
- **内置资源**：`resources/prompts/*.md`（5 个提示词模板）、`resources/models.manifest.json`（3 个本地模型条目，`verified: false`）、`resources/pricing.json`（模型与搜索计价，CNY）。
- **应用图标**：`src-tauri/icons/` 下 32/128/256/512 PNG 与多尺寸 ICO，由 `scripts/gen-icons.py`（Pillow）生成。
- **工程配套**：pnpm 脚本、`scripts/{dev,build,verify}.ps1`、`docs/开发指南.md`、`docs/技术验证清单.md`、ADR 0001–0005、Apache-2.0 `LICENSE`、`NOTICE`、`.gitignore` / `.gitattributes` / `.editorconfig`、GitHub Actions CI（windows-latest：Web 类型检查与构建 + Rust fmt/clippy/test）。

### 说明

- 本版本为**脚手架**，不可用于生产环境；Rust 侧业务逻辑与外部调用均为占位。
- 开发机若未安装 Rust/MSVC 工具链，可用 `pnpm dev:web` 在浏览器中验证 UI；Rust 检查在 CI（windows-latest）执行，或按 `docs/开发指南.md` 安装工具链后运行 `scripts/verify.ps1`。
- 内置提示词可由 `%APPDATA%\SnipAsk\prompts\*.md` 覆盖（用户 > 内置）。
