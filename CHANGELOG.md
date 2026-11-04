# 更新日志

本文件记录「框选 AI（Snip Ask）」的版本变更，格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 计划中

- 技术验证（Spike）：`Ctrl+Q` → 虚拟桌面冻结 → 前置 OCR → 框选 → 流式问答最小闭环（见 `docs/技术验证清单.md`）。
- 真实能力接入：Win32 `BitBlt`/WGC 捕获、RapidOCR/ONNX 推理、OpenAI 兼容流式网关、DeepSeek Search/Fetch、Qwen-MT、本地 llama.cpp/Ollama 引擎、Windows Credential Manager、模型下载与 SHA256 校验。
- 安装包（NSIS 用户级）、代码签名、更新通道与体积门禁。

## [0.1.0] - 2026-11-04

首个**基础架构（脚手架）**版本：可运行的空壳闭环，真实外部能力均为 `TODO(real)` 占位实现。

### 新增

- **分层结构**：Rust 侧 `contract / app / commands / domain / services / platform / events / resources` 单向依赖分层；前端 `app / shared(contract·ipc·theme·util) / components/ui / windows(main·overlay·result)`。
- **IPC 契约 v1**（`docs/接口契约.md`）：29 个命令、14 个事件、`AppError` 错误码表（SA-10xx–SA-90xx）、会话状态机、配置 Schema **v3**（含迁移链与原子写约束）、凭据存储约定、物理像素坐标约定；`src/shared/contract/` 与 `src-tauri/src/contract/` 双端同构。
- **配置中心**：默认配置、深度合并、校验、迁移、原子持久化与 `config.changed` 热生效分类。
- **骨架模式（skeleton）**：`advanced.skeletonMode = true` 时捕获/OCR/LLM/搜索/翻译/下载/更新均按契约 §12 的确定性占位实现，事件流与真实实现一致，无网络、无密钥、无模型也能完整走通 UI 闭环。
- **前端三窗口**：`main`（设置页 + 6 步首启引导）、`overlay`（冻结层 + 框选 + 放大镜 + 动作菜单）、`result`（流式输出 + 来源 + 追问），浏览器 mock 后端支持 `/?window=` 独立预览。
- **内置资源**：`resources/prompts/*.md`（5 个提示词模板）、`resources/models.manifest.json`（3 个本地模型条目，`verified: false`）、`resources/pricing.json`（模型与搜索计价，CNY）。
- **应用图标**：`src-tauri/icons/` 下 32/128/256/512 PNG 与多尺寸 ICO，由 `scripts/gen-icons.py`（Pillow）生成。
- **工程配套**：pnpm 脚本、`scripts/{dev,build,verify}.ps1`、`docs/开发指南.md`、`docs/技术验证清单.md`、ADR 0001–0005、Apache-2.0 `LICENSE`、`NOTICE`、`.gitignore` / `.gitattributes` / `.editorconfig`、GitHub Actions CI（windows-latest：Web 类型检查与构建 + Rust fmt/clippy/test）。

### 说明

- 本版本为**脚手架**，不可用于生产环境；Rust 侧业务逻辑与外部调用均为占位。
- 开发机若未安装 Rust/MSVC 工具链，可用 `pnpm dev:web` 在浏览器中验证 UI；Rust 检查在 CI（windows-latest）执行，或按 `docs/开发指南.md` 安装工具链后运行 `scripts/verify.ps1`。
- 内置提示词可由 `%APPDATA%\SnipAsk\prompts\*.md` 覆盖（用户 > 内置）。
