// S-09 spike 采集器（仅技术验证，不进入产品代码）。
//
// 目的：在同一台机器上对比三条捕获路径对「普通窗口」与「受保护窗口」的行为：
//   - BitBlt：GetDC(NULL) + CreateDIBSection + BitBlt(SRCCOPY|CAPTUREBLT)
//   - PrintWindow(PW_RENDERFULLCONTENT)：窗口自绘路径（对照组）
//   - WGC：Windows.Graphics.Capture（GraphicsCaptureItem via IGraphicsCaptureItemInterop +
//          Direct3D11CaptureFramePool + SoftwareBitmap 回读）
// 受保护窗口用 SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE=0x11) 构造，等价于
// Netflix/DRM/受保护内容对「截屏 API」的屏蔽方式（不做任何破解）。
//
// 输出：<out>/results.json + <out>/shots/*.png。
// 黑屏判定口径与产品 Rust 侧一致：luma<16 的像素占比 >= 99% 且均值 <16 且标准差 <6。
//
// 编译/运行见同目录 wgc_vs_bitblt.ps1。

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Windows.Forms;
using Windows.Graphics.Capture;
using Windows.Graphics.DirectX;
using Windows.Graphics.DirectX.Direct3D11;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace S09Capture;

// ───────────────────────────── Win32 / D3D11 / WinRT 激活 ─────────────────────────────

internal static class Native
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct MONITORINFO
    {
        public int cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public uint dwFlags;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct BITMAPINFOHEADER
    {
        public uint biSize;
        public int biWidth;
        public int biHeight;
        public ushort biPlanes;
        public ushort biBitCount;
        public uint biCompression;
        public uint biSizeImage;
        public int biXPelsPerMeter;
        public int biYPelsPerMeter;
        public uint biClrUsed;
        public uint biClrImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct BITMAPINFO
    {
        public BITMAPINFOHEADER bmiHeader;
        public uint bmiColors;
    }

    public const int SM_XVIRTUALSCREEN = 76;
    public const int SM_YVIRTUALSCREEN = 77;
    public const int SM_CXVIRTUALSCREEN = 78;
    public const int SM_CYVIRTUALSCREEN = 79;
    public const int MONITOR_DEFAULTTOPRIMARY = 1;
    public const uint WDA_NONE = 0x0;
    public const uint WDA_MONITOR = 0x1;
    public const uint WDA_EXCLUDEFROMCAPTURE = 0x11;
    public const int SRCCOPY = 0x00CC0020;
    public const int CAPTUREBLT = 0x40000000;
    public const uint SWP_NOSIZE = 0x0001;
    public const uint SWP_NOMOVE = 0x0002;
    public const uint SWP_NOACTIVATE = 0x0010;
    public const uint SWP_SHOWWINDOW = 0x0040;
    public static readonly IntPtr HWND_TOPMOST = new(-1);
    public const uint PW_RENDERFULLCONTENT = 0x00000002;

    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetWindowDisplayAffinity(IntPtr hWnd, uint affinity);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr insertAfter, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT pt, uint flags);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfoW(IntPtr hMonitor, ref MONITORINFO info);

    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr hdc);
    [DllImport("gdi32.dll")]
    public static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, int rop);
    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateDIBSection(IntPtr hdc, ref BITMAPINFO pbmi, uint usage, out IntPtr bits, IntPtr section, uint offset);

    [DllImport("d3d11.dll", EntryPoint = "D3D11CreateDevice", ExactSpelling = true, PreserveSig = true)]
    public static extern int D3D11CreateDevice(IntPtr adapter, int driverType, IntPtr software, uint flags,
        IntPtr featureLevels, uint featureLevelCount, uint sdkVersion, out IntPtr device, out int featureLevel, out IntPtr context);

    [DllImport("d3d11.dll", EntryPoint = "CreateDirect3D11DeviceFromDXGIDevice", ExactSpelling = true, PreserveSig = true)]
    public static extern int CreateDirect3D11DeviceFromDXGIDevice(IntPtr dxgiDevice, out IntPtr graphicsDevice);

    [DllImport("combase.dll")]
    public static extern int WindowsCreateString(IntPtr source, uint length, out IntPtr hstr);
    [DllImport("combase.dll")]
    public static extern int WindowsDeleteString(IntPtr hstr);
    [DllImport("combase.dll")]
    public static extern int RoGetActivationFactory(IntPtr hstr, ref Guid iid, out IntPtr factory);
}

/// <summary>
/// windows.graphics.capture.interop.h 的 IGraphicsCaptureItemInterop。
/// 注意：该接口直接派生自 IUnknown（不是 IInspectable），CreateForWindow 位于 vtable slot 3。
/// </summary>
[ComImport, Guid("3628E81B-3CAC-4C60-B7F4-23CE0E0C3356"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IGraphicsCaptureItemInterop
{
    [PreserveSig] int CreateForWindow(IntPtr window, ref Guid iid, out IntPtr result);
    [PreserveSig] int CreateForMonitor(IntPtr monitor, ref Guid iid, out IntPtr result);
}

// ───────────────────────────── 数据模型 ─────────────────────────────

internal sealed class RectInfo
{
    public int X { get; set; }
    public int Y { get; set; }
    public int Width { get; set; }
    public int Height { get; set; }

    public static RectInfo From(int x, int y, int w, int h) => new() { X = x, Y = y, Width = w, Height = h };
    public override string ToString() => $"({X},{Y}) {Width}x{Height}";
}

internal sealed class Frame
{
    public int Width;
    public int Height;
    public byte[] Bgra = Array.Empty<byte>();
}

internal sealed class FrameStats
{
    public int Width { get; set; }
    public int Height { get; set; }
    public long Pixels { get; set; }
    public double MeanLuma { get; set; }
    public double StdDev { get; set; }
    public double BlackRatio { get; set; }
    public double BrightRatio { get; set; }
    public double AlphaZeroRatio { get; set; }
    public int UniqueLumaBuckets { get; set; }
    public string Sha256 { get; set; } = "";
    public bool Black { get; set; }
}

internal sealed class CaptureSample
{
    public int Rep { get; set; }
    public string Path { get; set; } = "";
    public string Backend { get; set; } = "";
    public string Method { get; set; } = "";
    public bool Ok { get; set; }
    public string? Error { get; set; }
    public double ElapsedMs { get; set; }
    public RectInfo? SourceRegion { get; set; }
    public string? Png { get; set; }
    public FrameStats? Frame { get; set; }
    public FrameStats? Region { get; set; }
    public string? Note { get; set; }
}

internal sealed class ScenarioResult
{
    public string Name { get; set; } = "";
    public string Label { get; set; } = "";
    public string Description { get; set; } = "";
    public uint Affinity { get; set; }
    public string AffinityName { get; set; } = "";
    public bool AffinityApplied { get; set; }
    public string? AffinityError { get; set; }
    public bool Fullscreen { get; set; }
    public RectInfo? WindowRect { get; set; }
    public RectInfo? MonitorRect { get; set; }
    public List<CaptureSample> Samples { get; set; } = new();
}

internal sealed class PathAggregate
{
    public string Scenario { get; set; } = "";
    public string Path { get; set; } = "";
    public int Samples { get; set; }
    public int OkCount { get; set; }
    public int BlackCount { get; set; }
    public double BlackRatioMean { get; set; }
    public double BlackRatioMin { get; set; }
    public double BlackRatioMax { get; set; }
    public double MeanLumaMean { get; set; }
    public double StdDevMean { get; set; }
    public double ElapsedMsMean { get; set; }
    public List<string> Errors { get; set; } = new();
}

internal sealed class RunResult
{
    public string Tool { get; set; } = "s09-capture/1.0";
    public string GeneratedAt { get; set; } = "";
    public string OsDescription { get; set; } = "";
    public string OsVersion { get; set; } = "";
    public string DotnetVersion { get; set; } = "";
    public string ProcessDpiAwareness { get; set; } = "";
    public RectInfo? VirtualScreen { get; set; }
    public RectInfo? PrimaryMonitor { get; set; }
    public string D3dDeviceInfo { get; set; } = "";
    public Dictionary<string, double> Thresholds { get; set; } = new();
    public List<ScenarioResult> Scenarios { get; set; } = new();
    public List<PathAggregate> Summary { get; set; } = new();
}

// ───────────────────────────── 采集器 ─────────────────────────────

internal sealed class Capturer
{
    public const double BlackLumaMax = 16.0;
    public const double BlackRatioMin = 0.99;
    public const double BlackStdDevMax = 6.0;

    private static IDirect3DDevice? _device;
    private static string _deviceInfo = "(not initialized)";
    private static readonly Guid IidDxgiDevice = new("54ec77fa-1377-44e6-8c32-88fd5f44c84c");
    private static readonly Guid IidActivationFactory = new("00000035-0000-0000-C000-000000000046");
    private static readonly Guid IidGraphicsCaptureItem = new("79C3F95B-31F7-4EC2-A464-632EF5D30760");

    public static string DeviceInfo => _deviceInfo;

    private static IntPtr _itemInteropPtr = IntPtr.Zero;
    private static IGraphicsCaptureItemInterop? _itemInterop;

    private static void EnsureItemInterop()
    {
        if (_itemInterop != null) return;
        const string cls = "Windows.Graphics.Capture.GraphicsCaptureItem";
        IntPtr src = Marshal.StringToHGlobalUni(cls);
        try
        {
            int hr = Native.WindowsCreateString(src, (uint)cls.Length, out IntPtr hstr);
            if (hr < 0) throw new InvalidOperationException($"WindowsCreateString 0x{hr:X8}");
            try
            {
                var iidActivation = IidActivationFactory;
            hr = Native.RoGetActivationFactory(hstr, ref iidActivation, out IntPtr factory);
                if (hr < 0) throw new InvalidOperationException($"RoGetActivationFactory 0x{hr:X8}");
                try
                {
                    var iidInterop = new Guid("3628E81B-3CAC-4C60-B7F4-23CE0E0C3356");
                    hr = Marshal.QueryInterface(factory, in iidInterop, out _itemInteropPtr);
                    if (hr < 0) throw new InvalidOperationException($"QI IGraphicsCaptureItemInterop 0x{hr:X8}");
                    _itemInterop = (IGraphicsCaptureItemInterop)Marshal.GetObjectForIUnknown(_itemInteropPtr);
                }
                finally
                {
                    Marshal.Release(factory);
                }
            }
            finally
            {
                Native.WindowsDeleteString(hstr);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(src);
        }
    }

    private static IDirect3DDevice EnsureDevice()
    {
        if (_device != null) return _device;
        int hr = Native.D3D11CreateDevice(IntPtr.Zero, 1 /*D3D_DRIVER_TYPE_HARDWARE*/, IntPtr.Zero,
            0x20 /*D3D11_CREATE_DEVICE_BGRA_SUPPORT*/, IntPtr.Zero, 0, 7 /*D3D11_SDK_VERSION*/,
            out IntPtr d3dDevice, out int featureLevel, out IntPtr context);
        if (hr < 0)
        {
            hr = Native.D3D11CreateDevice(IntPtr.Zero, 0 /*UNKNOWN: 允许 WARP*/, IntPtr.Zero, 0x20,
                IntPtr.Zero, 0, 7, out d3dDevice, out featureLevel, out context);
        }
        if (hr < 0) throw new InvalidOperationException($"D3D11CreateDevice 0x{hr:X8}");
        try
        {
            var iidDxgiDevice = IidDxgiDevice;
            hr = Marshal.QueryInterface(d3dDevice, in iidDxgiDevice, out IntPtr dxgi);
            if (hr < 0) throw new InvalidOperationException($"QI IDXGIDevice 0x{hr:X8}");
            try
            {
                hr = Native.CreateDirect3D11DeviceFromDXGIDevice(dxgi, out IntPtr inspectable);
                if (hr < 0) throw new InvalidOperationException($"CreateDirect3D11DeviceFromDXGIDevice 0x{hr:X8}");
                _deviceInfo = $"D3D11 featureLevel=0x{featureLevel:X} (45184=11_0)";
                _device = WinRT.MarshalInterface<IDirect3DDevice>.FromAbi(inspectable);
            }
            finally
            {
                Marshal.Release(dxgi);
            }
        }
        finally
        {
            Marshal.Release(d3dDevice);
            if (context != IntPtr.Zero) Marshal.Release(context);
        }
        return _device;
    }

    private static GraphicsCaptureItem CreateItem(bool monitor, IntPtr handle)
    {
        EnsureItemInterop();
        var iid = IidGraphicsCaptureItem;
        int hr = monitor
            ? _itemInterop!.CreateForMonitor(handle, ref iid, out IntPtr itemPtr)
            : _itemInterop!.CreateForWindow(handle, ref iid, out itemPtr);
        if (hr < 0) throw new InvalidOperationException($"CreateFor{(monitor ? "Monitor" : "Window")} 0x{hr:X8}");
        return WinRT.MarshalInspectable<GraphicsCaptureItem>.FromAbi(itemPtr);
    }

    /// <summary>WGC 抓一帧（窗口或显示器）。失败返回 error，不抛异常。</summary>
    public static (Frame? frame, double elapsedMs, string? error) CaptureWgc(bool monitor, IntPtr handle, int timeoutMs)
    {
        var sw = Stopwatch.StartNew();
        try
        {
            var device = EnsureDevice();
            var item = CreateItem(monitor, handle);
            var size = item.Size;
            var pool = Direct3D11CaptureFramePool.CreateFreeThreaded(
                device, DirectXPixelFormat.B8G8R8A8UIntNormalized, 1, size);
            try
            {
                using var session = pool.CreateCaptureSession(item);
                try { session.IsBorderRequired = false; } catch { /* 旧系统无此属性 */ }
                session.StartCapture();
                Direct3D11CaptureFrame? frame = null;
                int waited = 0;
                while (frame == null && waited < timeoutMs)
                {
                    Thread.Sleep(20);
                    waited += 20;
                    frame = pool.TryGetNextFrame();
                    Application.DoEvents();
                }
                if (frame == null)
                    return (null, sw.Elapsed.TotalMilliseconds, $"frameTimeout({timeoutMs}ms)");
                using (frame)
                {
                    int w = frame.ContentSize.Width;
                    int h = frame.ContentSize.Height;
                    if (w <= 0 || h <= 0)
                        return (null, sw.Elapsed.TotalMilliseconds, $"invalidContentSize({w}x{h})");
                    var sb = SoftwareBitmap.CreateCopyFromSurfaceAsync(frame.Surface, BitmapAlphaMode.Ignore)
                        .AsTask().GetAwaiter().GetResult();
                    try
                    {
                        if (sb.BitmapPixelFormat != BitmapPixelFormat.Bgra8)
                        {
                            var converted = SoftwareBitmap.Convert(sb, BitmapPixelFormat.Bgra8, BitmapAlphaMode.Ignore);
                            sb.Dispose();
                            sb = converted;
                        }
                        var bytes = new byte[w * h * 4];
                        var buffer = new Windows.Storage.Streams.Buffer((uint)bytes.Length) { Length = (uint)bytes.Length };
                        sb.CopyToBuffer(buffer);
                        using var reader = DataReader.FromBuffer(buffer);
                        reader.ReadBytes(bytes);
                        return (new Frame { Width = w, Height = h, Bgra = bytes }, sw.Elapsed.TotalMilliseconds, null);
                    }
                    finally
                    {
                        sb.Dispose();
                    }
                }
            }
            finally
            {
                pool.Dispose();
            }
        }
        catch (Exception error)
        {
            return (null, sw.Elapsed.TotalMilliseconds, error.GetType().Name + ": " + error.Message);
        }
    }

    /// <summary>BitBlt 屏幕区域（物理像素）。</summary>
    public static (Frame? frame, double elapsedMs, string? error) CaptureBitBlt(int x, int y, int w, int h)
    {
        var sw = Stopwatch.StartNew();
        IntPtr screenDc = IntPtr.Zero, memDc = IntPtr.Zero, dib = IntPtr.Zero, old = IntPtr.Zero;
        try
        {
            screenDc = Native.GetDC(IntPtr.Zero);
            if (screenDc == IntPtr.Zero) throw new InvalidOperationException("GetDC(NULL) == 0");
            memDc = Native.CreateCompatibleDC(screenDc);
            var bmi = new Native.BITMAPINFO();
            bmi.bmiHeader.biSize = (uint)Marshal.SizeOf<Native.BITMAPINFOHEADER>();
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // top-down
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = 0; // BI_RGB
            dib = Native.CreateDIBSection(screenDc, ref bmi, 0, out IntPtr bits, IntPtr.Zero, 0);
            if (dib == IntPtr.Zero) throw new InvalidOperationException("CreateDIBSection failed");
            old = Native.SelectObject(memDc, dib);
            bool ok = Native.BitBlt(memDc, 0, 0, w, h, screenDc, x, y,
                Native.SRCCOPY | Native.CAPTUREBLT);
            if (!ok) throw new InvalidOperationException("BitBlt failed");
            var bytes = new byte[(long)w * h * 4];
            Marshal.Copy(bits, bytes, 0, bytes.Length);
            return (new Frame { Width = w, Height = h, Bgra = bytes }, sw.Elapsed.TotalMilliseconds, null);
        }
        catch (Exception error)
        {
            return (null, sw.Elapsed.TotalMilliseconds, error.GetType().Name + ": " + error.Message);
        }
        finally
        {
            if (memDc != IntPtr.Zero && old != IntPtr.Zero) Native.SelectObject(memDc, old);
            if (dib != IntPtr.Zero) Native.DeleteObject(dib);
            if (memDc != IntPtr.Zero) Native.DeleteDC(memDc);
            if (screenDc != IntPtr.Zero) Native.ReleaseDC(IntPtr.Zero, screenDc);
        }
    }

    /// <summary>PrintWindow(PW_RENDERFULLCONTENT) 窗口自绘对照。</summary>
    public static (Frame? frame, double elapsedMs, string? error) CapturePrintWindow(IntPtr hwnd, int w, int h)
    {
        var sw = Stopwatch.StartNew();
        IntPtr screenDc = IntPtr.Zero, memDc = IntPtr.Zero, dib = IntPtr.Zero, old = IntPtr.Zero;
        try
        {
            screenDc = Native.GetDC(IntPtr.Zero);
            memDc = Native.CreateCompatibleDC(screenDc);
            var bmi = new Native.BITMAPINFO();
            bmi.bmiHeader.biSize = (uint)Marshal.SizeOf<Native.BITMAPINFOHEADER>();
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = 0;
            dib = Native.CreateDIBSection(screenDc, ref bmi, 0, out IntPtr bits, IntPtr.Zero, 0);
            if (dib == IntPtr.Zero) throw new InvalidOperationException("CreateDIBSection failed");
            old = Native.SelectObject(memDc, dib);
            bool ok = Native.PrintWindow(hwnd, memDc, Native.PW_RENDERFULLCONTENT);
            var bytes = new byte[(long)w * h * 4];
            Marshal.Copy(bits, bytes, 0, bytes.Length);
            return (new Frame { Width = w, Height = h, Bgra = bytes }, sw.Elapsed.TotalMilliseconds,
                ok ? null : "PrintWindow returned false");
        }
        catch (Exception error)
        {
            return (null, sw.Elapsed.TotalMilliseconds, error.GetType().Name + ": " + error.Message);
        }
        finally
        {
            if (memDc != IntPtr.Zero && old != IntPtr.Zero) Native.SelectObject(memDc, old);
            if (dib != IntPtr.Zero) Native.DeleteObject(dib);
            if (memDc != IntPtr.Zero) Native.DeleteDC(memDc);
            if (screenDc != IntPtr.Zero) Native.ReleaseDC(IntPtr.Zero, screenDc);
        }
    }

    /// <summary>黑屏/内容统计。crop 为帧内坐标；为 null 时只统计整帧。</summary>
    public static (FrameStats full, FrameStats? region) Stats(Frame frame, RectInfo? crop)
    {
        static FrameStats Compute(byte[] bgra, int width, int height, int x0, int y0, int w, int h)
        {
            var lumaBuckets = new bool[256];
            long pixels = (long)w * h;
            double sum = 0, sum2 = 0;
            long black = 0, bright = 0, alphaZero = 0;
            for (int row = y0; row < y0 + h; row++)
            {
                int index = (row * width + x0) * 4;
                for (int col = 0; col < w; col++, index += 4)
                {
                    int b = bgra[index], g = bgra[index + 1], r = bgra[index + 2], a = bgra[index + 3];
                    double luma = 0.114 * b + 0.587 * g + 0.299 * r;
                    sum += luma;
                    sum2 += luma * luma;
                    int bucket = (int)Math.Round(Math.Clamp(luma, 0, 255));
                    lumaBuckets[bucket] = true;
                    if (luma < BlackLumaMax) black++;
                    if (luma >= 128) bright++;
                    if (a == 0) alphaZero++;
                }
            }
            double mean = pixels == 0 ? 0 : sum / pixels;
            double variance = pixels == 0 ? 0 : Math.Max(0, sum2 / pixels - mean * mean);
            double std = Math.Sqrt(variance);
            int unique = lumaBuckets.Count(v => v);
            var sha = Convert.ToHexString(SHA256.HashData(new ReadOnlySpan<byte>(bgra))).ToLowerInvariant();
            var stats = new FrameStats
            {
                Width = w,
                Height = h,
                Pixels = pixels,
                MeanLuma = Math.Round(mean, 3),
                StdDev = Math.Round(std, 3),
                BlackRatio = pixels == 0 ? 0 : Math.Round((double)black / pixels, 6),
                BrightRatio = pixels == 0 ? 0 : Math.Round((double)bright / pixels, 6),
                AlphaZeroRatio = pixels == 0 ? 0 : Math.Round((double)alphaZero / pixels, 6),
                UniqueLumaBuckets = unique,
                Sha256 = sha,
            };
            stats.Black = stats.Pixels > 0
                && stats.BlackRatio >= BlackRatioMin
                && stats.MeanLuma < BlackLumaMax
                && stats.StdDev < BlackStdDevMax;
            return stats;
        }

        var full = Compute(frame.Bgra, frame.Width, frame.Height, 0, 0, frame.Width, frame.Height);
        if (crop == null) return (full, null);
        int cx = Math.Clamp(crop.X, 0, Math.Max(0, frame.Width - 1));
        int cy = Math.Clamp(crop.Y, 0, Math.Max(0, frame.Height - 1));
        int cw = Math.Clamp(crop.Width, 0, frame.Width - cx);
        int ch = Math.Clamp(crop.Height, 0, frame.Height - cy);
        if (cw <= 0 || ch <= 0) return (full, null);
        return (full, Compute(frame.Bgra, frame.Width, frame.Height, cx, cy, cw, ch));
    }

    public static void SavePng(Frame frame, string path)
    {
        using var bitmap = new Bitmap(frame.Width, frame.Height, PixelFormat.Format32bppArgb);
        var data = bitmap.LockBits(new Rectangle(0, 0, frame.Width, frame.Height),
            ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
        try
        {
            Marshal.Copy(frame.Bgra, 0, data.Scan0, frame.Bgra.Length);
        }
        finally
        {
            bitmap.UnlockBits(data);
        }
        bitmap.Save(path, ImageFormat.Png);
    }
}

// ───────────────────────────── 场景窗口 ─────────────────────────────

/// <summary>确定性高对比内容：四象限色块 + 大号文字，便于肉眼与统计双重判别。</summary>
internal sealed class PatternForm : Form
{
    private readonly string _label;
    private readonly string _scenario;
    private static readonly Font TitleFont = new("Consolas", 30f, FontStyle.Bold);
    private static readonly Font BodyFont = new("Consolas", 16f);

    public PatternForm(string scenario, string label)
    {
        _scenario = scenario;
        _label = label;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        ShowInTaskbar = false;
        TopMost = true;
        BackColor = Color.Black;
        DoubleBuffered = true;
        Text = "S09 " + scenario;
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        int w = ClientSize.Width;
        int h = ClientSize.Height;
        g.Clear(Color.FromArgb(18, 32, 64));
        // 四象限：白 / 红 / 绿 / 蓝，保证亮度、方差、色彩都高。
        g.FillRectangle(Brushes.White, 0, 0, w / 2, h / 2);
        g.FillRectangle(Brushes.Crimson, w / 2, 0, w - w / 2, h / 2);
        g.FillRectangle(Brushes.LimeGreen, 0, h / 2, w / 2, h - h / 2);
        g.FillRectangle(Brushes.RoyalBlue, w / 2, h / 2, w - w / 2, h - h / 2);
        // 黑底白字条 + 白底黑字条（文字区域占比很小，不影响「整体非黑」判定）。
        g.FillRectangle(Brushes.Black, 40, h / 2 - 90, w - 80, 60);
        g.DrawString("S09 · " + _scenario, TitleFont, Brushes.Yellow, 56, h / 2 - 96);
        g.FillRectangle(Brushes.Black, 40, h - 120, w - 80, 44);
        g.DrawString(_label, BodyFont, Brushes.White, 56, h - 118);
        base.OnPaint(e);
    }
}

// ───────────────────────────── 主流程 ─────────────────────────────

internal static class Program
{
    private sealed class Scenario
    {
        public string Name = "";
        public string Label = "";
        public string Description = "";
        public uint Affinity;
        public bool Fullscreen;
    }

    private static readonly Scenario[] Scenarios =
    {
        new()
        {
            Name = "normal",
            Label = "S09 NORMAL 普通窗口（无 WDA）",
            Description = "普通顶层窗口（无 SetWindowDisplayAffinity）：三条捕获路径都应拿到内容。",
            Affinity = Native.WDA_NONE,
        },
        new()
        {
            Name = "wda_exclude",
            Label = "S09 WDA_EXCLUDEFROMCAPTURE 受保护窗口",
            Description = "SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE=0x11)：等价于 Netflix/DRM 受保护内容对截屏 API 的屏蔽。",
            Affinity = Native.WDA_EXCLUDEFROMCAPTURE,
        },
        new()
        {
            Name = "wda_monitor",
            Label = "S09 WDA_MONITOR 受保护窗口（旧口径）",
            Description = "SetWindowDisplayAffinity(WDA_MONITOR=0x01)：Win7 时代的捕获屏蔽口径，用于对照。",
            Affinity = Native.WDA_MONITOR,
        },
        new()
        {
            Name = "fullscreen_normal",
            Label = "S09 FULLSCREEN 无边框全屏（等价全屏独占的近似）",
            Description = "覆盖主屏的无边框置顶窗口：近似全屏应用；真·全屏独占（D3D exclusive）无法用窗口模拟，见记录「遗留」。",
            Affinity = Native.WDA_NONE,
            Fullscreen = true,
        },
        new()
        {
            Name = "fullscreen_wda_monitor",
            Label = "S09 FULLSCREEN + WDA_MONITOR（受保护全屏≈DRM 黑屏）",
            Description = "覆盖主屏 + WDA_MONITOR(0x01)：截屏得到整块黑屏，等价「DRM/受保护全屏视频」的黑屏表现。",
            Affinity = Native.WDA_MONITOR,
            Fullscreen = true,
        },
        new()
        {
            Name = "fullscreen_wda_exclude",
            Label = "S09 FULLSCREEN + WDA_EXCLUDEFROMCAPTURE（窗口从截屏中消失）",
            Description = "覆盖主屏 + WDA_EXCLUDEFROMCAPTURE(0x11)：窗口在截屏中「消失」，露出背后的桌面内容，不是黑屏。",
            Affinity = Native.WDA_EXCLUDEFROMCAPTURE,
            Fullscreen = true,
        },
    };

    private static string _outDir = "";
    private static int _reps = 3;

    [STAThread]
    private static int Main(string[] args)
    {
        try
        {
            if (!TryParseArgs(args, out string? argError))
            {
                Console.Error.WriteLine(argError);
                return 64;
            }
            Directory.CreateDirectory(_outDir);
            Directory.CreateDirectory(Path.Combine(_outDir, "raw"));
            Directory.CreateDirectory(Path.Combine(_outDir, "shots"));

            bool winformsDpi = false;
            try { winformsDpi = Application.SetHighDpiMode(HighDpiMode.PerMonitorV2); } catch { }
            bool dpiOk = Native.SetProcessDpiAwarenessContext(new IntPtr(-4));
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);

            var result = new RunResult
            {
                GeneratedAt = DateTimeOffset.Now.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture),
                OsDescription = RuntimeInformation.OSDescription,
                OsVersion = Environment.OSVersion.Version.ToString(),
                DotnetVersion = Environment.Version.ToString(),
                ProcessDpiAwareness = winformsDpi || dpiOk
                    ? "per-monitor-v2 (WinForms SetHighDpiMode=" + winformsDpi + ", SetProcessDpiAwarenessContext(-4)=" + dpiOk + ")"
                    : "system/default（未能确认 per-monitor-v2）",
                Thresholds = new Dictionary<string, double>
                {
                    ["blackLumaMax"] = Capturer.BlackLumaMax,
                    ["blackRatioMin"] = Capturer.BlackRatioMin,
                    ["blackStdDevMax"] = Capturer.BlackStdDevMax,
                },
            };

            int vx = Native.GetSystemMetrics(Native.SM_XVIRTUALSCREEN);
            int vy = Native.GetSystemMetrics(Native.SM_YVIRTUALSCREEN);
            int vw = Native.GetSystemMetrics(Native.SM_CXVIRTUALSCREEN);
            int vh = Native.GetSystemMetrics(Native.SM_CYVIRTUALSCREEN);
            result.VirtualScreen = RectInfo.From(vx, vy, vw, vh);
            Console.WriteLine($"virtual screen ({vx},{vy}) {vw}x{vh}");

            IntPtr primaryMonitor = Native.MonitorFromPoint(new Native.POINT { X = 0, Y = 0 }, Native.MONITOR_DEFAULTTOPRIMARY);
            var monitorInfo = new Native.MONITORINFO { cbSize = Marshal.SizeOf<Native.MONITORINFO>() };
            if (!Native.GetMonitorInfoW(primaryMonitor, ref monitorInfo))
                throw new InvalidOperationException("GetMonitorInfoW(primary) failed");
            var monitorRect = RectInfo.From(monitorInfo.rcMonitor.Left, monitorInfo.rcMonitor.Top,
                monitorInfo.rcMonitor.Right - monitorInfo.rcMonitor.Left,
                monitorInfo.rcMonitor.Bottom - monitorInfo.rcMonitor.Top);
            result.PrimaryMonitor = monitorRect;
            Console.WriteLine($"primary monitor {monitorRect}");

            // 预初始化 D3D11 设备（失败也要记录，WGC 路径随后会报错但 BitBlt 仍可跑）。
            try
            {
                var (probe, _, probeError) = Capturer.CaptureWgc(true, primaryMonitor, 2000);
                if (probe != null)
                {
                    var (full, _) = Capturer.Stats(probe, null);
                    Console.WriteLine($"WGC self-check on primary monitor: {probe.Width}x{probe.Height} meanLuma={full.MeanLuma} black={full.Black}");
                    result.D3dDeviceInfo = Capturer.DeviceInfo + "; wgcSelfCheck=ok";
                }
                else
                {
                    result.D3dDeviceInfo = Capturer.DeviceInfo + "; wgcSelfCheck=error: " + probeError;
                    Console.WriteLine("WGC self-check error: " + probeError);
                }
            }
            catch (Exception error)
            {
                result.D3dDeviceInfo = "(wgc init failed) " + error.GetType().Name + ": " + error.Message;
                Console.WriteLine("WGC init error: " + error.Message);
            }

            foreach (var scenario in Scenarios)
            {
                var scenarioResult = RunScenario(scenario, primaryMonitor, monitorRect, vx, vy, vw, vh);
                result.Scenarios.Add(scenarioResult);
            }

            result.Summary = BuildSummary(result.Scenarios);
            WriteResults(result);
            PrintSummary(result);
            return 0;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine("FATAL: " + error);
            return 1;
        }
    }

    private static bool TryParseArgs(string[] args, out string? error)
    {
        error = null;
        _outDir = Path.Combine(".", ".verify", "spikes", "S-09");
        for (int i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--out":
                    if (++i >= args.Length) { error = "--out 缺少路径"; return false; }
                    _outDir = args[i];
                    break;
                case "--reps":
                    if (++i >= args.Length || !int.TryParse(args[i], out _reps) || _reps < 1) { error = "--reps 需为正整数"; return false; }
                    break;
                default:
                    if (args[i].StartsWith("--", StringComparison.Ordinal)) { error = "未知参数 " + args[i]; return false; }
                    break;
            }
        }
        return true;
    }

    private static ScenarioResult RunScenario(Scenario scenario, IntPtr primaryMonitor, RectInfo monitorRect,
        int vx, int vy, int vw, int vh)
    {
        Console.WriteLine();
        Console.WriteLine($"=== {scenario.Name} (affinity={scenario.AffinityNameOrDefault()}) ===");
        var result = new ScenarioResult
        {
            Name = scenario.Name,
            Label = scenario.Label,
            Description = scenario.Description,
            Affinity = scenario.Affinity,
            AffinityName = scenario.AffinityNameOrDefault(),
            Fullscreen = scenario.Fullscreen,
            MonitorRect = monitorRect,
        };

        using var form = new PatternForm(scenario.Name, scenario.Label);
        form.Show();
        Application.DoEvents();
        if (scenario.Fullscreen)
        {
            Native.SetWindowPos(form.Handle, Native.HWND_TOPMOST, monitorRect.X, monitorRect.Y,
                monitorRect.Width, monitorRect.Height, Native.SWP_SHOWWINDOW | Native.SWP_NOACTIVATE);
        }
        else
        {
            Native.SetWindowPos(form.Handle, Native.HWND_TOPMOST, 120, 120, 900, 600,
                Native.SWP_SHOWWINDOW | Native.SWP_NOACTIVATE);
        }

        if (scenario.Affinity != Native.WDA_NONE)
        {
            result.AffinityApplied = Native.SetWindowDisplayAffinity(form.Handle, scenario.Affinity);
            if (!result.AffinityApplied)
                result.AffinityError = "SetWindowDisplayAffinity 返回 false（最后错误 " + Marshal.GetLastWin32Error() + "）";
        }
        else
        {
            result.AffinityApplied = false;
        }

        form.Invalidate();
        for (int i = 0; i < 25; i++) { Application.DoEvents(); Thread.Sleep(16); }

        Native.GetWindowRect(form.Handle, out var wr);
        var windowRect = RectInfo.From(wr.Left, wr.Top, wr.Right - wr.Left, wr.Bottom - wr.Top);
        result.WindowRect = windowRect;
        Console.WriteLine($"window rect {windowRect}; affinityApplied={result.AffinityApplied}");

        for (int rep = 1; rep <= _reps; rep++)
        {
            // 1) BitBlt 窗口区域
            result.Samples.Add(RunSample(scenario.Name, scenario.Fullscreen, rep, "bitblt_region",
                "BitBlt(GetDC(NULL)+SRCCOPY|CAPTUREBLT) 窗口区域",
                () => Capturer.CaptureBitBlt(windowRect.X, windowRect.Y, windowRect.Width, windowRect.Height),
                windowRect, null));

            // 2) BitBlt 整个虚拟桌面（观察「整帧黑屏检测」在局部受保护场景下的盲区）
            result.Samples.Add(RunSample(scenario.Name, scenario.Fullscreen, rep, "bitblt_desktop",
                "BitBlt(GetDC(NULL)+SRCCOPY|CAPTUREBLT) 虚拟桌面",
                () => Capturer.CaptureBitBlt(vx, vy, vw, vh),
                RectInfo.From(vx, vy, vw, vh),
                RectInfo.From(windowRect.X - vx, windowRect.Y - vy, windowRect.Width, windowRect.Height)));

            // 3) PrintWindow(PW_RENDERFULLCONTENT) 对照
            result.Samples.Add(RunSample(scenario.Name, scenario.Fullscreen, rep, "printwindow",
                "PrintWindow(PW_RENDERFULLCONTENT)",
                () => Capturer.CapturePrintWindow(form.Handle, windowRect.Width, windowRect.Height),
                windowRect, null));

            // 4) WGC 窗口项
            result.Samples.Add(RunSample(scenario.Name, scenario.Fullscreen, rep, "wgc_window",
                "Windows.Graphics.Capture GraphicsCaptureItem(CreateForWindow)",
                () => Capturer.CaptureWgc(false, form.Handle, 5000), windowRect, null));

            // 5) WGC 主屏显示器项（观察受保护窗口在显示器捕获中是否被屏蔽）
            result.Samples.Add(RunSample(scenario.Name, scenario.Fullscreen, rep, "wgc_monitor",
                "Windows.Graphics.Capture GraphicsCaptureItem(CreateForMonitor 主屏)",
                () => Capturer.CaptureWgc(true, primaryMonitor, 5000), monitorRect,
                RectInfo.From(windowRect.X - monitorRect.X, windowRect.Y - monitorRect.Y, windowRect.Width, windowRect.Height)));

            Application.DoEvents();
        }

        form.Hide();
        Application.DoEvents();
        return result;
    }

    private static CaptureSample RunSample(string scenarioName, bool fullscreen, int rep, string path,
        string method, Func<(Frame? frame, double elapsedMs, string? error)> capture,
        RectInfo sourceRegion, RectInfo? crop)
    {
        var sample = new CaptureSample
        {
            Rep = rep,
            Path = path,
            Backend = path.StartsWith("wgc", StringComparison.Ordinal) ? "wgc"
                : path == "printwindow" ? "printwindow" : "bitblt",
            Method = method,
            SourceRegion = sourceRegion,
        };
        try
        {
            var (frame, elapsedMs, error) = capture();
            sample.ElapsedMs = Math.Round(elapsedMs, 3);
            if (frame != null)
            {
                sample.Ok = true;
                var (full, region) = Capturer.Stats(frame, crop);
                sample.Frame = full;
                sample.Region = region;
                string pngName = $"{scenarioName}_{path}_rep{rep}.png";
                Capturer.SavePng(frame, Path.Combine(_outDir, "shots", pngName));
                sample.Png = "shots/" + pngName;
                if (crop != null && fullscreen)
                    sample.Note = "frame=整帧（全屏场景下窗口区域≈整帧）；region=窗口区域";
                else if (crop != null)
                    sample.Note = "frame=整帧；region=窗口所在区域";
            }
            else
            {
                sample.Ok = false;
                sample.Error = error ?? "unknown";
            }
        }
        catch (Exception error)
        {
            sample.Ok = false;
            sample.Error = error.GetType().Name + ": " + error.Message;
        }
        string luma = sample.Frame == null ? "-" : $"{sample.Frame.MeanLuma:F1}";
        string black = sample.Frame == null ? "-" : (sample.Frame.Black ? "BLACK" : "content");
        Console.WriteLine($"  rep{rep} {path,-16} ok={sample.Ok,-5} {sample.ElapsedMs,8:F1}ms  meanLuma={luma,-7} {black,-8} {(sample.Error ?? "")}");
        return sample;
    }

    private static List<PathAggregate> BuildSummary(List<ScenarioResult> scenarios)
    {
        var summary = new List<PathAggregate>();
        foreach (var scenario in scenarios)
        {
            foreach (var group in scenario.Samples.GroupBy(sample => sample.Path))
            {
                var samples = group.ToList();
                var withFrame = samples.Where(sample => sample.Frame != null).Select(sample => sample.Frame!).ToList();
                var aggregates = new PathAggregate
                {
                    Scenario = scenario.Name,
                    Path = group.Key,
                    Samples = samples.Count,
                    OkCount = samples.Count(sample => sample.Ok),
                    BlackCount = withFrame.Count(frame => frame.Black),
                    Errors = samples.Where(sample => sample.Error != null).Select(sample => sample.Error!).Distinct().ToList(),
                };
                if (withFrame.Count > 0)
                {
                    aggregates.BlackRatioMean = Math.Round(withFrame.Average(frame => frame.BlackRatio), 6);
                    aggregates.BlackRatioMin = withFrame.Min(frame => frame.BlackRatio);
                    aggregates.BlackRatioMax = withFrame.Max(frame => frame.BlackRatio);
                    aggregates.MeanLumaMean = Math.Round(withFrame.Average(frame => frame.MeanLuma), 3);
                    aggregates.StdDevMean = Math.Round(withFrame.Average(frame => frame.StdDev), 3);
                }
                aggregates.ElapsedMsMean = Math.Round(samples.Average(sample => sample.ElapsedMs), 3);
                summary.Add(aggregates);
            }
        }
        return summary;
    }

    private static void WriteResults(RunResult result)
    {
        var options = new JsonSerializerOptions
        {
            WriteIndented = true,
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        };
        var path = Path.Combine(_outDir, "raw", "results.json");
        File.WriteAllText(path, JsonSerializer.Serialize(result, options));
        Console.WriteLine();
        Console.WriteLine("results -> " + path);
    }

    private static void PrintSummary(RunResult result)
    {
        Console.WriteLine();
        Console.WriteLine("scenario / path                     ok  black  blackRatio(mean)  meanLuma  elapsedMs");
        foreach (var row in result.Summary)
        {
            Console.WriteLine($"{row.Scenario,-20} {row.Path,-16} {row.OkCount}/{row.Samples,-3} {row.BlackCount,-5} {row.BlackRatioMean,10:F6}      {row.MeanLumaMean,8:F2} {row.ElapsedMsMean,8:F1}");
            foreach (var error in row.Errors) Console.WriteLine($"    error: {error}");
        }
    }

    private static string AffinityNameOrDefault(this Scenario scenario)
    {
        return scenario.Affinity switch
        {
            Native.WDA_NONE => "WDA_NONE(0x0)",
            Native.WDA_MONITOR => "WDA_MONITOR(0x1)",
            Native.WDA_EXCLUDEFROMCAPTURE => "WDA_EXCLUDEFROMCAPTURE(0x11)",
            _ => "0x" + scenario.Affinity.ToString("X"),
        };
    }
}
