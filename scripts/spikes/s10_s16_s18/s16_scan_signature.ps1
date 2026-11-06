# S-16 ① 未签名安装包与主 exe 的签名 / Defender 扫描 / MOTW 证据采集
#
# 输出（默认 .verify/spikes/S-16-S-18/s16/）：
#   scan_signature.json     全量机器可读结果
#   defender_scan_setup.txt / defender_scan_release.txt   MpCmdRun 原始输出
#   motw_zone_identifier.txt MOTW 流内容
#
# 用法（Windows PowerShell 5.1；脚本文件为 UTF-8 BOM）：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/spikes/s10_s16_s18/s16_scan_signature.ps1
#   ... -SetupExe <path> -ReleaseExe <path> -OutDir <dir>
#
# 纪律：扫描用 -DisableRemediation，避免 Defender 在扫描时隔离/删除待验证产物；
#       本脚本不修改系统（只新增一个临时 MOTW 副本，结束后保留在 .verify 下供复核，不进入仓库）。

param(
    [string]$Root,
    [string]$SetupExe,
    [string]$ReleaseExe,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'

if (-not $Root) { $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path }
$Root = (Resolve-Path $Root).Path
if (-not $SetupExe) {
    $SetupExe = Get-ChildItem -Path (Join-Path $Root 'src-tauri\target\release\bundle\nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $ReleaseExe) { $ReleaseExe = Join-Path $Root 'src-tauri\target\release\snip-ask.exe' }
if (-not $OutDir) { $OutDir = Join-Path $Root '.verify\spikes\S-16-S-18\s16' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$motwDir = Join-Path $OutDir 'motw'
New-Item -ItemType Directory -Force -Path $motwDir | Out-Null

function Write-Utf8Json([string]$Path, $Object) {
    $Object | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -Path $Path
}

function Get-FileEvidence([string]$Path) {
    if (-not (Test-Path $Path)) { return $null }
    $f = Get-Item $Path
    $sig = $null
    try { $sig = Get-AuthenticodeSignature -FilePath $Path } catch { $sig = $null }
    $ver = $f.VersionInfo
    return [ordered]@{
        path            = $f.FullName
        bytes           = [int64]$f.Length
        mib             = [math]::Round($f.Length / 1MB, 3)
        lastWriteTime   = $f.LastWriteTime.ToString('o')
        sha256          = (Get-FileHash -Algorithm SHA256 -Path $Path).Hash
        signatureStatus = if ($sig) { $sig.Status.ToString() } else { $null }
        signatureStatusMessage = if ($sig) { $sig.StatusMessage } else { $null }
        signerSubject   = if ($sig -and $sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { $null }
        signerThumbprint = if ($sig -and $sig.SignerCertificate) { $sig.SignerCertificate.Thumbprint } else { $null }
        isOSBinary      = if ($sig) { $sig.IsOSBinary } else { $null }
        fileVersion     = if ($ver) { $ver.FileVersion } else { $null }
        productVersion  = if ($ver) { $ver.ProductVersion } else { $null }
    }
}

# ---------- 环境 / 杀软 ----------
$av = @()
try {
    $av = Get-CimInstance -Namespace 'root/SecurityCenter2' -ClassName AntiVirusProduct -ErrorAction Stop |
        Select-Object displayName, productState, pathToSignedProductExe, timestamp
} catch {
    $av = @([ordered]@{ error = $_.Exception.Message })
}

$defenderStatus = $null
try {
    $defenderStatus = Get-MpComputerStatus -ErrorAction Stop | Select-Object AMServiceEnabled, AntivirusEnabled,
        RealTimeProtectionEnabled, AntispywareEnabled, NISEnabled, AMEngineVersion, AntivirusSignatureVersion,
        AntivirusSignatureLastUpdated, QuickScanStartTime, FullScanStartTime
} catch {
    $defenderStatus = [ordered]@{ error = $_.Exception.Message }
}

$appHost = $null
try { $appHost = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppHost' -ErrorAction Stop |
    Select-Object EnableWebContentEvaluation } catch { }
$attachments = $null
try { $attachments = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Attachments' -ErrorAction Stop |
    Select-Object ScanWithAntiVirus } catch { }

$smartScreenLogs = @()
try {
    $smartScreenLogs = Get-WinEvent -ListLog '*SmartScreen*' -ErrorAction Stop |
        Select-Object LogName, IsEnabled, RecordCount
} catch {
    $smartScreenLogs = @([ordered]@{ error = $_.Exception.Message })
}

$envInfo = [ordered]@{
    timestamp      = (Get-Date).ToString('o')
    host           = $env:COMPUTERNAME
    user           = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    isAdmin        = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    osCaption      = (Get-CimInstance Win32_OperatingSystem).Caption
    osVersion      = (Get-CimInstance Win32_OperatingSystem).Version
    osBuild        = (Get-CimInstance Win32_OperatingSystem).BuildNumber
    cpu            = (Get-CimInstance Win32_Processor).Name
    ramGiB         = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 2)
    avProducts     = $av
    defender       = $defenderStatus
    appHost        = $appHost
    attachments    = $attachments
    smartScreenLogs = $smartScreenLogs
}

# ---------- 文件证据 ----------
$setupEvidence = Get-FileEvidence $SetupExe
$releaseEvidence = Get-FileEvidence $ReleaseExe

# ---------- Defender 自定义扫描 ----------
$mpCmdRun = $null
foreach ($candidate in @(
        'C:\Program Files\Windows Defender\MpCmdRun.exe',
        'C:\Program Files (x86)\Windows Defender\MpCmdRun.exe')) {
    if (Test-Path $candidate) { $mpCmdRun = $candidate; break }
}

$threatBefore = @()
try { $threatBefore = @(Get-MpThreatDetection -ErrorAction Stop) } catch { }
$scanResults = [ordered]@{}
if ($mpCmdRun) {
    $targets = [ordered]@{ setup = $SetupExe; releaseExe = $ReleaseExe }
    foreach ($name in $targets.Keys) {
        $target = $targets[$name]
        $scanLog = Join-Path $OutDir ("defender_scan_{0}.txt" -f $name)
        $scanStart = Get-Date
        $output = ''
        $code = $null
        if (Test-Path $target) {
            $output = (& $mpCmdRun -Scan -ScanType 3 -File $target -DisableRemediation 2>&1 | Out-String)
            $code = $LASTEXITCODE
        } else {
            $output = "target not found: $target"
            $code = -1
        }
        Set-Content -Encoding UTF8 -Path $scanLog -Value $output
        # MpCmdRun 退出码：0 = 扫描完成且无威胁；2 = 发现威胁（已被处置或需处置）。
        $verdict = switch ($code) {
            0 { 'no threat' }
            2 { 'threat found' }
            default { "unknown (exit=$code)" }
        }
        $scanResults[$name] = [ordered]@{
            target      = $target
            command     = "$mpCmdRun -Scan -ScanType 3 -File `"$target`" -DisableRemediation"
            exitCode    = $code
            verdict     = $verdict
            durationS   = [math]::Round(((Get-Date) - $scanStart).TotalSeconds, 2)
            logPath     = $scanLog
            rawOutput   = $output.Trim()
        }
    }
} else {
    $scanResults['error'] = 'MpCmdRun.exe not found'
}
$threatAfter = @()
try { $threatAfter = @(Get-MpThreatDetection -ErrorAction Stop) } catch { }

# ---------- MOTW（Zone.Identifier, ZoneId=3） ----------
$motwName = Split-Path -Leaf $SetupExe
$motwCopy = Join-Path $motwDir $motwName
Copy-Item -Path $SetupExe -Destination $motwCopy -Force
$zoneContent = "[ZoneTransfer]`r`nZoneId=3`r`nReferrerUrl=https://example.com/download`r`nHostUrl=https://example.com/download/$motwName`r`n"
Set-Content -Path $motwCopy -Stream 'Zone.Identifier' -Value $zoneContent -Encoding UTF8
$zoneReadback = (Get-Content -Path $motwCopy -Stream 'Zone.Identifier' -Raw)
$streams = @(Get-Item -Path $motwCopy -Stream '*' | Select-Object Stream, Length)
Set-Content -Encoding UTF8 -Path (Join-Path $OutDir 'motw_zone_identifier.txt') -Value ("# 文件：$motwCopy`r`n# Get-Item -Stream *`r`n" + ($streams | Format-Table -AutoSize | Out-String) + "`r`n# Zone.Identifier 内容`r`n" + $zoneReadback)
$motw = [ordered]@{
    copiedTo     = $motwCopy
    zoneId       = 3
    zoneContent  = $zoneContent.Trim()
    readback     = $zoneReadback.Trim()
    streams      = $streams
    sha256       = (Get-FileHash -Algorithm SHA256 -Path $motwCopy).Hash
}

# ---------- 汇总 ----------
$result = [ordered]@{
    spike       = 'S-16'
    step        = 'signature + defender scan + MOTW'
    environment = $envInfo
    files       = [ordered]@{
        setup      = $setupEvidence
        releaseExe = $releaseEvidence
    }
    defenderScan = [ordered]@{
        mpCmdRun       = $mpCmdRun
        threatCountBefore = $threatBefore.Count
        threatCountAfter  = $threatAfter.Count
        scan           = $scanResults
        note           = 'MpCmdRun -Scan -ScanType 3 为自定义扫描；-DisableRemediation 关闭处置动作以保留待验证产物。exit 0=无威胁，2=发现威胁。'
    }
    motw        = $motw
    smartScreen = [ordered]@{
        appHostEnableWebContentEvaluation = if ($appHost) { $appHost.EnableWebContentEvaluation } else { $null }
        attachmentManagerScanWithAntiVirus = if ($attachments) { $attachments.ScanWithAntiVirus } else { $null }
        logAvailability = $smartScreenLogs
        note = 'SmartScreen 的拦截提示是交互 UI；本环境 Microsoft-Windows-SmartScreen/Debug 日志未启用（IsEnabled=False），无法从事件日志取得判定。MOTW 行为验证见 s16_install_test.ps1（带 MOTW 的静默安装是否被阻断）。'
    }
}

$jsonPath = Join-Path $OutDir 'scan_signature.json'
Write-Utf8Json -Path $jsonPath -Object $result

Write-Host "[s16] 签名状态：setup=$($setupEvidence.signatureStatus) release=$($releaseEvidence.signatureStatus)"
foreach ($name in $scanResults.Keys) {
    if ($name -eq 'error') { continue }
    $r = $scanResults[$name]
    Write-Host ("[s16] Defender 扫描 {0}：exit={1} verdict={2}（{3:N1}s）" -f $name, $r.exitCode, $r.verdict, $r.durationS)
}
Write-Host "[s16] MOTW 副本：$motwCopy（ZoneId=3 已写入并回读）"
Write-Host "[s16] 结果：$jsonPath"
