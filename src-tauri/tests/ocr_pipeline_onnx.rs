//! T1 真实 OCR 集成测试（真实 ONNX 推理，不使用 mock）。
//!
//! 流程与产品一致：
//! 1. 读取 PNG 截图（`.verify/v1/ocr/cases.json` 用例：S-11/S-15 真实渲染截图 +
//!    `.verify/v1/ocr/input/live_*.png` 真实 Windows 桌面截图），解码为 **BGRA**
//!    （与 T2 捕获输出同格式：top-down、stride = width×4）；
//! 2. 构造 `FrozenFrame` → `OcrPipeline`（`with_real`）→ `scan_screen`（整屏 det，
//!    走 3.0 Mpx 像素预算）→ `recognize_selection`（rec-only 缓存路径）；
//! 3. 逐用例把识别行/置信度/耗时写入 `.verify/v1/ocr/out/<case>.json`；
//! 4. 断言：行数 > 0、字符准确率 ≥ 0.85（真实整屏案例用已知文本行召回率 ≥ 0.6）。
//!
//! 资产查找：`SNIP_ASK_OCR_DIR`（开发钩子）→ `src-tauri/resources/ocr`（本机 T5 已备好）。
//! 两者都没有时**跳过**（CI 无资产也能通过），并打印跳过原因。
//!
//! 复跑：
//! ```powershell
//! cd src-tauri
//! $env:SNIP_ASK_OCR_DIR = (Resolve-Path ..\src-tauri\resources\ocr).Path
//! cargo test --test ocr_pipeline_onnx -- --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::json;

use snip_ask_lib::contract::types::RectPhys;
use snip_ask_lib::platform::capture::FrozenFrame;
use snip_ask_lib::services::ocr_onnx::{OcrAssets, OnnxOcrEngine};
use snip_ask_lib::services::ocr_service::OcrPipeline;

/// 产品 OCR 置信度阈值（`ocr.textThreshold` 默认 0.6）。
const TEXT_THRESHOLD: f64 = 0.6;
/// 产品 det 长边上限（`ocr.detMaxEdge` 默认 3200）。
const DET_MAX_EDGE: u32 = 3200;
/// 字符准确率门禁（与 S-15 的 ≥95% 目标相比留出实现差异余量）。
const MIN_ACCURACY: f64 = 0.85;

#[derive(Debug, Deserialize)]
struct CasesFile {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    kind: String,
    png: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    truth: Option<String>,
    #[serde(default)]
    expect_lines: Option<Vec<String>>,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri 应有父目录")
        .to_path_buf()
}

fn find_assets() -> Option<PathBuf> {
    let has_models = |dir: &Path| {
        dir.join("ch_PP-OCRv4_det_infer.onnx").is_file()
            && dir.join("ch_PP-OCRv4_rec_infer.onnx").is_file()
            && dir.join("ch_ppocr_mobile_v2.0_cls_infer.onnx").is_file()
            && dir.join("ppocr_keys_v1.txt").is_file()
            && dir.join("onnxruntime.dll").is_file()
    };
    if let Some(dir) = std::env::var_os("SNIP_ASK_OCR_DIR") {
        let dir = PathBuf::from(dir);
        if has_models(&dir) {
            return Some(dir);
        }
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/ocr");
    has_models(&dir).then_some(dir)
}

/// 解码 PNG → BGRA（top-down；与 Win32 BitBlt 输出同布局）。
fn load_png_bgra(path: &Path) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error>> {
    use png::{BitDepth, ColorType};
    let file = std::fs::File::open(path)?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info()?;
    let width = reader.info().width;
    let height = reader.info().height;
    let mut buffer = vec![0u8; reader.output_buffer_size().ok_or("PNG 尺寸过大")?];
    reader.next_frame(&mut buffer)?;
    let (color, depth) = reader.output_color_type();
    if depth != BitDepth::Eight {
        return Err(format!("只支持 8bit PNG，实际 {depth:?}").into());
    }
    let pixels = width as usize * height as usize;
    let mut bgra = Vec::with_capacity(pixels * 4);
    match color {
        ColorType::Rgb => {
            for chunk in buffer.chunks_exact(3) {
                bgra.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
            }
        }
        ColorType::Rgba => {
            for chunk in buffer.chunks_exact(4) {
                bgra.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
        }
        ColorType::Grayscale => {
            for &lum in &buffer {
                bgra.extend_from_slice(&[lum, lum, lum, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            for chunk in buffer.chunks_exact(2) {
                let (lum, alpha) = (chunk[0], chunk[1]);
                bgra.extend_from_slice(&[lum, lum, lum, alpha]);
            }
        }
        other => return Err(format!("不支持的 PNG 颜色类型：{other:?}").into()),
    }
    if bgra.len() != pixels * 4 {
        return Err(format!("PNG 解码长度不符：期望 {}，实际 {}", pixels * 4, bgra.len()).into());
    }
    Ok((width, height, bgra))
}

/// 与 S-15 一致的归一化（NFKC + 去空白；Rust 侧只去空白，NFKC 由 Python 评分脚本负责）。
fn strip_whitespace(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn char_accuracy(ocr: &str, truth: &str) -> f64 {
    let a: Vec<char> = strip_whitespace(ocr).chars().collect();
    let b: Vec<char> = strip_whitespace(truth).chars().collect();
    if b.is_empty() {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / b.len() as f64
}

fn line_recall(ocr: &str, expect: &[String]) -> (usize, usize) {
    let haystack = strip_whitespace(ocr);
    let total = expect
        .iter()
        .filter(|line| !strip_whitespace(line).is_empty())
        .count();
    let matched = expect
        .iter()
        .filter(|line| {
            let needle = strip_whitespace(line);
            !needle.is_empty() && haystack.contains(&needle)
        })
        .count();
    (matched, total)
}

#[tokio::test]
async fn real_ocr_corpus_and_live_desktop_cases() {
    let Some(assets_dir) = find_assets() else {
        eprintln!(
            "跳过真实 OCR 集成测试：未找到 OCR 资产（设置 SNIP_ASK_OCR_DIR 或准备 src-tauri/resources/ocr/）"
        );
        return;
    };
    let cases_path = workspace_root().join(".verify/v1/ocr/cases.json");
    if !cases_path.is_file() {
        eprintln!("跳过真实 OCR 集成测试：缺少 {}", cases_path.display());
        return;
    }
    let cases: CasesFile =
        serde_json::from_str(&std::fs::read_to_string(&cases_path).expect("读取 cases.json"))
            .expect("解析 cases.json");
    assert!(!cases.cases.is_empty(), "OCR 证据用例不能为空");

    // 资产走「开发钩子」来源；models_dir 指向一个不存在的目录，避免误用真实用户数据。
    let nonexistent_models = workspace_root().join("target/ocr-models-unused");
    let assets = OcrAssets::resolve_from(None, &nonexistent_models, Some(&assets_dir));
    assert!(assets.is_ready(), "OCR 资产应齐全：{:?}", assets.missing());

    let engine = OnnxOcrEngine::new(assets);
    let load_started = Instant::now();
    engine.warmup().expect("加载 ONNX 会话失败");
    let load_ms = load_started.elapsed().as_millis() as u64;
    assert!(engine.is_loaded());

    let pipeline = OcrPipeline::with_real(Box::new(engine));
    let out_dir = workspace_root().join(".verify/v1/ocr/out");
    std::fs::create_dir_all(&out_dir).expect("创建证据输出目录");

    let mut summary = Vec::new();
    for case in &cases.cases {
        let png_path = workspace_root().join(&case.png);
        let (width, height, bgra) = load_png_bgra(&png_path)
            .unwrap_or_else(|err| panic!("读取截图 {} 失败：{err}", png_path.display()));
        let frame = FrozenFrame {
            width,
            height,
            data_url: None,
            bgra: Some(Arc::new(bgra)),
        };
        let frame_rect = RectPhys {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        };
        let session_id = format!("ocr-evidence-{}", case.name);

        let det_started = Instant::now();
        pipeline
            .scan_screen(
                &session_id,
                &frame,
                &frame_rect,
                DET_MAX_EDGE,
                0.0, // 检测阶段不过滤（框分数已在 DB 后处理内过滤）
            )
            .await
            .unwrap_or_else(|err| panic!("{}: 整屏 det 失败：{}", case.name, err.code));
        let det_ms = det_started.elapsed().as_millis() as u64;

        let rec_started = Instant::now();
        let scan = pipeline
            .recognize_selection(
                &session_id,
                &frame,
                &frame_rect,
                &frame_rect,
                DET_MAX_EDGE,
                TEXT_THRESHOLD,
            )
            .await
            .unwrap_or_else(|err| panic!("{}: 选区 rec 失败：{}", case.name, err.code));
        let rec_ms = rec_started.elapsed().as_millis() as u64;

        let lines: Vec<serde_json::Value> = scan
            .boxes
            .iter()
            .map(|item| {
                json!({
                    "text": item.text,
                    "confidence": item.confidence,
                    "bbox": {
                        "x": item.bbox.x,
                        "y": item.bbox.y,
                        "width": item.bbox.width,
                        "height": item.bbox.height,
                    },
                })
            })
            .collect();
        let mean_confidence = if lines.is_empty() {
            0.0
        } else {
            scan.boxes.iter().map(|item| item.confidence).sum::<f64>() / scan.boxes.len() as f64
        };

        let output = json!({
            "case": case.name,
            "kind": case.kind,
            "category": case.category,
            "engine": "rapidocr PP-OCRv4 det+rec+cls / onnxruntime 1.31.1 CPU (ort 2.0.0-rc.15, load-dynamic)",
            "assets_dir": assets_dir.display().to_string(),
            "load_ms": load_ms,
            "width": width,
            "height": height,
            "det_ms": det_ms,
            "rec_ms": rec_ms,
            "total_ms": det_ms + rec_ms,
            "text_threshold": TEXT_THRESHOLD,
            "det_max_edge": DET_MAX_EDGE,
            "has_text": scan.has_text,
            "line_count": lines.len(),
            "mean_confidence": mean_confidence,
            "preview": scan.preview,
            "lines": lines,
        });
        std::fs::write(
            out_dir.join(format!("{}.json", case.name)),
            serde_json::to_string_pretty(&output).expect("序列化 OCR 输出"),
        )
        .expect("写入 OCR 输出 JSON");

        // ── 断言：真实推理必须产出文字 ──
        assert!(
            !scan.boxes.is_empty(),
            "{}: 真实 OCR 未识别到任何文本行",
            case.name
        );
        assert!(
            scan.has_text,
            "{}: has_text 应为 true（识别到 {} 行）",
            case.name,
            scan.boxes.len()
        );
        assert!(
            !scan.preview.trim().is_empty(),
            "{}: preview 不能为空",
            case.name
        );

        let mut record = json!({
            "case": case.name,
            "kind": case.kind,
            "lines": scan.boxes.len(),
            "det_ms": det_ms,
            "rec_ms": rec_ms,
        });
        if let Some(truth) = case.truth.as_deref().filter(|text| !text.trim().is_empty()) {
            let accuracy = char_accuracy(&scan.preview, truth);
            record["char_accuracy"] = json!(accuracy);
            assert!(
                accuracy >= MIN_ACCURACY,
                "{}: 字符准确率 {:.4} 低于门禁 {MIN_ACCURACY}（真实推理质量异常）",
                case.name,
                accuracy
            );
        }
        if let Some(expect) = case.expect_lines.as_deref() {
            let (matched, total) = line_recall(&scan.preview, expect);
            record["known_line_matched"] = json!(matched);
            record["known_line_total"] = json!(total);
            if total > 0 && case.truth.as_deref().unwrap_or("").trim().is_empty() {
                let recall = matched as f64 / total as f64;
                assert!(
                    recall >= 0.6,
                    "{}: 真实整屏截图对已知文本行召回 {matched}/{total} 低于 60%",
                    case.name
                );
            }
        }
        summary.push(record);
        eprintln!(
            "[ocr-evidence] {} {}x{} lines={} det={}ms rec={}ms",
            case.name,
            width,
            height,
            scan.boxes.len(),
            det_ms,
            rec_ms
        );
    }

    std::fs::write(
        out_dir.join("_run.json"),
        serde_json::to_string_pretty(&json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "assets_dir": assets_dir.display().to_string(),
            "engine_load_ms": load_ms,
            "host": std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string()),
            "cases": summary,
        }))
        .expect("序列化运行摘要"),
    )
    .expect("写入运行摘要");
}
