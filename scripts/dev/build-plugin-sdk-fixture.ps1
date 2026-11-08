$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$source = Join-Path $root 'examples\plugin-sdk-fixture'
$outputDir = Join-Path $root 'build\plugin-packages'
$output = Join-Path $outputDir 'example.codex.page-sdk-1.0.1.ssmtpkg'
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
Compress-Archive -Path (Join-Path $source '*') -DestinationPath $output -CompressionLevel Optimal -Force
$hash = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
[pscustomobject]@{
    Package = $output
    Sha256 = $hash
} | ConvertTo-Json
