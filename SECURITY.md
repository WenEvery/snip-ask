# 安全策略

## 漏洞上报

**请勿通过公开 Issue 上报安全漏洞。** 请使用仓库的
[GitHub Security Advisory（私有漏洞报告）](../../security/advisories/new) 提交，
并尽量包含：

- 影响版本与平台（Windows 版本、架构）；
- 复现步骤或 PoC（不要附带真实用户截图、密钥或隐私数据）；
- 影响评估（读取密钥、任意文件读写、远程代码执行、SSRF、权限提升等）；
- 你希望的公开时间（默认 90 天后公开细节）。

维护者会在 7 个工作日内确认，并给出修复计划；修复发布后会在 Release Notes 中致谢（如你同意）。

## 支持范围

| 版本 | 状态 |
| --- | --- |
| 1.0.x | 正式版（Windows x64）：真实捕获 / OCR / 云端与本地推理 / 凭据 / 历史持久化；**未签名** NSIS 安装包（slim/full），安装包随仓库 `release/` 分发 |
| 0.1.x | 历史版本（骨架模式 MVP），不再提供安全修复 |
| < 0.1 | 未发布 |

> 1.0.0 安装包为未签名构建，Windows SmartScreen 可能提示「未知发布者」；
> 发行产物、SHA256 与安装/卸载说明见 [`release/README.md`](release/README.md)。
> **本软件不自动更新**（无 updater 插件、无更新检查、无更新通道）；安全修复需用户手动下载新版安装包覆盖安装，
> 见 [`docs/自定义指南.md`](docs/自定义指南.md) §1。
> 代码签名与稳定版支持策略将在具备签名证书后提供。

## 安全设计基线（实现时必须遵守）

依据[概要设计说明书](docs/概要设计说明书.md)第 9 章：

1. **密钥**：仅写入 Windows Credential Manager（service `SnipAsk`，user = provider id），不进入配置文件、日志、崩溃报告；
   界面只显示掩码；`config_export` 默认剔除密钥。
2. **网络**：仅允许 HTTPS；本地推理仅允许 `http://127.0.0.1`，端口绑定回环并优先使用随机空闲端口；
   不代理、不中转用户请求；**没有任何更新检查/遥测请求**。
3. **上传**：文本与图像分别受 `privacy.allowTextUpload` / `privacy.allowImageUpload` 控制，关闭后对应
   动作禁用并解释原因；上传前压缩图片（长边 ≤ 1600px）。
4. **完整性**：OCR 模型、本地模型与后端组件下载均校验 SHA256 与文件大小，失败删除重下；
   安装包发布附带 SHA256（`release/SHA256SUMS.txt`）。**无自动更新**，升级为用户显式覆盖安装。
5. **无遥测**：不采集任何遥测数据；不记录用户输入文本；历史记录仅本地 SQLite，可关闭、可清空。
6. **前端隔离**：表现层不接触密钥、不直接发网络请求，全部经由 Rust 侧命令与网关。
7. **上传开关校验**：后端在每次请求前校验开关状态，不能只依赖前端禁用按钮。
8. **外部内容**：结果窗渲染 AI 输出时不得使用 `dangerouslySetInnerHTML`，链接打开走白名单
   （仅 `https`）。
9. **依赖**：新增依赖须为宽松许可（MIT/Apache-2.0/BSD 优先），并在 [NOTICE](NOTICE) 登记；禁止引入
   来源不明或不可审计的二进制分发包。
10. **OCR/推理资产**：OCR 模型与 ONNX Runtime 只从 `resources/models.manifest.json` 声明的镜像下载并强校验；
    `full` 版随包资产同样记录 SHA256；`platform/capture.rs` 是唯一 `unsafe` Win32 出口。

## 已知限制（非漏洞）

- 全屏独占游戏、DRM 受保护内容可能捕获为黑屏（Windows 捕获 API 限制），返回 `SA-1001` 并提示，不做绕过；
- 无代码签名证书时 Windows SmartScreen 可能拦截安装包；**应用不自动更新**，安全修复需手动升级；
- 视觉模型可能产生幻觉，图片搜索结论会标注「AI 识别」与来源，不应视为事实；
- 历史库（本地 SQLite）按设计保存识别文本与回答（可关闭/清空/设置保留天数）；**不加密**，请按本地数据保护习惯管理；
- 本地 OCR/翻译质量随模型档位变化（极小档 0.5B 质量有限），敏感/关键内容建议云端 + 人工复核。

## 供应链

- Rust 依赖以 `Cargo.lock` 锁定、Node 依赖以 `pnpm-lock.yaml` 锁定，CI 使用锁定版本；
- 本地模型/OCR 模型文件不随仓库分发（`full` 版安装包内置 OCR 模型），下载源（HuggingFace / ModelScope /
  官方镜像）与校验值记录在 `resources/models.manifest.json`；OCR 资产由 `scripts/fetch-ocr-assets.ps1`
  按内置真实 SHA256 强校验；
- 发布产物需附带 SHA256 与来源说明；安装包内随附 `docs/`（自定义指南/许可/安全与隐私说明）。
