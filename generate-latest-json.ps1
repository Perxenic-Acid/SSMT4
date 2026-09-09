[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string]$Repo = "Perxenic-Acid/SSMT4",

    [Parameter(Mandatory = $false)]
    [string]$Tag,

    [Parameter(Mandatory = $false)]
    [string]$Version,

    [Parameter(Mandatory = $false)]
    [string]$Notes = "Manual release",

    [Parameter(Mandatory = $false)]
    [string]$Platform = "windows-x86_64",

    [Parameter(Mandatory = $false)]
    [string]$ExePath,

    [Parameter(Mandatory = $false)]
    [string]$SigPath,

    [Parameter(Mandatory = $false)]
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-ProjectRoot {
    $scriptDir = Split-Path -Parent $PSCommandPath
    return (Resolve-Path $scriptDir).Path
}

function Get-VersionFromExeName {
    param([Parameter(Mandatory = $true)][string]$FileName)

    if ($FileName -match "_(?<version>\d+\.\d+\.\d+)_") {
        return $Matches.version
    }

    return $null
}

function Get-VersionFromTauriConfig {
    param([Parameter(Mandatory = $true)][string]$ConfigPath)

    if (-not (Test-Path -LiteralPath $ConfigPath)) {
        return $null
    }

    $raw = Get-Content -LiteralPath $ConfigPath -Raw
    $cfg = $raw | ConvertFrom-Json
    return [string]$cfg.version
}

$projectRoot = Resolve-ProjectRoot
$bundleDir = Join-Path $projectRoot "src-tauri/target/release/bundle/nsis"

if (-not $ExePath) {
    if (-not (Test-Path -LiteralPath $bundleDir)) {
        throw "Bundle directory not found: $bundleDir"
    }

    $exe = Get-ChildItem -LiteralPath $bundleDir -File | Where-Object {
        $_.Name -match "-setup\.exe$"
    } | Sort-Object LastWriteTime -Descending | Select-Object -First 1

    if (-not $exe) {
        throw "No setup exe found in $bundleDir"
    }

    $ExePath = $exe.FullName
}

$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if (-not $SigPath) {
    $SigPath = "$ExePath.sig"
}

if (-not (Test-Path -LiteralPath $SigPath)) {
    throw "Signature file not found: $SigPath"
}

$SigPath = (Resolve-Path -LiteralPath $SigPath).Path

if (-not $Version) {
    $Version = Get-VersionFromExeName -FileName ([IO.Path]::GetFileName($ExePath))
}

if (-not $Version) {
    $tauriConfigPath = Join-Path $projectRoot "src-tauri/tauri.conf.json"
    $Version = Get-VersionFromTauriConfig -ConfigPath $tauriConfigPath
}

if (-not $Version) {
    throw "Unable to infer version. Please pass -Version explicitly."
}

if (-not $Tag) {
    $Tag = "v$Version"
}

if (-not $OutputPath) {
    $OutputPath = Join-Path (Split-Path -Parent $ExePath) "latest.json"
}

$signature = (Get-Content -LiteralPath $SigPath -Raw).Trim()
if (-not $signature) {
    throw "Signature file is empty: $SigPath"
}

$exeFileName = [IO.Path]::GetFileName($ExePath)
$downloadUrl = "https://github.com/$Repo/releases/download/$Tag/$exeFileName"
$pubDate = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

$latest = [ordered]@{
    version   = $Version
    notes     = $Notes
    pub_date  = $pubDate
    platforms = [ordered]@{
        $Platform = [ordered]@{
            signature = $signature
            url       = $downloadUrl
        }
    }
}

$json = $latest | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText(
    $OutputPath,
    $json,
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host "Generated latest.json successfully:" -ForegroundColor Green
Write-Host "  Output   : $OutputPath"
Write-Host "  Version  : $Version"
Write-Host "  Tag      : $Tag"
Write-Host "  Repo     : $Repo"
Write-Host "  Platform : $Platform"
Write-Host "  URL      : $downloadUrl"
