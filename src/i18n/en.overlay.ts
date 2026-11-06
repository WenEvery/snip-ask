/**
 * 框选层（overlay）/ 结果窗（result）英文词典（中文原文 → English）。
 *
 * key 必须是源码里出现的**精确中文串**；带变量用 `{name}` 占位（见 `t()`）。
 * 后端下发的动态文案（菜单动作 label / disabledReason、错误 message 等）不在此静态词典内，
 * 由调用方原样展示（见 task-6 报告的「未覆盖清单」）。
 *
 * 注意：key 必须保持**引号包裹**（中文是合法标识符，Prettier 默认会去掉引号，
 * 导致 `scripts/check-i18n.mjs` 的 key 正则扫描不到）；因此整个对象禁用 Prettier 重排。
 */
// prettier-ignore
export const EN_OVERLAY: Record<string, string> = {
  /* ───────── 框选层：动作菜单 ───────── */
  "动作菜单": "Action menu",
  "识别到的内容": "Recognized content",
  "回答模式": "Answer mode",
  "动作": "Actions",
  "快速": "Quick",
  "深入": "Deep",
  "深度": "Deep",
  "当前不可用": "Not available",
  "{label}：{reason}": "{label}: {reason}",
  "覆盖翻译": "Cover translation",
  "原位显示译文": "Translate in place",
  "没有文字": "No text",
  "把译文直接盖在原句位置（只在这个窗口里显示，不会改动原应用）":
    "Place the translation directly over the original sentence (shown only in this window; the original app is untouched)",
  "选区内没有可翻译的文字": "No translatable text in the selection",
  "重新框选": "Select again",
  "取消": "Cancel",

  /* ───────── 框选层：选框 / 放大镜 / 状态 ───────── */
  "宽 {width} × 高 {height} px（物理）": "W {width} × H {height} px (physical)",
  "没有读取到屏幕画面": "Screen not captured",
  "未识别到文本": "No text recognized",
  "选区内没有文字框": "No text boxes in the selection",
  "已读取屏幕画面": "Screen captured",
  "没能读取屏幕画面": "Screen not captured",
  "已读取屏幕画面，框选、放大镜和覆盖翻译都基于这张画面":
    "Screen captured — selection, magnifier and cover translation all use this frame",
  "没能读取屏幕画面（可能被系统保护），覆盖翻译只能显示译文":
    "Couldn't capture the screen (it may be protected by the system); cover translation can only show the translation",
  "识别中…": "Recognizing…",
  "正在识别选区…": "Recognizing the selection…",
  "正在执行动作…": "Running the action…",
  "拖拽选择区域 · Esc 取消 · Enter 确认 · 方向键微调 1px（Shift 10px）":
    "Drag to select · Esc to cancel · Enter to confirm · Arrow keys nudge by 1 px (Shift: 10 px)",

  /* ───────── 框选层：等待页 / 错误条 ───────── */
  "框选 AI": "Snip Ask",
  "按下框选热键（默认 Ctrl+Q），或右键托盘图标选择「框选」即可开始。":
    "Press the selection hotkey (Ctrl+Q by default), or right-click the tray icon and choose “Select” to start.",
  "预览模式：点击下方按钮模拟一次框选。": "Preview mode: click the button below to simulate a selection.",
  "等待开始框选…": "Waiting to start a selection…",
  "模拟触发框选": "Simulate a selection",
  "识别与翻译都在本机进行，只有你主动使用云端功能时才会上传。":
    "Recognition and translation run locally; data is uploaded only when you use a cloud feature.",
  "预览模式，不会访问真实屏幕与网络。": "Preview mode — the real screen and network are not accessed.",
  "关闭": "Close",

  /* ───────── 覆盖翻译（框选层） ───────── */
  "覆盖翻译层": "Cover translation layer",
  "覆盖翻译结果：{total} 段，当前 {view} 模式": "Cover translation: {total} blocks, {view} view",
  "覆盖": "Cover",
  "双语": "Bilingual",
  "原文": "Original",
  "没能读取到原来的画面（可能被系统保护），无法遮住原文；这里只显示译文。":
    "Couldn't capture the original screen (it may be protected by the system), so the original text can't be hidden; only the translation is shown here.",
  "覆盖翻译工具条": "Cover translation toolbar",
  "翻译中 {done}/{total}": "Translating {done}/{total}",
  "已翻译 {count} 段": "Translated {count} blocks",
  "显示模式": "Display mode",
  "只看原文（O）": "Show the original only (O)",
  "原文 + 译文对照（B）": "Original with translation (B)",
  "把译文直接盖在原文位置（C）": "Cover the original with the translation (C)",
  "本窗口画面 {width}×{height}（已读取）。覆盖翻译只改这里的显示，不会改动原应用。":
    "Window frame {width}×{height} (captured). Cover translation only changes what this window shows; the original app is untouched.",
  "本窗口画面 {width}×{height}（未读取）。覆盖翻译只改这里的显示，不会改动原应用。":
    "Window frame {width}×{height} (not captured). Cover translation only changes what this window shows; the original app is untouched.",
  "只在本窗口显示译文，不会改动原应用":
    "Shows the translation only in this window; the original app is untouched",
  "重译": "Retranslate",
  "重新截屏": "Recapture",

  /* ───────── 结果窗：顶栏 / 标题 / 空态 ───────── */
  "结果": "Result",
  "文本搜索": "Text search",
  "图片搜索": "Image search",
  "翻译": "Translate",
  "解答": "Answer",
  "复制文字": "Copy text",
  "第 {round} 轮": "Round {round}",
  "内容由 AI 生成，请自行核实": "AI-generated content; please verify",
  "AI 生成": "AI-generated",
  "预览": "Preview",
  "复制全文": "Copy all",
  "已复制": "Copied",
  "复制": "Copy",
  "等待框选结果…": "Waiting for a selection result…",
  "框选屏幕上的内容并选择一个动作，回答会显示在这里。":
    "Select something on screen and choose an action; the answer will appear here.",
  "检索来源（{count}）": "Sources ({count})",

  /* ───────── 结果窗：线程气泡 / 错误 ───────── */
  "问": "Q",
  "答": "A",
  "问：": "Q: ",
  "答：": "A: ",
  "（无内容）": "(No content)",
  "失败": "Failed",
  "已取消": "Cancelled",
  "可重试": "Retryable",
  "可以试试：{name}": "Try: {name}",
  "思考过程（{count} 字）": "Reasoning ({count} chars)",
  "已取消本次生成。": "Generation cancelled.",
  "用时 {value}": "Took {value}",
  "费用 {value}": "Cost {value}",
  "复制失败：剪贴板不可用，请手动选择文本复制。":
    "Copy failed: the clipboard is unavailable. Please select the text and copy it manually.",
  "操作失败，请稍后重试。": "Something went wrong. Please try again later.",
  "、": ", ",

  /* ───────── 结果窗：降级动作名 ───────── */
  "重试": "Retry",
  "改用浏览器搜索": "Search the web instead",
  "改用本地翻译": "Translate locally instead",
  "改用云端翻译": "Translate in the cloud instead",
  "打开设置": "Open settings",
  "改用图片识别": "Use image recognition instead",
  "下载本地模型": "Download the local model",
  "换用轻量模型": "Switch to a lightweight model",
  "更换热键": "Change the hotkey",
  "暂停热键": "Pause hotkeys",
  "切换镜像源": "Switch the mirror",
  "手动下载": "Download manually",
  "重新下载": "Download again",
  "恢复该分组默认值": "Restore group defaults",

  /* ───────── 结果窗：底部追问栏 ───────── */
  "正在生成回答…（结束后可继续追问）":
    "Generating an answer… (you can ask a follow-up when it finishes)",
  "继续追问…（Enter 发送，Shift+Enter 换行）":
    "Ask a follow-up… (Enter to send, Shift+Enter for a new line)",
  "取消生成": "Cancel generation",
  "重答": "Regenerate",
  "结束这次对话，回到等待框选的状态":
    "End this conversation and return to waiting for a selection",
  "结束对话": "End conversation",
  "发送中…": "Sending…",
  "发送": "Send",

  /* ───────── 后端动作禁用原因（ActionItem.disabledReason，src-tauri/src/domain/action.rs） ───────── */
  "未配置 API Key：请到设置 → AI 服务填写，或下载本地模型":
    "No API key configured: open Settings → AI Services to add one, or download a local model",
  "没有可用联网搜索能力：请到设置 → AI 服务为供应商勾选「联网搜索」，或改用浏览器搜索":
    "No provider has web search enabled: in Settings → AI Services, tick “Web search” for a provider, or use browser search instead",
  "没有可用的视觉能力：请到设置 → AI 服务为供应商勾选「视觉（看图）」":
    "No provider has vision enabled: in Settings → AI Services, tick “Vision (image)” for a provider",
  "没有可用的云端翻译能力：请到设置 → AI 服务为供应商勾选「翻译」，或改用本地翻译":
    "No provider has cloud translation enabled: in Settings → AI Services, tick “Translation” for a provider, or use local translation instead",
  "本地模型未下载：请到设置 → 模型库下载":
    "Local model not downloaded: open Settings → Model library to download it",
  "本地 OCR 不可用：请重新安装应用或到设置 → 高级检查":
    "Local OCR is unavailable: reinstall the app or check Settings → Advanced",
  "已开启仅本地模式，AI 搜索不可用": "Local-only mode is on, so AI search is unavailable",
  "已关闭文本上传，AI 搜索不可用": "Text upload is off, so AI search is unavailable",
  "已开启仅本地模式，图片搜索不可用": "Local-only mode is on, so image search is unavailable",
  "已关闭图像上传，图片搜索不可用": "Image upload is off, so image search is unavailable",
  "已开启仅本地模式，云端翻译不可用":
    "Local-only mode is on, so cloud translation is unavailable",
  "已关闭文本上传，云端翻译不可用": "Text upload is off, so cloud translation is unavailable",
  "已开启仅本地模式，云端解答不可用": "Local-only mode is on, so cloud answers are unavailable",
  "已关闭文本上传，解答不可用": "Text upload is off, so answers are unavailable",
  "未识别到文本，无法复制": "No text recognized; nothing to copy",

  /* ───────── 后端菜单 label（按中文原文兜底映射，未命中则原样显示） ───────── */
  "搜索": "Search",
  "AI 搜索": "Search",
};
