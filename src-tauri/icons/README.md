# 图标资源（`src-tauri/icons/`）

本目录存放 Tauri 打包与运行所需的图标，**由脚本生成，请勿手工修改**。

| 文件 | 尺寸 / 内容 | 用途 |
| --- | --- | --- |
| `32x32.png` | 32×32 RGBA PNG | 窗口图标、托盘小图标 |
| `128x128.png` | 128×128 RGBA PNG | 任务栏 / 窗口图标 |
| `128x128@2x.png` | 256×256 RGBA PNG | 高 DPI 窗口图标 |
| `icon.png` | 512×512 RGBA PNG | 应用图标、安装包图标源 |
| `icon.ico` | 16/24/32/48/64/128/256 多尺寸 ICO | Windows 可执行文件与安装包图标 |

## 重新生成

在仓库根目录执行（Windows PowerShell）：

```powershell
python -m pip install pillow   # 仅首次需要
python scripts/gen-icons.py
```

脚本会用 Pillow 以 4 倍超采样绘制图形后 LANCZOS 缩小，保证圆角与线条平滑；
生成结束后自动回读文件校验格式、尺寸与颜色模式（PNG 需为 RGBA，ICO 需包含
上表中的全部尺寸），校验失败时以非零码退出。

## 设计说明

- 圆角矩形渐变底：左上深蓝 `#1B3A8B` → 右下靛蓝 `#4F46E5`（45° 对角渐变）；
- 白色「选框四角」+ 中心四角星芒：对应产品「框选 → 星芒即 AI」的核心交互；
- 图形留出约 19% 的安全边距，缩小到 32×32 时仍可辨认；
- 颜色与 `docs/接口契约.md` §14 的强调色体系保持同一色相，避免与主题冲突。

## 维护约定

- 修改图标请改 `scripts/gen-icons.py` 中的参数（`COLOR_START` / `COLOR_END` /
  `CORNER_*` / `SPARK_*`），再重新生成，**不要**直接覆盖 PNG/ICO；
- 生成物需一并提交（`src-tauri/icons/**` 已在 `.gitattributes` 中标记为二进制）；
- `tauri.conf.json` 的 `bundle.icon` 引用本目录文件，增删文件时需同步修改配置。
