#requires -Version 5.1
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../recovery/Recover-ModTags.ps1'))
$root = Join-Path ([IO.Path]::GetTempPath()) ('ssmt-tag-test-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
# 仅在测试作用域模拟进程查询，允许用户实际运行 SSMT 时测试独立夹具。
function Get-Process { [CmdletBinding()] param([string[]]$Name) }

function Put($path, $data) {
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    [IO.File]::WriteAllText($path, (ConvertTo-Json -InputObject $data -Depth 30), $utf8)
}
function Check($condition, $message) { if (!$condition) { throw $message } }
function Fails($action, $pattern) {
    $caught = $false
    try { & $action } catch { $caught = $true; Check ($_.Exception.Message -match $pattern) "错误不匹配：$_" }
    Check $caught '预期操作失败，实际成功'
}
try {
    $mods = Join-Path $root 'game/Mods'
    $store = Join-Path $mods '.mod-tags'
    $backup = Join-Path $root 'backup/.mod-tags'
    $report = Join-Path $root 'report'
    $tag1 = @{ id = 'one'; name = '采集物'; color = '#123456'; createdAt = 1; updatedAt = 1 }
    $tag2 = @{ id = 'two'; name = '<NSFW>'; color = '#654321'; createdAt = 2; updatedAt = 2; iconFile = 'two.png' }
    Put (Join-Path $store 'tags.json') @($tag1)
    Put (Join-Path $store 'mod-mappings.json') @{ A = @('one') }
    Put (Join-Path $backup 'tags.json') @($tag1, $tag2)
    Put (Join-Path $backup 'mod-mappings.json') @{ A = @('two'); B = @('one', 'two') }
    Put (Join-Path $backup 'group-mappings.json') @{ Group = @('two') }
    [IO.Directory]::CreateDirectory((Join-Path $backup 'icons')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $backup 'icons/two.png'), 'icon fixture')
    [IO.Directory]::CreateDirectory((Join-Path $mods 'A')) | Out-Null
    $before = [IO.File]::ReadAllText((Join-Path $store 'mod-mappings.json'))
    & $script -ScanRoot $root -ReportDirectory (Join-Path $root 'inventory')
    $inventory = Get-Content -LiteralPath (Join-Path $root 'inventory/inventory.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Check (@($inventory.candidates).Count -eq 2) '定位未发现两个标签库'
    & $script -TargetModsPath $mods -BackupPath $backup -ReportDirectory $report
    Check ([IO.File]::ReadAllText((Join-Path $store 'mod-mappings.json')) -ceq $before) '预览修改了原文件'
    $planPath = Join-Path $report 'plan.json'
    $plan = Get-Content -LiteralPath $planPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Check (@($plan.changes).Count -eq 4) '新增关联数错误'
    Check ([IO.File]::ReadAllText((Join-Path $report 'preview.html')).Contains('&lt;NSFW&gt;')) 'HTML 未转义'
    & $script -ApplyPlan $planPath -WhatIf
    Check ([IO.File]::ReadAllText((Join-Path $store 'mod-mappings.json')) -ceq $before) 'WhatIf 修改了原文件'
    [IO.File]::AppendAllText((Join-Path $store 'mod-mappings.json'), ' ')
    Fails { & $script -ApplyPlan $planPath } '预览后数据已变化'
    [IO.File]::WriteAllText((Join-Path $store 'mod-mappings.json'), $before, $utf8)
    & $script -ApplyPlan $planPath -Confirm:$false
    $restored = Get-Content -LiteralPath (Join-Path $store 'mod-mappings.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Check (@($restored.A).Count -eq 2 -and @($restored.B).Count -eq 2) '未保留或恢复全部关联'
    Check (Test-Path -LiteralPath (Join-Path $store 'icons/two.png')) '未恢复图标'
    $rollbacks = @(Get-ChildItem -LiteralPath $mods -Directory -Force | Where-Object { $_.Name -like '.mod-tags-recovery-*' })
    Check ($rollbacks.Count -eq 1) '未创建恢复前副本'
    Check ([IO.File]::ReadAllText((Join-Path $rollbacks[0].FullName 'mod-mappings.json')) -ceq $before) '恢复前副本不正确'
    & $script -TargetModsPath $mods -BackupPath $backup -ReportDirectory (Join-Path $root 'second')
    $again = Get-Content -LiteralPath (Join-Path $root 'second/plan.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Check (@($again.changes).Count -eq 0) '重复恢复产生重复关联'
    $stableTags = [IO.File]::ReadAllText((Join-Path $store 'tags.json'))
    $locked = Join-Path $store 'mod-mappings.json'
    $stableMappings = [IO.File]::ReadAllText($locked)
    [IO.File]::SetAttributes($locked, [IO.FileAttributes]::ReadOnly)
    try { Fails { & $script -ApplyPlan (Join-Path $root 'second/plan.json') -Confirm:$false } 'Replace|替换|拒绝|denied|access' }
    finally { [IO.File]::SetAttributes($locked, [IO.FileAttributes]::Normal) }
    Check ([IO.File]::ReadAllText((Join-Path $store 'tags.json')) -ceq $stableTags) '失败后未回滚第一份 JSON'
    Check ([IO.File]::ReadAllText($locked) -ceq $stableMappings) '失败后破坏了原映射'
    [IO.File]::WriteAllText((Join-Path $store 'tags.json'), '{', $utf8)
    Fails { & $script -TargetModsPath $mods -BackupPath $backup -ReportDirectory (Join-Path $root 'damaged') } '当前数据损坏'
    & $script -TargetModsPath $mods -BackupPath $backup -AllowDamagedCurrent -ReportDirectory (Join-Path $root 'damaged')
    Put (Join-Path $backup 'mod-mappings.json') @{ '../escape' = @('one') }
    Fails { & $script -TargetModsPath $mods -BackupPath $backup -AllowDamagedCurrent -ReportDirectory (Join-Path $root 'unsafe') } '无效的标签相对路径'
    Write-Host '通过：定位、只读预览、HTML 转义、WhatIf、过期计划拒绝、合并恢复、图标恢复、原文件备份、幂等、中途失败回滚、损坏保护、路径检查。'
} finally {
    # 仅删除本测试创建且已核对边界的独立临时目录。
    $resolved = [IO.Path]::GetFullPath($root)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved).StartsWith('ssmt-tag-test-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
