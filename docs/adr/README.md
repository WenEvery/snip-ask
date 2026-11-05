# 架构决策记录（ADR）

本目录记录「框选 AI（Snip Ask）」的不可逆/高成本技术决策，采用
「状态 / 背景 / 决策 / 理由与备选 / 后果 / 参考」结构。新增决策请顺延编号，
并在本索引与 [`../../docs/实施架构.md` §11](../实施架构.md) 中同步。

| 编号 | 决策 | 状态 | 与设计文档的对应 |
| --- | --- | --- | --- |
| [ADR-0001](0001-tauri2-stack.md) | 桌面框架选 Tauri 2 + Rust + React/TS | 已接受 | 报告 §3.3；概要设计 §2.4；实施架构 §4 |
| [ADR-0002](0002-frontend-no-network-secrets.md) | 前端不接触网络与密钥，能力全部后置到 Rust | 已接受 | 契约 §1.7/§11；概要设计 §9；实施架构 §8 |
| [ADR-0003](0003-single-overlay-physical-pixels.md) | 单覆盖窗口覆盖虚拟桌面，坐标全程物理像素 | 已接受 | 契约 §1.5/§2；概要设计 §4.2；实施架构 §5 |
| [ADR-0004](0004-contract-first.md) | 契约优先：文档 + 双端 contract 同构，先定协议后写实现 | 已接受 | 契约 §16；实施架构 §1/§2 |
| [ADR-0005](0005-skeleton-mode.md) | 骨架模式：外部能力用确定性占位实现 | 已接受（v1.0.1 起为开发者配置、不在用户 UI 出现，默认关闭） | 契约 §12；实施架构 §9 |
| [ADR-0006](0006-vulkan-first-gpu-inference.md) | GPU 推理以 **Vulkan** 为主体、CUDA 为可选组件（跨 AMD/Intel/NVIDIA、依赖最小、无需额外 runtime） | 已接受 | 实施架构 §4.1/§9.3；契约 §4.1/§6；研究 §1 |
| [ADR-0007](0007-dual-installer-editions.md) | **双安装包**：slim（内置 ONNX Runtime，首次下载 OCR 模型 ≤25 MiB）/ full（内置全部 OCR 模型 ≤60 MiB），edition 编译期注入 | 已接受 | 契约 §6（`ocr_status` / `Stats.edition`）/§10；实施架构 §7；自定义指南 §8 |
| [ADR-0008](0008-remove-auto-update.md) | **彻底移除自动更新**：删 updater 插件与 `advanced.updateChannel`（schema v5）/`update:status` 事件；升级 = 覆盖安装或源码构建 | 已接受 | 契约 §4.1/§7/§16；自定义指南 §1；CHANGELOG 1.0.0 |
| [ADR-0009](0009-real-capture-ocr.md) | **真实捕获与本地 OCR**：Win32 BitBlt 为主捕获路径（WGC 不作主路径）+ RapidOCR PP-OCRv4/ONNX Runtime（`ort` load-dynamic、双约束像素预算、整屏 det 缓存 + 选区 rec-only） | 已接受 | 契约 §6（命令 1–5/40）/§10/§12；实施架构 §4.2/§5/§9；技术验证清单 S-02/S-03/S-09/S-11/S-15 |

## 约定

- 状态取值：`提议（Proposed）` / `已接受（Accepted）` / `已废弃（Deprecated）` / `被取代（Superseded by ADR-xxxx）`。
- 修改已接受的 ADR 需新增一份「取代」记录，不直接改写历史决策。
- ADR 只记录**为什么**，具体实现细节见 `docs/实施架构.md` 与代码注释。
- 历史设计文档（可行性研究报告、概要设计说明书）保留立项时的方案；被 ADR-0008 等取代的部分以 ADR 与
  [CHANGELOG](../../CHANGELOG.md) 为准。
