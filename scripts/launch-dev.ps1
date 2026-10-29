$ErrorActionPreference = 'Stop'

# 项目根目录由脚本位置推导，避免绑定开发者的本机路径。
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$DevUrl = if ($env:SSMT4_DEV_URL) { $env:SSMT4_DEV_URL } else { 'http://localhost:1422/' }
$DebugExe = Join-Path $ProjectRoot 'src-tauri\target\debug\ssmt4.exe'
$LogDir = Join-Path $ProjectRoot 'logs'
$StdoutLog = Join-Path $LogDir 'launch-dev-vite.stdout.log'
$StderrLog = Join-Path $LogDir 'launch-dev-vite.stderr.log'

function Show-LaunchError([string]$Message) {
    $shell = New-Object -ComObject WScript.Shell
    $null = $shell.Popup($Message, 20, 'SSMT4 Dev Launcher', 16)
}

function Test-DevServer {
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri $DevUrl -TimeoutSec 2
        return $response.StatusCode -eq 200 -and
            $response.Content -match 'id=["'']app["'']' -and
            $response.Content -match '@vite/client'
    }
    catch {
        return $false
    }
}

try {
    if (-not (Test-Path -LiteralPath $DebugExe -PathType Leaf)) {
        throw "Development executable not found: $DebugExe"
    }

    $running = Get-Process -Name 'ssmt4' -ErrorAction SilentlyContinue
    if ($running) {
        $debugRunning = $running | Where-Object { $_.Path -ieq $DebugExe }
        if ($debugRunning) {
            exit 0
        }
        throw 'The release version is already running. Close SSMT4 before opening the development build.'
    }

    if (-not (Test-DevServer)) {
        $bunCommand = Get-Command bun -ErrorAction SilentlyContinue
        $nodeCommand = Get-Command node -ErrorAction SilentlyContinue
        $viteCli = Join-Path $ProjectRoot 'node_modules\vite\bin\vite.js'

        if ($env:SSMT4_BUN) {
            if (-not (Test-Path -LiteralPath $env:SSMT4_BUN -PathType Leaf)) {
                throw "SSMT4_BUN does not point to a file: $env:SSMT4_BUN"
            }
            $devCommand = $env:SSMT4_BUN
            $devArguments = @('run', 'dev')
        }
        elseif ($bunCommand) {
            $devCommand = $bunCommand.Source
            $devArguments = @('run', 'dev')
        }
        elseif ($nodeCommand -and (Test-Path -LiteralPath $viteCli -PathType Leaf)) {
            $devCommand = $nodeCommand.Source
            $devArguments = @($viteCli)
        }
        else {
            throw 'Neither Bun nor a local Vite installation was found.'
        }

        New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
        $null = Start-Process -FilePath $devCommand -ArgumentList $devArguments `
            -WorkingDirectory $ProjectRoot -WindowStyle Hidden `
            -RedirectStandardOutput $StdoutLog -RedirectStandardError $StderrLog

        $deadline = (Get-Date).AddSeconds(30)
        while ((Get-Date) -lt $deadline -and -not (Test-DevServer)) {
            Start-Sleep -Milliseconds 400
        }
    }

    if (-not (Test-DevServer)) {
        throw "The Vite development server did not become ready. See $StdoutLog and $StderrLog."
    }

    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333'
    $null = Start-Process -FilePath $DebugExe -WorkingDirectory $ProjectRoot
}
catch {
    Show-LaunchError $_.Exception.Message
    exit 1
}
