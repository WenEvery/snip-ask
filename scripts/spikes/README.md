# scripts/spikes — P0 技术验证脚本

本目录对应 `docs/技术验证清单.md` 的 P0（Go 前提条件）spike 脚本。所有脚本可独立复跑，
原始数据写入 `.verify/spikes/<编号>/`（gitignored），结论与汇总写入 `docs/spikes/`。

## 目录

| 路径 | 用途 |
| --- | --- |
| `applab/applab.mjs` | 真实应用验证台：便携隔离启动 + WebView2 CDP 远程驱动（含 IPC 调用/截图） |
| `tools/screenshot.ps1` | DPI 感知的全虚拟桌面截图（物理像素，含负坐标区域） |
| `s01/` | S-01 混合 DPI 多屏坐标标定与误差测量 |
| `s02_s03/` | S-02/S-03 OCR 时延基准（PP-OCRv4 ONNX，CPU） |
| `s04/` | S-04 热键冲突辅助进程（RegisterHotKey 抢占） |
| `s05_s06/` | S-05/S-06 翻译与图片搜索时延基准 |
| `s07/` | S-07 首启引导离线/无 Key 走查驱动 |
| `s08/` | S-08 模型下载断点续传（Rust 测试 + 真实 Range 探测） |

## 真实应用验证台（applab）

应用是单实例的，且默认会写用户目录 `%APPDATA%\SnipAsk`；applab 用**便携模式**
（`.verify/applab/app/snip-ask.exe` + `portable.flag`）把数据隔离到
`.verify/applab/app/data/`，并通过 WebView2 的 `--remote-debugging-port=9333` 远程执行 JS。

```powershell
# 构建（改了 Rust 之后必须重建；前端改动由 Vite HMR 热更新）
cargo build --manifest-path src-tauri/Cargo.toml

# 启动（--reset 清空便携数据 = 首次启动；被他人占用时会自动等待）
node scripts/spikes/applab/applab.mjs start --owner <你的名字> --wait-sec 900 [--reset]

# 在 main 窗口里调用真实 IPC（Tauri 2 的全局注入对象）
node scripts/spikes/applab/applab.mjs eval --window main --js "window.__TAURI_INTERNALS__.invoke('config_get')"

# webview 截图（仅窗口内容；全屏取证用 tools/screenshot.ps1）
node scripts/spikes/applab/applab.mjs shot --window main --out .verify/shots/main.png

# 用完必须释放
node scripts/spikes/applab/applab.mjs stop --owner <你的名字>
```

约定：

- `start` 以状态文件 `.verify/applab/state.json` 实现独占锁（以应用进程存活判定），
  同一时刻只允许一个应用实例；`stop` 后其他成员才能启动。
- Vite dev server（`localhost:1420`）常驻复用，无需重复启动；本机 Vite 只监听 IPv6，
  探测请用 `localhost` 而不是 `127.0.0.1`。
- 应用窗口默认隐藏（`visible:false`）。需要看界面可先
  `eval --window main --js "window.__TAURI_INTERNALS__.invoke('window_show',{window:'main'})"`，
  或用 `capture_freeze` 触发 overlay。

## 资源纪律

- **应用实例独占**：见上，用 applab 的锁，不要 `taskkill` 别人的实例。
- **CPU 敏感基准独占**：跑正式时延测量前，以独占创建方式建立 `.verify/bench.lock`
  （内容：owner / pid / startedAt）；已存在且 <40 分钟且 pid 存活则等待，过期则接管；
  测完删除。测量期间不要并行编译或下载。
- 大文件/依赖请装到 `.verify/`（venv、llama.cpp、模型），不要污染仓库或全局环境。

## 相关文档

- 验收阈值与状态：`docs/技术验证清单.md`
- 逐项验证记录：`docs/spikes/`
