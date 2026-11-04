#requires -Version 5.1
<#
.SYNOPSIS
    DPI 感知的虚拟桌面截图（spike 共用；用于 overlay / 混合 DPI 的实机取证）。

.DESCRIPTION
    默认对「整个虚拟桌面」做 BitBlt 截图——包含所有显示器、负坐标区域，输出物理像素尺寸 PNG。
    进程先切换为 PerMonitorV2 DPI 感知，避免 150% 缩放下截到被系统放大的虚化画面。
    也可只截一块物理矩形（--X/--Y/--W/--H，物理像素，原点为虚拟桌面左上角）。

.PARAMETER Out
    输出 PNG 路径（相对仓库根目录或绝对路径）。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\spikes\tools\screenshot.ps1 -Out .verify\shots\full.png
#>
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$X = [int]::MinValue,
    [int]$Y = [int]::MinValue,
    [int]$W = 0,
    [int]$H = 0
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not [System.IO.Path]::IsPathRooted($Out)) { $Out = Join-Path $RepoRoot $Out }
$OutDir = Split-Path -Parent $Out
if ($OutDir) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class DpiAwareScreen {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int width, int height);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, int rop);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr hdc);
    public const int SRCCOPY = 0x00CC0020;
    public const int CAPTUREBLT = 0x40000000;
    public static IntPtr Enable() {
        // -4 = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2；失败则退回 system aware
        if (SetProcessDpiAwarenessContext(new IntPtr(-4))) return new IntPtr(-4);
        return IntPtr.Zero;
    }
}
"@

# 必须放在任何 System.Drawing/屏幕 API 调用之前
[void][DpiAwareScreen]::Enable()

Add-Type -AssemblyName System.Drawing

$vx = [DpiAwareScreen]::GetSystemMetrics(76)  # SM_XVIRTUALSCREEN
$vy = [DpiAwareScreen]::GetSystemMetrics(77)  # SM_YVIRTUALSCREEN
$vw = [DpiAwareScreen]::GetSystemMetrics(78)  # SM_CXVIRTUALSCREEN
$vh = [DpiAwareScreen]::GetSystemMetrics(79)  # SM_CYVIRTUALSCREEN

if ($X -ne [int]::MinValue) { $vx = $X }
if ($Y -ne [int]::MinValue) { $vy = $Y }
if ($W -gt 0) { $vw = $W }
if ($H -gt 0) { $vh = $H }

$screenDc = [DpiAwareScreen]::GetDC([IntPtr]::Zero)
$memDc = [DpiAwareScreen]::CreateCompatibleDC($screenDc)
$bitmap = [DpiAwareScreen]::CreateCompatibleBitmap($screenDc, $vw, $vh)
$old = [DpiAwareScreen]::SelectObject($memDc, $bitmap)
try {
    $ok = [DpiAwareScreen]::BitBlt($memDc, 0, 0, $vw, $vh, $screenDc, $vx, $vy, ([DpiAwareScreen]::SRCCOPY -bor [DpiAwareScreen]::CAPTUREBLT))
    if (-not $ok) { throw "BitBlt 失败" }
    $image = [System.Drawing.Image]::FromHbitmap($bitmap)
    try {
        $image.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $image.Dispose()
    }
    Write-Host ("[screenshot] {0}  {1}x{2} @ ({3},{4})  虚拟桌面物理像素" -f $Out, $vw, $vh, $vx, $vy)
} finally {
    [void][DpiAwareScreen]::SelectObject($memDc, $old)
    [void][DpiAwareScreen]::DeleteObject($bitmap)
    [void][DpiAwareScreen]::DeleteDC($memDc)
    [void][DpiAwareScreen]::ReleaseDC([IntPtr]::Zero, $screenDc)
}
