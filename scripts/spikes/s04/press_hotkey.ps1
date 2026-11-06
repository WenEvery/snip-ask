<#
.SYNOPSIS
  S-04 辅助工具：向系统注入一次组合键按键（触发应用已注册的全局热键）。

.DESCRIPTION
  用途：真机验证「暂停后热键完全释放 / 恢复后重新注册」时，需要一个可控方式触发
  `hotkeys.togglePause` 绑定。产品没有暴露 pause 的 IPC 命令（入口只有托盘菜单与
  togglePause 热键），这里用 keybd_event 生成真实键盘输入，由 OS 投递 WM_HOTKEY，
  走的是与非自动化场景完全相同的全局热键链路。

  注意：keybd_event 生成的输入会真实影响当前前台程序（本脚本只按下组合键，不输入文本）。

.PARAMETER Spec
  组合键（契约 §4.3 格式），如 Ctrl+Alt+P。

.PARAMETER HoldMs
  按下与释放之间的保持毫秒数，默认 40ms。

.EXAMPLE
  pwsh -NoProfile -File press_hotkey.ps1 -Spec Ctrl+Alt+P

.OUTPUTS
  stdout 一行 JSON：{"phase":"pressed","spec":"Ctrl+Alt+P","ok":true,...}；退出码 0 / 3。
#>
param(
  [string]$Spec = "Ctrl+Alt+P",
  [int]$HoldMs = 40
)

$ErrorActionPreference = "Stop"

function Fail {
  param([string]$Message)
  Write-Output ((@{ phase = "error"; spec = $Spec; message = $Message } | ConvertTo-Json -Compress))
  exit 3
}

$trimmed = $Spec.Trim()
$parts = $trimmed -split "\+"
if ($parts.Count -lt 2) { Fail "组合键非法（需 修饰键+主键）：$Spec" }

$vkList = New-Object System.Collections.Generic.List[uint32]
$modNames = New-Object System.Collections.Generic.List[string]
for ($i = 0; $i -lt $parts.Count - 1; $i++) {
  switch ($parts[$i].Trim().ToLowerInvariant()) {
    "ctrl" { $vkList.Add(0x11); $modNames.Add("Ctrl") }
    "alt" { $vkList.Add(0x12); $modNames.Add("Alt") }
    "shift" { $vkList.Add(0x10); $modNames.Add("Shift") }
    "win" { $vkList.Add(0x5B); $modNames.Add("Win") }
    default { Fail "未知修饰键：$($parts[$i])" }
  }
}
$keyText = $parts[$parts.Count - 1].Trim().ToUpperInvariant()
if ($keyText -match '^[A-Z]$') { $vkList.Add([uint32][char]$keyText) }
elseif ($keyText -match '^[0-9]$') { $vkList.Add([uint32][char]$keyText) }
else { Fail "主键非法（仅支持 A-Z / 0-9）：$keyText" }

if (-not ("S04InputNative" -as [type])) {
  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class S04InputNative {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
"@ -Language CSharp
}

$KEYEVENTF_KEYUP = 0x0002
$sw = [System.Diagnostics.Stopwatch]::StartNew()
try {
  # 修饰键按下 → 主键按下 → 主键抬起 → 修饰键反序抬起
  for ($i = 0; $i -lt $vkList.Count - 1; $i++) {
    [S04InputNative]::keybd_event([byte]$vkList[$i], 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 8
  }
  $mainVk = $vkList[$vkList.Count - 1]
  [S04InputNative]::keybd_event([byte]$mainVk, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds $HoldMs
  [S04InputNative]::keybd_event([byte]$mainVk, 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
  for ($i = $vkList.Count - 2; $i -ge 0; $i--) {
    Start-Sleep -Milliseconds 8
    [S04InputNative]::keybd_event([byte]$vkList[$i], 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
  }
} catch {
  Fail "keybd_event 注入失败：$($_.Exception.Message)"
}
$sw.Stop()

Write-Output ((@{
  phase = "pressed"
  spec = $trimmed
  normalized = (($modNames -join "+") + "+" + $keyText)
  ok = $true
  elapsedMs = [math]::Round($sw.Elapsed.TotalMilliseconds, 3)
} | ConvertTo-Json -Compress))
exit 0
