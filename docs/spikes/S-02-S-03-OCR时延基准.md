# S-02 / S-03 OCR 时延基准（整屏 det · 选区 det+rec）

| 项 | 内容 |
| --- | --- |
| 编号 | **S-02 / S-03**（共享任务 `task-5`） |
| 结论 | **S-02：DONE**（4K 双屏整屏 det，P95=926ms ≤ 1.2s）<br>**S-03：FAILED**（选区 det+rec 100 次 P95=968ms > 800ms）；**缓存复用路径 DONE**（P95=596ms） |
| 日期 | 2026-11-04（测量窗口 2026-11-05 00:08–00:15 +08:00） |
| 负责人 | ocr-bench（P0 验证智能体·后端测量） |
| 环境 | Windows 11 25H2（build 26200）/ i7-12650H（10C16T）/ 15.78 GiB / 无独显参与 / 主屏 2560x1440@150% + 副屏 1080x1920@100% |
| 引擎 | rapidocr-onnxruntime 1.4.4 自带 **PP-OCRv4** det/rec ONNX + ONNX Runtime 1.31.1（**CPUExecutionProvider**） |
| 原始数据 | `.verify/spikes/S-02-S-03/results/`（CSV/JSON，本机可复跑产物）；汇总 [summary.json](data/s02_s03/summary.json) |
| 复跑 | `python scripts/spikes/s02_s03/bench_ocr.py all`（详见 §12） |

---

## 1 结论速览

| 条目 | 口径（与清单一致） | 阈值 | 实测 P95 | 实测 max | 结论 |
| --- | --- | --- | --- | --- | --- |
| **S-02** | 4K 双屏冻结帧 **7680x2160**，只做 det + 降采样（长边 → detMaxEdge=3200），冻结帧就绪 → det 完成，30 次 | ≤ 1200ms | **926ms** | 994ms | **DONE**（余量 274ms） |
| S-02 对照 | 单屏 4K 3840x2160（→ 3200x1800），30 次 | — | 1695ms | 1739ms | **超出 1.2s**（重要发现，见 §5.2/§10.1） |
| S-02 对照 | 主屏 2560x1440（不降采样），30 次 | — | 979ms | 1029ms | 达标但余量仅 ~220ms |
| **S-03** | 选区 **det+rec**，5 张典型选区循环共 100 次 | ≤ 800ms | **968ms** | 1026ms | **FAILED**（超 168ms）；det 缓存复用 rec-only 596ms 达标，**需架构/阈值决策** |
| S-03 优化路径 | 整屏 det 缓存 → 选区只过滤 + **rec-only**，100 次 | ≤ 800ms | **596ms** | 639ms | **DONE**（架构既定路径） |
| 不阻塞交互 | worker 线程跑整屏 det 时主线程 60Hz 定时器（3600 帧） | — | 抖动 max 3.42ms / 0 帧掉帧 | — | **可行**（Python 量级参考，见 §7/§9） |

> 一句话：**整屏 det 在 4K 双屏降采样到 3200x900 后达标（926ms）**；但 `detMaxEdge` 按长边限制会让**单 4K 屏**反而更慢（1695ms）；**选区 det+rec 超阈值，必须走「整屏 det 缓存 + rec-only」路径（596ms 达标）**。

---

## 2 环境

| 项 | 值 |
| --- | --- |
| OS | Windows 11 25H2，build 26200（`platform` 报 `Windows-10-10.0.26200-SP0`；注册表 `ProductName` 显示 "Windows 10 Pro 25H2" 是 Win11 的兼容字段） |
| CPU | 12th Gen Intel(R) Core(TM) i7-12650H，6P+4E = 10 物理核 / 16 逻辑核 |
| 内存 | 15.78 GiB 总；测量开始时空闲 9.44 GiB |
| 显卡 | 无独显参与；ONNX Runtime 实测 `get_providers() = ['CPUExecutionProvider']` |
| 屏幕 | 主屏 2560x1440 @150%（物理像素），副屏 1080x1920 @100%（竖屏）；虚拟桌面物理像素原点 (-1080,-470)，尺寸 3640x1920 |
| Python | CPython 3.11.3（`.verify/venv-ocr`） |
| ONNX Runtime | **1.31.1**（pip `onnxruntime`，CPU） |
| OCR 包/模型 | `rapidocr-onnxruntime==1.4.4`，包内自带 PP-OCRv4 模型（见下），无需另下 ModelScope 模型 |
| 线程配置 | ORT `intra_op_num_threads=8`（主口径），`inter_op_num_threads=-1`（ORT 默认）；线程扫描见 §5.3 |
| 电源方案 | 平衡（GUID `381b4222-f694-41f0-9685-ff5bb260df2e`） |
| 构建 commit | `a9b352ed45855cd872b72223b914f57bc711781e`（测量期间工作区有其它成员的未提交改动，与本 OCR 基准路径无关） |

模型文件（来自 pip 包 `...\site-packages\rapidocr_onnxruntime\models\`）：

| 文件 | 大小（字节） | SHA-256 |
| --- | --- | --- |
| `ch_PP-OCRv4_det_infer.onnx` | 4,745,517 | `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9` |
| `ch_PP-OCRv4_rec_infer.onnx` | 10,857,958 | `48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b` |

推理真实性的自证：引擎对真实桌面截图输出了真实识别文本（详见 §6.4，例如 `DeepSeek-V4.1-Flash High`、`根据文档创建Git 仓库`、`1轮40步：198tok/s`），说明数据来自真实 ONNX 推理而非构造。

---

## 3 输入构造（真实桌面截图 → 4K / 双 4K / 选区）

本机无 4K 屏，4K 输入由**真实桌面截图**构造，所有构造方法、尺寸、sha256 记录在
`.verify/spikes/S-02-S-03/inputs/inputs_manifest.json`，脚本 [make_inputs.py](../../scripts/spikes/s02_s03/make_inputs.py)。

1. **真实截图**：`scripts/spikes/tools/screenshot.ps1`（PerMonitorV2 DPI 感知、BitBlt）输出整个虚拟桌面物理像素
   `desktop_full.png` 3640x1920（sha256 `4e29df56c04f51e3…`，2,154,303 B）。
2. **主屏真实裁剪** `screen_primary_2560x1440.png`：按主屏物理矩形 (0,0,2560,1440) 从虚拟桌面裁剪。
3. **单屏 4K** `screen_4k_3840x2160.png`：主屏内容 2560x1440 经 **1.5x LANCZOS** 放大到 3840x2160。
   这等价于「同一桌面在 4K 屏 @150% 缩放下的物理像素渲染」：4K@150% 时 UI 文本物理高度恰为逻辑尺寸的 1.5 倍，
   与 1.5x 放大后的文本物理尺寸一致；差异仅在于放大后的文字边缘比原生 4K 渲染更糊（不影响时延，见 §9）。
4. **4K 双屏冻结帧** `dual_4k_7680x2160.png`：两块 3840x2160 横向拼接（左/右均为上述 4K 内容），
   模拟 4K+4K 双屏冻结帧的物理像素尺寸 **7680x2160**。两屏同源（内容一致）是刻意的最坏情况：两屏都文字密集，
   det 后处理的框数翻倍。
5. **降采样对照**：`dual_4k_det_3200x900.png`（detMaxEdge=3200 的等价输入）、`dual_4k_det_1920x540.png`。
6. **选区**（从 4K 单屏裁剪，代表 4K 屏上框选后送 OCR 的物理像素区域）：
   `sel_1200x800_dense`（正文/侧栏混合）、`sel_800x600_dense_small`（消息气泡区）、`sel_1200x800_sidebar`（会话列表短文本）、
   `sel_800x600_sparse`（菜单栏附近）、`sel_800x600_sparse_blank`（几乎空白），共 5 张。

输入统计：

| 输入 | 尺寸（物理像素） | 用途 |
| --- | --- | --- |
| `desktop_full.png` | 3640x1920 | 真实桌面全虚拟桌面（源） |
| `screen_primary_2560x1440.png` | 2560x1440 | S-02 对照（不降采样，62 框） |
| `screen_4k_3840x2160.png` | 3840x2160 | S-02 对照（→3200x1800，57 框） |
| `dual_4k_7680x2160.png` | 7680x2160 | **S-02 主用例**（→3200x900，113 框） |
| `sel_1200x800_dense.png` | 1200x800 | S-03（7 框，长行正文） |
| `sel_1200x800_sidebar.png` | 1200x800 | S-03（14 框，短文本多行） |
| `sel_800x600_dense_small.png` | 800x600 | S-03（5 框） |
| `sel_800x600_sparse.png` | 800x600 | S-03（11 框） |
| `sel_800x600_sparse_blank.png` | 800x600 | S-03（3 框，近空白） |

---

## 4 方法

### 4.1 引擎与配置

- det/rec 直接用 RapidOCR 包的 `TextDetector` / `TextRecognizer`（复用其官方预处理与 DB/C TC 后处理），
  不重新实现预处理，避免与生产 RapidOCR 语义漂移；模型输入为 BGR uint8（与 PaddleOCR 训练口径一致）。
- 与产品 `config.json` 对齐：`detMaxEdge=3200`、`textThreshold=0.6`（作为 det `box_thresh`）、
  整屏只做 det、选区做 det+rec、**不做 cls 方向分类**（架构写明 det+rec；屏幕文本基本水平）。
- ORT 会话参数：CPU EP、8 intra 线程、`enable_cpu_mem_arena=false`、`graph_optimization_level=ORT_ENABLE_ALL`（RapidOCR 默认）。
- 冷启动额外记录：det 会话 285ms + rec 会话 254ms（≈540ms，仅首次；不计入阈值的稳态时延）。

### 4.2 计时点与统计

- 全部用 `time.perf_counter()`，单位毫秒。
- **S-02**：冻结帧（native HxWx3，RGB→BGR 已就绪）→ ① 降采样（`cv2.resize INTER_AREA`，长边 → 3200）
  → ② det（含 det 内部 mult-of-32 resize、normalize、ONNX 推理、DB 后处理）。**可比耗时 = ①+②**
  （`total_ms`）；截图 BitBlt 不在内（生产由原生代码在冻结阶段完成，故本基准只计 OCR 阶段）。
- **S-03**：选区（native）→ det → 轴对齐裁框 → rec（batch=6，RapidOCR 默认）。记录 det/裁剪/rec 拆分与 total。
- warmup 每次配置先跑 **3 次**且不计入统计；正式样本 S-02 每用例 **30 次**、S-03 **100 次**（5 张选区循环）。
- 统计：P50/P95/max/mean/min/std 全部来自原始样本（不做平滑、不剔除离群）；P95 用 `numpy.percentile` 线性插值。

### 4.3 测量纪律（独占 + 负载证据）

- 测量前按团队约定**独占创建** `.verify/bench.lock`（owner=ocr-bench、pid、startedAt）。
  实测 00:08:42 拿到锁（ext-bench 的 S-05/S-06 测量 23:56–00:08 先完成并释放），00:14:5x 测完自动删除。
- 前后负载快照 + 3 秒粒度监视器：`.verify/spikes/S-02-S-03/results/load_monitor.csv`、`postload_snapshot.txt`、
  `preload_snapshot.txt`。时间线：

| 时间 | CPU 总占用 | llama-server / cargo / rustc | 说明 |
| --- | --- | --- | --- |
| 00:08:24–00:08:41（测量前） | **2%–15%** | 0 / 0 / 0 | 安静窗口达标 |
| 00:08:42 | 建锁 | 0 / 0 / 0 | owner=ocr-bench pid=30544 |
| 00:08:44–00:14:36（测量中） | 60%–93% | 0 / 0 / 0 | **负载来自本基准自身**（30+100+…次真实 ONNX 推理） |
| 00:14:54（测量后） | 20% | 0 / 0 / 0 | 锁已删除 |

- 作废数据说明：23:55 的 S-03 预跑（当时 CPU 100% + 别的成员的 `llama-server` 在跑）**只用于验证脚本代码路径，不进入本报告任何统计**。
- 安静窗口内未执行 pip 安装/下载/编译。

---

## 5 S-02 整屏 det 结果

### 5.1 主用例：4K 双屏冻结帧 7680x2160（detMaxEdge=3200，30 次）

| 量 | mean | P50 | P95 | max | min | std |
| --- | --- | --- | --- | --- | --- | --- |
| 降采样 7680x2160 → 3200x900 | 10.0 | 10.0 | 11.6 | 11.8 | 7.0 | 1.2 |
| det（3200x900 输入，含预处理+推理+后处理） | 741 | 719 | 913 | 983 | 669 | 76 |
| **可比总耗时（降采样+det）** | **754** | **730** | **926** | **994** | **683** | **76** |

- 降采样比例 2.4x（长边 7680→3200）；det 实际输入 3200x900；检测框 **113 个**（两屏各 56–57）。
- **结论：P95=926ms ≤ 1200ms → DONE**，余量 274ms（P95）/ 206ms（max）。

### 5.2 对照组（30 次）

| 用例 | 冻结帧 | det 输入 | 检测框 | 降采样 | 可比总耗时 P50 | P95 | max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **4K 双屏（主用例）** | 7680x2160 | 3200x900（2.88 Mpx） | 113 | 2.4x | 730ms | **926ms** | 994ms |
| 单屏 4K | 3840x2160 | 3200x1800（5.76 Mpx） | 57 | 1.2x | 1567ms | **1695ms** | 1739ms |
| 主屏 2560x1440 | 2560x1440 | 2560x1440（3.69 Mpx） | 62 | 1.0x | 930ms | **979ms** | 1029ms |

**重要发现（实施边界）**：`detMaxEdge` 限制的是「整帧长边」，因此 **det 计算量取决于宽高比**，而不是屏幕数量：
双 4K（3.56:1）被压到 3200x900 = 2.88 Mpx；单 4K（1.78:1）却是 3200x1800 = **5.76 Mpx（2 倍）**。结果是
**单 4K 屏反而比 4K 双屏更慢、且超 1.2s 阈值**（1695ms）。这直接影响「detMaxEdge 按长边」这一配置语义——
**实施时必须按最长边 3200 在单 4K 下重新评估：单 4K 建议 detMaxEdge≈2400–2800（或按屏分块 det）**，处置见 §10.1。

> 质量旁证（框数代理指标，无 GT）：同一屏内容在「双 4K 的 1600x900/屏」下检出 56.5 框/屏，
> 在「单 4K 的 3200x1800」下检出 57 框/屏，差 −1%；detMaxEdge 提到 4000（每屏 2000x1125）时框数 119（+5%），
> 提到 4800 时 116（+2.7%）。即 **在当前输入上，把 3200 再往上提对「检出框数」几乎没有收益**（见 §5.4）。

### 5.3 ORT 线程数扫描（双 4K det，每档 8 次，含 3 次 warmup）

| intra_op 线程 | P50 | P95 | max |
| --- | --- | --- | --- |
| 4 | 822ms | 1013ms | 1075ms |
| **6** | **767ms** | **952ms** | 971ms |
| **8（主口径）** | 771ms | 988ms | 1004ms |
| 12 | 861ms | 1024ms | 1078ms |
| 16 | 1166ms | 1381ms | 1412ms |

**6–8 线程最优；16 线程（=逻辑核数）因 P/E 核超订阅明显退化（+50% 耗时）**。生产 Rust 侧不要用「逻辑核数」当默认线程数。

### 5.4 detMaxEdge 扫描（双 4K，每档 6 次）

| detMaxEdge | det 输入 | 像素量 | 检测框 | P50 | P95 |
| --- | --- | --- | --- | --- | --- |
| **3200（当前配置）** | 3200x900 | 2.88 Mpx | 113 | **770ms** | **824ms** |
| 4000 | 4000x1125 | 4.50 Mpx | 119 | 1210ms | 1277ms |
| 4800 | 4800x1350 | 6.48 Mpx | 116 | 1862ms | 2072ms |

- 提高到 4000/4800 只多 5% / 2.7% 框，却多 57% / 142% 耗时（P50 已超 1.2s 阈值），**不划算**。
- 本机 det 吞吐约 **3.7–4.0 Mpx/s**（由 2.88 Mpx≈720ms、5.76 Mpx≈1554ms 反推），这是外推低配机型的依据（§10.5）。

---

## 6 S-03 选区 det+rec 结果

### 6.1 汇总（5 张选区循环共 100 次，threads=8）

| 量 | mean | P50 | P95 | max | min | std |
| --- | --- | --- | --- | --- | --- | --- |
| det | 187 | 180 | 254 | 291 | 131 | 38 |
| 裁框（crop） | 0.16 | 0.14 | 0.34 | 1.19 | 0.05 | 0.14 |
| rec | 257 | 188 | 693 | 725 | 35 | 218 |
| **总（det+rec）** | **449** | **386** | **968** | **1026** | 173 | 249 |

**结论：P95=968ms > 800ms → FAILED（超 168ms）**。rec 是主要成本（P95 693ms），长行文字把 P95 拉高。

### 6.2 分选区（各 20 次）

| 选区 | 框数 | det P95 | rec P95 | 总 P50 | 总 P95 | max |
| --- | --- | --- | --- | --- | --- | --- |
| `sel_1200x800_dense`（长行正文 7 框） | 7 | 273ms | **725ms** | 895ms | **982ms** | 1026ms |
| `sel_1200x800_sidebar`（短文本 14 框） | 14 | 239ms | 295ms | 444ms | 542ms | 582ms |
| `sel_800x600_dense_small`（5 框） | 5 | 197ms | 295ms | 385ms | 468ms | 477ms |
| `sel_800x600_sparse`（11 框） | 11 | 202ms | 195ms | 278ms | 368ms | 595ms |
| `sel_800x600_sparse_blank`（3 框） | 3 | 185ms | 85ms | 199ms | 277ms | 421ms |

- 最差是 **1200x800 文字密集长行**（rec 单次 ~660ms：7 行被 padding 到同一 batch 宽度约 800px，PP-OCRv4 rec 是宽度敏感的）。
- 注意 800x600 的 det 存在**固定开销**：RapidOCR `limit_type=min, limit_side_len=736` 会把短边<736 的选区**上采样**到 992x736（0.73 Mpx，比原始 0.48 Mpx 多 50% 像素），这是小选区 det 仍要 ~160ms 的原因。

### 6.3 优化路径：整屏 det 缓存 → 选区只过滤 + rec（100 次）

| 量 | mean | P50 | P95 | max |
| --- | --- | --- | --- | --- |
| rec-only 总（含裁框） | 194 | 131 | **596** | 639 |
| 其中 rec | 190 | 128 | 587 | 631 |

- 分选区 P95：`dense` 624ms（长行仍最高）、`sidebar` 142ms、`dense_small` 174ms、`sparse` 96ms、`sparse_blank` 37ms。
- 去掉选区 det 后 **P95 596ms ≤ 800ms → 该路径 DONE**；这正是 `docs/概要设计说明书.md` §缓存策略
  「`ocr_session_id` 内缓存整屏检测结果，用户重选只重过滤、不重复推理」的落地口径。
- 若必须走「选区 det+rec」（首建会话/缓存失效），需优化（§10.3）。

### 6.4 真实识别文本样本（推理真实性证据）

| 选区 | 识别样本（截断） |
| --- | --- |
| `sel_1200x800_dense` | `>深度求索中，用时12分57秒…`；`正在更新计划·todo_write`；`验证台已端到端跑通（便携应用隔离+WebView2CDP+真实IP`；`>执行了命令，已调用工具，已写入文件等` |
| `sel_1200x800_sidebar` | `根据文档创建Git 仓库`；`54分钟`；`50分钟`；`根据文档搭建基础架构`；`框选AIPO技术验证` |
| `sel_800x600_sparse` | `更多`；`自动化任务`；`插件`；`新会话` |

（文本来自本机真实桌面截图，仅作「真实 ONNX 推理」证据，未外发。）

---

## 7 不阻塞选框交互（60Hz 定时器抖动）

方法：worker 线程循环跑「双 4K 整屏 det（降采样+det）」，主线程用忙等 60Hz 定时器（16.667ms 周期）记录 3600 帧间隔；
同时记录无后台的基线。结果：

| 场景 | 帧数 | interval P50 | interval max | 绝对抖动 mean | 抖动 P95 | 抖动 max | >2×周期帧数 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 基线（无后台） | 120 | 16.667ms | 16.747ms | 0.005ms | 0.036ms | 0.08ms | 0 |
| 后台跑整屏 det | 3600 | 16.667ms | 20.083ms | 0.014ms | 0.020ms | **3.42ms** | **0** |

- 主线程 3600 帧无一帧掉帧（>2×周期），最大绝对抖动 3.42ms → **后台 det 不阻塞选框交互在「主线程节奏」上成立**。
- 该测试为 Python 线程模拟（生产是 Rust 线程、无 GIL），只作量级参考；且本次测到 worker 侧 det 单次耗时出现
  12–13s 的极端离群（见 §9「Python 模拟的 artifact」），因此**生产耗时请以 §5 的独立测量为准**，本节只回答「主线程是否被卡」。

---

## 8 阈值对比与结论（按 `docs/技术验证清单.md` §4 模板）

```text
编号：S-02
结论：DONE
日期：2026-11-04
环境：Windows 11 25H2 26200 / i7-12650H 10C16T / 15.78GB / 无独显 / 2560x1440@150%+1080x1920@100% / commit a9b352e
步骤：真实桌面截图 → 构造 4K 单屏与 4K 双屏冻结帧 7680x2160 → 降采样到 detMaxEdge=3200（3200x900）→ PP-OCRv4 det
      （ORT 1.31.1 CPU EP，intra_op=8）30 次，warmup 3 次不计入，perf_counter 打点（降采样+det 分别计时）
原始数据：total P50=730ms P95=926ms max=994ms（resize P95=11.6ms + det P95=913ms）；检测框 113；
          .verify/spikes/S-02-S-03/results/s02_raw.csv、s02_det.json；对照：单 4K P95=1695ms、2560x1440 P95=979ms
结果：P95=926ms ≤ 1200ms 达标（余量 274ms）；测量窗口无其它重负载（前 CPU 2–15%、llama/cargo/rustc=0）
处置：保持 detMaxEdge=3200（双 4K 达标）；但需改「长边」语义为「像素预算」以覆盖单 4K 屏（见 §10.1）
遗留：真实 4K 双屏实机复测、低配机型（i5/16GB）复测、INT8 模型评估
```

```text
编号：S-03
结论：FAILED（strict：选区 det+rec，100 次 P95=968ms > 800ms）；
      det 缓存复用 rec-only P95=596ms 达标 —— 需架构/阈值决策（是否强制走缓存路径）
日期：2026-11-04
环境：同 S-02（ORT 1.31.1 CPU EP，intra_op=8，rec batch=6，无 cls）
步骤：5 张典型选区（800x600 / 1200x800，疏密各异）循环 100 次，每次 = 选区 det → 裁框 → PP-OCRv4 rec；
      warmup 3 次不计入；另测「整屏 det 缓存 → 选区只过滤 + rec」对照 100 次
原始数据：det+rec total P50=386ms P95=968ms max=1026ms（det P95=254ms、rec P95=693ms）；
          rec-only total P50=131ms P95=596ms max=639ms；.verify/.../s03_raw.csv、s03_det_rec.json
结果：det+rec 口径超阈值 168ms（最差：1200x800 文字密集长行 P95=982ms）；rec-only 缓存口径 596ms 达标
处置：生产按架构走「整屏 det 缓存 + 选区 rec-only」（DONE）；若必须选区 det+rec，按 §10.3 优化后复测
遗留：rec 长行切分/批量调优落地、真实生产 Rust 实现端到端打点复测
```

---

## 9 局限

1. **CPU 与生产 Rust 实现的差异**：det 的 ONNX 推理本身与生产同（ORT C++ CPU EP），但本基准的 DB 后处理、归一化、
   裁框走的是 RapidOCR 的 **Python** 实现（pyclipper/shapely/numpy），比 Rust 侧通常更慢 → **本报告数字偏保守**；
   生产实现预计略快，但不应指望数量级差异。
2. **输入是合成 4K**：4K 内容由 2560x1440 真实截图 1.5x LANCZOS 放大（等效 4K@150% 的物理像素文本大小），
   文字边缘比原生 4K 渲染更糊。时延只取决于像素量与框数，**不受模糊影响**；但 S-03 的 rec 准确率不能据此下结论（本报告不评准确率）。
3. **框数为质量代理指标**：本基准没有 OCR GT，downsample 的「信息丢失」只用检测框数近似（±5% 量级），
   不能替代准确率/召回率评测。
4. **单机、单次窗口**：只有一台机器、一个安静窗口（00:08–00:15），未做跨天/多轮重复；P95 由 30/100 个样本估计，
   尾部（P99）置信度有限。
5. **16 线程 vs 目标低配（i5/16GB）**：本机为 10 核（6P+4E）/16 逻辑，且 ORT 在 6–8 线程时最优；目标低配机
   （如 4P+4E / 8 线程或更老 i5）吞吐大概率更低（保守按 0.5–0.7x 外推），S-02 的 926ms 可能涨到 1.3–1.8s。
   低配机型必须在真实硬件上复测，或按 §10.5 收紧像素预算/换 INT8。
6. **Python 线程模拟的 artifact**：§7 的 60Hz 抖动测试中，主线程忙等 + worker 线程 ORT 推理共存时，worker 侧 det
   单次耗时出现 12–13s 的极端离群（Python GIL/忙等/线程过度订阅的综合 artifact，非 det 真实耗时）；因此该测试
   **只用于回答「主线程是否被卡」**，不能用于评价生产耗时。
7. **其他静默变量**：电源方案为「平衡」（未固定高性能）、测量期间系统后台有本 Harness 等常驻进程（CPU 2–15%），
   未做 CPU 亲和性/频率锁定；这对 P95 有 5% 量级噪声影响。

---

## 10 处置建议

### 10.1 detMaxEdge 语义：从「长边」改为「det 输入像素预算」（优先级最高）

- 现状：双 4K 7680x2160 → 3200x900（2.88 Mpx）达标；**单 4K 3840x2160 → 3200x1800（5.76 Mpx）超阈值 1695ms**。
  同一份配置在单 4K 屏上必失败，这是「按长边限制」的语义缺陷。
- 建议：把 `detMaxEdge` 改为像素预算 `detMaxPixels ≈ 3.0 Mpx`（或 `long_edge = min(3200, sqrt(budget × w/h))`）：
  - 双 4K 7680x2160 → 3200x900（不变，926ms✓）
  - 单 4K 3840x2160 → ≈2310x1300（3.0 Mpx，按本机吞吐 ~750ms✓）——与 Lead 建议的「单 4K 用 detMaxEdge≈2400–2800」一致，
    取 2400–2800 时对应 3.4–4.4 Mpx / 约 850–1100ms，仍建议优先按像素预算而非长边配置
  - 2560x1440 → 2048x1152（2.36 Mpx，~650ms；当前不降采样 979ms，会更从容）
- 兜底：若实现层暂不改语义，则至少对「单 4K 屏」做分块 det（如按左右两块各 1920x2160 → 长边 1920，合计像素与双 4K 同级），
  或按屏分别 det；两种方式的 det 总像素都接近 2.9–3.0 Mpx，仍能守住 1.2s。
- 兼容性：对双 4K 行为不变；对单 4K/2K 屏是纯收益。**建议作为实现阶段修改点**（本验证不改应用代码）。

### 10.2 ORT 线程数

- 整屏 det 用 **intra_op=6–8**（本机 6 线程 P50 最优 767ms，16 线程退化到 1166ms）。
- 生产是「后台单 worker」，建议显式设 6–8（不要留 ORT 默认按逻辑核数），并保持 worker 可取消（取消点在 det 前/rec 批次间）。

### 10.3 选区 OCR：必须走 det 缓存路径；兜底路径需优化

- **首选**：按 `docs/概要设计说明书.md`/实施架构的缓存策略——整屏 det 结果按 `ocr_session_id` 缓存，
  用户重选只做「框过滤 + rec」，实测 **P95=596ms ≤ 800ms（DONE）**；配合可取消 worker 即满足 S-03 体验要求。
- **兜底（首次会话无缓存 / 选区超出缓存范围）** 若必须 det+rec，按以下优化后复测（预计可从 968ms 压到 ~700–800ms）：
  1. 选区 det 关闭「短边 <736 上采样」（RapidOCR `limit_type=min` 的固定开销，800x600 被放大到 992x736，可省 ~50–90ms）；
  2. rec 批大小 6 → 12/16，减少 batch 边界；对超长行（宽 >~1000px）先切分再 rec（长行是 P95 的主因：dense 选区 rec 占 664ms）；
  3. 保持 `textThreshold=0.6` 过滤低分框（当前选区框数 3–14，过滤后 rec 量可控）。
- 若优化后仍无法稳定 ≤800ms，则产品上应保证「选区 OCR 永远命中整屏 det 缓存」，并把无缓存场景作为可接受的降级（约 1s 级）。

### 10.4 质量/配置

- 双 4K 下把 detMaxEdge 从 3200 再提高到 4000/4800 对检出框数只有 +5%/+2.7%，但耗时会超阈值 → **保持 3200（或按 §10.1 像素预算）**。
- `textThreshold=0.6` 直接作为 det box_thresh 与生产过滤一致，框数 113（双 4K）合理。

### 10.5 低配机型（目标 i5/16GB 或同级）

- 本机 det 吞吐 ≈3.7–4.0 Mpx/s；若目标机按 0.5–0.7x 外推，则要保住 1.2s 需把 det 输入压到 **≈1.5–2.0 Mpx**
  （双 4K 长边 ≈2300–2600，每屏 ≈1150–1300 px 宽），会牺牲部分小字检出。
- 更优路径：**INT8 量化的 PP-OCRv4 det**（预计 1.5–2x 提速）或更轻的 det 模型；本基准未测，列为遗留项。
- 兜底策略：低配机可把「整屏 det」转为可选的「按需 det」（用户框选后再对选区做 det+rec），或把 S-02 阈值按机型分档。

---

## 11 遗留

| # | 事项 | 优先级 | 备注 |
| --- | --- | --- | --- |
| 1 | 真实 4K 双屏实机复测 S-02（原生 4K 截图，非 1.5x 合成） | P0 | 需 4K+4K 实机 |
| 2 | 低配机型（i5/16GB）复测 S-02/S-03，并按 §10.5 调参 | P0 | 阈值按目标机型验收 |
| 3 | INT8 PP-OCRv4 det/rec 量化模型基准（吞吐/质量） | P1 | 低配机达标更优路径 |
| 4 | rec 长行切分 + batch 调参落地后重测 S-03 兜底路径 | P1 | 目标 P95 ≤800ms |
| 5 | 生产 Rust 实现端到端 `Instant` 打点（冻结完成→`ocr:status(done)`），含截图/事件派发 | P0 | 本基准只覆盖 OCR 阶段 |
| 6 | 缓存失效/跨屏选区等边界用例的时延与正确性 | P2 | 与 S-02/S-03 缓存策略联动 |

---

## 12 复跑命令与数据索引

```powershell
# 0) 环境（一次即可；安静窗口内不要跑安装/下载/编译）
python -m venv .verify/venv-ocr
.verify/venv-ocr/Scripts/python.exe -m pip install onnxruntime rapidocr-onnxruntime opencv-python-headless pillow "numpy<2.3"

# 1) 真实桌面截图（DPI 感知，输出虚拟桌面物理像素 3640x1920）
powershell -ExecutionPolicy Bypass -File scripts/spikes/tools/screenshot.ps1 -Out .verify/spikes/S-02-S-03/inputs/desktop_full.png

# 2) 构造 4K / 双 4K / 降采样 / 选区输入（写入 inputs_manifest.json 含 sha256）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/make_inputs.py

# 3) 冒烟（真实推理自证：打印检测框与识别文本）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py smoke --threads 8

# 4) 正式测量（自动独占 .verify/bench.lock，结束自动释放；约 5 分钟）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py all

# 5) 单项复跑（同样会建锁/释放）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py s02  --reps 30 --threads 8
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py s03  --reps 100 --threads 8
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py jitter --reps 20 --threads 8
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py edge-sweep --reps 6
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py thread-sweep --reps 8
```

| 产物 | 路径 |
| --- | --- |
| 本报告 | `docs/spikes/S-02-S-03-OCR时延基准.md` |
| 汇总数据（报告内嵌数字来源） | [docs/spikes/data/s02_s03/summary.json](data/s02_s03/summary.json) |
| 基准脚本 | [bench_ocr.py](../../scripts/spikes/s02_s03/bench_ocr.py)、[bench_common.py](../../scripts/spikes/s02_s03/bench_common.py) |
| 输入构造脚本 | [make_inputs.py](../../scripts/spikes/s02_s03/make_inputs.py) |
| 原始数据（本机，gitignored） | `.verify/spikes/S-02-S-03/results/`：`s02_raw.csv`（90 行逐次样本）、`s02_det.json`、`s03_raw.csv`（100 行）、`s03_rec_only_raw.csv`、`s03_det_rec.json`、`jitter.json`、`edge_sweep.json`、`thread_sweep.json`、`load_monitor.csv`、`preload_snapshot.txt`、`postload_snapshot.txt`、`bench_run.log` |
| 输入清单（本机，gitignored） | `.verify/spikes/S-02-S-03/inputs/inputs_manifest.json`（尺寸/构造方法/sha256） |
| 完整运行日志 | `.verify/spikes/S-02-S-03/results/bench_run.log` |

---

## 13 实施落地（2026-11-05，S-03 缓存口径 + S-02 像素预算）

> 本节为**追加更新**：用户已确认落地「S-03 缓存口径 + OCR 像素预算」。§8 中 S-03 的「需架构/阈值决策」
> 与 §10.1 的「建议作为实现阶段修改点」在本节转为**已实施**；其余测量数据（§5–§7、§9）不变。

### 13.1 结论变更

| 条目 | 原结论（§8） | 实施后 |
| --- | --- | --- |
| **S-03** 选区 OCR | FAILED（det+rec P95=968ms）；缓存复用路径 DONE（P95=596ms），处置「需架构/阈值决策」 | **已实施 det 缓存 → rec-only（实测 rec-only P95=596ms ≤800ms，处置改为「已实施」）**；无缓存/无命中时回退选区 det+rec（约 1s 级降级，见 §10.3） |
| **S-02** 整屏 det | DONE（双 4K P95=926ms）；单 4K 1695ms 超阈值，处置「改长边语义为像素预算」 | **已采用像素预算 `DET_PIXEL_BUDGET = 3.0 Mpx`**：单 4K 3840x2160 的 det 输入由 3200x1800（5.76 Mpx）收敛为 2309x1299（3.0 Mpx）；按本机 det 吞吐 ≈3.7–4.0 Mpx/s 外推约 **~750ms**（具体数值见 ocr-bench 复核），双 4K 行为不变（3200x900，926ms） |

### 13.2 代码落地

| 位置 | 内容 |
| --- | --- |
| `services/ocr_service.rs` · `det_input_size` | det 输入尺寸**双约束**：长边 ≤ `det_max_edge`（配置 `ocr.detMaxEdge`=3200）**且** 宽×高 ≤ `DET_PIXEL_BUDGET`（3.0 Mpx）；等比缩放、`floor` 取整**绝不放大**；返回 `(w, h, scale)`，框坐标 × `1/scale` 还原为帧局部物理像素（取整误差 ≤1px） |
| `services/ocr_service.rs` · `OcrEngine` | trait 拆为 `detect`（整屏只 det，帧局部物理框）/ `recognize`（只 rec 补文本）/ `detect_and_recognize`（选区兜底 det+rec）；`SkeletonOcrEngine` 三者均实现，占位语义与契约 150/400ms 时序不变（时序由 `commands::capture` 的后台任务控制，引擎不 sleep） |
| `services/ocr_service.rs` · `OcrPipeline` | `scan_screen`：det → 帧局部框 + `frame_origin`（虚拟桌面左上角，可负）平移为绝对物理坐标 → 按 `ocr_session_id` 缓存；`recognize_selection`：缓存命中且有框与选区相交 → **只 rec**（不再 det）；缓存缺失 / 无命中 / 空框 → 回退 `detect_and_recognize`；`invalidate/clear` 支持新会话失效 |
| `app/state.rs` | `AppState.ocr: OcrPipeline`（骨架模式 `with_skeleton()`），跨会话复用同一实例 |
| `commands/capture.rs` | 新会话开始 `state.ocr.clear()`；后台任务在 `ocr:status(scanning)`（150ms）之后做整屏 det 并缓存，`done` 仍按 400ms 契约时序发出（det 更快不提前、更慢不额外等待） |
| `commands/selection.rs` | `selection_commit` 改用 `recognize_selection`（缓存 rec-only 优先，兜底 det+rec）生成 `preview/has_text/boxes`；保留 S-07 能力门禁逻辑 |

坐标口径：缓存框为**绝对物理像素**（虚拟桌面左上角为原点，可负），负坐标副屏选区可直接与缓存框做相交判定；
引擎 `detect/recognize` 使用**帧局部物理像素**，由 pipeline 平移，避免多显示器坐标歧义。

### 13.3 像素预算单测结果（`cargo test`）

| 输入帧（物理） | det_max_edge=3200 | 输出 det 输入 | 像素量 | 断言 |
| --- | --- | --- | --- | --- |
| 2560x1440 | 长边不超 | 2309x1299 | 2,999,391 ≤3.0Mpx | 不放大 ✔ / 预算 ✔ |
| 3840x2160（单 4K） | 3200x1800 会超预算 | **2309x1299** | 2,999,391 ≤3.0Mpx | 由 5.76 Mpx 收敛 ✔ |
| 7680x2160（双 4K） | 3200 | **3200x900** | 2,880,000 ≤3.0Mpx | 与 S-02 主用例一致 ✔ |
| 5120x1440（32:9 超宽） | 3200 | 3200x900 | 2,880,000 | ✔ |
| 2160x3840（9:16 超长） | 3200 后仍超预算 | 1299x2309 | 2,999,391 | 宽<高保持 ✔ |
| 320x200 / 800x600 / 1920x1080 / 1x1 | — | 原尺寸 | — | **绝不放大** ✔ |

覆盖矩阵单测：8 种真实/极端帧 × 3 档 det_max_edge（1600/3200/4800）全部断言「不放大、长边 ≤ 上限、像素 ≤ 3.0Mpx」。

### 13.4 缓存 / 兜底路径覆盖（fake engine + 骨架引擎）

| 路径 | 断言 | 结果 |
| --- | --- | --- |
| 整屏 det 缓存 | 帧局部框按 `frame_origin` 平移为绝对坐标；`detect` 调用 1 次；缓存条目数正确 | ✔ |
| 缓存命中 → rec-only | `detect` 仍为 1 次、`recognize` 1 次、`detect_and_recognize` 0 次；文本由 rec 补齐 | ✔ |
| 过滤边界 | 部分相交（选区不含框中心）也命中；不相交/低置信度框被过滤 | ✔ |
| 兜底路径 | 无缓存 / 缓存为空框 / 过滤后无命中 → `detect_and_recognize` 恰好 1 次，rec 0 次 | ✔ |
| 会话失效 | 旧键缓存不被新键命中（新会话走兜底）；`invalidate` 后缓存为空 | ✔ |
| 取消 / 空框 | 空选区（0 宽）不触发任何推理，返回空 `OcrScan` | ✔ |
| 骨架占位语义 | scan→缓存→rec-only 后仍返回固定占位文案、`hasText=true`（与历史行为一致） | ✔ |

测试数：`ocr_service` 单测 **19 项**（含 6 项 `det_input_size`、7 项 pipeline）+ 集成测试
`src-tauri/tests/ocr_pipeline_spike.rs` **9 项**；合计 28 项与本落地相关，全部通过。

### 13.5 复跑

```powershell
$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"
# 像素预算 + pipeline 单元/集成测试（无需 app、无需模型）
cargo test --manifest-path src-tauri/Cargo.toml --lib ocr
cargo test --manifest-path src-tauri/Cargo.toml --test ocr_pipeline_spike
# 全量
cargo test --manifest-path src-tauri/Cargo.toml
```

> 注：本节的 P95 数值沿用 §6.3 的 rec-only 实测（596ms）与 §5.2 的 det 实测（双 4K 926ms / 单 4K 1695ms）；
> 像素预算对单 4K 的时延改善为**按吞吐外推**，真机 Python 复测见 ocr-bench 的复核结果。

### 13.6 真机冒烟（2026-11-05 00:54，真实 debug 应用，仅验证接线不测时延）

```powershell
node scripts/spikes/applab/applab.mjs start --owner dpi-coords --wait-sec 60
node scripts/spikes/applab/applab.mjs eval --window main --js "window.__TAURI_INTERNALS__.invoke('capture_freeze')"
node scripts/spikes/s01/s01_pointer_probe.mjs --vd -1080,-470,3640,1920 --out .verify/spikes/S-02-S-03/ocr_smoke_pointer.json
node scripts/spikes/applab/applab.mjs stop --owner dpi-coords
```

- `capture_freeze` 成功并在 150ms/400ms 契约时序发出 `ocr:status`；后台整屏 det + 缓存路径无 panic/无 warn。
- 跨屏拖选 6 组提交（走 `selection_commit → OcrPipeline::recognize_selection`）：菜单正常出现
  （`hasMenu=true`），预览为骨架占位文案，`hasText=true` —— 证明新接线（state/capture/selection）在真实应用内可用。
- 证据：`.verify/spikes/S-02-S-03/ocr_smoke_pointer.json`、`ocr_smoke_pointer-overlay.png`（本机 gitignored）。
- 说明：本次只验证接线与契约行为，不产生时延数字；时延以 §5/§6 的 Python 基准与 ocr-bench 复核为准。

---

## 14 像素预算复核（2026-11-05，真实 ONNX 实测·task-12）

> 本节为**追加更新**，位于 §13（实施落地）之后。目的：用真实 PP-OCRv4 ONNX 实测复核 item2 的
> 「OCR 像素预算」口径，为 §13.1 的**外推值**提供实测依据，并给出**推荐预算值**。
> 口径规则（与 §13.2 `det_input_size` 实现一致）：
> `scale = min(长边上限 3200 / max(w,h), sqrt(budget_px / (w×h)))`，`scale ≥ 1` 时不放大。
> 因此：双 4K 7680x2160 → **3200x900（2.88Mpx，仍受长边约束）**；单 4K 3840x2160 → **2309x1299（3.0Mpx）**；
> 2560x1440 → 2309x1299（3.0Mpx）；1080p 及以下不放大。
> 环境/引擎/统计口径与 §2–§4 完全一致（PP-OCRv4 ONNX、ORT 1.31.1 CPU EP、intra_op=8、warmup 3 次不计入、
> `perf_counter` 打点、可比耗时 = 降采样 + det）。

### 14.1 头条用例复核（各 30 次，最终干净窗口）

| 用例 | 帧（物理像素） | det 输入 | P50 | P95 | max | mean | 检测框 | 结论 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **双 4K（主用例）** | 7680x2160 | **3200x900**（2.88Mpx） | **724ms** | **743ms** | 778ms | 726ms | 113 | **DONE**（≤1.2s，余量 457ms；与 §5.2 的 926ms 同量级，本窗口更快） |
| **单 4K（新口径）** | 3840x2160 | **2309x1299**（3.00Mpx） | **753ms** | **794ms** | 837ms | 756ms | 58 | **DONE**（旧口径同窗口 P95=1682ms，**−53%**） |
| 单 4K（旧口径对照，10 次） | 3840x2160 | 3200x1800（5.76Mpx） | 1631ms | 1682ms | 1690ms | — | 57 | **FAILED**（>1.2s；与 §5.2 干净值 1695ms 一致） |
| 主屏 2560x1440（附带） | 2560x1440 | 2309x1299（3.00Mpx） | 752ms | 761ms | 762ms | — | 57 | 达标（旧口径不降采样 979ms，§5.2） |

- 与 §13.1 外推值的关系：§13.1 按吞吐外推「单 4K 约 ~750ms」→ 实测 **P50=753ms**，外推准确；
  P95=794ms（≤1.2s）。双 4K 实测 P95=743ms（§13.1 引用 §5.2 的 926ms 为前一日窗口值；两轮均达标、行为不变）。
- 检测框数：单 4K 新口径 58 框 vs 旧口径 57 框（**无损失**，框数代理指标）。

### 14.2 预算扫描（单 4K，各 20 次；取第 2 轮 cargo=0 的干净段）

| 预算 | det 输入 | 像素量 | P50 | P95 | max | 检测框 |
| --- | --- | --- | --- | --- | --- | --- |
| **2.5 Mpx** | 2108x1186 | 2.500Mpx | **621ms** | **632ms** | 646ms | **59** |
| **3.0 Mpx** | 2309x1299 | 2.999Mpx | 765ms | 800ms | 808ms | 58 |
| **3.5 Mpx** | 2494x1403 | 3.499Mpx | 898ms | 930ms | 957ms | 57 |

- 三档全部 ≤1.2s；框数 59 / 58 / 57 随预算单调但差异仅 ±2（与旧口径 57 相当），**没有数据表明 2.5–3.0Mpx 会丢失小字**（框数为代理指标，非召回率）。
- 3.0 与 2.5 的差异：多保留 20% 像素（对 rec 质量更稳），P95 多 168ms；3.5 比 3.0 慢约 25% 但框数不再增加。

### 14.3 推荐值

| 档位 | 预算 | 依据 |
| --- | --- | --- |
| **默认（推荐）** | **3.0 Mpx** + 长边上限 3200 | 单 4K P95=794ms、双 4K P95=743ms，均 ≤1.2s（余量 406/457ms）；相对旧口径单 4K 从 1682ms → 794ms（−53%）且双 4K 行为不变；比 2.5 多 20% 像素保留小字/rec 质量，比 3.5 快 ~25%，是速度与保真的均衡点；与 §13.2 已实现的 `DET_PIXEL_BUDGET=3.0Mpx` 一致 |
| **低配 profile（建议预留）** | 2.5 Mpx | P95=632ms；按低配机 0.5–0.7x 吞吐外推约 0.9–1.3s，仍有机会守住 1.2s；框数 59（无代理损失）。3.0 在低配机外推 P95≈1.1–1.6s，有超阈值风险 |
| 不推荐 | 3.5 Mpx | P95=930ms 虽仍达标，但比 3.0 慢 ~25% 且框数不再增加（57 vs 58），对低配机余量更小 |

### 14.4 测量窗口与污染处理（如实记录）

| 轮次 | 时间（+08:00） | 负载 | 处理 |
| --- | --- | --- | --- |
| 第 1 轮 | 00:48:24–00:51:2x | 00:49:01 起 hotkey-probe 的 `cargo test --test provider_probe_spike`（cargo 最多 9 + rustc 8）编译，主屏用例 P95 被拉到 3442ms | **全部作废**；备份 `budget_sweep_contaminated.json` / `budget_run_contaminated.log` |
| 第 2 轮 | 00:53:38–00:56:18 | 开头 00:53:48–00:54:22 有 1–3 个 cargo 进程（无 rustc），早期用例可能轻微受影响；00:54:45 之后 cargo=0 | 预算扫描（14.2）与主屏段取此轮 cargo=0 段；单 4K 30 次改用第 3 轮 |
| 第 3 轮 | 00:56:48–00:57:42 | **全程 cargo/rustc/llama-server=0**，CPU 60–77% 为本基准自身推理 | 14.1 两个 30 次头条用例取此轮（`budget_headline_clean.json`） |

- 锁：第 1、2 轮均独占 `.verify/bench.lock`（owner=ocr-bench）；第 3 轮为释放锁后的即时补测（未再建锁，但全程监视器确认无 cargo/rustc，且此时其它成员已收到释放通知、处于写文档状态）。
- 前后负载快照：`preload_snapshot_phase2.txt`（第 2 轮前：CPU 7%、cargo/rustc/llama-server=0）、
  `postload_snapshot_phase2.txt`（第 3 轮后：CPU 5%、无重负载）；3 秒粒度监视器：
  `load_monitor_phase2.csv`、`load_monitor_phase2b.csv`。

### 14.5 数据与复跑

| 产物 | 路径 |
| --- | --- |
| 头条用例原始数据（第 3 轮，30 次） | `.verify/spikes/S-02-S-03/results/budget_headline_clean.json`、`budget_headline_clean_raw.csv`、`budget_headline_clean.log` |
| 全量用例原始数据（第 2 轮） | `.verify/spikes/S-02-S-03/results/budget_sweep.json`、`budget_sweep_raw.csv` |
| 作废轮次备份 | `.verify/spikes/S-02-S-03/results/budget_sweep_contaminated.json`、`budget_sweep_raw_contaminated.csv`、`budget_run_contaminated.log` |
| 汇总数据（含 `budget_sweep` 字段） | [docs/spikes/data/s02_s03/summary.json](data/s02_s03/summary.json) |
| 复跑 | `.verify/venv-ocr/Scripts/python.exe scripts/spikes/s02_s03/bench_ocr.py budget`（自动独占建锁/释放，约 3 分钟；输出含本轮 6 个用例） |
