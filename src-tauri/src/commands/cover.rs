//! 覆盖式翻译命令（契约 v4 / 研究 §2.4）。
//!
//! `translate_cover` 接收前端合并后的段块（`CoverBlock[]`），批处理翻译后返回逐块译文；
//! 同时推送 `translate:progress` / `translate:ready` 事件（overlay 用事件驱动进度与渲染）。
//!
//! 为什么新增命令而不是复用 `menu_execute`：覆盖翻译需要**结构化输入输出**（块 id ↔ 译文映射），
//! 事件流 `task:delta` 只能承载纯文本，无法还原块映射（见 docs 契约修订记录）。
//!
//! 关键行为（研究 §7.3 / task-4 验收）：
//! 1. 会话校验：必须是当前活动会话，后端阶段保持 `menu` / `showingResult`（**不新增会话阶段**，
//!    `translating → covered` 只是前端子阶段，见 spec §3.5）；
//! 2. 按 [`COVER_BATCH_SIZE`]（10）块/请求批处理，复用 [`crate::services::translator`]；
//! 3. 会话级 LRU 译文缓存 `sha1(原文 + 目标语言 + 模型)`（容量 512，见 `CoverCache`）；
//! 4. 每批发 `translate:progress`（done/total），结束时发 `translate:ready`；
//! 5. 失败块保留原文并 `failed = true`（不伪造成功）；骨架模式返回确定性占位且 `engine = "skeleton"`。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Instant;

use parking_lot::Mutex;
use tauri::{AppHandle, State};

use crate::app::state::AppState;
use crate::contract::config::{Config, TranslateBackend};
use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::events::{TranslateProgressPayload, TranslateReadyPayload};
use crate::contract::types::{ActionId, CoverBlock, CoverResult, CoveredBlock, SessionId};
use crate::domain::session::SessionPhase;
use crate::events::{emit_translate_progress, emit_translate_ready};
use crate::services::action_router::ActionRouter;
use crate::services::translator::{
    cover_cache_key, plan_batches, CoverCache, CoverTranslator, COVER_CACHE_CAPACITY,
    MAX_COVER_BLOCK_CHARS,
};

/// 单次请求的批大小（研究 §2.4-3：8–10 块/请求）。
pub const COVER_BATCH_SIZE: usize = 10;

/// 单次覆盖翻译允许的最大块数（防御异常前端输入）。
pub const MAX_COVER_BLOCKS: usize = 500;

static COVER_CACHE: OnceLock<Mutex<CoverCache>> = OnceLock::new();

fn cover_cache() -> &'static Mutex<CoverCache> {
    COVER_CACHE.get_or_init(|| Mutex::new(CoverCache::new(COVER_CACHE_CAPACITY)))
}

/// 校验并规范化前端段块：丢弃空文本块、拒绝空/重复 id 与非法 bbox、截断超长文本。
pub fn prepare_blocks(blocks: Vec<CoverBlock>) -> AppResult<Vec<CoverBlock>> {
    if blocks.len() > MAX_COVER_BLOCKS {
        return Err(AppError::invalid_config(
            "blocks",
            format!("块数量 {} 超过上限 {MAX_COVER_BLOCKS}", blocks.len()),
        ));
    }
    let mut seen: HashSet<String> = HashSet::with_capacity(blocks.len());
    let mut prepared = Vec::with_capacity(blocks.len());
    for mut block in blocks {
        block.text = block.text.trim().to_string();
        if block.text.is_empty() {
            continue;
        }
        let id = block.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::invalid_config("blocks", "块 id 不能为空"));
        }
        if !seen.insert(id.clone()) {
            return Err(AppError::invalid_config(
                "blocks",
                format!("块 id 重复：{id}"),
            ));
        }
        let bbox = &block.bbox;
        if !bbox.x.is_finite()
            || !bbox.y.is_finite()
            || !bbox.width.is_finite()
            || !bbox.height.is_finite()
        {
            return Err(AppError::invalid_config(
                "blocks",
                format!("块 {id} 的 bbox 含非有限数值"),
            ));
        }
        if bbox.width < 0.0 || bbox.height < 0.0 {
            return Err(AppError::invalid_config(
                "blocks",
                format!("块 {id} 的 bbox 宽高不能为负"),
            ));
        }
        let char_count = block.text.chars().count();
        if char_count > MAX_COVER_BLOCK_CHARS {
            block.text = block.text.chars().take(MAX_COVER_BLOCK_CHARS).collect();
        }
        block.id = id;
        prepared.push(block);
    }
    Ok(prepared)
}

/// 缓存键里的「模型」维度：本地模型 / 云端模型 / 骨架占位。
pub fn cache_model(config: &Config) -> String {
    if config.advanced.skeleton_mode {
        "skeleton".to_string()
    } else {
        match config.translate.backend {
            TranslateBackend::Local => format!("local:{}", config.local.model),
            TranslateBackend::Cloud => config.translate.model.clone(),
        }
    }
}

/// 选择覆盖翻译执行器（骨架 / 云端 / 本地），并如实返回 engine 标签。
///
/// 云端供应商未配置密钥时与 `AppState::gateway_for` 的降级语义一致：
/// 退化为确定性骨架占位且 `engine = "skeleton"`（**不伪造成功**）。
async fn build_translator(
    app: &AppHandle,
    state: &AppState,
    config: &Config,
    target_lang: &str,
) -> AppResult<CoverTranslator> {
    if config.advanced.skeleton_mode {
        return Ok(CoverTranslator::skeleton());
    }

    let routed = ActionRouter.resolve(config, ActionId::Translate, config.modes.translate, true)?;

    if routed.engine == "local" {
        let gateway = state.gateway_for(app, &routed).await?;
        return Ok(CoverTranslator::with_gateway(
            "local",
            gateway,
            config.modes.translate,
            target_lang,
        ));
    }

    let provider = config
        .providers
        .iter()
        .find(|provider| provider.enabled && provider.id == routed.engine);
    let has_key = match provider {
        Some(provider) => state
            .credential_service()
            .provider_key(&provider.id)
            .await?
            .is_some(),
        None => false,
    };
    if !has_key {
        tracing::warn!(
            engine = %routed.engine,
            "覆盖翻译：供应商未配置密钥，返回骨架占位（engine=skeleton）"
        );
        return Ok(CoverTranslator::skeleton());
    }

    let gateway = state.gateway_for(app, &routed).await?;
    Ok(CoverTranslator::with_gateway(
        routed.engine.clone(),
        gateway,
        config.modes.translate,
        target_lang,
    ))
}

/// 命令：覆盖翻译。
#[tauri::command]
pub async fn translate_cover(
    app: AppHandle,
    state: State<'_, AppState>,
    session: SessionId,
    blocks: Vec<CoverBlock>,
    target_lang: Option<String>,
) -> AppResult<CoverResult> {
    let started = Instant::now();

    // 1) 会话校验：当前会话 + 允许的后端阶段（menu / showingResult，不新增会话阶段）。
    {
        let store = state.session.lock();
        if !store.is_current(&session) {
            return Err(AppError::invalid_state(format!(
                "会话 {session} 不是当前活动会话"
            )));
        }
        let phase = store.current_phase();
        if !matches!(phase, SessionPhase::Menu | SessionPhase::ShowingResult) {
            return Err(AppError::invalid_state(format!(
                "会话处于 {} 阶段，不允许覆盖翻译（允许：menu / showingResult）",
                phase.as_str()
            )));
        }
    }

    // 2) 输入校验。
    let blocks = prepare_blocks(blocks)?;
    if blocks.is_empty() {
        return Err(AppError::new(
            ErrorCode::OcrFailed,
            Some("没有可覆盖翻译的文本块".to_string()),
        ));
    }

    let config = state.snapshot_config();
    let target_lang = target_lang
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| config.translate.target_lang.clone());
    let total = blocks.len() as u32;
    let model = cache_model(&config);

    // 3) 选择执行器（骨架 / 云端 / 本地）。
    let translator = build_translator(&app, &state, &config, &target_lang).await?;
    let engine = translator.engine().to_string();

    // 4) 批处理 + 会话级 LRU 缓存 + 进度事件。
    let mut by_id: HashMap<String, CoveredBlock> = HashMap::with_capacity(blocks.len());
    let mut done = 0u32;
    for (start, end) in plan_batches(blocks.len(), COVER_BATCH_SIZE) {
        let batch = &blocks[start..end];
        let mut pending: Vec<CoverBlock> = Vec::with_capacity(batch.len());
        for block in batch {
            let key = cover_cache_key(&session, &block.text, &target_lang, &model);
            let hit = cover_cache().lock().get(&key);
            match hit {
                Some(translation) => {
                    by_id.insert(
                        block.id.clone(),
                        CoveredBlock {
                            id: block.id.clone(),
                            source: block.text.clone(),
                            translation,
                            failed: false,
                            engine: engine.clone(),
                        },
                    );
                }
                None => pending.push(block.clone()),
            }
        }

        if !pending.is_empty() {
            let covered = translator.translate_blocks(&pending).await;
            for block in covered {
                if !block.failed {
                    let key = cover_cache_key(&session, &block.source, &target_lang, &model);
                    cover_cache().lock().insert(key, block.translation.clone());
                }
                by_id.insert(block.id.clone(), block);
            }
        }

        done += (end - start) as u32;
        emit_translate_progress(
            &app,
            &TranslateProgressPayload {
                session: session.clone(),
                done,
                total,
            },
        );
    }

    // 5) 保持与请求相同的块顺序（前端按 id 索引，顺序仍然是稳定契约）。
    let results: Vec<CoveredBlock> = blocks
        .iter()
        .filter_map(|block| by_id.remove(&block.id))
        .collect();
    let translated_blocks = results.iter().filter(|block| !block.failed).count() as u32;

    emit_translate_ready(
        &app,
        &TranslateReadyPayload {
            session: session.clone(),
            blocks: results.clone(),
            engine: engine.clone(),
        },
    );

    Ok(CoverResult {
        session,
        blocks: results,
        engine,
        latency_ms: started.elapsed().as_millis() as u64,
        total_blocks: total,
        translated_blocks,
    })
}
