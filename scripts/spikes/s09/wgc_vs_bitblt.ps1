#requires -Version 5.1
<#
.SYNOPSIS
    S-09 截图失败降级（DRM / 受保护内容 / WGC 对比）取证脚本。

.DESCRIPTION
    构建并运行同目录的 S09Capture（.NET 10 + WinRT 投影 + GDI BitBlt + D3D11）采集器：
      场景 = 普通窗口 / WDA_EXCLUDEFROMCAPTURE(0x11) / WDA_MONITOR(0x01) / 无边框全屏
            / 全屏+0x01（≈DRM 黑屏）/ 全屏+0x11（窗口从截屏消失）
      路径 = BitBlt 窗口区域 / BitBlt 整个虚拟桌面 / PrintWindow(PW_RENDERFULLCONTENT)
            / WGC 窗口项 / WGC 主屏显示器项
    输出原始数据到 -OutDir（默认 .verify/spikes/S-09，gitignored）：
      raw/results.json、raw/env.json、raw/manifest.json、shots/*.png
    并把「精选聚合数据」写到 docs/spikes/data/s09/（可入 git）：
      summary.json（每条路径的聚合统计 + 自动校验项）、env.json、shots/（缩略图）。

    本脚本只用于技术验证：不改动产品代码、不启动应用（applab）、不做 pnpm 构建。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\spikes\s09\wgc_vs_bitblt.ps1 -Reps 3

.EXAMPLE
    # 只重跑采集器（跳过构建）
    powershell -ExecutionPolicy Bypass -File scripts\spikes\s09\wgc_vs_bitblt.ps1 -NoBuild -Reps 1
#>
param(
    [string]$OutDir = ".verify/spikes/S-09",
    [int]$Reps = 3,
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Resolve-RepoPath([string]$path) {
    if ([System.IO.Path]::IsPathRooted($path)) { return $path }
    return (Join-Path $script:RepoRoot $path)
}

function Write-JsonFile([string]$path, $value, [int]$depth = 24) {
    $dir = Split-Path -Parent $path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    $text = $value | ConvertTo-Json -Depth $depth
    [System.IO.File]::WriteAllText($path, $text, $script:Utf8NoBom)
}

function Write-TextFile([string]$path, [string]$text) {
    $dir = Split-Path -Parent $path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [System.IO.File]::WriteAllText($path, $text, $script:Utf8NoBom)
}

# ───────────────────────── 环境快照 ─────────────────────────

function Get-EnvSnapshot {
    Add-Type -AssemblyName System.Windows.Forms | Out-Null
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class S09Env {
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT pt, uint flags);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr hmon, int dpiType, out uint dpiX, out uint dpiY);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
}
"@
    # 必须在取 Screen 之前设置 DPI 感知，否则拿到的是缩放后的逻辑坐标
    [void][S09Env]::SetProcessDpiAwarenessContext([IntPtr](-4))

    $osKey = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $cs = Get-CimInstance Win32_ComputerSystem
    $gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, AdapterRAM)

    $displays = @()
    foreach ($screen in [System.Windows.Forms.Screen]::AllScreens) {
        $pt = New-Object S09Env+POINT
        $pt.X = [int]($screen.Bounds.X + $screen.Bounds.Width / 2)
        $pt.Y = [int]($screen.Bounds.Y + $screen.Bounds.Height / 2)
        $hmon = [S09Env]::MonitorFromPoint($pt, 1)
        $dpiX = 96; $dpiY = 96
        $hr = [S09Env]::GetDpiForMonitor($hmon, 0, [ref]$dpiX, [ref]$dpiY)
        $displays += [ordered]@{
            deviceName = $screen.DeviceName
            primary    = $screen.Primary
            bounds     = [ordered]@{ x = $screen.Bounds.X; y = $screen.Bounds.Y; width = $screen.Bounds.Width; height = $screen.Bounds.Height }
            dpiX       = $dpiX
            dpiY       = $dpiY
            scale      = [Math]::Round($dpiX / 96.0, 2)
            dpiApiHr   = $hr
        }
    }

    $vx = [S09Env]::GetSystemMetrics(76); $vy = [S09Env]::GetSystemMetrics(77)
    $vw = [S09Env]::GetSystemMetrics(78); $vh = [S09Env]::GetSystemMetrics(79)
    $commit = (& git -C $script:RepoRoot rev-parse --short HEAD 2>$null)
    $branch = (& git -C $script:RepoRoot rev-parse --abbrev-ref HEAD 2>$null)

    return [ordered]@{
        generatedAt        = (Get-Date).ToString("yyyy-MM-ddTHH:mm:sszzz")
        repoCommit         = ($commit | Out-String).Trim()
        repoBranch         = ($branch | Out-String).Trim()
        os                 = [ordered]@{
            caption        = $osKey.ProductName
            displayVersion = $osKey.DisplayVersion
            build          = "$($osKey.CurrentBuild).$($osKey.UBR)"
            edition        = $osKey.EditionID
            architecture   = $env:PROCESSOR_ARCHITECTURE
        }
        cpu                = [ordered]@{
            name  = $cpu.Name
            cores = $cpu.NumberOfCores
            logicalProcessors = $cpu.NumberOfLogicalProcessors
            maxClockMHz = $cpu.MaxClockSpeed
        }
        memoryGiB          = [Math]::Round($cs.TotalPhysicalMemory / 1GB, 2)
        gpu                = $gpu
        displays           = $displays
        virtualDesktop     = [ordered]@{ x = $vx; y = $vy; width = $vw; height = $vh }
        runtime            = [ordered]@{
            powershell  = $PSVersionTable.PSVersion.ToString()
            dotnetSdk   = (& dotnet --version 2>$null | Out-String).Trim()
            dotnetRuntime = (& dotnet --list-runtimes 2>$null | Where-Object { $_ -match 'Microsoft\.NETCore\.App 10\.' } | Select-Object -First 1)
        }
        processDpiAwareness = "per-monitor-v2（脚本 Get-EnvSnapshot 内 SetProcessDpiAwarenessContext(-4)）"
        note               = "DRM/Netflix 真实播放、锁屏、真·全屏独占未覆盖；用 SetWindowDisplayAffinity 构造等价的受保护窗口，见 docs/spikes/S-09-*.md「遗留」。"
    }
}

function Get-FileSha256([string]$path) {
    return (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant()
}

# ───────────────────────── 构建 & 运行采集器 ─────────────────────────

function Build-CaptureTool([string]$workDir) {
    $srcDir = $PSScriptRoot
    New-Item -ItemType Directory -Force -Path $workDir | Out-Null
    Copy-Item -Path @((Join-Path $srcDir "Program.cs"), (Join-Path $srcDir "S09Capture.csproj")) -Destination $workDir -Force
    $csproj = Join-Path $workDir "S09Capture.csproj"
    Write-Host "[s09] dotnet build ..."
    $log = & dotnet build $csproj -c Release --nologo -v m -o (Join-Path $workDir "bin") 2>&1
    $log | ForEach-Object { Write-Host "  $_" }
    if ($LASTEXITCODE -ne 0) { throw "dotnet build 失败（退出码 $LASTEXITCODE），见上方输出" }
    $exe = Join-Path $workDir "bin/S09Capture.exe"
    if (-not (Test-Path $exe)) { throw "构建成功但找不到 $exe" }
    return $exe
}

# ───────────────────────── 聚合 ─────────────────────────

function Get-PathAggregate($scenario, [string]$pathName) {
    $samples = @($scenario.samples | Where-Object { $_.path -eq $pathName })
    if ($samples.Count -eq 0) { return $null }
    $withFrame = @($samples | Where-Object { $null -ne $_.frame })
    $withRegion = @($samples | Where-Object { $null -ne $_.region })
    $frameCount = $withFrame.Count
    $blackCount = @($withFrame | Where-Object { $_.frame.black -eq $true }).Count
    $okCount = @($samples | Where-Object { $_.ok -eq $true }).Count
    if ($okCount -eq 0) {
        $verdict = "noFrame"
    } elseif ($frameCount -gt 0 -and $blackCount -eq $frameCount) {
        $verdict = "black"
    } elseif ($frameCount -gt 0) {
        $verdict = "content"
    } else {
        $verdict = "error"
    }
    $aggregate = [ordered]@{
        samples          = $samples.Count
        okCount          = $okCount
        frameCount       = $frameCount
        blackCount       = $blackCount
        meanBlackRatio   = $null
        minBlackRatio    = $null
        maxBlackRatio    = $null
        meanLuma         = $null
        meanStdDev       = $null
        meanElapsedMs    = [Math]::Round((($samples | Measure-Object -Property elapsedMs -Average).Average), 3)
        rep1Sha256       = $null
        regionMeanBlackRatio = $null
        regionBlackCount = $null
        verdict          = $verdict
        errors           = @($samples | Where-Object { $_.error } | ForEach-Object { $_.error } | Sort-Object -Unique)
    }
    if ($frameCount -gt 0) {
        $aggregate.meanBlackRatio = [Math]::Round((($withFrame | ForEach-Object { $_.frame.blackRatio } | Measure-Object -Average).Average), 6)
        $aggregate.minBlackRatio = ($withFrame | ForEach-Object { $_.frame.blackRatio } | Measure-Object -Minimum).Minimum
        $aggregate.maxBlackRatio = ($withFrame | ForEach-Object { $_.frame.blackRatio } | Measure-Object -Maximum).Maximum
        $aggregate.meanLuma = [Math]::Round((($withFrame | ForEach-Object { $_.frame.meanLuma } | Measure-Object -Average).Average), 3)
        $aggregate.meanStdDev = [Math]::Round((($withFrame | ForEach-Object { $_.frame.stdDev } | Measure-Object -Average).Average), 3)
        $rep1 = @($withFrame | Sort-Object rep)[0]
        $aggregate.rep1Sha256 = $rep1.frame.sha256
    }
    if ($withRegion.Count -gt 0) {
        $aggregate.regionMeanBlackRatio = [Math]::Round((($withRegion | ForEach-Object { $_.region.blackRatio } | Measure-Object -Average).Average), 6)
        $aggregate.regionBlackCount = @($withRegion | Where-Object { $_.region.black -eq $true }).Count
    }
    return $aggregate
}

function Get-ScenarioAggregate($scenario, [string[]]$paths) {
    $pathResults = [ordered]@{}
    foreach ($path in $paths) {
        $agg = Get-PathAggregate $scenario $path
        if ($null -ne $agg) { $pathResults[$path] = $agg }
    }
    return [ordered]@{
        name            = $scenario.name
        label           = $scenario.label
        description     = $scenario.description
        affinity        = $scenario.affinity
        affinityName    = $scenario.affinityName
        affinityApplied = $scenario.affinityApplied
        fullscreen      = $scenario.fullscreen
        windowRect      = $scenario.windowRect
        paths           = $pathResults
    }
}

function Find-PathAgg($summary, [string]$scenarioName, [string]$pathName) {
    foreach ($sc in $summary.scenarios) {
        if ($sc.name -eq $scenarioName -and $null -ne $sc.paths.$pathName) { return $sc.paths.$pathName }
    }
    return $null
}

function Add-Check($checks, [string]$name, [bool]$pass, [string]$detail) {
    $checks += [ordered]@{ name = $name; pass = $pass; detail = $detail }
    return , $checks
}

function Build-Checks($summary) {
    $checks = @()
    $normalBitblt = Find-PathAgg $summary "normal" "bitblt_region"
    $normalWgc = Find-PathAgg $summary "normal" "wgc_window"
    $normalMonitor = Find-PathAgg $summary "normal" "wgc_monitor"
    $excludeBitblt = Find-PathAgg $summary "wda_exclude" "bitblt_region"
    $excludeWgc = Find-PathAgg $summary "wda_exclude" "wgc_window"
    $excludeMonitor = Find-PathAgg $summary "wda_exclude" "wgc_monitor"
    $monitorBitblt = Find-PathAgg $summary "wda_monitor" "bitblt_region"
    $monitorWgc = Find-PathAgg $summary "wda_monitor" "wgc_window"
    $monitorWgcMonitor = Find-PathAgg $summary "wda_monitor" "wgc_monitor"
    $monitorDesktop = Find-PathAgg $summary "wda_monitor" "bitblt_desktop"
    $fsMonitorBitblt = Find-PathAgg $summary "fullscreen_wda_monitor" "bitblt_region"
    $fsMonitorDesktop = Find-PathAgg $summary "fullscreen_wda_monitor" "bitblt_desktop"
    $fsMonitorWgc = Find-PathAgg $summary "fullscreen_wda_monitor" "wgc_window"
    $fsMonitorWgcMonitor = Find-PathAgg $summary "fullscreen_wda_monitor" "wgc_monitor"
    $fsExcludeBitblt = Find-PathAgg $summary "fullscreen_wda_exclude" "bitblt_region"
    $fsExcludeWgc = Find-PathAgg $summary "fullscreen_wda_exclude" "wgc_window"

    $checks = Add-Check $checks "normal_bitblt_content" ($normalBitblt.verdict -eq "content") `
        ("BitBlt 普通窗口区域 blackRatio={0}, meanLuma={1} → {2}" -f $normalBitblt.meanBlackRatio, $normalBitblt.meanLuma, $normalBitblt.verdict)
    $checks = Add-Check $checks "normal_wgc_content" ($normalWgc.verdict -eq "content") `
        ("WGC 窗口项 blackRatio={0}, meanLuma={1} → {2}" -f $normalWgc.meanBlackRatio, $normalWgc.meanLuma, $normalWgc.verdict)
    $checks = Add-Check $checks "normal_wgc_equals_bitblt_pixels" ($normalWgc.rep1Sha256 -eq $normalBitblt.rep1Sha256) `
        ("普通窗口 BitBlt 与 WGC 窗口项 rep1 帧 sha256 相同 = {0}" -f ($normalWgc.rep1Sha256 -eq $normalBitblt.rep1Sha256))
    $checks = Add-Check $checks "normal_wgc_monitor_region_content" ($normalMonitor.regionMeanBlackRatio -lt 0.99) `
        ("WGC 主屏捕获中窗口区域 blackRatio={0}（应≈窗口内容，非黑）" -f $normalMonitor.regionMeanBlackRatio)

    $checks = Add-Check $checks "wda_monitor_bitblt_region_black" ($monitorBitblt.verdict -eq "black") `
        ("WDA_MONITOR(0x01) BitBlt 窗口区域 blackRatio={0}（期望 1.0）" -f $monitorBitblt.meanBlackRatio)
    $checks = Add-Check $checks "wda_monitor_bitblt_desktop_region_black" ($monitorDesktop.regionBlackCount -eq $monitorDesktop.samples) `
        ("WDA_MONITOR 整桌面 BitBlt 中窗口区域 blackRatio={0}（整帧 blackRatio={1}）" -f $monitorDesktop.regionMeanBlackRatio, $monitorDesktop.meanBlackRatio)
    $checks = Add-Check $checks "wda_monitor_bitblt_desktop_frame_not_black" ($monitorDesktop.verdict -eq "content") `
        ("混合双屏整帧 blackRatio={0} < 0.99 → 整帧黑判定不会触发（需按区域/切片判定）" -f $monitorDesktop.meanBlackRatio)
    $checks = Add-Check $checks "wda_monitor_wgc_window_black" ($monitorWgc.verdict -eq "black") `
        ("WDA_MONITOR WGC 窗口项 blackRatio={0} → WGC 不能绕过保护" -f $monitorWgc.meanBlackRatio)
    $checks = Add-Check $checks "wda_monitor_wgc_monitor_region_black" ($monitorWgcMonitor.regionBlackCount -eq $monitorWgcMonitor.samples) `
        ("WDA_MONITOR WGC 主屏捕获中窗口区域 blackRatio={0}（整帧 blackRatio={1}，窗口只占部分屏幕）" -f $monitorWgcMonitor.regionMeanBlackRatio, $monitorWgcMonitor.meanBlackRatio)

    $checks = Add-Check $checks "wda_exclude_bitblt_region_not_black" ($excludeBitblt.verdict -eq "content") `
        ("WDA_EXCLUDEFROMCAPTURE(0x11) BitBlt 区域 blackRatio={0} → 窗口从截屏消失、露出背后内容而非黑屏" -f $excludeBitblt.meanBlackRatio)
    $checks = Add-Check $checks "wda_exclude_wgc_window_no_frame" ($excludeWgc.verdict -eq "noFrame") `
        ("WDA_EXCLUDEFROMCAPTURE WGC 窗口项 0/{0} 次拿到帧（每次 5000ms 超时）" -f $excludeWgc.samples)
    $checks = Add-Check $checks "wda_exclude_wgc_monitor_region_not_black" ($excludeMonitor.regionMeanBlackRatio -lt 0.99) `
        ("WDA_EXCLUDEFROMCAPTURE WGC 主屏捕获中窗口区域 blackRatio={0}（窗口缺失，露出背后内容）" -f $excludeMonitor.regionMeanBlackRatio)

    $checks = Add-Check $checks "fullscreen_wda_monitor_bitblt_region_black" ($fsMonitorBitblt.verdict -eq "black") `
        ("全屏+WDA_MONITOR BitBlt 主屏区域 blackRatio={0}（≈DRM 全屏黑屏）" -f $fsMonitorBitblt.meanBlackRatio)
    $checks = Add-Check $checks "fullscreen_wda_monitor_desktop_frame_not_black" ($fsMonitorDesktop.verdict -eq "content") `
        ("全屏+WDA_MONITOR 虚拟桌面整帧 blackRatio={0} < 0.99（混合 DPI 双屏只有主屏黑）→ 整帧判定漏检" -f $fsMonitorDesktop.meanBlackRatio)
    $checks = Add-Check $checks "fullscreen_wda_monitor_wgc_black" ($fsMonitorWgc.verdict -eq "black" -and $fsMonitorWgcMonitor.verdict -eq "black") `
        ("全屏+WDA_MONITOR WGC 窗口项/主屏项 blackRatio={0}/{1}" -f $fsMonitorWgc.meanBlackRatio, $fsMonitorWgcMonitor.meanBlackRatio)
    $checks = Add-Check $checks "fullscreen_wda_exclude_bitblt_not_black" ($fsExcludeBitblt.verdict -eq "content") `
        ("全屏+WDA_EXCLUDEFROMCAPTURE BitBlt 主屏区域 blackRatio={0} → 不是黑屏" -f $fsExcludeBitblt.meanBlackRatio)
    $checks = Add-Check $checks "fullscreen_wda_exclude_wgc_no_frame" ($fsExcludeWgc.verdict -eq "noFrame") `
        ("全屏+WDA_EXCLUDEFROMCAPTURE WGC 窗口项 0/{0} 拿到帧" -f $fsExcludeWgc.samples)

    return $checks
}

# ───────────────────────── 精选截图（缩小后可入 git） ─────────────────────────

function Save-DownscaledPng([string]$source, [string]$target, [int]$maxWidth = 1280) {
    Add-Type -AssemblyName System.Drawing | Out-Null
    $image = [System.Drawing.Image]::FromFile($source)
    try {
        $ratio = [Math]::Min(1.0, $maxWidth / $image.Width)
        $w = [Math]::Max(1, [int]($image.Width * $ratio))
        $h = [Math]::Max(1, [int]($image.Height * $ratio))
        $bitmap = New-Object System.Drawing.Bitmap($w, $h)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.DrawImage($image, 0, 0, $w, $h)
            } finally { $graphics.Dispose() }
            $dir = Split-Path -Parent $target
            if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
            $bitmap.Save($target, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
    } finally { $image.Dispose() }
}

$CuratedShots = @(
    @{ Source = "normal_bitblt_region_rep1.png";       Target = "normal-bitblt-region.png" },
    @{ Source = "normal_wgc_window_rep1.png";          Target = "normal-wgc-window.png" },
    @{ Source = "wda_monitor_bitblt_region_rep1.png";  Target = "wda-monitor-bitblt-region-black.png" },
    @{ Source = "wda_exclude_bitblt_region_rep1.png";  Target = "wda-exclude-bitblt-region-window-missing.png" },
    @{ Source = "wda_exclude_wgc_monitor_rep1.png";    Target = "wda-exclude-wgc-monitor-window-missing.png" },
    @{ Source = "fullscreen_wda_monitor_wgc_monitor_rep1.png"; Target = "fullscreen-wda-monitor-wgc-monitor-black.png" }
)

# ───────────────────────── 主流程 ─────────────────────────

$outDirAbs = Resolve-RepoPath $OutDir
$buildDir = Resolve-RepoPath ".verify/spikes/S-09/tool/build"
$curatedDir = Resolve-RepoPath "docs/spikes/data/s09"
New-Item -ItemType Directory -Force -Path (Join-Path $outDirAbs "raw"), (Join-Path $outDirAbs "shots") | Out-Null

Write-Host "[s09] repo root : $script:RepoRoot"
Write-Host "[s09] output dir: $outDirAbs"
Write-Host "[s09] reps      : $Reps"

$exe = $null
if (-not $NoBuild) {
    $exe = Build-CaptureTool $buildDir
} else {
    $exe = Join-Path $buildDir "bin/S09Capture.exe"
    if (-not (Test-Path $exe)) { throw "-NoBuild 但找不到已构建的 $exe，请先不带 -NoBuild 运行一次" }
}

$envSnapshot = Get-EnvSnapshot
Write-JsonFile (Join-Path $outDirAbs "raw/env.json") $envSnapshot

Write-Host "[s09] running capture tool ..."
$runOutput = & $exe --out $outDirAbs --reps $Reps 2>&1
$runOutput | ForEach-Object { Write-Host "  $_" }
if ($LASTEXITCODE -ne 0) { throw "S09Capture 运行失败（退出码 $LASTEXITCODE）" }

$resultsPath = Join-Path $outDirAbs "raw/results.json"
if (-not (Test-Path $resultsPath)) { throw "采集器未生成 $resultsPath" }
$results = Get-Content -Raw -Encoding UTF8 $resultsPath | ConvertFrom-Json

$allPaths = @("bitblt_region", "bitblt_desktop", "printwindow", "wgc_window", "wgc_monitor")
$scenarioSummaries = @()
foreach ($scenario in $results.scenarios) {
    $scenarioSummaries += Get-ScenarioAggregate $scenario $allPaths
}

$summary = [ordered]@{
    tool             = "s09-capture/1.0"
    generatedAt      = $results.generatedAt
    capturedBy       = "scripts/spikes/s09/wgc_vs_bitblt.ps1 ($Reps reps)"
    repoCommit       = $envSnapshot.repoCommit
    thresholds       = $results.thresholds
    virtualDesktop   = $results.virtualScreen
    primaryMonitor   = $results.primaryMonitor
    d3dDeviceInfo    = $results.d3dDeviceInfo
    rawDataDir       = ".verify/spikes/S-09/raw/results.json（gitignored）"
    scenarios        = $scenarioSummaries
    checks           = @()
}
$summary.checks = Build-Checks $summary

# 精选数据落 docs/spikes/data/s09/（小体积、可入 git）
Write-JsonFile (Join-Path $curatedDir "summary.json") $summary
$curatedEnv = [ordered]@{
    generatedAt    = $envSnapshot.generatedAt
    repoCommit     = $envSnapshot.repoCommit
    os             = $envSnapshot.os
    cpu            = $envSnapshot.cpu
    memoryGiB      = $envSnapshot.memoryGiB
    gpu            = $envSnapshot.gpu
    displays       = $envSnapshot.displays
    virtualDesktop = $envSnapshot.virtualDesktop
    runtime        = $envSnapshot.runtime
    note           = $envSnapshot.note
}
Write-JsonFile (Join-Path $curatedDir "env.json") $curatedEnv

foreach ($shot in $CuratedShots) {
    $source = Join-Path $outDirAbs ("shots/" + $shot.Source)
    if (Test-Path $source) {
        Save-DownscaledPng $source (Join-Path $curatedDir ("shots/" + $shot.Target))
    } else {
        Write-Warning "[s09] 缺少截图 $source"
    }
}

# 清单 / 可选复跑信息
$manifest = [ordered]@{
    tool             = "scripts/spikes/s09/wgc_vs_bitblt.ps1"
    toolVersion      = "s09-capture/1.0"
    generatedAt      = (Get-Date).ToString("yyyy-MM-ddTHH:mm:sszzz")
    repoCommit       = $envSnapshot.repoCommit
    reps             = $Reps
    outDir           = (Resolve-Path $outDirAbs).Path
    sourceHashes     = [ordered]@{
        "Program.cs"        = Get-FileSha256 (Join-Path $PSScriptRoot "Program.cs")
        "S09Capture.csproj" = Get-FileSha256 (Join-Path $PSScriptRoot "S09Capture.csproj")
        "wgc_vs_bitblt.ps1" = Get-FileSha256 $PSCommandPath
    }
    builtExe         = $exe
    builtExeSha256   = if (Test-Path $exe) { Get-FileSha256 $exe } else { $null }
    resultsJson      = $resultsPath
    curatedSummary   = (Join-Path $curatedDir "summary.json")
    curatedEnv       = (Join-Path $curatedDir "env.json")
    rawShots         = (Get-ChildItem (Join-Path $outDirAbs "shots") -Filter *.png | Select-Object -ExpandProperty Name)
    runStdout        = @($runOutput)
    notes            = @(
        "WGC 走 Windows.Graphics.Capture：IGraphicsCaptureItemInterop vtable slot 3/4 + Direct3D11CaptureFramePool + SoftwareBitmap 回读。",
        "受保护窗口通过 SetWindowDisplayAffinity 构造：WDA_EXCLUDEFROMCAPTURE=0x11 / WDA_MONITOR=0x01；不做任何破解。",
        "真·全屏独占（D3D exclusive）与 Netflix 真实播放、锁屏场景未覆盖，见记录「遗留」。"
    )
}
Write-JsonFile (Join-Path $outDirAbs "raw/manifest.json") $manifest

# 控制台汇总
Write-Host ""
Write-Host "=== checks ==="
foreach ($check in $summary.checks) {
    $mark = if ($check.pass) { "PASS" } else { "FAIL" }
    Write-Host ("[{0}] {1} :: {2}" -f $mark, $check.name, $check.detail)
}
$failed = @($summary.checks | Where-Object { -not $_.pass })
Write-Host ""
Write-Host ("[s09] checks {0}/{1} passed" -f ($summary.checks.Count - $failed.Count), $summary.checks.Count)
Write-Host ("[s09] raw     -> {0}" -f $resultsPath)
Write-Host ("[s09] curated -> {0}\summary.json, env.json, shots/" -f $curatedDir)
if ($failed.Count -gt 0) {
    Write-Host "[s09] FAILED checks:"
    $failed | ForEach-Object { Write-Host ("  - " + $_.name + " :: " + $_.detail) }
    exit 2
}
exit 0
