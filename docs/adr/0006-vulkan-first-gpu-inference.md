# ADR-0006：GPU 推理以 Vulkan 为主体、CUDA 为可选组件

| 项目 | 内容 |
| --- | --- |
| 状态 | 已接受（Accepted） |
| 日期 | 2026-11-05 |
| 关联需求 | 追加需求一「本地模型算力走 GPU，设置可选 CPU/GPU」 |
| 关联文档 | [追加需求可行性研究](../追加需求可行性研究.md) §1；[接口契约](../接口契约.md) §4.1 / §6；[实施架构](../实施架构.md) §4.1 / §9.3 |
| 证据 | `docs/research/GPU本地推理-证据报告.md`、`.verify/research/gpu/`（gitignored）、`.verify/evidence/`（本批真机：`Vulkan0` 探测 / `-ngl 99 --device Vulkan0` 真实推理 90 ms vs CPU `-ngl 0` 482 ms / `/health` 2188 ms / 停止后无残留；**AMD 未测**） |

## 背景

产品需要在「本地翻译 / 本地问答」时把 llama.cpp 推理放到 GPU，并在设置里提供「计算设备：自动 / CPU / GPU」。
Windows 上 llama.cpp 的 GPU 后端有三条常见路径：

1. **Vulkan**：跨厂商（AMD Radeon / Intel Arc / NVIDIA 全支持），随 llama.cpp 构建分发，不依赖额外 runtime；
2. **CUDA**：仅 NVIDIA，性能通常略优，但需要 NVIDIA 驱动 + CUDA runtime（组件 + runtime 约 0.9 GB）；
3. **CPU**：永远可用，但 1.8B Q4 在本机实测 P50 11.76s / P95 21.84s（研究 §1.2），不满足 8s 门槛。

主安装包有 **≤25 MiB** 体积门禁（S-18），不可能把 GPU 后端全部塞进包内。用户 GPU 分布中最广的是
NVIDIA 独显，但 AMD / Intel 核显与独显同样普遍；若默认只支持 CUDA，AMD/Intel 用户会被直接排除在
「GPU 加速」之外——这正是本需求的核心约束。

## 决策

1. **默认 GPU 后端为 Vulkan**：`local.gpuBackend = "auto"` 时解析顺序固定为
   **Vulkan → CUDA → CPU**；`vulkan` → Vulkan → CPU（不回退 CUDA）；`cuda` → CUDA → Vulkan → CPU；
   `local.device = "cpu"` → 只用 CPU。
2. **CUDA 作为 NVIDIA 用户的可选组件**：仅当用户显式选择 `cuda`，或 `auto` 下 Vulkan 不可用且检测到
   NVIDIA 设备/已安装 CUDA 组件时才尝试。
3. **分发策略**：主安装包只带 **CPU 后端**；**Vulkan 后端作为默认 GPU 组件按需下载**；
   CUDA 组件（含 runtime）作为可选组件按需下载。全部走既有 Range 断点续传 + SHA256 + 镜像机制。
   体积口径：研究期（2026-11，llama.cpp 2.28 构建）估算 Vulkan ≈55.8 MB、CUDA + runtime ≈0.9 GB；
   本批实际清单改用官方 `b12128` 构建（`resources/backends.manifest.json` 为唯一事实来源）：
   **Vulkan 37,684,327 B**、CUDA 288,918,265 B + cudart 391,443,627 B、CPU 24,730,477 B，
   且 Vulkan 组件已在真机完成实下 + SHA256 校验 + 解压 + `--list-devices` 复测。
4. **设备判定只认 llama.cpp 自身输出**（`llama-server --list-devices`）：不得把 `nvidia-smi` 作为
   「是否有可用 GPU」的前置判据；`nvidia-smi` 只作为补充展示（可选、失败静默）。
5. **首次运行保守回退**：默认 `device = "auto"` 在无后端/无可用设备/显存不足时自动回退 CPU，
   `engine:status.fallbackReason` 说明原因，不因 GPU 缺失导致启动失败。
6. **显式传 `-ngl`**：CPU 用 `-ngl 0`，GPU 用 `local.gpuLayers`（默认 99），避免不同构建的默认语义差异
   （LM Studio 构建省略默认全量 offload，官方构建省略 = 0 层）。

## 理由

- **覆盖 AMD / Intel / NVIDIA**：Vulkan 是唯一同时覆盖三家的 llama.cpp GPU 后端；实测本机 RTX 4060 Laptop
  的 Vulkan 与 CUDA 性能接近（1.8B 产品提示词：Vulkan P50 1 264 / P95 4 081 ms，CUDA P50 1 256 / P95 3 747 ms，
  Vulkan 略慢 2–10%），但 Vulkan 的兼容面大得多，作为默认的收益/代价比最高。
- **依赖最小、分发简单**：Vulkan 组件本批清单为 37,684,327 B（研究期估算 ≈55.8 MB），无需额外 runtime DLL；
  CUDA 组件 + runtime 合计约 0.68 GB（清单实测），且需遵守 NVIDIA CUDA EULA。把 CUDA 作为默认会让主包/下载
  体积与合规成本显著上升，却只服务一类用户。
- **无需额外 runtime 安装**：Windows 10/11 的显卡驱动通常自带 Vulkan 支持，用户无需单独安装 CUDA Toolkit；
  缺驱动时可靠回退 CPU。
- **探测逻辑中立**：以 `--list-devices` 为唯一可用判据，避免探测层写死 NVIDIA 假设；CUDA 缺失 DLL 的
  经典 `0xC0000135` 问题（研究 §1.2）只影响可选 CUDA 路径。
- **成本可控**：Vulkan 与 CUDA 共用同一套 `EngineAttempt` / 回退链 / 状态上报，多一条后端只多一个组件清单项。

## 备选方案与未采用原因

| 备选 | 未采用原因 |
| --- | --- |
| **CUDA 作为默认 GPU 后端** | 只覆盖 NVIDIA，直接排除 AMD/Intel；组件 + runtime 约 0.68 GB（清单实测；研究期估算 ≈0.9 GB），与 25 MiB 主包门禁及按需下载原则冲突；需接受 NVIDIA EULA 再分发约束 |
| **DirectML / ONNX Runtime GPU** | llama.cpp 的 Windows 分发不提供 DirectML 后端；引入第二套推理栈会显著增加集成与包体成本，且与现有 llama.cpp 引擎/日志/参数体系不兼容 |
| **SYCL（Intel oneAPI）** | 只覆盖 Intel，且需要额外 runtime；收益面小于 Vulkan |
| **只做 CPU、不做 GPU** | 研究实测 1.8B CPU P95 21.84s，不满足 8s 门槛；0.5B 虽达标但质量有限，丢失本需求最大收益 |
| **把 GPU 后端全部塞进主安装包** | 违反 S-18 体积门禁（≤25 MiB），且多数用户用不到全部后端 |

## 后果

- **正面**：AMD/Intel/NVIDIA 用户都能通过同一条 Vulkan 路径获得 GPU 加速；主包保持轻量；CUDA 用户可按需获得
  略优性能；GPU 不可用时可靠回退 CPU。
- **代价 / 风险**：
  - Vulkan 驱动质量参差（老驱动/虚拟机可能不可用），需要 `useable=false` + 可操作原因的正确降级；
  - 两个后端（Vulkan/CUDA）需要各自维护组件清单、版本、DLL 布局与体积；
  - GPU 推理会唤醒独显、增加耗电/发热，必须保留 CPU 选项（省电/续航策略）；
  - **AMD 真机未纳入本批实测**（开发机为 NVIDIA RTX 4060 Laptop）。本决策的 AMD 可用性依据是
    「Vulkan 路径无 NVIDIA 专属依赖」的代码审查（不以 `nvidia-smi` 为判据、不链接 CUDA DLL、设备由
    llama.cpp 自带 Vulkan 后端枚举），**不等价于 AMD 实机验证**；AMD 实机列入遗留验证项。
- **后续**：若 Vulkan 在某些 AMD/Intel 驱动上出现稳定性问题，优先修探测/回退与文档，而不是把 CUDA 提为默认；
  如需更强覆盖可评估再引入其他后端，但必须保持「厂商无关优先」原则。

## 参考

- [追加需求可行性研究](../追加需求可行性研究.md) §1.2–§1.4（实测数字、显存、踩坑、分发体积）
- [GPU 本地推理证据报告](../research/GPU本地推理-证据报告.md)
- [接口契约](../接口契约.md) §4.1（`device` / `gpuBackend` / `gpuLayers`）、§6（命令 30–37）、§7（`engine:status`）
- [实施架构](../实施架构.md) §4.1（本地 GPU 模块）、§9.3（追加需求实现边界）
- [开发指南](../开发指南.md) §8（GPU / Vulkan 后端与验证钩子）
