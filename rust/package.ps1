#requires -Version 7.0
<#
.SYNOPSIS
  打包 Claude 指纹切换器为可分发目录
.DESCRIPTION
  先跑测试、再编译、校验产物比源码新、然后复制到 dist，最后生成 SHA256SUMS.txt。

  几个刻意的设计：
    * 脚本自己执行 cargo build，而不是"捡一个已有的 exe 就复制" ——
      否则很容易把旧二进制（甚至 CLI 不可用的版本）连同"看起来正常"的
      校验和一起发出去，而校验和无法反映源码状态。
    * 校验 exe 的修改时间不早于任何 src/*.rs，杜绝发布落后于源码的产物。
    * 用 --remap-path-prefix 抹掉构建机上的绝对路径：Cargo 会把 registry 与
      build 脚本的路径写进 panic/diagnostic 字符串，`strip = true` 清不掉，
      发布二进制里会残留开发者的 Windows 用户名与目录结构。
    * 说明文件只取 rust\README.md（Rust 版手册），不回退到仓库根目录的 README。
#>
param(
    [string]$OutDir = (Join-Path (Split-Path $PSScriptRoot -Parent) 'dist')
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$root = Split-Path $PSScriptRoot -Parent
$rust = $PSScriptRoot
$exe = Join-Path $rust 'target\release\claude-fingerprint.exe'

# --- 安全护栏：$OutDir 可被调用者传入，而下面会递归删除它 -------------
# 要求解析后的绝对路径位于仓库内，避免误删任意目录。
$rootFull = [System.IO.Path]::GetFullPath($root).TrimEnd('\')
$outFull = [System.IO.Path]::GetFullPath($OutDir).TrimEnd('\')
if (-not $outFull.StartsWith($rootFull + '\', [StringComparison]::OrdinalIgnoreCase)) {
    Write-Host "✗ 拒绝执行：输出目录不在仓库内" -ForegroundColor Red
    Write-Host "   仓库: $rootFull"
    Write-Host "   输出: $outFull"
    exit 1
}

# --- 构建（含测试门禁）------------------------------------------------
Write-Host "→ 运行单元测试…" -ForegroundColor Cyan
Push-Location $rust
try {
    & cargo test --release --quiet
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ 单元测试未通过，已中止打包" -ForegroundColor Red
        exit 1
    }

    Write-Host "→ 编译 release（抹掉构建机绝对路径）…" -ForegroundColor Cyan
    # 让二进制里不残留构建机用户名、目录结构与 registry 镜像路径。
    #
    # 这里对 registry 源目录做**逐目录**精确映射：cargo 会把每个 crate 源码的
    # 绝对路径交给 rustc（panic/diagnostic 位置用），只映射 %USERPROFILE% 会剩下
    # 相对片段；而镜像目录名本身还带一个构建机哈希后缀（如
    # `rsproxy.cn-e3de039b2554c837`），所以连它一起映射成中性名字。
    $remaps = @(
        "--remap-path-prefix=$env:USERPROFILE=/build/home"
        "--remap-path-prefix=$root=/build/project"
    )
    $regSrc = Join-Path $env:USERPROFILE '.cargo\registry\src'
    if (Test-Path $regSrc) {
        Get-ChildItem $regSrc -Directory | ForEach-Object {
            $remaps += "--remap-path-prefix=$($_.FullName)=/build/cargo/$($_.Name.Split('-')[0])"
        }
    }
    $env:RUSTFLAGS = $remaps -join ' '
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) {
        Write-Host "✗ 编译失败，已中止打包" -ForegroundColor Red
        exit 1
    }
} finally {
    Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
    Pop-Location
}

if (-not (Test-Path $exe)) {
    Write-Host "✗ 找不到 $exe" -ForegroundColor Red
    exit 1
}

# --- 新鲜度校验：产物不得落后于源码 -----------------------------------
$exeTime = (Get-Item $exe).LastWriteTimeUtc
$newer = Get-ChildItem (Join-Path $rust 'src') -Filter *.rs -File |
    Where-Object { $_.LastWriteTimeUtc -gt $exeTime }
if ($newer) {
    Write-Host "✗ 产物比源码旧，拒绝打包（下面这些源文件比 exe 新）：" -ForegroundColor Red
    $newer | ForEach-Object { Write-Host "   $($_.Name)  $($_.LastWriteTime)" }
    exit 1
}

# --- 组装 dist --------------------------------------------------------
if (Test-Path $outFull) { Remove-Item $outFull -Recurse -Force }
New-Item -ItemType Directory -Path $outFull -Force | Out-Null

Copy-Item $exe (Join-Path $outFull 'claude-fingerprint.exe')

$bat = @'
@echo off
chcp 65001 >nul
cd /d "%~dp0"
start "" "%~dp0claude-fingerprint.exe"
'@
Set-Content -Path (Join-Path $outFull '启动指纹切换器.bat') -Value $bat -Encoding utf8

# 说明：只认 rust\README.md。复制时把指向仓库根的相对链接改写成 dist 内的实际文件，
# 否则分发出去的手册里 [MIT](../LICENSE) 是个死链。评审报告只留本地、不入库，
# rust\README.md 不再引用它，这里也就不需要改写规则。
$readme = Join-Path $rust 'README.md'
if (Test-Path $readme) {
    $md = Get-Content $readme -Raw -Encoding UTF8
    # 分发目录里的手册不能引用仓库内的相对路径，否则全是死链。
    $md = $md -replace '\]\(\.\./LICENSE\)', '](LICENSE)'
    $md = $md -replace '\]\(\.\./THIRD-PARTY-LICENSES\.md\)', '](THIRD-PARTY-LICENSES.md)'
    Set-Content -Path (Join-Path $outFull 'README.md') -Value $md -Encoding UTF8 -NoNewline
} else {
    Write-Host "⚠ 未找到 $readme，dist 里将没有使用手册" -ForegroundColor Yellow
}

# MIT 要求随分发附带版权声明与许可原文
$license = Join-Path $root 'LICENSE'
if (Test-Path $license) { Copy-Item $license (Join-Path $outFull 'LICENSE') }

# 第三方依赖许可：本项目的直接依赖里有 206 个是 Apache-2.0，
# 而 Apache-2.0 §4 要求二进制分发时随附许可证副本与保留声明。
# 缺这个文件是真实的合规缺口，不是"可选的加分项"。
$thirdParty = Join-Path $root 'THIRD-PARTY-LICENSES.md'
if (Test-Path $thirdParty) {
    Copy-Item $thirdParty (Join-Path $outFull 'THIRD-PARTY-LICENSES.md')
} else {
    Write-Host "⚠ 未找到 THIRD-PARTY-LICENSES.md —— 分发目录将缺少第三方许可声明" -ForegroundColor Yellow
    Write-Host "  可用 cargo install --locked --features cli cargo-about 重新生成" -ForegroundColor Yellow
}

# --- 校验和 -----------------------------------------------------------
$exeOut = Join-Path $outFull 'claude-fingerprint.exe'
$hash = (Get-FileHash $exeOut -Algorithm SHA256).Hash
$size = [math]::Round((Get-Item $exeOut).Length / 1MB, 2)
$version = (Select-String -Path (Join-Path $rust 'Cargo.toml') -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
Push-Location $root
try { $commit = (& git rev-parse --short HEAD 2>$null) } finally { Pop-Location }
if (-not $commit) { $commit = 'unknown' }

Set-Content -Path (Join-Path $outFull 'SHA256SUMS.txt') -Encoding utf8 -Value @"
claude-fingerprint.exe  $hash

版本: $version    提交: $commit
大小: $size MB
构建时间: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')

核对方法 (PowerShell):
  Get-FileHash .\claude-fingerprint.exe -Algorithm SHA256
"@

# --- 隐私自检：确认构建机路径没有残留在二进制里 -----------------------
Write-Host "→ 检查二进制是否残留构建机路径…" -ForegroundColor Cyan
$bytes = [System.IO.File]::ReadAllBytes($exeOut)
$ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
$leaks = @()
foreach ($needle in @($env:USERPROFILE, $root, $env:USERNAME)) {
    if ($needle -and $ascii.Contains($needle)) { $leaks += $needle }
}
if ($leaks.Count -gt 0) {
    Write-Host "⚠ 二进制里仍能找到这些本机字符串（可能泄露构建环境）：" -ForegroundColor Yellow
    $leaks | ForEach-Object { Write-Host "   $_" }
} else {
    Write-Host "  ✓ 未发现本机路径/用户名残留" -ForegroundColor Green
}

Write-Host ""
Write-Host "✓ 打包完成 -> $outFull" -ForegroundColor Green
Write-Host "  claude-fingerprint.exe  $size MB   v$version ($commit)"
Write-Host "  SHA256  $hash" -ForegroundColor Cyan
Get-ChildItem $outFull | ForEach-Object { Write-Host "  - $($_.Name)" }
