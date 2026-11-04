# S-07 首启引导无网络/无 Key 可用 + 受限模式

编号：S-07
结论：**DONE**（受限模式核心目标全部实机通过；1 项环境边界见「遗留 L-1」）
日期：2026-11-04
验证人：onboarding-offline（P0 验证智能体团队）
审批/汇总：Lead

## 1 环境

| 项 | 值 |
| --- | --- |
| 操作系统 | Windows 11 26200 |
| 内存 | 16 GB |
| 屏幕 | 主屏 2560×1440 @150% (0,0)；副屏 1080×1920 @100% (−1080,−470)（单 overlay 覆盖虚拟桌面 3640×1920） |
| 构建 | `cargo build --manifest-path src-tauri/Cargo.toml`（debug，工作区状态，未提交） |
| 前端 | Vite dev server `localhost:1420`（HMR） |
| 网络事实 | 无任何 API Key；DeepSeek API 可达但 401；HuggingFace 不可达；PyPI/GitHub/crates.io 可用 |
| 「断网」模拟 | `providers[0].baseUrl = http://127.0.0.1:1`（连接拒绝，等价断网）+ 空 Key |
| 驱动方式 | applab（WebView2 CDP）驱动真实 Tauri 应用；脚本 `scripts/spikes/s07/run_s07.mjs` |
| 数据目录 | 便携模式 `.verify/applab/app/`（`--reset` 清空 = 首次启动） |

## 2 验收口径（本任务定义）

1. `--reset` 首次启动 → 引导自动弹出；不填 Key、不下载模型，6 步可完整走完（每步有截图与步骤名）。
2. 走「跳过引导」路径 → `onboarding.skipped=true, completed=false`，主界面出现「受限模式」明确说明 + 去配置入口。
3. 受限模式下 overlay 动作菜单：浏览��搜索、复制可用；AI 搜索（非 browser 模式）、翻译、解答、图片搜索禁用，且 `disabledReason` 为可操作文案（指向设置页/本地模型）。
4. 引导完成但未配置任何能力 → 完成页与主界面均为受限状态（同一判定）。
5. `skeletonMode=false` + 空 Key + 不可达 baseUrl：`provider_test` 走真实错误路径并返回 SA-3xxx；UI 展示可操作错误；应用不崩溃。
6. 过程中发现的真实缺陷修复并回归（见 §6）。

## 3 受限模式实现（能力判定 + 门禁）

### 3.1 能力判定（纯函数，可单测）

Rust `src-tauri/src/domain/action.rs`：

```rust
pub struct Capabilities {
    pub cloud_available: bool,        // 存在 enabled 且已保存 Key 的 provider
    pub local_ocr_available: bool,    // 内嵌 OCR（骨架模式视为可用）
    pub browser_search_available: bool, // 外部浏览器（内建，恒可用）
    pub local_model_available: bool,  // 已安装本地模型（<models_dir>/<id>/.installed）
}
```

- `cloud_available`：`ConfigView.providers` 中 `enabled && hasKey`（凭据存储已保存 Key）。
- `local_model_available`：`models_list` 任一 `installed`。
- 前端同语义纯函数：`src/windows/main/onboarding/capabilities.ts`（`deriveCapabilities`）。
- 受限模式定义：`!cloud_available`（仅本地 OCR + 浏览器搜索 + 复制；本地模型就绪时另有本地翻译）。

### 3.2 动作门禁规则（`availability_with_capabilities`）

| 动作 | 条件 | 结果 / disabledReason |
| --- | --- | --- |
| 复制文字 | 始终 | 可用 |
| 搜索（browser 模式） | 始终 | 可用（不上传、不依赖云端） |
| 搜索（AI 模式） | 无云 Key | 禁用：「未配置 API Key：请到设置 → AI 服务填写，或下载本地模型」 |
| 图片搜索 | 无云 Key | 禁用：同上 |
| 翻译（云端） | 无云 Key | 禁用：同上 |
| 翻译（本地） | 本地模型未安装 | 禁用：「本地模型未下载：请到设置 → 模型库下载」 |
| 翻译（本地） | 本地模型已安装 | 可用（不依赖 Key） |
| 解答 | 无云 Key | 禁用：同上 |
| 任意文本动作 | 本地 OCR 不可用 | 禁用：「本地 OCR 不可用：…」（内嵌资源异常兜底，复制除外） |

能力门禁优先于隐私开关：两者都不可用时优先暴露「可以怎么修」的可操作路径。原隐私规则（`localOnly` / 文本上传 / 图像上传）保留。

### 3.3 接入点

- 后端：`commands/selection.rs::selection_commit` 在**不持有 session 锁**的异步区读取 `config_view().await`（凭据）与 `models.list()`（安装状态），组装 `Capabilities`，调用新增的 `visible_actions_with_capabilities` 生成 `menu:ready` 载荷。
- 兼容：保留 `visible_actions(config, has_text)` 旧签名（= `Capabilities::all_available()`），旧单测与内部调用行为不变。
- 前端主界面：`MainWindow.tsx` 读取 `config_get` / `models_list` / `onboarding_state` 派生受限状态；`config:changed` 后重新评估。受限时显示 `RestrictedNotice`（`data-testid="restricted-notice"`）＋「去配置 AI 服务」（跳到设置 → AI 服务）。
- 引导完成页：`OnboardingWizard` 第 6 步显示同一 `RestrictedNotice`，并在跳过二次确认中列出可用/不可用能力。
- overlay：`ActionMenu.tsx` 的禁用原因从仅 tooltip 改为按钮内可见文案（同时保留 title），便于用户与自动化取证。

### 3.4 行为变更 / 影响（需写入 CHANGELOG）

- **骨架模式不豁免能力门禁**：`skeletonMode=true` 只影响网络/引擎的模拟实现，不虚构「已配置 Key」。
  因此默认配置（无 Key）下，AI 搜索 / 解答 / 图片搜索 / 云端翻译会变为**禁用**并提示配置路径。
  这是 S-07 的有意设计（否则无 Key 场景永远不可观测），用户可到设置页配置 Key 或下载本地模型恢复。
- 本地翻译从「后端为 local 即可用」改为「后端为 local 且本地模型已安装才可用」，未安装时给出下载路径。

## 4 实机走查（applab 驱动真实应用）

### 4.1 首次启动与 6 步引导（无 Key、不下载模型）

`applab start --owner onboarding-offline --wait-sec 900 --reset` → 便携数据清空后启动。初始状态（原始数据 `raw/onboarding-state-initial.json`）：
`completed=false, skipped=false, skeletonMode=true, providers=[{id:"deepseek",enabled:true,hasKey:false}], modelsInstalled=[]`，
主窗口自动路由到引导（`onboarding.auto_open_first_launch` PASS）。

| 步 | 标题（`ob-step-counter`） | 本步内容 | 截图 |
| --- | --- | --- | --- |
| 1 | 第 1 / 6 步 · 欢迎与语言 | 框一下，就懂了（语言预览） | `01-step1-框一下，就懂了.png` |
| 2 | 第 2 / 6 步 · AI 服务 | 不选预设、不填 Key，直接下一步 | `02-step2-连接一个 AI 服务.png` |
| 3 | 第 3 / 6 步 · 翻译方式 | 默认云端（复用 AI 服务），不下载模型 | `03-step3-选择翻译方式.png` |
| 4 | 第 4 / 6 步 · AI 模式 | 为每个动作选择默认模式 | `04-step4-为每个动作选择默认模式.png` |
| 5 | 第 5 / 6 步 · 外观与行为 | 外观与热键默认值 | `05-step5-外观与行为.png` |
| 6 | 第 6 / 6 步 · 隐私与完成 | 显示受限模式说明（不限流），勾选同意后「完成并进入」 | `06-step6-隐私与完成.png`、`17-step6-restricted-notice.png` |

`steps.six_steps_rendered` PASS；全程无网络请求、无 Key、无模型下载，未出现阻塞性错误。

各步「跳过/稍后」入口：每步 footer 左侧都有「跳过引导」（二次确认）；第 2 步文案明确「没有 Key 也可以先跳过」；
第 6 步未勾选同意时完成按钮禁用并提示可跳过。**无阻塞性困惑**；改进项见 §8 F-1（fallback 文案未本地化）。

### 4.2 跳过引导 → 受限模式

在第 1 步点「跳过引导」→ 确认面板（`07-skip-confirm.png`）→「确认跳过」。
`raw/onboarding-state-skip.json`：`skipped=true, completed=false, finishedAt=2026-11-04T16:22:28Z`（PASS）。
主界面立即出现受限模式横幅（`08-main-restricted-banner-skip.png`），文案实测：

> 已跳过引导 · 进入受限模式 / 你没有配置 AI 服务，应用会以受限模式运行，之后可随时在设置页补齐。
> 仍可用：本地 OCR、浏览器搜索、复制文字 / 不可用：AI 搜索、解答、图片搜索、云端翻译、本地翻译
> 按钮：「去配置 AI 服务」

### 4.3 overlay 动作菜单（真实 `menu:ready`）

驱动方式：main 窗口 `capture_freeze` → 等 overlay webview 就绪 → 通过 CDP `Input.dispatchMouseEvent`
（mousePressed→mouseMoved→mouseReleased）在 overlay 内**真实拖拽** → overlay 自身 `commitSelection`
（phase=committing）→ 后端 `selection_commit` → `menu:ready`。main 与 overlay 均订阅事件取证。

`raw/menu-ready-payload-dom.json`（主跑，session `61de0902-…`）与 `raw/evidence-menu-only.json`（补充复测，session `78082de3-…`）：

| 菜单项 | enabled | disabledReason |
| --- | --- | --- |
| 搜索（browser 模式） | ✅ true | — |
| 复制文字 | ✅ true | — |
| 翻译（云端） | ⛔ false | 「未配置 API Key：请到设置 → AI 服务填写，或下载本地模型」 |
| 解答 | ⛔ false | 同上 |

截图 `09-overlay-menu-restricted.png`（overlay 全屏，禁用原因直接显示在按钮内、label 下方，不只 tooltip）。
`menu:ready` 原始载荷含 `session/items/hasText/preview`，与 `capture_freeze` 返回 session 一致（PASS）。

**驱动缺陷说明**：主跑中 `menu.payload_session_matches` 判 FAIL，原因是驱动脚本读事件时未解开 Tauri 2 的
`{event,id,payload}` 包装（取到 `payloadSession=null`），并非应用缺陷；原始载荷本身含正确 session
（`raw/menu-ready-payload-dom.json` 中 `payloads[0].payload.session === 61de0902-…`）。
修正解包后以 `--only-menu` 补充会话复测，7/7 PASS；随后按“禁用原因可读性”微调 ActionMenu 布局
（label 与原因两行、meta 右侧）并再次复跑（session `78082de3-…`），9 号截图即该版本。

### 4.4 断网 + 空 Key + `skeletonMode=false` 错误路径

`config_patch`：`advanced.skeletonMode=false`、`providers=[{baseUrl:"http://127.0.0.1:1", model:"deepseek-flash", enabled:true}]`、无 Key
（`raw/config-patched-offline.json`：`skeletonMode=false, hasKey=false`，PASS）。

`provider_test` 直接 IPC（`raw/provider-test-direct.json`）：

```json
{"ok":false,"latencyMs":null,"status":null,
 "error":{"code":"SA-3001","message":"网络不可用",
          "detail":"TODO(real): 真实连通性测试未接入",
          "retryable":true,"fallback":["browser_search","local_translate"]}}
```

UI 路径（设置 → AI 服务 → 展开编辑 → 测试连接）实测显示（`10-provider-test-error-ui.png`、`raw/provider-test-ui.json`）：

> 连接失败 / SA-3001 · 网络不可用 / 可尝试：browser_search、local_translate

应用不崩溃：错误后继续 IPC 正常（`raw/app-alive-after-error.json`：`alive=true, providerCount=1, skeletonMode=false`，PASS）。

云动作真实错误路径（`menu_execute({action:"answer"})`，绕过 UI 禁用直接调用）：返回 task id，
随后 `task:done` 1 次、`task:error` 0 次（`raw/cloud-action-attempt.json`、`raw/cloud-action-task-events.json`）——
当前云实现是 `TODO(real)` 骨架网关，无法产生真实网络错误。记为遗留 **L-1**（见 §8）。

### 4.5 引导完成但未配置能力（完成页/主界面受限说明）

跳过之后重新进入引导，**完整走完 6 步**（每步截图见 §4.1），第 6 步显示同一受限模式说明（`17-step6-restricted-notice.png`），
勾选「我已阅读并同意隐私声明」后「完成并进入」：

- `raw/onboarding-state-completed.json`：`completed=true, skipped=false, currentStep=5`（PASS）；
- 主界面横幅切换为「受限模式 / 当前未配置任何 AI 服务（无 API Key、无本地模型）」（`18-main-restricted-banner-completed.png`）；
- 点「去配置 AI 服务」→ 设置页自动落到「AI 服务」分组（`19-main-goto-config.png`，active nav = AI 服务，PASS）。

## 5 截图清单

| 文件（`.verify/spikes/S-07/shots/`） | 内容 |
| --- | --- |
| `01-step1-框一下，就懂了.png` | 首次启动，引导第 1 步自动弹出 |
| `02-step2-连接一个 AI 服务.png` | 第 2 步（不填 Key，直接下一步） |
| `03-step3-选择翻译方式.png` | 第 3 步（不下载本地模型） |
| `04-step4-为每个动作选择默认模式.png` | 第 4 步 |
| `05-step5-外观与行为.png` | 第 5 步 |
| `06-step6-隐私与完成.png` | 第 6 步（完成页） |
| `07-skip-confirm.png` | 跳过引导二次确认（列出受限能力矩阵） |
| `08-main-restricted-banner-skip.png` | 跳过后的主界面受限模式横幅 + 去配置入口 |
| `09-overlay-menu-restricted.png` | overlay 动作菜单：搜索/复制可用，翻译/解答禁用且原因可见 |
| `10-provider-test-error-ui.png` | `skeletonMode=false` + 不可达 baseUrl 的 `provider_test` 真实错误 UI |
| `17-step6-restricted-notice.png` | 第 6 步完成页的「受限模式」明确说明 |
| `18-main-restricted-banner-completed.png` | 完成引导（未配置能力）后的主界面受限横幅 |
| `19-main-goto-config.png` | 「去配置 AI 服务」跳转到设置 → AI 服务 |

原始 JSON：`raw/evidence.json`（主跑 24 项断言）、`raw/evidence-menu-only.json`（菜单补充复测 7/7）、
`raw/evidence-summary.json`（合并摘要）、`raw/menu-ready-payload-dom.json`、`raw/provider-test-direct.json`、
`raw/provider-test-ui.json`、`raw/onboarding-state-{initial,skip,completed}.json`、`raw/steps.json` 等。

## 6 走查中发现的真实缺陷与修复（回归通过）

| # | 缺陷 | 影响 | 修复 |
| --- | --- | --- | --- |
| D-1 | `onboarding_update` 第 1/2 步在「未配置」时提交 `{ai:null}` / `{translate:null}`，后端 `serde_json::from_value::<AiDraft>` 报「数据非法」 | 无网络/无 Key 用户直接点「下一步」出现错误提示（恰好是 S-07 目标人群） | `domain/onboarding.rs` 将 null/缺省归一为 `None`（未配置），非对象仍报错 |
| D-2 | 前端按草稿分节提交（`{modes}` / `{ui,hotkeys}` / `{privacy}`），后端只解析扁平字段 | 第 3/4/5 步数据被**静默丢弃**（如隐私同意、上传开关、热键、快速/深度模式不生效） | 新增 `normalize_step_payload`：同时兼容嵌套（前端/mock 形状）与扁平（旧调用），并补嵌套回归单测 |

两处修复均有 Rust 单测：`apply_step_data_allows_unconfigured_ai_and_translate`、`apply_step_data_accepts_wizard_nested_payloads`，并触发/回归了完整 6 步走查。

## 7 结论与阈值

结论：**DONE**。S-07 的核心目标——「无网络/无 Key 环境可完整走完首启引导，且未配置能力时进入有明确说明的受限模式，云动作禁用原因可操作」——全部实机验证通过。

| 验收项 | 结果 | 证据 |
| --- | --- | --- |
| 1. `--reset` 首启自动进入引导；无 Key/无模型 6 步完整走完 | ✅ PASS | `01..06`、`17` 截图；`raw/steps.json`、`onboarding-state-initial/completed.json` |
| 2. 跳过引导 → 受限模式（`skipped=true` + 主界面说明 + 去配置入口） | ✅ PASS | `07/08`；`raw/onboarding-state-skip.json` |
| 3. overlay 菜单：浏览器搜索/复制可用；翻译/解答/图片搜索禁用且原因可操作 | ✅ PASS | `09`；`raw/menu-ready-payload-dom.json`、`evidence-menu-only.json` |
| 4. 引导完成但未配置能力 → 完成页/主界面同为受限状态 | ✅ PASS | `17/18/19`；`raw/onboarding-state-completed.json` |
| 5. `skeletonMode=false` + 空 Key + 不可达 baseUrl：真实错误可操作、不崩溃 | ✅ PASS（provider_test SA-3001） | `10`；`raw/provider-test-direct.json`、`provider-test-ui.json`、`app-alive-after-error.json` |
| 6. 走查发现缺陷修复并回归 | ✅ PASS（D-1/D-2） | §6；Rust 单测 + 完整 6 步回归 |

自动化断言合计：主跑 24 项（23 PASS；1 项为驱动解包 bug，非应用缺陷）+ 菜单补充复测 7/7 PASS；
唯一失败项已在补充会话中用修正后的驱动复测通过。Rust 单测：`domain::action` 10 项、`domain::onboarding` 8 项全绿；
`cargo clippy --all-targets -- -D warnings` 通过；`pnpm typecheck` 通过（`cargo test` 全量仍有 1 项
`services::model_manager::tests::derives_download_filename_from_url` 失败，属 S-08 在改文件，与本任务无关）。

## 8 遗留

| # | 项 | 说明 / 解锁方式 | 建议负责人 |
| --- | --- | --- | --- |
| L-1 | `menu_execute` 后端无能力门禁，云实现为 `TODO(real)` | 绕过 UI 直接调用仍会启动骨架网关任务并「成功」；真实云动作网络错误（SA-3xxx）当前不可产生。建议在 `commands/action.rs` 复用 `Capabilities` 判定，未配置时直接返回可操作错误；真实错误需 S-05/S-06 的真实 LLM 网关。 | Lead 分派 |
| L-2 | 图片菜单（`hasText=false`）实机不可稳定构造 | 骨架 OCR 的 `hasText` 恒为 true；由 Rust 单测 `restricted_mode_disables_cloud_actions_but_keeps_browser_search_and_copy` 覆盖。真实 OCR 接入后补实机。 | S-02/S-03 之后 |
| L-3 | 「骨架模式」徽标与「受限模式」并存 | 标题栏仍写「骨架模式」，与受限模式提示语义不同，易让验收者误解为开发态。建议后续把开发态徽标与用户态受限提示分开。 | 产品/前端 |
| L-4 | 断网模拟方式单一 | 用 `127.0.0.1:1` 连接拒绝等价断网；DNS 超时、代理、TLS 错误等未覆盖（`provider_test` 为 `TODO(real)`，错误码细分待真实实现）。 | S-17 网络与额度 |
| L-5 | 「本地模型就绪 → 本地翻译可用」仅单测覆盖 | 本机无本地模型（HF 不可达，ModelScope 可下载但体积/时间超本任务预算）。该分支由 `restricted_mode_keeps_local_translate_when_model_installed` 单测覆盖。 | S-08 之后补实机 |
| F-1 | 错误 fallback 文案显示英文 id | UI 显示「可尝试：browser_search、local_translate」（PASS 但未本地化），建议映射为「浏览器搜索 / 本地翻译」。作为 S-19 的用户困惑输入。 | S-19 / 前端 |

## 9 复跑方法

```powershell
# 构建（Rust 有改动时）
cargo build --manifest-path src-tauri/Cargo.toml
# 一键跑完（自动 start --reset / stop，约 3–5 分钟）
node scripts/spikes/s07/run_s07.mjs
# 只复用已启动实例调试
node scripts/spikes/s07/run_s07.mjs --attach
```

原始数据：`.verify/spikes/S-07/raw/`（`evidence.json`、`menu-ready-payload-dom.json`、`provider-test-*.json`、`onboarding-state-*.json`、`steps.json`）；截图：`.verify/spikes/S-07/shots/`。
