param(
    [Parameter(Mandatory)][string]$MigotoDirectory,
    [string]$InjectorPath,
    [ValidateRange(1, 3600)][int]$TimeoutSeconds = 120,
    [string]$PluginHostConfig,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$MigotoDirectory = (Resolve-Path -LiteralPath $MigotoDirectory).Path
if (-not (Test-Path -LiteralPath (Join-Path $MigotoDirectory 'd3dx.ini') -PathType Leaf)) {
    throw '请选择已经配置终末地目标路径的 SSMT 3Dmigoto 目录，其中应有 d3dx.ini。'
}
if (-not $InjectorPath) {
    $InjectorPath = Join-Path $PSScriptRoot 'Run.exe'
    if (-not (Test-Path -LiteralPath $InjectorPath)) {
        $InjectorPath = Join-Path $PSScriptRoot '..\..\src-tauri\resources\Run.exe'
    }
}
$InjectorPath = (Resolve-Path -LiteralPath $InjectorPath).Path
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path ([IO.Path]::GetTempPath()) ('SSMT-EFMI-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $PID)
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$eventsPath = Join-Path $OutputDirectory 'events.jsonl'
$logPath = Join-Path $OutputDirectory 'injector.log'
$arguments = "--machine-readable --inject-mode direct --wait-for-target --process-timeout $TimeoutSeconds"
if ($PluginHostConfig) {
    $PluginHostConfig = (Resolve-Path -LiteralPath $PluginHostConfig).Path
    $arguments += " --plugin-host-config `"$PluginHostConfig`""
}
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
$report = [ordered]@{
    startedAt = (Get-Date).ToString('o')
    elevated = $elevated
    injector = $InjectorPath
    injectorSha256 = (Get-FileHash -LiteralPath $InjectorPath -Algorithm SHA256).Hash
    migotoDirectory = $MigotoDirectory
    arguments = $arguments
    processes = @()
}
$seen = [Collections.Generic.HashSet[uint32]]::new()
$process = $null
Write-Host "日志目录：$OutputDirectory"
Write-Host "正在等待终末地（$TimeoutSeconds 秒）。请在官方启动器启用 DirectX 11，然后启动游戏。"
Write-Host '请关闭其他 XXMI/SSMT 注入器；本脚本不修改启动参数、父进程或 d3dx.ini。'
try {
    # 正式 Run.exe 需要管理员权限；日志重定向要求从管理员 PowerShell 运行本脚本。
    $process = Start-Process -FilePath $InjectorPath -ArgumentList $arguments -WorkingDirectory $MigotoDirectory `
        -WindowStyle Hidden -RedirectStandardOutput $eventsPath -RedirectStandardError $logPath -PassThru
    $null = $process.Handle
    do {
        try {
            foreach ($game in @(Get-CimInstance Win32_Process -Filter "Name='Endfield.exe'")) {
                if ($seen.Add([uint32]$game.ProcessId)) {
                    $parent = Get-CimInstance Win32_Process -Filter "ProcessId=$($game.ParentProcessId)"
                    $report.processes += [ordered]@{
                        pid = $game.ProcessId
                        executable = $game.ExecutablePath
                        commandLine = $game.CommandLine
                        parentPid = $game.ParentProcessId
                        parentExecutable = $parent.ExecutablePath
                        parentCommandLine = $parent.CommandLine
                    }
                }
            }
        } catch {
            # WMI 采集失败不能影响已运行的注入器。
            $report.processQueryError = $_.Exception.Message
        }
    } while (-not $process.WaitForExit(500))
    $process.WaitForExit()
    $report.exitCode = $process.ExitCode
    $events = @(Get-Content -LiteralPath $eventsPath -Encoding UTF8 | ConvertFrom-Json)
    $report.runtimeDetected = $events.event -contains 'runtime_detected'
    $report.launchComplete = $events.event -contains 'launch_complete'
    $report.errors = @($events | Where-Object event -eq 'launch_error')
    Get-Content -LiteralPath $logPath | Write-Host
    if ($report.exitCode -ne 0 -or -not $report.launchComplete) {
        throw "注入未完成。请回传日志目录：$OutputDirectory"
    }
    Write-Host '已验证指定运行时 DLL 被加载。还需在游戏中确认 Mod / Hunting 是否生效。'
} catch {
    $report.error = $_.Exception.Message
    if (-not $elevated -and -not $process) {
        Write-Host '正式注入器需要管理员权限；请用管理员 PowerShell 运行同一命令。'
    }
    throw
} finally {
    $report.finishedAt = (Get-Date).ToString('o')
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'report.json') -Encoding UTF8
    if ($process) {
        # 中止采集时只结束本脚本启动的注入器，绝不结束游戏。
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
    Write-Host "保留日志：$OutputDirectory"
}
