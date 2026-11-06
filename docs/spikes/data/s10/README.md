# docs/spikes/data/s10 — S-10 聚合数据（可入 git，小文件）

原始数据在 `.verify/spikes/S-10/`（gitignored，含逐次样本与完整日志）；这里只放结论所需的最小聚合。
口径定义（冷启动时点、内存/CPU 折算、进程树 vs 主进程）见 `docs/spikes/S-10-冷启动与空闲资源.md` §3。

| 文件 | 内容 |
| --- | --- |
| `coldstart_summary.json` | release 稳态 N=10 + 首次启动对照 N=3 + debug 对照 N=10 的 P50/P95/max/min、逐次样本、环境基线 |
| `idle_summary.json` | release 空闲 30 分钟（360 样本）内存/CPU/系统负载统计 + debug 5 分钟补充；含阈值判定与口径说明 |
| `README.md` | 本说明 |

复跑/更新（Windows PowerShell 5.1；脚本 UTF-8 BOM）：

```powershell
# 安静窗口内（无 cargo/pnpm/OCR/llama 进程；放 .verify/p1/quiet/startedAt）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode coldstart -Label release -Runs 10 -SeedFrom .verify/applab/app/data
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode idle -Label release -IdleMinutes 30 -SampleSec 5
```

说明：`coldstart_summary.json` / `idle_summary.json` 是 `.verify/spikes/S-10/raw/` 下对应 summary JSON 的聚合抽取
（由 `scripts/spikes/s10_s16_s18/` 的测量脚本产出，人工/脚本抽取小体积字段）；更新记录时保持字段名与 `docs/spikes/S-10-冷启动与空闲资源.md` 一致。
