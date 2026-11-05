//! 供应商连通性测试命令（契约 §6 第 17 条 / §8.2 错误码）。
//!
//! 契约修订语义：`provider_test` 兼作密钥保存通道 ——
//! `draft.apiKey` 非空且测试成功时写入凭据存储（骨架模式为内存实现），
//! 使 `ConfigView.providers[].hasKey / keyMask` 立即反映；测试失败不覆盖已有密钥。
//!
//! 真实连通性（skeletonMode = false）：
//! - OpenAI 兼容 `GET {baseUrl}/models`（DeepSeek 即 `https://api.deepseek.com/models`）；
//! - `Authorization: Bearer <apiKey>`、`User-Agent: snip-ask/<version>`、连接 5s / 总 15s 超时；
//! - 结果映射见 [`probe_provider`] 与 [`map_http_status`]：401/403→SA-3002、
//!   402→SA-3003、429→SA-3004、超时→SA-3005、DNS/连接失败/5xx→SA-3001、其他 4xx→SA-5001。
//!   说明：task 描述把 429/超时归并到 3003/3001；这里按契约 §8.2 的独立错误码映射，
//!   与前端 `AiServiceStep.actionableHint()` 现有分支一致（详见 `docs/spikes/S-17-*.md`）。
//!
//! Key 读取：优先 `draft.apiKey`，为空时读凭据存储里该 provider 已保存的 Key；
//! 两处都没有时返回 `ok=false` + `SA-3002`（可在引导页/设置页直接看到「先填写 Key」）。
//!
//! 安全：Key 只用于请求头，不写日志、不进 `TestResult`、不回显。

use std::time::{Duration, Instant};

use reqwest::StatusCode;
use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::config::{validate, Config, Provider, ProviderKind};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::ConfigChangedPayload;
use crate::contract::types::{ProviderDraft, TestResult};
use crate::events::emit_config_changed;
use crate::services::translator::QwenMtTranslator;

/// 真实连通性测试：连接超时。
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 真实连通性测试：总超时。
const PROBE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 探测超时参数（可注入，便于单测/集成测试使用短超时）。
#[derive(Debug, Clone, Copy)]
pub struct ProbeOptions {
    /// 连接超时。
    pub connect_timeout: Duration,
    /// 请求总超时。
    pub request_timeout: Duration,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            connect_timeout: PROBE_CONNECT_TIMEOUT,
            request_timeout: PROBE_REQUEST_TIMEOUT,
        }
    }
}

/// 探测明细（`TestResult` + `/models` 返回的可用模型 id 列表，供日志/冒烟记录）。
#[derive(Debug, Clone)]
pub struct ProbeOutcome {
    /// 契约 §6 返回结构。
    pub result: TestResult,
    /// `/models` 响应里的模型 id 列表（解析失败为空）。
    pub available_models: Vec<String>,
}

/// 拼接 OpenAI 兼容的模型列表 URL：去掉 baseUrl 末尾斜杠后追加 `/models`。
pub fn models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim_end_matches('/'))
}

/// 校验探测草稿：复用配置校验（baseUrl 仅允许 https 或 http://127.0.0.1、id 格式等）。
pub fn validate_probe_draft(config: &Config, draft: &ProviderDraft) -> AppResult<()> {
    let provider = Provider {
        id: draft.id.clone(),
        name: draft.name.clone(),
        base_url: draft.base_url.clone(),
        model: draft.model.clone(),
        kind: draft.kind,
        caps: draft.caps.clone(),
        temperature: draft.temperature.unwrap_or(0.2),
        max_tokens: draft.max_tokens.unwrap_or(1024),
        enabled: true,
        extra: serde_json::Map::new(),
    };
    let mut probe_config = config.clone();
    probe_config.providers = vec![provider];
    validate(&probe_config)
}

/// 未配置 Key 时的确定性失败结果（`ok=false` + SA-3002），供前端给出可操作提示。
pub fn missing_api_key_result() -> TestResult {
    TestResult {
        ok: false,
        latency_ms: None,
        model: None,
        status: None,
        error: Some(
            AppError::new(
                ErrorCode::AuthFailed,
                Some("draft.apiKey 与凭据存储中均无该供应商 Key".to_string()),
            )
            .with_message("尚未填写 API Key：请先输入 Key 再测试连接")
            .with_fallback(vec!["open_settings"]),
        ),
    }
}

/// 选择要展示的模型：优先配置里的模型名（若出现在 `/models` 中），否则取列表首项，
/// 再退化为配置的模型名（响应无法解析时）。
fn pick_model(preferred: Option<&str>, available: &[String]) -> Option<String> {
    if let Some(preferred) = preferred {
        if available.iter().any(|item| item == preferred) {
            return Some(preferred.to_string());
        }
    }
    available
        .first()
        .cloned()
        .or_else(|| preferred.map(str::to_string))
}

/// 解析 `/models` 响应体里的模型 id 列表（OpenAI 兼容：`{"data":[{"id":"..."}]}`）。
fn parse_model_ids(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .get("data")
        .and_then(|data| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// HTTP 状态码 → 契约错误码映射（非 2xx 时使用，100% 由测试断言）。
pub fn map_http_status(status: StatusCode, base_url: &str) -> AppError {
    let code = status.as_u16();
    let host = host_of(base_url);
    let http_detail = format!(
        "GET {}/models 返回 HTTP {code}（host={host}）",
        base_url.trim_end_matches('/')
    );
    match code {
        401 | 403 => AppError::new(ErrorCode::AuthFailed, Some(http_detail))
            .with_message(format!(
                "API Key 无效或没有权限（HTTP {code}）：请检查 Key 是否完整、与所选供应商匹配"
            ))
            .with_fallback(vec!["open_settings"]),
        402 => AppError::new(ErrorCode::QuotaExceeded, Some(http_detail))
            .with_message("账户额度不足（HTTP 402）：请充值，或先换用其他供应商")
            .with_fallback(vec!["open_settings"]),
        429 => AppError::new(ErrorCode::RateLimited, Some(http_detail))
            .with_message("请求被限流（HTTP 429）：请稍后重试，或降低调用频率")
            .with_retryable(true)
            .with_fallback(vec!["retry"]),
        500..=599 => AppError::new(ErrorCode::NetworkUnavailable, Some(http_detail))
            .with_message(format!("供应商服务端错误（HTTP {code}）：可稍后重试"))
            .with_retryable(true)
            .with_fallback(vec!["retry"]),
        _ => AppError::new(ErrorCode::InvalidConfig, Some(http_detail))
            .with_message(format!(
                "供应商接口返回 HTTP {code}：请检查 Base URL 是否正确（应为 OpenAI 兼容地址）"
            ))
            .with_context(serde_json::json!({ "status": code, "field": "providers.baseUrl" })),
    }
}

/// 传输层错误（超时 / DNS / 连接失败）→ 契约错误码映射。
///
/// 判定顺序有讲究：Windows 上「连接被拒绝」的错误有时也会被标记为 timeout 类，
/// 因此先判 `is_connect()`（连接/DNS 失败 → SA-3001），再判 `is_timeout()`（超时 → SA-3005）。
pub fn map_transport_error(err: &reqwest::Error, base_url: &str) -> AppError {
    let host = host_of(base_url);
    let detail = format!("请求 {} 失败：{err}", models_url(base_url));
    if err.is_connect() {
        return network_unavailable_error(detail, &host);
    }
    if err.is_timeout() {
        return AppError::new(ErrorCode::Timeout, Some(detail))
            .with_message("连接超时：检查网络 / 代理，并确认 Base URL 是否正确")
            .with_retryable(true)
            .with_fallback(vec!["retry", "browser_search"])
            .with_context(serde_json::json!({ "host": host }));
    }
    network_unavailable_error(detail, &host)
}

fn network_unavailable_error(detail: String, host: &str) -> AppError {
    // DNS 解析失败 / 连接拒绝 / TLS 握手失败等都归为网络不可用。
    AppError::new(ErrorCode::NetworkUnavailable, Some(detail))
        .with_message("网络不可用：无法连接供应商，请检查网络 / 代理 / Base URL")
        .with_retryable(true)
        .with_fallback(vec!["browser_search", "local_translate"])
        .with_context(serde_json::json!({ "host": host }))
}

/// 仅取 host 用于错误上下文/日志（不含 query / path）。
fn host_of(base_url: &str) -> String {
    url::Url::parse(base_url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "<invalid>".to_string())
}

/// 真实连通性探测（含 `/models` 明细），供命令与冒烟测试使用。
///
/// 安全：`api_key` 只进入请求头；返回/日志中不含 Key。
pub async fn probe_provider_detail(
    base_url: &str,
    api_key: &str,
    preferred_model: Option<&str>,
    options: ProbeOptions,
) -> AppResult<ProbeOutcome> {
    // 防御性 URL 校验：配置层已用同一白名单校验，此处保证独立调用同样拒绝非 https / 非本机。
    let parsed = url::Url::parse(base_url).map_err(|err| {
        AppError::invalid_config("providers.baseUrl", format!("不是合法 URL：{err}"))
    })?;
    let scheme_ok = parsed.scheme() == "https"
        || (parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1"));
    if !scheme_ok {
        return Err(AppError::invalid_config(
            "providers.baseUrl",
            "仅允许 https 或 http://127.0.0.1",
        ));
    }

    let client = crate::platform::http::client_builder_for(base_url)
        .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(options.connect_timeout)
        .timeout(options.request_timeout)
        .build()
        .map_err(|err| {
            AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(format!("HTTP 客户端初始化失败：{err}")),
            )
        })?;

    let url = models_url(base_url);
    let started = Instant::now();
    let response = match client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            // 传输层失败（超时 / DNS / 连接拒绝 / TLS）也以 TestResult 返回，
            // 让引导页/设置页能显示 AppError 的可操作提示，而不是抛 IPC 异常。
            let latency_ms = elapsed_ms(started);
            return Ok(ProbeOutcome {
                result: TestResult {
                    ok: false,
                    latency_ms: Some(latency_ms),
                    model: None,
                    status: None,
                    error: Some(map_transport_error(&err, base_url)),
                },
                available_models: Vec::new(),
            });
        }
    };
    let latency_ms = elapsed_ms(started);
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    let result = if status.is_success() {
        let available_models = parse_model_ids(&body);
        TestResult {
            ok: true,
            latency_ms: Some(latency_ms),
            model: pick_model(preferred_model, &available_models),
            status: Some(status.as_u16()),
            error: None,
        }
    } else {
        TestResult {
            ok: false,
            latency_ms: Some(latency_ms),
            model: None,
            status: Some(status.as_u16()),
            error: Some(map_http_status(status, base_url)),
        }
    };

    Ok(ProbeOutcome {
        available_models: parse_model_ids(&body),
        result,
    })
}

/// 真实连通性探测（`TestResult` 视图；命令使用）。
pub async fn probe_provider(
    base_url: &str,
    api_key: &str,
    preferred_model: Option<&str>,
    options: ProbeOptions,
) -> AppResult<TestResult> {
    Ok(
        probe_provider_detail(base_url, api_key, preferred_model, options)
            .await?
            .result,
    )
}

/// 探测 DashScope Qwen-MT 原生端点（契约 v5：`Provider.kind = qwenMt`）。
///
/// 健康检查方式：用一条极短的英译中请求打真实原生端点
/// （`POST {base}/api/v1/services/aigc/text-generation/generation`），
/// 校验 HTTP 200 且 `output.choices[0].message.content` 非空。
/// 该请求会产生极少量翻译 token 费用（可接受；不会泄露 Key）。
pub async fn probe_qwen_mt_detail(
    base_url: &str,
    api_key: &str,
    preferred_model: Option<&str>,
    options: ProbeOptions,
) -> AppResult<ProbeOutcome> {
    let parsed = url::Url::parse(base_url).map_err(|err| {
        AppError::invalid_config("providers.baseUrl", format!("不是合法 URL：{err}"))
    })?;
    let scheme_ok = parsed.scheme() == "https"
        || (parsed.scheme() == "http" && parsed.host_str() == Some("127.0.0.1"));
    if !scheme_ok {
        return Err(AppError::invalid_config(
            "providers.baseUrl",
            "仅允许 https 或 http://127.0.0.1",
        ));
    }
    let model = preferred_model
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("qwen-mt-lite")
        .to_string();
    let endpoint = crate::services::translator::qwen_mt_generation_url(base_url);
    let body = serde_json::json!({
        "model": model,
        "input": { "messages": [{ "role": "user", "content": "Hello" }] },
        "parameters": {
            "translation_options": { "source_lang": "English", "target_lang": "Chinese" },
            "result_format": "message",
        },
    });
    let payload = serde_json::to_string(&body)
        .map_err(|err| AppError::invalid_config("providers", format!("请求体序列化失败：{err}")))?;
    let client = crate::platform::http::client_builder_for(base_url)
        .user_agent(concat!("snip-ask/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(options.connect_timeout)
        .timeout(options.request_timeout)
        .build()
        .map_err(|err| {
            AppError::new(
                ErrorCode::NetworkUnavailable,
                Some(format!("HTTP 客户端初始化失败：{err}")),
            )
        })?;
    let started = Instant::now();
    let response = match client
        .post(&endpoint)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let latency_ms = elapsed_ms(started);
            return Ok(ProbeOutcome {
                result: TestResult {
                    ok: false,
                    latency_ms: Some(latency_ms),
                    model: None,
                    status: None,
                    error: Some(QwenMtTranslator::map_transport_error(&err)),
                },
                available_models: Vec::new(),
            });
        }
    };
    let latency_ms = elapsed_ms(started);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Ok(ProbeOutcome {
            result: TestResult {
                ok: false,
                latency_ms: Some(latency_ms),
                model: None,
                status: Some(status.as_u16()),
                error: Some(QwenMtTranslator::map_http_error(status.as_u16(), &text)),
            },
            available_models: Vec::new(),
        });
    }
    // HTTP 200 也可能是 DashScope 业务错误；以原生 status_code/code 判定。
    match QwenMtTranslator::parse_response(&text) {
        Ok(_) => Ok(ProbeOutcome {
            result: TestResult {
                ok: true,
                latency_ms: Some(latency_ms),
                model: Some(model.clone()),
                status: Some(status.as_u16()),
                error: None,
            },
            available_models: vec![model],
        }),
        Err(error) => Ok(ProbeOutcome {
            result: TestResult {
                ok: false,
                latency_ms: Some(latency_ms),
                model: None,
                status: Some(status.as_u16()),
                error: Some(error),
            },
            available_models: Vec::new(),
        }),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 读取探测用 Key：优先 `draft.apiKey`，为空则读凭据存储里该 provider 已保存的 Key。
async fn resolve_api_key(state: &AppState, draft: &ProviderDraft) -> AppResult<Option<String>> {
    if let Some(secret) = draft
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
    {
        return Ok(Some(secret.to_string()));
    }
    state.credential_service().provider_key(&draft.id).await
}

/// 命令：测试供应商连通性（成功时顺带保存 API Key）。
#[tauri::command]
pub async fn provider_test(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> AppResult<TestResult> {
    // 1) 校验草稿：复用配置校验（baseUrl 白名单、id 格式等）。
    validate_probe_draft(&state.snapshot_config(), &draft)?;

    if draft.model.trim().is_empty() {
        return Err(AppError::invalid_config("model", "模型名不能为空"));
    }

    // 2) 连通性测试：骨架模式保持契约 §12 的确定性成功；真实模式请求 `{baseUrl}/models`。
    let result = if state.is_skeleton() {
        tokio::time::sleep(Duration::from_millis(180)).await;
        TestResult {
            ok: true,
            latency_ms: Some(180),
            model: Some(draft.model.clone()),
            status: Some(200),
            error: None,
        }
    } else {
        let Some(api_key) = resolve_api_key(&state, &draft).await? else {
            tracing::info!(provider = %draft.id, "供应商连通性测试跳过：未配置 API Key");
            return Ok(missing_api_key_result());
        };
        let outcome = if draft.kind == ProviderKind::QwenMt {
            // Qwen-MT：原生 `translation_options` 端点健康检查（极短翻译请求）。
            probe_qwen_mt_detail(
                &draft.base_url,
                &api_key,
                Some(draft.model.as_str()),
                ProbeOptions::default(),
            )
            .await?
        } else {
            probe_provider_detail(
                &draft.base_url,
                &api_key,
                Some(draft.model.as_str()),
                ProbeOptions::default(),
            )
            .await?
        };
        // 只记录脱敏结果（不含 Key / 响应体原文）。
        tracing::info!(
            provider = %draft.id,
            kind = ?draft.kind,
            host = %host_of(&draft.base_url),
            ok = outcome.result.ok,
            status = ?outcome.result.status,
            latency_ms = ?outcome.result.latency_ms,
            model = ?outcome.result.model,
            model_count = outcome.available_models.len(),
            "供应商连通性测试完成"
        );
        outcome.result
    };

    // 3) 测试成功且提供了 apiKey → 写入凭据存储（失败不覆盖已有密钥）。
    if result.ok {
        if let Some(secret) = draft
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
        {
            let service = state.credential_service();
            match service.set_provider_key(&draft.id, secret).await {
                Ok(()) => {
                    tracing::info!(provider = %draft.id, "供应商密钥已保存（不回显）");
                    // 若供应商已在配置中，广播 config:changed 让前端刷新 hasKey / keyMask。
                    let config: Config = state.snapshot_config();
                    if config.providers.iter().any(|item| item.id == draft.id) {
                        let view = state.config_view().await;
                        emit_config_changed(
                            &app,
                            &ConfigChangedPayload {
                                config: view,
                                changed_keys: vec!["providers".to_string()],
                            },
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(code = %err.code, "保存供应商密钥失败");
                }
            }
        }
    }

    Ok(result)
}
