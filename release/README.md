# 框选 AI（Snip Ask）· v0.1.0 发行说明

本目录是 **v0.1.0（骨架模式 MVP）** 的发行产物归档。安装包由仓库源码在本地 Windows 环境构建，
与该版本源码同一提交入库，可直接安装体验。

## 1. 发行产物

| 文件 | 大小 | SHA256 |
| --- | --- | --- |
| [`SnipAsk_0.1.0_x64-setup.exe`](SnipAsk_0.1.0_x64-setup.exe) | 2,810,494 B（2.68 MiB） | `066347985d081db59438e926f1302c99304696e5c3cd37921811a27eeed7de25` |
| [`SHA256SUMS.txt`](SHA256SUMS.txt) | — | 上述校验和的机器可读副本 |

- 产物类型：NSIS 安装包（Windows x64），`installMode=currentUser`（当前用户安装，不需要管理员权限、不写 HKLM）；
- 版本：`0.1.0`（`package.json` / `src-tauri/tauri.conf.json` / 安装包版本资源一致）；
- 签名状态：**未签名**（`Get-AuthenticodeSignature` → `NotSigned`），预期内，见 §5；
- 安装包内含应用与内置资源（提示词、清单、定价），**不含** OCR/本地推理模型与 GPU 后端组件（由应用内按需下载）。

## 2. 校验下载完整性

```powershell
# 方式一：与 SHA256SUMS.txt 比对
Get-FileHash .\release\SnipAsk_0.1.0_x64-setup.exe -Algorithm SHA256
Get-Content .\release\SHA256SUMS.txt

# 方式二：certutil
certutil -hashfile .\release\SnipAsk_0.1.0_x64-setup.exe SHA256
```

预期值：`066347985d081db59438e926f1302c99304696e5c3cd37921811a27eeed7de25`。
两者不一致时**不要运行**该安装包，重新获取。

## 3. 安装与卸载

### 安装

```powershell
# 交互式：双击 SnipAsk_0.1.0_x64-setup.exe，按向导完成
# 静默安装（当前用户）：
Start-Process .\release\SnipAsk_0.1.0_x64-setup.exe -ArgumentList '/S' -Wait
```

- 默认安装目录：`%LOCALAPPDATA%\SnipAsk\`（安装 `snip-ask.exe` 与 `uninstall.exe`）；
- 安装后创建开始菜单与桌面快捷方式，注册 HKCU 卸载项（`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\SnipAsk`）；
- 安装完成后从快捷方式或 `%LOCALAPPDATA%\SnipAsk\snip-ask.exe` 启动。

### 卸载

```powershell
# 交互式：设置 → 应用 → 已安装的应用 → SnipAsk → 卸载
# 静默卸载：
& "$env:LOCALAPPDATA\SnipAsk\uninstall.exe" /S
```

卸载会删除程序文件、卸载器、开始菜单/桌面快捷方式与卸载注册项，但**保留用户数据**（见 §4）。

## 4. 数据目录

| 内容 | 路径 | 卸载后 |
| --- | --- | --- |
| 配置 / 日志 / 历史库 | `%APPDATA%\SnipAsk\` | 保留 |
| 本地模型 / 缓存 / 运行时 | `%LOCALAPPDATA%\SnipAsk\`（与默认安装目录同名） | 保留，可能残留 `models/cache/runtime` |
| WebView2 用户数据 | `%LOCALAPPDATA%\com.snipask.app\` | 静默卸载默认保留 |

彻底清理：先卸载，再手动删除以上目录。便携模式（程序目录存在 `portable.flag`）下数据写入程序目录旁 `data\`。

> 已知设计问题：默认安装目录 `%LOCALAPPDATA%\SnipAsk` 与应用本地数据目录同名，卸载时 NSIS 的非递归
> `RMDir` 会因数据存在而失败，留下目录。后续版本建议改为 `%LOCALAPPDATA%\Programs\SnipAsk`（见
> [S-16 记录 §6.1](../docs/spikes/S-16-无签名安装包与杀软误报.md)）。

## 5. 未签名与 SmartScreen

当前安装包未做代码签名。从网络下载后首次运行时，Windows SmartScreen 可能提示
「Windows 已保护你的电脑」（未知发布者）：

1. 确认 SHA256 与 §2 一致（来源可信）；
2. 点击「更多信息」→「仍要运行」。

本机 Windows 11 + Windows Defender（实时防护开启）实测：Defender 自定义扫描无威胁、静默安装/启动/卸载均正常。
覆盖矩阵与未测环境（Win10 干净机、火绒等）见 [S-16 验证记录](../docs/spikes/S-16-无签名安装包与杀软误报.md)。

## 6. 构建与验证信息

| 项 | 值 |
| --- | --- |
| 构建命令 | `pnpm build`（内部依次 `pnpm build:web` → `cargo build --release` → NSIS 打包） |
| 构建时间 | 2026-11-05（Asia/Shanghai） |
| Rust | 1.99.1（MSVC 工具链，VS 2022 Build Tools） |
| Tauri CLI | 2.12.4（Tauri 2，`custom-protocol` feature 已启用） |
| Node.js / pnpm | 24.14.1 / 11.21.0 |
| NSIS | Tauri 内置 makensis（`%LOCALAPPDATA%\tauri\NSIS`） |
| 打包目标 | `nsis`，语言 SimpChinese + English，用户级安装 |
| 构建退出码 | 0（`release profile [optimized] … 2m 35s`） |

发布前质量门禁（同一源码树，2026-11-05）：

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS（0 warning） |
| `cargo test` | PASS：**376 passed / 0 failed / 6 ignored**（16 个测试套件） |
| `pnpm typecheck` | PASS |
| `pnpm test`（覆盖布局 25 + 线程模型 7） | PASS |
| `pnpm build:web` | PASS（120 modules） |
| 安装/卸载冒烟（本次发行包） | PASS（见下） |

本次发行包冒烟结果（Windows 11 x64，当前用户）：

- 静默安装 `/S` → 退出码 0，安装目录出现 `snip-ask.exe`（7,489,536 B）与 `uninstall.exe`，注册 HKCU 卸载项；
- 启动应用 6s → 进程存活、主窗口标题「框选 AI」、响应正常、工作集 40.1 MiB；
- 静默卸载 `/S` → 退出码 0，程序文件、卸载项、开始菜单/桌面快捷方式全部清除；
- 安装包签名状态 `NotSigned`，版本资源 `0.1.0`。

## 7. 已知限制

- **骨架模式 MVP**：可完整走通「`Ctrl+Q` → 冻结 → 框选 → 动作菜单 → 流式结果」UI 闭环，但真实 Win32 捕获、
  RapidOCR/ONNX 推理、云端 OpenAI 兼容网关、Windows Credential Manager 等仍为 `TODO(real)` 占位；
- 本地 GPU 推理已在 NVIDIA RTX 4060 Laptop 真机验证（Vulkan 优先），**AMD/Intel 真机未测**；
- 安装包未签名、未提供自动更新通道（`tauri-plugin-updater` 未接）；
- 覆盖翻译的真实 OCR/翻译质量依赖前置能力接入；连续提问历史持久化未做；
- 跨环境验收缺口：Win10 干净机、火绒/360 等国产杀软、SmartScreen 交互 UI、真人可用性（S-19）未验证。

完整状态矩阵见仓库根 [README「当前状态」](../README.md#当前状态)；技术验证结论见
[`docs/技术验证清单.md`](../docs/技术验证清单.md) 与 [`docs/spikes/`](../docs/spikes/README.md)。

## 8. 从源码复现

```powershell
pnpm install
pnpm typecheck
pnpm test
powershell -ExecutionPolicy Bypass -File scripts\verify.ps1   # 全量门禁（需 Rust）
pnpm build                                                    # 生成安装包
Get-FileHash .\src-tauri\target\release\bundle\nsis\SnipAsk_0.1.0_x64-setup.exe -Algorithm SHA256
```

> 复现出的安装包哈希不一定与仓库归档完全一致（NSIS 打包时间戳/压缩顺序等因素），
> 以「同一源码版本、可安装可运行」为准；发布归档以本目录 SHA256 为校验基准。

## 9. 许可

安装包与源码同受 [Apache-2.0](../LICENSE) 约束；第三方组件与模型许可见 [NOTICE](../NOTICE)。
本地模型不随安装包分发，由用户在应用内按需下载并遵守各自模型卡条款。
