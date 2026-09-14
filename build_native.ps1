[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Debug"
)

$ErrorActionPreference = "Stop"

$RepoRoot = $PSScriptRoot

$NativeDir = Join-Path $RepoRoot "native"
$BuildDir  = Join-Path $NativeDir "build"
$DistDir   = Join-Path $NativeDir "dist\$Configuration"

$ResourceDir = Join-Path $RepoRoot "src-tauri\resources"

$CargoProfile = if ($Configuration -eq "Release") {
    "release"
} else {
    "debug"
}

function Invoke-Checked {
    param(
        [string]$Command,
        [string[]]$Arguments
    )

    Write-Host ">> $Command $($Arguments -join ' ')"

    & $Command @Arguments

    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

Write-Host ""
Write-Host "=== Building SSMT Native [$Configuration] ==="
Write-Host ""

# ------------------------------------------------------------
# Rust
# ------------------------------------------------------------

$cargoArgs = @(
    "build",
    "--manifest-path", (Join-Path $NativeDir "Cargo.toml"),
    "-p", "ssmt-plugin-host"
)

if ($Configuration -eq "Release") {
    $cargoArgs += "--release"
}

Invoke-Checked "cargo" $cargoArgs

# ------------------------------------------------------------
# C++
# ------------------------------------------------------------

Invoke-Checked "cmake" @(
    "-S", $NativeDir,
    "-B", $BuildDir
)

Invoke-Checked "cmake" @(
    "--build", $BuildDir,
    "--config", $Configuration,
    "--parallel"
)

# ------------------------------------------------------------
# Collect Rust artifact into native/dist
# ------------------------------------------------------------

New-Item `
    -ItemType Directory `
    -Path $DistDir `
    -Force `
    | Out-Null

$PluginHostSource = Join-Path `
    $NativeDir `
    "target\$CargoProfile\ssmt_plugin_host.dll"

$PluginHostDist = Join-Path `
    $DistDir `
    "SSMT-PluginHost.dll"

if (-not (Test-Path -LiteralPath $PluginHostSource)) {
    throw "PluginHost build artifact not found: $PluginHostSource"
}

Copy-Item `
    -LiteralPath $PluginHostSource `
    -Destination $PluginHostDist `
    -Force

# ------------------------------------------------------------
# Verify complete runtime set
# ------------------------------------------------------------

$Artifacts = @(
    "Run.exe",
    "SSMT-Player-Tweaks.dll",
    "SSMT-PluginHost.dll"
)

foreach ($Name in $Artifacts) {
    $Path = Join-Path $DistDir $Name

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Native artifact not found: $Path"
    }
}

# ------------------------------------------------------------
# Stage for Tauri
# ------------------------------------------------------------

New-Item `
    -ItemType Directory `
    -Path $ResourceDir `
    -Force `
    | Out-Null

foreach ($Name in $Artifacts) {
    Copy-Item `
        -LiteralPath (Join-Path $DistDir $Name) `
        -Destination (Join-Path $ResourceDir $Name) `
        -Force
}

Write-Host ""
Write-Host "=== Native build complete ==="

foreach ($Name in $Artifacts) {
    Write-Host "  $Name"
}

Write-Host ""
Write-Host "Staged to:"
Write-Host "  $ResourceDir"