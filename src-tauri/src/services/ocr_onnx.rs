//! 真实本地 OCR 引擎（RapidOCR PP-OCRv4 / ONNX Runtime 1.31，契约 v5）。
//!
//! 目标：把 v0.1 骨架 OCR 升级为**真实 ONNX 推理**，满足 [`OcrEngine`] trait
//! （`detect` / `recognize` / `detect_and_recognize`），并新增 [`OcrImage`] 入口
//! 与 [`OcrPipeline::with_real`](crate::services::ocr_service::OcrPipeline::with_real)。
//!
//! # 模型与运行时（spec §4）
//!
//! | 资产 | 固定文件名 | 来源 |
//! | --- | --- | --- |
//! | det | `ch_PP-OCRv4_det_infer.onnx` | 随包 `resources/ocr/`（full）或下载 `<models_dir>/ocr/`（slim） |
//! | rec | `ch_PP-OCRv4_rec_infer.onnx` | 同上 |
//! | cls | `ch_ppocr_mobile_v2.0_cls_infer.onnx` | 同上 |
//! | dict | `ppocr_keys_v1.txt`（6623 行） | 同上 |
//! | 运行时 | `onnxruntime.dll`（ONNX Runtime 1.31.1） | 随包 `<resource_dir>\ocr\` |
//!
//! 解析顺序：**随包 → 数据目录（`<models_dir>/ocr/`）→ 开发钩子 `SNIP_ASK_OCR_DIR`**。
//! `set_ort_dylib_path()` 在启动早期把解析到的 DLL 写入 `ORT_DYLIB_PATH`（`ort` crate
//! `load-dynamic`），[`OnnxOcrEngine`] 懒加载时还会以 `ort::init_from(dll)` 显式兜底，
//! 因此不依赖环境变量的设置时序（但 spec 要求仍保留环境变量写入）。
//!
//! # 前处理 / 后处理（对齐 RapidOCR 1.4.4 `config.yaml`）
//!
//! - **det**：双约束降采样（长边 ≤ `ocr.detMaxEdge`、宽×高 ≤ 3.0 Mpx，**不放大**）→ 32 对齐
//!   → 极扁/极矮图 letterbox（`height ≤ 30` 或 `宽高比 > 8`）→ `(x/255-0.5)/0.5` NCHW；
//!   后处理 = DB：`thresh=0.3` 二值化 → 2×2 膨胀 → 连通域 → 轴对齐包围盒（等价
//!   `minAreaRect` 的轴对齐投影）→ `box_thresh` 分数过滤 → `unclip_ratio=1.6` 外扩 →
//!   还原原图坐标 → RapidOCR `sorted_boxes`（上→下、左→右，行内 y 容差 10px）。
//! - **rec**：按框裁切（高/宽 ≥ 1.5 先旋转 90°，与 RapidOCR 一致）→ 高 48、宽
//!   `ceil(48 × w/h)`（批内 `max_wh_ratio`、上限 `48×320/48`）→ 归一化 + 右侧 0 填充 →
//!   CTC 贪心解码（去重复、去 blank、置信度 = 选中列概率均值）。
//! - **cls**：高 48、宽 ≤ 192；`180°` 概率 > 0.9 时把裁切图旋转 180° 再送 rec。
//!
//! # 与骨架实现的差异（如实记录）
//!
//! - DB 后处理用**轴对齐包围盒**（契约 [`OcrBox::bbox`] 本身就是轴对齐 `RectPhys`），
//!   不使用旋转框；`unclip` 用「面积 / 周长」的等比外扩近似 pyclipper 圆角外扩。
//!   对横排文字行等价性高（S-15 语料对照见 `.verify/v1/ocr/`）。
//! - 不实现 RapidOCR 的 `return_word_box`（单词框坐标）与 `slow` 分数模式。
//! - GPU EP（CUDA/DML）不启用：RapidOCR 1.4.4 的 Windows 预编译 `onnxruntime.dll`
//!   在本机实测仅含 CPU EP；OCR 模型小（det 4.7 MB / rec 10.9 MB），CPU 即可达标
//!   （S-02/S-03）。Vulkan/CUDA 属于本地 **LLM** 推理链路，与 OCR 无关。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::contract::errors::{AppError, AppResult, ErrorCode};
use crate::contract::types::{OcrAssetStatus, OcrBox, OcrStatus, RectPhys};
use crate::platform::capture::FrozenFrame;
use crate::services::ocr_service::{det_input_size, OcrEngine};

// ───────────────────────────── 常量 ─────────────────────────────

/// 运行期资产目录名（随包 `<resource_dir>\ocr\`、下载 `<models_dir>\ocr\`）。
pub const OCR_DIR_NAME: &str = "ocr";
/// det 模型固定文件名。
pub const DET_FILE_NAME: &str = "ch_PP-OCRv4_det_infer.onnx";
/// rec 模型固定文件名。
pub const REC_FILE_NAME: &str = "ch_PP-OCRv4_rec_infer.onnx";
/// cls 模型固定文件名。
pub const CLS_FILE_NAME: &str = "ch_ppocr_mobile_v2.0_cls_infer.onnx";
/// rec 字典固定文件名。
pub const DICT_FILE_NAME: &str = "ppocr_keys_v1.txt";
/// ONNX Runtime 运行时 DLL 固定文件名。
pub const ORT_DLL_NAME: &str = "onnxruntime.dll";

/// 开发钩子环境变量：指向包含 5 个资产的目录（spec §4 解析顺序兜底）。
pub const ENV_OCR_DIR: &str = "SNIP_ASK_OCR_DIR";
/// `ort` crate `load-dynamic` 读取的运行时路径环境变量。
pub const ENV_ORT_DYLIB: &str = "ORT_DYLIB_PATH";

/// 资产 id（清单 / `OcrStatus.missing` 用）。
pub const ASSET_DET: &str = "ocr-det";
/// rec 资产 id。
pub const ASSET_REC: &str = "ocr-rec";
/// cls 资产 id。
pub const ASSET_CLS: &str = "ocr-cls";
/// 字典资产 id。
pub const ASSET_DICT: &str = "ocr-dict";

/// DB 二值化阈值（RapidOCR `Det.thresh`）。
pub const DET_THRESH: f32 = 0.3;
/// DB 框分数阈值（RapidOCR `Det.box_thresh`；产品 `ocr.textThreshold` 作用于 rec 置信度）。
pub const DET_BOX_THRESH: f32 = 0.5;
/// DB 最大候选框数。
pub const DET_MAX_CANDIDATES: usize = 1000;
/// unclip 外扩比例（RapidOCR 1.4.4 `Det.unclip_ratio`）。
pub const DET_UNCLIP_RATIO: f32 = 1.6;
/// 最小框边长（DB 后处理 `min_size`）。
pub const DET_MIN_SIZE: f32 = 3.0;
/// 是否做 2×2 膨胀（RapidOCR `Det.use_dilation`）。
pub const DET_USE_DILATION: bool = true;
/// det 输入对齐粒度（DB 网络下采样）。
pub const DET_ALIGN: u32 = 32;

/// rec 输入高度（RapidOCR `Rec.rec_img_shape` 的 H）。
pub const REC_IMG_HEIGHT: u32 = 48;
/// rec 基准宽度（`rec_img_shape` 的 W = 320）。
pub const REC_BASE_WIDTH: u32 = 320;
/// rec 单批数量（`rec_batch_num`）。
pub const REC_BATCH_NUM: usize = 6;

/// cls 输入高度（`cls_image_shape` 的 H）。
pub const CLS_IMG_HEIGHT: u32 = 48;
/// cls 输入最大宽度（`cls_image_shape` 的 W = 192）。
pub const CLS_IMG_MAX_WIDTH: u32 = 192;
/// cls 单批数量（`cls_batch_num`）。
pub const CLS_BATCH_NUM: usize = 6;
/// cls 判 180° 的阈值（`cls_thresh`）。
pub const CLS_THRESH: f32 = 0.9;

/// letterbox 触发的最小高度（`Global.min_height`）。
pub const MIN_TEXT_HEIGHT: u32 = 30;
/// letterbox 触发的宽高比（`Global.width_height_ratio`）。
pub const MAX_WIDTH_HEIGHT_RATIO: f32 = 8.0;

/// 单次最多 rec 的框数（防御：极端截图 det 出过多框导致 rec 过慢）。
pub const MAX_REC_BOXES: usize = 256;

/// 契约口径的 OCR 资产元数据（manifest 未包含时兜底；数值来自 T5 实测）。
const OCR_ASSET_SPECS: [(&str, &str, u64, &str); 4] = [
    (
        ASSET_DET,
        "OCR 检测模型（PP-OCRv4 det）",
        4_745_517,
        "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
    ),
    (
        ASSET_REC,
        "OCR 识别模型（PP-OCRv4 rec）",
        10_857_958,
        "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b",
    ),
    (
        ASSET_CLS,
        "OCR 方向分类模型（PP-OCR mobile cls）",
        585_532,
        "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c",
    ),
    (
        ASSET_DICT,
        "OCR 识别字典（ppocr_keys_v1）",
        26_250,
        "a1c84d9bdb9ab29043c58896224d32941783eb821629618416dcb08f12886492",
    ),
];

// ───────────────────────────── 错误辅助 ─────────────────────────────

fn ocr_failed(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::OcrFailed, Some(message.into()))
}

fn engine_unavailable(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::OcrEngineUnavailable, Some(message.into()))
}

/// 缺模型：`message` 为面向用户的可操作提示（去设置页下载 / 换 full 版）。
fn model_missing(actionable: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::ModelMissing, None)
        .with_message(actionable)
        .with_fallback(vec!["download_model", "ocr_status"])
}

// ───────────────────────────── 资产解析 ─────────────────────────────

/// 资产来源（对应 spec §4 解析顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetSource {
    /// 随包 `<resource_dir>\ocr\`（full 版 / 两个版本都有的运行时）。
    Bundled,
    /// 数据目录 `<models_dir>\ocr\`（slim 版下载）。
    Installed,
    /// 开发钩子 `SNIP_ASK_OCR_DIR`。
    DevHook,
}

impl AssetSource {
    /// 契约/日志用字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            AssetSource::Bundled => "bundled",
            AssetSource::Installed => "installed",
            AssetSource::DevHook => "dev",
        }
    }

    /// 是否来自随包资源。
    pub fn is_bundled(self) -> bool {
        matches!(self, AssetSource::Bundled)
    }
}

/// 解析后的单个资产。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    /// 资产 id（`ocr-det` 等）。
    pub id: &'static str,
    /// 实际文件路径。
    pub path: PathBuf,
    /// 来源。
    pub source: AssetSource,
    /// 文件大小（字节；元数据失败时为 0）。
    pub size_bytes: u64,
}

impl ResolvedAsset {
    fn new(id: &'static str, path: PathBuf, source: AssetSource) -> Self {
        let size_bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        Self {
            id,
            path,
            source,
            size_bytes,
        }
    }
}

/// 运行期 OCR 资产集合（可能部分缺失）。
#[derive(Debug, Clone, Default)]
pub struct OcrAssets {
    /// ONNX Runtime DLL。
    pub runtime: Option<ResolvedAsset>,
    /// det 模型。
    pub det: Option<ResolvedAsset>,
    /// rec 模型。
    pub rec: Option<ResolvedAsset>,
    /// cls 模型。
    pub cls: Option<ResolvedAsset>,
    /// rec 字典。
    pub dict: Option<ResolvedAsset>,
}

impl OcrAssets {
    /// 解析资产：随包 `resource_dir`（由 [`set_resource_dir`] 注入）→ `<models_dir>/ocr/`
    /// → 开发钩子 `SNIP_ASK_OCR_DIR`。
    ///
    /// 每个资产独立按顺序取「第一个存在的文件」，因此 slim 版可以是
    /// 「运行时随包 + 模型在数据目录」的混合来源。
    pub fn resolve(models_dir: &Path) -> Self {
        let resource_dir = resource_dir();
        let dev_dir = dev_ocr_dir();
        Self::resolve_from(resource_dir, models_dir, dev_dir.as_deref())
    }

    /// 显式目录版本（单测 / 集成测试用）。
    ///
    /// - `resource_dir`：安装目录（内部会拼 `\ocr`），`None` = 无随包资源；
    /// - `models_dir`：模型根目录（内部会拼 `\ocr`），slim 下载位置；
    /// - `dev_dir`：开发钩子目录（直接包含资产文件），`None` = 不使用。
    pub fn resolve_from(
        resource_dir: Option<&Path>,
        models_dir: &Path,
        dev_dir: Option<&Path>,
    ) -> Self {
        let bundled_dir = resource_dir.map(|dir| dir.join(OCR_DIR_NAME));
        let installed_dir = models_dir.join(OCR_DIR_NAME);

        let mut dirs: Vec<(PathBuf, AssetSource)> = Vec::new();
        if let Some(dir) = bundled_dir {
            dirs.push((dir, AssetSource::Bundled));
        }
        dirs.push((installed_dir, AssetSource::Installed));
        if let Some(dir) = dev_dir {
            dirs.push((dir.to_path_buf(), AssetSource::DevHook));
        }

        let find = |id: &'static str, file_name: &str| -> Option<ResolvedAsset> {
            dirs.iter().find_map(|(dir, source)| {
                let path = dir.join(file_name);
                path.is_file()
                    .then(|| ResolvedAsset::new(id, path, *source))
            })
        };

        Self {
            runtime: find(ASSET_DICT, ORT_DLL_NAME).map(|asset| ResolvedAsset {
                id: "ocr-runtime",
                ..asset
            }),
            det: find(ASSET_DET, DET_FILE_NAME),
            rec: find(ASSET_REC, REC_FILE_NAME),
            cls: find(ASSET_CLS, CLS_FILE_NAME),
            dict: find(ASSET_DICT, DICT_FILE_NAME),
        }
    }

    /// 运行时 + 全部模型/字典都可用。
    pub fn is_ready(&self) -> bool {
        self.runtime.is_some()
            && self.det.is_some()
            && self.rec.is_some()
            && self.cls.is_some()
            && self.dict.is_some()
    }

    /// 运行时 + 全部资产都来自随包资源（full 版）。
    pub fn bundled_all(&self) -> bool {
        self.runtime.as_ref().is_some_and(|a| a.source.is_bundled())
            && self.det.as_ref().is_some_and(|a| a.source.is_bundled())
            && self.rec.as_ref().is_some_and(|a| a.source.is_bundled())
            && self.cls.as_ref().is_some_and(|a| a.source.is_bundled())
            && self.dict.as_ref().is_some_and(|a| a.source.is_bundled())
    }

    /// 缺失资产 id（含运行时 `ocr-runtime`）；用于 `OcrStatus.missing` 与错误提示。
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.runtime.is_none() {
            missing.push("ocr-runtime");
        }
        for (id, asset) in [
            (ASSET_DET, self.det.is_some()),
            (ASSET_REC, self.rec.is_some()),
            (ASSET_CLS, self.cls.is_some()),
            (ASSET_DICT, self.dict.is_some()),
        ] {
            if !asset {
                missing.push(id);
            }
        }
        missing
    }

    /// 运行时 DLL 路径。
    pub fn runtime_path(&self) -> Option<&Path> {
        self.runtime.as_ref().map(|asset| asset.path.as_path())
    }

    fn asset(&self, id: &str) -> Option<&ResolvedAsset> {
        match id {
            ASSET_DET => self.det.as_ref(),
            ASSET_REC => self.rec.as_ref(),
            ASSET_CLS => self.cls.as_ref(),
            ASSET_DICT => self.dict.as_ref(),
            _ => None,
        }
    }
}

/// 随包资源根目录（安装目录；内部解析 `<resource_dir>\ocr\`）。
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 启动早期注入随包资源根目录（**不是** `\ocr` 子目录）。
///
/// 幂等：重复调用只保留首次值（多窗口 / 测试可安全重复调用）。
pub fn set_resource_dir(resource_dir: &Path) {
    let _ = RESOURCE_DIR.set(resource_dir.to_path_buf());
}

/// 已注入的随包资源根目录。
pub fn resource_dir() -> Option<&'static Path> {
    RESOURCE_DIR.get().map(PathBuf::as_path)
}

/// 开发钩子目录（`SNIP_ASK_OCR_DIR`）。
fn dev_ocr_dir() -> Option<PathBuf> {
    std::env::var_os(ENV_OCR_DIR)
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
}

/// 把解析到的 `onnxruntime.dll` 写入 `ORT_DYLIB_PATH`（已设置则不覆盖），返回路径。
///
/// 必须在任何 `ort` session 创建之前调用（spec §4）；[`OnnxOcrEngine`] 内部还会
/// 用 `ort::init_from()` 显式加载同一 DLL，避免调用时序问题。
pub fn set_ort_dylib_path(assets: &OcrAssets) -> Option<PathBuf> {
    let path = assets.runtime_path()?.to_path_buf();
    if std::env::var_os(ENV_ORT_DYLIB).is_none() {
        // edition 2021 下 `set_var` 是安全函数；本函数在 app setup 早期调用（单线程）。
        std::env::set_var(ENV_ORT_DYLIB, &path);
        tracing::debug!(path = %path.display(), "已写入 ORT_DYLIB_PATH");
    }
    Some(path)
}

// ───────────────────────────── ort 全局环境 ─────────────────────────────

/// 进程级 ONNX Runtime 环境（`load-dynamic` 只加载一次）。
static ORT_ENV: OnceLock<ort::environment::Environment> = OnceLock::new();
/// 环境初始化串行化（避免并发 `init_from` 重复初始化）。
static ORT_INIT_LOCK: Mutex<()> = Mutex::new(());

/// 初始化 ort 全局环境（幂等）。
fn ensure_ort_env(dll: &Path) -> AppResult<&'static ort::environment::Environment> {
    if let Some(env) = ORT_ENV.get() {
        return Ok(env);
    }
    let _guard = ORT_INIT_LOCK.lock();
    if let Some(env) = ORT_ENV.get() {
        return Ok(env);
    }
    let env = ort::init_from(dll)
        .map_err(|err| {
            engine_unavailable(format!(
                "无法动态加载 ONNX Runtime（{}）：{err}",
                dll.display()
            ))
        })?
        .with_name("snipask-ocr")
        .build()
        .map_err(|err| engine_unavailable(format!("创建 ONNX Runtime 环境失败：{err}")))?;
    let _ = ORT_ENV.set(env);
    ORT_ENV
        .get()
        .ok_or_else(|| engine_unavailable("ONNX Runtime 环境初始化失败"))
}

// ───────────────────────────── 位图 / 像素工具 ─────────────────────────────

/// OCR 输入位图：**top-down、stride = width × 4 的 BGRA**（与 Win32 BitBlt 原始缓冲一致）。
#[derive(Debug, Clone)]
pub struct OcrImage {
    /// 宽（物理像素）。
    pub width: u32,
    /// 高（物理像素）。
    pub height: u32,
    /// BGRA 像素（长度必须 = width × height × 4）。
    pub bgra: Arc<Vec<u8>>,
}

impl OcrImage {
    /// 构造并校验像素长度。
    pub fn new(width: u32, height: u32, bgra: Arc<Vec<u8>>) -> AppResult<Self> {
        let expected = width as usize * height as usize * 4;
        if expected == 0 {
            return Err(ocr_failed("OCR 输入图像尺寸为 0"));
        }
        if bgra.len() != expected {
            return Err(ocr_failed(format!(
                "OCR 输入像素长度不符：期望 {expected} 字节（{width}×{height}×4），实际 {} 字节",
                bgra.len()
            )));
        }
        Ok(Self {
            width,
            height,
            bgra,
        })
    }

    /// 从冻结帧构造（真实捕获的 `bgra` 必填）。
    pub fn from_frame(frame: &FrozenFrame) -> AppResult<Self> {
        let bgra = frame.bgra.clone().ok_or_else(|| {
            engine_unavailable("当前冻结帧没有像素数据（骨架模式或捕获降级帧），无法执行真实 OCR")
        })?;
        Self::new(frame.width, frame.height, bgra)
    }
}

/// 从整帧裁出 `[x0, x1) × [y0, y1)`（**帧局部**坐标，调用方保证已 clamp 且非空）。
fn crop_bgra(image: &OcrImage, x0: u32, y0: u32, x1: u32, y1: u32) -> OcrImage {
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    let mut out = Vec::with_capacity(w * h * 4);
    let stride = image.width as usize * 4;
    for y in y0 as usize..y1 as usize {
        let start = y * stride + x0 as usize * 4;
        out.extend_from_slice(&image.bgra[start..start + w * 4]);
    }
    OcrImage {
        width: w as u32,
        height: h as u32,
        bgra: Arc::new(out),
    }
}

/// 复制为黑底 letterbox（上下各 `pad` 行）。
fn letterbox_bgra(image: &OcrImage, pad: u32) -> Arc<Vec<u8>> {
    if pad == 0 {
        return image.bgra.clone();
    }
    let w = image.width as usize;
    let h = image.height as usize;
    let padded_h = h + pad as usize * 2;
    let mut out = vec![0u8; w * padded_h * 4];
    let row_bytes = w * 4;
    let dst_offset = pad as usize * row_bytes;
    out[dst_offset..dst_offset + row_bytes * h].copy_from_slice(image.bgra.as_slice());
    Arc::new(out)
}

/// 计算 letterbox 的上下留白（对齐 RapidOCR `maybe_add_letterbox`）。
///
/// 触发条件：高度 ≤ [`MIN_TEXT_HEIGHT`] 或宽高比 > [`MAX_WIDTH_HEIGHT_RATIO`]。
/// 目标高度 `new_h = max(w / ratio, min_height) × 2`，上下各补 `(new_h - h) / 2`。
pub fn letterbox_padding(width: u32, height: u32) -> u32 {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let too_short = height <= MIN_TEXT_HEIGHT as f32;
    let too_wide = width / height > MAX_WIDTH_HEIGHT_RATIO;
    if !too_short && !too_wide {
        return 0;
    }
    let new_h = (width / MAX_WIDTH_HEIGHT_RATIO)
        .max(MIN_TEXT_HEIGHT as f32)
        .max(height)
        * 2.0;
    ((new_h - height) / 2.0) as u32
}

/// det 输入尺寸：双约束（沿用 [`det_input_size`]）后向下对齐到 32 的倍数。
///
/// 返回 `(resized_w, resized_h)`。向下对齐保证：**不放大**、宽×高不超过 3.0 Mpx 预算
/// （`det_input_size` 已保证预算，向下对齐只会更小；极小图最多对齐到 32）。
pub fn det_resize_size(width: u32, height: u32, det_max_edge: u32) -> (u32, u32) {
    let (base_w, base_h, _scale) = det_input_size(width, height, det_max_edge);
    let align = |value: u32| -> u32 { (value / DET_ALIGN * DET_ALIGN).max(DET_ALIGN) };
    (align(base_w), align(base_h))
}

/// 双线性缩放 BGRA → CHW f32（BGR 顺序，数值 `(x/255 - 0.5) / 0.5`）。
///
/// 与 `cv2.resize(INTER_LINEAR)` 一致使用半像素中心映射：
/// `src = (dst + 0.5) × scale - 0.5`，边界 clamp。
fn resize_bgra_to_chw(
    src: &[u8],
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
    out: &mut [f32],
) -> AppResult<()> {
    let (sw_u, sh_u, dw_u, dh_u) = (sw as usize, sh as usize, dw as usize, dh as usize);
    if src.len() < sw_u * sh_u * 4 {
        return Err(ocr_failed("缩放源像素长度不足"));
    }
    if out.len() < 3 * dw_u * dh_u {
        return Err(ocr_failed("缩放目标缓冲长度不足"));
    }
    let scale_x = sw as f32 / dw as f32;
    let scale_y = sh as f32 / dh as f32;
    let plane = dw_u * dh_u;
    for dy in 0..dh_u {
        let fy = (dy as f32 + 0.5) * scale_y - 0.5;
        let y0 = fy.floor();
        let wy = fy - y0;
        let y0i = (y0 as i64).clamp(0, sh_u as i64 - 1) as usize;
        let y1i = (y0 as i64 + 1).clamp(0, sh_u as i64 - 1) as usize;
        for dx in 0..dw_u {
            let fx = (dx as f32 + 0.5) * scale_x - 0.5;
            let x0 = fx.floor();
            let wx = fx - x0;
            let x0i = (x0 as i64).clamp(0, sw_u as i64 - 1) as usize;
            let x1i = (x0 as i64 + 1).clamp(0, sw_u as i64 - 1) as usize;

            let i00 = (y0i * sw_u + x0i) * 4;
            let i01 = (y0i * sw_u + x1i) * 4;
            let i10 = (y1i * sw_u + x0i) * 4;
            let i11 = (y1i * sw_u + x1i) * 4;
            let w00 = (1.0 - wx) * (1.0 - wy);
            let w01 = wx * (1.0 - wy);
            let w10 = (1.0 - wx) * wy;
            let w11 = wx * wy;
            for ch in 0..3 {
                let v = src[i00 + ch] as f32 * w00
                    + src[i01 + ch] as f32 * w01
                    + src[i10 + ch] as f32 * w10
                    + src[i11 + ch] as f32 * w11;
                out[ch * plane + dy * dw_u + dx] = v / 127.5 - 1.0;
            }
        }
    }
    Ok(())
}

/// 旋转 90°（Mat 形状交换；`counter_clockwise=false` 为顺时针）。
fn rotate_bgra_90(image: &OcrImage, counter_clockwise: bool) -> OcrImage {
    let (sw, sh) = (image.width as usize, image.height as usize);
    let (dw, dh) = (sh, sw);
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..sh {
        for x in 0..sw {
            let src = (y * sw + x) * 4;
            let (dx, dy) = if counter_clockwise {
                (y, sw - 1 - x)
            } else {
                (sh - 1 - y, x)
            };
            let dst = (dy * dw + dx) * 4;
            out[dst..dst + 4].copy_from_slice(&image.bgra[src..src + 4]);
        }
    }
    OcrImage {
        width: dw as u32,
        height: dh as u32,
        bgra: Arc::new(out),
    }
}

/// 旋转 180°。
fn rotate_bgra_180(image: &OcrImage) -> OcrImage {
    let (w, h) = (image.width as usize, image.height as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * 4;
            let dst = ((h - 1 - y) * w + (w - 1 - x)) * 4;
            out[dst..dst + 4].copy_from_slice(&image.bgra[src..src + 4]);
        }
    }
    OcrImage {
        width: image.width,
        height: image.height,
        bgra: Arc::new(out),
    }
}

/// 裁切 → 若高/宽 ≥ 1.5 旋转 90°（对齐 RapidOCR `get_rotate_crop_image`）。
fn crop_for_rec(image: &OcrImage, bbox: &RectPhys) -> Option<OcrImage> {
    let x0 = bbox.x.max(0.0).floor() as i64;
    let y0 = bbox.y.max(0.0).floor() as i64;
    let x1 = (bbox.x + bbox.width).min(image.width as f64).ceil() as i64;
    let y1 = (bbox.y + bbox.height).min(image.height as f64).ceil() as i64;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let crop = crop_bgra(image, x0 as u32, y0 as u32, x1 as u32, y1 as u32);
    if crop.height as f32 / crop.width.max(1) as f32 >= 1.5 {
        // RapidOCR：np.rot90 为逆时针。
        Some(rotate_bgra_90(&crop, true))
    } else {
        Some(crop)
    }
}

// ─────────────────────── 多行合并框拆分（v1.0.5） ───────────────────────

/// 一个框的高度超过「框高中位数」的该倍数时，视为可能合并了多行。
pub const MERGED_BOX_HEIGHT_RATIO: f32 = 1.65;
/// 行拆分：一行被判为「有墨迹」所需的最小像素占比（相对框宽）。
const ROW_INK_MIN_RATIO: f32 = 0.012;
/// 行拆分：灰度与行背景中位数的最小差异（兼顾深色/浅色主题）。
const ROW_INK_LUMA_DELTA: f32 = 34.0;

/// 框高数组的中位数（小于 4px 的框不参与，避免噪声）。
fn median_box_height(boxes: &[OcrBox]) -> f32 {
    let mut heights: Vec<f32> = boxes
        .iter()
        .map(|item| item.bbox.height as f32)
        .filter(|height| *height >= 4.0)
        .collect();
    if heights.is_empty() {
        return 0.0;
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights[heights.len() / 2]
}

/// 对「异常高」的框按行墨迹投影拆成多行；拆不出 2 行以上时返回 `None`。
///
/// 墨迹判定：每行取灰度中位数作为背景，统计与背景差异显著的像素数。深色/浅色主题通用，
/// 不依赖固定颜色。返回坐标与输入框同一空间（绝对/局部由调用方保证）。
fn split_merged_box(image: &OcrImage, bbox: &RectPhys) -> Option<Vec<RectPhys>> {
    let x0 = bbox.x.max(0.0).floor() as u32;
    let y0 = bbox.y.max(0.0).floor() as u32;
    let x1 = ((bbox.x + bbox.width).min(image.width as f64)).ceil() as u32;
    let y1 = ((bbox.y + bbox.height).min(image.height as f64)).ceil() as u32;
    if x1 <= x0 + 2 || y1 <= y0 + 4 {
        return None;
    }
    let width = (x1 - x0) as usize;
    let height = (y1 - y0) as usize;
    let stride = width * 4;
    let pixels = image.bgra.as_slice();
    if pixels.len() < (y0 as usize + height) * image.width as usize * 4 {
        return None;
    }

    let mut active = Vec::with_capacity(height);
    for row in 0..height {
        let start = (y0 as usize + row) * image.width as usize * 4 + x0 as usize * 4;
        let row_pixels = &pixels[start..start + stride];
        // 灰度直方图求中位数（行背景）。
        let mut histogram = [0u32; 256];
        for pixel in row_pixels.chunks_exact(4) {
            let luma = luma(pixel[2], pixel[1], pixel[0]);
            histogram[luma as usize] += 1;
        }
        let half = (width / 2) as u32;
        let mut seen = 0u32;
        let mut background = 0u8;
        for (value, count) in histogram.iter().enumerate() {
            seen += count;
            if seen >= half.max(1) {
                background = value as u8;
                break;
            }
        }
        let mut ink = 0usize;
        for pixel in row_pixels.chunks_exact(4) {
            let luma = luma(pixel[2], pixel[1], pixel[0]);
            if (luma as f32 - background as f32).abs() >= ROW_INK_LUMA_DELTA {
                ink += 1;
            }
        }
        let min_ink = ((width as f32 * ROW_INK_MIN_RATIO) as usize).max(3);
        active.push(ink >= min_ink);
    }

    // 闭运算：把 2 行以内的空洞补上（防止字形间隙把一行拆开）。
    let mut closed = active.clone();
    for index in 0..height {
        if active[index] {
            continue;
        }
        let near_before = (1..=2).any(|d| index >= d && active[index - d]);
        let near_after = (1..=2).any(|d| index + d < height && active[index + d]);
        if near_before && near_after {
            closed[index] = true;
        }
    }

    // 连续墨迹带（行区间）。
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    while cursor < height {
        if !closed[cursor] {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < height && closed[cursor] {
            cursor += 1;
        }
        let end = cursor; // exclusive
                          // 上下各扩 4px（保留字形上下缘与 det unclip 余量），并丢弃过矮噪声带。
        let top = start.saturating_sub(4);
        let bottom = (end + 4).min(height);
        if bottom - top >= 6 {
            bands.push((top, bottom));
        }
    }
    if bands.len() < 2 || bands.len() > 6 {
        return None;
    }
    // 相邻带之间若空隙过小（< 3px），合并（避免把同一行拆成两段）。
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for band in bands {
        if let Some(last) = merged.last_mut() {
            if band.0 <= last.1 + 4 {
                last.1 = band.1;
                continue;
            }
        }
        merged.push(band);
    }
    if merged.len() < 2 {
        return None;
    }
    Some(
        merged
            .into_iter()
            .map(|(top, bottom)| RectPhys {
                x: bbox.x,
                y: bbox.y + top as f64,
                width: bbox.width,
                height: (bottom - top) as f64,
            })
            .collect(),
    )
}

/// 把检测框中「明显高于中位行高」的框替换为按行拆分的子框；其余框原样保留。
fn expand_merged_boxes(image: &OcrImage, boxes: &[OcrBox], median_height: f32) -> Vec<OcrBox> {
    if median_height < 4.0 {
        return boxes.to_vec();
    }
    let threshold = (median_height * MERGED_BOX_HEIGHT_RATIO).max(median_height + 8.0);
    let mut expanded: Vec<OcrBox> = Vec::with_capacity(boxes.len());
    for item in boxes {
        let suspicious = item.bbox.height as f32 > threshold;
        if suspicious {
            // 只要宽高比不是极端的竖排单字，就尝试按行拆分。
            if item.bbox.width as f32 > median_height * 1.5 {
                if let Some(parts) = split_merged_box(image, &item.bbox) {
                    let mut split_count = 0usize;
                    for part in parts {
                        expanded.push(OcrBox {
                            text: String::new(),
                            confidence: item.confidence,
                            bbox: part,
                        });
                        split_count += 1;
                    }
                    if split_count >= 2 {
                        continue;
                    }
                    // 拆分结果不足 2 行时回退为原框。
                    while split_count > 0 {
                        expanded.pop();
                        split_count -= 1;
                    }
                }
            }
        }
        expanded.push(item.clone());
    }
    expanded
}

/// BGR → 灰度（BT.601 整数近似）。
fn luma(blue: u8, green: u8, red: u8) -> u8 {
    ((red as u32 * 299 + green as u32 * 587 + blue as u32 * 114) / 1000) as u8
}

// ───────────────────────────── DB det 后处理 ─────────────────────────────

/// 概率图 → 轴对齐文字框（坐标为 `dest_w × dest_h` 空间）。
///
/// 直径近似：连通域（2×2 膨胀后）→ 包围盒 → 平均概率分数 →
/// `unclip`（`area × ratio / perimeter` 等比外扩）。
pub fn db_postprocess(
    prob: &[f32],
    map_w: u32,
    map_h: u32,
    dest_w: u32,
    dest_h: u32,
) -> Vec<RectPhys> {
    let (mw, mh) = (map_w as usize, map_h as usize);
    if mw == 0 || mh == 0 || prob.len() < mw * mh {
        return Vec::new();
    }
    // 1) 二值化 + 2×2 膨胀（mask=true 表示文字区域）。
    let mut mask = vec![false; mw * mh];
    for i in 0..mw * mh {
        mask[i] = prob[i] > DET_THRESH;
    }
    if DET_USE_DILATION {
        let mut dilated = vec![false; mw * mh];
        for y in 0..mh {
            for x in 0..mw {
                let mut hit = false;
                // cv2.dilate 2×2（anchor 默认中心）≈ 取左上方向邻域最大值。
                for (dy, dx) in [(0i64, 0i64), (-1, 0), (0, -1), (-1, -1)] {
                    let ny = y as i64 + dy;
                    let nx = x as i64 + dx;
                    if ny >= 0 && nx >= 0 && ny < mh as i64 && nx < mw as i64 {
                        hit |= mask[ny as usize * mw + nx as usize];
                    }
                }
                dilated[y * mw + x] = hit;
            }
        }
        mask = dilated;
    }

    // 2) 8-连通域 → 包围盒 + 分数均值。
    let mut visited = vec![false; mw * mh];
    let mut stack: Vec<usize> = Vec::new();
    let mut rects: Vec<RectPhys> = Vec::new();
    for start in 0..mw * mh {
        if !mask[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (mw, 0usize, mh, 0usize);
        let mut count = 0usize;
        let mut prob_sum = 0.0f64;
        while let Some(idx) = stack.pop() {
            let x = idx % mw;
            let y = idx / mw;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            count += 1;
            prob_sum += prob[idx] as f64;
            // 8 邻域。
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let ny = y as i64 + dy;
                    let nx = x as i64 + dx;
                    if ny < 0 || nx < 0 || ny >= mh as i64 || nx >= mw as i64 {
                        continue;
                    }
                    let nidx = ny as usize * mw + nx as usize;
                    if mask[nidx] && !visited[nidx] {
                        visited[nidx] = true;
                        stack.push(nidx);
                    }
                }
            }
        }
        if count < 4 {
            continue;
        }
        let score = (prob_sum / count as f64) as f32;
        let bw = (max_x - min_x + 1) as f32;
        let bh = (max_y - min_y + 1) as f32;
        if bw.min(bh) < DET_MIN_SIZE || score < DET_BOX_THRESH {
            continue;
        }
        // 3) unclip 外扩（面积 / 周长 × ratio），再对齐 DB 的「二次最小边」过滤。
        let area = bw * bh;
        let perimeter = 2.0 * (bw + bh);
        let distance = if perimeter > 0.0 {
            area * DET_UNCLIP_RATIO / perimeter
        } else {
            0.0
        };
        let x0 = min_x as f32 - distance;
        let y0 = min_y as f32 - distance;
        let x1 = max_x as f32 + 1.0 + distance;
        let y1 = max_y as f32 + 1.0 + distance;
        if (x1 - x0).min(y1 - y0) < DET_MIN_SIZE + 2.0 {
            continue;
        }
        // 4) 映射到 dest 空间并裁剪。
        let sx = dest_w as f32 / mw as f32;
        let sy = dest_h as f32 / mh as f32;
        let dx0 = (x0 * sx).clamp(0.0, dest_w as f32);
        let dy0 = (y0 * sy).clamp(0.0, dest_h as f32);
        let dx1 = (x1 * sx).clamp(0.0, dest_w as f32);
        let dy1 = (y1 * sy).clamp(0.0, dest_h as f32);
        let width = dx1 - dx0;
        let height = dy1 - dy0;
        if width <= 3.0 || height <= 3.0 {
            continue;
        }
        rects.push(RectPhys {
            x: dx0 as f64,
            y: dy0 as f64,
            width: width as f64,
            height: height as f64,
        });
        if rects.len() >= DET_MAX_CANDIDATES {
            break;
        }
    }
    sort_det_boxes(rects)
}

/// RapidOCR `sorted_boxes`：先按 `(y, x)` 排序，再做「同一行 y 差 < 10px 时按 x 重排」。
pub fn sort_det_boxes(mut boxes: Vec<RectPhys>) -> Vec<RectPhys> {
    boxes.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    if boxes.len() < 2 {
        return boxes;
    }
    for i in 0..boxes.len() - 1 {
        let mut j = i as isize;
        while j >= 0 {
            let cur = boxes[j as usize];
            let next = boxes[j as usize + 1];
            if (next.y - cur.y).abs() < 10.0 && next.x < cur.x {
                boxes.swap(j as usize, j as usize + 1);
                j -= 1;
            } else {
                break;
            }
        }
    }
    boxes
}

// ───────────────────────────── CTC 解码 ─────────────────────────────

/// 字典字符表：`["blank", 6623 个字, " "]`，索引 0 = CTC blank。
#[derive(Debug, Clone)]
pub struct OcrDict {
    /// 6623 个字典字符（不含 blank / 空格）。
    pub chars: Vec<String>,
}

impl OcrDict {
    /// 从 `ppocr_keys_v1.txt` 文本解析（逐行，保留行内空格、去掉行尾 CR/LF）。
    pub fn parse(content: &str) -> AppResult<Self> {
        let chars: Vec<String> = content.lines().map(str::to_string).collect();
        if chars.len() < 100 {
            return Err(ocr_failed(format!(
                "OCR 字典行数异常：{} 行（期望 6623 行）",
                chars.len()
            )));
        }
        Ok(Self { chars })
    }

    /// 从文件加载。
    pub fn from_path(path: &Path) -> AppResult<Self> {
        let content = std::fs::read_to_string(path).map_err(|err| {
            model_missing(format!("读取 OCR 字典 {} 失败：{err}", path.display()))
        })?;
        Self::parse(&content)
    }

    /// 索引 → 字符：0 = blank（`None`），1..=N = 字典，N+1 = 空格。
    pub fn char_at(&self, index: usize) -> Option<&str> {
        if index == 0 {
            None
        } else if index <= self.chars.len() {
            Some(self.chars[index - 1].as_str())
        } else if index == self.chars.len() + 1 {
            Some(" ")
        } else {
            None
        }
    }
}

/// CTC 贪心解码（对齐 RapidOCR `CTCLabelDecode`）。
///
/// `probs` 布局 `[timesteps, classes]`（单样本）；返回 `(text, confidence)`。
/// 置信度 = 被选中列（非 blank、非重复）的最大概率均值；无选中时 0。
pub fn ctc_decode(
    probs: &[f32],
    timesteps: usize,
    classes: usize,
    dict: &OcrDict,
) -> (String, f32) {
    let mut text = String::new();
    let mut conf_sum = 0.0f64;
    let mut conf_count = 0usize;
    let mut prev = usize::MAX;
    for t in 0..timesteps {
        let row = &probs[t * classes..(t + 1) * classes];
        let mut best = 0usize;
        let mut best_p = f32::MIN;
        for (idx, &p) in row.iter().enumerate() {
            if p > best_p {
                best_p = p;
                best = idx;
            }
        }
        if best != 0 && best != prev {
            if let Some(ch) = dict.char_at(best) {
                text.push_str(ch);
                conf_sum += best_p.max(0.0) as f64;
                conf_count += 1;
            }
        }
        prev = best;
    }
    let confidence = if conf_count == 0 {
        0.0
    } else {
        (conf_sum / conf_count as f64) as f32
    };
    (text, confidence)
}

// ───────────────────────────── 会话集 ─────────────────────────────

/// 已加载的 ONNX 会话 + 字典（一次加载，进程内复用）。
struct LoadedOcr {
    det: Mutex<ort::session::Session>,
    rec: Mutex<ort::session::Session>,
    cls: Mutex<ort::session::Session>,
    dict: OcrDict,
}

/// 单次 OCR 的几何参数（det 前处理 / 坐标还原）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DetGeometry {
    resized_w: u32,
    resized_h: u32,
    pad: u32,
}

impl DetGeometry {
    fn new(width: u32, height: u32, det_max_edge: u32) -> Self {
        let pad = letterbox_padding(width, height);
        let padded_h = height + pad * 2;
        let (resized_w, resized_h) = det_resize_size(width, padded_h, det_max_edge);
        Self {
            resized_w,
            resized_h,
            pad,
        }
    }
}

fn session_intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .max(1)
}

impl LoadedOcr {
    fn load(assets: &OcrAssets) -> AppResult<Self> {
        let dll = assets
            .runtime_path()
            .ok_or_else(|| engine_unavailable(missing_message(&assets.missing())))?;
        let (det_path, rec_path, cls_path, dict_path) =
            match (&assets.det, &assets.rec, &assets.cls, &assets.dict) {
                (Some(det), Some(rec), Some(cls), Some(dict)) => {
                    (&det.path, &rec.path, &cls.path, &dict.path)
                }
                _ => return Err(model_missing(missing_message(&assets.missing()))),
            };

        let started = std::time::Instant::now();
        let env = ensure_ort_env(dll)?;
        let dict = OcrDict::from_path(dict_path)?;

        let intra = session_intra_threads();
        let load_session = |path: &Path| -> AppResult<ort::session::Session> {
            let mut builder = ort::session::Session::builder(env)
                .map_err(|err| ocr_failed(format!("创建 ONNX 会话失败：{err}")))?
                .with_intra_threads(intra)
                .map_err(|err| ocr_failed(format!("设置 ONNX 线程数失败：{err}")))?
                .with_inter_threads(1)
                .map_err(|err| ocr_failed(format!("设置 ONNX 线程数失败：{err}")))?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|err| ocr_failed(format!("设置 ONNX 图优化级别失败：{err}")))?;
            builder
                .commit_from_file(path)
                .map_err(|err| ocr_failed(format!("加载 ONNX 模型 {} 失败：{err}", path.display())))
        };

        let loaded = Self {
            det: Mutex::new(load_session(det_path)?),
            rec: Mutex::new(load_session(rec_path)?),
            cls: Mutex::new(load_session(cls_path)?),
            dict,
        };
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            intra,
            dict_lines = loaded.dict.chars.len(),
            "OCR ONNX 会话加载完成"
        );
        Ok(loaded)
    }

    /// 整帧 det（返回帧局部物理像素框，`text` 为空、`confidence` 为 det 分数）。
    fn detect_frame(&self, image: &OcrImage, det_max_edge: u32) -> AppResult<Vec<OcrBox>> {
        let geom = DetGeometry::new(image.width, image.height, det_max_edge);
        let padded = letterbox_bgra(image, geom.pad);
        let padded_h = image.height + geom.pad * 2;
        let mut tensor = vec![0.0f32; 3 * geom.resized_w as usize * geom.resized_h as usize];
        resize_bgra_to_chw(
            padded.as_slice(),
            image.width,
            padded_h,
            geom.resized_w,
            geom.resized_h,
            &mut tensor,
        )?;

        // det 推理 + 后处理（坐标在 resized 空间）。
        let rects = {
            let mut session = self.det.lock();
            let input = ort::value::Tensor::from_array((
                [1usize, 3, geom.resized_h as usize, geom.resized_w as usize],
                tensor,
            ))
            .map_err(|err| ocr_failed(format!("构造 det 输入张量失败：{err}")))?;
            let outputs = session
                .run(ort::inputs![input])
                .map_err(|err| ocr_failed(format!("det 推理失败：{err}")))?;
            let shape = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|err| ocr_failed(format!("读取 det 输出失败：{err}")))?;
            let (dims, prob) = shape;
            let dims: Vec<i64> = dims.iter().copied().collect();
            if dims.len() != 4 {
                return Err(ocr_failed(format!(
                    "det 输出维度异常：{dims:?}（期望 [N,1,H,W]）"
                )));
            }
            let map_h = dims[2].max(1) as u32;
            let map_w = dims[3].max(1) as u32;
            db_postprocess(prob, map_w, map_h, geom.resized_w, geom.resized_h)
        };

        // resized 空间 → padded 空间 → 原图（减 letterbox 留白）。
        let padded_h = image.height + geom.pad * 2;
        let scale_x = image.width as f64 / geom.resized_w as f64;
        let scale_y = padded_h as f64 / geom.resized_h as f64;
        let mut boxes = Vec::with_capacity(rects.len());
        for rect in rects {
            let x0 = rect.x * scale_x;
            let y0 = rect.y * scale_y - geom.pad as f64;
            let x1 = (rect.x + rect.width) * scale_x;
            let y1 = (rect.y + rect.height) * scale_y - geom.pad as f64;
            let x0 = x0.clamp(0.0, image.width as f64);
            let y0 = y0.clamp(0.0, image.height as f64);
            let x1 = x1.clamp(0.0, image.width as f64);
            let y1 = y1.clamp(0.0, image.height as f64);
            if x1 - x0 <= 1.0 || y1 - y0 <= 1.0 {
                continue;
            }
            boxes.push(OcrBox {
                text: String::new(),
                confidence: 1.0,
                bbox: RectPhys {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                },
            });
        }
        Ok(boxes)
    }

    /// 对给定框做 cls（>0.9 判 180° 旋转）+ rec，返回与输入同序的结果。
    ///
    /// **v1.0.5**：DB 在低分辨率整屏 det 下可能把相邻多行合并成一个高框；直接整框送 rec 会得到
    /// 空文本/低置信度（实测三行合并框 rec 置信度 0.00），随后被 `textThreshold` 过滤 → 用户看到
    /// 「部分文字没识别」。这里先按行墨迹投影把异常高的框拆成多行，再逐行 rec。
    fn recognize_boxes(&self, image: &OcrImage, boxes: &[OcrBox]) -> AppResult<Vec<OcrBox>> {
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        let median_height = median_box_height(boxes);
        let expanded = expand_merged_boxes(image, boxes, median_height);
        let max = expanded.len().min(MAX_REC_BOXES);
        // 1) 裁切（必要时旋转 90°）。
        let mut crops: Vec<Option<OcrImage>> = Vec::with_capacity(max);
        let mut results: Vec<OcrBox> = Vec::with_capacity(max);
        for item in &expanded[..max] {
            let crop = crop_for_rec(image, &item.bbox);
            crops.push(crop);
            results.push(item.clone());
        }

        // 2) cls：分批推理，>0.9 的 180° 旋转裁切图。
        self.classify_crops(&mut crops)?;

        // 3) rec：分批推理，CTC 解码。
        let recognized = self.recognize_crops(&crops)?;
        for (slot, (text, confidence)) in results.iter_mut().zip(recognized) {
            slot.text = text;
            slot.confidence = confidence as f64;
        }
        Ok(results)
    }

    fn classify_crops(&self, crops: &mut [Option<OcrImage>]) -> AppResult<()> {
        let indices: Vec<usize> = crops
            .iter()
            .enumerate()
            .filter_map(|(idx, crop)| crop.as_ref().map(|_| idx))
            .collect();
        if indices.is_empty() {
            return Ok(());
        }
        for chunk in indices.chunks(CLS_BATCH_NUM) {
            let batch_w = CLS_IMG_MAX_WIDTH as usize;
            let mut tensor = vec![0.0f32; chunk.len() * 3 * CLS_IMG_HEIGHT as usize * batch_w];
            for (slot, &idx) in chunk.iter().enumerate() {
                let Some(crop) = crops[idx].as_ref() else {
                    continue;
                };
                let ratio = crop.width as f32 / crop.height.max(1) as f32;
                let resized_w =
                    ((CLS_IMG_HEIGHT as f32 * ratio).ceil() as u32).clamp(1, CLS_IMG_MAX_WIDTH);
                let dst = &mut tensor[slot * 3 * CLS_IMG_HEIGHT as usize * batch_w
                    ..(slot + 1) * 3 * CLS_IMG_HEIGHT as usize * batch_w];
                resize_bgra_into_padded(
                    crop.bgra.as_slice(),
                    crop.width,
                    crop.height,
                    resized_w,
                    CLS_IMG_HEIGHT,
                    batch_w as u32,
                    CLS_IMG_HEIGHT,
                    dst,
                )?;
            }
            let mut session = self.cls.lock();
            let input = ort::value::Tensor::from_array((
                [chunk.len(), 3, CLS_IMG_HEIGHT as usize, batch_w],
                tensor,
            ))
            .map_err(|err| ocr_failed(format!("构造 cls 输入张量失败：{err}")))?;
            let outputs = session
                .run(ort::inputs![input])
                .map_err(|err| ocr_failed(format!("cls 推理失败：{err}")))?;
            let (dims, probs) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|err| ocr_failed(format!("读取 cls 输出失败：{err}")))?;
            let dims: Vec<i64> = dims.iter().copied().collect();
            if dims.len() != 2 || dims[1] != 2 {
                return Err(ocr_failed(format!("cls 输出维度异常：{dims:?}")));
            }
            for (slot, &idx) in chunk.iter().enumerate() {
                let p0 = probs[slot * 2];
                let p1 = probs[slot * 2 + 1];
                if p1 > CLS_THRESH && p1 >= p0 {
                    let rotated = crops[idx].as_ref().map(rotate_bgra_180);
                    if let Some(rotated) = rotated {
                        crops[idx] = Some(rotated);
                    }
                }
            }
        }
        Ok(())
    }

    fn recognize_crops(&self, crops: &[Option<OcrImage>]) -> AppResult<Vec<(String, f32)>> {
        let mut output: Vec<(String, f32)> = vec![(String::new(), 0.0); crops.len()];
        let indices: Vec<usize> = crops
            .iter()
            .enumerate()
            .filter_map(|(idx, crop)| crop.as_ref().map(|_| idx))
            .collect();
        if indices.is_empty() {
            return Ok(output);
        }
        for chunk in indices.chunks(REC_BATCH_NUM) {
            // 批内最大宽高比（下限 320/48，与 RapidOCR 一致）。
            let mut max_wh_ratio = REC_BASE_WIDTH as f32 / REC_IMG_HEIGHT as f32;
            for &idx in chunk {
                if let Some(crop) = crops[idx].as_ref() {
                    max_wh_ratio = max_wh_ratio.max(crop.width as f32 / crop.height.max(1) as f32);
                }
            }
            let batch_w = (REC_IMG_HEIGHT as f32 * max_wh_ratio).ceil() as usize;
            let mut tensor = vec![0.0f32; chunk.len() * 3 * REC_IMG_HEIGHT as usize * batch_w];
            for (slot, &idx) in chunk.iter().enumerate() {
                let Some(crop) = crops[idx].as_ref() else {
                    continue;
                };
                let ratio = crop.width as f32 / crop.height.max(1) as f32;
                let resized_w =
                    ((REC_IMG_HEIGHT as f32 * ratio).ceil() as u32).clamp(1, batch_w as u32);
                let dst = &mut tensor[slot * 3 * REC_IMG_HEIGHT as usize * batch_w
                    ..(slot + 1) * 3 * REC_IMG_HEIGHT as usize * batch_w];
                resize_bgra_into_padded(
                    crop.bgra.as_slice(),
                    crop.width,
                    crop.height,
                    resized_w,
                    REC_IMG_HEIGHT,
                    batch_w as u32,
                    REC_IMG_HEIGHT,
                    dst,
                )?;
            }
            let mut session = self.rec.lock();
            let input = ort::value::Tensor::from_array((
                [chunk.len(), 3, REC_IMG_HEIGHT as usize, batch_w],
                tensor,
            ))
            .map_err(|err| ocr_failed(format!("构造 rec 输入张量失败：{err}")))?;
            let outputs = session
                .run(ort::inputs![input])
                .map_err(|err| ocr_failed(format!("rec 推理失败：{err}")))?;
            let (dims, probs) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|err| ocr_failed(format!("读取 rec 输出失败：{err}")))?;
            let dims: Vec<i64> = dims.iter().copied().collect();
            if dims.len() != 3 {
                return Err(ocr_failed(format!(
                    "rec 输出维度异常：{dims:?}（期望 [N,T,C]）"
                )));
            }
            let timesteps = dims[1].max(1) as usize;
            let classes = dims[2].max(1) as usize;
            for (slot, &idx) in chunk.iter().enumerate() {
                let start = slot * timesteps * classes;
                let slice = &probs[start..start + timesteps * classes];
                output[idx] = ctc_decode(slice, timesteps, classes, &self.dict);
            }
        }
        Ok(output)
    }
}

/// 把 BGRA 图像缩放到 `(resized_w × img_h)` 并归一化写入 CHW 槽（右侧零填充到 `slot_w`）。
#[allow(clippy::too_many_arguments)]
fn resize_bgra_into_padded(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    resized_w: u32,
    img_h: u32,
    slot_w: u32,
    slot_h: u32,
    dst: &mut [f32],
) -> AppResult<()> {
    let resized_w = resized_w.clamp(1, slot_w);
    let plane = slot_w as usize * slot_h as usize;
    if dst.len() < 3 * plane {
        return Err(ocr_failed("rec/cls 输入缓冲长度不足"));
    }
    let mut tmp = vec![0.0f32; 3 * resized_w as usize * img_h as usize];
    resize_bgra_to_chw(src, src_w, src_h, resized_w, img_h, &mut tmp)?;
    for ch in 0..3 {
        for y in 0..img_h as usize {
            let src_row = ch * resized_w as usize * img_h as usize + y * resized_w as usize;
            let dst_row = ch * plane + y * slot_w as usize;
            dst[dst_row..dst_row + resized_w as usize]
                .copy_from_slice(&tmp[src_row..src_row + resized_w as usize]);
        }
    }
    Ok(())
}

// ───────────────────────────── 公开引擎 ─────────────────────────────

/// 真实 OCR 引擎（懒加载；实现 [`OcrEngine`]）。
///
/// - 构造 [`OnnxOcrEngine::new`] **不加载模型**（启动快、slim 版首次使用前模型可能还没下载）；
/// - 首次推理时加载 det/rec/cls 三个 session（约 300–600 ms）并在进程内复用；
/// - 模型下载完成后再调用即可自动生效，无需重启。
#[derive(Clone)]
pub struct OnnxOcrEngine {
    inner: Arc<EngineInner>,
}

struct EngineInner {
    assets: OcrAssets,
    loaded: Mutex<Option<Arc<LoadedOcr>>>,
    load_lock: Mutex<()>,
}

impl OnnxOcrEngine {
    /// 创建懒加载引擎（不读取模型文件）。
    pub fn new(assets: OcrAssets) -> Self {
        Self {
            inner: Arc::new(EngineInner {
                assets,
                loaded: Mutex::new(None),
                load_lock: Mutex::new(()),
            }),
        }
    }

    /// 立即加载模型（可选；测试 / 预热门闩用）。
    pub fn load(assets: OcrAssets) -> AppResult<Self> {
        let engine = Self::new(assets);
        engine.ensure_loaded()?;
        Ok(engine)
    }

    /// 资产快照。
    pub fn assets(&self) -> &OcrAssets {
        &self.inner.assets
    }

    /// 是否已加载模型。
    pub fn is_loaded(&self) -> bool {
        self.inner.loaded.lock().is_some()
    }

    /// 预热（后台线程调用；等价于首次推理前的加载）。
    pub fn warmup(&self) -> AppResult<()> {
        self.ensure_loaded().map(|_| ())
    }

    /// 资产是否齐全（不加载模型）。
    pub fn is_ready(&self) -> bool {
        self.inner.assets.is_ready()
    }

    fn ensure_loaded(&self) -> AppResult<Arc<LoadedOcr>> {
        if let Some(loaded) = self.inner.loaded.lock().clone() {
            return Ok(loaded);
        }
        let _guard = self.inner.load_lock.lock();
        if let Some(loaded) = self.inner.loaded.lock().clone() {
            return Ok(loaded);
        }
        match LoadedOcr::load(&self.inner.assets) {
            Ok(loaded) => {
                let loaded = Arc::new(loaded);
                *self.inner.loaded.lock() = Some(loaded.clone());
                Ok(loaded)
            }
            Err(err) => Err(err),
        }
    }
}

/// 把同步推理放到 blocking 线程池（避免阻塞 tokio worker）。
async fn run_blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| ocr_failed(format!("OCR 推理任务失败：{err}")))?
}

#[async_trait]
impl OcrEngine for OnnxOcrEngine {
    async fn detect(&self, frame: &FrozenFrame, det_max_edge: u32) -> AppResult<Vec<OcrBox>> {
        let image = OcrImage::from_frame(frame)?;
        let engine = self.clone();
        run_blocking(move || {
            let loaded = engine.ensure_loaded()?;
            loaded.detect_frame(&image, det_max_edge)
        })
        .await
    }

    async fn recognize(&self, frame: &FrozenFrame, boxes: Vec<OcrBox>) -> AppResult<Vec<OcrBox>> {
        let image = OcrImage::from_frame(frame)?;
        let engine = self.clone();
        run_blocking(move || {
            let loaded = engine.ensure_loaded()?;
            loaded.recognize_boxes(&image, &boxes)
        })
        .await
    }

    async fn detect_and_recognize(
        &self,
        frame: &FrozenFrame,
        frame_origin: &RectPhys,
        rect_phys: &RectPhys,
        det_max_edge: u32,
    ) -> AppResult<Vec<OcrBox>> {
        let image = OcrImage::from_frame(frame)?;
        let origin = frame_origin.normalize();
        let rect = rect_phys.normalize();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Ok(Vec::new());
        }
        // 选区 → 帧局部坐标（四舍五入后裁剪到帧范围）。
        let x0 = (rect.x - origin.x).floor().max(0.0) as i64;
        let y0 = (rect.y - origin.y).floor().max(0.0) as i64;
        let x1 = ((rect.x + rect.width - origin.x).ceil() as i64).min(image.width as i64);
        let y1 = ((rect.y + rect.height - origin.y).ceil() as i64).min(image.height as i64);
        if x1 <= x0 || y1 <= y0 {
            return Ok(Vec::new());
        }
        let (x0, y0, x1, y1) = (x0 as u32, y0 as u32, x1 as u32, y1 as u32);
        let crop = crop_bgra(&image, x0, y0, x1, y1);
        let offset_x = origin.x + x0 as f64;
        let offset_y = origin.y + y0 as f64;

        let engine = self.clone();
        run_blocking(move || {
            let loaded = engine.ensure_loaded()?;
            let local_boxes = loaded.detect_frame(&crop, det_max_edge)?;
            let recognized = loaded.recognize_boxes(&crop, &local_boxes)?;
            Ok(recognized
                .into_iter()
                .map(|mut item| {
                    item.bbox.x += offset_x;
                    item.bbox.y += offset_y;
                    item
                })
                .filter(|item| !item.text.trim().is_empty())
                .collect())
        })
        .await
    }
}

// ───────────────────────────── OCR 状态 ─────────────────────────────

/// 缺失资产提示文案（面向用户可操作）。
fn missing_message(missing: &[&str]) -> String {
    if missing.contains(&"ocr-runtime") {
        return format!(
            "缺少 ONNX Runtime 运行时（{ORT_DLL_NAME}）。slim/full 安装包内均随附，请重新安装或检查 `<安装目录>\\{OCR_DIR_NAME}\\{ORT_DLL_NAME}`。"
        );
    }
    let names: Vec<&str> = missing
        .iter()
        .filter_map(|id| {
            OCR_ASSET_SPECS
                .iter()
                .find(|spec| spec.0 == *id)
                .map(|spec| spec.1)
        })
        .collect();
    let detail = if names.is_empty() {
        missing.join("、")
    } else {
        names.join("、")
    };
    format!(
        "缺少 OCR 模型：{detail}。可在设置页「本地模型」下载 OCR 资产（约 16 MB），或安装 full 版（模型随包）。"
    )
}

/// 构建 `ocr_status` 返回（契约 v5 §3.3）。
pub fn build_status(assets: &OcrAssets) -> OcrStatus {
    let manifest = crate::domain::models::parse_manifest(crate::resources::MODELS_MANIFEST).ok();
    let mut statuses = Vec::with_capacity(OCR_ASSET_SPECS.len());
    for (id, default_name, default_size, default_sha) in OCR_ASSET_SPECS {
        let entry = manifest
            .as_ref()
            .and_then(|manifest| crate::domain::models::find_entry(manifest, id));
        let resolved = assets.asset(id);
        let name = entry
            .map(|entry| entry.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| default_name.to_string());
        let sha256 = entry
            .map(|entry| entry.sha256.clone())
            .filter(|sha| !sha.is_empty())
            .unwrap_or_else(|| default_sha.to_string());
        let size_bytes = entry
            .map(|entry| entry.size_bytes)
            .filter(|size| *size > 0)
            .unwrap_or(default_size);
        statuses.push(OcrAssetStatus {
            id: id.to_string(),
            name,
            installed: resolved.is_some(),
            bundled: resolved.is_some_and(|asset| asset.source.is_bundled()),
            size_bytes,
            sha256,
        });
    }

    let runtime_ready = assets.runtime.is_some();
    let bundled = runtime_ready
        && assets
            .runtime
            .as_ref()
            .is_some_and(|asset| asset.source.is_bundled())
        && statuses.iter().all(|status| status.bundled);
    let missing = assets.missing();
    let ready = runtime_ready && missing.iter().all(|id| *id == "ocr-runtime");
    let message = if ready {
        None
    } else {
        Some(missing_message(&missing))
    };

    OcrStatus {
        ready,
        bundled,
        runtime_ready,
        runtime_path: assets.runtime_path().map(|path| path.display().to_string()),
        assets: statuses,
        missing: missing.into_iter().map(str::to_string).collect(),
        message,
    }
}

/// `ocr_status` 命令实现（Lead 接线用）：每次调用重新解析资产（下载模型后立刻反映）。
///
/// - `resource_dir`：`app.path().resource_dir().ok()`（安装目录，内部拼 `\ocr`）；
/// - `models_dir`：`state.paths.models_dir`（内部拼 `\ocr`）。
pub fn ocr_status(resource_dir: Option<&Path>, models_dir: &Path) -> OcrStatus {
    let assets = OcrAssets::resolve_from(resource_dir, models_dir, dev_ocr_dir().as_deref());
    build_status(&assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("snip-ask-ocr-{tag}-{}", uuid::Uuid::new_v4()))
    }

    fn write_dummy(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    /// 在 `root/ocr/` 下写入 5 个假资产，返回该目录。
    fn dummy_ocr_dir(root: &Path) -> PathBuf {
        let dir = root.join(OCR_DIR_NAME);
        for name in [
            ORT_DLL_NAME,
            DET_FILE_NAME,
            REC_FILE_NAME,
            CLS_FILE_NAME,
            DICT_FILE_NAME,
        ] {
            write_dummy(&dir.join(name), 16);
        }
        dir
    }

    // ───────── 资产解析优先级 ─────────

    #[test]
    fn resolve_priority_bundled_then_installed_then_dev() {
        let root = temp_root("resolve");
        let resource = root.join("resource");
        let models = root.join("models");
        // 开发钩子指向「直接包含资产文件的目录」（resolve_from 不再拼 `ocr` 子目录）。
        let dev = dummy_ocr_dir(&root.join("dev"));
        dummy_ocr_dir(&resource);
        dummy_ocr_dir(&models);

        let assets = OcrAssets::resolve_from(Some(&resource), &models, Some(&dev));
        assert!(assets.is_ready());
        assert!(assets.bundled_all());
        assert_eq!(assets.det.unwrap().source, AssetSource::Bundled);
        assert_eq!(assets.runtime.unwrap().source, AssetSource::Bundled);

        // 随包缺失 → 数据目录。
        std::fs::remove_file(resource.join(OCR_DIR_NAME).join(DET_FILE_NAME)).unwrap();
        let assets = OcrAssets::resolve_from(Some(&resource), &models, Some(&dev));
        assert_eq!(assets.det.as_ref().unwrap().source, AssetSource::Installed);
        assert!(!assets.bundled_all());

        // 随包 + 数据目录都缺 → 开发钩子（spec §4：bundled → installed → dev hook）。
        std::fs::remove_file(resource.join(OCR_DIR_NAME).join(REC_FILE_NAME)).unwrap();
        std::fs::remove_file(models.join(OCR_DIR_NAME).join(REC_FILE_NAME)).unwrap();
        let assets = OcrAssets::resolve_from(Some(&resource), &models, Some(&dev));
        assert_eq!(assets.rec.unwrap().source, AssetSource::DevHook);

        // 无任何来源 → missing 全量。
        let empty = OcrAssets::resolve_from(None, &root.join("nope"), None);
        assert!(!empty.is_ready());
        assert_eq!(empty.missing().len(), 5);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_mixed_sources_for_slim_layout() {
        // slim：运行时随包，模型在数据目录（下载）。
        let root = temp_root("slim");
        let resource = root.join("resource");
        let models = root.join("models");
        write_dummy(&resource.join(OCR_DIR_NAME).join(ORT_DLL_NAME), 16);
        dummy_ocr_dir(&models);
        let assets = OcrAssets::resolve_from(Some(&resource), &models, None);
        assert!(assets.is_ready());
        assert_eq!(
            assets.runtime.as_ref().unwrap().source,
            AssetSource::Bundled
        );
        assert_eq!(assets.rec.as_ref().unwrap().source, AssetSource::Installed);
        assert!(!assets.bundled_all());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────── det 几何 ─────────

    #[test]
    fn letterbox_only_for_short_or_wide_images() {
        assert_eq!(letterbox_padding(1920, 1080), 0);
        assert_eq!(letterbox_padding(3640, 1920), 0);
        // 极扁（高 20，宽 1920）：new_h = max(1920/8, 30) × 2 = 480，上下各 230。
        assert_eq!(letterbox_padding(1920, 20), 230);
        // 宽高比 > 8（1000x40）：new_h = max(125, 30) × 2 = 250，pad = 105。
        assert_eq!(letterbox_padding(1000, 40), 105);
        // 宽高比正好 8 不触发。
        assert_eq!(letterbox_padding(800, 100), 0);
    }

    #[test]
    fn det_resize_never_upscales_normal_screens_and_aligns_32() {
        for (fw, fh) in [(2560u32, 1440u32), (3840, 2160), (1920, 1080), (7680, 2160)] {
            let (rw, rh) = det_resize_size(fw, fh, 3200);
            assert!(rw <= fw && rh <= fh, "不放大：{fw}x{fh} -> {rw}x{rh}");
            assert_eq!(rw % DET_ALIGN, 0);
            assert_eq!(rh % DET_ALIGN, 0);
            assert!(
                (rw * rh) as u64 <= crate::services::ocr_service::DET_PIXEL_BUDGET,
                "预算：{fw}x{fh} -> {rw}x{rh}"
            );
            assert!(rw.max(rh) <= 3200);
        }
        // 单 4K → 2309x1299 向下对齐后 2304x1280。
        let (rw, rh) = det_resize_size(3840, 2160, 3200);
        assert_eq!((rw, rh), (2304, 1280));
        // 1080p 不触发降采样，只做 32 向下对齐。
        assert_eq!(det_resize_size(1920, 1080, 3200), (1920, 1056));
    }

    #[test]
    fn det_resize_handles_letterboxed_wide_crop() {
        // 极扁 1000x40：letterbox 后 1000x250，长边/预算都不触发降采样。
        let pad = letterbox_padding(1000, 40);
        let (rw, rh) = det_resize_size(1000, 40 + pad * 2, 3200);
        assert_eq!((rw, rh), (992, 224));
    }

    // ───────── CTC 解码 ─────────

    fn dict_of(n: usize) -> OcrDict {
        // 索引 1='a', 2='b', 3='c', ...；用 ascii 小写循环。
        let chars: Vec<String> = (0..n)
            .map(|i| char::from(b'a' + (i % 26) as u8).to_string())
            .collect();
        OcrDict { chars }
    }

    fn one_hot(idx: usize, classes: usize) -> Vec<f32> {
        let mut row = vec![0.0f32; classes];
        row[idx] = 0.9;
        row
    }

    #[test]
    fn ctc_decode_collapses_repeats_and_drops_blank() {
        let dict = dict_of(26);
        let classes = dict.chars.len() + 2; // blank + 空格
                                            // 序列：blank, a, a, blank, b, c
        let seq = [0usize, 1, 1, 0, 2, 3];
        let mut probs = Vec::new();
        for idx in seq {
            probs.extend(one_hot(idx, classes));
        }
        let (text, conf) = ctc_decode(&probs, seq.len(), classes, &dict);
        assert_eq!(text, "abc");
        assert!((conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn ctc_decode_space_index_is_last_class() {
        let dict = dict_of(26);
        let classes = dict.chars.len() + 2;
        let space = classes - 1;
        let seq = [1usize, space, 2];
        let mut probs = Vec::new();
        for idx in seq {
            probs.extend(one_hot(idx, classes));
        }
        let (text, _conf) = ctc_decode(&probs, seq.len(), classes, &dict);
        assert_eq!(text, "a b");
    }

    #[test]
    fn ctc_decode_all_blank_yields_empty_text() {
        let dict = dict_of(26);
        let classes = dict.chars.len() + 2;
        let mut probs = Vec::new();
        for _ in 0..4 {
            probs.extend(one_hot(0, classes));
        }
        let (text, conf) = ctc_decode(&probs, 4, classes, &dict);
        assert!(text.is_empty());
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn dict_rejects_suspiciously_short_files() {
        assert!(OcrDict::parse("a\nb\nc\n").is_err());
        let content = (0..6623)
            .map(|i| format!("字{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        // 无末尾换行 → 6623 行（与真实字典文件一致：末尾换行不影响 lines()）。
        let dict = OcrDict::parse(&format!("{content}\n")).unwrap();
        assert_eq!(dict.chars.len(), 6623);
        assert_eq!(dict.char_at(0), None);
        assert_eq!(dict.char_at(1), Some("字0"));
        assert_eq!(dict.char_at(6623), Some("字6622"));
        assert_eq!(dict.char_at(6624), Some(" "));
    }

    // ───────── DB 后处理 ─────────

    /// 在 `map` 上画一个 `prob=0.9` 的实心矩形。
    fn paint_rect(map: &mut [f32], w: usize, x0: usize, y0: usize, rw: usize, rh: usize) {
        for y in y0..y0 + rh {
            for x in x0..x0 + rw {
                map[y * w + x] = 0.9;
            }
        }
    }

    #[test]
    fn db_postprocess_finds_and_expands_text_regions() {
        let (mw, mh) = (160usize, 96usize);
        let mut prob = vec![0.0f32; mw * mh];
        paint_rect(&mut prob, mw, 10, 12, 60, 14);
        paint_rect(&mut prob, mw, 10, 40, 60, 14);
        let boxes = db_postprocess(&prob, mw as u32, mh as u32, 320, 192);
        assert_eq!(boxes.len(), 2);
        // 排序：上→下。
        assert!(boxes[0].y < boxes[1].y);
        // unclip 外扩：宽度大于原始 60/160*320=120，高度大于 14/96*192=28。
        assert!(boxes[0].width > 120.0);
        assert!(boxes[0].height > 28.0);
        // 同列左侧对齐。
        assert!((boxes[0].x - boxes[1].x).abs() < 1e-6);
    }

    #[test]
    fn db_postprocess_respects_box_threshold() {
        let (mw, mh) = (64usize, 64usize);
        let mut prob = vec![0.0f32; mw * mh];
        // 平均分 0.4 < DET_BOX_THRESH(0.5) → 过滤。
        paint_rect(&mut prob, mw, 8, 8, 20, 8);
        for v in prob.iter_mut() {
            *v = 0.4;
        }
        assert!(db_postprocess(&prob, mw as u32, mh as u32, 128, 128).is_empty());
    }

    #[test]
    fn db_postprocess_empty_map_returns_empty() {
        assert!(db_postprocess(&[], 0, 0, 10, 10).is_empty());
        assert!(db_postprocess(&[0.0; 16], 4, 4, 8, 8).is_empty());
    }

    #[test]
    fn sort_det_boxes_orders_same_row_left_to_right() {
        let b = |x: f64, y: f64| RectPhys {
            x,
            y,
            width: 50.0,
            height: 20.0,
        };
        // y 差 < 10 视为同一行 → 按 x 升序。
        let sorted = sort_det_boxes(vec![b(200.0, 100.0), b(20.0, 104.0), b(80.0, 101.0)]);
        assert_eq!(
            sorted.iter().map(|r| r.x).collect::<Vec<_>>(),
            vec![20.0, 80.0, 200.0]
        );
        // 不同行 → 按行（y）。
        let sorted = sort_det_boxes(vec![b(0.0, 200.0), b(100.0, 10.0)]);
        assert_eq!(sorted[0].y, 10.0);
    }

    // ───────── 像素工具 ─────────

    fn bgra_image(w: u32, h: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> OcrImage {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&fill(x, y));
            }
        }
        OcrImage::new(w, h, Arc::new(data)).unwrap()
    }

    #[test]
    fn ocr_image_validates_pixel_length() {
        let ok = OcrImage::new(2, 2, Arc::new(vec![0u8; 16]));
        assert!(ok.is_ok());
        let bad = OcrImage::new(2, 2, Arc::new(vec![0u8; 15]));
        assert!(bad.is_err());
        let zero = OcrImage::new(0, 2, Arc::new(vec![]));
        assert!(zero.is_err());
    }

    #[test]
    fn from_frame_requires_bgra() {
        let frame = FrozenFrame::skeleton(100, 50);
        assert!(OcrImage::from_frame(&frame).is_err());
        let frame = FrozenFrame {
            width: 2,
            height: 2,
            data_url: None,
            bgra: Some(Arc::new(vec![0u8; 16])),
        };
        assert!(OcrImage::from_frame(&frame).is_ok());
    }

    #[test]
    fn crop_bgra_keeps_rows_and_channels() {
        // x + y 作为像素值，便于断言裁剪位置。
        let image = bgra_image(4, 3, |x, y| [x as u8, y as u8, 7, 255]);
        let crop = crop_bgra(&image, 1, 1, 3, 3);
        assert_eq!((crop.width, crop.height), (2, 2));
        // 第一个像素 = 原图 (1,1) → [1,1,7,255]。
        assert_eq!(&crop.bgra[..4], &[1, 1, 7, 255]);
        // 最后一个像素 = 原图 (2,2) → [2,2,7,255]。
        let last = crop.bgra.len() - 4;
        assert_eq!(&crop.bgra[last..], &[2, 2, 7, 255]);
    }

    #[test]
    fn rotate_180_reverses_pixels() {
        let image = bgra_image(2, 2, |x, y| [(y * 2 + x) as u8, 0, 0, 255]);
        let rotated = rotate_bgra_180(&image);
        // 原顺序 0,1,2,3 → 旋转后 3,2,1,0。
        let values: Vec<u8> = rotated.bgra.chunks(4).map(|px| px[0]).collect();
        assert_eq!(values, vec![3, 2, 1, 0]);
    }

    #[test]
    fn resize_normalizes_to_neg_one_one() {
        // 2x1：黑、白。缩放到 2x1 应保持 [黑, 白] → 归一化 [-1, 1]。
        let image = bgra_image(2, 1, |x, _y| {
            if x == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let mut out = vec![0.0f32; 3 * 2];
        resize_bgra_to_chw(image.bgra.as_slice(), 2, 1, 2, 1, &mut out).unwrap();
        assert!((out[0] - (-1.0)).abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6);
        // 三个通道一致（灰度）。
        assert!((out[0] - out[2]).abs() < 1e-6);
    }

    #[test]
    fn crop_for_rec_rotates_tall_boxes() {
        // 竖排 20x60（h/w=3 ≥1.5）→ 旋转后 60x20。
        let image = bgra_image(20, 60, |_x, _y| [9, 9, 9, 255]);
        let bbox = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 60.0,
        };
        let crop = crop_for_rec(&image, &bbox).unwrap();
        assert_eq!((crop.width, crop.height), (60, 20));
    }

    // ───────── OCR 状态 ─────────

    #[test]
    fn status_reports_missing_assets_and_actionable_message() {
        let root = temp_root("status-missing");
        let assets = OcrAssets::resolve_from(None, &root.join("models"), None);
        let status = build_status(&assets);
        assert!(!status.ready);
        assert!(!status.runtime_ready);
        assert!(!status.bundled);
        assert_eq!(status.missing.len(), 5);
        assert!(status.missing.contains(&"ocr-runtime".to_string()));
        let message = status.message.unwrap();
        assert!(
            message.contains("ONNX Runtime"),
            "缺运行时时提示应含 ONNX Runtime：{message}"
        );
        // 契约字段：4 条资产 + 真实清单元数据（det 的真实 size/sha 来自 manifest）。
        assert_eq!(status.assets.len(), 4);
        let det = status
            .assets
            .iter()
            .find(|asset| asset.id == ASSET_DET)
            .unwrap();
        assert_eq!(det.size_bytes, 4_745_517);
        assert_eq!(
            det.sha256,
            "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9"
        );
        assert!(!det.installed && !det.bundled);
    }

    #[test]
    fn status_ready_when_all_assets_bundled() {
        let root = temp_root("status-ready");
        let resource = root.join("resource");
        dummy_ocr_dir(&resource);
        let assets = OcrAssets::resolve_from(Some(&resource), &root.join("models"), None);
        let status = build_status(&assets);
        assert!(status.ready);
        assert!(status.runtime_ready);
        assert!(status.bundled);
        assert!(status.missing.is_empty());
        assert!(status.message.is_none());
        assert!(status.runtime_path.unwrap().ends_with(ORT_DLL_NAME));
        assert!(status.assets.iter().all(|asset| asset.installed));
        assert!(status.assets.iter().all(|asset| asset.bundled));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_not_bundled_for_slim_layout() {
        let root = temp_root("status-slim");
        let resource = root.join("resource");
        let models = root.join("models");
        write_dummy(&resource.join(OCR_DIR_NAME).join(ORT_DLL_NAME), 16);
        dummy_ocr_dir(&models);
        let assets = OcrAssets::resolve_from(Some(&resource), &models, None);
        let status = build_status(&assets);
        assert!(status.ready);
        assert!(!status.bundled);
        assert!(status.message.is_none());
        assert!(status.assets.iter().all(|asset| asset.installed));
        assert!(status.assets.iter().all(|asset| !asset.bundled));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────── 引擎构造（不加载模型） ─────────

    #[test]
    fn engine_construct_is_lazy_and_reports_assets() {
        let root = temp_root("engine");
        let assets = OcrAssets::resolve_from(None, &root.join("models"), None);
        let engine = OnnxOcrEngine::new(assets);
        assert!(!engine.is_loaded());
        assert!(!engine.is_ready());
        // 缺资产时 warmup 返回可操作错误（SA-4001 / SA-2001），而不是 panic。
        let err = engine.warmup().unwrap_err();
        assert!(
            err.code == "SA-2001" || err.code == "SA-4001",
            "code = {}",
            err.code
        );
    }

    #[test]
    fn merged_box_is_split_into_text_rows() {
        // 深色背景 + 3 条白色文本带（每条约 12px 高）。
        let image = bgra_image(200, 110, |x, y| {
            let in_text_row =
                (10..22).contains(&y) || (45..57).contains(&y) || (80..92).contains(&y);
            let in_text_column = (10..190).contains(&x);
            if in_text_row && in_text_column {
                [255, 255, 255, 255]
            } else {
                [24, 24, 24, 255]
            }
        });
        let bbox = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 110.0,
        };
        let parts = split_merged_box(&image, &bbox).expect("三段文本应被拆成三行");
        assert_eq!(parts.len(), 3, "parts = {parts:?}");
        // 拆分后的行框应覆盖原框宽度且互不重叠。
        for part in &parts {
            assert!((part.width - 200.0).abs() < 1.0);
        }
        assert!(parts
            .windows(2)
            .all(|pair| pair[0].y + pair[0].height <= pair[1].y + 1.0));
    }

    #[test]
    fn single_line_box_is_not_split() {
        let image = bgra_image(200, 40, |x, y| {
            if (10..22).contains(&y) && (10..190).contains(&x) {
                [255, 255, 255, 255]
            } else {
                [24, 24, 24, 255]
            }
        });
        let bbox = RectPhys {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 40.0,
        };
        assert!(split_merged_box(&image, &bbox).is_none());
    }

    #[test]
    fn expand_merged_boxes_uses_median_height() {
        let image = bgra_image(300, 160, |x, y| {
            let in_text_row =
                (10..24).contains(&y) || (60..74).contains(&y) || (110..124).contains(&y);
            if in_text_row && (10..290).contains(&x) {
                [255, 255, 255, 255]
            } else {
                [24, 24, 24, 255]
            }
        });
        let boxes = vec![
            OcrBox {
                text: "a".into(),
                confidence: 1.0,
                bbox: RectPhys {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 28.0,
                },
            },
            OcrBox {
                text: "b".into(),
                confidence: 1.0,
                bbox: RectPhys {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 28.0,
                },
            },
            OcrBox {
                text: "merged".into(),
                confidence: 1.0,
                bbox: RectPhys {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 150.0,
                },
            },
        ];
        let median = median_box_height(&boxes);
        assert!((median - 28.0).abs() < 0.01);
        let expanded = expand_merged_boxes(&image, &boxes, median);
        // 两个正常框保留；合并框被拆成 ≥2 个行框。
        assert!(expanded.len() >= 4, "expanded = {expanded:?}");
        assert!(expanded.iter().filter(|item| item.text == "merged").count() == 0);
    }
}
