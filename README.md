# 框选 AI（Snip Ask）

> 轻量快速、托盘常驻、可开机自启、高度自定义的 Windows 框选解答 / 翻译 / 搜索工具。
> 按 `Ctrl+Q` 冻结屏幕 → 框选任意区域 → 搜索 / 翻译 / 解答，结果以流式输出秒级呈现。

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2B%20x64-0078D4.svg)
![Release](https://img.shields.io/badge/release-v1.0.4-2ea44f.svg)
![Status](https://img.shields.io/badge/status-stable-brightgreen.svg)

**框选 AI** 是 BYOK（Bring Your Own Key）的桌面工具：没有服务端、不收集截图、**不收集任何用户信息**
（无遥测、无崩溃上报、无统计）；用户自备 AI 服务密钥，截图与文本只在用户主动触发云端动作时才发送。
开源协议为 [Apache-2.0](LICENSE)。

本仓库为 **v1.0.4**：在本机完成屏幕捕获与文字识别，AI 推理可走本地 llama.cpp，也可用 DeepSeek
（对话 / 搜索 / 抓取）、DashScope Qwen-MT 或任意 OpenAI 兼容服务；API Key 存在 Windows 凭据管理器，
历史消息保存在本机。提供 **内置 OCR 模型版 / 在线安装版** 两个安装包（[`release/`](release/README.md)）与
**[《自定义指南》](docs/自定义指南.md)**。**本软件不自动更新**（见[升级方式](#升级与自定义)）。

> **能力边界（如实标注）**：屏幕捕获、文字识别、凭据与历史均在本机完成；**AMD / Intel 真机 GPU 未测**
> （Vulkan 路径按「无 NVIDIA 专属依赖」论证）、**安装包未做代码签名**（SmartScreen 提示属预期）、
> **本机无 DashScope Key**（Qwen-MT 仅协议级验收，真实翻译未验收）。逐项证据与遗留见
> 本地证据 `.verify/verify-v1.0.4/SUMMARY.md`（gitignored）与
> [`docs/技术验证清单.md`](docs/技术验证清单.md)。

---

## 下载与安装

### 发行产物（双版本）

| 产物 | 说明 | 体积 / 门禁 |
| --- | --- | --- |
| [`release/SnipAsk_1.0.4_x64-setup.exe`](release/SnipAsk_1.0.4_x64-setup.exe) | **在线安装版（在线下载 OCR 模型）**：内置应用 + 文字识别运行时 + `docs/`；首次使用文字识别时下载 4 个模型/字典（约 16 MB） | **7,784,602 B（7.42 MiB）** / ≤ 25 MiB |
| [`release/SnipAsk_1.0.4_x64-full-setup.exe`](release/SnipAsk_1.0.4_x64-full-setup.exe) | **内置 OCR 模型版**：在线安装版的全部内容 + 文字识别模型/字典，离线开箱即用 | **22,567,768 B（21.52 MiB）** / ≤ 60 MiB |
| [`release/SHA256SUMS.txt`](release/SHA256SUMS.txt) | 两个安装包的 SHA256 校验和 | — |
| [`release/README.md`](release/README.md) | 发行说明：逐包体积/SHA256、构建信息、安装/卸载、已知限制 | — |

> 两版功能完全一致，差异只在「文字识别模型是否随包」。选择建议：
> 能联网/想要更小安装包 → **在线安装版**；离线/内网/不想首次下载模型 → **内置 OCR 模型版**。
> 体积为 v1.0.4 最终归档产物实测（2026-11-06）；SHA256 以 `release/SHA256SUMS.txt` 为准。

### 安装步骤

1. 下载安装包，先校验完整性：

   ```powershell
   Get-FileHash .\SnipAsk_1.0.4_x64-setup.exe -Algorithm SHA256
   # 与 release\SHA256SUMS.txt 中对应条目比对
   ```

2. 双击运行安装包；或静默安装（当前用户）：
   `Start-Process .\SnipAsk_1.0.4_x64-setup.exe -ArgumentList '/S'`。
3. 安装完成后启动 **SnipAsk**；首次启动进入 6 步引导（无网络 / 无 API Key 可跳过并进入受限模式）。
4. 按 `Ctrl+Q` 开始框选；托盘左键可直接打开设置，右键菜单可开始框选、重新运行引导、暂停热键或退出。

> **未签名提示**：v1.0.4 安装包**未做代码签名**。首次运行时 Windows SmartScreen 可能显示
> 「Windows 已保护你的电脑」，点击「更多信息 → 仍要运行」即可。已在 Windows 11 + Defender 实机验证
> 安装/启动/卸载且 Defender 扫描无威胁（[S-16 记录](docs/spikes/S-16-无签名安装包与杀软误报.md)）；
> Win10 干净机、火绒等环境未覆盖。

### 窗口与托盘操作（v1.0.1–v1.0.4）

- 主窗口**没有 Windows 系统标题栏**，使用应用内自绘标题栏：拖拽移动、最小化、最大化/还原、
  关闭按钮，以及四边与四角的八向缩放手柄；
- **点关闭按钮或按 `Alt+F4` 不会退出应用**：窗口隐藏到托盘，框选热键继续可用。要彻底退出请用托盘菜单「退出」；
- **托盘左键单击直接打开设置**（右键才显示菜单）；「重新运行引导」随时可用，引导完成后左侧不再显示引导入口；
- 设置页分组：核心 / 外观 / AI 服务 / **搜索** / 翻译 / **隐私** / 动作与热键 / 高级 / 自定义与关于。

### 卸载与数据目录

- **卸载**：设置 → 应用 → 已安装的应用 → **SnipAsk** → 卸载；或运行
  `%LOCALAPPDATA%\SnipAsk\uninstall.exe`。**v1.0.4 起卸载会询问是否同时删除本地数据**：
  勾选「删除应用数据」/ 静默 `/S` / 末尾确认框选「是」→ 删除配置、历史、日志、提示词、已下载模型与后端；
  选「否」→ 仅删程序、保留数据（覆盖安装不会删除任何用户数据）。
- **数据位置**：

  | 内容 | 路径 |
  | --- | --- |
  | 配置 / 提示词 / 日志 / 历史库 / 本地后端 | `%APPDATA%\SnipAsk\` |
  | 本地模型 / OCR 模型 / 缓存 / 运行时 | `%LOCALAPPDATA%\SnipAsk\`（安装目录同名；v1.0.4 在删除数据时一并清理） |
  | WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` |

  若卸载时选择保留数据，可之后手动删除上述目录（并在 Windows 凭据管理器删除 `SnipAsk/` 条目）。
  便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`。

---

## 升级与自定义

**本软件不自动更新**：没有更新检查、不联网查询版本、没有更新通道设置。
**不收集任何用户信息**：无遥测、无崩溃上报、无使用统计（v1.0.1 已删除配置中的遥测字段）。
升级方式：

1. 下载新版安装包**覆盖安装**（保留配置、提示词、历史与凭据；v1.0.0 及更早的配置会在首次启动时自动迁移到新版）；
2. 或按[《自定义指南》](docs/自定义指南.md) §10 从源码构建你自己的版本（Apache-2.0 允许修改分发）。

**自定义**（完整说明见 [`docs/自定义指南.md`](docs/自定义指南.md)）：

- 提示词覆盖：`%APPDATA%\SnipAsk\prompts\*.md`（7 个模板，用户 > 内置；v1.0.4 新增本地术语表分支模板）；
- 术语表、目标语言、搜索方式、隐私开关、热键、动作菜单、默认模式：设置页或 `config.json`（schema v6）；
- 本地模型 / 后端替换：`SNIP_ASK_MODEL_PATH`、`SNIP_ASK_BACKENDS_DIR` 等调试钩子；
- 接本地 Ollama / llama-server / 自建 OpenAI 兼容网关：新增供应商（`providers[].kind`）；
- 界面内入口：设置 → **自定义与关于**（版本 / 安装包类型「已内置文字识别模型 / 首次使用时下载」/ 文字识别就绪
  与目录入口；一键打开配置、提示词、模型、日志、后端、安装文档目录 / 无自动更新说明）。

---

## 核心交互

按 `Ctrl+Q` 冻结屏幕 → 拖拽框选目标区域（支持方向键微调、回车确认、`Esc` 取消）→
在动作菜单中选择 **搜索 / 翻译 / 解答 / 复制**，结果在结果窗中流式呈现，可追问、重答、复制、关闭。

- **屏幕捕获（BitBlt）**：冻结整个虚拟桌面（物理像素、支持负坐标与混合 DPI）；受保护内容（DRM/全屏独占）
  会明确提示无法截取，不做绕过。
- **本机文字识别（PP-OCRv4）**：整屏后台检测（有像素预算，不阻塞选框）+ 选区内识别；
  「有文字」时提供文本搜索、翻译、解答、复制；「无文字/图片」时提供图片搜索、看图解答、翻译。
- **云端与本地推理**：DeepSeek（对话 / 图片搜索 / Search / Fetch）、DashScope Qwen-MT 专用翻译、
  通用 OpenAI 兼容供应商；本地 llama.cpp（**Vulkan 优先 / CUDA 可选 / CPU 回退**）。
- **覆盖式翻译**：翻译结果**原位覆盖**冻结帧上的原文（模糊原区域 + 按版式重绘译文），
  可切「原文 / 双语 / 覆盖」；只在框选画面上显示，不改动目标应用。
- **连续提问与历史**：结果窗多轮追问；成功任务写入本机历史库，重启后可在主窗口浏览历史会话与线程。
- **BYOK 与隐私**：API Key 只存 Windows Credential Manager，界面仅掩码；上传文本/截图可分别关闭，
  「仅本机处理」可禁用全部云端动作。

---

## 当前状态

| 层次 | 内容 | 状态 |
| --- | --- | --- |
| 域逻辑 | 会话状态机、选区/坐标换算、文字存在判定、动作注册表、费用估算、多轮线程（纯逻辑 + 单测） | ✅ 已实现 |
| IPC 契约 | **43 命令 / 15 事件（契约 v6）** / `AppError` 错误码表 / 配置 Schema **v6**，TS 与 Rust 双端同构 | ✅ 已实现 |
| 配置中心 | 默认值、深度合并、校验、迁移（**1→2→3→4→5→6**）、原子写 + `.bak`、`config:changed` 热生效分类 | ✅ 已实现 |
| 前端窗口 | `main`（自绘标题栏、设置页、自定义与关于、历史、6 步引导）、`overlay`（冻结层 + 选框 + 放大镜 + 动作菜单 + 覆盖翻译层）、`result`（线程式流式输出 + 来源 + 追问） | ✅ 已实现 |
| **屏幕捕获** | Win32 BitBlt 虚拟桌面捕获（BGRA + PNG）、黑屏/空帧判定与降级提示（[ADR-0009](docs/adr/0009-real-capture-ocr.md)） | ✅ 已实现（本机真机测试） |
| **本机文字识别** | RapidOCR PP-OCRv4（det/rec/cls + 字典）、ONNX Runtime（`ort` load-dynamic）、像素预算、整屏 det 缓存 + 选区 rec-only | ✅ 已实现（字符准确率 S-15 实测 99.29–99.38%） |
| **云端推理/搜索** | DeepSeek 对话/视觉流式、`tools/search` + `tools/fetch`、图片搜索四阶段、Qwen-MT、通用 OpenAI 兼容 | ✅ 已实现（Qwen-MT 真实翻译因本机无 Key 未验收） |
| **本地 GPU 推理** | GPU 探测、显存预算、组件按需下载、引擎监督者（Vulkan → CUDA → CPU 回退） | ✅ 已实现（NVIDIA RTX 4060 Laptop 真机；**AMD/Intel 真机未测**） |
| **凭据** | Windows Credential Manager（keyring，service `SnipAsk`），界面仅掩码 | ✅ 已实现 |
| **历史持久化** | `messages` 表、`history_list` / `history_thread` / `history_clear`，重启后可回溯 | ✅ 已实现 |
| **双安装包** | 在线安装版（在线下载 OCR 模型，≤25 MiB）/ 内置 OCR 模型版（离线可用，≤60 MiB），安装包类型在「自定义与关于」标注 | ✅ v1.0.4 已归档（7.42 / 21.52 MiB） |
| **v1.0.1 修复批** | 自绘窗口/关闭到托盘/托盘左键设置、引导入口隐藏、搜索与隐私独立分组、四个动作默认模式、模型下载 idle/取消、内存真实值、引擎启停、去遥测、用户文案重写 | ✅ 已实现 |
| **v1.0.2 修复批** | 结果窗不透明铺满 + 高度自适应（到屏高 85% 后内部滚动）、引擎就绪后清空旧「模型缺失」提示、自绘标题栏最小化/最大化/8 向缩放权限补齐 | ✅ 已实现 |
| **v1.0.3 修复** | 本地 HY-MT1.5-1.8B 翻译改用精简提示词（`LOCAL_MODELS_MINIMAL_PROMPT`），不再复述原文；新增单测 | ✅ 已实现 |
| **v1.0.4 修复** | 本地翻译语言码→中文语言名 + 术语表分支 + 复述检测/自动降级（不再回显原文）；NSIS 卸载钩子彻底清理模型/配置/后端（静默/勾选/确认，更新保留数据） | ✅ 已实现并真机验证 |
| **自动更新** | updater 插件/服务、`advanced.updateChannel`、`update:status` 全部移除（[ADR-0008](docs/adr/0008-remove-auto-update.md)） | ✅ 已移除（升级 = 覆盖安装） |
| 工程底座 | 分层目录、内置资源、图标、pnpm 脚本、`scripts/*.ps1`、CI（windows-latest）、ADR-0001～0009 | ✅ 已实现 |
| **覆盖式翻译** | 段块合并 + 自适应布局纯函数 / 冻结帧 Canvas 三态 / `translate_cover` 批处理 + 会话级 LRU | ✅ 已实现（布局单测 25/25；真实 OCR/翻译质量依赖真实链路） |
| **连续提问** | 上下文注入 / `task:started.parent` / 失败可继续追问 / 线程式结果窗 | ✅ 已实现（回归 9/9 + 线程 UI 7/7） |

图例：✅ 已实现/已交付。逐模块（M01–M20）映射见 [`docs/实施架构.md` §6](docs/实施架构.md)。
**已知限制**：AMD/Intel 真机 GPU 未测、无代码签名、DashScope 真实翻译未验收（本机无 Key）、
本地小模型译文质量/格式保持仍弱于云端（但 v1.0.4 起不再回显原文，并会自动降级/报错）、
8GB 机型降档提示缺失、Win10/国产杀软/真人可用性（S-19）未覆盖、CI 未在 GitHub runner 实跑、
关闭窗口 = 托盘常驻（v1.0.1 新设计）、Markdown 渲染/多语言待打磨。
完整遗留见 [`docs/实施架构.md` §9](docs/实施架构.md) 与本地证据 `.verify/verify-v1.0.4/SUMMARY.md`。

---

## 快速开始

### 终端用户（直接安装）

无需配置开发环境：从 [`release/`](release/README.md) 取得 **在线安装版** 或 **内置 OCR 模型版** → 校验 `SHA256SUMS.txt`
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

### 构建双安装包（在线安装版 / 内置 OCR 模型版）

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1   # 准备 OCR 资产（SHA256 强校验）
powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1  # 构建两版 + 体积门禁 + 安装冒烟
```

产物归档到 `release/`：`SnipAsk_1.0.4_x64-setup.exe`（在线安装版）、
`SnipAsk_1.0.4_x64-full-setup.exe`（内置 OCR 模型版）、`SHA256SUMS.txt`；
体积门禁：在线安装版 ≤25 MiB、内置 OCR 模型版 ≤60 MiB。详见[《自定义指南》§10](docs/自定义指南.md)。

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
| `scripts\build-installers.ps1` | 构建在线安装版 / 内置 OCR 模型版双安装包：edition 注入、体积门禁、安装/启动/卸载冒烟、归档与 SHA256 |
| `scripts\gen-icons.py` | 用 Pillow 重新生成 `src-tauri/icons/` 图标并校验尺寸/格式 |

---

## 仓库结构

```text
框选ai/
├── release/                  # 发行产物：在线安装版 / 内置 OCR 模型版 NSIS 安装包、SHA256SUMS.txt、发行说明
├── docs/                     # 文档：自定义指南、实施架构、接口契约、开发指南、技术验证清单、ADR、spikes、README
├── resources/                # 编译内嵌资源：prompts/*.md、models.manifest.json（含 OCR 资产）、backends.manifest.json、pricing.json
├── scripts/                  # dev/build/verify/fetch-ocr-assets/build-installers.ps1 + gen-icons.py
├── .github/workflows/ci.yml  # CI：Web typecheck/单测/构建 + Rust fmt/clippy/test + NSIS 体积门禁（windows-latest）
├── index.html                # 单入口，按 ?window= 分发（main/overlay/result）
├── package.json / pnpm-lock.yaml
├── src/                      # React 18 + TS + Vite 前端（WebView2 内运行）
│   ├── app/                  # 窗口分发、错误兜底、IPC 事件 hook
│   ├── shared/               # contract（类型/命令名/默认值，schema v6）、ipc（tauri/mock）、theme、util
│   ├── styles/               # tokens.css（契约 §14）+ base.css
│   ├── components/ui/        # 通用展示组件
│   └── windows/              # main（自绘标题栏、设置/引导/OCR 状态/历史）、overlay（框选/菜单/覆盖翻译）、result（线程式结果窗）
└── src-tauri/                # Rust 后端（Tauri 2）
    ├── src/contract/         # 对外契约结构（错误/类型/配置/事件/常量，契约 v6）
    ├── src/app/              # 生命周期、AppState（frozen/history）、托盘、窗口、热键
    ├── src/commands/         # 43 个 Tauri 命令（薄层：校验 → service → 返回；含 history.rs）
    ├── src/domain/           # 纯逻辑（无 IO，可单测）
    ├── src/services/         # 能力编排（捕获/OCR/网关/搜索/翻译/历史/引擎/后端组件）
    ├── src/platform/         # 唯一允许 unsafe/Win32 的适配层（capture/credential/gpu/http 等）
    ├── src/events/           # 强类型事件发送助手（15 个事件）
    ├── src/resources/        # include_str! 内嵌仓库 resources/
    ├── migrations/           # SQLite Schema（001_init + 002_messages）
    ├── tauri.slim.conf.json / tauri.full.conf.json   # 双版本 bundle 资源布局
    └── icons/                # 应用图标（由 scripts/gen-icons.py 生成）
```

完整目录树、分层规则与模块映射见 [`docs/实施架构.md`](docs/实施架构.md) §2–§6。

---

## 工程约定

1. **契约优先**：`docs/接口契约.md` 是前后端边界的唯一事实来源（**契约 v6**）。新增/修改命令或事件必须先改契约，
   再同步 `src/shared/contract/*` 与 `src-tauri/src/contract/*`；**破坏性**变更递增 `IPC_CONTRACT_VERSION`
   （当前 `1`），纯增量只更新修订记录与配置 Schema 版本（当前 **v6**）。流程见
   [`docs/开发指南.md`](docs/开发指南.md)「契约变更流程」。
2. **分层依赖单向**：`commands → services → domain`，`platform` 只被 `services` 调用，
   `contract` 不依赖任何内部模块；`domain` 不得出现 Tauri、文件/网络 IO、`std::process`。
3. **错误模型统一**：命令一律返回 `Result<T, AppError>`，`AppError { code, message, detail, retryable, fallback[], context }`；
   错误码见契约 §8.2，前端按 `fallback` 提供降级按钮，`SA-1099` 静默处理。
4. **无密钥穿越边界**：任何命令入参/出参、事件载荷不得出现明文 API Key；`ConfigView` 只回掩码与 `hasKey`；
   密钥仅存 Windows 凭据管理器，日志禁止记录密钥、OCR 原文与完整查询串。
5. **坐标全程物理像素**：`rectPhys` 以虚拟桌面左上角为原点（可为负），前端不得换算后回传逻辑坐标。
6. **开发者配置**：`advanced.skeletonMode` 默认 `false`，v1.0.1 起**不在用户界面出现**；只有开发调试时才手动
   在配置文件中开启（开启后外部能力走确定性占位实现，契约 §12）。
7. **不自动更新、不收集用户信息**：无 updater 依赖/配置/事件（[ADR-0008](docs/adr/0008-remove-auto-update.md)）；
   无遥测、无崩溃上报、无使用统计（v1.0.1 删除 `advanced.telemetry`），升级 = 覆盖安装或源码构建。
8. **代码风格**：Rust 用 `cargo fmt` + `cargo clippy -D warnings`，TS 严格模式 + Prettier；
   单文件 ≤ 300 行，单一职责，文件命名见开发指南「代码风格」。

---

## 文档索引

| 文档 | 说明 |
| --- | --- |
| [`docs/README.md`](docs/README.md) | **文档索引**（安装包内随附） |
| [`docs/自定义指南.md`](docs/自定义指南.md) | **核心使用文档**：不自动更新与升级、提示词覆盖、术语表、配置 v6 速查、模型/后端替换、OpenAI 兼容接入、双版本差异、源码构建 |
| [`docs/可行性研究报告.md`](docs/可行性研究报告.md) | 需求基线、功能清单、技术选型、成本、合规、验收标准（历史基线） |
| [`docs/概要设计说明书.md`](docs/概要设计说明书.md) | 总体设计、模块 M01–M20、关键流程、数据设计、性能与安全（历史基线） |
| [`docs/实施架构.md`](docs/实施架构.md) | 代码结构、分层规则、运行时形态、模块映射、数据与资源、实现边界与遗留 |
| [`docs/接口契约.md`](docs/接口契约.md) | **IPC 契约 v6**：43 命令 / 15 事件 / 配置 Schema v6 / 错误码 / 开发者配置 / 主题 token |
| [`docs/开发指南.md`](docs/开发指南.md) | 环境准备、常用命令、契约变更流程、调试、**GPU/Vulkan**、**OCR/捕获调试钩子**、**双安装包构建**、常见问题 |
| [`docs/技术验证清单.md`](docs/技术验证清单.md) | spike 任务表（S-01～S-36+）与结论状态 |
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
| 安装包 | 在线安装版 ≤ 25 MiB（不含文字识别模型）；内置 OCR 模型版 ≤ 60 MiB（含模型）；GPU 后端按需下载 |
| 引导 | 首启 ≤ 3 分钟完成最小配置；无网络/无 Key 可跳过并进入受限模式；下载支持断点续传 + SHA256 |

**v1.0.4（本版本）修复摘要**：

- **本地翻译「翻译后仍然为原文」**：根因是本地精简提示词把 BCP-47 语言码（`zh-CHS` 等）直接发给本地小模型，
  真实模型不认识语言码而原样复述原文；非空结果又被当作成功、不降级不报错。修复：本地精简提示词与覆盖翻译提示词
  渲染中文语言名（`简体中文`）；术语表非空走 `translate_minimal_glossary.md`；新增复述检测（仅「与原文完全相同
  且不含目标语言文字」才判 `SA-1099`），命中后自动降级云端/返回可操作错误；本地翻译任务改为缓冲校验后再推送前端。
  真实 App 命令路径（qwen2.5-0.5b + `zh-CHS`）输出 `那只敏捷的棕色狐狸跳过了那只懒惰的狗。`（788ms，
  `sameAsInput=false`）；三模型提示词矩阵 9/9 不再回显原文。
- **卸载无法彻底卸载（含已下载模型）**：Tauri NSIS 内置「删除应用数据」只删 `com.snipask.app`，不清
  `%APPDATA%\SnipAsk` 与 `%LOCALAPPDATA%\SnipAsk`（模型/后端）→ 残留。修复：新增
  `src-tauri/nsis/installer-hooks.nsh`（`bundle.windows.nsis.installerHooks`），静默 `/S` 彻底清理、
  勾选「删除应用数据」清理、交互未勾选时再次确认（默认「是」）；更新 `/UPDATE` 保留数据；
  安装冒烟以「数据目录 + 模型探针」残留断言作为 pass 门禁（`dataRemoved=true`、`dataResidue=[]`）。
- 无契约变更（配置 Schema 6 / 43 命令 / 15 事件 / `IPC_CONTRACT_VERSION=1` 不变）；双安装包用 Tauri 2.12.4
  重建并归档（`SnipAsk_1.0.4_x64-setup.exe` / `SnipAsk_1.0.4_x64-full-setup.exe`，**7.42 MiB / 21.52 MiB**，
  门禁 PASS，安装/启动/关闭隐藏到托盘/卸载 + 数据清理冒烟 PASS）；SHA256 以 `release/SHA256SUMS.txt` 为准；
- **v1.0.4 最终门禁（全绿）**：`cargo fmt --check` 0；`cargo clippy --all-targets -- -D warnings` 0 warning；
  `cargo test --all-targets` **495 passed / 0 failed / 20 ignored**（lib 356 含新增 11 个）；
  `pnpm typecheck` 0、`pnpm test` **48/48**、`pnpm build:web` 0；证据见本地
  `.verify/verify-v1.0.4/SUMMARY.md`（gitignored）。

**v1.0.3（前置版本）修复摘要**：

- **本地 HY-MT 翻译复述原文**：本地模型 `hy-mt-1.8b-q4` 之前走产品长提示词 `translate.md`，该 1.8B 翻译模型
  直接复述原文且被当作成功（不降级云端）。现将 HY-MT 与 Qwen2.5 0.5B/1.5B 一起归入精简提示词名单
  （`LOCAL_MODELS_MINIMAL_PROMPT`），改用 `translate_minimal.md`；云端 DeepSeek 不受影响；
- **真实 App 路径验证**：`translate.backend=local` + HY-MT（Vulkan，offload 33 层）→ `translate_test`
  两条输入均输出中文译文（**103ms / 166ms**，非复述原文）；修复前同一长提示词下输出英文原文；
- 补单元测试：本地 HY-MT → `translate_minimal`、Qwen2.5 → `translate_minimal`、Qwen3-4B/云端 → `translate`；
- 无契约变更（配置 Schema 6 / 43 命令 / 15 事件不变）；双安装包由 T5 重建并归档
  （`SnipAsk_1.0.3_x64-setup.exe` / `SnipAsk_1.0.3_x64-full-setup.exe`，**7.42 MiB / 21.52 MiB**，
  门禁 PASS，安装/启动/关闭隐藏到托盘/卸载冒烟 PASS）；SHA256 以 `release/SHA256SUMS.txt` 为准；
- **v1.0.3 最终门禁（全绿）**：`cargo fmt --check` 0；`cargo clippy --all-targets -- -D warnings` 0 warning；
  `cargo test --all-targets` **484 passed / 0 failed / 20 ignored**（lib 350 含新增 1 个）；
  `pnpm typecheck` 0、`pnpm test` **48/48**、`pnpm build:web` 0；证据见本地
  `.verify/verify-v1.0.3/SUMMARY.md`（gitignored）。

**v1.0.2（前置版本）修复摘要**：

- **结果窗**：改为不透明面板并铺满窗口，高度随回答内容自适应（内容少时窗口收紧；内容多时到屏幕 85% 后
  内部滚动），去掉窗口外圈透明边、系统阴影与固定 420px 高度造成的下方空白；
- **引擎提示**：模型安装、引擎就绪后自动清空旧的「本地模型缺失」提示，仅未启动时展示可读原因；
- **自绘标题栏**：补齐窗口权限，最小化/最大化/还原/起始缩放/设置尺寸真实生效（结果窗自动高度也依赖 `set-size`）；
- 新增 4 个单元测试（引擎提示合并：就绪清旧原因 / 未停止附原因 / 引擎自身原因优先 / 启动中不附加旧原因）；
- 真机 E2E（cloud）：标题栏按钮真实生效；结果窗卡片与窗口逐像素一致（**460×312**，无透明边/固定空白）；
- 双安装包由 T5 重建并归档（`SnipAsk_1.0.2_x64-setup.exe` / `SnipAsk_1.0.2_x64-full-setup.exe`，
  **7.42 MiB / 21.52 MiB**，门禁 PASS，两版安装/启动/关闭隐藏到托盘/卸载冒烟 PASS）；SHA256 以
  `release/SHA256SUMS.txt` 为准；
- **v1.0.2 最终门禁（全绿）**：`cargo fmt --check` 0；`cargo clippy --all-targets -- -D warnings` 0 warning；
  `cargo test --all-targets` **484 passed / 0 failed / 20 ignored**（lib 345 + 新增 4 个引擎提示测试）；
  `pnpm typecheck` 0、`pnpm test` **48/48**、`pnpm build:web` 0；
- **v1.0.2 真机 E2E（cloud PASS）**：标题栏最小化/最大化按钮真实生效；结果窗 **460×312** 卡片与窗口
  逐像素一致（无透明边/固定空白）；证据见本地 `.verify/verify-v1.0.2/SUMMARY.md`（gitignored）。

**v1.0.1（前置版本）验证摘要（2026-11-06）**：

- 最终门禁全绿：`cargo fmt --check` 0、`cargo clippy --all-targets -- -D warnings` 0 warning、
  `cargo test --all-targets` **480 passed / 0 failed / 15 ignored**、`pnpm typecheck` 0、`pnpm test` 48/48、
  `pnpm build:web` 0；
- 真机 form-check PASS：界面无工程术语（`banned=[]`）、引导完成后导航仅「设置/历史」、自绘标题栏 + 8 向缩放手柄、
  `WM_CLOSE` 隐藏到托盘（进程存活）并可重新显示、内存真实值 46MB、版本 1.0.1、模型默认 idle→下载/取消状态正确、
  搜索/隐私/默认模式设置保存生效；
- 真机 E2E（cloud/local `ok=true`）：cloud 翻译 0.75s / AI 搜索 3.58s / 图片搜索 10.02s /
  `translate_cover` 5/5（4.09s）/ 历史 3 轮；local Vulkan offload 29 层、本地翻译 1.46s（1.5B，含提示词残留，
  S-14 已知问题）；证据 `.verify/verify-v1.0.2/SUMMARY.md`（含 v1.0.1 基线）。

**v1.0.0（前置版本）验证摘要（真实应用 / 真实模型 / 真实网络，2026-11-05/06）**：

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
- **不收集任何用户信息**：无遥测、无崩溃上报、无使用统计、无更新检查；没有服务端，不代理请求。
  API Key 仅存 Windows 凭据管理器，截图与历史只在本机。
- **隐私**：默认不上传截图；只有用户主动触发云端动作（AI 搜索 / 图片搜索 / 翻译 / 解答）时才发送
  对应文本或图像，且可在设置 → 隐私中分别关闭上传开关；「仅本机处理」可禁用全部云端动作。
- **数据目录**：配置、提示词、日志、历史库在 `%APPDATA%\SnipAsk\`，模型/OCR 模型/缓存在
  `%LOCALAPPDATA%\SnipAsk\`；便携版（程序目录存在 `portable.flag`）时写入程序目录旁 `data\`。
  详见[实施架构 §7](docs/实施架构.md) 与[自定义指南](docs/自定义指南.md)。
- **无自动更新**：不检查更新、不联网查询版本；升级方式见[升级与自定义](#升级与自定义)。
