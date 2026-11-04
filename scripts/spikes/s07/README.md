# scripts/spikes/s07 — 首启引导离线 / 无 Key 与受限模式走查

对应任务：**S-07 首启引导无网络/无 Key 可用 + 受限模式（P0）**。
结论与逐项证据见 [`docs/spikes/S-07-首启引导无网络无Key与受限模式.md`](../../../docs/spikes/S-07-首启引导无网络无Key与受限模式.md)。

## 前置条件

1. Rust 改动已构建（`src-tauri/target/debug/snip-ask.exe` 含 S-07 能力门禁）：
   `cargo build --manifest-path src-tauri/Cargo.toml`
2. Vite dev server 常驻 `localhost:1420`（applab 会复用，必要时自动拉起）。
3. 应用全机单实例；脚本通过 applab 的 `.verify/applab/state.json` 锁独占，被他人占用时
   `start --wait-sec 900` 会轮询等待（不要 `taskkill` 别人的实例）。

## 用法

```powershell
# 完整跑一遍：会自动 start --reset（= 首次启动）并在结束时 stop
node scripts/spikes/s07/run_s07.mjs

# 只跑 overlay 动作菜单取证（不 reset；用于改完 ActionMenu/门禁后的快速回归）
node scripts/spikes/s07/run_s07.mjs --only-menu

# 调试：复用已启动实例（不 start/stop，不动已有状态）
node scripts/spikes/s07/run_s07.mjs --attach

# 保留实例继续人工查看
node scripts/spikes/s07/run_s07.mjs --keep
```

菜单取证原理：`capture_freeze` 后 overlay webview 才动态创建；脚本通过 CDP 连接 `window=overlay` 的
target，用 `Input.dispatchMouseEvent`（mousePressed→mouseMoved→mouseReleased）在 overlay 内**真实拖拽**，
让 overlay 自己的 `SelectionLayer` 走 commit（phase=committing），后端 `menu:ready` 到达后渲染
`ActionMenu`；随后用 `Runtime.evaluate` 读按钮 `disabled` / `disabledReason` 并截图。
（注意：从 main 直接调 `selection_commit` 不会切到菜单相，必须由 overlay 自己 commit。）

脚本全程只用 applab 的 CDP 通道驱动 **真实应用**：

- `eval --window main/overlay` 执行 JS（点击真实按钮 / 调用真实 IPC）；
- `shot --window main/overlay` 截图；
- 断言值全部来自真实 IPC 返回值（`config_get` / `onboarding_state` / `provider_test`）
  或真实渲染 DOM，不写死、不 mock。

## 覆盖的流程

| 阶段 | 操作 | 证据 |
| --- | --- | --- |
| a | `--reset` 首次启动 → 引导自动弹出；不填 Key 走完 6 步（每步截图/步骤名） | `shots/01..06-*.png`、`raw/steps.json` |
| b | 跳过引导（跳过 → 二次确认）→ 主界面受限模式提示 | `shots/07-skip-confirm.png`、`08-main-restricted-banner-skip.png`、`raw/onboarding-state-skip.json` |
| b | overlay 动作菜单：搜索/复制可用，翻译/解答禁用且原因可操作 | `shots/09-overlay-menu-restricted.png`、`raw/menu-ready-payload-dom.json` |
| c | `skeletonMode=false` + `baseUrl=http://127.0.0.1:1` + 空 Key：`provider_test` 真实错误路径（SA-3xxx）与 UI 错误展示、不崩溃 | `shots/10-provider-test-error-ui.png`、`raw/provider-test-direct.json`、`raw/provider-test-ui.json`、`raw/app-alive-after-error.json` |
| a/b | 引导完成但无任何能力 → 完成页 / 主界面受限说明 + 「去配置」入口 | `shots/17..19-*.png`、`raw/onboarding-state-completed.json` |

## 输出（gitignored）

- `.verify/spikes/S-07/shots/`：实机截图；
- `.verify/spikes/S-07/raw/`：IPC 返回、`menu:ready` 载荷、断言汇总 `evidence.json`。

## 已知边界

- 真实 LLM/搜索网络实现为 `TODO(real)`（骨架网关），因此「云动作真实网络错误」无法在
  当前代码库产生；`provider_test` 是唯一可实测的真实网络错误路径。详见 S-07 文档「遗留 L-1」。
- 图片搜索菜单（`hasText=false`）在骨架 OCR 下无法从 UI 稳定构造，由 Rust 单测
  `restricted_mode_disables_cloud_actions_but_keeps_browser_search_and_copy` 覆盖。
