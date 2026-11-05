//! T2 真实屏幕捕获集成测试（**需要真实交互桌面**）。
//!
//! 默认 `#[ignore]`，CI 无桌面环境不会失败；本机/真机验证时显式运行：
//!
//! ```powershell
//! cd src-tauri
//! cargo test --test capture_real -- --ignored --nocapture
//! ```
//!
//! 若真实桌面存在无关隐私内容，可设置 `SNIP_ASK_CAPTURE_REDACT_RECT="x,y,w,h"`（帧局部物理像素）
//! 让证据 PNG 的该区域用纯色块遮盖（原始 BGRA 不落地，几何/统计不受影响）；本机验证用的是
//! `SNIP_ASK_CAPTURE_REDACT_RECT="0,0,1080,1920"`（副屏）。
//!
//! 产出真实证据到 `.verify/v1/capture/`：
//! - `capture-real.png`：Win32 BitBlt 抓取的**真实虚拟桌面截图**（物理像素 1:1）；
//! - `capture-real.json`：Win32 虚拟屏幕/显示器几何、帧尺寸、像素预算统计、DPI 感知、
//!   PNG/dataURL 体积、耗时；用于与 Tauri `available_monitors`（应用运行时）对照。
//!
//! 判定口径与产品一致：`luma_stats_from_bgra` + `is_capture_failed`（S-09）。

use std::path::PathBuf;
use std::time::Instant;

use base64::Engine as _;
use serde_json::json;

use snip_ask_lib::platform::capture::{
    encode_png_bytes, ensure_dpi_awareness, is_capture_failed, system_monitors,
    system_virtual_desktop_rect, FrozenFrame, ScreenCapturer, Win32Capturer, PNG_DATA_URL_PREFIX,
};
use snip_ask_lib::platform::coords;

/// 证据遮盖（可选）：`SNIP_ASK_CAPTURE_REDACT_RECT="x,y,w,h"`（帧局部物理像素）。
///
/// 真实桌面可能有与任务无关的隐私/密钥内容；设置该环境变量后，证据 PNG 的该矩形
/// 用纯色块遮盖（BGRA 原始帧不落地）。JSON 会记录 `redaction` 区域与原因。
fn redact_rect_from_env() -> Option<(u32, u32, u32, u32)> {
    let spec = std::env::var("SNIP_ASK_CAPTURE_REDACT_RECT").ok()?;
    let parts: Vec<u32> = spec
        .split(',')
        .map(|part| part.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 4 {
        return None;
    }
    Some((parts[0], parts[1], parts[2], parts[3]))
}

/// 用纯色块遮盖 BGRA 帧的指定矩形（证据脱敏；不改变几何/统计口径）。
fn masked_bgra(bgra: &[u8], width: u32, height: u32, rect: (u32, u32, u32, u32)) -> Vec<u8> {
    let mut masked = bgra.to_vec();
    let (rx, ry, rw, rh) = rect;
    let x1 = (rx as u64 + rw as u64).min(width as u64) as u32;
    let y1 = (ry as u64 + rh as u64).min(height as u64) as u32;
    for y in ry..y1 {
        for x in rx..x1 {
            let offset = (y as usize * width as usize + x as usize) * 4;
            masked[offset..offset + 4].copy_from_slice(&[24, 24, 28, 255]);
        }
    }
    masked
}

/// 证据目录：`<repo>/.verify/v1/capture`（`cargo test` 的 CWD 是 crate 根）。
fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.verify/v1/capture")
}

fn png_dimensions(png_bytes: &[u8]) -> (u32, u32) {
    assert!(png_bytes.len() > 24, "PNG 过短: {}", png_bytes.len());
    assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG 签名不符");
    assert_eq!(&png_bytes[12..16], b"IHDR", "首个 chunk 必须是 IHDR");
    let width = u32::from_be_bytes([png_bytes[16], png_bytes[17], png_bytes[18], png_bytes[19]]);
    let height = u32::from_be_bytes([png_bytes[20], png_bytes[21], png_bytes[22], png_bytes[23]]);
    (width, height)
}

/// 真实抓取一次虚拟桌面：断言几何/像素/非黑，并落盘 PNG + JSON 证据。
#[tokio::test]
#[ignore = "需要真实交互桌面：cargo test --test capture_real -- --ignored --nocapture"]
async fn captures_real_virtual_desktop_and_writes_evidence() {
    let dir = evidence_dir();
    std::fs::create_dir_all(&dir).expect("创建证据目录");

    // ── 1) 环境前置：DPI 感知 + Win32 几何 ──
    let awareness = ensure_dpi_awareness();
    assert!(
        awareness.is_per_monitor(),
        "捕获前必须是逐显示器 DPI 感知，实际 {awareness:?}"
    );
    let expected = system_virtual_desktop_rect().expect("真实桌面必须有虚拟屏幕矩形");
    let monitors = system_monitors();
    assert!(!monitors.is_empty(), "EnumDisplayMonitors 至少一个显示器");
    let union = coords::union_rect(monitors.iter().map(|monitor| monitor.rect)).expect("非空");
    assert_eq!(
        union, expected,
        "Win32 虚拟屏幕矩形必须等于显示器外接矩形（否则 overlay/OCR 坐标会偏移）"
    );

    // ── 2) 真实捕获 ──
    let started = Instant::now();
    let frame: FrozenFrame = Win32Capturer::new()
        .capture_virtual_desktop()
        .await
        .expect("真实捕获成功（需要未锁屏的交互桌面）");
    let capture_elapsed_ms = started.elapsed().as_millis() as u64;

    // ── 3) 几何断言：帧 = 虚拟桌面物理像素外接矩形 ──
    let expected_width = expected.width.round() as u32;
    let expected_height = expected.height.round() as u32;
    assert_eq!(frame.width, expected_width, "帧宽必须等于虚拟屏幕宽");
    assert_eq!(frame.height, expected_height, "帧高必须等于虚拟屏幕高");
    assert!(
        frame.width >= 100 && frame.height >= 100,
        "尺寸异常: {frame:?}"
    );

    // ── 4) BGRA 供 OCR：长度 = w*h*4，顶朝下 ──
    assert!(frame.has_complete_bgra(), "真实帧必须带完整 BGRA");
    let bgra = frame.bgra.as_deref().expect("bgra");
    assert_eq!(
        bgra.len(),
        frame.width as usize * frame.height as usize * 4,
        "BGRA 长度必须 = width*height*4"
    );

    // ── 5) 真实帧不是黑屏（S-09 判定；真实桌面应有内容） ──
    let stats = frame.luma_stats().expect("真实帧亮度统计");
    assert!(
        !is_capture_failed(Some(&stats)),
        "真实桌面捕获不应判黑: {stats:?}"
    );

    // ── 6) PNG data URL：真 PNG、尺寸一致 ──
    let data_url = frame.data_url.as_ref().expect("真实帧必须有 data URL");
    assert!(data_url.starts_with(PNG_DATA_URL_PREFIX));
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_url.trim_start_matches(PNG_DATA_URL_PREFIX))
        .expect("data URL 必须是合法 base64");
    assert_eq!(
        png_dimensions(&png_bytes),
        (frame.width, frame.height),
        "PNG 尺寸必须与帧尺寸一致"
    );
    let png_path = dir.join("capture-real.png");
    std::fs::write(&png_path, &png_bytes).expect("写真实截图 PNG");

    // ── 6.1) 可选证据脱敏：直接对 BGRA 原始帧做遮盖后重新编码写盘 ──
    let redaction = redact_rect_from_env();
    if let Some(rect) = redaction {
        let masked = masked_bgra(bgra, frame.width, frame.height, rect);
        let masked_png = encode_png_bytes(frame.width, frame.height, &masked)
            .expect("遮盖后的帧必须能重新编码 PNG");
        assert_eq!(
            png_dimensions(&masked_png),
            (frame.width, frame.height),
            "遮盖后 PNG 尺寸必须不变"
        );
        std::fs::write(&png_path, masked_png).expect("写遮盖后的证据 PNG");
    }

    // ── 7) 几何/统计证据 JSON（与 Tauri available_monitors 对照用） ──
    let json_path = dir.join("capture-real.json");
    let evidence = json!({
        "kind": "real-win32-bitblt",
        "capturedAt": chrono::Utc::now().to_rfc3339(),
        "dpiAwareness": awareness.as_str(),
        "win32VirtualDesktop": {
            "x": expected.x,
            "y": expected.y,
            "width": expected.width,
            "height": expected.height,
        },
        "win32Monitors": monitors.iter().map(|monitor| monitor.to_json()).collect::<Vec<_>>(),
        "frame": {
            "width": frame.width,
            "height": frame.height,
            "bgraBytes": bgra.len(),
            "dataUrlChars": data_url.len(),
            "pngBytes": png_bytes.len(),
        },
        "stats": stats.to_json(),
        "timing": { "captureAndEncodeMs": capture_elapsed_ms },
        "redaction": redaction.map(|(x, y, width, height)| json!({
            "rect": { "x": x, "y": y, "width": width, "height": height },
            "reason": "unrelated personal content on screen; solid mask applied to the saved evidence PNG (BGRA/geometry/statistics unchanged)",
            "env": "SNIP_ASK_CAPTURE_REDACT_RECT",
        })),
        "png": png_path.to_string_lossy(),
        "runnable": "$env:SNIP_ASK_CAPTURE_REDACT_RECT='0,0,1080,1920'; cargo test --test capture_real -- --ignored --nocapture",
    });
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&evidence).expect("序列化证据"),
    )
    .expect("写证据 JSON");

    println!("真实捕获证据已落盘：");
    println!("  PNG : {}", png_path.display());
    println!("  JSON: {}", json_path.display());
    if let Some(rect) = redaction {
        println!(
            "  PNG 已遮盖区域 x={} y={} w={} h={}（SNIP_ASK_CAPTURE_REDACT_RECT）",
            rect.0, rect.1, rect.2, rect.3
        );
    }
    println!(
        "  帧 {}x{} @ origin ({}, {})，{} 个显示器，capture+encode {}ms，PNG {} 字节",
        frame.width,
        frame.height,
        expected.x,
        expected.y,
        monitors.len(),
        capture_elapsed_ms,
        png_bytes.len()
    );
    for monitor in &monitors {
        println!(
            "  显示器 ({:.0},{:.0}) {:.0}x{:.0} primary={} dpi={:.0}%",
            monitor.rect.x,
            monitor.rect.y,
            monitor.rect.width,
            monitor.rect.height,
            monitor.primary,
            monitor.dpi_x as f64 / 96.0 * 100.0
        );
    }
}

/// 冻结帧的 BGRA 与 PNG 必须描述同一画面（抽样比较 5 个点，防止编码/裁剪错位）。
///
/// 依赖真实帧（`#[ignore]`）：取帧后用 png 解码核对抽样像素颜色一致。
#[tokio::test]
#[ignore = "需要真实交互桌面：cargo test --test capture_real -- --ignored --nocapture"]
async fn real_frame_png_pixels_match_bgra_samples() {
    let frame = Win32Capturer::new()
        .capture_virtual_desktop()
        .await
        .expect("真实捕获成功");
    let bgra = frame.bgra.as_deref().expect("bgra");
    let data_url = frame.data_url.as_ref().expect("data url");
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_url.trim_start_matches(PNG_DATA_URL_PREFIX))
        .expect("base64");
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("PNG 头");
    let buffer_size = reader.output_buffer_size().expect("PNG 输出缓冲大小");
    let mut buffer = vec![0u8; buffer_size];
    let info = reader.next_frame(&mut buffer).expect("PNG 解码");
    assert_eq!((info.width, info.height), (frame.width, frame.height));

    // 均匀抽样 5 个点（含左上/中心/右下附近），核对 BGRA 与 PNG RGBA 一致。
    let points = [
        (0u32, 0u32),
        (frame.width / 2, frame.height / 2),
        (frame.width - 1, 0),
        (0, frame.height - 1),
        (frame.width * 3 / 4, frame.height * 3 / 4),
    ];
    for (x, y) in points {
        let bgra_offset = (y as usize * frame.width as usize + x as usize) * 4;
        let png_offset = (y as usize * info.width as usize + x as usize) * 4;
        let bgra_px = &bgra[bgra_offset..bgra_offset + 4];
        let png_px = &buffer[png_offset..png_offset + 4];
        assert_eq!(
            (png_px[0], png_px[1], png_px[2]),
            (bgra_px[2], bgra_px[1], bgra_px[0]),
            "({x},{y}) BGRA 与 PNG 像素不一致"
        );
        assert_eq!(png_px[3], 255, "PNG alpha 必须强制 255（不透明）");
    }
    println!("PNG/BGRA 抽样像素一致：{} 个点", points.len());
}
