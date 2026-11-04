# scripts/spikes/s08 — S-08 模型下载断点续传与校验

对应 `docs/spikes/S-08-模型下载断点续传与校验.md`。原始数据写入 `.verify/spikes/S-08/`（gitignored）。

## 脚本

| 文件 | 用途 |
| --- | --- |
| `probe_range.ps1` | 对 ModelScope 真实模型文件 URL 发 `Range: bytes=0-1023`（只取 1 KB，不下载整模型），验证 206 + Content-Range；同时探测清单 URL（预期 404）、中段 Range、越界 416、HuggingFace（预期超时） |

### 复跑

```powershell
# 真实镜像 Range 探测（约 6~10s；原始响应头/CSV/JSON/Markdown 汇总写入 .verify/spikes/S-08/）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s08/probe_range.ps1

# 不需要 HuggingFace 超时探测时可加 -SkipHf
```

> **编码注意**：`probe_range.ps1` 含中文注释，必须以 UTF-8 **带 BOM** 保存。
> Windows PowerShell 5.1 对无 BOM 的 UTF-8 按 ANSI/GBK 解码，会报
> `Missing ')' in method call` / `Expressions are only allowed as the first element of a pipeline`。
> 编辑后如丢失 BOM，用下面命令补回（PowerShell 7 `pwsh` 无此限制）：
>
> ```powershell
> $p = "scripts/spikes/s08/probe_range.ps1"
> $t = [IO.File]::ReadAllText($p, [Text.UTF8Encoding]::new($false))
> [IO.File]::WriteAllText($p, $t, [Text.UTF8Encoding]::new($true))
> ```

## 相关测试与复跑命令

```powershell
# 集成测试（本地 std::net::TcpListener HTTP server，12 passed / 2 ignored）
cargo test --manifest-path src-tauri/Cargo.toml --test model_download_spike -- --nocapture --test-threads=1

# 真实网络（ModelScope，1 KB；需联网）
cargo test --manifest-path src-tauri/Cargo.toml --test model_download_spike real_modelscope -- --ignored --nocapture
```

实现与结论见 `docs/spikes/S-08-模型下载断点续传与校验.md`。
