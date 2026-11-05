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
