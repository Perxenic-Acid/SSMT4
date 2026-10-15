[CmdletBinding()]
param(
    [string]$GameConfigPath = (Join-Path $env:LOCALAPPDATA 'SSMT4GlobalConfigs\Games\WWMI\Config.json'),
    [switch]$ValidateOnly,
    [switch]$NoDialogs
)

$ErrorActionPreference = 'Stop'

function Show-LaunchMessage {
    param([string]$Message, [switch]$IsError)
    if ($NoDialogs) {
        Write-Host $Message
        return
    }
    Add-Type -AssemblyName System.Windows.Forms
    $icon = if ($IsError) { 'Error' } else { 'Information' }
    [System.Windows.Forms.MessageBox]::Show($Message, '鸣潮：DLSS5 + Mod', 'OK', $icon) | Out-Null
}

try {
    $config = Get-Content -Raw -LiteralPath $GameConfigPath -Encoding UTF8 | ConvertFrom-Json
    $runtimeDir = $config.installDir
    $gameExe = $config.targetExePath
    if (-not $runtimeDir -or -not $gameExe) {
        throw '未找到鸣潮安装设置，请先在 SSMT4 中配置鸣潮。'
    }
    $gameDir = Split-Path -Parent $gameExe
    $runExe = Join-Path $runtimeDir 'Run.exe'
    $iniPath = Join-Path $runtimeDir 'd3dx.ini'
    $sourceDll = Join-Path $gameDir 'dxgi.dll'
    $preloadDll = Join-Path $gameDir 'ReShade64.dll'
    foreach ($path in @($gameExe, $runExe, $iniPath, $sourceDll,
            (Join-Path $gameDir 'dlss5-bridge.addon64'),
            (Join-Path $gameDir 'renodx-dlss5.addon64'))) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "所需文件不存在：$path`r`n请检查鸣潮、WWMI 和 RHI 的安装。"
        }
    }

    $helper = Join-Path $PSScriptRoot 'TestEnvironment.ps1'
    if (-not (Test-Path -LiteralPath $helper)) {
        $helper = Join-Path (Split-Path -Parent $PSScriptRoot) 'runtime\TestEnvironment.ps1'
    }
    if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) {
        throw '启动脚本不完整，请重新部署启动脚本。'
    }
    . $helper

    if ($ValidateOnly) {
        [pscustomobject]@{ GameExe = $gameExe; RuntimeDir = $runtimeDir; ReShade = $sourceDll; Preload = $preloadDll }
        exit 0
    }

    if (Get-Process -Name 'Client-Win64-Shipping' -ErrorAction SilentlyContinue) {
        Show-LaunchMessage '鸣潮已经在运行。请先退出游戏，再双击此快捷方式。'
        exit 0
    }

    # 提权整个启动脚本，才能隐藏注入器窗口并保存启动失败日志。
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]$identity
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        $arguments = '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + $PSCommandPath +
            '" -GameConfigPath "' + $GameConfigPath + '"'
        Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -Verb RunAs -WindowStyle Hidden -ArgumentList $arguments
        exit 0
    }

    # RHI 更新原 dxgi.dll 后，同步副本，保留直接启动游戏的原有入口。
    if (-not (Test-Path -LiteralPath $preloadDll) -or
        (Get-FileHash -LiteralPath $sourceDll).Hash -ne (Get-FileHash -LiteralPath $preloadDll).Hash) {
        Copy-Item -LiteralPath $sourceDll -Destination $preloadDll -Force
    }

    $dlls = @($config.extraDlls | Where-Object { $_ -is [string] -and $_.Trim() } | ForEach-Object { $_.Trim() })
    if ($dlls.Count -eq 0 -and $config.extraDll) {
        $dlls = @($config.extraDll.Trim())
    }
    if ($dlls -notcontains $preloadDll) {
        $dlls += $preloadDll
    }
    if (-not (Test-Path -LiteralPath "$iniPath.before-dlss5-launch.bak")) {
        Copy-Item -LiteralPath $iniPath -Destination "$iniPath.before-dlss5-launch.bak"
    }
    Set-TestLoaderSetting $iniPath 'target' $gameExe
    Set-TestLoaderSetting $iniPath 'launch' $gameExe
    Set-TestLoaderSetting $iniPath 'launch_args' '-dx11 -krqlv=hd'
    Set-TestLoaderSetting $iniPath 'inject_dlls' ($dlls -join '|')

    $logDir = Join-Path $env:LOCALAPPDATA 'SSMT4LaunchLogs'
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $outputLog = Join-Path $logDir "WWMI-$stamp.jsonl"
    $errorLog = Join-Path $logDir "WWMI-$stamp.log"
    $process = Start-Process -FilePath $runExe -WorkingDirectory $runtimeDir -WindowStyle Hidden `
        -ArgumentList '--machine-readable' -RedirectStandardOutput $outputLog -RedirectStandardError $errorLog -Wait -PassThru
    $events = Get-Content -Raw -LiteralPath $outputLog -Encoding UTF8
    if ($process.ExitCode -ne 0 -or $events -notmatch '"event"\s*:\s*"launch_complete"') {
        throw "启动失败。请把下面文件交给维护者：`r`n$outputLog`r`n$errorLog"
    }
} catch {
    Show-LaunchMessage $_.Exception.Message -IsError
    exit 1
}
