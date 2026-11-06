# ADR-0007：双安装包版本（slim 安装后下载 / full 内置 OCR 模型）

| 项目 | 内容 |
| --- | --- |
| 状态 | 已接受（Accepted） |
| 日期 | 2026-11-06 |
| 关联需求 | v1.0.0 用户决策「双安装包」：slim（安装后下载 OCR 模型）与 full（内置 OCR 模型） |
| 关联文档 | [接口契约](../接口契约.md) §4.1 / §6（`ocr_status` / `Stats.edition`）/ §10；[实施架构](../实施架构.md) §4.2 / §7；[自定义指南](../自定义指南.md) §8 |
| 证据 | `.verify/plan/spec-v1.md` §0/§4；OCR 资产实测大小见 §决策；构建脚本 `scripts/build-installers.ps1`、`scripts/fetch-ocr-assets.ps1`，体积门禁与安装冒烟输出见 `.verify/verify-v1.0.0/`（T5 归档） |

## 背景

v1.0.0 把「真实本地 OCR」变成主路径：RapidOCR PP-OCRv4 的 ONNX 资产 + ONNX Runtime 是硬依赖。
按本机实测：

| 资产 | 字节数 |
| --- | --- |
| `onnxruntime.dll`（ONNX Runtime 1.31.1） | 19,288,888 |
| `ch_PP-OCRv4_det_infer.onnx` | 4,745,517 |
| `ch_PP-OCRv4_rec_infer.onnx` | 10,857,958 |
| `ch_ppocr_mobile_v2.0_cls_infer.onnx` | 585,532 |
| `ppocr_keys_v1.txt` | 26,250 |

运行时 + 模型合计约 **35.5 MB**，而沿用 v0.1 的 **NSIS 体积门禁是 ≤ 25 MiB**，单包无法同时满足
「开箱即用 OCR」与「轻量安装包」。同时存在两类用户：一类希望安装包尽可能小、能联网；
一类在内网/离线机器上，首次使用就必须有 OCR。

## 决策

1. **交付两个 NSIS 安装包**（同一份源码、两次编译）：
   - **slim**：`SnipAsk_<版本>_x64-setup.exe` —— 内置应用 + `onnxruntime.dll` + `docs/`，**不含** OCR 模型；
     首次使用 OCR 时按 `resources/models.manifest.json` 的 `role="ocr"` 条目下载 4 个模型/字典（约 16 MB）
     到 `%LOCALAPPDATA%\SnipAsk\models\ocr\`。
   - **full**：`SnipAsk_<版本>_x64-full-setup.exe` —— slim 的全部内容 + 4 个 OCR 模型/字典，离线开箱即用。
2. **体积门禁**：slim **≤ 25 MiB**（沿用 v0.1 口径），full **≤ 60 MiB**（新增）。超限即构建失败（退出码 1）。
3. **edition 编译期注入**：`scripts/build-installers.ps1` 对每个版本先 `cargo clean -p snip-ask`
   再以 `$env:SNIP_ASK_EDITION = "slim" | "full"` 编译，使 `option_env!("SNIP_ASK_EDITION")` 生效；
   未注入（`pnpm dev` / `pnpm build`）为 `"dev"`。`Stats.edition` 与设置页「自定义与关于」据此展示版本类型。
4. **资产来源与校验**：`scripts/fetch-ocr-assets.ps1` 按「本机开发缓存（`.verify/venv-ocr/**`，不入库）→
   manifest 镜像 URL → 脚本内置官方镜像」顺序准备 `src-tauri/resources/ocr/`，每个文件按内置真实
   SHA256 + 字节数强校验，不匹配立即失败；该目录整体 `.gitignore`（二进制不入库）。
5. **运行期解析顺序**（slim/full 统一）：随包 `<resource_dir>\ocr\` → 数据目录
   `%LOCALAPPDATA%\SnipAsk\models\ocr\` → 开发兜底 `runtime\` → 开发钩子 `SNIP_ASK_OCR_DIR`；
   启动时把解析到的 `onnxruntime.dll` 写入 `ORT_DYLIB_PATH`（`ort` crate `load-dynamic`），必须在创建任何 session 之前。
6. **可观测性**：新增 `ocr_status` 命令（就绪/缺少哪些资产/来源 bundled 或 installed）与 `Stats.ocrReady`，
   让「未下载模型」成为可解释状态，而不是运行中才失败。
7. **安装包随附 `docs/`**：README、自定义指南、LICENSE、NOTICE、SECURITY、PRIVACY，供应用内
   `open_path("docs")` 打开（两版一致）。

## 理由

- **体积与离线需求同时满足**：slim 守住 25 MiB 与「轻量快速」的产品定位；full 用 60 MiB 换「零联网可用」。
- **模型可增量更新**：OCR 模型走既有 manifest 下载通道（Range 断点续传 / SHA256 / 镜像回退），
  与本地 LLM 模型复用同一套机制，无需新基础设施。
- **一份代码两版产物**：差异只在「是否随包 4 个文件」与 edition 常量，Rust/前端逻辑完全一致，维护成本低。
- **失败可解释**：`ocr_status.missing` 与可操作 `message` 让用户知道缺什么、去哪下载，而不是黑盒失败。

## 备选方案与未采用原因

| 备选 | 未采用原因 |
| --- | --- |
| 只发单包（内置模型） | 超越 25 MiB 体积门禁，且多数用户并不需要离线 OCR；安装包变大影响下载与SmartScreen体验 |
| 只发 slim（全部下载） | 内网/离线机器首次使用直接不可用；ONNX Runtime 也走下载会进一步增加首次失败面 |
| 运行时也做下载 | 体积只省 19 MB，却把「应用根本不能 OCR」变成下载态；运行时随包才能保证 slim 也可用 |
| 用系统自带 Windows OCR 替代 ONNX | 识别质量/版面/语言与 PP-OCRv4 有差距（S-15 实测字符准确率 99.3%），且无法覆盖覆盖翻译的框需求 |

## 后果

- **正面**：轻量用户与离线用户各取所需；OCR 资产有明确来源/校验/状态；安装包体积可门禁、可回归。
- **代价 / 风险**：
  - 需要构建、冒烟、归档**两个产物**，发布流程与证据翻倍；
  - slim 首次 OCR 依赖网络与模型源可达性（下载失败要能重试、提示可操作）；
  - full 包更大，SmartScreen/下载耗时增加（**仍未签名**，见 [ADR-0008](0008-remove-auto-update.md) 的发布口径）；
  - OCR 资产不进仓库，复现构建需先跑 `fetch-ocr-assets.ps1`（脚本支持本机缓存与 `-NoDownload`）。
- **后续**：若未来 OCR 模型显著变小，可评估把 slim 也内置；若代码签名落地，两版都需签名并重测安装冒烟。

## 参考

- [接口契约](../接口契约.md) §4.1（`ocr.*`）、§6（`ocr_status` / `Stats.edition`）、§10（资源与安装布局）
- [实施架构](../实施架构.md) §7（数据与资源布局）、§10（构建与验证）
- [自定义指南](../自定义指南.md) §8（双版本差异、手动放置 OCR 模型、离线构建）
- `scripts/fetch-ocr-assets.ps1`、`scripts/build-installers.ps1`
