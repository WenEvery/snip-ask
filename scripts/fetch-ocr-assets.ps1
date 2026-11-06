#Requires -Version 5.1
<#
.SYNOPSIS
    框选 AI · OCR 运行期资产准备脚本（v1.0.0，T5）。

.DESCRIPTION
    把安装包/运行期需要的 OCR 资产准备到 `src-tauri/resources/ocr/`（整目录 .gitignore）：

      onnxruntime.dll                         ONNX Runtime 1.31.1（ort crate load-dynamic 加载）
      ch_PP-OCRv4_det_infer.onnx              PP-OCRv4 文本检测
      ch_PP-OCRv4_rec_infer.onnx              PP-OCRv4 文本识别
      ch_ppocr_mobile_v2.0_cls_infer.onnx     方向分类
      ppocr_keys_v1.txt                       rec 字典（6,623 行，UTF-8）

    来源优先级（逐个尝试，成功即停）：
      1) 本机开发缓存：`.verify/venv-ocr/**`（RapidOCR / onnxruntime pip 包；本机已存在）；
      2) `src-tauri/resources/models.manifest.json` 中 4 条 OCR 条目（T1 维护）声明的镜像 URL；
      3) 脚本内置官方镜像（PaddleOCR bcebos tar / PyPI onnxruntime wheel / NuGet nupkg）。
    字典 `ppocr_keys_v1.txt` 另有离线路径：从已就位的 rec ONNX 的 metadata `character` 字段提取
    （RapidOCR 1.4.x 把字典内嵌进模型 metadata；提取并补末尾换行后与 PaddleOCR 官方文件**字节一致**）。

    每个文件都按内置真实 SHA256 + 字节数强校验；不匹配立即失败（退出码 1）。
    幂等：目标已存在且校验通过 → 跳过；`-Force` 强制重取；`-VerifyOnly` 只校验不下载。

.PARAMETER Destination
    资产输出目录，默认 `src-tauri/resources/ocr`。

.PARAMETER VerifyOnly
    只校验 Destination 现有资产，缺失/不匹配 → 退出码 1；不写任何文件（除证据 JSON）。

.PARAMETER NoDownload
    只用本机缓存（含 metadata 提取），不联网；缺失即失败。用于离线构建。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1 -VerifyOnly
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\fetch-ocr-assets.ps1 -Force -NoDownload

.NOTES
    退出码：0 = 全部资产就位且校验通过；1 = 资产缺失/校验失败/下载失败；3 = 环境不满足（缺目录等）。
#>
[CmdletBinding()]
param(
    [string]$RepoRoot = '',
    [string]$Destination = '',
    [string]$ManifestPath = '',
    [string]$EvidencePath = '',
    [switch]$Force,
    [switch]$VerifyOnly,
    [switch]$NoDownload,
    [switch]$ForceDownload
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# ---------------------------------------------------------------------------
# 资产表：真实大小 / SHA256 均来自本机 `.verify/venv-ocr`（RapidOCR 1.4.4 + onnxruntime 1.31.1），
# 并经与官方文件比对；改动这里必须同步 T1 的 resources/models.manifest.json。
# ---------------------------------------------------------------------------
$Assets = @(
    [pscustomobject]@{
        Name         = 'onnxruntime.dll'
        Description  = 'ONNX Runtime 1.31.1 x64（load-dynamic 运行时）'
        Size         = 19288888
        Sha256       = '8482d2ee174e28504f99730fb01d6861575870283de4ed3a897ad39170d9a9b9'
        LocalSources = @('.verify\venv-ocr\Lib\site-packages\onnxruntime\capi\onnxruntime.dll')
        Downloads    = @(
            [pscustomobject]@{ Kind = 'pypi-wheel'; Package = 'onnxruntime'; Version = '1.31.1'; Match = 'win_amd64\.whl$'; Inner = 'onnxruntime.dll' },
            [pscustomobject]@{ Kind = 'url'; Url = 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.31.1/microsoft.ml.onnxruntime.1.31.1.nupkg'; Inner = 'onnxruntime.dll' }
        )
        FromRecOnnx  = $false
    }
    [pscustomobject]@{
        Name         = 'ch_PP-OCRv4_det_infer.onnx'
        Description  = 'PP-OCRv4 文本检测模型'
        Size         = 4745517
        Sha256       = 'd2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9'
        LocalSources = @('.verify\venv-ocr\Lib\site-packages\rapidocr_onnxruntime\models\ch_PP-OCRv4_det_infer.onnx')
        Downloads    = @(
            [pscustomobject]@{ Kind = 'pypi-wheel'; Package = 'rapidocr_onnxruntime'; Version = '1.4.4'; Match = '\.whl$'; Inner = 'ch_PP-OCRv4_det_infer.onnx' }
        )
        FromRecOnnx  = $false
    }
    [pscustomobject]@{
        Name         = 'ch_PP-OCRv4_rec_infer.onnx'
        Description  = 'PP-OCRv4 文本识别模型'
        Size         = 10857958
        Sha256       = '48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b'
        LocalSources = @('.verify\venv-ocr\Lib\site-packages\rapidocr_onnxruntime\models\ch_PP-OCRv4_rec_infer.onnx')
        Downloads    = @(
            [pscustomobject]@{ Kind = 'pypi-wheel'; Package = 'rapidocr_onnxruntime'; Version = '1.4.4'; Match = '\.whl$'; Inner = 'ch_PP-OCRv4_rec_infer.onnx' }
        )
        FromRecOnnx  = $false
    }
    [pscustomobject]@{
        Name         = 'ch_ppocr_mobile_v2.0_cls_infer.onnx'
        Description  = 'PP-OCR 方向分类模型'
        Size         = 585532
        Sha256       = 'e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c'
        LocalSources = @('.verify\venv-ocr\Lib\site-packages\rapidocr_onnxruntime\models\ch_ppocr_mobile_v2.0_cls_infer.onnx')
        Downloads    = @(
            [pscustomobject]@{ Kind = 'pypi-wheel'; Package = 'rapidocr_onnxruntime'; Version = '1.4.4'; Match = '\.whl$'; Inner = 'ch_ppocr_mobile_v2.0_cls_infer.onnx' }
        )
        FromRecOnnx  = $false
    }
    [pscustomobject]@{
        Name         = 'ppocr_keys_v1.txt'
        Description  = 'rec 字典（6,623 行，UTF-8）'
        Size         = 26250
        Sha256       = 'a1c84d9bdb9ab29043c58896224d32941783eb821629618416dcb08f12886492'
        LocalSources = @('.verify\venv-ocr\Lib\site-packages\rapidocr_onnxruntime\models\ppocr_keys_v1.txt')
        Downloads    = @(
            [pscustomobject]@{ Kind = 'url'; Url = 'https://cdn.jsdelivr.net/gh/PaddlePaddle/PaddleOCR@main/ppocr/utils/ppocr_keys_v1.txt'; Inner = 'ppocr_keys_v1.txt' },
            [pscustomobject]@{ Kind = 'url'; Url = 'https://gitee.com/paddlepaddle/PaddleOCR/raw/release/2.7/ppocr/utils/ppocr_keys_v1.txt'; Inner = 'ppocr_keys_v1.txt' }
        )
        FromRecOnnx  = $true
    }
)

# ---------------------------------------------------------------------------
# 基础工具
# ---------------------------------------------------------------------------
function Resolve-RepoRoot {
    if ($RepoRoot) { return (Resolve-Path -LiteralPath $RepoRoot).Path }
    return (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
}

function Get-FileSha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Format-Bytes([int64]$Bytes) {
    return ('{0:N0} B（{1:N2} MiB）' -f $Bytes, ($Bytes / 1MB))
}

function Write-Utf8JsonNoBom([string]$Path, $Object) {
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    $json = $Object | ConvertTo-Json -Depth 12
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding($false)))
}

function Invoke-DownloadFile([string]$Url, [string]$OutFile) {
    Write-Host ("      [下载] {0}" -f $Url) -ForegroundColor DarkGray
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing -TimeoutSec 900
    $sw.Stop()
    Write-Host ("      [下载完成] {0}（{1:N1} MiB，{2:N0} ms）" -f (Split-Path -Leaf $OutFile), ((Get-Item -LiteralPath $OutFile).Length / 1MB), $sw.ElapsedMilliseconds) -ForegroundColor DarkGray
}

function Test-ArchiveMagic([string]$Path) {
    $fs = [System.IO.File]::OpenRead($Path)
    try {
        $head = New-Object byte[] 512
        $read = $fs.Read($head, 0, 512)
        if ($read -ge 4 -and $head[0] -eq 0x50 -and $head[1] -eq 0x4B) { return $true }   # ZIP / nupkg / whl
        if ($read -ge 262) {
            $ustar = [System.Text.Encoding]::ASCII.GetString($head, 257, 5)
            if ($ustar -eq 'ustar') { return $true }                                        # TAR
        }
        return $false
    } finally {
        $fs.Dispose()
    }
}

function Expand-AssetArchive([string]$ArchivePath, [string]$DestDir, [string]$InnerFileName) {
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
    $tar = Get-Command tar -ErrorAction SilentlyContinue
    if (-not $tar) { throw '未找到 tar.exe（Windows 10 1803+ 自带 bsdtar），无法解压下载的归档' }
    & tar -xf $ArchivePath -C $DestDir 2>&1 | ForEach-Object { Write-Host ("      [tar] {0}" -f $_) -ForegroundColor DarkGray }
    if ($LASTEXITCODE -ne 0) { throw ("tar 解压失败（exit={0}）：{1}" -f $LASTEXITCODE, $ArchivePath) }
    $found = Get-ChildItem -LiteralPath $DestDir -Recurse -File -Filter $InnerFileName -ErrorAction SilentlyContinue |
        Sort-Object { $_.FullName.Length } | Select-Object -First 1
    if (-not $found) { throw ("归档内未找到 {0}：{1}" -f $InnerFileName, $ArchivePath) }
    return $found.FullName
}

# 下载结果可能是「目标文件本体」也可能是归档（tar/zip/nupkg/whl）；本函数负责识别并取出目标文件。
# 返回可用的本地文件路径（交给调用方复制到 staging 并做最终 SHA256 校验），失败返回 $null。
function Resolve-DownloadedAsset([string]$Downloaded, $Asset, [string]$TempDir, [string]$InnerHint = '') {
    if ((Get-FileSha256 $Downloaded) -eq $Asset.Sha256) { return $Downloaded }
    if (-not (Test-ArchiveMagic $Downloaded)) { return $null }
    $inner = if ($InnerHint) { $InnerHint } else { $Asset.Name }
    $extractDir = Join-Path $TempDir ('extract-' + [guid]::NewGuid().ToString('N'))
    try {
        return (Expand-AssetArchive $Downloaded $extractDir $inner)
    } catch {
        Write-Host ("      [解压/取文件失败] {0}" -f $_.Exception.Message) -ForegroundColor Yellow
        return $null
    }
}

function Get-PypiWheelUrl([string]$Package, [string]$Version, [string]$Match = 'win_amd64\.whl$') {
    $meta = Invoke-RestMethod -Uri ("https://pypi.org/pypi/{0}/{1}/json" -f $Package, $Version) -TimeoutSec 90
    $wheel = @($meta.urls | Where-Object { $_.filename -match $Match }) | Select-Object -First 1
    if (-not $wheel) { throw ("PyPI 上没有 {0} {1} 的匹配 wheel（Match={2}）" -f $Package, $Version, $Match) }
    return $wheel.url
}

# 从 rec ONNX 的 metadata 提取字典（RapidOCR 1.4.x 内嵌 character）。
function New-DictFromRecOnnx([string]$RepoRootPath, [string]$RecOnnxPath, [string]$OutPath, [string]$TempDir) {
    $py = Join-Path $RepoRootPath '.verify\venv-ocr\Scripts\python.exe'
    if (-not (Test-Path -LiteralPath $py)) {
        Write-Host '      [字典] 未找到 .verify/venv-ocr 的 python，跳过 metadata 提取' -ForegroundColor Yellow
        return $false
    }
    if (-not (Test-Path -LiteralPath $RecOnnxPath)) {
        Write-Host ("      [字典] 未找到 rec ONNX：{0}，跳过 metadata 提取" -f $RecOnnxPath) -ForegroundColor Yellow
        return $false
    }
    $pyFile = Join-Path $TempDir 'extract_dict.py'
    $pyCode = @'
import sys
try:
    import onnxruntime as ort
except Exception as exc:  # noqa: BLE001
    sys.stderr.write("onnxruntime import failed: %s\n" % exc)
    sys.exit(3)
rec_path, out_path = sys.argv[1], sys.argv[2]
session = ort.InferenceSession(rec_path, providers=["CPUExecutionProvider"])
meta = session.get_modelmeta().custom_metadata_map
text = meta.get("character")
if not text:
    sys.stderr.write("rec onnx has no 'character' metadata\n")
    sys.exit(4)
with open(out_path, "wb") as handle:
    handle.write(text.encode("utf-8"))
    handle.write(b"\n")
'@
    [System.IO.File]::WriteAllText($pyFile, $pyCode, (New-Object System.Text.UTF8Encoding($false)))
    Write-Host '      [字典] 从 rec ONNX metadata 提取 ppocr_keys_v1.txt' -ForegroundColor DarkGray
    & $py $pyFile $RecOnnxPath $OutPath 2>&1 | ForEach-Object { Write-Host ("      [dict-extract] {0}" -f $_) -ForegroundColor DarkGray }
    return ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $OutPath))
}

# 读取 T1 的 models.manifest.json，提取 4 条 OCR 条目（容错多种字段命名）。
function Get-ManifestOcrEntries([string]$Path) {
    $map = @{}
    if (-not (Test-Path -LiteralPath $Path)) { return $map }
    try {
        $manifest = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        Write-Host ("      [manifest] 解析失败，忽略：{0}" -f $_.Exception.Message) -ForegroundColor Yellow
        return $map
    }
    $arrays = @()
    if ($manifest -is [System.Array]) { $arrays += , $manifest }
    foreach ($key in @('models', 'assets', 'items', 'entries', 'ocrModels', 'resources')) {
        $prop = $manifest.PSObject.Properties[$key]
        if ($prop -and $prop.Value -is [System.Array]) { $arrays += , $prop.Value }
    }
    foreach ($arr in $arrays) {
        foreach ($entry in $arr) {
            if (-not $entry) { continue }
            $id = $null
            foreach ($name in @('id', 'assetId', 'name', 'key')) {
                $p = $entry.PSObject.Properties[$name]
                if ($p -and $p.Value) { $id = [string]$p.Value; break }
            }
            if (-not $id -or $id -notmatch '^ocr-') { continue }
            $params = $entry.PSObject.Properties['params']
            $paramObj = if ($params) { $params.Value } else { $null }
            $fileName = $null
            foreach ($name in @('fileName', 'file', 'filename')) {
                if ($paramObj) { $p = $paramObj.PSObject.Properties[$name]; if ($p -and $p.Value) { $fileName = [string]$p.Value; break } }
                $p = $entry.PSObject.Properties[$name]
                if (-not $fileName -and $p -and $p.Value) { $fileName = [string]$p.Value }
            }
            $urls = @()
            foreach ($src in @($entry, $paramObj)) {
                if (-not $src) { continue }
                foreach ($name in @('urls', 'mirrors', 'downloadUrls', 'url', 'sources')) {
                    $p = $src.PSObject.Properties[$name]
                    if ($p -and $p.Value) {
                        foreach ($u in @($p.Value)) { if ($u -is [string] -and $u -match '^https?://') { $urls += $u } }
                    }
                }
            }
            $sha = $null
            foreach ($src in @($entry, $paramObj)) {
                if (-not $src) { continue }
                foreach ($name in @('sha256', 'sha256Hex', 'checksum')) {
                    $p = $src.PSObject.Properties[$name]
                    if ($p -and $p.Value) { $sha = ([string]$p.Value).ToLowerInvariant(); break }
                }
                if ($sha) { break }
            }
            $size = $null
            foreach ($src in @($entry, $paramObj)) {
                if (-not $src) { continue }
                foreach ($name in @('size', 'sizeBytes', 'bytes')) {
                    $p = $src.PSObject.Properties[$name]
                    if ($p -and $p.Value -and [int64]$p.Value -gt 0) { $size = [int64]$p.Value; break }
                }
                if ($size) { break }
            }
            $map[$id] = [pscustomobject]@{
                Id       = $id
                FileName = $fileName
                Urls     = @($urls | Select-Object -Unique)
                Sha256   = $sha
                Size     = $size
            }
        }
    }
    return $map
}

function Find-ManifestEntryForAsset($ManifestMap, [string]$FileName) {
    foreach ($key in $ManifestMap.Keys) {
        if ($ManifestMap[$key].FileName -eq $FileName) { return $ManifestMap[$key] }
    }
    return $null
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
$root = Resolve-RepoRoot
if (-not $Destination) { $Destination = Join-Path $root 'src-tauri\resources\ocr' }
if (-not $ManifestPath) {
    # 模型清单在家目录 resources/（仓库根，T1 维护）；兼容 src-tauri/resources/ 旧位置。
    $manifestCandidates = @(
        (Join-Path $root 'resources\models.manifest.json'),
        (Join-Path $root 'src-tauri\resources\models.manifest.json')
    )
    foreach ($candidate in $manifestCandidates) {
        if (Test-Path -LiteralPath $candidate) { $ManifestPath = $candidate; break }
    }
    if (-not $ManifestPath) { $ManifestPath = $manifestCandidates[0] }
}
if (-not $EvidencePath) { $EvidencePath = Join-Path $root '.verify\v1\installer\ocr-assets.json' }

Write-Host ''
Write-Host '=== 框选 AI · OCR 资产准备 ===' -ForegroundColor Cyan
Write-Host ("仓库根目录：{0}" -f $root) -ForegroundColor DarkGray
Write-Host ("目标目录  ：{0}" -f $Destination) -ForegroundColor DarkGray
Write-Host ("模式      ：{0}" -f $(if ($VerifyOnly) { '只校验' } elseif ($NoDownload) { '仅本机缓存' } else { '本机缓存 + 下载' })) -ForegroundColor DarkGray
Write-Host ''

if (-not (Test-Path -LiteralPath $Destination)) {
    if ($VerifyOnly) {
        Write-Host ("[失败] 目标目录不存在：{0}" -f $Destination) -ForegroundColor Red
        exit 3
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
}

$manifestMap = Get-ManifestOcrEntries $ManifestPath
if ($manifestMap.Count -gt 0) {
    Write-Host ("[manifest] 已读取 {0} 条 OCR 条目：{1}" -f $manifestMap.Count, (($manifestMap.Keys | Sort-Object) -join ', ')) -ForegroundColor DarkGray
} else {
    Write-Host '[manifest] 未找到可用 OCR 条目（T1 尚未落地或格式不同），仅使用本机缓存/内置镜像' -ForegroundColor DarkGray
}

$tempDir = Join-Path $env:TEMP ('snipask-ocr-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

$report = [ordered]@{
    tool        = 'scripts/fetch-ocr-assets.ps1'
    version     = '1.0.0'
    timestamp   = (Get-Date).ToString('o')
    repoRoot    = $root
    destination = $Destination
    manifest    = $ManifestPath
    mode        = $(if ($VerifyOnly) { 'verify-only' } elseif ($NoDownload) { 'local-only' } else { 'fetch' })
    environment = [ordered]@{
        os        = (Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue).Caption
        psVersion = $PSVersionTable.PSVersion.ToString()
        tar       = [bool](Get-Command tar -ErrorAction SilentlyContinue)
        python    = (Test-Path -LiteralPath (Join-Path $root '.verify\venv-ocr\Scripts\python.exe'))
    }
    assets      = @()
    ok          = $true
    failures    = @()
}

$recOnnxInDest = Join-Path $Destination 'ch_PP-OCRv4_rec_infer.onnx'

try {
    foreach ($asset in $Assets) {
        $destPath = Join-Path $Destination $asset.Name
        Write-Host ("[{0}]" -f $asset.Name) -ForegroundColor Cyan
        Write-Host ("      {0}（期望 {1} / {2}）" -f $asset.Description, (Format-Bytes $asset.Size), $asset.Sha256.Substring(0, 16)) -ForegroundColor DarkGray

        $record = [ordered]@{
            name        = $asset.Name
            description = $asset.Description
            path        = $destPath
            expected    = [ordered]@{ size = [int64]$asset.Size; sha256 = $asset.Sha256 }
            source      = $null
            size        = $null
            sha256      = $null
            verified    = $false
            skipped     = $false
            error       = $null
        }

        # 1) 已存在 → 校验后跳过
        if ((Test-Path -LiteralPath $destPath) -and -not $Force) {
            $curSize = (Get-Item -LiteralPath $destPath).Length
            $curSha = Get-FileSha256 $destPath
            if ($curSize -eq $asset.Size -and $curSha -eq $asset.Sha256) {
                Write-Host '      [跳过] 已存在且校验通过' -ForegroundColor DarkGray
                $record.source = 'existing'
                $record.size = $curSize
                $record.sha256 = $curSha
                $record.verified = $true
                $record.skipped = $true
                $report.assets += $record
                continue
            }
            Write-Host ("      [重建] 现有文件校验不符（{0} B / {1}）" -f $curSize, $curSha.Substring(0, 16)) -ForegroundColor Yellow
        }

        if ($VerifyOnly) {
            $record.error = if (Test-Path -LiteralPath $destPath) { 'sha256/size 校验失败' } else { '文件缺失' }
            Write-Host ("      [失败] {0}" -f $record.error) -ForegroundColor Red
            $report.failures += ("{0}: {1}" -f $asset.Name, $record.error)
            $report.ok = $false
            $report.assets += $record
            continue
        }

        $staging = Join-Path $tempDir $asset.Name
        $acquired = $false
        $sourceLabel = $null

        # 2) 本机缓存（-ForceDownload 时跳过，用于验证 CI 的下载分支）
        if (-not $ForceDownload) {
            foreach ($rel in $asset.LocalSources) {
                $candidate = Join-Path $root $rel
                if (-not (Test-Path -LiteralPath $candidate)) { continue }
                $candSha = Get-FileSha256 $candidate
                $candSize = (Get-Item -LiteralPath $candidate).Length
                if ($candSize -ne $asset.Size -or $candSha -ne $asset.Sha256) {
                    Write-Host ("      [跳过本机缓存] {0} 校验不符（{1} B / {2}）" -f $candidate, $candSize, $candSha.Substring(0, 16)) -ForegroundColor Yellow
                    continue
                }
                Copy-Item -LiteralPath $candidate -Destination $staging -Force
                $acquired = $true
                $sourceLabel = 'local:' + $rel
                Write-Host ("      [本机缓存] {0}" -f $rel) -ForegroundColor DarkGray
                break
            }
        }

        # 3) 字典离线路径：从 rec ONNX metadata 提取（需 rec 已就位）
        if (-not $acquired -and -not $ForceDownload -and $asset.FromRecOnnx) {
            if (New-DictFromRecOnnx $root $recOnnxInDest $staging $tempDir) {
                $acquired = $true
                $sourceLabel = 'rec-onnx-metadata'
                Write-Host '      [metadata 提取成功]' -ForegroundColor DarkGray
            }
        }

        # 4) manifest 镜像 URL
        if (-not $acquired) {
            $mEntry = Find-ManifestEntryForAsset $manifestMap $asset.Name
            if ($mEntry -and $mEntry.Sha256 -and $mEntry.Sha256 -ne $asset.Sha256) {
                Write-Host ("      [警告] manifest 中 {0} 的 sha256 与脚本内置值不一致（manifest={1}, builtin={2}），以脚本内置值为准；请 T1 对齐" -f $mEntry.Id, $mEntry.Sha256, $asset.Sha256) -ForegroundColor Yellow
            }
            if ($mEntry -and $mEntry.Urls.Count -gt 0) {
                foreach ($url in $mEntry.Urls) {
                    if ($NoDownload) { break }
                    try {
                        $dl = Join-Path $tempDir ('dl-' + [guid]::NewGuid().ToString('N'))
                        Invoke-DownloadFile $url $dl
                        $resolved = Resolve-DownloadedAsset -Downloaded $dl -Asset $asset -TempDir $tempDir
                        if ($resolved -and (Get-FileSha256 $resolved) -eq $asset.Sha256) {
                            Copy-Item -LiteralPath $resolved -Destination $staging -Force
                            $acquired = $true
                            $sourceLabel = 'manifest-url:' + $url
                            break
                        } elseif ($resolved) {
                            Write-Host ("      [跳过] manifest 镜像内容 SHA256 不符：{0}" -f $url) -ForegroundColor Yellow
                        }
                    } catch {
                        Write-Host ("      [下载失败] {0}" -f $_.Exception.Message) -ForegroundColor Yellow
                    }
                }
            }
        }

        # 5) 内置镜像
        if (-not $acquired -and -not $NoDownload) {
            foreach ($dlSpec in $asset.Downloads) {
                try {
                    $dl = $null
                    if ($dlSpec.Kind -eq 'pypi-wheel') {
                        $match = if ($dlSpec.PSObject.Properties['Match'] -and $dlSpec.Match) { [string]$dlSpec.Match } else { 'win_amd64\.whl$' }
                        $wheelUrl = Get-PypiWheelUrl $dlSpec.Package $dlSpec.Version $match
                        $dl = Join-Path $tempDir ('wheel-' + [guid]::NewGuid().ToString('N') + '.whl')
                        Invoke-DownloadFile $wheelUrl $dl
                        $resolved = Expand-AssetArchive $dl (Join-Path $tempDir 'wheel') $dlSpec.Inner
                    } else {
                        $dl = Join-Path $tempDir ('dl-' + [guid]::NewGuid().ToString('N'))
                        Invoke-DownloadFile $dlSpec.Url $dl
                        $resolved = Resolve-DownloadedAsset -Downloaded $dl -Asset $asset -TempDir $tempDir -InnerHint $dlSpec.Inner
                    }
                    if (-not $resolved) { continue }
                    if ((Get-FileSha256 $resolved) -ne $asset.Sha256) {
                        Write-Host ("      [跳过] 镜像内容 SHA256 不符：{0}" -f $dlSpec.Url) -ForegroundColor Yellow
                        continue
                    }
                    Copy-Item -LiteralPath $resolved -Destination $staging -Force
                    $acquired = $true
                    $sourceLabel = if ($dlSpec.Kind -eq 'pypi-wheel') { 'pypi-wheel:' + $dlSpec.Package + '/' + $dlSpec.Version } else { 'builtin-url:' + $dlSpec.Url }
                    break
                } catch {
                    Write-Host ("      [内置镜像失败] {0}" -f $_.Exception.Message) -ForegroundColor Yellow
                }
            }
        }

        if (-not $acquired) {
            $record.error = if ($NoDownload) { '本机缓存缺失（-NoDownload）' } else { '所有来源均失败' }
            Write-Host ("      [失败] {0}" -f $record.error) -ForegroundColor Red
            $report.failures += ("{0}: {1}" -f $asset.Name, $record.error)
            $report.ok = $false
            $report.assets += $record
            continue
        }

        # 6) 最终校验 + 原子落位
        $stagedSize = (Get-Item -LiteralPath $staging).Length
        $stagedSha = Get-FileSha256 $staging
        $record.size = $stagedSize
        $record.sha256 = $stagedSha
        $record.source = $sourceLabel
        if ($stagedSize -ne $asset.Size -or $stagedSha -ne $asset.Sha256) {
            $record.error = ("SHA256/大小校验失败：实际 {0} B / {1}，期望 {2} B / {3}" -f $stagedSize, $stagedSha, $asset.Size, $asset.Sha256)
            Write-Host ("      [失败] {0}" -f $record.error) -ForegroundColor Red
            $report.failures += ("{0}: {1}" -f $asset.Name, $record.error)
            $report.ok = $false
            $report.assets += $record
            continue
        }
        $tmpDest = $destPath + '.tmp'
        Copy-Item -LiteralPath $staging -Destination $tmpDest -Force
        Move-Item -LiteralPath $tmpDest -Destination $destPath -Force
        $record.verified = $true
        Write-Host ("      [OK] {0}，来源 {1}" -f (Format-Bytes $stagedSize), $sourceLabel) -ForegroundColor Green
        $report.assets += $record
    }
} finally {
    if (Test-Path -LiteralPath $tempDir) { Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue }
}

$report.finishedAt = (Get-Date).ToString('o')
$report.totalBytes = (@($report.assets | ForEach-Object { if ($_.size) { [int64]$_.size } else { 0 } }) | Measure-Object -Sum).Sum
Write-Utf8JsonNoBom -Path $EvidencePath -Object $report

Write-Host ''
if ($report.ok) {
    Write-Host ("[完成] 5 个 OCR 资产已就位：{0}（合计 {1}）" -f $Destination, (Format-Bytes ([int64]$report.totalBytes))) -ForegroundColor Green
    Write-Host ("       证据：{0}" -f $EvidencePath) -ForegroundColor DarkGray
    exit 0
}

Write-Host ('[失败] {0} 个资产未就位：' -f $report.failures.Count) -ForegroundColor Red
foreach ($f in $report.failures) { Write-Host ("       - {0}" -f $f) -ForegroundColor Red }
Write-Host ("       证据：{0}" -f $EvidencePath) -ForegroundColor DarkGray
exit 1
