#requires -Version 5.1
<#
.SYNOPSIS
    S-01 overlay 窗口几何探针（DPI 感知，物理像素）。

.DESCRIPTION
    枚举当前进程的顶层窗口，找出 snip-ask 的 overlay 窗口（标题「框选 AI · 选框」），
    报告其物理矩形 / client 尺寸 / DPI / DWM 扩展边框，并与虚拟桌面（所有显示器外接矩形）
    比较，输出 JSON + 断言结果。

    用途：回答「overlay 到底覆盖没覆盖副屏」，是 S-01 修复前/后的实测证据。
    进程必须先切 PerMonitorV2，否则 GetWindowRect 会返回 DPI 虚拟化坐标。

.PARAMETER Out
    输出 JSON 路径（相对仓库根目录或绝对）。默认 .verify/spikes/S-01/s01_overlay_probe.json。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\spikes\s01\s01_overlay_probe.ps1
#>
param(
    [string]$Out = ".verify/spikes/S-01/s01_overlay_probe.json",
    [int]$TolerancePx = 2
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $RepoRoot $Out }
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class S01ProbeWindow {
    public long HWND;
    public int Pid;
    public string ProcessName = "";
    public string Title = "";
    public string ClassName = "";
    public bool Visible;
    public int X; public int Y; public int W; public int H;          // GetWindowRect（物理）
    public int Cx; public int Cy; public int Cw; public int Ch;      // GetClientRect
    public int DwmX; public int DwmY; public int DwmW; public int DwmH;
    public bool DwmOk;
    public int ClientX; public int ClientY;                            // ClientToScreen(0,0) 物理坐标
    public long Style;
    public uint Dpi;
    public long ExStyle;
}

public static class S01ProbeNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }

    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc proc, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, StringBuilder text, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr hWnd, out int pid);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr hWnd, int index);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out RECT value, int size);

    public const int GWL_STYLE = -16;
    public const int GWL_EXSTYLE = -20;
    public const int DWMWA_EXTENDED_FRAME_BOUNDS = 9;

    public static int EnablePerMonitorV2() {
        return SetProcessDpiAwarenessContext(new IntPtr(-4)) ? 1 : 0;
    }

    public static List<S01ProbeWindow> EnumTopLevel() {
        List<S01ProbeWindow> result = new List<S01ProbeWindow>();
        EnumWindowsProc proc = delegate(IntPtr hWnd, IntPtr lParam) {
            S01ProbeWindow item = new S01ProbeWindow();
            item.HWND = hWnd.ToInt64();
            int pid = 0;
            GetWindowThreadProcessId(hWnd, out pid);
            item.Pid = pid;
            StringBuilder sb = new StringBuilder(512);
            GetWindowTextW(hWnd, sb, sb.Capacity);
            item.Title = sb.ToString();
            StringBuilder cls = new StringBuilder(256);
            GetClassNameW(hWnd, cls, cls.Capacity);
            item.ClassName = cls.ToString();
            item.Visible = IsWindowVisible(hWnd);
            RECT r;
            if (GetWindowRect(hWnd, out r)) {
                item.X = r.Left; item.Y = r.Top;
                item.W = r.Right - r.Left; item.H = r.Bottom - r.Top;
            }
            RECT cr;
            if (GetClientRect(hWnd, out cr)) {
                item.Cx = cr.Left; item.Cy = cr.Top;
                item.Cw = cr.Right - cr.Left; item.Ch = cr.Bottom - cr.Top;
            }
            RECT dwm;
            item.DwmOk = DwmGetWindowAttribute(hWnd, DWMWA_EXTENDED_FRAME_BOUNDS, out dwm, Marshal.SizeOf(typeof(RECT))) == 0;
            if (item.DwmOk) {
                item.DwmX = dwm.Left; item.DwmY = dwm.Top;
                item.DwmW = dwm.Right - dwm.Left; item.DwmH = dwm.Bottom - dwm.Top;
            }
            item.Dpi = GetDpiForWindow(hWnd);
            item.Style = GetWindowLongPtrW(hWnd, GWL_STYLE).ToInt64();
            item.ExStyle = GetWindowLongPtrW(hWnd, GWL_EXSTYLE).ToInt64();
            POINT origin;
            origin.X = 0; origin.Y = 0;
            if (ClientToScreen(hWnd, ref origin)) { item.ClientX = origin.X; item.ClientY = origin.Y; }
            result.Add(item);
            return true;
        };
        EnumWindows(proc, IntPtr.Zero);
        GC.KeepAlive(proc);
        return result;
    }
}
"@

$aware = [S01ProbeNative]::EnablePerMonitorV2()
$windows = [S01ProbeNative]::EnumTopLevel()

# 目标进程名判定：snip-ask（便携 exe 同理）
$all = @()
foreach ($w in $windows) {
    $procName = ""
    try { $procName = (Get-Process -Id $w.Pid -ErrorAction Stop).ProcessName } catch { $procName = "?" }
    $w.ProcessName = $procName
    if ($procName -like "snip-ask*") { $all += $w }
}

$target = $all | Where-Object { $_.Title -like "*选框*" } | Select-Object -First 1
if (-not $target) {
    # 兜底：非 main/result 且可见的 snip-ask 窗口
    $target = $all | Where-Object { $_.Visible -and $_.Title -notlike "*设置*" } | Select-Object -First 1
}

# 虚拟桌面 = 所有显示器外接矩形
$monitors = @()
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class S01ProbeMonitors {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct MONITORINFOEX { public int cbSize; public RECT rcMonitor; public RECT rcWork; public uint dwFlags; [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice; }
    public delegate bool MonitorEnumProc(IntPtr hMonitor, IntPtr hdc, ref RECT rect, IntPtr data);
    [DllImport("user32.dll")] public static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnumProc proc, IntPtr data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool GetMonitorInfoW(IntPtr hMonitor, ref MONITORINFOEX mi);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr hmonitor, int dpiType, out uint dpiX, out uint dpiY);
    public static List<S01ProbeMonitor> Enum() {
        List<S01ProbeMonitor> result = new List<S01ProbeMonitor>();
        MonitorEnumProc proc = delegate(IntPtr hMon, IntPtr hdc, ref RECT rect, IntPtr data) {
            S01ProbeMonitor m = new S01ProbeMonitor();
            MONITORINFOEX mi = new MONITORINFOEX();
            mi.cbSize = Marshal.SizeOf(typeof(MONITORINFOEX));
            if (GetMonitorInfoW(hMon, ref mi)) {
                m.X = mi.rcMonitor.Left; m.Y = mi.rcMonitor.Top;
                m.W = mi.rcMonitor.Right - mi.rcMonitor.Left; m.H = mi.rcMonitor.Bottom - mi.rcMonitor.Top;
                m.Device = mi.szDevice == null ? "" : mi.szDevice;
                m.Primary = (mi.dwFlags & 1) != 0;
            }
            uint dx = 0, dy = 0;
            if (GetDpiForMonitor(hMon, 0, out dx, out dy) == 0) { m.Dpi = dx; }
            result.Add(m);
            return true;
        };
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, proc, IntPtr.Zero);
        GC.KeepAlive(proc);
        return result;
    }
}
public class S01ProbeMonitor { public int X; public int Y; public int W; public int H; public uint Dpi; public string Device = ""; public bool Primary; }
"@
$mons = [S01ProbeMonitors]::Enum()
$vdLeft = ($mons | ForEach-Object { $_.X } | Measure-Object -Minimum).Minimum
$vdTop = ($mons | ForEach-Object { $_.Y } | Measure-Object -Minimum).Minimum
$vdW = (($mons | ForEach-Object { $_.X + $_.W } | Measure-Object -Maximum).Maximum) - $vdLeft
$vdH = (($mons | ForEach-Object { $_.Y + $_.H } | Measure-Object -Maximum).Maximum) - $vdTop

$result = [ordered]@{
    generatedAt = (Get-Date -Format "o")
    dpiAware    = if ($aware -eq 1) { "PER_MONITOR_AWARE_V2(-4)" } else { "FAILED" }
    virtualDesktop = [ordered]@{ x = $vdLeft; y = $vdTop; width = $vdW; height = $vdH }
    monitors = @($mons | ForEach-Object { [ordered]@{ device = $_.Device; primary = $_.Primary; rect = [ordered]@{ x = $_.X; y = $_.Y; width = $_.W; height = $_.H }; dpi = $_.Dpi } })
    snipAskWindows = @($all | ForEach-Object {
            [ordered]@{
                hwnd = $_.HWND; title = $_.Title; className = $_.ClassName; visible = $_.Visible
                rect = [ordered]@{ x = $_.X; y = $_.Y; width = $_.W; height = $_.H }
                client = [ordered]@{ width = $_.Cw; height = $_.Ch }
                dwmFrame = if ($_.DwmOk) { [ordered]@{ x = $_.DwmX; y = $_.DwmY; width = $_.DwmW; height = $_.DwmH } } else { $null }
                dpi = $_.Dpi
                topmost = (($_.ExStyle -band 0x8) -ne 0)
            }
        })
    overlay = $null
    assertion = $null
}

if ($target) {
    $vdMatch = ([math]::Abs($target.X - $vdLeft) -le $TolerancePx -and
        [math]::Abs($target.Y - $vdTop) -le $TolerancePx -and
        [math]::Abs($target.W - $vdW) -le $TolerancePx -and
        [math]::Abs($target.H - $vdH) -le $TolerancePx)
    # 前端坐标换算真正依赖的是 client 区：client(0,0) 必须落在虚拟桌面左上角，
    # client 尺寸必须等于虚拟桌面物理尺寸。
    $clientMatch = ([math]::Abs($target.ClientX - $vdLeft) -le $TolerancePx -and
        [math]::Abs($target.ClientY - $vdTop) -le $TolerancePx -and
        [math]::Abs($target.Cw - $vdW) -le $TolerancePx -and
        [math]::Abs($target.Ch - $vdH) -le $TolerancePx)
    $result.overlay = [ordered]@{
        found = $true
        hwnd = $target.HWND
        title = $target.Title
        visible = $target.Visible
        rect = [ordered]@{ x = $target.X; y = $target.Y; width = $target.W; height = $target.H }
        client = [ordered]@{ x = $target.ClientX; y = $target.ClientY; width = $target.Cw; height = $target.Ch }
        dwmFrame = if ($target.DwmOk) { [ordered]@{ x = $target.DwmX; y = $target.DwmY; width = $target.DwmW; height = $target.DwmH } } else { $null }
        dpi = $target.Dpi
        scale = [math]::Round($target.Dpi / 96.0, 4)
        topmost = (($target.ExStyle -band 0x8) -ne 0)
        styles = [ordered]@{
            hasCaption = (($target.Style -band 0x00C00000) -ne 0)
            hasThickFrame = (($target.Style -band 0x00040000) -ne 0)
            isPopup = (($target.Style -band 0x80000000) -ne 0)
            layered = (($target.ExStyle -band 0x00080000) -ne 0)
        }
        deltaFromVirtualDesktop = [ordered]@{
            dx = $target.X - $vdLeft; dy = $target.Y - $vdTop
            dwidth = $target.W - $vdW; dheight = $target.H - $vdH
            clientDx = $target.ClientX - $vdLeft; clientDy = $target.ClientY - $vdTop
            clientDwidth = $target.Cw - $vdW; clientDheight = $target.Ch - $vdH
        }
    }
    $result.assertion = [ordered]@{
        tolerancePx         = $TolerancePx
        overlayEqualsVirtualDesktop = $vdMatch
        clientEqualsVirtualDesktop  = $clientMatch
        expectedRect        = "$vdLeft,$vdTop ${vdW}x${vdH}"
        actualRect          = "$($target.X),$($target.Y) $($target.W)x$($target.H)"
        actualClient        = "$($target.ClientX),$($target.ClientY) $($target.Cw)x$($target.Ch)"
    }
} else {
    $result.overlay = [ordered]@{ found = $false }
    $result.assertion = [ordered]@{
        tolerancePx = $TolerancePx
        overlayEqualsVirtualDesktop = $false
        clientEqualsVirtualDesktop = $false
        expectedRect = "$vdLeft,$vdTop ${vdW}x${vdH}"
        actualRect = "(未找到 overlay 窗口)"
        actualClient = "(未找到 overlay 窗口)"
    }
}

$result | ConvertTo-Json -Depth 10 | Set-Content -Path $Out -Encoding utf8

Write-Host "== S-01 overlay 几何探针 ==" -ForegroundColor Cyan
Write-Host ("进程 DPI 感知：{0}" -f $result.dpiAware)
Write-Host ("虚拟桌面：x={0} y={1} {2}x{3}" -f $vdLeft, $vdTop, $vdW, $vdH)
$mons | ForEach-Object { Write-Host ("  显示器 {0} rect=({1},{2}) {3}x{4} dpi={5}" -f $_.Device, $_.X, $_.Y, $_.W, $_.H, $_.Dpi) }
if ($target) {
    Write-Host ("overlay HWND={0} 标题=[{1}] visible={2}" -f $target.HWND, $target.Title, $target.Visible)
    Write-Host ("  GetWindowRect = ({0},{1}) {2}x{3}   clientOrigin=({4},{5}) clientSize={6}x{7}   dpi={8} (scale {9})  topmost={10}" -f `
            $target.X, $target.Y, $target.W, $target.H, $target.ClientX, $target.ClientY, $target.Cw, $target.Ch, $target.Dpi, ([math]::Round($target.Dpi / 96.0, 2)), (($target.ExStyle -band 0x8) -ne 0))
    if ($target.DwmOk) { Write-Host ("  DWM frame    = ({0},{1}) {2}x{3}" -f $target.DwmX, $target.DwmY, $target.DwmW, $target.DwmH) }
    Write-Host ("  样式：caption={0} thickFrame={1} popup={2} layered={3}" -f `
            (($target.Style -band 0x00C00000) -ne 0), (($target.Style -band 0x00040000) -ne 0), (($target.Style -band 0x80000000) -ne 0), (($target.ExStyle -band 0x00080000) -ne 0))
}
Write-Host ("断言：窗口外框 == 虚拟桌面（≤{0}px）：{1}" -f $TolerancePx, $result.assertion.overlayEqualsVirtualDesktop) -ForegroundColor $(if ($result.assertion.overlayEqualsVirtualDesktop) { "Green" } else { "Red" })
Write-Host ("断言：client 区原点是虚拟桌面左上角且尺寸一致（≤{0}px）：{1}" -f $TolerancePx, $result.assertion.clientEqualsVirtualDesktop) -ForegroundColor $(if ($result.assertion.clientEqualsVirtualDesktop) { "Green" } else { "Red" })
Write-Host ("  期望外框 {0}  实际 {1}" -f $result.assertion.expectedRect, $result.assertion.actualRect)
Write-Host ("  期望 client {0}  实际 {1}" -f $result.assertion.expectedRect, $result.assertion.actualClient)
Write-Host ("输出：{0}" -f $Out)
