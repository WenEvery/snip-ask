# S-16 ③ 真实安装 / 启动 / 卸载与残留检查（未签名 NSIS，currentUser）
#
# 流程：
#   1) 安装前系统状态快照（卸载注册表项 / 安装目录 / 开始菜单与桌面快捷方式 / HKCU Run / 应用数据目录）；
#   2) 用「带 MOTW 的安装包副本」静默安装（/S），记录是否被阻断、耗时、退出码；
#      若未成功安装则用不带 MOTW 的原始安装包重试（用于区分「MOTW/SmartScreen 阻断」与「安装器本身失败」）；
#   3) 安装结果核验（注册表项、安装目录文件、快捷方式、已安装主 exe 签名）；
#   4) 启动已安装应用：从进程创建到日志出现「框选 AI 初始化完成」的时间、5s 后内存/CPU、WebView2 子进程；
#      然后关闭（先 CloseMainWindow，5s 不退再 Stop-Process，记录是否强杀）；
#   5) 静默卸载（uninstall.exe /S），轮询卸载完成；
#   6) 卸载后快照 + 残留差异（安装目录/注册表/快捷方式/Run 项/应用数据）。
#
# 安全：默认安装到 currentUser 默认目录 %LOCALAPPDATA%\SnipAsk（与本地数据目录同名，见记录 §风险）；
#       不删除既有用户数据；既有 %APPDATA%\SnipAsk 会被应用正常读写（安装版数据目录），只做前后差异记录。
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s16_install_test.ps1
#   ... -SetupExe <path> -OutDir <dir> [-SkipUninstall] [-InstallDir <dir>]
#
# 退出码：0 = 安装/启动/卸载核心步骤均完成（可能有遗留项，写入 JSON）；非 0 = 关键步骤失败（见 JSON）。

param(
    [string]$Root,
    [string]$SetupExe,
    [string]$OutDir,
    [string]$InstallDir,
    [switch]$SkipUninstall
)

$ErrorActionPreference = 'Stop'

if (-not $Root) { $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path }
$Root = (Resolve-Path $Root).Path
if (-not $SetupExe) {
    $SetupExe = Get-ChildItem -Path (Join-Path $Root 'src-tauri\target\release\bundle\nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $OutDir) { $OutDir = Join-Path $Root '.verify\spikes\S-16-S-18\s16' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Write-Utf8Json([string]$Path, $Object) {
    $Object | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 -Path $Path
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

function Get-SnipAskProcesses {
    return @(Get-Process -Name 'snip-ask' -ErrorAction SilentlyContinue)
}

function Get-UninstallEntries {
    $keys = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    $entries = @()
    foreach ($k in $keys) {
        foreach ($item in (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue)) {
            if ($item.DisplayName -and $item.DisplayName -match 'SnipAsk') {
                $entries += [ordered]@{
                    key              = $item.PSPath -replace '^Microsoft\.PowerShell\.Core\\Registry::', ''
                    displayName      = $item.DisplayName
                    displayVersion   = $item.DisplayVersion
                    publisher        = $item.Publisher
                    installLocation  = $item.InstallLocation
                    uninstallString  = $item.UninstallString
                    estimatedSize    = $item.EstimatedSize
                }
            }
        }
    }
    return ,$entries
}

function Get-DirListing([string]$Path, [int]$MaxDepth = 1) {
    if (-not (Test-Path $Path)) { return $null }
    $items = Get-ChildItem -Path $Path -Force -Recurse -ErrorAction SilentlyContinue |
        Where-Object {
            $rel = $_.FullName.Substring($Path.Length).TrimStart('\')
            $level = ($rel.Split('\').Count - 1)
            return ($level -le $MaxDepth)
        }
    $totalBytes = ($items | Measure-Object -Property Length -Sum -ErrorAction SilentlyContinue).Sum
    return [ordered]@{
        exists      = $true
        itemCount   = @($items).Count
        totalBytes  = if ($totalBytes) { [int64]$totalBytes } else { 0 }
        topLevel    = @(Get-ChildItem -Path $Path -Force -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
    }
}

function Get-Shortcuts([string]$Pattern) {
    $roots = @(
        (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
        (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'),
        [Environment]::GetFolderPath('Desktop'),
        [Environment]::GetFolderPath('CommonDesktopDirectory')
    )
    $found = @()
    foreach ($r in $roots) {
        if (Test-Path $r) {
            $found += @(Get-ChildItem -Path $r -Recurse -Filter '*.lnk' -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match $Pattern } | Select-Object -ExpandProperty FullName)
        }
    }
    return ,$found
}

function Get-RunValue([string]$Name) {
    try {
        $v = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $Name -ErrorAction Stop
        return $v.$Name
    } catch { return $null }
}

function Get-StateSnapshot([string]$Phase) {
    $procs = Get-SnipAskProcesses
    return [ordered]@{
        phase            = $Phase
        timestamp        = (Get-Date).ToString('o')
        uninstallEntries = Get-UninstallEntries
        installDirDefault = Get-DirListing (Join-Path $env:LOCALAPPDATA 'SnipAsk')
        startMenu        = Get-Shortcuts 'SnipAsk'
        runValue         = [ordered]@{ name = 'SnipAsk'; value = (Get-RunValue 'SnipAsk') }
        appDataRoaming   = [ordered]@{ path = (Join-Path $env:APPDATA 'SnipAsk'); listing = Get-DirListing (Join-Path $env:APPDATA 'SnipAsk') }
        appDataLocal     = [ordered]@{ path = (Join-Path $env:LOCALAPPDATA 'SnipAsk'); listing = Get-DirListing (Join-Path $env:LOCALAPPDATA 'SnipAsk') }
        webviewLocal     = [ordered]@{ path = (Join-Path $env:LOCALAPPDATA 'com.snipask.app'); exists = (Test-Path (Join-Path $env:LOCALAPPDATA 'com.snipask.app')) }
        webviewRoaming   = [ordered]@{ path = (Join-Path $env:APPDATA 'com.snipask.app'); exists = (Test-Path (Join-Path $env:APPDATA 'com.snipask.app')) }
        processes        = @($procs | Select-Object Id, ProcessName, StartTime)
    }
}

function Wait-Until([scriptblock]$Condition, [int]$TimeoutMs = 60000, [int]$IntervalMs = 500) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds $IntervalMs
    }
    return $false
}

function Get-NewestLogFile([string]$Dir) {
    if (-not (Test-Path $Dir)) { return $null }
    return Get-ChildItem -Path $Dir -Filter 'snipask.log*' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

$result = [ordered]@{
    spike = 'S-16'
    step = 'install / launch / uninstall'
    timestamp = (Get-Date).ToString('o')
    setupExe = $SetupExe
    setupSha256 = if (Test-Path $SetupExe) { (Get-FileHash -Algorithm SHA256 -Path $SetupExe).Hash } else { $null }
    environment = [ordered]@{
        osCaption = (Get-CimInstance Win32_OperatingSystem).Caption
        osBuild = (Get-CimInstance Win32_OperatingSystem).BuildNumber
        user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        isAdmin = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        installMode = 'currentUser'
        requestedInstallDir = $InstallDir
    }
    steps = [ordered]@{}
    verdict = [ordered]@{}
    residue = [ordered]@{}
}

# ---------- 0) 清理环境 ----------
foreach ($p in Get-SnipAskProcesses) {
    Write-Warning "[s16] 测量前发现残留 snip-ask 进程 pid=$($p.Id)，先停止"
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 1

$stateBefore = Get-StateSnapshot 'before-install'
$result.steps['before'] = $stateBefore

# ---------- 1) 安装（先 MOTW 副本，失败则原包重试） ----------
$motwCopy = Join-Path (Join-Path $OutDir 'motw') (Split-Path -Leaf $SetupExe)
if (-not (Test-Path $motwCopy)) {
    Write-Warning "[s16] 未找到 MOTW 副本（$motwCopy），先运行 s16_scan_signature.ps1；本次将退化为不带 MOTW 的安装"
}
$installArgs = @('/S')
if ($InstallDir) { $installArgs += "/D=$InstallDir" }

function Invoke-Installer([string]$InstallerPath, [string]$Label) {
    $log = [ordered]@{ label = $Label; installer = $InstallerPath; args = ($installArgs -join ' ') }
    if (-not (Test-Path $InstallerPath)) { $log['error'] = 'installer not found'; return $log }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $InstallerPath -ArgumentList $installArgs -PassThru -ErrorAction Stop
    $log['pid'] = $proc.Id
    $exited = $proc.WaitForExit(180000)
    $sw.Stop()
    $log['exitedWithinTimeout'] = $exited
    $log['durationMs'] = [int]$sw.ElapsedMilliseconds
    try { $log['exitCode'] = $proc.ExitCode } catch { $log['exitCode'] = $null }
    # 安装器可能在安装完成后才退出；以注册表/目录出现为准（最多再等 60s）
    $installed = Wait-Until -TimeoutMs 60000 -Condition {
        $dirOk = Test-Path (Join-Path $env:LOCALAPPDATA 'SnipAsk\snip-ask.exe')
        if ($InstallDir) { $dirOk = (Test-Path (Join-Path $InstallDir 'snip-ask.exe')) }
        $regOk = ((Get-UninstallEntries).Count -gt 0)
        return ($dirOk -or $regOk)
    }
    $log['installedDetected'] = $installed
    # 若检测到已安装但安装器进程仍未退出（例如被 SmartScreen/兼容性提示挂起），终止该安装器进程，
    # 避免后续步骤与残留进程互相干扰（安装本身已完成）。
    if ($installed) {
        $p2 = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
        if ($p2) {
            $log['installerTerminatedAfterDetection'] = $true
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            foreach ($c in (Get-ProcessTree $proc.Id)) {
                Stop-Process -Id ([int]$c.ProcessId) -Force -ErrorAction SilentlyContinue
            }
        }
    }
    return $log
}

$installLog = $null
if (Test-Path $motwCopy) {
    $installLog = Invoke-Installer $motwCopy 'motw'
    $result.steps['installWithMotw'] = $installLog
}
if (-not ($installLog -and $installLog.installedDetected)) {
    if (Test-Path $motwCopy) {
        $result.verdict['motwInstallBlockedOrFailed'] = $true
        Write-Warning '[s16] 带 MOTW 的静默安装未成功 → 记录为 MOTW 场景失败/疑似被 SmartScreen 阻断，改用原始安装包重试'
    }
    $installLog = Invoke-Installer $SetupExe 'plain'
    $result.steps['installPlain'] = $installLog
}
$result.verdict['installed'] = [bool]($installLog -and $installLog.installedDetected)
if (-not $result.verdict['installed']) {
    $result.verdict['failure'] = '安装未被检测到（注册表/安装目录均未出现）'
    Write-Utf8Json -Path (Join-Path $OutDir 'install_uninstall.json') -Object $result
    Write-Error '[s16] 安装失败，终止后续步骤（详见 install_uninstall.json）'
    exit 2
}

# ---------- 2) 安装结果核验 ----------
$entries = Get-UninstallEntries
$entry = if ($entries.Count -gt 0) { $entries[0] } else { $null }
$resolvedInstallDir = if ($InstallDir) { $InstallDir }
    elseif ($entry -and $entry.installLocation) { $entry.installLocation.Trim('"') }
    else { Join-Path $env:LOCALAPPDATA 'SnipAsk' }
$installedExe = Join-Path $resolvedInstallDir 'snip-ask.exe'
$installedUninstaller = Join-Path $resolvedInstallDir 'uninstall.exe'
$stateAfterInstall = Get-StateSnapshot 'after-install'
$result.steps['afterInstall'] = $stateAfterInstall
$result.steps['installSummary'] = [ordered]@{
    installDir       = $resolvedInstallDir
    installDirListing = Get-DirListing $resolvedInstallDir
    installedExe     = if (Test-Path $installedExe) { [ordered]@{
        path = $installedExe; bytes = (Get-Item $installedExe).Length
        sha256 = (Get-FileHash -Algorithm SHA256 -Path $installedExe).Hash
        signatureStatus = (Get-AuthenticodeSignature -FilePath $installedExe).Status.ToString()
        fileVersion = (Get-Item $installedExe).VersionInfo.FileVersion
    } } else { $null }
    uninstaller      = if (Test-Path $installedUninstaller) { [ordered]@{
        path = $installedUninstaller; bytes = (Get-Item $installedUninstaller).Length
        signatureStatus = (Get-AuthenticodeSignature -FilePath $installedUninstaller).Status.ToString()
    } } else { $null }
    uninstallEntry   = $entry
    startMenu        = Get-Shortcuts 'SnipAsk'
    runValueAfterInstall = (Get-RunValue 'SnipAsk')
}

# ---------- 3) 启动已安装应用 ----------
$launch = [ordered]@{}
if (Test-Path $installedExe) {
    $logDir = Join-Path $env:APPDATA 'SnipAsk\logs'
    $logBefore = Get-NewestLogFile $logDir
    $logOffsetBefore = if ($logBefore) { $logBefore.Length } else { 0 }
    $launch['logFileBefore'] = if ($logBefore) { $logBefore.FullName } else { $null }
    $launch['logBytesBefore'] = $logOffsetBefore

    $mount = Start-Process -FilePath $installedExe -PassThru -ErrorAction Stop
    $procStartUtc = $mount.StartTime.ToUniversalTime()
    $launch['pid'] = $mount.Id
    $launch['processStartLocal'] = $mount.StartTime.ToString('o')

    # 轮询日志中「框选 AI 初始化完成」出现时刻
    $readyAt = $null
    $readyLine = $null
    $logFileAtReady = $null
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 40000) {
        $logNow = Get-NewestLogFile $logDir
        if ($logNow) {
            $len = $logNow.Length
            $from = 0
            if ($logBefore -and $logNow.FullName -eq $logBefore.FullName) { $from = $logOffsetBefore }
            if ($len -gt $from) {
                $fs = [System.IO.File]::Open($logNow.FullName, 'Open', 'Read', 'ReadWrite')
                try {
                    [void]$fs.Seek($from, 'Begin')
                    $reader = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
                    $appended = $reader.ReadToEnd()
                } finally { $fs.Dispose() }
                $match = ($appended -split "`r?`n") | Where-Object { $_ -match '初始化完成' } | Select-Object -Last 1
                if ($match) {
                    $tsText = ($match -split '\s+')[0]
                    try {
                        $readyAt = [DateTimeOffset]::Parse($tsText).UtcDateTime
                        $readyLine = $match
                        $logFileAtReady = $logNow.FullName
                        break
                    } catch { }
                }
            }
        }
        Start-Sleep -Milliseconds 25
    }
    $sw.Stop()
    $launch['readyDetected'] = ($null -ne $readyAt)
    $launch['readyLine'] = $readyLine
    $launch['logFileAtReady'] = $logFileAtReady
    $launch['pollBudgetMs'] = 40000
    if ($readyAt) {
        $launch['readyAtUtc'] = $readyAt.ToString('o')
        $launch['timeToReadyMs'] = [math]::Round(($readyAt - $procStartUtc).TotalMilliseconds, 1)
        $launch['timeToReadyFromNowMs'] = [int]$sw.ElapsedMilliseconds
    }

    # 5s 后资源快照（空闲基线）
    Start-Sleep -Seconds 5
    $proc = Get-Process -Id $mount.Id -ErrorAction SilentlyContinue
    if ($proc) {
        $children = Get-ProcessTree $mount.Id
        $childProcs = @()
        foreach ($c in $children) {
            $cp = Get-Process -Id ([int]$c.ProcessId) -ErrorAction SilentlyContinue
            if ($cp) {
                $childProcs += [ordered]@{
                    pid = $cp.Id; name = $cp.ProcessName
                    workingSetBytes = [int64]$cp.WorkingSet64
                    privateBytes = [int64]$cp.PrivateMemorySize64
                }
            }
        }
        $launch['after5s'] = [ordered]@{
            mainWorkingSetBytes = [int64]$proc.WorkingSet64
            mainPrivateBytes = [int64]$proc.PrivateMemorySize64
            threadCount = $proc.Threads.Count
            handleCount = $proc.HandleCount
            totalCpuMs = [int64]$proc.TotalProcessorTime.TotalMilliseconds
            childProcesses = $childProcs
            childCount = $childProcs.Count
            treeWorkingSetBytes = [int64](($proc.WorkingSet64) + (($childProcs | Measure-Object -Property workingSetBytes -Sum).Sum))
        }
    } else {
        $launch['after5s'] = [ordered]@{ error = 'process exited before 5s snapshot' }
    }

    # 关闭：优先 CloseMainWindow，5s 不退则强杀
    $closed = $false
    try { $closed = $proc.CloseMainWindow() } catch { $closed = $false }
    $launch['closeMainWindowSent'] = $closed
    $exited = Wait-Until -TimeoutMs 8000 -IntervalMs 250 -Condition { -not (Get-Process -Id $mount.Id -ErrorAction SilentlyContinue) }
    $launch['gracefulExit'] = $exited
    if (-not $exited) {
        Stop-Process -Id $mount.Id -Force -ErrorAction SilentlyContinue
        $launch['forcedKill'] = $true
        Start-Sleep -Seconds 1
        # 清理 WebView2 子进程（若父进程已终止但子进程残留）
        foreach ($c in (Get-ProcessTree $mount.Id)) {
            Stop-Process -Id ([int]$c.ProcessId) -Force -ErrorAction SilentlyContinue
        }
    } else {
        $launch['forcedKill'] = $false
    }
    $launch['exited'] = -not (Get-Process -Id $mount.Id -ErrorAction SilentlyContinue)
    $launch['runtimeSeconds'] = [math]::Round(((Get-Date) - $mount.StartTime).TotalSeconds, 1)
} else {
    $launch['error'] = "installed exe not found: $installedExe"
}
$result.steps['launch'] = $launch

# ---------- 4) 静默卸载 ----------
$uninstall = [ordered]@{}
if (-not $SkipUninstall) {
    # 卸载器可能在应用运行时中止，确保已退出
    foreach ($p in Get-SnipAskProcesses) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 2
    $uninstaller = if (Test-Path $installedUninstaller) { $installedUninstaller }
        elseif ($entry -and $entry.uninstallString) { ($entry.uninstallString -replace '^"|"$', '') }
        else { $null }
    $uninstall['uninstaller'] = $uninstaller
    if ($uninstaller -and (Test-Path $uninstaller)) {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $up = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -ErrorAction Stop
        $uninstall['pid'] = $up.Id
        $exited = $up.WaitForExit(180000)
        $uninstall['exitedWithinTimeout'] = $exited
        try { $uninstall['exitCode'] = $up.ExitCode } catch { $uninstall['exitCode'] = $null }
        # NSIS 卸载器可能把自身复制到 %TEMP% 再执行，等待其实际清理由状态判定
        $removed = Wait-Until -TimeoutMs 120000 -IntervalMs 500 -Condition {
            $entryGone = ((Get-UninstallEntries).Count -eq 0)
            $exeGone = -not (Test-Path $installedExe)
            if ($InstallDir) { $dirGone = -not (Test-Path $InstallDir) } else { $dirGone = $null }
            return ($entryGone -and $exeGone)
        }
        $sw.Stop()
        $uninstall['removalDetected'] = $removed
        $uninstall['durationMs'] = [int]$sw.ElapsedMilliseconds
        $uninstall['processesLeftAfterUninstall'] = @(Get-SnipAskProcesses | Select-Object Id, ProcessName)
    } else {
        $uninstall['error'] = 'uninstaller not found'
    }
    $stateAfterUninstall = Get-StateSnapshot 'after-uninstall'
    $result.steps['afterUninstall'] = $stateAfterUninstall
}
$result.steps['uninstall'] = $uninstall

# ---------- 5) 残留差异 ----------
$stateEnd = Get-StateSnapshot 'end'
$result.steps['end'] = $stateEnd
$before = $stateBefore
$after = $stateEnd

$residue = [ordered]@{
    uninstallEntryLeft = (Get-UninstallEntries)
    installDirLeft = [ordered]@{
        defaultDir = (Join-Path $env:LOCALAPPDATA 'SnipAsk')
        exists = (Test-Path (Join-Path $env:LOCALAPPDATA 'SnipAsk'))
        listing = Get-DirListing (Join-Path $env:LOCALAPPDATA 'SnipAsk') 1
    }
    startMenuLeft = Get-Shortcuts 'SnipAsk'
    runValueLeft = (Get-RunValue 'SnipAsk')
    appDataRoamingExists = (Test-Path (Join-Path $env:APPDATA 'SnipAsk'))
    appDataLocalExists = (Test-Path (Join-Path $env:LOCALAPPDATA 'SnipAsk'))
    webviewLocalExists = (Test-Path (Join-Path $env:LOCALAPPDATA 'com.snipask.app'))
    webviewRoamingExists = (Test-Path (Join-Path $env:APPDATA 'com.snipask.app'))
    note = '安装目录/注册表/快捷方式应被卸载器清除；%APPDATA%\SnipAsk 与 %LOCALAPPDATA%\SnipAsk（安装版数据目录）按设计保留（应用数据），本次只记录差异、不删除。'
}
$residue['appDataRoamingNewFiles'] = @()
if ($after.appDataRoaming.listing -and $before.appDataRoaming.listing) {
    $residue['appDataRoamingNewFiles'] = @($after.appDataRoaming.listing.topLevel | Where-Object { $before.appDataRoaming.listing.topLevel -notcontains $_ })
}
$result.residue = $residue

# ---------- 判定 ----------
$result.verdict['corePass'] = [bool]($result.verdict['installed'] -and (($launch['readyDetected']) -or ($launch['after5s'] -and -not $launch['after5s']['error'])))
$result.verdict['installBlocked'] = -not $result.verdict['installed']
$result.verdict['motwBlockedSuspect'] = [bool]($result.verdict['motwInstallBlockedOrFailed'] -and $result.verdict['installed'])
if ($result.verdict['motwBlockedSuspect']) {
    $result.verdict['motwNote'] = '带 MOTW 的静默安装未成功，但无 MOTW 的同一安装包成功 → MOTW/SmartScreen 场景下需要人工交互确认（或静默被阻断），列入遗留。'
}
if (-not $SkipUninstall) {
    $result.verdict['uninstallClean'] = [bool](
        ((Get-UninstallEntries).Count -eq 0) -and
        (-not (Test-Path $installedExe)) -and
        (@(Get-Shortcuts 'SnipAsk').Count -eq 0) -and
        -not (Get-RunValue 'SnipAsk')
    )
}

$jsonPath = Join-Path $OutDir 'install_uninstall.json'
Write-Utf8Json -Path $jsonPath -Object $result

Write-Host "[s16] 安装：$($result.verdict['installed']) dir=$resolvedInstallDir"
Write-Host ("[s16] 启动：readyDetected={0} timeToReady={1}ms after5sWS={2:N1}MiB" -f $launch['readyDetected'], $launch['timeToReadyMs'], $(if ($launch['after5s'] -and $launch['after5s'].mainWorkingSetBytes) { $launch['after5s'].mainWorkingSetBytes / 1MB } else { 0 }))
Write-Host ("[s16] 卸载：uninstallClean={0} removalDetected={1}" -f $result.verdict['uninstallClean'], $uninstall['removalDetected'])
Write-Host "[s16] 结果：$jsonPath"

if (-not $result.verdict['corePass']) { exit 3 }
exit 0
