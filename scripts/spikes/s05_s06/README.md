# S-05 / S-06 时延基准脚本包（ext-bench）

本目录是 `docs/spikes/S-05-翻译链路时延.md` 与 `docs/spikes/S-06-图片搜索端到端时延.md`
的全部可复跑脚本。所有数字都来自真实请求/真实本地推理；**无 Key 或依赖缺失时脚本以非零码
退出并打印 BLOCKED 与解锁步骤**，不编造数据。

## 1. 文件清单

| 文件 | 用途 | 真实测量条件 |
| --- | --- | --- |
| `bench_translate.py` | S-05 云端：200 字中文样例 → OpenAI 兼容 `chat/completions`，20 次 P50/P95/max + HTTP 状态/首字节/token 用量/失败率 | 需 `SNIPASK_BENCH_*`（URL/模型/Key） |
| `bench_local_translate.mjs` | S-05 本地：启动 `llama-server` + HY-MT1.5-1.8B Q4_K_M，200 字样例 20 次，记录冷加载、P50/P95/max、服务端 prefill/decode/tok-s、进程 WorkingSet | 需 `llama-server.exe` + GGUF |
| `bench_image_search.py` | S-06：①视觉识别 → ②生成检索词 → ③Search → ④摘要，quick/deep 两档，20 次端到端 + 分阶段时延 | 需 vision 模型 Key + 检索 API Key |
| `run_local_bench.ps1` | 本地正式测量运行器：安静窗口等待 → 线程候选调优 → 正式 10 次 → 输出长度敏感性 → 确认无 llama-server 残留 | 同 `bench_local_translate.mjs` |
| `benchlock.ps1` | `.verify/bench.lock` CPU 独占锁（acquire/status/release，40 分钟陈旧锁接管） | — |

原始数据目录：`.verify/spikes/S-05-S-06/raw/`；汇总数据目录：`docs/spikes/data/s05_s06/`。

## 1.1 本次实测摘要（2026-11-04，详见 S-05/S-06 文档）

- S-05 云端：**BLOCKED**（无 Key，DeepSeek 空 Key 实测 401）；脚本可一键复跑，`--self-test` 框架自检通过（MOCK）。
- S-05 本地：**FAILED**——i7-12650H CPU 16 线程、HY-MT1.5-1.8B Q4_K_M、`temperature=0.2`、200 字完整译文：
  P50 **11.76s** / P95 **21.84s** / 最快一次 9.89s（阈值 ≤8s）；prefill P50 3.0s、decode P50 8.5s、150 token、
  17.6 tok/s；冷加载 1.5–2.9s；常驻 WorkingSet 2.05–2.10 GiB；`max_tokens=100` 截断时 P95 7.09s（译文不完整）。
- S-06 图片搜索：**BLOCKED**（无 vision/检索 Key）；mock 自检框架通过（quick 121ms / deep 143ms 为人为 mock 延时，
  不作验收依据）。拿到 Key 后按文档 §5 清单 15 分钟可完成。

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

## 4. S-06 图片搜索（填 Key 后 15 分钟复跑）

```powershell
$env:SNIPASK_BENCH_VISION_BASE_URL = "https://api.openai.com/v1"   # 任意 OpenAI 兼容 vision 端点
$env:SNIPASK_BENCH_VISION_MODEL    = "gpt-4o-mini"                 # 或 qwen-vl-max 等
$env:SNIPASK_BENCH_VISION_API_KEY  = "<Key>"
$env:SNIPASK_BENCH_SEARCH_PROVIDER = "tavily"                      # 或 serper
$env:SNIPASK_BENCH_SEARCH_API_KEY  = "<检索 Key>"
python scripts/spikes/s05_s06/bench_image_search.py --modes quick,deep --runs 20
```

- 阈值：quick P95 ≤ 10s、deep P95 ≤ 25s；`--runs` 为每档次数。
- 检索服务为 `none`（默认）时脚本打印 BLOCKED：纯 mock Search 不满足「1 次真实 Search」的验收口径。
- 无 Key 时先跑框架自检（数据为 MOCK，不得作为 DONE 依据）：

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
