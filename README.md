# 框选 AI（Snip Ask）

> 轻量快速、托盘常驻、可开机自启、高度自定义的 Windows 框选解答 / 翻译 / 搜索工具。
> 按 `Ctrl+Q` 冻结屏幕 → 框选任意区域 → 搜索 / 翻译 / 解答，结果以流式输出秒级呈现。

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2B%20x64-0078D4.svg)
![Release](https://img.shields.io/badge/release-v0.1.0-2ea44f.svg)
![Status](https://img.shields.io/badge/status-skeleton%20MVP-orange.svg)

**框选 AI** 是 BYOK（Bring Your Own Key）的桌面工具：没有服务端、不收集截图、不做遥测；
用户自备 AI 服务密钥，截图与文本只在用户主动触发云端动作时才发送。开源协议为
[Apache-2.0](LICENSE)。

本仓库为 **v0.1.0 收尾版本**：源码、文档与可直接安装的 Windows x64 安装包
（[`release/SnipAsk_0.1.0_x64-setup.exe`](release/SnipAsk_0.1.0_x64-setup.exe)，2.68 MiB）一并入库，
不配置开发环境也能直接体验完整交互闭环。

> **当前能力边界：骨架模式 MVP。** 分层结构、IPC 契约（v4：38 命令 / 16 事件）、配置中心、三窗口前端、
> **本地 GPU 推理（Vulkan 优先 / CUDA 可选 / CPU 回退）**、**覆盖式翻译（冻结帧 Canvas）**、
> **连续提问（多轮追问）** 均已实现；真实 Win32 捕获、RapidOCR/ONNX 推理、云端 OpenAI 兼容网关等
> 仍为 trait 桩 + `TODO(real)` 占位。安装包内可完整体验「热键 → 冻结 → 框选 → 菜单 → 流式结果」闭环，
> 详见[当前状态](#当前状态)。

---

## 下载与安装

### 发行产物

| 产物 | 路径 | 大小 | 说明 |
| --- | --- | --- | --- |
| Windows x64 安装包（NSIS） | [`release/SnipAsk_0.1.0_x64-setup.exe`](release/SnipAsk_0.1.0_x64-setup.exe) | 2.68 MiB | 用户级安装（当前用户），无需管理员权限 |
| SHA256 校验和 | [`release/SHA256SUMS.txt`](release/SHA256SUMS.txt) | — | 下载后请务必校验 |
| 发行说明 / 构建信息 | [`release/README.md`](release/README.md) | — | 构建命令、工具链版本、安装/卸载、已知限制 |

### 安装步骤

1. 取得 `release/SnipAsk_0.1.0_x64-setup.exe`，先校验完整性：

   ```powershell
   Get-FileHash .\release\SnipAsk_0.1.0_x64-setup.exe -Algorithm SHA256
   # 与 release\SHA256SUMS.txt 中的值比对
   ```

2. 双击运行安装包；或静默安装（当前用户）：
   `Start-Process .\release\SnipAsk_0.1.0_x64-setup.exe -ArgumentList '/S'`。
3. 安装完成后启动 **SnipAsk**；首次启动进入 6 步引导，无网络 / 无 API Key 可跳过并进入受限模式。
4. 按 `Ctrl+Q` 开始框选；托盘菜单可打开设置、暂停热键或退出。

> **未签名提示**：0.1.0 安装包**未做代码签名**。从网络下载后首次运行时，Windows SmartScreen 可能显示
> 「Windows 已保护你的电脑」，点击「更多信息 → 仍要运行」即可。已在 Windows 11 + Defender 实机验证
> 安装/启动/卸载且 Defender 扫描无威胁，详见 [S-16 记录](docs/spikes/S-16-无签名安装包与杀软误报.md)。

### 卸载与数据目录

- **卸载**：设置 → 应用 → 已安装的应用 → **SnipAsk** → 卸载；或运行
  `%LOCALAPPDATA%\SnipAsk\uninstall.exe`。卸载会移除程序文件与快捷方式，但**保留用户数据**。
- **数据位置**：

  | 内容 | 路径 |
  | --- | --- |
  | 配置 / 日志 / 历史库 | `%APPDATA%\SnipAsk\` |
  | 本地模型 / 缓存 / 运行时 | `%LOCALAPPDATA%\SnipAsk\`（安装目录同名，卸载后可能残留 `models/cache/runtime`） |
  | WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` |

  彻底清理：先卸载，再手动删除上述目录。便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`。

---

## 目录

- [下载与安装](#下载与安装)
- [核心交互](#核心交互)
- [界面预览（骨架模式）](#界面预览骨架模式)
- [当前状态](#当前状态)
- [快速开始](#快速开始)
- [仓库结构](#仓库结构)
- [工程约定](#工程约定)
- [文档索引](#文档索引)
- [验收与性能目标](#验收与性能目标)
- [许可与隐私](#许可与隐私)

## 核心交互

按 `Ctrl+Q` 冻结虚拟桌面 → 拖拽框选目标区域（支持方向键微调、回车确认、`Esc` 取消）→
在动作菜单中选择 **搜索 / 翻译 / 解答 / 复制**，结果在结果窗中流式呈现，可追问、重答、复制、关闭。

- **有文字**：菜单提供文本搜索、翻译、解答、复制文字；框选后立即在后台做整屏 OCR 预扫描。
- **无文字 / 图片**：菜单提供图片搜索（视觉识别 → 检索词 → 联网检索 → 带来源结论）、看图解答、翻译。
- **快速 / 深度两档**：按功能与场景切换，快速档低时延低成本，深度档允许更多检索与更长输出。
- **本地 GPU 推理（契约 v4）**：本地模型可走 GPU，**Vulkan 优先**（AMD / Intel / NVIDIA 通用、依赖最小），
  CUDA 为 NVIDIA 可选组件，不可用时自动回退 CPU；设置页可选「计算设备：自动 / CPU / GPU」与 GPU 层数。
- **覆盖式翻译（契约 v4）**：翻译结果**原位覆盖**冻结帧上的原文（模糊原区域 + 按版式重绘译文），
  可切「原文 / 双语 / 覆盖」；仅在框选画面上显示，不改动目标应用。
- **连续提问（契约 v4）**：回答后可就同一内容**多轮追问**，结果窗以对话线程展示，支持重答与清空线程。
- **BYOK**：默认 DeepSeek 视觉/搜索能力，翻译可走云端（Qwen-MT）或本地小模型，全部由用户配置。

## 界面预览（骨架模式）

以下截图来自 `pnpm dev:web` 的浏览器 mock 预览（骨架模式下的确定性占位数据，真实后端接入后交互与布局不变）：

| 设置 · 核心 | 设置 · AI 服务 |
| --- | --- |
| ![设置-核心](docs/images/settings-general.png) | ![设置-AI服务](docs/images/settings-ai.png) |

| 首启引导 · AI 服务（第 2 步） | 结果窗（流式 + 追问） |
| --- | --- |
| ![引导-AI服务](docs/images/onboarding-ai.png) | ![结果窗](docs/images/result-window.png) |

| 设置 · 动作与热键 |
| --- |
| ![设置-动作与热键](docs/images/settings-actions.png) |

> 冻结层/选框/放大镜/动作菜单的 overlay 截图未入库（浏览器 mock 的 `?window=overlay` 可直接查看）。

### 真实运行（Release 构建，骨架模式）

以下为 `pnpm build` 产物在 Windows 11（150% 缩放）实机运行时的截图：

| 框选后的动作菜单（真实窗口） | 结果窗（流式输出 + 用量） |
| --- | --- |
| ![真实动作菜单](docs/images/real-run-menu.png) | ![真实结果窗](docs/images/real-run-result.png) |

## 当前状态

| 层次 | 内容 | 状态 |
| --- | --- | --- |
| 域逻辑 | 会话状态机、选区/坐标换算、文字存在判定、动作注册表、费用估算、多轮线程（纯逻辑 + 单测） | ✅ 已实现 |
| IPC 契约 | **38 命令 / 16 事件（契约 v4）** / `AppError` 错误码表 / 配置 Schema **v4**，TS 与 Rust 双端同构 | ✅ 已实现 |
| 配置中心 | 默认值、深度合并、校验、迁移（**v4←v3←v2←v1**）、原子写 + `.bak`、`config:changed` 热生效分类 | ✅ 已实现 |
| 骨架模式 | 捕获/OCR/LLM/搜索/翻译/下载/更新按契约 §12 确定性占位，事件流与真实实现一致 | ✅ 已实现 |
| 前端窗口 | `main`（设置页 + 6 步首启引导）、`overlay`（冻结层 + 选框 + 放大镜 + 动作菜单 + 覆盖翻译层）、`result`（线程式流式输出 + 来源 + 追问） | ✅ 已实现 |
| 工程底座 | 分层目录、内置资源、图标、pnpm 脚本、`scripts/*.ps1`、CI（windows-latest）、ADR | ✅ 已实现 |
| **发行产物** | Windows x64 NSIS 安装包 + SHA256 校验和随仓库入库（[`release/`](release/README.md)）；Release 构建实机安装/启动/卸载验证 | ✅ 已交付（v0.1.0，未签名） |
| **本地 GPU 推理** *(契约 v4)* | GPU 探测（`--list-devices`）/ 显存预算 / 后端组件按需下载（Vulkan 默认、CUDA 可选）/ 引擎监督者（Vulkan → CUDA → CPU）/ OpenAI 兼容流式网关 / 设置页计算设备 | 🟢 已实现并真机验证（NVIDIA RTX 4060 Laptop：`Vulkan0` 探测、`-ngl 99` 真实推理、`/health` 2188ms、无残留；**AMD 真机未测**，Vulkan 为厂商无关路径） |
| **覆盖式翻译** *(契约 v4)* | 段块合并 + 自适应字号/换行/气泡布局算法（纯函数 + node 测试）/ 冻结帧 Canvas 覆盖层（三态）/ `translate_cover` 批处理 + 会话级 LRU + `translate:progress`/`ready` | 🟢 已实现（布局算法 25/25；浏览器 mock 真实位图三态演示；真实 OCR/翻译质量仍受前置 `TODO(real)` 限制） |
| **连续提问** *(契约 v4)* | F1 修复（不再必然 `SA-1099`）/ `TurnStore` 上下文注入 / `task:started.parent` / 失败可继续追问 / 线程式结果窗 / 清空线程 = `result_close` | 🟢 已实现并回归验证（F1 回归 9/9、线程 UI 7/7；历史持久化属完整版，未做） |
| Win32 捕获 | `BitBlt`/WGC 截取虚拟桌面、多屏混合 DPI（`platform::capture`，trait 已定义） | 🟡 `TODO(real)` |
| 本地 OCR | RapidOCR/ONNX 推理、两段式 det/rec、阈值判定接入 | 🟡 `TODO(real)` |
| HTTP 网关（云端） | OpenAI 兼容流式 SSE、DeepSeek Search/Fetch、Qwen-MT、重试与费用计量 | 🟡 `TODO(real)`（本批已提供本地/云端共用 `HttpLlmGateway`，云端接入仍待补） |
| 凭据 / 更新 | Windows Credential Manager 实装、`tauri-plugin-updater` 签名通道 | 🟡 `TODO(real)` |

图例：✅ 脚手架内完整可运行/可测；🟢 追加需求批次（契约 v4）新增，已实现并通过本批门禁（真机项见验证报告）；
🟡 接口与骨架就绪，真实能力待接入。逐模块（M01–M20 + M12b）的映射与状态见 [`docs/实施架构.md` §6](docs/实施架构.md)。

> 骨架模式下所有外部能力都有确定性占位实现：无网络、无密钥、无本地模型也能完整走通
> 「热键 → 冻结 → 框选 → 菜单 → 流式结果」闭环，便于前后端并行开发与手动验收。
>
> **真实编译/运行验证（2026-11-04）**：Rust 1.99.1 + MSVC 14.51 下 `cargo build` / `cargo test`（148 项）/
> `cargo clippy -D warnings` / `cargo fmt --check` 全部通过；release 应用实机启动并跑通
> 「`Ctrl+Q` → 框选 → 动作菜单 → 流式结果窗」端到端。注意：直接 `cargo build --release` 不会启用
> Tauri 的 `custom-protocol` feature（会退化为连接 devUrl），桌面发布请用 `pnpm build`（或
> `cargo build --release --features custom-protocol`）。
>
> **收尾发布验证（2026-11-05）**：最终源码在 Rust 1.99.1 / Node 24.14.1 / pnpm 11.21 下通过
> `cargo fmt --check`、`cargo clippy --all-targets -D warnings`（0 warning）、`cargo test`（**376 passed / 0 failed / 6 ignored**）、
> `pnpm typecheck`、`pnpm test`（布局 25 + 线程 7）与 `pnpm build:web`；由 `pnpm build` 生成的 NSIS 安装包
> 已随仓库入库（大小与 SHA256 见 [`release/SHA256SUMS.txt`](release/SHA256SUMS.txt)）。

## 快速开始

### 终端用户（直接安装）

无需配置开发环境：从 [`release/`](release/README.md) 取得
[`SnipAsk_0.1.0_x64-setup.exe`](release/SnipAsk_0.1.0_x64-setup.exe) → 校验 `SHA256SUMS.txt` → 双击安装。
安装、卸载、数据目录与未签名提示见[下载与安装](#下载与安装)。

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
pnpm build      # tauri build：生成 NSIS 安装包
```

### 检查与构建

```powershell
pnpm typecheck    # tsc --noEmit
pnpm test         # 前端纯逻辑单测：覆盖布局 25 项 + 线程模型 7 项（Node 原生 TS）
pnpm build:web    # tsc --noEmit && vite build → dist/
pnpm build        # tauri build（需 Rust）→ src-tauri\target\release\bundle\nsis\*.exe

# 一键校验（等价 CI：typecheck + 单测 + build:web + Rust fmt/clippy/test；
# 无 Rust 时跳过 Rust 检查并给出提示）
powershell -ExecutionPolicy Bypass -File scripts\verify.ps1
```

> 发布用安装包由 `pnpm build` 生成在 `src-tauri\target\release\bundle\nsis\`；仓库内的
> `release\SnipAsk_0.1.0_x64-setup.exe` 即该产物归档（构建信息与校验和见 [`release/README.md`](release/README.md)）。

辅助脚本：

| 脚本 | 作用 |
| --- | --- |
| `scripts\dev.ps1` | 检测 Node/pnpm/Rust；有 Rust 启动 `pnpm dev`，无 Rust 提示改用 `pnpm dev:web` 并给出安装步骤 |
| `scripts\build.ps1` | `pnpm install` → `pnpm build:web`；有 Rust 时再构建安装包，否则说明跳过原因与产物位置 |
| `scripts\verify.ps1` | Web typecheck/单测/build + Rust fmt/clippy/test；输出 PASS/SKIP/FAIL 汇总，失败退出码 1 |
| `scripts\gen-icons.py` | 用 Pillow 重新生成 `src-tauri/icons/` 图标并校验尺寸/格式 |

## 仓库结构

```text
框选ai/
├── release/                  # 发行产物：v0.1.0 NSIS 安装包、SHA256SUMS.txt、发行说明
├── docs/                     # 设计文档：实施架构、接口契约、开发指南、技术验证清单、ADR、研究/验证记录
├── resources/                # 编译内嵌资源：prompts/*.md、models.manifest.json、backends.manifest.json、pricing.json
├── scripts/                  # dev/build/verify.ps1 + gen-icons.py
├── .github/workflows/ci.yml  # CI：Web typecheck/单测/构建 + Rust fmt/clippy/test + NSIS 体积门禁（windows-latest）
├── index.html                # 单入口，按 ?window= 分发（main/overlay/result）
├── package.json / pnpm-lock.yaml
├── src/                      # React 18 + TS + Vite 前端（WebView2 内运行）
│   ├── app/                  # 窗口分发、错误兜底、IPC 事件 hook
│   ├── shared/               # contract（类型/命令名/默认值）、ipc（tauri/mock）、theme、util
│   ├── styles/               # tokens.css（契约 §14）+ base.css
│   ├── components/ui/        # 通用展示组件
│   └── windows/              # main（设置/引导）、overlay（框选/菜单/覆盖翻译）、result（线程式结果窗）
└── src-tauri/                # Rust 后端（Tauri 2）
    ├── src/contract/         # 对外契约结构（错误/类型/配置/事件/常量）
    ├── src/app/              # 生命周期、AppState、托盘、窗口、热键
    ├── src/commands/         # 38 个 Tauri 命令（薄层：校验 → service → 返回）
    ├── src/domain/           # 纯逻辑（无 IO，可单测）
    ├── src/services/         # 能力编排（trait 注入，骨架模式可替换）
    ├── src/platform/         # 唯一允许 unsafe/Win32 的适配层
    ├── src/events/           # 强类型事件发送助手
    ├── src/resources/        # include_str! 内嵌仓库 resources/
    ├── tests/                # 契约序列化、覆盖翻译、GPU 探测、SSE 网关等集成测试
    ├── migrations/           # SQLite 初始 Schema
    └── icons/                # 应用图标（由 scripts/gen-icons.py 生成）
```

完整目录树、分层规则与模块映射见 [`docs/实施架构.md`](docs/实施架构.md) §2–§6。

## 工程约定

1. **契约优先**：`docs/接口契约.md` 是前后端边界的唯一事实来源。新增/修改命令或事件必须先改契约，
   再同步 `src/shared/contract/*` 与 `src-tauri/src/contract/*`；**破坏性**变更递增 `IPC_CONTRACT_VERSION`
   （当前 `1`），纯增量只更新修订记录与配置 Schema 版本（当前 **v4**）。流程见
   [`docs/开发指南.md`](docs/开发指南.md)「契约变更流程」。
2. **分层依赖单向**：`commands → services → domain`，`platform` 只被 `services` 调用，
   `contract` 不依赖任何内部模块；`domain` 不得出现 Tauri、文件/网络 IO、`std::process`。
3. **错误模型统一**：命令一律返回 `Result<T, AppError>`，`AppError { code, message, detail, retryable, fallback[], context }`；
   错误码见契约 §8.2，前端按 `fallback` 提供降级按钮，`SA-1099` 静默处理。
4. **无密钥穿越边界**：任何命令入参/出参、事件载荷不得出现明文 API Key；`ConfigView` 只回掩码与 `hasKey`；
   密钥仅存 Windows 凭据管理器，日志禁止记录密钥、OCR 原文与完整查询串。
5. **坐标全程物理像素**：`rectPhys` 以虚拟桌面左上角为原点（可为负），前端不得换算后回传逻辑坐标。
6. **骨架模式默认开启**：`advanced.skeletonMode = true`，外部能力一律走确定性占位实现；
   真实实现以 `// TODO(real): ...` 标注并保持同一 trait。
7. **代码风格**：Rust 用 `cargo fmt` + `cargo clippy -D warnings`，TS 严格模式 + Prettier；
   单文件 ≤ 300 行，单一职责，文件命名见开发指南「代码风格」。

## 文档索引

| 文档 | 说明 |
| --- | --- |
| [`docs/可行性研究报告.md`](docs/可行性研究报告.md) | 需求基线、功能清单、技术选型、成本、合规、验收标准 |
| [`docs/概要设计说明书.md`](docs/概要设计说明书.md) | 总体设计、模块 M01–M20、关键流程、数据设计、性能与安全 |
| [`docs/实施架构.md`](docs/实施架构.md) | 脚手架代码结构、分层规则、运行时形态、模块映射、构建与验证 |
| [`docs/接口契约.md`](docs/接口契约.md) | **IPC 契约 v1 · 追加需求批次 v4**：38 命令 / 16 事件 / 配置 Schema v4 / 错误码 / 骨架模式 / 主题 token |
| [`docs/开发指南.md`](docs/开发指南.md) | 环境准备、常用命令、契约变更流程、调试、**GPU / Vulkan 后端与验证钩子**、常见问题、代码风格 |
| [`docs/技术验证清单.md`](docs/技术验证清单.md) | 可行性报告「Go 前提条件」与 §3.4 技术难点转成的 spike 任务表 |
| [`docs/追加需求可行性研究.md`](docs/追加需求可行性研究.md) | 追加需求（本地 GPU 推理 / 覆盖式翻译 / 连续提问）可行性、实测证据与工作量；配套证据见 [`docs/research/`](docs/research/README.md) |
| [`docs/adr/`](docs/adr/) | ADR-0001～0006：技术栈、前端无网络密钥、单覆盖层物理像素、契约优先、骨架模式、**Vulkan 优先的 GPU 推理** |
| [`release/README.md`](release/README.md) | 发行产物说明：v0.1.0 安装包、SHA256、构建信息、安装/卸载与已知限制 |
| [`CHANGELOG.md`](CHANGELOG.md) | 版本变更记录 |
| [`LICENSE`](LICENSE) / [`NOTICE`](NOTICE) | Apache-2.0 许可全文 / 第三方组件与模型许可声明 |

## 验收与性能目标

摘自《可行性研究报告》§2.3 与 §7.2（目标值，实际以技术验证与真实环境测试为准）：

| 类别 | 指标 |
| --- | --- |
| 启动与常驻 | 冷启动 ≤ 1.5s；空闲内存 ≤ 120MB；空闲 CPU < 0.5%；托盘常驻 + 单实例 |
| 交互 | 热键 → 选框 ≤ 250ms；动作菜单 ≤ 100ms；多屏混合 DPI 框选坐标误差 ≤ 2px |
| OCR | 整屏文字检测 ≤ 1.2s（4K 双屏，后台不阻塞选框）；选区 OCR ≤ 800ms；准确率 ≥ 95% |
| AI 时延 | 快速模式首字 ≤ 1.5s；AI 搜索首字 ≤ 3s；图片搜索快速模式 ≤ 10s、深度 ≤ 25s |
| 翻译 | 云端 200 字 ≤ 1.5s；本地 1.8B（Q4，CPU）≤ 8s（首次加载另计）；本地 1.8B **GPU（Vulkan/CUDA）** P50 1.26s / P95 3.75s（研究实测，RTX 4060 Laptop） |
| 本地 GPU | Vulkan 真实推理可用（`--list-devices` 见 `Vulkan0`）；**Vulkan 优先 / CUDA 可选 / CPU 回退**；AMD/Intel 走同一 Vulkan 路径（**AMD 真机未测**）；显存预算 `vramMB + 512 MiB` |
| 覆盖翻译 | 100 图真实语料：2288 行 → 1552 段块（−32.2%）；推荐配置溢出 0%（研究实测）；字号中位 0.80×源行高；Canvas 渲染预期 0.1–0.3s/100 块 |
| 连续提问 | 任务完成后追问不再必然 `SA-1099`；上下文含上一轮回答；线程式结果窗（仅最后一条流式） |
| 体积 | 主安装包 ≤ 25MB（不含 OCR 模型），含模型 ≤ 60MB；GPU 后端按需下载（Vulkan 默认、CUDA 可选；清单实际：Vulkan 37,684,327 B、CUDA 288,918,265 B + cudart 391,443,627 B，见 `resources/backends.manifest.json`；研究期估算 ≈55.8MB / ≈0.9GB） |
| 引导 | 首启 ≤ 3 分钟完成最小配置；无网络/无 Key 可跳过并进入受限模式；下载支持断点续传 + SHA256 |

可执行的验证任务（负责人、方法、阈值、状态）见 [`docs/技术验证清单.md`](docs/技术验证清单.md)。

**P0 Spike 实测与门禁处置（2026-11-04/05，真实应用/真实模型/真实网络）**：
S-01 混合 DPI 多屏坐标（跨屏误差 0.01px）、S-02 整屏 OCR det（像素预算 3.0Mpx 后双 4K P95 743ms / 单 4K P95 794ms）、
S-04 全局热键与冲突、S-07 离线受限模式 + **引导页三档 AI 服务**、S-08 模型下载断点续传与校验 = **DONE**；
S-03 已按决策落地「整屏 det 缓存 → 选区 rec-only」（P95 596ms，原 det+rec 口径 968ms 保留为对照）；
S-05 云端（DeepSeek，关闭 thinking）首字 P95 0.87s、本地**极小档** P95 3.21s = **DONE**，本地 1.8B「普通档」CPU P95 21.84s
不满足 8s → 按用户决策作为**可选质量档并显式提示**；S-06 图片搜索 = **DONE**（DeepSeek 视觉 + 官方 Search/Fetch 工具 API，
快速 P95 6.01s / 深度 P95 8.29s，各 20 次 0 失败）。
逐项记录与原始数据索引见 [`docs/spikes/`](docs/spikes/README.md)，复跑脚本见 [`scripts/spikes/`](scripts/spikes/README.md)。
剩余决策项：S-05 云端**总耗时**口径（P95 1.75s vs ≤1.5s，首字已达标）建议明确为「首字」。

**P1 Spike 实测（2026-11-05，AI 助手团队；真实应用/真实模型/真实网络）**：
S-09 捕获降级（WGC 实机可用且与 BitBlt 像素级一致；WDA_MONITOR 保护面 BitBlt/WGC 均纯黑、WDA_EXCLUDEFROMCAPTURE 下
WGC 3/3 无帧；产品落地黑屏检测 + 降级链 + SA-1001）、S-10 启动与常驻（release 冷启动 P95 858ms、空闲主进程
WS max 36.8MiB、空闲 CPU 均值 0.0022%）、S-11 文字存在判定（推荐阈值组合误判 2.0%/漏判 0%）、S-15 OCR 字符准确率
（native 99.38% / 生产预算口径 99.29%）、S-16 未签名安装包（安装/启动 840ms/卸载通过、Defender 无威胁）、
S-17 网络异常（真实 IPC 20/20 错误映射 + 断网降级链）、S-18 体积门禁（NSIS 2.57MiB ≤25MB，收尾版 2.68MiB；CI 已加 bundle job）、
S-20 托盘/单实例/热键（第二实例 10/10、退出后 Ctrl+Q 无残留 5/5）= **DONE**（各有遗留）；
**S-14 云端 DONE / 本地 FAILED**（本地格式保持 68.1%）；**S-12 FAILED（来源可访问率 59.6%，乐观 94.1% <95%）**；
**S-13 IN PROGRESS**（8GB 降档提示缺失、OOM 未实测）；**S-19 BLOCKED**（需 3 名真人新用户）。
记录索引见 [`docs/spikes/`](docs/spikes/README.md) §6。

**追加需求批次（契约 v4）验证（2026-11-05）**：全量门禁通过 —— `cargo fmt --check`、`cargo clippy --offline --all-targets -- -D warnings`（0 warning）、
`cargo test --offline`（**376 passed / 0 failed / 6 ignored**，16 个测试套件）、`pnpm typecheck`、`node --test translationLayout`（25/25）、
`node --test thread`（7/7）、`pnpm build:web`。真机 GPU（NVIDIA RTX 4060 Laptop）：`local_gpu_probe` 识别 `Vulkan0`（`useable=true`、
预算 2112/7180 MB fits）；真实 Vulkan 引擎 `-ngl 99 --device Vulkan0` → `/health` **2188 ms**、`offloaded 33/33 layers`、
真实推理输出 **90 ms**（CPU `-ngl 0` 同请求 482 ms，约 5.4×），停止后无残留；Vulkan 组件真实下载 37,684,327 B + SHA256 + Range 续传通过。
**AMD/Intel 实机未测**（Vulkan 路径无 NVIDIA 专属依赖为论证依据），CUDA 真实推理未实测；覆盖翻译的真实 OCR/翻译质量待前置能力接入。
完整矩阵与复跑命令见 `.verify/verify-20261105-2311/SUMMARY.md`（gitignored，本地证据）；
最终发布的安装包、构建信息与 SHA256 见 [`release/README.md`](release/README.md)。

## 许可与隐私

- **许可**：[Apache-2.0](LICENSE)。第三方组件、OCR/推理依赖与模型许可见 [NOTICE](NOTICE)；
  本地翻译模型不随安装包分发，由用户在应用内按需下载，其许可单独标注并遵守模型卡条款。
- **BYOK**：不提供服务端、不代理用户请求、不收集遥测；API Key 仅存 Windows 凭据管理器。
- **隐私**：默认不上传截图；只有用户主动触发云端动作（AI 搜索 / 图片搜索 / 翻译 / 解答）时才发送
  对应文本或图像，且可在设置中分别关闭上传开关；`privacy.localOnly` 可禁用全部云端动作。
- **数据目录**：配置、日志、历史库在 `%APPDATA%\SnipAsk\`，模型与缓存在 `%LOCALAPPDATA%\SnipAsk\`；
  便携版（程序目录存在 `portable.flag`）时写入程序目录旁 `data\`。详见实施架构 §7。
