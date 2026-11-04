#Requires -Version 5.1
<#
.SYNOPSIS
    框选 AI · 构建脚本。

.DESCRIPTION
    1) pnpm install（安装/补齐前端依赖）
    2) pnpm build:web（tsc + vite → dist/）
    3) 有 Rust 工具链时执行 pnpm build（tauri build，生成安装包）；
       无 Rust 时跳过桌面端构建，并说明缺工具链时可用产物与安装包将出现的位置。

.NOTES
    从仓库根目录（或任意目录）运行：powershell -ExecutionPolicy Bypass -File scripts\build.ps1
#>

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

Write-Host ""
Write-Host "=== 框选 AI · 构建 ===" -ForegroundColor Cyan
Write-Host "仓库根目录：$RepoRoot" -ForegroundColor DarkGray
Write-Host ""

# ---------------------------------------------------------------- 工具链检测

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host "[错误] 未找到 Node.js，请安装 Node.js ≥ 20：https://nodejs.org/zh-cn" -ForegroundColor Red
    exit 1
}
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Host "[错误] 未找到 pnpm，请先安装：corepack enable 或 npm install -g pnpm@latest" -ForegroundColor Red
    exit 1
}

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$hasRust = ($null -ne $cargo) -and (Test-Path -LiteralPath (Join-Path $RepoRoot "src-tauri\Cargo.toml"))

# ---------------------------------------------------------------- 1. 安装依赖

Write-Host "[1/3] 安装前端依赖：pnpm install" -ForegroundColor Cyan
& pnpm install
if ($LASTEXITCODE -ne 0) {
    Write-Host "[错误] pnpm install 失败（退出码 $LASTEXITCODE）。" -ForegroundColor Red
    exit $LASTEXITCODE
}

# ---------------------------------------------------------------- 2. Web 构建

Write-Host ""
Write-Host "[2/3] 构建前端：pnpm build:web" -ForegroundColor Cyan
& pnpm build:web
if ($LASTEXITCODE -ne 0) {
    Write-Host "[错误] pnpm build:web 失败（退出码 $LASTEXITCODE）。" -ForegroundColor Red
    exit $LASTEXITCODE
}
Write-Host "       Web 产物：$RepoRoot\dist" -ForegroundColor DarkGray

# ---------------------------------------------------------------- 3. 桌面端构建

Write-Host ""
if ($hasRust) {
    Write-Host "[3/3] 构建桌面安装包：pnpm build（tauri build，可能耗时数分钟）" -ForegroundColor Cyan
    & pnpm build
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[错误] pnpm build 失败（退出码 $LASTEXITCODE）。" -ForegroundColor Red
        Write-Host "       常见原因：link.exe 缺失（需 VS Build Tools「使用 C++ 的桌面开发」）、WebView2 未安装。" -ForegroundColor Yellow
        exit $LASTEXITCODE
    }
    Write-Host "       桌面端产物：src-tauri\target\release\bundle\（NSIS 安装包位于 nsis\ 子目录）" -ForegroundColor DarkGray
} else {
    Write-Host "[跳过] 未检测到 Rust 工具链（cargo），跳过桌面端构建。" -ForegroundColor Yellow
    Write-Host "       已完成的 Web 产物：$RepoRoot\dist" -ForegroundColor DarkGray
    Write-Host "       安装 Rust 后重新运行本脚本，或直接执行 pnpm build，安装包将输出到：" -ForegroundColor DarkGray
    Write-Host "         src-tauri\target\release\bundle\nsis\*.exe" -ForegroundColor DarkGray
    Write-Host "       安装步骤见 docs/开发指南.md「环境准备」。" -ForegroundColor DarkGray
}

Write-Host ""
Write-Host "构建流程结束。" -ForegroundColor Green
