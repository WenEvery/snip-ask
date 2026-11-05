# S-10 冷启动与空闲资源测量（release / debug 两种构建）
#
# 冷启动口径：进程创建（Process.Start 的 StartTime，UTC）→ 应用日志出现「框选 AI 初始化完成」
#             （该行由 app/lifecycle.rs 在托盘创建 + 全局热键注册 + 隐藏窗口预创建之后输出，
#              是「托盘可交互」最接近的应用内可观测时点）。
#            附加证据：同次运行日志中不得出现「托盘创建失败 / 全局热键注册失败」；可用 CDP 端口时间作交叉校验。
# 空闲内存口径：主进程 Private WorkingSet（Get-Counter 'Process(snip-ask)\Working Set - Private'）
#               与 WorkingSet64 / PrivateMemorySize64；另记录含 WebView2 子进程的整棵进程树汇总（补充参考）。
# 空闲 CPU 口径：进程树 CPU 时间差 / 墙钟差 / 逻辑核数 × 100%（≈ 任务管理器「CPU」列，占总核百分比），
#               同时给出「占单核百分比」；每 SampleSec 采样一次。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s10_measure.ps1 -Mode coldstart -Label release -Runs 10
#   ... -Mode idle -Label release -IdleMinutes 30 -SampleSec 5
#   ... -Mode all  -Label debug  -Runs 10 -IdleMinutes 10
#
# 依赖：被测 exe 存在；debug 构建需要 Vite dev server（http://localhost:1420，未启动时本脚本会临时拉起并结束）；
#      release 构建直接读内嵌前端资源，无需 Vite。
# 纪律：测量期间不要并行构建/OCR/本地模型推理（由 .verify/p1/quiet/ 标记与 lead 协调）。

param(
    [ValidateSet('coldstart', 'idle', 'all')]
    [string]$Mode = 'all',
    # 标签用于区分数据目录与输出文件名；冷启动稳态推荐 release/debug，首次启动对照可用 release-fr
    [string]$Label = 'release',
    [string]$Root,
    [string]$ExePath,
    # 可选：把已配置用户的 config.json/onboarding.json 播种到便携数据目录（模拟「已配置用户」稳态；
    # 不传则首次启动会走引导，测得的是首次启动口径）
    [string]$SeedFrom,
    [int]$Runs = 10,
    [int]$IdleMinutes = 30,
    [int]$SampleSec = 5,
    [int]$WarmupSec = 60,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'

if (-not $Root) { $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path }
$Root = (Resolve-Path $Root).Path
if (-not $ExePath) {
    $ExePath = if ($Label -match 'debug') {
        Join-Path $Root 'src-tauri\target\debug\snip-ask.exe'
    } else {
        Join-Path $Root 'src-tauri\target\release\snip-ask.exe'
    }
}
if (-not (Test-Path $ExePath)) { throw "被测 exe 不存在：$ExePath" }
if (-not $OutDir) { $OutDir = Join-Path $Root ".verify\spikes\S-10\raw" }
$workDir = Join-Path $Root ".verify\spikes\S-10\app-$Label"
$rawDir = $OutDir
New-Item -ItemType Directory -Force -Path $rawDir, $workDir | Out-Null

$exeName = 'snip-ask.exe'
$workExe = Join-Path $workDir $exeName
Copy-Item -Path $ExePath -Destination $workExe -Force
Set-Content -Path (Join-Path $workDir 'portable.flag') -Value "S-10 portable data dir`n" -Encoding ASCII
$dataDir = Join-Path $workDir 'data'
$logDir = Join-Path $dataDir 'logs'
$procName = 'snip-ask'

# 可选：播种「已配置用户」数据（config.json/onboarding.json），使冷启动测量对应稳态而非首次引导
$seedInfo = [ordered]@{ from = $SeedFrom; applied = $false; files = @() }
if ($SeedFrom -and (Test-Path $SeedFrom)) {
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    foreach ($f in @('config.json', 'onboarding.json')) {
        $src = Join-Path $SeedFrom $f
        if (Test-Path $src) {
            Copy-Item -Path $src -Destination (Join-Path $dataDir $f) -Force
            $seedInfo.files += $f
        }
    }
    if (@($seedInfo.files).Count -gt 0) { $seedInfo.applied = $true }
    Write-Host ("[s10] 已播种便携数据：{0}（{1}）" -f $SeedFrom, ($seedInfo.files -join ','))
} elseif ($SeedFrom) {
    Write-Warning "[s10] -SeedFrom 不存在：$SeedFrom（按首次启动口径测量）"
}

$VITE_URL = 'http://localhost:1420/'
$viteProc = $null
$viteStartedByUs = $false

function Write-Utf8Json([string]$Path, $Object) {
    $Object | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path $Path
}

function Get-SystemSnapshot {
    $os = Get-CimInstance Win32_OperatingSystem
    $cpuPct = $null
    try {
        $samples = Get-Counter '\Processor(_Total)\% Processor Time' -MaxSamples 1 -ErrorAction Stop
        $cpuPct = [math]::Round($samples.CounterSamples[0].CookedValue, 2)
    } catch { }
    return [ordered]@{
        timestamp = (Get-Date).ToString('o')
        cpuPercentTotal = $cpuPct
        availableMemoryMiB = [math]::Round($os.FreePhysicalMemory / 1024, 1)
        logicalProcessors = [Environment]::ProcessorCount
    }
}

function Get-ProcessTree([int]$ProcessId) {
    $all = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue
    $byParent = @{}
    foreach ($p in $all) {
        $pp = [int]$p.ParentProcessId
        if (-not $byParent.ContainsKey($pp)) { $byParent[$pp] = @() }
        $byParent[$pp] += $p
    }
    $result = New-Object System.Collections.ArrayList
    $queue = New-Object System.Collections.Queue
    $queue.Enqueue($ProcessId)
    while ($queue.Count -gt 0) {
        $cur = [int]$queue.Dequeue()
        if (-not $byParent.ContainsKey($cur)) { continue }
        foreach ($child in $byParent[$cur]) {
            [void]$result.Add($child)
            $queue.Enqueue([int]$child.ProcessId)
        }
    }
    return ,$result.ToArray()
}

function Get-ProcessMetrics([int]$ProcessId) {
    $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $proc) { return $null }
    $children = Get-ProcessTree $ProcessId
    $treeCpuMs = [double]$proc.TotalProcessorTime.TotalMilliseconds
    $treeWs = [int64]$proc.WorkingSet64
    $childCount = 0
    $childWs = [int64]0
    $childCpuMs = [double]0
    foreach ($c in $children) {
        $cp = Get-Process -Id ([int]$c.ProcessId) -ErrorAction SilentlyContinue
        if ($cp) {
            $childCount += 1
            $childWs += [int64]$cp.WorkingSet64
            $childCpuMs += [double]$cp.TotalProcessorTime.TotalMilliseconds
        }
    }
    $privateWs = $null
    try {
        $sample = Get-Counter ("\Process({0})\Working Set - Private" -f $procName) -MaxSamples 1 -ErrorAction Stop
        $privateWs = [int64]$sample.CounterSamples[0].CookedValue
    } catch { }
    return [ordered]@{
        pid = $proc.Id
        timestamp = (Get-Date).ToString('o')
        workingSetBytes = [int64]$proc.WorkingSet64
        privateMemoryBytes = [int64]$proc.PrivateMemorySize64
        privateWorkingSetBytes = $privateWs
        threadCount = $proc.Threads.Count
        handleCount = $proc.HandleCount
        mainCpuMs = [double]$proc.TotalProcessorTime.TotalMilliseconds
        childCount = $childCount
        childWorkingSetBytes = $childWs
        childCpuMs = $childCpuMs
        treeWorkingSetBytes = [int64]($treeWs + $childWs)
        treeCpuMs = [double]($treeCpuMs + $childCpuMs)
    }
}

function Test-ViteUp {
    try {
        $r = Invoke-WebRequest -Uri $VITE_URL -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
        return ($r.StatusCode -eq 200)
    } catch {
        try {
            $c = New-Object System.Net.Sockets.TcpClient
            $iar = $c.BeginConnect('127.0.0.1', 1420, $null, $null)
            $ok = $iar.AsyncWaitHandle.WaitOne(300)
            $connected = $ok -and $c.Connected
            $c.Close()
            return [bool]$connected
        } catch { return $false }
    }
}

function Ensure-Vite {
    if (Test-ViteUp) { Write-Host '[s10] 复用已在运行的 Vite（localhost:1420）'; return }
    Write-Host '[s10] 启动临时 Vite dev server（pnpm dev:web）…'
    $script:viteProc = Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', 'pnpm dev:web' -WorkingDirectory $Root -PassThru -WindowStyle Hidden
    $script:viteStartedByUs = $true
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 60000) {
        if (Test-ViteUp) { Write-Host '[s10] Vite 就绪'; return }
        Start-Sleep -Milliseconds 300
    }
    throw 'Vite dev server 未在 60s 内就绪'
}

function Stop-Vite {
    if ($script:viteStartedByUs -and $script:viteProc) {
        foreach ($c in (Get-ProcessTree $script:viteProc.Id)) {
            Stop-Process -Id ([int]$c.ProcessId) -Force -ErrorAction SilentlyContinue
        }
        Stop-Process -Id $script:viteProc.Id -Force -ErrorAction SilentlyContinue
        Write-Host '[s10] 已结束临时 Vite'
    }
}

function Stop-App([int]$ProcessId) {
    if ($ProcessId -le 0) { return }
    foreach ($c in (Get-ProcessTree $ProcessId)) {
        Stop-Process -Id ([int]$c.ProcessId) -Force -ErrorAction SilentlyContinue
    }
    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Wait-AppGone([int]$TimeoutMs = 15000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        $left = @(Get-Process -Name $procName -ErrorAction SilentlyContinue)
        if ($left.Count -eq 0) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Get-NewestLogFile([string]$Dir) {
    if (-not (Test-Path $Dir)) { return $null }
    return Get-ChildItem -Path $Dir -Filter 'snipask.log*' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

function Get-NewestLogPath([string]$Dir) {
    if (-not (Test-Path $Dir)) { return $null }
    # 按文件名（含日期后缀）倒序，避免依赖运行中被占用的文件元数据
    $files = @(Get-ChildItem -Path $Dir -Filter 'snipask.log*' -File -ErrorAction SilentlyContinue | Sort-Object Name -Descending)
    if ($files.Count -eq 0) { return $null }
    return $files[0].FullName
}

function Read-LogText([string]$Path, [int64]$From = 0) {
    # 关键：应用进程持有日志文件句柄时，用 FileShare=ReadWrite 直接读取可立即看到新写入内容
    # （Get-ChildItem 的 Length/LastWriteTime 在文件打开期间可能陈旧）。
    try {
        $fs = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
        try {
            [void]$fs.Seek($From, 'Begin')
            $reader = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
            try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
        } finally { $fs.Dispose() }
    } catch { return $null }
}

function Read-AppendedLogText([string]$File, [int64]$FromOffset) {
    return (Read-LogText -Path $File -From $FromOffset)
}

function Set-HotkeyProbe {
    if (-not ('HotkeyProbe' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class HotkeyProbe {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr hWnd, int id);
}
'@
    }
}
function Test-CtrlQOwned([int]$ProbeId) {
    Set-HotkeyProbe
    $ok = [HotkeyProbe]::RegisterHotKey([IntPtr]::Zero, $ProbeId, 0x0002, 0x51)
    if ($ok) {
        [void][HotkeyProbe]::UnregisterHotKey([IntPtr]::Zero, $ProbeId)
        return [ordered]@{ registered = $true; ownedByOther = $false; win32Error = 0; note = 'Ctrl+Q 当前空闲（探测后已注销）' }
    }
    $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    return [ordered]@{ registered = $false; ownedByOther = ($err -eq 1409); win32Error = $err; note = 'Ctrl+Q 已被占用（1409 = 已被注册，通常为被测应用）' }
}

# ---------- 前置：确认无其它应用实例 ----------
$existing = @(Get-Process -Name $procName -ErrorAction SilentlyContinue)
if ($existing.Count -gt 0) {
    throw "测量前存在 snip-ask 进程（pid=$($existing.Id -join ',')）；S-10 需在安静窗口独占应用，请先让持锁者 stop。"
}

$powerPlanName = $null
try {
    $powerPlanName = (Get-CimInstance -Namespace 'root\cimv2\power' -ClassName Win32_PowerPlan -Filter 'IsActive=True' -ErrorAction Stop).ElementName
} catch { $powerPlanName = $null }

$summary = [ordered]@{
    spike = 'S-10'
    label = $Label
    mode = $Mode
    timestamp = (Get-Date).ToString('o')
    exePath = $ExePath
    workExe = $workExe
    exeBytes = (Get-Item $ExePath).Length
    exeSha256 = (Get-FileHash -Algorithm SHA256 -Path $ExePath).Hash
    environment = [ordered]@{
        host = $env:COMPUTERNAME
        osCaption = (Get-CimInstance Win32_OperatingSystem).Caption
        osBuild = (Get-CimInstance Win32_OperatingSystem).BuildNumber
        cpu = (Get-CimInstance Win32_Processor).Name
        logicalProcessors = [Environment]::ProcessorCount
        ramGiB = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 2)
        powerPlan = $powerPlanName
        systemBaseline = (Get-SystemSnapshot)
    }
    coldStart = $null
    idle = $null
    seed = $seedInfo
}

# Ctrl+Q 基线（应用未运行时应为空闲）
$summary['environment']['ctrlQBaseline'] = (Test-CtrlQOwned 9101)

try {
    if ($Mode -eq 'coldstart' -or $Mode -eq 'all') {
        if ($Label -eq 'debug') { Ensure-Vite } else { Write-Host '[s10] release 构建使用内嵌前端，无需 Vite' }
        $runList = @()
        for ($i = 1; $i -le $Runs; $i += 1) {
            # 清理上一次实例（含 WebView2 子进程），并等待命名互斥体/用户数据锁释放
            $prev = @(Get-Process -Name $procName -ErrorAction SilentlyContinue)
            foreach ($p in $prev) { Stop-App $p.Id }
            if (-not (Wait-AppGone 15000)) { throw "第 $i 次启动前无法结束上一实例" }
            Start-Sleep -Milliseconds 1500

            $sw = [Diagnostics.Stopwatch]::StartNew()
            $proc = Start-Process -FilePath $workExe -WorkingDirectory $workDir -PassThru -ErrorAction Stop
            $procStartUtc = $proc.StartTime.ToUniversalTime()
            $startRecordWall = (Get-Date).ToUniversalTime()
            $run = [ordered]@{
                index = $i
                pid = $proc.Id
                processStartUtc = $procStartUtc.ToString('o')
                swStartUtc = $startRecordWall.ToString('o')
                readyDetected = $false
                cdpPort = $null
            }
            # 读取策略（实测踩坑）：应用运行期间，Windows 目录项元数据（Length/LastWriteTime）对
            # Get-ChildItem 是「陈旧」的（文件被独占打开时 NTFS 不急着更新），但以 FileShare=ReadWrite
            # 直接打开文件读取可以立即看到新写入的内容。因此不用「按 Length 增量读」，改为
            # 「整文件读取 + 只接受行内时间戳 >= 本次进程创建时间」的解析（文件仅数 KB，开销可忽略）。
            $readyAt = $null
            $readyLine = $null
            $warnings = @()
            $liveDetected = $false
            while ($sw.ElapsedMilliseconds -lt 20000) {
                $logPath = Get-NewestLogPath $logDir
                if ($logPath) {
                    $txt = Read-LogText $logPath
                    if ($txt) {
                        foreach ($line in ($txt -split "`r?`n")) {
                            if ($line -match '托盘创建失败|全局热键注册失败') { $warnings += $line.Trim() }
                            if ($line -match '初始化完成') {
                                $tsText = ($line -split '\s+')[0]
                                try {
                                    $t = [DateTimeOffset]::Parse($tsText).UtcDateTime
                                    if ($t -lt $procStartUtc.AddSeconds(-1)) { continue }
                                    if (-not $readyAt -or $t -lt $readyAt) { $readyAt = $t; $readyLine = $line.Trim() }
                                } catch { }
                            }
                        }
                    }
                }
                if ($readyAt) { $liveDetected = $true; break }
                Start-Sleep -Milliseconds 25
            }
            $sw.Stop()
            $run['readyDetected'] = ($null -ne $readyAt)
            $run['livePollDetected'] = $liveDetected
            $run['readyLine'] = $readyLine
            $run['setupWarnings'] = @($warnings | Select-Object -Unique)
            $run['wallPollMs'] = [int]$sw.ElapsedMilliseconds
            if ($readyAt) {
                $run['readyAtUtc'] = $readyAt.ToString('o')
                $run['timeToReadyMs'] = [math]::Round(($readyAt - $procStartUtc).TotalMilliseconds, 1)
            }
            $run['ctrlQOwnedByApp'] = (Test-CtrlQOwned (9200 + $i))
            Start-Sleep -Milliseconds 500
            $run['metricsAfterReady'] = Get-ProcessMetrics $proc.Id
            $stopWatch = [Diagnostics.Stopwatch]::StartNew()
            Stop-App $proc.Id
            $gone = Wait-AppGone 15000
            $stopWatch.Stop()
            $run['stopped'] = $gone
            $run['stopMs'] = [int]$stopWatch.ElapsedMilliseconds
            $runList += ,$run
            Write-Host ("[s10] 冷启动 {0}/{1}：{2} ms（pid {3}，pidgone={4}）" -f $i, $Runs, $run['timeToReadyMs'], $proc.Id, $gone)
        }
        $valid = @($runList | Where-Object { $_.readyDetected -and $null -ne $_.timeToReadyMs } | ForEach-Object { [double]$_.timeToReadyMs } | Sort-Object)
        $pct = {
            param($sorted, [double]$p)
            if ($sorted.Count -eq 0) { return $null }
            if ($sorted.Count -eq 1) { return $sorted[0] }
            $rank = ($sorted.Count - 1) * $p
            $lo = [math]::Floor($rank); $hi = [math]::Ceiling($rank)
            if ($lo -eq $hi) { return $sorted[[int]$lo] }
            return $sorted[[int]$lo] + ($sorted[[int]$hi] - $sorted[[int]$lo]) * ($rank - $lo)
        }
        $summary['coldStart'] = [ordered]@{
            runs = $Runs
            validRuns = $valid.Count
            timeToReadyMs = [ordered]@{
                p50 = if ($valid.Count) { [math]::Round((& $pct $valid 0.50), 1) } else { $null }
                p95 = if ($valid.Count) { [math]::Round((& $pct $valid 0.95), 1) } else { $null }
                max = if ($valid.Count) { [math]::Round($valid[$valid.Count - 1], 1) } else { $null }
                min = if ($valid.Count) { [math]::Round($valid[0], 1) } else { $null }
                all = $valid
            }
            thresholdMs = 1500
            pass = ($valid.Count -gt 0 -and $valid[$valid.Count - 1] -le 1500)
            samples = $runList
        }
        $summary['coldStart'] | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path (Join-Path $rawDir ("coldstart-{0}.json" -f $Label))
        Write-Host ("[s10] 冷启动汇总（{0}）：valid={1}/{2} P50={3}ms P95={4}ms max={5}ms（阈值 1500ms）" -f $Label, $valid.Count, $Runs, $summary['coldStart'].timeToReadyMs.p50, $summary['coldStart'].timeToReadyMs.p95, $summary['coldStart'].timeToReadyMs.max)
    }

    if ($Mode -eq 'idle' -or $Mode -eq 'all') {
        if ($Label -eq 'debug') { Ensure-Vite }
        $prev = @(Get-Process -Name $procName -ErrorAction SilentlyContinue)
        foreach ($p in $prev) { Stop-App $p.Id }
        [void](Wait-AppGone 15000)
        Start-Sleep -Milliseconds 1500

        $proc = Start-Process -FilePath $workExe -WorkingDirectory $workDir -PassThru -ErrorAction Stop
        $procStartUtc = $proc.StartTime.ToUniversalTime()
        # 等待就绪（整文件读取 + 行内时间戳 >= 进程创建；理由同冷启动段注释）
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $readyLineIdle = $null
        while ($sw.ElapsedMilliseconds -lt 30000) {
            $logPath = Get-NewestLogPath $logDir
            if ($logPath) {
                $txt = Read-LogText $logPath
                if ($txt) {
                    foreach ($line in ($txt -split "`r?`n")) {
                        if ($line -match '初始化完成') {
                            $tsText = ($line -split '\s+')[0]
                            try {
                                $t = [DateTimeOffset]::Parse($tsText).UtcDateTime
                                if ($t -ge $procStartUtc.AddSeconds(-1)) { $readyLineIdle = $line.Trim(); break }
                            } catch { }
                        }
                    }
                }
            }
            if ($readyLineIdle) { break }
            Start-Sleep -Milliseconds 25
        }
        $sw.Stop()
        Write-Host ("[s10] 空闲测量：应用已就绪（{0} ms），预热 {1}s 后开始 {2} 分钟采样（每 {3}s）" -f [int]$sw.ElapsedMilliseconds, $WarmupSec, $IdleMinutes, $SampleSec)
        Start-Sleep -Seconds $WarmupSec

        $samples = @()
        $sampleCount = [int](($IdleMinutes * 60) / $SampleSec)
        $prevCpu = $null
        $prevWall = $null
        $stop = $false
        for ($k = 1; $k -le $sampleCount; $k += 1) {
            $loopStart = Get-Date
            $metrics = Get-ProcessMetrics $proc.Id
            $sys = Get-SystemSnapshot
            if (-not $metrics) { Write-Warning '[s10] 采样期间应用进程退出，终止空闲采样'; break }
            $now = Get-Date
            if ($prevCpu -ne $null) {
                $elapsed = ($now - $prevWall).TotalMilliseconds
                $deltaCpu = ($metrics.treeCpuMs - $prevCpu)
                $metrics['deltaCpuMs'] = $deltaCpu
                $metrics['elapsedMs'] = $elapsed
                $metrics['cpuPercentOfOneCore'] = if ($elapsed -gt 0) { [math]::Round($deltaCpu / $elapsed * 100, 4) } else { $null }
                $metrics['cpuPercentOfSystem'] = if ($elapsed -gt 0) { [math]::Round(($deltaCpu / $elapsed / [Environment]::ProcessorCount) * 100, 4) } else { $null }
            }
            $metrics['system'] = $sys
            $samples += ,$metrics
            $prevCpu = $metrics.treeCpuMs
            $prevWall = $now
            if ($k % 30 -eq 0 -or $k -eq 1) {
                Write-Host ("[s10] 采样 {0}/{1}：mainWS={2:N1}MiB privateWS={3} treeWS={4:N1}MiB sysCpu={5}%" -f $k, $sampleCount, ($metrics.workingSetBytes / 1MB), $(if ($metrics.privateWorkingSetBytes) { [math]::Round($metrics.privateWorkingSetBytes / 1MB, 1) } else { '-' }), ($metrics.treeWorkingSetBytes / 1MB), $sys.cpuPercentTotal)
            }
            $spentMs = ((Get-Date) - $loopStart).TotalMilliseconds
            $remainingMs = ($SampleSec * 1000) - $spentMs
            if ($remainingMs -gt 0) { Start-Sleep -Milliseconds ([int]$remainingMs) }
        }
        Stop-App $proc.Id
        [void](Wait-AppGone 15000)

        $series = { param($prop) @($samples | ForEach-Object { $_[$prop] } | Where-Object { $_ -ne $null }) }
        $stat = {
            param($values)
            $v = @($values | Sort-Object)
            if ($v.Count -eq 0) { return [ordered]@{ count = 0; p50 = $null; p95 = $null; max = $null; mean = $null } }
            $idx50 = [math]::Floor(($v.Count - 1) * 0.5); $idx95 = [math]::Floor(($v.Count - 1) * 0.95)
            $mean = ($v | Measure-Object -Average).Average
            return [ordered]@{
                count = $v.Count
                p50 = $v[[int]$idx50]
                p95 = $v[[int]$idx95]
                max = $v[$v.Count - 1]
                mean = [math]::Round($mean, 2)
            }
        }
        $ws = & $stat (& $series 'workingSetBytes')
        $pws = & $stat (& $series 'privateWorkingSetBytes')
        $priv = & $stat (& $series 'privateMemoryBytes')
        $treeWs = & $stat (& $series 'treeWorkingSetBytes')
        $cpuSys = & $stat ((& $series 'cpuPercentOfSystem') | Where-Object { $_ -ne $null })
        $cpuCore = & $stat ((& $series 'cpuPercentOfOneCore') | Where-Object { $_ -ne $null })
        $sysCpuSeries = & $series 'system'
        $sysCpuValues = @($sysCpuSeries | ForEach-Object { $_['cpuPercentTotal'] } | Where-Object { $_ -ne $null })

        $summary['idle'] = [ordered]@{
            idleMinutes = $IdleMinutes
            sampleSec = $SampleSec
            scheduledSamples = $sampleCount
            actualSamples = $samples.Count
            memory = [ordered]@{
                workingSetBytes = $ws
                privateWorkingSetBytes = $pws
                privateMemoryBytes = $priv
                treeWorkingSetBytes = $treeWs
                thresholdMiB = 120
                mainWorkingSetMiB = [ordered]@{ p50 = if ($ws.p50) { [math]::Round($ws.p50 / 1MB, 2) } else { $null }; p95 = if ($ws.p95) { [math]::Round($ws.p95 / 1MB, 2) } else { $null }; max = if ($ws.max) { [math]::Round($ws.max / 1MB, 2) } else { $null } }
                mainPrivateWorkingSetMiB = [ordered]@{ p50 = if ($pws.p50) { [math]::Round($pws.p50 / 1MB, 2) } else { $null }; p95 = if ($pws.p95) { [math]::Round($pws.p95 / 1MB, 2) } else { $null }; max = if ($pws.max) { [math]::Round($pws.max / 1MB, 2) } else { $null } }
                treeWorkingSetMiB = [ordered]@{ p50 = if ($treeWs.p50) { [math]::Round($treeWs.p50 / 1MB, 2) } else { $null }; p95 = if ($treeWs.p95) { [math]::Round($treeWs.p95 / 1MB, 2) } else { $null }; max = if ($treeWs.max) { [math]::Round($treeWs.max / 1MB, 2) } else { $null } }
            }
            cpu = [ordered]@{
                percentOfSystem = $cpuSys
                percentOfOneCore = $cpuCore
                thresholdPercentOfSystem = 0.5
                pass = ($cpuSys.mean -ne $null -and $cpuSys.mean -lt 0.5)
            }
            systemCpuPercent = [ordered]@{
                max = if ($sysCpuValues.Count) { ($sysCpuValues | Measure-Object -Maximum).Maximum } else { $null }
                mean = if ($sysCpuValues.Count) { [math]::Round((($sysCpuValues | Measure-Object -Average).Average), 2) } else { $null }
            }
            memoryPass = ($ws.max -ne $null -and ($ws.max / 1MB) -le 120)
            samples = $samples
        }
        $summary['idle'] | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path (Join-Path $rawDir ("idle-{0}.json" -f $Label))
        Write-Host ("[s10] 空闲汇总（{0}）：mainWS p95={1}MiB max={2}MiB（阈值 120MiB）；CPU(system) max={3}% mean={4}%（阈值 <0.5%）；samples={5}" -f $Label, $summary['idle'].memory.mainWorkingSetMiB.p95, $summary['idle'].memory.mainWorkingSetMiB.max, $summary['idle'].cpu.percentOfSystem.max, $summary['idle'].cpu.percentOfSystem.mean, $samples.Count)
    }
} finally {
    Stop-Vite
    foreach ($p in @(Get-Process -Name $procName -ErrorAction SilentlyContinue)) { Stop-App $p.Id }
    $summary['finishedAt'] = (Get-Date).ToString('o')
    $summaryPath = Join-Path $OutDir ("summary-{0}-{1}.json" -f $Mode, $Label)
    Write-Utf8Json -Path $summaryPath -Object $summary
    Write-Host "[s10] 汇总：$summaryPath"
}
