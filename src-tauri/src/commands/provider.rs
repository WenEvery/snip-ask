//! 供应商连通性测试命令（契约 §6 第 17 条）。
//!
//! 契约修订语义：`provider_test` 兼作密钥保存通道 ——
//! `draft.apiKey` 非空且测试成功时写入凭据存储（骨架模式为内存实现），
//! 使 `ConfigView.providers[].hasKey / keyMask` 立即反映；测试失败不覆盖已有密钥。

use std::time::Duration;

use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::config::{validate, Config, Provider};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::ConfigChangedPayload;
use crate::contract::types::{ProviderDraft, TestResult};
use crate::events::emit_config_changed;

/// 命令：测试供应商连通性（成功时顺带保存 API Key）。
#[tauri::command]
pub async fn provider_test(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> AppResult<TestResult> {
    // 1) 校验草稿：复用配置校验（baseUrl 白名单、id 格式等）。
    let provider = Provider {
        id: draft.id.clone(),
        name: draft.name.clone(),
        base_url: draft.base_url.clone(),
        model: draft.model.clone(),
        caps: draft.caps.clone(),
        temperature: draft.temperature.unwrap_or(0.2),
        max_tokens: draft.max_tokens.unwrap_or(1024),
        enabled: true,
        extra: serde_json::Map::new(),
    };
    let mut probe_config = state.snapshot_config();
    probe_config.providers = vec![provider];
    validate(&probe_config)?;

    if draft.model.trim().is_empty() {
        return Err(AppError::invalid_config("model", "模型名不能为空"));
    }

    // 2) 连通性测试：骨架模式返回确定性成功；真实实现为 TODO(real)。
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
        // TODO(real): 使用 CredentialRef::Llm 读取密钥后请求 `{baseUrl}/v1/models`，
        // 429/5xx/超时退避重试 ≤2 次，401/402 不重试。
        TestResult {
            ok: false,
            latency_ms: None,
            model: None,
            status: None,
            error: Some(AppError::new(
                ErrorCode::NetworkUnavailable,
                Some("TODO(real): 真实连通性测试未接入".to_string()),
            )),
        }
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
                    // 若供应商已在配置中，广播 config.changed 让前端刷新 hasKey / keyMask。
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
