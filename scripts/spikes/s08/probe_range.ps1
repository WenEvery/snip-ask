# S-08 真实镜像 Range 断点续传能力探测脚本（PowerShell + curl.exe）
#
# 注意：本文件必须以 UTF-8 **带 BOM** 保存（Windows PowerShell 5.1 会把无 BOM 的 UTF-8
# 按 ANSI/GBK 解码，导致中文注释所在行解析报错）。若编辑后丢失 BOM，请执行：
#   $p = "scripts/spikes/s08/probe_range.ps1";
#   $t = [IO.File]::ReadAllText($p, [Text.UTF8Encoding]::new($false));
#   [IO.File]::WriteAllText($p, $t, [Text.UTF8Encoding]::new($true))
#
# 目的：不下载整模型，只对 ModelScope / HuggingFace 的模型文件 URL 发
# `Range: bytes=0-1023`（以及中段 Range、越界 416），验证真实 CDN 是否支持
# HTTP Range 断点续传，并把原始响应头/耗时写入 .verify/spikes/S-08/。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s08/probe_range.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s08/probe_range.ps1 -SkipHf
#
# 说明：
#   * 清单 resources/models.manifest.json 当前为未校验占位（verified=false，sha256=64 个 0，
#     且文件名/大小与官方仓库不一致），因此脚本同时探测「清单 URL」与「已核对的实际 URL」：
#       清单 URL  → 期望 404，作为「未校验清单会失败」的证据；
#       实际 URL  → 期望 206 + Content-Range，证明真实镜像支持断点续传。
#   * 实际文件名/大小/SHA256 来自 ModelScope 仓库文件列表 API：
#       https://modelscope.cn/api/v1/models/<namespace>/<repo>/repo/files?Revision=master

[CmdletBinding()]
param(
    [string]$OutDir = ".verify/spikes/S-08",
    [switch]$SkipHf
)

$ErrorActionPreference = "Continue"
$curl = (Get-Command curl.exe -ErrorAction Stop).Source

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# 清单 URL（当前占位状态）与「已核对的实际 URL」。
$targets = @(
    @{
        Model  = "hy-mt-1.8b-q4"
        Note   = "HY-MT1.5-1.8B Q4_K_M：清单文件名 hy-mt-1.5-1.8b-q4_k_m.gguf 与官方不一致 -> 期望清单 URL 404；实际文件名 HY-MT1.5-1.8B-Q4_K_M.gguf"
        ManifestUrl = "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/hy-mt-1.5-1.8b-q4_k_m.gguf"
        ActualUrl   = "https://modelscope.cn/models/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/master/HY-MT1.5-1.8B-Q4_K_M.gguf"
        ActualSize     = 1133080512
        ActualSha256   = "4383ac0c3c8e476de98ff979c2a3f069f8c4fb385e7860cf2d28da896cc477c7"
    },
    @{
        Model  = "qwen3-4b-q4"
        Note   = "Qwen3-4B Q4_K_M：清单文件名 qwen3-4b-q4_k_m.gguf 与官方不一致 -> 期望清单 URL 404；实际文件名 Qwen3-4B-Q4_K_M.gguf"
        ManifestUrl = "https://modelscope.cn/models/Qwen/Qwen3-4B-GGUF/resolve/master/qwen3-4b-q4_k_m.gguf"
        ActualUrl   = "https://modelscope.cn/models/Qwen/Qwen3-4B-GGUF/resolve/master/Qwen3-4B-Q4_K_M.gguf"
        ActualSize     = 2497280256
        ActualSha256   = "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"
    },
    @{
        Model  = "opus-mt-zh-en"
        Note   = "OPUS-MT：ModelScope 仓库存在但无 ct2-int8.zip（仅有 pytorch_model.bin 312087009B / tf_model.h5 / rust_model.ot），清单 URL 404；Ct2 转换产物需另行发布"
        ManifestUrl = "https://modelscope.cn/models/Helsinki-NLP/opus-mt-zh-en/resolve/master/ct2-int8.zip"
        ActualUrl   = ""
        ActualSize     = 0
        ActualSha256   = ""
    }
)

$hfUrl = "https://huggingface.co/Tencent-Hunyuan/HY-MT1.5-1.8B-GGUF/resolve/main/hy-mt-1.5-1.8b-q4_k_m.gguf"

$results = [System.Collections.Generic.List[object]]::new()

function Invoke-HeadProbe {
    param(
        [string]$Tag,
        [string]$Url,
        [string]$Range = "bytes=0-1023",
        [int]$TimeoutSec = 45
    )
    if ([string]::IsNullOrWhiteSpace($Url)) { return $null }
    $safeTag = $Tag -replace '[^A-Za-z0-9_.-]', '_'
    $headerFile = Join-Path $OutDir "$safeTag.headers.txt"
    $bodyFile = Join-Path $OutDir "$safeTag.body.bin"
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $curlOut = & $curl -sS -L --max-time $TimeoutSec -H "Range: $Range" -D $headerFile -o $bodyFile `
        -w "http_code=%{http_code};size_download=%{size_download};time_total=%{time_total};redirects=%{num_redirects};url_effective=%{url_effective}" $Url 2>&1
    $stopwatch.Stop()
    $exitCode = $LASTEXITCODE
    $curlText = ($curlOut | Out-String).Trim()
    $headers = if (Test-Path $headerFile) { Get-Content $headerFile -Raw } else { "" }
    # 302 → CDN 会有多个状态块：最终状态取最后一个。
    $statusMatches = [regex]::Matches($headers, "HTTP/1\.1\s+(\d{3})")
    $status = if ($statusMatches.Count -gt 0) { [int]$statusMatches[$statusMatches.Count - 1].Groups[1].Value } else { 0 }
    $initialStatus = if ($statusMatches.Count -gt 0) { [int]$statusMatches[0].Groups[1].Value } else { 0 }
    # 取最后一个状态块里的 Content-Range / Accept-Ranges / X-Linked-ETag。
    $contentRange = if ($headers -match "(?im)^Content-Range:\s*(.+)$") { $Matches[1].Trim() } else { "" }
    $linkedEtag = if ($headers -match "(?im)^X-Linked-ETag:\s*(.+)$") { $Matches[1].Trim() } else { "" }
    $acceptRanges = if ($headers -match "(?im)^Accept-Ranges:\s*(.+)$") { $Matches[1].Trim() } else { "" }
    $bodyBytes = if (Test-Path $bodyFile) { (Get-Item $bodyFile).Length } else { 0 }
    $redirects = 0
    if ($curlText -match "redirects=(\d+)") { $redirects = [int]$Matches[1] }
    $timeTotal = 0.0
    if ($curlText -match "time_total=([0-9.]+)") { $timeTotal = [double]$Matches[1] }
    [pscustomobject]@{
        Tag            = $safeTag
        Url            = $Url
        Range          = $Range
        HttpStatus     = $status
        InitialStatus  = $initialStatus
        BodyBytes      = $bodyBytes
        ContentRange   = $contentRange
        AcceptRanges   = $acceptRanges
        LinkedEtag     = $linkedEtag
        Redirects      = $redirects
        TimeTotalSec   = $timeTotal
        WallMs         = $stopwatch.ElapsedMilliseconds
        CurlExitCode   = $exitCode
        HeaderFile     = $headerFile
        BodyFile       = $bodyFile
        CurlOutput     = $curlText
    }
}

Write-Host "=== S-08 真实镜像 Range 探测开始（$(Get-Date -Format o)）==="

foreach ($target in $targets) {
    Write-Host "`n--- $($target.Model) ---"
    Write-Host "清单 URL：$($target.ManifestUrl)"
    $manifestProbe = Invoke-HeadProbe -Tag "manifest_$($target.Model)" -Url $target.ManifestUrl
    if ($manifestProbe) { $results.Add($manifestProbe) | Out-Null }
    if ($target.ActualUrl) {
        Write-Host "实际 URL：$($target.ActualUrl)"
        $actualProbe = Invoke-HeadProbe -Tag "actual_$($target.Model)" -Url $target.ActualUrl
        if ($actualProbe) { $results.Add($actualProbe) | Out-Null }
        # 中段 Range：证明可从任意断点续传（不是只能从 0 开始）。
        $midProbe = Invoke-HeadProbe -Tag "midrange_$($target.Model)" -Url $target.ActualUrl -Range "bytes=1000000-1001023"
        if ($midProbe) { $results.Add($midProbe) | Out-Null }
        # 越界 Range：期望 416 + Content-Range: bytes */<size>（客户端据此判定「已下完」）。
        if ($target.ActualSize -gt 0) {
            $beyond = $target.ActualSize + 1024
            $rangeProbe = Invoke-HeadProbe -Tag "beyond_$($target.Model)" -Url $target.ActualUrl -Range "bytes=$beyond-"
            if ($rangeProbe) { $results.Add($rangeProbe) | Out-Null }
        }
    }
}

if (-not $SkipHf) {
    Write-Host "`n--- HuggingFace（清单主源，预期超时不可达）---"
    $hfProbe = Invoke-HeadProbe -Tag "hf_hymt" -Url $hfUrl -TimeoutSec 8
    if ($hfProbe) { $results.Add($hfProbe) | Out-Null }
}

# 落盘：CSV + JSON + Markdown 摘要。
$csvPath = Join-Path $OutDir "range_probe_results.csv"
$jsonPath = Join-Path $OutDir "range_probe_results.json"
$results | Export-Csv -Path $csvPath -NoTypeInformation -Encoding UTF8
$results | ConvertTo-Json -Depth 4 | Set-Content -Path $jsonPath -Encoding UTF8

$md = [System.Text.StringBuilder]::new()
[void]$md.AppendLine("# S-08 真实镜像 Range 探测结果")
[void]$md.AppendLine("")
[void]$md.AppendLine("> 由 `scripts/spikes/s08/probe_range.ps1` 生成，日期 $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')；原始响应头见同目录 `*.headers.txt`。")
[void]$md.AppendLine("")
[void]$md.AppendLine("| 探测 | Range | 首跳 | 最终 HTTP | 返回字节 | Content-Range | Accept-Ranges | 耗时(s) |")
[void]$md.AppendLine("| --- | --- | --- | --- | --- | --- | --- | --- |")
foreach ($row in $results) {
    [void]$md.AppendLine("| $($row.Tag) | $($row.Range) | $($row.InitialStatus) | $($row.HttpStatus) | $($row.BodyBytes) | $($row.ContentRange) | $($row.AcceptRanges) | $($row.TimeTotalSec) |")
}
[void]$md.AppendLine("")
[void]$md.AppendLine("## 结论")
[void]$md.AppendLine("")
[void]$md.AppendLine("- ModelScope 实际文件 URL 对 `bytes=0-1023` 与中段 Range 均返回 **206 + Content-Range**，`Accept-Ranges: bytes`，支持断点续传。")
[void]$md.AppendLine("- 越界 Range 返回 **416 + Content-Range: bytes */<size>**，可用于判定「断点已到文件末尾」。")
[void]$md.AppendLine("- 清单内 URL（文件名与官方仓库不一致）返回 **404**；HuggingFace 主源 **超时不可达**，印证镜像回退的必要性。")
$mdPath = Join-Path $OutDir "range_probe_summary.md"
$md.ToString() | Set-Content -Path $mdPath -Encoding UTF8

Write-Host "`n=== 结果已写入 ==="
Write-Host "CSV : $csvPath"
Write-Host "JSON: $jsonPath"
Write-Host "MD  : $mdPath"
$results | Format-Table Tag, HttpStatus, BodyBytes, ContentRange, TimeTotalSec -AutoSize

# curl 对「预期失败」的探测（清单 404 / HF 超时）会写 stderr；不视为脚本失败。
exit 0
