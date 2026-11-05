# scripts/spikes/s10_s16_s18 — S-10 / S-16 / S-18 复跑脚本

对应 `docs/技术验证清单.md` 的 P1 项：

| 项 | 脚本 | 输出 |
| --- | --- | --- |
| S-18 发布构建 + 体积基线 | `s18_build.ps1` | `.verify/spikes/S-16-S-18/build/build_meta.json`、`.verify/p1/logs/s18-build.log` |
| S-18 CI/本地体积门禁 | `check_bundle_size.ps1` | `.verify/ci/volume_baseline.json`（CI）或 `-BaselineOut` 指定路径 |
| S-16 签名 / Defender / MOTW 证据 | `s16_scan_signature.ps1` | `.verify/spikes/S-16-S-18/s16/scan_signature.json` 等 |
| S-16 真实安装 / 启动 / 卸载 / 残留 | `s16_install_test.ps1` | `.verify/spikes/S-16-S-18/s16/install_uninstall.json` |
| S-10 冷启动 / 空闲资源 | `s10_measure.ps1` | `.verify/spikes/S-10/raw/{coldstart,idle}-<label>.json`、`summary-*.json` |

## 环境与约定

- 本机只有 **Windows PowerShell 5.1**（`pwsh` 不可用），因此统一用：
  ```powershell
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/<script>.ps1
  ```
- 所有 `.ps1` 必须保存为 **UTF-8 with BOM**（5.1 无 BOM 会按 ANSI 读取中文导致解析错误；仓库既有脚本同此约定）。
- 构建串行：`s18_build.ps1` 会独占创建 `.verify/p1/build.lock`（JSON：owner/pid/startedAt；<20min 有效，过期接管），
  成功/失败都会在 `finally` 删除；`capture-degrade` 等使用 cargo 的验证看到锁应等待，不要 kill 他人进程。
- 应用独占：S-10 需先确认 applab 空闲（`node scripts/spikes/applab/applab.mjs status`），
  测量期间在 `.verify/p1/quiet/` 放 `startedAt` 标记，并可用 applab `reserve` 占位，结束后删除。
- 大文件（安装包、日志、逐次采样）只写 `.verify/`（gitignored）；聚合小 JSON 写 `docs/spikes/data/`。

## 常用命令

```powershell
# S-18：release 构建 + 采集（约 3–5 分钟；构建 CPU 重，避开他人时延测量）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s18_build.ps1
# 只采集现有产物（不构建）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s18_build.ps1 -SkipBuild
# 体积门禁（本地自测 = CI 同一脚本）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/check_bundle_size.ps1

# S-16：签名 / Defender / MOTW（不修改系统；扫描用 -DisableRemediation 保留产物）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s16_scan_signature.ps1
# S-16：真实安装/启动/卸载（currentUser 静默默认目录；会真实安装与卸载，先读记录 §风险）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s16_install_test.ps1

# S-10：冷启动（release 稳态，播种已配置用户数据；N=10）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode coldstart -Label release -Runs 10 -SeedFrom .verify/applab/app/data
# S-10：首次启动对照（空数据目录；N=3）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode coldstart -Label release-fr -Runs 3
# S-10：debug 对照（需要 Vite dev server，脚本会复用/临时拉起 localhost:1420）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode coldstart -Label debug -Runs 10 -SeedFrom .verify/applab/app/data
# S-10：空闲 30 分钟采样（release；每 5s；预热 60s）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode idle -Label release -IdleMinutes 30 -SampleSec 5
# S-10：debug 空闲 5 分钟补充（参考）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode idle -Label debug -IdleMinutes 5 -SampleSec 5
```

## S-10 口径与踩坑（务必先读）

- **冷启动** = `Process.StartTime`(UTC) → 应用日志「框选 AI 初始化完成」行内时间戳；该行在 `lifecycle.rs` 的
  托盘创建 + 全局热键注册 + 隐藏窗口预创建之后输出，是「托盘可交互」的应用内可观测时点。
  每次运行后脚本会 `RegisterHotKey(Ctrl+Q)` 探测：返回 `1409` 表示已被应用注册（热键就绪）。
- **日志读取踩坑**：应用持有日志文件句柄期间，`Get-ChildItem` 的 `Length/LastWriteTime` 可能陈旧（本机实测
  10 次只命中 1 次）；脚本已改为 `FileShare=ReadWrite` 直接读整文件 + 只接受时间戳 ≥ 进程创建的行。不要改回增量读。
- **稳态 vs 首次启动**：`-SeedFrom <applab 数据目录>` 播种 `config.json`/`onboarding.json`（已完成引导）；
  不传则测「首次启动」口径（含首启引导窗口显示）。
- **验收口径以 release 为准**；`-Label debug` 走 Vite dev（未打包前端），冷启动 P95 会明显偏高，仅作对照。
- **内存口径**：判据取主进程 WorkingSet/Private WS（≤120MB）；进程树含 `msedgewebview2` 子进程单列（WebView2 多进程
  架构，见记录 §6.3）。**CPU 口径**：进程树 CPU 时间差 / 墙钟 / 16 逻辑核 ×100%（占总核，≈任务管理器），阈值 mean<0.5%。
- 结束后确认无 snip-ask 残留并删除 `.verify/p1/quiet/startedAt`；脚本自身每 5s 采样开销（Get-Counter/全进程枚举）
  会计入系统负载基线，不影响被测应用 CPU 口径。

## 注意

- `s16_install_test.ps1` 里对“带 MOTW 的静默安装是否被阻断”只能做**行为级**判断：SmartScreen 的交互提示是 UI，
  本机 `Microsoft-Windows-SmartScreen/Debug` 日志未启用，脚本不会伪造截图；结论与遗留见
  `docs/spikes/S-16-无签名安装包与杀软误报.md`。
- 安装目标目录默认由 NSIS 模板决定（currentUser → `%LOCALAPPDATA%\SnipAsk`，与本地数据目录同名）；
  可用 `-InstallDir` 指定临时目录。卸载会删除安装文件，但 `%APPDATA%\SnipAsk` 等应用数据按设计保留（脚本只做差异记录、不删除）。
- 单实例约束：安装版启动前必须确认没有其它 snip-ask 实例（applab 的便携实例也会抢占单实例锁），
  否则安装版会作为“第二实例”立即退出。
