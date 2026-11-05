# 框选 AI（Snip Ask）

> 轻量快速、托盘常驻、可开机自启、高度自定义的 Windows 框选解答 / 翻译 / 搜索工具。
> 按 `Ctrl+Q` 冻结屏幕 → 框选任意区域 → 搜索 / 翻译 / 解答，结果以流式输出秒级呈现。

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2B%20x64-0078D4.svg)
![Release](https://img.shields.io/badge/release-v1.0.0-2ea44f.svg)
![Status](https://img.shields.io/badge/status-real%20product-brightgreen.svg)

**框选 AI** 是 BYOK（Bring Your Own Key）的桌面工具：没有服务端、不收集截图、不做遥测；
用户自备 AI 服务密钥，截图与文本只在用户主动触发云端动作时才发送。开源协议为 [Apache-2.0](LICENSE)。

本仓库为 **v1.0.0**：真实 Win32 捕获、真实本地 OCR（RapidOCR PP-OCRv4 / ONNX Runtime）、
本地 llama.cpp 推理、云端 DeepSeek Chat/Search/Fetch 与 Qwen-MT、Windows Credential Manager、
历史消息持久化，并提供 **slim / full 双安装包**（[`release/`](release/README.md)）与
**[《自定义指南》](docs/自定义指南.md)**。**本软件不自动更新**（见[升级方式](#升级与自定义)）。

> **能力边界（如实标注）**：真实能力已接入并完成本机验证；**AMD / Intel 真机 GPU 未测**
> （Vulkan 路径按「无 NVIDIA 专属依赖」论证）、**安装包未做代码签名**（SmartScreen 提示属预期）、
> **本机无 DashScope Key**（Qwen-MT 仅协议级验收，真实翻译未验收）。逐项证据与遗留见
> 本地证据 `.verify/verify-v1.0.0/SUMMARY.md`（gitignored）与
> [`docs/技术验证清单.md`](docs/技术验证清单.md)。

---

## 下载与安装

### 发行产物（双版本）

| 产物 | 说明 | 体积 / 门禁 |
| --- | --- | --- |
| [`release/SnipAsk_1.0.0_x64-setup.exe`](release/SnipAsk_1.0.0_x64-setup.exe) | **slim（安装后下载版）**：内置应用 + ONNX Runtime + `docs/`；首次使用 OCR 时下载 4 个模型/字典（约 16 MB） | **7,772,978 B（7.41 MiB）** / ≤ 25 MiB |
| [`release/SnipAsk_1.0.0_x64-full-setup.exe`](release/SnipAsk_1.0.0_x64-full-setup.exe) | **full（内置 OCR 模型版）**：slim 的全部内容 + OCR 模型/字典，离线开箱即用 | **22,564,103 B（21.52 MiB）** / ≤ 60 MiB |
| [`release/SHA256SUMS.txt`](release/SHA256SUMS.txt) | 两个安装包的 SHA256 校验和 | — |
| [`release/README.md`](release/README.md) | 发行说明：逐包体积/SHA256、构建信息、安装/卸载、已知限制 | — |

> 两版功能完全一致，差异只在「OCR 模型是否随包」与安装包体积。选择建议：
> 能联网/想要更小安装包 → **slim**；离线/内网/不想首次下载模型 → **full**。
> 体积为 v1.0.0 最终归档产物实测（2026-11-06）；SHA256 以 `release/SHA256SUMS.txt` 为准。

### 安装步骤

1. 下载安装包，先校验完整性：

   ```powershell
   Get-FileHash .\SnipAsk_1.0.0_x64-setup.exe -Algorithm SHA256
   # 与 release\SHA256SUMS.txt 中对应条目比对
   ```

2. 双击运行安装包；或静默安装（当前用户）：
   `Start-Process .\SnipAsk_1.0.0_x64-setup.exe -ArgumentList '/S'`。
3. 安装完成后启动 **SnipAsk**；首次启动进入 6 步引导（无网络 / 无 API Key 可跳过并进入受限模式）。
4. 按 `Ctrl+Q` 开始框选；托盘菜单可打开设置、暂停热键或退出。

> **未签名提示**：v1.0.0 安装包**未做代码签名**。首次运行时 Windows SmartScreen 可能显示
> 「Windows 已保护你的电脑」，点击「更多信息 → 仍要运行」即可。已在 Windows 11 + Defender 实机验证
> 安装/启动/卸载且 Defender 扫描无威胁（[S-16 记录](docs/spikes/S-16-无签名安装包与杀软误报.md)）；
> Win10 干净机、火绒等环境未覆盖。

### 卸载与数据目录

- **卸载**：设置 → 应用 → 已安装的应用 → **SnipAsk** → 卸载；或运行
  `%LOCALAPPDATA%\SnipAsk\uninstall.exe`。卸载会移除程序文件与快捷方式，但**保留用户数据**。
- **数据位置**：

  | 内容 | 路径 |
  | --- | --- |
  | 配置 / 提示词 / 日志 / 历史库 | `%APPDATA%\SnipAsk\` |
  | 本地模型 / OCR 模型 / 缓存 / 运行时 | `%LOCALAPPDATA%\SnipAsk\`（安装目录同名，卸载后可能残留 `models/cache/runtime`） |
  | WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` |

  彻底清理：先卸载，再手动删除上述目录（并在 Windows 凭据管理器删除 `SnipAsk/` 条目）。
  便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`。

---

## 升级与自定义

**本软件不自动更新**：没有更新检查、不联网查询版本、没有更新通道设置。
升级方式：

1. 下载新版安装包**覆盖安装**（保留配置、提示词、历史与凭据）；
2. 或按[《自定义指南》](docs/自定义指南.md) §10 从源码构建你自己的版本（Apache-2.0 允许修改分发）。

**自定义**（完整说明见 [`docs/自定义指南.md`](docs/自定义指南.md)）：

- 提示词覆盖：`%APPDATA%\SnipAsk\prompts\*.md`（6 个模板，用户 > 内置）；
- 术语表、目标语言、热键、动作菜单、模式、隐私开关：设置页或 `config.json`（schema v5）；
- 本地模型 / 后端替换：`SNIP_ASK_MODEL_PATH`、`SNIP_ASK_BACKENDS_DIR` 等调试钩子；
- 接本地 Ollama / llama-server / 自建 OpenAI 兼容网关：新增供应商（`providers[].kind`）；
- 界面内入口：设置 → **自定义与关于**（版本 / 版本类型 slim·full·dev / OCR 状态 / 一键打开
  配置、提示词、模型、日志、后端、安装文档目录 / 无自动更新说明）。

---

## 核心交互

按 `Ctrl+Q` 真实捕获并冻结屏幕 → 拖拽框选目标区域（支持方向键微调、回车确认、`Esc` 取消）→
在动作菜单中选择 **搜索 / 翻译 / 解答 / 复制**，结果在结果窗中流式呈现，可追问、重答、复制、关闭。

- **真实捕获（BitBlt）**：冻结虚拟桌面（物理像素、支持负坐标与混合 DPI），黑屏/受保护内容按
  `SA-1001` 明确降级提示；WGC 不作主路径（与 BitBlt 像素级一致且不能绕过 DRM）。
- **真实本地 OCR（RapidOCR PP-OCRv4）**：整屏后台 det（像素预算，不阻塞选框）+ 选区 rec-only；
  「有文字」时提供文本搜索、翻译、解答、复制；「无文字/图片」时提供图片搜索、看图解答、翻译。
- **云端与本地推理**：DeepSeek（Chat / 图片搜索 / Search / Fetch）、DashScope Qwen-MT 专用翻译、
  通用 OpenAI 兼容供应商；本地 llama.cpp（**Vulkan 优先 / CUDA 可选 / CPU 回退**）。
- **覆盖式翻译**：翻译结果**原位覆盖**冻结帧上的原文（模糊原区域 + 按版式重绘译文），
  可切「原文 / 双语 / 覆盖」；只在框选画面上显示，不改动目标应用。
- **连续提问与历史**：结果窗多轮追问；成功任务写入 `history.db`，重启后可在主窗口浏览历史会话与线程。
- **BYOK**：API Key 只存 Windows Credential Manager，界面仅掩码；`privacy.localOnly` 可禁用全部云端动作。

---

## 当前状态

| 层次 | 内容 | 状态 |
| --- | --- | --- |
| 域逻辑 | 会话状态机、选区/坐标换算、文字存在判定、动作注册表、费用估算、多轮线程（纯逻辑 + 单测） | ✅ 已实现 |
| IPC 契约 | **43 命令 / 15 事件（契约 v5）** / `AppError` 错误码表 / 配置 Schema **v5**，TS 与 Rust 双端同构 | ✅ 已实现 |
| 配置中心 | 默认值、深度合并、校验、迁移（**1→2→3→4→5**）、原子写 + `.bak`、`config:changed` 热生效分类 | ✅ 已实现 |
| 前端窗口 | `main`（设置页 + 自定义与关于 + 历史 + 6 步引导）、`overlay`（冻结层 + 选框 + 放大镜 + 动作菜单 + 覆盖翻译层）、`result`（线程式流式输出 + 来源 + 追问） | ✅ 已实现 |
| **真实捕获** | Win32 BitBlt 虚拟桌面捕获（BGRA + PNG）、黑屏/空帧判定、`SA-1001` 降级（[ADR-0009](docs/adr/0009-real-capture-ocr.md)） | ✅ 已实现（本机真机测试） |
| **真实 OCR** | RapidOCR PP-OCRv4（det/rec/cls + 字典）、ONNX Runtime（`ort` load-dynamic）、像素预算、整屏 det 缓存 + 选区 rec-only | ✅ 已实现（字符准确率 S-15 实测 99.29–99.38%） |
| **云端推理/搜索** | DeepSeek Chat/视觉流式、`tools/search` + `tools/fetch`、图片搜索四阶段、Qwen-MT、通用 OpenAI 兼容 | ✅ 已实现（Qwen-MT 真实翻译因本机无 Key 未验收） |
| **本地 GPU 推理** | GPU 探测、显存预算、组件按需下载、引擎监督者（Vulkan → CUDA → CPU 回退） | ✅ 已实现（NVIDIA RTX 4060 Laptop 真机；**AMD/Intel 真机未测**） |
| **凭据** | Windows Credential Manager（keyring，service `SnipAsk`），界面仅掩码 | ✅ 已实现 |
| **历史持久化** | `messages` 表、`history_list` / `history_thread` / `history_clear`，重启后可回溯 | ✅ 已实现 |
| **双安装包** | slim（安装后下载 OCR 模型，≤25 MiB）/ full（内置模型，≤60 MiB），`Stats.edition` 标注 | ✅ 已交付（体积/SHA256 见 release） |
| **自动更新** | updater 插件/服务、`advanced.updateChannel`、`update:status` 全部移除（[ADR-0008](docs/adr/0008-remove-auto-update.md)） | ✅ 已移除（升级 = 覆盖安装） |
| 工程底座 | 分层目录、内置资源、图标、pnpm 脚本、`scripts/*.ps1`、CI（windows-latest）、ADR-0001～0009 | ✅ 已实现 |
| **覆盖式翻译** | 段块合并 + 自适应布局纯函数 / 冻结帧 Canvas 三态 / `translate_cover` 批处理 + 会话级 LRU | ✅ 已实现（布局单测 25/25；真实 OCR/翻译质量依赖真实链路） |
| **连续提问** | 上下文注入 / `task:started.parent` / 失败可继续追问 / 线程式结果窗 | ✅ 已实现（回归 9/9 + 线程 UI 7/7） |

图例：✅ 已实现/已交付。逐模块（M01–M20）映射见 [`docs/实施架构.md` §6](docs/实施架构.md)。
**已知限制**：AMD/Intel 真机 GPU 未测、无代码签名、DashScope 真实翻译未验收（本机无 Key）、
本地小模型翻译格式保持不完美（E2E local 1.5B 实测含提示词残留）、
8GB 机型降档提示缺失、Win10/国产杀软/真人可用性（S-19）未覆盖、Markdown 渲染/多语言待打磨。
完整遗留见 [`docs/实施架构.md` §9](docs/实施架构.md) 与本地证据 `.verify/verify-v1.0.0/SUMMARY.md`。

---

## 快速开始

### 终端用户（直接安装）

无需配置开发环境：从 [`release/`](release/README.md) 取得 slim 或 full 安装包 → 校验 `SHA256SUMS.txt`
→ 双击安装。安装、卸载、数据目录、未签名提示与双版本差异见[下载与安装](#下载与安装)。

### 环境要求（从源码构建）

| 工具 | 版本 | 说明 |
| --- | --- | --- |
| Windows | 10（1903+）/ 11，x64 | 目标平台 |
| Node.js | ≥ 20（推荐 22 LTS；前端 TS 单测需 ≥ 22.18） | 前端构建与 `node --test` 单测 |
| pnpm | ≥ 9（仓库按 11 校验） | 包管理，可用 `corepack enable` 启用 |
| Rust | stable（≥ 1.77） | **仅桌面版需要**；通过 [rustup](https://rustup.rs/) 安装 |
| VS Build Tools 2022 | 「使用 C++ 的桌面开发」工作负载 | 提供 MSVC 链接器 `link.exe` 与 Windows SDK |
| WebView2 Runtime | Win11 自带 | Windows 10 由安装器或系统更新安装 |

> 若本机**未安装 Rust/MSVC**：可先用浏览器 mock 预览前端，Rust 侧由 CI
> （windows-latest 上的 fmt/clippy/test）兜底。安装步骤见 [`docs/开发指南.md`](docs/开发指南.md)。

### 安装依赖

```powershell
pnpm install
```

### 浏览器预览（无需 Rust）

```powershell
pnpm dev:web
```

- `http://localhost:1420/`：主窗口（设置页 + 首启引导），mock 后端，数据存 `localStorage`；
- `http://localhost:1420/?window=overlay`：冻结层 + 选框 + 动作菜单；
- `http://localhost:1420/?window=result`：结果窗（流式输出、来源、追问）。

### 桌面版（需要 Rust 工具链）

```powershell
pnpm dev        # tauri dev：Vite 1420 + Rust 后端
pnpm build      # tauri build：生成开发版（edition=dev）NSIS 安装包
```

### 构建 slim / full 双安装包

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1   # 准备 OCR 资产（SHA256 强校验）
powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1  # 构建两版 + 体积门禁 + 安装冒烟
```

产物归档到 `release/`：`SnipAsk_1.0.0_x64-setup.exe`（slim）、`SnipAsk_1.0.0_x64-full-setup.exe`（full）、
`SHA256SUMS.txt`；体积门禁 slim ≤25 MiB、full ≤60 MiB。详见[《自定义指南》§10](docs/自定义指南.md)。

### 检查与构建

```powershell
pnpm typecheck    # tsc --noEmit
pnpm test         # 前端纯逻辑单测（覆盖布局 25 + 线程模型 7 等）
pnpm build:web    # tsc --noEmit && vite build → dist/

# 一键校验（等价 CI：typecheck + 单测 + build:web + Rust fmt/clippy/test；
# 无 Rust 时跳过 Rust 检查并给出提示）
powershell -ExecutionPolicy Bypass -File scripts\verify.ps1
```

辅助脚本：

| 脚本 | 作用 |
| --- | --- |
| `scripts\dev.ps1` | 检测 Node/pnpm/Rust；有 Rust 启动 `pnpm dev`，无 Rust 提示改用 `pnpm dev:web` 并给出安装步骤 |
| `scripts\build.ps1` | `pnpm install` → `pnpm build:web`；有 Rust 时再构建安装包，否则说明跳过原因与产物位置 |
| `scripts\verify.ps1` | Web typecheck/单测/build + Rust fmt/clippy/test；输出 PASS/SKIP/FAIL 汇总，失败退出码 1 |
| `scripts\fetch-ocr-assets.ps1` | 准备 OCR 运行期资产到 `src-tauri/resources/ocr/`（本机缓存 → manifest 镜像，SHA256/大小强校验） |
| `scripts\build-installers.ps1` | 构建 slim/full 双安装包：edition 注入、体积门禁、安装/启动/卸载冒烟、归档与 SHA256 |
| `scripts\gen-icons.py` | 用 Pillow 重新生成 `src-tauri/icons/` 图标并校验尺寸/格式 |

---

## 仓库结构

```text
框选ai/
├── release/                  # 发行产物：slim/full NSIS 安装包、SHA256SUMS.txt、发行说明
├── docs/                     # 文档：自定义指南、实施架构、接口契约、开发指南、技术验证清单、ADR、spikes、README
├── resources/                # 编译内嵌资源：prompts/*.md、models.manifest.json（含 OCR 资产）、backends.manifest.json、pricing.json
├── scripts/                  # dev/build/verify/fetch-ocr-assets/build-installers.ps1 + gen-icons.py
├── .github/workflows/ci.yml  # CI：Web typecheck/单测/构建 + Rust fmt/clippy/test + NSIS 体积门禁（windows-latest）
├── index.html                # 单入口，按 ?window= 分发（main/overlay/result）
├── package.json / pnpm-lock.yaml
├── src/                      # React 18 + TS + Vite 前端（WebView2 内运行）
│   ├── app/                  # 窗口分发、错误兜底、IPC 事件 hook
│   ├── shared/               # contract（类型/命令名/默认值，schema v5）、ipc（tauri/mock）、theme、util
│   ├── styles/               # tokens.css（契约 §14）+ base.css
│   ├── components/ui/        # 通用展示组件
│   └── windows/              # main（设置/引导/OCR 状态/历史）、overlay（框选/菜单/覆盖翻译）、result（线程式结果窗）
└── src-tauri/                # Rust 后端（Tauri 2）
    ├── src/contract/         # 对外契约结构（错误/类型/配置/事件/常量，契约 v5）
    ├── src/app/              # 生命周期、AppState（frozen/history）、托盘、窗口、热键
    ├── src/commands/         # 43 个 Tauri 命令（薄层：校验 → service → 返回；含 history.rs）
    ├── src/domain/           # 纯逻辑（无 IO，可单测）
    ├── src/services/         # 能力编排（捕获/OCR/网关/搜索/翻译/历史/引擎/后端组件）
    ├── src/platform/         # 唯一允许 unsafe/Win32 的适配层（capture/credential/gpu 等）
    ├── src/events/           # 强类型事件发送助手（15 个事件）
    ├── src/resources/        # include_str! 内嵌仓库 resources/
    ├── migrations/           # SQLite Schema（001_init + 002_messages）
    ├── tauri.slim.conf.json / tauri.full.conf.json   # 双版本 bundle 资源布局
    └── icons/                # 应用图标（由 scripts/gen-icons.py 生成）
```

完整目录树、分层规则与模块映射见 [`docs/实施架构.md`](docs/实施架构.md) §2–§6。

---

## 工程约定

1. **契约优先**：`docs/接口契约.md` 是前后端边界的唯一事实来源（**契约 v5**）。新增/修改命令或事件必须先改契约，
   再同步 `src/shared/contract/*` 与 `src-tauri/src/contract/*`；**破坏性**变更递增 `IPC_CONTRACT_VERSION`
   （当前 `1`），纯增量只更新修订记录与配置 Schema 版本（当前 **v5**）。流程见
   [`docs/开发指南.md`](docs/开发指南.md)「契约变更流程」。
2. **分层依赖单向**：`commands → services → domain`，`platform` 只被 `services` 调用，
   `contract` 不依赖任何内部模块；`domain` 不得出现 Tauri、文件/网络 IO、`std::process`。
3. **错误模型统一**：命令一律返回 `Result<T, AppError>`，`AppError { code, message, detail, retryable, fallback[], context }`；
   错误码见契约 §8.2，前端按 `fallback` 提供降级按钮，`SA-1099` 静默处理。
4. **无密钥穿越边界**：任何命令入参/出参、事件载荷不得出现明文 API Key；`ConfigView` 只回掩码与 `hasKey`；
   密钥仅存 Windows 凭据管理器，日志禁止记录密钥、OCR 原文与完整查询串。
5. **坐标全程物理像素**：`rectPhys` 以虚拟桌面左上角为原点（可为负），前端不得换算后回传逻辑坐标。
6. **骨架模式默认关闭**：`advanced.skeletonMode = false`（v1.0.0 默认走真实能力）；手动置 `true` 时外部能力
   走确定性占位实现，仅用于联调/排查（契约 §12）。
7. **不自动更新**：无 updater 依赖/配置/事件；升级 = 覆盖安装或源码构建（[ADR-0008](docs/adr/0008-remove-auto-update.md)）。
8. **代码风格**：Rust 用 `cargo fmt` + `cargo clippy -D warnings`，TS 严格模式 + Prettier；
   单文件 ≤ 300 行，单一职责，文件命名见开发指南「代码风格」。

---

## 文档索引

| 文档 | 说明 |
| --- | --- |
| [`docs/README.md`](docs/README.md) | **文档索引**（安装包内随附） |
| [`docs/自定义指南.md`](docs/自定义指南.md) | **核心使用文档**：不自动更新与升级、提示词覆盖、术语表、配置 v5 速查、模型/后端替换、OpenAI 兼容接入、双版本差异、源码构建 |
| [`docs/可行性研究报告.md`](docs/可行性研究报告.md) | 需求基线、功能清单、技术选型、成本、合规、验收标准（历史基线） |
| [`docs/概要设计说明书.md`](docs/概要设计说明书.md) | 总体设计、模块 M01–M20、关键流程、数据设计、性能与安全（历史基线） |
| [`docs/实施架构.md`](docs/实施架构.md) | 代码结构、分层规则、运行时形态、模块映射、数据与资源、实现边界与遗留 |
| [`docs/接口契约.md`](docs/接口契约.md) | **IPC 契约 v5**：43 命令 / 15 事件 / 配置 Schema v5 / 错误码 / 骨架模式 / 主题 token |
| [`docs/开发指南.md`](docs/开发指南.md) | 环境准备、常用命令、契约变更流程、调试、**GPU/Vulkan**、**OCR/捕获调试钩子**、**双安装包构建**、常见问题 |
| [`docs/技术验证清单.md`](docs/技术验证清单.md) | spike 任务表（S-01～S-34）与结论状态 |
| [`docs/追加需求可行性研究.md`](docs/追加需求可行性研究.md) | 追加需求（本地 GPU 推理 / 覆盖式翻译 / 连续提问）可行性、实测证据与工作量 |
| [`docs/adr/`](docs/adr/) | ADR-0001～0009（含 [双安装包](docs/adr/0007-dual-installer-editions.md)、[移除自动更新](docs/adr/0008-remove-auto-update.md)、[真实捕获与 OCR](docs/adr/0009-real-capture-ocr.md)） |
| [`docs/spikes/`](docs/spikes/README.md) | 技术验证（spike）逐项记录与原始数据索引 |
| [`release/README.md`](release/README.md) | 发行产物说明：双安装包、SHA256、构建信息、安装/卸载与已知限制 |
| [`CHANGELOG.md`](CHANGELOG.md) | 版本变更记录 |
| [`SECURITY.md`](SECURITY.md) / [`PRIVACY.md`](PRIVACY.md) | 安全策略 / 隐私声明 |
| [`LICENSE`](LICENSE) / [`NOTICE`](NOTICE) | Apache-2.0 许可全文 / 第三方组件与模型许可声明 |

---

## 验收与性能目标

摘自《可行性研究报告》§2.3 与 §7.2（目标值；实测口径与证据见[技术验证清单](docs/技术验证清单.md)与 spike 记录）：

| 类别 | 指标 |
| --- | --- |
| 启动与常驻 | 冷启动 ≤ 1.5s；空闲内存 ≤ 120MB；空闲 CPU < 0.5%；托盘常驻 + 单实例 |
| 交互 | 热键 → 选框 ≤ 250ms；动作菜单 ≤ 100ms；多屏混合 DPI 框选坐标误差 ≤ 2px |
| OCR | 整屏文字检测 ≤ 1.2s（4K 双屏，后台不阻塞选框）；选区 OCR ≤ 800ms；字符准确率 ≥ 95% |
| AI 时延 | 快速模式首字 ≤ 1.5s；AI 搜索首字 ≤ 3s；图片搜索快速模式 ≤ 10s、深度 ≤ 25s |
| 翻译 | 云端 200 字 ≤ 1.5s（首字）；本地 1.8B（Q4，CPU）≤ 8s（实测不达标，作为可选质量档并提示）；本地 1.8B GPU P50 1.26s / P95 3.75s（RTX 4060 Laptop） |
| 本地 GPU | Vulkan 真实推理可用（`--list-devices` 见 `Vulkan0`）；**Vulkan 优先 / CUDA 可选 / CPU 回退**；AMD/Intel 走同一 Vulkan 路径（**AMD 真机未测**） |
| 覆盖翻译 | 100 图真实语料：2288 行 → 1552 段块（−32.2%）；推荐配置溢出 0%；字号中位 0.80×源行高 |
| 连续提问 | 任务完成后追问不再必然 `SA-1099`；上下文含上一轮回答；线程式结果窗 |
| 历史 | 成功任务持久化到 `messages` 表；重启后可列出会话与完整线程；可关闭/可清空 |
| 安装包 | slim ≤ 25 MiB（不含 OCR 模型）；full ≤ 60 MiB（含 OCR 模型）；GPU 后端按需下载 |
| 引导 | 首启 ≤ 3 分钟完成最小配置；无网络/无 Key 可跳过并进入受限模式；下载支持断点续传 + SHA256 |

**v1.0.0 验证摘要（真实应用 / 真实模型 / 真实网络，2026-11-05/06）**：

- **捕获/OCR**：S-01 混合 DPI 跨屏误差 0.01px；S-02 整屏 det P95 743–794ms（独立基准）；S-03 选区 rec-only P95 596ms；
  S-09 BitBlt 与 WGC 像素级一致、DRM 黑帧降级；S-11 文字存在判定误判 2.0%/漏判 0%；
  S-15 PP-OCRv4 字符准确率 99.38%（native）/ 99.29%（生产预算口径），100/100 张 ≥95%。
  **v1.0.0 真实引擎证据**（`.verify/v1/ocr/`）：7 个真实用例（5 张渲染截图 + 真实桌面面板 + 真实整虚拟桌面），
  2890 字符字符准确率 **98.13%**（≥95% 达标；分用例最低代码页 95.75%，真实桌面面板 100%，整虚拟桌面已知行召回 7/8）；
  整屏 det 0.84–1.18s（`cargo test` 实测口径，含 Rust 前后处理）。
- **云端/本地**：S-05 DeepSeek 关闭 thinking 首字 P95 0.87s；S-06 图片搜索 quick P95 6.01s / deep P95 8.29s
  （各 20 次 0 失败）；S-21～S-24 GPU 探测/真实 Vulkan 推理（90ms vs CPU 482ms）/回退链/组件下载；
  S-27 连续提问回归 9/9。
- **工程**：S-10 release 冷启动 P95 858ms、空闲 CPU 0.0022%；S-16 未签名安装/启动/卸载通过、Defender 无威胁；
  S-18 体积门禁 CI 化；S-20 单实例/热键释放通过。
- **app 级真实 E2E（cloud/local 两阶段 `ok=true`，已验收，2026-11-06）**：
  cloud：真实捕获 → 真实选区 OCR（`hasText=true`、5 行）→ 翻译 1.47s / AI 搜索 2.89s /
  图片搜索 10.04s（deepseek 真实 HTTP）→ `translate_cover` 全部块成功（4.33s）→ `history_list`/`history_thread`（3 轮）；
  local：Vulkan 引擎 ready（offload 29 层 / RTX 4060 Laptop / 1.5B Q4）→ 本地翻译 0.75s，但**译文含 1 行提示词残留**
  （S-14 本地小模型格式问题，质量不达标）；证据 `.verify/v1/e2e/`（本地）。
- **遗留**：AMD/Intel 真机未测、无代码签名、DashScope 真实翻译未验收、8GB 降档提示缺失、S-19 真人走查 BLOCKED；
  S-12 来源可访问率 59.6%、S-14 本地格式保持 68.1% 为已知质量缺口。
- **v1.0.0 最终门禁（全绿）**：`cargo fmt --check` PASS；`cargo clippy --all-targets -- -D warnings` PASS（0 warning）；
  `cargo test --all-targets` **479 passed / 0 failed / 20 ignored**；`pnpm typecheck` 0、`pnpm test` **48/48**、
  `pnpm build:web` 0。双安装包已归档并冒烟 PASS（slim 7.41 MiB / full 21.52 MiB，SHA256 见
  `release/SHA256SUMS.txt`）；完整矩阵见本地证据 `.verify/verify-v1.0.0/SUMMARY.md`（gitignored）。

逐项记录与原始数据索引见 [`docs/spikes/`](docs/spikes/README.md)，复跑脚本见 [`scripts/spikes/`](scripts/spikes/README.md)。

---

## 许可与隐私

- **许可**：[Apache-2.0](LICENSE)。第三方组件、ONNX Runtime、OCR 模型与本地推理依赖的许可见 [NOTICE](NOTICE)；
  OCR 模型与本地翻译模型由安装包内置或按需下载，其许可见模型卡条款。
- **BYOK**：不提供服务端、不代理用户请求、不收集遥测；API Key 仅存 Windows 凭据管理器。
- **隐私**：默认不上传截图；只有用户主动触发云端动作（AI 搜索 / 图片搜索 / 翻译 / 解答）时才发送
  对应文本或图像，且可在设置中分别关闭上传开关；`privacy.localOnly` 可禁用全部云端动作。
- **数据目录**：配置、提示词、日志、历史库在 `%APPDATA%\SnipAsk\`，模型/OCR 模型/缓存在
  `%LOCALAPPDATA%\SnipAsk\`；便携版（程序目录存在 `portable.flag`）时写入程序目录旁 `data\`。
  详见[实施架构 §7](docs/实施架构.md) 与[自定义指南](docs/自定义指南.md)。
- **无自动更新**：不检查更新、不联网查询版本；升级方式见[升级与自定义](#升级与自定义)。
