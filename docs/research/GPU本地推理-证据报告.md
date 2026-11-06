# 本地模型 GPU 推理可行性 + 设置可选 CPU/GPU（task-7 研究报告）

> 执行者：**gpu-infer**（共享任务 `task-7`）｜日期：2026-11-05（实测窗口 21:20–22:15 Asia/Shanghai）
> 本目录：`.verify/research/gpu/`（只写本目录，未改任何仓库跟踪文件、未 git add/commit、未启动 applab、未碰密钥）
> 被测机器：Windows 11 / i7-12650H / 15.78 GiB / **RTX 4060 Laptop 8188 MiB**（本机实测）
> 判定问题：**本地模型算力能否走 GPU？设置里能否提供 CPU/GPU 选择？** 结论：**能，且收益很大（1.8B 200 字 P95 从 10.1s → 3.75s）；但需把「GPU 探测 + 显存预算 + 失败回退 + 可选组件分发」做出来，默认建议 `auto`。**

---

## 0 结论摘要（TL;DR）

| 问题 | 实测结论 |
| --- | --- |
| GPU 是否可用 | **可用且稳定**。CUDA12（llama.cpp 2.67.0）与 Vulkan（2.28.2）两套后端在 RTX 4060 Laptop 8GB 上均跑通；17 组基准 **0 失败**、每组结束 `llama-server` 残留 = 0 |
| 1.8B（产品质量档）能否 ≤8s | **能**。CPU 实测 P50 9.77s / P95 10.14s（S-05 旧数据 P95 21.84s）；**CUDA GPU P50 1.26s / P95 3.75s / max 3.99s**（20 次）→ **P50 加速 7.8×、P95 2.7×（去掉长尾 7.9×）** |
| 极小档 0.5B / 1.5B | 0.5B 产品提示词：3.99s→**0.58s**（P50 6.9×）；0.5B 精简提示词：3.09s→**0.69s**；1.5B 精简提示词：6.44s→**1.09s**（5.9×） |
| tok/s | 1.8B：CPU 20.6 → **CUDA 125.7 / Vulkan 129.7 tok/s**（6.1×）；0.5B：51.7 → **227.3**；1.5B：25.7 → **135.4** |
| prefill（首字近似） | 1.8B：CPU 2,462ms → **CUDA 58ms / Vulkan 82ms**（约 42×）；流式首字预计 **<0.2s**（本基准为非流式，见 §4.5） |
| 显存 | 空载桌面已占 **2,201 / 8,188 MiB（26.9%）**；模型加载增量：0.5B **≈0.57GB**、1.5B **≈1.23GB**、1.8B **≈1.50GB**；1.8B 跑满时总占用峰值 **3,712 MiB（45.3%）**，仍有 **≈4.4GB** 余量 → **适配 8GB** |
| 温耗 | 1.8B 满载：GPU 利用率 82–84%、温度峰值 61–66°C、整卡功耗峰值 76–85W（空载 44°C / 9.8W）；笔记本需关注续航与风扇，**建议插电策略/温控提示** |
| 推荐后端 | **双后端**：默认组件用 **Vulkan（新增体积 ≈56MB，跨 NVIDIA/AMD/Intel）**，NVIDIA 用户可选 **CUDA 组件（新增 ≈0.9GB）**；本机两者总耗时几乎相同（1.8B P50 1.256s vs 1.264s），CUDA 在 prefill/加载略优 |
| 默认值 | **`local.device = auto`**（有可用 GPU 且显存预算够→GPU；否则 CPU；启动/加载失败自动回退 CPU 并提示），而非硬编码 GPU |
| 工作量 | GPU 增量约 **14.5 人日**（配置 1.5 / 探测与预算 2.5 / 引擎与回退 4.0 / 前端 1.5 / 打包分发 2.5 / 测试 2.0 / 文档 0.5）；MVP（手动开关 + 手动放置后端 + 设置项）约 **4.5 人日** |

---

## 1 环境（本机实测）

| 项 | 值 | 来源 |
| --- | --- | --- |
| OS | Microsoft Windows 11 Pro 10.0.26200 | `raw/env.json` |
| CPU | 12th Gen Intel Core i7-12650H（10C16T） | `raw/env.json` |
| RAM | 15.78 GiB | `raw/env.json` |
| GPU | **NVIDIA GeForce RTX 4060 Laptop GPU，8188 MiB，驱动 595.97，compute cap 8.9**（`nvidia-smi` 报 `CUDA Version: 13.2`） | `raw/env.json`、`logs/` |
| 虚拟显示适配器 | **MuMu Virtual Display Adapter**、**GameViewer Virtual Display Adapter**（DXGI 层存在 2 个虚拟适配器，GPU 探测必须过滤） | `raw/env.json` `display_adapters` |
| 桌面空载显存 | **2,201 MiB / 8,188 MiB（26.9%）**，util 29%、44°C、9.78W、P5（util 29% 来自 Wallpaper Engine 动画等桌面常驻，不是 0 负载） | `raw/env.json`、`raw/gpu_*.csv` |
| 后端（LM Studio 安装，本机既有） | CPU：`llama.cpp-win-x86_64-avx2-2.28.2`（version 1 / fe2adf0，MSVC 19.44）<br>CUDA12：`llama.cpp-win-x86_64-nvidia-cuda12-avx2-2.67.0`（0.8.0-dev / 0893ec9，Clang 19.1）<br>Vulkan：`llama.cpp-win-x86_64-vulkan-avx2-2.28.2`（version 1 / fe2adf0） | `raw/env.json`、`summary.json` `exe_version` |
| CUDA 运行时目录 | `%USERPROFILE%\.lmstudio\extensions\backends\vendor\win-llama-cuda12-vendor-v2`（cudart64_12.dll 0.5MB、cublas64_12.dll 108.4MB、cublasLt64_12.dll 643.4MB） | `logs/`、目录实测 |
| 模型（GGUF Q4_K_M） | 0.5B：491,400,032 B（sha256 74a4da8c…a9db）<br>1.5B：1,117,320,736 B（6a1a2eb6…407e）<br>1.8B：1,133,080,512 B（4383ac0c…477c7） | `summary.json` 的 SHA256 复算 |
| 基准脚本 | `scripts/spikes/s05_s06/bench_local_translate.mjs`（**复用未改**：`--exe/--model/--gpu-layers/--threads/--runs/--warmup/--out/--log-file/--prompt-variant`） | 仓库文件 |
| 推理参数 | `-c 4096 -t 16 --host 127.0.0.1 --port 39281 --no-webui`，采样与产品一致：temperature 0.2、top_p 0.95、max_tokens 1024；200 字样例 | 脚本默认值 |
| 宿主负载 | 测量期间另有 overlay-translate agent 的 OCR 批处理与短命 CPU llama-server（-ngl 0），详见 §2.5 | 实测记录 |

> **驱动/CUDA 版本口径**：任务描述写「CUDA 12」，本机 `nvidia-smi` 实际报驱动 595.97 / **CUDA 13.2**；我们使用的是 LM Studio 的 **CUDA 12 runtime（cudart64_12/cublas64_12）** 后端，在 595.97 驱动上正常（CUDA 12 runtime 向后兼容，驱动 ≥525 即可）。

---

## 2 方法与测量纪律

### 2.1 复用脚本 + 自建执行器

- 时延测量完全复用 `bench_local_translate.mjs`（未改一行）：拉取 `/v1/chat/completions`（**非流式**）、`warmup 2 + 正式 N 次`、记录客户端总耗时/首字节、服务端 `prompt_ms/predicted_ms/tok/s`、`/health` 冷加载耗时、进程 WorkingSet/Private/Peak。
- 新增执行器（本目录）：`run_matrix.ps1`（用例矩阵 + nvidia-smi 采样 + VRAM 探针 + 残留断言）、`sample_gpu.ps1`（500ms nvidia-smi 采样）、`analyze.mjs`（汇总）、`stats.mjs`（报告统计）。
- **GPU 独占**：每用例前断言「本 harness 端口 39281/39282 无 llama-server 残留」，用例结束 `taskkill /PID /T /F` 并断言残留 0；全程同一时刻只有一个 llama-server。
- **显存采样**：`nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu,temperature.gpu,power.draw,pstate --format=csv,noheader,nounits`，500ms/次；另做「只加载、不推理、静置 3s」探针，取 5 次中位数作为「模型加载后 VRAM」。
- 用例矩阵（共 **17 组**，每组 warmup 2 + N 次）：

| # | 用例 | 后端 | 模型/提示词 | ngl | N |
| --- | --- | --- | --- | --- | --- |
| 1 | cuda-18b-product（-20r） | CUDA 2.67.0 | HY-MT 1.8B / 产品提示词 | 99 | 10 + **20** |
| 2 | cuda-18b-minimal | CUDA | 1.8B / 精简提示词 | 99 | 10 |
| 3 | cuda-15b-minimal | CUDA | Qwen2.5-1.5B / 精简 | 99 | 10 |
| 4 | cuda-05b-product | CUDA | Qwen2.5-0.5B / 产品 | 99 | 10 |
| 5 | cuda-05b-minimal | CUDA | 0.5B / 精简 | 99 | 10 |
| 6 | cuda-18b-ngl16 / ngl8 / ngl24 | CUDA | 1.8B / 产品 | 16 / 8 / 24 | 5 |
| 7 | cuda-18b-ngl0（异常对照） | CUDA | 1.8B / 产品 | 未传 `-ngl` | 3 |
| 8 | vulkan-18b-product（-20r） | Vulkan 2.28.2 | 1.8B / 产品 | 99 | 10 + **20** |
| 9 | vulkan-15b-minimal / 05b-product / 05b-minimal | Vulkan | 同上 | 99 | 10 |
| 10 | cpu-18b-product | CPU 2.28.2 | 1.8B / 产品 | 0 | 10 |
| 11 | cpu-05b-product | CPU | 0.5B / 产品 | 0 | 10 |

> 提示词口径与 S-05 对齐：**1.8B 用产品 `resources/prompts/translate.md`（762 字符）**；**1.5B/0.5B 用精简提示词**（S-05 §10 结论：小模型用产品提示词会复述原文）。

### 2.2 判定口径

- 阈值沿用 `docs/技术验证清单.md` §2 / S-05：**本地 1.8B Q4 200 字总耗时 ≤8s**（首次加载另计）。
- 本基准为**非流式**请求，客户端「首字节」= 响应完成，因此 **TTFT 无法直接测**；用服务端 `prefill_ms`（生成首 token 前的 prompt 处理）作为首字下界，见 §4.5 与 §13 局限。
- P50/P95 用线性插值分位（与既有脚本一致）。

### 2.3 干净窗口与污染

- 本机为**多 agent 共享机**。第一轮 GPU 用例（21:30–21:36）期间，overlay-translate 正在跑 OCR 批处理（8 线程）与短命 CPU llama-server（`-ngl 0`，端口 8791–8796）；**这些不占显存，但抢 CPU**，会导致 wall time 抖动。
- 之后该 agent 主动让出窗口（21:37 起 CPU 干净），我们补跑了**干净窗口的 20 次 1.8B 组 + ngl 扫描**；**报告以干净窗口数据为主**，并给出污染轮次对照（见 §4.3 注）。
- 另：第一版执行器的收尾清理误用「按镜像名 taskkill 所有 llama-server」，**误杀了 overlay agent 的 CPU server 两次**（其日志表现为 10–50s 内被 TerminateProcess）。已在执行器中修正为「只 kill 命令行含本 harness 端口（39281/39282）的进程」。此事故不影响本报告数据（对方最终用自管进程+缓存完成 86 段翻译），但**恰好证明产品侧进程清理必须按 PID/进程树，不能按镜像名**（见风险 R11）。
- 免责：本机桌面常驻（Edge/WebView2/Wallpaper Engine/NVIDIA Overlay/DSH GUI 等）占 2.2GB 显存、CPU 有背景负载，绝对数值偏保守；同机 CPU 对照（`cpu-*`）与 GPU 组在同一窗口、同一后台环境下测得，**相对加速比可信**。

### 2.4 判定口径的对照基准

| 角色 | 数据 | 说明 |
| --- | --- | --- |
| CPU 同窗实测 | 1.8B product **P50 9,767 / P95 10,137ms**；0.5B product **P50 3,989 / P95 4,111ms** | 本次干净窗口，`raw/bench_cpu-*.json` |
| S-05 历史 CPU | 1.8B P50 11,759 / P95 21,837ms（tok 17.56）；0.5B product P50 3,782 / P95 4,128；0.5B minimal P95 3,209；1.5B minimal P95 6,723 | `docs/spikes/S-05-翻译链路时延.md`，引用处已标注 |
| 云端（S-05） | DeepSeek 关 thinking：首字 P50 0.61s / P95 0.87s；总耗时 P50 1.51s / P95 1.75s | 用于「本地 GPU vs 云端」对照 |

---

## 3 后端可用性实测

### 3.1 CUDA12（llama.cpp 2.67.0）——可用，但有运行时 DLL 依赖

- **必须在子进程 PATH 中注入 CUDA 运行时目录**（LM Studio 布局）才能启动：
  ```powershell
  $env:PATH = "$env:USERPROFILE\.lmstudio\extensions\backends\vendor\win-llama-cuda12-vendor-v2;$env:PATH"
  ```
- **失败复现（强证据）**：不带该 PATH 直接运行
  `llama-server.exe --list-devices` → **退出码 -1073741515（0xC0000135 = STATUS_DLL_NOT_FOUND，即 0xC0000135）**，无 stderr（DLL 加载阶段失败）。日志：`logs/cuda_nopath_stdout*.txt`。
- 带 PATH 后：`--version` → `version: 0.8.0-dev (build 1, commit 0893ec9)`；`--list-devices` → **`CUDA0: NVIDIA GeForce RTX 4060 Laptop GPU (8187 MiB, 7106 MiB free)`**。
- 17 组 CUDA 用例 **全部 0 失败**、全部 `exit=0`、无残留。

### 3.2 Vulkan（llama.cpp 2.28.2）——可用，零外部运行时依赖

- 不需要注入 PATH（Vulkan loader 在系统/驱动中），直接可跑：`--list-devices` → **`Vulkan0: NVIDIA GeForce RTX 4060 Laptop GPU (7956 MiB, 7180 MiB free)`**。
- 注意 Vulkan 报的可用显存比 CUDA 少 ~230 MiB（8187 vs 7956；WDDM/驱动开销），预算判定要用各后端的真实枚举值。
- 全部 Vulkan 用例 0 失败、无残留。

### 3.3 CPU（2.28.2）——对照组

- 纯 CPU `-ngl 0`（不传 CUDA 运行时、CPU 专用构建）：1.8B product P50 9.77s / P95 10.14s / tok 20.64；0.5B product P50 3.99s / P95 4.11s / tok 51.69。与 S-05 结论一致（S-05 当时窗口更吵，P95 21.84s）。

### 3.4 关键启动参数语义（实测陷阱）

| 参数 | 结论 | 证据 |
| --- | --- | --- |
| `-ngl N` | 显式传 `-ngl 0` **确实不 offload**（VRAM 增量仅 157 MiB = CUDA 上下文开销）；显式 `-ngl 99/16/8/24` 与显存增量线性对应 | 手动探针：`-ngl 0` → Δ157MiB；`-ngl 99` → Δ1,497MiB |
| **不传 `-ngl`** | **LM Studio 构建默认「自动全量 offload」**：`cuda-18b-ngl0` 用例（脚本在 gpuLayers=0 时不加 `-ngl`）实测 VRAM Δ1,497MiB、P50 1.25s → 等价全 GPU，**不是 CPU**。→ **产品必须显式传 `-ngl`（CPU 传 0、GPU 传 99/层数），绝不能省略** | `raw/bench_cuda-18b-ngl0.json`（P50 1,254.8ms、tok 127） |
| `--device CUDA0` / `--device Vulkan0` | **支持**，可显式指定设备（多 GPU/核显场景必需） | `-ngl 99 --device CUDA0` 启动成功、Δ1,497MiB；Vulkan 同理 Δ1,403MiB |
| `--list-devices` | **产品做 GPU 探测的最佳探针**：直接枚举 llama.cpp 实际可用的后端/设备/总/空闲显存 | 两后端输出见 §3.1/3.2 |
| `--split-mode` / `--tensor-split` / `--main-gpu` / `--fit` | 均存在（2.28.2/2.67.0 帮助文本已核）；`--fit` 可按设备内存自动调整未设置的参数，可作为「显存不足自动降档」的备用方案 | `logs/cuda_help2.txt`、`logs/vulkan_help.txt` |

### 3.5 设备枚举与虚拟适配器

- DXGI/WMI 层能看到 **3 个显示适配器**（RTX 4060 + MuMu 虚拟显示 + GameViewer 虚拟显示）。若产品用 DXGI 枚举做探测，**必须排除虚拟/软件适配器**（vendorId/deviceId 白名单或 `DedicatedVideoMemory>0` 且非 Microsoft Basic Render）。
- 实测 llama.cpp 的 `--list-devices` **只列出 NVIDIA 物理卡**（CUDA0 / Vulkan0），是更可靠的「能不能用」判据（因为它连运行时一起验证了）。

### 3.6 打包布局验证：DLL 放 exe 旁边即可，无需 PATH

- 构造测试目录：CUDA 后端全部文件 + `cudart64_12.dll` + `cublas64_12.dll` + `cublasLt64_12.dll` **与 llama-server.exe 同目录**，PATH 中**不含** vendor 目录 → `--list-devices` **exit=0，正常输出 CUDA0**。
- 结论：**产品分发时可把 CUDA runtime DLL 与被测 exe 放同一目录**（Windows DLL 搜索顺序：exe 目录优先），无需改 PATH；本研究的 PATH 注入只是 LM Studio 目录布局的适配。
- 证据：`logs/packtest_listdevices.txt`、`logs/packtest_note.txt`（测试用硬链接，未复制 0.9GB）。

---

## 4 时延基准（200 字中文，非流式，`temperature=0.2`）

### 4.1 主表（干净窗口，客户端总耗时）

| 模型 / 提示词 | 后端 | n | **P50 (ms)** | **P95 (ms)** | max (ms) | tok/s P50 | prefill P50 | decode P50 | 冷加载(ms) | 失败 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| HY-MT 1.8B / 产品 | **CPU** 2.28.2 | 10 | 9,767.3 | 10,136.6 | 10,171.3 | 20.64 | 2,462.4 | 7,260.5 | 1,472 | 0 |
| HY-MT 1.8B / 产品 | **CUDA** 2.67.0（20 次主口径） | 20 | **1,256.0** | **3,746.6** | 3,988.1 | **125.69** | **58.1** | 1,186.6 | 1,133 | 0 |
| HY-MT 1.8B / 产品 | CUDA（10 次，含污染轮对照） | 10 | 1,273.8 | 3,937.1 | 4,054.6 | 127.24 | 58.2 | 1,198.9 | 1,670 | 0 |
| HY-MT 1.8B / 产品 | **Vulkan** 2.28.2（20 次主口径） | 20 | 1,264.3 | **4,081.2** | 4,098.3 | **129.65** | 82.3 | 1,159.1 | 1,901 | 0 |
| HY-MT 1.8B / 产品 | Vulkan（10 次，含污染轮对照） | 10 | 4,110.6 | 4,471.0 | 4,474.3 | 120.81 | 84.8 | 4,018.6 | 3,645 | 0 |
| HY-MT 1.8B / 精简 | CUDA | 10 | 1,280.1 | 1,350.7 | 1,389.7 | 129.17 | 25.7 | 1,243.7 | 1,120 | 0 |
| Qwen2.5-1.5B / 精简 | **CPU**（S-05 §10.2） | 10 | 6,443.4 | 6,723.4 | 6,894.5 | 25.66 | — | — | 1,445 | 0 |
| Qwen2.5-1.5B / 精简 | **CUDA** | 10 | **1,087.4** | **1,143.9** | 1,148.4 | 135.35 | 24.0 | 1,061.0 | 1,556 | 0 |
| Qwen2.5-1.5B / 精简 | **Vulkan** | 10 | 1,026.3 | 1,105.9 | 1,147.9 | 151.31 | 40.0 | 981.4 | 2,593 | 0 |
| Qwen2.5-0.5B / 产品 | **CPU** | 10 | 3,989.1 | 4,111.1 | 4,131.7 | 51.69 | 1,543.7 | 2,418.2 | 913 | 0 |
| Qwen2.5-0.5B / 产品 | **CUDA** | 10 | **578.7** | **591.0** | 594.9 | 227.26 | 30.4 | 545.6 | 917 | 0 |
| Qwen2.5-0.5B / 产品 | **Vulkan** | 10 | 600.2 | 618.8 | 622.2 | 226.23 | 40.2 | 552.5 | 2,208 | 0 |
| Qwen2.5-0.5B / 精简 | CPU（S-05 §10.1） | 10 | 3,091.8 | 3,209.4 | — | — | — | — | 1,341 | 0 |
| Qwen2.5-0.5B / 精简 | **CUDA** | 10 | 691.9 | 733.3 | 737.6 | 221.78 | 23.2 | 667.4 | 811 | 0 |
| Qwen2.5-0.5B / 精简 | **Vulkan** | 10 | 715.1 | 753.7 | 762.9 | 215.25 | 25.7 | 682.5 | 1,465 | 0 |

> 原始数据：`raw/bench_<case>.json`（每行一次请求的 total/ttfb/prompt/decode/tok/输出文本），汇总见 `summary.json`。所有用例 `failures=0`、`residue=false`。

### 4.2 加速比（GPU vs CPU，同模型同提示词同参数）

| 场景 | CPU P50/P95 | CUDA P50/P95 | 加速比 P50 / P95 | tok/s 加速 |
| --- | --- | --- | --- | --- |
| **1.8B + 产品提示词** | 9,767 / 10,137 | **1,256 / 3,747** | **7.8× / 2.7×**（非长尾 P95 1,290 → **7.9×**） | 20.6 → 125.7 = **6.1×** |
| 1.8B + 精简提示词 | 未测（S-05 无） | 1,280 / 1,351 | —（对比 1.8B CPU 产品口径 7.6×/7.5×） | — |
| 1.5B + 精简（vs S-05 CPU） | 6,443 / 6,723 | 1,087 / 1,144 | **5.9× / 5.9×** | 25.7 → 135.4 = **5.3×** |
| 0.5B + 产品提示词 | 3,989 / 4,111 | 579 / 591 | **6.9× / 7.0×** | 51.7 → 227.3 = **4.4×** |
| 0.5B + 精简（vs S-05 CPU） | 3,092 / 3,209 | 692 / 733 | **4.5× / 4.4×** | — |

- **Vulkan vs CUDA**（本机）：1.8B P50 1,264 vs 1,256（≈持平），tok/s 129.7 vs 125.7（Vulkan +3%），prefill 82 vs 58ms（CUDA 快），冷加载 1,901 vs 1,133ms（CUDA 快）。**注意两者 llama.cpp 版本不同（2.28.2 vs 2.67.0），不是严格同版本 A/B**；结论应为「本机两后端总耗时同量级，CUDA 在 prefill/加载略优」。

### 4.3 长尾：1.8B 偶发「不停止」重复生成（P95 的主因，与 GPU 无关）

- 1.8B 产品提示词下，部分请求不会及时输出 EOS，`completion_tokens` 从正常的 **150** 膨胀到 **414–537**（max_tokens=1024 截断前自行停止），单次耗时从 1.25s 变为 3.7–4.5s。**CPU 上同样存在**（S-05 P95 21.84s 的来源之一）。
- 统计（干净窗口）：CUDA 20 次中 **3/20** 长尾；Vulkan 20 次中 **9/20**（样本小，不能断言后端差异；更可能是随机采样）。
- **去掉长尾后的「正常请求」分布**：CUDA P50 1,252 / P95 1,290ms；Vulkan P50 1,239 / P95 1,264ms。
- 含义：① 1.8B GPU 即使按最差 P95（3.75–4.08s）也 **≤8s**；② 但若产品要承诺「≤2s」，必须做**重复检测/stop 序列/max_tokens 限长**，否则长尾会突破；③ 部分 offload（§5.4）下长尾会把单次推到 20s，**风险显著放大**。

### 4.4 与云端对照（S-05 数据）

| 链路 | 首字 P50/P95 | 总耗时 P50/P95 |
| --- | --- | --- |
| 云端 DeepSeek（thinking 关） | 0.61s / 0.87s | 1.51s / 1.75s |
| **本地 1.8B GPU（本机）** | prefill ≈58ms（≈首字下界，见 §4.5） | **1.26s / 3.75s**（正常请求 1.25/1.29s） |
| 本地 1.8B CPU | prefill 2.46s | 9.77s / 10.14s |

**产品含义（重要）**：GPU 让「本地 1.8B 质量档」的**总耗时进入云端同级**（P50 甚至更快），同时保持离线/隐私；P95 略差于云端（3.7s vs 1.75s，长尾造成），仍远优于 8s 阈值。

### 4.5 流式首字推算（本基准非流式，给出方法与下界）

- 非流式请求的「首字节 = 完整响应」，故 TTFT 不可直接测；但服务端 `prefill_ms` 是「prompt 处理完、开始生成首 token」的时间：
  - 1.8B：CPU 2,462ms → CUDA **58ms** / Vulkan **82ms**
  - 1.5B：CPU ≈708ms（S-05） → CUDA **24ms**；0.5B：CPU 1,544ms → CUDA **30ms**
- 合理预期：**本地 GPU + `stream:true` 的首 token 在 0.1–0.2s 量级**（含 HTTP/进程内开销），远低于云端首字阈值 1.5s。**建议实现流式后补一条实测**（见 §13 遗留）。

---

## 5 显存、功耗与 8GB 适配

### 5.1 空载基线（关键约束）

| 指标 | 值 |
| --- | --- |
| 总显存 | 8,188 MiB |
| 桌面空载已用 | **2,082–2,326 MiB**（本次会话波动；`env.json` 采样 2,201 MiB = 26.9%），来源：WebView2/Edge、Wallpaper Engine、NVIDIA Overlay、DSH GUI、Steam 等 |
| 空载功耗/温度 | 9.78W / 44°C / P5（util 29% 为桌面动画） |
| 可用（扣桌面后） | **≈5.9–6.1 GB** |

### 5.2 模型加载后显存（只加载不推理，静置 3s 取中位数）

| 模型 | 后端 | VRAM 增量 | 加载后总占用 | 8GB 剩余 | 备注 |
| --- | --- | --- | --- | --- | --- |
| 1.8B Q4（ngl 99） | CUDA | **+1,498 MiB** | 3,706 MiB | ≈4.4 GB | 权重 1.08GB + KV/compute |
| 1.8B Q4（ngl 99） | Vulkan | +1,403 MiB | 3,611 MiB | ≈4.5 GB | |
| 1.5B Q4（ngl 99） | CUDA | +1,226 MiB | 3,427 MiB | ≈4.7 GB | |
| 1.5B Q4（ngl 99） | Vulkan | +1,125–1,133 MiB | 3,333 MiB | ≈4.8 GB | |
| 0.5B Q4（ngl 99） | CUDA | +566–573 MiB | 2,767 MiB | ≈5.4 GB | |
| 0.5B Q4（ngl 99） | Vulkan | +474–482 MiB | 2,682 MiB | ≈5.4 GB | |
| 1.8B（ngl 16） | CUDA | +900 MiB | 3,101 MiB | ≈5.0 GB | 部分 offload |
| 1.8B（ngl 8） | CUDA | +616 MiB | 2,817 MiB | ≈5.3 GB | 部分 offload |
| 1.8B（ngl 24） | CUDA | +1,180 MiB | 3,381 MiB | ≈4.8 GB | 部分 offload |

- 推理时峰值再高约 0–55 MiB（1.8B 全量实测峰值总占用 **3,712 MiB**，即 45.3%）。
- 对比 `resources/models.manifest.json` 的 `minRamMB`（内存口径），GPU 显存需求更小：**建议清单新增 `vramMB`：0.5B→700、1.5B→1300、1.8B→1600**（含 overhead 与余量）。

### 5.3 8GB 机型预算表（本机实测口径，扣桌面 2.2GB）

| 场景 | 模型 | 需要 VRAM（增量） | 加载后总占用 | 8GB 是否可跑 | 建议 |
| --- | --- | --- | --- | --- | --- |
| 桌面空载 2.2GB | 1.8B | 1.50 GB | 3.7 GB | ✅ 余 4.4GB | 默认可用 |
| 桌面 + 浏览器 4GB | 1.8B | 1.50 GB | 5.5 GB | ✅ 余 2.7GB | 可用，但需预算检查 |
| 桌面 + 3D 游戏 6GB | 1.8B | 1.50 GB | 7.5 GB | ⚠️ 余 0.7GB | **建议回退 CPU 或降 ngl** |
| 桌面 + 游戏 7GB+ | 1.8B | 1.50 GB | 8.5GB | ❌ 超限 | 回退 CPU（或 WDDM 共享内存→严重变慢） |
| 任意 | 0.5B | 0.57 GB | ≤2.9GB | ✅ | 极小档对显存几乎无压力 |

### 5.4 部分 offload 扫描（CUDA，1.8B 产品提示词，N=5）

| `-ngl` | VRAM 增量 | P50 (ms) | P95 (ms) | max (ms) | tok/s | 相对 CPU 加速 |
| --- | --- | --- | --- | --- | --- | --- |
| 0（CPU 对照） | 0 | 9,767 | 10,137 | 10,171 | 20.64 | 1.0× |
| 8 | +616 MiB | 6,321 | **20,178** | 20,991 | 25.65 | 1.55× |
| 16 | +900 MiB | 4,575 | 4,696 | 4,698 | 35.60 | 2.13× |
| 24 | +1,180 MiB | 2,907 | **8,174** | 9,489 | 54.62 | 3.36× |
| 99（全量） | +1,498 MiB | **1,256** | 3,747 | 3,988 | 125.69 | 7.78× |

**结论**：部分 offload 是**有效的降档手段**（ngl16 仍能 4.6s ≤8s），但**长尾风险陡增**（ngl8 单次 20s、ngl24 P95 8.2s 超阈）→ 产品降档策略应优先「回退 CPU + 明确提示」，`gpuLayers` 只作为高级选项，并建议配合 `max_tokens` 限长/重复检测。

### 5.5 温度 / 功耗 / 利用率（采样峰值）

| 用例 | GPU util 中位/峰值 | 温度峰值 | 功耗峰值 | 备注 |
| --- | --- | --- | --- | --- |
| 空载 | 29% / 42% | 44–50°C | 9.8–17W | 桌面动画 |
| 0.5B CUDA 推理 | 67% / 70% | 56°C | 52–54W | |
| 1.5B CUDA 推理 | 78% / 80% | 61°C | 66W | |
| 1.8B CUDA 推理 | 82% / 84–87% | 61°C | 76W | |
| 1.8B Vulkan 推理 | 82% / 94% | 66°C | 81–85W | |
| 1.8B 加载（探针） | — | 48–50°C | 18W | |

- 单次 200 字翻译仅 1.3s 满载，**瞬时功耗高但持续时间短**，对笔记本散热的冲击有限；但**批量/连续翻译**会持续 80W 级负载（电池续航与风扇噪音需产品提示）。
- 建议（可选增强）：检测 `AC/DC`（`GetSystemPowerStatus`）或温度，提供「仅插电时使用 GPU」开关；或对连续任务做 1–2s 节流。

### 5.6 宿主内存（GPU offload 的内存红利）

| 模式 | llama-server WorkingSet |
| --- | --- |
| 1.8B GPU（ngl 99） | **≈1,471 MB**（CUDA）/ 1,232 MB（Vulkan） |
| 1.8B CPU（S-13 实测） | ≈2,102 MB |
| 0.5B GPU | ≈769 MB（CUDA）/ 538 MB（Vulkan） |

→ GPU 模式把权重放进显存，**宿主内存反而更省**（1.8B 省约 0.6GB），对 8GB/16GB 内存机型都是加分项。

---

## 6 对 S-05 / S-13 阈值的影响

| 验收口径（`docs/技术验证清单.md`） | 阈值 | CPU 实测 | **GPU 实测** | 判定 |
| --- | --- | --- | --- | --- |
| 本地 1.8B Q4 200 字（总耗时） | ≤8,000 ms | P50 9,767 / P95 10,137（S-05 历史 11,759/21,837） | **P50 1,256 / P95 3,747 / max 3,988** | **DONE（GPU）/ FAILED（CPU 维持原判）** |
| 极小档 0.5B + 精简提示词（200 字） | P95 ≤8s（产品目标 3.2s） | P95 3,209（S-05） | **P95 692（CUDA）/ 715（Vulkan）** | **DONE，且比目标快 4.4–4.6×** |
| 1.5B + 精简提示词（200 字） | ≤8,000 ms | P95 6,723（S-05） | **P95 1,144（CUDA）/ 1,106（Vulkan）** | **DONE（5.9×）** |
| 本地「快速模式首字」 | ≤1,500 ms | CPU prefill 2.46s（1.8B）→ 不达标 | **CUDA prefill 58ms / Vulkan 82ms**（流式首字预计 <0.2s） | **预期 DONE，待流式实测** |
| S-13 资源占用（内存） | 常驻内存 ≤ 清单 | 0.5B 0.55GiB / 1.5B 1.70GiB / 1.8B 2.10GiB | GPU 模式宿主内存更低（1.8B 1.47GB），**新增显存占用 0.57/1.23/1.50GB** | **需把「显存」纳入 8GB 机型降档判定** |

**产品口径结论**：
1. **1.8B 产品档 + GPU 后从「不可用/慢路径」升级为「可用且快」**（P95 3.75s，正常请求 1.29s），是本次研究最大的产品收益；
2. 「极小档 0.5B」在 GPU 下 P95 <0.75s，但**质量结论不变**（产品提示词下仍复述中文，实测 10/10 CJK 79.5%；GPU 只是更快，不改变质量）→ 仍须走精简提示词并保留 UI 质量提示；
3. 8GB 机型**必须把显存纳入降档判定**（桌面已占 2.2GB；游戏/浏览器可占 4–7GB），否则会出现「加载失败」或「WDDM 共享显存→严重变慢」；
4. 对 S-13 的补充：**GPU 模式会新增 0.57–1.50GB 显存占用**，建议在模型清单增加 `vramMB` 并在设置/引导页做「显存预算」提示，与内存降档同一套逻辑。

---

## 7 结论

1. **GPU 完全可用**：RTX 4060 Laptop 8GB + 驱动 595.97 上，CUDA12 与 Vulkan 两套 llama.cpp 后端均跑通，17 组 × (10–20) 次基准 **0 失败**；CUDA 需带 CUDA runtime DLL（本机由 LM Studio vendor 目录提供，0xC0000135 可复现），**产品把 DLL 与 exe 同目录即可**（已实测验证）。
2. **加速显著且真实**：1.8B 产品档 **P50 7.8× / P95 2.7×（正常请求 7.9×）**，tok/s 6.1×，prefill 42×；0.5B 6.9×；1.5B 5.9×。
3. **1.8B 本地质量档达到产品阈值**：GPU 下 P95 3.75s ≤ 8s（CPU 为 10.1s 不达标），且总耗时进入云端同级（P50 1.26s vs 云端 1.51s）。
4. **显存适配 8GB**：桌面空载 2.2GB + 1.8B 模型 1.5GB = 3.7GB（45%），余 4.4GB；但**游戏/浏览器抢占时需回退/降档**，默认策略必须是 auto + 预算检查 + 失败回退。
5. **后端推荐**：**双后端**——默认随包的按需组件用 **Vulkan**（≈56MB、无需 CUDA runtime、跨厂商），NVIDIA 用户可选 **CUDA 组件**（≈0.9GB，prefill/加载略优）；探测到 NVIDIA 且有 CUDA 组件时优先 CUDA，否则 Vulkan，再否则 CPU。
6. **默认值**：`local.device = auto`（不是硬编码 gpu），`gpuLayers` 默认「全部」（99，实际按显存预算可降）；**CPU 永远保留为兜底**，且**启动时必须显式传 `-ngl`**（LM Studio 构建省略时会自动全量 offload，实测踩坑）。
7. **实施成本可控**：GPU 增量约 **14.5 人日**，MVP **≈4.5 人日** 可先落地「手动 CPU/GPU 开关 + 手动放置后端 + 设置项 + 回退」。

---

## 8 风险与缓解

| # | 风险 | 实测/依据 | 影响 | 缓解 |
| --- | --- | --- | --- | --- |
| R1 | **显存被浏览器/游戏/Wallpaper 抢占** | 桌面空载已占 2.2GB（26.9%）；8GB 机型游戏可占 4–7GB | 加载失败或退化到共享内存（严重变慢） | 启动前查 `free VRAM`；预算 = 模型 `vramMB` + 512MB 预留；不足则回退 CPU 并可提示「关闭游戏/浏览器后重试」 |
| R2 | **WDDM 共享显存误判** | 驱动 595.97 为 WDDM；CUDA 报告 8187MiB，Vulkan 报告 7956MiB（差 231MB） | 看总量「够」但分配会溢出到系统内存，tok/s 骤降 | 用 **free** 而不是 total 判定；留 512–1024MB 余量；运行中若 tok/s 低于预期阈值 → 降档/回退 |
| R3 | **笔记本功耗/温度/续航** | 1.8B 满载峰值 76–85W、61–66°C；空载 9.8W | 电池续航、风扇噪音；长时间连续翻译发热 | 「仅插电使用 GPU」选项（`GetSystemPowerStatus`）；连续任务节流；UI 提示 |
| R4 | **驱动兼容（旧驱动/无 CUDA）** | 缺 CUDA runtime → **0xC0000135**；旧驱动可能不满足 CUDA 12 runtime | CUDA 后端启动失败 | 失败回退 Vulkan → CPU；安装前提示驱动版本；把「启动失败→可操作错误」纳入错误码 |
| R5 | **CUDA runtime 体积与许可** | 实测 cudart+cublas+cublasLt = **752.4MB**，ggml-cuda.dll 143.8MB，CUDA 后端目录合计 168.5MB → **额外 ≈0.9GB**；Vulkan 后端目录 55.8MB（ggml-vulkan 39.4MB） | 安装包/磁盘占用；NVIDIA CUDA EULA 再分发条款 | **按需下载组件**（复用 S-08 下载/校验/断点续传）；Vulkan 为默认组件；CUDA 可选；随包附 license/notice |
| R6 | **Vulkan 性能差异** | 本机 1.8B P50 1.264s vs CUDA 1.256s（≈持平），tok +3%，prefill +42%，加载 +68%；但两版本不同（2.28.2 vs 2.67.0） | 不同厂商/驱动差异可能更大；INTEL/AMD 上表现未知 | 以「能用」为目标；保留 CUDA 选项；发布前在 AMD/Intel 实机抽测 |
| R7 | **启动时延** | 冷加载（页面缓存热）：CUDA 1.8B 1.13–1.67s，Vulkan 1.90–3.65s；CPU 1.47s | 首次翻译前等待；与 S-10 冷启动口径独立 | 空闲时预热（`-ngl` 加载模型）+ `engine:status` 事件；加载超时/失败可操作错误 |
| R8 | **多 GPU / 虚拟适配器** | 本机有 2 个虚拟显示适配器（MuMu、GameViewer） | DXGI 误判把虚拟卡当 GPU | 用 llama.cpp `--list-devices`（实测只列物理 NVIDIA）；DXGI 兜底时过滤虚拟/基本渲染适配器；`--device CUDA0/Vulkan0` 显式指定 |
| R9 | **模型质量不受加速影响，但 0.5B 仍不可直接当翻译后端** | GPU 下 0.5B + 产品提示词 10/10 仍复述中文（CJK 79.5%，输出 sha256 == 输入 sha256） | 用户误以为「本地=可用」 | 沿用 S-05 结论：0.5B 只配精简提示词并标注「质量一般」；1.8B/1.5B 用产品/精简模板路由 |
| R10 | **长尾重复生成** | 1.8B 产品提示词：150 tokens 正常，偶发 414–537 tokens（3/20 CUDA、9/20 Vulkan 样本）；CPU 同样存在 | P95 从 1.3s 抬到 3.7–4.5s；部分 offload 下可到 20s | 重复检测/早停（同 n-gram 重复）、`max_tokens` 限长、stop 序列；P95 监控 |
| R11 | **本地进程管理（含清理误伤）** | 本研究第一版 harness 按镜像名 kill 误杀了其它 agent 的 llama-server | 误杀用户其它程序 | **按 PID/进程树 kill**（`taskkill /PID x /T /F`），禁止按镜像名；应用退出/崩溃时兜底清理；端口独占（39281–39289 探测） |
| R12 | **模型/后端版本升级回归** | 本次用 LM Studio 打包的 llama.cpp（CUDA 2.67.0 / Vulkan 2.28.2），非官方 release | 升级后参数/默认行为变化（如 `-ngl` 默认值） | 固定后端版本 + 集成测试覆盖「CPU/GPU 各跑一次 1.8B 200 字 ≤8s」；版本升级走灰度 |

---

## 9 产品设计（最小改动清单）

> 目标：**设置里可选「自动 / CPU / GPU」**，GPU 可用时明显更快、不可用/显存不足时无感回退，且分发体积可控。
> 现状：`LocalPrefs` 只有 `engine/model/port/idleUnloadSec`（`src-tauri/src/contract/config.rs:527-554`；TS `src/shared/contract/types.ts:156-161`；默认值 `src/shared/contract/defaults.ts:91-97`），**全仓没有任何 GPU/ngl 相关代码**（`src` 与 `src-tauri/src` 全量检索 0 命中），本地引擎真实启动仍是 `TODO(real)`（`src-tauri/src/app/lifecycle.rs:69-70`、`:124`；`src-tauri/src/platform/process.rs` 只有 `StoppedSupervisor`）。

### 9.1 配置契约（Rust）

`src-tauri/src/contract/config.rs`：

```rust
/// 本地推理计算设备。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LocalDevice {
    /// 自动：有可用 GPU 且显存预算足够则用 GPU，否则 CPU。
    #[default]
    Auto,
    /// 强制 CPU（-ngl 0）。
    Cpu,
    /// 强制 GPU（-ngl 按预算/配置，失败回退 CPU 并提示）。
    Gpu,
}

/// 本地推理偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalPrefs {
    pub engine: LocalEngine,
    pub model: String,
    pub port: u16,
    pub idle_unload_sec: u32,
    /// 新增：计算设备（auto/cpu/gpu）。
    pub device: LocalDevice,
    /// 新增：GPU 卸载层数（99 = 全部；0 等价纯 CPU；高级选项）。
    pub gpu_layers: u16,
    /// 新增（可选，M2）：GPU 后端偏好 auto/cuda/vulkan。
    pub gpu_backend: LocalGpuBackend,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            engine: LocalEngine::LlamaCpp,
            model: "hy-mt-1.8b-q4".to_string(),
            port: 39281,
            idle_unload_sec: 600,
            device: LocalDevice::Auto,   // 默认自动
            gpu_layers: 99,              // 全部层（实际由预算/降档决定）
            gpu_backend: LocalGpuBackend::Auto,
            extra: serde_json::Map::new(),
        }
    }
}
```

校验（`validate()`，`config.rs:933` 起；遵循「非法字段 → `SA-5001` + 字段名」）：

```rust
// 局部
if config.local.gpu_layers > 999 {
    return Err(AppError::invalid_config("local.gpuLayers", "取值范围 [0,999]（99=全部层）"));
}
// device 枚举非法由 serde 反序列化报 SA-5001（migrate 的 from_value 已包装）
```
- 语义：`device=cpu` 时启动参数固定 `-ngl 0`（**不拒绝 gpuLayers**，忽略即可）；`device=gpu` 且 `gpuLayers=0` 视为非法组合（可拒绝或启动时纠正为 99 + 提示）。
- `config.rs:1256` 起的测试需同步：`migrate_from_v1_*` 期望 `schema_version` 变为新版本。

### 9.2 Schema 版本与迁移兼容策略（推荐 **v4**）

- 现状：`CONFIG_SCHEMA_VERSION = 3`（`config.rs:22`）；`validate()` 要求 `schema_version == CONFIG_SCHEMA_VERSION`（`config.rs:933-939`）；`migrate()` 是 `1→2→3` 链 + 末尾强制写当前版本（`config.rs:1215-1248`）。
- **推荐 B 方案（显式 v4）**：

```rust
pub const CONFIG_SCHEMA_VERSION: u32 = 4;

fn migrate_v3_to_v4(value: &mut serde_json::Value) {
    let defaults = serde_json::json!({
        "local": { "device": "auto", "gpuLayers": 99, "gpuBackend": "auto" }
    });
    fill_defaults(value, &defaults);
}
// migrate(): if current < 4 { migrate_v3_to_v4(&mut value); current = 4; }
// 测试：migrate_from_v3_keeps_user_values（保留 local.engine/model/port/idleUnloadSec，新增字段落默认）；
//       migrate_from_v1/v2 断言 schema_version == 4。
```
- 兼容性：
  - **旧→新**：v1/v2/v3 配置迁移后自动获得 `device=auto`，用户无感。
  - **新→旧**（回滚/多实例）：旧版反序列化时未知字段进 `local.extra`（`#[serde(flatten)]`），**不会丢**；但旧版不认识 `schemaVersion:4` 会拒绝 — 回滚需接受「旧版拒绝新配置」或提供导出降级。（若产品不做回滚兼容，这是可接受代价。）
  - **A 方案（不升版本）**：只加 `#[serde(default)]` 字段、保持 v3。好处：旧版可读新配置、零迁移；坏处：契约版本不再反映字段变更，且 `validate()` 不校验新字段。**若团队希望零风险上线，A 也可行**；建议在 `docs/接口契约.md` 同步 §4.1 schema 与校验规则（`接口契约.md:138/172/205/221`）。

### 9.3 TS 类型与默认值

`src/shared/contract/types.ts`：
```ts
export type LocalDevice = "auto" | "cpu" | "gpu";
export type LocalGpuBackend = "auto" | "cuda" | "vulkan";
export interface LocalPrefs {
  engine: LocalEngine;
  model: string;
  port: number;
  idleUnloadSec: number;
  device: LocalDevice;      // 新增
  gpuLayers: number;        // 新增（99 = 全部）
  gpuBackend: LocalGpuBackend; // 新增（可选）
}
```
`src/shared/contract/defaults.ts`（local 段）：`device: "auto", gpuLayers: 99, gpuBackend: "auto"`。
`src/windows/main/settings/presets.ts`（与 `LOCAL_ENGINES` 同风格）：
```ts
export const LOCAL_DEVICES: { value: LocalDevice; label: string; description: string }[] = [
  { value: "auto", label: "自动（推荐）", description: "检测到可用 GPU 且显存足够时使用 GPU，否则 CPU" },
  { value: "cpu", label: "CPU", description: "兼容性最好，速度较慢" },
  { value: "gpu", label: "GPU（推荐）", description: "本地模型加速，需显卡驱动；失败自动回退 CPU" },
];
export const GPU_LAYER_PRESETS = [{ value: 99, label: "全部层（推荐）" }, { value: 32, label: "32 层" }, { value: 16, label: "16 层" }, { value: 0, label: "0 层（纯 CPU）" }];
```
`diff.ts` 的 `CONFIG_KEYS` 已含 `"local"`，**无需改动**（config_patch 深度合并；`local.*` 变更按契约触发引擎重启，`docs/接口契约.md:221`）。

### 9.4 设置页「本地翻译」面板（`TranslateSection.tsx:146-185`）

最小 UI（在「推理引擎」之后插入）：
1. **计算设备**：`SegmentedControl<LocalDevice>`（自动（推荐）/ CPU / GPU（推荐））→ `updateLocal({ device })`。
2. **设备状态行**（InfoRow，只读）：如「已检测：NVIDIA GeForce RTX 4060 Laptop GPU（显存 8187 MiB，空闲 7106 MiB，CUDA12 可用）」/「未检测到可用 GPU，将使用 CPU」。数据来自新增 `local_gpu_probe` 命令（§9.6），失败时降级为「未知」。
3. **GPU 层数（高级）**：`NumberField`（0–99，默认 99，仅 `device!=="cpu"` 时显示；hint：「99=全部层；显存不足可降低，但可能变慢」）。
4. **提示**：选中 GPU 时若探测到空闲显存 < 模型预算 + 512MB，显示 inline warning「显存可能不足，建议关闭游戏/浏览器，或将设备切换为自动」；回退发生时在保存/试译结果处提示「已自动回退 CPU：原因 …」。
5. 变更后保存走现有 `SaveBar`/`buildPatch`；保存提示补充「切换计算设备会重启本地引擎」（现有文案已有「切换本地引擎会触发引擎重启」，`TranslateSection.tsx:97`）。

### 9.5 llama-server 启动参数（引擎侧，`platform/process.rs` 真实实现时）

| device | 参数 |
| --- | --- |
| cpu | `-m <gguf> -c 4096 -t 16 -ngl 0 --host 127.0.0.1 --port 39281 --no-webui` |
| gpu（全部） | `-ngl 99 --device CUDA0`（或 `Vulkan0`）+ 同上；**DLL 与 exe 同目录或注入 PATH** |
| gpu（降档） | `-ngl <layers>`（按显存预算），如 1.8B → 24/16 |
| auto | 探测后按上面两条之一 |

- **必须显式传 `-ngl`**（LM Studio 构建省略 = 自动全量 offload，实测踩坑；官方 llama.cpp 省略 = 0 层，语义不同 → 以显式传参消除差异）。
- `-t`：GPU 模式下 CPU 线程主要做采样/tokenize，保持 16 或降为 8（实测 16 可用；降线程可减少对其它应用的干扰）。
- 启动失败/超时 → 明确错误（复用 S-13 建议：`SA-9001` 类 + `fallback: ["cpu"]`），不要卡 UI。
- 空闲卸载（`idleUnloadSec=600`）与 GPU 显存释放：停止进程树即释放；`nvidia-smi` 实测停止后显存回到基线（2201MiB）。

### 9.6 GPU 探测与显存预算

新增 Rust 模块（如 `platform/gpu.rs`）+ 命令 `local_gpu_probe`（返回结构进契约）：

```ts
interface GpuProbe {
  available: boolean;
  backend: "cuda" | "vulkan" | "cpu";
  name: string | null;          // "NVIDIA GeForce RTX 4060 Laptop GPU"
  vramTotalMiB: number | null;
  vramFreeMiB: number | null;
  driverVersion: string | null;
  reason: string | null;        // 不可用/回退原因（可操作文案）
}
```
探测顺序（**快速失败、结果可信**）：
1. `nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader`（有则拿到物理卡与总量；Windows 普遍有 NVIDIA 驱动；AMD/Intel 无此命令）；
2. **首选**：`<bundled llama-server> --list-devices`（实测输出 `CUDA0/Vulkan0: <name> (<total> MiB, <free> MiB free)`；它同时验证了后端 DLL 可加载 —— 这是「真能用」判据）；
3. DXGI 兜底：`IDXGIFactory::EnumAdapters` 过滤虚拟/软件适配器（本机 2 个虚拟显示适配器），只接受 `DedicatedVideoMemory > 0` 的硬件适配器；**不把 SharedSystemMemory 当可用**；
4. 缓存探测结果（如 60s），失败不阻塞 UI。
- 预算判定：`need = model.vramMB(manifest) + 512MiB`；`free >= need` 才允许 GPU；否则 CPU/降档。
- 建议 manifest 新增 `vramMB`：0.5B=700、1.5B=1300、1.8B=1600（依据 §5.2 实测增量 + 余量），与 `minRamMB` 并列。

### 9.7 失败回退与降档链

```
选择 GPU（auto 或显式）
  ├─ 探测不可用 / free VRAM 不足 → CPU（auto 静默；显式 GPU 提示原因）
  ├─ 启动失败（0xC0000135 / 非零退出 / health 超时 20s）
  │    ├─ CUDA → 尝试 Vulkan（若组件存在）→ CPU
  │    └─ Vulkan → CPU
  ├─ 启动成功但 tok/s 显著低于预期（<30% 基线，疑似共享显存/抢占）→ 提示 + 可回退
  └─ 运行中 OOM/崩溃 → 重启一次（降 ngl 或 CPU），UI 给可操作提示
```
- 回退必须**用户可感知但不打断**：`engine:status` 事件携带 `device` 与 `fallbackReason`；结果窗/设置页展示「本机已用 CPU（GPU 显存不足）」。
- 显存不足的「降档」顺序建议：`-ngl 99 → 24 → 16 → CPU`（§5.4 数据支撑：24/16 层仍 ≤8s 但长尾风险升高，因此优先 CPU、把 ngl 降档作为高级/手动选项）。

### 9.8 进程管理（顺带修复的既有欠账）

- `platform/process.rs` 的 `EngineSupervisor` 真实实现：端口探测 39281–39289 → 启动 → `/health` 轮询（超时 20s）→ 空闲 `idleUnloadSec` 卸载 → 退出/崩溃时 **按 PID 进程树** `taskkill /T /F`。
- 严禁按镜像名 kill（本研究误伤实证，见 §2.3 与 R11）。
- 启动参数中注入 device/ngl/backend；把实际 `-ngl`、后端、显存预算写进 `engine:status` 便于诊断。

### 9.9 打包与分发（体积实测）

| 方案 | 新增体积（实测） | 适用 | 建议 |
| --- | --- | --- | --- |
| 仅 CPU | CPU 后端目录 16.1 MB | 所有机型兜底 | 现有安装包（S-18 门禁 ≤25MiB）已含 |
| **Vulkan 后端** | **后端目录 55.8 MB**（ggml-vulkan.dll 39.4 MB；无需额外 runtime；系统 `vulkan-1.dll` 1.6MB 由驱动提供） | NVIDIA/AMD/Intel | **默认 GPU 组件（按需下载）** |
| **CUDA12 后端** | 后端目录 168.5 MB + CUDA runtime 752.4 MB = **≈921 MB**（≈0.9GB） | NVIDIA 用户，追求 prefill/加载略优 | **可选组件（按需下载）**，随包不可行（远超 S-18 的 25MiB 门禁） |
| 模型 | 不随包（S-08 按需下载，0.5B 469MB / 1.5B 1.07GB / 1.8B 1.08GB） | 全部 | 不变 |

- 分发：复用 S-08 的下载/断点续传/SHA256 校验（`resources/models.manifest.json` 同机制，新增 `backends` 清单：`{id, kind: "vulkan"|"cuda", sizeBytes, sha256, urls, minDriver}`）。
- 安装布局建议：`%LOCALAPPDATA%\SnipAsk\backends\llama-vulkan\`、`...\llama-cuda12\`（**CUDA runtime DLL 与 exe 同目录**，实测可行 §3.6）；探测/启动时按需选择。
- 许可：llama.cpp（MIT）；Vulkan loader（Apache-2.0）+ 驱动由厂商提供；CUDA runtime 再分发需遵守 NVIDIA CUDA EULA（附 notice/license），发布前法务确认（见 R5）。
- 驱动门槛：CUDA 12 runtime 需 NVIDIA 驱动 ≥525（本机 595.97 ✅）；Vulkan 需驱动支持 Vulkan 1.1+；启动失败时必须可回退。

### 9.10 需同步的文档/契约

- `docs/接口契约.md` §4.1 默认配置（`:138` 的 local JSON）、`LocalPrefs`（`:172`）、校验规则（`:205`，新增 `local.gpuLayers ∈ [0,999]`）、`config_patch` 生效方式（`:221`，`local.*` 重启引擎，新增「切换设备/后端重启引擎」说明）；
- `resources/models.manifest.json` 增加 `vramMB` 字段（`resources/models.manifest.json` 4 个模型条目）；
- `docs/技术验证清单.md` S-05/S-13 记录 GPU 数据与「显存降档」新验收项（**本报告只提供证据，不改文档**；由 Lead 合并到可行性报告）。

---

## 10 工作量估算与 MVP 路径

### 10.1 分层估算（人日，1 人；含自测）

| 层 | 内容 | 人日 |
| --- | --- | --- |
| 配置/契约（Rust） | `LocalDevice`/`gpuLayers`/`gpuBackend`、默认值、`validate`、`migrate_v3_to_v4`、单测（含 v1/v2/v3 迁移） | **1.5** |
| GPU 探测 + 预算判定 + IPC | `--list-devices`/`nvidia-smi`/DXGI 兜底、虚拟适配器过滤、`local_gpu_probe` 命令与 TS 契约、缓存、单测（解析样例） | **2.5** |
| 引擎监督者与回退 | `EngineSupervisor` 真实实现（spawn/health/卸载/进程树清理）、`-ngl`/`--device` 参数、CUDA→Vulkan→CPU 回退、降档、`engine:status` 扩展 | **4.0**（其中 GPU 增量 ≈2.0，其余是 local 引擎既有欠账） |
| 前端设置 | TS 类型/默认值/presets、`TranslateSection` 设备选择+层数+状态+提示、回退提示、试译联动 | **1.5** |
| 打包/分发 | backends 清单 + 下载/校验/解压/目录布局、许可证、体积门禁 CI、升级策略 | **2.5** |
| 测试/验证 | 单测 + 集成（有/无 GPU、缺 DLL、显存不足模拟）+ 真机矩阵（CPU/GPU × 0.5B/1.5B/1.8B）+ 回归 | **2.0** |
| 文档/文案 | 契约、设置文案、错误码、FAQ | **0.5** |
| **合计（GPU 增量）** | | **≈14.5** |

### 10.2 MVP 路径（建议分 3 步）

| 阶段 | 范围 | 人日 | 验收 |
| --- | --- | --- | --- |
| **MVP-1（可先做）** | 配置 v4 + 手动 `device` 开关 + 设置页设备选择 + 手动放置后端（开发环境）+ `-ngl` 显式传参 + 启动失败回退 CPU 提示 | **≈4.5** | 设置切 GPU 后 1.8B 200 字 P95 ≤8s；切回 CPU 正常；无 GPU 机器不崩 |
| **MVP-2** | `auto` 探测（`--list-devices`/nvidia-smi）+ 显存预算 + 失败回退链 + Vulkan 组件按需下载 | **≈+4.5** | 8GB 机器自动选择；显存不足自动 CPU；Vulkan 组件下载安装后可跑 |
| **MVP-3** | CUDA 组件（可选）+ `gpuLayers` 降档 + 长尾治理（max_tokens/重复检测）+ 温耗/插电策略 + 灰度 | **≈+5.5** | NVIDIA 用户可用 CUDA；异常场景有可操作提示；发布体积/许可合规 |
| 可选增强 | 仅插电用 GPU、连续任务节流、AMD/Intel 实机验证 | +1.5–2.5 | — |

---

## 11 复跑命令

> 全部脚本在 `.verify/research/gpu/`，只读复用 `scripts/spikes/s05_s06/bench_local_translate.mjs`，**不改仓库代码**。PowerShell 5.1 环境（脚本为 ASCII-only，避免 5.1 按 ANSI 读中文注释的问题）。

```powershell
# 0) 前置：确认 GPU 空闲、无本 harness 残留
nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv
@(Get-Process llama-server -ErrorAction SilentlyContinue).Count   # 期望 0

# 1) 跑完整矩阵（分 4 批；每批约 2–6 分钟；本机总耗时约 15–20 分钟）
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .verify/research/gpu/run_gpu_only.ps1     # CUDA+Vulkan 11 组 + 显存探针
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .verify/research/gpu/run_cpu_only.ps1     # CPU 抽测 2 组
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .verify/research/gpu/run_clean_extra.ps1  # 干净窗口 20 次 1.8B + ngl16
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .verify/research/gpu/run_ngl_sweep.ps1    # 部分 offload ngl8/24

# 2) 汇总（生成 summary.json / report_stats.json）
node .verify/research/gpu/analyze.mjs
node .verify/research/gpu/stats.mjs

# 3) 只跑某组（耗时最短）
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .verify\research\gpu\run_matrix.ps1 -Only cuda-05b-product
```

```powershell
# 直接调用原基准脚本（等价命令，CUDA 需 PATH）
$env:PATH = "$env:USERPROFILE\.lmstudio\extensions\backends\vendor\win-llama-cuda12-vendor-v2;$env:PATH"
$cuda = "$env:USERPROFILE\.lmstudio\extensions\backends\llama.cpp-win-x86_64-nvidia-cuda12-avx2-2.67.0\llama-server.exe"
$vulk = "$env:USERPROFILE\.lmstudio\extensions\backends\llama.cpp-win-x86_64-vulkan-avx2-2.28.2\llama-server.exe"
$m18  = "$PWD\.verify\spikes\S-05-S-06\artifacts\HY-MT1.5-1.8B-Q4_K_M.gguf"

node scripts/spikes/s05_s06/bench_local_translate.mjs --exe $cuda --model $m18 `
  --gpu-layers 99 --threads 16 --runs 20 --warmup 2 --prompt-variant product `
  --out .verify\research\gpu\raw\bench_manual_cuda.json

# GPU 探测 / 失败复现 / 打包布局验证
& $cuda --list-devices                      # 需 PATH；期望 CUDA0: ... 8187 MiB, 7106 MiB free
# 去掉 vendor PATH 后：& $cuda --list-devices  → exit -1073741515 (0xC0000135 STATUS_DLL_NOT_FOUND)
nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu,temperature.gpu,power.draw --format=csv
```

> 复跑前建议：关闭游戏/浏览器释放显存；确认 CPU 安静窗口（否则 CPU 组会偏高、GPU 组 P95 抖动）；每组结束后 `Get-Process llama-server` 应为 0。

---

## 12 数据文件清单（本目录）

| 文件 | 内容 |
| --- | --- |
| `REPORT.md` | 本报告 |
| `summary.json` | 全部用例汇总：P50/P95/max、tok/s、prefill/decode、加载时间、失败数、VRAM 峰值/增量、GPU 利用率、温度/功耗、与 S-05 CPU 参考对照 |
| `report_stats.json` | 报告统计（长尾拆分、正常请求分布、CPU 负载、并行 llama-server、残留） |
| `raw/env.json` | 环境快照（OS/CPU/RAM/GPU/驱动/后端版本/空载基线/虚拟显示适配器） |
| `raw/run_manifest.json` | 最近一轮执行清单（用例 + 起止 + 基线） |
| `raw/bench_<case>.json` | **原始逐次测量**（每次请求 total/ttfb/prompt/decode/tok/completion_tokens/输出文本 sha256） |
| `raw/wrapper_<case>.json` | 每组起止时间、CPU 负载、并行 llama-server、残留断言、基线 |
| `raw/gpu_<case>.csv` | nvidia-smi 500ms 采样（时间戳/已用/总量/util/温度/功耗/P-state） |
| `raw/vram_probe.json` | 「只加载不推理」显存探针（CUDA/Vulkan × 0.5B/1.5B/1.8B/ngl16/ngl8/ngl24） |
| `logs/llama_server_<case>.log` | 每组 llama-server 日志（含逐次 timing） |
| `logs/bench_<case>.log` | bench 客户端控制台输出 |
| `logs/cuda_nopath_*.txt` | **CUDA 缺 runtime DLL 的 0xC0000135 证据** |
| `logs/cuda_help*.txt`、`logs/vulkan_help.txt` | `--device/-ngl/--split-mode/--fit` 等参数支持证据 |
| `logs/packtest_*.txt` | 「DLL 放 exe 同目录、不靠 PATH」打包验证 |
| `run_matrix.ps1` / `sample_gpu.ps1` / `run_gpu_only.ps1` / `run_cpu_only.ps1` / `run_clean_extra.ps1` / `run_ngl_sweep.ps1` | 执行器（复用原基准脚本，未改仓库代码） |
| `analyze.mjs` / `stats.mjs` | 汇总/统计脚本 |

---

## 13 局限与未验证（诚实边界）

1. **非流式基准**：`bench_local_translate.mjs` 用 `stream:false`，`ttfb` 等于总耗时，**TTFT 未直接测**；用服务端 `prefill_ms` 作首字下界（§4.5），建议流式实现后补测「首 token 时延」。
2. **后端版本不一致**：CUDA 为 llama.cpp **2.67.0**、Vulkan/CPU 为 **2.28.2**（本机 LM Studio 安装所致），CUDA vs Vulkan 不是严格同版本 A/B；产品若用官方 release 需复验。
3. **样本量**：非 1.8B 组 N=10；1.8B 主口径 N=20（干净窗口）。长尾（重复生成）随机性强，P95 估计仍有波动（CUDA 3/20、Vulkan 9/20），建议产品侧以「正常请求分布 + 长尾防护」双口径验收。
4. **共享机干扰**：首轮 GPU 用例与 overlay agent 的 OCR/短命 CPU llama-server 并行（不占显存、抢 CPU）；报告主口径用干净窗口，但**绝对数值仍受本机桌面常驻（2.2GB 显存、背景 CPU）影响**，换机/换环境需重测。
5. **未模拟真实抢占**：未在「游戏/浏览器占 4–7GB 显存」下实测降级行为（只有预算推断）；未复现 WDDM 共享内存溢出；建议用 `--fit-device-limit` 或真实游戏窗口复验。
6. **未覆盖的机型/系统**：无 8GB 内存真机（本机 16GB）、无 Win10、无 AMD/Intel 显卡、无核显/独显混合本（Optimus）切换场景、无 4K/多屏叠加显存压力。
7. **无长时间稳定性/温控曲线**：未做 30 分钟连续翻译、未测电池模式、未测睡眠唤醒后 GPU 状态；未测驱动升级后的回归。
8. **未测流式 + 取消**：请求中断（用户取消/关闭结果窗）时 GPU 释放、与 `idleUnloadSec` 的配合未验证。
9. **未覆盖 Ollama/CT2 引擎**：本设计只覆盖 llama.cpp 内置后端；Ollama 走自管服务（GPU 参数由 Ollama 管理），CTranslate2 是否用 CUDA 另议。
10. **误杀事故**：第一版 harness 按镜像名清理误杀其它 agent 的 CPU llama-server 两次（已修正并在 §2.3/R11 记录）；不影响本报告时延数据，但作为产品进程管理的反面教材。
