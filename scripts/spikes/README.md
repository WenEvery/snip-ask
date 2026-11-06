# scripts/spikes — 技术验证脚本（P0 + P1）

本目录对应 `docs/技术验证清单.md` 的 spike 脚本：P0（Go 前提条件）S-01 ~ S-08 与
P1（技术难点/验收标准）S-09 ~ S-20。所有脚本可独立复跑，原始数据写入
`.verify/spikes/<编号>/`（gitignored），结论与汇总写入 `docs/spikes/`。

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
| `s09/` | **P1** S-09 受保护内容捕获对比：BitBlt / PrintWindow / WGC × WDA_MONITOR / WDA_EXCLUDEFROMCAPTURE（.NET + WinRT） |
| `s11_s15/` | **P1** S-11 文字存在判定阈值（面积/置信度/行高/行数） + S-15 OCR 字符准确率（100 张语料 + PP-OCRv4 det/rec + CER） |
| `s12_s14/` | **P1** S-12 图片搜索可信度（真实 DeepSeek 视觉 + 官方 Search/Fetch）与 S-14 翻译一致性（云端 + 本地 llama.cpp） |
| `s13_s17_s19_s20/` | **P1** S-13 本地模型资源与降档、S-17 网络异常矩阵（真实 app IPC）、S-19 引导自动走查 + 真人走查协议、S-20 托盘/单实例/热键释放 |
| `s10_s16_s18/` | **P1** S-10 冷启动/空闲资源采样、S-16 未签名安装包与杀软、S-18 安装包体积门禁 |

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

## P1 脚本包（S-09 ~ S-20，2026-11-05）

| 脚本包 | 入口（详见各目录 README） |
| --- | --- |
| `s09/` | `powershell -File scripts/spikes/s09/wgc_vs_bitblt.ps1 -Reps 3`（需 .NET 10；产物 `.verify/spikes/S-09/raw/`） |
| `s11_s15/` | `.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py all`（语料重建 `build_corpus.py`） |
| `s12_s14/` | `python scripts/spikes/s12_s14/bench_translate_consistency.py` / `bench_image_search_trust.py`（无 Key 时先 `--mock-self-check`，MOCK 不作验收） |
| `s13_s17_s19_s20/` | `node scripts/spikes/s13_s17_s19_s20/run_s17.mjs`（mock 矩阵 + 真实 app IPC）、`run_s20.mjs`、`run_s13.mjs`、`run_s19.mjs`；`S-19-新用户走查协议.md` |
| `s10_s16_s18/` | `s18_build.ps1` + `check_bundle_size.ps1`（CI 同脚本）、`s16_scan_signature.ps1`、`s16_install_test.ps1`、`s10_coldstart.ps1` |

P1 纪律：

- **云端 Key 只放 `.verify/p1/key.txt`**（gitignored，单行）；脚本只从该文件/环境变量读取，不打印、不落盘。
  S-12/S-14 的真实云端复跑依赖用户提供 Key；无 Key 时脚本以 BLOCKED/MOCK 标注退出，不编造数据。
- **S-16 会真实安装/卸载 NSIS**（currentUser，默认 `%LOCALAPPDATA%\SnipAsk`，可卸载）；复跑前确认没有
  其它 `snip-ask` 实例（单实例插件会阻止已安装版启动）。
- **S-10 需要安静窗口**：冷启动与 30 分钟空闲采样期间禁止 cargo/pnpm build、OCR 基准、llama-server；
  用 `.verify/p1/quiet/` 标记协调。
- P1 应用验证复用 `applab`（单实例锁）与 `s04/` 的热键/托盘探测脚本；托盘原生菜单点击受 Win11
  溢出区/前台激活限制（见 S-20 记录遗留）。

## 相关文档

- 验收阈值与状态：`docs/技术验证清单.md`
- 逐项验证记录：`docs/spikes/`
