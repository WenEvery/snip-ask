#requires -Version 5.1
<#
.SYNOPSIS
    S-01 混合 DPI 多屏坐标标定工具（独立 Win32，不依赖产品代码）。

.DESCRIPTION
    做三件事，全部是**真机物理像素**测量，不 mock：

      1) 显示器原始数据：EnumDisplayMonitors / GetMonitorInfoW / GetDpiForMonitor /
         GetSystemMetrics(SM_*VIRTUALSCREEN)。进程先切 PerMonitorV2（-4），否则拿到的是
         被 DPI 虚拟化的逻辑坐标（已踩坑）。
      2) 十字线像素标定：创建一个覆盖**全虚拟桌面**的无边框置顶窗口（SetWindowPos 物理坐标，
         允许负坐标），在窗口内按物理坐标画十字线；用 BitBlt 抓全虚拟桌面 PNG，程序化检测
         每个十字中心的像素位置，与理论物理坐标比较，输出最大/平均误差（≤2px 判定）。
      3) 跨屏拖选往返换算：物理坐标 → overlay 窗口 client CSS → 回算物理坐标（含反向拖拽、
         负坐标、跨屏），输出误差；并对照「错误地用主屏 dpr=1.5 当全局 dpr」时的误差量级。

.PARAMETER OutDir
    输出目录（相对仓库根目录或绝对路径）。默认 .verify/spikes/S-01/。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\spikes\s01\s01_dpi_calibration.ps1
#>
param(
    [string]$OutDir = ".verify/spikes/S-01"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $RepoRoot $OutDir }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"

# ───────────────────────────── Win32 / GDI / User32 ─────────────────────────────
# 说明：crate 有 #![forbid(unsafe_code)]，Win32 调用只出现在 spike 脚本里（本文件即唯一入口）。
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Diagnostics;

public class S01MonitorData {
    public long HMONITOR;
    public int X; public int Y; public int W; public int H;          // MONITORINFOEX.rcMonitor
    public int WorkX; public int WorkY; public int WorkW; public int WorkH;
    public bool Primary;
    public uint DpiX; public uint DpiY;                              // GetDpiForMonitor(MDT_EFFECTIVE_DPI)
    public string Device = "";
    public int CbX; public int CbY; public int CbW; public int CbH;  // EnumDisplayMonitors 回调 rect
    public bool MatchesCallback;
}

public static class S01Native {
    public const int SM_XVIRTUALSCREEN  = 76;
    public const int SM_YVIRTUALSCREEN  = 77;
    public const int SM_CXVIRTUALSCREEN = 78;
    public const int SM_CYVIRTUALSCREEN = 79;
    public const int SM_CMONITORS       = 80;

    public const int WM_PAINT      = 0x000F;
    public const int WM_ERASEBKGND = 0x0014;
    public const int WM_DESTROY    = 0x0002;
    public const int PM_REMOVE     = 0x0001;
    public const int SW_SHOWNOACTIVATE = 4;
    public const uint WS_POPUP = 0x80000000;
    public const int WS_EX_TOPMOST    = 0x00000008;
    public const int WS_EX_TOOLWINDOW = 0x00000080;
    public const int WS_EX_NOACTIVATE = 0x08000000;
    public const uint SWP_NOACTIVATE  = 0x0010;
    public const uint SWP_SHOWWINDOW  = 0x0040;
    public const uint SWP_NOMOVE      = 0x0002;
    public const uint SWP_NOSIZE      = 0x0001;
    public const int SRCCOPY   = 0x00CC0020;
    public const int CAPTUREBLT = 0x40000000;

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct MONITORINFOEX {
        public int cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct PAINTSTRUCT {
        public IntPtr hdc; public int fErase; public RECT rcPaint;
        public int fRestore; public int fIncUpdate;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 32)] public byte[] rgbReserved;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct WNDCLASSEX {
        public uint cbSize; public uint style; public WndProcDelegate lpfnWndProc;
        public int cbClsExtra; public int cbWndExtra;
        public IntPtr hInstance; public IntPtr hIcon; public IntPtr hCursor; public IntPtr hbrBackground;
        public string lpszMenuName; public string lpszClassName; public IntPtr hIconSm;
    }
    [StructLayout(LayoutKind.Sequential)] public struct MSG {
        public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam;
        public uint time; public POINT pt;
    }

    public delegate bool MonitorEnumProc(IntPtr hMonitor, IntPtr hdcMonitor, ref RECT lprcMonitor, IntPtr dwData);
    public delegate IntPtr WndProcDelegate(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);
    [DllImport("user32.dll")] public static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr lprcClip, MonitorEnumProc lpfnEnum, IntPtr dwData);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool GetMonitorInfoW(IntPtr hMonitor, ref MONITORINFOEX lpmi);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr hmonitor, int dpiType, out uint dpiX, out uint dpiY);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("kernel32.dll")] public static extern IntPtr GetModuleHandleW(string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr RegisterClassExW(ref WNDCLASSEX wc);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateWindowExW(int exStyle, string cls, string title, uint style,
        int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr hwnd, IntPtr rect, bool erase);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr insertAfter, int x, int y, int w, int h, uint flags);
    [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG msg, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessageW(ref MSG msg);
    [DllImport("user32.dll")] public static extern IntPtr DefWindowProcW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern IntPtr BeginPaint(IntPtr hwnd, out PAINTSTRUCT paint);
    [DllImport("user32.dll")] public static extern bool EndPaint(IntPtr hwnd, ref PAINTSTRUCT paint);
    [DllImport("user32.dll")] public static extern int FillRect(IntPtr hdc, ref RECT rect, IntPtr brush);

    [DllImport("gdi32.dll")] public static extern IntPtr CreateSolidBrush(int color);
    [DllImport("gdi32.dll")] public static extern IntPtr CreatePen(int style, int width, int color);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool MoveToEx(IntPtr hdc, int x, int y, IntPtr oldPoint);
    [DllImport("gdi32.dll")] public static extern bool LineTo(IntPtr hdc, int x, int y);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int width, int height);
    [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, int rop);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr hdc);

    static int ColorRef(int r, int g, int b) { return (r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16); }
    static IntPtr HWND_TOPMOST { get { return new IntPtr(-1); } }

    // ───────── 进程 DPI 感知 ─────────
    public static int EnablePerMonitorV2() {
        // -4 = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        return SetProcessDpiAwarenessContext(new IntPtr(-4)) ? 1 : 0;
    }

    // ───────── 显示器枚举（原始数据） ─────────
    public static List<S01MonitorData> EnumMonitors() {
        List<S01MonitorData> result = new List<S01MonitorData>();
        MonitorEnumProc proc = delegate(IntPtr hMon, IntPtr hdcMon, ref RECT rect, IntPtr data) {
            S01MonitorData item = new S01MonitorData();
            item.HMONITOR = hMon.ToInt64();
            item.CbX = rect.Left; item.CbY = rect.Top;
            item.CbW = rect.Right - rect.Left; item.CbH = rect.Bottom - rect.Top;
            // 回调 rect 是物理坐标；再取 MONITORINFOEX（cbSize 用真实结构大小，别抄 40）
            MONITORINFOEX mi = new MONITORINFOEX();
            mi.cbSize = Marshal.SizeOf(typeof(MONITORINFOEX));
            if (GetMonitorInfoW(hMon, ref mi)) {
                item.X = mi.rcMonitor.Left; item.Y = mi.rcMonitor.Top;
                item.W = mi.rcMonitor.Right - mi.rcMonitor.Left;
                item.H = mi.rcMonitor.Bottom - mi.rcMonitor.Top;
                item.WorkX = mi.rcWork.Left; item.WorkY = mi.rcWork.Top;
                item.WorkW = mi.rcWork.Right - mi.rcWork.Left;
                item.WorkH = mi.rcWork.Bottom - mi.rcWork.Top;
                item.Primary = (mi.dwFlags & 1) != 0;   // MONITORINFOF_PRIMARY
                item.Device = mi.szDevice == null ? "" : mi.szDevice;
            } else {
                item.X = item.CbX; item.Y = item.CbY; item.W = item.CbW; item.H = item.CbH;
            }
            item.MatchesCallback = (item.X == item.CbX && item.Y == item.CbY && item.W == item.CbW && item.H == item.CbH);
            uint dx = 0, dy = 0;
            int hr = GetDpiForMonitor(hMon, 0, out dx, out dy);   // 0 = MDT_EFFECTIVE_DPI
            if (hr == 0) { item.DpiX = dx; item.DpiY = dy; }
            result.Add(item);
            return true;
        };
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, proc, IntPtr.Zero);
        GC.KeepAlive(proc);
        return result;
    }

    // ───────── 标定窗口（覆盖全虚拟桌面，物理坐标，允许负坐标） ─────────
    static IntPtr _hwnd = IntPtr.Zero;
    static bool _classReady = false;
    static WndProcDelegate _wndProc = null;
    static List<int[]> _crosses = new List<int[]>();   // {物理x, 物理y, 臂长}

    static bool EnsureClass() {
        if (_classReady) return true;
        _wndProc = new WndProcDelegate(StaticWndProc);
        WNDCLASSEX wc = new WNDCLASSEX();
        wc.cbSize = (uint)Marshal.SizeOf(typeof(WNDCLASSEX));
        wc.lpfnWndProc = _wndProc;
        wc.hInstance = GetModuleHandleW(null);
        wc.lpszClassName = "S01CalibrationWindow";
        wc.hbrBackground = IntPtr.Zero;
        wc.lpszMenuName = null;
        if (RegisterClassExW(ref wc) == IntPtr.Zero) return false;
        _classReady = true;
        return true;
    }

    static void PaintCb(IntPtr hwnd) {
        PAINTSTRUCT ps;
        IntPtr hdc = BeginPaint(hwnd, out ps);
        try {
            RECT client;
            GetClientRect(hwnd, out client);
            IntPtr bg = CreateSolidBrush(ColorRef(2, 3, 4));   // 近黑，避免与桌面内容混淆
            try { FillRect(hdc, ref client, bg); } finally { DeleteObject(bg); }
            RECT wr;
            GetWindowRect(hwnd, out wr);
            int ox = wr.Left, oy = wr.Top;
            IntPtr pen = CreatePen(0, 3, ColorRef(255, 0, 255));  // 3px 品红十字线
            IntPtr old = SelectObject(hdc, pen);
            try {
                for (int i = 0; i < _crosses.Count; i++) {
                    int[] c = _crosses[i];
                    int cx = c[0] - ox;
                    int cy = c[1] - oy;
                    int arm = c[2];
                    MoveToEx(hdc, cx - arm, cy, IntPtr.Zero);
                    LineTo(hdc, cx + arm + 1, cy);
                    MoveToEx(hdc, cx, cy - arm, IntPtr.Zero);
                    LineTo(hdc, cx, cy + arm + 1);
                }
            } finally {
                SelectObject(hdc, old);
                DeleteObject(pen);
            }
        } finally {
            EndPaint(hwnd, ref ps);
        }
    }

    static IntPtr StaticWndProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam) {
        switch (msg) {
            case WM_PAINT: PaintCb(hWnd); return IntPtr.Zero;
            case WM_ERASEBKGND: return new IntPtr(1);
            default: return DefWindowProcW(hWnd, msg, wParam, lParam);
        }
    }

    public static void PumpFor(int ms) {
        Stopwatch sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < ms) {
            MSG msg;
            while (PeekMessageW(out msg, IntPtr.Zero, 0, 0, PM_REMOVE)) {
                TranslateMessage(ref msg);
                DispatchMessageW(ref msg);
            }
            Thread.Sleep(5);
        }
    }

    public static bool CreateCalibrationWindow(int x, int y, int w, int h, int[] crossesFlat, int settleMs) {
        if (!EnsureClass()) return false;
        _crosses = new List<int[]>();
        for (int i = 0; i + 2 < crossesFlat.Length; i += 3) {
            _crosses.Add(new int[] { crossesFlat[i], crossesFlat[i + 1], crossesFlat[i + 2] });
        }
        _hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            "S01CalibrationWindow", "S01 标定（全虚拟桌面）", WS_POPUP,
            x, y, w, h, IntPtr.Zero, IntPtr.Zero, GetModuleHandleW(null), IntPtr.Zero);
        if (_hwnd == IntPtr.Zero) return false;
        ShowWindow(_hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(_hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(_hwnd, IntPtr.Zero, true);
        UpdateWindow(_hwnd);
        PumpFor(settleMs);
        // DPI 切换 / 首次显示可能由系统移动窗口：重新断言物理几何，再刷新一次。
        SetWindowPos(_hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(_hwnd, IntPtr.Zero, true);
        UpdateWindow(_hwnd);
        PumpFor(300);
        return true;
    }

    public static bool ResetCalibrationWindowRect(int x, int y, int w, int h) {
        if (_hwnd == IntPtr.Zero) return false;
        bool ok = SetWindowPos(_hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        InvalidateRect(_hwnd, IntPtr.Zero, true);
        UpdateWindow(_hwnd);
        PumpFor(200);
        return ok;
    }

    public static int[] GetCalibrationWindowRect() {
        RECT r;
        if (_hwnd == IntPtr.Zero || !GetWindowRect(_hwnd, out r)) return new int[] { 0, 0, 0, 0 };
        return new int[] { r.Left, r.Top, r.Right, r.Bottom };
    }

    public static int[] GetCalibrationWindowClientRect() {
        RECT r;
        if (_hwnd == IntPtr.Zero || !GetClientRect(_hwnd, out r)) return new int[] { 0, 0, 0, 0 };
        return new int[] { r.Left, r.Top, r.Right, r.Bottom };
    }

    public static uint GetCalibrationWindowDpi() {
        return _hwnd == IntPtr.Zero ? 0 : GetDpiForWindow(_hwnd);
    }

    public static void DestroyCalibrationWindow() {
        if (_hwnd != IntPtr.Zero) {
            DestroyWindow(_hwnd);
            _hwnd = IntPtr.Zero;
            PumpFor(120);
        }
    }
}
"@

# DPI 感知必须早于任何 System.Drawing / 屏幕度量调用
$dpiAware = [S01Native]::EnablePerMonitorV2()

Add-Type -AssemblyName System.Drawing

# ───────────────────────────── 1) 显示器原始数据 ─────────────────────────────
$monitors = [S01Native]::EnumMonitors()
$smX = [S01Native]::GetSystemMetrics([S01Native]::SM_XVIRTUALSCREEN)
$smY = [S01Native]::GetSystemMetrics([S01Native]::SM_YVIRTUALSCREEN)
$smW = [S01Native]::GetSystemMetrics([S01Native]::SM_CXVIRTUALSCREEN)
$smH = [S01Native]::GetSystemMetrics([S01Native]::SM_CYVIRTUALSCREEN)
$smCount = [S01Native]::GetSystemMetrics([S01Native]::SM_CMONITORS)

$vdLeft = ($monitors | ForEach-Object { $_.X } | Measure-Object -Minimum).Minimum
$vdTop = ($monitors | ForEach-Object { $_.Y } | Measure-Object -Minimum).Minimum
$vdRight = ($monitors | ForEach-Object { $_.X + $_.W } | Measure-Object -Maximum).Maximum
$vdBottom = ($monitors | ForEach-Object { $_.Y + $_.H } | Measure-Object -Maximum).Maximum
$vdW = $vdRight - $vdLeft
$vdH = $vdBottom - $vdTop

$monitorPayload = [ordered]@{
    generatedAt      = (Get-Date -Format "o")
    script           = "scripts/spikes/s01/s01_dpi_calibration.ps1"
    processDpiAware  = if ($dpiAware -eq 1) { "PER_MONITOR_AWARE_V2(-4)" } else { "FAILED" }
    systemMetrics    = [ordered]@{
        SM_XVIRTUALSCREEN  = $smX
        SM_YVIRTUALSCREEN  = $smY
        SM_CXVIRTUALSCREEN = $smW
        SM_CYVIRTUALSCREEN = $smH
        SM_CMONITORS       = $smCount
    }
    unionFromMonitors = [ordered]@{ x = $vdLeft; y = $vdTop; width = $vdW; height = $vdH }
    monitors         = @($monitors | ForEach-Object {
            [ordered]@{
                hmonitor     = $_.HMONITOR
                device       = $_.Device
                primary      = $_.Primary
                rect         = [ordered]@{ x = $_.X; y = $_.Y; width = $_.W; height = $_.H }
                workArea     = [ordered]@{ x = $_.WorkX; y = $_.WorkY; width = $_.WorkW; height = $_.WorkH }
                callbackRect = [ordered]@{ x = $_.CbX; y = $_.CbY; width = $_.CbW; height = $_.CbH }
                callbackMatchesMonitorInfo = $_.MatchesCallback
                dpi          = [ordered]@{ effectiveX = $_.DpiX; effectiveY = $_.DpiY; scale = [math]::Round($_.DpiX / 96.0, 4) }
            }
        })
}
$monitorPayload | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $OutDir "s01_monitors.json") -Encoding utf8

Write-Host ""
Write-Host "== S-01 显示器原始数据 ==" -ForegroundColor Cyan
Write-Host ("进程 DPI 感知：{0}" -f $monitorPayload.processDpiAware)
Write-Host ("SM_*VIRTUALSCREEN：x={0} y={1} w={2} h={3}  count={4}" -f $smX, $smY, $smW, $smH, $smCount)
Write-Host ("显示器外接矩形（由 EnumDisplayMonitors 计算）：x={0} y={1} w={2} h={3}" -f $vdLeft, $vdTop, $vdW, $vdH)
$monitors | ForEach-Object {
    Write-Host ("  [{0}] {1}  rect=({2},{3}) {4}x{5}  dpi={6}  scale={7}  primary={8}  cbMatch={9}" -f `
            $_.HMONITOR, $_.Device, $_.X, $_.Y, $_.W, $_.H, $_.DpiX, [math]::Round($_.DpiX / 96.0, 2), $_.Primary, $_.MatchesCallback)
}

# ───────────────────────────── 2) 十字线像素标定 ─────────────────────────────
# 标定点（物理像素，虚拟桌面左上角可为负）。注意：**必须落在真实显示器覆盖区内**，
# 外接矩形 (-1080,-470,3640,1920) 的右上/右下有「死区」（主屏上方/下方没有屏），
# 标定点若落进死区，屏幕上根本不可见 → 会误判为丢失。死区单独作为对照项测量。
#  - 主屏/副屏各内部点（内缩 40px）
#  - 两屏交界 x=0 两侧（y 取 100，保证主屏一侧 y≥0 可见）
#  - 负坐标区域（副屏 only）与负 y 区域
#  - 各屏真实边缘内缩 2px（臂长收短到 2 避免出入屏幕/出图）
$arms = 12
$crossDefs = @(
    @{ name = "primary_TL40";    x = 40;    y = 40;    arm = $arms }
    @{ name = "primary_TR40";    x = 2520;  y = 40;    arm = $arms }
    @{ name = "primary_BR40";    x = 2520;  y = 1400;  arm = $arms }
    @{ name = "primary_center";  x = 1280;  y = 720;   arm = $arms }
    @{ name = "secondary_TL40";  x = -1040; y = -430;  arm = $arms }
    @{ name = "secondary_BL40";  x = -1040; y = 1410;  arm = $arms }
    @{ name = "secondary_BR40";  x = -40;   y = 1410;  arm = $arms }
    @{ name = "secondary_center"; x = -540; y = 490;   arm = $arms }
    @{ name = "boundary_secondary_side"; x = -6; y = 720;  arm = $arms }
    @{ name = "boundary_primary_side";   x = 6;  y = 1000; arm = $arms }
    @{ name = "negative_area";   x = -900;  y = 800;   arm = $arms }
    @{ name = "negative_y_area"; x = -540;  y = -300;  arm = $arms }
    @{ name = "edge_left_2px_secondary";   x = -1078; y = 700;  arm = 2 }
    @{ name = "edge_right_2px_primary";    x = 2557;  y = 700;  arm = 2 }
    @{ name = "edge_top_2px_secondary";    x = -540;  y = -468; arm = 2 }
    @{ name = "edge_top_2px_primary";      x = 1280;  y = 2;    arm = 2 }
    @{ name = "edge_bottom_2px_secondary"; x = -540;  y = 1447; arm = 2 }
    @{ name = "edge_bottom_2px_primary";   x = 1280;  y = 1437; arm = 2 }
)
# 外接矩形内、但没有显示器覆盖的「死区」：overlay 窗口几何上会覆盖，屏幕上不可见，
# 预期检测不到十字（用于说明「外接矩形 ≠ 屏幕可见区域」）。
$deadZoneDefs = @(
    @{ name = "dead_above_primary";      x = 2000;  y = -300; arm = $arms }
    @{ name = "dead_below_primary";      x = 2000;  y = 1448; arm = $arms }
)
$crossFlat = @()
foreach ($c in $crossDefs) { $crossFlat += @([int]$c.x, [int]$c.y, [int]$c.arm) }
foreach ($c in $deadZoneDefs) { $crossFlat += @([int]$c.x, [int]$c.y, [int]$c.arm) }

$created = [S01Native]::CreateCalibrationWindow($vdLeft, $vdTop, $vdW, $vdH, $crossFlat, 700)
if (-not $created) { throw "标定窗口创建失败" }
# 最终几何（DPI 切换后二次断言）
[void][S01Native]::ResetCalibrationWindowRect($vdLeft, $vdTop, $vdW, $vdH)
$winRect = [S01Native]::GetCalibrationWindowRect()
$winClient = [S01Native]::GetCalibrationWindowClientRect()
$winDpi = [S01Native]::GetCalibrationWindowDpi()
$winScale = if ($winDpi -gt 0) { $winDpi / 96.0 } else { 1.0 }

Write-Host ""
Write-Host "== 标定窗口几何（物理像素，GetWindowRect） ==" -ForegroundColor Cyan
Write-Host ("窗口：({0},{1}) {2}x{3}   dpi={4}  scale={5}" -f $winRect[0], $winRect[1], ($winRect[2] - $winRect[0]), ($winRect[3] - $winRect[1]), $winDpi, $winScale)
$winOk = ([math]::Abs($winRect[0] - $vdLeft) -le 2 -and [math]::Abs($winRect[1] - $vdTop) -le 2 -and `
        [math]::Abs(($winRect[2] - $winRect[0]) - $vdW) -le 2 -and [math]::Abs(($winRect[3] - $winRect[1]) - $vdH) -le 2)
Write-Host ("窗口矩形 == 虚拟桌面（≤2px）：{0}" -f $winOk)

# 全虚拟桌面截图（DPI 感知，物理像素）
$shotPath = Join-Path $OutDir "s01_calibration.png"
$screenDc = [S01Native]::GetDC([IntPtr]::Zero)
$memDc = [S01Native]::CreateCompatibleDC($screenDc)
$bitmap = [S01Native]::CreateCompatibleBitmap($screenDc, $vdW, $vdH)
$old = [S01Native]::SelectObject($memDc, $bitmap)
try {
    $ok = [S01Native]::BitBlt($memDc, 0, 0, $vdW, $vdH, $screenDc, $vdLeft, $vdTop,
        ([S01Native]::SRCCOPY -bor [S01Native]::CAPTUREBLT))
    if (-not $ok) { throw "BitBlt 失败" }
    $image = [System.Drawing.Image]::FromHbitmap($bitmap)
    try { $image.Save($shotPath, [System.Drawing.Imaging.ImageFormat]::Png) } finally { $image.Dispose() }
    Write-Host ("全虚拟桌面截图：{0}  {1}x{2} @ ({3},{4})" -f $shotPath, $vdW, $vdH, $vdLeft, $vdTop)
} finally {
    [void][S01Native]::SelectObject($memDc, $old)
    [void][S01Native]::DeleteObject($bitmap)
    [void][S01Native]::DeleteDC($memDc)
    [void][S01Native]::ReleaseDC([IntPtr]::Zero, $screenDc)
}
[void][S01Native]::DestroyCalibrationWindow()

# 程序化检测十字中心：在期望位置 ±24px 内找品红像素（R>=250,G<=8,B>=250），取质心
$bmp = [System.Drawing.Bitmap]::new($shotPath)
$imgW = $bmp.Width
$imgH = $bmp.Height
$detections = @()
$errs = @()
foreach ($c in $crossDefs) {
        $ex = [int]$c.x - $vdLeft
        $ey = [int]$c.y - $vdTop
        $sx = [math]::Max(0, $ex - 24); $ex2 = [math]::Min($imgW - 1, $ex + 24)
        $sy = [math]::Max(0, $ey - 24); $ey2 = [math]::Min($imgH - 1, $ey + 24)
        $sumX = 0.0; $sumY = 0.0; $n = 0
        for ($py = $sy; $py -le $ey2; $py++) {
            for ($px = $sx; $px -le $ex2; $px++) {
                $col = $bmp.GetPixel($px, $py)
                if ($col.R -ge 250 -and $col.G -le 8 -and $col.B -ge 250) {
                    $sumX += $px; $sumY += $py; $n += 1
                }
            }
        }
        $det = [ordered]@{
            name       = $c.name
            expected   = [ordered]@{ x = [int]$c.x; y = [int]$c.y }
            pixelsHit  = $n
            detected   = $null
            error      = $null
        }
        if ($n -gt 0) {
            $cx = $sumX / $n + $vdLeft
            $cy = $sumY / $n + $vdTop
            $dx = $cx - $c.x
            $dy = $cy - $c.y
            $det.detected = [ordered]@{ x = [math]::Round($cx, 2); y = [math]::Round($cy, 2) }
            $det.error = [ordered]@{ dx = [math]::Round($dx, 2); dy = [math]::Round($dy, 2); dist = [math]::Round([math]::Sqrt($dx * $dx + $dy * $dy), 2) }
            $errs += [math]::Sqrt($dx * $dx + $dy * $dy)
        } else {
            $errs += 9999.0
        }
        $detections += $det
    }

$maxErr = ($errs | Measure-Object -Maximum).Maximum
$meanErr = [math]::Round((($errs | Measure-Object -Average).Average), 3)
$detectedCount = @($detections | Where-Object { $_.pixelsHit -gt 0 }).Count
$within2 = $maxErr -le 2.0

# 死区对照：预期「中心不可见」。注意 BitBlt 对无显示器区域的内容未定义，且十字的臂可能伸进
# 相邻可见屏幕（如主屏下边缘外 8px 处的死区，臂的上半截在主屏上可见），因此只在中心 ±4px
# 的窄窗口内断言「中心像素不可见」。
$deadZoneResults = @()
foreach ($c in $deadZoneDefs) {
    $ex = [int]$c.x - $vdLeft
    $ey = [int]$c.y - $vdTop
    $sx = [math]::Max(0, $ex - 4); $ex2 = [math]::Min($imgW - 1, $ex + 4)
    $sy = [math]::Max(0, $ey - 4); $ey2 = [math]::Min($imgH - 1, $ey + 4)
    $n = 0
    for ($py = $sy; $py -le $ey2; $py++) {
        for ($px = $sx; $px -le $ex2; $px++) {
            $col = $bmp.GetPixel($px, $py)
            if ($col.R -ge 250 -and $col.G -le 8 -and $col.B -ge 250) { $n += 1 }
        }
    }
    $covered = $false
    foreach ($m in $monitors) {
        if ([int]$c.x -ge $m.X -and [int]$c.x -lt $m.X + $m.W -and [int]$c.y -ge $m.Y -and [int]$c.y -lt $m.Y + $m.H) { $covered = $true }
    }
    $deadZoneResults += [ordered]@{
        name = $c.name
        phys = @([int]$c.x, [int]$c.y)
        coveredByMonitor = $covered
        expectedInvisible = -not $covered
        magentaPixelsCenter = $n
        verifiedInvisible = ($n -eq 0)
    }
}
$bmp.Dispose()
Write-Host ""
Write-Host "== 外接矩形死区对照（无显示器覆盖，预期不可见） ==" -ForegroundColor DarkYellow
$deadZoneResults | ForEach-Object {
    Write-Host ("  {0,-20} phys=({1},{2}) 显示器覆盖={3} 中心品红像素={4} 验证不可见={5}" -f `
            $_.name, $_.phys[0], $_.phys[1], $_.coveredByMonitor, $_.magentaPixelsCenter, $_.verifiedInvisible)
}

Write-Host ""
Write-Host "== 十字线像素标定（期望物理坐标 vs 截图检测质心） ==" -ForegroundColor Cyan
$detections | ForEach-Object {
    if ($_.pixelsHit -gt 0) {
        Write-Host ("  {0,-18} 期望=({1},{2}) 检测=({3},{4}) err=({5},{6}) dist={7}px  hit={8}" -f `
                $_.name, $_.expected.x, $_.expected.y, $_.detected.x, $_.detected.y, $_.error.dx, $_.error.dy, $_.error.dist, $_.pixelsHit)
    } else {
        Write-Host ("  {0,-18} 期望=({1},{2}) 未检测到十字线！" -f $_.name, $_.expected.x, $_.expected.y) -ForegroundColor Red
    }
}
Write-Host ("检测点：{0}/{1}  最大误差={2}px  平均误差={3}px  ≤2px={4}" -f $detectedCount, $crossDefs.Count, [math]::Round($maxErr, 3), $meanErr, $within2) -ForegroundColor $(if ($within2) { "Green" } else { "Red" })

# ───────────────────────────── 3) 跨屏拖选往返换算 ─────────────────────────────
# 模型：物理点 p → client CSS c = (p - origin)/dpr → 回算 p' = origin + c*dpr。
# dpr 取「标定窗口实际 DPI 缩放」（= WebView 的 devicePixelRatio 口径）。
$roundtripCases = @(
    @{ name = "cross_primary_to_secondary"; start = @(100, 100);   end = @(-900, 800) }
    @{ name = "cross_reverse";             start = @(-900, 800);  end = @(100, 100) }
    @{ name = "secondary_to_primary";      start = @(-540, 490);  end = @(1280, 720) }
    @{ name = "negative_area_drag";        start = @(-900, 800);  end = @(200, 200) }
    @{ name = "primary_corner_drag";       start = @(2400, 1300); end = @(-1000, -400) }
    @{ name = "single_pixel_step";         start = @(0, 0);       end = @(1, 1) }
)

function Convert-Point {
    param([int[]]$p, [double]$dpr, [int]$originX, [int]$originY)
    [ordered]@{
        css    = @([math]::Round(($p[0] - $originX) / $dpr, 4), [math]::Round(($p[1] - $originY) / $dpr, 4))
        back   = @([math]::Round($originX + (($p[0] - $originX) / $dpr) * $dpr, 4), [math]::Round($originY + (($p[1] - $originY) / $dpr) * $dpr, 4))
        # 真实鼠标 client 坐标可能是整数 CSS 像素：模拟量化损失
        cssInt = @([math]::Round(($p[0] - $originX) / $dpr), [math]::Round(($p[1] - $originY) / $dpr))
    }
}
function Get-RoundtripError {
    param([int[]]$p, [double]$dpr, [int]$originX, [int]$originY)
    $css = @((($p[0] - $originX) / $dpr), (($p[1] - $originY) / $dpr))
    $backFloat = @(($originX + $css[0] * $dpr), ($originY + $css[1] * $dpr))
    $cssInt = @([math]::Round($css[0]), [math]::Round($css[1]))
    $backInt = @(($originX + $cssInt[0] * $dpr), ($originY + $cssInt[1] * $dpr))
    [ordered]@{
        errFloat = [math]::Round([math]::Sqrt([math]::Pow($backFloat[0] - $p[0], 2) + [math]::Pow($backFloat[1] - $p[1], 2)), 4)
        errInt   = [math]::Round([math]::Sqrt([math]::Pow($backInt[0] - $p[0], 2) + [math]::Pow($backInt[1] - $p[1], 2)), 4)
    }
}

$rtResults = @()
$rtErrsFloat = @()
$rtErrsInt = @()
foreach ($case in $roundtripCases) {
    $sx = $case.start[0]; $sy = $case.start[1]
    $ex = $case.end[0];   $ey = $case.end[1]
    $sCss = Convert-Point -p @($sx, $sy) -dpr $winScale -originX $vdLeft -originY $vdTop
    $eCss = Convert-Point -p @($ex, $ey) -dpr $winScale -originX $vdLeft -originY $vdTop
    $sErr = Get-RoundtripError -p @($sx, $sy) -dpr $winScale -originX $vdLeft -originY $vdTop
    $eErr = Get-RoundtripError -p @($ex, $ey) -dpr $winScale -originX $vdLeft -originY $vdTop
    $widthPhys = [math]::Abs($ex - $sx)
    $heightPhys = [math]::Abs($ey - $sy)
    $rtResults += [ordered]@{
        name        = $case.name
        startPhys   = @($sx, $sy)
        endPhys     = @($ex, $ey)
        startCss    = $sCss.css
        endCss      = $eCss.css
        startCssInt = $sCss.cssInt
        endCssInt   = $eCss.cssInt
        startBack   = $sCss.back
        endBack     = $eCss.back
        errFloat    = [ordered]@{ start = $sErr.errFloat; end = $eErr.errFloat }
        errInt      = [ordered]@{ start = $sErr.errInt; end = $eErr.errInt }
        widthPhys   = $widthPhys
        heightPhys  = $heightPhys
        crossesMonitors = ($sx -ge 0) -ne ($ex -ge 0) -or ($sy -ge 0) -ne ($ey -ge 0)
    }
    $rtErrsFloat += $sErr.errFloat; $rtErrsFloat += $eErr.errFloat
    $rtErrsInt += $sErr.errInt; $rtErrsInt += $eErr.errInt
}

# 错误做法对照：overlay 窗口实际 dpr=winScale（本机跨屏窗口 Windows 取 144/96=1.5），
# 若错误地把主屏 dpr=1.5 或「逐屏 dpr」当全局 dpr 下发，看误差量级。
$wrongDprResults = @()
foreach ($c in $crossDefs) {
    $px = [double]$c.x; $py = [double]$c.y
    # 前端 toPhys：origin + clientX * wrongDpr；而真实 clientX = (p-origin)/winScale
    $wrong = [ordered]@{
        name          = $c.name
        phys          = @($px, $py)
        errIfDpr15    = [math]::Round([math]::Sqrt([math]::Pow((($px - $vdLeft) / $winScale * 1.5 + $vdLeft) - $px, 2) + [math]::Pow((($py - $vdTop) / $winScale * 1.5 + $vdTop) - $py, 2)), 2)
        errIfDprPerMon = $null
    }
    $monDpr = 1.5
    foreach ($m in $monitors) {
        if ($px -ge $m.X -and $px -lt $m.X + $m.W -and $py -ge $m.Y -and $py -lt $m.Y + $m.H) { $monDpr = $m.DpiX / 96.0 }
    }
    $wrong.errIfDprPerMon = [math]::Round([math]::Sqrt([math]::Pow((($px - $vdLeft) / $winScale * $monDpr + $vdLeft) - $px, 2) + [math]::Pow((($py - $vdTop) / $winScale * $monDpr + $vdTop) - $py, 2)), 2)
    $wrongDprResults += $wrong
}

$rtMaxFloat = [math]::Round((($rtErrsFloat | Measure-Object -Maximum).Maximum), 4)
$rtMaxInt = [math]::Round((($rtErrsInt | Measure-Object -Maximum).Maximum), 4)
$rtWithin2 = ($rtMaxInt -le 2.0)

Write-Host ""
Write-Host ("== 跨屏拖选往返换算（overlay 窗口 dpr={0}，origin=({1},{2})） ==" -f $winScale, $vdLeft, $vdTop) -ForegroundColor Cyan
$rtResults | ForEach-Object {
    Write-Host ("  {0,-26} {1}->{2}  css={3}->{4}  floatErr=({5},{6}) intErr=({7},{8})" -f `
            $_.name, ($_.startPhys -join ","), ($_.endPhys -join ","), ($_.startCss -join ","), ($_.endCss -join ","), `
            $_.errFloat.start, $_.errFloat.end, $_.errInt.start, $_.errInt.end)
}
Write-Host ("往返最大误差：浮点 CSS={0}px  整数 CSS={1}px  ≤2px={2}" -f $rtMaxFloat, $rtMaxInt, $rtWithin2) -ForegroundColor $(if ($rtWithin2) { "Green" } else { "Red" })
Write-Host ""
Write-Host ("== 对照：错误地使用主屏 dpr=1.5 或逐屏 dpr（窗口实际 dpr={0}） ==" -f $winScale) -ForegroundColor Yellow
$wrongDprResults | ForEach-Object {
    Write-Host ("  {0,-18} phys=({1},{2})  err(dpr=1.5)={3}px  err(逐屏dpr)={4}px" -f $_.name, $_.phys[0], $_.phys[1], $_.errIfDpr15, $_.errIfDprPerMon)
}

$calibrationPayload = [ordered]@{
    generatedAt       = (Get-Date -Format "o")
    dpiAware          = $monitorPayload.processDpiAware
    virtualDesktop    = [ordered]@{ x = $vdLeft; y = $vdTop; width = $vdW; height = $vdH }
    window            = [ordered]@{
        targetRect = [ordered]@{ x = $vdLeft; y = $vdTop; width = $vdW; height = $vdH }
        actualRect = [ordered]@{ x = $winRect[0]; y = $winRect[1]; width = ($winRect[2] - $winRect[0]); height = ($winRect[3] - $winRect[1]) }
        clientRect = [ordered]@{ width = ($winClient[2] - $winClient[0]); height = ($winClient[3] - $winClient[1]) }
        dpi        = $winDpi
        scale      = [math]::Round($winScale, 4)
        coversVirtualDesktop = $winOk
        note       = "PerMonitorV2 窗口跨混合 DPI 桌面时，GetDpiForWindow 由覆盖率最大的显示器决定（本机 144=1.5x）；GDI 绘制仍按物理像素，十字线位置在所有屏上均无 DWM 位图缩放。"
    }
    crosses           = $detections
    deadZones         = $deadZoneResults
    detection         = [ordered]@{
        expectedPoints = $crossDefs.Count
        detectedPoints = $detectedCount
        maxErrorPx     = [math]::Round($maxErr, 3)
        meanErrorPx    = $meanErr
        within2Px      = $within2
        screenshot     = "s01_calibration.png"
        deadZonePointsExpectedInvisible = $deadZoneDefs.Count
        deadZonePointsVerifiedInvisible = @($deadZoneResults | Where-Object { $_.verifiedInvisible }).Count
    }
    roundtrip         = [ordered]@{
        dpr                 = [math]::Round($winScale, 4)
        origin              = @($vdLeft, $vdTop)
        cases               = $rtResults
        maxErrorFloatCssPx  = $rtMaxFloat
        maxErrorIntCssPx    = $rtMaxInt
        within2Px           = $rtWithin2
    }
    wrongDprContrast  = $wrongDprResults
}
$calibrationPayload | ConvertTo-Json -Depth 10 | Set-Content -Path (Join-Path $OutDir "s01_calibration.json") -Encoding utf8

# 机器可读摘要（供聚合脚本）
$summary = [ordered]@{
    result               = if ($winOk -and $within2 -and $rtWithin2 -and ($detectedCount -eq $crossDefs.Count)) { "DONE" } else { "FAILED" }
    virtualDesktopRect   = "$vdLeft,$vdTop ${vdW}x${vdH}"
    windowRect           = "$($winRect[0]),$($winRect[1]) $($winRect[2]-$winRect[0])x$($winRect[3]-$winRect[1])"
    windowDpi            = $winDpi
    windowScale          = $winScale
    crossMaxErrorPx      = [math]::Round($maxErr, 3)
    crossMeanErrorPx     = $meanErr
    crossWithin2Px       = $within2
    roundtripMaxIntCssPx = $rtMaxInt
    roundtripWithin2Px   = $rtWithin2
    timestamp            = $Stamp
}
$summary | ConvertTo-Json -Depth 4 | Set-Content -Path (Join-Path $OutDir "s01_calibration_summary.json") -Encoding utf8

Write-Host ""
Write-Host ("== 结论：{0} ==" -f $summary.result) -ForegroundColor $(if ($summary.result -eq "DONE") { "Green" } else { "Red" })
Write-Host ("输出目录：{0}" -f $OutDir)
Write-Host ("复跑：powershell -ExecutionPolicy Bypass -File scripts\spikes\s01\s01_dpi_calibration.ps1 -OutDir .verify/spikes/S-01")
