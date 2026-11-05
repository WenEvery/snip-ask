# docs/spikes/data/s16_s18 — S-16 / S-18 聚合数据（可入 git，小文件）

原始数据在 `.verify/spikes/S-16-S-18/`（gitignored）；这里只放结论所需的最小聚合数据。

| 文件 | 内容 | 用途 |
| --- | --- | --- |
| `volume_baseline.json` | 2026-11-05 真实构建（commit `3f1b263`）的 NSIS setup / release 主 exe 体积、SHA256、工具链版本、时间 | S-18 体积趋势基线；CI `check_bundle_size.ps1` 用 `-BaselineRef` 对比并打印 Δ |
| `model_size_estimate.json` | 「含模型」两种口径：OCR 模型（PP-OCRv4 ONNX）16,189,007 B；本地 LLM（清单 GGUF 4 条）4,436,333,600 B；含模型安装包估算区间与结论 | S-18「含模型 ≤60MB」结论依据（不实际打包模型） |

复跑/更新：

```powershell
# 重新构建并更新基线（约 3–5 分钟）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s18_build.ps1
# 体积门禁 + 生成/更新基线 JSON（CI 同一脚本）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/check_bundle_size.ps1 -BaselineOut docs/spikes/data/s16_s18/volume_baseline.json
```

说明：`volume_baseline.json` 由 `check_bundle_size.ps1` 生成；更新时请保留 commit 与时间字段，便于趋势追溯。
