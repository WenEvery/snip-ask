<#
.SYNOPSIS
  S-04 辅助工具：用 Win32 RegisterHotKey 占住（或探测）一个全局热键。

.DESCRIPTION
  等价模拟 QQ / 微信 / 输入法等第三方程序抢占 Ctrl+Q（本机未安装这些软件，无法实机复现，
  因此用**同键 RegisterHotKey** 这个 OS 级等价手段，见 docs/spikes/S-04-*.md「局限性」）。

  行为：
    - HoldSec > 0 ：注册成功后保持 N 秒（占住），期间状态写入 -StatusFile，退出前 UnregisterHotKey；
    - HoldSec <= 0：仅探测（注册成功后立即注销），用于判断「该组合当前是否空闲」。

  与产品侧口径一致：产品用 tauri-plugin-global-shortcut（底层 global-hotkey 以
  `RegisterHotKey(hwnd, id, MOD_NOREPEAT|mods, vk)` 注册），本脚本用
  `RegisterHotKey(NULL, ...)` 注册到本线程消息队列；二者都在同一桌面的全局热键表里，
  同一组合互斥，因此 Win32 返回 1409（ERROR_HOTKEY_ALREADY_REGISTERED）即为「被占用」。

.PARAMETER Spec
  组合键，形如 Ctrl+Q / Ctrl+Shift+Y / Alt+Shift+9（契约 §4.3 同款格式，大小写不敏感）。

.PARAMETER HoldSec
  保持秒数；0 或负数表示仅探测（注册后立即注销）。

.PARAMETER StatusFile
  可选：把每次状态 JSON 追加写入该文件（供编排脚本轮询），目录会自动创建。

.EXAMPLE
  pwsh -NoProfile -File hold_hotkey.ps1 -Spec Ctrl+Q -HoldSec 0
  pwsh -NoProfile -File hold_hotkey.ps1 -Spec Ctrl+Q -HoldSec 60 -StatusFile .verify/spikes/S-04/tmp/hold.json

.OUTPUTS
  stdout 每行一个 JSON：{"phase":"registered|released|error","spec":"Ctrl+Q","registered":true,...}
  退出码：0 = 注册成功；2 = 注册失败（通常是 1409 已被占用）；3 = 参数错误 / 环境不支持。
#>
param(
  [string]$Spec = "Ctrl+Q",
  [int]$HoldSec = 15,
  [string]$StatusFile = ""
)

$ErrorActionPreference = "Stop"
$HotkeyId = 0x5104

function Write-StatusFile {
  param([hashtable]$Record)
  if (-not $StatusFile) { return }
  $dir = Split-Path -Parent $StatusFile
  if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $line = ($Record | ConvertTo-Json -Compress)
  Add-Content -LiteralPath $StatusFile -Value $line -Encoding UTF8
}

function Emit {
  param([hashtable]$Record)
  $Record.pid = $PID
  $Record.ts = [DateTimeOffset]::UtcNow.ToString("o")
  $line = ($Record | ConvertTo-Json -Compress)
  Write-Output $line
  Write-StatusFile $Record
}

function Fail {
  param([string]$Message, [int]$Code = 3)
  Emit @{ phase = "error"; spec = $Spec; message = $Message; exitCode = $Code }
  exit $Code
}

# ---- 1) 解析组合键（与契约 §4.3 同款格式） ----
$trimmed = $Spec.Trim()
$parts = $trimmed -split "\+"
if ($parts.Count -lt 2) { Fail "组合键非法（需 修饰键+主键）：$Spec" }
$mods = 0
$modNames = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $parts.Count - 1; $i++) {
  $name = $parts[$i].Trim().ToLowerInvariant()
  switch ($name) {
    "ctrl" { $mods = $mods -bor 0x0002; $modNames.Add("Ctrl") }
    "alt" { $mods = $mods -bor 0x0001; $modNames.Add("Alt") }
    "shift" { $mods = $mods -bor 0x0004; $modNames.Add("Shift") }
    "win" { $mods = $mods -bor 0x0008; $modNames.Add("Win") }
    default { Fail "未知修饰键：$($parts[$i])" }
  }
}
$keyText = $parts[$parts.Count - 1].Trim().ToUpperInvariant()
$vk = -1
if ($keyText -match '^[A-Z]$') { $vk = [int][char]$keyText }
elseif ($keyText -match '^[0-9]$') { $vk = [int][char]$keyText }
else { Fail "主键非法（仅支持 A-Z / 0-9）：$keyText" }

# ---- 2) P/Invoke（产品代码禁 unsafe；辅助脚本可用 Add-Type） ----
if (-not ("S04HotkeyNative" -as [type])) {
  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class S04HotkeyNative {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr hWnd, int id);
}
"@ -Language CSharp
}

$MOD_NOREPEAT = 0x4000
$flags = [uint32]($mods -bor $MOD_NOREPEAT)

# ---- 3) 注册 ----
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$ok = [S04HotkeyNative]::RegisterHotKey([IntPtr]::Zero, $HotkeyId, $flags, [uint32]$vk)
$sw.Stop()
$lastError = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
$registerUs = [int64]($sw.Elapsed.TotalMilliseconds * 1000)

if (-not $ok) {
  $hint = switch ($lastError) {
    1409 { "ERROR_HOTKEY_ALREADY_REGISTERED：该组合已被本机其它程序/线程占用" }
    0 { "RegisterHotKey 返回 false 但 GetLastError=0（少见，可能被过滤）" }
    default { "Win32 错误码 $lastError" }
  }
  Emit @{
    phase = "registered"
    spec = $trimmed
    normalized = (($modNames -join "+") + "+" + $keyText)
    registered = $false
    win32Error = $lastError
    hint = $hint
    holdSec = $HoldSec
    registerUs = $registerUs
  }
  exit 2
}

Emit @{
  phase = "registered"
  spec = $trimmed
  normalized = (($modNames -join "+") + "+" + $keyText)
  registered = $true
  win32Error = 0
  holdSec = $HoldSec
  registerUs = $registerUs
}

if ($HoldSec -le 0) {
  # 仅探测：立即释放
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $unregistered = [S04HotkeyNative]::UnregisterHotKey([IntPtr]::Zero, $HotkeyId)
  $sw.Stop()
  $unregError = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
  Emit @{
    phase = "released"
    spec = $trimmed
    unregistered = [bool]$unregistered
    win32Error = $unregError
    unregisterUs = [int64]($sw.Elapsed.TotalMilliseconds * 1000)
    probeOnly = $true
  }
  exit 0
}

# ---- 4) 占住指定时长（本线程存活即保持注册；无需消息循环） ----
Start-Sleep -Seconds $HoldSec

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$unregistered = [S04HotkeyNative]::UnregisterHotKey([IntPtr]::Zero, $HotkeyId)
$sw.Stop()
$unregError = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
Emit @{
  phase = "released"
  spec = $trimmed
  unregistered = [bool]$unregistered
  win32Error = $unregError
  unregisterUs = [int64]($sw.Elapsed.TotalMilliseconds * 1000)
  probeOnly = $false
}
exit 0
