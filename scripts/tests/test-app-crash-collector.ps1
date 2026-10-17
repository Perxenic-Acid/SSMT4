#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../diagnostics/Collect-SSMTAppCrash.ps1')

function Assert-Equal {
    param($Actual, $Expected, [string]$Message)
    if ($Actual -cne $Expected) { throw "$Message；预期 '$Expected'，实际 '$Actual'" }
}

function New-EventFixture {
    param([int]$Id, [string]$Data, [string]$Attributes = '')
    return @"
<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
  <System>
    <Provider Name="Application Error" />
    <EventID $Attributes>$Id</EventID>
    <TimeCreated SystemTime="2026-10-17T06:10:32.1557844Z" />
    <EventRecordID>278936</EventRecordID>
  </System>
  <EventData>$Data</EventData>
</Event>
"@
}

# 用户提供的 1005：首项为空，第三项为 0，不能据此生成模块或异常代码。
$event1005 = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1005 @'
<Data /><Data>ssmt4</Data><Data>00000000</Data><Data>0</Data>
'@ 'Qualifiers="49152"')
Assert-Equal $event1005.eventId 1005 '1005 事件类型'
Assert-Equal $event1005.application 'ssmt4' '保留空项后的应用位置'
Assert-Equal $event1005.data['[0]'] '' '保留空文件名'
Assert-Equal $event1005.data['[2]'] '00000000' '保留文件错误值'
Assert-Equal $event1005.module $null '1005 不生成故障模块'
Assert-Equal $event1005.exceptionCode $null '1005 不生成异常代码'
Assert-Equal $event1005.recordId '278936' '保留记录编号'
Assert-Equal $event1005.timeUtc '2026-10-17T06:10:32.1557844Z' '保留 UTC 时间'

# EventID 无属性、有命名字段与旧版仅位置字段都必须可解析。
$event1000 = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1000 @'
<Data Name="AppName">SSMT4.exe</Data>
<Data Name="ModuleName">ssmt4.exe</Data>
<Data Name="ExceptionCode">c000001d</Data>
<Data Name="FaultingOffset">0000000000123456</Data>
<Data Name="AppPath">C:\测试 [目录]\ssmt4.exe</Data>
'@)
Assert-Equal $event1000.eventId 1000 '无属性的事件类型'
Assert-Equal $event1000.module 'ssmt4.exe' '读取命名故障模块'
Assert-Equal $event1000.exceptionCode 'c000001d' '读取命名异常代码'
Assert-Equal $event1000.faultOffset '0000000000123456' '读取命名偏移'
Assert-Equal $event1000.appPath 'C:\测试 [目录]\ssmt4.exe' '读取中文安装路径'

$legacy = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1000 @'
<Data>ssmt4.exe</Data><Data>4.2.0</Data><Data>0</Data>
<Data>ntdll.dll</Data><Data>10.0</Data><Data>0</Data>
<Data>c0000006</Data><Data>000000000000abcd</Data><Data>1234</Data><Data>0</Data>
<Data>C:\SSMT\ssmt4.exe</Data>
'@)
Assert-Equal $legacy.module 'ntdll.dll' '读取旧版位置模块'
Assert-Equal $legacy.exceptionCode 'c0000006' '读取旧版位置异常代码'
Assert-Equal $legacy.faultOffset '000000000000abcd' '读取旧版位置偏移'
Assert-Equal $legacy.appPath 'C:\SSMT\ssmt4.exe' '读取旧版位置路径'

$event1001 = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1001 @'
<Data Name="EventName">APPCRASH</Data><Data Name="P1">ssmt4.exe</Data>
<Data Name="P7">c000001d</Data>
'@)
Assert-Equal $event1001.eventId 1001 '保留 WER 事件'
Assert-Equal $event1001.data.P7 'c000001d' '保留原始 WER 签名'
Assert-Equal $event1001.exceptionCode $null 'WER 签名不冒充事件 1000 字段'
$legacyWer = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1001 @'
<Data>0</Data><Data>1</Data><Data>APPCRASH</Data><Data>0</Data><Data>0</Data><Data>ssmt4.exe</Data>
'@)
Assert-Equal $legacyWer.application 'ssmt4.exe' '读取旧版 WER 应用名'

$otherApp = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1000 @'
<Data Name="AppName">other.exe</Data><Data Name="ModuleName">ssmt4.exe</Data>
'@)
Assert-Equal $otherApp $null '不采集仅模块名提到 SSMT 的其他应用'
$emptyData = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1005 '<Data />')
Assert-Equal $emptyData $null '忽略无应用字段的事件'
$unrelated1001 = ConvertFrom-SSMTCrashEventXml (New-EventFixture 1001 '<Data>Unrelated service stopped.</Data>')
Assert-Equal $unrelated1001 $null '事件 ID 相同但无应用字段时不能收录'
$malformedRejected = $false
try { ConvertFrom-SSMTCrashEventXml '<Event>' | Out-Null } catch { $malformedRejected = $true }
Assert-Equal $malformedRejected $true '损坏 XML 交由采集调用方记录错误'

Assert-Equal (Test-SSMTWerReport "AppName=SSMT4.exe`r`n") $true 'WER 应用名大小写兼容'
Assert-Equal (Test-SSMTWerReport 'AppPath=C:\测试 [目录]\ssmt4.exe') $true 'WER 安装路径'
Assert-Equal (Test-SSMTWerReport 'Sig[0].Value=ssmt4.exe') $true 'WER 首签名'
Assert-Equal (Test-SSMTWerReport "AppName=other.exe`r`nLoadedModule[4]=C:\SSMT\ssmt4.exe") $false '不采集其他应用报告'
Assert-Equal (Test-SSMTWerReport 'AppName=ssmt4.exe.old') $false '应用名完整匹配'

Write-Host '应用闪退采集离线回归通过（1000/1001/1005、空项、命名/位置字段、WER 过滤）。'
