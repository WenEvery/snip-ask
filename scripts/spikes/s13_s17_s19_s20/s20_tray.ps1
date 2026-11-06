<#
.SYNOPSIS
  S-20 helper: tray host window / tray icon registration / native menu probing (Win32 + UIA + DPI aware).

.DESCRIPTION
  Environment facts recorded by S-04 and re-confirmed by S-20:
    - Win11 hides new tray icons in the "hidden icons" overflow area; when hidden,
      Shell_NotifyIconGetRect fails and tray-icon's WM_USER_TRAYICON handler returns early,
      so TrackPopupMenu never shows the native #32768 menu.
    - This machine's taskbar is auto-hide; when hidden Shell_TrayWnd sits at y~1438 (2px visible).
  This script:
    1) -Action status   : find tray host window (class tray_icon_app), scan Shell_NotifyIconGetRect uIDs,
                          search UIA (taskbar + overflow flyout) for the app tray icon;
    2) -Action showicon : PostMessage WM_USER_SHOW_TRAYICON(6005) + wparam=1 to move the icon
                          from overflow into the notification area;
    3) -Action click    : reveal taskbar (auto-hide), locate icon via UIA, real mouse click, retry x3,
                          check whether the native #32768 menu window appears; fallback PostMessage;
    4) -Action select   : when menu visible, Down x (ItemIndex+1) + Enter (separator skipped by keyboard
                          navigation, so quit uses ItemIndex 4);
    5) -Action windows  : enumerate top-level windows of the app process (class/title/visible/rect).

  Output: one JSON line to stdout. Exit 0 success / 2 target not found / 3 exception.
  ASCII-only strings on purpose (PS 5.1 safety). Call with:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File s20_tray.ps1 -Action status
#>
param(
  [ValidateSet("status", "windows", "showicon", "click", "select", "probe")][string]$Action = "status",
  [int]$ItemIndex = 0,
  [string]$Keyword = "框选|SnipAsk|Snip Ask|snip-ask"
)

$ErrorActionPreference = "Stop"

$script:result = [ordered]@{ ok = $false; action = $Action; steps = @() }
function Add-Step {
  param([string]$Name, [hashtable]$Data = @{})
  $item = [ordered]@{ step = $Name }
  foreach ($k in $Data.Keys) { $item[$k] = $Data[$k] }
  $script:result.steps += $item
}
function Emit {
  param([bool]$Ok = $true, [int]$Code = 0)
  $script:result.ok = $Ok
  Write-Output ($script:result | ConvertTo-Json -Compress -Depth 8)
  exit $Code
}

if (-not ("S20Native" -as [type])) {
  Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
[StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
public struct NOTIFYICONIDENTIFIER { public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem; }
public static class S20Native {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int nMaxCount);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("shell32.dll", SetLastError=true)] public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);

    public static List<IntPtr> EnumByClass(uint pid, string cls) {
        var found = new List<IntPtr>();
        EnumWindows(delegate(IntPtr hWnd, IntPtr l) {
            var sb = new StringBuilder(256); GetClassName(hWnd, sb, 256);
            if (cls.Length == 0 || sb.ToString() == cls) {
                uint p; GetWindowThreadProcessId(hWnd, out p);
                if (pid == 0 || p == pid) found.Add(hWnd);
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static List<Dictionary<string, object>> Windows(uint pid) {
        var list = new List<Dictionary<string, object>>();
        EnumWindows(delegate(IntPtr hWnd, IntPtr l) {
            uint p; GetWindowThreadProcessId(hWnd, out p);
            if (p == pid) {
                var cls = new StringBuilder(256); GetClassName(hWnd, cls, 256);
                var title = new StringBuilder(512); GetWindowTextW(hWnd, title, 512);
                RECT rect; GetWindowRect(hWnd, out rect);
                var item = new Dictionary<string, object>();
                item["handle"] = hWnd.ToInt64();
                item["class"] = cls.ToString();
                item["title"] = title.ToString();
                item["visible"] = IsWindowVisible(hWnd);
                item["rect"] = string.Format("{0},{1},{2},{3}", rect.Left, rect.Top, rect.Right, rect.Bottom);
                list.Add(item);
            }
            return true;
        }, IntPtr.Zero);
        return list;
    }

    public static Dictionary<string, object> ProbeIcon(IntPtr trayHwnd) {
        var found = new Dictionary<string, object>();
        for (uint uid = 0; uid <= 8; uid++) {
            var nid = new NOTIFYICONIDENTIFIER();
            nid.cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER));
            nid.hWnd = trayHwnd;
            nid.uID = uid;
            nid.guidItem = Guid.Empty;
            RECT rect;
            int hr = Shell_NotifyIconGetRect(ref nid, out rect);
            if (hr == 0) {
                found["uid"] = uid;
                found["hr"] = hr;
                found["rect"] = string.Format("{0},{1},{2},{3}", rect.Left, rect.Top, rect.Right, rect.Bottom);
                found["cx"] = (rect.Left + rect.Right) / 2;
                found["cy"] = (rect.Top + rect.Bottom) / 2;
                return found;
            }
        }
        found["uid"] = -1;
        found["hr"] = -1;
        return found;
    }
}
"@ -Language CSharp
}

[void][S20Native]::SetProcessDpiAwarenessContext([IntPtr](-4))

$appProc = Get-Process snip-ask -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $appProc) {
  Add-Step "app" @{ found = $false }
  Emit $false 2
}
Add-Step "app" @{ found = $true; pid = $appProc.Id }

$trayWindows = @([S20Native]::EnumByClass([uint32]$appProc.Id, "tray_icon_app"))
Add-Step "tray-host" @{ count = $trayWindows.Count; handles = @($trayWindows | ForEach-Object { $_.ToInt64() }) }
if ($trayWindows.Count -eq 0) { Emit $false 2 }
$trayHwnd = $trayWindows[0]

$menuHwnd = [S20Native]::FindWindowW("#32768", $null)
$menuVisible = ($menuHwnd -ne [IntPtr]::Zero) -and [S20Native]::IsWindowVisible($menuHwnd)
$script:result.menu = @{ handle = $(if ($menuHwnd -eq [IntPtr]::Zero) { 0 } else { $menuHwnd.ToInt64() }); visible = $menuVisible }

function Get-UiaTarget {
  Add-Type -AssemblyName UIAutomationClient
  Add-Type -AssemblyName UIAutomationTypes
  $root = [System.Windows.Automation.AutomationElement]::RootElement
  $taskbarCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, "Shell_TrayWnd")
  $taskbar = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $taskbarCond)
  $target = $null
  if ($taskbar) {
    $els = $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($el in $els) {
      if ($el.Current.Name -and $el.Current.Name -match $Keyword) {
        $rect0 = $el.Current.BoundingRectangle
        if ($el.Current.Name -notmatch "运行窗口" -and $rect0.Width -gt 0 -and $rect0.Width -le 64) { $target = $el; break }
      }
    }
  }
  if (-not $target) {
    $chev = $null
    if ($taskbar) {
      $els = $taskbar.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
      foreach ($el in $els) { if ($el.Current.Name -and $el.Current.Name -match "隐藏的图标|hidden icons") { $chev = $el; break } }
    }
    if ($chev) {
      try { $chev.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() } catch { }
      Start-Sleep -Milliseconds 900
      $flyCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, "TopLevelWindowForOverflowXamlIsland")
      $fly = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $flyCond)
      if ($fly) {
        $els = $fly.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($el in $els) { if ($el.Current.Name -and $el.Current.Name -match $Keyword) { $target = $el; break } }
      }
    }
  }
  return $target
}

if ($Action -eq "windows") {
  $wins = [S20Native]::Windows([uint32]$appProc.Id)
  $script:result.windows = $wins
  $script:result.visibleCount = @($wins | Where-Object { $_.visible -eq $true }).Count
  Add-Step "enumerate-windows" @{ total = $wins.Count; visible = $script:result.visibleCount }
  Emit $true 0
}

if ($Action -eq "status") {
  $icon = [S20Native]::ProbeIcon($trayHwnd)
  $script:result.icon = $icon
  Add-Step "shell-notify-icon" $icon
  try {
    $target = Get-UiaTarget
    $hits = @()
    if ($target) {
      $r = $target.Current.BoundingRectangle
      $hits += [ordered]@{ name = $target.Current.Name; type = $target.Current.ControlType.ProgrammaticName; rect = "$($r.X),$($r.Y),$($r.Width),$($r.Height)" }
    }
    $script:result.uiaHits = $hits
    Add-Step "uia-search" @{ hitCount = $hits.Count }
  } catch {
    Add-Step "uia-error" @{ message = $_.Exception.Message }
    $script:result.uiaHits = @()
  }
  Emit $true 0
}

if ($Action -eq "showicon") {
  $posted = [S20Native]::PostMessageW($trayHwnd, [uint32]6005, [IntPtr]1, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 900
  $icon = [S20Native]::ProbeIcon($trayHwnd)
  Add-Step "post-show-icon" @{ posted = [bool]$posted }
  $script:result.icon = $icon
  Emit ($icon.uid -ge 0) 0
}

if ($Action -eq "click") {
  # auto-hide taskbar: move cursor to bottom edge to reveal it first
  [void][S20Native]::SetCursorPos(1280, 1439)
  Start-Sleep -Milliseconds 900
  $clicked = $false
  $mode = "none"
  $attempts = New-Object System.Collections.Generic.List[object]
  for ($attempt = 1; $attempt -le 3 -and -not $clicked; $attempt++) {
    try {
      $target = Get-UiaTarget
      if ($target) {
        $r = $target.Current.BoundingRectangle
        $cx = [int]($r.X + $r.Width / 2); $cy = [int]($r.Y + $r.Height / 2)
        $attempts.Add([ordered]@{ attempt = $attempt; name = $target.Current.Name; rect = "$($r.X),$($r.Y),$($r.Width),$($r.Height)"; cx = $cx; cy = $cy })
        [void][S20Native]::SetCursorPos($cx, $cy)
        Start-Sleep -Milliseconds 300
        [S20Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 90
        [S20Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 1100
        $tryMenu = [S20Native]::FindWindowW("#32768", $null)
        if (($tryMenu -ne [IntPtr]::Zero) -and [S20Native]::IsWindowVisible($tryMenu)) { $clicked = $true; $mode = "uia-real-mouse" }
      } else {
        $attempts.Add([ordered]@{ attempt = $attempt; found = $false })
        Start-Sleep -Milliseconds 700
      }
    } catch {
      $attempts.Add([ordered]@{ attempt = $attempt; error = $_.Exception.Message })
    }
  }
  $script:result.clickAttempts = $attempts
  if (-not $clicked) {
    [void][S20Native]::PostMessageW($trayHwnd, [uint32]6005, [IntPtr]1, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    [void][S20Native]::PostMessageW($trayHwnd, [uint32]6002, [IntPtr]::Zero, [IntPtr]0x0202)
    Start-Sleep -Milliseconds 900
    $mode = "postmessage"
  }
  $menuHwnd = [S20Native]::FindWindowW("#32768", $null)
  $menuVisible = ($menuHwnd -ne [IntPtr]::Zero) -and [S20Native]::IsWindowVisible($menuHwnd)
  $script:result.menu = @{ handle = $(if ($menuHwnd -eq [IntPtr]::Zero) { 0 } else { $menuHwnd.ToInt64() }); visible = $menuVisible }
  Add-Step "menu-after-click" @{ mode = $mode; visible = $menuVisible }
  Emit $menuVisible 0
}

if ($Action -eq "select") {
  $menuHwnd = [S20Native]::FindWindowW("#32768", $null)
  $menuVisible = ($menuHwnd -ne [IntPtr]::Zero) -and [S20Native]::IsWindowVisible($menuHwnd)
  if (-not $menuVisible) { Add-Step "select-precondition" @{ menuVisible = $false }; Emit $false 2 }
  $VK_DOWN = 0x28; $VK_RETURN = 0x0D; $KEYEVENTF_KEYUP = 0x0002
  $downCount = $ItemIndex + 1
  for ($i = 0; $i -lt $downCount; $i++) {
    [S20Native]::keybd_event($VK_DOWN, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 50
    [S20Native]::keybd_event($VK_DOWN, 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 70
  }
  [S20Native]::keybd_event($VK_RETURN, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 50
  [S20Native]::keybd_event($VK_RETURN, 0, $KEYEVENTF_KEYUP, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 500
  $after = [S20Native]::FindWindowW("#32768", $null)
  $stillVisible = ($after -ne [IntPtr]::Zero) -and [S20Native]::IsWindowVisible($after)
  Add-Step "select-item" @{ itemIndex = $ItemIndex; downCount = $downCount; menuStillVisible = $stillVisible }
  Emit (-not $stillVisible) 0
}

if ($Action -eq "probe") {
  $icon = [S20Native]::ProbeIcon($trayHwnd)
  $script:result.icon = $icon
  Add-Step "probe-icon" $icon
  Emit $true 0
}

Emit $false 3
