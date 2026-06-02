# 构建并安装 AskHuman 到用户目录（Windows）。
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\AskHuman" }
Set-Location $ScriptDir

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
  Write-Error "需要 pnpm（npm i -g pnpm）"; exit 1
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Write-Error "需要 Rust 工具链（https://rustup.rs）"; exit 1
}

Write-Host "==> 安装前端依赖"
pnpm install

Write-Host "==> 构建前端 (dist/)"
pnpm build

Write-Host "==> 编译 release（前端资源在此步骤被嵌入）"
cargo build --release --manifest-path src-tauri/Cargo.toml

$Bin = "src-tauri\target\release\AskHuman.exe"
if (-not (Test-Path $Bin)) { Write-Error "未找到编译产物 $Bin"; exit 1 }

Write-Host "==> 安装到 $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $Bin (Join-Path $InstallDir "AskHuman.exe") -Force

Write-Host "==> 完成：$InstallDir\AskHuman.exe"
Write-Host "提示: 请将 $InstallDir 加入 PATH 后即可在终端使用 AskHuman。"
