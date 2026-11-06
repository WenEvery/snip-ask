<#
.SYNOPSIS
  S-17 预置冒烟：用真实 DeepSeek Key 验证 provider_test 的 `/models` 链路（脱敏输出）。

.DESCRIPTION
  Key 只从环境变量 `SNIPASK_BENCH_API_KEY` 读取，本脚本不接收参数传入 Key、不写死、不落盘。
  执行 `cargo test --test provider_probe_spike -- --ignored --nocapture`（真实冒烟用例），
  原始脱敏数据由测试写到 `.verify/spikes/S-17/deepseek-smoke-*.json`。
  防御性处理：无论测试输出含什么，都会把 Key 明文替换为 `***` 后再打印。

.EXAMPLE
  $env:SNIPASK_BENCH_API_KEY = '<真实 DeepSeek Key>'
  pwsh -NoProfile -File scripts/spikes/s17/smoke_deepseek.ps1
#>
param(
  [switch]$NoRun
)

$ErrorActionPreference = "Continue"
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
$OutputEncoding = [System.Text.Encoding]::UTF8

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

if (-not $env:SNIPASK_BENCH_API_KEY) {
  Write-Host "[s17] 缺少环境变量 SNIPASK_BENCH_API_KEY（请只在当前会话设置真实 Key，不要写进脚本/仓库）"
  exit 2
}
$apiKey = $env:SNIPASK_BENCH_API_KEY

if ($NoRun) {
  Write-Host "[s17] -NoRun：仅检查环境变量已设置（长度 $($apiKey.Length)，不打印内容）"
  exit 0
}

Push-Location $Repo
try {
  Write-Host "[s17] 运行真实 DeepSeek 冒烟（mock 用例由普通 cargo test 覆盖，不在此执行）…"
  $raw = & cargo test --manifest-path "src-tauri/Cargo.toml" --test provider_probe_spike -- --ignored --nocapture 2>&1
  $code = $LASTEXITCODE
  $text = (@($raw) | ForEach-Object { "$_" }) -join "`n"
  # 防御性脱敏：绝不把 Key 打到终端/日志。
  if ($apiKey.Length -gt 8) {
    $text = $text.Replace($apiKey, "***")
  }
  $interesting = $text -split "`n" | Where-Object {
    $_ -match "DeepSeek 冒烟|SKIP|原始数据|test result|FAILED|panicked|running \d+ test"
  }
  Write-Host ($interesting -join "`n")
  $latest = Get-ChildItem -LiteralPath (Join-Path $Repo ".verify\spikes\S-17") -Filter "deepseek-smoke-*.json" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($latest) {
    Write-Host "[s17] 原始脱敏数据：$($latest.FullName)"
    $record = Get-Content -LiteralPath $latest.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    Write-Host ("[s17] 摘要：ok={0} status={1} latencyMs={2} modelCount={3} models={4} errorCode={5}" -f `
        $record.ok, $record.status, $record.latencyMs, $record.modelCount, (($record.models | Select-Object -First 5) -join ","), $record.errorCode)
  }
  exit $code
} finally {
  Pop-Location
}
