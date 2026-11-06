<#
.SYNOPSIS
  S-13 helper: run llama-server inside a Windows Job Object with a hard process-memory limit,
  and report load time / resident memory / crash-or-exit behaviour.

.DESCRIPTION
  Purpose: simulate a low-memory machine (e.g. an 8 GB box already under pressure) for the local
  translation model. JOBOBJECT_EXTENDED_LIMIT_INFORMATION.ProcessMemoryLimit/JobMemoryLimit makes
  the kernel fail allocations beyond the limit; we then observe whether llama-server fails cleanly
  (exit code + stderr log) instead of taking the whole machine down.

  -LimitMB 0 means "no job limit" (baseline measurement, same binary/args as S-05).
  Output: one JSON line to stdout. All strings are ASCII to stay PS5.1-safe.
  The script kills the process tree in finally; caller also verifies no leftovers.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File s13_job.ps1 `
    -Exe "<llama-server.exe>" -Model "<model.gguf>" -LimitMB 6144 -Port 39283
#>
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$Model,
  [int]$LimitMB = 6144,
  [int]$Port = 39283,
  [int]$Threads = 16,
  [int]$Context = 4096,
  [int]$HealthTimeoutSec = 180,
  [switch]$NoMmap
)

$ErrorActionPreference = "Stop"
$result = [ordered]@{
  ok = $false
  limitMB = $LimitMB
  exe = $Exe
  model = (Split-Path -Leaf $Model)
  port = $Port
  noLimit = ($LimitMB -le 0)
  loaded = $false
  alive = $false
  exitCode = $null
  loadMs = $null
  workingSetMB = $null
  privateMB = $null
  peakWorkingSetMB = $null
  timedOut = $false
  jobAssigned = $false
  pid = $null
  stderrTail = @()
  error = $null
}

$outLog = Join-Path $env:TEMP ("s13_llama_out_" + [Guid]::NewGuid().ToString("N") + ".log")
$errLog = Join-Path $env:TEMP ("s13_llama_err_" + [Guid]::NewGuid().ToString("N") + ".log")

if (-not ("S13JobNative" -as [type])) {
  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class S13JobNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct IO_COUNTERS {
        public ulong ReadOperationCount; public ulong WriteOperationCount; public ulong OtherOperationCount;
        public ulong ReadTransferCount; public ulong WriteTransferCount; public ulong OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
        public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr CreateJobObject(IntPtr lpJobAttributes, string lpName);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool SetInformationJobObject(IntPtr hJob, int infoClass, IntPtr lpInfo, uint cbInfo);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool AssignProcessToJobObject(IntPtr hJob, IntPtr hProcess);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool CloseHandle(IntPtr h);
}
"@ -Language CSharp
}

$JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100
$JOB_OBJECT_LIMIT_JOB_MEMORY = 0x00000200
$JobObjectExtendedLimitInformation = 9

$job = [IntPtr]::Zero
if (-not $result.noLimit) {
  $job = [S13JobNative]::CreateJobObject([IntPtr]::Zero, $null)
  if ($job -eq [IntPtr]::Zero) {
    $result.error = "CreateJobObject failed: " + [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Output ($result | ConvertTo-Json -Compress -Depth 6)
    exit 3
  }
  $limitBytes = [uint64]$LimitMB * 1MB
  $info = New-Object S13JobNative+JOBOBJECT_EXTENDED_LIMIT_INFORMATION
  $info.BasicLimitInformation.LimitFlags = [uint32]($JOB_OBJECT_LIMIT_PROCESS_MEMORY -bor $JOB_OBJECT_LIMIT_JOB_MEMORY)
  $info.ProcessMemoryLimit = [UIntPtr]$limitBytes
  $info.JobMemoryLimit = [UIntPtr]$limitBytes
  $size = [System.Runtime.InteropServices.Marshal]::SizeOf([type][S13JobNative+JOBOBJECT_EXTENDED_LIMIT_INFORMATION])
  $infoPtr = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($size)
  [System.Runtime.InteropServices.Marshal]::StructureToPtr($info, $infoPtr, $false)
  $setOk = [S13JobNative]::SetInformationJobObject($job, $JobObjectExtendedLimitInformation, $infoPtr, [uint32]$size)
  [System.Runtime.InteropServices.Marshal]::FreeHGlobal($infoPtr)
  if (-not $setOk) {
    $result.error = "SetInformationJobObject failed: " + [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    [void][S13JobNative]::CloseHandle($job)
    Write-Output ($result | ConvertTo-Json -Compress -Depth 6)
    exit 3
  }
}

$proc = $null
try {
  $argList = @("-m", "`"$Model`"", "-c", "$Context", "-t", "$Threads", "--port", "$Port", "--host", "127.0.0.1", "--no-webui", "-ngl", "0")
  if ($NoMmap) { $argList += "--no-mmap" }
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $proc = Start-Process -FilePath $Exe -WorkingDirectory (Split-Path -Parent $Exe) -ArgumentList $argList -PassThru -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog
  $result.pid = $proc.Id
  # 缓存进程句柄（在作业分配前），并立即分配到 Job Object
  $handle = $proc.Handle
  $assigned = $true
  if (-not $result.noLimit) {
    $assigned = [S13JobNative]::AssignProcessToJobObject($job, $handle)
    $result.jobAssigned = [bool]$assigned
    if (-not $assigned) {
      $result.error = "AssignProcessToJobObject failed: " + [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    }
  } else {
    $result.jobAssigned = $false
  }

  $deadline = (Get-Date).AddSeconds($HealthTimeoutSec)
  $healthy = $false
  while ((Get-Date) -lt $deadline) {
    if ($proc.HasExited) { break }
    try {
      $resp = Invoke-WebRequest -Uri ("http://127.0.0.1:$Port/health") -UseBasicParsing -TimeoutSec 2
      if ($resp.StatusCode -eq 200) { $healthy = $true; break }
    } catch {
      # not ready yet
    }
    Start-Sleep -Milliseconds 250
  }
  $sw.Stop()
  $result.loadMs = [int64]$sw.Elapsed.TotalMilliseconds
  $result.alive = -not $proc.HasExited

  if ($healthy) {
    $result.loaded = $true
    Start-Sleep -Seconds 2
    $proc.Refresh()
    $result.workingSetMB = [math]::Round($proc.WorkingSet64 / 1MB, 1)
    $result.privateMB = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1)
    $result.peakWorkingSetMB = [math]::Round($proc.PeakWorkingSet64 / 1MB, 1)
  } else {
    if (-not $proc.HasExited) {
      $result.timedOut = $true
      $proc.WaitForExit(5000) | Out-Null
    }
    if ($proc.HasExited) {
      $result.exitCode = $proc.ExitCode
      $result.alive = $false
      $result.loadMs = $null
    }
    Start-Sleep -Milliseconds 500
    try { $proc.Refresh(); $result.privateMB = [math]::Round($proc.PrivateMemorySize64 / 1MB, 1) } catch { }
  }
  $result.ok = $true
} catch {
  $result.error = $_.Exception.Message
} finally {
  if ($proc) {
    try {
      if (-not $proc.HasExited) { $proc.Kill() }
      $proc.WaitForExit(8000) | Out-Null
    } catch { }
    try { $proc.Dispose() } catch { }
  }
  if ($job -ne [IntPtr]::Zero) { [void][S13JobNative]::CloseHandle($job) }
  Start-Sleep -Milliseconds 400
  # Final cleanup: none of our llama-server children may survive.
  try {
    $left = @(Get-Process llama-server -ErrorAction SilentlyContinue)
    if ($left.Count -gt 0) { $left | Stop-Process -Force -ErrorAction SilentlyContinue }
  } catch { }
  foreach ($log in @($errLog, $outLog)) {
    if (Test-Path $log) {
      $raw = @(Get-Content -LiteralPath $log -Encoding UTF8)
      $lines = @($raw | Where-Object { ([string]$_).Trim().Length -gt 0 } | ForEach-Object { [string]$_ })
      if ($log -eq $errLog) { $result.stderrTail = @($lines | Select-Object -Last 25) }
      else { $result.stdoutTail = @($lines | Select-Object -Last 10) }
      Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    }
  }
}

Write-Output ($result | ConvertTo-Json -Compress -Depth 6)
