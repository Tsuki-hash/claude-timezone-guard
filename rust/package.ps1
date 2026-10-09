#requires -Version 7.0
<#
.SYNOPSIS
  打包 Claude 指纹切换器为可分发目录
.DESCRIPTION
  把 release 编译产物 + 说明文件 + 启动器复制到 dist 目录，
  产出可直接拷贝到别的 Windows 机器使用的文件夹。
#>
param(
    [string]$OutDir = "D:\Agent-Project\Hermes\claude-tools\dist"
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

# 3) 说明（优先用 rust 目录内的，没有就跳过）
$readme = Join-Path $PSScriptRoot 'README.md'
if (-not (Test-Path $readme)) { $readme = Join-Path $root 'README.md' }
if (Test-Path $readme) { Copy-Item $readme (Join-Path $OutDir 'README.md') }

$size = [math]::Round((Get-Item (Join-Path $OutDir 'claude-fingerprint.exe')).Length / 1MB, 2)
Write-Host "✓ 打包完成 -> $OutDir" -ForegroundColor Green
Write-Host "  claude-fingerprint.exe  ${size} MB"
Get-ChildItem $OutDir | ForEach-Object { Write-Host "  - $($_.Name)" }
