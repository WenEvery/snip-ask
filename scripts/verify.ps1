#Requires -Version 5.1
<#
.SYNOPSIS
    框选 AI · 一键校验脚本（本地等价 CI）。

.DESCRIPTION
    依次执行：
      Web：pnpm typecheck → pnpm build:web
      Rust（检测到 cargo 时）：cargo fmt --check → cargo clippy -D warnings → cargo test
    未安装 Rust 工具链时明确打印「跳过 Rust 检查（未安装工具链）」，最终输出 PASS/SKIP/FAIL 汇总；
    存在 FAIL 时以退出码 1 结束。

.NOTES
    从仓库根目录（或任意目录）运行：powershell -ExecutionPolicy Bypass -File scripts\verify.ps1
#>

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

Write-Host ""
Write-Host "=== 框选 AI · 校验 ===" -ForegroundColor Cyan
Write-Host "仓库根目录：$RepoRoot" -ForegroundColor DarkGray
Write-Host ""

$script:Results = New-Object System.Collections.Generic.List[object]

# 记录一条校验结果并按状态着色输出
function Add-Result {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Status,
        [string]$Detail = ""
    )
    $script:Results.Add([pscustomobject]@{
            Name   = $Name
            Status = $Status
            Detail = $Detail
        })
    switch ($Status) {
        "PASS" { $color = "Green" }
        "SKIP" { $color = "Yellow" }
        default { $color = "Red" }
    }
    Write-Host ("[{0}] {1}" -f $Status, $Name) -ForegroundColor $color
    if ($Detail) {
        Write-Host ("       {0}" -f $Detail) -ForegroundColor DarkGray
    }
}

# 执行一条命令：非零退出码视为失败（PowerShell 不会因原生命令失败而自动抛错，需显式判断）
function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @()
    )
    Write-Host ""
    Write-Host ("==> {0}：{1} {2}" -f $Name, $FilePath, ($Arguments -join " ")) -ForegroundColor Cyan
    # pnpm 等工具会把进度/横幅写到 stderr；Windows PowerShell 5.1 在 $ErrorActionPreference=Stop
    # 下会把原生命令的 stderr 当作 NativeCommandError 抛出，导致「实际成功却判为失败」。
    # 因此调用期间临时改为 Continue，并把 stderr 合并到标准输出后照常输出。
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $exitCode = 0
    try {
        & $FilePath @Arguments 2>&1 | Out-Host
        $exitCode = $LASTEXITCODE
    } catch {
        Add-Result -Name $Name -Status "FAIL" -Detail $_.Exception.Message
        $ErrorActionPreference = $previousErrorAction
        return $false
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    if ($null -ne $exitCode -and $exitCode -ne 0) {
        Add-Result -Name $Name -Status "FAIL" -Detail ("退出码 {0}" -f $exitCode)
        return $false
    }
    Add-Result -Name $Name -Status "PASS"
    return $true
}

# ---------------------------------------------------------------- 工具链检测

$hasPnpm = $null -ne (Get-Command pnpm -ErrorAction SilentlyContinue)
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$hasCargo = ($null -ne $cargo) -and (Test-Path -LiteralPath (Join-Path $RepoRoot "src-tauri\Cargo.toml"))

# ---------------------------------------------------------------- Web 检查

if (-not $hasPnpm) {
    Add-Result -Name "Web 工具链" -Status "FAIL" -Detail "未找到 pnpm；请安装 Node.js ≥ 20 与 pnpm ≥ 9 后重试"
} else {
    Invoke-Step -Name "TypeScript 类型检查" -FilePath "pnpm" -Arguments @("typecheck") | Out-Null
    Invoke-Step -Name "Web 构建" -FilePath "pnpm" -Arguments @("build:web") | Out-Null
}

# ---------------------------------------------------------------- 前端纯逻辑单测
# 使用 Node 原生 TypeScript 类型擦除（需 Node ≥ 22.18；开发机/CI 为 Node 24）。
$nodeTests = @(
    @{ Name = "覆盖翻译布局单测"; Path = "src/windows/overlay/translationLayout.test.ts" },
    @{ Name = "连续提问线程单测"; Path = "src/windows/result/thread.test.ts" }
)
foreach ($test in $nodeTests) {
    if (Test-Path -LiteralPath (Join-Path $RepoRoot $test.Path)) {
        Invoke-Step -Name $test.Name -FilePath "node" -Arguments @("--test", $test.Path) | Out-Null
    } else {
        Add-Result -Name $test.Name -Status "SKIP" -Detail ("未找到 {0}" -f $test.Path)
    }
}

# ---------------------------------------------------------------- Rust 检查

if (-not $hasCargo) {
    Write-Host ""
    Write-Host "跳过 Rust 检查（未安装工具链）。" -ForegroundColor Yellow
    Write-Host "  安装方式：https://rustup.rs/ （stable，≥ 1.77），并安装 Visual Studio Build Tools「使用 C++ 的桌面开发」。" -ForegroundColor DarkGray
    Write-Host "  提示：CI 会在 windows-latest 上执行 fmt/clippy/test，本地未装 Rust 不影响 Web 校验。" -ForegroundColor DarkGray
    Add-Result -Name "Rust：cargo fmt --check" -Status "SKIP" -Detail "未安装 Rust 工具链"
    Add-Result -Name "Rust：cargo clippy -D warnings" -Status "SKIP" -Detail "未安装 Rust 工具链"
    Add-Result -Name "Rust：cargo test" -Status "SKIP" -Detail "未安装 Rust 工具链"
} else {
    $manifest = "src-tauri/Cargo.toml"
    Invoke-Step -Name "Rust：cargo fmt --check" -FilePath "cargo" `
        -Arguments @("fmt", "--manifest-path", $manifest, "--", "--check") | Out-Null
    Invoke-Step -Name "Rust：cargo clippy -D warnings" -FilePath "cargo" `
        -Arguments @("clippy", "--manifest-path", $manifest, "--all-targets", "--", "-D", "warnings") | Out-Null
    Invoke-Step -Name "Rust：cargo test" -FilePath "cargo" `
        -Arguments @("test", "--manifest-path", $manifest) | Out-Null
}

# ---------------------------------------------------------------- 汇总

Write-Host ""
Write-Host "================ 校验汇总 ================" -ForegroundColor Cyan
$summary = $script:Results | Format-Table -AutoSize -Property Status, Name, Detail | Out-String
Write-Host $summary

$failCount = @($script:Results | Where-Object { $_.Status -eq "FAIL" }).Count
$skipCount = @($script:Results | Where-Object { $_.Status -eq "SKIP" }).Count
$passCount = @($script:Results | Where-Object { $_.Status -eq "PASS" }).Count

Write-Host ("PASS {0} 项 / SKIP {1} 项 / FAIL {2} 项" -f $passCount, $skipCount, $failCount) -ForegroundColor Cyan

if ($failCount -gt 0) {
    Write-Host "校验未通过。" -ForegroundColor Red
    exit 1
}

Write-Host "校验通过（SKIP 项需在安装对应工具链或 CI 上补验）。" -ForegroundColor Green
exit 0
