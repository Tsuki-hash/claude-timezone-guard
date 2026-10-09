#requires -Version 7.0
<#
.SYNOPSIS
  打包 Claude 指纹切换器为可分发目录
.DESCRIPTION
  把 release 编译产物 + 说明文件 + 启动器复制到 dist 目录，
  产出可直接拷贝到别的 Windows 机器使用的文件夹，并打印 SHA256 供用户核对。

  说明文件取 rust\README.md（Rust 版的完整手册），不再回退到仓库根目录的
  旧版 README —— 那样会在 dist 里放一份描述已废弃 PowerShell 实现的文档。
#>
param(
    [string]$OutDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'dist')
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$root = Split-Path $PSScriptRoot -Parent
$rust = Join-Path $root 'rust'
$exe  = Join-Path $rust 'target\release\claude-fingerprint.exe'

if (-not (Test-Path $exe)) {
    Write-Host "✗ 找不到 $exe —— 先跑 cargo build --release" -ForegroundColor Red
    exit 1
}

# 先自检：单测不过就不应该打包
Write-Host "→ 运行单元测试…" -ForegroundColor Cyan
Push-Location $rust
try {
    & cargo test --release --quiet
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ 单元测试未通过，已中止打包" -ForegroundColor Red
        exit 1
    }
} finally {
    Pop-Location
}

if (Test-Path $OutDir) { Remove-Item $OutDir -Recurse -Force }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

# 1) 主程序
Copy-Item $exe (Join-Path $OutDir 'claude-fingerprint.exe')

# 2) 双击启动器
$bat = @'
@echo off
chcp 65001 >nul
cd /d "%~dp0"
start "" "%~dp0claude-fingerprint.exe"
'@
Set-Content -Path (Join-Path $OutDir '启动指纹切换器.bat') -Value $bat -Encoding utf8

# 3) 说明：只认 rust\README.md（Rust 版手册）。
#    刻意不回退到根目录 README：那份是旧 PowerShell 版，放进 dist 会误导用户。
$readme = Join-Path $PSScriptRoot 'README.md'
if (Test-Path $readme) {
    Copy-Item $readme (Join-Path $OutDir 'README.md')
} else {
    Write-Host "⚠ 未找到 $readme，dist 里将没有使用手册" -ForegroundColor Yellow
}

# MIT 许可证要求随分发附带版权声明与许可原文
$license = Join-Path $root 'LICENSE'
if (Test-Path $license) { Copy-Item $license (Join-Path $OutDir 'LICENSE') }

# 4) 校验和：用户需要一个能核对二进制的方式
$exeOut  = Join-Path $OutDir 'claude-fingerprint.exe'
$hash    = (Get-FileHash $exeOut -Algorithm SHA256).Hash
$size    = [math]::Round((Get-Item $exeOut).Length / 1MB, 2)

Set-Content -Path (Join-Path $OutDir 'SHA256SUMS.txt') -Encoding utf8 -Value @"
claude-fingerprint.exe  $hash

大小: $size MB
构建时间: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')

核对方法 (PowerShell):
  Get-FileHash .\claude-fingerprint.exe -Algorithm SHA256
"@

Write-Host ""
Write-Host "✓ 打包完成 -> $OutDir" -ForegroundColor Green
Write-Host "  claude-fingerprint.exe  $size MB"
Write-Host "  SHA256  $hash" -ForegroundColor Cyan
Get-ChildItem $OutDir | ForEach-Object { Write-Host "  - $($_.Name)" }
