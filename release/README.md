# 框选 AI（Snip Ask）· v1.0.4 发行说明

本目录是 **v1.0.4（用户反馈修复版）** 的双版本发行产物归档，由仓库源码在本地 Windows 环境构建，
与该版本源码同一提交入库，可直接安装使用。

> v1.0.3 与更早（v1.0.2 / v1.0.1 / v1.0.0 / v0.1.0 早期预览版）的安装包**已从本目录移除**；历史版本请见仓库提交历史。
> 本软件**不包含自动更新、也不包含遥测/使用统计上报**：升级方式 = 下载新版安装包覆盖安装，
> 或按《自定义指南》自行源码构建。

## 0 v1.0.4 主要变化（相对 v1.0.3，用户反馈修复）

- **本地翻译「翻译后仍然为原文」修复**：用户反馈本地翻译（Qwen2.5-0.5B 等）试译后仍显示英文原文。根因是本地精简提示词
  把 `zh-CHS` 这类 BCP-47 语言码直接发给小模型，真实本地模型不认识语言码、**原样复述原文**；非空结果又被当作成功，
  不触发云端降级也不报错。修复：
  - 本地精简提示词与覆盖翻译提示词把语言码渲染为中文语言名（如 `zh-CHS` → `简体中文`）；云端 `translate.md` 不变；
  - 术语表非空时改用 `translate_minimal_glossary.md`（术语表与指令同一段，避免小模型复述术语表行）；
  - 新增复述检测：本地结果与原文完全相同且不含目标语言文字时判 `SA-1099`，自动降级云端/返回可操作错误；
    试译、框选翻译、覆盖翻译三条路径均接入；本地翻译任务改为缓冲校验后再推送前端。
  - **真实 App 命令路径验证 PASS**（`.verify/v1/fix/v104-local-translate.json`）：Vulkan / qwen2.5-0.5b-instruct-q4 /
    `zh-CHS`，`The quick brown fox jumps over the lazy dog.` → `那只敏捷的棕色狐狸跳过了那只懒惰的狗。`（788ms），
    第二句同样为中文译文；三模型提示词矩阵 9/9 不再回显原文。
- **卸载程序无法完全卸载（含已下载模型）修复**：Tauri NSIS 内置「删除应用数据」只清理
  `%APPDATA%\com.snipask.app` / `%LOCALAPPDATA%\com.snipask.app`（BUNDLEID），不清理本应用的
  `%APPDATA%\SnipAsk`（配置/历史/日志/提示词/后端）与 `%LOCALAPPDATA%\SnipAsk`（模型/缓存）→ 模型和后端残留。修复：
  新增 `src-tauri/nsis/installer-hooks.nsh`（`bundle.windows.nsis.installerHooks`）：
  - 静默 `/S` → 彻底清理；勾选「删除应用数据」→ 清理；交互未勾选 → 额外确认（默认「是」，选「否」保留）；
  - 清理 `%APPDATA%\SnipAsk`、`%LOCALAPPDATA%\SnipAsk`（含 `models/`、`backends/`、`runtime/`）、`com.snipask.app`
    与 HKCU Run / 安装器元数据 / 卸载注册项；
  - 覆盖安装 `/UPDATE` **绝不删除用户数据**；删除前 `SetOutPath "$TEMP"` 解锁同名安装目录，被占用文件以 `/REBOOTOK` 兜底；
  - 安装冒烟新增模型探针与目录残留断言并纳入 pass 门禁（`dataRemoved=true / dataResidue=[]`）。
- 继承 v1.0.3：本地 HY-MT1.5-1.8B 改用精简提示词；继承 v1.0.2：结果窗不透明铺满/高度自适应、引擎旧提示清理、
  自绘标题栏按钮权限；继承 v1.0.1：关闭到托盘、下载 `idle`、真实内存值、引擎启停、去遥测（schema v6）。
- 版本 `1.0.4`；其余能力（真实捕获/OCR/云端推理/凭据/历史/双安装包）与 v1.0.3 一致；**契约 v6 不变**
  （43 命令 / 15 事件 / 配置 Schema 6 / `IPC_CONTRACT_VERSION=1`）。

## 1 发行产物

| 文件 | 版本类型 | 大小 | SHA256 |
| --- | --- | --- | --- |
| [`SnipAsk_1.0.4_x64-setup.exe`](SnipAsk_1.0.4_x64-setup.exe) | **slim**（安装后下载版） | 7,784,602 B（7.42 MiB） | `7269e25bc75f76fc786357fe815f13c8610e6c406d78c515776d8bd0cf9efcdd` |
| [`SnipAsk_1.0.4_x64-full-setup.exe`](SnipAsk_1.0.4_x64-full-setup.exe) | **full**（内置 OCR 模型版） | 22,567,768 B（21.52 MiB） | `21944e24504edc98760d8514a6a5085d30d6d8a6e5cd5f716cfe422a67bf8264` |
| [`SHA256SUMS.txt`](SHA256SUMS.txt) | — | — | 上述两个安装包的机器可读校验和 |

- 产物类型：NSIS 安装包（Windows x64），`installMode=currentUser`（当前用户安装，不需要管理员权限、不写 HKLM）；
- 版本：`1.0.4`（`package.json` / `Cargo.toml` / `tauri.conf.json` / 安装包版本资源一致）；
- 签名状态：**未签名**（`Get-AuthenticodeSignature` → `NotSigned`），预期内，见 §7；
- 两个版本都内置：应用本体、ONNX Runtime 1.31.1 运行时（`ocr/onnxruntime.dll`）、提示词、模型/定价清单，
  以及安装目录 `docs/`（README、自定义指南、LICENSE、NOTICE、SECURITY、PRIVACY）。

## 2 双版本差异（slim vs full）

| 能力 | slim（安装后下载版） | full（内置 OCR 模型版） |
| --- | --- | --- |
| 应用本体 + 文档 | 内置 | 内置 |
| ONNX Runtime 运行时 `ocr/onnxruntime.dll` | **内置** | **内置** |
| OCR 模型（det / rec / cls + `ppocr_keys_v1.txt`） | **不内置**，首次使用 OCR 时下载（≈16 MB） | **内置**（离线开箱即用） |
| 安装包体积（本次实测） | **7.42 MiB**（7,784,602 B） | **21.52 MiB**（22,567,768 B） |
| 适用场景 | 体积敏感、首次 OCR 可联网 | 离线 / 内网 / 不想额外下载 |

- slim 下载的模型落盘到 `%LOCALAPPDATA%\SnipAsk\models\ocr\`（文件名固定：`ch_PP-OCRv4_det_infer.onnx`、
  `ch_PP-OCRv4_rec_infer.onnx`、`ch_ppocr_mobile_v2.0_cls_infer.onnx`、`ppocr_keys_v1.txt`），下载后离线可用；
- 两者运行期解析顺序一致：随包 `<安装目录>\ocr\` → 数据目录 `%LOCALAPPDATA%\SnipAsk\models\ocr\` → 开发钩子 `SNIP_ASK_OCR_DIR`；
- 应用内「设置 → 自定义与关于」会显示当前安装包类型（内置 OCR 模型版 / 在线安装版 / 开发构建）与 OCR 就绪状态。

## 3 校验下载完整性

```powershell
# 方式一：与 SHA256SUMS.txt 比对
Get-Content .\release\SHA256SUMS.txt
Get-FileHash .\release\SnipAsk_1.0.4_x64-setup.exe -Algorithm SHA256
Get-FileHash .\release\SnipAsk_1.0.4_x64-full-setup.exe -Algorithm SHA256

# 方式二：certutil
certutil -hashfile .\release\SnipAsk_1.0.4_x64-setup.exe SHA256
certutil -hashfile .\release\SnipAsk_1.0.4_x64-full-setup.exe SHA256
```

预期值与 §1 表格一致。两者不一致时**不要运行**该安装包，重新获取。

> 复现构建出的安装包哈希不一定与仓库归档完全一致（NSIS 打包时间戳/压缩顺序等因素），
> 以「同一源码版本、可安装可运行」为准；发布归档以本目录 `SHA256SUMS.txt` 为校验基准。

## 4 安装与卸载

### 安装

```powershell
# 交互式：双击对应版本的 setup.exe，按向导完成
# 静默安装（当前用户，任选 slim/full 之一）：
Start-Process .\release\SnipAsk_1.0.4_x64-setup.exe -ArgumentList '/S' -Wait
Start-Process .\release\SnipAsk_1.0.4_x64-full-setup.exe -ArgumentList '/S' -Wait
```

- 默认安装目录：`%LOCALAPPDATA%\SnipAsk\`（安装 `snip-ask.exe`、`uninstall.exe`、`ocr\` 与 `docs\`）；
- 安装后创建开始菜单与桌面快捷方式，注册 HKCU 卸载项（`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SnipAsk`）；
- 安装完成后从快捷方式或 `%LOCALAPPDATA%\SnipAsk\snip-ask.exe` 启动；首次启动按引导完成语言/供应商/快捷键设置。

### 卸载（v1.0.4 起可选彻底清理数据）

```powershell
# 交互式：设置 → 应用 → 已安装的应用 → SnipAsk → 卸载
#   卸载确认页勾选「删除应用数据」→ 同时删除配置/历史/日志/提示词/后端/模型/缓存；
#   若不勾选，卸载末尾仍会弹一次确认（默认「是」= 删除，选「否」= 仅删程序、保留数据）。
# 静默卸载（彻底清理，推荐用于干净重装）：
& "$env:LOCALAPPDATA\SnipAsk\uninstall.exe" /S
```

- 卸载会删除程序文件、`ocr\`/`docs\`、卸载器、开始菜单/桌面快捷方式与卸载注册项；
- v1.0.4 卸载钩子在「删除应用数据 / 静默 / 交互确认」时清理 `%APPDATA%\SnipAsk`、`%LOCALAPPDATA%\SnipAsk`
  （含已下载模型与 GPU/CPU 后端）及 `com.snipask.app`（WebView2），**不再残留已下载模型**；
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
**移除遥测/使用统计上报**（配置 schema 升至 v6，无任何使用数据外发），我们鼓励你自行修改：

- 应用内入口：**设置 → 自定义与关于**（版本 / 版本类型 / OCR 就绪状态；一键打开配置、提示词、模型、日志、
  后端、安装文档目录；并说明本软件不会自动更新、不上报遥测）；
- 可改点（详见安装目录 `docs\自定义指南.md`）：
  - 提示词覆盖：`%APPDATA%\SnipAsk\prompts\*.md`（7 个模板：回答/翻译/翻译精简/翻译精简术语表/搜索/追问等）；
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
| 构建时间 | 2026-11-06 14:00–14:05（Asia/Shanghai；两次 release 编译 + NSIS 打包） |
| Rust | 1.99.1（MSVC 工具链，VS 2022 Build Tools） |
| Tauri CLI | 2.12.4（Tauri 2，`custom-protocol` feature 已启用） |
| Node.js / pnpm | 24.14.1 / 11.21.0 |
| NSIS | Tauri 内置 makensis 3.13（`%LOCALAPPDATA%\tauri\NSIS`）；使用 `installerHooks` 卸载钩子 |
| 打包目标 | `nsis`，语言 SimpChinese + English，用户级安装 |
| 体积门禁 | slim ≤ 25 MiB、full ≤ 60 MiB（`scripts/build-installers.ps1` 内置，超限失败） |
| OCR 资产 | `scripts/fetch-ocr-assets.ps1` 准备：本机缓存 → `resources/models.manifest.json` 镜像 → 官方镜像；SHA256 强校验 |

发布前质量门禁（同一源码树，2026-11-06，Lead 统一执行）：

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS（0 warning） |
| `cargo test --all-targets` | PASS：**495 passed / 0 failed / 20 ignored**（lib 356；新增 11 个 v1.0.4 单元测试：语言名映射、提示词分支、复述检测、降级/覆盖翻译） |
| `pnpm typecheck` | PASS |
| `pnpm test`（前端纯逻辑） | PASS：**48 / 48** |
| `pnpm build:web` | PASS（131 modules） |
| **真实 App 命令路径（v1.0.4 本地翻译）** | **PASS**：applab 便携 debug（`app_stats.version=1.0.4`），`translate.backend=local` + qwen2.5-0.5b + `localOnly=true` + `zh-CHS`；`translate_test` 两条输入均输出中文译文（788ms / 768ms，`sameAsInput=false`）；Vulkan、offload 25 层；证据 `.verify/v1/fix/v104-local-translate.{json,md}` |
| 本地模型提示词矩阵（0.5B / 1.5B / HY-MT1.8B） | **PASS**：极简/术语表/英文目标共 9 组全部正确翻译、不回显原文、不输出术语表行；旧提示词在 0.5B/1.5B 上复现原回显问题；证据 `.verify/v1/fix/v104-local-prompt-matrix.json` |
| 双版本构建 + 体积门禁（`scripts/build-installers.ps1`） | PASS：slim 7.42 MiB / full 21.52 MiB |
| edition 注入自检（slim/full 主程序 SHA256 不同） | PASS |
| 双安装包真实安装/启动/卸载 + **数据清理断言** | PASS ×2（0 关键失败；`dataRemoved=true`、`dataResidue=[]`、卸载目录/注册项无残留、用户真实数据冒烟后恢复） |

本次双安装包真实冒烟结果（Windows 11 x64 build 26200，currentUser，静默 `/S` 安装 → 启动 6s → 关闭隐藏到托盘 → 写入模型探针 → 静默卸载 `/S`）：

- slim：安装退出码 0（3.4s）；安装目录 9 个文件、27,400,327 B = `snip-ask.exe`（7,967,744 B，版本资源 1.0.4）
  + `uninstall.exe` + `ocr\onnxruntime.dll`（19,288,888 B，SHA256 与构建源一致）+ `docs\` 6 个文档，**无任何模型文件**；
  启动 6s 后进程存活、主窗口「框选 AI」可见、工作集 41.5 MB；截图见 `screenshots/launch-slim.png`；
- full：安装退出码 0（4.4s）；安装目录 13 个文件、43,615,680 B = slim 全部 + `ocr\` 下 det/rec/cls/`ppocr_keys_v1.txt`，
  4 个模型/字典 SHA256 与 `src-tauri/resources/ocr/` 源文件一致；启动后主窗口可见、工作集 37.5 MB；
- **关闭行为**：对运行中的应用发送关闭窗口（`WM_CLOSE`）后进程仍存活 → `closeBehavior=hidden-to-tray`；
- 两版：`Get-AuthenticodeSignature` → `NotSigned`，版本资源 `1.0.4`；静默卸载退出码 0（约 1.5s），
  卸载项/程序文件/开始菜单/桌面快捷方式全部清除，**安装目录不存在、`%LOCALAPPDATA%\SnipAsk` / `%APPDATA%\SnipAsk` /
  `com.snipask.app` 与模型探针全部不存在（`dataRemoved=true`、`dataResidue=[]`、注册项无残留）**；
- 卸载前临时移走的用户 `%APPDATA%\SnipAsk`、`%LOCALAPPDATA%\SnipAsk`、WebView2 数据目录在冒烟后全部恢复
  （`appDataSnipAskRestored=true`，用户真实模型 491 MB / 后端 106 MB 核对无变化）；
- 冒烟证据（完整 JSON、安装目录文件清单+SHA256、启动截图）：`.verify/v1/installer/`
  （`smoke-slim.json`、`smoke-full.json`、`screenshots/launch-slim.png`、`screenshots/launch-full.png`）。

## 9 已知限制

- 安装包**未签名**（见 §7）；未提供自动更新、不含遥测（设计如此，见 §6）；
- **关闭窗口 / `Alt+F4` 是隐藏到托盘而非退出**（v1.0.1 交互设计）：卸载前请先用托盘菜单退出应用；
- **slim** 首次使用 OCR 需联网下载约 16 MB 模型（det/rec/cls + 字典）；离线环境请用 **full**；
- 本地 GPU 推理已在本机 NVIDIA RTX 4060 Laptop 真机验证（Vulkan 优先），**AMD/Intel 真机未测**；
- 本地小模型（Qwen2.5-0.5B、HY-MT1.5-1.8B 等）v1.0.4 起不再回显原文，但译文质量/格式保持仍弱于云端
  （1.8B 能力边界，与 S-14 结论一致）；建议重要内容用云端翻译，离线场景用本地；
- 本地复述检测刻意保守：仅「与原文完全相同且不含目标语言文字」才判失败，轻微改写/加标点不会判；
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
  冒烟会先备份用户真实数据，结束后恢复；静默卸载路径会断言「数据目录 + 模型探针」全部清理，
  交互卸载（勾选复选框 / 未勾选确认框 Yes/No）为人工验证项；
- 完整门禁（Rust fmt/clippy/test + 前端 + 安装包）：见仓库根 `scripts/verify.ps1` 与 `README.md`。

## 11 许可

安装包与源码同受 [Apache-2.0](../LICENSE) 约束；第三方组件与模型许可见 [NOTICE](../NOTICE)。
full 版内置的 OCR 模型/字典来自 PaddleOCR / RapidOCR 官方资产（Apache-2.0），许可与归属见 `NOTICE`；
slim 版下载的模型同样遵循各自模型卡条款。
