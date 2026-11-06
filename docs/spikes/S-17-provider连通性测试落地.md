# S-17 预置：provider_test 真实连通性测试落地

> 对应 task-11；契约 §6 第 17 条（`provider_test`）/ §8.2（错误码表）。
> 本记录只写实现与测试，不改 `docs/技术验证清单.md` 的 P0 状态表（S-17 正式结论仍待完整验证）。
> 原始数据：`.verify/spikes/S-17/`（gitignored）；脚本：`scripts/spikes/s17/`。

## 结论

**实现完成 + mock 场景测试全绿；真实 DeepSeek 冒烟见下文。**

- `skeletonMode = true`：保持契约 §12 的确定性成功（180ms 占位），现有依赖 skeleton 的测试/引导流程行为不变；
- `skeletonMode = false`：走真实 `GET {baseUrl}/models`（OpenAI 兼容），带 `Authorization: Bearer <key>`、
  `User-Agent: snip-ask/<version>`、连接 5s / 总 15s 超时，返回契约 `TestResult`；
- Key 读取：优先 `draft.apiKey`，为空读 `CredentialService`（`SnipAsk/llm/<providerId>`）；
  两处都空 → `ok=false` + `SA-3002`（可操作提示「请先输入 Key」），不抛 IPC 异常；
- 测试：`src-tauri/tests/provider_probe_spike.rs` 用 `std::net::TcpListener` 起本地 mock server，
  覆盖 200 / 401 / 403 / 402 / 429 / 500 / 503 / 404 / 超时 / 连接拒绝 / 尾斜杠 / 非 https 拒绝 /
  响应非 JSON / 结果不含 Key，共 **15 个常规用例 + 1 个默认 ignored 的真实冒烟**；
- 真实 DeepSeek 冒烟：**PASS —— `ok=true`、HTTP 200、`/models` 返回 `deepseek-flash` / `deepseek-v4-pro` 两个模型，`TestResult.latencyMs=208ms`（wall 209ms）**，详见下文「真实 DeepSeek 冒烟」。

## 实现

文件：`src-tauri/src/commands/provider.rs`

1. **校验不变**：复用 `validate()`（baseUrl 仅 https 或 `http://127.0.0.1`、id 格式、模型名非空），
   新增 `validate_probe_draft(config, draft)` 供命令与测试共用；
2. **URL**：`models_url(base_url) = base_url.trim_end_matches('/') + "/models"`
   （DeepSeek 即 `https://api.deepseek.com/models`）；
3. **请求**：`reqwest::Client`（native-tls、system-proxy），`connect_timeout=5s`、`timeout=15s`，
   `Authorization: Bearer`、`Accept: application/json`、UA `snip-ask/<CARGO_PKG_VERSION>`；
4. **结果**：
   - 2xx：`ok=true, status, latencyMs, model`（优先配置模型名，若不在 `/models` 列表中则取列表首项）；
     额外解析 `available_models` 供日志/冒烟记录（不进入契约出参）；
   - 非 2xx：`ok=false, status, latencyMs, error=AppError`；
   - 传输层失败（超时/DNS/连接拒绝/TLS）：`ok=false, status=null, latencyMs, error=AppError`（同样以
     `TestResult` 返回，前端能显示带 `fallback` 的可操作提示，而不是只弹 IPC 错误）；
   - 防御性 URL 白名单校验（绕过配置层直接调用也拒绝非 https / 非 127.0.0.1）。
5. **Key 保存行为不变**：仅当 `result.ok` 且 `draft.apiKey` 非空时写凭据存储并广播 `config:changed`；
   失败不覆盖已有 Key。
6. **日志脱敏**：只记录 `provider/host/ok/status/latencyMs/model/model_count`，不记录 Key 与响应体原文。

## 错误码映射（契约 §8.2，100% 由测试断言 code / retryable / fallback）

| 场景 | HTTP | 错误码 | retryable | fallback | 前端提示（`actionableHint`） |
| --- | --- | --- | --- | --- | --- |
| Key 无效/无权限 | 401 / 403 | `SA-3002` AuthFailed | false | `open_settings` | 「Key 无效或没有权限…」 |
| 额度不足 | 402 | `SA-3003` QuotaExceeded | false | `open_settings` | 「账户额度不足…」 |
| 限流 | 429 | `SA-3004` RateLimited | **true** | `retry` | 「请求被限流：稍后再试…」 |
| 超时 | —（客户端超时） | `SA-3005` Timeout | **true** | `retry`, `browser_search` | 「连接超时：检查网络 / 代理…」 |
| DNS / 连接拒绝 / TLS | — | `SA-3001` NetworkUnavailable | **true** | `browser_search`, `local_translate` | 「网络不可用…」 |
| 服务端错误 | 5xx | `SA-3001` NetworkUnavailable | **true** | `retry` | 「网络不可用…」（可重试） |
| 其他 4xx | 404 等 | `SA-5001` InvalidConfig | false | `reset_group` | 配置非法 |
| 未配置 Key | — | `SA-3002` AuthFailed（`ok=false`） | false | `open_settings` | 「先填写 Key」 |

> **与 task-11 文本的偏差说明**：task 文本写「402/429→SA-3003；超时→SA-3001」，Lead 已确认以契约 §8.2
> 为准：429 用独立的 `SA-3004`（retryable）、超时用 `SA-3005`（retryable），否则前端
> `AiServiceStep.actionableHint()` 会把限流显示成「额度不足」、把超时显示成「网络不可用」。

## 测试

文件：`src-tauri/tests/provider_probe_spike.rs`（mock server：`std::net::TcpListener` + 独立线程，
记录请求行/请求头，按行为返回响应或挂起）。

| 用例 | 断言要点 |
| --- | --- |
| `success_200_returns_ok_with_models_and_sends_auth_headers` | `ok/status=200/model=deepseek-flash/latencyMs`；请求行 `GET /models HTTP/1.1`；`Authorization: Bearer <key>`；`User-Agent: snip-ask/…`；解析出 2 个模型 id |
| `model_falls_back_to_first_available_when_preferred_missing` | 配置模型不在列表 → 取列表首项 |
| `success_200_with_non_json_body_still_ok` | 2xx 但响应非 JSON 仍 `ok=true`，model 回退配置值 |
| `maps_401_and_403_to_auth_failed` | `SA-3002`、retryable=false、fallback 含 `open_settings` |
| `maps_402_to_quota_exceeded` | `SA-3003`、retryable=false、fallback 含 `open_settings` |
| `maps_429_to_rate_limited_retryable` | `SA-3004`、retryable=true、fallback 含 `retry` |
| `maps_5xx_to_retryable_network_error` | 500/503 → `SA-3001`、retryable=true、fallback 含 `retry` |
| `maps_other_4xx_to_invalid_config` | 404 → `SA-5001`、retryable=false、fallback 含 `reset_group` |
| `maps_timeout_to_timeout_error` | 服务器收请求不回包 + 300ms 注入超时 → `SA-3005`、retryable=true、无 status，且实际在 2s 内返回 |
| `maps_connection_refused_to_network_unavailable` | 无监听端口 → `SA-3001`、retryable=true、无 status |
| `trailing_slash_is_normalized_to_single_models_path` | `''`/`'/'`/`'///'` 三种尾斜杠都请求 `/models`；`models_url` 纯函数断言 |
| `rejects_non_https_base_url` | `http://api.deepseek.com`、`ftp://…` → `SA-5001`（detail 指向 `providers.baseUrl`）；`http://127.0.0.1` 放行 |
| `rejects_disallowed_url_before_network` | 绕过配置层直接调用探测也不向非白名单地址发请求（`SA-5001`） |
| `missing_api_key_result_is_actionable_auth_error` | 缺 Key 分支 `ok=false`、`SA-3002`、fallback `open_settings`、message 含 Key 指引 |
| `test_result_never_contains_api_key` | 成功/失败路径的 `TestResult` 均不含完整 Key 或 `sk-` 片段 |
| `deepseek_live_smoke`（ignored） | 从 `SNIPASK_BENCH_API_KEY` 读 Key；`/models` 200 且包含 `deepseek-flash`；写脱敏 JSON 到 `.verify/spikes/S-17/`；网络失败保留证据不算失败 |

运行（mock，不需要外网/Key）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test provider_probe_spike
```

## 真实 DeepSeek 冒烟（脱敏）

命令（Key 只从环境变量读，不写死、不落盘）：

```powershell
$env:SNIPASK_BENCH_API_KEY = '<真实 DeepSeek Key>'
cargo test --manifest-path src-tauri/Cargo.toml --test provider_probe_spike -- --ignored --nocapture
# 或：pwsh -NoProfile -File scripts/spikes/s17/smoke_deepseek.ps1
```

结果：

- 时间：`2026-11-04T17:00:00Z`（本机 2026-11-05 01:00:00 +08:00；等价命令见上）
- 端点：`GET https://api.deepseek.com/models`
- `ok` / `status`：`true` / `200`
- `latencyMs`（`TestResult`）/ `wallMs`（测试总耗时）：`208` / `209`
- `model`（`TestResult.model`）：`deepseek-flash`
- `/models` 模型 id（数量 2）：`["deepseek-flash", "deepseek-v4-pro"]`
- 原始数据：`.verify/spikes/S-17/deepseek-smoke-20261104-170000.json`
- 泄漏检查：原始数据与 `TestResult` 均不含完整 Key / `sk-` 片段（`apiKeyProvided: true` 仅布尔值）。
- Key 处理：仅环境变量输入；原始 JSON 记录不含 Key（`apiKeyProvided: true` 仅布尔值）。

## 安全

- 真实 Key 不在代码/文档/日志/`TestResult`/测试输出中出现；`scripts/spikes/s17/smoke_deepseek.ps1`
  还会对 cargo 输出做一次 `Key → ***` 的防御性替换；
- `provider_test` 只在请求头中使用 Key；错误 detail 只含 host 与 HTTP 状态，不含响应体原文（避免泄漏）；
- 测试用假 Key（`sk-test-probe-not-a-real-key`）仅用于断言请求头与「不回显」，不是真实凭据。

## 改动文件

- `src-tauri/src/commands/provider.rs`：真实连通性实现（`probe_provider` / `probe_provider_detail` /
  `ProbeOptions` / `ProbeOutcome` / `validate_probe_draft` / `models_url` / 错误映射 / Key 解析 / 缺 Key 结果）；
  skeleton 分支与 Key 保存逻辑保持原语义。
- `src-tauri/tests/provider_probe_spike.rs`（新增）：mock server 场景测试 + 真实冒烟（ignored）。
- `scripts/spikes/s17/README.md`、`scripts/spikes/s17/smoke_deepseek.ps1`（新增）。
- 本文档。

契约字段未改（`TestResult` / `ProviderDraft` / `AppError` 原样）。

## 遗留 / 建议

1. 自动重试退避（429/5xx ≤2 次）尚未实现：测试连接是用户主动操作、失败信息已可操作，暂不自动重试；
   若后续接入真实对话链路，建议在网关层统一实现退避（S-17 完整验证时评估）。
2. `/models` 响应只用于展示与选择模型，不校验用户填写的模型名是否真实存在（部分供应商不提供 `/models`
   或权限不足返回 404）；若需要强校验，可在 S-17 完整验证时补充「模型探测失败但补全接口可用」的分支。
3. 真实冒烟需要外网与有效 Key，未纳入 CI；只在有 Key 的环境手工执行（Key 由环境变量注入）。
