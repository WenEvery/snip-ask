# S-12 / S-14 原始数据与汇总

生成时间：2026-11-05T19:24:49.128148+08:00

本目录文件由 `scripts/spikes/s12_s14/build_data_summary.py` 从
`.verify/spikes/S-12-S-14/` 复制/聚合并生成，均为真实测量输出；mock 文件在文件名与
`meta.mode` 中明确标注，**不得作为验收依据**。

## 文件

- `README.md`
- `summary_s12_s14.json`
- `s14_translate_consistency_real.json`
- `s14_translate_consistency_v3fix.json`
- `s14_translate_consistency_mock.json`
- `s12_image_search_trust_real.json`
- `s12_image_search_trust_mock.json`
- `s12_corpus_manifest.json`
- `s12_example_text.png`
- `s12_example_chart.png`
- `s12_example_product.png`
- `s12_example_code.png`

## 关键口径

- S-14：24 条样例（中→英 12 / 英→中 12），含换行/空行/列表/编号/代码块/URL/邮箱/占位符/标识符/术语；
  术语命中率与格式保留率逐项判定，目标均 100%；每条结果都带 `backend` 与 `backend_label`。
- S-12：30 张 headless Edge 真实渲染截图（文字 8 / 图表 8 / 商品 7 / 代码 7，其中 4 张标记为
  `expect_uncertain`）；四段=识别结果/检索词/结论/来源；来源可访问率为脚本 HEAD/GET 实测（8s 超时）；
  来源真实性=URL 是否出现在本次 Search 返回中（识别编造）。
- 云端 Key 不落盘：`s14_*`/`s12_*` JSON 只含 `api_key_provided` 与脱敏 host。

## 本次关键数字

- S-14 `cloud:deepseek-flash/translate.md`：REAL / DONE；术语 1.0、格式 1.0、译文有效 0.9583、整例 23/24
- S-14 `local:qwen2.5-0.5b-instruct-q4/translate_minimal.md`：REAL / FAILED；术语 0.2667、格式 0.766、译文有效 0.2917、整例 5/24
- S-14 `local:qwen2.5-0.5b-instruct-q4/translate_minimal+lang-names`：REAL / FAILED；术语 0.4667、格式 0.766、译文有效 0.6667、整例 9/24
- S-14 `local:qwen2.5-0.5b-instruct-q4/translate.md`：REAL / FAILED；术语 0.0、格式 0.7872、译文有效 0.2083、整例 2/24
- S-14 `local:qwen2.5-0.5b-instruct-q4/translate_minimal+format`：REAL / FAILED（存在请求失败）；术语 1.0、格式 0.6、译文有效 0.5714、整例 1/24
- S-14 `local:qwen2.5-1.5b-instruct-q4/translate_minimal.md`：REAL / FAILED；术语 1.0、格式 0.6809、译文有效 1.0、整例 10/24
- S-14 `local:qwen2.5-0.5b-instruct-q4/translate_minimal_v2`：REAL / FAILED；术语 0.2667、格式 0.7447、译文有效 0.2917、整例 4/24
- S-14 `local:qwen2.5-1.5b-instruct-q4/translate_minimal_v2`：REAL / FAILED；术语 0.6、格式 0.8936、译文有效 0.7917、整例 10/24
- S-14 处置复核 `local:qwen2.5-1.5b-instruct-q4/translate_minimal_v3`：REAL / FAILED；术语 0.8、格式 0.8723、译文有效 0.9167、整例 16/24
- S-14 处置复核 `local:qwen2.5-0.5b-instruct-q4/translate_minimal_v3`：REAL / FAILED；术语 0.2667、格式 0.6596、译文有效 0.25、整例 3/24
- S-12 真实链路：四段 1.0、检索词相关 0.9、来源可访问 raw=0.5956（剔除不可判定 0.9101、乐观上界 0.9412；客观失效 8 / 反爬 26 / 网络 21）、不确定标注 1.0；判定 FAILED
