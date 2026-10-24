param(
    [Parameter(Mandatory = $true)]
    [string]$SecondaryMotionRoot,

    [Parameter(Mandatory = $true)]
    [string]$DllPath,

    [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifestRoot = Join-Path $repoRoot "integration/secondary-motion/3.2.0"
$stagingRoot = Join-Path $repoRoot ("build/plugin-package-staging/ssmt.secondary-motion-" + $PID + "-" + [DateTime]::UtcNow.Ticks)
$packageRoot = Join-Path $repoRoot "build/plugin-packages"
$publishRoot = Join-Path $repoRoot "publish-local"

$SecondaryMotionRoot = (Resolve-Path -LiteralPath $SecondaryMotionRoot).Path
$DllPath = (Resolve-Path -LiteralPath $DllPath).Path
if ((Split-Path -Leaf $DllPath) -ne "sbm.dll") {
    throw "DllPath must point to sbm.dll"
}
if (!(Test-Path -LiteralPath (Join-Path $SecondaryMotionRoot "data/characters.default.json"))) {
    throw "SecondaryMotionRoot is missing data/characters.default.json"
}
if (!(Test-Path -LiteralPath (Join-Path $SecondaryMotionRoot "presets/Default.json"))) {
    throw "SecondaryMotionRoot is missing presets/Default.json"
}

New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "plugin"), (Join-Path $stagingRoot "SecondaryMotion/data"), (Join-Path $stagingRoot "SecondaryMotion/presets"), (Join-Path $stagingRoot "SecondaryMotion/runtime"), $packageRoot, $publishRoot | Out-Null

Copy-Item -LiteralPath (Join-Path $manifestRoot "ssmt-plugin.json") -Destination $stagingRoot
Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $manifestRoot) "README.md") -Destination (Join-Path $stagingRoot "README.md")
Copy-Item -LiteralPath $DllPath -Destination (Join-Path $stagingRoot "plugin/sbm.dll")
Copy-Item -LiteralPath (Join-Path $SecondaryMotionRoot "data/characters.default.json") -Destination (Join-Path $stagingRoot "SecondaryMotion/data/characters.default.json")

Get-ChildItem -LiteralPath (Join-Path $SecondaryMotionRoot "presets") -File -Filter "*.json" |
    Where-Object { $_.Name -notlike "*.template.json" } |
    Copy-Item -Destination (Join-Path $stagingRoot "SecondaryMotion/presets")

Copy-Item -LiteralPath (Join-Path $SecondaryMotionRoot "runtime/config.json") -Destination (Join-Path $stagingRoot "SecondaryMotion/runtime/config.json")

$license = Join-Path (Split-Path -Parent $SecondaryMotionRoot) "LICENSE"
$notices = Join-Path (Split-Path -Parent $SecondaryMotionRoot) "THIRD_PARTY_NOTICES.md"
if (Test-Path -LiteralPath $license) {
    Copy-Item -LiteralPath $license -Destination (Join-Path $stagingRoot "LICENSE")
}
if (Test-Path -LiteralPath $notices) {
    Copy-Item -LiteralPath $notices -Destination (Join-Path $stagingRoot "THIRD_PARTY_NOTICES.md")
}

if (!$OutputPath.Trim()) {
    $OutputPath = Join-Path $packageRoot "ssmt.secondary-motion-3.2.0.ssmtpkg"
}
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
$outputParent = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Force -Path $outputParent | Out-Null

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$stream = [IO.File]::Open($OutputPath, [IO.FileMode]::Create, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
$archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $false)
try {
    Get-ChildItem -LiteralPath $stagingRoot -Recurse -File | ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($stagingRoot, $_.FullName).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive,
            $_.FullName,
            $relative,
            [IO.Compression.CompressionLevel]::Optimal
        ) | Out-Null
    }
}
finally {
    $archive.Dispose()
    $stream.Dispose()
}

$publishCopy = Join-Path $publishRoot (Split-Path -Leaf $OutputPath)
Copy-Item -LiteralPath $OutputPath -Destination $publishCopy -Force

Get-Item -LiteralPath $OutputPath, $publishCopy |
    Select-Object FullName, Length, LastWriteTime
Get-FileHash -LiteralPath $OutputPath -Algorithm SHA256 |
    Select-Object Path, Hash
