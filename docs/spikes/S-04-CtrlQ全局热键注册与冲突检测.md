# S-04 · `Ctrl+Q` 全局热键注册与冲突检测验证

> 记录模板：`docs/技术验证清单.md §4`；原始数据：`.verify/spikes/S-04/`（gitignored）；
> 复跑脚本：`scripts/spikes/s04/`（README 见该目录）。

## 结论

**DONE** —— 四项验收全部达到：

| 验收阈值（技术验证清单 S-04） | 实测 | 结论 |
| --- | --- | --- |
| 注册成功率 **100%** | 20/20 次「app_quit + 重启」后 `hotkey_probe("Ctrl+Q")` 均 `available=true`，且辅助进程同键注册均被拒（1409）；基线 20/20 次探测 `available=true`；无冲突组合 40/40 次 `available=true` | ✅ |
| 冲突时 **100%** 给出可操作提示与改键引导 | 辅助进程先占 `Ctrl+Q` 再启动应用：`hotkey_probe` 20/20 返回 `available=false, conflictWith="system"`；设置页显示可操作文案并保留「检测 / 点按键改键」入口（截图 `s04-conflict-system-hint-002639.png`），应用日志有注册失败告警 | ✅ |
| 暂停后热键**完全释放** | `togglePause` 触发暂停后，辅助进程 `RegisterHotKey(Ctrl+Q)` **成功**（`win32Error=0`，注册耗时 3.9ms）；恢复后辅助进程再次 1409、`hotkey_probe` 恢复 `available=true` | ✅ |
| （附加）探测不泄漏注册 | 对空闲组合连续 probe 两次后辅助进程立即注册成功 20/20（无残留占用） | ✅ |

## 编号/日期/环境

- 编号：S-04
- 结论：**DONE**
- 日期：2026-11-04（验证时间 23:57–00:16，滚动到 2026-11-05 凌晨；文档按任务约定记 2026-11-04）
- 环境：
  - Windows 11 26200（Windows PowerShell 5.1.26100.9711；Node v24）
  - CPU i7-12650H / 内存 16GB
  - 屏幕：主屏 2560x1440@150% 位于 (0,0)，副屏 1080x1920@100% 位于 (-1080,-470)
  - 构建：`cargo build --manifest-path src-tauri/Cargo.toml`（debug，Tauri 2.12 / Rust 1.99），git commit `a9b352e` + 本任务未提交改动
  - 真实应用验证台：`scripts/spikes/applab/applab.mjs`（便携数据目录 `.verify/applab/app/`，owner=hotkey-probe）
  - 第三方进程对照：验证时本机确有微信进程 `Weixin` / `WeChatAppEx`、微软输入法 `ChsIME` 在运行；但它们默认不占用 `Ctrl+Q`，因此「QQ/微信/输入法抢占同键」无法实机复现，改用同键 `RegisterHotKey` 辅助进程**等价模拟**（见「局限性」）。

## 步骤

1. **实现 OS 级探测（代码）**：`src-tauri/src/app/hotkeys.rs` 新增 `probe_os` / `probe_os_with`：
   - 复用 `tauri-plugin-global-shortcut` 的 `is_registered` / `register` / `unregister`（插件在 OS 注册失败时返回 `Err`），无需 `unsafe`（crate 为 `#![forbid(unsafe_code)]`）；
   - 若当前绑定已被本应用注册 → 视为可用（不重复注册/注销）；
   - 否则临时 `register` + 立即 `unregister`：失败 → `available=false, conflictWith="system"`；成功 → 结合配置内同键绑定判定；
   - `commands/system.rs::hotkey_probe` 改为调用 `probe_os`（命令参数/返回契约字段未变）；
   - 前端 `settings/hotkey.ts` / `onboarding/hotkey.ts` 新增 `hotkeyStatusHint()`：把 `"system"` 映射为「被其他程序占用（如 QQ / 微信 / 输入法），请点上方按键换一个组合，例如 Ctrl+Shift+Q」，配置内冲突仍显示字段对应的动作名（`HotkeyField.tsx` / `HotkeyRecorder.tsx` 各改 1~2 行调用）。
2. **辅助工具**：`scripts/spikes/s04/hold_hotkey.ps1`（P/Invoke `RegisterHotKey` 占住/探测，输出 Win32 错误码与耗时；`-HoldSec 0` 仅探测）、`press_hotkey.ps1`（`keybd_event` 注入真实组合键）、`tray_click.ps1`（Win32 托盘菜单自动化，本轮未作为判据）、`run_validation.ps1`（全矩阵编排）。
3. **真机验证**（一次 run 跑完矩阵，`run-20261105-001219.jsonl`）：
   1) 基线：应用启动后 `hotkey_probe("Ctrl+Q")` ×20；辅助进程同键注册 ×20；无冲突组合 `Ctrl+Shift+Y` / `Alt+Shift+9` 各 ×20；
   2) 20 次 `app_quit` + `applab start` 重启，每次验证「辅助进程注册 Ctrl+Q 被拒 1409」+「`hotkey_probe` available」；
   3) OS 冲突场景：先启动辅助进程占住 `Ctrl+Q`，再启动应用（应用注册必然失败），`hotkey_probe` ×20 + 设置页 UI 截图（首轮截图只拍到页面顶部；后补 `-OnlyPhase conflict` 复跑一次，滚动到热键行复拍 `s04-conflict-system-hint-002639.png`，20/20 同结果）；
   4) 暂停/恢复：`config_patch` 绑 `togglePause=Ctrl+Alt+P` → `keybd_event` 发一次 → 断言辅助进程可注册 `Ctrl+Q`（硬判据）→ 再发一次恢复 → 断言辅助进程再次 1409 且 probe 可用；
   5) probe 泄漏对抗：对空闲组合连测两次后立刻用辅助进程注册 ×20；
   6) 记录每步 Win32 错误码/调用耗时，原始日志落盘。

## 原始数据

路径：`.verify/spikes/S-04/logs/run-20261105-001219.jsonl`（正式全矩阵 run，179 条 JSONL）、`run-20261105-002625.jsonl`（冲突场景复跑 + UI 截图补拍，32 条）、`run-20261105-000303.jsonl`（首轮，含 2 次外部并发导致的重启噪声，见「结果/说明」）、截图 `.verify/spikes/S-04/shots/`。

| 指标 | 数值 |
| --- | --- |
| `hotkey_probe("Ctrl+Q")`（应用已持有） | 20/20 `available=true`；耗时 P50 **2.3ms**、max **2.8ms**（IPC+OS 判定；`is_registered` 命中，不实际 register） |
| 辅助进程 `RegisterHotKey("Ctrl+Q")`（应用已持有） | 20/20 失败，Win32 **1409** `ERROR_HOTKEY_ALREADY_REGISTERED` |
| 无冲突组合探测 | `Ctrl+Shift+Y` 20/20、`Alt+Shift+9` 20/20，共 40/40 `available=true`；单次探测含临时 register+unregister |
| 20 次重启注册 | **20/20**（`restart_startFailures=0`、`restart_registrationFail=0`）；每次启动后辅助进程注册 Ctrl+Q 均 1409 |
| OS 冲突场景 | 20/20 `available=false, conflictWith="system"`；应用日志：`WARN ... 热键注册失败（可能被其它程序占用） hotkey=Ctrl+Q action="capture" error=HotKey already registered`；探测侧日志 `HotKey already registered ... elapsed_us=64~163` |
| 设置页 UI 文案 | 红字「被其他程序占用（如 QQ / 微信 / 输入法），请点上方按键换一个组合，例如 Ctrl+Shift+Q」，同一行有「检测」与按键改键入口（截图 `s04-conflict-system-hint-002639.png`，20/20 复跑同结果） |
| 暂停后释放 | `aux.registered=true, win32Error=0, registerUs=3903`（暂停生效后辅助进程能占住 Ctrl+Q） |
| 暂停后 probe 语义 | `hotkey_probe("Ctrl+Q") = {valid:true, available:false, conflictWith:"hotkeys.capture"}` —— 应用已不持有该键（`Free`），但配置里仍绑定，故按「配置内冲突」映射；**释放的硬判据以辅助进程注册成功为准** |
| 恢复后 | 辅助进程再次被拒（1409）、`hotkey_probe("Ctrl+Q") = {available:true, conflictWith:null}` |
| probe 无泄漏 | 20/20：`probe`×2 后辅助进程立即注册成功（`win32Error=0`） |

关键日志片段：

```text
2026-11-04T16:15:41.040482Z WARN snip_ask_lib::app::hotkeys: 热键注册失败（可能被其它程序占用）
    hotkey=Ctrl+Q action="capture" error=HotKey already registered: HotKey { mods: Modifiers(CONTROL), key: KeyQ, id: 524323 }
2026-11-04T16:15:43.649333Z INFO snip_ask_lib::app::hotkeys: OS 级探测：注册被拒绝（判定为被其它程序/系统占用）
    spec=Ctrl+Q elapsed_us=68 error=HotKey already registered: ...
```

## 结果与判定

- 全部达到验收阈值（见上表）。
- `conflictWith` 映射（契约字段不变，仅扩展取值语义）：

| 场景 | valid | available | conflictWith | 前端文案 |
| --- | --- | --- | --- | --- |
| 格式非法 | false | false | null | 「格式不合法：需至少一个修饰键 + 字母/数字，例如 Ctrl+Shift+Q」 |
| 与应用当前已注册绑定相同（自探测） | true | true | null | 「可用」 |
| 被其它程序/系统占用（OS register 失败） | true | false | `"system"` | 「被其他程序占用（如 QQ / 微信 / 输入法），请点上方按键换一个组合，例如 Ctrl+Shift+Q」 |
| 配置内同键重复绑定（键空闲或 ≥2 字段重复） | true | false | `hotkeys.*` 字段名 | 「与应用内「<动作名>」重复，请点上方按键换一个组合」 |

- 说明（首轮 18/20 的分类）：`run-20261105-000303.jsonl` 中 2 次重启失败均为**外部并发 start 造成 applab 启动竞态**（1 次 `EBUSY: copyfile snip-ask.exe`、1 次 CDP 30s 未就绪），不是注册失败；随后 applab 增加「清理后等待 + 冷却 + 自动重试」并由本脚本对启动失败单独分类重试后，正式 run 为 20/20。最终 20 次样本中启动失败 0 次、注册失败 0 次。

## 处置

- 达标项：OS 级可用性探测、`"system"` 映射与可操作文案、暂停释放/恢复重新注册，按文档记录到实现边界；`hotkey_probe` 成为真实的「OS 占用检测」实现（原实现只做格式+配置内冲突）。
- **验证中发现并修复一个真实缺陷**：原 `set_paused(true)` 会注销包括 `togglePause` 在内的全部热键，导致「暂停 / 恢复热键」暂停后无法再用热键恢复（只能托盘恢复）。已改为：暂停时**只保留 `togglePause`**（`register_all` 按暂停状态过滤、`on_shortcut` 暂停时仅放行 `togglePause`），其余热键（含 `capture`）全部释放。这一改动同时使「热键暂停 → 热键恢复」可验证、且行为符合托盘文案「暂停 / 恢复热键」。
- 前端改动仅「映射文案 + 调用共享函数」，未改契约字段。

## 改动文件

- `src-tauri/src/app/hotkeys.rs`：`probe_os` / `probe_os_with` / `OsProbe` / `SYSTEM_CONFLICT` / `conflict_fields`；暂停只保留 `togglePause` 的注册/回调逻辑；新增 7 个单测。
- `src-tauri/src/commands/system.rs`：`hotkey_probe` 改为 `AppHandle + probe_os`（返回契约不变）。
- `src-tauri/tests/s04_hotkey_probe.rs`（新增）：跨 crate 边界验证 `system` / `hotkeys.*` 映射与序列化（6 个测试）。
- `src/windows/main/settings/hotkey.ts`、`src/windows/main/onboarding/hotkey.ts`：`hotkeyStatusHint()` 与字段名→动作名映射（`"system"` = OS 占用）。
- `src/windows/main/settings/components/HotkeyField.tsx`、`src/windows/main/onboarding/components/HotkeyRecorder.tsx`：各 1~2 行改为调用共享提示函数（Lead 已批准范围扩展）。
- `scripts/spikes/s04/`：`hold_hotkey.ps1`、`press_hotkey.ps1`、`tray_click.ps1`、`run_validation.ps1`、`README.md`。
- 本文档。

（`src-tauri/src/platform/hotkey.rs` 未改：解析/规范化已满足需求。）

## 局限性

1. **无法实机复现 QQ/微信/输入法抢占同键**：本机装有微信（`Weixin`/`WeChatAppEx`）与微软输入法，但其默认热键为 `Ctrl+Alt+W` / `Ctrl+Alt+A` 等，不占用 `Ctrl+Q`，且不安装 QQ。故用「同键 `RegisterHotKey` 辅助进程」等价模拟第三方抢占——两者都在同一桌面的全局热键表内互斥，OS 判定路径与真实第三方完全一致（Win32 1409）；差异只在于抢占方不是一个完整的 QQ/微信进程。
2. **辅助进程用 `RegisterHotKey(NULL, …)` 注册到自身线程消息队列**，产品用 `global-hotkey` 的隐藏窗口注册；二者都占用同一全局热键，互斥语义等价。
3. **暂停判据以辅助进程注册成功为准**（比页面状态更硬）。暂停后 `hotkey_probe("Ctrl+Q")` 返回 `available=false, conflictWith="hotkeys.capture"` 而非 `system`，这是「应用已释放、配置仍绑定」的正确映射，不代表释放失败。
4. 设置页截图（`s04-conflict-system-hint-002639.png`）已滚动到「框选并唤起」热键行，可看到红字提示与「检测 / 按键改键」入口；提示文案同时以 DOM 文本形式记录在 `run-20261105-001219.jsonl` 的 `conflict/ui-status` 中。
5. 托盘菜单 Win32 自动化（`tray_click.ps1`）在本机失败：托盘图标被 Win11 收进溢出区时 `Shell_NotifyIconGetRect` 取不到矩形，`TrackPopupMenu` 不弹出（`#32768` 不可见）。因此暂停/恢复判据采用可控的 `togglePause` 热键路径，未把托盘菜单自动化作为证据；该脚本保留供后续修整。
6. Windows PowerShell 5.1 传参给原生 node 时会丢掉 `--js` 参数中的双引号，脚本内 JS 参数一律使用单引号对象字面量；`.ps1` 文件需 UTF-8 BOM（本机 PowerShell 5.1 默认按 ANSI 解析）。

## 遗留

- 托盘「暂停热键」菜单项在 Win11 溢出区下的可访问性（自动化/无障碍）未验证，建议由 S-20（托盘/单实例/热键释放）补充人肉走查。
- `hotkey_probe` 无「正在编辑哪个字段」入参，自探测与「同键绑定到另一动作」在「应用已持有该键」时不可区分（已知局限，已在代码注释与上文说明）；若产品后续需要精确区分，可在命令增加可选字段参数后再评估契约变更。
- 真实 QQ/微信环境下的抢占复现建议在装有机会的测试机补一条人工对照。
