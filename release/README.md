# 框选 AI（Snip Ask）· v1.1.0 发行说明

本目录是 **v1.1.0（功能完整性与英文界面版）** 的双版本发行产物归档，由仓库源码在本地 Windows 环境构建，
与该版本源码同一提交入库，可直接安装使用。

> v1.0.6 与更早（v1.0.5～v0.1.0）的安装包**已从本目录移除**；历史版本见仓库提交历史。
> 本软件**不包含自动更新、也不包含遥测/使用统计上报**：升级方式 = 下载新版安装包覆盖安装，
> 或按《自定义指南》自行源码构建。**v1.1.0 起支持中英双语界面**（设置 → 核心 → 界面语言）。

## 0 v1.1.0 主要变化（相对 v1.0.6）

- **英文界面完成**：主窗口 / 设置 / 首次引导 / 历史 / 框选层 / 结果窗全部前端文案支持英文；
  设置里切换语言后所有窗口自动重载生效。英文覆盖 818 个静态文案（`scripts/check-i18n.mjs` 门禁）；
  后端动态错误文本仍为中文（见 §9 限制）。
- **设置项全部生效**（此前 UI 存在但后端未使用）：
  - `app.startMinimized`：非首启时按配置隐藏到托盘 / 显示主窗口（首启仍显示引导）；
  - `actions.defaultAction`：框选后自动执行默认动作（热键触发的动作优先，失败回退菜单）；
  - `ui.resultWindow.*`：结果窗初始宽高、置顶、位置策略（跟随选区 / 固定居中）、内容不透明度；
  - `ocr.preserveBoxes`：关闭后不再返回文字框（覆盖翻译随之不可用）。
- **provider caps 真正生效**：`vision` / `search` / `thinking` / `translate` 参与动作可用性与引擎选择；
  AI 搜索需 `search`、图片搜索需 `vision`、云端翻译需 `translate`（Qwen-MT 亦视为可翻译）、
  深度解答优先 `thinking`；云端翻译降级链只含可翻译供应商。
- **配置 Schema 6 → 7**：自动迁移给 `kind=openAiCompatible`（含 DeepSeek/缺省 kind）的 provider
  补 `translate` cap，升级后云端翻译不会被误禁；幂等、不覆盖 Qwen-MT 用户配置。
- **`advanced.proxy` 接入**：LLM 网关、搜索/工具网关、provider_test、模型与后端下载统一走进程级代理；
  回环地址始终直连；支持 `http(s)://[user:pass@]host:port`，非法值保存返回 `SA-5001`，需重启生效。
- **图像搜索隐私修复延续**（v1.0.6）：只上传**框选区域**截图，不再上传整张桌面。
- 继承 v1.0.5/v1.0.4：复制文字只复制原文并原样回显、`autoCopy`、OCR 合并框行拆分、
  本地翻译语言名 + 复述检测、卸载彻底清理模型/配置/后端。
- **契约**：配置 Schema **7**；命令仍 **43** / 事件仍 **15** / `IPC_CONTRACT_VERSION` 仍 **1**。

## 1 发行产物

| 文件 | 版本类型 | 大小 | SHA256 |
| --- | --- | --- | --- |
| [`SnipAsk_1.1.0_x64-setup.exe`](SnipAsk_1.1.0_x64-setup.exe) | **slim**（安装后下载版） | 7,816,641 B（7.45 MiB） | `debe34a4cf697eeee2b70461feb9401c6d56e9ee7ba522399a400211e59d8fc8` |
| [`SnipAsk_1.1.0_x64-full-setup.exe`](SnipAsk_1.1.0_x64-full-setup.exe) | **full**（内置 OCR 模型版） | 22,606,726 B（21.56 MiB） | `f6b807138a2d692cb08a9343c409cb194fca544b4950df53fc2c1ec599445304` |
| [`SHA256SUMS.txt`](SHA256SUMS.txt) | — | — | 上述两个安装包的机器可读校验和 |

- 产物类型：NSIS 安装包（Windows x64），`installMode=currentUser`（当前用户安装，不需要管理员权限、不写 HKLM）；
- 版本：`1.1.0`（`package.json` / `Cargo.toml` / `tauri.conf.json` / 安装包版本资源一致）；
- 签名状态：**未签名**（`Get-AuthenticodeSignature` → `NotSigned`），预期内，见 §7；
- 两个版本都内置：应用本体、ONNX Runtime 1.31.1 运行时（`ocr/onnxruntime.dll`）、提示词、模型/定价清单，
  以及安装目录 `docs/`（README、自定义指南、LICENSE、NOTICE、SECURITY、PRIVACY）。

## 2 双版本差异（slim vs full）

| 能力 | slim（安装后下载版） | full（内置 OCR 模型版） |
| --- | --- | --- |
| 应用本体 + 文档 | 内置 | 内置 |
| ONNX Runtime 运行时 `ocr/onnxruntime.dll` | **内置** | **内置** |
| OCR 模型（det / rec / cls + `ppocr_keys_v1.txt`） | **不内置**，首次使用 OCR 时下载（≈16 MB） | **内置**（离线开箱即用） |
| 安装包体积（本次实测） | **7.45 MiB**（7,816,641 B） | **21.56 MiB**（22,606,726 B） |
| 适用场景 | 体积敏感、首次 OCR 可联网 | 离线 / 内网 / 不想额外下载 |

- slim 下载的模型落盘到 `%LOCALAPPDATA%\SnipAsk\models\ocr\`（文件名固定：`ch_PP-OCRv4_det_infer.onnx`、
  `ch_PP-OCRv4_rec_infer.onnx`、`ch_ppocr_mobile_v2.0_cls_infer.onnx`、`ppocr_keys_v1.txt`），下载后离线可用；
- 两者运行期解析顺序一致：随包 `<安装目录>\ocr\` → 数据目录 `%LOCALAPPDATA%\SnipAsk\models\ocr\` → 开发钩子 `SNIP_ASK_OCR_DIR`；
- 应用内「设置 → 自定义与关于」会显示当前安装包类型（内置 OCR 模型版 / 在线安装版 / 开发构建）与 OCR 就绪状态。

## 3 校验下载完整性

```powershell
Get-Content .\release\SHA256SUMS.txt
Get-FileHash .\release\SnipAsk_1.1.0_x64-setup.exe -Algorithm SHA256
Get-FileHash .\release\SnipAsk_1.1.0_x64-full-setup.exe -Algorithm SHA256
certutil -hashfile .\release\SnipAsk_1.1.0_x64-setup.exe SHA256
```

预期值与 §1 表格一致。两者不一致时**不要运行**该安装包，重新获取。

> 复现构建出的安装包哈希不一定与仓库归档完全一致（NSIS 打包时间戳/压缩顺序等因素），
> 以「同一源码版本、可安装可运行」为准；发布归档以本目录 `SHA256SUMS.txt` 为校验基准。

## 4 安装与卸载

### 安装

```powershell
# 交互式：双击对应版本的 setup.exe，按向导完成
# 静默安装（当前用户，任选 slim/full 之一）：
Start-Process .\release\SnipAsk_1.1.0_x64-setup.exe -ArgumentList '/S' -Wait
Start-Process .\release\SnipAsk_1.1.0_x64-full-setup.exe -ArgumentList '/S' -Wait
```

- 默认安装目录：`%LOCALAPPDATA%\SnipAsk\`（安装 `snip-ask.exe`、`uninstall.exe`、`ocr\` 与 `docs\`）；
- 安装后创建开始菜单与桌面快捷方式，注册 HKCU 卸载项（`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SnipAsk`）；
- 安装完成后从快捷方式或 `%LOCALAPPDATA%\SnipAsk\snip-ask.exe` 启动；首次启动按引导完成语言/供应商/快捷键设置。

### 卸载（v1.0.4 起可选彻底清理数据；v1.1.0 保持）

```powershell
# 交互式：设置 → 应用 → 已安装的应用 → SnipAsk → 卸载
#   卸载确认页勾选「删除应用数据」→ 同时删除配置/历史/日志/提示词/后端/模型/缓存；
#   若不勾选，卸载末尾仍会弹一次确认（默认「是」= 删除，选「否」= 仅删程序、保留数据）。
# 静默卸载（彻底清理，推荐用于干净重装）：
& "$env:LOCALAPPDATA\SnipAsk\uninstall.exe" /S
```

- 卸载会删除程序文件、`ocr\`/`docs\`、卸载器、开始菜单/桌面快捷方式与卸载注册项；
- 删除数据时清理 `%APPDATA%\SnipAsk`、`%LOCALAPPDATA%\SnipAsk`（含已下载模型与 GPU/CPU 后端）及
  `com.snipask.app`（WebView2），**不残留已下载模型**；
- 覆盖安装（更新）不会删除任何用户数据；卸载前建议先通过托盘菜单「退出」应用，避免文件占用。

## 5 数据目录

| 内容 | 路径 | 卸载（选择删除数据 / 静默）后 |
| --- | --- | --- |
| 配置 / 日志 / 提示词覆盖 / 本地后端 | `%APPDATA%\SnipAsk\` | 删除 |
| 历史会话库 | `%APPDATA%\SnipAsk\history.db` | 删除 |
| OCR 模型 / 本地推理模型 / 缓存 / 运行时（与默认安装目录同名） | `%LOCALAPPDATA%\SnipAsk\` | 删除（含 `models\`、`backends\`、`runtime\`） |
| WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` | 删除 |

- 交互卸载未勾选「删除应用数据」且确认框选「否」时，上述数据保留，便于重装后继续使用；
- 便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`，不在本安装包卸载钩子范围内（手动删除程序目录即可）。

## 6 自定义与关于（无自动更新 · 无遥测）

自 v1.0.0 起**移除全部自动更新能力**（无 updater、无更新通道、无联网检查更新）；v1.0.1 起进一步
**移除遥测/使用统计上报**（配置 schema 升级后无任何使用数据外发），我们鼓励你自行修改：

- 应用内入口：**设置 → 自定义与关于**（版本 / 版本类型 / OCR 就绪状态；一键打开配置、提示词、模型、日志、
  后端、安装文档目录；并说明本软件不会自动更新、不上报遥测）；
- 可改点（详见安装目录 `docs\自定义指南.md`）：
  - 提示词覆盖：`%APPDATA%\SnipAsk\prompts\*.md`（7 个模板）；
  - 术语表、自动复制、默认动作、每个动作的快速/深度模式；
  - 全局热键（6 个真实可用项）：`capture` / `quickTranslate` / `imageSearch` / `answer` /
    `togglePause` / `openSettings`；
  - provider 能力 caps（`vision`/`search`/`thinking`/`translate`）与 `advanced.proxy`（HTTP(S)，重启生效）；
  - 本地模型 / 后端替换（`SNIP_ASK_MODEL_PATH` / `SNIP_ASK_BACKENDS_DIR` / `SNIP_ASK_OCR_DIR` 等调试钩子）；
  - 云端供应商接入：DeepSeek、DashScope Qwen-MT、通用 OpenAI 兼容（自定义 baseUrl/model）；
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
| 构建命令 | `powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1` |
| 构建时间 | 2026-11-06 23:08–23:13（Asia/Shanghai） |
| Rust | 1.99.1（MSVC 工具链，VS 2022 Build Tools） |
| Tauri CLI | 2.12.4（Tauri 2，`custom-protocol` feature 已启用） |
| Node.js / pnpm | 24.14.1 / 11.21.0 |
| NSIS | Tauri 内置 makensis 3.13；使用 `installerHooks` 卸载钩子 |
| 体积门禁 | slim ≤ 25 MiB、full ≤ 60 MiB（`scripts/build-installers.ps1` 内置，超限失败） |
| OCR 资产 | `scripts/fetch-ocr-assets.ps1` 准备：本机缓存 → `resources/models.manifest.json` 镜像 → 官方镜像；SHA256 强校验 |

发布前质量门禁（同一源码树，2026-11-06）：

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS（0 warning） |
| `cargo test --all-targets` | PASS：**533 passed / 0 failed / 21 ignored** |
| `pnpm typecheck` | PASS |
| `pnpm test`（前端纯逻辑） | PASS：**48 / 48** |
| `pnpm build:web` | PASS（136 modules） |
| `node scripts/check-i18n.mjs` | PASS：818 个静态 `t()` key 100% 有英文翻译 |
| **真实 App E2E（v1.1.0）** | **PASS**：schema 6→7 迁移 + deepseek 补 translate；英文主窗口/结果窗；`Ctrl+Shift+Y` 快速翻译输出中文译文；结果窗宽 620、内容透明度 0.8；图片搜索/解答热键进入捕获态（`.verify/v1/fix/v110-e2e.json`） |
| 双版本构建 + 体积门禁 | PASS：slim 7.45 MiB / full 21.56 MiB |
| edition 注入自检（slim/full 主程序 SHA256 不同） | PASS |
| 双安装包真实安装/启动/隐藏托盘/卸载 + 数据清理断言 | PASS ×2（0 关键失败 / 0 警告；`dataRemoved=true`、`dataResidue=[]`、卸载目录/注册项无残留、用户真实数据冒烟后恢复） |

本次双安装包真实冒烟结果（Windows 11 x64 build 26200，currentUser，静默 `/S` 安装 → 启动 6s → 关闭隐藏到托盘 → 写入模型探针 → 静默卸载 `/S`）：

- slim：安装退出码 0（3.1s）；安装目录 9 个文件、27,462,194 B = `snip-ask.exe`（8,028,160 B，版本资源 1.1.0）
  + `uninstall.exe` + `ocr\onnxruntime.dll`（19,288,888 B）+ `docs\` 6 个文档，**无任何模型文件**；
  启动后进程存活、主窗口「框选 AI」可见、工作集 36.8 MB；
- full：安装退出码 0（4.4s）；安装目录 13 个文件、43,677,547 B = slim 全部 + `ocr\` 下 det/rec/cls/`ppocr_keys_v1.txt`，
  4 个模型/字典 SHA256 与 `src-tauri/resources/ocr/` 源文件一致；启动后工作集 36.8 MB；
- **关闭行为**：对运行中的应用发送关闭窗口（`WM_CLOSE`）后进程仍存活 → `closeBehavior=hidden-to-tray`；
- 两版：`Get-AuthenticodeSignature` → `NotSigned`，版本资源 `1.1.0`；静默卸载退出码 0（约 1.1s），
  卸载项/程序文件/开始菜单/桌面快捷方式全部清除，**安装目录不存在、`%LOCALAPPDATA%\SnipAsk` / `%APPDATA%\SnipAsk` /
  `com.snipask.app` 与模型探针全部不存在（`dataRemoved=true`、`dataResidue=[]`、注册项无残留）**；
- 卸载前临时移走的用户数据目录在冒烟后全部恢复；
- 冒烟证据：`.verify/v1/installer/`（`smoke-slim.json`、`smoke-full.json`、`screenshots/launch-*.png`）。

## 9 已知限制

- 安装包**未签名**（见 §7）；未提供自动更新、不含遥测（设计如此，见 §6）；
- **关闭窗口 / `Alt+F4` 是隐藏到托盘而非退出**：卸载前请先用托盘菜单退出应用；
- **slim** 首次使用 OCR 需联网下载约 16 MB 模型；离线环境请用 **full**；
- 本地 GPU 推理已在本机 NVIDIA RTX 4060 Laptop 真机验证（Vulkan 优先），**AMD/Intel 真机未测**；
- 本地小模型（Qwen2.5-0.5B、HY-MT1.5-1.8B 等）v1.0.4 起不再回显原文，但译文质量/格式保持仍弱于云端；
- **英文界面覆盖所有前端 UI**；后端动态错误文本（AppError.message/detail）、动作禁用原因的少数动态文案、
  供应商/模型/设备名等数据仍可能显示中文（见 `docs/功能验证清单.md`）；
- 结果窗「不透明度」作用于内容层（窗口保持不透明，避免 WebView2 透明窗口在部分显卡上的闪烁）；
- 图片搜索的选区裁切以单元测试 + 代码路径验收；未用真实 Key 上传图片做端到端复验；
- `advanced.proxy` 仅支持 HTTP(S)（未启用 SOCKS），需重启应用生效；
- 显示器热插拔/分辨率变化不会立即重排框选层（下一次框选会重新对齐）；
- 跨环境验收缺口：Win10 干净机、火绒/360 等国产杀软、SmartScreen 交互 UI、真人可用性未覆盖；
- CI 仅做双版本构建 + 体积门禁，真实安装/启动/卸载冒烟由本地流水线覆盖并归档证据。

## 10 从源码复现

```powershell
pnpm install
pnpm typecheck
pnpm test
pnpm build:web
node scripts/check-i18n.mjs
powershell -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1
powershell -ExecutionPolicy Bypass -File scripts\build-installers.ps1
Get-Content .\release\SHA256SUMS.txt
```

- 只想构建单个版本：`... scripts\build-installers.ps1 -Edition slim`（或 `-Edition full`）；
- 只跑冒烟（复用 `release/` 已有安装包）：`... scripts\build-installers.ps1 -SkipBuild`；
- 完整门禁（Rust fmt/clippy/test + 前端 + i18n + 安装包）：见仓库根 `README.md` 与 `scripts/verify.ps1`。

## 11 许可

安装包与源码同受 [Apache-2.0](../LICENSE) 约束；第三方组件与模型许可见 [NOTICE](../NOTICE)。
full 版内置的 OCR 模型/字典来自 PaddleOCR / RapidOCR 官方资产（Apache-2.0），许可与归属见 `NOTICE`；
slim 版下载的模型同样遵循各自模型卡条款。
