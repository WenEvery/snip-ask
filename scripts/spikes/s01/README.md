# scripts/spikes/s01 —— S-01 混合 DPI 多屏坐标一致性 spike

本目录是 S-01（P0：混合 DPI 多屏下框选坐标误差 ≤2px、跨屏拖选不跳变）的**独立、可复跑**验证工具。
全部为真机物理像素测量，不 mock、不依赖单元测试桩。

| 脚本 | 依赖 | 说明 |
| --- | --- | --- |
| `s01_dpi_calibration.ps1` | 无（不占 app 实例） | Win32/WinForms：PerMonitorV2 + 全虚拟桌面十字线标定窗口 + BitBlt 全桌面截图 + 程序化像素检测 + 往返换算 + 显示器原始数据 |
| `s01_overlay_probe.ps1` | 真实 app 已 `capture_freeze` | PerMonitorV2 枚举 overlay HWND，输出外框/client 原点/client 尺寸/DPI/DWM 边框/样式，断言 client 区 == 虚拟桌面 |
| `s01_pointer_probe.mjs` | 真实 app + WebView2 CDP(9333) | `Input.dispatchMouseEvent` 合成跨屏拖拽（正/反向、负坐标、跨 x=0、近全虚拟桌面），读 DOM 选框回算物理坐标误差（≤2px），并留 DOM 截图 |

原始输出统一写 `.verify/spikes/S-01/`（已 gitignore），报告见
`docs/spikes/S-01-混合DPI多屏坐标一致性.md`。

## 1 前置

- Windows 10/11；PowerShell 5.1 即可（脚本用 `Add-Type` 内联 C# 调 Win32，**不需要** unsafe Rust）。
- `cargo` 在 PATH：`$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"`。
- 需要真实 app 的步骤先构建：`cargo build --manifest-path src-tauri/Cargo.toml`。
- **applab 独占纪律**：同一时刻只能有一个应用实例；`start` / `stop` 必须带自己的 `--owner`，
  跑完必须 `stop`。脚本说明见 `scripts/spikes/applab/applab.mjs` 头注释。

## 2 一键流程

```powershell
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"

# ① 纯 Win32 标定（不需要 app）
powershell -ExecutionPolicy Bypass -File scripts/spikes/s01/s01_dpi_calibration.ps1 -OutDir .verify/spikes/S-01

# ② 真实应用（排他锁；跑完必须 stop）
node scripts/spikes/applab/applab.mjs start --owner dpi-coords --wait-sec 900
node scripts/spikes/applab/applab.mjs eval --window main --js "window.__TAURI_INTERNALS__.invoke('capture_freeze')"

# ③ overlay client 几何断言
powershell -ExecutionPolicy Bypass -File scripts/spikes/s01/s01_overlay_probe.ps1 -Out .verify/spikes/S-01/s01_overlay_after.json

# ④ CDP 跨屏拖选测量（参数是期望的虚拟桌面物理矩形 x,y,w,h）
node scripts/spikes/s01/s01_pointer_probe.mjs --vd -1080,-470,3640,1920 --out .verify/spikes/S-01/s01_pointer_probe.json

# ⑤ 全虚拟桌面人工对照截图
powershell -ExecutionPolicy Bypass -File scripts/spikes/tools/screenshot.ps1 -Out .verify/spikes/S-01/s01_overlay_after_fullscreen.png

# ⑥ 释放实例
node scripts/spikes/applab/applab.mjs stop --owner dpi-coords
```

## 3 三个关键口径（踩坑记录，改脚本/产品代码前必读）

1. **进程必须先 PerMonitorV2**：PowerShell/Node 默认 DPI-unaware，`GetWindowRect`、
   `GetSystemMetrics`、`ClientToScreen` 会返回 DPI 虚拟化后的逻辑坐标。本目录脚本都先调
   `SetProcessDpiAwarenessContext(-4)`；产品侧由 Tauri/tao 保证（manifest PMv2）。
2. **overlay 坐标依赖的是 client 区，不是外框**：
   前端 `toPhys = virtualRect.origin + clientXY * dpr`。因此必须满足
   - `ClientToScreen(0,0)` == virtualRect 左上角；
   - `GetClientRect` 尺寸 == virtualRect 物理尺寸；
   - 下发的 dpr == WebView `devicePixelRatio`（= 窗口 `GetDpiForWindow/96`）。
   无边框窗口仍可能带 DWM 阴影 inset（本机实测修复前 client 原点相对外框偏 (11,2)px），
   所以只看 `GetWindowRect` 会漏掉一整类偏移 bug。
3. **单个跨屏窗口只有一个 dpr**：`devicePixelRatio` 是每 WebView 一个值，不能按指针所在屏逐屏改。
   本机 Windows 按「窗口与显示器重叠面积最大者」定窗口 DPI（144，即使左上角在 100% 副屏上）。
   逐屏混用 dpr 的误差量级见 `s01_calibration.json → wrongDprContrast`（本机最大 716px）。

## 4 输出文件

| 文件 | 说明 |
| --- | --- |
| `s01_monitors.json` | 显示器原始数据 + `SM_*VIRTUALSCREEN` + 外接矩形 |
| `s01_calibration.json` | 十字线检测明细、死区对照、往返换算、错误 dpr 对照 |
| `s01_calibration_summary.json` | 机器可读小结（result/最大误差/窗口 dpi） |
| `s01_calibration.png` | 全虚拟桌面十字线截图（3640x1920 @ 本机） |
| `s01_overlay_*.json` | overlay 几何快照（修复前/后） |
| `s01_pointer_probe.json` / `-overlay.png` | 跨屏拖选误差明细 + overlay DOM 截图 |
| `s01_*_fullscreen.png` | 全虚拟桌面人工对照截图 |

## 5 排障

- **PowerShell 5.1 解析报 `using System;` / here-string 提前结束**：脚本文件必须是 **UTF-8 with BOM**
  （含中文注释时尤其重要）。若用编辑器重写过，请另存为「UTF-8 带 BOM」。
- **`GetSystemMetrics` 返回 0 / 坐标像被缩放**：忘了 PMv2，或调用顺序在 `Add-Type` 之前。
- **`MONITORINFOEX.szDevice` 全 0**：`cbSize` 必须用真实结构大小（本机 104），不是 40；
  只取矩形时用 `MONITORINFO`（40）亦可。
- **CDP 连不上（fetch failed / 9333 无响应）**：app 未启动或已被其他 spike 的 `start` 杀掉；
  `node scripts/spikes/applab/applab.mjs status` 确认 owner 与 pid。
- **`cargo test` 卡在 "Blocking waiting for file lock"**：其他成员在编译，等待重试即可（不要 `cargo clean`）。
- **标定窗口/截图期间不要跑其他人的全屏截图**：会互相抢桌面内容（各自窗口都置顶）。
