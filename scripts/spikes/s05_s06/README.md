# S-05 / S-06 时延基准脚本包（ext-bench）

本目录是 `docs/spikes/S-05-翻译链路时延.md` 与 `docs/spikes/S-06-图片搜索端到端时延.md`
的全部可复跑脚本。所有数字都来自真实请求/真实本地推理；**无 Key 或依赖缺失时脚本以非零码
退出并打印 BLOCKED 与解锁步骤**，不编造数据。

## 1. 文件清单

| 文件 | 用途 | 真实测量条件 |
| --- | --- | --- |
| `bench_translate.py` | S-05 云端：200 字中文样例 → OpenAI 兼容 `chat/completions`，20 次 P50/P95/max + 首字节/TTFT/总耗时/token 用量/失败率 | 需 `SNIPASK_BENCH_*`（URL/模型/Key）；`--stream` 记录 TTFT |
| `bench_local_translate.mjs` | S-05 本地：启动 `llama-server` + 1.8B/0.5B GGUF，200 字样例，记录冷加载、P50/P95/max、服务端 prefill/decode/tok-s、进程 WorkingSet | 需 `llama-server.exe` + GGUF |
| `bench_image_search.py` | S-06：①视觉识别 → ②生成检索词 → ③Search → ④摘要，quick/deep 两档；`--search-provider deepseek` 用官方 Search，deep 另含并发 Fetch；`--allow-mock-search` 仅做部分估算 | 需 DeepSeek Key（vision + Search/Fetch 同一 Key） |
| `bench_deepseek_tools.py` | S-06 参考：DeepSeek Search/Fetch 单独时延（各 20 次，含 429/5xx 重试与 404 处理） | 需 DeepSeek Key |
| `probe_s06_negative.py` | phase-1 历史能力探测（视觉是否可用 / 旧 `/search` 是否 404），保留为历史证据 | 需 DeepSeek Key |
| `run_local_bench.ps1` | 本地 1.8B 正式测量运行器：安静窗口 → 线程调优 → 正式 10 次 → 长度敏感性 → 无残留确认 | 同 `bench_local_translate.mjs` |
| `benchlock.ps1` / `build_data_summary.py` | CPU 独占锁；原始数据聚合到 `docs/spikes/data/s05_s06/` | — |

**Key 纪律**：所有脚本只从环境变量读 Key，**不打印、不落盘**（响应/配置 JSON 只含 `api_key_provided: true` 与脱敏 host）。

原始数据目录：`.verify/spikes/S-05-S-06/raw/`；汇总数据目录：`docs/spikes/data/s05_s06/`。

## 1.1 本次实测摘要（2026-11-04/05，详见 S-05/S-06 文档）

- **S-05 云端（DeepSeek `deepseek-flash`，stream 20 次）**：
  - 默认 thinking：TTFT P50 3.47s / P95 4.59s，总耗时 P50 4.05s / P95 5.15s → **两口径均 FAILED**（reasoning_tokens 384–736）。
  - `thinking:{"type":"disabled"}`：TTFT P50 0.61s / P95 0.87s → **首字口径 DONE**；总耗时 P50 1.51s / P95 1.75s → 总耗时口径 FAILED（差 ~0.25s）。
  - 结论：网关必须显式关闭 DeepSeek 思考才能满足「快速模式首字 ≤1.5s」。
- **S-05 本地 1.8B（CPU 16 线程）**：**FAILED**——P50 11.76s / P95 21.84s / 最快 9.89s（≤8s 阈值）；150 token、17.6 tok/s；
  常驻 WorkingSet 2.05–2.10 GiB；`max_tokens=100` 截断 P95 7.09s（译文不完整）。
- **S-05 极小模型 Qwen2.5-0.5B Q4_K_M（CPU 16 线程，10 次）**：时延 **DONE**——P50 3.78s / P95 4.13s、51 tok/s、
  常驻 0.51–0.55 GiB、冷加载 1.34s；**但产品 `translate.md` 提示词下 10/10 输出中文原文（未翻译）**。
  候选复核（task-13）：换极简指令「将下面的中文翻译成英文，只输出译文：」后 0.5B **可正常翻译**（200 字 P95 3.21s、
  30 字 P95 0.42s，质量一般但可用）；`few-shot` 对 200 字有害（只输出示例）。**Qwen2.5-1.5B Q4 + 极简指令**：
  200 字 P95 6.72s、25.7 tok/s、常驻 1.67–1.70 GiB、冷加载 1.44s，质量更好。极小档推荐
  **0.5B + 极简提示词**（质量优先用 1.5B），两者都不能配产品 `translate.md`。
- **S-06 图片搜索：DONE**（真实 DeepSeek Search/Fetch 已解阻）：quick 端到端 **P95 6,006.64ms ≤10s**、
  deep **P95 8,290.75ms ≤25s**（各 20 次，0 失败）；阶段 P50：quick vision 2.52s + search 0.28s + summary 2.47s；
  deep vision 2.50s + search(3×10) 0.94s + fetch(2 并发) 0.15s + summary 3.16s。Search 单独 P50 292ms/P95 413ms；
  Fetch（稳定 URL）P50 84ms/P95 3.20s。deep 长尾 max 23.1s（失效 URL 8s×2 重试），仍 <25s；生产建议 Fetch 5–8s 超时、失败回退摘要。

## 2. S-05 云端（填 Key 后一键复跑）

```powershell
$env:SNIPASK_BENCH_BASE_URL = "https://api.deepseek.com/v1"      # 或 DashScope 兼容模式
$env:SNIPASK_BENCH_MODEL    = "deepseek-chat"                    # 或 qwen-mt-turbo / qwen-mt-lite
$env:SNIPASK_BENCH_API_KEY  = "<你的 Key>"
python scripts/spikes/s05_s06/bench_translate.py --runs 20 --warmup 2
# 可选：Qwen-MT 专用参数透传
# $env:SNIPASK_BENCH_EXTRA_JSON = '{"translation_options":{"source_lang":"Chinese","target_lang":"English"}}'
```

- 阈值：P95 ≤ 1500ms（200 字，非流式端到端）；脚本自动打印 DONE/FAILED 并写 JSON。
- 无 Key → 打印 BLOCKED 并以退出码 2 结束（可安全复跑，不会造假）。
- `--self-test`：脚本内置 mock OpenAI 服务，只验证框架；输出标注 `MOCK_SELF_TEST`，**不是验收结论**。

## 3. S-05 本地（llama.cpp）

依赖二选一：

1. **本机已有 llama.cpp 预编译**（本次实际使用）：LM Studio 后端
   `%USERPROFILE%\.lmstudio\extensions\backends\llama.cpp-win-x86_64-avx2-2.28.2\llama-server.exe`
   （llama.cpp 官方 `llama-server` 前端，LM Studio 打包；脚本会自动探测它）。
2. 官方 release 解压：`llama-bXXXXX-bin-win-cpu-x64.zip` → `llama-server.exe`，
   用 `--exe` 或 `$env:LLAMA_SERVER_EXE` 指定。

模型（ModelScope，已实测可下载，sha256 与清单核对一致）：

```powershell
curl.exe -L -o .verify/spikes/S-05-S-06/artifacts/HY-MT1.5-1.8B-Q4_K_M.gguf `
  "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/HY-MT1.5-1.8B-Q4_K_M.gguf"
# 期望 sha256: 4383ac0c3c8e476de98ff979c2a3f069f8c4fb385e7860cf2d28da896cc477c7
```

查看依赖是否齐备（不加载模型）：

```powershell
node scripts/spikes/s05_s06/bench_local_translate.mjs --self-check
```

正式测量（含安静窗口等待与线程调优，自动停 llama-server）：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s05_s06/run_local_bench.ps1 `
  -ThreadList "6,16" -Runs 10
```

单次基准（等价底层命令）：

```powershell
node scripts/spikes/s05_s06/bench_local_translate.mjs --threads 6 --runs 20 --warmup 2
```

采样参数与产品配置对齐：`temperature=0.2`（`config.providers[].temperature` 默认值）、
`max_tokens=1024`（默认值）、`top_p=0.95`。`--temperature/--max-tokens/--top-k/--repeat-penalty`
可覆盖。**不要用模型卡推荐的 temperature=0.7 复跑对比**：本次实测 0.7 会出现重复输出、
耗时翻倍，与产品默认配置不一致。

## 4. S-06 图片搜索（真实 DeepSeek Search/Fetch，已 DONE）

**实测结论**：quick P95 **6,006.64ms**、deep P95 **8,290.75ms**（阈值 10s / 25s，各 20 次 0 失败）。
视觉用 `deepseek-flash`（thinking disabled）；检索用官方 `POST /v1/tools/search`；deep 额外并发 `POST /v1/tools/fetch` 2 条正文。

```powershell
$env:SNIPASK_BENCH_API_KEY           = "<DeepSeek Key>"       # 仅运行时环境变量
$env:SNIPASK_BENCH_VISION_BASE_URL   = "https://api.deepseek.com"
$env:SNIPASK_BENCH_VISION_MODEL      = "deepseek-flash"
$env:SNIPASK_BENCH_VISION_API_KEY    = "<DeepSeek Key>"
$env:SNIPASK_BENCH_VISION_EXTRA_JSON = '{"thinking":{"type":"disabled"}}'
$env:SNIPASK_BENCH_SEARCH_PROVIDER   = "deepseek"
$env:SNIPASK_BENCH_FETCH_PROVIDER    = "deepseek"

# 完整四阶段流水线：quick/deep 各 warmup 2 + 20 次
python scripts/spikes/s05_s06/bench_image_search.py --modes quick,deep --runs 20 --warmup 2 --deep-fetch 2
# Search/Fetch 单独时延（各 20 次）
python scripts/spikes/s05_s06/bench_deepseek_tools.py --runs-search 20 --runs-fetch 20 --count 5
```

- 阈值：quick P95 ≤10s、deep P95 ≤25s；`--runs` 为每档次数；`--search-count` 默认 quick 5 / deep 10。
- 生产建议：Fetch 单次 timeout 5–8s、失败回退 Search 摘要、跳过 404；deep 结果 30 条（3×10）tokens ~6.5k，可去重压缩。
- 其他检索服务（tavily/serper）与 `--allow-mock-search`（PARTIAL）保留作对比/自检，**mock 不作验收**。
- 无 Key 时框架自检（MOCK，不得作为 DONE 依据）：

```powershell
python scripts/spikes/s05_s06/bench_image_search.py --mock-self-check --runs 3
```

## 5. CPU 独占纪律与复现注意

- 正式时延测量前确认 `pwsh scripts/spikes/s05_s06/benchlock.ps1 status` 为 `no lock`，
  并 `acquire`；测完 `release`。存在他人锁（<40 分钟且 pid 存活）时等待，不要抢。
- 本次正式本地测量由长驻 holder 进程持锁（`owner=ext-bench`），`run_local_bench.ps1`
  自身也会等待 CPU 负载 ≤40% 再开跑，并在结束时确认 `Get-Process llama-server` 无残留。
- 云端基准只占网络；本地基准占满 CPU，必须安静窗口，否则 P95 会被并行编译/OCR 基准污染
  （本次有实测对照，见 S-05 文档「并行负载污染对照」）。
- 环境变量里的 Key 脚本不会打印；JSON 里的 `base_url_masked` 只含 host。
