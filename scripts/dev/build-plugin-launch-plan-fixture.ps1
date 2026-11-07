$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$sourceRoot = Join-Path $root 'examples\plugin-launch-plan-fixture'
$outputDir = Join-Path $root 'build\plugin-packages'
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null

$packages = foreach ($name in @('alpha', 'beta')) {
    $source = Join-Path $sourceRoot $name
    $manifest = Get-Content -LiteralPath (Join-Path $source 'ssmt-plugin.json') -Encoding UTF8 -Raw | ConvertFrom-Json
    $output = Join-Path $outputDir "$($manifest.id)-$($manifest.version).ssmtpkg"
    Compress-Archive -Path (Join-Path $source '*') -DestinationPath $output -CompressionLevel Optimal -Force
    [pscustomobject]@{
        Plugin = $manifest.id
        Package = $output
        Sha256 = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$packages | ConvertTo-Json -Depth 3
