# scripts/spikes/s13_s17_s19_s20 — P1 应用运行时验证脚本

对应 `docs/技术验证清单.md` 的 **S-13 / S-17 / S-19 / S-20**（P1）。全部脚本可独立复跑；
原始数据写 `.verify/spikes/S-13-S-17-S-19-S-20/`（gitignored），精选聚合写
`docs/spikes/data/s13_s17_s19_s20/`，逐项结论写 `docs/spikes/S-13-*.md`、`S-17-*.md`、`S-19-*.md`、`S-20-*.md`。

| 文件 | 用途 |
| --- | --- |
| `lib.mjs` | 共享工具：applab 起停 / CDP eval / waitFor / 截图 / 落盘 / 未捕获异常收集器 |
| `mock_provider.mjs` | S-17 本地 HTTP mock（OpenAI 兼容 `/models`）：200/401/402/429/5xx/超时/非 JSON/慢响应，端口默认 39117，请求头脱敏落盘 |
| `run_s17.mjs` | S-17 主驱动：mock 故障矩阵（20 用例真实 IPC）+ 断网 `menu:ready` 降级链 + 设置页真实错误 UI + 未捕获异常 |
| `run_s17_real.mjs` | S-17 补充：**真实 DeepSeek** `provider_test`（Key 仅运行时从 `.verify/p1/key.txt` 读入，不打印不落盘）+ S-13 前端现状探测 |
| `s20_tray.ps1` | S-20 托盘探测：托盘宿主窗口、`Shell_NotifyIconGetRect` uID 扫描、UIA 定位图标、真实点击尝试、原生菜单选择、应用顶层窗口枚举（DPI 感知） |
| `run_s20.mjs` | S-20 主驱动：第二实例 ×10、托盘探测、暂停/恢复热键 ×3、正常退出后 `Ctrl+Q` 无残留 ×5 |
| `run_s13.mjs` | S-13：0.5B/1.5B/1.8B 常驻内存与冷加载复核、Job Object 低内存模拟、minRamMB 降档判定表、前端降档逻辑检索 |
| `run_s13_ui.mjs` | S-13 补充：设置 → 翻译 → 本地（离线）→ 模型库静态内存文案 + `navigator.deviceMemory` 实机取证（截图 `shots/s13/02-*`） |
| `s13_job.ps1` | S-13 辅助（C# P/Invoke）：Job Object 进程内存上限 + `/health` 就绪判定 + 进程/内存/退出码记录 |
| `run_s19.mjs` | S-19 自动走查：`--reset` 首次启动 6 步引导逐步计时 + 截图（参考数据，不冒充真人） |
| `S-19-新用户走查协议.md` | S-19 真人验收协议（3 人、计时表、卡点表、完成标准、解锁步骤） |

## 前置条件

- Node ≥ 20（本机 v24）；Windows PowerShell 5.1（本机无 `pwsh`：一律
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File`；`.ps1` 需 **UTF-8 BOM**，否则中文关键字乱码）。
- 应用独占：`node scripts/spikes/applab/applab.mjs reserve --owner app-runtime --minutes 90`；
  `start --owner app-runtime --wait-sec 900 [--reset]`；结束务必 `stop --owner app-runtime` 并确认
  `Get-Process snip-ask` 残留 = 0。
- **不要重建 Rust**（`src-tauri/target/debug/snip-ask.exe` 已构建；改动前先与 Lead 对齐）。
- S-13 需要 LM Studio 内置 llama-server（`%USERPROFILE%\.lmstudio\extensions\backends\llama.cpp-win-x86_64-avx2-*`）
  与 `.verify/spikes/S-05-S-06/artifacts/` 下 3 个 gguf（S-05 遗留）。

## 复跑命令

```powershell
# S-17（mock 故障矩阵；不耗外网/Key）
node scripts/spikes/s13_s17_s19_s20/run_s17.mjs

# S-17（真实 DeepSeek；Key 由 Lead 放 .verify/p1/key.txt）
node scripts/spikes/s13_s17_s19_s20/run_s17_real.mjs

# S-20（单实例 / 托盘 / 热键释放；约 3–4 分钟，含 5 次重启）
node scripts/spikes/s13_s17_s19_s20/run_s20.mjs

# S-13（常驻内存 + Job Object 模拟 + 降档表；不占 Tauri 应用，只起 llama-server）
node scripts/spikes/s13_s17_s19_s20/run_s13.mjs

# S-19（自动走查参考数据：--reset 首次启动）
node scripts/spikes/s13_s17_s19_s20/run_s19.mjs
```

## 关键约定 / 踩坑记录

1. **applab 单实例独占**：`state.json` 以应用进程存活判定；应用被他人占用时 `--wait-sec` 轮询等待；
   预约（`reserve`）可跨多次 start/stop 存活，防插队。
2. **环境事实**：应用默认 `visible:false`，需要看界面先 `invoke('window_show',{window:'main'})`；
   Vite 只监听 IPv6（探测用 `localhost`）；本机任务栏**自动隐藏** + 托盘图标默认在**溢出区** →
   原生托盘菜单自动化受限（详见 `docs/spikes/S-20-托盘单实例热键释放.md`「局限性」）。
3. **`config_patch` 的数组字段是整体替换**：`providers` 必须传完整对象（含 model/caps/temperature/maxTokens/enabled），
   否则 model/caps 被清空，设置页会报「模型名不能为空」。
4. **进程内凭据存储**：本构建 `AppState.credentials = InMemoryCredentialStore`（Windows Credential Manager 为 TODO(real)），
   假 Key 只存在进程内存、退出即丢；S-17 mock 矩阵依赖这一点（成功用例会把假 Key 写进内存，故
   「无 Key」用例必须排在成功用例之前）。
5. **PowerShell 输出污染**：`Get-Content` 返回的字符串带 PSPath/PSDrive 等 NoteProperty，直接
   `ConvertTo-Json` 会把 .NET 程序集元数据序列化进来（实测产生 67MB JSON）——必须 `[string]$_` 物化后再输出。
6. **Job Object 限额本机不生效**：`SetInformationJobObject` 成功、`AssignProcessToJobObject` 成功，
   但 200MB 上限下 llama-server Private 仍达 660MB（`--no-mmap` 亦如此）→ 无法用该手段复现 OOM（见 S-13 记录）。
7. **清理纪律**：mock server / llama-server / snip-ask 均不得残留；`run_s13.mjs` 结束会断言
   `Get-Process llama-server` 为 0，`run_s20.mjs` 结束会断言 `Get-Process snip-ask` 为 0。
