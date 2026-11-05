# 框选 AI（Snip Ask）· v1.0.2 发行说明

本目录是 **v1.0.2（修复版）** 的双版本发行产物归档，由仓库源码在本地 Windows 环境构建，
与该版本源码同一提交入库，可直接安装使用。

> v1.0.1 与更早（v1.0.0 / v0.1.0 骨架模式 MVP）的安装包**已从本目录移除**；历史版本请见仓库提交历史。
> 本软件**不包含自动更新、也不包含遥测/使用统计上报**：升级方式 = 下载新版安装包覆盖安装，
> 或按《自定义指南》自行源码构建。

## 0 v1.0.2 主要变化（相对 v1.0.1，用户反馈修复）

- **结果窗透明外框 / 固定空白修复**：结果窗不再透明、无系统阴影，卡片铺满窗口；高度随内容自适应
  （内容少时收紧，内容超过屏幕 85% 后内部滚动），去掉原固定 420px 高度造成的下方透明/空白；
- **引擎已就绪仍提示「本地模型缺失」修复**：旧的失败提示在模型安装完成、引擎就绪后自动清空；
  仅引擎未启动时显示原因（新增 4 个单元测试覆盖该逻辑）；
- **自绘标题栏按钮无响应修复**：补齐窗口权限（最小化 / 最大化 / 还原 / 起始缩放 / `set-size`），
  真机 E2E 已验证最小化、最大化按钮真实生效；
- 继承 v1.0.1：自绘标题栏 + 关闭/`Alt+F4` 隐藏到托盘、模型下载 `idle` 状态与取消/重试、真实内存值、
  引擎启停、搜索/隐私/默认模式设置、移除遥测（配置 schema **v6**）；
- 版本 `1.0.2`；其余能力（真实捕获/OCR/云端推理/凭据/历史/双安装包）与 v1.0.1 一致。

## 1 发行产物

| 文件 | 版本类型 | 大小 | SHA256 |
| --- | --- | --- | --- |
| [`SnipAsk_1.0.2_x64-setup.exe`](SnipAsk_1.0.2_x64-setup.exe) | **slim**（安装后下载版） | 7,778,687 B（7.42 MiB） | `d0e2df6b19fc964f243f9f80ca7a330f47cd94e87fa6b9d4d1f23dd9ec3ec5b2` |
| [`SnipAsk_1.0.2_x64-full-setup.exe`](SnipAsk_1.0.2_x64-full-setup.exe) | **full**（内置 OCR 模型版） | 22,564,280 B（21.52 MiB） | `67af3437f682cc13e51491d3ac62e39714efb4ebf7d71d548cbed475fa99c901` |
| [`SHA256SUMS.txt`](SHA256SUMS.txt) | — | — | 上述两个安装包的机器可读校验和 |

- 产物类型：NSIS 安装包（Windows x64），`installMode=currentUser`（当前用户安装，不需要管理员权限、不写 HKLM）；
- 版本：`1.0.2`（`package.json` / `Cargo.toml` / `tauri.conf.json` / 安装包版本资源一致）；
- 签名状态：**未签名**（`Get-AuthenticodeSignature` → `NotSigned`），预期内，见 §7；
- 两个版本都内置：应用本体、ONNX Runtime 1.31.1 运行时（`ocr/onnxruntime.dll`）、提示词、模型/定价清单，
  以及安装目录 `docs/`（README、自定义指南、LICENSE、NOTICE、SECURITY、PRIVACY）。

## 2 双版本差异（slim vs full）

| 能力 | slim（安装后下载版） | full（内置 OCR 模型版） |
| --- | --- | --- |
| 应用本体 + 文档 | 内置 | 内置 |
| ONNX Runtime 运行时 `ocr/onnxruntime.dll` | **内置** | **内置** |
| OCR 模型（det / rec / cls + `ppocr_keys_v1.txt`） | **不内置**，首次使用 OCR 时下载（≈16 MB） | **内置**（离线开箱即用） |
| 安装包体积（本次实测） | **7.42 MiB**（7,778,687 B） | **21.52 MiB**（22,564,280 B） |
| 适用场景 | 体积敏感、首次 OCR 可联网 | 离线 / 内网 / 不想额外下载 |

- slim 下载的模型落盘到 `%LOCALAPPDATA%\SnipAsk\models\ocr\`（文件名固定：`ch_PP-OCRv4_det_infer.onnx`、
  `ch_PP-OCRv4_rec_infer.onnx`、`ch_ppocr_mobile_v2.0_cls_infer.onnx`、`ppocr_keys_v1.txt`），下载后离线可用；
- 两者运行期解析顺序一致：随包 `<安装目录>\ocr\` → 数据目录 `%LOCALAPPDATA%\SnipAsk\models\ocr\` → 开发钩子 `SNIP_ASK_OCR_DIR`；
- 应用内「设置 → 自定义与关于」会显示当前版本类型（`完整版(full)` / `精简版(slim)` / `开发构建(dev)`）与 OCR 就绪状态。

## 3 校验下载完整性

```powershell
# 方式一：与 SHA256SUMS.txt 比对
Get-Content .\release\SHA256SUMS.txt
Get-FileHash .\release\SnipAsk_1.0.2_x64-setup.exe -Algorithm SHA256
Get-FileHash .\release\SnipAsk_1.0.2_x64-full-setup.exe -Algorithm SHA256

# 方式二：certutil
certutil -hashfile .\release\SnipAsk_1.0.2_x64-setup.exe SHA256
certutil -hashfile .\release\SnipAsk_1.0.2_x64-full-setup.exe SHA256
```

预期值与 §1 表格一致。两者不一致时**不要运行**该安装包，重新获取。

> 复现构建出的安装包哈希不一定与仓库归档完全一致（NSIS 打包时间戳/压缩顺序等因素），
> 以「同一源码版本、可安装可运行」为准；发布归档以本目录 `SHA256SUMS.txt` 为校验基准。

## 4 安装与卸载

### 安装

```powershell
# 交互式：双击对应版本的 setup.exe，按向导完成
# 静默安装（当前用户，任选 slim/full 之一）：
Start-Process .\release\SnipAsk_1.0.2_x64-setup.exe -ArgumentList '/S' -Wait
Start-Process .\release\SnipAsk_1.0.2_x64-full-setup.exe -ArgumentList '/S' -Wait
```

- 默认安装目录：`%LOCALAPPDATA%\SnipAsk\`（安装 `snip-ask.exe`、`uninstall.exe`、`ocr\` 与 `docs\`）；
- 安装后创建开始菜单与桌面快捷方式，注册 HKCU 卸载项（`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SnipAsk`）；
- 安装完成后从快捷方式或 `%LOCALAPPDATA%\SnipAsk\snip-ask.exe` 启动；首次启动按引导完成语言/供应商/快捷键设置。

### 卸载

```powershell
# 交互式：设置 → 应用 → 已安装的应用 → SnipAsk → 卸载
# 静默卸载：
& "$env:LOCALAPPDATA\SnipAsk\uninstall.exe" /S
```

卸载会删除程序文件、`ocr\`/`docs\`、卸载器、开始菜单/桌面快捷方式与卸载注册项，但**保留用户数据**（见 §5）。

### 窗口与托盘交互（v1.0.1 起；v1.0.2 补齐标题栏按钮权限）

- 主窗口**没有系统标题栏**：使用自绘标题栏（可拖动移动窗口），支持最小化/最大化/还原与 8 向边缘缩放
  （v1.0.2 修复了自绘标题栏按钮无响应问题，真机验证最小化/最大化真实生效）；
- **关闭按钮 / `Alt+F4` = 隐藏到托盘**（进程继续常驻，后台快捷键与托盘功能保持可用），不是退出程序；
- **托盘图标左键单击 = 打开设置**；托盘右键菜单提供「重新运行引导」与「退出」（退出请用托盘菜单，或先在设置中关闭）；
- 需要完全退出/卸载时，请先通过托盘菜单退出应用，再执行卸载，避免文件占用。

## 5 数据目录

| 内容 | 路径 | 卸载后 |
| --- | --- | --- |
| 配置 / 日志 / 提示词覆盖 | `%APPDATA%\SnipAsk\` | 保留 |
| 历史会话库 | `%APPDATA%\SnipAsk\history.db` | 保留 |
| OCR 模型 / 本地推理模型 / 缓存 / 运行时（与默认安装目录同名） | `%LOCALAPPDATA%\SnipAsk\` | 保留 |
| WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` | 静默卸载默认保留 |

彻底清理：先卸载，再手动删除以上目录。便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`。

> 已知设计问题：默认安装目录 `%LOCALAPPDATA%\SnipAsk` 与应用本地数据目录同名，NSIS 非递归卸载会因数据存在
> 而留下空目录（v1.0.2 实测仍如此，属预期）；后续版本建议改为 `%LOCALAPPDATA%\Programs\SnipAsk`。

## 6 自定义与关于（无自动更新 · 无遥测）

自 v1.0.0 起**移除全部自动更新能力**（无 updater、无更新通道、无联网检查更新）；v1.0.1 起进一步
**移除遥测/使用统计上报**（配置 schema 升至 v6，无任何使用数据外发），我们鼓励你自行修改：

- 应用内入口：**设置 → 自定义与关于**（版本 / 版本类型 / OCR 就绪状态；一键打开配置、提示词、模型、日志、
  后端、安装文档目录；并说明本软件不会自动更新、不上报遥测）；
- 可改点（详见安装目录 `docs\自定义指南.md`）：
  - 提示词覆盖：`%APPDATA%\SnipAsk\prompts\*.md`（支持回答/翻译/搜索/追问等模板与变量）；
  - 术语表：配置 `translate.glossary`；
  - 完整配置字段速查（schema v6，含 `advanced.skeletonMode` 调试、`local.*` GPU、`ocr.*` 阈值、`providers[].kind`）；
  - 本地模型 / 后端替换（`SNIP_ASK_MODEL_PATH` / `SNIP_ASK_BACKENDS_DIR` / `SNIP_ASK_OCR_DIR` 等调试钩子）；
  - 云端供应商接入：DeepSeek、DashScope Qwen-MT、通用 OpenAI 兼容（自定义 baseUrl/model）；
  - 双版本差异与 OCR 模型位置（见 §2）；
- 升级方式：下载新版安装包**覆盖安装**，或按 §10 从源码构建（Apache-2.0 允许修改与再分发，请保留
  `LICENSE`/`NOTICE` 与第三方许可声明）。

## 7 未签名与 SmartScreen

当前安装包未做代码签名。从网络下载后首次运行时，Windows SmartScreen 可能提示
「Windows 已保护你的电脑」（未知发布者）：

1. 确认 SHA256 与 §1/§3 一致（来源可信）；
2. 点击「更多信息」→「仍要运行」。

本机 Windows 11 + Windows Defender（实时防护开启）实测：Defender 自定义扫描无威胁、静默安装/启动/卸载均正常。
覆盖矩阵与未测环境（Win10 干净机、火绒等）见 [S-16 验证记录](../docs/spikes/S-16-无签名安装包与杀软误报.md)。

## 8 构建与验证信息

| 项 | 值 |
| --- | --- |
| 构建命令 | `powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1`（内部：prepare OCR assets → slim/full 各 `SNIP_ASK_EDITION` + `pnpm exec tauri build --config src-tauri/tauri.<edition>.conf.json`） |
| 构建时间 | 2026-11-06 04:39–04:49（Asia/Shanghai；slim 构建+打包 360.8s、full 160.7s，含 edition clean 与 NSIS 打包；slim 构建窗口内另有门禁 cargo 任务并行、等锁导致耗时偏长） |
| Rust | 1.99.1（MSVC 工具链，VS 2022 Build Tools） |
| Tauri CLI | 2.12.4（Tauri 2，`custom-protocol` feature 已启用） |
| Node.js / pnpm | 24.14.1 / 11.21.0 |
| NSIS | Tauri 内置 makensis（`%LOCALAPPDATA%\tauri\NSIS`） |
| 打包目标 | `nsis`，语言 SimpChinese + English，用户级安装 |
| 体积门禁 | slim ≤ 25 MiB、full ≤ 60 MiB（`scripts/build-installers.ps1` 内置，超限失败） |
| OCR 资产 | `scripts/fetch-ocr-assets.ps1` 准备：本机缓存 → `resources/models.manifest.json` 镜像 → 官方镜像；SHA256 强校验 |

发布前质量门禁（同一源码树，2026-11-06，Lead 统一执行）：

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS（0 warning） |
| `cargo test --all-targets` | PASS：`--all-targets` 全绿（lib 345 个 + 本次新增 4 个「引擎提示」单元测试；含真实 OCR 推理、local engine 真机、cloud mock、model_download 等集成测试） |
| `pnpm typecheck` | PASS |
| `pnpm test`（前端纯逻辑） | PASS：**48 / 48** |
| `pnpm build:web` | PASS（131 modules） |
| 真机 form-check（v1.0.1 修复项：无工程词/引导导航/自绘标题栏+8 向缩放/关闭隐藏可重开/内存真实值/模型 idle+下载+取消/设置保存） | PASS（`.verify/v1/fix/form-check.json`） |
| 真机 E2E（v1.0.2 结果窗 + 标题栏按钮） | **PASS**：真实框选 → 真实 OCR → 云端 DeepSeek；结果窗尺寸 **460×312** 与卡片逐像素一致（无透明外框/固定空白）、内容超屏内部滚动；自绘标题栏最小化/最大化按钮真实生效；证据 `.verify/v1/e2e/` |
| 双版本构建 + 体积门禁（`scripts/build-installers.ps1`） | PASS：slim ≤ 25 MiB / full ≤ 60 MiB |
| edition 注入自检（slim/full 主程序 SHA256 不同） | PASS |
| 双安装包真实安装/启动/卸载冒烟 | PASS ×2（0 关键失败 / 0 警告；`closeBehavior=hidden-to-tray` 已验证） |

本次双安装包真实冒烟结果（Windows 11 x64 build 26200，currentUser，静默 `/S` 安装 → 启动 6s → 关闭隐藏到托盘验证 → 静默卸载）：

- slim：安装退出码 0（3.3s）；安装目录 9 个文件、27,380,321 B = `snip-ask.exe`（7,950,848 B，版本资源 1.0.2）
  + `uninstall.exe` + `ocr\onnxruntime.dll`（19,288,888 B，SHA256 与构建源一致）+ `docs\` 6 个文档
  （README/自定义指南/LICENSE/NOTICE/SECURITY/PRIVACY），**无任何模型文件**；启动 6s 后进程存活、
  主窗口「框选 AI」可见、响应正常、工作集 36.7 MB；截图见 `screenshots/launch-slim.png`；
- full：安装退出码 0（4.2s）；安装目录 13 个文件、43,595,613 B = slim 全部 + `ocr\` 下 det/rec/cls/`ppocr_keys_v1.txt`，
  4 个模型/字典 SHA256 与 `src-tauri/resources/ocr/` 源文件完全一致；启动 6s 后进程存活、主窗口可见、响应正常、
  工作集 36.5 MB；截图见 `screenshots/launch-full.png`；
- **关闭行为验证**：对运行中的两版应用发送关闭窗口（`WM_CLOSE`）后进程仍存活 → `closeBehavior=hidden-to-tray`，
  符合「关闭/Alt+F4 隐藏到托盘」设计（随后强制结束以完成卸载步骤）；
- 两版：`Get-AuthenticodeSignature` → `NotSigned`，版本资源 `1.0.2`；静默卸载退出码 0，卸载项/程序文件/开始菜单
  与桌面快捷方式全部清除（仅保留空的本地数据目录，符合 §5 预期）；冒烟前临时移走的用户 `%APPDATA%\SnipAsk`、
  `%LOCALAPPDATA%\SnipAsk`、WebView2 数据目录在冒烟后全部恢复（`restored=true`，真实用户数据未被破坏）；
- 冒烟证据（完整 JSON、安装目录文件清单+SHA256、启动截图）：`.verify/v1/installer/`
  （`smoke-slim.json`、`smoke-full.json`、`screenshots/launch-slim.png`、`screenshots/launch-full.png`）。

## 9 已知限制

- 安装包**未签名**（见 §7）；未提供自动更新、不含遥测（设计如此，见 §6）；
- **关闭窗口 / `Alt+F4` 是隐藏到托盘而非退出**（v1.0.1 交互设计）：卸载前请先用托盘菜单退出应用；
- 卸载后 `%LOCALAPPDATA%\SnipAsk` 可能保留空目录/用户数据（安装目录与本地数据目录同名的历史设计）；
- **slim** 首次使用 OCR 需联网下载约 16 MB 模型（det/rec/cls + 字典）；离线环境请用 **full**；
- 本地 GPU 推理已在本机 NVIDIA RTX 4060 Laptop 真机验证（Vulkan 优先），**AMD/Intel 真机未测**；
- 跨环境验收缺口：Win10 干净机、火绒/360 等国产杀软、SmartScreen 交互 UI、真人可用性未覆盖；
- CI 仅做双版本构建 + 体积门禁（GitHub runner 无真实交互桌面），真实安装/启动/卸载冒烟由本地流水线覆盖并归档证据。

## 10 从源码复现

```powershell
pnpm install
pnpm typecheck
pnpm test
pnpm build:web
powershell -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1   # 准备 OCR 资产（SHA256 校验）
powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1   # 双版本构建 + 门禁 + 安装冒烟 + release 归档
Get-Content .\release\SHA256SUMS.txt
```

- 只想构建单个版本：`... scripts\build-installers.ps1 -Edition slim`（或 `-Edition full`）；
- 只跑冒烟（复用 `release/` 已有安装包）：`... scripts\build-installers.ps1 -SkipBuild`；
- 完整门禁（Rust fmt/clippy/test + 前端 + 安装包）：见仓库根 `scripts/verify.ps1` 与 `README.md`。

## 11 许可

安装包与源码同受 [Apache-2.0](../LICENSE) 约束；第三方组件与模型许可见 [NOTICE](../NOTICE)。
full 版内置的 OCR 模型/字典来自 PaddleOCR / RapidOCR 官方资产（Apache-2.0），许可与归属见 `NOTICE`；
slim 版下载的模型同样遵循各自模型卡条款。
