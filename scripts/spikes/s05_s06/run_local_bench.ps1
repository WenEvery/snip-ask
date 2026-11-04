#!/usr/bin/env pwsh
# S-05 本地翻译正式时延测量运行器（安静窗口 + 线程调优 + 正式测量 + 输出长度敏感性）
#
# 前置：已通过 benchlock.ps1 以外的持有进程/本脚本自行声明占用（本脚本会在开始时检查 CPU 负载并等待安静窗口）。
# 用法：
#   pwsh scripts/spikes/s05_s06/run_local_bench.ps1                          # 默认线程候选 6/16，正式 10 次
#   pwsh scripts/spikes/s05_s06/run_local_bench.ps1 -ThreadList "6,8,16" -Runs 10
#   pwsh scripts/spikes/s05_s06/run_local_bench.ps1 -SkipSensitivity
#
# 退出后自动确认无 llama-server 残留；原始 JSON 输出到 .verify/spikes/S-05-S-06/raw/。
# 注意：线程候选用字符串传入（避免 powershell.exe -File 把 "6,16" 误绑定成整数 616）。
param(
    [string]$ThreadList = '6,16',
    [int]$Runs = 10,
    [int]$SensitivityTokens = 100,
    [int]$QuietLoadMax = 40,
    [int]$QuietWaitSec = 300,
    [switch]$SkipSensitivity
)
$ThreadCandidates = @($ThreadList -split ',' | ForEach-Object { [int]($_.Trim()) })
$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$Raw = Join-Path $Root '.verify\spikes\S-05-S-06\raw'
New-Item -ItemType Directory -Force -Path $Raw | Out-Null
$Stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$Log = Join-Path $Raw "formal_local_$Stamp.log"
function Log([string]$Message) {
    $line = "[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $Message
    Write-Output $line
    Add-Content -Path $Log -Value $line -Encoding utf8
}

Log "=== S-05 本地翻译正式测量开始 stamp=$Stamp ==="
Log ("exe 候选: " + (Get-ChildItem (Join-Path $env:USERPROFILE '.lmstudio\extensions\backends\llama.cpp-win-x86_64-avx2-2.28.2') -Filter 'llama-server.exe' -ErrorAction SilentlyContinue | Select-Object -First 1).FullName)

# 1) 等待安静窗口（避免并行编译/OCR 基准污染时延）
$waited = 0
while ($waited -lt $QuietWaitSec) {
    $load = (Get-CimInstance Win32_Processor).LoadPercentage
    Log "当前 CPU 负载 = $load%（安静阈值 ≤ $QuietLoadMax%）"
    if ($load -le $QuietLoadMax) { break }
    Start-Sleep -Seconds 10
    $waited += 10
}
$loadAtStart = (Get-CimInstance Win32_Processor).LoadPercentage
Log "开始测量时 CPU 负载 = $loadAtStart%"

# 2) 线程候选调优（每档 warmup 1 + 测量 2 次，只用于选线程，不计入正式数据）
$tuneResults = @()
foreach ($threads in $ThreadCandidates) {
    $out = Join-Path $Raw "tune_threads$threads`_$Stamp.json"
    Log "--- 线程调优 threads=$threads ---"
    & node (Join-Path $PSScriptRoot 'bench_local_translate.mjs') --threads $threads --runs 2 --warmup 1 --out $out 2>&1 |
        Where-Object { $_ -match '\[加载\]|\[客户端\]|\[服务端\]|\[判定\]|\[内存\]' } |
        ForEach-Object { Log $_ }
    if (Test-Path $out) {
        $j = Get-Content $out -Raw -Encoding utf8 | ConvertFrom-Json
        if ($j.summary.total_ms) {
            $tuneResults += [pscustomobject]@{
                threads   = $threads
                mean_ms   = $j.summary.total_ms.mean_ms
                total_p50 = $j.summary.total_ms.p50_ms
                decode_p50 = $j.summary.server_predicted_ms.p50_ms
                toks_per_s = $j.summary.tok_per_sec.p50_ms
                file      = (Split-Path $out -Leaf)
            }
        }
    }
    Start-Sleep -Seconds 2
}
if (-not $tuneResults.Count) {
    Log '错误：线程调优没有产生有效结果，退出。'
    exit 1
}
$best = $tuneResults | Sort-Object mean_ms | Select-Object -First 1
$table = $tuneResults | Format-Table -AutoSize | Out-String
Log "线程调优汇总:`n$table"
Log ("选用 threads={0}（mean {1}ms, decode_p50 {2}ms, {3} tok/s）" -f $best.threads, $best.mean_ms, $best.decode_p50, $best.toks_per_s)

# 3) 正式测量（安静窗口内）
$formalOut = Join-Path $Raw "local_translate_formal_$Stamp.json"
$loadBeforeFormal = (Get-CimInstance Win32_Processor).LoadPercentage
Log "--- 正式测量 threads=$($best.threads) runs=$Runs（开始负载 $loadBeforeFormal%）---"
& node (Join-Path $PSScriptRoot 'bench_local_translate.mjs') --threads $best.threads --runs $Runs --warmup 2 --out $formalOut 2>&1 |
    Where-Object { $_ -match '\[加载\]|\[客户端\]|\[服务端\]|\[判定\]|\[内存\]|原始数据' } |
    ForEach-Object { Log $_ }
$loadAfterFormal = (Get-CimInstance Win32_Processor).LoadPercentage
Log "正式测量结束负载 = $loadAfterFormal%"

# 4) 输出长度敏感性：限制 max_tokens=100（验证 decode 时长与 token 数近似线性）
$sensOut = $null
if (-not $SkipSensitivity) {
    $sensOut = Join-Path $Raw "local_translate_sens$SensitivityTokens`_$Stamp.json"
    Log "--- 输出长度敏感性 max_tokens=$SensitivityTokens（2 次）---"
    & node (Join-Path $PSScriptRoot 'bench_local_translate.mjs') --threads $best.threads --runs 2 --warmup 0 --max-tokens $SensitivityTokens --out $sensOut 2>&1 |
        Where-Object { $_ -match '\[加载\]|\[客户端\]|\[服务端\]|\[判定\]|原始数据' } |
        ForEach-Object { Log $_ }
}

# 5) 收尾：确认 llama-server 无残留
Start-Sleep -Seconds 2
$leftover = @(Get-Process llama-server -ErrorAction SilentlyContinue)
Log ("llama-server 残留进程数 = {0}" -f $leftover.Count)
if ($leftover.Count -gt 0) {
    $leftover | ForEach-Object { Log ("残留 pid={0}，强制结束" -f $_.Id); Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 1
    Log ("清理后残留进程数 = {0}" -f @(Get-Process llama-server -ErrorAction SilentlyContinue).Count)
}

# 6) 汇总
$summaryPath = Join-Path $Raw "formal_local_summary_$Stamp.json"
$summary = [ordered]@{
    stamp              = $Stamp
    started_at         = $Stamp
    quiet_load_threshold = $QuietLoadMax
    load_at_start      = $loadAtStart
    load_before_formal = $loadBeforeFormal
    load_after_formal  = $loadAfterFormal
    thread_tuning      = $tuneResults
    chosen_threads     = $best.threads
    formal_json        = (Split-Path $formalOut -Leaf)
    sensitivity_json   = if ($sensOut) { Split-Path $sensOut -Leaf } else { $null }
    llama_server_leftover = 0
    log                = (Split-Path $Log -Leaf)
}
$summary | ConvertTo-Json -Depth 6 | Out-File -FilePath $summaryPath -Encoding utf8
Log "汇总：$summaryPath"
Log '=== S-05 本地翻译正式测量结束 ==='
