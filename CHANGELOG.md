# 更新日志

本文件记录「框选 AI（Snip Ask）」的版本变更，格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 计划中

- 技术验证（Spike）：`Ctrl+Q` → 虚拟桌面冻结 → 前置 OCR → 框选 → 流式问答最小闭环（见 `docs/技术验证清单.md`）。
- 真实能力接入：Win32 `BitBlt`/WGC 捕获、RapidOCR/ONNX 推理、OpenAI 兼容流式网关、DeepSeek Search/Fetch、Qwen-MT、本地 llama.cpp/Ollama 引擎、Windows Credential Manager、模型下载与 SHA256 校验。
- 安装包（NSIS 用户级）、代码签名、更新通道与体积门禁。

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

### 修复（真实编译与运行验证，2026-11-04）

- **事件名合法化**：Tauri 2 的 `Emitter::emit` 不允许事件名包含 `.`（原 `overlay.ready` 等 14 个事件在真实运行中全部发射失败），统一改为 `域:动作` 形式（`overlay:ready`、`task:delta` …），并同步契约文档、TS/Rust 常量、mock 与契约测试。
- **overlay 窗口尺寸**：`WebviewWindowBuilder::{inner_size, position}` 接受**逻辑像素**，原实现误传主显示器**物理尺寸**，在 150% 缩放下窗口被放大到 3840×2160 而越界；改为按 `dpr` 换算，并在复用窗口时重新对齐几何。
- **`overlay.ready` 冷启动竞态**：overlay 窗口首次创建时 WebView 尚未完成加载，广播的 `overlay:ready` 会丢失（骨架/真实捕获均必现）；`capture_freeze` 增加**幂等语义**（已有冻结/选区会话则复用并补发快照），配合 `Builder::on_page_load` 补发与前端挂载后主动拉取，三条路径兜底。
- **日志降噪**：捕获/事件诊断日志降为 `debug` 级别（默认 `info` 不刷屏），保留生命周期与失败告警。
- **验证结果**：`cargo build`（debug / release / `--features custom-protocol`）、`cargo test`（132 单测 + 16 契约测试）、`cargo clippy -D warnings`、`cargo fmt --check` 全部通过；release 应用实机启动并完成「`Ctrl+Q` → 框选 → 动作菜单 → 流式结果窗」端到端闭环（骨架模式），配置/历史库/日志落盘正常。

## [0.1.0] - 2026-11-04

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
