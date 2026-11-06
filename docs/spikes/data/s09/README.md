# docs/spikes/data/s09 — S-09 精选聚合数据

来源：`scripts/spikes/s09/wgc_vs_bitblt.ps1 -Reps 3`（2026-11-05，commit `3f1b263`）。
原始数据（全量采样 90 点 + 84 张截图）在 `.verify/spikes/S-09/`（gitignored）。

| 文件 | 内容 |
| --- | --- |
| `summary.json` | 6 场景 × 5 路径的聚合统计（blackRatio/meanLuma/stdDev/耗时/rep1 sha256/区域统计/verdict）+ **17 项自动校验**（全部 pass） |
| `env.json` | 环境快照：Windows build 26200.9711 / i7-12650H / 15.78GiB / RTX 4060 + 虚拟显示适配器 / 双屏混合 DPI / 虚拟桌面 `(-1080,-470) 3640×1920` |
| `shots/normal-bitblt-region.png`、`shots/normal-wgc-window.png` | 普通窗口：两路径画面一致（sha256 完全相同） |
| `shots/wda-monitor-bitblt-region-black.png` | `WDA_MONITOR(0x01)`：BitBlt 纯黑（3 KB PNG） |
| `shots/wda-exclude-bitblt-region-window-missing.png` | `WDA_EXCLUDEFROMCAPTURE(0x11)`：窗口从截屏消失、截到背后桌面/窗口 |
| `shots/fullscreen-wda-monitor-wgc-monitor-black.png` | 全屏 + 0x01：WGC 主屏捕获整帧黑 |

结论与阈值口径见 [`docs/spikes/S-09-截图失败降级与WGC对比.md`](../../S-09-截图失败降级与WGC对比.md)。
