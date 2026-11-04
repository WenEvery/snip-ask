# S-17 预置：provider_test 真实连通性冒烟（DeepSeek）

用途：用真实 DeepSeek Key 验证应用「测试连接」的 `/models` 链路（200 / 模型列表 / 耗时 / 错误映射），
并把脱敏原始数据写入 `.verify/spikes/S-17/`。

**Key 只从环境变量读取，绝不写死、不落盘、不进日志。**

## 一键冒烟

```powershell
# 1) 设置真实 Key（仅当前会话环境变量）
$env:SNIPASK_BENCH_API_KEY = '<真实 DeepSeek Key>'

# 2) 跑被忽略的真实冒烟测试（会写 .verify/spikes/S-17/deepseek-smoke-*.json）
cargo test --manifest-path src-tauri/Cargo.toml --test provider_probe_spike -- --ignored --nocapture

# 3) 也可用本目录脚本（等价，额外打印脱敏摘要路径）
pwsh -NoProfile -File scripts/spikes/s17/smoke_deepseek.ps1
```

预期：`deepseek_live_smoke` 打印
`ok=true status=200 latencyMs≈N models=["deepseek-flash", "deepseek-v4-pro", ...]`，
并在 `.verify/spikes/S-17/` 留下不含 Key 的 JSON 记录。若网络/代理不可达，测试会保留失败证据
（`errorCode`/`errorMessage`），按 Lead 口径不算任务失败。

> 2026-11-04 实测（DeepSeek 官方端点，Key 来自环境变量）：`ok=true`、HTTP 200、
> `latencyMs=208ms`（wall 209ms）、模型 `["deepseek-flash", "deepseek-v4-pro"]`；
> 原始数据 `.verify/spikes/S-17/deepseek-smoke-20261104-170000.json`（不含 Key）。

## mock 场景测试（不耗外网）

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test provider_probe_spike
```

覆盖：200 / 401 / 403 / 402 / 429 / 500 / 503 / 404 / 超时 / 连接拒绝 / 尾斜杠 /
非 https 拒绝 / 响应非 JSON / 结果不含 Key（15 个用例，1 个真实冒烟默认 ignored）。

## 错误码映射（契约 §8.2）

| 场景 | 错误码 | retryable | fallback |
| --- | --- | --- | --- |
| 401 / 403 | SA-3002 AuthFailed | 否 | open_settings |
| 402 | SA-3003 QuotaExceeded | 否 | open_settings |
| 429 | SA-3004 RateLimited | 是 | retry |
| 超时 | SA-3005 Timeout | 是 | retry, browser_search |
| DNS / 连接拒绝 / TLS / 5xx | SA-3001 NetworkUnavailable | 是 | browser_search, local_translate（5xx 为 retry） |
| 其他 4xx | SA-5001 InvalidConfig | 否 | reset_group |
| 未配置 Key | SA-3002 AuthFailed（ok=false，不抛 IPC 错） | 否 | open_settings |
