//! 屏幕捕获（M04 / T2 真实实现）。
//!
//! 抽象为 [`ScreenCapturer`] trait：
//! - **真实实现** [`Win32Capturer`]：`GetDC(NULL)` + `CreateDIBSection`（32bpp 顶朝下）+
//!   `BitBlt(SRCCOPY|CAPTUREBLT)` 捕获**虚拟桌面物理像素外接矩形**（允许负原点），产出
//!   原始 BGRA（供 OCR）与 PNG data URL（供 overlay 显示）。
//! - **骨架实现** [`SkeletonCapturer`]：不调用 Win32，用主显示器物理尺寸合成帧，
//!   `data_url = None`、`bgra = None`（`advanced.skeletonMode = true` 时使用，CI/无桌面安全）。
//!
//! # 坐标与 DPI 口径（S-01）
//!
//! - 帧尺寸 = 所有显示器物理矩形的最小外接矩形（`GetSystemMetrics(SM_*VIRTUALSCREEN)`），
//!   与 Tauri `available_monitors` 外接矩形同口径；**混合 DPI 不缩放**；
//! - 捕获前 best-effort 设置进程 DPI 感知为 `PER_MONITOR_AWARE_V2`（tao/Tauri 已设置则忽略报错），
//!   否则 `GetDC(NULL)` 可能返回 DPI 虚拟化后的画面（尺寸/像素都错）；
//! - 帧局部坐标 + 帧原点（= 虚拟桌面左上角，可为负）= 绝对物理坐标，OCR/选区都用绝对坐标。
//!
//! # S-09 黑屏检测与降级（纯 safe 算法 + 真实捕获）
//!
//! 真实 BitBlt 调用集中在本文件的 `platform::capture::win32` 子模块（唯一 `unsafe` 出口）；
//! 判定仍是**可单测的纯函数**：像素亮度统计 → 黑屏/空帧判定 → 降级动作 → SA-1001 映射。
//! 真实对比证据见 `docs/spikes/S-09-截图失败降级与WGC对比.md`，关键实测结论：
//!
//! - **WDA_MONITOR(0x01) / 真实 DRM 保护面**：BitBlt 与 WGC 都只能得到纯黑帧
//!   （本次 0x01 实测 900×600 区域 blackRatio=1.000000、均值 0.00、标准差 0.00），
//!   WGC **不能绕过**保护，降级后应返回 SA-1001 并给出可操作提示；
//! - **WDA_EXCLUDEFROMCAPTURE(0x11)**：窗口在截屏中「消失」（露出背后内容，不是黑屏），
//!   WGC 窗口项 **完全取不到帧**（3/3 次 5s 超时）——因此降级链除了「黑帧」还要处理
//!   「空帧 / 未取到帧」；
//! - 黑屏阈值按 **目标区域** 定义（`>99% 像素亮度 <16 且均值 <16 且标准差 <6`）。混合 DPI
//!   双屏下若只有主屏被受保护内容覆盖，虚拟桌面整帧黑占比只有 ~76%，整帧判定会漏检；
//!   因此本模块提供 [`region_luma_stats`]（绝对物理 rect + 帧原点 → 区域统计），
//!   `selection_commit` 对**选区切片**调用；冻结时另按显示器切片做诊断日志。
//!
//! # 真实证据
//!
//! 集成测试 `src-tauri/tests/capture_real.rs`（`#[ignore]`，需要真实桌面）与运行期环境变量
//! `SNIP_ASK_CAPTURE_DUMP=<dir>` 均会落盘真实 PNG + 几何/统计 JSON 到 `.verify/v1/capture/`。

#![allow(unsafe_code)]

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::json;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{FrozenFrameMeta, RectPhys, VirtualDesktop};

use super::coords;

/// PNG data URL 前缀（overlay `<img src>` 直接可用）。
pub const PNG_DATA_URL_PREFIX: &str = "data:image/png;base64,";

/// 黑屏/内容判定默认采样上限（约 1 Mpx）；整帧/区域统计都按该预算降采样，
/// 把 7 Mpx 虚拟桌面的判定从数百毫秒降到毫秒级（S-09「按像素预算降采样」）。
pub const DEFAULT_STATS_MAX_SAMPLES: usize = 1_000_000;

/// 冻结帧：屏幕上某一时刻的静态画面。
///
/// 字段契约见 `spec-v1.md` §3.4：`data_url` 与 `bgra` 在真实捕获时必填；
/// 骨架模式两者均为 `None`（前端渲染占位背景，OCR 走骨架引擎）。
#[derive(Debug, Clone, Default)]
pub struct FrozenFrame {
    /// 帧宽（物理像素，虚拟桌面外接矩形）。
    pub width: u32,
    /// 帧高（物理像素，虚拟桌面外接矩形）。
    pub height: u32,
    /// PNG base64 data URL；真实捕获必填，骨架模式为 `None`。
    pub data_url: Option<String>,
    /// 原始 BGRA（顶朝下，stride = `width * 4`），供 OCR 直接消费。
    pub bgra: Option<Arc<Vec<u8>>>,
}

impl FrozenFrame {
    /// 骨架帧：只有尺寸，无像素数据。
    pub fn skeleton(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data_url: None,
            bgra: None,
        }
    }

    /// 由会话里持久化的 `FrozenFrameMeta` 还原（仅尺寸 + data URL，无 BGRA）。
    ///
    /// 用于 overlay 快照 / 旧会话兼容；OCR 需要 `bgra` 时应优先用
    /// `AppState::frozen_frame()` 取完整帧。
    pub fn from_meta(meta: &FrozenFrameMeta) -> Self {
        Self {
            width: meta.width,
            height: meta.height,
            data_url: meta.data_url.clone(),
            bgra: None,
        }
    }

    /// 由真实捕获的 BGRA 构造冻结帧：编码 PNG data URL + 持有 BGRA（OCR 用）。
    pub fn from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> AppResult<Self> {
        let data_url = encode_png_data_url(width, height, &bgra)?;
        Ok(Self {
            width,
            height,
            data_url: Some(data_url),
            bgra: Some(Arc::new(bgra)),
        })
    }

    /// 转换为契约中的 `FrozenFrameMeta`。
    pub fn to_meta(&self) -> FrozenFrameMeta {
        FrozenFrameMeta {
            width: self.width,
            height: self.height,
            data_url: self.data_url.clone(),
        }
    }

    /// BGRA 缓冲长度是否符合 `width * height * 4`（溢出/截断均返回 false）。
    pub fn has_complete_bgra(&self) -> bool {
        self.bgra.as_ref().is_some_and(|bgra| {
            (self.width as usize)
                .checked_mul(self.height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .is_some_and(|expected| bgra.len() >= expected)
        })
    }

    /// 整帧亮度统计（按 [`DEFAULT_STATS_MAX_SAMPLES`] 采样）；无 BGRA 返回 `None`。
    pub fn luma_stats(&self) -> Option<FrameLumaStats> {
        let bgra = self.bgra.as_deref()?;
        if !self.has_complete_bgra() {
            return None;
        }
        Some(luma_stats_from_bgra_sampled(
            bgra,
            DEFAULT_STATS_MAX_SAMPLES,
        ))
    }

    /// 目标区域（绝对物理坐标）的亮度统计；区域与帧不相交 / 无 BGRA 时返回 `None`。
    pub fn region_luma_stats(
        &self,
        frame_origin: &RectPhys,
        target: &RectPhys,
    ) -> Option<FrameLumaStats> {
        let bgra = self.bgra.as_deref()?;
        if !self.has_complete_bgra() {
            return None;
        }
        region_luma_stats(
            bgra,
            self.width,
            self.height,
            frame_origin,
            target,
            DEFAULT_STATS_MAX_SAMPLES,
        )
    }
}

/// 屏幕捕获抽象（真实实现 [`Win32Capturer`]，骨架实现 [`SkeletonCapturer`]）。
#[async_trait]
pub trait ScreenCapturer: Send + Sync {
    /// 捕获虚拟桌面。
    async fn capture_virtual_desktop(&self) -> AppResult<FrozenFrame>;
}

// ───────────────────────── 虚拟桌面几何（Tauri 口径，S-01） ─────────────────────────

/// 虚拟桌面几何（S-01：多显示器外接矩形 + overlay 尺寸 / 坐标换算基准）。
///
/// - `rect`：**所有显示器物理矩形的最小外接矩形**，允许负坐标（副屏在主屏左上时本机为
///   `(-1080,-470) 3640x1920`）；与 Win32 `GetSystemMetrics(SM_*VIRTUALSCREEN)` 口径一致。
/// - `dpr`：**预期**窗口缩放比，取覆盖虚拟桌面左上角的显示器的 `scale_factor()`
///   （Windows 决定跨屏窗口 DPI 的归属屏）。overlay 的最终 dpr 必须以窗口实际
///   `scale_factor()` 为准（见 `app::windows::overlay_virtual_desktop`），因为单个跨屏
///   WebView 只有一个 devicePixelRatio。
pub fn virtual_desktop_geometry(app: &tauri::AppHandle) -> VirtualDesktop {
    let monitors = app.available_monitors().unwrap_or_default();
    let rects: Vec<RectPhys> = monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            RectPhys {
                x: position.x as f64,
                y: position.y as f64,
                width: size.width as f64,
                height: size.height as f64,
            }
        })
        .collect();
    let Some(rect) = coords::union_rect(rects) else {
        // 没有任何显示器（远程会话异常）：回退 1080p，保证 overlay 仍可创建。
        return VirtualDesktop {
            rect: RectPhys {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            dpr: 1.0,
        };
    };
    let anchor = monitors
        .iter()
        .find(|monitor| {
            let position = monitor.position();
            coords::rect_contains(&rect, position.x as f64, position.y as f64)
        })
        .or_else(|| monitors.first());
    let dpr = anchor.map(|monitor| monitor.scale_factor()).unwrap_or(1.0);
    coords::virtual_desktop(rect.x, rect.y, rect.width, rect.height, dpr)
}

/// 捕获帧尺寸与 Tauri 显示器外接矩形的差异（诊断；>1px 才认为不一致）。
///
/// 真实捕获以 Win32 虚拟屏幕矩形为源；overlay 以 Tauri `available_monitors` 为源。
/// 二者理论上一致（同一 `GetMonitorInfo` 数据源），不一致会导致 OCR 绝对坐标整体偏移，
/// 因此在 `capture_freeze` 中记录 warn 并写入证据 JSON。
pub fn geometry_mismatch(frame: &FrozenFrame, tauri_rect: &RectPhys) -> Option<String> {
    let expected_width = tauri_rect.width.round().max(1.0);
    let expected_height = tauri_rect.height.round().max(1.0);
    let dw = (frame.width as f64 - expected_width).abs();
    let dh = (frame.height as f64 - expected_height).abs();
    if dw <= 1.0 && dh <= 1.0 {
        return None;
    }
    Some(format!(
        "捕获帧 {}x{} 与 Tauri 虚拟桌面 {:.0}x{:.0}（原点到 {:.0},{:.0}）不一致：Δw={dw:.0} Δh={dh:.0}",
        frame.width, frame.height, expected_width, expected_height, tauri_rect.x, tauri_rect.y
    ))
}

// ───────────────────────── PNG 编码（真实帧 → overlay data URL） ─────────────────────────

/// BGRA（顶朝下）→ PNG 字节。GDI BitBlt 的 alpha 通道不可信，统一写 255（不透明）。
pub fn encode_png_bytes(width: u32, height: u32, bgra: &[u8]) -> AppResult<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| frame_data_error("帧尺寸溢出（width * height * 4 超出 usize）"))?;
    if width == 0 || height == 0 || bgra.len() < expected {
        return Err(frame_data_error(&format!(
            "帧数据长度不符：期望 {expected} 字节（{width}x{height} BGRA），实际 {} 字节",
            bgra.len()
        )));
    }

    // BGRA → RGBA（PNG 无 BGRA 颜色类型），alpha 强制 255。
    let mut rgba = Vec::with_capacity(expected);
    for px in bgra[..expected].chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Fast：fdeflate 专用实现，7 Mpx 桌面帧编码 ~100ms 量级（Balanced 更慢、体积略小）。
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|err| frame_data_error(&format!("PNG 写头失败: {err}")))?;
        writer
            .write_image_data(&rgba)
            .map_err(|err| frame_data_error(&format!("PNG 编码失败: {err}")))?;
    }
    Ok(png_bytes)
}

/// BGRA（顶朝下）→ `data:image/png;base64,...`（overlay `<img src>` 直接可用）。
pub fn encode_png_data_url(width: u32, height: u32, bgra: &[u8]) -> AppResult<String> {
    let png_bytes = encode_png_bytes(width, height, bgra)?;
    let base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(format!("{PNG_DATA_URL_PREFIX}{base64}"))
}

/// 按**绝对物理选区**从冻结帧裁出 PNG data URL（图片搜索只上传用户框选区域，不上传整桌面）。
///
/// - `frame_origin` = 捕获时虚拟桌面左上角（可为负），`target` = 选区绝对物理矩形；
/// - 帧无 BGRA（骨架模式）/与选区不相交时返回 `Ok(None)`，调用方应退化为「不上传图片」；
/// - 像素越界或长度不符返回可操作错误（不静默上传错误图像）。
pub fn cropped_data_url(
    frame: &FrozenFrame,
    frame_origin: &RectPhys,
    target: &RectPhys,
) -> AppResult<Option<String>> {
    let Some(bgra) = frame.bgra.as_deref() else {
        return Ok(None);
    };
    let Some((x, y, width, height)) =
        region_local_rect(frame.width, frame.height, frame_origin, target)
    else {
        return Ok(None);
    };
    let stride = frame.width as usize * 4;
    let expected = stride
        .checked_mul(frame.height as usize)
        .ok_or_else(|| frame_data_error("冻结帧尺寸溢出"))?;
    if bgra.len() < expected {
        return Err(frame_data_error("冻结帧像素长度不足，无法裁切选区"));
    }
    let row_bytes = width as usize * 4;
    let mut cropped = Vec::with_capacity(row_bytes * height as usize);
    for row in y..(y + height) {
        let start = row as usize * stride + x as usize * 4;
        cropped.extend_from_slice(&bgra[start..start + row_bytes]);
    }
    encode_png_data_url(width, height, &cropped).map(Some)
}

/// 帧数据/编码错误：SA-1001（可重试，不谎称已尝试 WGC）。
fn frame_data_error(detail: &str) -> AppError {
    AppError::new(ErrorCode::CaptureFailed, Some(detail.to_string()))
        .with_message("屏幕捕获失败：帧数据不完整或编码失败，请重试")
        .with_fallback(["retry"])
        .with_context(json!({ "capture.backend": CaptureBackend::BitBlt.as_str() }))
}

// ───────────────────────── S-09：黑屏检测 / 降级决策（纯 safe，无 Win32） ─────────────────────────

/// 黑像素的亮度上限（0–255 灰度，含）。纯黑帧的像素亮度为 0，受保护/DRM 黑屏实测为 0。
pub const BLACK_LUMA_MAX: f64 = 16.0;
/// 判黑所需的最小黑像素占比（> 99%），避免深色 UI 被误判。
pub const BLACK_RATIO_MIN: f64 = 0.99;
/// 判黑所需的最大亮度标准差（< 6）；黑底白字/深色图片会因方差过大而不判黑。
pub const BLACK_STD_DEV_MAX: f64 = 6.0;

/// 一帧的亮度统计（灰度口径）。
///
/// 由采样像素（BGRA / RGBA 四字节一组，见 [`luma_stats_from_bgra`]）计算；真实捕获实现
/// 按像素预算降采样（[`luma_stats_from_bgra_sampled`]），避免逐像素统计成为冻结时延瓶颈。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLumaStats {
    /// 参与统计的像素数（0 表示空帧）。
    pub pixels: usize,
    /// 亮度 `< BLACK_LUMA_MAX` 的像素数。
    pub black_pixels: usize,
    /// 平均亮度（0–255）。
    pub mean_luma: f64,
    /// 亮度总体方差。
    pub variance: f64,
}

impl FrameLumaStats {
    /// 黑像素占比（0.0–1.0）；空帧返回 0。
    pub fn black_ratio(&self) -> f64 {
        if self.pixels == 0 {
            0.0
        } else {
            self.black_pixels as f64 / self.pixels as f64
        }
    }

    /// 亮度标准差。
    pub fn std_dev(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }

    /// 是否为空帧（没有可统计像素）。
    pub fn is_empty(&self) -> bool {
        self.pixels == 0
    }

    /// 诊断/证据用 JSON（与 spike 工具字段名一致：blackRatio / meanLuma / stdDev）。
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "pixels": self.pixels,
            "blackPixels": self.black_pixels,
            "blackRatio": self.black_ratio(),
            "meanLuma": self.mean_luma,
            "stdDev": self.std_dev(),
        })
    }
}

/// BT.601 亮度（B/G/R 顺序，与 spike 工具逐字一致）。
fn luma_of_bgra(px: &[u8]) -> f64 {
    0.114 * f64::from(px[0]) + 0.587 * f64::from(px[1]) + 0.299 * f64::from(px[2])
}

/// 从 BGRA 像素缓冲（每像素 4 字节：B/G/R/A）统计亮度；不足 4 字节的尾巴忽略。
///
/// 亮度按 BT.601 加权：`0.114*B + 0.587*G + 0.299*R`，与 spike 工具
/// `scripts/spikes/s09/Program.cs` 的判定口径逐字一致（记录里的 blackRatio 可直接对照）。
pub fn luma_stats_from_bgra(pixels: &[u8]) -> FrameLumaStats {
    let mut count = 0usize;
    let mut black_pixels = 0usize;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    for px in pixels.chunks_exact(4) {
        let luma = luma_of_bgra(px);
        count += 1;
        sum += luma;
        sum_sq += luma * luma;
        if luma < BLACK_LUMA_MAX {
            black_pixels += 1;
        }
    }
    stats_from_sums(count, black_pixels, sum, sum_sq)
}

/// 降采样版亮度统计：像素数超过 `max_samples` 时按固定步长均匀采样。
///
/// 判黑阈值作用在**采样集**上（纯黑帧 100% 采样为黑，内容帧远离阈值，结论不变）；
/// `max_samples == 0` 或像素数不足时退化为全量 [`luma_stats_from_bgra`]。
pub fn luma_stats_from_bgra_sampled(pixels: &[u8], max_samples: usize) -> FrameLumaStats {
    let total = pixels.len() / 4;
    if total == 0 {
        return FrameLumaStats {
            pixels: 0,
            black_pixels: 0,
            mean_luma: 0.0,
            variance: 0.0,
        };
    }
    if max_samples == 0 || total <= max_samples {
        return luma_stats_from_bgra(pixels);
    }
    let step = total.div_ceil(max_samples).max(1);
    let mut count = 0usize;
    let mut black_pixels = 0usize;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut index = 0usize;
    while index < total {
        let offset = index * 4;
        let luma = luma_of_bgra(&pixels[offset..offset + 4]);
        count += 1;
        sum += luma;
        sum_sq += luma * luma;
        if luma < BLACK_LUMA_MAX {
            black_pixels += 1;
        }
        index += step;
    }
    stats_from_sums(count, black_pixels, sum, sum_sq)
}

fn stats_from_sums(count: usize, black_pixels: usize, sum: f64, sum_sq: f64) -> FrameLumaStats {
    let mean_luma = if count == 0 { 0.0 } else { sum / count as f64 };
    let variance = if count == 0 {
        0.0
    } else {
        (sum_sq / count as f64 - mean_luma * mean_luma).max(0.0)
    };
    FrameLumaStats {
        pixels: count,
        black_pixels,
        mean_luma,
        variance,
    }
}

/// 把绝对物理矩形裁到帧内，得到帧局部整数矩形 `(x, y, w, h)`；不相交返回 `None`。
///
/// 输入使用四舍五入到物理像素；`RectPhys` 允许负原点（虚拟桌面左上角为负）。
pub fn region_local_rect(
    frame_width: u32,
    frame_height: u32,
    frame_origin: &RectPhys,
    target: &RectPhys,
) -> Option<(u32, u32, u32, u32)> {
    let target = target.normalize();
    if target.width <= 0.0 || target.height <= 0.0 {
        return None;
    }
    let frame_width_f = frame_width as f64;
    let frame_height_f = frame_height as f64;
    let left = (target.x - frame_origin.x).round();
    let top = (target.y - frame_origin.y).round();
    let right = (target.x + target.width - frame_origin.x).round();
    let bottom = (target.y + target.height - frame_origin.y).round();
    let x0 = left.clamp(0.0, frame_width_f);
    let y0 = top.clamp(0.0, frame_height_f);
    let x1 = right.clamp(0.0, frame_width_f);
    let y1 = bottom.clamp(0.0, frame_height_f);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// 目标区域（**绝对物理坐标**，可为负）的亮度统计；与帧不相交 / 空区域返回 `None`。
///
/// S-09 处置：黑屏判定必须按目标区域切片执行——混合 DPI 双屏下整帧黑占比可能只有
/// ~76%，整帧阈值会漏检；`selection_commit` 用本函数判定「选区是否受保护/黑屏」。
pub fn region_luma_stats(
    frame: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_origin: &RectPhys,
    target: &RectPhys,
    max_samples: usize,
) -> Option<FrameLumaStats> {
    let (x, y, width, height) = region_local_rect(frame_width, frame_height, frame_origin, target)?;
    let expected = (frame_width as usize)
        .checked_mul(frame_height as usize)
        .and_then(|pixels| pixels.checked_mul(4))?;
    if frame.len() < expected {
        return None;
    }
    let total = width as usize * height as usize;
    if total == 0 {
        return None;
    }
    let step = if max_samples == 0 || total <= max_samples {
        1
    } else {
        total.div_ceil(max_samples).max(1)
    };
    let mut count = 0usize;
    let mut black_pixels = 0usize;
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut index = 0usize;
    while index < total {
        let row = index / width as usize;
        let column = index % width as usize;
        let offset = ((y as usize + row) * frame_width as usize + (x as usize + column)) * 4;
        let luma = luma_of_bgra(&frame[offset..offset + 4]);
        count += 1;
        sum += luma;
        sum_sq += luma * luma;
        if luma < BLACK_LUMA_MAX {
            black_pixels += 1;
        }
        index += step;
    }
    Some(stats_from_sums(count, black_pixels, sum, sum_sq))
}

/// 黑屏判定：黑像素占比 ≥ [`BLACK_RATIO_MIN`]、均值 < [`BLACK_LUMA_MAX`] 且标准差
/// < [`BLACK_STD_DEV_MAX`]。空帧不算黑帧（空帧另由 [`is_capture_failed`] 判失败）。
pub fn is_black_frame(stats: &FrameLumaStats) -> bool {
    !stats.is_empty()
        && stats.black_ratio() >= BLACK_RATIO_MIN
        && stats.mean_luma < BLACK_LUMA_MAX
        && stats.std_dev() < BLACK_STD_DEV_MAX
}

/// 捕获是否失败：空帧（没取到画面）或黑屏。
///
/// 真实降级链中 `None` 表示 WGC 窗口项超时/未取到帧——S-09 实测
/// `WDA_EXCLUDEFROMCAPTURE` 窗口的 WGC 窗口项 3/3 次均为 `None`，因此必须按失败处理。
pub fn is_capture_failed(stats: Option<&FrameLumaStats>) -> bool {
    match stats {
        None => true,
        Some(stats) => stats.is_empty() || is_black_frame(stats),
    }
}

/// 捕获后端（契约 §8.2 的 SA-1001 fallback `["wgc", "retry"]` 对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    /// 主路径：BitBlt 屏幕拷贝。
    BitBlt,
    /// 降级路径：Windows.Graphics.Capture。
    Wgc,
}

impl CaptureBackend {
    /// 契约/诊断用的后端名（`capture.backend` 上下文的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BitBlt => "bitblt",
            Self::Wgc => "wgc",
        }
    }
}

/// 单条捕获路径的失败原因（用于填写 SA-1001 的 detail / context）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFailure {
    /// 空帧 / 未取到画面（如 WGC 窗口项超时）。
    EmptyFrame,
    /// 黑屏（DRM / 受保护内容 / 全屏独占的典型表现）。
    BlackFrame,
}

/// 由亮度统计推导失败原因；`None` 视为空帧。
pub fn classify_failure(stats: Option<&FrameLumaStats>) -> CaptureFailure {
    match stats {
        Some(stats) if !stats.is_empty() && is_black_frame(stats) => CaptureFailure::BlackFrame,
        _ => CaptureFailure::EmptyFrame,
    }
}

/// 构造 SA-1001 捕获失败错误（retryable = true，fallback = `["wgc", "retry"]`）。
///
/// 契约字段由 [`AppError::new`] 按错误码表补全；本函数额外：
/// - 覆盖面向用户的 message 为可操作提示（DRM/全屏独占 + 已尝试 WGC）；
/// - 在 `context` 记录 `capture.backend` 与 `capture.blackFrame` / `capture.blackRatio`，
///   便于日志与前端排查（UI 会显示 `message`，详见 `src/windows/overlay/OverlayWindow.tsx`）。
///
/// 真实运行期只实现了 BitBlt（WGC 非主路径，见 spec §7），实际错误构造请用
/// [`bitblt_capture_failed_error`]（不谎称已尝试 WGC）。
pub fn capture_failed_error(backend: CaptureBackend, stats: Option<&FrameLumaStats>) -> AppError {
    let failure = classify_failure(stats);
    let reason = match failure {
        CaptureFailure::EmptyFrame => "未能取到画面".to_string(),
        CaptureFailure::BlackFrame => {
            let stats = stats.expect("BlackFrame 必然带有亮度统计");
            format!(
                "画面为黑屏（黑像素占比 {:.1}%，平均亮度 {:.1}）",
                stats.black_ratio() * 100.0,
                stats.mean_luma
            )
        }
    };
    let detail = format!("backend={}：{}", backend.as_str(), reason);
    let mut error = AppError::new(ErrorCode::CaptureFailed, Some(detail))
        .with_message(
            "屏幕捕获失败：画面可能受保护（DRM/受保护视频）或处于全屏独占，WGC 也无法获取；请退出独占应用或换用其他截图方式后重试",
        )
        // 与契约 §8.2 SA-1001 的 fallback 一致，显式写出便于单测断言与前端兜底。
        .with_fallback(["wgc", "retry"]);
    let context = match (failure, stats) {
        (CaptureFailure::BlackFrame, Some(stats)) => json!({
            "capture.backend": backend.as_str(),
            "capture.blackFrame": true,
            "capture.blackRatio": stats.black_ratio(),
            "capture.meanLuma": stats.mean_luma,
        }),
        _ => json!({
            "capture.backend": backend.as_str(),
            "capture.blackFrame": false,
        }),
    };
    error = error.with_context(context);
    error
}

/// BitBlt 主路径失败（WGC 未实现/未尝试）的 SA-1001。
///
/// 与 [`capture_failed_error`] 保持相同的 code / retryable / fallback / 上下文，
/// 但 message 不谎称「已尝试 WGC」，改为可操作的「退出独占应用 / 改用系统截图」。
pub fn bitblt_capture_failed_error(stats: Option<&FrameLumaStats>) -> AppError {
    capture_failed_error(CaptureBackend::BitBlt, stats).with_message(
        "屏幕捕获失败：画面可能受保护（DRM/受保护视频）或处于全屏独占；请退出独占应用、解除锁屏或改用系统截图工具后重试",
    )
}

/// 选区切片黑屏失败的 SA-1001（S-09「按目标区域判定」）。
///
/// `selection_commit` 在真实帧上对选区做黑/空判定：选区纯黑说明用户框到的是 DRM 黑面
/// 或全屏独占画面，直接返回可操作错误，而不是静默返回空 OCR 结果。
pub fn region_capture_failed_error(region: &RectPhys, stats: &FrameLumaStats) -> AppError {
    let region = region.normalize();
    let detail = format!(
        "backend=bitblt：选区黑屏（region=({:.0},{:.0},{:.0},{:.0})，黑像素占比 {:.1}%，平均亮度 {:.1}）",
        region.x,
        region.y,
        region.width,
        region.height,
        stats.black_ratio() * 100.0,
        stats.mean_luma
    );
    AppError::new(ErrorCode::CaptureFailed, Some(detail))
        .with_message(
            "选区内画面为黑屏（可能受 DRM 保护或处于全屏独占）；请重新框选，或退出独占应用后重试",
        )
        .with_fallback(["retry"])
        .with_context(json!({
            "capture.backend": CaptureBackend::BitBlt.as_str(),
            "capture.blackFrame": true,
            "capture.blackRatio": stats.black_ratio(),
            "capture.meanLuma": stats.mean_luma,
            "capture.region": {
                "x": region.x,
                "y": region.y,
                "width": region.width,
                "height": region.height,
            },
        }))
}

// ───────────────────────── 骨架捕获器 ─────────────────────────

/// 骨架捕获器：不调用 Win32，用虚拟桌面物理尺寸合成帧。
pub struct SkeletonCapturer {
    app: tauri::AppHandle,
}

impl SkeletonCapturer {
    /// 创建骨架捕获器。
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait]
impl ScreenCapturer for SkeletonCapturer {
    async fn capture_virtual_desktop(&self) -> AppResult<FrozenFrame> {
        let desktop = virtual_desktop_geometry(&self.app);
        let width = desktop.rect.width.max(1.0).round() as u32;
        let height = desktop.rect.height.max(1.0).round() as u32;
        if width == 0 || height == 0 {
            return Err(AppError::new(
                ErrorCode::CaptureFailed,
                Some("无法获取显示器尺寸".to_string()),
            ));
        }
        Ok(FrozenFrame::skeleton(width, height))
    }
}

// ───────────────────────── 真实捕获：DPI / 显示器几何 ─────────────────────────

/// 进程 DPI 感知状态（捕获分辨率正确性的前提）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpiAwareness {
    /// 本次调用设置成功（测试二进制/未初始化进程）。
    Set,
    /// Tauri/tao 已设置（生产进程的常态）。
    AlreadyPerMonitor,
    /// 设置失败且当前非逐显示器感知：捕获可能被 DPI 虚拟化。
    Unaware,
    /// 非 Windows 平台。
    Unsupported,
}

impl DpiAwareness {
    /// 诊断用名称。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Set => "perMonitorV2(set)",
            Self::AlreadyPerMonitor => "perMonitorV2(already)",
            Self::Unaware => "unaware",
            Self::Unsupported => "unsupported",
        }
    }

    /// 是否逐显示器 DPI 感知（只有此时 BitBlt 尺寸 == 物理像素）。
    pub fn is_per_monitor(self) -> bool {
        matches!(self, Self::Set | Self::AlreadyPerMonitor)
    }
}

/// 一个显示器的 Win32 物理矩形 + DPI（诊断/证据用）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorInfo {
    /// 显示器物理矩形（虚拟桌面坐标，可为负）。
    pub rect: RectPhys,
    /// 是否主显示器。
    pub primary: bool,
    /// 有效 DPI（X）。
    pub dpi_x: u32,
    /// 有效 DPI（Y）。
    pub dpi_y: u32,
}

impl MonitorInfo {
    /// 诊断/证据用 JSON。
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "x": self.rect.x,
            "y": self.rect.y,
            "width": self.rect.width,
            "height": self.rect.height,
            "primary": self.primary,
            "dpiX": self.dpi_x,
            "dpiY": self.dpi_y,
            "scaleFactor": self.dpi_x as f64 / 96.0,
        })
    }
}

/// 确保进程为逐显示器 DPI 感知（幂等；结果缓存）。
///
/// 生产进程由 tao/Tauri 在事件循环初始化时设置，本函数再设置会返回失败——此时以
/// `GetThreadDpiAwarenessContext` 复核，已是逐显示器感知即视为成功。测试二进制没有
/// Tauri 初始化，本函数会真正设置，保证证据与生产口径一致。
pub fn ensure_dpi_awareness() -> DpiAwareness {
    static CACHED: OnceLock<DpiAwareness> = OnceLock::new();
    *CACHED.get_or_init(win32_impl::detect_and_set_dpi_awareness)
}

/// Win32 虚拟屏幕物理矩形（`SM_*VIRTUALSCREEN`，允许负原点）；不可用返回 `None`。
pub fn system_virtual_desktop_rect() -> Option<RectPhys> {
    win32_impl::virtual_screen_rect()
}

/// Win32 `EnumDisplayMonitors` 的显示器列表（按 x/y 排序，便于证据对照）；非 Windows 为空。
pub fn system_monitors() -> Vec<MonitorInfo> {
    win32_impl::enumerate_monitors()
}

// ───────────────────────── 真实捕获：Win32 BitBlt ─────────────────────────

/// 真实捕获器（BitBlt 虚拟桌面）。构造无参，便于集成测试直接使用。
#[derive(Debug, Clone, Copy, Default)]
pub struct Win32Capturer;

impl Win32Capturer {
    /// 创建真实捕获器。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ScreenCapturer for Win32Capturer {
    async fn capture_virtual_desktop(&self) -> AppResult<FrozenFrame> {
        // GDI BitBlt + PNG 编码是同步阻塞的（本机 7 Mpx 实测约 100–300ms），
        // 放到 blocking 线程池，避免占用 tokio worker 影响命令/事件分发。
        tokio::task::spawn_blocking(capture_virtual_desktop_real)
            .await
            .map_err(|err| {
                AppError::new(
                    ErrorCode::CaptureFailed,
                    Some(format!("捕获任务异常终止: {err}")),
                )
                .with_message("屏幕捕获失败：捕获线程异常，请重试")
            })?
    }
}

/// 同步执行一次真实虚拟桌面捕获（供 [`Win32Capturer`] 与集成测试使用）。
///
/// 流程：DPI 感知 → Win32 虚拟屏幕矩形 → BitBlt 到 32bpp DIB → BGRA 拷贝 → PNG data URL。
/// 黑屏/空帧判定由 `services::capture_service::CaptureService::freeze` 统一执行（复用既有
/// SA-1001 逻辑），本函数只负责「拿到像素」。
pub fn capture_virtual_desktop_real() -> AppResult<FrozenFrame> {
    use std::time::Instant;

    let started = Instant::now();
    let awareness = ensure_dpi_awareness();
    if !awareness.is_per_monitor() {
        tracing::warn!(
            target: "snip_ask_lib::trace",
            awareness = awareness.as_str(),
            "进程不是逐显示器 DPI 感知，真实捕获尺寸可能被 DPI 虚拟化"
        );
    }

    let rect = system_virtual_desktop_rect().ok_or_else(|| {
        AppError::new(
            ErrorCode::CaptureFailed,
            Some(
                "GetSystemMetrics(SM_*VIRTUALSCREEN) 返回非法矩形（可能处于锁屏/远程会话）"
                    .to_string(),
            ),
        )
        .with_message("屏幕捕获失败：无法读取屏幕尺寸（可能处于锁屏或远程会话），请解锁后重试")
        .with_fallback(["retry"])
    })?;
    let width = rect.width.round().max(1.0) as u32;
    let height = rect.height.round().max(1.0) as u32;

    let bgra = win32_impl::capture_bgra(&rect)?;
    if bgra.len() != width as usize * height as usize * 4 {
        return Err(frame_data_error(&format!(
            "BitBlt 返回像素长度不符：期望 {}，实际 {}",
            width as usize * height as usize * 4,
            bgra.len()
        )));
    }
    let frame = FrozenFrame::from_bgra(width, height, bgra)?;

    if let Some(stats) = frame.luma_stats() {
        tracing::debug!(
            target: "snip_ask_lib::trace",
            width,
            height,
            rect_x = rect.x,
            rect_y = rect.y,
            awareness = awareness.as_str(),
            black_ratio = stats.black_ratio(),
            mean_luma = stats.mean_luma,
            data_url_chars = frame.data_url.as_ref().map(String::len).unwrap_or(0),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "真实捕获完成"
        );
    }
    Ok(frame)
}

// ───────────────────────── 运行期真实证据落盘 ─────────────────────────

/// 捕获证据目录：环境变量 `SNIP_ASK_CAPTURE_DUMP=<dir>`（开发/验证钩子，生产不设置即无副作用）。
pub fn capture_dump_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("SNIP_ASK_CAPTURE_DUMP")
        .map(std::path::PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
}

/// 若设置了 `SNIP_ASK_CAPTURE_DUMP`，把真实帧 PNG + 几何/统计 JSON 落盘（证据用）。
///
/// 返回写出的 PNG 路径；未设置环境变量或写失败返回 `None`（**不影响捕获主流程**）。
/// `context` 由调用方补充（Tauri 虚拟桌面几何、overlay 对齐结果等）。
pub fn maybe_dump_capture(
    frame: &FrozenFrame,
    context: serde_json::Value,
) -> Option<std::path::PathBuf> {
    let dir = capture_dump_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let bgra = frame.bgra.as_deref()?;
    if !frame.has_complete_bgra() {
        return None;
    }

    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f").to_string();
    let png_bytes = encode_png_bytes(frame.width, frame.height, bgra).ok()?;
    let png_path = dir.join(format!("capture-{stamp}.png"));
    std::fs::write(&png_path, &png_bytes).ok()?;

    let monitors: Vec<serde_json::Value> =
        system_monitors().iter().map(MonitorInfo::to_json).collect();
    let meta = json!({
        "capturedAt": chrono::Utc::now().to_rfc3339(),
        "png": png_path.to_string_lossy(),
        "pngBytes": png_bytes.len(),
        "width": frame.width,
        "height": frame.height,
        "bgraBytes": bgra.len(),
        "dataUrlChars": frame.data_url.as_ref().map(String::len).unwrap_or(0),
        "dpiAwareness": ensure_dpi_awareness().as_str(),
        "win32VirtualDesktop": system_virtual_desktop_rect().map(|rect| json!({
            "x": rect.x,
            "y": rect.y,
            "width": rect.width,
            "height": rect.height,
        })),
        "win32Monitors": monitors,
        "frameStats": frame.luma_stats().map(|stats| stats.to_json()),
        "context": context,
    });
    let meta_path = dir.join(format!("capture-{stamp}.json"));
    std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta).ok()?).ok()?;
    Some(png_path)
}

// ───────────────────────── 平台实现（Windows） ─────────────────────────

#[cfg(windows)]
mod win32_impl {
    //! 唯一允许 `unsafe` 的 Win32 调用集中区（文件级 `#![allow(unsafe_code)]`）。
    //!
    //! 资源释放顺序：DIB 位图 → 还原 DC 旧对象 → 删除 HBITMAP → 删除内存 DC → 释放屏幕 DC，
    //! 全部路径都保证清理（失败早退时也逐个释放）。

    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::{copy_nonoverlapping, null_mut};

    use super::*;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors,
        GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CAPTUREBLT, DIB_RGB_COLORS, HDC, HGDIOBJ, HMONITOR, MONITORINFO, ROP_CODE, SRCCOPY,
    };
    use windows::Win32::UI::HiDpi::{
        GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor, GetThreadDpiAwarenessContext,
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        MDT_EFFECTIVE_DPI,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    /// 逐显示器 DPI 感知（≥ `PER_MONITOR_AWARE`）判定。
    fn is_per_monitor_aware() -> bool {
        // SAFETY: 两个 API 都是无参数的线程本地查询，无内存安全前提。
        unsafe {
            let context = GetThreadDpiAwarenessContext();
            GetAwarenessFromDpiAwarenessContext(context).0 >= 2
        }
    }

    pub(super) fn detect_and_set_dpi_awareness() -> DpiAwareness {
        // SAFETY: 无参数/进程级设置，调用失败只返回错误，不涉及未初始化内存。
        unsafe {
            if is_per_monitor_aware() {
                return DpiAwareness::AlreadyPerMonitor;
            }
            if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_ok() {
                return DpiAwareness::Set;
            }
            // 并发/抢先设置或已有消息循环：以实际状态为准。
            if is_per_monitor_aware() {
                DpiAwareness::AlreadyPerMonitor
            } else {
                DpiAwareness::Unaware
            }
        }
    }

    pub(super) fn virtual_screen_rect() -> Option<RectPhys> {
        // SAFETY: GetSystemMetrics 是纯查询。
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if width <= 0 || height <= 0 {
                return None;
            }
            Some(RectPhys {
                x: x as f64,
                y: y as f64,
                width: width as f64,
                height: height as f64,
            })
        }
    }

    /// `EnumDisplayMonitors` 回调：把显示器信息写入 `LPARAM` 指向的 `Vec`。
    ///
    /// SAFETY: 调用方保证 `data.0` 指向调用栈上有效且独占的 `Vec<MonitorInfo>`，
    /// 且回调只在 `EnumDisplayMonitors` 同步执行期间被调用。
    unsafe extern "system" fn collect_monitor(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let monitors = &mut *(data.0 as *mut Vec<MonitorInfo>);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        // SAFETY: info 已按 cbSize 初始化；hmonitor 由系统在回调中给出。
        if unsafe { GetMonitorInfoW(hmonitor, &mut info) }.as_bool() {
            let mut dpi_x = 96u32;
            let mut dpi_y = 96u32;
            // GetDpiForMonitor 在部分虚拟显示器上可能失败，失败时按 96 记录（不影响几何）。
            let _ =
                unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            monitors.push(MonitorInfo {
                rect: RectPhys {
                    x: info.rcMonitor.left as f64,
                    y: info.rcMonitor.top as f64,
                    width: (info.rcMonitor.right - info.rcMonitor.left) as f64,
                    height: (info.rcMonitor.bottom - info.rcMonitor.top) as f64,
                },
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
                dpi_x,
                dpi_y,
            });
        }
        BOOL(1)
    }

    pub(super) fn enumerate_monitors() -> Vec<MonitorInfo> {
        let mut monitors: Vec<MonitorInfo> = Vec::new();
        // SAFETY: 回调仅在本次同步调用期间访问 monitors；LPARAM 只携带该 Vec 的地址。
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(collect_monitor),
                LPARAM(&mut monitors as *mut Vec<MonitorInfo> as isize),
            );
        }
        monitors.sort_by(|a, b| {
            (a.rect.x as i64, a.rect.y as i64).cmp(&(b.rect.x as i64, b.rect.y as i64))
        });
        monitors
    }

    /// 捕获绝对物理矩形（虚拟桌面坐标，允许负值）为顶朝下 BGRA。
    pub(super) fn capture_bgra(rect: &RectPhys) -> AppResult<Vec<u8>> {
        let x = rect.x.round() as i32;
        let y = rect.y.round() as i32;
        let width = rect.width.round().max(1.0) as i32;
        let height = rect.height.round().max(1.0) as i32;
        // SAFETY: GetDC/ReleaseDC 成对；BitBlt 只在有效的屏幕 DC 与内存 DC 之间拷贝。
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return Err(capture_api_error("GetDC(NULL) 返回空句柄（桌面不可访问）"));
            }
            let result = capture_into_dib(screen, x, y, width, height);
            let _ = ReleaseDC(None, screen);
            result
        }
    }

    /// 在给定屏幕 DC 上 BitBlt 到 32bpp 顶朝下 DIB，返回 BGRA。
    ///
    /// # Safety
    /// `screen` 必须是 `GetDC(NULL)` 返回的有效屏幕 DC（调用方保证生命周期覆盖本函数）。
    unsafe fn capture_into_dib(
        screen: HDC,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> AppResult<Vec<u8>> {
        let mem = unsafe { CreateCompatibleDC(Some(screen)) };
        if mem.0.is_null() {
            return Err(capture_api_error("CreateCompatibleDC 失败"));
        }

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // 负高度 = 顶朝下（第 0 行即屏幕顶部），与 FrozenFrame.bgra 的 stride 口径一致。
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let mut bits: *mut c_void = null_mut();
        let bitmap = match unsafe {
            CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(err) => {
                let _ = unsafe { DeleteDC(mem) };
                return Err(capture_api_error(&format!("CreateDIBSection 失败: {err}")));
            }
        };
        if bits.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            let _ = unsafe { DeleteDC(mem) };
            return Err(capture_api_error("CreateDIBSection 未返回像素指针"));
        }

        let old = unsafe { SelectObject(mem, HGDIOBJ(bitmap.0)) };
        // CAPTUREBLT：包含分层窗口（overlay 隐藏时无影响；保证多窗口合成画面完整）。
        let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        let blt = unsafe { BitBlt(mem, 0, 0, width, height, Some(screen), x, y, rop) };

        let len = (width as usize) * (height as usize) * 4;
        let mut buffer = vec![0u8; len];
        // SAFETY: bits 指向至少 width*height*4 字节的 DIB 像素缓冲（CreateDIBSection 保证）。
        if blt.is_ok() {
            unsafe { copy_nonoverlapping(bits as *const u8, buffer.as_mut_ptr(), len) };
        }

        unsafe {
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
        }

        blt.map_err(|err| {
            capture_api_error(&format!(
                "BitBlt 失败（虚拟桌面 {width}x{height} @ {x},{y}）: {err}"
            ))
        })?;
        Ok(buffer)
    }
}

#[cfg(not(windows))]
mod win32_impl {
    use super::*;

    pub(super) fn detect_and_set_dpi_awareness() -> DpiAwareness {
        DpiAwareness::Unsupported
    }

    pub(super) fn virtual_screen_rect() -> Option<RectPhys> {
        None
    }

    pub(super) fn enumerate_monitors() -> Vec<MonitorInfo> {
        Vec::new()
    }
}

/// 真实捕获失败（Win32 API 级）的可操作 SA-1001：不伪造「已尝试 WGC」。
fn capture_api_error(detail: &str) -> AppError {
    AppError::new(ErrorCode::CaptureFailed, Some(detail.to_string()))
        .with_message(
            "屏幕捕获失败：无法读取屏幕画面（可能处于锁屏/远程会话，或显示驱动异常）；请解锁后重试，或改用系统截图工具",
        )
        .with_fallback(["retry"])
        .with_context(json!({ "capture.backend": CaptureBackend::BitBlt.as_str() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 BGRA 缓冲：每个像素给相同灰度值（B=G=R=v，BT.601 加权恰好等于 v）。
    fn bgra_uniform(pixels: usize, luma: u8) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            buffer.extend_from_slice(&[luma, luma, luma, 255]);
        }
        buffer
    }

    fn black_stats(pixels: usize) -> FrameLumaStats {
        FrameLumaStats {
            pixels,
            black_pixels: pixels,
            mean_luma: 0.0,
            variance: 0.0,
        }
    }

    fn content_stats() -> FrameLumaStats {
        // 30% 纯白 + 70% 中灰：均值≈107、方差大，肯定不是黑屏。
        let mut buffer = bgra_uniform(70_000, 40);
        buffer.extend_from_slice(&bgra_uniform(30_000, 255));
        luma_stats_from_bgra(&buffer)
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectPhys {
        RectPhys {
            x,
            y,
            width,
            height,
        }
    }

    /// 解出 PNG 的 IHDR 宽高（bytes 16..24），避免额外解码依赖。
    fn png_dimensions(png_bytes: &[u8]) -> (u32, u32) {
        assert!(png_bytes.len() > 24, "PNG 过短: {}", png_bytes.len());
        assert_eq!(&png_bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG 签名");
        assert_eq!(&png_bytes[12..16], b"IHDR", "首个 chunk 必须是 IHDR");
        let width =
            u32::from_be_bytes([png_bytes[16], png_bytes[17], png_bytes[18], png_bytes[19]]);
        let height =
            u32::from_be_bytes([png_bytes[20], png_bytes[21], png_bytes[22], png_bytes[23]]);
        (width, height)
    }

    // ───────── 既有 S-09 判定口径（保持） ─────────

    #[test]
    fn luma_stats_follow_bt601_weights() {
        let white = luma_stats_from_bgra(&bgra_uniform(4, 255));
        assert_eq!(white.pixels, 4);
        assert_eq!(white.black_pixels, 0);
        assert!((white.mean_luma - 255.0).abs() < 1e-6);
        assert!(white.variance.abs() < 1e-6);

        // BGRA：B=0, G=0, R=255 → 0.299*255 = 76.245
        let red = luma_stats_from_bgra(&[0, 0, 255, 255]);
        assert!((red.mean_luma - 76.245).abs() < 1e-3);

        // 不足 4 字节的尾巴被忽略
        let malformed = luma_stats_from_bgra(&[0, 0, 0, 255, 42]);
        assert_eq!(malformed.pixels, 1);
    }

    #[test]
    fn normal_frame_is_not_black_and_not_failed() {
        let stats = content_stats();
        assert!(!is_black_frame(&stats));
        assert!(!is_capture_failed(Some(&stats)));
    }

    #[test]
    fn pure_black_frame_is_detected() {
        let black = black_stats(1920 * 1080);
        assert!(is_black_frame(&black));
        assert!(is_capture_failed(Some(&black)));
        assert_eq!(classify_failure(Some(&black)), CaptureFailure::BlackFrame);
    }

    #[test]
    fn near_black_gradient_boundary() {
        // 全部亮度 0..=15（<16）→ 判黑（均值 7.5、标准差 ~4.6 < 6）
        let mut data = Vec::new();
        for value in 0..=15u8 {
            for _ in 0..100 {
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let stats = luma_stats_from_bgra(&data);
        assert!((stats.black_ratio() - 1.0).abs() < 1e-9);
        assert!(is_black_frame(&stats), "全 <16 亮度应判黑: {stats:?}");

        // 混入 16/17 → 黑占比 16/18 ≈ 0.889 < 0.99 → 不判黑（边界外）
        let mut data = Vec::new();
        for value in 0..=17u8 {
            for _ in 0..100 {
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let stats = luma_stats_from_bgra(&data);
        assert!(stats.black_ratio() < BLACK_RATIO_MIN);
        assert!(!is_black_frame(&stats), "黑占比不足不应判黑: {stats:?}");
    }

    #[test]
    fn dark_ui_with_small_bright_text_is_not_black() {
        // 99.5% 纯黑 + 0.5% 白字：黑占比 ≥0.99、均值 <16，但方差极大 → 不能判黑，
        // 否则深色主题的截图会被误当成捕获失败。
        let mut data = bgra_uniform(19_900, 0);
        data.extend_from_slice(&bgra_uniform(100, 255));
        let stats = luma_stats_from_bgra(&data);
        assert!(stats.black_ratio() >= BLACK_RATIO_MIN);
        assert!(stats.mean_luma < BLACK_LUMA_MAX);
        assert!(stats.std_dev() > BLACK_STD_DEV_MAX);
        assert!(
            !is_black_frame(&stats),
            "深色 UI + 少量亮文字不应判黑: {stats:?}"
        );
    }

    #[test]
    fn empty_frame_is_capture_failure() {
        let empty = luma_stats_from_bgra(&[]);
        assert!(empty.is_empty());
        assert!(!is_black_frame(&empty), "空帧不是黑帧");
        assert!(is_capture_failed(None), "None（WGC 未取到帧）按失败处理");
        assert!(is_capture_failed(Some(&empty)));
        assert_eq!(classify_failure(None), CaptureFailure::EmptyFrame);
        assert_eq!(classify_failure(Some(&empty)), CaptureFailure::EmptyFrame);
    }

    #[test]
    fn capture_failed_error_maps_to_sa1001_with_wgc_fallback() {
        let error = capture_failed_error(CaptureBackend::Wgc, Some(&black_stats(1920 * 1080)));
        assert_eq!(error.code, ErrorCode::CaptureFailed);
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable, "SA-1001 必须 retryable");
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        assert!(
            error.message.contains("受保护"),
            "message 应可操作: {}",
            error.message
        );
        assert!(
            error.message.contains("全屏独占"),
            "message 应可操作: {}",
            error.message
        );
        let detail = error.detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains("backend=wgc"),
            "detail 应记录后端: {detail}"
        );
        assert!(detail.contains("黑屏"), "detail 应说明失败类型: {detail}");
        let context = error.context.expect("应有 capture.backend 上下文");
        assert_eq!(context["capture.backend"], "wgc");
        assert_eq!(context["capture.blackFrame"], true);
        assert_eq!(context["capture.blackRatio"], 1.0);
    }

    #[test]
    fn capture_failed_error_handles_empty_frame_from_bitblt() {
        let error = capture_failed_error(CaptureBackend::BitBlt, None);
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable);
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        let detail = error.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("backend=bitblt"), "detail: {detail}");
        assert!(detail.contains("未能取到画面"), "detail: {detail}");
        let context = error.context.expect("应有 capture.backend 上下文");
        assert_eq!(context["capture.backend"], "bitblt");
        assert_eq!(context["capture.blackFrame"], false);
    }

    #[test]
    fn thresholds_match_spike_tool() {
        // 与 scripts/spikes/s09/Program.cs 的 BlackLumaMax/BlackRatioMin/BlackStdDevMax 一致
        assert!((BLACK_LUMA_MAX - 16.0).abs() < 1e-9);
        assert!((BLACK_RATIO_MIN - 0.99).abs() < 1e-9);
        assert!((BLACK_STD_DEV_MAX - 6.0).abs() < 1e-9);
    }

    #[test]
    fn backend_names_match_contract_fallback_tokens() {
        assert_eq!(CaptureBackend::BitBlt.as_str(), "bitblt");
        assert_eq!(CaptureBackend::Wgc.as_str(), "wgc");
        assert_eq!(
            ErrorCode::fallback(ErrorCode::CaptureFailed),
            &["wgc", "retry"]
        );
    }

    // ───────── T2：FrozenFrame / PNG 编码 ─────────

    #[test]
    fn frame_to_meta_keeps_null_data_url() {
        let frame = FrozenFrame::skeleton(1920, 1080);
        let meta = frame.to_meta();
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
        assert!(meta.data_url.is_none());
        assert!(frame.bgra.is_none());
        assert!(!frame.has_complete_bgra());
    }

    #[test]
    fn frozen_frame_from_bgra_sets_data_url_and_arc() {
        let mut bgra = Vec::new();
        for _ in 0..(4 * 3) {
            bgra.extend_from_slice(&[10, 20, 30, 255]);
        }
        let frame = FrozenFrame::from_bgra(4, 3, bgra.clone()).expect("编码成功");
        assert_eq!(frame.width, 4);
        assert_eq!(frame.height, 3);
        assert!(frame.has_complete_bgra());
        assert_eq!(frame.bgra.as_deref().map(Vec::len), Some(bgra.len()));

        let data_url = frame.data_url.clone().expect("真实帧必须有 data URL");
        assert!(data_url.starts_with(PNG_DATA_URL_PREFIX));
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode(data_url.trim_start_matches(PNG_DATA_URL_PREFIX))
            .expect("data URL 必须是合法 base64");
        assert_eq!(png_dimensions(&png_bytes), (4, 3));
        // meta 保留 data URL（overlay 快照用），但不含 BGRA。
        assert!(frame.to_meta().data_url.is_some());
    }

    #[test]
    fn frozen_frame_from_meta_roundtrip() {
        let meta = FrozenFrameMeta {
            width: 2560,
            height: 1440,
            data_url: Some(format!("{PNG_DATA_URL_PREFIX}AAAA")),
        };
        let frame = FrozenFrame::from_meta(&meta);
        assert_eq!(frame.width, 2560);
        assert_eq!(frame.height, 1440);
        assert!(frame.bgra.is_none());
        assert_eq!(frame.data_url, meta.data_url);
    }

    #[test]
    fn cropped_data_url_only_encodes_selection_region() {
        // 4×3 帧，每个像素用坐标着色；裁 (1,1,2,2) 后 PNG 宽度/高度必须是 2×2。
        let mut bgra = Vec::new();
        for y in 0..3u8 {
            for x in 0..4u8 {
                bgra.extend_from_slice(&[x, y, 9, 255]);
            }
        }
        let frame = FrozenFrame::from_bgra(4, 3, bgra).expect("编码成功");
        let origin = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 3.0,
        };
        let target = RectPhys {
            x: 1.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
        };
        let url = cropped_data_url(&frame, &origin, &target)
            .expect("裁切成功")
            .expect("真实帧必须产出 data URL");
        assert!(url.starts_with(PNG_DATA_URL_PREFIX));
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode(url.trim_start_matches(PNG_DATA_URL_PREFIX))
            .expect("data URL 必须是合法 base64");
        assert_eq!(png_dimensions(&png_bytes), (2, 2));
    }

    #[test]
    fn cropped_data_url_handles_negative_origin_and_out_of_range() {
        let mut bgra = Vec::new();
        for _ in 0..(4 * 3) {
            bgra.extend_from_slice(&[10, 20, 30, 255]);
        }
        let frame = FrozenFrame::from_bgra(4, 3, bgra).expect("编码成功");
        // 帧原点 (-100,-50)：绝对矩形 (-99,-49,2,2) → 帧局部 (1,1,2,2)。
        let origin = RectPhys {
            x: -100.0,
            y: -50.0,
            width: 4.0,
            height: 3.0,
        };
        let target = RectPhys {
            x: -99.0,
            y: -49.0,
            width: 2.0,
            height: 2.0,
        };
        let url = cropped_data_url(&frame, &origin, &target)
            .expect("裁切成功")
            .expect("相交区域必须产出 data URL");
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode(url.trim_start_matches(PNG_DATA_URL_PREFIX))
            .expect("合法 base64");
        assert_eq!(png_dimensions(&png_bytes), (2, 2));

        // 与帧完全不相交 → None（调用方不上传图片）。
        let outside = RectPhys {
            x: 50.0,
            y: 50.0,
            width: 10.0,
            height: 10.0,
        };
        assert!(cropped_data_url(&frame, &origin, &outside)
            .expect("裁切成功")
            .is_none());

        // 骨架帧无 BGRA → None（不伪造图片）。
        let skeleton = FrozenFrame::skeleton(4, 3);
        assert!(cropped_data_url(&skeleton, &origin, &target)
            .expect("裁切成功")
            .is_none());
    }

    #[test]
    fn encode_png_rejects_truncated_buffer() {
        let error = encode_png_bytes(10, 10, &[0u8; 100]).expect_err("长度不足必须失败");
        assert_eq!(error.code, "SA-1001");
        let error = encode_png_bytes(0, 10, &[]).expect_err("零宽必须失败");
        assert_eq!(error.code, "SA-1001");
    }

    #[test]
    fn frozen_frame_luma_stats_rejects_truncated_frame() {
        let frame = FrozenFrame {
            width: 10,
            height: 10,
            data_url: None,
            bgra: Some(Arc::new(vec![0u8; 100])),
        };
        assert!(!frame.has_complete_bgra());
        assert!(frame.luma_stats().is_none());
    }

    // ───────── T2：采样统计 / 区域切片 ─────────

    #[test]
    fn sampled_stats_match_full_stats_when_below_budget() {
        let data = bgra_uniform(1000, 200);
        assert_eq!(
            luma_stats_from_bgra_sampled(&data, 1_000_000),
            luma_stats_from_bgra(&data)
        );
        assert_eq!(
            luma_stats_from_bgra_sampled(&data, 0),
            luma_stats_from_bgra(&data)
        );
    }

    #[test]
    fn sampled_stats_detect_large_black_frame() {
        // 1920x1080 纯黑，采样预算 10000：仍应 100% 判黑。
        let data = bgra_uniform(1920 * 1080, 0);
        let stats = luma_stats_from_bgra_sampled(&data, 10_000);
        assert!(stats.pixels > 9_000, "采样数应接近预算: {}", stats.pixels);
        assert!(is_black_frame(&stats), "大黑帧采样后仍须判黑: {stats:?}");
    }

    #[test]
    fn sampled_stats_detect_mixed_large_frame_as_content() {
        // 75% 黑 + 25% 白：采样后黑占比仍约 0.75 < 0.99 → 不判黑（S-09 混合双屏口径）。
        let pixels = 400_000usize;
        let mut data = bgra_uniform(pixels * 3 / 4, 0);
        data.extend_from_slice(&bgra_uniform(pixels / 4, 255));
        let stats = luma_stats_from_bgra_sampled(&data, 20_000);
        assert!(stats.black_ratio() < BLACK_RATIO_MIN);
        assert!(!is_black_frame(&stats), "{stats:?}");
    }

    #[test]
    fn region_stats_crop_with_negative_origin() {
        // 虚拟桌面 (-1080,-470) 4x4：左半黑、右半白。
        let width = 4u32;
        let height = 1u32;
        let origin = rect(-1080.0, -470.0, 4.0, 1.0);
        let mut frame = Vec::new();
        for _ in 0..2 {
            frame.extend_from_slice(&[0, 0, 0, 255]);
        }
        for _ in 0..2 {
            frame.extend_from_slice(&[255, 255, 255, 255]);
        }

        // 左半区域（绝对坐标 x=-1080..-1078）→ 全部纯黑。
        let left = region_luma_stats(
            &frame,
            width,
            height,
            &origin,
            &rect(-1080.0, -470.0, 2.0, 1.0),
            0,
        )
        .expect("左半区域");
        assert_eq!(left.pixels, 2);
        assert!(is_black_frame(&left), "{left:?}");

        // 右半区域 → 全白，不判黑。
        let right = region_luma_stats(
            &frame,
            width,
            height,
            &origin,
            &rect(-1078.0, -470.0, 2.0, 1.0),
            0,
        )
        .expect("右半区域");
        assert!(!is_black_frame(&right), "{right:?}");
        assert!((right.mean_luma - 255.0).abs() < 1e-6);
    }

    #[test]
    fn region_stats_clamp_and_empty() {
        let origin = rect(0.0, 0.0, 10.0, 10.0);
        let frame = bgra_uniform(100, 128);
        // 超出帧外的部分被裁掉：请求 (5,5) 20x20 → 只剩 5x5。
        let local = region_local_rect(10, 10, &origin, &rect(5.0, 5.0, 20.0, 20.0));
        assert_eq!(local, Some((5, 5, 5, 5)));
        // 完全在帧外 → None。
        assert_eq!(
            region_local_rect(10, 10, &origin, &rect(-50.0, -50.0, 10.0, 10.0)),
            None
        );
        // 空区域 → None。
        assert_eq!(
            region_local_rect(10, 10, &origin, &rect(1.0, 1.0, 0.0, 0.0)),
            None
        );
        assert!(
            region_luma_stats(&frame, 10, 10, &origin, &rect(100.0, 100.0, 5.0, 5.0), 0).is_none()
        );
    }

    #[test]
    fn region_capture_failed_error_is_sa1001_with_region_context() {
        let error =
            region_capture_failed_error(&rect(-100.0, -50.0, 200.0, 100.0), &black_stats(20_000));
        assert_eq!(error.code, "SA-1001");
        assert!(error.retryable);
        assert_eq!(error.fallback, vec!["retry".to_string()]);
        assert!(error.message.contains("选区"), "{}", error.message);
        assert!(error.message.contains("DRM"), "{}", error.message);
        assert!(error.message.contains("黑屏"), "{}", error.message);
        let context = error.context.expect("区域上下文");
        assert_eq!(context["capture.blackFrame"], true);
        assert_eq!(context["capture.region"]["x"], -100.0);
        assert_eq!(context["capture.region"]["width"], 200.0);
    }

    #[test]
    fn bitblt_error_does_not_claim_wgc_attempt() {
        let error = bitblt_capture_failed_error(Some(&black_stats(100)));
        assert_eq!(error.code, "SA-1001");
        assert_eq!(error.fallback, vec!["wgc".to_string(), "retry".to_string()]);
        assert!(
            !error.message.contains("WGC"),
            "message 不应谎称 WGC: {}",
            error.message
        );
        assert!(error.message.contains("全屏独占"));
        assert_eq!(error.context.expect("上下文")["capture.backend"], "bitblt");
    }

    #[test]
    fn geometry_mismatch_tolerates_one_pixel() {
        let frame = FrozenFrame::skeleton(3640, 1920);
        assert!(geometry_mismatch(&frame, &rect(-1080.0, -470.0, 3640.0, 1920.0)).is_none());
        assert!(geometry_mismatch(&frame, &rect(-1080.0, -470.0, 3640.9, 1920.0)).is_none());
        let diff = geometry_mismatch(&frame, &rect(-1080.0, -470.0, 2560.0, 1440.0))
            .expect("尺寸差异必须被发现");
        assert!(diff.contains("3640x1920"), "{diff}");
        assert!(diff.contains("2560x1440"), "{diff}");
    }

    #[test]
    fn stats_json_matches_spike_field_names() {
        let value = black_stats(10).to_json();
        assert_eq!(value["blackRatio"], 1.0);
        assert_eq!(value["meanLuma"], 0.0);
        assert_eq!(value["stdDev"], 0.0);
        assert_eq!(value["pixels"], 10);
    }

    // ───────── 真实桌面（需交互桌面，CI 上 #[ignore] 跳过） ─────────

    #[test]
    #[ignore = "需要真实桌面：cargo test --test capture_real -- --ignored --nocapture"]
    fn real_desktop_geometry_is_available() {
        let rect = system_virtual_desktop_rect().expect("真实桌面必须有虚拟屏幕矩形");
        assert!(rect.width >= 100.0 && rect.height >= 100.0, "{rect:?}");
        let monitors = system_monitors();
        assert!(
            !monitors.is_empty(),
            "EnumDisplayMonitors 应至少返回一个显示器"
        );
        // Win32 虚拟屏幕矩形必须等于显示器外接矩形（两者同源，不一致会导致 overlay 偏移）。
        let union = coords::union_rect(monitors.iter().map(|monitor| monitor.rect)).expect("非空");
        assert_eq!(union, rect, "虚拟屏幕矩形 != 显示器外接矩形");
    }
}
