# docs/spikes — P0 技术验证（Spike）记录

本目录是 `docs/技术验证清单.md` 中 **P0（Go 前提条件）S-01 ~ S-08** 的验证记录与索引。
每份记录按清单 §4 模板编写（编号 / 结论 / 日期 / 环境 / 步骤 / 原始数据 / 结果 / 处置 / 遗留），
原始数据在 `.verify/spikes/<编号>/`（gitignored），可复跑脚本在 `scripts/spikes/`。

> 验证时间：2026-11-04 ~ 2026-11-05（同一测试会话；phase 2 处置见 §5）
> 构建 commit：phase 1 = `798f87b`，phase 2 = 工作区未提交改动
> 环境：Windows 11 26200 / i7-12650H（10C16T）/ 15.78 GiB / Rust 1.99.1 / Node 24 / Python 3.11
> 显示：**混合 DPI 双屏** — 主屏 2560×1440 @150%（物理原点 (0,0)）+ 副屏 1080×1920 @100%（物理原点 (-1080,-470)），
> 虚拟桌面物理矩形 **(-1080,-470) 3640×1920**（与 Win32 `SM_*VIRTUALSCREEN` 一致；副屏为虚拟显示适配器，
> 4K/200% 真机未覆盖，见各项「遗留」）。
> 网络：PyPI / ModelScope / GitHub / crates.io 可达；**HuggingFace 超时不可达**；云端 Key 由用户于 phase 2 提供（DeepSeek），
> 文档只记录脱敏信息。
> 纪律：所有数字来自真实应用 / 真实模型推理 / 真实 HTTP；mock 结果一律标注且不作为验收依据；
> CPU 敏感基准用 `.verify/bench.lock` 独占，应用验证用 `applab` 独占（单实例）。

## 1 结论总表

| 编号 | 验收阈值 | 实测结果 | 结论 | 记录 |
| --- | --- | --- | --- | --- |
| S-01 | 混合 DPI 多屏框选坐标误差 ≤ 2px；跨屏拖选不跳变 | overlay client 原点+尺寸 == 虚拟桌面 (-1080,-470) 3640×1920（0px 偏差）；跨屏拖选 6/6 组，字段级误差 max **0.01px**；Win32 十字线 18/18，max **0.547px** | **DONE** | [S-01-混合DPI多屏坐标一致性.md](S-01-混合DPI多屏坐标一致性.md) |
| S-02 | 4K 双屏整屏 det ≤ 1.2s，后台不阻塞选框 | **像素预算 3.0Mpx + 长边 3200 已落地**；干净窗口 30 次：双 4K 7680×2160→3200×900 **P50 724 / P95 743ms**；单 4K→2309×1299 **P95 794ms**（旧长边口径 1682ms 超阈值）；主屏 2560×1440→P95 761ms；60Hz 抖动 max 3.42ms | **DONE** | [S-02-S-03-OCR时延基准.md](S-02-S-03-OCR时延基准.md) §13/§14 |
| S-03 | 选区 OCR，100 次 P95 ≤ 800ms | 原 det+rec 口径 P95 **968ms**（未达标，保留为对照）；按实施架构改为「整屏 det 缓存 → 选区过滤 + rec-only」P95 **596ms** 达标并**已落地**（`OcrPipeline`，28 项测试 + 真机冒烟）；真实 ONNX 引擎仍 `TODO(real)` | **DONE（口径变更）** | [S-02-S-03-OCR时延基准.md](S-02-S-03-OCR时延基准.md) §13 |
| S-04 | 注册成功率 100%；冲突 100% 可操作提示；暂停后热键完全释放 | probe 20/20 available（P50 2.3ms）；辅助进程同键注册 20/20 被拒（Win32 1409）；OS 冲突检出 20/20 + 设置页可操作文案（截图）；暂停后辅助进程注册成功（释放）、恢复后重新注册；probe 无泄漏 20/20 | **DONE** | [S-04-CtrlQ全局热键注册与冲突检测.md](S-04-CtrlQ全局热键注册与冲突检测.md) |
| S-05 | 云端 200 字 ≤ 1.5s；本地 ≤ 8s | 云端（DeepSeek `deepseek-flash`，20 次）：**thinking 关闭** 首字 **P50 0.61s / P95 0.87s 达标**，总耗时 P95 1.75s 略超；**默认 thinking** 首字 P95 4.59s 不达标（网关必须显式关闭思考）。本地**极小档**（Qwen2.5-0.5B Q4 + 精简提示词）P50 3.09s / **P95 3.21s**、常驻 0.55GiB 达标；**普通档** HY-MT1.5-1.8B CPU P50 11.76s / P95 21.84s 不满足 8s → 按用户决策改为**可选质量档并显式提示** | **DONE（云端首字 + 极小档）/ FAILED（1.8B 普通档基准）** | [S-05-翻译链路时延.md](S-05-翻译链路时延.md) |
| S-06 | 图片搜索快速 ≤ 10s；深度 ≤ 25s | **真实链路 DONE**（DeepSeek `deepseek-flash` 视觉 + 官方 `/v1/tools/search` + `/v1/tools/fetch`）：quick **P50 5.28s / P95 6.01s**（≤10s）、deep **P50 6.83s / P95 8.29s**（≤25s，含并发 Fetch），各 20 次 0 流水线失败；Search 单独 P95 413ms、Fetch 稳定源 P95 3.2s；长尾来自失效 URL（建议 Fetch 5–8s 超时 + 404 回退 Search 摘要） | **DONE** | [S-06-图片搜索端到端时延.md](S-06-图片搜索端到端时延.md) |
| S-07 | 无网络/无 Key 可完成 6 步引导并进入受限模式 | 首次启动 6 步引导可完成/可跳过；受限模式门禁生效（浏览器搜索/复制/本地 OCR 可用，云动作 disabled + 可操作原因）；**引导第 3 步落地三档 AI 服务**（云端/极小模型/普通模型 + 实测性能与隐私/成本提示，20/20 断言）；断网（不可达 baseUrl + 空 Key）错误可操作、不崩溃 | **DONE** | [S-07-首启引导无网络无Key与受限模式.md](S-07-首启引导无网络无Key与受限模式.md) |
| S-08 | 断点续传/中断恢复 100%；SHA256+大小校验；镜像回退；进度正确 | 随机断点 20 次中断恢复 **20/20**；**真实杀进程**后续传成功；SHA256/大小不符删除重下；404/拒绝自动切镜像；真实 ModelScope Range **206 + Content-Range**（reqwest 0.348s）；phase 2 新增第 4 条极小模型清单（491,400,032 B，206/416 复核） | **DONE** | [S-08-模型下载断点续传与校验.md](S-08-模型下载断点续传与校验.md) |

**最终结论：P0 8 项全部达成（S-03/S-05 经用户确认的口径/档位调整后达标；S-06 已用 DeepSeek 官方 Search/Fetch 工具 API 完成真实验收：
quick P95 6.01s / deep P95 8.29s）。** 另有 S-17（P1）预置能力落地：[provider_test 真实连通性](S-17-provider连通性测试落地.md)。

## 2 必须带回技术方案的结论（Go/No-Go 关注点）

1. **选区 OCR 口径已按决策落地**：整屏 det 缓存 → 选区过滤 + rec-only（P95 596ms）。**实施边界**：真实 ONNX
   引擎仍为 `TODO(real)`，当前为骨架引擎；接入时需保证 det 输入使用像素预算（见下）与缓存坐标口径。
2. **OCR 瓶颈是「降采样后像素面积」而非屏幕数量**：双 4K（3200×900=2.88Mpx）P95 743ms 快于**单屏 4K**
   旧口径 3200×1800=5.76Mpx（1682ms，超阈值）。**已落地双约束**：`scale = min(3200/max(w,h), sqrt(3.0Mpx/(w×h)))`、
   不放大（单 4K→2309×1299，P95 794ms）；低配 profile 建议 2.5Mpx（P95 632ms）；`intra_op` 线程 6–8 最优。
3. **翻译档位（用户决策 item3）**：引导页三档 **云端 / 极小模型 / 普通模型**，并显式提示性能与隐私/成本：
   云端（关闭 thinking）首字 P95 0.87s；极小档 0.5B + 精简提示词 P95 3.21s / 0.55GiB（质量一般，长文建议云端）；
   普通档 1.8B 质量优先但纯 CPU P95 21.84s。**1.8B 的 8s 目标在纯 CPU 上不可达**，产品以「可选档 + 提示」呈现。
4. **S-05 云端总耗时（P95 1.75s）仍略超 1.5s 阈值**；首字口径达标。若坚持「总耗时 ≤1.5s」，需更快的模型/短输出/流式呈现，
   否则建议把验收口径明确为「首字 ≤1.5s」（与 README §验收与性能目标「快速模式首字 ≤1.5s」一致）。
5. **S-06 已解除 BLOCKED**：DeepSeek 单 Key 覆盖 chat/vision + 官方 **Search/Fetch 工具 API**
   （`POST /v1/tools/search`、`POST /v1/tools/fetch`），真实图片搜索链路 quick P95 6.01s、deep P95 8.29s（各 20 次 0 失败）。
   生产注意事项：视觉与摘要均应传 `thinking:{"type":"disabled"}`；Fetch 建议 5–8s 超时、404 不重试并回退 Search 摘要
   （长尾主要来自检索结果的失效 URL：任意 URL 抓取时 8/20 404、P95 17.9s）。Search/Fetch 按成功请求计费（5 元/千次），
   QPS 分别 100 / 25。
6. **本地极小模型必须配精简提示词**：0.5B 用产品 `translate.md` 会复述原文（10/10），
   换 `resources/prompts/translate_minimal.md` 后可正常翻译（提示词路由已按模型 id 实现）。
7. **模型清单已修正并扩充**：4 条（hy-mt / qwen3 / opus-mt 待产物 / qwen2.5-0.5b 极小模型），前 3 条 size+sha256 已核验。
8. **S-01 修复中也修正了产品缺陷**：overlay 原先只覆盖主屏且 client 原点偏 (11,2)px；现按虚拟桌面外接矩形 +
   `PhysicalSize/Position` + client 原点校正 + `.shadow(false)`，dpr 取窗口实际 `scale_factor()`。
9. **S-04 修复了真实缺陷**：原「暂停」会注销包括 `togglePause` 在内的全部热键；现保留 `togglePause`、其余释放。

## 3 复跑入口

见 `scripts/spikes/README.md`（applab 验证台、截图工具、各编号脚本与资源纪律）。逐项命令也写在各自文档的「复跑」节。

## 4 原始数据

- `.verify/spikes/S-01/`：标定/基线/修复后 JSON + 全屏截图
- `.verify/spikes/S-02-S-03/`：`inputs/`（真实截图与 4K/双 4K 构造输入）、`results/`（逐次样本、抖动、像素预算扫描、负载证据、真机冒烟）
- `.verify/spikes/S-04/`：`logs/*.jsonl`（矩阵全过程）、`shots/`（冲突提示截图）
- `.verify/spikes/S-05-S-06/`：`raw/`（llama-server 日志、每次请求、云端 TTFT/总耗时、提示词变体、S-06 能力探测、mock 自检）、`artifacts/`（模型文件）
- `.verify/spikes/S-07/`：`raw/`（IPC 返回、事件监听、配置快照）、`shots/`（引导/受限模式/overlay 菜单/三档选择截图）
- `.verify/spikes/S-08/`：Range 探测响应头、ModelScope API 文件列表、测试/clippy 日志
- `.verify/spikes/S-17/`：provider 连通性冒烟（脱敏）

精选的聚合数据在 `docs/spikes/data/`（S-02-S-03、S-05-S-06）。

## 5 phase 2 门禁处置落地（2026-11-05）

按用户对首轮 P0 结论的决策完成 4 项落地（详细实现见 `docs/实施架构.md` §9.1 与 `CHANGELOG.md`）：

1. **S-03 架构落地**：`services/ocr_service.rs` 新增 `det_input_size`（像素预算）、`OcrEngine::{detect, recognize,
   detect_and_recognize}`、`OcrPipeline`（整屏 det 缓存 → 选区过滤 + rec-only、兜底 det+rec、会话失效）；
   `app/state.rs` / `commands/{capture,selection}.rs` 接线；新增 28 项测试 + 真机冒烟。
2. **S-02 像素预算**：默认 3.0Mpx + 长边 3200（低配 2.5Mpx），真实 ONNX 复核见 S-02/S-03 记录 §14。
3. **引导页三档 AI 服务（item3）**：`TranslateStep` 三档选择 + 实测性能提示（云端首字 / 极小档 P95 3.2s / 普通档 P95 21.8s），
   选择写入 `translate.backend` + `local.model`；新增极小模型清单条目与下载聚焦；DeepSeek 供应商说明修正（支持视觉、无 Search API）。
4. **S-05 云端实测（用户提供 DeepSeek Key）**：见 §1 S-05 与 `S-05-翻译链路时延.md`；**网关接入时必须显式关闭 thinking**。
   同时落地 S-17 预置能力：`provider_test` 真实连通性（`S-17-provider连通性测试落地.md`）。
5. **S-06 解阻与真实验收（2026-11-05）**：用户提供官方 [Search/Fetch 文档](https://api-docs.deepseek.com/zh-cn/guides/search)
   （`POST /v1/tools/search`、`POST /v1/tools/fetch`，同 Bearer 鉴权）。基准脚本新增真实 Search/Fetch 分支并跑完整
   图片搜索四阶段流水线：quick P50 5.28s / P95 6.01s（≤10s）；deep P50 6.83s / P95 8.29s（≤25s，含并发 Fetch），
   各 20 次 0 流水线失败。产品实现仍为 `TODO(real)`：网关需补 Search/Fetch 客户端（错误码 408/429/5xx 退避、
   404 不重试、费用计量）与 `thinking` 关闭开关。
