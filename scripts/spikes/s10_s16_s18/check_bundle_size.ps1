# S-18 安装包体积门禁（CI 与本地共用）
#
# 检查 target/release/bundle/nsis 下 NSIS 安装包体积是否 ≤ 阈值（默认 25 MiB，不含模型），
# 并输出体积基线 JSON（供 CI 产物归档与趋势对比）。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/check_bundle_size.ps1
#   ... -ThresholdMiB 25 -BaselineOut .verify/ci/volume_baseline.json -BaselineRef docs/spikes/data/s16_s18/volume_baseline.json
#
# 退出码：0 = 通过（或允许跳过）；1 = 超阈值；2 = 找不到安装包（构建失败/产物缺失）。
# CI 跳过机制：环境变量 SKIP_BUNDLE_SIZE_GATE=true 时打印跳过说明并返回 0（用于无 NSIS 环境的降级，
#   风险见 docs/spikes/S-18-安装包体积门禁.md §遗留）。

param(
    [string]$Root,
    [string]$BundleDir,
    [double]$ThresholdMiB = 25,
    [string]$BaselineOut,
    [string]$BaselineRef,
    [string]$Commit,
    [switch]$AllowMissing
)

$ErrorActionPreference = 'Stop'

if ($env:SKIP_BUNDLE_SIZE_GATE -eq 'true') {
    Write-Host '[s18] SKIP_BUNDLE_SIZE_GATE=true：跳过分组体积门禁（CI 降级路径，风险见记录 §遗留）'
    exit 0
}

if (-not $Root) { $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path }
$Root = (Resolve-Path $Root).Path
if (-not $BundleDir) { $BundleDir = Join-Path $Root 'src-tauri\target\release\bundle\nsis' }
if (-not $BaselineOut) { $BaselineOut = Join-Path $Root '.verify\ci\volume_baseline.json' }
if (-not $Commit) {
    try { $Commit = (& git -C $Root rev-parse HEAD 2>$null) } catch { $Commit = $null }
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $BaselineOut) | Out-Null

$setup = Get-ChildItem -Path $BundleDir -Filter '*-setup.exe' -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
$releaseExe = Get-Item (Join-Path $Root 'src-tauri\target\release\snip-ask.exe') -ErrorAction SilentlyContinue

if (-not $setup) {
    if ($AllowMissing) {
        Write-Warning "[s18] 未找到 NSIS 安装包（$BundleDir，AllowMissing=true），跳过"
        exit 0
    }
    Write-Error "[s18] 未找到 NSIS 安装包：$BundleDir"
    exit 2
}

$setupMiB = [math]::Round($setup.Length / 1MB, 3)
$pass = $setupMiB -le $ThresholdMiB

$tauriCliVersion = $null
try { $tauriCliVersion = (& pnpm tauri --version 2>$null | Select-Object -First 1) } catch { }
$nodeVersion = $null
try { $nodeVersion = (& node --version 2>$null) } catch { }
$rustcVersion = $null
try { $rustcVersion = (& rustc --version 2>$null) } catch { }
$osCaption = (Get-CimInstance Win32_OperatingSystem).Caption
$osBuild = (Get-CimInstance Win32_OperatingSystem).BuildNumber

$baseline = [ordered]@{
    spike = 'S-18'
    timestamp = (Get-Date).ToString('o')
    commit = $Commit
    builder = [ordered]@{
        tauriCli = $tauriCliVersion
        node = $nodeVersion
        rustc = $rustcVersion
        os = $osCaption
        osBuild = $osBuild
    }
    artifacts = [ordered]@{
        setupExe = [ordered]@{
            name = $setup.Name
            bytes = [int64]$setup.Length
            mib = $setupMiB
            lastWriteTime = $setup.LastWriteTime.ToString('o')
            sha256 = (Get-FileHash -Algorithm SHA256 -Path $setup.FullName).Hash
        }
        releaseExe = if ($releaseExe) { [ordered]@{
            name = $releaseExe.Name
            bytes = [int64]$releaseExe.Length
            mib = [math]::Round($releaseExe.Length / 1MB, 3)
            sha256 = (Get-FileHash -Algorithm SHA256 -Path $releaseExe.FullName).Hash
        } } else { $null }
    }
    threshold = [ordered]@{ setupMiB = $ThresholdMiB; scope = '不含模型' }
    pass = $pass
    baselineRef = $BaselineRef
}

$baseline | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -Path $BaselineOut

# 与基线参考值对比趋势（可选）
$trend = $null
if ($BaselineRef -and (Test-Path $BaselineRef)) {
    try {
        $ref = Get-Content -Raw -Encoding UTF8 $BaselineRef | ConvertFrom-Json
        $refMiB = [double]$ref.artifacts.setupExe.mib
        $trend = [ordered]@{
            refPath = $BaselineRef
            refMiB = $refMiB
            refCommit = $ref.commit
            deltaMiB = [math]::Round($setupMiB - $refMiB, 3)
            deltaPercent = if ($refMiB -gt 0) { [math]::Round(($setupMiB - $refMiB) / $refMiB * 100, 2) } else { $null }
        }
    } catch {
        $trend = [ordered]@{ error = $_.Exception.Message }
    }
}
if ($trend) { Write-Host ("[s18] 体积趋势：本次 {0} MiB vs 基线 {1} MiB（Δ {2} MiB / {3}%），基线 commit {4}" -f $setupMiB, $trend.refMiB, $trend.deltaMiB, $trend.deltaPercent, $trend.refCommit) }

Write-Host ("[s18] NSIS 安装包：{0} = {1:N0} B = {2} MiB（阈值 {3} MiB，{4}）" -f $setup.Name, $setup.Length, $setupMiB, $ThresholdMiB, $(if ($pass) { 'PASS' } else { 'FAIL' }))
if ($releaseExe) { Write-Host ("[s18] release 主 exe：{0} = {1:N0} B = {2} MiB" -f $releaseExe.Name, $releaseExe.Length, [math]::Round($releaseExe.Length / 1MB, 3)) }
Write-Host "[s18] 体积基线：$BaselineOut"

if (-not $pass) { exit 1 }
exit 0
