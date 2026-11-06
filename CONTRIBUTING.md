# 贡献指南

感谢参与「框选 AI（Snip Ask）」。本项目是 Windows 桌面客户端（Tauri 2 + Rust + React），
所有功能设计以 `docs/` 下的两份文档为准：

- [可行性研究报告](docs/可行性研究报告.md)（需求基线 v1.4）
- [概要设计说明书](docs/概要设计说明书.md)（架构与接口 v1.0）

## 开发环境

| 工具 | 版本 | 用途 |
| --- | --- | --- |
| Windows 10 1903+ / 11 x64 | — | 目标平台 |
| Node.js | ≥ 20.19（推荐 22 LTS+） | 前端 |
| pnpm | ≥ 9 | 包管理（不要混用 npm/yarn，锁文件为 `pnpm-lock.yaml`） |
| Rust stable | ≥ 1.77 | 通过 [rustup](https://rustup.rs/) 安装 `x86_64-pc-windows-msvc` |
| MSVC Build Tools | VS 2022「C++ 生成工具」 | Rust 链接器 |
| WebView2 Runtime | Win11 自带 | Tauri 运行时 |

初始化：

```powershell
pnpm install
pnpm dev:web             # 仅前端 + mock 后端：http://localhost:1420
pnpm dev                 # 完整应用（tauri dev，需 Rust 工具链）
```

## 目录与模块约定

- Rust 代码按 `src-tauri/src/{contract,app,commands,domain,services,platform,events,resources}` 单向
  分层（`contract` 只依赖标准库与 serde，不反向依赖上层）；新增文件请标注对应模块号
  （M01–M20，见概要设计 3.1，映射见 [docs/实施架构.md](docs/实施架构.md)）。
- 前端代码按 `src/{app,shared,components,windows}` 组织：`shared/contract` 是 IPC 类型与名称常量的
  **唯一事实来源**，必须与 Rust 侧 `src-tauri/src/contract/` 逐字段一致（见
  [docs/接口契约.md](docs/接口契约.md)）；三个窗口分别在 `src/windows/{main,overlay,result}`。
- 表现层不直接发网络请求、不接触密钥；网络、凭据、推理一律在 Rust 侧完成（概要设计 D5）。
- 平台相关代码（Win32 / DPI / Credential Manager）只能出现在 `src-tauri/src/platform/`。

## 代码规范

- Rust：`cargo fmt`（rustfmt，最大宽 100）、`cargo clippy --all-targets` 无警告；错误统一用
  `AppError`（SA-xxxx 错误码），禁止 `unwrap()`/`expect()` 出现在可失败路径，禁止向日志输出密钥与
  用户文本内容。
- TypeScript：`strict` + `noUnusedLocals`；禁止 `any`（必要时 `unknown` + 类型窄化）；禁止用
  `dangerouslySetInnerHTML` 渲染外部内容。
- 样式：CSS 变量 + 原生 CSS，新增设计 token 写入 `src/styles/tokens.css`。
- 提交前至少执行：

```powershell
pnpm typecheck
pnpm build:web
cd src-tauri; cargo fmt --check; cargo clippy --all-targets; cargo test
```

## 提交与 PR

- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/)：
  `feat: ...`、`fix: ...`、`docs: ...`、`refactor: ...`、`test: ...`、`chore: ...`、`perf: ...`；
  模块相关可带 scope，例如 `feat(ocr): 接入 RapidOCR 两段式推理`。
- 一个 PR 只做一件事；描述里写清动机、改动范围、验证方式（含手测矩阵，如缩放 100/125/150/200%、
  双屏混合 DPI、深色模式）。
- 改动以下内容时**必须同步更新设计文档**并在 PR 中说明：
  - IPC 命令/事件（概要设计 5.1 / 5.2）
  - 配置字段（6.1）与错误码（5.5）
  - 模块划分（3.1）与验收指标（可行性报告 7.2）
- 涉及 UI 的改动请附截图或录屏；涉及性能的改动请附测量数据（怎么测的、环境是什么）。

## 禁止提交的内容

- 任何 API Key、Token、Cookie、`.env` 文件、个人凭据；
- 模型文件（`*.gguf`、`*.onnx` 等）与本地数据库、日志、截图；
- `node_modules/`、`dist/`、`src-tauri/target/` 等构建产物（已在 `.gitignore` 中忽略）。

## 新增能力（动作 / 供应商 / 模型）

- 新动作：在动作注册表（`src/constants/actions.ts` 与契约的动作枚举）中新增 `ActionId`，并补齐
  动作菜单配置、提示词模板与前端分发；需要后端能力时按 `src-tauri/src/services/` 的网关 trait 扩展。
- 新供应商：实现 `src-tauri/src/services/llm_gateway.rs`（或对应网关 trait）的适配器，只依赖
  OpenAI 兼容协议或明确适配层，不把供应商字段写死进业务逻辑。
- 新本地模型：更新 `resources/models.manifest.json`（`id`、`sizeBytes`、`sha256`、`license`、
  `minRamMB`、镜像 `urls`），并在 NOTICE 中补充许可说明；SHA256 必须为实际校验值，禁止占位。

## 论坛/沟通

- 问题反馈请提供：Windows 版本与缩放、屏幕数量/分辨率、Tauri/Rust 版本、复现步骤、脱敏日志
  （注意先删除截图与密钥）。
- 安全漏洞请勿公开提 Issue，见 [SECURITY.md](SECURITY.md)。

## 许可

向本仓库提交贡献即表示同意以 [Apache-2.0](LICENSE) 许可发布你的贡献。
