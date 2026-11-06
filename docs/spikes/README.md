# docs/spikes — 技术验证（Spike）记录

本目录是 `docs/技术验证清单.md` 验证记录与索引：**P0（Go 前提条件）S-01 ~ S-08** 见 §1–§5；
**P1（技术难点/验收标准）S-09 ~ S-20** 见 §6–§9。
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

---

## 6 P1 结论总表（S-09 ~ S-20，2026-11-05）

> 本轮 P1 由 **AI 助手团队（Agent Teams）** 执行（5 个并行工作流 + Lead 集成）。除 S-19 的真人走查外，证据均来自
> 真实应用（Tauri + applab 驱动）、真实模型（PP-OCRv4 ONNX / llama.cpp）、真实 HTTP（DeepSeek 官方 API）；
> mock 仅用于契约自检且在记录中明确标注、不作验收。环境与 P0 相同（Win11 26200 / i7-12650H / 15.78 GiB /
> 混合 DPI 双屏），构建 commit `3f1b263`（S-09 的 Rust 改动在其测量后合入，见记录）。
> 逐项行片段已合入 `docs/技术验证清单.md` §1。

| 编号 | 验收阈值 | 实测结果 | 结论 | 记录 |
| --- | --- | --- | --- | --- |
| S-09 | 黑屏可检测并返回 SA-1001(retryable)、自动尝试 WGC、界面说明、不崩溃 | 独立 .NET10+WinRT 工具 6 场景×5 路径×3 次=90 采样点，17/17 校验通过：WGC 实机可用且普通窗口帧 sha256 与 BitBlt 完全一致；WDA_MONITOR/DRM 保护面 BitBlt+PrintWindow+WGC **全部纯黑**（WGC 不能绕过）；WDA_EXCLUDEFROMCAPTURE 窗口从截屏消失、WGC 窗口项 3/3 次 5s 无帧；混合 DPI 整帧 blackRatio 0.764851（整帧阈值会漏检）；产品 safe Rust 落地黑屏检测+降级链+SA-1001+16 项单测 | **DONE（等价受保护内容验证完成）** | [S-09](S-09-截图失败降级与WGC对比.md) |
| S-10 | 冷启动 ≤1.5s；空闲内存 ≤120MB；空闲 CPU <0.5% | 安静窗口（无 cargo/OCR/llama；applab 空闲）内 release 验收：冷启动 N=10 **P50 787.2 / P95 858.3 / max 876.8 ms** ≤1.5s（每次运行后 Ctrl+Q 均 1409=应用持有）；首次启动对照 N=3 max 1139.3ms；debug/Vite dev 对照 P95 1506ms（仅参考）；30 分钟空闲（360 样本/5s）主进程 WS **max 36.80MiB**（P50 34.97）、Private WS max 10.20MiB、CPU 均值 **0.00217%**（P95 0.0195%、max 0.1755%，占总核口径）→ 三项均达标；进程树含 7 个 WebView2 子进程 WS 451–463MiB 单列（口径待产品确认）；无泄漏迹象（线程/句柄/子进程稳定） | **DONE**（内存口径待确认，见遗留） | [S-10](S-10-冷启动与空闲资源.md) |
| S-11 | 误判 ≤5%、漏判 ≤5%、阈值可配置 | 100 张语料（50 真实渲染+50 合成负样本，三重校验 100/100）；真实 PP-OCRv4 det；推荐 `det≥0.6 + 单行框面积≥400px² + 单行框高≤70px + 行数≥1` → 误判 **2.0%**、漏判 **0%**；仅置信度基线误判 38%，仅加面积下限 18%；阈值脚本参数化 | **DONE**（产品配置化/DPI 归一化遗留） | [S-11](S-11-文字存在判定阈值.md) |
| S-12 | 四段 100%；来源可访问率 ≥95%；不确定 100% 标注 | 30 张真实四段流水线（视觉→检索词→官方 Search→摘要）0 失败；四段 **30/30**、不确定标注 **4/4**；来源可访问率 **81/136=59.6%**（客观失效 8/反爬 26/网络不可达 21；剔除不可判定 91.0%、乐观上界 **94.1%**，均 <95%）；1 条模型改写 URL(404)+2 条截断 | **FAILED（来源维度）/ DONE（四段+不确定标注）** | [S-12](S-12-图片搜索可信度.md) |
| S-13 | 内存不足 100% 提示降档/切云端；不 OOM；符合 minRamMB | 常驻 0.5B/1.5B/1.8B = **520.8/1715.3/2102.4 MB**、冷加载 1.96/3.39/5.68s（与 S-05 一致）；Job Object 6GB→200MB 上限本机**未被内核强制**（200MB 下 Private 660MB 仍存活）→ OOM 未实测；前端无运行时内存判定、**8GB 降档提示缺失** | **IN PROGRESS** | [S-13](S-13-本地翻译模型资源与降档.md) |
| S-14 | 术语 100%；格式 100%；标注后端 | 云端 `deepseek-flash`+`translate.md`：术语 **15/15=100%**、格式 **47/47=100%**、24/24 成功（P50 761ms）；本地最优 1.5B：术语 100% 但格式 **68.1%**（提示词标题行被复述），0.5B 更差；v3 修复 1.5B 格式 87.2%/术语 80% 仍不达标；`engine` 已贯穿事件/历史/UI，缺 model/promptKey，真实网关仍 TODO(real) | **DONE（云端）/ FAILED（本地）/ 部分满足（后端标注）** | [S-14](S-14-翻译一致性.md) |
| S-15 | 字符准确率 ≥95% | 100 张真实渲染截图（5 类×20，44,905 真值字符）；PP-OCRv4 det+rec：**native 99.38% / budget（生产 3.0Mpx 预算）99.29%**，100/100 张 ≥95%（最低 95.49%）；分类型最低 code 98.83%；两口径差 −0.09pp | **DONE** | [S-15](S-15-OCR字符准确率.md) |
| S-16 | 未签名安装/启动不被阻断且有说明；签名后无拦截 | 真实 NSIS 安装/启动（840.4ms 到「初始化完成」）/静默卸载（4.6s）通过；Defender `MpCmdRun -Scan -ScanType 3` 对 setup/主 exe **found no threats**；三处 exe **NotSigned**；MOTW ZoneId=3 静默安装未被阻断；卸载项/文件/快捷方式/Run 全清；火绒/Win10/交互 SmartScreen/签名后对比未覆盖 | **DONE（核心通过，4 项遗留）** | [S-16](S-16-无签名安装包与杀软误报.md) |
| S-17 | 错误码准确；降级链可用；无未捕获异常 | 真实 app IPC `provider_test` **20/20** 用例与契约 §8.2 一致（401/403→SA-3002、402→SA-3003、429→SA-3004、5xx→SA-3001、其他 4xx→SA-5001、超时→SA-3005、DNS/拒绝→SA-3001）；断网+无 Key 的 `menu:ready` 降级链（浏览器搜索/复制可用、云动作可操作原因）；设置页真实错误 UI；真实 DeepSeek provider_test **200/350ms**；uncaught 0 | **DONE**（动作级端到端待真实网关，遗留 L-1） | [S-17](S-17-网络异常与额度降级.md) |
| S-18 | 主包 ≤25MB；含模型 ≤60MB；CI 记录趋势 | 真实 `pnpm tauri build`（3m26s）→ NSIS setup **2,693,865 B = 2.57 MiB**（PASS，余量 22.4MiB）；release exe 6.83 MiB（+17.4%）；含 OCR 模型估算 11.8–18.0 MiB（成立），含本地 LLM GGUF 4.13 GiB（不成立，按需下载设计）；CI bundle job ≤25MiB 门禁+基线归档已落地 | **DONE** | [S-18](S-18-安装包体积门禁.md) |
| S-19 | 3 名新用户 ≤3 分钟完成最小配置 | 3 名真人不可得；已交付《新用户走查协议》（3 人/计时表/卡点表/完成标准/解锁步骤）；`--reset` 首次启动 6 步自动走查 **4.24s**（逐步 537/666/779/579/621/768ms）、截图 7 张、uncaught 0（自动走查仅参考） | **BLOCKED** | [S-19](S-19-引导可用性.md) |
| S-20 | 第二实例唤起并退出；托盘菜单正确；退出后热键无残留 | 第二实例 **10/10**（唤起、事件 +1、进程数恒 1、第二进程均值 2053ms 退出）；退出后 `Ctrl+Q` 无残留 **5/5**（退出前辅助进程 1409 → 退出后注册成功）；暂停/恢复 **3/3**；托盘宿主 `tray_icon_app` + Shell 图标注册确认、5 个菜单项等价入口核验；原生菜单点击受 Win11 溢出区限制 | **DONE**（原生托盘点击人工补证，遗留 L-1） | [S-20](S-20-托盘单实例热键释放.md) |

**汇总**：12 项中 **DONE 8**（S-09、S-10、S-11、S-15、S-16、S-17、S-18、S-20，各自带遗留）、
部分通过 1（S-14：云端 DONE / 本地 FAILED）、FAILED 1（S-12 来源可访问率）、IN PROGRESS 1（S-13）、
BLOCKED 1（S-19 需真人）。

## 7 P1 关键结论与处置（进产品/架构）

与 [`docs/实施架构.md`](../实施架构.md) §9.2 同源，逐项处置：

1. **捕获降级**：WGC 不是 DRM 的「万能绕过」；`WDA_EXCLUDEFROMCAPTURE` 会让窗口从截屏消失且 WGC 无帧，因此
   降级链必须同时处理「黑帧」与「空帧/无帧」。黑屏阈值按整帧定义（luma<16 占比 ≥99%、stdDev<6），但混合 DPI
   多屏单屏受保护时整帧 blackRatio 仅 0.765 → 真实实现须按**显示器切片/目标区域**判定（遗留）。
2. **OCR 阈值**：仅置信度 0.6 的误判 38% 不可接受；推荐 `det≥0.6 + 面积≥400px² + 行高≤70px + 行数≥1`（2.0%/0%）。
   面积/行高/行数必须配置化并做 DPI 归一化；边界小线段可加一次 rec 复核。
3. **OCR 准确率**：PP-OCRv4 在渲染语料上 99.3–99.4%，生产 3.0Mpx 像素预算可继续用；代码行号列质量另列口径。
4. **翻译一致性**：云端 100% 通过；本地不能只靠提示词，需要产品层「术语回填 + 逐段翻译 + 格式校验/回退云端」；
   本地结果必须标注后端（model/promptKey 待补）。
5. **图片搜索可信度**：四段/不确定标注没问题，**来源可访问率是硬伤**（59.6%，乐观 94.1%）；展示前 HEAD/GET 预校验、
   只引用 Search 返回 URL、检索词实体增强；Fetch 建议 5–8s 超时、404 回退 Search 摘要（与 S-06 一致）。
6. **本地资源**：0.5B/1.5B/1.8B 常驻 521/1715/2102 MB；8GB 机型必须能提示降档或切云端（前端缺口，产品待补）；
   Job Object 在本机未强制限额，OOM 行为需 8GB 实机复验。
7. **网络/降级**：错误映射 20/20 与契约一致；断网时浏览器搜索/复制可用、云动作带可操作原因；真实云动作端到端
   仍待网关（`task_runner` 仍 SkeletonLlmGateway），429/5xx 自动退避未实现。
8. **托盘/单实例/热键**：核心行为 10/10、5/5、3/3 通过；原生托盘菜单点击自动化受环境限制，需人工补 1 条。
9. **构建/发布**：体积门禁 2.57 MiB 远低于 25MB，CI 已加 bundle job；未签名包可安装/启动/卸载且 Defender 无威胁，
   但 Win10/火绒/SmartScreen UI/签名后对比未覆盖；**安装目录与数据目录重合**需产品决策。
10. **引导可用性**：真人 3 人走查仍为 BLOCKED；协议与自动走查脚本已就绪（4.24s 仅参考）。
11. **冷启动/空闲（S-10）**：release 冷启动 P95 858ms（≤1.5s）、空闲主进程 WS max 36.8MiB（≤120MB）、
    空闲 CPU 均值 0.0022%（<0.5%）均达标；debug/Vite dev 口径冷启动 P95 1506ms 仅作对照；
    **空闲内存「主进程 vs 含 7 个 WebView2 子进程的进程树（451–463MiB）」口径需产品确认**（建议判据=主进程），
    8GB/真冷启动/睡眠唤醒未覆盖。

## 8 P1 复跑与原始数据

| 项 | 脚本 | 原始数据（gitignored） |
| --- | --- | --- |
| S-09 | `scripts/spikes/s09/`（.NET 10 + WinRT WGC + GDI BitBlt） | `.verify/spikes/S-09/{raw,shots}`（results.json + 84 PNG） |
| S-10 | `scripts/spikes/s10_s16_s18/s10_coldstart.ps1` | `.verify/spikes/S-10/` |
| S-11/S-15 | `scripts/spikes/s11_s15/{build_corpus,run_ocr_eval}.py`（`.verify/venv-ocr`） | `.verify/spikes/S-11-S-15/` |
| S-12/S-14 | `scripts/spikes/s12_s14/`（Key 放 `.verify/p1/key.txt`） | `.verify/spikes/S-12-S-14/` |
| S-13/S-17/S-19/S-20 | `scripts/spikes/s13_s17_s19_s20/`（applab 驱动 + mock HTTP + Job Object） | `.verify/spikes/S-13-S-17-S-19-S-20/` |
| S-16/S-18 | `scripts/spikes/s10_s16_s18/`（构建/体积/签名扫描/真实安装） | `.verify/spikes/S-16-S-18/` |

精选聚合数据（小体积、随仓库提供）在 `docs/spikes/data/{s09,s10,s11_s15,s12_s14,s13_s17_s19_s20,s16_s18}/`。
P1 指标汇总见 `docs/spikes/data/` 各 `summary*.json`；`docs/spikes/README.md` §6 的数字均可从对应 summary 复核。

## 9 关于「AI 助手执行」的说明（口径透明）

- 本轮 P1 的**执行者、标注者、复核者均为 AI 助手**（Agent Teams），不是外部真人；记录中出现的「执行者复核/
  目视抽检」均指执行者本人（AI）的核对，**不是独立人工评审**。S-15 的真值来自「构造内容 + DOM→截图三重自动校验」，
  一致性由 `body.innerText` 校验保证，10 张目视抽检只作补充；S-19 明确 BLOCKED，需真人补做。
- 全部时延/体积/内存/错误码数字来自真实进程、真实模型、真实 HTTP；mock 仅用于契约自检并明确标注。
- 用户提供的 DeepSeek Key 仅存于 gitignored `.verify/p1/key.txt` 并在运行时读取，仓库/日志/文档均无泄露（全仓扫描）。
