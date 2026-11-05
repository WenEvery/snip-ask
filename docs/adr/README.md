# 架构决策记录（ADR）

本目录记录「框选 AI（Snip Ask）」脚手架阶段的不可逆/高成本技术决策，采用
「状态 / 背景 / 决策 / 理由与备选 / 后果 / 参考」结构。新增决策请顺延编号，
并在本索引与 [`../../docs/实施架构.md` §11](../实施架构.md) 中同步。

| 编号 | 决策 | 状态 | 与设计文档的对应 |
| --- | --- | --- | --- |
| [ADR-0001](0001-tauri2-stack.md) | 桌面框架选 Tauri 2 + Rust + React/TS | 已接受 | 报告 §3.3；概要设计 §2.4；实施架构 §4 |
| [ADR-0002](0002-frontend-no-network-secrets.md) | 前端不接触网络与密钥，能力全部后置到 Rust | 已接受 | 契约 §1.7/§11；概要设计 §9；实施架构 §8 |
| [ADR-0003](0003-single-overlay-physical-pixels.md) | 单覆盖窗口覆盖虚拟桌面，坐标全程物理像素 | 已接受 | 契约 §1.5/§2；概要设计 §4.2；实施架构 §5 |
| [ADR-0004](0004-contract-first.md) | 契约优先：文档 + 双端 contract 同构，先定协议后写实现 | 已接受 | 契约 §16；实施架构 §1/§2 |
| [ADR-0005](0005-skeleton-mode.md) | 骨架模式：外部能力用确定性占位实现 | 已接受 | 契约 §12；实施架构 §9 |
| [ADR-0006](0006-vulkan-first-gpu-inference.md) | GPU 推理以 **Vulkan** 为主体、CUDA 为可选组件（跨 AMD/Intel/NVIDIA、依赖最小、无需额外 runtime） | 已接受 | 实施架构 §4.1/§9.3；契约 §4.1/§6；研究 §1 |

## 约定

- 状态取值：`提议（Proposed）` / `已接受（Accepted）` / `已废弃（Deprecated）` / `被取代（Superseded by ADR-xxxx）`。
- 修改已接受的 ADR 需新增一份「取代」记录，不直接改写历史决策。
- ADR 只记录**为什么**，具体实现细节见 `docs/实施架构.md` 与代码注释。
