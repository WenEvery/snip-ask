/**
 * 浏览器预览模式的静态演示数据（不连接任何真实服务）。
 * 文案保持自然，贴近真实结果，便于在没有安装桌面版时预览界面。
 */

import type { ModelInfo, SourceRef } from "../contract/types";

export const SKELETON_PREFIX = "【预览】";

/** 与契约 §12 完全一致的占位识别文本 */
export const SKELETON_OCR_PREVIEW = "这是一段示例识别文字，用于预览框选后的操作界面。";

/** 固定占位回答（分 8 块流式推送；总长 ≈ 320 字 ≈ 40 字/块） */
export const SKELETON_ANSWER = [
  "这是浏览器预览里的示例回答，用来说明结果窗的显示效果。",
  "安装桌面版并配置好模型服务后，框选文字点「解答」，这里会显示模型生成的完整回答，",
  "并随生成过程实时输出。回答下方会给出所用模型、耗时与费用，方便判断是否值得继续追问。",
  "你可以在设置里调整主题、语言与结果窗外观，也可以配置自己的模型服务。",
  "预览模式不会访问网络、不会读取你的配置，也不会写入任何真实数据。",
].join("");

/** 追问回答（固定占位） */
export const SKELETON_FOLLOWUP_ANSWER =
  "这是追问的示例回答。桌面版会结合上一轮的问答内容继续作答，方便在同一个话题上深入追问。";

/** 图片搜索的 2 条占位来源（契约 §12：图片搜索附 2 条 sources） */
export const SKELETON_SOURCES: SourceRef[] = [
  {
    title: "示例来源 · 百科条目",
    url: "https://example.com/preview/wiki",
    siteName: "example.com",
    snippet: "这里是检索结果的摘要示例，桌面版会显示真实网页的标题、站点与摘要。",
  },
  {
    title: "示例来源 · 官方文档",
    url: "https://example.com/preview/docs",
    siteName: "example.com",
    snippet: "点击来源可在浏览器中打开原网页；这里仅用于预览来源列表的排版。",
  },
];

/**
 * 与仓库 `resources/models.manifest.json` 对应的占位模型列表（均未安装）。
 * 静态字段（名称/大小/许可/链接/参数）与清单逐字段一致，仅补运行时字段：
 * `installed=false`、`installedBytes=0`、`downloadState="idle"`（未下载）。
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
      downloadState: "idle",
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
      downloadState: "idle",
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
      downloadState: "idle",
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
      downloadState: "idle",
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
      downloadState: "idle",
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
      downloadState: "idle",
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
      downloadState: "idle",
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
