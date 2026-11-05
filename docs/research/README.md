# docs/research — 追加需求可行性研究证据包（2026-11-05）

本目录是 [`docs/追加需求可行性研究.md`](../追加需求可行性研究.md) 的配套证据（可随仓库提供的小体积副本）；
完整原始数据（逐次请求 JSON、nvidia-smi 500ms 采样、OCR 逐图 JSON、原型生成脚本、复现器）在 `.verify/research/`（gitignored）。

| 文件 | 内容 |
| --- | --- |
| [`GPU本地推理-证据报告.md`](GPU本地推理-证据报告.md) | 需求一完整证据：17 组真实基准（CUDA/Vulkan/CPU × 0.5B/1.5B/1.8B）、VRAM/功耗/ngl 扫描、S-05 阈值影响、最小改动清单（file:line）、人日与局限 |
| [`覆盖翻译-证据报告.md`](覆盖翻译-证据报告.md) | 需求二完整证据：100 张真实语料 OCR（2288 行）版面量化、6 组 before/after 原型、方案 A/B/C 对比、工程落点与人日 |
| [`连续提问-证据报告.md`](连续提问-证据报告.md) | 需求三完整证据：现状核实（file:line）、F1 致命缺陷推导链、MVP/完整版设计、迁移草案、人日与风险 |
| [`data/gpu-summary.json`](data/gpu-summary.json) | GPU 基准汇总（P50/P95/tok/s/VRAM/温耗/加速比，供机器复核） |
| [`data/overlay-proto-summary.json`](data/overlay-proto-summary.json) | 覆盖翻译逐图/逐块版面统计（100 张语料） |
| [`data/followup-dynamic-probe.json`](data/followup-dynamic-probe.json) | Lead 用真实应用 IPC 动态复现「追问必然 SA-1099」的证据 |
| [`images/overlay-notepad-compare.jpg`](images/overlay-notepad-compare.jpg) | 真实 Win32 记事本窗口的覆盖翻译 before/after 对照 |
| [`images/overlay-web-compare.jpg`](images/overlay-web-compare.jpg) | 网页截图的覆盖翻译 before/after 对照 |

**口径与局限**（三份证据报告均逐条列出，此处摘要）：

- 环境：Windows 11 26200 / i7-12650H / 15.78 GiB / RTX 4060 Laptop 8 GB（驱动 595.97；CUDA12 runtime + Vulkan）；
  桌面空载已占约 2.2 GB 显存与部分 CPU（Wallpaper Engine/Edge/DSH 等），绝对数值偏保守；同机 CPU/GPU 对照可信。
- GPU：非流式基准，TTFT 以 `prefill_ms` 作下界（预计流式首字 <0.2 s）；CUDA（2.67.0）与 Vulkan/CPU（2.28.2）版本不同，
  不是严格同版本 A/B；未在真实游戏抢占显存、8 GB 内存机、Win10、AMD/Intel 显卡上复测。
- 覆盖翻译：原型译文来自本地 0.5B（质量一般，仅验证版式）；语言对为中→英；滚动/视频跟随未实现（方案明确不推荐实时模式）。
- 连续提问：F1 以「逐行代码路径 + 同构复现 + 真实 IPC 动态复现」三重证明；真实 LLM 网关仍 `TODO(real)`，多轮质量待网关接入后验收。
- 执行：AI 助手团队（3 条并行研究线 + Lead 集成），非真人产品评审；所有 mock 均明确标注且不作为验收依据。
