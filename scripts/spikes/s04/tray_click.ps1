<#
.SYNOPSIS
  S-04 辅助工具：通过 Win32 直接触发产品托盘菜单（暂停 / 恢复热键的唯一非热键入口）。

.DESCRIPTION
  背景：产品没有暴露 pause 的 IPC 命令；暂停后 togglePause 热键本身也被注销，只能通过
  托盘菜单「暂停热键」恢复。托盘菜单由 tray-icon crate 创建：
    - 宿主是一个隐藏窗口，窗口类名固定为 "tray_icon_app"，菜单在收到
      WM_USER_TRAYICON(6002) + lParam=WM_LBUTTONUP 时用 TrackPopupMenu 弹出；
    - 菜单项顺序由 app/tray.rs 固定：开始框选 / 设置 / 重新运行引导 / 暂停热键 / -- / 退出。
  因此本脚本不依赖 UIA 名称：按类名 + 进程名找到托盘宿主窗口，PostMessage 弹菜单，
  再用真实键盘事件（Down ×N + Enter）选择「暂停热键」。暂停状态只改托盘 tooltip，
  菜单文案不变，所以恢复用同一项。

.PARAMETER ItemIndex
  菜单项零基下标（默认 3 = 「暂停热键」，见 app/tray.rs 的菜单顺序）。

.PARAMETER PauseMs
  PostMessage 后等待菜单弹出的毫秒数（默认 500）。

.PARAMETER ListOnly
  只打印找到的托盘宿主窗口信息与菜单窗口，不点击（调试用）。

.EXAMPLE
  pwsh -NoProfile -File tray_click.ps1
  pwsh -NoProfile -File tray_click.ps1 -ItemIndex 3

.OUTPUTS
  stdout 一行 JSON；退出码 0 成功 / 2 未找到托盘窗口或菜单 / 3 异常。
#>
param(
  [int]$ItemIndex = 3,
  [int]$PauseMs = 500,
  [switch]$ListOnly
)

$ErrorActionPreference = "Stop"
$script:steps = New-Object System.Collections.Generic.List[object]

function Add-Step {
  param([string]$Name, [hashtable]$Data = @{})
  $item = [ordered]@{ step = $Name }
  foreach ($k in $Data.Keys) { $item[$k] = $Data[$k] }
  $script:steps.Add($item)
}

function Emit-Result {
  param([bool]$Ok, [string]$Message)
  $payload = [ordered]@{ ok = $Ok; message = $Message; itemIndex = $ItemIndex; steps = $script:steps }
  Write-Output ($payload | ConvertTo-Json -Compress -Depth 8)
}

if (-not ("S04TrayNative" -as [type])) {
  Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class S04TrayNative {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll", SetLastError = true)] public static extern IntPtr FindWindowW(string lpClassName, string lpWindowName);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool PostMessageW(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);

    public static List<IntPtr> FindTrayWindows(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows(delegate(IntPtr hWnd, IntPtr lParam) {
            var sb = new StringBuilder(256);
            GetClassName(hWnd, sb, 256);
            if (sb.ToString() == "tray_icon_app") {
                uint winPid;
                GetWindowThreadProcessId(hWnd, out winPid);
                if (pid == 0 || winPid == pid) { found.Add(hWnd); }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@ -Language CSharp
}

# ---- 1) 定位应用进程与托盘宿主窗口 ----
$appProc = Get-Process snip-ask -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $appProc) {
  Emit-Result $false "未找到 snip-ask 进程（应用未运行）"
  exit 2
}
Add-Step "app-process" @{ pid = $appProc.Id; name = $appProc.ProcessName }

$trayWindows = @([S04TrayNative]::FindTrayWindows([uint32]$appProc.Id))
Add-Step "find-tray-window" @{ count = $trayWindows.Count; handles = @($trayWindows | ForEach-Object { $_.ToInt64() }) }
if ($trayWindows.Count -eq 0) {
  Emit-Result $false "未找到窗口类 tray_icon_app（pid=$($appProc.Id)）"
  exit 2
}
$trayHwnd = $trayWindows[0]

$menuWindow = [S04TrayNative]::FindWindowW("#32768", $null)
Add-Step "menu-window-before" @{ handle = $(if ($menuWindow -eq [IntPtr]::Zero) { 0 } else { $menuWindow.ToInt64() }) }

if ($ListOnly) {
  Emit-Result $true "list-only"
  exit 0
}

# ---- 2) PostMessage 触发弹菜单（WM_USER_TRAYICON=6002, lParam=WM_LBUTTONUP=0x0202） ----
$WM_USER_TRAYICON = [uint32]6002
$WM_LBUTTONUP = [int]0x0202
$posted = [S04TrayNative]::PostMessageW($trayHwnd, $WM_USER_TRAYICON, [IntPtr]::Zero, [IntPtr]$WM_LBUTTONUP)
Add-Step "post-show-menu" @{ posted = [bool]$posted; win32Error = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error() }

Start-Sleep -Milliseconds $PauseMs

$menuWindow = [S04TrayNative]::FindWindowW("#32768", $null)
$menuVisible = ($menuWindow -ne [IntPtr]::Zero) -and [S04TrayNative]::IsWindowVisible($menuWindow)
Add-Step "menu-window-after" @{ handle = $(if ($menuWindow -eq [IntPtr]::Zero) { 0 } else { $menuWindow.ToInt64() }); visible = $menuVisible }
if (-not $menuVisible) {
  # 重试一次：有些情况下 SetForegroundWindow/TrackPopupMenu 需要一次额外唤醒
  [S04TrayNative]::PostMessageW($trayHwnd, $WM_USER_TRAYICON, [IntPtr]::Zero, [IntPtr]$WM_LBUTTONUP) | Out-Null
  Start-Sleep -Milliseconds $PauseMs
  $menuWindow = [S04TrayNative]::FindWindowW("#32768", $null)
  $menuVisible = ($menuWindow -ne [IntPtr]::Zero) -and [S04TrayNative]::IsWindowVisible($menuWindow)
  Add-Step "menu-window-retry" @{ handle = $(if ($menuWindow -eq [IntPtr]::Zero) { 0 } else { $menuWindow.ToInt64() }); visible = $menuVisible }
}
if (-not $menuVisible) {
  Emit-Result $false "托盘菜单未弹出（未发现可见的 #32768 窗口）"
  exit 2
}

# ---- 3) 键盘选择菜单项：Down ×(ItemIndex+1) 后 Enter ----
$VK_DOWN = 0x28
$VK_RETURN = 0x0D
$KEYEVENTF_KEYUP = 0x0002
$downCount = $ItemIndex + 1
for ($i = 0; $i -lt $downCount; $i++) {
  [S04TrayNative]::keybd_event($VK_DOWN, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 40
  [S04TrayNative]::keybd_event($VK_DOWN, 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 60
}
Add-Step "select-item" @{ downCount = $downCount }
[S04TrayNative]::keybd_event($VK_RETURN, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 40
[S04TrayNative]::keybd_event($VK_RETURN, 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 300

$menuAfter = [S04TrayNative]::FindWindowW("#32768", $null)
$stillVisible = ($menuAfter -ne [IntPtr]::Zero) -and [S04TrayNative]::IsWindowVisible($menuAfter)
Add-Step "menu-closed" @{ stillVisible = $stillVisible }

Emit-Result $true "已通过托盘菜单选择第 $ItemIndex 项（0=开始框选,1=设置,2=重新运行引导,3=暂停热键）"
exit 0
