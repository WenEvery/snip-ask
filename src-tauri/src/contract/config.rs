//! 持久化配置模型（契约 §4）。
//!
//! 覆盖内容：
//! - §4.1 默认配置（Schema v5）与全部子结构；
//! - §4.2 枚举字面量与 [`ConfigView`]（providers 换成带密钥状态视图）；
//! - §4.3 约束校验 [`validate`]；
//! - 版本迁移 [`migrate`]（1 → 2 → 3 → 4 → 5，未知字段保留）；
//! - 深合并 [`merge_patch`]；
//! - 快捷键规范化（`Ctrl+Q` 形式）。
//!
//! 所有子结构均启用 `#[serde(default)]`（缺字段补默认）与 `#[serde(flatten)]`
//! 的 `extra` 字段（未知字段原样保留，契约 §4.3 / §16）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::errors::{AppError, AppResult};
use super::types::{ActionId, Mode};

/// 当前配置 Schema 版本。
pub const CONFIG_SCHEMA_VERSION: u32 = 5;

// ---------------------------------------------------------------------------
// 枚举（契约 §4.2）
// ---------------------------------------------------------------------------

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Language {
    /// 简体中文。
    #[default]
    #[serde(rename = "zh-CN")]
    ZhCn,
    /// 英文。
    #[serde(rename = "en-US")]
    EnUs,
}

/// 主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// 跟随系统。
    #[default]
    System,
    /// 浅色。
    Light,
    /// 深色。
    Dark,
}

/// 字号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    /// 小。
    Sm,
    /// 中。
    #[default]
    Md,
    /// 大。
    Lg,
}

/// 结果窗位置策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResultPosition {
    /// 跟随选区。
    #[default]
    Follow,
    /// 固定位置。
    Fixed,
}

/// 搜索模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// 浏览器搜索。
    #[default]
    Browser,
    /// AI 搜索。
    Ai,
}

/// 翻译后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranslateBackend {
    /// 云端。
    #[default]
    Cloud,
    /// 本地。
    Local,
}

/// 本地推理引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LocalEngine {
    /// llama.cpp。
    #[default]
    #[serde(rename = "llama.cpp")]
    LlamaCpp,
    /// Ollama。
    #[serde(rename = "ollama")]
    Ollama,
    /// CTranslate2。
    #[serde(rename = "ct2")]
    Ct2,
}

/// 计算设备（`local.device`，契约 v4）。
///
/// - `Auto`：优先 GPU，不可用（无后端/无设备/显存不足）时自动回退 CPU；
/// - `Cpu`：强制 CPU（省电/续航/发热策略，或用户显式选择）；
/// - `Gpu`：要求 GPU，仍不可用时回退 CPU 并在 `engine:status` 标注 `fallbackReason`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LocalDevice {
    /// 自动（默认）。
    #[default]
    Auto,
    /// 强制 CPU。
    Cpu,
    /// 优先 GPU。
    Gpu,
}

impl LocalDevice {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalDevice::Auto => "auto",
            LocalDevice::Cpu => "cpu",
            LocalDevice::Gpu => "gpu",
        }
    }

    /// 是否允许使用 GPU。
    pub fn allows_gpu(&self) -> bool {
        !matches!(self, LocalDevice::Cpu)
    }
}

/// GPU 后端（`local.gpuBackend`，契约 v4）。
///
/// **Vulkan 优先**：`Auto` 的解析顺序是 `Vulkan → CUDA → CPU`。Vulkan 是跨厂商后端
/// （AMD / Intel / NVIDIA 通用），CUDA 仅作为 NVIDIA 用户的可选组件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GpuBackend {
    /// 自动（Vulkan → CUDA → CPU）。
    #[default]
    Auto,
    /// Vulkan（跨厂商，推荐；AMD/Intel/NVIDIA 均可用）。
    Vulkan,
    /// CUDA（NVIDIA 可选组件）。
    Cuda,
}

impl GpuBackend {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            GpuBackend::Auto => "auto",
            GpuBackend::Vulkan => "vulkan",
            GpuBackend::Cuda => "cuda",
        }
    }

    /// 是否为具体后端（非 `Auto`）。
    pub fn is_concrete(&self) -> bool {
        !matches!(self, GpuBackend::Auto)
    }
}

/// 实际使用的推理后端（含 CPU；配置里的 [`GpuBackend`] 不含 CPU，因为 CPU 是回退结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LocalBackend {
    /// Vulkan（跨厂商）。
    Vulkan,
    /// CUDA（NVIDIA）。
    Cuda,
    /// CPU。
    #[default]
    Cpu,
}

impl LocalBackend {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalBackend::Vulkan => "vulkan",
            LocalBackend::Cuda => "cuda",
            LocalBackend::Cpu => "cpu",
        }
    }

    /// 是否为 GPU 后端。
    pub fn is_gpu(&self) -> bool {
        !matches!(self, LocalBackend::Cpu)
    }

    /// 对应的配置枚举（CPU 无对应值，返回 `None`）。
    pub fn as_gpu_backend(&self) -> Option<GpuBackend> {
        match self {
            LocalBackend::Vulkan => Some(GpuBackend::Vulkan),
            LocalBackend::Cuda => Some(GpuBackend::Cuda),
            LocalBackend::Cpu => None,
        }
    }

    /// 组件目录名（`<backends_dir>/<variant>/`）。
    pub fn variant(&self) -> &'static str {
        self.as_str()
    }
}

/// 供应商 API 类型（契约 v5）。
///
/// - `openAiCompatible`：OpenAI 兼容 Chat Completions（`/v1/chat/completions`），
///   同时适用于 DeepSeek、通用 OpenAI 兼容服务与本地 llama-server；
/// - `qwenMt`：DashScope 原生 Qwen-MT 翻译 API（`translation_options`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// OpenAI 兼容（默认）。
    #[default]
    OpenAiCompatible,
    /// DashScope Qwen-MT 专用翻译 API。
    QwenMt,
}

/// 模型能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cap {
    /// 视觉。
    Vision,
    /// 搜索。
    Search,
    /// 思考。
    Thinking,
    /// 翻译。
    Translate,
}

// ---------------------------------------------------------------------------
// 子结构（契约 §4.1 / §4.2）
// ---------------------------------------------------------------------------

/// 应用级偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppPrefs {
    /// 界面语言。
    pub language: Language,
    /// 开机自启。
    pub autostart: bool,
    /// 单实例。
    pub single_instance: bool,
    /// 启动时最小化到托盘。
    pub start_minimized: bool,
    /// 未知字段（前向兼容保留）。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            language: Language::ZhCn,
            autostart: false,
            single_instance: true,
            start_minimized: true,
            extra: serde_json::Map::new(),
        }
    }
}

/// 结果窗偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResultWindowPrefs {
    /// 位置策略。
    pub position: ResultPosition,
    /// 是否置顶。
    pub pin: bool,
    /// 不透明度。
    pub opacity: f64,
    /// 宽度（逻辑像素）。
    pub width: f64,
    /// 高度（逻辑像素）。
    pub height: f64,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for ResultWindowPrefs {
    fn default() -> Self {
        Self {
            position: ResultPosition::Follow,
            pin: true,
            opacity: 1.0,
            width: 460.0,
            height: 420.0,
            extra: serde_json::Map::new(),
        }
    }
}

/// 外观偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPrefs {
    /// 主题。
    pub theme: Theme,
    /// 强调色。
    pub accent: String,
    /// 字号。
    pub font_size: FontSize,
    /// 结果窗偏好。
    pub result_window: ResultWindowPrefs,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: "blue".to_string(),
            font_size: FontSize::Md,
            result_window: ResultWindowPrefs::default(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 全局热键偏好（空串表示未绑定）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    /// 开始框选。
    pub capture: String,
    /// 划词。
    pub word_grab: String,
    /// 快速翻译。
    pub quick_translate: String,
    /// 图片搜索。
    pub image_search: String,
    /// 翻译。
    pub translate: String,
    /// 解答。
    pub answer: String,
    /// 切换快 / 深模式。
    pub toggle_mode: String,
    /// 暂停 / 恢复热键。
    pub toggle_pause: String,
    /// 打开设置。
    pub open_settings: String,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            capture: "Ctrl+Q".to_string(),
            word_grab: String::new(),
            quick_translate: String::new(),
            image_search: String::new(),
            translate: String::new(),
            answer: String::new(),
            toggle_mode: String::new(),
            toggle_pause: String::new(),
            open_settings: String::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 动作菜单配置（每类菜单的动作顺序）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MenuPrefs {
    /// 文本动作顺序。
    pub text: Vec<ActionId>,
    /// 图像动作顺序。
    pub image: Vec<ActionId>,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for MenuPrefs {
    fn default() -> Self {
        Self {
            text: vec![
                ActionId::Search,
                ActionId::Translate,
                ActionId::Answer,
                ActionId::Copy,
            ],
            image: vec![ActionId::ImageSearch, ActionId::Answer, ActionId::Translate],
            extra: serde_json::Map::new(),
        }
    }
}

/// 动作偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionPrefs {
    /// 菜单顺序。
    pub menu: MenuPrefs,
    /// 默认动作（回车执行）。
    pub default_action: Option<ActionId>,
    /// 默认模式。
    pub default_mode: Mode,
    /// 结果是否自动复制。
    pub auto_copy: bool,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// LLM 供应商配置（不含密钥）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    /// 唯一 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// API Base URL。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// API 类型（契约 v5；默认 OpenAI 兼容）。
    pub kind: ProviderKind,
    /// 能力。
    pub caps: Vec<Cap>,
    /// 温度。
    pub temperature: f64,
    /// 最大输出 token。
    pub max_tokens: u32,
    /// 是否启用。
    pub enabled: bool,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            model: String::new(),
            kind: ProviderKind::OpenAiCompatible,
            caps: Vec::new(),
            temperature: 0.2,
            max_tokens: 1024,
            enabled: true,
            extra: serde_json::Map::new(),
        }
    }
}

/// 带密钥状态视图的供应商（命令出参）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    /// 唯一 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// API Base URL。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// API 类型（契约 v5；默认 OpenAI 兼容）。
    pub kind: ProviderKind,
    /// 能力。
    pub caps: Vec<Cap>,
    /// 温度。
    pub temperature: f64,
    /// 最大输出 token。
    pub max_tokens: u32,
    /// 是否启用。
    pub enabled: bool,
    /// 是否已配置密钥。
    pub has_key: bool,
    /// 密钥掩码（`••••` + 后四位）；未配置为 `null`。
    pub key_mask: Option<String>,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ProviderView {
    /// 由配置项 + 密钥状态构造。
    pub fn from_provider(provider: &Provider, has_key: bool, key_mask: Option<String>) -> Self {
        Self {
            id: provider.id.clone(),
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            kind: provider.kind,
            caps: provider.caps.clone(),
            temperature: provider.temperature,
            max_tokens: provider.max_tokens,
            enabled: provider.enabled,
            has_key,
            key_mask,
            extra: provider.extra.clone(),
        }
    }
}

/// 深度模式预算。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeepBudget {
    /// 搜索次数。
    pub search: i32,
    /// 抓取次数。
    pub fetch: i32,
    /// 最大输出 token。
    pub max_tokens: u32,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for DeepBudget {
    fn default() -> Self {
        Self {
            search: 3,
            fetch: 2,
            max_tokens: 1500,
            extra: serde_json::Map::new(),
        }
    }
}

/// 各动作默认模式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModesPrefs {
    /// 文本搜索默认模式。
    pub search: Mode,
    /// 图片搜索默认模式。
    pub image_search: Mode,
    /// 翻译默认模式。
    pub translate: Mode,
    /// 解答默认模式。
    pub answer: Mode,
    /// 深度预算。
    pub deep_budget: DeepBudget,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 搜索偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchPrefs {
    /// 搜索模式。
    pub mode: SearchMode,
    /// 搜索引擎标识。
    pub engine: String,
    /// 自定义 URL 模板（含 `{q}` 占位）。
    pub engine_url_template: Option<String>,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for SearchPrefs {
    fn default() -> Self {
        Self {
            mode: SearchMode::Browser,
            engine: "bing".to_string(),
            engine_url_template: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// 术语表条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GlossaryEntry {
    /// 原文。
    pub from: String,
    /// 译文。
    pub to: String,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 翻译偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslatePrefs {
    /// 翻译后端。
    pub backend: TranslateBackend,
    /// 云端供应商 id。
    pub provider_id: Option<String>,
    /// 专用翻译模型名。
    pub model: String,
    /// 目标语言（BCP-47 / Qwen 语言码）。
    pub target_lang: String,
    /// 术语表。
    pub glossary: Vec<GlossaryEntry>,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for TranslatePrefs {
    fn default() -> Self {
        Self {
            backend: TranslateBackend::Cloud,
            provider_id: Some("deepseek".to_string()),
            model: "qwen-mt-lite".to_string(),
            target_lang: "zh-CHS".to_string(),
            glossary: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 本地推理偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalPrefs {
    /// 本地引擎。
    pub engine: LocalEngine,
    /// 本地模型 id。
    pub model: String,
    /// 监听端口（仅 127.0.0.1）。
    pub port: u16,
    /// 空闲卸载秒数。
    pub idle_unload_sec: u32,
    /// 计算设备（`auto` / `cpu` / `gpu`，契约 v4）。
    pub device: LocalDevice,
    /// GPU 后端（`auto` / `vulkan` / `cuda`，契约 v4；`auto` = Vulkan 优先）。
    pub gpu_backend: GpuBackend,
    /// offload 到 GPU 的层数（契约 v4；99 = 全部，0 = 不 offload）。
    pub gpu_layers: u32,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        Self {
            engine: LocalEngine::LlamaCpp,
            model: "hy-mt-1.8b-q4".to_string(),
            port: 39281,
            idle_unload_sec: 600,
            device: LocalDevice::Auto,
            gpu_backend: GpuBackend::Auto,
            gpu_layers: 99,
            extra: serde_json::Map::new(),
        }
    }
}

impl LocalPrefs {
    /// 实际参与 offload 的层数（`device = cpu` 时恒为 0）。
    pub fn effective_gpu_layers(&self) -> u32 {
        if self.device.allows_gpu() {
            self.gpu_layers
        } else {
            0
        }
    }
}

/// OCR 偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OcrPrefs {
    /// OCR 引擎（当前仅 `rapidocr`）。
    pub engine: String,
    /// 检测长边上限。
    pub det_max_edge: u32,
    /// 文本置信度阈值。
    pub text_threshold: f64,
    /// 是否保留文字框。
    pub preserve_boxes: bool,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for OcrPrefs {
    fn default() -> Self {
        Self {
            engine: "rapidocr".to_string(),
            det_max_edge: 3200,
            text_threshold: 0.6,
            preserve_boxes: true,
            extra: serde_json::Map::new(),
        }
    }
}

/// 隐私偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrivacyPrefs {
    /// 允许文本上传。
    pub allow_text_upload: bool,
    /// 允许图像上传。
    pub allow_image_upload: bool,
    /// 仅本地模式。
    pub local_only: bool,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for PrivacyPrefs {
    fn default() -> Self {
        Self {
            allow_text_upload: true,
            allow_image_upload: true,
            local_only: false,
            extra: serde_json::Map::new(),
        }
    }
}

/// 历史偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryPrefs {
    /// 是否记录历史。
    pub enabled: bool,
    /// 最多保留条数。
    pub max_entries: u32,
    /// 保留天数。
    pub retention_days: u32,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for HistoryPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 1000,
            retention_days: 90,
            extra: serde_json::Map::new(),
        }
    }
}

/// 高级偏好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdvancedPrefs {
    /// 代理（需重启应用）。
    pub proxy: Option<String>,
    /// 遥测（默认关闭，且不实现）。
    pub telemetry: bool,
    /// 骨架模式（调试开关；v1.0.0 默认关闭，走真实捕获/OCR/LLM）。
    pub skeleton_mode: bool,
    /// 未知字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for AdvancedPrefs {
    fn default() -> Self {
        Self {
            proxy: None,
            telemetry: false,
            skeleton_mode: false,
            extra: serde_json::Map::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Config / ConfigView
// ---------------------------------------------------------------------------

/// 持久化配置（不含密钥），契约 §4.1。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Schema 版本。
    pub schema_version: u32,
    /// 应用偏好。
    pub app: AppPrefs,
    /// 外观偏好。
    pub ui: UiPrefs,
    /// 热键偏好。
    pub hotkeys: Hotkeys,
    /// 动作偏好。
    pub actions: ActionPrefs,
    /// LLM 供应商。
    pub providers: Vec<Provider>,
    /// 模式偏好。
    pub modes: ModesPrefs,
    /// 搜索偏好。
    pub search: SearchPrefs,
    /// 翻译偏好。
    pub translate: TranslatePrefs,
    /// 本地推理偏好。
    pub local: LocalPrefs,
    /// OCR 偏好。
    pub ocr: OcrPrefs,
    /// 隐私偏好。
    pub privacy: PrivacyPrefs,
    /// 历史偏好。
    pub history: HistoryPrefs,
    /// 高级偏好。
    pub advanced: AdvancedPrefs,
    /// 未知顶层字段（前向兼容保留）。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            app: AppPrefs {
                language: Language::ZhCn,
                autostart: false,
                single_instance: true,
                start_minimized: true,
                extra: serde_json::Map::new(),
            },
            ui: UiPrefs::default(),
            hotkeys: Hotkeys::default(),
            actions: ActionPrefs::default(),
            providers: vec![
                Provider {
                    id: "deepseek".to_string(),
                    name: "DeepSeek".to_string(),
                    base_url: "https://api.deepseek.com".to_string(),
                    model: "deepseek-flash".to_string(),
                    kind: ProviderKind::OpenAiCompatible,
                    caps: vec![Cap::Vision, Cap::Search, Cap::Thinking],
                    temperature: 0.2,
                    max_tokens: 1024,
                    enabled: true,
                    extra: serde_json::Map::new(),
                },
                Provider {
                    id: "dashscope".to_string(),
                    name: "DashScope（Qwen-MT 翻译）".to_string(),
                    base_url: "https://dashscope.aliyuncs.com".to_string(),
                    model: "qwen-mt-lite".to_string(),
                    kind: ProviderKind::QwenMt,
                    caps: vec![Cap::Translate],
                    temperature: 0.2,
                    max_tokens: 1024,
                    enabled: false,
                    extra: serde_json::Map::new(),
                },
                Provider {
                    id: "generic-openai".to_string(),
                    name: "通用 OpenAI 兼容".to_string(),
                    base_url: "https://api.openai.com".to_string(),
                    model: "gpt-4o-mini".to_string(),
                    kind: ProviderKind::OpenAiCompatible,
                    caps: vec![Cap::Vision, Cap::Search, Cap::Thinking, Cap::Translate],
                    temperature: 0.2,
                    max_tokens: 1024,
                    enabled: false,
                    extra: serde_json::Map::new(),
                },
            ],
            modes: ModesPrefs::default(),
            search: SearchPrefs::default(),
            translate: TranslatePrefs::default(),
            local: LocalPrefs::default(),
            ocr: OcrPrefs::default(),
            privacy: PrivacyPrefs::default(),
            history: HistoryPrefs::default(),
            advanced: AdvancedPrefs::default(),
            extra: serde_json::Map::new(),
        }
    }
}

/// 命令返回的配置视图（providers 换为带密钥状态视图），契约 §4.2。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    /// Schema 版本。
    pub schema_version: u32,
    /// 应用偏好。
    pub app: AppPrefs,
    /// 外观偏好。
    pub ui: UiPrefs,
    /// 热键偏好。
    pub hotkeys: Hotkeys,
    /// 动作偏好。
    pub actions: ActionPrefs,
    /// 供应商视图。
    pub providers: Vec<ProviderView>,
    /// 模式偏好。
    pub modes: ModesPrefs,
    /// 搜索偏好。
    pub search: SearchPrefs,
    /// 翻译偏好。
    pub translate: TranslatePrefs,
    /// 本地推理偏好。
    pub local: LocalPrefs,
    /// OCR 偏好。
    pub ocr: OcrPrefs,
    /// 隐私偏好。
    pub privacy: PrivacyPrefs,
    /// 历史偏好。
    pub history: HistoryPrefs,
    /// 高级偏好。
    pub advanced: AdvancedPrefs,
    /// 未知顶层字段。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ConfigView {
    /// 由配置 + 每个提供商的密钥状态 `(hasKey, keyMask)` 构造。
    pub fn from_config(config: &Config, provider_status: &[(bool, Option<String>)]) -> Self {
        let providers = config
            .providers
            .iter()
            .enumerate()
            .map(|(index, provider)| {
                let (has_key, key_mask) =
                    provider_status.get(index).cloned().unwrap_or((false, None));
                ProviderView::from_provider(provider, has_key, key_mask)
            })
            .collect();
        Self {
            schema_version: config.schema_version,
            app: config.app.clone(),
            ui: config.ui.clone(),
            hotkeys: config.hotkeys.clone(),
            actions: config.actions.clone(),
            providers,
            modes: config.modes.clone(),
            search: config.search.clone(),
            translate: config.translate.clone(),
            local: config.local.clone(),
            ocr: config.ocr.clone(),
            privacy: config.privacy.clone(),
            history: config.history.clone(),
            advanced: config.advanced.clone(),
            extra: config.extra.clone(),
        }
    }
}

impl From<&Config> for ConfigView {
    fn from(config: &Config) -> Self {
        Self::from_config(config, &[])
    }
}

impl Default for ConfigView {
    fn default() -> Self {
        Self::from_config(&Config::default(), &[])
    }
}

// ---------------------------------------------------------------------------
// 快捷键规范（契约 §4.3）
// ---------------------------------------------------------------------------

/// 快捷键修饰键规范顺序。
fn modifier_rank(modifier: &str) -> u8 {
    match modifier {
        "Ctrl" => 0,
        "Alt" => 1,
        "Shift" => 2,
        _ => 3,
    }
}

/// 规范化快捷键字符串为 `Ctrl+Shift+Q` 形式。
///
/// 非法输入（含空串）返回 `None`；空串由调用方视为「未绑定」。
pub fn normalize_hotkey(spec: &str) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    // 最后一段必须是单个字母或数字（大小写不敏感，规范化为大写）。
    let key = parts.last()?;
    let mut key_chars = key.chars();
    let key_char = key_chars.next()?;
    if key_chars.next().is_some() {
        return None;
    }
    let key_upper = key_char.to_ascii_uppercase();
    if !(key_upper.is_ascii_uppercase() || key_upper.is_ascii_digit()) {
        return None;
    }
    // 前若干段必须是修饰键（大小写不敏感）。
    let mut modifiers: Vec<&'static str> = Vec::new();
    for raw in &parts[..parts.len() - 1] {
        let lower = raw.to_ascii_lowercase();
        let canonical = match lower.as_str() {
            "ctrl" | "control" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            "win" => "Win",
            _ => return None,
        };
        if !modifiers.contains(&canonical) {
            modifiers.push(canonical);
        }
    }
    if modifiers.is_empty() {
        return None;
    }
    modifiers.sort_by_key(|item| modifier_rank(item));
    let mut normalized = modifiers.join("+");
    normalized.push('+');
    normalized.push(key_upper);
    Some(normalized)
}

/// 校验快捷键是否符合契约 §4.3 语义（空串允许，表示未绑定）。
pub fn is_valid_hotkey(spec: &str) -> bool {
    spec.trim().is_empty() || normalize_hotkey(spec).is_some()
}

// ---------------------------------------------------------------------------
// 校验（契约 §4.3）
// ---------------------------------------------------------------------------

fn validate_provider_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
    })
}

fn validate_base_url(base_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(base_url).map_err(|err| format!("不是合法 URL：{err}"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("不得包含用户信息（user:pass@）".to_string());
    }
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if parsed.host_str() == Some("127.0.0.1") => Ok(()),
        "http" => Err("http 仅允许 127.0.0.1".to_string()),
        other => Err(format!("仅允许 https 或 http://127.0.0.1，当前为 {other}")),
    }
}

fn validate_hotkey_field(field: &str, spec: &str) -> AppResult<()> {
    if is_valid_hotkey(spec) {
        Ok(())
    } else {
        Err(AppError::invalid_config(
            field,
            "快捷键必须匹配 (Ctrl|Alt|Shift|Win)[+...]+[A-Z0-9]",
        ))
    }
}

/// 按契约 §4.3 校验整份配置；非法返回 `SA-5001`（detail 带字段名）。
pub fn validate(config: &Config) -> AppResult<()> {
    if config.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(AppError::invalid_config(
            "schemaVersion",
            format!("必须为 {CONFIG_SCHEMA_VERSION}"),
        ));
    }

    let mut seen_ids: HashSet<&str> = HashSet::new();
    for (index, provider) in config.providers.iter().enumerate() {
        let prefix = format!("providers[{index}]");
        if !validate_provider_id(&provider.id) {
            return Err(AppError::invalid_config(
                &format!("{prefix}.id"),
                "仅允许 [a-z0-9-_]{1,32}",
            ));
        }
        if !seen_ids.insert(provider.id.as_str()) {
            return Err(AppError::invalid_config(
                &format!("{prefix}.id"),
                "供应商 id 重复",
            ));
        }
        validate_base_url(&provider.base_url)
            .map_err(|reason| AppError::invalid_config(&format!("{prefix}.baseUrl"), reason))?;
        if provider.temperature < 0.0 || provider.temperature > 2.0 {
            return Err(AppError::invalid_config(
                &format!("{prefix}.temperature"),
                "取值范围 [0, 2]",
            ));
        }
        if provider.max_tokens == 0 || provider.max_tokens > 131_072 {
            return Err(AppError::invalid_config(
                &format!("{prefix}.maxTokens"),
                "取值范围 [1, 131072]",
            ));
        }
    }

    let budget = &config.modes.deep_budget;
    if !(0..=5).contains(&budget.search) {
        return Err(AppError::invalid_config(
            "modes.deepBudget.search",
            "取值范围 [0,5]",
        ));
    }
    if !(0..=5).contains(&budget.fetch) {
        return Err(AppError::invalid_config(
            "modes.deepBudget.fetch",
            "取值范围 [0,5]",
        ));
    }
    if !(128..=8192).contains(&budget.max_tokens) {
        return Err(AppError::invalid_config(
            "modes.deepBudget.maxTokens",
            "取值范围 [128,8192]",
        ));
    }

    let opacity = config.ui.result_window.opacity;
    if !(0.3..=1.0).contains(&opacity) {
        return Err(AppError::invalid_config(
            "ui.resultWindow.opacity",
            "取值范围 [0.3,1.0]",
        ));
    }
    if config.history.max_entries > 100_000 {
        return Err(AppError::invalid_config(
            "history.maxEntries",
            "取值范围 [0,100000]",
        ));
    }
    if config.history.retention_days > 3650 {
        return Err(AppError::invalid_config(
            "history.retentionDays",
            "取值范围 [0,3650]",
        ));
    }
    if config.local.port < 1024 {
        return Err(AppError::invalid_config(
            "local.port",
            "取值范围 [1024,65535]",
        ));
    }
    if config.local.model.trim().is_empty() {
        return Err(AppError::invalid_config("local.model", "不能为空"));
    }
    // 契约 v4：GPU 层数上限 512（99 = 全部；0 = 不 offload；显存紧张可设 8–24）。
    if config.local.gpu_layers > 512 {
        return Err(AppError::invalid_config(
            "local.gpuLayers",
            "取值范围 [0,512]",
        ));
    }
    // 契约 v4：idle 卸载上限 30 天（0 = 常驻）。
    if config.local.idle_unload_sec > 2_592_000 {
        return Err(AppError::invalid_config(
            "local.idleUnloadSec",
            "取值范围 [0,2592000]",
        ));
    }

    for (menu_name, actions) in [
        ("text", &config.actions.menu.text),
        ("image", &config.actions.menu.image),
    ] {
        let mut seen: HashSet<ActionId> = HashSet::new();
        for action in actions {
            if !seen.insert(*action) {
                return Err(AppError::invalid_config(
                    &format!("actions.menu.{menu_name}"),
                    format!("动作 {} 重复出现", action),
                ));
            }
        }
    }
    if let Some(default_action) = config.actions.default_action {
        let in_text = config.actions.menu.text.contains(&default_action);
        let in_image = config.actions.menu.image.contains(&default_action);
        if !in_text && !in_image {
            return Err(AppError::invalid_config(
                "actions.defaultAction",
                "必须属于某个菜单",
            ));
        }
    }

    let hotkeys: [(&str, &String); 9] = [
        ("hotkeys.capture", &config.hotkeys.capture),
        ("hotkeys.wordGrab", &config.hotkeys.word_grab),
        ("hotkeys.quickTranslate", &config.hotkeys.quick_translate),
        ("hotkeys.imageSearch", &config.hotkeys.image_search),
        ("hotkeys.translate", &config.hotkeys.translate),
        ("hotkeys.answer", &config.hotkeys.answer),
        ("hotkeys.toggleMode", &config.hotkeys.toggle_mode),
        ("hotkeys.togglePause", &config.hotkeys.toggle_pause),
        ("hotkeys.openSettings", &config.hotkeys.open_settings),
    ];
    for (field, spec) in hotkeys {
        validate_hotkey_field(field, spec)?;
    }

    if config.ocr.engine != "rapidocr" {
        return Err(AppError::invalid_config(
            "ocr.engine",
            "当前仅支持 rapidocr",
        ));
    }
    if !(0.0..=1.0).contains(&config.ocr.text_threshold) {
        return Err(AppError::invalid_config(
            "ocr.textThreshold",
            "取值范围 [0,1]",
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 深合并（契约 §4.4 config_patch）
// ---------------------------------------------------------------------------

fn merge_value(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                if value.is_null() {
                    // null 表示清除可选值：删除后由默认值补齐（Option → null）。
                    base_map.remove(&key);
                } else {
                    merge_value(
                        base_map.entry(key).or_insert(serde_json::Value::Null),
                        value,
                    );
                }
            }
        }
        (base_slot, patch_value) => {
            *base_slot = patch_value;
        }
    }
}

/// 深合并 `patch` 到 `config`，随后校验；任何失败都不会修改 `config`。
///
/// 未知字段保留：`Config` 各级结构体的 `extra` 会原样往返。
/// 类型不匹配返回 `SA-5001`（detail 指向反序列化位置）。
pub fn merge_patch(config: &mut Config, patch: serde_json::Value) -> AppResult<()> {
    let mut base = serde_json::to_value(&*config)?;
    merge_value(&mut base, patch);
    let merged: Config = serde_json::from_value(base)
        .map_err(|err| AppError::invalid_config("patch", format!("类型不匹配：{err}")))?;
    validate(&merged)?;
    *config = merged;
    Ok(())
}

// ---------------------------------------------------------------------------
// 版本迁移（契约 §4.3：1 → 2 → 3）
// ---------------------------------------------------------------------------

/// 深填充默认值：仅补齐缺失字段，不覆盖已有值。
fn fill_defaults(target: &mut serde_json::Value, defaults: &serde_json::Value) {
    match (target, defaults) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(default_map)) => {
            for (key, default_value) in default_map {
                match target_map.get_mut(key) {
                    Some(child) => fill_defaults(child, default_value),
                    None => {
                        target_map.insert(key.clone(), default_value.clone());
                    }
                }
            }
        }
        // 数组：默认模板只有一个元素时，把它补齐到每个已有元素（仅用于 providers）。
        (serde_json::Value::Array(target_items), serde_json::Value::Array(default_items)) => {
            if let Some(template) = default_items.first() {
                for item in target_items.iter_mut() {
                    fill_defaults(item, template);
                }
            }
        }
        _ => {}
    }
}

fn migrate_v1_to_v2(value: &mut serde_json::Value) {
    let defaults = serde_json::json!({
        "app": { "startMinimized": true },
        "ui": { "accent": "blue" },
        "hotkeys": {
            "wordGrab": "",
            "quickTranslate": "",
            "imageSearch": "",
            "translate": "",
            "answer": "",
            "toggleMode": "",
            "togglePause": "",
            "openSettings": ""
        },
        "actions": {
            "defaultAction": null,
            "defaultMode": "quick",
            "autoCopy": false
        },
        "modes": {
            "deepBudget": { "search": 3, "fetch": 2, "maxTokens": 1500 }
        },
        "search": { "engineUrlTemplate": null },
        "providers": [
            { "name": "Default", "caps": [], "temperature": 0.2, "maxTokens": 1024, "enabled": true }
        ]
    });
    // providers[].name 缺省时用 id 兜底，避免出现空展示名。
    // 必须在 fill_defaults 之前执行，否则默认模板里的 `name: "Default"` 会抢先占位。
    if let Some(providers) = value
        .get_mut("providers")
        .and_then(serde_json::Value::as_array_mut)
    {
        for provider in providers {
            let Some(object) = provider.as_object_mut() else {
                continue;
            };
            let id = object
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("provider")
                .to_string();
            if object
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map_or(true, str::is_empty)
            {
                object.insert("name".to_string(), serde_json::Value::String(id));
            }
        }
    }

    fill_defaults(value, &defaults);
}

fn migrate_v2_to_v3(value: &mut serde_json::Value) {
    let defaults = serde_json::json!({
        "ui": {
            "resultWindow": { "position": "follow", "pin": true, "opacity": 1.0, "width": 460, "height": 420 }
        },
        "translate": {
            "providerId": "deepseek",
            "glossary": []
        },
        "local": { "port": 39281, "idleUnloadSec": 600 },
        "ocr": { "textThreshold": 0.6, "preserveBoxes": true },
        "privacy": { "localOnly": false },
        "history": { "retentionDays": 90 },
        "advanced": { "telemetry": false, "skeletonMode": true }
    });
    fill_defaults(value, &defaults);
}

/// v3 → v4：新增 `local.device` / `local.gpuBackend` / `local.gpuLayers`。
///
/// 默认值刻意选择「行为不变」的保守值之外的最小破坏：`device=auto` 会在装有 GPU 后端时启用
/// GPU（研究 §1 的收益点），但**没有下载后端/没有可用设备时会自动回退 CPU**，因此对旧用户
/// 不会造成启动失败。`gpuBackend=auto` 的解析顺序为 Vulkan → CUDA → CPU。
fn migrate_v3_to_v4(value: &mut serde_json::Value) {
    let defaults = serde_json::json!({
        "local": { "device": "auto", "gpuBackend": "auto", "gpuLayers": 99 }
    });
    fill_defaults(value, &defaults);
}

/// v4 → v5：移除已删除的 `advanced.updateChannel` 更新通道；把历史默认值
/// `advanced.skeletonMode = true` 升为 `false`（v1.0.0 默认走真实能力，用户仍可手动改回）。
fn migrate_v4_to_v5(value: &mut serde_json::Value) {
    if let Some(advanced) = value
        .get_mut("advanced")
        .and_then(serde_json::Value::as_object_mut)
    {
        advanced.remove("updateChannel");
        advanced.insert("skeletonMode".to_string(), serde_json::json!(false));
    }
}

/// 读取任意版本配置 JSON，按 `1 → 2 → 3 → 4 → 5` 链迁移并校验（未知字段保留）。
pub fn migrate(mut value: serde_json::Value) -> AppResult<Config> {
    let version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as u32;
    if version == 0 || version > CONFIG_SCHEMA_VERSION {
        return Err(AppError::invalid_config(
            "schemaVersion",
            format!("不支持的版本 {version}（当前 {CONFIG_SCHEMA_VERSION}）"),
        ));
    }
    let mut current = version;
    if current < 2 {
        migrate_v1_to_v2(&mut value);
        current = 2;
        tracing::debug!(from = 1, to = 2, "配置迁移完成");
    }
    if current < 3 {
        migrate_v2_to_v3(&mut value);
        current = 3;
        tracing::debug!(from = 2, to = 3, "配置迁移完成");
    }
    if current < 4 {
        migrate_v3_to_v4(&mut value);
        current = 4;
        tracing::debug!(from = 3, to = 4, "配置迁移完成");
    }
    if current < 5 {
        migrate_v4_to_v5(&mut value);
        tracing::debug!(from = 4, to = 5, "配置迁移完成");
    }
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "schemaVersion".to_string(),
            serde_json::json!(CONFIG_SCHEMA_VERSION),
        );
    }
    let config: Config = serde_json::from_value(value).map_err(|err| {
        AppError::invalid_config("schemaVersion", format!("迁移结果无法解析：{err}"))
    })?;
    validate(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_contract_json() {
        let expected = serde_json::json!({
            "schemaVersion": 5,
            "app": { "language": "zh-CN", "autostart": false, "singleInstance": true, "startMinimized": true },
            "ui": {
                "theme": "system",
                "accent": "blue",
                "fontSize": "md",
                "resultWindow": { "position": "follow", "pin": true, "opacity": 1.0, "width": 460.0, "height": 420.0 }
            },
            "hotkeys": {
                "capture": "Ctrl+Q", "wordGrab": "", "quickTranslate": "", "imageSearch": "",
                "translate": "", "answer": "", "toggleMode": "", "togglePause": "", "openSettings": ""
            },
            "actions": {
                "menu": {
                    "text": ["search", "translate", "answer", "copy"],
                    "image": ["imageSearch", "answer", "translate"]
                },
                "defaultAction": null,
                "defaultMode": "quick",
                "autoCopy": false
            },
            "providers": [
                {
                    "id": "deepseek", "name": "DeepSeek", "baseUrl": "https://api.deepseek.com",
                    "model": "deepseek-flash", "kind": "openAiCompatible",
                    "caps": ["vision", "search", "thinking"],
                    "temperature": 0.2, "maxTokens": 1024, "enabled": true
                },
                {
                    "id": "dashscope", "name": "DashScope（Qwen-MT 翻译）",
                    "baseUrl": "https://dashscope.aliyuncs.com", "model": "qwen-mt-lite",
                    "kind": "qwenMt", "caps": ["translate"],
                    "temperature": 0.2, "maxTokens": 1024, "enabled": false
                },
                {
                    "id": "generic-openai", "name": "通用 OpenAI 兼容",
                    "baseUrl": "https://api.openai.com", "model": "gpt-4o-mini",
                    "kind": "openAiCompatible",
                    "caps": ["vision", "search", "thinking", "translate"],
                    "temperature": 0.2, "maxTokens": 1024, "enabled": false
                }
            ],
            "modes": {
                "search": "quick", "imageSearch": "quick", "translate": "quick", "answer": "quick",
                "deepBudget": { "search": 3, "fetch": 2, "maxTokens": 1500 }
            },
            "search": { "mode": "browser", "engine": "bing", "engineUrlTemplate": null },
            "translate": {
                "backend": "cloud", "providerId": "deepseek", "model": "qwen-mt-lite",
                "targetLang": "zh-CHS", "glossary": []
            },
            "local": {
                "engine": "llama.cpp", "model": "hy-mt-1.8b-q4", "port": 39281, "idleUnloadSec": 600,
                "device": "auto", "gpuBackend": "auto", "gpuLayers": 99
            },
            "ocr": { "engine": "rapidocr", "detMaxEdge": 3200, "textThreshold": 0.6, "preserveBoxes": true },
            "privacy": { "allowTextUpload": true, "allowImageUpload": true, "localOnly": false },
            "history": { "enabled": true, "maxEntries": 1000, "retentionDays": 90 },
            "advanced": { "proxy": null, "telemetry": false, "skeletonMode": false }
        });
        assert_eq!(serde_json::to_value(Config::default()).unwrap(), expected);
        validate(&Config::default()).expect("默认配置必须通过校验");
    }

    #[test]
    fn validate_rejects_invalid_provider() {
        let mut config = Config::default();
        config.providers[0].id = "DeepSeek".to_string();
        assert_eq!(validate(&config).unwrap_err().code, "SA-5001");

        let mut config = Config::default();
        config.providers[0].base_url = "http://api.deepseek.com".to_string();
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.providers[0].base_url = "https://user:pass@api.deepseek.com".to_string();
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.providers[0].base_url = "http://127.0.0.1:39281".to_string();
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_values() {
        let mut config = Config::default();
        config.modes.deep_budget.fetch = 9;
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.ui.result_window.opacity = 0.1;
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.local.port = 80;
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.actions.menu.text = vec![ActionId::Search, ActionId::Search];
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config
            .actions
            .menu
            .text
            .retain(|action| *action != ActionId::Copy);
        config.actions.default_action = Some(ActionId::Copy);
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.hotkeys.capture = "Ctrl+Q+".to_string();
        assert!(validate(&config).is_err());

        let mut config = Config::default();
        config.hotkeys.translate = "ctrl+shift+t".to_string();
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn merge_patch_is_deep_and_keeps_unknown_fields() {
        let mut config = Config::default();
        let patch = serde_json::json!({
            "ui": { "theme": "dark" },
            "privacy": { "localOnly": true },
            "futureFlag": 42
        });
        merge_patch(&mut config, patch).unwrap();
        assert_eq!(config.ui.theme, Theme::Dark);
        assert_eq!(config.ui.accent, "blue");
        assert!(config.privacy.local_only);
        assert_eq!(
            config
                .extra
                .get("futureFlag")
                .and_then(serde_json::Value::as_i64),
            Some(42)
        );
    }

    #[test]
    fn merge_patch_rejects_type_mismatch_without_touching_state() {
        let mut config = Config::default();
        let before = config.clone();
        let patch = serde_json::json!({ "ui": { "theme": 123 } });
        let err = merge_patch(&mut config, patch).unwrap_err();
        assert_eq!(err.code, "SA-5001");
        assert_eq!(config, before);
    }

    #[test]
    fn merge_patch_null_clears_optional_value() {
        let mut config = Config::default();
        config.search.engine_url_template = Some("https://example.com?q={q}".to_string());
        merge_patch(
            &mut config,
            serde_json::json!({ "search": { "engineUrlTemplate": null } }),
        )
        .unwrap();
        assert!(config.search.engine_url_template.is_none());
    }

    #[test]
    fn migrate_from_v1_fills_defaults_and_keeps_unknown() {
        let raw = serde_json::json!({
            "schemaVersion": 1,
            "app": { "language": "en-US", "autostart": false, "singleInstance": true },
            "ui": { "theme": "light", "fontSize": "sm", "resultWindow": { "position": "fixed", "opacity": 0.8 } },
            "hotkeys": { "capture": "Alt+Q" },
            "actions": { "menu": { "text": ["search"], "image": ["imageSearch"] } },
            "providers": [{ "id": "deepseek", "baseUrl": "https://api.deepseek.com", "model": "deepseek-flash" }],
            "modes": { "search": "quick", "imageSearch": "quick", "translate": "quick", "answer": "quick" },
            "search": { "mode": "browser", "engine": "bing" },
            "translate": { "backend": "cloud", "model": "qwen-mt-lite", "targetLang": "zh-CHS" },
            "local": { "engine": "llama.cpp", "model": "hy-mt-1.8b-q4" },
            "ocr": { "engine": "rapidocr", "detMaxEdge": 3200 },
            "privacy": { "allowTextUpload": true, "allowImageUpload": true },
            "history": { "enabled": true, "maxEntries": 1000 },
            "advanced": { "proxy": null, "updateChannel": "stable" },
            "unknownGroup": { "hello": "world" }
        });
        let config = migrate(raw).unwrap();
        assert_eq!(config.schema_version, 5);
        assert_eq!(config.app.language, Language::EnUs);
        assert_eq!(config.ui.result_window.width, 460.0);
        assert!(config.ui.result_window.pin);
        assert_eq!(config.hotkeys.capture, "Alt+Q");
        assert_eq!(config.hotkeys.translate, "");
        assert_eq!(config.providers[0].name, "deepseek");
        assert_eq!(config.providers[0].temperature, 0.2);
        assert_eq!(config.modes.deep_budget.max_tokens, 1500);
        assert_eq!(config.local.port, 39281);
        assert_eq!(config.local.device, LocalDevice::Auto);
        assert_eq!(config.local.gpu_backend, GpuBackend::Auto);
        assert_eq!(config.local.gpu_layers, 99);
        assert!(
            !config.advanced.skeleton_mode,
            "v5 迁移必须关闭历史默认开启的骨架模式"
        );
        assert!(
            !config.advanced.extra.contains_key("updateChannel"),
            "v5 迁移必须移除 updateChannel"
        );
        assert_eq!(config.providers[0].kind, ProviderKind::OpenAiCompatible);
        assert!(config.extra.contains_key("unknownGroup"));
    }

    #[test]
    fn migrate_from_v3_adds_gpu_fields_and_keeps_user_values() {
        let raw = serde_json::json!({
            "schemaVersion": 3,
            "local": { "engine": "llama.cpp", "model": "qwen2.5-0.5b-instruct-q4", "port": 39285, "idleUnloadSec": 300 },
            "translate": { "backend": "local", "model": "qwen-mt-lite", "targetLang": "en" }
        });
        let config = migrate(raw).unwrap();
        assert_eq!(config.schema_version, 5);
        // 用户已有值保持不变
        assert_eq!(config.local.model, "qwen2.5-0.5b-instruct-q4");
        assert_eq!(config.local.port, 39285);
        assert_eq!(config.local.idle_unload_sec, 300);
        assert_eq!(config.translate.backend, TranslateBackend::Local);
        // v4 新字段取默认值
        assert_eq!(config.local.device, LocalDevice::Auto);
        assert_eq!(config.local.gpu_backend, GpuBackend::Auto);
        assert_eq!(config.local.gpu_layers, 99);
    }

    #[test]
    fn local_device_and_backend_serialize_as_contract_literals() {
        let mut config = Config::default();
        config.local.device = LocalDevice::Gpu;
        config.local.gpu_backend = GpuBackend::Vulkan;
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["local"]["device"], "gpu");
        assert_eq!(value["local"]["gpuBackend"], "vulkan");
        assert_eq!(value["local"]["gpuLayers"], 99);
        // 反序列化：非法枚举值必须被拒绝（由 serde 生成可操作错误）。
        let mut json = serde_json::to_value(&config).unwrap();
        json["local"]["gpuBackend"] = serde_json::json!("rocm");
        assert!(serde_json::from_value::<Config>(json).is_err());
        // cpu 设备时 effective_gpu_layers 恒为 0
        config.local.device = LocalDevice::Cpu;
        assert_eq!(config.local.effective_gpu_layers(), 0);
        assert!(LocalDevice::Auto.allows_gpu());
        assert_eq!(GpuBackend::Auto.as_str(), "auto");
    }

    #[test]
    fn validate_rejects_gpu_layers_out_of_range() {
        let mut config = Config::default();
        config.local.gpu_layers = 513;
        assert_eq!(validate(&config).unwrap_err().code, "SA-5001");
        config.local.gpu_layers = 0;
        validate(&config).expect("0 合法（CPU offload）");
        config.local.gpu_layers = 24;
        validate(&config).expect("24 合法（显存降档）");
        config.local.model = "  ".to_string();
        assert_eq!(validate(&config).unwrap_err().code, "SA-5001");
    }

    #[test]
    fn migrate_from_v2_keeps_user_values() {
        let raw = serde_json::json!({
            "schemaVersion": 2,
            "ui": { "theme": "dark", "resultWindow": { "opacity": 0.9, "width": 500 } },
            "privacy": { "allowTextUpload": false },
            "advanced": { "skeletonMode": false }
        });
        let config = migrate(raw).unwrap();
        assert_eq!(config.ui.result_window.opacity, 0.9);
        assert_eq!(config.ui.result_window.width, 500.0);
        assert!(!config.advanced.skeleton_mode);
        assert!(!config.privacy.allow_text_upload);
    }

    #[test]
    fn migrate_rejects_future_version() {
        let raw = serde_json::json!({ "schemaVersion": 99 });
        assert!(migrate(raw).is_err());
    }

    #[test]
    fn hotkey_specs_are_normalized() {
        assert_eq!(normalize_hotkey("ctrl+q").as_deref(), Some("Ctrl+Q"));
        assert_eq!(
            normalize_hotkey("SHIFT+ctrl+a").as_deref(),
            Some("Ctrl+Shift+A")
        );
        assert_eq!(normalize_hotkey("alt+digit").as_deref(), None);
        assert_eq!(normalize_hotkey("q").as_deref(), None);
        assert_eq!(normalize_hotkey("Ctrl+").as_deref(), None);
        assert_eq!(normalize_hotkey("ctrl+1").as_deref(), Some("Ctrl+1"));
        assert!(is_valid_hotkey(""));
        assert!(is_valid_hotkey("Win+Shift+S"));
        assert!(!is_valid_hotkey("Ctrl++Q"));
    }

    #[test]
    fn migrate_from_v4_removes_update_channel_and_disables_skeleton() {
        let raw = serde_json::json!({
            "schemaVersion": 4,
            "advanced": { "proxy": null, "updateChannel": "preview", "telemetry": true, "skeletonMode": true }
        });
        let config = migrate(raw).unwrap();
        assert_eq!(config.schema_version, 5);
        assert!(!config.advanced.skeleton_mode);
        assert!(config.advanced.telemetry);
        assert!(!config.advanced.extra.contains_key("updateChannel"));
        let value = serde_json::to_value(&config).unwrap();
        assert!(value["advanced"].get("updateChannel").is_none());
    }

    #[test]
    fn provider_kind_serializes_and_defaults_to_openai_compatible() {
        let config = Config::default();
        assert_eq!(config.providers[0].kind, ProviderKind::OpenAiCompatible);
        assert_eq!(config.providers[1].kind, ProviderKind::QwenMt);
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["providers"][0]["kind"], "openAiCompatible");
        assert_eq!(value["providers"][1]["kind"], "qwenMt");
        // 旧配置缺 kind → 默认 OpenAI 兼容（前向兼容）
        let raw = serde_json::json!({
            "schemaVersion": 5,
            "providers": [{ "id": "deepseek", "name": "DeepSeek", "baseUrl": "https://api.deepseek.com", "model": "deepseek-flash" }]
        });
        let config = migrate(raw).unwrap();
        assert_eq!(config.providers[0].kind, ProviderKind::OpenAiCompatible);
        // 非法 kind 被 serde 拒绝
        let mut json = serde_json::to_value(Config::default()).unwrap();
        json["providers"][0]["kind"] = serde_json::json!("anthropic");
        assert!(serde_json::from_value::<Config>(json).is_err());
    }
}
