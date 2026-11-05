# scripts/spikes/s11_s15 — S-11 文字存在判定阈值 / S-15 OCR 字符准确率

对应 `docs/技术验证清单.md` 的 **S-11（P1）** 与 **S-15（P1）**。真实推理使用
`rapidocr-onnxruntime 1.4.4` 包内 PP-OCRv4 det/rec ONNX（与 S-02/S-03 同一套模型），
复用 `../s02_s03/bench_ocr.py` 的引擎加载、像素预算降采样与裁框逻辑，只补充「det 置信度保留」与「评分/评测」。
原始数据写入 `.verify/spikes/S-11-S-15/`（gitignored），聚合结论写入
`docs/spikes/S-11-文字存在判定阈值.md`、`docs/spikes/S-15-OCR字符准确率.md` 与
`docs/spikes/data/s11_s15/`。

## 目录

| 路径 | 用途 |
| --- | --- |
| `build_corpus.py` | 构造 100 张真实渲染截图（headless Edge 自建 HTML，5 类 × 20）+ 50 张合成无文字图；写逐图 manifest / 真值 / DOM 校验 |
| `run_ocr_eval.py` | `det` / `sweep` / `judge` / `ocr` / `all` 子命令：S-11 阈值扫描与 S-15 det+rec+CER |
| `../s02_s03/bench_ocr.py` | 被复用：`OcrEngines`（det/rec 加载、线程、降采样）、`budget_target_size`、`load_bgr` |

## 环境

```powershell
# 一次性（已就绪）：.verify/venv-ocr 内 rapidocr-onnxruntime 1.4.4 / onnxruntime 1.31.1 / opencv / pillow / numpy
.verify/venv-ocr/Scripts/python.exe -m pip install onnxruntime rapidocr-onnxruntime opencv-python-headless pillow "numpy<2.3"

# CER 用 rapidfuzz 做快速编辑距离与删除/插入/替换拆分（未安装时脚本退化为纯 Python DP，结果一致但更慢）
.verify/venv-ocr/Scripts/python.exe -m pip install rapidfuzz
```

headless Edge 路径写死在 `build_corpus.py` 的 `EDGE`：
`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`（Windows 11 自带）。

## 复跑

```powershell
# 0) 小样本冒烟（每类 2 张真实 + 6 张合成；约 15 秒）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py --real-per-cat 2 --synth 6

# 1) 全量语料（100 真实 + 50 合成；约 2.5 分钟，100 次 headless Edge 渲染）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/build_corpus.py

# 2) S-11 阶段 A：100 张逐图整屏 det（生产像素预算口径），缓存每个框的面积与 DB 置信度
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py det

# 3) S-11 阶段 B：面积/置信度/行数三阈值网格扫描 + 推荐组合
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py sweep

# 3b) 阈值参数化验证：直接评估指定组合（可指向任意一组参数）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py judge `
  --min-area 40 --min-confidence 0.5 --min-lines 1

# 4) S-15：100 张真实截图 det+rec，native / budget 两口径 CER（约 3–5 分钟）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py ocr --profiles native,budget

# 5) 一次跑完 + 生成 docs/spikes/data/s11_s15/summary.json
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py all

# 6) 只从已有结果重新生成汇总（不重跑推理）
.verify/venv-ocr/Scripts/python.exe scripts/spikes/s11_s15/run_ocr_eval.py summary
```

> 本任务**不测时延阈值**，不需要 `.verify/bench.lock`；但 OCR 是 CPU 密集操作，避免与
> `llama-server`/`cargo build` 同时满负载（会显著拉长单张耗时，不影响准确率结论）。
> 若机器上有 S-10 空闲采样等安静窗口标记（`.verify/p1/quiet/`），应先让出。

## 输出

| 产物 | 路径 | 内容 |
| --- | --- | --- |
| 语料 manifest | `.verify/spikes/S-11-S-15/corpus/corpus_manifest.json` | 150 张逐图元数据：id/类型/尺寸/真值行/真值文本/DOM→截图一致性校验/sha256 |
| 真实截图 / HTML | `.verify/spikes/S-11-S-15/corpus/real/`、`corpus/html/` | 100 张 PNG + 生成用 HTML（可单独打开复核） |
| 合成负样本 | `.verify/spikes/S-11-S-15/corpus/synth/` | 50 张 PNG（纯色/渐变/噪声/图标/线条/几何/空白 UI） |
| S-11 逐框数据 | `.verify/spikes/S-11-S-15/results/s11_boxes.json` | 每图 det 框（x/y/w/h/面积/置信度）、det 输入尺寸、耗时 |
| S-11 逐图结果 | `.../results/s11_per_image.csv` | 每图框数/最大置信度/最大面积/真值/推荐阈值判定/基线判定 |
| S-11 扫描 | `.../results/s11_sweep_all_combos.csv`、`s11_sweep.json` | 全部组合的混淆矩阵、推荐组合、分类型统计、失败样本、2D 曲线 |
| S-15 逐图结果 | `.../results/s15_per_image.csv`、`s15_ocr.json` | 两口径 × 两 variant 的 CER/准确率/删除/插入/替换/拼接文本 |
| 失败样本（小尺寸） | `.verify/spikes/S-11-S-15/failures/` | S-11 误判样本、S-15 Top 失败截图缩小版 + 关键局部裁剪 + `failure_assets.json` |
| 聚合数据（进 git） | `docs/spikes/data/s11_s15/summary.json`、`corpus_index.json` | 报告内嵌数字来源 + 语料索引 |

## S-11 判定语义

```text
pass(box)  = det_score >= --min-confidence
             且 box.area_px >= --min-area
             且 (可选) box.area_px <= --max-area
             且 (可选) box.height_px <= --max-box-height
hasText    = count(pass) >= --min-lines
```

- `area_px` 是**还原到原图物理像素**后的轴对齐框面积（宽 × 高），`height_px` 为 `y` 方向边长；
  阈值按 **100% 缩放口径**配置，高 DPI 真机应把面积 ÷ `scale_factor²`、行高 ÷ `scale_factor` 后再比较；
- 测量时 det 的 `box_thresh` 降到 `0.05`（只为让低分框进入统计，便于扫描 `--min-confidence`）；
  产品当前 `box_thresh = ocr.textThreshold = 0.6`，其可见框等价于本扫描中 `score>=0.6` 的子集；
- 本轮结论（详见 `docs/spikes/S-11-文字存在判定阈值.md`）：
  - 仅置信度（产品现状）误判 **38%**；只设面积下限的三阈值原义最佳仍 **18%** —— 都不达标；
  - **推荐**：`det≥0.6 + 面积≥400px² + 行高≤70px + 行数≥1` → 误判 **2%**、漏判 **0%**；
  - 备选（纯三阈值，面积区间）：`det≥0.65 + 1000≤面积≤5000 + 行数≥1` → 误判 2%、漏判 0%
    （但对「大字号/只有标题」的页面有漏判风险，仅作兜底）；
  - 真正落到产品时，若需要行高/面积/行数阈值，需把它们配置化并做 DPI 归一化（记录 §6 给了最小改动方案）。

## S-15 口径

- **native**：帧原分辨率 det → 原分辨率裁框 rec；
- **budget**：按生产 `det_input_size`（长边 ≤3200 且 ≤3.0Mpx，不放大）降采样帧后 det →
  框坐标还原到原帧 → **从原帧裁框 rec**（与 `ocr_service.rs` 的 det 预算口径一致）；
- 阅读顺序：框按中心 y 排序 → 相邻 y 差 > 0.5×行高则换行 → 行内按中心 x 排序 → 行间 `\n` 拼接；
- 文本归一化：NFKC + 去除所有空白；CER = 编辑距离 / max(1, 真值字符数)，准确率 = 1-CER；
- 每个口径同时给出 `all`（所有 rec 框）与 `rec_conf_0.6`（产品 `textThreshold` 过滤后，主口径）两个 variant。
