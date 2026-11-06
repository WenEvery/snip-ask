#Requires -Version 5.1
<#
.SYNOPSIS
    框选 AI · 开发启动脚本。

.DESCRIPTION
    检测 Node.js / pnpm / Rust 工具链：
      - 有 Rust：安装依赖（如缺失）后执行 `pnpm dev`（tauri dev，Vite 1420 + Rust）。
      - 无 Rust：提示可先用 `pnpm dev:web` 在浏览器中预览（mock 后端），并给出安装 Rust 的步骤，不视为错误。

.NOTES
    从仓库根目录（或任意目录）运行：powershell -ExecutionPolicy Bypass -File scripts\dev.ps1
#>

$ErrorActionPreference = "Stop"

# 以脚本所在位置定位仓库根目录，避免依赖当前工作目录
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $RepoRoot

Write-Host ""
Write-Host "=== 框选 AI · 开发启动 ===" -ForegroundColor Cyan
Write-Host "仓库根目录：$RepoRoot" -ForegroundColor DarkGray
Write-Host ""

# ---------------------------------------------------------------- 工具链检测

$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) {
    Write-Host "[错误] 未找到 Node.js。" -ForegroundColor Red
    Write-Host "        请安装 Node.js ≥ 20（推荐 22 LTS）：https://nodejs.org/zh-cn" -ForegroundColor Yellow
    exit 1
}

$nodeVersion = (& node --version).Trim()
Write-Host "[OK] Node.js $nodeVersion"

$pnpm = Get-Command pnpm -ErrorAction SilentlyContinue
if (-not $pnpm) {
    Write-Host "[错误] 未找到 pnpm。" -ForegroundColor Red
    Write-Host "        请先安装：corepack enable，或 npm install -g pnpm@latest" -ForegroundColor Yellow
    exit 1
}
Write-Host ("[OK] pnpm {0}" -f (& pnpm --version).Trim())

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
$tauriManifest = Join-Path $RepoRoot "src-tauri\Cargo.toml"
$hasRust = ($null -ne $cargo) -and (Test-Path -LiteralPath $tauriManifest)

# ---------------------------------------------------------------- 无 Rust：浏览器预览

if (-not $hasRust) {
    Write-Host ""
    Write-Host "[提示] 未检测到 Rust 工具链（cargo）。" -ForegroundColor Yellow
    Write-Host "       桌面版（pnpm dev）需要 Rust + MSVC 链接器；当前可先用浏览器预览前端：" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "         pnpm install      # 首次需要" -ForegroundColor White
    Write-Host "         pnpm dev:web      # 浏览器 mock 预览 http://localhost:1420/" -ForegroundColor White
    Write-Host ""
    Write-Host "       可选窗口：http://localhost:1420/?window=overlay" -ForegroundColor DarkGray
    Write-Host "                 http://localhost:1420/?window=result" -ForegroundColor DarkGray
    Write-Host ""
    Write-Host "       安装桌面开发工具链（装好后重新运行本脚本）：" -ForegroundColor Yellow
    Write-Host "         1) Rustup：https://rustup.rs/ （stable，≥ 1.77）" -ForegroundColor White
    Write-Host "         2) Visual Studio Build Tools 2022，勾选「使用 C++ 的桌面开发」" -ForegroundColor White
    Write-Host "            （含 MSVC v143 生成工具与 Windows 10/11 SDK）" -ForegroundColor White
    Write-Host "         3) WebView2 Runtime（Win11 自带；Win10 由安装器引导安装）" -ForegroundColor White
    Write-Host ""
    Write-Host "       详见 docs/开发指南.md「环境准备」。" -ForegroundColor DarkGray
    Write-Host ""
    exit 0
}

Write-Host ("[OK] Rust {0}" -f ((& cargo --version) -replace '^cargo ', ''))

# ---------------------------------------------------------------- 安装依赖并启动

$nodeModules = Join-Path $RepoRoot "node_modules"
if (-not (Test-Path -LiteralPath $nodeModules)) {
    Write-Host ""
    Write-Host "[1/2] 未找到 node_modules，正在执行 pnpm install ..." -ForegroundColor Cyan
    & pnpm install
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[错误] pnpm install 失败（退出码 $LASTEXITCODE）。" -ForegroundColor Red
        exit $LASTEXITCODE
    }
} else {
    Write-Host "[1/2] 依赖已安装（node_modules 存在），跳过 pnpm install。"
}

Write-Host "[2/2] 启动桌面开发模式：pnpm dev（首次编译 Rust 可能较慢）..." -ForegroundColor Cyan
Write-Host ""
& pnpm dev
if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "[错误] pnpm dev 退出，退出码 $LASTEXITCODE。" -ForegroundColor Red
    Write-Host "       常见原因：link.exe 缺失（需 VS Build Tools C++ 工作负载）、WebView2 未安装、1420 端口被占用。" -ForegroundColor Yellow
    Write-Host "       详见 docs/开发指南.md「常见问题」。" -ForegroundColor DarkGray
    exit $LASTEXITCODE
}
