# S-04 真机验证编排脚本（Ctrl+Q 全局热键注册 / 冲突检测 / 暂停释放）
#
# 前提：Vite 常驻（applab start 会自动确保）；debug exe 已构建：
#   cargo build --manifest-path src-tauri/Cargo.toml
#
# 用法：
#   pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1
#   pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1 -RestartCycles 20 -ProbeCycles 20
#   pwsh -NoProfile -File scripts/spikes/s04/run_validation.ps1 -OnlyPhase conflict
#
# 产出：
#   .verify/spikes/S-04/logs/run-<stamp>.jsonl   原始记录（每行一条 JSON）
#   .verify/spikes/S-04/shots/*.png              截图
#   .verify/spikes/S-04/tmp/*                    辅助进程状态文件
#
# 纪律：本脚本独占 applab 应用实例（owner=hotkey-probe），结束时自动 stop。
param(
  [int]$ProbeCycles = 20,
  [int]$RestartCycles = 20,
  [string]$Owner = "hotkey-probe",
  [string]$OnlyPhase = "",
  [switch]$SkipScreenshot
)

$ErrorActionPreference = "Continue"
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
$OutputEncoding = [System.Text.Encoding]::UTF8

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
Set-Location $Repo
$HostExe = (Get-Process -Id $PID).Path
$LogRoot = Join-Path $Repo ".verify\spikes\S-04"
$LogDir = Join-Path $LogRoot "logs"
$TmpDir = Join-Path $LogRoot "tmp"
$ShotDir = Join-Path $LogRoot "shots"
New-Item -ItemType Directory -Force -Path $LogDir, $TmpDir, $ShotDir | Out-Null
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$RawLog = Join-Path $LogDir "run-$Stamp.jsonl"
$Applab = "scripts/spikes/applab/applab.mjs"

function Write-Record {
  param([string]$Phase, [hashtable]$Data = @{})
  $rec = [ordered]@{ ts = (Get-Date).ToString("o"); phase = $Phase }
  foreach ($k in $Data.Keys) { $rec[$k] = $Data[$k] }
  $line = ($rec | ConvertTo-Json -Compress -Depth 8)
  Add-Content -LiteralPath $RawLog -Value $line -Encoding UTF8
  Write-Host $line
}

function Invoke-Node {
  param([string[]]$NodeArgs)
  $output = & node @NodeArgs 2>&1
  $code = $LASTEXITCODE
  $text = (@($output) | ForEach-Object { "$_" }) -join "`n"
  return @{ code = $code; text = $text }
}

function Invoke-Applab {
  param([string[]]$ApplabArgs)
  $all = New-Object System.Collections.Generic.List[string]
  $all.Add($Applab)
  foreach ($a in $ApplabArgs) { $all.Add($a) }
  return Invoke-Node $all.ToArray()
}

function Extract-Json {
  param([string]$Text)
  $lines = $Text -split "`n"
  for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i].TrimStart().StartsWith("{")) {
      $candidate = ($lines[$i..($lines.Count - 1)] -join "`n").Trim()
      try { return ($candidate | ConvertFrom-Json) } catch { }
    }
  }
  return $null
}

function Invoke-Eval {
  param([string]$Js)
  $r = Invoke-Applab @("eval", "--window", "main", "--js", $Js)
  $json = Extract-Json $r.text
  if ($null -eq $json) { return @{ ok = $false; exitCode = $r.code; raw = $r.text } }
  return $json
}

function Invoke-Ipc {
  param([string]$Command, [string]$ParamsJson = "{}")
  $js = "(async () => { const t0 = performance.now(); const value = await window.__TAURI_INTERNALS__.invoke('$Command', $ParamsJson); return { elapsedMs: performance.now() - t0, value }; })()"
  return Invoke-Eval $js
}

function Invoke-Probe {
  param([string]$Spec)
  $js = "(async () => { const t0 = performance.now(); const status = await window.__TAURI_INTERNALS__.invoke('hotkey_probe', { spec: '$Spec' }); return { elapsedMs: performance.now() - t0, status }; })()"
  $r = Invoke-Eval $js
  return $r
}

function Start-App {
  $r = Invoke-Applab @("start", "--owner", $Owner, "--wait-sec", "900")
  if ($r.code -ne 0) { throw "applab start 失败（exit=$($r.code)）：$($r.text)" }
  return $r.text
}

function Stop-App {
  $r = Invoke-Applab @("stop", "--owner", $Owner)
  return $r
}

function Wait-AppExit {
  param([int]$TimeoutSec = 20)
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    $procs = @(Get-Process snip-ask -ErrorAction SilentlyContinue)
    if ($procs.Count -eq 0) { return $true }
    Start-Sleep -Milliseconds 250
  }
  return $false
}

# ---- 辅助进程：hold_hotkey.ps1 ----
function Invoke-HoldProbe {
  # 同步「仅探测」：注册成功即注销；退出码 0=空闲/成功，2=被占用
  param([string]$Spec)
  $out = & $HostExe -NoProfile -File (Join-Path $PSScriptRoot "hold_hotkey.ps1") -Spec $Spec -HoldSec 0 2>&1
  $code = $LASTEXITCODE
  $records = @()
  foreach ($line in @($out)) {
    $text = "$line"
    if ($text.TrimStart().StartsWith("{")) {
      try { $records += ($text | ConvertFrom-Json) } catch { }
    }
  }
  $registered = $null
  $win32Error = $null
  $registerUs = $null
  foreach ($rec in $records) {
    if ($rec.phase -eq "registered") {
      $registered = $rec.registered
      $win32Error = $rec.win32Error
      $registerUs = $rec.registerUs
    }
  }
  return @{
    spec = $Spec
    exitCode = $code
    registered = $registered
    win32Error = $win32Error
    registerUs = $registerUs
    records = $records
    raw = (@($out) | ForEach-Object { "$_" }) -join "`n"
  }
}

function Start-HoldBackground {
  # 后台占住热键：启动辅助进程并等待 StatusFile 中出现 registered 记录
  param([string]$Spec, [int]$HoldSec = 180)
  $token = [guid]::NewGuid().ToString("N").Substring(0, 8)
  $statusFile = Join-Path $TmpDir "hold-$token.jsonl"
  $outFile = Join-Path $TmpDir "hold-$token.out"
  $proc = Start-Process -FilePath $HostExe `
    -ArgumentList @("-NoProfile", "-File", (Join-Path $PSScriptRoot "hold_hotkey.ps1"), "-Spec", $Spec, "-HoldSec", "$HoldSec", "-StatusFile", $statusFile) `
    -RedirectStandardOutput $outFile -NoNewWindow -PassThru
  $deadline = (Get-Date).AddSeconds(15)
  $registered = $null
  while ((Get-Date) -lt $deadline) {
    if (Test-Path -LiteralPath $statusFile) {
      $lines = @(Get-Content -LiteralPath $statusFile -ErrorAction SilentlyContinue)
      foreach ($line in $lines) {
        try {
          $rec = $line | ConvertFrom-Json
          if ($rec.phase -eq "registered") { $registered = $rec; break }
        } catch { }
      }
      if ($registered) { break }
    }
    Start-Sleep -Milliseconds 120
  }
  return @{
    spec = $Spec
    proc = $proc
    procId = $proc.Id
    statusFile = $statusFile
    outFile = $outFile
    registered = $registered
  }
}

function Stop-HoldBackground {
  param($Handle)
  if (-not $Handle) { return }
  try {
    if (-not $Handle.proc.HasExited) {
      Stop-Process -Id $Handle.procId -Force -ErrorAction SilentlyContinue
      Start-Sleep -Milliseconds 250
    }
  } catch { }
}

function Get-AppLogTail {
  param([int]$Lines = 40)
  $base = Join-Path $Repo ".verify\applab\app\data"
  if (-not (Test-Path -LiteralPath $base)) { return @() }
  $logFiles = @(Get-ChildItem -LiteralPath $base -Recurse -File -Filter "snipask.log*" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
  if ($logFiles.Count -eq 0) { return @() }
  $tail = @(Get-Content -LiteralPath $logFiles[0].FullName -Tail $Lines -ErrorAction SilentlyContinue)
  return $tail
}

function Test-Phase {
  param([string]$Name)
  return ($OnlyPhase -eq "" -or $OnlyPhase -eq $Name -or $OnlyPhase -eq "all")
}

Write-Record "run/start" @{
  stamp = $Stamp
  owner = $Owner
  probeCycles = $ProbeCycles
  restartCycles = $RestartCycles
  onlyPhase = $OnlyPhase
  hostExe = $HostExe
  rawLog = $RawLog
  # 真实第三方热键占用观察（Lead 提示：本机装有微信，但默认不占 Ctrl+Q）
  thirdPartyProcesses = @(Get-Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ProcessName -match "Weixin|WeChat|^QQ|Sogou|QQPinyin|Baidu|Ime" } |
    Select-Object -First 20 -ExpandProperty ProcessName)
}

$summary = [ordered]@{}

try {
  # =====================================================================
  # Phase A：基线 —— 应用启动后 Ctrl+Q 已注册；hotkey_probe 判定可用；
  #          辅助进程同键注册被拒（1409）；无冲突组合 20 次 100% 可用。
  # =====================================================================
  if (Test-Phase "baseline") {
    Stop-App | Out-Null
    Start-Sleep -Milliseconds 500
    Start-App | Out-Null
    Start-Sleep -Milliseconds 800

    # A1：探测默认绑定 Ctrl+Q ×N（要求 100% available=true）
    $okCount = 0
    $failCount = 0
    $latencies = @()
    $warmup = Invoke-Probe "Ctrl+Q"
    Write-Record "baseline/probe-warmup" @{ result = $warmup }
    for ($i = 1; $i -le $ProbeCycles; $i++) {
      $r = Invoke-Probe "Ctrl+Q"
      $status = $null
      $ms = $null
      if ($r.ok) { $status = $r.value.status; $ms = $r.value.elapsedMs }
      if ($status -and $status.available -eq $true) { $okCount++ } else { $failCount++ }
      if ($null -ne $ms) { $latencies += [double]$ms }
      Write-Record "baseline/probe-ctrl-q" @{ i = $i; ok = ($status -and $status.available -eq $true); elapsedMs = $ms; status = $status; rawOk = $r.ok }
    }
    $summary["baseline_probeCtrlQ_ok"] = $okCount
    $summary["baseline_probeCtrlQ_fail"] = $failCount
    if ($latencies.Count -gt 0) {
      $summary["baseline_probeCtrlQ_latencyMs_p50"] = [math]::Round(($latencies | Sort-Object)[[int]($latencies.Count / 2)], 3)
      $summary["baseline_probeCtrlQ_latencyMs_max"] = [math]::Round(($latencies | Measure-Object -Maximum).Maximum, 3)
    }

    # A2：辅助进程同键注册 ×N（要求 100% 被拒、Win32 1409）——证明应用真实持有 Ctrl+Q
    $denied = 0
    $allowed = 0
    $codes = @()
    for ($i = 1; $i -le $ProbeCycles; $i++) {
      $h = Invoke-HoldProbe "Ctrl+Q"
      $isDenied = ($h.registered -eq $false)
      if ($isDenied) { $denied++ } else { $allowed++ }
      $codes += $h.win32Error
      Write-Record "baseline/aux-register-ctrl-q" @{ i = $i; denied = $isDenied; win32Error = $h.win32Error; registerUs = $h.registerUs; exitCode = $h.exitCode }
      Start-Sleep -Milliseconds 30
    }
    $summary["baseline_auxDenied"] = $denied
    $summary["baseline_auxAllowed"] = $allowed
    $summary["baseline_auxWin32Codes"] = @($codes | Sort-Object -Unique)

    # A3：无冲突组合 ×N（要求 100% available）
    $candidateOk = 0
    $candidateFail = 0
    foreach ($spec in @("Ctrl+Shift+Y", "Alt+Shift+9")) {
      for ($i = 1; $i -le $ProbeCycles; $i++) {
        $r = Invoke-Probe $spec
        $status = $null
        $ms = $null
        if ($r.ok) { $status = $r.value.status; $ms = $r.value.elapsedMs }
        if ($status -and $status.available -eq $true) { $candidateOk++ } else { $candidateFail++ }
        Write-Record "baseline/probe-candidate" @{ spec = $spec; i = $i; ok = ($status -and $status.available -eq $true); elapsedMs = $ms; status = $status }
      }
    }
    $summary["baseline_candidateOk"] = $candidateOk
    $summary["baseline_candidateFail"] = $candidateFail
  }

  # =====================================================================
  # Phase B：重启 ×N —— 每次 app_quit + 重新启动后，辅助进程注册 Ctrl+Q
  #          必须 100% 被拒（1409），且 hotkey_probe(Ctrl+Q) 可用。
  # =====================================================================
  if (Test-Phase "restart") {
    if (-not (Get-Process snip-ask -ErrorAction SilentlyContinue)) {
      Start-App | Out-Null
      Start-Sleep -Milliseconds 800
    }
    $restartOk = 0
    $restartFail = 0
    $startFailures = 0
    for ($i = 1; $i -le $RestartCycles; $i++) {
      $quit = Invoke-Ipc "app_quit"
      Write-Record "restart/app-quit" @{ i = $i; ipcOk = $quit.ok; raw = $quit.raw }
      Start-Sleep -Milliseconds 400
      $exited = Wait-AppExit -TimeoutSec 20
      if (-not $exited) {
        Write-Record "restart/force-kill" @{ i = $i; reason = "app_quit 后进程未在 20s 内退出" }
        Stop-Process -Name snip-ask -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 400
      }
      # applab 启动失败（外部并发干扰 / CDP 未就绪）不计入「注册成功率」，单独分类并重试一次
      $startError = $null
      for ($attempt = 1; $attempt -le 2; $attempt++) {
        try {
          Start-App | Out-Null
          $startError = $null
          break
        } catch {
          $startError = $_.Exception.Message
          Write-Record "restart/start-retry" @{ i = $i; attempt = $attempt; error = $startError }
          Start-Sleep -Milliseconds 1500
        }
      }
      if ($startError) {
        $startFailures++
        Write-Record "restart/start-failed" @{ i = $i; attempts = 2; error = $startError }
        continue
      }
      Start-Sleep -Milliseconds 700
      $aux = Invoke-HoldProbe "Ctrl+Q"
      $probe = Invoke-Probe "Ctrl+Q"
      $probeAvailable = ($probe.ok -and $probe.value.status.available -eq $true)
      $cycleOk = ($aux.registered -eq $false) -and $probeAvailable
      if ($cycleOk) { $restartOk++ } else { $restartFail++ }
      Write-Record "restart/cycle" @{
        i = $i
        ok = $cycleOk
        auxDenied = ($aux.registered -eq $false)
        auxWin32Error = $aux.win32Error
        probeAvailable = $probeAvailable
        probeElapsedMs = $(if ($probe.ok) { $probe.value.elapsedMs } else { $null })
      }
    }
    $summary["restart_ok"] = $restartOk
    $summary["restart_registrationFail"] = $restartFail
    $summary["restart_startFailures"] = $startFailures
    $summary["restart_startedSamples"] = ($restartOk + $restartFail)
  }

  # =====================================================================
  # Phase C：OS 冲突（等价模拟 QQ/微信先占 Ctrl+Q）——
  #          先启动辅助进程占住 Ctrl+Q，再启动应用（其注册必然失败），
  #          hotkey_probe(Ctrl+Q) 必须 100% available=false + conflictWith="system"；
  #          设置页 UI 显示可操作文案（截图）。
  # =====================================================================
  if (Test-Phase "conflict") {
    Stop-App | Out-Null
    Start-Sleep -Milliseconds 500
    $hold = Start-HoldBackground -Spec "Ctrl+Q" -HoldSec 240
    Write-Record "conflict/aux-hold" @{ registered = $hold.registered; procId = $hold.procId; statusFile = $hold.statusFile }
    if (-not ($hold.registered -and $hold.registered.registered -eq $true)) {
      throw "辅助进程未能占住 Ctrl+Q（可能被应用/其它进程占用）"
    }
    $startError = $null
    try { Start-App | Out-Null } catch { $startError = $_.Exception.Message }
    if ($startError) {
      Stop-HoldBackground $hold
      throw "冲突场景启动应用失败：$startError"
    }
    Start-Sleep -Milliseconds 1200

    # C1：20 次探测，要求 100% available=false + conflictWith="system"
    $sysCount = 0
    $otherCount = 0
    for ($i = 1; $i -le $ProbeCycles; $i++) {
      $r = Invoke-Probe "Ctrl+Q"
      $status = $null
      $ms = $null
      if ($r.ok) { $status = $r.value.status; $ms = $r.value.elapsedMs }
      $isSystem = ($status -and $status.available -eq $false -and $status.conflictWith -eq "system")
      if ($isSystem) { $sysCount++ } else { $otherCount++ }
      Write-Record "conflict/probe-ctrl-q" @{ i = $i; systemConflict = $isSystem; elapsedMs = $ms; status = $status }
    }
    $summary["conflict_systemDetected"] = $sysCount
    $summary["conflict_systemMissed"] = $otherCount

    # C2：应用日志中的注册失败告警
    $logTail = Get-AppLogTail -Lines 120
    $hotkeyWarns = @($logTail | Where-Object { $_ -match "热键注册失败" })
    Write-Record "conflict/app-log" @{ warnCount = $hotkeyWarns.Count; lines = $hotkeyWarns | Select-Object -First 5 }

    # C3：设置页 UI 证据（截图 + 提示文案）
    if (-not $SkipScreenshot) {
      $show = Invoke-Ipc "window_show" "{'window':'main'}"
      Write-Record "conflict/window-show" @{ result = $show.raw }
      Start-Sleep -Milliseconds 600
      # 若当前是首启引导（主界面显示引导向导），先点顶栏「设置」切到设置页
      $switchJs = @'
(() => {
  if (document.querySelector('.settings-nav-item')) return { ok: true, stage: 'already-settings' };
  const buttons = [...document.querySelectorAll('button')];
  const topSettings = buttons.find((b) => b.textContent.trim() === '设置');
  if (!topSettings) return { ok: false, stage: 'no-top-settings', buttons: buttons.slice(0, 12).map((b) => b.textContent.trim().slice(0, 20)) };
  topSettings.click();
  return { ok: true, stage: 'clicked-top-settings' };
})()
'@
      $switch = Invoke-Eval $switchJs
      Write-Record "conflict/ui-switch-settings" @{ result = $switch }
      Start-Sleep -Milliseconds 800
      $navJs = @'
(() => {
  const navs = [...document.querySelectorAll('.settings-nav-item')];
  if (!navs.length) return { ok: false, reason: 'settings-nav 不存在', text: document.body.innerText.slice(0, 200) };
  const target = navs.find((b) => b.textContent.includes('动作与热键')) || navs.find((b) => b.textContent.includes('热键'));
  if (!target) return { ok: false, navs: navs.map((b) => b.textContent) };
  target.click();
  return { ok: true, clicked: target.textContent };
})()
'@
      $nav = Invoke-Eval $navJs
      Write-Record "conflict/ui-nav" @{ result = $nav }
      Start-Sleep -Milliseconds 700
      $clickJs = @'
(() => {
  const rows = [...document.querySelectorAll('.sa-hotkey')];
  if (!rows.length) return { ok: false, reason: '未找到 .sa-hotkey 行', body: document.body.innerText.slice(0, 300) };
  const first = rows[0];
  const btn = [...first.querySelectorAll('button')].find((b) => b.textContent.includes('检测'));
  if (!btn) return { ok: false, reason: '未找到检测按钮' };
  btn.click();
  return { ok: true, value: (first.querySelector('.sa-hotkey-btn') || {}).textContent };
})()
'@
      $click = Invoke-Eval $clickJs
      Write-Record "conflict/ui-click-probe" @{ result = $click }
      Start-Sleep -Milliseconds 1200
      $readJs = @'
(() => {
  const rows = [...document.querySelectorAll('.sa-hotkey')];
  if (!rows.length) return { ok: false };
  const first = rows[0];
  return {
    ok: true,
    value: (first.querySelector('.sa-hotkey-btn') || {}).textContent,
    status: (first.querySelector('.sa-hotkey-status') || {}).textContent || null,
    body: document.body.innerText.slice(0, 800),
  };
})()
'@
      $read = Invoke-Eval $readJs
      Write-Record "conflict/ui-status" @{ result = $read }
      # 把第一行热键（框选并唤起）滚动到视口中央，保证提示文案出现在截图里
      $scrollJs = @'
(() => {
  const row = document.querySelector('.sa-hotkey');
  if (!row) return { ok: false };
  row.scrollIntoView({ block: 'center', inline: 'nearest' });
  return { ok: true, y: window.scrollY, rect: row.getBoundingClientRect().top };
})()
'@
      $scroll = Invoke-Eval $scrollJs
      Write-Record "conflict/ui-scroll" @{ result = $scroll }
      Start-Sleep -Milliseconds 500
      $shotPath = Join-Path $ShotDir "s04-conflict-system-hint-$(Get-Date -Format 'HHmmss').png"
      $shot = Invoke-Applab @("shot", "--window", "main", "--out", $shotPath)
      Write-Record "conflict/ui-screenshot" @{ path = $shotPath; ok = ($shot.code -eq 0) }
      $summary["conflict_uiHint"] = $(if ($read.ok) { $read.value.status } else { $null })
      $summary["conflict_screenshot"] = $shotPath
    }

    Stop-HoldBackground $hold
    Start-Sleep -Milliseconds 400
  }

  # =====================================================================
  # Phase D：暂停后完全释放 / 恢复后重新注册（可控热键路径）
  #   D1 先 config_patch 绑定 togglePause=Ctrl+Alt+P（改 hotkeys.* 触发重新注册）
  #   D2 用 keybd_event 发一次 Ctrl+Alt+P → 应用 set_paused(true)：
  #      其余热键全部释放、仅保留 togglePause（见 app/hotkeys.rs register_all）
  #   D3 暂停判据（硬）：辅助进程 RegisterHotKey(Ctrl+Q) 成功；
  #      同时 hotkey_probe(Ctrl+Q) 不得再显示 available=true（应用已不持有该键）
  #   D4 恢复：再发一次 Ctrl+Alt+P → 应用重新注册；辅助进程再次 1409 + probe available
  # =====================================================================
  if (Test-Phase "pause") {
    Stop-App | Out-Null
    Start-Sleep -Milliseconds 500
    Start-App | Out-Null
    Start-Sleep -Milliseconds 900

    # 注意：这里必须用单引号 JS 对象字面量——Windows PowerShell 5.1 传给原生 node 时
    # 会吃掉双引号（applab eval 的 --js 参数），故参数 JSON 用 JS 单引号写法。
    $patch = Invoke-Ipc "config_patch" "{'patch':{'hotkeys':{'capture':'Ctrl+Q','togglePause':'Ctrl+Alt+P'}}}"
    $patchToggle = $null
    if ($patch.ok) { $patchToggle = $patch.value.value.hotkeys.togglePause }
    Write-Record "pause/config-patch" @{ ok = $patch.ok; togglePause = $patchToggle; raw = $patch.raw }
    Start-Sleep -Milliseconds 800

    # 前置检查：应用持有 Ctrl+Q（辅助进程应被拒 1409）
    $pre = Invoke-HoldProbe "Ctrl+Q"
    Write-Record "pause/pre-check" @{ appHolds = ($pre.registered -eq $false); win32Error = $pre.win32Error }

    # D2：keybd_event 注入 Ctrl+Alt+P 触发 togglePause（真实全局热键链路）
    $press = & $HostExe -NoProfile -File (Join-Path $PSScriptRoot "press_hotkey.ps1") -Spec "Ctrl+Alt+P" 2>&1
    Write-Record "pause/hotkey-press" @{ exitCode = $LASTEXITCODE; raw = (@($press) | ForEach-Object { "$_" }) -join "`n" }
    Start-Sleep -Milliseconds 900

    # D3：硬判据 —— 辅助进程必须能注册 Ctrl+Q（证明产品已完全释放该键）
    $pausedAux = Invoke-HoldProbe "Ctrl+Q"
    $pausedProbe = Invoke-Probe "Ctrl+Q"
    $pausedProbeAvailable = ($pausedProbe.ok -and $pausedProbe.value.status.available -eq $true)
    Write-Record "pause/verify-released" @{
      method = "togglePause-hotkey"
      auxRegistered = $pausedAux.registered
      auxWin32Error = $pausedAux.win32Error
      auxRegisterUs = $pausedAux.registerUs
      probeAvailable = $pausedProbeAvailable
      probeStatus = $(if ($pausedProbe.ok) { $pausedProbe.value.status } else { $null })
      probeElapsedMs = $(if ($pausedProbe.ok) { $pausedProbe.value.elapsedMs } else { $null })
    }
    $releaseOk = ($pausedAux.registered -eq $true)
    $summary["pause_released"] = $releaseOk
    if (-not $releaseOk) { Write-Record "pause/released-failed" @{ note = "辅助进程未能注册 Ctrl+Q，说明热键未完全释放" } }

    # D4：再发一次 Ctrl+Alt+P 恢复（暂停时该热键保留，见 register_all/on_shortcut）
    $press2 = & $HostExe -NoProfile -File (Join-Path $PSScriptRoot "press_hotkey.ps1") -Spec "Ctrl+Alt+P" 2>&1
    Write-Record "pause/hotkey-resume-press" @{ exitCode = $LASTEXITCODE; raw = (@($press2) | ForEach-Object { "$_" }) -join "`n" }
    Start-Sleep -Milliseconds 1200
    $resumedAux = Invoke-HoldProbe "Ctrl+Q"
    $resumedProbe = Invoke-Probe "Ctrl+Q"
    $resumedProbeAvailable = ($resumedProbe.ok -and $resumedProbe.value.status.available -eq $true)
    Write-Record "pause/verify-resumed" @{
      method = "togglePause-hotkey"
      auxRegistered = $resumedAux.registered
      auxWin32Error = $resumedAux.win32Error
      probeAvailable = $resumedProbeAvailable
      probeStatus = $(if ($resumedProbe.ok) { $resumedProbe.value.status } else { $null })
    }
    $summary["resume_method"] = "togglePause-hotkey"
    $summary["resume_ok"] = (($resumedAux.registered -eq $false) -and $resumedProbeAvailable)
  }

  # =====================================================================
  # Phase E：probe 不泄漏注册（对抗测试）——OS 级探测是「临时 register + 立即 unregister」，
  #   必须证明探测后该键立刻重新可用，不能干扰用户/其它程序。
  # =====================================================================
  if (Test-Phase "probe-leak") {
    if (-not (Get-Process snip-ask -ErrorAction SilentlyContinue)) {
      Start-App | Out-Null
      Start-Sleep -Milliseconds 800
    }
    $leakOk = 0
    $leakFail = 0
    foreach ($spec in @("Ctrl+Shift+Y", "Alt+Shift+9")) {
      for ($i = 1; $i -le 10; $i++) {
        $p1 = Invoke-Probe $spec
        $p2 = Invoke-Probe $spec
        $aux = Invoke-HoldProbe $spec
        $ok = ($p1.ok -and $p1.value.status.available -eq $true) -and
              ($p2.ok -and $p2.value.status.available -eq $true) -and
              ($aux.registered -eq $true)
        if ($ok) { $leakOk++ } else { $leakFail++ }
        Write-Record "probe-leak" @{
          spec = $spec
          i = $i
          ok = $ok
          probe1Available = $(if ($p1.ok) { $p1.value.status.available } else { $null })
          probe2Available = $(if ($p2.ok) { $p2.value.status.available } else { $null })
          auxRegisteredAfterProbe = $aux.registered
          auxWin32Error = $aux.win32Error
        }
      }
    }
    $summary["probeLeak_ok"] = $leakOk
    $summary["probeLeak_fail"] = $leakFail
  }

  Write-Record "run/summary" @{ summary = $summary }
} catch {
  Write-Record "run/error" @{ message = $_.Exception.Message; stack = $_.ScriptStackTrace }
  Write-Record "run/summary" @{ summary = $summary; failed = $true }
} finally {
  try { Stop-App | Out-Null } catch { }
  Write-Record "run/end" @{ rawLog = $RawLog }
  Write-Host ""
  Write-Host "原始日志：$RawLog"
  Write-Host ("摘要：" + ($summary | ConvertTo-Json -Compress -Depth 6))
}
