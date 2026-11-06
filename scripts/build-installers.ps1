#Requires -Version 5.1
<#
.SYNOPSIS
    框选 AI · 双版本安装包构建脚本（v1.0.0，T5）。

.DESCRIPTION
    构建两个 NSIS 安装包：
      slim：内置 ONNX Runtime 运行时 + 应用 + 文档，不含 OCR 模型（首次用 OCR 时按 manifest 下载）；
      full：slim 的全部内容 + 4 个 OCR 模型/字典（离线开箱即用）。

    流程：
      0) 校验工具链 / 版本 / 必需文档资源；
      1) 运行 scripts/fetch-ocr-assets.ps1 准备 src-tauri/resources/ocr/ 资产（幂等 + SHA256 强校验）；
      2) 对每个版本：`cargo clean -p snip-ask`（保证 option_env!(SNIP_ASK_EDITION) 重新编译）→
         `$env:SNIP_ASK_EDITION=<edition>` → `pnpm tauri build --config src-tauri/tauri.<edition>.conf.json`；
      3) 产物改名归档到 release/：SnipAsk_<ver>_x64-setup.exe（slim）、SnipAsk_<ver>_x64-full-setup.exe（full）；
      4) 体积门禁：slim ≤ 25 MiB，full ≤ 60 MiB；生成 release/SHA256SUMS.txt；
      5) 默认执行真实安装冒烟（-SkipSmoke 可跳过）：静默安装 /S → 校验安装目录资源布局 → 启动 6s
         （进程存活 + 主窗口可见 + 截图）→ 关闭应用及本应用 llama-server 子进程 → 写入模型探针 →
         静默卸载 /S → 残留检查（卸载退出码、%LOCALAPPDATA%\SnipAsk、%APPDATA%\SnipAsk、
         %LOCALAPPDATA%\com.snipask.app、模型探针），并作为 pass 门禁写入 smoke JSON。
         冒烟前把用户现有 %APPDATA%\SnipAsk、%LOCALAPPDATA%\SnipAsk、WebView2 数据目录、
         HKCU Run 值与安装器元数据键（HKCU\Software\SnipAsk\SnipAsk）临时移走
         （目录为同卷重命名，快速且不破坏数据），冒烟结束（含失败）在 finally 中恢复。
      6) 构建日志 / 体积 / SHA256 / 冒烟 JSON / 截图 → .verify/v1/installer/。

.PARAMETER Edition
    slim / full / both（默认 both）。

.PARAMETER SkipBuild
    跳过 cargo/tauri 构建，只用 release/ 下已有安装包做体积门禁与冒烟。

.PARAMETER SkipSmoke
    跳过真实安装冒烟（不推荐用于最终交付）。

.PARAMETER AllowMissingDocs
    开发期应急：允许 docs 资源缺失，自动生成只映射已存在文档的临时 overlay（产物不可作为最终交付）。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-installers.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-installers.ps1 -Edition slim -SkipSmoke
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-installers.ps1 -SkipBuild -Edition both

.NOTES
    退出码：0 = 全部通过；1 = 构建/体积门禁失败；2 = 安装冒烟失败；3 = 前置条件不满足（文档/资产/工具链）。
#>
[CmdletBinding()]
param(
    [ValidateSet('slim', 'full', 'both')]
    [string]$Edition = 'both',
    [string]$RepoRoot = '',
    [string]$OutputDir = '',
    [bool]$RequireDocs = $true,
    [switch]$SkipFetch,
    [switch]$SkipBuild,
    [switch]$SkipSmoke,
    [switch]$AllowMissingDocs,
    [switch]$KeepOldArtifacts,
    [switch]$NoClean,
    [int]$SmokeSeconds = 6
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$SlimThresholdMiB = 25
$FullThresholdMiB = 60
$SlimThresholdBytes = [int64]$SlimThresholdMiB * 1MB
$FullThresholdBytes = [int64]$FullThresholdMiB * 1MB

# 安装目录 docs/ 资源（源路径相对仓库根，目标相对安装目录）。顺序即 overlay 中映射顺序。
$DocsResources = @(
    [pscustomobject]@{ Source = 'docs\README.md'; Target = 'docs/README.md'; Required = $true }
    [pscustomobject]@{ Source = 'docs\自定义指南.md'; Target = 'docs/自定义指南.md'; Required = $true }
    [pscustomobject]@{ Source = 'LICENSE'; Target = 'docs/LICENSE'; Required = $true }
    [pscustomobject]@{ Source = 'NOTICE'; Target = 'docs/NOTICE'; Required = $true }
    [pscustomobject]@{ Source = 'SECURITY.md'; Target = 'docs/SECURITY.md'; Required = $true }
    [pscustomobject]@{ Source = 'PRIVACY.md'; Target = 'docs/PRIVACY.md'; Required = $true }
)

# full 版额外随包的 OCR 资产（文件名与 spec §4 一致）
$FullModelAssets = @(
    'ch_PP-OCRv4_det_infer.onnx'
    'ch_PP-OCRv4_rec_infer.onnx'
    'ch_ppocr_mobile_v2.0_cls_infer.onnx'
    'ppocr_keys_v1.txt'
)
$RuntimeAsset = 'onnxruntime.dll'

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
    $json = $Object | ConvertTo-Json -Depth 20
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding($false)))
}

# 把 PowerShell 脚本 shim（如 npm 全局安装的 pnpm.ps1）解析为原生 .cmd/.exe：
# Windows PowerShell 5.1 下把数组参数传给 .ps1 shim 会被折叠成单个参数（pnpm 会报 ".\tauri 不是命令"），
# 传原生 .cmd 则按数组展开，行为与 cmd 中一致。
function Resolve-NativeExe([string]$Name) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $cmd) { return $Name }
    $src = $cmd.Source
    if ($src -and $src -match '\.ps1$') {
        foreach ($ext in @('.cmd', '.exe')) {
            $alt = [System.IO.Path]::ChangeExtension($src, $ext)
            if (Test-Path -LiteralPath $alt) { return $alt }
        }
    }
    if ($src) { return $src }
    return $Name
}

function Invoke-Logged([string]$WorkDir, [string]$LogPath, [string]$Label, [string[]]$Command) {
    $exe = Resolve-NativeExe $Command[0]
    $display = @($exe) + @($Command[1..($Command.Count - 1)])
    Write-Host ("      $ {0}" -f ($display -join ' ')) -ForegroundColor DarkGray
    Add-Content -LiteralPath $LogPath -Value ("`n### {0} :: {1}" -f (Get-Date).ToString('o'), ($display -join ' ')) -Encoding UTF8
    Push-Location -LiteralPath $WorkDir
    $previousEap = $ErrorActionPreference
    try {
        # 外部命令的 stderr（cargo/tauri 的 warning 等）不能被当作终止错误
        $ErrorActionPreference = 'Continue'
        $nativeArgs = @($Command[1..($Command.Count - 1)])
        & $exe @nativeArgs 2>&1 | ForEach-Object {
            $line = if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.ToString() } else { [string]$_ }
            Write-Host ("      {0}" -f $line) -ForegroundColor DarkGray
            Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8
        }
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousEap
        Pop-Location
    }
}

function Get-UninstallEntries {
    $entries = @()
    foreach ($k in @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*')) {
        foreach ($item in (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue)) {
            if ($item.DisplayName -and $item.DisplayName -match 'SnipAsk') { $entries += $item }
        }
    }
    return , $entries
}

function Get-SnipAskProcesses {
    return @(Get-Process -Name 'snip-ask' -ErrorAction SilentlyContinue)
}

# 本应用本地引擎子进程：llama-server.exe 位于 %APPDATA%\SnipAsk\backends\<runtime>\ 下。
# 冒烟卸载前必须终止它，否则它持有的 models\*.gguf 会让卸载钩子的 RMDir /r 只能落
# /REBOOTOK，静默卸载后立即断言会看到「残留」。只按路径/命令行精确匹配本应用目录，
# 绝不误杀其它应用的 llama-server（LM Studio / Ollama 等）。
function Get-SnipAskBackendProcesses {
    $found = @()
    foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name = 'llama-server.exe'" -ErrorAction SilentlyContinue)) {
        $exe = [string]$p.ExecutablePath
        $cmd = [string]$p.CommandLine
        $isOurs = ($exe -like '*\SnipAsk\backends\*') -or ($cmd -like '*\SnipAsk\backends\*')
        if ($isOurs) { $found += $p }
    }
    return $found
}

function Stop-SnipAskBackendProcesses {
    $killed = @()
    foreach ($p in @(Get-SnipAskBackendProcesses)) {
        $killed += [ordered]@{
            pid            = [int]$p.ProcessId
            name           = [string]$p.Name
            executablePath = [string]$p.ExecutablePath
            commandLine    = [string]$p.CommandLine
        }
        Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
    }
    $gone = $true
    if ($killed.Count -gt 0) {
        $gone = Wait-Until -TimeoutMs 15000 -IntervalMs 250 -Condition { @(Get-SnipAskBackendProcesses).Count -eq 0 }
    }
    return [ordered]@{
        found              = @($killed | ForEach-Object { $_.executablePath })
        killed             = $killed
        exitedWithinTimeout = $gone
        left               = @(Get-SnipAskBackendProcesses | ForEach-Object { [int]$_.ProcessId })
    }
}

function Get-SnipAskShortcuts {
    $roots = @(
        (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
        [Environment]::GetFolderPath('Desktop')
    )
    $found = @()
    foreach ($r in $roots) {
        if (Test-Path -LiteralPath $r) {
            $found += @(Get-ChildItem -LiteralPath $r -Recurse -Filter '*.lnk' -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match 'SnipAsk' } | Select-Object -ExpandProperty FullName)
        }
    }
    return $found
}

function Get-RunValue {
    try {
        $v = Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'SnipAsk' -ErrorAction Stop
        return $v.'SnipAsk'
    } catch { return $null }
}

function Get-DirInventory([string]$Path, [int]$MaxDepth = 3) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $items = @(Get-ChildItem -LiteralPath $Path -Force -Recurse -File -ErrorAction SilentlyContinue | Where-Object {
            $rel = $_.FullName.Substring($Path.Length).TrimStart('\')
            return (($rel.Split('\').Count - 1) -le $MaxDepth)
        })
    $total = ($items | Measure-Object -Property Length -Sum).Sum
    return [ordered]@{
        path      = $Path
        fileCount = $items.Count
        totalBytes = if ($total) { [int64]$total } else { [int64]0 }
        files     = @($items | Sort-Object FullName | ForEach-Object {
                [ordered]@{
                    relative = $_.FullName.Substring($Path.Length).TrimStart('\')
                    bytes    = [int64]$_.Length
                    sha256   = if ($_.Length -le 64MB) { Get-FileSha256 $_.FullName } else { $null }
                }
            })
    }
}

function Wait-Until([scriptblock]$Condition, [int]$TimeoutMs = 120000, [int]$IntervalMs = 500) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds $IntervalMs
    }
    return $false
}

function Save-PrimaryScreenPng([string]$Path) {
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    try {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try { $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size) } finally { $g.Dispose() }
        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $bmp.Dispose() }
}

# ---------------------------------------------------------------------------
# 用户数据保护：冒烟前把现有数据目录/注册项移走，冒烟后恢复（finally 保证）
# ---------------------------------------------------------------------------
function New-UserStateBackup([string]$EvidenceDir) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $backupRoot = Join-Path $env:LOCALAPPDATA ('SnipAsk_smoke_backup_' + $stamp)
    New-Item -ItemType Directory -Force -Path $backupRoot | Out-Null
    $dirs = [ordered]@{
        roamingSnipAsk     = (Join-Path $env:APPDATA 'SnipAsk')
        localSnipAsk       = (Join-Path $env:LOCALAPPDATA 'SnipAsk')
        roamingWebview     = (Join-Path $env:APPDATA 'com.snipask.app')
        localWebview       = (Join-Path $env:LOCALAPPDATA 'com.snipask.app')
    }
    # 安装器元数据（Tauri 模板 MANUPRODUCTKEY = HKCU\Software\<厂商>\<产品>，默认值=安装位置，
    # 另有 "Installer Language"）。记录后移除：① 让冒烟安装到默认目录、不覆盖用户真实安装位置；
    # ② 新版卸载钩子会清理该键（静默彻底卸载语义），冒烟结束必须恢复，避免破坏用户状态。
    $manuProductKeyPath = 'HKCU:\Software\SnipAsk\SnipAsk'
    $manuProductKey = [ordered]@{
        key                   = $manuProductKeyPath
        existed               = $false
        installLocation       = $null
        installLocationKind   = $null
        installerLanguage     = $null
        installerLanguageKind = $null
    }
    if (Test-Path -LiteralPath $manuProductKeyPath) {
        $manuProductKey.existed = $true
        try {
            $manuKey = Get-Item -LiteralPath $manuProductKeyPath -ErrorAction Stop
            try {
                $manuProductKey.installLocation = $manuKey.GetValue('', $null)
                $manuProductKey.installLocationKind = $manuKey.GetValueKind('').ToString()
            } catch { }
            try {
                $manuProductKey.installerLanguage = $manuKey.GetValue('Installer Language', $null)
                $manuProductKey.installerLanguageKind = $manuKey.GetValueKind('Installer Language').ToString()
            } catch { }
        } catch { }
        Remove-Item -LiteralPath $manuProductKeyPath -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ("      [备份] 注册项 {0}（安装位置/安装语言）" -f $manuProductKeyPath) -ForegroundColor DarkGray
    }

    $state = [ordered]@{
        backupRoot = $backupRoot
        moved      = @()
        dirs       = $dirs
        runValue   = (Get-RunValue)
        manuProductKey = $manuProductKey
        note       = '冒烟期间临时移走用户数据/注册项；结束后 finally 恢复。'
    }
    foreach ($key in $dirs.Keys) {
        $path = $dirs[$key]
        if (Test-Path -LiteralPath $path) {
            $dest = Join-Path $backupRoot $key
            Move-Item -LiteralPath $path -Destination $dest -Force
            $state.moved += [ordered]@{ key = $key; original = $path; backup = $dest }
            Write-Host ("      [备份] {0} → {1}" -f $path, $dest) -ForegroundColor DarkGray
        }
    }
    # 主窗口自启项：记录后移除，冒烟结束恢复
    try { Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'SnipAsk' -ErrorAction SilentlyContinue } catch { }
    return $state
}

function Restore-UserState($State, [string]$EvidenceDir) {
    if (-not $State) { return }
    $results = @()
    foreach ($p in Get-SnipAskProcesses) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    # 恢复前也清理本应用 llama-server 子进程：否则它可能仍持有冒烟目录中的模型文件，
    # 导致「删除冒烟产物 / 移回用户备份」失败。
    $null = Stop-SnipAskBackendProcesses
    Start-Sleep -Milliseconds 500
    # 先清掉冒烟新建的目录（此时原数据已移走，目录内必为测试产物）
    foreach ($key in $State.dirs.Keys) {
        $path = $State.dirs[$key]
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
            $results += [ordered]@{ action = 'removed-smoke-artifact'; path = $path; removed = -not (Test-Path -LiteralPath $path) }
        }
    }
    foreach ($item in $State.moved) {
        if (Test-Path -LiteralPath $item.backup) {
            Move-Item -LiteralPath $item.backup -Destination $item.original -Force
            $results += [ordered]@{ action = 'restored'; original = $item.original; backup = $item.backup; restored = (Test-Path -LiteralPath $item.original) }
            Write-Host ("      [恢复] {0}" -f $item.original) -ForegroundColor DarkGray
        }
    }
    if ($State.runValue) {
        try {
            New-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'SnipAsk' -Value $State.runValue -PropertyType String -Force | Out-Null
            $results += [ordered]@{ action = 'restored-run-value'; name = 'SnipAsk'; value = $State.runValue; restored = ((Get-RunValue) -eq $State.runValue) }
        } catch {
            $results += [ordered]@{ action = 'restore-run-value-failed'; error = $_.Exception.Message }
        }
    }
    # 恢复安装器元数据键（安装位置 / 安装语言）；卸载钩子会删除它，冒烟后必须还原。
    # 注意：Get-Item 返回的 RegistryKey 是只读句柄，写入必须走 Registry provider 的 cmdlet。
    if ($State.manuProductKey -and $State.manuProductKey.existed) {
        try {
            $keyPath = $State.manuProductKey.key
            if (-not (Test-Path -LiteralPath $keyPath)) { New-Item -Path $keyPath -Force | Out-Null }
            if ($null -ne $State.manuProductKey.installLocation) {
                # Registry provider 语义：Set-Item 写入键的默认值
                Set-Item -LiteralPath $keyPath -Value $State.manuProductKey.installLocation -ErrorAction Stop
            }
            if ($null -ne $State.manuProductKey.installerLanguage) {
                $valueType = 'String'
                if ($State.manuProductKey.installerLanguageKind -eq 'DWord') { $valueType = 'DWord' }
                elseif ($State.manuProductKey.installerLanguageKind -eq 'QWord') { $valueType = 'QWord' }
                New-ItemProperty -LiteralPath $keyPath -Name 'Installer Language' -Value $State.manuProductKey.installerLanguage -PropertyType $valueType -Force | Out-Null
            }
            $results += [ordered]@{ action = 'restored-manu-product-key'; key = $keyPath; restored = (Test-Path -LiteralPath $keyPath) }
        } catch {
            $results += [ordered]@{ action = 'restore-manu-product-key-failed'; error = $_.Exception.Message }
        }
    }
    if ((Test-Path -LiteralPath $State.backupRoot) -and -not (Get-ChildItem -LiteralPath $State.backupRoot -Force -ErrorAction SilentlyContinue)) {
        Remove-Item -LiteralPath $State.backupRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    return $results
}

# ---------------------------------------------------------------------------
# 安装冒烟（真实静默安装 / 启动 / 卸载）
# ---------------------------------------------------------------------------
function Invoke-InstallSmoke([string]$SetupExe, [string]$EditionName, [string]$Version, [string]$EvidenceDir, [string[]]$FullAssetNames, [string[]]$ExpectedDocTargets) {
    $swTotal = [Diagnostics.Stopwatch]::StartNew()
    $result = [ordered]@{
        tool        = 'scripts/build-installers.ps1 -Smoke'
        edition     = $EditionName
        version     = $Version
        setupExe    = $SetupExe
        setupBytes  = (Get-Item -LiteralPath $SetupExe).Length
        setupSha256 = Get-FileSha256 $SetupExe
        startedAt   = (Get-Date).ToString('o')
        environment = [ordered]@{
            os        = (Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue).Caption
            build     = (Get-CimInstance Win32_OperatingSystem -ErrorAction SilentlyContinue).BuildNumber
            user      = [Security.Principal.WindowsIdentity]::GetCurrent().Name
            isAdmin   = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
            psVersion = $PSVersionTable.PSVersion.ToString()
        }
        checks      = [ordered]@{}
        steps       = [ordered]@{}
        verdict     = [ordered]@{ pass = $false; criticalFailures = @(); warnings = @() }
    }

    $evidenceBackupDir = Join-Path $EvidenceDir 'smoke-backup'
    New-Item -ItemType Directory -Force -Path $EvidenceBackupDir | Out-Null

    $state = $null
    try {
        # ---------- 0) 环境准备 ----------
        $running = @(Get-SnipAskProcesses)
        if ($running.Count -gt 0) {
            $result.verdict.warnings += ("冒烟前发现 {0} 个 snip-ask 进程，已强制退出" -f $running.Count)
            foreach ($p in $running) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
            Start-Sleep -Seconds 1
        }
        # 本应用 llama-server 子进程也必须在「备份用户数据」之前退出：
        # 其 exe 位于 %APPDATA%\SnipAsk\backends（会被备份移动），且可能持有
        # %LOCALAPPDATA%\SnipAsk\models\*.gguf（会被卸载钩子删除）。只清理本应用目录下的进程。
        $backendBefore = Stop-SnipAskBackendProcesses
        if ($backendBefore.killed.Count -gt 0) {
            $result.verdict.warnings += ("冒烟前发现 {0} 个本应用 llama-server 子进程，已强制退出" -f $backendBefore.killed.Count)
        }
        $result.steps.backendProcessesBeforeSmoke = $backendBefore
        if (-not $backendBefore.exitedWithinTimeout) {
            $result.verdict.warnings += ('冒烟前本应用 llama-server 子进程未在 15s 内退出：{0}' -f (($backendBefore.left) -join ', '))
        }
        $state = New-UserStateBackup $evidenceBackupDir
        $result.steps.backup = [ordered]@{
            backupRoot = $state.backupRoot
            moved      = $state.moved
            runValue   = $state.runValue
        }

        # ---------- 1) 静默安装 ----------
        if (-not (Test-Path -LiteralPath $SetupExe)) { throw ("安装包不存在：{0}" -f $SetupExe) }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process -FilePath $SetupExe -ArgumentList '/S' -PassThru -ErrorAction Stop
        $exited = $proc.WaitForExit(240000)
        $sw.Stop()
        $install = [ordered]@{
            pid                = $proc.Id
            args               = '/S'
            exitedWithinTimeout = $exited
            durationMs         = [int]$sw.ElapsedMilliseconds
            exitCode           = $null
        }
        try { $install.exitCode = $proc.ExitCode } catch { }
        if ($exited -and $install.exitCode -ne 0) { $result.verdict.criticalFailures += ('安装器退出码非 0：{0}' -f $install.exitCode) }
        $detected = Wait-Until -TimeoutMs 120000 -Condition {
            $e = Get-UninstallEntries
            return (($e.Count -gt 0) -or (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'SnipAsk\snip-ask.exe')))
        }
        $install.installedDetected = $detected
        if (-not $detected) { $result.verdict.criticalFailures += '安装未被检测到（注册表/安装目录均未出现）' }
        $result.steps.install = $install

        $entries = Get-UninstallEntries
        $entry = if ($entries.Count -gt 0) { $entries[0] } else { $null }
        $installDir = if ($entry -and $entry.InstallLocation) { $entry.InstallLocation.Trim('"') } else { Join-Path $env:LOCALAPPDATA 'SnipAsk' }
        $installedExe = Join-Path $installDir 'snip-ask.exe'
        $uninstaller = Join-Path $installDir 'uninstall.exe'
        $result.steps.installState = [ordered]@{
            installDir       = $installDir
            installDirExists = (Test-Path -LiteralPath $installDir)
            installedExe     = $installedExe
            installedExeBytes = if (Test-Path -LiteralPath $installedExe) { (Get-Item -LiteralPath $installedExe).Length } else { $null }
            installedExeVersion = if (Test-Path -LiteralPath $installedExe) { (Get-Item -LiteralPath $installedExe).VersionInfo.FileVersion } else { $null }
            installedExeSignature = if (Test-Path -LiteralPath $installedExe) { (Get-AuthenticodeSignature -LiteralPath $installedExe).Status.ToString() } else { $null }
            uninstaller      = $uninstaller
            uninstallEntry   = if ($entry) { [ordered]@{
                    key             = $entry.PSPath -replace '^Microsoft\.PowerShell\.Core\\Registry::', ''
                    displayName     = $entry.DisplayName
                    displayVersion  = $entry.DisplayVersion
                    publisher       = $entry.Publisher
                    installLocation = $entry.InstallLocation
                    uninstallString = $entry.UninstallString
                    estimatedSize   = $entry.EstimatedSize
                } } else { $null }
            shortcuts        = @(Get-SnipAskShortcuts)
        }
        if (-not (Test-Path -LiteralPath $installedExe)) { $result.verdict.criticalFailures += ("安装目录缺少 snip-ask.exe：{0}" -f $installedExe) }
        if (-not (Test-Path -LiteralPath $uninstaller)) { $result.verdict.criticalFailures += ("安装目录缺少 uninstall.exe：{0}" -f $uninstaller) }
        if ($entry -and $entry.DisplayVersion -and $entry.DisplayVersion -ne $Version) {
            $result.verdict.warnings += ("卸载项 DisplayVersion={0} 与源码版本 {1} 不一致" -f $entry.DisplayVersion, $Version)
        }

        # ---------- 2) 安装目录资源布局校验 ----------
        $ocrDir = Join-Path $installDir 'ocr'
        $docsDir = Join-Path $installDir 'docs'
        $runtimePath = Join-Path $ocrDir $RuntimeAsset
        $checks = [ordered]@{
            runtimeDll = [ordered]@{
                path     = $runtimePath
                exists   = (Test-Path -LiteralPath $runtimePath)
                bytes    = if (Test-Path -LiteralPath $runtimePath) { (Get-Item -LiteralPath $runtimePath).Length } else { $null }
                sha256   = if (Test-Path -LiteralPath $runtimePath) { Get-FileSha256 $runtimePath } else { $null }
            }
            docs = @()
            ocrModels = @()
        }
        $assetDir = Join-Path $root 'src-tauri\resources\ocr'
        if (-not $checks.runtimeDll.exists) { $result.verdict.criticalFailures += ("slim/full 均应随包 ocr/{0}，但缺失" -f $RuntimeAsset) }
        if ($checks.runtimeDll.sha256 -and $checks.runtimeDll.sha256 -ne (Get-FileSha256 (Join-Path $assetDir $RuntimeAsset))) {
            $result.verdict.criticalFailures += '随包 onnxruntime.dll 与 resources 源文件 SHA256 不一致'
        }

        foreach ($doc in $DocsResources) {
            $targetPath = Join-Path $docsDir ($doc.Target -replace '^docs/', '')
            $check = [ordered]@{ target = $doc.Target; path = $targetPath; required = ([bool]($ExpectedDocTargets -contains $doc.Target)); exists = (Test-Path -LiteralPath $targetPath) }
            if ($check.exists) { $check.bytes = (Get-Item -LiteralPath $targetPath).Length }
            $checks.docs += $check
            if ($check.required -and -not $check.exists) { $result.verdict.criticalFailures += ("安装目录 docs 缺少 {0}" -f $doc.Target) }
        }

        $modelExpectations = if ($EditionName -eq 'full') { $FullAssetNames } else { @() }
        foreach ($name in $modelExpectations) {
            $path = Join-Path $ocrDir $name
            $check = [ordered]@{
                name   = $name
                path   = $path
                exists = (Test-Path -LiteralPath $path)
                bytes  = if (Test-Path -LiteralPath $path) { (Get-Item -LiteralPath $path).Length } else { $null }
                sha256 = if (Test-Path -LiteralPath $path) { Get-FileSha256 $path } else { $null }
            }
            $checks.ocrModels += $check
            if (-not $check.exists) { $result.verdict.criticalFailures += ("full 版安装目录 ocr 缺少模型 {0}" -f $name) }
            else {
                $src = Join-Path $assetDir $name
                if ((Get-FileSha256 $src) -ne $check.sha256) { $result.verdict.criticalFailures += ("full 版随包模型 {0} 与 resources 源文件 SHA256 不一致" -f $name) }
            }
        }
        if ($EditionName -eq 'slim') {
            $leaked = @(Get-ChildItem -LiteralPath $ocrDir -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne $RuntimeAsset })
            $checks.slimModelLeak = @($leaked | Select-Object Name, Length)
            if ($leaked.Count -gt 0) { $result.verdict.criticalFailures += ('slim 版安装目录 ocr/ 不应包含模型文件，实际：' + (($leaked | ForEach-Object { $_.Name }) -join ', ')) }
        }
        $checks.installDirInventory = Get-DirInventory $installDir 2
        $result.checks.resources = $checks

        # ---------- 3) 启动应用 6s ----------
        $launch = [ordered]@{}
        if (Test-Path -LiteralPath $installedExe) {
            $launch.procStartLocal = (Get-Date).ToString('o')
            $appProc = Start-Process -FilePath $installedExe -PassThru -ErrorAction Stop
            $launch.pid = $appProc.Id
            $windowOk = Wait-Until -TimeoutMs 30000 -IntervalMs 250 -Condition {
                $p = Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue
                if (-not $p) { return $false }
                return ($p.MainWindowHandle -ne 0)
            }
            $launch.mainWindowDetected = $windowOk
            if (-not $windowOk) { $result.verdict.warnings += '启动 30s 内未检测到主窗口句柄（应用可能仍在初始化/或为托盘启动）' }
            $elapsed = [int]((Get-Date) - [datetime]::Parse($launch.procStartLocal)).TotalMilliseconds
            $remain = [int]($SmokeSeconds * 1000 - $elapsed)
            if ($remain -gt 0) { Start-Sleep -Milliseconds $remain }
            $p = Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue
            $launch.aliveAfterInterval = [bool]$p
            if ($p) {
                $launch.mainWindowTitle = $p.MainWindowTitle
                $launch.mainWindowHandle = [int64]$p.MainWindowHandle
                $launch.workingSetBytes = [int64]$p.WorkingSet64
                $launch.privateBytes = [int64]$p.PrivateMemorySize64
                $launch.responding = $p.Responding
                $launch.threads = $p.Threads.Count
            } else {
                $result.verdict.criticalFailures += '启动后应用进程提前退出'
            }
            try {
                $shot = Join-Path (Join-Path $EvidenceDir 'screenshots') ('launch-' + $EditionName + '.png')
                Save-PrimaryScreenPng $shot
                $launch.screenshot = $shot
                $launch.screenshotBytes = (Get-Item -LiteralPath $shot).Length
            } catch {
                $launch.screenshotError = $_.Exception.Message
                $result.verdict.warnings += ('截图失败：{0}' -f $_.Exception.Message)
            }
            # 关闭：先 CloseMainWindow，8s 不退则强杀
            $closed = $false
            try { $closed = $p.CloseMainWindow() } catch { }
            $launch.closeMainWindowSent = $closed
            $graceful = Wait-Until -TimeoutMs 8000 -IntervalMs 250 -Condition { -not (Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue) }
            $launch.gracefulExit = $graceful
            if (-not $graceful) {
                # v1.0.1 起「关闭窗口 / Alt+F4 = 隐藏到托盘」，进程存活属预期行为；记录后强制结束以免影响卸载。
                $stillAlive = [bool](Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue)
                $launch.closeBehavior = if ($stillAlive) { 'hidden-to-tray' } else { 'unknown' }
                Stop-Process -Id $appProc.Id -Force -ErrorAction SilentlyContinue
                $launch.forcedKill = $true
            } else {
                $launch.closeBehavior = 'exited'
                $launch.forcedKill = $false
            }
            foreach ($p2 in Get-SnipAskProcesses) { Stop-Process -Id $p2.Id -Force -ErrorAction SilentlyContinue }
            Start-Sleep -Seconds 1
            $launch.exited = -not (Get-Process -Id $appProc.Id -ErrorAction SilentlyContinue)
        } else {
            $launch.error = 'installed exe not found'
            $result.verdict.criticalFailures += '无法启动：安装目录缺少 snip-ask.exe'
        }
        $result.steps.launch = $launch

        # ---------- 4) 静默卸载 ----------
        # 卸载前应用主进程已退出，但本地引擎子进程 llama-server.exe
        #（位于 %APPDATA%\SnipAsk\backends\<runtime>\ 下）可能仍持有
        # %LOCALAPPDATA%\SnipAsk\models\*.gguf，导致卸载钩子的 RMDir /r 只能落 /REBOOTOK，
        # 静默卸载后立即断言会看到「残留」。这里只终止「路径/命令行位于本应用
        # %APPDATA%\SnipAsk\backends 下」的 llama-server，绝不误杀其它应用的同名进程。
        $uninstall = [ordered]@{}
        foreach ($p in Get-SnipAskProcesses) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        $uninstall.backendProcesses = Stop-SnipAskBackendProcesses
        if (-not $uninstall.backendProcesses.exitedWithinTimeout) {
            $result.verdict.warnings += ('卸载前本应用 llama-server 子进程未在 15s 内退出：{0}' -f (($uninstall.backendProcesses.left) -join ', '))
        }
        if ($uninstall.backendProcesses.killed.Count -gt 0) {
            Write-Host ("      [清理] 终止本应用 llama-server 子进程 {0} 个" -f $uninstall.backendProcesses.killed.Count) -ForegroundColor DarkGray
        }
        Start-Sleep -Milliseconds 500

        # 写入「已下载模型」探针：验证卸载器 / NSIS 卸载钩子是否真的清理本地数据。
        # 真实用户数据已在备份步骤移走，这里只会写到测试产生的目录；探针大小 2 MiB。
        $localDataRoot = Join-Path $env:LOCALAPPDATA 'SnipAsk'
        $roamingDataRoot = Join-Path $env:APPDATA 'SnipAsk'
        $probeDir = Join-Path (Join-Path $localDataRoot 'models') 'e2e-uninstall-probe'
        $probeFile = Join-Path $probeDir 'probe.gguf'
        $roamingProbe = Join-Path $roamingDataRoot 'e2e-uninstall-probe.txt'
        try {
            New-Item -ItemType Directory -Force -Path $probeDir | Out-Null
            $probePayload = New-Object 'byte[]' (2MB)
            [System.IO.File]::WriteAllBytes($probeFile, $probePayload)
            New-Item -ItemType Directory -Force -Path $roamingDataRoot | Out-Null
            [System.IO.File]::WriteAllText($roamingProbe, "SnipAsk uninstall probe`n", (New-Object System.Text.UTF8Encoding($false)))
        } catch {
            $result.verdict.criticalFailures += ('写入卸载探针失败：{0}' -f $_.Exception.Message)
        }
        $localFilesBefore = @(Get-ChildItem -LiteralPath $localDataRoot -Force -Recurse -File -ErrorAction SilentlyContinue)
        $roamingFilesBefore = @(Get-ChildItem -LiteralPath $roamingDataRoot -Force -Recurse -File -ErrorAction SilentlyContinue)
        $uninstall.preState = [ordered]@{
            localDataRoot        = $localDataRoot
            localDataFileCount   = $localFilesBefore.Count
            localDataBytes       = if ($localFilesBefore.Count -gt 0) { [int64](($localFilesBefore | Measure-Object -Property Length -Sum).Sum) } else { [int64]0 }
            roamingDataRoot      = $roamingDataRoot
            roamingDataFileCount = $roamingFilesBefore.Count
            roamingDataBytes     = if ($roamingFilesBefore.Count -gt 0) { [int64](($roamingFilesBefore | Measure-Object -Property Length -Sum).Sum) } else { [int64]0 }
            modelProbe           = $probeFile
            modelProbeBytes      = if (Test-Path -LiteralPath $probeFile) { (Get-Item -LiteralPath $probeFile).Length } else { $null }
            modelProbeSha256     = if (Test-Path -LiteralPath $probeFile) { Get-FileSha256 $probeFile } else { $null }
            roamingProbe         = $roamingProbe
            roamingProbeExists   = (Test-Path -LiteralPath $roamingProbe)
        }

        if (Test-Path -LiteralPath $uninstaller) {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $up = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -ErrorAction Stop
            $uExited = $up.WaitForExit(240000)
            $sw.Stop()
            $uninstall.pid = $up.Id
            $uninstall.exitedWithinTimeout = $uExited
            $uninstall.durationMs = [int]$sw.ElapsedMilliseconds
            try { $uninstall.exitCode = $up.ExitCode } catch { }
            if (-not $uExited) {
                $result.verdict.criticalFailures += '卸载器在 240s 内未退出'
            } elseif ($null -eq $uninstall.exitCode) {
                $result.verdict.warnings += '无法读取卸载器退出码（Start-Process 未返回 ExitCode）'
            } elseif ($uninstall.exitCode -ne 0) {
                $result.verdict.criticalFailures += ('卸载器退出码非 0：{0}' -f $uninstall.exitCode)
            }
            $removed = Wait-Until -TimeoutMs 180000 -Condition {
                $e = Get-UninstallEntries
                return (($e.Count -eq 0) -and (-not (Test-Path -LiteralPath $installedExe)))
            }
            $uninstall.removedDetected = $removed
            if (-not $removed) { $result.verdict.criticalFailures += '卸载后仍检测到卸载注册项或 snip-ask.exe' }
        } else {
            $uninstall.error = 'uninstaller not found'
            $result.verdict.criticalFailures += '无法卸载：安装目录缺少 uninstall.exe'
        }
        $leftShortcuts = @(Get-SnipAskShortcuts)
        $uninstall.shortcutsLeft = $leftShortcuts
        if ($leftShortcuts.Count -gt 0) { $result.verdict.warnings += ('卸载后残留快捷方式：{0}' -f ($leftShortcuts -join '; ')) }

        # ---------- 4b) 卸载后数据清理断言（必须在 finally 恢复用户备份之前）----------
        # 静默卸载 /S 的约定是「彻底删除本地数据」；轮询 15s 以吸收 NSIS 卸载器退出后的极短尾延迟
        #（以及 /REBOOTOK 之外可能的重试窗口）。三处核心路径 + 模型探针必须全部消失。
        $dataRoots = [ordered]@{
            localAppDataSnipAsk    = $localDataRoot
            roamingAppDataSnipAsk  = $roamingDataRoot
            localAppDataWebView2   = (Join-Path $env:LOCALAPPDATA 'com.snipask.app')
            roamingAppDataWebView2 = (Join-Path $env:APPDATA 'com.snipask.app')
        }
        $dataClean = Wait-Until -TimeoutMs 15000 -IntervalMs 250 -Condition {
            $left = @($dataRoots.Values | Where-Object { Test-Path -LiteralPath $_ })
            return ($left.Count -eq 0)
        }
        $residue = @()
        foreach ($key in $dataRoots.Keys) {
            $path = $dataRoots[$key]
            if (-not (Test-Path -LiteralPath $path)) { continue }
            $files = @(Get-ChildItem -LiteralPath $path -Force -Recurse -File -ErrorAction SilentlyContinue)
            $residue += [ordered]@{
                key         = $key
                path        = $path
                fileCount   = $files.Count
                totalBytes  = if ($files.Count -gt 0) { [int64](($files | Measure-Object -Property Length -Sum).Sum) } else { [int64]0 }
                sampleFiles = @($files | Select-Object -First 10 -ExpandProperty FullName)
            }
        }
        $uninstall.dataRoots = $dataRoots
        $uninstall.dataCleanedWithinWait = $dataClean
        $uninstall.modelProbeRemoved = -not (Test-Path -LiteralPath $probeFile)
        $uninstall.roamingProbeRemoved = -not (Test-Path -LiteralPath $roamingProbe)
        $uninstall.dataResidue = @($residue)
        $uninstall.dataRemoved = ($dataClean -and (@($residue).Count -eq 0) -and $uninstall.modelProbeRemoved -and $uninstall.roamingProbeRemoved)
        if (-not $uninstall.modelProbeRemoved) {
            $result.verdict.criticalFailures += ('卸载后模型探针仍存在（卸载器未清理 %LOCALAPPDATA%\SnipAsk\models）：{0}' -f $probeFile)
        }
        if (-not $uninstall.dataRemoved) {
            $desc = @($residue | ForEach-Object { '{0}（{1} 个文件，{2:N0} B）' -f $_.path, $_.fileCount, $_.totalBytes }) -join '; '
            $result.verdict.criticalFailures += ('静默卸载后本地数据残留：{0}' -f $desc)
        }
        $uninstall.registryResidue = [ordered]@{
            manuProductKey = (Test-Path -LiteralPath 'HKCU:\Software\SnipAsk\SnipAsk')
            runValue       = [bool](Get-RunValue)
        }
        if ($uninstall.registryResidue.manuProductKey -or $uninstall.registryResidue.runValue) {
            $registryLeft = @()
            if ($uninstall.registryResidue.manuProductKey) { $registryLeft += 'HKCU\Software\SnipAsk\SnipAsk' }
            if ($uninstall.registryResidue.runValue) { $registryLeft += 'HKCU ...\Run\SnipAsk' }
            $result.verdict.warnings += ('卸载后仍残留注册项：{0}' -f ($registryLeft -join '; '))
        }
        $uninstall.behaviorMatrix = [ordered]@{
            silent               = '自动化验证（发布门禁）：/S 静默卸载必须删除数据目录与模型探针'
            updateMode           = '未自动覆盖：$UpdateMode=1（覆盖安装/更新）必须保留数据，需按人工清单验证'
            interactiveChecked   = '未自动覆盖：交互卸载勾选「删除应用数据」必须清理数据'
            interactiveUnchecked = '未自动覆盖：交互卸载未勾选时弹 MessageBox（默认 Yes=清理，No=保留）'
            note                 = '未自动覆盖项为人工/后续专项自动化验证；静默路径为发布必过项。'
        }
        $uninstall.installDirLeft = [ordered]@{
            path   = $installDir
            exists = (Test-Path -LiteralPath $installDir)
            files  = if (Test-Path -LiteralPath $installDir) { @(Get-ChildItem -LiteralPath $installDir -Force -Recurse -File -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName) } else { @() }
        }
        $result.steps.uninstall = $uninstall
    } catch {
        $result.verdict.criticalFailures += ('冒烟异常：{0}' -f $_.Exception.Message)
        $result.error = $_.Exception.Message
    } finally {
        # 无论成败：恢复用户数据
        try {
            $restore = Restore-UserState $state $evidenceBackupDir
            $result.steps.restore = [ordered]@{
                results = $restore
                appDataSnipAskRestored = (Test-Path -LiteralPath (Join-Path $env:APPDATA 'SnipAsk'))
            }
        } catch {
            $result.verdict.criticalFailures += ('恢复用户数据失败：{0}' -f $_.Exception.Message)
            $result.steps.restore = [ordered]@{ error = $_.Exception.Message }
        }
        foreach ($p in Get-SnipAskProcesses) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
        $swTotal.Stop()
        $result.durationMs = [int]$swTotal.ElapsedMilliseconds
        $result.finishedAt = (Get-Date).ToString('o')
        $result.verdict.pass = ($result.verdict.criticalFailures.Count -eq 0)
        Write-Utf8JsonNoBom -Path (Join-Path $EvidenceDir ('smoke-' + $EditionName + '.json')) -Object $result
        Write-Host ("      冒烟结果：{0}（{1} 项关键失败 / {2} 项警告）" -f $(if ($result.verdict.pass) { 'PASS' } else { 'FAIL' }), $result.verdict.criticalFailures.Count, $result.verdict.warnings.Count) -ForegroundColor $(if ($result.verdict.pass) { 'Green' } else { 'Red' })
        foreach ($f in $result.verdict.criticalFailures) { Write-Host ("        - {0}" -f $f) -ForegroundColor Red }
    }
    return $result
}

# ---------------------------------------------------------------------------
# overlay 处理
# ---------------------------------------------------------------------------
function Get-EditionConfigPath([string]$TauriDir, [string]$EditionName) {
    return (Join-Path $TauriDir ('tauri.' + $EditionName + '.conf.json'))
}

# 开发期应急：文档尚未齐备时，过滤掉缺失的 docs 映射，生成临时 overlay（产物不可最终交付）。
function New-DevOverlay([string]$RepoRootPath, [string]$TauriDir, [string]$EditionName, [string]$BaseConfigPath, [string]$TmpDir, $Docs) {
    $config = Get-Content -LiteralPath $BaseConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $resources = [ordered]@{}
    foreach ($prop in $config.bundle.resources.PSObject.Properties) {
        $srcRelativeToRepo = $prop.Name
        $exists = $true
        if ($srcRelativeToRepo -match '^\.\./') {
            $exists = Test-Path -LiteralPath (Join-Path $TauriDir $srcRelativeToRepo)
        } elseif ($srcRelativeToRepo -match '^resources/') {
            $exists = Test-Path -LiteralPath (Join-Path $TauriDir $srcRelativeToRepo)
        }
        if ($exists) { $resources[$prop.Name] = $prop.Value }
        else { Write-Host ("      [dev-overlay] 跳过缺失资源映射：{0}" -f $prop.Name) -ForegroundColor Yellow }
    }
    $devConfig = [ordered]@{ bundle = [ordered]@{ resources = $resources } }
    if (-not (Test-Path -LiteralPath $TmpDir)) { New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null }
    $path = Join-Path $TmpDir ('tauri.' + $EditionName + '.dev.conf.json')
    Write-Utf8JsonNoBom -Path $path -Object $devConfig
    return $path
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
$root = Resolve-RepoRoot
Set-Location -LiteralPath $root
if (-not $OutputDir) { $OutputDir = Join-Path $root 'release' }
if (-not (Test-Path -LiteralPath $OutputDir)) { New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null }
$TauriDir = Join-Path $root 'src-tauri'
$EvidenceDir = Join-Path $root '.verify\v1\installer'
$TmpDir = Join-Path $EvidenceDir 'tmp'
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $EvidenceDir 'logs') | Out-Null

$result = [ordered]@{
    tool        = 'scripts/build-installers.ps1'
    version     = '1.0.0'
    startedAt   = (Get-Date).ToString('o')
    repoRoot    = $root
    releases    = @()
    gates       = [ordered]@{ slimMiB = $SlimThresholdMiB; fullMiB = $FullThresholdMiB }
    smoke       = @()
    ok          = $true
    failures    = @()
}

function Add-Failure([string]$Message) {
    $result.ok = $false
    $result.failures += $Message
    Write-Host ("[失败] {0}" -f $Message) -ForegroundColor Red
}

Write-Host ''
Write-Host '=== 框选 AI · 双版本安装包构建 ===' -ForegroundColor Cyan
Write-Host ("仓库根目录：{0}" -f $root) -ForegroundColor DarkGray
Write-Host ("版本来源  ：src-tauri/tauri.conf.json" ) -ForegroundColor DarkGray
Write-Host ''

# ---------- 0) 前置校验 ----------
$confPath = Join-Path $TauriDir 'tauri.conf.json'
if (-not (Test-Path -LiteralPath $confPath)) { Add-Failure 'src-tauri/tauri.conf.json 不存在'; exit 3 }
$conf = Get-Content -LiteralPath $confPath -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$conf.version
$productName = [string]$conf.productName
$result.version = $version
$result.productName = $productName

Write-Host ("[0/6] 前置校验：版本 {0}，产品 {1}" -f $version, $productName) -ForegroundColor Cyan
foreach ($tool in @('node', 'pnpm', 'cargo')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Add-Failure ("未找到工具链：{0}" -f $tool); exit 3 }
}
if (-not (Test-Path -LiteralPath (Join-Path $root 'node_modules'))) {
    Write-Host '      [提示] node_modules 不存在，执行 pnpm install' -ForegroundColor Yellow
    $code = Invoke-Logged $root (Join-Path $EvidenceDir 'logs\pnpm-install.log') 'pnpm install' @('pnpm', 'install')
    if ($code -ne 0) { Add-Failure ('pnpm install 失败：{0}' -f $code); exit 3 }
}

$missingDocs = @()
$missingAnyDocs = @()
foreach ($doc in $DocsResources) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $doc.Source))) {
        $missingAnyDocs += $doc.Source
        if ($doc.Required) { $missingDocs += $doc.Source }
        else { Write-Host ("      [提示] 可选文档缺失（跳过）：{0}" -f $doc.Source) -ForegroundColor Yellow }
    }
}
$result.docsCheck = [ordered]@{ missingRequired = $missingDocs; missingAny = $missingAnyDocs; allowMissingDocs = [bool]$AllowMissingDocs }
if ($missingDocs.Count -gt 0) {
    if ($AllowMissingDocs) {
        Write-Host ("      [警告] -AllowMissingDocs：缺 {0}，将生成不含缺失文档的临时 overlay（不可最终交付）" -f ($missingDocs -join ', ')) -ForegroundColor Yellow
    } else {
        Add-Failure ('安装目录 docs/ 必需资源缺失：{0}（T7 文档任务未完成；仅开发期可用 -AllowMissingDocs 跳过）' -f ($missingDocs -join ', '))
        exit 3
    }
}

# 资产准备
$fetchScript = Join-Path $root 'scripts\fetch-ocr-assets.ps1'
$fetchArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $fetchScript)
if ($SkipFetch) { $fetchArgs += '-VerifyOnly' }
Write-Host ''
Write-Host '[1/6] 准备 OCR 资产（src-tauri/resources/ocr）' -ForegroundColor Cyan
$fetchCode = Invoke-Logged $root (Join-Path $EvidenceDir 'logs\fetch-ocr-assets.log') 'fetch-ocr-assets' (@('powershell') + $fetchArgs)
if ($fetchCode -ne 0) { Add-Failure ('OCR 资产准备失败（exit={0}），见 .verify/v1/installer/logs/fetch-ocr-assets.log' -f $fetchCode); exit 3 }
$assetDir = Join-Path $TauriDir 'resources\ocr'
$assetHashes = [ordered]@{}
foreach ($name in @($RuntimeAsset) + $FullModelAssets) {
    $p = Join-Path $assetDir $name
    if (-not (Test-Path -LiteralPath $p)) { Add-Failure ("OCR 资产缺失：{0}" -f $p); exit 3 }
    $assetHashes[$name] = [ordered]@{ bytes = (Get-Item -LiteralPath $p).Length; sha256 = Get-FileSha256 $p }
}
$result.ocrAssets = $assetHashes

# ---------- 构建 ----------
$editions = if ($Edition -eq 'both') { @('slim', 'full') } else { @($Edition) }
$builtArtifacts = @()

foreach ($ed in $editions) {
    Write-Host ''
    Write-Host ("[2/6] 构建 {0} 版安装包" -f $ed) -ForegroundColor Cyan
    $baseConfig = Get-EditionConfigPath $TauriDir $ed
    if (-not (Test-Path -LiteralPath $baseConfig)) { Add-Failure ("缺少 overlay 配置：{0}" -f $baseConfig); continue }
    $buildConfig = $baseConfig
    if ($missingAnyDocs.Count -gt 0) {
        $buildConfig = New-DevOverlay $root $TauriDir $ed $baseConfig $TmpDir $DocsResources
        Write-Host ("      [dev] 使用临时 overlay：{0}" -f $buildConfig) -ForegroundColor Yellow
    }

    $targetName = if ($ed -eq 'full') { ('{0}_{1}_x64-full-setup.exe' -f $productName, $version) } else { ('{0}_{1}_x64-setup.exe' -f $productName, $version) }
    $logPath = Join-Path $EvidenceDir ('logs\build-' + $ed + '.log')
    $buildInfo = [ordered]@{
        edition           = $ed
        overlay           = $buildConfig
        buildLog          = $logPath
        artifact          = (Join-Path $OutputDir $targetName)
        built             = $false
        buildSeconds      = $null
        bundleRawPath     = $null
        bytes             = $null
        sizeMiB           = $null
        sha256            = $null
        gateMiB           = if ($ed -eq 'full') { $FullThresholdMiB } else { $SlimThresholdMiB }
        gatePass          = $false
        error             = $null
    }
    if (Test-Path -LiteralPath $logPath) { Remove-Item -LiteralPath $logPath -Force -ErrorAction SilentlyContinue }

    if ($SkipBuild) {
        if (Test-Path -LiteralPath $buildInfo.artifact) {
            Write-Host ("      [SkipBuild] 复用 {0}" -f $buildInfo.artifact) -ForegroundColor DarkGray
            $buildInfo.built = $true
        } else {
            $buildInfo.error = 'SkipBuild 但产物不存在'
            Add-Failure ("{0}：{1}" -f $ed, $buildInfo.error)
        }
    } else {
        $swBuild = [Diagnostics.Stopwatch]::StartNew()
        try {
            # edition 编译期常量注入：默认先清 app crate 的 release 产物（不动 debug/测试产物），
            # 保证 option_env!("SNIP_ASK_EDITION") 重新求值；构建目录被其他成员占用时重试。
            if (-not $NoClean) {
                Write-Host '      [clean] cargo clean -p snip-ask --release（仅 app crate 的 release 产物）' -ForegroundColor DarkGray
                $cleanOk = $false
                for ($attempt = 1; $attempt -le 6; $attempt++) {
                    $cleanCode = Invoke-Logged $TauriDir $logPath 'cargo clean -p snip-ask --release' @('cargo', 'clean', '-p', 'snip-ask', '--release')
                    if ($cleanCode -eq 0) { $cleanOk = $true; break }
                    if ($attempt -lt 6) {
                        Write-Host '      [clean] 构建目录被其他 cargo 进程占用，10s 后重试…' -ForegroundColor Yellow
                        Start-Sleep -Seconds 10
                    }
                }
                $buildInfo.cleanOk = $cleanOk
                if (-not $cleanOk) {
                    Write-Host '      [警告] cargo clean 多次失败（其他成员可能正在构建/跑测试）；继续构建，依赖 cargo 的 env 依赖跟踪重编 app crate，最终以双版本主程序哈希自检确认 edition 生效' -ForegroundColor Yellow
                }
            }

            $env:SNIP_ASK_EDITION = $ed
            Write-Host ("      [env] SNIP_ASK_EDITION={0}" -f $ed) -ForegroundColor DarkGray
            $buildCode = Invoke-Logged $root $logPath ('pnpm exec tauri build --config ' + $buildConfig) @('pnpm', 'exec', 'tauri', 'build', '--config', $buildConfig)
            if ($buildCode -ne 0) { throw ('pnpm tauri build 失败：{0}' -f $buildCode) }
            $swBuild.Stop()
            $buildInfo.buildSeconds = [math]::Round($swBuild.Elapsed.TotalSeconds, 1)

            $bundleDir = Join-Path $TauriDir 'target\release\bundle\nsis'
            $bundle = @(Get-ChildItem -LiteralPath $bundleDir -Filter '*-setup.exe' -File -ErrorAction SilentlyContinue |
                Sort-Object LastWriteTime -Descending | Select-Object -First 1)
            if ($bundle.Count -eq 0) { throw ('未找到 NSIS 产物：{0}' -f $bundleDir) }
            $buildInfo.bundleRawPath = $bundle[0].FullName
            Copy-Item -LiteralPath $bundle[0].FullName -Destination $buildInfo.artifact -Force
            # 记录该 edition 编译出的主程序指纹（slim/full 的 AppStats.edition 来自编译期常量，
            # 两个版本的 snip-ask.exe 应不同；若相同说明 SNIP_ASK_EDITION 未生效）
            $appExe = Join-Path $TauriDir 'target\release\snip-ask.exe'
            if (Test-Path -LiteralPath $appExe) {
                $buildInfo.appExe = [ordered]@{
                    path   = $appExe
                    bytes  = [int64](Get-Item -LiteralPath $appExe).Length
                    sha256 = Get-FileSha256 $appExe
                }
            }
            $buildInfo.built = $true
        } catch {
            $swBuild.Stop()
            $buildInfo.error = $_.Exception.Message
            Add-Failure ("{0} 版构建失败：{1}" -f $ed, $buildInfo.error)
        } finally {
            Remove-Item Env:SNIP_ASK_EDITION -ErrorAction SilentlyContinue
        }
    }

    if ($buildInfo.built -and (Test-Path -LiteralPath $buildInfo.artifact)) {
        $item = Get-Item -LiteralPath $buildInfo.artifact
        $buildInfo.bytes = [int64]$item.Length
        $buildInfo.sizeMiB = [math]::Round($item.Length / 1MB, 2)
        $buildInfo.sha256 = Get-FileSha256 $buildInfo.artifact
        $gateBytes = if ($ed -eq 'full') { $FullThresholdBytes } else { $SlimThresholdBytes }
        $buildInfo.gatePass = ($item.Length -le $gateBytes)
        Write-Host ("      产物：{0}" -f $buildInfo.artifact) -ForegroundColor Green
        Write-Host ("      体积：{0}（门禁 ≤ {1} MiB）→ {2}" -f (Format-Bytes $item.Length), $buildInfo.gateMiB, $(if ($buildInfo.gatePass) { 'PASS' } else { 'FAIL' })) -ForegroundColor $(if ($buildInfo.gatePass) { 'Green' } else { 'Red' })
        Write-Host ("      SHA256：{0}" -f $buildInfo.sha256) -ForegroundColor DarkGray
        if (-not $buildInfo.gatePass) { Add-Failure ("{0} 版体积超门禁：{1:N2} MiB > {2} MiB" -f $ed, $buildInfo.sizeMiB, $buildInfo.gateMiB) }
        $builtArtifacts += $buildInfo
    }
    $result.releases += $buildInfo
}

# edition 注入自检：两个版本都构建时，主程序 SHA256 应不同（否则说明 SNIP_ASK_EDITION 没生效）
$editionBinaries = @($result.releases | Where-Object { $_.built -and $_.appExe } )
if ($editionBinaries.Count -eq 2) {
    $distinct = ($editionBinaries[0].appExe.sha256 -ne $editionBinaries[1].appExe.sha256)
    $result.editionInjection = [ordered]@{
        distinctAppBinaries = $distinct
        slimAppSha256       = ($editionBinaries | Where-Object { $_.edition -eq 'slim' }).appExe.sha256
        fullAppSha256       = ($editionBinaries | Where-Object { $_.edition -eq 'full' }).appExe.sha256
        note                = 'slim/full 的 snip-ask.exe 应不同（编译期 AppStats.edition）；相同则 SNIP_ASK_EDITION 未生效'
    }
    if ($distinct) {
        Write-Host '      版本注入自检：slim/full 主程序 SHA256 不同 → PASS' -ForegroundColor Green
    } else {
        Write-Host '      版本注入自检：slim/full 主程序 SHA256 相同 → 警告（SNIP_ASK_EDITION 可能未生效）' -ForegroundColor Yellow
        $result.failures += 'edition 注入自检失败：slim/full 主程序 SHA256 相同'
        $result.ok = $false
    }
}

# ---------- 3) SHA256SUMS.txt ----------
Write-Host ''
Write-Host '[3/6] 生成 release/SHA256SUMS.txt' -ForegroundColor Cyan
$sumsPath = Join-Path $OutputDir 'SHA256SUMS.txt'
$sumLines = @()
$artifactsToSum = @(Get-ChildItem -LiteralPath $OutputDir -Filter ('{0}_{1}_*-setup.exe' -f $productName, $version) -File -ErrorAction SilentlyContinue | Sort-Object Name)
foreach ($a in $artifactsToSum) {
    $sumLines += ('{0}  {1}' -f (Get-FileSha256 $a.FullName), $a.Name)
}
[System.IO.File]::WriteAllText($sumsPath, (($sumLines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
foreach ($line in $sumLines) { Write-Host ("      {0}" -f $line) -ForegroundColor DarkGray }
$result.sha256s = [ordered]@{ file = $sumsPath; entries = $sumLines }

# ---------- 4) 冒烟 ----------
if (-not $SkipSmoke) {
    Write-Host ''
    Write-Host '[4/6] 真实安装冒烟（静默安装 / 启动 / 文件布局 / 静默卸载）' -ForegroundColor Cyan
    $expectedDocTargets = @($DocsResources | Where-Object { Test-Path -LiteralPath (Join-Path $root $_.Source) } | ForEach-Object { $_.Target })
    foreach ($artifact in $builtArtifacts) {
        Write-Host ("      ---- {0}：{1}" -f $artifact.edition, $artifact.artifact) -ForegroundColor Cyan
        $smoke = Invoke-InstallSmoke -SetupExe $artifact.artifact -EditionName $artifact.edition -Version $version -EvidenceDir $EvidenceDir -FullAssetNames $FullModelAssets -ExpectedDocTargets $expectedDocTargets
        $smokeUninstall = $smoke.steps.uninstall
        $result.smoke += [ordered]@{
            edition          = $artifact.edition
            pass             = [bool]$smoke.verdict.pass
            dataRemoved      = [bool]$smokeUninstall.dataRemoved
            dataResidue      = @($smokeUninstall.dataResidue)
            exitCode         = $smokeUninstall.exitCode
            criticalFailures = $smoke.verdict.criticalFailures
            warnings         = $smoke.verdict.warnings
            evidence         = (Join-Path $EvidenceDir ('smoke-' + $artifact.edition + '.json'))
        }
        if (-not $smoke.verdict.pass) { Add-Failure ("{0} 版安装冒烟失败" -f $artifact.edition) }
    }
} else {
    Write-Host ''
    Write-Host '[4/6] 已跳过安装冒烟（-SkipSmoke）' -ForegroundColor Yellow
    $result.smokeSkipped = $true
}

# ---------- 5) 证据汇总 ----------
$result.finishedAt = (Get-Date).ToString('o')
$summaryPath = Join-Path $EvidenceDir 'build-summary.json'
Write-Utf8JsonNoBom -Path $summaryPath -Object $result

Write-Host ''
Write-Host '[5/6] 证据与归档' -ForegroundColor Cyan
Write-Host ("      构建汇总：{0}" -f $summaryPath) -ForegroundColor DarkGray
Write-Host ("      SHA256  ：{0}" -f $sumsPath) -ForegroundColor DarkGray
Write-Host ("      构建日志：{0}" -f (Join-Path $EvidenceDir 'logs')) -ForegroundColor DarkGray

Write-Host ''
if ($result.ok) {
    Write-Host '[完成] 双版本安装包构建与冒烟全部通过。' -ForegroundColor Green
    foreach ($r in $result.releases) {
        if ($r.built) { Write-Host ("      {0,-4} {1}  {2}" -f $r.edition, (Format-Bytes $r.bytes), $r.sha256) -ForegroundColor Green }
    }
    exit 0
}

Write-Host ('[失败] {0} 项：' -f $result.failures.Count) -ForegroundColor Red
foreach ($f in $result.failures) { Write-Host ("      - {0}" -f $f) -ForegroundColor Red }
exit 1
