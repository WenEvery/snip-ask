# S-12 / S-14 脚本包（ai-quality）

本目录是 `docs/spikes/S-12-图片搜索可信度.md` 与 `docs/spikes/S-14-翻译一致性.md` 的全部可复跑脚本。
所有数字来自**真实** DeepSeek 接口 / 真实 llama.cpp 推理 / 真实 headless Edge 截图；mock 结果一律标注
`MOCK_SELF_TEST` 且**不得作为验收依据**。无 Key 或依赖缺失时脚本以非零码退出并打印 BLOCKED 与解锁步骤，
不编造数据。

## 1 文件清单

| 文件 | 用途 | 真实测量条件 |
| --- | --- | --- |
| `s14_cases.py` | S-14 样例集（24 条中→英/英→中，含换行/空行/列表/编号/代码块/URL/邮箱/占位符/标识符/术语）+ 术语渲染 + 判分纯函数 | 无需依赖 |
| `bench_translate_consistency.py` | S-14 主基准：云端 DeepSeek chat/completions 与本地 llama-server 各臂，计算术语命中率/格式保留率/译文有效性/后端标注，输出逐例 JSON | 云端需 Key；本地需 llama-server.exe + GGUF |
| `make_corpus.py` | S-12 语料生成：自建 30 个 HTML（文字 8/图表 8/商品 7/代码 7）→ headless Edge 真实截图 + `corpus.json` 元数据 | 需 msedge.exe |
| `bench_image_search_trust.py` | S-12 主基准：真实四段流水线（视觉识别 → 检索词 → 官方 Search → 摘要）+ 四段/检索词/来源可访问/来源真实性/不确定标注检查；`--rescore` 可对已存输出离线重评分 | 需 Key；语料需先生成 |
| `build_data_summary.py` | 把 `.verify/spikes/S-12-S-14/raw/` 最新结果聚合到 `docs/spikes/data/s12_s14/` 并生成 README | — |

原始数据目录：`.verify/spikes/S-12-S-14/`（`raw/`、`corpus/html|shots`、`logs/`、`raw/llama-logs/`）；
精选聚合：`docs/spikes/data/s12_s14/`。

## 2 Key 放置与纪律（S-12 / S-14 云端部分）

- Key **只允许**放在 `.verify/p1/key.txt`（单行、gitignored）或环境变量 `SNIPASK_BENCH_API_KEY`（临时）。
- 脚本只从这两处读取，**不打印、不写日志、不写文档/JSON**；输出 JSON 只含 `api_key_provided: true` 与脱敏 host。
- **绝对不要**把 Key 写入 `scripts/`、`docs/`、任何 git 跟踪文件或分享日志。
- 无 Key 时：`--mock-self-check` 可验证框架；不带 mock 直接跑会打印 BLOCKED（退出码 2）并给出解锁步骤。

自检云端连通性（不落盘 Key）：

```powershell
$k = (Get-Content .verify/p1/key.txt -Raw).Trim()
Invoke-WebRequest -Headers @{ Authorization = "Bearer $k" } https://api.deepseek.com/models -UseBasicParsing
```

## 3 S-14 翻译一致性（术语 + 格式 + 后端标注）

### 3.1 样例与判分

- 24 条样例（中→英 12 / 英→中 12）：普通段落、多行、无序/有序列表、代码块、URL+邮箱、占位符、版本号/标识符、
  术语命中、空行段落、错误码、长段落；每条样例带 `required_terms`（术语表命中词）与逐项 `format_checks`。
- **术语命中率** = 命中 required_terms / 总数（目标 100%）。
- **格式保留率** = 通过格式检查项 / 总数，按 `lines_exact` / `blank_lines_min` / `bullets_min` / `numbered_min` /
  `code_fences_exact` / `contains`（URL/邮箱/占位符/标识符/代码内容）逐项统计（目标 100%）。
- **译文有效性**（辅助，识别「复述原文」失败模式）：中→英 CJK 占比 <30%；英→中 CJK >15% 且不等于原文。
- **后端标注**：每条记录写 `backend` + `backend_label`（如 `cloud:deepseek-flash/translate.md`、
  `local:qwen2.5-0.5b-instruct-q4/translate_minimal.md`），100% 标注。

### 3.2 臂（backend × 提示词 × 模型）

| 臂 | 后端 | 模型 | 提示词 | 说明 |
| --- | --- | --- | --- | --- |
| `cloud` | 云端 DeepSeek | `deepseek-flash`（thinking disabled） | 产品 `translate.md` | 产品默认云端路径 |
| `local-tiny-minimal` | 本地 llama-server | Qwen2.5-0.5B Q4 | 产品 `translate_minimal.md`（BCP-47 语言码） | 极小档推荐口径 |
| `local-tiny-minimal-names` | 本地 | 0.5B | 同上，语言码→「中文/英文」 | 诊断臂（定位语言码问题） |
| `local-tiny-product` | 本地 | 0.5B | 产品 `translate.md` | 对照（复现 S-05「复述原文」） |
| `local-tiny-minimal-format` | 本地 | 0.5B | minimal + 正文末尾追加格式守卫句 | 实验臂（验证守卫句位置是否有害） |
| `local-15b-minimal` | 本地 | Qwen2.5-1.5B Q4 | 产品 `translate_minimal.md` | 质量优先档候选 |
| `local-tiny-minimal-v2` | 本地 | 0.5B | 候选修复版（语言名称 + 术语并入指令 + `<<< >>>` 分隔 + 前置格式约束） | 处置复核臂 |
| `local-15b-minimal-v2` | 本地 | 1.5B | 候选修复版 | 处置复核臂 |
| `local-tiny-minimal-v3` | 本地 | 0.5B | 候选修复版 v3（只修「术语表标题行被复述」结构缺陷） | 处置复核臂 |
| `local-15b-minimal-v3` | 本地 | 1.5B | 候选修复版 v3 | 处置复核臂（本地最优） |

### 3.3 命令

```powershell
# 本地极小档（无 Key 也可跑）
python scripts/spikes/s12_s14/bench_translate_consistency.py --arms local-tiny-minimal

# 云端（需 Key；无 Key 时该臂 BLOCKED，本地臂继续）
python scripts/spikes/s12_s14/bench_translate_consistency.py --arms cloud

# 多臂一次跑（推荐安静窗口；llama-server 默认端口 39281，被占用时自动选空闲端口）
python scripts/spikes/s12_s14/bench_translate_consistency.py `
  --arms cloud,local-tiny-minimal,local-tiny-minimal-names,local-tiny-product,`
local-tiny-minimal-format,local-15b-minimal,local-tiny-minimal-v2,local-15b-minimal-v2 --threads 8

# 可选：指定 llama-server / GGUF
$env:LLAMA_SERVER_EXE = "$env:USERPROFILE\.lmstudio\extensions\backends\llama.cpp-win-x86_64-avx2-2.28.2\llama-server.exe"
$env:LLAMA_MODEL_GGUF = "$PWD\.verify\spikes\S-05-S-06\artifacts\qwen2.5-0.5b-instruct-q4_k_m.gguf"

# 无 Key 框架自检（MOCK，不作验收）
python scripts/spikes/s12_s14/bench_translate_consistency.py --mock-self-check
```

- llama-server 启动参数：`-m <gguf> -c 4096 -t <threads> --port <free> --host 127.0.0.1 --no-webui`，
  启动后轮询 `/health`；跑完 `taskkill /T /F` 并按「启动前 PID 基线」清理残留，输出 `shutdown.clean`。
- 采样：云端 temperature=0（thinking disabled），本地 temperature=0.2（产品 provider 默认）、top_p=0.95、max_tokens=2048。
- **资源纪律**：本地臂占 CPU。若其他成员正在跑 OCR/编译基准，请等安静窗口再跑（S-14 不测时延，但并发会导致
  llama-server 请求失败、数据无效）；跑完确认 `Get-Process llama-server` 无残留。

## 4 S-12 图片搜索可信度

```powershell
# 0) 生成 30 张语料（headless Edge）
python scripts/spikes/s12_s14/make_corpus.py

# 1) 无 Key 框架自检（MOCK，不作验收；不联网）
python scripts/spikes/s12_s14/bench_image_search_trust.py --mock-self-check

# 2) 真实 30 张四段流水线（需 Key；含来源 HEAD/GET 8s 实测）
python scripts/spikes/s12_s14/bench_image_search_trust.py --limit 30

# 3) 对已保存输出做离线重评分（修正检查口径时不重复调用付费接口）
python scripts/spikes/s12_s14/bench_image_search_trust.py --rescore <raw.json>

# 4) 聚合精选数据
python scripts/spikes/s12_s14/build_data_summary.py
```

- 检查定义见 `docs/spikes/S-12-图片搜索可信度.md` §2.3：四段齐全 / 检索词相关性 / 来源可访问率（HEAD 回退 GET，
  8s 超时，2xx/3xx）/ 来源真实性（URL 身份 `scheme://host/path`，编造与截断分别计数）/ 不确定标注。
- 来源可访问率同时输出三种口径：raw、剔除反爬/网络不可达、把反爬/网络视为可访问的上界。
- 语料元数据（`expected_query_terms`、`recognizable_content`、`expect_uncertain`）随 `corpus.json` 一起产出，
  便于复算与扩测。

## 5 本轮实测摘要

见：
- `docs/spikes/S-14-翻译一致性.md`（术语/格式/后端标注，本地 vs 云端对照）
- `docs/spikes/S-12-图片搜索可信度.md`（四段/检索词/来源/不确定标注，30 张）
- `docs/spikes/data/s12_s14/summary_s12_s14.json`（机器可读聚合）

关键数字（2026-11-05，真实链路）：

- **S-14 云端 DONE**：`deepseek-flash + translate.md`，术语 **15/15=100%**、格式 **47/47=100%**、24/24 请求成功。
- **S-14 本地 FAILED**：0.5B 各配置术语 ≤46.7%、格式 ≤76.6%、译文有效 ≤66.7%；1.5B + minimal 术语 **100%** 但格式
  **68.1%**（根因：产品 `translate_minimal.md` 的「术语表」标题行被模型复述，多出一行）；候选 v3 把 1.5B 格式
  提到 **87.2%**、术语 80%，仍不达 100%。
- **S-12 FAILED（来源维度）**：四段 **30/30=100%**、不确定标注 **4/4=100%**；来源可访问率 raw **59.6%**
  （客观失效 8 / 反爬 26 / 网络不可达 21；剔除不可判定 91.0%、乐观上界 94.1%，均 <95%）；来源真实性
  1 条模型改写 URL + 2 条截断；检索词自动关键词 27/30、人工复核 30/30。
- 两个脚本的 `--mock-self-check` 均通过（MOCK 数据，不作验收）。

## 6 退出码与无 Key 行为

| 脚本 | 0 | 1 | 2 | 3 |
| --- | --- | --- | --- | --- |
| `bench_translate_consistency.py` | 至少一个臂完成真实测量（阈值是否达标见 JSON/输出） | — | 全部 BLOCKED（无 Key/无模型） | 异常 |
| `bench_image_search_trust.py` | 真实链路完成 或 mock 自检通过 | — | 无 Key / 无语料 | 异常 |
| `make_corpus.py` | 30 张全部成功 | 有截图失败 | — | — |

mock 输出永远带 `meta.mode = MOCK_SELF_TEST` 与 `labels: ["MOCK"]`（S-14）或 `MOCK_SELF_TEST`（S-12），
控制台打印「非验收数据」。
