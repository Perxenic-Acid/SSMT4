[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Debug",

    [switch]$BuildTestPlugin,

    [switch]$DeployToTestRuntime
)

$ErrorActionPreference = "Stop"

$RepoRoot = $PSScriptRoot
$NativeDir = Join-Path $RepoRoot "native"
$BuildDir = Join-Path $NativeDir "build"
$DistDir = Join-Path $NativeDir "dist\$Configuration"
$ResourceDir = Join-Path $RepoRoot "src-tauri\resources"
$TestRuntimeDir = Join-Path `
    $env:USERPROFILE `
    "Desktop\SSMT3\SSMTDefaultCacheFolder\3Dmigoto\GIMI"

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

$cargoArgs = @(
    "build",
    "--manifest-path", (Join-Path $NativeDir "Cargo.toml"),
    "-p", "ssmt-plugin-host"
)

if ($Configuration -eq "Release") {
    $cargoArgs += "--release"
}

Invoke-Checked "cargo" $cargoArgs

Invoke-Checked "cmake" @(
    "-S", $NativeDir,
    "-B", $BuildDir
)

Invoke-Checked "cmake" @(
    "--build", $BuildDir,
    "--config", $Configuration,
    "--parallel"
)

New-Item `
    -ItemType Directory `
    -Path $DistDir `
    -Force `
    | Out-Null

$PluginHostSource = Join-Path `
    $NativeDir `
    "target\$CargoProfile\ssmt_plugin_host.dll"

$PluginHostDist = Join-Path $DistDir "SSMT-PluginHost.dll"

if (-not (Test-Path -LiteralPath $PluginHostSource)) {
    throw "PluginHost build artifact not found: $PluginHostSource"
}

Copy-Item `
    -LiteralPath $PluginHostSource `
    -Destination $PluginHostDist `
    -Force

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

if ($BuildTestPlugin) {
    $testPluginArgs = @(
        "build",
        "--manifest-path", (Join-Path $NativeDir "Cargo.toml"),
        "-p", "ssmt-test-plugin"
    )

    if ($Configuration -eq "Release") {
        $testPluginArgs += "--release"
    }

    Invoke-Checked "cargo" $testPluginArgs

    $TestPluginSource = Join-Path `
        $NativeDir `
        "target\$CargoProfile\ssmt_test_plugin.dll"

    $TestPluginDirectory = Join-Path $NativeDir "Plugins"
    $TestPluginDestination = Join-Path `
        $TestPluginDirectory `
        "ssmt_test_plugin.dll"

    if (-not (Test-Path -LiteralPath $TestPluginSource)) {
        throw "Test plugin build artifact not found: $TestPluginSource"
    }

    New-Item `
        -ItemType Directory `
        -Path $TestPluginDirectory `
        -Force `
        | Out-Null

    Copy-Item `
        -LiteralPath $TestPluginSource `
        -Destination $TestPluginDestination `
        -Force

    Write-Host "Test plugin staged to: $TestPluginDestination"
}

if ($DeployToTestRuntime) {
    if (-not (Test-Path -LiteralPath $TestRuntimeDir -PathType Container)) {
        throw "Test runtime directory does not exist: $TestRuntimeDir"
    }

    foreach ($Name in $Artifacts) {
        Copy-Item `
            -LiteralPath (Join-Path $DistDir $Name) `
            -Destination (Join-Path $TestRuntimeDir $Name) `
            -Force
    }

    Write-Host "Native runtime deployed to: $TestRuntimeDir"
}

Write-Host ""
Write-Host "=== Native build complete ==="

foreach ($Name in $Artifacts) {
    Write-Host "  $Name"
}

Write-Host ""
Write-Host "Staged to:"
Write-Host "  $ResourceDir"
