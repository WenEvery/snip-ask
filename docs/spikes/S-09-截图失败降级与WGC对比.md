# S-09 截图失败降级（DRM / 全屏独占）与 WGC 对比

> 对应 `docs/技术验证清单.md` S-09（P1）：播放 Netflix/受保护视频、进入全屏独占游戏、锁屏后捕获；
> 对比 BitBlt 与 WGC 两条路径，验收「黑屏能被检测并返回 `SA-1001`（retryable），自动尝试 WGC；
> 界面给出明确说明，不崩溃」。记录格式按清单 §4 模板。

## 1 编号与结论

- **编号**：S-09（P1）
- **结论**：**DONE（等价受保护内容验证完成）**

核心结论（2026-11-05 本机实测）：

1. **WGC 链路实机跑通**：独立 spike 工具用 `Windows.Graphics.Capture`
   （`IGraphicsCaptureItemInterop` + `Direct3D11CaptureFramePool` + `SoftwareBitmap` 回读）成功抓帧；
   普通窗口 900×600 的 BitBlt 帧与 WGC 窗口项帧 **sha256 完全相同**
   （`cd9a8571cf907c34…`，3/3 次），证明 WGC 结果与 BitBlt 像素级一致、不是 mock/占位。
2. **`WDA_MONITOR(0x01)` / 真实 DRM 保护面 → 黑屏且 WGC 无法绕过**：BitBlt 窗口区域、PrintWindow、
   WGC 窗口项三者都只得到 **纯黑帧**（blackRatio=1.000000、meanLuma=0.00、stdDev=0.00，3/3 次）。
   因此「BitBlt 黑屏 → 尝试 WGC → 仍黑 → `SA-1001`」的降级链是正确的；WGC 不是万能绕过。
3. **`WDA_EXCLUDEFROMCAPTURE(0x11)` → 窗口从截屏中「消失」，不是黑屏**：BitBlt 截到的是**背后桌面/其他窗口**
   （blackRatio=0.018248，不是黑屏）；WGC 窗口项 **3/3 次在 5000ms 内完全取不到帧**
   （平均 7753ms 返回 `frameTimeout`）。这证明检测逻辑除了「黑帧」还必须把「未取到帧/空帧」判为失败，
   否则 0x11 类受保护内容会被静默截成错误画面。
4. **整帧黑阈值在混合 DPI 部分遮挡时漏检**：全屏 + `WDA_MONITOR` 时主屏区域 blackRatio=1.000000，
   但**虚拟桌面整帧** blackRatio 只有 **0.764851**（副屏仍有内容）→ 按整帧 99% 阈值不会判黑。
   真实捕获实现必须按**目标区域/显示器切片**判定（见 §7 处置、§8 遗留）。
5. **界面/错误契约链路成立**：`SA-1001` 的 `retryable=true`、`fallback=["wgc","retry"]` 均在契约常量表中
   （`contract/errors.rs:145-164`），overlay 前端以 `role="alert"` 渲染 `error.code` + `error.message`
   （`OverlayWindow.tsx:366-368`），捕获失败通过 `Err(AppError)` 返回并取消会话、无 panic
   （`commands/capture.rs:99-104`）。纯 safe 算法已落地并新增 16 项单测。

> 说明：应用当前捕获仍是 `SkeletonCapturer`（`data_url=None`），**真实 Win32 捕获不进入产品代码**
> （`Cargo.toml [lints.rust] unsafe_code = "forbid"`）。因此「真实对比」由独立 spike 工具完成，
> 产品侧只落纯 safe 的「黑屏检测 + 降级决策 + SA-1001 映射」及单测。真机 Netflix/全屏独占/锁屏未覆盖，
> 已在 §8 遗留逐项列出。

## 2 日期与环境

| 项 | 值 |
| --- | --- |
| 日期 | 2026-11-05（采集 19:01，本机时间） |
| 构建 commit | `3f1b263`（`git rev-parse --short HEAD`；工作区未提交改动：本任务只改 `src-tauri/src/platform/capture.rs`、`src-tauri/src/services/capture_service.rs`，并新增 scripts/docs/`.verify` 产物） |
| OS | **Windows 11 25H2 build 26200.9711**（`HKLM\…\CurrentVersion` 的 `ProductName` 兼容性仍显示 “Windows 10 Pro”，属系统已知命名；`DisplayVersion=25H2`、`UBR=9711`） |
| CPU | 12th Gen Intel Core i7-12650H（10 核 16 线程，10C16T） |
| 内存 | 15.78 GiB |
| 显卡 | NVIDIA GeForce RTX 4060 Laptop GPU（驱动 32.0.15.9597）+ 2 个虚拟显示适配器（MuMu / GameViewer） |
| 显示 | **混合 DPI 双屏**：主屏 2560×1440 @150%（物理原点 (0,0)）+ 副屏 1080×1920 @100%（物理原点 (-1080,-470)）；虚拟桌面物理矩形 **(-1080,-470) 3640×1920** |
| 采集工具链 | .NET SDK 10.0.100 / TFM `net10.0-windows10.0.26100.0`（WinRT 投影）/ PowerShell 5.1.26100.9711 |
| Rust | rustc 1.99.1 / cargo 1.99.1（stable-x86_64-pc-windows-msvc） |
| 采集时桌面状态 | DSH Web GUI、Vite 终端等其他协作进程窗口在场（仅影响「受保护窗口背后露出的内容」画面取证，不影响各路径的黑/非黑模式结论；见 §8 遗留） |

## 3 步骤

### 3.1 独立 spike 工具（不碰产品代码）

- `scripts/spikes/s09/S09Capture.csproj` + `Program.cs`：.NET 10 控制台工具（`UseWindowsForms` 用来造窗口；
  输出 PNG/JSON，`unsafe` 未使用）。
  - **BitBlt**：`GetDC(NULL)` + `CreateDIBSection` + `BitBlt(SRCCOPY|CAPTUREBLT)` 到 32bpp 顶朝下 DIB；
  - **PrintWindow**：`PrintWindow(hwnd, memDC, PW_RENDERFULLCONTENT=2)` 对照；
  - **WGC**：`RoGetActivationFactory("Windows.Graphics.Capture.GraphicsCaptureItem")` → QI
    `IGraphicsCaptureItemInterop`（头文件 `windows.graphics.capture.interop.h` 中该接口**直接派生自 IUnknown**，
    故 `CreateForWindow/CreateForMonitor` 在 vtable slot 3/4；item IID `79C3F95B-31F7-4EC2-A464-632EF5D30760`）
    → `Direct3D11CaptureFramePool.CreateFreeThreaded`（D3D11 设备经 `D3D11CreateDevice` +
    `CreateDirect3D11DeviceFromDXGIDevice`）→ `TryGetNextFrame` → `SoftwareBitmap.CreateCopyFromSurfaceAsync`
    → `CopyToBuffer` + `DataReader` 回读 BGRA；`IsBorderRequired=false` 关闭捕获黄框。
  - **受保护窗口**：`SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE=0x11 / WDA_MONITOR=0x01)`；
    内容为确定性四色象限 + 大号文字，便于肉眼与统计双重判别。
- `scripts/spikes/s09/wgc_vs_bitblt.ps1`：构建并运行上述工具（构建产物只写 `.verify/`），生成环境快照、
  原始数据、精选聚合数据，并执行 **17 项自动校验**（全部 PASS 才 `exit 0`）。

### 3.2 场景 × 路径矩阵（每个场景 3 次重复，共 90 个采样点）

| 场景 | 窗口 | affinity | 等价真实场景 |
| --- | --- | --- | --- |
| `normal` | 900×600 @ (120,120) | WDA_NONE | 普通内容基线 |
| `wda_exclude` | 900×600 @ (120,120) | `WDA_EXCLUDEFROMCAPTURE(0x11)` | 受保护窗口（从截屏中排除） |
| `wda_monitor` | 900×600 @ (120,120) | `WDA_MONITOR(0x01)` | 捕获时显示为黑屏的受保护窗口（DRM 黑屏近似） |
| `fullscreen_normal` | 覆盖主屏 2560×1440 | WDA_NONE | 普通全屏应用（全屏独占的近似） |
| `fullscreen_wda_monitor` | 覆盖主屏 | `WDA_MONITOR(0x01)` | **全屏播放受保护视频 ≈ 整屏黑** |
| `fullscreen_wda_exclude` | 覆盖主屏 | `WDA_EXCLUDEFROMCAPTURE(0x11)` | 全屏受保护窗口「消失」 |

每个场景依次采集 5 条路径：`bitblt_region`（BitBlt 窗口区域）、`bitblt_desktop`（BitBlt 整个虚拟桌面，
并单独统计窗口所在区域）、`printwindow`、`wgc_window`（WGC 窗口项）、`wgc_monitor`（WGC 主屏显示器项）。

### 3.3 黑屏判定口径（工具与产品 Rust 侧一致）

`luma = 0.114·B + 0.587·G + 0.299·R`；**黑像素 = luma < 16**；
**判黑 = 黑像素占比 ≥ 99% 且整帧均值 < 16 且亮度标准差 < 6**（避免深色 UI / 黑底白字误判）。
工具输出 `blackRatio / meanLuma / stdDev / uniqueLumaBuckets / sha256`，PNG 落盘供人工核对。

### 3.4 产品侧（safe Rust）落地

- `src-tauri/src/platform/capture.rs`：`FrameLumaStats`、`luma_stats_from_bgra`、`is_black_frame`、
  `is_capture_failed`、`CaptureBackend`、`classify_failure`、`capture_failed_error`（SA-1001 + 上下文）；
- `src-tauri/src/services/capture_service.rs`：`CaptureOutcome`、`CaptureDecision`、
  `resolve_capture(bitblt, wgc)` **降级决策链**（BitBlt 内容→用；黑屏/空帧→WGC；WGC 内容→用；
  WGC 黑屏/空帧/无帧→`SA-1001`）；
- 新增 **16 项单测**（正常帧不误判、纯黑帧、近黑渐变边界、深色 UI+亮字、空帧/无帧、SA-1001 字段、
  决策链各分支、阈值与工具一致、后端名与契约 fallback token 一致）。

### 3.5 复跑命令

```powershell
# 真实捕获对比（构建到 .verify，不启动应用、不跑 pnpm build）
powershell -ExecutionPolicy Bypass -File scripts\spikes\s09\wgc_vs_bitblt.ps1 -Reps 3

# 产品侧纯算法单测
cargo test --manifest-path src-tauri/Cargo.toml capture
# 全量（本机实测 194 passed / 0 failed）
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
rustfmt --check --edition 2021 src-tauri/src/platform/capture.rs src-tauri/src/services/capture_service.rs
```

## 4 原始数据

原始数据均在 `.verify/spikes/S-09/`（gitignored）：

| 路径 | 内容 |
| --- | --- |
| `.verify/spikes/S-09/raw/results.json` | 90 个采样点全量结果（112,382 B）：每点含场景/路径/rep/耗时/源区域/整帧统计/区域统计/sha256/PNG 路径 |
| `.verify/spikes/S-09/raw/env.json` | 环境快照（OS build、CPU、内存、显卡、显示器 DPI/bounds、虚拟桌面、dotnet/PS 版本、commit） |
| `.verify/spikes/S-09/raw/manifest.json` | 脚本/源文件/可执行文件 sha256、运行 stdout、产物清单 |
| `.verify/spikes/S-09/shots/*.png` | **84 张**原始截图（90 个采样点 − 6 个 WGC 无帧 = 84；含整虚拟桌面 3640×1920 与各路径画面） |

精选聚合数据已入 git：`docs/spikes/data/s09/`
（`summary.json` 57,036 B：6 场景 × 5 路径聚合 + 17 项 checks；`env.json`；`shots/` 6 张缩略证据图）。

工具与源文件指纹（`manifest.json` 记录）：

| 文件 | sha256 |
| --- | --- |
| `scripts/spikes/s09/Program.cs` | `7a54f9cdb6d68e999db63c0b8d91bf23cd183a8fee937a82b48a65d6a843b7d8` |
| `scripts/spikes/s09/S09Capture.csproj` | `5f04b5bbfa5725a597d3539ba087933cdf0e567246a943cfa8301c1a03b962b9` |
| `scripts/spikes/s09/wgc_vs_bitblt.ps1` | `72941a3e850d86912df40b8da6b0374582598103e7eff1a72ecaf20602553287` |
| `S09Capture.exe`（构建产物） | `c9ba29e0fcbab66e976cee347cd95b6ccc83bf050f32dad275f06c526e264586` |

## 5 结果

### 5.1 主结果表（3 次重复，均取 3 次聚合；`black` 为按 §3.3 口径的判黑）

| 场景 | 路径 | ok/样本 | 判黑次数 | blackRatio | meanLuma | stdDev | 平均耗时 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| normal | bitblt_region | 3/3 | 0 | 0.140630 | 126.508 | 79.255 | 9.7ms |
| normal | printwindow | 3/3 | 0 | 0.140630 | 126.508 | 79.255 | 8.1ms |
| normal | **wgc_window** | 3/3 | 0 | 0.140630 | 126.508 | 79.255 | 48.8ms |
| normal | wgc_monitor（窗口区域） | 3/3 | 0 | 0.140630 | 126.508 | — | 57.6ms |
| wda_monitor | **bitblt_region** | 3/3 | **3** | **1.000000** | **0.000** | **0.000** | 10.7ms |
| wda_monitor | printwindow | 3/3 | 3 | 1.000000 | 0.000 | 0.000 | 11.8ms |
| wda_monitor | **wgc_window** | 3/3 | **3** | **1.000000** | **0.000** | **0.000** | 52.5ms |
| wda_monitor | bitblt_desktop（窗口区域） | 3/3 | 3 | 1.000000 | 0.000 | — | 65.5ms |
| wda_monitor | wgc_monitor（窗口区域） | 3/3 | 3 | 1.000000 | 0.000 | — | 76.4ms |
| wda_exclude | **bitblt_region** | 3/3 | 0 | **0.018248** | **200.447** | 85.316 | 13.2ms |
| wda_exclude | printwindow | 3/3 | 3 | 1.000000 | 0.000 | 0.000 | 13.6ms |
| wda_exclude | **wgc_window** | **0/3** | — | — | — | — | **7753.5ms**（`frameTimeout(5000ms)`） |
| wda_exclude | wgc_monitor（窗口区域） | 3/3 | 0 | 0.018437 | — | — | 90.0ms |
| fullscreen_normal | bitblt_region | 3/3 | 0 | 0.065285 | 137.188 | 72.927 | 39.7ms |
| fullscreen_normal | wgc_window | 3/3 | 0 | 0.065285 | 137.188 | 72.927 | 67.8ms |
| fullscreen_wda_monitor | **bitblt_region** | 3/3 | **3** | **1.000000** | **0.000** | **0.000** | 42.2ms |
| fullscreen_wda_monitor | **bitblt_desktop（整帧）** | 3/3 | **0** | **0.764851** | 36.394 | 75.624 | 78.2ms |
| fullscreen_wda_monitor | bitblt_desktop（主屏区域） | 3/3 | 3 | 1.000000 | 0.000 | — | — |
| fullscreen_wda_monitor | **wgc_window** | 3/3 | **3** | 1.000000 | 0.000 | 0.044 | 74.7ms |
| fullscreen_wda_monitor | **wgc_monitor** | 3/3 | **3** | 1.000000 | 0.000 | 0.044 | 83.2ms |
| fullscreen_wda_exclude | **bitblt_region** | 3/3 | 0 | **0.074485** | **173.270** | 96.790 | 43.4ms |
| fullscreen_wda_exclude | printwindow | 3/3 | 3 | 1.000000 | 0.000 | 0.000 | 48.8ms |
| fullscreen_wda_exclude | **wgc_window** | **0/3** | — | — | — | — | **7770.5ms**（`frameTimeout(5000ms)`） |

补充（同一次运行）：

- normal 场景 **BitBlt 帧、PrintWindow 帧、WGC 窗口项帧 sha256 完全相同**
  = `cd9a8571cf907c348ad55dd52484dda4157fba017d5a6f898c1073fa742c2c42`；
- wda_monitor 的 BitBlt/PrintWindow/WGC 窗口项三帧 sha256 均为
  `942dcd30f2ceecbf…`（纯黑），三路径在保护面上完全等价（都拿不到内容）；
- fullscreen_wda_monitor：BitBlt 窗口区域 sha `cdea412a9ed18f71…`（纯黑）；WGC 窗口项/主屏项 sha
  `97b938aed04f3701…`（blackRatio=0.9998，均值 0.00、stdDev 0.044，残留约 0.02% 非零像素为捕获边界像素，
  仍满足判黑阈值）；
- fullscreen_normal：BitBlt 与 WGC 的 blackRatio/meanLuma 完全相同（0.065285 / 137.188），sha256 不同
  （`2bc5877a…` vs `9f4a1e18…`，属窗口边界/合成时序的亚像素差异，不影响判黑结论）。

### 5.2 自动校验（工具脚本 `-Reps 3` 输出：**17/17 PASS**）

覆盖：普通窗口 BitBlt/WGC 非黑且像素一致；`WDA_MONITOR` 的 BitBlt/WGC 窗口区域与 WGC 主屏区域均为黑；
`WDA_MONITOR` 整桌面**整帧不判黑**（暴露漏检边界）；`WDA_EXCLUDEFROMCAPTURE` 的 BitBlt/WGC 主屏区域非黑、
WGC 窗口项 0/3 无帧；全屏 + `WDA_MONITOR` 区域黑、整帧 non-black、WGC 黑；全屏 + 0x11 非黑且 WGC 无帧。
明细见 `docs/spikes/data/s09/summary.json` 的 `checks` 数组。

### 5.3 逐条验收对照

| 验收项（清单 S-09） | 证据 | 结论 |
| --- | --- | --- |
| 黑屏能被检测 | 黑屏判定纯函数 + 10 项单测（纯黑、近黑渐变边界、深色 UI 不误判、空帧）；spike 实测纯黑帧 blackRatio=1.000000、mean=0.00、std=0.00 | ✅ 成立 |
| 返回 `SA-1001`（retryable） | `capture_failed_error` 构造 `ErrorCode::CaptureFailed="SA-1001"`；单测断言 `retryable=true`；契约 `errors.rs:145-164` | ✅ 成立 |
| 自动尝试 WGC | `resolve_capture` 决策链（BitBlt 黑/空 → `FallbackToWgc`）+ 6 项单测；spike 实测 WGC 正常窗口可抓帧且与 BitBlt 像素一致、受保护窗口 WGC 无帧/黑 | ✅ 策略成立（真实调用待接入，见 §7/§8） |
| 界面给出明确说明 | overlay `error` 状态：`toAppError(...)`（`src/shared/ipc/tauri.ts:25-54`）→ `OverlayWindow.tsx:366-368` 以 `role="alert"` 渲染 `error.code` + `error.message`；`capture_failed_error` 的 message 为可操作中文（「受保护/全屏独占/WGC 也无法获取/请退出独占应用…」） | ✅ 成立（代码事实；未跑应用截图，见 §8） |
| 不崩溃 | `SkeletonCapturer` 失败路径返回 `Err(AppError)` 并取消会话（`commands/capture.rs:99-104`），前端 `.catch(handleError)`；纯函数无 `unsafe`、无未捕获异常（`capture_failed_error` 中唯一的 `expect` 由 `BlackFrame` 分支保证有统计）；`cargo test` 194 passed、`cargo clippy -D warnings` 通过 | ✅ 成立 |

### 5.4 为什么 WGC 不能当「万能绕过」

实测把两种「保护」分开，结论不同但都指向同一处理方式：

- `WDA_MONITOR(0x01)`（对应真实 DRM 保护面：捕获时整块变黑）：**BitBlt 黑 + WGC 也黑** → 只有报错一条路；
- `WDA_EXCLUDEFROMCAPTURE(0x11)`（窗口从捕获中排除）：**BitBlt 截到背后内容（不是黑屏）+ WGC 窗口项无帧** →
  如果只看「是否黑屏」会误判成功并截到错误画面，因此必须把 WGC 空帧/超时也判失败。

二者最终都映射到 `SA-1001`（retryable，fallback `["wgc","retry"]`），WGC 只是「多数遮挡/深色场景下更可能拿到内容」的
增强路径，而不是绕过受保护内容的工具（不做任何破解）。

## 6 产品落地（safe Rust）细节

| 文件 | 新增内容 |
| --- | --- |
| `src-tauri/src/platform/capture.rs` | `BLACK_LUMA_MAX/BLACK_RATIO_MIN/BLACK_STD_DEV_MAX`、`FrameLumaStats`、`luma_stats_from_bgra`、`is_black_frame`、`is_capture_failed`、`CaptureBackend{as_str}`、`CaptureFailure`、`classify_failure`、`capture_failed_error`；模块文档写入 S-09 结论与「整帧阈值漏检」边界 |
| `src-tauri/src/services/capture_service.rs` | `CaptureOutcome{Content,BlackFrame,EmptyFrame}`、`CaptureDecision{UseFrame,FallbackToWgc,Failed}`、`resolve_capture(bitblt, wgc)`；模块文档写入降级链与实测依据 |

- `capture_failed_error` 的 `context` 记录 `capture.backend`（`"bitblt"`/`"wgc"`）、`capture.blackFrame`、
  `capture.blackRatio`、`capture.meanLuma`；`detail` 形如 `backend=wgc：画面为黑屏（…）`，便于日志排查。
- 验证命令与结果：
  - `cargo test --manifest-path src-tauri/Cargo.toml capture` → **18 passed / 0 failed**
    （含 1 个既有无关测试；本任务新增 16 项：capture 侧 10 + 服务侧 6）；
  - `cargo test --manifest-path src-tauri/Cargo.toml` → 全量 **194 passed / 0 failed**；
  - `cargo clippy --all-targets -- -D warnings` → 通过；`rustfmt --check`（仅两个改动文件）→ 通过。
- 与 `release-metrics` 的 cargo 构建按 `.verify/p1/build.lock` 串行（等待/持锁后删除，未 kill 他人进程）。

## 7 处置

1. **达标 → 记录实施边界**：黑屏检测/降级决策/SA-1001 映射以纯 safe 函数落地并单测；真实捕获接入时按本记录
   的判定口径与降级链执行（`capture.backend` 写入错误上下文）。
2. **真实捕获实现注意事项（交接给 S-09 后续实现/真实捕获任务）**：
   - 黑屏判定按**目标区域/显示器切片**执行，不要只对虚拟桌面整帧判定：混合 DPI 双屏下
     「全屏+WDA_MONITOR」实测整帧 blackRatio 仅 **0.764851**（<99% 阈值）会漏检，而主屏区域为 1.000000；
   - WGC 窗口项对 `WDA_EXCLUDEFROMCAPTURE` 窗口无帧，需设置**短超时**（本机成功场景 WGC 首帧耗时 48.8–90.0ms，
     各 3 次均值；未做 P50/P95 压测，见遗留 4），超时即判失败并走 SA-1001，避免冻结交互被拖慢（工具中 5000ms 仅为取证留足余量）；
   - `PrintWindow(PW_RENDERFULLCONTENT)` 对两种保护窗口都返回**纯黑**，不能作为降级/兜底路径；
   - `SA-1001` 的 message 已是可操作文案；前端 overlay 现有 `error` 展示即可，无需新增 UI。
3. **建议的 0x11 增强探测（未实现，超出本任务 write scope/unsafe 约束）**：捕获前用
   `GetWindowDisplayAffinity` 枚举目标区域内的窗口，或对前台窗口做 WGC 窗口项探测（无帧即视为受保护），
   作为「黑屏检测」的补充；实现时需保持不阻塞 UI。

## 8 遗留

| # | 遗留项 | 影响 | 解锁/跟进步骤 |
| --- | --- | --- | --- |
| 1 | **真机 Netflix/受保护视频播放、锁屏、真·全屏独占（D3D exclusive）未覆盖** | 本记录用 `SetWindowDisplayAffinity` 等价构造受保护窗口，未覆盖 DRM 播放器自身实现差异 | 装 Netflix/Edge/Chrome 播放 DRM 视频、运行全屏独占游戏各跑一次；锁屏场景需在锁屏界面运行探针（计划任务/SYSTEM 服务）。工具当前只捕获自建窗口，需扩展「前台窗口/指定 hwnd/monitor」参数 |
| 2 | **整帧黑阈值漏检**：混合 DPI 双屏只有主屏黑时整帧 blackRatio 0.764851 | 若实现按整帧判定，全屏 DRM 会漏检 | 捕获实现按显示器切片/选区判定（§7.2），阈值建议保持同一口径但作用域改为区域；补真机双屏验证 |
| 3 | **0x11「窗口消失」无法靠黑屏检测发现** | 仅靠 BitBlt 会截到背后内容、静默出错 | 采用 §7.3 的 `GetWindowDisplayAffinity` 枚举/前台窗口 WGC 探测；在真实捕获任务中实现并回归本记录 17 项校验 |
| 4 | WGC 首帧延迟仅统计 4 个场景各 3 次（48.8–92.0ms），未做 P50/P95 与取消/超时策略压测 | 影响冻结时延与超时取值 | 真实捕获接入后做 100 次 P95 基准（可与 S-10 空闲资源采样错开） |
| 5 | 采集时桌面有其他协作进程窗口，受保护窗口「背后内容」非受控画面 | 仅影响「窗口消失后露出内容」的取证观感，不影响模式结论 | 如需严格对照：增加「受保护窗口 + 已知背景图案窗口」场景（工具已具备造窗口能力，可扩展） |
| 6 | 未跑 applab/mock 截图：界面证据为代码引用 | 无法提供 UI 实拍图 | 本任务约定不启动应用；如需 UI 截图，可在 applab 空闲时用浏览器 mock 的 `capture_freeze` 失败注入验证（需前端支持 mock 错误注入，当前无） |
| 7 | 4K/200% 单屏、8GB 内存机型未覆盖（沿用 S-01/S-05 环境遗留） | 结论的环境外推受限 | 后续多环境回归时按本工具复跑（工具只依赖 dotnet + Windows） |

## 9 附：本任务文件清单

新增/修改：

- `scripts/spikes/s09/S09Capture.csproj`、`scripts/spikes/s09/Program.cs`、`scripts/spikes/s09/wgc_vs_bitblt.ps1`、`scripts/spikes/s09/README.md`
- `src-tauri/src/platform/capture.rs`（+ S-09 纯函数与单测）、`src-tauri/src/services/capture_service.rs`（+ 降级决策与单测）
- `docs/spikes/S-09-截图失败降级与WGC对比.md`（本文件）、`docs/spikes/data/s09/{summary.json,env.json,README.md,shots/*.png}`
- `.verify/spikes/S-09/`（原始数据，gitignored）、`.verify/p1/rows/S-09.md`（清单行片段，由 Lead 合入）

未改：`docs/技术验证清单.md`、`docs/spikes/README.md`、`scripts/spikes/README.md`、`CHANGELOG.md`（统一由 Lead 合入）。
