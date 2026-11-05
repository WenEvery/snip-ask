/**
 * mock 后端的静态演示数据（契约 §12 骨架模式）。
 * 所有文案统一前缀「【骨架模式】」，保证浏览器预览与真实后端行为可区分。
 */

import type { ModelInfo, SourceRef } from "../contract/types";

export const SKELETON_PREFIX = "【骨架模式】";

/** 与契约 §12 完全一致的占位识别文本 */
export const SKELETON_OCR_PREVIEW = "【骨架模式】本地 OCR 未接入，此为占位识别文本。";

/** 固定占位回答（分 8 块流式推送；总长 ≈ 320 字 ≈ 40 字/块） */
export const SKELETON_ANSWER = [
  "这是【骨架模式】的固定占位回答：当前脚手架尚未接入真实大模型、OCR 与网络搜索，",
  "所以你看到的内容是确定性生成的演示文本。它用于验证「框选 → 动作菜单 → 流式输出 → 结果窗」",
  "的完整链路：任务先发出 task:started，再以 25–40ms 的间隔分块推送 task:delta，",
  "最后用 task:done 携带 usage 与真实耗时收尾。接入真实能力时，只需替换服务层实现，",
  "事件流与前端交互保持不变。你可以在设置页切换主题与强调色、配置供应商密钥（仅内存模拟）、",
  "测试模型下载进度，或重新运行首启引导；这些操作的命令与事件同样遵循 IPC 契约 v1。",
  "若要体验真实后端，请在具备 Rust 工具链的环境中运行 pnpm dev（本机当前无 Rust 环境）。",
  "骨架模式不会访问网络、不读取密钥、不写入真实配置，可安全用于 UI 联调与手动验收。",
].join("");

/** 追问回答（固定占位） */
export const SKELETON_FOLLOWUP_ANSWER =
  "【骨架模式】这是追问的占位回答：当前会话未接入真实模型，追问链路仅用于验证上下文与流式渲染。接入真实模型后，这里会结合上一轮结果继续作答。";

/** 图片搜索的 2 条占位来源（契约 §12：图片搜索附 2 条 sources） */
export const SKELETON_SOURCES: SourceRef[] = [
  {
    title: "【骨架模式】示例来源 · 维基百科",
    url: "https://example.com/skeleton/wiki",
    siteName: "example.com",
    snippet: "【骨架模式】占位摘要：此处将展示真实检索结果的标题、站点与摘要。",
  },
  {
    title: "【骨架模式】示例来源 · 官方文档",
    url: "https://example.com/skeleton/docs",
    siteName: "example.com",
    snippet: "【骨架模式】占位摘要：来源列表仅用于验证结果窗渲染与跳转逻辑。",
  },
];

/**
 * 与仓库 `resources/models.manifest.json` 对应的占位模型列表（均未安装）。
 * 静态字段（名称/大小/许可/链接/参数）与清单逐字段一致，仅补运行时字段：
 * `installed=false`、`installedBytes=0`、`downloadState="queued"`。
 */
export function createPlaceholderModels(): ModelInfo[] {
  return [
    {
      id: "hy-mt-1.8b-q4",
      name: "HY-MT1.5-1.8B（Q4_K_M）",
      kind: "gguf",
      sizeBytes: 1_258_291_200,
      sha256: "0".repeat(64),
      license: "Tencent Hunyuan Community License",
      minRamMB: 4096,
      vramMB: 1600,
      urls: [
        "https://huggingface.co/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/main/hy-mt-1.5-1.8b-q4_k_m.gguf",
        "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/hy-mt-1.5-1.8b-q4_k_m.gguf",
      ],
      params: { quant: "Q4_K_M", context: 8192, languages: 38, engine: "llama.cpp" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    {
      id: "qwen3-4b-q4",
      name: "Qwen3-4B（Q4_K_M）",
      kind: "gguf",
      sizeBytes: 2_684_354_560,
      sha256: "0".repeat(64),
      license: "Apache-2.0",
      minRamMB: 8192,
      vramMB: 3200,
      urls: [
        "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/qwen3-4b-q4_k_m.gguf",
        "https://modelscope.cn/models/Qwen/Qwen3-4B-GGUF/resolve/master/qwen3-4b-q4_k_m.gguf",
      ],
      params: { quant: "Q4_K_M", context: 32768, languages: 119, engine: "llama.cpp" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    {
      id: "opus-mt-zh-en",
      name: "OPUS-MT 中英翻译（CTranslate2）",
      kind: "ct2",
      sizeBytes: 314_572_800,
      sha256: "0".repeat(64),
      license: "CC-BY-4.0",
      minRamMB: 2048,
      vramMB: 0,
      urls: [
        "https://huggingface.co/Helsinki-NLP/opus-mt-zh-en/resolve/main/ct2-int8.zip",
        "https://modelscope.cn/models/Helsinki-NLP/opus-mt-zh-en/resolve/master/ct2-int8.zip",
      ],
      params: { format: "ctranslate2", direction: "zh-en", quant: "int8", beamSize: 4 },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    // ── 契约 v5：OCR 本地模型（RapidOCR PP-OCRv4 ONNX；由 T1 的 manifest 对齐） ──
    {
      id: "ocr-det",
      name: "OCR 文本检测（PP-OCRv4 det）",
      kind: "onnx",
      sizeBytes: 4_745_517,
      sha256: "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
      license: "Apache-2.0",
      minRamMB: 0,
      vramMB: 0,
      urls: [],
      params: { role: "ocr", fileName: "ch_PP-OCRv4_det_infer.onnx" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    {
      id: "ocr-rec",
      name: "OCR 文字识别（PP-OCRv4 rec）",
      kind: "onnx",
      sizeBytes: 10_857_958,
      sha256: "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b",
      license: "Apache-2.0",
      minRamMB: 0,
      vramMB: 0,
      urls: [],
      params: { role: "ocr", fileName: "ch_PP-OCRv4_rec_infer.onnx" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    {
      id: "ocr-cls",
      name: "OCR 方向分类（PP-OCRv4 cls）",
      kind: "onnx",
      sizeBytes: 585_532,
      sha256: "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c",
      license: "Apache-2.0",
      minRamMB: 0,
      vramMB: 0,
      urls: [],
      params: { role: "ocr", fileName: "ch_ppocr_mobile_v2.0_cls_infer.onnx" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
    {
      id: "ocr-dict",
      name: "OCR 字典（ppocr_keys_v1）",
      kind: "onnx",
      sizeBytes: 26_250,
      sha256: "a1c84d9bdb9ab29043c58896224d32941783eb821629618416dc08f12886492",
      license: "Apache-2.0",
      minRamMB: 0,
      vramMB: 0,
      urls: [],
      params: { role: "ocr", fileName: "ppocr_keys_v1.txt" },
      installed: false,
      installedBytes: 0,
      downloadState: "queued",
    },
  ];
}

/** 按块数把文本切成近似等长的块（末块可能略短） */
export function splitChunks(text: string, chunkCount: number): string[] {
  if (text.length === 0) return [""];
  const size = Math.max(1, Math.ceil(text.length / chunkCount));
  const chunks: string[] = [];
  for (let start = 0; start < text.length; start += size) {
    chunks.push(text.slice(start, start + size));
  }
  return chunks;
}
