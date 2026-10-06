[CmdletBinding()]
param([switch]$Build)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$resourcesDir = Join-Path $projectRoot 'src-tauri\resources'
$runtimeDir = Join-Path $projectRoot 'runtime'
$source = Join-Path $runtimeDir 'x64\Release\d3d11.dll'
$target = Join-Path $resourcesDir 'd3d11.dll'

if (-not (Test-Path -LiteralPath $resourcesDir -PathType Container)) {
    throw "SSMT 资源目录不存在：$resourcesDir"
}

if ($Build) {
    # runtime/release.ps1 负责构建，但其部署目标在这里限定为应用资源目录。
    & (Join-Path $runtimeDir 'release.ps1') -TestRuntimeDir $resourcesDir
    if ($LASTEXITCODE -ne 0) {
        throw "SSMT Runtime 构建失败：$LASTEXITCODE"
    }
    Remove-Item -LiteralPath (Join-Path $resourcesDir 'd3d11.pdb') -Force -ErrorAction SilentlyContinue
} else {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "SSMT Runtime 构建产物不存在：$source"
    }
    Copy-Item -LiteralPath $source -Destination $target -Force
}

if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
    throw "SSMT Runtime 未暂存：$target"
}
Write-Host "SSMT Runtime 已暂存：$target"
