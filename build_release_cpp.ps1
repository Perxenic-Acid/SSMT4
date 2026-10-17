[CmdletBinding()]
param(
    [string]$Architecture = "x64",
    [string]$Generator = "",
    [switch]$Clean
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$SourceDir = Join-Path $RepoRoot "native"
$BuildDir = Join-Path $RepoRoot "build\native-release"
$DistDir = Join-Path $SourceDir "dist\Release"
$NativeArtifacts = @(
    (Join-Path $DistDir "Run.exe")
)

if (-not (Test-Path -LiteralPath (Join-Path $SourceDir "CMakeLists.txt"))) {
    throw "Native submodule 未初始化。请运行：git submodule update --init --recursive"
}

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    throw "未找到 cmake，请先安装 CMake。"
}

function Invoke-CMake {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)

    & cmake @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cmake 失败（退出码 $LASTEXITCODE）：cmake $($Arguments -join ' ')"
    }
}

if ($Clean -and (Test-Path $BuildDir)) {
    Remove-Item -LiteralPath $BuildDir -Recurse -Force
}

$configureArgs = @("-S", $SourceDir, "-B", $BuildDir)
$configureArgs += @("-DSSMT_BUILD_PLAYER_TWEAKS=OFF", "-DSSMT_BUILD_GIMI_BRIDGE=OFF", "-DSSMT_BUILD_POSER_PROBE=OFF")

if ($Generator) {
    $configureArgs += @("-G", $Generator)
} elseif (-not $env:CMAKE_GENERATOR) {
    $cmakeHelp = cmake --help
    if ($cmakeHelp -match "Visual Studio 18 2026") {
        $configureArgs += @("-G", "Visual Studio 18 2026")
    } elseif ($cmakeHelp -match "Visual Studio 17 2022") {
        $configureArgs += @("-G", "Visual Studio 17 2022")
    }
}

if (($configureArgs -contains "Visual Studio 18 2026") -or ($configureArgs -contains "Visual Studio 17 2022")) {
    $configureArgs += @("-A", $Architecture)
} else {
    $configureArgs += "-DCMAKE_BUILD_TYPE=Release"
}

Remove-Item -LiteralPath $NativeArtifacts -Force -ErrorAction SilentlyContinue

try {
    Invoke-CMake @configureArgs
    Invoke-CMake --build $BuildDir --config Release --parallel

    foreach ($artifact in $NativeArtifacts) {
        if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
            throw "Native Release 构建未生成 dist 产物：$artifact"
        }
    }
}
catch {
    Remove-Item -LiteralPath $NativeArtifacts -Force -ErrorAction SilentlyContinue
    throw
}
