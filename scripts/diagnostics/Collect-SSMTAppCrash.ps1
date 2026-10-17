#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateRange(1, 720)][int]$Hours = 72,
    [string]$AppPath,
    [string]$OutputDirectory = [Environment]::GetFolderPath('Desktop')
)

function ConvertFrom-SSMTCrashEventXml {
    param([Parameter(Mandatory)][string]$Xml)
    $document = [xml]$Xml
    # 不用 PowerShell 的 XML 属性适配器：无属性的 Data 会被转成 string，丢失空项和节点方法。
    $system = $document.DocumentElement.SelectSingleNode("*[local-name()='System']")
    $eventId = [int]$system.SelectSingleNode("*[local-name()='EventID']").InnerText
    $dataNodes = @($document.DocumentElement.SelectNodes("*[local-name()='EventData']/*[local-name()='Data']"))
    $values = @($dataNodes | ForEach-Object { $_.InnerText })
    $fields = [ordered]@{}
    $index = 0
    foreach ($node in $dataNodes) {
        $name = $node.GetAttribute('Name')
        if (-not $name) { $name = "[$index]" }
        $fields[$name] = $node.InnerText
        $index++
    }
    $application = $null
    switch ($eventId) {
        1000 { $application = if ($fields.Contains('AppName')) { $fields.AppName } else { $values[0] } }
        1001 { $application = if ($fields.Contains('P1')) { $fields.P1 } elseif ($values.Count -gt 5) { $values[5] } }
        1005 { if ($values.Count -gt 1) { $application = $values[1] } }
    }
    $application = [string]$application
    if ($application -notmatch '^ssmt4(?:\.exe)?$') { return }
    $result = [ordered]@{
        eventId = $eventId
        recordId = $system.SelectSingleNode("*[local-name()='EventRecordID']").InnerText
        timeUtc = $system.SelectSingleNode("*[local-name()='TimeCreated']").GetAttribute('SystemTime')
        application = $application
        module = $null
        exceptionCode = $null
        faultOffset = $null
        appPath = $null
        data = $fields
    }
    # 1005 的第三项是文件错误值，不能把它冒充为 1000 的异常代码。
    if ($eventId -eq 1000) {
        foreach ($mapping in @(
            @('module', 'ModuleName', 3), @('exceptionCode', 'ExceptionCode', 6),
            @('faultOffset', 'FaultingOffset', 7), @('appPath', 'AppPath', 10)
        )) {
            $result[$mapping[0]] = if ($fields.Contains($mapping[1])) { $fields[$mapping[1]] }
                elseif ($values.Count -gt $mapping[2]) { $values[$mapping[2]] } else { $null }
        }
    }
    [pscustomobject]$result
}

function Test-SSMTWerReport {
    param([string]$Text)
    return $Text -match '(?im)^AppName=ssmt4(?:\.exe)?\s*$' -or
        $Text -match '(?im)^AppPath=.*[\\/]ssmt4\.exe\s*$' -or
        $Text -match '(?im)^Sig\[0\]\.Value=ssmt4\.exe\s*$'
}

# 点源只加载解析函数，供离线夹具验证；直接运行才读取本机日志。
if ($MyInvocation.InvocationName -eq '.') { return }
$ErrorActionPreference = 'Stop'
$since = (Get-Date).AddHours(-$Hours)
$reportDirectory = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) (
    'SSMT-AppCrash-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
[IO.Directory]::CreateDirectory($reportDirectory) | Out-Null
$errors = [Collections.Generic.List[string]]::new()
$events = [Collections.Generic.List[object]]::new()
$xmlEvents = [Collections.Generic.List[string]]::new()
try {
    Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1000, 1001, 1005; StartTime = $since } `
        -MaxEvents 4000 -ErrorAction Stop | ForEach-Object {
        $xml = $_.ToXml()
        try {
            $event = ConvertFrom-SSMTCrashEventXml $xml
            if ($event) { $events.Add($event); $xmlEvents.Add($xml) }
        } catch { $errors.Add("事件解析失败：$($_.Exception.Message)") }
    }
} catch { $errors.Add("事件读取：$($_.Exception.Message)") }
ConvertTo-Json -InputObject @($events.ToArray()) -Depth 8 |
    Set-Content -LiteralPath (Join-Path $reportDirectory 'events.json') -Encoding UTF8
('<Events>' + ($xmlEvents -join "`r`n") + '</Events>') |
    Set-Content -LiteralPath (Join-Path $reportDirectory 'events.xml') -Encoding UTF8

$werFiles = [Collections.Generic.List[string]]::new()
$werRoots = @(
    (Join-Path $env:ProgramData 'Microsoft\Windows\WER\ReportArchive'),
    (Join-Path $env:ProgramData 'Microsoft\Windows\WER\ReportQueue'),
    (Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\WER\ReportArchive'),
    (Join-Path $env:LOCALAPPDATA 'Microsoft\Windows\WER\ReportQueue')
)
foreach ($root in $werRoots) {
    if (-not (Test-Path -LiteralPath $root)) { continue }
    try {
        $directories = @(Get-ChildItem -LiteralPath $root -Directory -ErrorAction Stop |
            Where-Object LastWriteTime -ge $since | Sort-Object LastWriteTime -Descending | Select-Object -First 200)
        foreach ($directory in $directories) {
            $path = Join-Path $directory.FullName 'Report.wer'
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
            try {
                if ((Get-Item -LiteralPath $path).Length -gt 1MB) { continue }
                # ReadAllText 自动识别 WER 常用的 UTF-16 BOM。
                $text = [IO.File]::ReadAllText($path)
                if (-not (Test-SSMTWerReport $text)) { continue }
                $name = 'wer-{0:D3}.txt' -f $werFiles.Count
                $text | Set-Content -LiteralPath (Join-Path $reportDirectory $name) -Encoding UTF8
                $werFiles.Add($path)
            } catch { $errors.Add("WER 文件读取：$path；$($_.Exception.Message)") }
        }
    } catch { $errors.Add("WER 目录读取：$root；$($_.Exception.Message)") }
}

$cpu = @()
$os = $null
try { $cpu = @(Get-CimInstance Win32_Processor | Select-Object Name, Manufacturer, Architecture) }
catch { $errors.Add("CPU 查询：$($_.Exception.Message)") }
try { $os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture }
catch { $errors.Add("系统查询：$($_.Exception.Message)") }

if (-not $AppPath) {
    $AppPath = $events | Where-Object appPath | Sort-Object timeUtc -Descending |
        Select-Object -First 1 -ExpandProperty appPath
    if (-not $AppPath) { $AppPath = Join-Path $env:LOCALAPPDATA 'ssmt4\ssmt4.exe' }
}
$files = [Collections.Generic.List[object]]::new()
try {
    $AppPath = [IO.Path]::GetFullPath($AppPath)
    $paths = @($AppPath)
    $packagesDirectory = Join-Path (Split-Path -Parent $AppPath) 'resources\PluginPackages'
    if (Test-Path -LiteralPath $packagesDirectory -PathType Container) {
        $paths += @(Get-ChildItem -LiteralPath $packagesDirectory -File -Filter '*.ssmtpkg' |
            Select-Object -First 64 -ExpandProperty FullName)
    }
    foreach ($path in $paths) {
        try {
            $file = Get-Item -LiteralPath $path -ErrorAction Stop
            $files.Add([pscustomobject]@{
                path = $path; size = $file.Length; modifiedUtc = $file.LastWriteTimeUtc.ToString('o')
                version = $file.VersionInfo.FileVersion
                sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
            })
        } catch { $errors.Add("安装文件读取：$path；$($_.Exception.Message)") }
    }
} catch { $errors.Add("安装路径查询：$($_.Exception.Message)") }
$summary = [ordered]@{
    collectedAt = (Get-Date).ToString('o'); since = $since.ToString('o')
    cpu = $cpu; os = $os; appPath = $AppPath; files = @($files.ToArray())
    eventCount = $events.Count; event1000Count = @($events | Where-Object eventId -eq 1000).Count
    werReports = @($werFiles.ToArray()); errors = @($errors.ToArray())
    limits = '最近 4000 条相关类型事件；每个 WER 目录最近 200 份报告；最多 64 个随包插件。'
}
$summary | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath (Join-Path $reportDirectory 'summary.json') -Encoding UTF8
$zipPath = $reportDirectory + '.zip'
Compress-Archive -LiteralPath $reportDirectory -DestinationPath $zipPath
Write-Host "事件 1000：$($summary.event1000Count) 条；SSMT 相关事件：$($events.Count) 条；WER：$($werFiles.Count) 份。"
Write-Host "诊断包：$zipPath"
if ($errors.Count) { Write-Host '部分信息未能读取，原因已记入 summary.json；其余结果已保留。' }
