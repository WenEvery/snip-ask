# 框选 AI（Snip Ask）

> 在 Windows 上框选屏幕任意区域，立即 **搜索 / 翻译 / 解答**。
> 本地 OCR + 本地或云端 AI，默认只在本机处理，开源可审计。

[![Release](https://img.shields.io/badge/release-v1.1.0-2ea44f.svg)](#下载)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078d4.svg)](#系统要求)

![框选后选择动作](docs/images/real-run-menu.png)

## 它是什么

按一次快捷键框选屏幕上的文字或图片，弹出动作菜单：

- **搜索**：把选中内容交给 AI 联网搜索并给出结论；
- **翻译**：覆盖式显示译文（保留原文排版），或生成译文；
- **解答**：解释代码、分析图表、总结内容；
- **复制文字**：只把 OCR 原文写进剪贴板，不调用 AI；
- **图片搜索**：以图搜图，**只上传你框选的区域**。

无需先截图保存、无需切换窗口，适合读论文、看代码、处理外语网页/游戏/PDF。

## 特点

- **框选即用**：全局热键（默认 `Ctrl+Q`）→ 框选 → 选动作，全程不离开当前窗口。
- **本地 OCR**：RapidOCR PP-OCRv4 + ONNX Runtime，屏幕文字本地识别；可选「内置模型版」完全离线。
- **AI 可本地可云端**：本地 `llama.cpp`（Vulkan / CUDA / CPU 回退）跑 Qwen2.5 / HY-MT；也可接 DeepSeek、DashScope Qwen-MT、任意 OpenAI 兼容服务。
- **隐私可控**：默认不上传图片；图片搜索只传框选区域；可开「仅本地模式」；无遥测、无自动更新。
- **翻译覆盖层**：译文按原位置覆盖显示，尽量保持字号/排版，阅读外文资料不跳窗。
- **完整工作流**：流式结果、追问、重答、复制、历史记录、系统托盘、单实例、卸载一键清数据。
- **开源**：Apache-2.0，前端 React + TypeScript，后端 Rust + Tauri 2；可自行构建/修改/分发。

![结果窗](docs/images/real-run-result.png)

## 下载

从本仓库 `release/` 下载（v1.1.0，未签名；SmartScreen 提示时选择「更多信息 → 仍要运行」）：

| 版本 | 文件 | 说明 |
| --- | --- | --- |
| 在线安装版 | [`release/SnipAsk_1.1.0_x64-setup.exe`](release/SnipAsk_1.1.0_x64-setup.exe)（7.45 MiB） | 体积小；首次使用 OCR 时下载约 16 MB 模型 |
| 内置 OCR 模型版 | [`release/SnipAsk_1.1.0_x64-full-setup.exe`](release/SnipAsk_1.1.0_x64-full-setup.exe)（21.56 MiB） | 离线开箱即用（含 OCR 模型） |
| 校验和 | [`release/SHA256SUMS.txt`](release/SHA256SUMS.txt) | 下载后先校验 |

- 系统要求：Windows 10/11 x64、WebView2、建议 8 GB+ 内存；本地模型建议 NVIDIA GPU（Vulkan）。
- 升级：下载新版覆盖安装；卸载时可选删除模型/配置/历史。
- 更详细说明：[`release/README.md`](release/README.md)。

## 快速开始

1. 安装并启动，按引导选择语言、AI 服务（云端 Key 或本地模型）、热键；
2. 按 `Ctrl+Q` 框选屏幕区域；
3. 在动作菜单中选择搜索 / 翻译 / 解答 / 复制；
4. 结果窗中可追问、重答、复制，或结束对话。

首次使用云端服务需要到「设置 → AI 服务」填入自己的 API Key（只保存在本机 Windows Credential Manager）。
不想用云端：下载本地模型并选择「本地翻译 / 本地模型」，或开启「仅本地模式」。

## 从源码构建

需要 Node.js 22+、pnpm 11、Rust 1.99+、VS 2022 Build Tools（MSVC）、WebView2。

```powershell
pnpm install
pnpm typecheck
pnpm test
pnpm build:web

# 准备 OCR 资产（SHA256 校验）并构建双版本安装包（含体积门禁/安装冒烟）
powershell -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1
powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1
```

开发调试：`pnpm tauri dev`。完整文档见 [`docs/`](docs/README.md)（接口契约、架构、自定义指南、开发指南、验证清单）。

## 技术栈

- **前端**：React 18 + TypeScript + Vite；中英双语界面；自绘框选层/结果窗。
- **后端**：Rust + Tauri 2；Win32 屏幕捕获、系统剪贴板、全局热键、托盘、单实例。
- **识图**：RapidOCR PP-OCRv4（det/rec/cls）ONNX + ONNX Runtime 1.31；整屏 det 缓存 + 选区 rec。
- **推理**：OpenAI 兼容 SSE 网关；本地 llama.cpp（Vulkan/CUDA/CPU）；DeepSeek / DashScope Qwen-MT / 自定义 provider。
- **存储**：配置/历史/日志在 `%APPDATA%\SnipAsk`；模型/缓存/后端在 `%LOCALAPPDATA%\SnipAsk`；凭据在 Windows Credential Manager。

## 已知限制

- 安装包**未签名**（无代码签名证书），SmartScreen 可能提示；请校验 SHA256。
- 本地 GPU 推理在 NVIDIA RTX 4060 Laptop 实测；**AMD / Intel 真机未验证**（Vulkan 理论可用）。
- 本地小模型翻译质量弱于云端；英文界面覆盖全部前端 UI，个别后端错误原文仍为中文。
- 关闭窗口 = 隐藏到托盘（设计如此）；退出请用托盘菜单。
- 未做 Win10 干净机 / 国产杀软 / 真人可用性全覆盖。

## 许可

[Apache-2.0](LICENSE)。第三方组件、模型与字体许可见 [NOTICE](NOTICE)。
本项目与 DeepSeek、阿里云 DashScope、PaddleOCR 等无隶属关系；API Key 与模型由用户自行提供。

---

**关键词**：screen translation, OCR, screenshot translate, 框选翻译, 屏幕翻译, 截屏翻译, AI 搜索, 图片搜索, 本地大模型, llama.cpp, RapidOCR, Tauri, Windows, 开源
