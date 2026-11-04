# S-05 / S-06 原始数据与汇总

生成时间：2026-11-05T01:12:08.545018+08:00

本目录文件由 `scripts/spikes/s05_s06/build_data_summary.py` 从 `.verify/spikes/S-05-S-06/raw/` 复制/聚合，均为真实测量输出。

## 文件

- `README.md`
- `cloud_blocked_evidence.json`
- `cloud_effort2_low_smoke.json`
- `cloud_effort_low_smoke.json`
- `cloud_translate_selftest_final.json`
- `cloud_translate_stream20_nothink_20261105-005221.json`
- `cloud_translate_stream_20_20261105-004841.json`
- `dashscope_empty_key_body.json`
- `deepseek_capability_probe_20261105-005322.json`
- `deepseek_empty_key_body.json`
- `download.log`
- `download_qwen15b.log`
- `download_tiny.log`
- `formal_local_20261105-000321.log`
- `formal_local_summary_20261105-000321.json`
- `image_search_mock_20261104-235243.json`
- `image_search_mock_20261104-235528.json`
- `image_search_partial_deepseek_20261105-005527.json`
- `local_translate_20261104-235408.json`
- `local_translate_20261104-235614.json`
- `local_translate_formal_20261105-000321.json`
- `local_translate_sens100_20261105-000321.json`
- `local_translate_tiny_qwen05b_20261105-010106.json`
- `summary_s05_s06.json`
- `tiny_candidate_qwen15b_minimal_20261105-010829.json`
- `tiny_candidate_qwen15b_minimal_30_20261105-010829.json`
- `tiny_variant_minimal-fewshot_200_20261105-010658.json`
- `tiny_variant_minimal-fewshot_30_20261105-010658.json`
- `tiny_variant_minimal_200_20261105-010658.json`
- `tiny_variant_minimal_30_20261105-010658.json`
- `tiny_variant_product_200_20261105-010658.json`
- `tiny_variant_product_30_20261105-010658.json`
- `tune_threads16_20261105-000321.json`
- `tune_threads6_20261105-000321.json`

## 关键口径

- 所有 JSON 由脚本自动生成，未手工改数；mock/PARTIAL 文件在 `meta.mode` 与 `labels` 中明确标注，不得作为验收依据。
- 云端：DeepSeek `deepseek-flash`，stream=true；默认 thinking 与关闭 thinking 两组均记录 TTFT/总耗时。
- 本地 1.8B：正式 10 次线程 16；污染期数据保留为对照。
- 极小模型 Qwen2.5-0.5B：正式 10 次线程 16，记录 tok/s、内存、冷加载与输出样例。
- S-06：DeepSeek 单 Key 视觉可用（真实截图识别成功）但无 Search API（404），验收仍 BLOCKED；PARTIAL 测量只用于估算视觉腿时延。
