# S-18 发布构建与体积基线采集（release + NSIS）
#
# 用途：在真实仓库上执行 `pnpm tauri build`，产出 release 主 exe 与 NSIS 安装包，
#       并采集体积/构建元数据写入 .verify/spikes/S-16-S-18/build/。
# 并发纪律：构建前独占创建 .verify/p1/build.lock（JSON：owner/pid/startedAt/note），
#       若锁已存在且 pid 存活且年龄 < 20 分钟则等待；过期（或 pid 已退出）则接管。
#       构建结束（成功/失败）在 finally 中删除锁。
#
# 用法：
#   pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s18_build.ps1
#   pwsh ... -File ... -SkipBuild          # 只采集现有产物体积（不重新构建）
#
# 退出码：0 成功；非 0 构建失败。

param(
    [string]$Root,
    [switch]$SkipBuild,
    [int]$StaleMinutes = 20,
    [string]$Owner = 'release-metrics'
)

$ErrorActionPreference = 'Stop'

if (-not $Root) {
    $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
}
$Root = (Resolve-Path $Root).Path

$verifyRoot = Join-Path $Root '.verify'
$lockPath   = Join-Path $verifyRoot 'p1\build.lock'
$logDir     = Join-Path $verifyRoot 'p1\logs'
$outDir     = Join-Path $verifyRoot 'spikes\S-16-S-18\build'
$buildLog   = Join-Path $logDir 's18-build.log'
foreach ($d in @((Join-Path $verifyRoot 'p1'), $logDir, $outDir)) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
}

function Test-PidAlive([int]$Pid_) {
    if ($Pid_ -le 0) { return $false }
    try { return $null -ne (Get-Process -Id $Pid_ -ErrorAction Stop) } catch { return $false }
}

function Get-LockInfo {
    if (-not (Test-Path $lockPath)) { return $null }
    try { return (Get-Content -Raw -Encoding UTF8 $lockPath | ConvertFrom-Json) } catch { return $null }
}

# ---------- 获取构建锁 ----------
$acquired = $false
$waitStart = Get-Date
while (-not $acquired) {
    $info = Get-LockInfo
    if ($null -eq $info) {
        try {
            $null = New-Item -ItemType File -Path $lockPath -ErrorAction Stop
            $lock = [ordered]@{
                owner     = $Owner
                pid       = $PID
                startedAt = (Get-Date).ToString('o')
                note      = if ($SkipBuild) { 's18_build.ps1 -SkipBuild（仅采集）' } else { 'pnpm tauri build（release + NSIS）' }
                host      = $env:COMPUTERNAME
            }
            $lock | ConvertTo-Json -Compress | Set-Content -Encoding UTF8 -Path $lockPath
            $acquired = $true
            Write-Host "[s18] 已获取构建锁 $lockPath（pid=$PID）"
        } catch {
            Start-Sleep -Seconds 5
        }
        continue
    }
    $ageMin = ((Get-Date) - $waitStart).TotalMinutes
    $lockAgeMin = ((Get-Date) - (Get-Item $lockPath).LastWriteTime).TotalMinutes
    $alive = Test-PidAlive ([int]$info.pid)
    if ((-not $alive) -or ($lockAgeMin -gt $StaleMinutes)) {
        Write-Warning ("[s18] 构建锁过期/持有者已退出：owner={0} pid={1} alive={2} age={3:N1}min → 接管" -f $info.owner, $info.pid, $alive, $lockAgeMin)
        Remove-Item $lockPath -Force -ErrorAction SilentlyContinue
        continue
    }
    if ($ageMin -gt $StaleMinutes + 10) {
        throw "[s18] 等待构建锁超时（owner=$($info.owner) pid=$($info.pid)）"
    }
    Write-Host ("[s18] 构建锁被占用：owner={0} pid={1} alive={2} age={3:N1}min，等待 10s" -f $info.owner, $info.pid, $alive, $lockAgeMin)
    Start-Sleep -Seconds 10
}

# ---------- 采集环境元数据 ----------
function Get-CmdVersion([string]$cmd, [string[]]$args_) {
    try { return ((& $cmd @args_) 2>&1 | Select-Object -First 1).ToString().Trim() } catch { return $null }
}

$meta = [ordered]@{
    spike        = 'S-18'
    date         = (Get-Date).ToString('o')
    host         = $env:COMPUTERNAME
    osCaption    = (Get-CimInstance Win32_OperatingSystem).Caption
    osVersion    = (Get-CimInstance Win32_OperatingSystem).Version
    osBuild      = (Get-CimInstance Win32_OperatingSystem).BuildNumber
    cpu          = (Get-CimInstance Win32_Processor).Name
    ramGiB       = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 2)
    node         = (Get-Command node).Source
    nodeVersion  = (Get-CmdVersion 'node' @('--version'))
    pnpmVersion  = (Get-CmdVersion 'pnpm' @('--version'))
    rustcVersion = (Get-CmdVersion 'rustc' @('--version'))
    cargoVersion = (Get-CmdVersion 'cargo' @('--version'))
    tauriCli     = (Get-CmdVersion 'pnpm' @('tauri', '--version'))
    gitCommit    = (Get-CmdVersion 'git' @('rev-parse', 'HEAD'))
    gitDescribe  = (Get-CmdVersion 'git' @('describe', '--always', '--dirty'))
    skipBuild    = [bool]$SkipBuild
}

$exitCode = 0
$buildStart = Get-Date
try {
    if (-not $SkipBuild) {
        Write-Host "[s18] 开始 release 构建：pnpm tauri build（日志 → $buildLog）"
        # 构建日志写 .verify，不落仓库；使用 cmd 重定向以保留 pnpm/cargo 的原始输出
        Push-Location $Root
        try {
            $cmdLine = 'pnpm tauri build > "{0}" 2>&1' -f $buildLog
            cmd /c $cmdLine
            $exitCode = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        Write-Host ("[s18] 构建结束，退出码 {0}（耗时 {1:N1}s）" -f $exitCode, ((Get-Date) - $buildStart).TotalSeconds)
    } else {
        Write-Host '[s18] -SkipBuild：跳过构建，仅采集现有产物'
    }

    $meta['build'] = [ordered]@{
        command    = if ($SkipBuild) { $null } else { 'pnpm tauri build' }
        exitCode   = $exitCode
        startedAt  = $buildStart.ToString('o')
        finishedAt = (Get-Date).ToString('o')
        durationS  = [math]::Round(((Get-Date) - $buildStart).TotalSeconds, 2)
        logPath    = if ($SkipBuild) { $null } else { $buildLog }
    }

    # ---------- 产物体积 ----------
    $nsisDir = Join-Path $Root 'src-tauri\target\release\bundle\nsis'
    $setupExe = Get-ChildItem -Path $nsisDir -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $releaseExe = Get-Item (Join-Path $Root 'src-tauri\target\release\snip-ask.exe') -ErrorAction SilentlyContinue
    $releaseDll = Get-Item (Join-Path $Root 'src-tauri\target\release\snip_ask_lib.dll') -ErrorAction SilentlyContinue
    $msiExe = Get-ChildItem -Path (Join-Path $Root 'src-tauri\target\release\bundle\msi') -Filter '*.msi' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1

    $bytesOf = { param($f) if ($null -ne $f) { [int64]$f.Length } else { $null } }

    $meta['artifacts'] = [ordered]@{
        setupExe = if ($setupExe) { [ordered]@{
            path = $setupExe.FullName; bytes = [int64]$setupExe.Length
            mib = [math]::Round($setupExe.Length / 1MB, 3); lastWriteTime = $setupExe.LastWriteTime.ToString('o')
        } } else { $null }
        releaseExe = if ($releaseExe) { [ordered]@{
            path = $releaseExe.FullName; bytes = [int64]$releaseExe.Length
            mib = [math]::Round($releaseExe.Length / 1MB, 3); lastWriteTime = $releaseExe.LastWriteTime.ToString('o')
        } } else { $null }
        releaseDll = if ($releaseDll) { [ordered]@{
            path = $releaseDll.FullName; bytes = [int64]$releaseDll.Length; mib = [math]::Round($releaseDll.Length / 1MB, 3)
        } } else { $null }
        msi = if ($msiExe) { [ordered]@{ path = $msiExe.FullName; bytes = [int64]$msiExe.Length; mib = [math]::Round($msiExe.Length / 1MB, 3) } } else { $null }
        thresholdMib = 25
        passNoModel = if ($setupExe) { [math]::Round($setupExe.Length / 1MB, 3) -le 25 } else { $null }
    }

    $metaPath = Join-Path $outDir 'build_meta.json'
    $meta | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -Path $metaPath
    Write-Host "[s18] 元数据已写入 $metaPath"

    if ($exitCode -ne 0) {
        Write-Warning "[s18] 构建失败（exit=$exitCode），已保留日志 $buildLog"
        exit $exitCode
    }
    if ($null -eq $setupExe) {
        Write-Warning '[s18] 未找到 NSIS 安装包产物'
        exit 11
    }
    Write-Host ("[s18] 安装包：{0} = {1:N0} B = {2:N2} MiB（阈值 25 MiB，{3}）" -f $setupExe.Name, $setupExe.Length, ($setupExe.Length / 1MB), $(if ($setupExe.Length / 1MB -le 25) { 'PASS' } else { 'FAIL' }))
    exit 0
}
catch {
    Write-Error $_
    exit 1
}
finally {
    if ($acquired -and (Test-Path $lockPath)) {
        $info = Get-LockInfo
        if ($null -ne $info -and [int]$info.pid -eq $PID) {
            Remove-Item $lockPath -Force -ErrorAction SilentlyContinue
            Write-Host '[s18] 已释放构建锁'
        }
    }
}
