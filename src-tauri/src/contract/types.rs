//! 跨边界基础类型（契约 §3 / §5 / §6 补充结构 / §9 / §10）。
//!
//! 约定（契约 §15）：
//! - 结构体一律 `#[serde(rename_all = "camelCase")]`；
//! - `Option` 字段序列化为 `null`，不使用 `skip_serializing_if`；
//! - 枚举按契约字面量序列化（`Mode` 用 `lowercase`）；
//! - ID 均为 UUID v4 小写字符串，用 `#[serde(transparent)]` newtype 表达。
//!
//! 本模块不依赖任何内部模块（可依赖 `config` 中的枚举，二者同属契约层）。

use std::fmt;

use serde::{Deserialize, Serialize};

use super::config::{
    Cap, FontSize, GpuBackend, Language, LocalBackend, ResultPosition, Theme, TranslateBackend,
};
use super::errors::AppError;

/// 会话 ID（UUID v4 小写字符串）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

/// 任务 ID（UUID v4 小写字符串）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub String);

/// 下载 ID（UUID v4 小写字符串）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DownloadId(pub String);

macro_rules! impl_id_common {
    ($ty:ty) => {
        impl $ty {
            /// 取出内部字符串。
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 是否为空 ID。
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $ty {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $ty {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

impl_id_common!(SessionId);
impl_id_common!(TaskId);
impl_id_common!(DownloadId);

/// 引擎 ID（供应商 / 本地引擎 / `skeleton` 等）。
pub type EngineId = String;

/// AI 模式：仅快速 / 深度两档（契约 §1.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 快速：关闭思考、短输出。
    #[default]
    Quick,
    /// 深度：开启思考、多步检索、长输出。
    Deep,
}

impl Mode {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Quick => "quick",
            Mode::Deep => "deep",
        }
    }

    /// 从契约字面量解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "quick" => Some(Mode::Quick),
            "deep" => Some(Mode::Deep),
            _ => None,
        }
    }

    /// 两档模式互换（overlay 内快捷键切换）。
    pub fn toggle(&self) -> Self {
        match self {
            Mode::Quick => Mode::Deep,
            Mode::Deep => Mode::Quick,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 动作 ID（契约 §3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionId {
    /// 文本搜索。
    Search,
    /// 图片搜索。
    ImageSearch,
    /// 翻译。
    Translate,
    /// 解答。
    Answer,
    /// 复制文字。
    Copy,
}

impl ActionId {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionId::Search => "search",
            ActionId::ImageSearch => "imageSearch",
            ActionId::Translate => "translate",
            ActionId::Answer => "answer",
            ActionId::Copy => "copy",
        }
    }

    /// 从契约字面量解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "search" => Some(ActionId::Search),
            "imageSearch" => Some(ActionId::ImageSearch),
            "translate" => Some(ActionId::Translate),
            "answer" => Some(ActionId::Answer),
            "copy" => Some(ActionId::Copy),
            _ => None,
        }
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 动作适用的目标类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionTarget {
    /// 仅文本。
    Text,
    /// 仅图像。
    Image,
    /// 文本与图像皆可。
    Both,
}

/// 物理像素矩形，原点为虚拟桌面左上角（可为负）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectPhys {
    /// 左上角 x（物理像素，可为负）。
    pub x: f64,
    /// 左上角 y（物理像素，可为负）。
    pub y: f64,
    /// 宽度（物理像素，可能为负表示反向拖拽，需 `normalize`）。
    pub width: f64,
    /// 高度（物理像素，可能为负表示反向拖拽，需 `normalize`）。
    pub height: f64,
}

/// 逻辑（CSS）像素矩形。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectLogical {
    /// 左上角 x（CSS 像素）。
    pub x: f64,
    /// 左上角 y（CSS 像素）。
    pub y: f64,
    /// 宽度（CSS 像素）。
    pub width: f64,
    /// 高度（CSS 像素）。
    pub height: f64,
}

/// 虚拟桌面信息：物理矩形 + 缩放比。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDesktop {
    /// 虚拟桌面物理矩形（多显示器外接矩形）。
    pub rect: RectPhys,
    /// 设备像素比。
    pub dpr: f64,
}

/// OCR 文本行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBox {
    /// 识别文本。
    pub text: String,
    /// 置信度 0..=1。
    pub confidence: f64,
    /// 文本行物理矩形（契约 v4 起 JSON 键名为 `bbox`，与 `CoverBlock.bbox` 一致）。
    pub bbox: RectPhys,
}

/// 冻结帧元信息（骨架模式下 `dataUrl` 为 `null`）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenFrameMeta {
    /// 帧宽（物理像素）。
    pub width: u32,
    /// 帧高（物理像素）。
    pub height: u32,
    /// DataURL；骨架模式为 `null`。
    pub data_url: Option<String>,
}

/// `selection_commit` 返回的选区识别结果（契约 §5）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionResult {
    /// 所属会话。
    pub session: SessionId,
    /// 提交的选区（已规范化为正宽高）。
    pub rect_phys: RectPhys,
    /// 选区内是否存在文字。
    pub has_text: bool,
    /// 选区预览文本。
    pub preview: String,
    /// 选区命中的 OCR 文本行。
    pub boxes: Vec<OcrBox>,
}

/// 动作菜单项（`menu:ready` 载荷元素）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    /// 动作 ID。
    pub id: ActionId,
    /// 展示文案。
    pub label: String,
    /// 作用目标。
    pub target: ActionTarget,
    /// 当前模式。
    pub mode: Mode,
    /// 是否可用。
    pub enabled: bool,
    /// 不可用原因；可用时为 `null`。
    pub disabled_reason: Option<String>,
    /// 绑定的快捷键（规范化后）；未绑定为 `null`。
    pub shortcut: Option<String>,
}

/// `menu_execute` / `task_followup` 的可选参数。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunActionOptions {
    /// 用户编辑后的文本（缺省取选区预览文本）。
    pub text: Option<String>,
    /// 目标语言覆盖（翻译用）。
    pub target_lang: Option<String>,
    /// 覆盖式翻译（研究 §2：翻译覆盖原句）；`translate_cover` 命令使用。
    pub cover: Option<bool>,
}

/// 覆盖翻译：前端把 OCR 行合并后的段块发给后端（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverBlock {
    /// 前端生成的稳定 id（`"b0"`、`"b1"`…）。
    pub id: String,
    /// 段块原文（多行以 `\n` 连接）。
    pub text: String,
    /// 段块外接矩形（物理像素）。
    pub bbox: RectPhys,
    /// OCR 置信度（缺省 0）。
    #[serde(default)]
    pub confidence: f64,
}

/// 单块覆盖结果（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoveredBlock {
    /// 对应 [`CoverBlock::id`]。
    pub id: String,
    /// 原文。
    pub source: String,
    /// 译文；失败时与原文相同。
    pub translation: String,
    /// 该块翻译是否失败（失败时前端保留原文并标红）。
    pub failed: bool,
    /// 实际使用的引擎（`skeleton` / 供应商 id / `local`）。
    pub engine: String,
}

/// 覆盖翻译整体结果（`translate_cover` 返回 / `translate:ready` 载荷）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverResult {
    /// 会话 ID。
    pub session: SessionId,
    /// 逐块结果（顺序与请求一致）。
    pub blocks: Vec<CoveredBlock>,
    /// 实际使用的引擎。
    pub engine: String,
    /// 端到端耗时（毫秒）。
    pub latency_ms: u64,
    /// 请求块总数。
    pub total_blocks: u32,
    /// 成功翻译块数。
    pub translated_blocks: u32,
}

/// 流式增量类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    /// 正文增量。
    Text,
    /// 思考过程增量。
    Reasoning,
    /// 元信息增量。
    Meta,
}

/// 结算货币（当前仅人民币）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Currency {
    /// 人民币。
    #[default]
    #[serde(rename = "CNY")]
    Cny,
}

impl Currency {
    /// 契约字面量。
    pub fn as_str(&self) -> &'static str {
        match self {
            Currency::Cny => "CNY",
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Token 用量与费用估算（契约 §5）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    /// 输入 token 数。
    pub input_tokens: u64,
    /// 输出 token 数。
    pub output_tokens: u64,
    /// 命中缓存的输入 token 数。
    pub cached_input_tokens: u64,
    /// 估算费用（人民币）。
    pub cost_estimate: f64,
    /// 结算货币，固定 `CNY`。
    pub currency: Currency,
}

/// 结果来源引用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    /// 标题。
    pub title: String,
    /// 链接。
    pub url: String,
    /// 站点名。
    pub site_name: Option<String>,
    /// 摘要片段。
    pub snippet: Option<String>,
}

/// `task:started` 事件载荷（脚手架扩展事件，契约 §7）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStartedPayload {
    /// 任务 ID。
    pub task: TaskId,
    /// 会话 ID。
    pub session: SessionId,
    /// 动作。
    pub action: ActionId,
    /// 模式。
    pub mode: Mode,
    /// 实际使用的引擎。
    pub engine: EngineId,
    /// 父任务：追问/重答时指向上一轮任务；首轮为 `null`（契约 v4）。
    #[serde(default)]
    pub parent: Option<TaskId>,
}

/// `task:done` 事件载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDonePayload {
    /// 任务 ID。
    pub task: TaskId,
    /// Token 用量与费用。
    pub usage: TokenUsage,
    /// 端到端耗时（毫秒）。
    pub latency_ms: u64,
    /// 实际使用的引擎。
    pub engine: EngineId,
    /// 来源列表（无来源为空数组）。
    pub sources: Vec<SourceRef>,
}

/// `provider_test` 入参（契约 §6 补充结构）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraft {
    /// 供应商 id，仅 `[a-z0-9-_]{1,32}`。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// API Base URL。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// 能力列表。
    pub caps: Vec<Cap>,
    /// 明文 API Key（仅命令入参，绝不落盘 / 不出现在出参）。
    #[serde(default)]
    pub api_key: Option<String>,
    /// 温度。
    pub temperature: Option<f64>,
    /// 最大输出 token。
    pub max_tokens: Option<u32>,
}

/// `provider_test` 返回。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    /// 是否连通。
    pub ok: bool,
    /// 延迟（毫秒）。
    pub latency_ms: Option<u64>,
    /// 命中的模型名。
    pub model: Option<String>,
    /// HTTP 状态码（骨架模式为 200 占位）。
    pub status: Option<u16>,
    /// 失败时的错误。
    pub error: Option<AppError>,
}

/// 模型下载状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    /// 排队中。
    #[default]
    Queued,
    /// 下载中。
    Downloading,
    /// 校验中。
    Verifying,
    /// 已安装。
    Installed,
    /// 失败。
    Failed,
    /// 已取消。
    Cancelled,
}

/// 模型文件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    /// GGUF（llama.cpp）。
    Gguf,
    /// CTranslate2。
    Ct2,
    /// ONNX（OCR）。
    Onnx,
}

/// 模型运行时可合并状态（静态清单 + 本地安装状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// 模型 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 文件类型。
    pub kind: ModelKind,
    /// 文件大小（字节）。
    pub size_bytes: u64,
    /// SHA256。
    pub sha256: String,
    /// 许可标识。
    pub license: String,
    /// 建议内存（MB）。
    #[serde(rename = "minRamMB")]
    pub min_ram_mb: u32,
    /// 推理时显存预算（MB，契约 v4；用于 GPU 是否放得下的判定）。
    #[serde(rename = "vramMB")]
    pub vram_mb: u32,
    /// 下载地址（主源 + 镜像）。
    pub urls: Vec<String>,
    /// 扩展参数（量化、上下文等）。
    pub params: serde_json::Value,
    /// 是否已安装。
    pub installed: bool,
    /// 已安装占用字节数。
    pub installed_bytes: u64,
    /// 下载状态。
    pub download_state: DownloadState,
}

/// 模型清单文件（`resources/models.manifest.json`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelManifest {
    /// 清单版本。
    pub version: u32,
    /// 是否已签名校验（脚手架阶段恒 false）。
    pub verified: bool,
    /// 模型条目。
    pub models: Vec<ModelManifestEntry>,
}

/// 模型清单条目（对应 [`ModelInfo`] 的静态部分）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelManifestEntry {
    /// 模型 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 文件类型。
    pub kind: Option<ModelKind>,
    /// 文件大小（字节）。
    pub size_bytes: u64,
    /// SHA256。
    pub sha256: String,
    /// 许可标识。
    pub license: String,
    /// 建议内存（MB）。
    #[serde(rename = "minRamMB")]
    pub min_ram_mb: u32,
    /// 推理时显存预算（MB，契约 v4）。
    #[serde(rename = "vramMB")]
    pub vram_mb: u32,
    /// 下载地址。
    pub urls: Vec<String>,
    /// 扩展参数。
    pub params: serde_json::Value,
}

/// `hotkey_probe` 返回。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// 原始输入。
    pub spec: String,
    /// 是否合法。
    pub valid: bool,
    /// 是否可注册（合法且无冲突）。
    pub available: bool,
    /// 规范化后的快捷键；非法为 `null`。
    pub normalized: Option<String>,
    /// 冲突的配置项名称；无冲突为 `null`。
    pub conflict_with: Option<String>,
}

/// 本地推理进程状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EngineStatus {
    /// 未运行。
    #[default]
    Stopped,
    /// 启动中。
    Starting,
    /// 就绪。
    Ready,
    /// 失败。
    Failed,
}

/// 本地推理后端组件（`backends_list` 元素，契约 v4）。
///
/// 主安装包只带 CPU 后端；**Vulkan 后端作为默认 GPU 组件按需下载**（跨 AMD/Intel/NVIDIA），
/// CUDA 后端作为 NVIDIA 用户的可选组件按需下载。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    /// 组件 id：`"vulkan"` / `"cuda"` / `"cpu"`。
    pub id: String,
    /// 展示名。
    pub label: String,
    /// 对应后端枚举。
    pub backend: LocalBackend,
    /// llama.cpp 版本（如 `2.28.2`）。
    pub version: String,
    /// 压缩包字节数。
    pub size_bytes: u64,
    /// SHA256（校验下载完整性）。
    pub sha256: String,
    /// 许可标识（如 `MIT` / `NVIDIA CUDA EULA`）。
    pub license: String,
    /// 下载地址（主源 + 镜像）。
    pub urls: Vec<String>,
    /// 需要的显卡厂商（`"nvidia"` 表示仅 NVIDIA 可用；Vulkan/CPU 为 `null`）。
    pub requires_vendor: Option<String>,
    /// 是否已安装。
    pub installed: bool,
    /// 已安装占用字节数。
    pub installed_bytes: u64,
    /// 下载状态。
    pub download_state: DownloadState,
}

/// GPU 设备（来自 `llama-server --list-devices` 或 DXGI/WMI 兜底，契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    /// 后端内设备序号（`Vulkan0` → 0）。
    pub index: u32,
    /// 设备名（如 `AMD Radeon RX 7800 XT`）。
    pub name: String,
    /// 该设备所属后端（Vulkan / CUDA）。
    pub backend: LocalBackend,
    /// 显存总量（MiB）。
    pub vram_total_mb: u64,
    /// 当前空闲显存（MiB）；未知为 0。
    pub vram_free_mb: u64,
    /// 厂商：`nvidia` / `amd` / `intel` / `unknown`。
    pub vendor: Option<String>,
    /// 是否虚拟/软件适配器（`Microsoft Basic Render Driver` 等，不计入可用）。
    pub virtual_adapter: bool,
}

/// 单个 GPU 后端的可用性（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuBackendStatus {
    /// 后端（含 CPU 项）。
    pub backend: LocalBackend,
    /// 组件（`llama-server.exe`）是否已就位。
    pub installed: bool,
    /// `installed` 且 `--list-devices` 能列出非虚拟设备。
    pub available: bool,
    /// 可执行文件路径。
    pub exe_path: Option<String>,
    /// 版本（`--version` 首行解析）。
    pub version: Option<String>,
    /// 不可用原因（面向用户的中文短句）。
    pub reason: Option<String>,
}

/// 显存预算结论（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuBudget {
    /// 需求显存（`model.vramMB + 512`）。
    pub required_mb: u64,
    /// 目标设备当前空闲显存。
    pub available_mb: u64,
    /// 是否放得下。
    pub fits: bool,
}

/// `local_gpu_probe` 返回（契约 v4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProbeResult {
    /// 探测时间（RFC3339）。
    pub checked_at: String,
    /// 是否命中 60s 缓存。
    pub cached: bool,
    /// 是否存在可用 GPU 后端。
    pub useable: bool,
    /// 推荐后端（Vulkan 优先）。
    pub recommended: Option<GpuBackend>,
    /// 发现的 GPU 设备。
    pub devices: Vec<GpuDevice>,
    /// 三个后端的状态（vulkan / cuda / cpu，顺序固定）。
    pub backends: Vec<GpuBackendStatus>,
    /// 显存预算（有可用设备 + 已知模型时给出）。
    pub budget: Option<GpuBudget>,
    /// 整体不可用原因。
    pub issue: Option<String>,
}

/// `app_stats` 返回（契约 §6 补充结构）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// 应用版本（CARGO_PKG_VERSION）。
    pub version: String,
    /// IPC 契约版本。
    pub contract_version: u32,
    /// 运行时长（秒）。
    pub uptime_sec: u64,
    /// 内存占用（MB）；骨架模式返回 0。
    pub memory_mb: u64,
    /// 当前活动会话。
    pub active_session: Option<SessionId>,
    /// 活动任务数。
    pub active_tasks: u32,
    /// 本地引擎状态。
    pub local_engine: EngineStatus,
    /// 已安装模型数。
    pub models_installed: u32,
    /// 是否骨架模式。
    pub skeleton_mode: bool,
}

/// 首启引导状态（契约 §9）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OnboardingState {
    /// 是否完成引导。
    pub completed: bool,
    /// 是否跳过引导。
    pub skipped: bool,
    /// 引导契约版本（当前 1）。
    pub version: u32,
    /// 当前步骤 0..=5；完成后保留最后位置。
    pub current_step: i32,
    /// 开始时间（ISO8601）。
    pub started_at: String,
    /// 更新时间（ISO8601）。
    pub updated_at: String,
    /// 草稿数据。
    pub data: OnboardingDraft,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self {
            completed: false,
            skipped: false,
            version: 1,
            current_step: 0,
            started_at: String::new(),
            updated_at: String::new(),
            data: OnboardingDraft::default(),
        }
    }
}

/// 引导草稿（契约 §9）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OnboardingDraft {
    /// 界面语言。
    pub language: Language,
    /// AI 服务配置草稿。
    pub ai: Option<AiDraft>,
    /// 翻译方式草稿。
    pub translate: Option<TranslateDraft>,
    /// 各动作默认模式（部分字段）。
    pub modes: std::collections::BTreeMap<String, Mode>,
    /// 外观与行为草稿。
    pub ui: UiDraft,
    /// 热键草稿。
    pub hotkeys: HotkeysDraft,
    /// 隐私草稿。
    pub privacy: PrivacyDraft,
    /// 完成时间（ISO8601）。
    pub finished_at: Option<String>,
}

/// 引导第 1 步：AI 服务草稿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AiDraft {
    /// 供应商 id。
    pub provider_id: String,
    /// Base URL。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// 是否已通过连接测试。
    pub tested: bool,
}

/// 引导第 2 步：翻译方式草稿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslateDraft {
    /// 云端 / 本地。
    pub backend: TranslateBackend,
    /// 本地模型 id。
    pub model_id: Option<String>,
    /// 云端模型名。
    pub cloud_model: String,
}

/// 引导第 4 步：外观与行为草稿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UiDraft {
    /// 主题。
    pub theme: Theme,
    /// 字号。
    pub font_size: FontSize,
    /// 开机自启。
    pub autostart: bool,
    /// 结果窗位置。
    pub result_position: ResultPosition,
}

/// 引导第 4 步：热键草稿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HotkeysDraft {
    /// 框选热键。
    pub capture: String,
}

impl Default for HotkeysDraft {
    fn default() -> Self {
        Self {
            capture: "Ctrl+Q".to_string(),
        }
    }
}

/// 引导第 5 步：隐私草稿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PrivacyDraft {
    /// 是否同意隐私条款。
    pub agreed: bool,
    /// 允许文本上传。
    pub allow_text_upload: bool,
    /// 允许图像上传。
    pub allow_image_upload: bool,
}

/// 窗口标签（契约 §2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowLabel {
    /// 主窗口：设置页 + 首启引导。
    Main,
    /// 覆盖层：冻结层 + 选框 + 动作菜单。
    Overlay,
    /// 结果窗。
    Result,
}

impl WindowLabel {
    /// 契约字面量（也是 Tauri 窗口 label）。
    pub fn as_str(&self) -> &'static str {
        match self {
            WindowLabel::Main => "main",
            WindowLabel::Overlay => "overlay",
            WindowLabel::Result => "result",
        }
    }
}

impl fmt::Display for WindowLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
