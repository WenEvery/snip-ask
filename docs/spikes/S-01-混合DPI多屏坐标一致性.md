# S-01 混合 DPI 多屏坐标一致性

> 验证记录（按 `docs/技术验证清单.md` §4 模板）。原始数据在 `.verify/spikes/S-01/`（gitignored），
> 脚本可复跑，命令见文末「复跑」一节。

```text
编号：S-01
结论：DONE
日期：2026-11-04（标定/基线 23:52-23:53，修复后真机回归 2026-11-05 00:21）
环境：Windows 11 26200（26200）/ i7-12650H（16 线程）/ 15.8GB RAM / WebView2 138 / Rust 1.99.1 /
      显示器：主屏 \\.\DISPLAY1 2560x1440@150%（物理原点 (0,0)，GetDpiForMonitor=144）+
      副屏 \\.\DISPLAY2 1080x1920@100%（物理原点 (-1080,-470)，=96，竖屏）/ 虚拟桌面物理矩形
      (-1080,-470) 3640x1920 / 构建：v0.1.0 骨架模式（debug）
```

## 1 验收阈值与判定口径

| 项 | 阈值 |
| --- | --- |
| 框选坐标误差（指针 → 物理像素） | ≤ **2px** |
| 跨屏拖选 | 不跳变（含反向拖拽、负坐标、跨 x=0 交界） |
| overlay 覆盖 | client 区 == 虚拟桌面物理矩形（外接矩形，含负坐标），误差 ≤2px |

判定用两类独立测量：
1. **Win32 物理像素标定**（不经过产品代码）：全虚拟桌面大窗口画十字线 → BitBlt 截图 → 程序化检测
   十字中心像素 → 与理论物理坐标比较；
2. **真实应用端到端**：DPI 感知枚举 overlay HWND 的 client 几何 + CDP 合成指针拖拽 → 读取 DOM
   选框位置/尺寸标签 → 回算物理坐标比较。

## 2 方法 / 工具

| 工具 | 路径 | 作用 |
| --- | --- | --- |
| Win32 标定 | `scripts/spikes/s01/s01_dpi_calibration.ps1` | PerMonitorV2；全虚拟桌面无边框置顶窗口按物理坐标画十字线；BitBlt 全桌面截图；程序化检测像素误差；物理↔CSS 往返换算；EnumDisplayMonitors / GetMonitorInfoW / GetDpiForMonitor / SM_*VIRTUALSCREEN 原始数据 |
| overlay 几何探针 | `scripts/spikes/s01/s01_overlay_probe.ps1` | PerMonitorV2 枚举 snip-ask 顶层窗口：GetWindowRect / ClientToScreen / GetClientRect / GetDpiForWindow / DWM 扩展边框 / 样式位，断言 overlay **client 区** == 虚拟桌面 |
| 跨屏拖选探针 | `scripts/spikes/s01/s01_pointer_probe.mjs` | 连 WebView2 CDP（9333），读 `devicePixelRatio`，用 `Input.dispatchMouseEvent` 派发真实拖拽（跨屏正/反向、负坐标、跨 x=0、近全虚拟桌面），读 `.overlay-selection__box` CSS 与「宽×高（物理）」标签回算误差 |
| 全屏截图 | `scripts/spikes/tools/screenshot.ps1` | DPI 感知 BitBlt 全虚拟桌面（人工对照取证） |

> 关键工程约束：crate 有 `#![forbid(unsafe_code)]`，所有 Win32 调用只存在于上述 PowerShell/C# spike
> 脚本；产品代码不允许 unsafe，也不直接调 Win32（Tauri API + `Physical*` 类型）。

## 3 显示器原始数据（2026-11-04 23:52 实测）

进程先 `SetProcessDpiAwarenessContext(-4)`（PerMonitorV2），否则 `GetWindowRect`/`GetSystemMetrics`
返回被 DPI 虚拟化的坐标（已踩坑）。

| 项 | 值 |
| --- | --- |
| `SM_XVIRTUALSCREEN` / `SM_YVIRTUALSCREEN` | **-1080 / -470** |
| `SM_CXVIRTUALSCREEN` / `SM_CYVIRTUALSCREEN` | **3640 / 1920** |
| `SM_CMONITORS` | 2 |

| 显示器 | rcMonitor（物理） | rcWork | GetDpiForMonitor | 缩放 | primary | 回调 rect == MONITORINFO |
| --- | --- | --- | --- | --- | --- | --- |
| `\\.\DISPLAY1` | (0,0) 2560x1440 | (0,0) 2560x1392 | 144 | 1.5 | 是 | 是 |
| `\\.\DISPLAY2` | (-1080,-470) 1080x1920 | (-1080,-470) 1080x1920 | 96 | 1.0 | 否 | 是 |

`EnumDisplayMonitors` 回调 rect 与 `GetMonitorInfoW`（`MONITORINFOEX`，`cbSize` 用真实结构大小
104，**不要**硬编码 40/照抄 32 字符设备名——曾遇到全 0）完全一致。外接矩形由两屏 rcMonitor 计算为
`(-1080,-470) 3640x1920`，与 `SM_*VIRTUALSCREEN` 一致。

> 死区：外接矩形的右上（主屏上方，x>0 且 y<0）与右下（主屏下方，x>0 且 y>1440）**没有任何显示器
> 覆盖**。overlay 窗口几何上覆盖这些区域，但屏幕上不可见；标定点必须落在真实显示器内（工具里用
> 两个死区点做了对照，见 4.1）。

## 4 测量结果

### 4.1 Win32 十字线像素标定（工具 1，不依赖产品代码）

窗口/图像参数：置顶无边框窗口 `SetWindowPos` 到 (-1080,-470) 3640x1920（物理），
`GetWindowRect` 实测 **(-1080,-470) 3640x1920**（== 虚拟桌面，误差 0）；`GetDpiForWindow`=144（1.5）；
全虚拟桌面 BitBlt 截图 `s01_calibration.png` 3640x1920 @ (-1080,-470)。

| 标定点（物理） | 期望 | 截图检测质心 | 误差 (px) |
| --- | --- | --- | --- |
| primary_TL40 | (40,40) | (40.26,40.26) | 0.37 |
| primary_TR40 | (2520,40) | (2520.26,40.26) | 0.37 |
| primary_BR40 | (2520,1400) | (2520.26,1400.26) | 0.37 |
| primary_center | (1280,720) | (1280.26,720.26) | 0.37 |
| secondary_TL40 | (-1040,-430) | (-1039.74,-429.74) | 0.37 |
| secondary_BL40 | (-1040,1410) | (-1039.74,1410.26) | 0.37 |
| secondary_BR40 | (-40,1410) | (-39.74,1410.26) | 0.37 |
| secondary_center | (-540,490) | (-539.74,490.26) | 0.37 |
| boundary_secondary_side | (-6,720) | (-5.74,720.26) | 0.37 |
| boundary_primary_side | (6,1000) | (6.26,1000.26) | 0.37 |
| negative_area | (-900,800) | (-899.74,800.26) | 0.37 |
| negative_y_area | (-540,-300) | (-539.74,-299.74) | 0.37 |
| edge_left_2px_secondary | (-1078,700) | (-1077.57,700.33) | 0.55 |
| edge_right_2px_primary | (2557,700) | (2556.89,700.37) | 0.39 |
| edge_top_2px_secondary | (-540,-468) | (-539.67,-467.57) | 0.55 |
| edge_top_2px_primary | (1280,2) | (1280.33,2.43) | 0.55 |
| edge_bottom_2px_secondary | (-540,1447) | (-539.63,1446.89) | 0.39 |
| edge_bottom_2px_primary | (1280,1437) | (1280.37,1436.89) | 0.39 |

| 统计量 | 值 |
| --- | --- |
| 检测成功 | **18 / 18** |
| 平均误差 | **0.405 px** |
| P50 / P95 / max | **0.37 / 0.55 / 0.547 px** |
| ≤2px 判定 | **通过** |

说明：~0.26px 的系统性质心偏移来自 GDI 3px 宽笔端点（半开区间）与端点像素覆盖，
属**测量工具固有偏差**，远小于 2px 阈值。额外关键结论：**PerMonitorV2 窗口跨混合 DPI 时
DWM 不做位图缩放**——十字线在 150% 主屏与 100% 副屏上都落在精确物理位置（Window DPI 取 144，
只是决定 WebView/字体缩放，不改变像素几何）。

死区对照（预期不可见，中心 ±4px 内无品红像素）：

| 死区点 | 显示器覆盖 | 中心品红像素 | 结论 |
| --- | --- | --- | --- |
| (2000,-300) 主屏上方 | 否 | 0 | 验证不可见 |
| (2000,1448) 主屏下方 | 否 | 0 | 验证不可见 |

### 4.2 跨屏拖选往返换算（工具 1，理论）

口径：物理 → client CSS `c=(p-origin)/dpr` → 回算 `p'=origin+c*dpr`（dpr=窗口实际 1.5，
origin=(-1080,-470)）。

| 用例 | 物理起点→终点 | CSS 起点→终点（浮点） | 浮点误差 | 整数 CSS 误差 |
| --- | --- | --- | --- | --- |
| cross_primary_to_secondary | (100,100)→(-900,800) | (786.67,380)→(120,846.67) | 0 | 0.5 / 0.5 |
| cross_reverse | (-900,800)→(100,100) | (120,846.67)→(786.67,380) | 0 | 0.5 / 0.5 |
| secondary_to_primary | (-540,490)→(1280,720) | (360,640)→(1573.33,793.33) | 0 | 0 / 0.707 |
| negative_area_drag | (-900,800)→(200,200) | (120,846.67)→(853.33,446.67) | 0 | 0.5 / 0.707 |
| primary_corner_drag | (2400,1300)→(-1000,-400) | (2320,1180)→(53.33,46.67) | 0 | 0 / 0.707 |
| single_pixel_step | (0,0)→(1,1) | (720,313.33)→(720.67,314) | 0 | 0.5 / 0.5 |

浮点 CSS 往返误差 0；**整数 CSS**（鼠标 client 坐标被量化）最大 **0.707px**（=1.5 dpr 下的量化上限
`dpr/2`），≤2px 通过。dpr=1.0 时为 0，dpr=2.0 时上限 1px。

对照「错误做法」的误差量级（同样本点，窗口实际 dpr=1.5）：

| 错误策略 | 副屏样本点误差 |
| --- | --- |
| 把主屏 dpr=1.5 当全局 dpr（窗口实际 1.5） | 0（本机巧合一致） |
| 逐屏 dpr 混用（主屏 1.5 / 副屏 1.0 逐点切换） | 最大 **716.16px**（副屏右下）、边界 534px、副屏中心 367px |

结论：**单个跨屏 overlay 只能有一个 dpr**（= WebView 实际 devicePixelRatio）；逐屏切换 dpr 会让
100% 副屏上的坐标整体偏移数百像素，是必须在架构上避免的做法。

### 4.3 修复前基线缺陷（真实应用实测，2026-11-04 23:53）

`capture_freeze` 后 DPI 感知枚举 overlay HWND（`s01_overlay_baseline.json`）：

| 项 | 实测 | 期望（虚拟桌面） |
| --- | --- | --- |
| HWND 标题 | `框选 AI · 选框`（visible=true, topmost） | — |
| GetWindowRect（外框） | **(0,0) 2582x1453** | (-1080,-470) 3640x1920 |
| ClientToScreen(0,0)（client 原点） | **(11, 2)** | (-1080,-470) |
| GetClientRect（client 尺寸） | **2560x1440** | 3640x1920 |
| GetDpiForWindow | 144（1.5） | — |
| DWM 扩展边框 | (9,0) 2564x1444 | — |
| 样式 | caption=true, thickFrame=false, popup=false | — |

overlay WebView：`devicePixelRatio=1.5, innerWidth=1707, innerHeight=960`
（= 2560/1.5、1440/1.5，证实只覆盖主屏）。

证据截图：`s01_overlay_baseline_fullscreen.png`（全虚拟桌面 3640x1920：左侧副屏原样可见，
右侧主屏被 overlay 占位层覆盖）。

两个缺陷：
1. **只覆盖主屏**：`virtual_desktop()` 用 `primary_monitor()`，overlay 只创建 2560x1440；
   副屏 (-1080,-470) 区域完全没有 overlay —— 在副屏上框选不可用；
2. **client 原点偏移 (11,2)px**（更隐蔽）：Tauri builder 的 `.position/.inner_size` 被当作逻辑像素，
   且 tao 无边框窗口保留 DWM 阴影 inset，`GetWindowRect`(外框) 与 client 区不一致。即使单屏，
   前端 `toPhys` 也会整体偏 (11,2)px（>2px 阈值，单屏场景同样不合格）。

### 4.4 修复内容

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/platform/coords.rs` | 新增纯函数 `union_rect`（多矩形外接，含负坐标/负宽高归一化）、`rect_contains`（左闭右开）、`mixed_dpr_error`（逐屏混用 dpr 的误差量级）；补 7 个单测 |
| `src-tauri/src/platform/capture.rs` | `primary_monitor_geometry` → `virtual_desktop_geometry`：用 `app.available_monitors()` 的物理矩形做外接矩形；`SkeletonCapturer` 帧尺寸改为虚拟桌面口径 |
| `src-tauri/src/services/capture_service.rs` | `virtual_desktop()` 改返回外接矩形 |
| `src-tauri/src/app/windows.rs` | overlay 改为**物理**几何：`set_size(PhysicalSize)`（client 尺寸）+ `set_position(PhysicalPosition)`（外框位置）+ 按 `inner_position()` 迭代校正 client 原点；`.shadow(false)` 消除 DWM 阴影 inset；`show_overlay` 前后各对齐一次；新增 `settle_overlay_geometry()`（异步等 DPI 消息稳定后复核，最多 5×40ms）；新增 `overlay_virtual_desktop()`：**dpr 取 overlay 窗口实际 `scale_factor()`**；`emit_overlay_snapshot` 同步改用 |
| `src-tauri/src/commands/capture.rs` | `show_overlay` 后 `settle_overlay_geometry` 再下发 `overlay:ready`（rect=外接矩形、dpr=窗口实际缩放比）；幂等路径同样处理 |
| `src/windows/overlay/coords.ts` | 补契约注释：client 原点 == virtualRect 原点、dpr == WebView devicePixelRatio、禁止逐屏改 dpr（换算公式不变，实测无需改算法） |
| `src-tauri/tests/s01_multimonitor_geometry.rs` | 新增 6 个集成测试：本机实测布局外接矩形、负坐标/竖向/横向排布、统一 dpr 换算不变式（1.0/1.25/1.5/2.0）、逐屏混用 dpr 偏移量级、死区与交界点归属 |

设计要点：
- **为什么 dpr 要取窗口 scale_factor**：单个跨屏 WebView 只有一个 devicePixelRatio；本机 Windows 按
  「窗口与各显示器重叠面积最大者」决定窗口 DPI（实测 144，即使左上角在 100% 副屏上）。下发主屏/逐屏
  dpr 都会让副屏坐标按比例错位（4.2 对照表）。
- **为什么还要 settle**：Windows 的 DPI/窗口位置消息异步投递，`show()` 后可能再重排；对齐循环幂等，
  已对齐时 40ms 即返回。

### 4.5 修复后真机回归（2026-11-05 00:21，真实 debug 应用）

复跑链路：`applab start` → 主窗口 `invoke('capture_freeze')` → Win32 overlay 探针 → CDP 跨屏拖选探针
（6 组）→ 全虚拟桌面截图 → `applab stop`。

**① overlay 几何（`s01_overlay_after.json`，DPI 感知 Win32）**

| 项 | 修复前 | 修复后 | 期望 | 误差 |
| --- | --- | --- | --- | --- |
| GetWindowRect（外框） | (0,0) 2582x1453 | **(-1080,-470) 3640x1920** | (-1080,-470) 3640x1920 | **0px** |
| ClientToScreen(0,0)（client 原点） | (11, 2) | **(-1080,-470)** | (-1080,-470) | **0px** |
| GetClientRect（client 尺寸） | 2560x1440 | **3640x1920** | 3640x1920 | **0px** |
| DWM 扩展边框 | (9,0) 2564x1444 | (-1080,-470) 3640x1920 | — | 0px |
| GetDpiForWindow | 144 (1.5) | 144 (1.5) | — | — |
| 断言「外框 == 虚拟桌面 ≤2px」 | **False** | **True** | — | — |
| 断言「client 原点+尺寸 == 虚拟桌面 ≤2px」 | **False** | **True** | — | — |

**② overlay WebView 口径**：`devicePixelRatio=1.5`、CSS 视口 `2427x1280`
（= 3640/1.5 × 1920/1.5，2427 为 2426.67 取整）、`screenX/screenY=(-720,-314)`
（= (-1080)/1.5, (-470)/1.5，证明 client 原点确实在虚拟桌面左上角）。
CDP 拖拽回算得到的 implied dpr = **1.5**，与后端下发 dpr 一致（`payloadDprMatchesWebviewDpr=true`）。

**③ CDP 合成指针跨屏拖选（`s01_pointer_probe.json`）**

| 用例 | 起点→终点（物理） | 期望选区（物理） | 回算选区（物理） | 最大误差 |
| --- | --- | --- | --- | --- |
| cross_primary_to_secondary | (100,100)→(-900,800) | (-900,100) 1000x700 | (-900,100) 1000x700 | **0** |
| cross_secondary_to_primary | (-900,800)→(100,100) | (-900,100) 1000x700 | (-900,100) 1000x700 | **0** |
| cross_into_negative_y | (1280,720)→(-540,-300) | (-540,-300) 1820x1020 | (-540,-300) 1819.99x1020 | **0.01px** |
| straddle_x0_boundary | (-200,700)→(200,700) | (-200,700) 400x0 | (-200,700) 400x0 | **0** |
| near_full_virtual_desktop | (-1000,-400)→(2500,1380) | (-1000,-400) 3500x1780 | (-1000,-400) 3499.99x1780.01 | **0.01px** |
| negative_area_reverse_drag | (-900,800)→(-100,100) | (-900,100) 800x700 | (-900,100) 800x700 | **0** |

- 6/6 组通过，**字段级最大误差 0.01px、平均 0.003px**（阈值 2px）；选框物理尺寸标签（「宽×高（物理）」）
  与期望一致（labelMatchesExpected 全 true）。
- 0.01px 来自 CSS 浮点显示精度（`toCssRect` 的除法/浏览器取整），非坐标语义误差。
- **提交链路仍可用**：最后留下一次有效跨屏选框并 `mouseReleased` → `selection_commit` 成功、
  `hasMenu=true`、选框仍渲染（overlay 端到端没被几何修复破坏）。

**④ 全屏截图 / 覆盖对照（`s01_overlay_after_fullscreen.png`，3640x1920）**

截图内容：overlay 深色占位网格（骨架模式 `frame.dataUrl=null`）覆盖整个虚拟桌面，跨两屏的选框
（蓝框 + 尺寸标签）、动作菜单在物理位置渲染；主屏上方死区显示为黑（无显示器，预期）。

像素抽样（16x16 平均亮度，物理点 → 截图像素）：

| 采样点（物理） | 修复前亮度 | 修复后亮度 | 说明 |
| --- | --- | --- | --- |
| secondary_center (-540,490) | 63.4（壁纸） | 22.3（overlay） | 副屏被 overlay 覆盖 |
| secondary_bottom (-540,1300) | 0.0（壁纸暗部） | 20.3（overlay 网格） | 副屏出现 overlay |
| secondary_top (-200,-400) | 0.0 | 23.3 | 负坐标区域被覆盖 |
| primary_center (1280,720) | 39.0 | 27.0 | 主屏仍覆盖 |
| primary_topleft (300,300) | 21.4 | 29.8 | 主屏仍覆盖 |
| primary_bottomright (2400,1300) | 17.0 | 18.7 | 主屏仍覆盖 |

修复前副屏看到的是壁纸（亮/暗处 63.4/0），修复后整屏统一为 overlay 占位层（~20-30），
与「client 区 = 虚拟桌面」的 Win32 断言互相印证。


## 5 结果与结论

| 验收项 | 阈值 | 实测 | 判定 |
| --- | --- | --- | --- |
| overlay client 区 == 虚拟桌面（含负坐标） | ≤2px | 外框/client 原点/client 尺寸均 **0px** 偏差 | **通过** |
| 混合 DPI 多屏框选坐标误差 | ≤2px | 端到端 6 组跨屏拖选最大 **0.01px**；Win32 标定 18/18 最大 **0.547px** | **通过** |
| 跨屏拖选不跳变（正/反向、负坐标、跨 x=0、近全桌面） | 不跳变 | 6/6 组字段级误差 ≤0.01px；提交后菜单正常 | **通过** |
| 换算算法往返误差（浮点/整数 CSS） | ≤2px | 浮点 0px；整数 CSS 最大 0.707px | **通过** |
| dpr 口径 | 必须等于 WebView devicePixelRatio | 1.5==1.5，payload 与拖拽回算一致 | **通过** |

**结论：DONE**（本机混合 DPI 双屏：2560x1440@150% + 1080x1920@100%，虚拟桌面
(-1080,-470) 3640x1920）。

修复 2 个真实缺陷：
1. `virtual_desktop()` 只用主屏 → overlay 只覆盖主屏；改为所有显示器物理矩形外接矩形；
2. Tauri 逻辑像素 + DWM 阴影 inset 导致 client 原点相对外框偏 (11,2)px（单屏也会错）；
   改为物理几何 + client 原点迭代校正 + `settle_overlay_geometry` 异步复核；
   同时 dpr 改为取 overlay 窗口实际 `scale_factor()`（混合 DPI 下统一值，禁止逐屏 dpr）。

**处置**：达标 → 建议把「overlay client 区必须等于虚拟桌面外接矩形、dpr=窗口实际 scale_factor、
单个跨屏 overlay 不允许逐屏 dpr」三条写入实施架构「实现边界 / M06 坐标约定」；本仓库已用
`src-tauri/tests/s01_multimonitor_geometry.rs`（6 项）+ `platform/coords.rs` 单测（7 项）固化。


## 6 遗留 / 风险

1. **4K 真机未覆盖 / 副屏可能是虚拟显示器**：本机枚举到 `MuMu Virtual Display Adapter` 与
   `GameViewer Virtual Display Adapter`，副屏 1080x1920 大概率由虚拟显示适配器提供（非物理 4K 屏）。
   因此「4K@200% + 1080p@100%」组合未实测；理论换算不变式已由单测覆盖 1.0/1.25/1.5/2.0，但需在
   真实 4K 机器复跑标定 + 端到端（尤其 200% 缩放下的 WebView devicePixelRatio 与窗口 DPI 归属）。
2. **DPI 归属屏假设**：窗口跨屏 DPI 由 Windows「最大重叠显示器」决定；`overlay_virtual_desktop` 以
   `scale_factor()` 自适应，但若未来显示器排布让最大重叠屏在运行中变化（拖动/热插拔），需要监听
   `WindowEvent::ScaleFactorChanged` 主动重发 `overlay:ready`（当前每次 `capture_freeze` 重新对齐兜底）。
3. **跨屏 UI 尺寸观感**：统一 dpr=1.5 时，100% 副屏上的 overlay UI（提示文字、动作菜单）按 150% 逻辑
   尺寸渲染会显得偏小（坐标精度不受影响）。若产品要求各屏 UI 尺寸一致，需要改为「每显示器一个
   overlay 窗口」或按屏分片渲染 —— 属于 UX 增强，不影响 S-01 坐标验收。
4. **死区区域**：外接矩形内的死区（主屏上方/下方）虽然没有显示器，但 overlay 仍接收指针事件；
   命中测试应视为无效选区（后续可在 `selection_commit` 用 `rect_contains` 逐屏校验）。
5. 真实捕获（BitBlt/WGC）仍为 `TODO(real)`：S-01 只验证了「冻结帧尺寸/overlay 几何/坐标换算」口径，
   真实帧内容的多屏拼接与负坐标偏移需在实现捕获时按本矩形口径回归。

## 7 复跑

```powershell
# 0) 前置：cargo 在 PATH；应用实例按 scripts/spikes/README.md 的 applab 纪律排队（同一时刻只跑一个 app）
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"

# 1) Win32 标定（不需要产品 app；会短暂覆盖整个虚拟桌面 ~2s）
powershell -ExecutionPolicy Bypass -File scripts/spikes/s01/s01_dpi_calibration.ps1 -OutDir .verify/spikes/S-01

# 2) 构建 debug 可执行文件（改了 Rust 后）
cargo build --manifest-path src-tauri/Cargo.toml

# 3) 起真实应用（独占锁，跑完必须 stop）
node scripts/spikes/applab/applab.mjs start --owner dpi-coords --wait-sec 900

# 4) 触发冻结（overlay 惰性创建）
node scripts/spikes/applab/applab.mjs eval --window main --js "window.__TAURI_INTERNALS__.invoke('capture_freeze')"

# 5) overlay client 几何断言（PerMonitorV2，物理像素）
powershell -ExecutionPolicy Bypass -File scripts/spikes/s01/s01_overlay_probe.ps1 -Out .verify/spikes/S-01/s01_overlay_after.json

# 6) CDP 合成指针跨屏拖选（读 devicePixelRatio + DOM 选框回算物理误差）
node scripts/spikes/s01/s01_pointer_probe.mjs --vd -1080,-470,3640,1920 --out .verify/spikes/S-01/s01_pointer_probe.json

# 7) 全虚拟桌面人工对照截图
powershell -ExecutionPolicy Bypass -File scripts/spikes/tools/screenshot.ps1 -Out .verify/spikes/S-01/s01_overlay_after_fullscreen.png

# 8) 释放实例（必须）
node scripts/spikes/applab/applab.mjs stop --owner dpi-coords
```

## 8 原始数据文件（`.verify/spikes/S-01/`，gitignored）

| 文件 | 内容 |
| --- | --- |
| `s01_monitors.json` | 显示器原始数据（EnumDisplayMonitors/GetMonitorInfoW/GetDpiForMonitor/SM_*VIRTUALSCREEN） |
| `s01_calibration.json` / `s01_calibration_summary.json` | 十字线期望/检测/误差、死区对照、往返换算、错误 dpr 对照 |
| `s01_calibration.png` | 全虚拟桌面十字线标定截图（3640x1920） |
| `s01_overlay_baseline.json` | 修复前 overlay 几何（缺陷证据） |
| `s01_overlay_baseline_fullscreen.png` | 修复前全屏截图（overlay 只覆盖主屏） |
| `s01_overlay_after.json` | 修复后 overlay 几何（client 原点/尺寸断言） |
| `s01_pointer_probe.json` / `s01_pointer_probe-overlay.png` | 修复后跨屏拖选测量 + overlay DOM 截图 |
| `s01_overlay_after_fullscreen.png` | 修复后全虚拟桌面截图 |
