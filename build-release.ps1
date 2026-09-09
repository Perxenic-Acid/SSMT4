<#
.SYNOPSIS
  一键打包 SSMT4 Release：构建 + 签名 + latest.json + 收集产物
.DESCRIPTION
  自动完成：
    1. 自动从 ~/.ssmt（项目外）读取更新签名私钥与密码，无需手动设置环境变量
    2. 构建 native/ submodule，并部署、验证必需的 Native 二进制
    3. bun run tauri build 构建并签名安装包
    4. 调用 generate-latest-json.ps1 生成更新元数据
    5. 将安装包 + .sig + latest.json 收集到项目根目录 publish/ 并自动打开
.EXAMPLE
  .\build-release.ps1                    # 使用当前版本号打包
  .\build-release.ps1 -Version 4.2.0     # 先升级版本号再打包
  .\build-release.ps1 -Notes "修复了xxx" # 指定更新说明
.PARAMETER Version
  可选。目标版本号（x.y.z）。提供后会自动写入 src-tauri/tauri.conf.json。
.PARAMETER Notes
  可选。更新说明，写入 latest.json。
.PARAMETER Repo
  可选。发布仓库（用于生成下载 URL），默认 Perxenic-Acid/SSMT4。
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$Notes = "Manual release",
    [string]$Repo = "Perxenic-Acid/SSMT4"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $projectRoot

$keyDir = Join-Path $HOME ".ssmt"
$privateKeyPath = Join-Path $keyDir "updater-signing.key"
$passwordFile = Join-Path $keyDir "updater-signing-key.password"

# ---------- 1. 密钥检查（自动从项目外读取，脚本本身不含任何密钥） ----------
if (-not (Test-Path -LiteralPath $privateKeyPath)) {
    throw "未找到更新签名私钥：$privateKeyPath`n请先运行：bunx tauri signer generate -w `"$privateKeyPath`" 生成密钥。"
}
if (-not (Test-Path -LiteralPath $passwordFile)) {
    throw "未找到私钥密码文件：$passwordFile"
}

$env:TAURI_SIGNING_PRIVATE_KEY_PATH = $privateKeyPath
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content -LiteralPath $passwordFile -Raw).Trim()

# ---------- 2. 版本号处理 ----------
$tauriConfigPath = Join-Path $projectRoot "src-tauri/tauri.conf.json"
$rawConfig = Get-Content -LiteralPath $tauriConfigPath -Raw

if ($Version) {
    if ($Version -notmatch "^\d+\.\d+\.\d+$") {
        throw "版本号格式应为 x.y.z，例如 4.2.0"
    }
    $updated = $rawConfig -replace '("version"\s*:\s*)"\d+\.\d+\.\d+"', ('${1}"' + $Version + '"')
    [System.IO.File]::WriteAllText($tauriConfigPath, $updated, [System.Text.UTF8Encoding]::new($false))
    Write-Host "已更新版本号 -> $Version" -ForegroundColor Cyan
}
else {
    $cfg = $rawConfig | ConvertFrom-Json
    $Version = [string]$cfg.version
}
Write-Host "本次打包版本：$Version" -ForegroundColor Cyan

# ---------- 3. 检查 bun ----------
if (-not (Get-Command bun -ErrorAction SilentlyContinue)) {
    throw "未找到 bun，请先安装：https://bun.sh"
}

# ---------- 4. 构建并验证 Native Release 产物 ----------
Write-Host "`n[1/4] 构建 Native Release ..." -ForegroundColor Yellow
$nativeDistDir = Join-Path $projectRoot "native/dist/Release"
$resourcesDir = Join-Path $projectRoot "src-tauri/resources"
$nativeArtifacts = @(
    @{
        Source = Join-Path $nativeDistDir "Run.exe"
        Target = Join-Path $resourcesDir "Run.exe"
    },
    @{
        Source = Join-Path $nativeDistDir "SSMT-Player-Tweaks.dll"
        Target = Join-Path $resourcesDir "SSMT-Player-Tweaks.dll"
    }
)

$nativeStagingFiles = @($nativeArtifacts | ForEach-Object { $_.Target })
Remove-Item -LiteralPath $nativeStagingFiles -Force -ErrorAction SilentlyContinue

try {
    & (Join-Path $projectRoot "build_release_cpp.ps1")

    foreach ($artifact in $nativeArtifacts) {
        if (-not (Test-Path -LiteralPath $artifact.Source -PathType Leaf)) {
            throw "Native Release 构建后缺少 dist 产物：$($artifact.Source)"
        }

        Copy-Item -LiteralPath $artifact.Source -Destination $artifact.Target -Force
    }

    foreach ($stagedFile in $nativeStagingFiles) {
        if (-not (Test-Path -LiteralPath $stagedFile -PathType Leaf)) {
            throw "Native Release 产物暂存失败：$stagedFile"
        }
    }
}
catch {
    Remove-Item -LiteralPath $nativeStagingFiles -Force -ErrorAction SilentlyContinue
    throw
}

# ---------- 5. 构建 Tauri（scripts/tauri-with-signing.mjs 会自动完成签名） ----------
Write-Host "`n[2/4] 开始 Tauri 构建（含更新签名）..." -ForegroundColor Yellow
bun run tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build 失败（退出码 $LASTEXITCODE）" }

# ---------- 6. 生成 latest.json ----------
Write-Host "`n[3/4] 生成 latest.json ..." -ForegroundColor Yellow
& (Join-Path $projectRoot "generate-latest-json.ps1") -Repo $Repo -Version $Version -Notes $Notes

# ---------- 7. 收集产物到 publish/ ----------
Write-Host "`n[4/4] 收集产物到 publish/ ..." -ForegroundColor Yellow
$bundleDir = Join-Path $projectRoot "src-tauri/target/release/bundle/nsis"
$publishDir = Join-Path $projectRoot "publish"
New-Item -ItemType Directory -Force -Path $publishDir | Out-Null

$exe = Get-ChildItem -LiteralPath $bundleDir -File |
    Where-Object { $_.Name -match "-setup\.exe$" } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $exe) { throw "未找到安装包：$bundleDir" }

$sig = "$($exe.FullName).sig"
if (-not (Test-Path -LiteralPath $sig)) { throw "未找到签名文件：$sig" }

Copy-Item -LiteralPath $exe.FullName -Destination $publishDir -Force
Copy-Item -LiteralPath $sig -Destination $publishDir -Force
Copy-Item -LiteralPath (Join-Path $bundleDir "latest.json") -Destination $publishDir -Force

Write-Host "`n============================================" -ForegroundColor Green
Write-Host "✅ 打包完成！发布产物在：" -ForegroundColor Green
Get-ChildItem -LiteralPath $publishDir -File | ForEach-Object { Write-Host "  $($_.FullName)" -ForegroundColor Green }
Write-Host "============================================" -ForegroundColor Green
Write-Host "`n下一步：把这些文件上传到 GitHub Release（Tag 填 v$Version）：" -ForegroundColor Cyan
Write-Host "  https://github.com/$Repo/releases/new?tag=v$Version" -ForegroundColor Cyan

Start-Process explorer.exe -ArgumentList "`"$publishDir`""
