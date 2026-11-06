# scripts/spikes/s09 — S-09 截图失败降级（DRM / 全屏独占）与 WGC 对比

对应 `docs/技术验证清单.md` S-09（P1），记录见
[`docs/spikes/S-09-截图失败降级与WGC对比.md`](../../../docs/spikes/S-09-截图失败降级与WGC对比.md)。

本目录是**独立 spike 工具**：不修改产品代码、不启动应用（applab）、不跑 `pnpm build`。
构建/运行产物全部写到 `.verify/`（gitignored），不污染仓库。

## 文件

| 文件 | 用途 |
| --- | --- |
| `S09Capture.csproj` | .NET 10（`net10.0-windows10.0.26100.0`，取 WinRT 投影）控制台工具 |
| `Program.cs` | 造两种窗口（普通 / `SetWindowDisplayAffinity` 受保护）→ BitBlt / PrintWindow / WGC 三条路径抓帧 → 黑屏统计 → JSON + PNG |
| `wgc_vs_bitblt.ps1` | 宿主脚本：构建、运行、环境快照、精选聚合、17 项自动校验 |
| `README.md` | 本文件 |

## 一键复跑

```powershell
# 在仓库根目录执行；-Reps 为每场景重复次数（默认 3）
powershell -ExecutionPolicy Bypass -File scripts\spikes\s09\wgc_vs_bitblt.ps1 -Reps 3

# 只重跑采集器（跳过 dotnet build），需先成功构建过一次
powershell -ExecutionPolicy Bypass -File scripts\spikes\s09\wgc_vs_bitblt.ps1 -NoBuild -Reps 1
```

退出码：`0` = 全部校验 PASS；`2` = 有校验 FAIL；`1`/其它 = 构建或运行失败。

## 场景 × 路径

| 场景 | affinity | 等价真实场景 |
| --- | --- | --- |
| `normal` | WDA_NONE | 普通内容基线 |
| `wda_exclude` | `WDA_EXCLUDEFROMCAPTURE(0x11)` | 受保护窗口从截屏中排除 |
| `wda_monitor` | `WDA_MONITOR(0x01)` | 受保护窗口捕获时变黑（DRM 黑屏近似） |
| `fullscreen_normal` | WDA_NONE | 普通全屏应用（全屏独占近似） |
| `fullscreen_wda_monitor` | 全屏 + `0x01` | 全屏播放受保护视频 ≈ 整屏黑 |
| `fullscreen_wda_exclude` | 全屏 + `0x11` | 全屏受保护窗口「消失」 |

每条场景采集 5 条路径：`bitblt_region`、`bitblt_desktop`（整虚拟桌面，含窗口区域统计）、`printwindow`、
`wgc_window`（WGC 窗口项）、`wgc_monitor`（WGC 主屏显示器项）。

## 输出

- `.verify/spikes/S-09/raw/results.json`：全量采样（含整帧/区域统计、sha256、PNG 路径、耗时、错误）；
- `.verify/spikes/S-09/raw/env.json`：环境快照；`raw/manifest.json`：源文件/构建产物 sha256 与运行输出；
- `.verify/spikes/S-09/shots/*.png`：原始截图；
- `docs/spikes/data/s09/summary.json|env.json|shots/`：可入 git 的精选聚合数据与缩略证据图（脚本自动生成）。

## 实现要点（复跑/改动时注意）

- **WGC**：`RoGetActivationFactory("Windows.Graphics.Capture.GraphicsCaptureItem")` →
  QI `IGraphicsCaptureItemInterop`（`windows.graphics.capture.interop.h` 中直接派生自 **IUnknown**，
  `CreateForWindow/CreateForMonitor` 在 vtable slot 3/4；item 接口 IID `79C3F95B-31F7-4EC2-A464-632EF5D30760`）
  → `Direct3D11CaptureFramePool.CreateFreeThreaded` + D3D11 设备
  （`D3D11CreateDevice` + `CreateDirect3D11DeviceFromDXGIDevice`）→ `TryGetNextFrame` →
  `SoftwareBitmap.CreateCopyFromSurfaceAsync(..., BitmapAlphaMode.Ignore)` → `CopyToBuffer` + `DataReader` 回读 BGRA。
  `IsBorderRequired=false` 关闭捕获边框。
- **黑屏口径**（与产品 Rust 侧 `platform/capture.rs` 一致）：`luma = 0.114·B + 0.587·G + 0.299·R`，
  黑像素 = luma < 16；判黑 = 黑像素占比 ≥ 99% 且均值 < 16 且标准差 < 6。
- **受保护窗口**只使用公开 API `SetWindowDisplayAffinity`（0x11 / 0x01），不做任何破解。
- 采集窗口为置顶无边框窗口，内容为确定性四色象限 + 文字，因此普通窗口的 BitBlt 帧与 WGC 帧应逐字节一致
  （脚本的 `normal_wgc_equals_bitblt_pixels` 校验即验证这一点）。
- 进程在造窗口前设置 Per-Monitor-V2 DPI 感知；坐标一律物理像素（与 S-01 口径一致）。
- Windows 侧首次运行会从 NuGet 还原 `Microsoft.Windows.SDK.NET.Ref`（WinRT 投影），需网络。
