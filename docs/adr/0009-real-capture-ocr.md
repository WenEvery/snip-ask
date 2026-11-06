# ADR-0009：真实捕获（Win32 BitBlt）与真实本地 OCR（RapidOCR PP-OCRv4 / ONNX Runtime）

| 项目 | 内容 |
| --- | --- |
| 状态 | 已接受（Accepted） |
| 日期 | 2026-11-06 |
| 关联需求 | v1.0.0 用户决策「真实捕获 + 真实本地 OCR」；替换 v0.1 的 `TODO(real)` 桩 |
| 关联文档 | [接口契约](../接口契约.md) §6（命令 1–5 / 40）/ §10 / §12；[实施架构](../实施架构.md) §4.2 / §5 / §9；[自定义指南](../自定义指南.md) §8 |
| 证据 | `docs/spikes/S-09-截图失败降级与WGC对比.md`（WGC 对比 / 黑屏降级）、`S-02-S-03-OCR时延基准.md`（像素预算 / rec-only 口径）、`S-11-文字存在判定阈值.md`、`S-15-OCR字符准确率.md`；v1.0.0 实现测试 `.verify/v1/capture/`、`.verify/v1/ocr/` |

## 背景

v0.1.0 的捕获与 OCR 都是 trait 桩：`platform::capture` 只合成占位帧，`ocr_service` 只给占位框。
v1.0.0 要把「框选 → 识别 → 动作菜单」变成真实链路，必须先定两件事：

1. **捕获后端**：Windows 上主流是 `BitBlt`（GDI）与 WGC（`Windows.Graphics.Capture`）。项目内 spike
   （S-09）已实测：普通窗口下两者**像素级一致**（窗口项帧 SHA256 相同）；但 DRM / `WDA_MONITOR` 保护面
   两者都只能得到纯黑，WGC **不能绕过**保护；`WDA_EXCLUDEFROMCAPTURE` 下窗口从截屏消失、WGC 窗口项
   5s 内取不到帧。WGC 还引入 COM/D3D11 初始化与更复杂的生命周期。
2. **OCR 引擎**：需中文为主、能给出字符框（覆盖翻译要用）、可离线、可随包分发。候选包括
   Windows 自带 OCR、Tesseract、PaddleOCR/RapidOCR（ONNX）。S-15 用 PP-OCRv4 实测字符准确率
   99.38%（native）/ 99.29%（生产像素预算口径），100/100 张 ≥95%。

## 决策

### 捕获

1. **以 Win32 BitBlt 作为唯一主捕获路径**：`GetDC(NULL)` + `CreateDIBSection`（32bpp 顶朝下）+
   `BitBlt(SRCCOPY | CAPTUREBLT)` 捕获**虚拟桌面物理像素外接矩形**（允许负原点），产出：
   - 原始 BGRA（供 OCR，避免二次编码）；
   - PNG data URL（供 overlay 冻结层显示）。
2. **坐标与 DPI 口径不变**（契约 §1）：帧 = 所有显示器物理外接矩形，混合 DPI **不缩放**；捕获前
   best-effort 设置进程 `PER_MONITOR_AWARE_V2`；帧局部坐标 + 帧原点（可负）= 绝对物理坐标。
3. **WGC 不作主路径**；保留「黑帧 / 空帧 → 降级 → `SA-1001`」链：
   - 黑屏判定按**目标区域**（`>99% 像素亮度 <16 且均值 <16 且标准差 <6`），而不是整帧——
     S-09 实测混合 DPI 双屏下若只有主屏被保护，整帧黑占比仅 ~76%，整帧判定会漏检；
   - `selection_commit` 对选区切片判定；冻结时按显示器切片做诊断日志；
   - 失败返回 `SA-1001`（retryable，`fallback = ["wgc","retry"]`），overlay 以可操作文案展示，不崩溃。
4. **真实捕获的唯一 `unsafe` 出口**收敛到 `platform::capture` 的 Win32 子模块；判定算法保持纯 safe 纯函数可单测。

### OCR

5. **引擎固定为 RapidOCR PP-OCRv4（ONNX）+ ONNX Runtime**：det / rec / cls 三个 ONNX 模型 + rec 字典；
   通过 `ort` crate **`load-dynamic`** 加载，启动时把解析到的 `onnxruntime.dll` 路径写入
   **`ORT_DYLIB_PATH`**（必须在创建任何 session 之前）。
6. **两段式流水线**（S-02/S-03 结论）：
   - 整屏只做 **det**，输入受**双约束像素预算**（长边 ≤ `ocr.detMaxEdge`(3200) 且 宽×高 ≤ 3.0 Mpx，
     绝不放大）；后台单 worker 串行，不阻塞选框；
   - 选区优先用**整屏 det 缓存 → 按选区过滤 + rec-only**（P95 596ms ≤ 800ms）；缓存缺失/无命中才回退
     选区 det+rec（原口径 P95 968ms，保留为对照）；缓存按 `ocr_session_id` 失效。
7. **文字存在判定**用 S-11 推荐阈值组合：`det 置信度 ≥ ocr.textThreshold(0.6)` + 单行框面积 ≥ 400px² +
   单行框高 ≤ 70px + 行数 ≥ 1（仅置信度基线误判 38%，组合后误判 2.0% / 漏判 0%）。
8. **资产与就绪状态**：模型/运行时解析顺序见 [ADR-0007](0007-dual-installer-editions.md)；
   新增 `ocr_status` 命令与 `Stats.ocrReady` 暴露「运行时 + 全部模型」是否就绪；缺失时给出可操作提示，
   而不是在第一次 OCR 时才报错。
9. **骨架模式保留**：`advanced.skeletonMode = true` 时仍是确定性占位引擎（契约 §12 时序不变），
   用于联调/排查；v1.0.0 默认 `false`。

## 理由

- **BitBlt 够用且简单**：S-09 已证明与 WGC 像素级一致；DRM 场景两者都无解（WGC 不绕过保护），
  引入 WGC 只增加 COM/D3D 复杂度与失败面，不带来覆盖收益。黑帧/空帧检测 + 降级才是实际收益点。
- **PP-OCRv4 质量达标**：字符准确率 99.29–99.38%（100/100 张 ≥95%），且 det 框可直接供覆盖翻译使用。
- **离线可用**：ONNX Runtime + 模型全本地运行，不依赖网络/云 OCR；与「隐私优先、无服务器」一致。
- **性能可控**：像素预算把整屏 det 从单 4K 1682ms 降到 794ms（双 4K 743ms）；rec-only 让选区 OCR
  596ms 达标；后台 det 不阻塞交互。
- **可解释失败**：DRM/黑屏 → `SA-1001` + 降级链；模型缺失 → `ocr_status.missing` + 可操作提示。

## 备选方案与未采用原因

| 备选 | 未采用原因 |
| --- | --- |
| WGC 作为主捕获路径 | S-09 实测不能绕过 DRM；COM/D3D11 生命周期复杂、增加集成成本；与 BitBlt 像素一致，无质量收益 |
| 复用 Windows 自带 OCR（`Windows.Media.Ocr`） | 中文质量/版面与 PP-OCRv4 有差距；无法保证字符框与覆盖翻译需求；跨 Win10 版本能力不一致 |
| Tesseract | 中文准确率与版面弱于 PP-OCRv4；需要额外语言数据；S-15 口径下无优势 |
| 云 OCR（百度/腾讯等） | 与「本地优先、隐私、无服务器」冲突，且引入 Key/计费/网络失败面 |
| 仅做整屏 det+rec、不缓存 | 选区 P95 968ms 超 800ms 门槛（S-03）；缓存 + rec-only 是实测达标方案 |
| 引入 PP-Structure（表格/公式） | 超出 v1.0.0 范围；表格/公式识别列为后续能力（S-15 遗留） |

## 后果

- **正面**：捕获/OCR 成为真实能力，动作菜单的「有文字/无文字」判定、覆盖翻译框、历史记录都有真实输入；
  OCR 质量与性能有实测依据；DRM 场景有明确错误与提示。
- **代价 / 风险**：
  - 捕获是 `unsafe` Win32 代码，必须严格限定在 `platform::capture` 并靠集成测试（真实桌面）覆盖；
  - OCR 运行时 + 模型约 35.5 MB，直接导致**双安装包**决策（[ADR-0007](0007-dual-installer-editions.md)）；
  - 黑屏检测对「部分保护」场景需按区域切片，整帧判定会漏检（已在实现中按选区/显示器切片处理）；
  - **静态表格/公式/多列阅读顺序**仍不理想（S-15 遗留）；代码行号列易漏识；
  - 首次 OCR 需加载模型，冷启动耗时另计（模型资产就绪后由后台预热）。
- **后续**：如需更高版面还原，评估 PP-Structure / 阅读顺序模型；如遇 BitBlt 特殊全屏独占问题，
  再评估 WGC 作为**可选**后端（而非替换主路径）。

## 参考

- [接口契约](../接口契约.md) §6（命令 1–5、40）/ §10（资源）/ §12（骨架模式时序）
- [实施架构](../实施架构.md) §4.2（捕获/OCR 模块）、§5（线程模型）、§9（实现边界）
- [技术验证清单](../技术验证清单.md) S-02 / S-03 / S-09 / S-11 / S-15
- `src-tauri/src/platform/capture.rs`、`src-tauri/src/services/ocr_service.rs`
