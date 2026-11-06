# S-04 · Ctrl+Q 全局热键注册与冲突检测（辅助脚本）

对应验证项：`docs/技术验证清单.md` S-04 —— 注册成功率 100%；冲突时 100% 给出可操作提示与改键引导；暂停后热键完全释放。

结论记录：`docs/spikes/S-04-CtrlQ全局热键注册与冲突检测.md`（原始数据在 `.verify/spikes/S-04/`，已 gitignore）。

## 脚本

| 脚本 | 用途 |
| --- | --- |
| `hold_hotkey.ps1` | 用 Win32 `RegisterHotKey(NULL, id, MOD_NOREPEAT\|mods, vk)` 占住指定组合键（模拟 QQ/微信/输入法抢占），`-HoldSec 0` 为「仅探测」（注册后立即注销）。输出 JSON 含 Win32 错误码（1409 = 已被占用）与注册耗时。 |
| `press_hotkey.ps1` | 用 `keybd_event` 注入一次真实组合键按键（触发应用已注册的 `togglePause` 热键，作为暂停入口的备选路径）。 |
| `tray_click.ps1` | 直接驱动产品托盘菜单：按窗口类名 `tray_icon_app` + 进程名找到托盘宿主窗口，`PostMessage(WM_USER_TRAYICON=6002, lParam=WM_LBUTTONUP)` 弹出原生菜单，再用键盘 `Down`×N + `Enter` 选择菜单项（默认第 3 项 =「暂停热键」，顺序见 `src-tauri/src/app/tray.rs`）。用于暂停/恢复，因为产品没有暴露 pause 的 IPC 命令。 |
| `run_validation.ps1` | 真机验证编排：基线探测 / 20 次重启注册 / OS 冲突场景 / 暂停释放与恢复，写 JSONL 原始日志与截图。 |

## 运行

```powershell
# 1) 构建（改过 Rust 必须重建；前端改动由 Vite HMR 生效）
cargo build --manifest-path src-tauri/Cargo.toml

# 2) 一键跑完整矩阵（独占 applab 实例 owner=hotkey-probe，结束自动 stop）
pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1

# 可选：只跑某一阶段 / 调整次数
pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1 -OnlyPhase conflict
pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1 -RestartCycles 20 -ProbeCycles 20

# 单独使用辅助进程（例如手工占住 Ctrl+Q 60 秒）
pwsh -NoProfile -File scripts/spikes/s04/hold_hotkey.ps1 -Spec Ctrl+Q -HoldSec 60
pwsh -NoProfile -File scripts/spikes/s04/hold_hotkey.ps1 -Spec Ctrl+Q -HoldSec 0   # 仅探测

# 单独驱动托盘菜单
pwsh -NoProfile -File scripts/spikes/s04/tray_click.ps1 -ListOnly
pwsh -NoProfile -File scripts/spikes/s04/tray_click.ps1 -ItemIndex 3
```

## 输出

- `.verify/spikes/S-04/logs/run-<timestamp>.jsonl`：逐条原始记录（探测结果 / 延迟 / Win32 错误码 / 托盘与热键操作）。
- `.verify/spikes/S-04/shots/*.png`：设置页冲突提示截图。
- `.verify/spikes/S-04/tmp/`：辅助进程状态文件与输出。

## 注意

- **应用实例独占**：`run_validation.ps1` 通过 applab 的 `--owner hotkey-probe` 排队，结束自动 `stop`；不要 `taskkill` 别人的实例，也不要在别人占用时 `--force`。
- **探测会临时注册热键**：`probe_os` 对未被应用持有的组合会临时 `register` 再立即 `unregister`（毫秒级），验证期间不要与其它 UI 自动化并行。
- **PowerShell 脚本必须是 UTF-8 BOM**：本机 `pwsh` 实为 Windows PowerShell 5.1，无 BOM 的 UTF-8 中文会被按 ANSI 解析导致语法错误；仓库内脚本已带 BOM。新增脚本请保持 BOM。
- 辅助进程只用于**等价模拟**第三方抢占（QQ/微信/输入法未安装或未占用该键），结论中已说明该局限性；产品代码未引入任何 `unsafe`。
