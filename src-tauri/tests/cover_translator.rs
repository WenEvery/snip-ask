//! 覆盖翻译后端纯逻辑测试（T4）。
//!
//! 覆盖：缓存键（SHA-1 / 规范化）、LRU 缓存淘汰、批处理切分、骨架占位译文、
//! 真实网关失败时「保留原文 + failed=true」、命令层输入校验。
//!
//! 真实 GPU/网络链路不在本文件内（由 T1/T2 的真机验证负责）；这里不伪造成功路径。

use std::sync::Arc;

use async_trait::async_trait;

use snip_ask_lib::commands::cover::{cache_model, prepare_blocks, COVER_BATCH_SIZE};
use snip_ask_lib::contract::config::{Config, TranslateBackend};
use snip_ask_lib::contract::errors::{AppError, AppResult, ErrorCode};
use snip_ask_lib::contract::types::{ActionId, CoverBlock, Mode, RectPhys, SessionId, TaskId};
use snip_ask_lib::services::llm_gateway::{LlmGateway, LlmRequest, StreamChunk, TaskOutcome};
use snip_ask_lib::services::translator::{
    cover_cache_key, normalize_cache_text, plan_batches, sha1_hex, CoverCache, CoverTranslator,
    COVER_CACHE_CAPACITY,
};

fn block(id: &str, text: &str) -> CoverBlock {
    CoverBlock {
        id: id.to_string(),
        text: text.to_string(),
        bbox: RectPhys {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 20.0,
        },
        confidence: 0.9,
    }
}

fn session(id: &str) -> SessionId {
    SessionId::from(id)
}

/* ───────────────────────── SHA-1 / 缓存键 ───────────────────────── */

#[test]
fn sha1_matches_known_vectors() {
    // FIPS-180-1 已知向量
    assert_eq!(sha1_hex(""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        sha1_hex("The quick brown fox jumps over the lazy dog"),
        "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
    );
    // 跨 64 字节块边界（55/56/64/65/1000 字节）
    for length in [55usize, 56, 64, 65, 1000] {
        let input = "a".repeat(length);
        assert_eq!(sha1_hex(&input).len(), 40);
    }
}

#[test]
fn cache_text_is_normalized() {
    assert_eq!(normalize_cache_text("  hello   world \n"), "hello world");
    assert_eq!(
        normalize_cache_text("缓存  口径\n已落地"),
        "缓存 口径 已落地"
    );
    // 空白差异不影响缓存键
    let a = cover_cache_key(&session("s1"), "hello  world", "en", "m1");
    let b = cover_cache_key(&session("s1"), " hello world\n", "en", "m1");
    assert_eq!(a, b);
    // 会话 / 语言 / 模型 / 原文任一变化 → 不同键
    let base = cover_cache_key(&session("s1"), "hello", "en", "m1");
    assert_ne!(base, cover_cache_key(&session("s2"), "hello", "en", "m1"));
    assert_ne!(base, cover_cache_key(&session("s1"), "hello", "zh", "m1"));
    assert_ne!(base, cover_cache_key(&session("s1"), "hello", "en", "m2"));
    assert_ne!(base, cover_cache_key(&session("s1"), "hello!", "en", "m1"));
}

/* ───────────────────────── LRU 缓存 ───────────────────────── */

#[test]
fn cover_cache_is_lru_with_bounded_capacity() {
    let mut cache = CoverCache::new(2);
    assert!(cache.is_empty());
    assert_eq!(cache.capacity(), 2);
    assert_eq!(COVER_CACHE_CAPACITY, 512);

    cache.insert("a".to_string(), "A".to_string());
    cache.insert("b".to_string(), "B".to_string());
    // 访问 a → b 成为最久未使用
    assert_eq!(cache.get("a").as_deref(), Some("A"));
    cache.insert("c".to_string(), "C".to_string());
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get("b"), None);
    assert_eq!(cache.get("a").as_deref(), Some("A"));
    assert_eq!(cache.get("c").as_deref(), Some("C"));

    // 更新已有键不增加容量，并刷新 LRU 位置
    cache.insert("a".to_string(), "A2".to_string());
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get("a").as_deref(), Some("A2"));

    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.get("a"), None);
}

#[test]
fn cache_capacity_is_at_least_one() {
    let mut cache = CoverCache::new(0);
    assert_eq!(cache.capacity(), 1);
    cache.insert("k".to_string(), "v".to_string());
    cache.insert("k2".to_string(), "v2".to_string());
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get("k"), None);
    assert_eq!(cache.get("k2").as_deref(), Some("v2"));
}

/* ───────────────────────── 批处理切分 ───────────────────────── */

#[test]
fn batches_are_split_by_research_batch_size() {
    assert_eq!(COVER_BATCH_SIZE, 10);
    assert_eq!(plan_batches(0, 10), Vec::<(usize, usize)>::new());
    assert_eq!(plan_batches(3, 10), vec![(0, 3)]);
    assert_eq!(plan_batches(10, 10), vec![(0, 10)]);
    assert_eq!(plan_batches(25, 10), vec![(0, 10), (10, 20), (20, 25)]);
    // 非法批大小按 1 处理，避免死循环
    assert_eq!(plan_batches(3, 0), vec![(0, 1), (1, 2), (2, 3)]);
}

/* ───────────────────────── 骨架模式 ───────────────────────── */

#[tokio::test]
async fn skeleton_translator_returns_deterministic_placeholders() {
    let translator = CoverTranslator::skeleton();
    assert_eq!(translator.engine(), "skeleton");
    assert!(translator.is_skeleton());

    let blocks = vec![block("b0", "缓存口径已落地"), block("b1", "Hello")];
    let first = translator.translate_blocks(&blocks).await;
    let second = translator.translate_blocks(&blocks).await;
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    for (index, result) in first.iter().enumerate() {
        assert_eq!(result.id, blocks[index].id);
        assert_eq!(result.source, blocks[index].text);
        assert_eq!(result.engine, "skeleton");
        assert!(!result.failed);
        assert!(result.translation.contains(&blocks[index].text));
    }
    // 空块列表不 panic
    assert!(translator.translate_blocks(&[]).await.is_empty());
}

/* ───────────────────────── 真实网关失败降级 ───────────────────────── */

#[tokio::test]
async fn gateway_failure_keeps_source_and_marks_failed() {
    // 生产 `HttpLlmGateway` 当前为骨架实现（返回 SA-9001），正好覆盖「网关失败」路径。
    let gateway = Arc::new(snip_ask_lib::services::local_gateway::HttpLlmGateway::new(
        "http://127.0.0.1:39281/v1",
        None,
        "local-model",
    ));
    let translator = CoverTranslator::with_gateway("local", gateway, Mode::Quick, "en-US");
    assert_eq!(translator.engine(), "local");
    assert!(!translator.is_skeleton());

    let blocks = vec![block("b0", "第一块"), block("b1", "第二块")];
    let results = translator.translate_blocks(&blocks).await;
    assert_eq!(results.len(), 2);
    for (index, result) in results.iter().enumerate() {
        assert!(result.failed, "网关失败必须标记 failed");
        assert_eq!(result.translation, blocks[index].text);
        assert_eq!(result.source, blocks[index].text);
        assert_eq!(result.engine, "local");
    }
}

/// 测试用成功网关：把原文包成 `[译]原文` 流式返回。
struct EchoGateway;

#[async_trait]
impl LlmGateway for EchoGateway {
    async fn stream(
        &self,
        request: &LlmRequest,
        sink: &(dyn Fn(StreamChunk) + Send + Sync),
    ) -> AppResult<TaskOutcome> {
        if request.action != ActionId::Translate {
            return Err(AppError::new(
                ErrorCode::InvalidState,
                Some("Expected translate action".to_string()),
            ));
        }
        // 分两段流式返回，验证命令层正确拼接
        sink(StreamChunk {
            kind: snip_ask_lib::contract::types::StreamKind::Text,
            chunk: "[译]".to_string(),
        });
        sink(StreamChunk {
            kind: snip_ask_lib::contract::types::StreamKind::Text,
            chunk: request.input_text.clone(),
        });
        Ok(TaskOutcome {
            usage: Default::default(),
            latency_ms: 1,
            engine: request.engine.clone(),
            sources: Vec::new(),
        })
    }
}

#[tokio::test]
async fn gateway_success_collects_streamed_translation() {
    let translator =
        CoverTranslator::with_gateway("deepseek", Arc::new(EchoGateway), Mode::Quick, "en-US");
    let results = translator.translate_blocks(&[block("b0", "你好")]).await;
    assert_eq!(results.len(), 1);
    assert!(!results[0].failed);
    assert_eq!(results[0].translation, "[译]你好");
    assert_eq!(results[0].engine, "deepseek");
    assert_eq!(results[0].source, "你好");
}

/* ───────────────────────── 命令层输入校验 ───────────────────────── */

#[test]
fn prepare_blocks_drops_blank_and_rejects_invalid() {
    let prepared = prepare_blocks(vec![
        block("b0", "  中文块  "),
        block("b1", "   "), // 空文本 → 丢弃
        block("b2", "英文块"),
    ])
    .unwrap();
    assert_eq!(prepared.len(), 2);
    assert_eq!(prepared[0].id, "b0");
    assert_eq!(prepared[0].text, "中文块");
    assert_eq!(prepared[1].id, "b2");

    // 空 id
    assert_eq!(
        prepare_blocks(vec![block("", "文本")]).unwrap_err().code,
        "SA-5001"
    );
    // 重复 id
    assert_eq!(
        prepare_blocks(vec![block("b0", "一"), block("b0", "二")])
            .unwrap_err()
            .code,
        "SA-5001"
    );
    // 非法 bbox（NaN / 负宽）
    let mut nan = block("b0", "文本");
    nan.bbox.width = f64::NAN;
    assert_eq!(prepare_blocks(vec![nan]).unwrap_err().code, "SA-5001");
    let mut negative = block("b0", "文本");
    negative.bbox.height = -1.0;
    assert_eq!(prepare_blocks(vec![negative]).unwrap_err().code, "SA-5001");
    // 空输入合法（由命令层再报 OCR 失败）
    assert!(prepare_blocks(Vec::new()).unwrap().is_empty());
}

#[test]
fn cache_model_reflects_engine_selection() {
    // v1.0.0：默认走真实能力（skeletonMode=false，云端翻译 qwen-mt-lite）。
    let config = Config::default();
    assert_eq!(cache_model(&config), "qwen-mt-lite");

    // 骨架调试模式下缓存键固定为 skeleton。
    let mut config = Config::default();
    config.advanced.skeleton_mode = true;
    assert_eq!(cache_model(&config), "skeleton");

    let mut config = Config::default();
    config.advanced.skeleton_mode = false;
    config.translate.backend = TranslateBackend::Local;
    config.local.model = "hy-mt-1.8b-q4".to_string();
    assert_eq!(cache_model(&config), "local:hy-mt-1.8b-q4");

    let mut config = Config::default();
    config.advanced.skeleton_mode = false;
    config.translate.backend = TranslateBackend::Cloud;
    config.translate.model = "deepseek-chat".to_string();
    assert_eq!(cache_model(&config), "deepseek-chat");
}

#[test]
fn task_id_type_is_publicly_constructible() {
    // 保证跨模块测试可构造（TaskId 出现在 LlmRequest 中）
    let _ = TaskId::from("cover");
}
