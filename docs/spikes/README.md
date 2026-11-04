# docs/spikes — P0 技术验证（Spike）记录

本目录是 `docs/技术验证清单.md` 中 **P0（Go 前提条件）S-01 ~ S-08** 的验证记录与索引。
每份记录按清单 §4 模板编写（编号 / 结论 / 日期 / 环境 / 步骤 / 原始数据 / 结果 / 处置 / 遗留），
原始数据在 `.verify/spikes/<编号>/`（gitignored），可复跑脚本在 `scripts/spikes/`。

> 验证时间：2026-11-04 ~ 2026-11-05（同一测试会话）
> 构建 commit：`a9b352e`（工作区含本轮全部未提交改动）
> 环境：Windows 11 26200 / i7-12650H（10C16T）/ 15.78 GiB / Rust 1.99.1 / Node 24 / Python 3.11
> 显示：**混合 DPI 双屏** — 主屏 2560×1440 @150%（物理原点 (0,0)）+ 副屏 1080×1920 @100%（物理原点 (-1080,-470)），
> 虚拟桌面物理矩形 **(-1080,-470) 3640×1920**（与 Win32 `SM_*VIRTUALSCREEN` 一致；副屏为虚拟显示适配器，
> 4K/200% 真机未覆盖，见各项「遗留」）。
> 网络：PyPI / ModelScope / GitHub / crates.io 可达；**HuggingFace 超时不可达**；**本机无任何云端 API Key**。
> 纪律：所有数字来自真实应用 / 真实模型推理 / 真实 HTTP；mock 结果一律标注且不作为验收依据；
> CPU 敏感基准用 `.verify/bench.lock` 独占，应用验证用 `applab` 独占（单实例）。

## 1 结论总表

| 编号 | 验收阈值 | 实测结果 | 结论 | 记录 |
| --- | --- | --- | --- | --- |
| S-01 | 混合 DPI 多屏框选坐标误差 ≤ 2px；跨屏拖选不跳变 | overlay client 原点+尺寸 == 虚拟桌面 (-1080,-470) 3640×1920（≤2px 断言 True）；跨屏拖选 6/6 组通过，字段级误差 max **0.01px**；Win32 十字线 18/18，max **0.547px** | **DONE** | [S-01-混合DPI多屏坐标一致性.md](S-01-混合DPI多屏坐标一致性.md) |
| S-02 | 4K 双屏整屏 det ≤ 1.2s，后台不阻塞选框 | 7680×2160 → det 输入 3200×900，30 次 **P50 730ms / P95 926ms / max 994ms**；60Hz 定时器抖动 max 3.42ms、0 帧超 2 周期 | **DONE** | [S-02-S-03-OCR时延基准.md](S-02-S-03-OCR时延基准.md) |
| S-03 | 选区 det+rec，100 次 P95 ≤ 800ms | 100 次 **P50 386ms / P95 968ms / max 1026ms**（超 168ms）→ **FAILED**；另一口径「整屏 det 缓存复用 → rec-only」P95 **596ms** 达标（需架构/阈值决策） | **FAILED** | [S-02-S-03-OCR时延基准.md](S-02-S-03-OCR时延基准.md) |
| S-04 | 注册成功率 100%；冲突 100% 可操作提示；暂停后热键完全释放 | probe 20/20 available（P50 2.3ms）；辅助进程同键注册 20/20 被拒（Win32 1409）；OS 冲突检出 20/20 + 设置页可操作文案（截图）；暂停后辅助进程注册成功（释放）、恢复后重新注册；probe 无注册泄漏 20/20 | **DONE** | [S-04-CtrlQ全局热键注册与冲突检测.md](S-04-CtrlQ全局热键注册与冲突检测.md) |
| S-05 | 云端 200 字 ≤ 1.5s；本地 1.8B(Q4, CPU) ≤ 8s | 云端 **BLOCKED**（无 Key：DeepSeek 401 / DashScope 400）；本地真机 llama.cpp + HY-MT1.5-1.8B Q4_K_M：**P50 11.76s / P95 21.84s / 最小 9.89s**（10/10 成功）→ 超阈值 | **FAILED**（本地）/ **BLOCKED**（云端） | [S-05-翻译链路时延.md](S-05-翻译链路时延.md) |
| S-06 | 图片搜索快速 ≤ 10s；深度 ≤ 25s | **BLOCKED**：无 vision Key、无检索 Key；4 阶段基准脚本与 mock 框架自检已就绪，拿到 Key 后约 15 分钟可复跑 | **BLOCKED** | [S-06-图片搜索端到端时延.md](S-06-图片搜索端到端时延.md) |
| S-07 | 无网络/无 Key 可完成 6 步引导并进入受限模式；动作禁用原因可操作 | 首次启动 6 步引导可完成并可跳过；受限模式生效：浏览器搜索/复制/本地 OCR 可用，AI 搜索/解答/图片搜索/云端翻译 disabled 且文案指向配置入口；断网（baseUrl 不可达 + 空 Key）错误可操作、不崩溃；截图取证 | **DONE** | [S-07-首启引导无网络无Key与受限模式.md](S-07-首启引导无网络无Key与受限模式.md) |
| S-08 | 断点续传/中断恢复 100%；SHA256+大小校验；镜像回退；进度/速度/剩余时间正确 | 随机断点 20 次中断恢复 **20/20 = 100%**；**真实杀进程**后续传成功（Range 起点==已发字节）；SHA256/大小不符删除重下；404/连接拒绝自动切镜像；真实 ModelScope Range **206 + Content-Range**；reqwest 0.348s | **DONE** | [S-08-模型下载断点续传与校验.md](S-08-模型下载断点续传与校验.md) |

汇总：**DONE 5 项**（S-01、S-02、S-04、S-07、S-08）；**FAILED 2 项**（S-03 部分达标、S-05 本地不达标）；
**BLOCKED 1 项**（S-06，且 S-05 云端腿同样 BLOCKED）。按清单约定，FAILED/BLOCKED 项需回到技术方案决策后再进入 MVP。

## 2 必须带回技术方案的结论（Go/No-Go 关注点）

1. **S-03 选区 OCR 未达标，但存在明确优化路径**：det+rec P95 968ms（>800ms），主要成本是 rec（P95 693ms）；
   架构若改为「整屏 det 缓存 → 选区过滤 + rec-only」，P95 596ms 可达标。**需确认是否修改实施架构 M05 的选区 OCR 路径与验收口径。**
2. **OCR 瓶颈是「降采样后像素面积」而非屏幕数量**：4K 双屏（3200×900=2.88Mpx）P95 926ms 反而快于**单屏 4K**
   （3200×1800=5.76Mpx）P95 **1695ms**（超阈值）。建议把 `ocr.detMaxEdge=3200` 改为「像素预算 ≈3.0Mpx」，
   单 4K 用 2400–2800 或分块；`intra_op` 线程 6–8 最优（16 线程退化）。
3. **本地翻译 1.8B Q4 在 CPU 上无法满足 8s**（P50 11.76s，最快 9.89s，17.6 tok/s，常驻 ≈2.1GiB）。
   处置：默认云端 + 本地流式标注慢；改阈值分级（纯 CPU P50 ≤12s）或 GPU offload；或改用专用小模型（如 OPUS-MT，
   但其 Ct2 产物目前官方仓库缺失）；输出限长 + prefix cache。
4. **S-05 云端 / S-06 需要 API Key 才能完成**：本机无 Key，脚本已就绪（`scripts/spikes/s05_s06/README.md`），
   填入 `SNIPASK_BENCH_*` 环境变量即可复跑；S-06 另需搜索 API。
5. **模型清单已修正**：原 `resources/models.manifest.json` 三个 URL 实测 404；hy-mt / qwen3 已按官方仓库改为
   正确文件名 + 大小 + SHA256（`verified` 仍为 false，因 opus-mt 缺 Ct2 产物，`urls` 置空并注明）。
6. **S-01 修复中也修正了产品缺陷**：overlay 原先只覆盖主屏，且 client 原点比外框偏 (11,2)px
   （单屏也会整体偏移）；现按虚拟桌面外接矩形 + `PhysicalSize/Position` + client 原点校正 + `.shadow(false)`，
   并修正 dpr 语义（取窗口实际 `scale_factor()`，不能逐屏混用）。
7. **S-04 修复了真实缺陷**：原「暂停」会注销包括 `togglePause` 在内的全部热键，导致暂停后无法用热键恢复；
   现暂停时保留 `togglePause`、其余释放，并有真机证据。

## 3 复跑入口

见 `scripts/spikes/README.md`（applab 验证台、截图工具、各编号脚本与资源纪律）。逐项命令也写在各自文档的「复跑」节。

## 4 原始数据

- `.verify/spikes/S-01/`：标定/基线/修复后 JSON + 全屏截图
- `.verify/spikes/S-02-S-03/`：`inputs/`（真实截图与 4K/双 4K 构造输入）、`results/`（90+100 条逐次样本、抖动、参数扫描、负载证据）
- `.verify/spikes/S-04/`：`logs/*.jsonl`（矩阵全过程）、`shots/`（冲突提示截图）
- `.verify/spikes/S-05-S-06/`：`raw/`（llama-server 日志、每次请求记录、云端 401/400 证据、mock 自检）
- `.verify/spikes/S-07/`：`raw/`（IPC 返回、事件监听、配置快照）、`shots/`（引导/受限模式/overlay 菜单截图）
- `.verify/spikes/S-08/`：Range 探测响应头、ModelScope API 文件列表、测试/clippy 日志

精选的聚合数据在 `docs/spikes/data/`（S-02-S-03、S-05-S-06）。
