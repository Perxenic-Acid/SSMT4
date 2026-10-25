# 在其他 Windows 机器上恢复 Mod 标签

把 `scripts/recovery/Recover-ModTags.ps1` 单独复制到目标机器即可。支持 Windows PowerShell 5.1 和 PowerShell 7，无需安装依赖。运行恢复前完全退出 SSMT。首次可用 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Recover-ModTags.ps1 ...`，该参数只作用于本次进程。

## 1. 定位标签库及备份

```powershell
.\Recover-ModTags.ps1 -ScanRoot 'D:\','E:\' -ReportDirectory '.\tag-inventory'
```

不传 ScanRoot 时扫描所有就绪的本地固定磁盘。输出 `preview.html` 和 `inventory.json`，列出候选目录、标签数、Mod/分组关联路径数、文件路径及哈希、读取错误。扫描默认不修改标签数据；报告目录会被创建或更新。

搜索 `tags.json`、`mod-mappings.json`、`group-mappings.json` 及其点号后缀副本，包含隐藏目录。不跟随目录链接，并跳过 `.git`、`node_modules`、回收站和系统保留目录；访问失败逐项记录。外接磁盘、网络盘或目录链接实际目标需通过 ScanRoot 显式指定真实路径。ZIP/7z 等压缩备份应先解压。备份目录中若存在多个同类后缀版本，请把同一时间点的选定文件复制到独立目录并改为标准文件名；脚本不会猜选某个时间点。孤立的 .tmp 会显示为候选，但不会自动选为备份。

## 2. 生成恢复预览

```powershell
.\Recover-ModTags.ps1 -TargetModsPath 'D:\GIMI\Mods' `
  -BackupPath 'E:\backup-1\Mods\.mod-tags','E:\backup-2\Mods\.mod-tags' `
  -ReportDirectory '.\tag-preview'
```

TargetModsPath 指向目标 **Mods 目录**；BackupPath 指向包含标签 JSON 的目录。多个游戏须分别预览和恢复，不能将不同游戏的库混合作为备份。

`preview.html` 包含：来源、目标、全部标签在恢复前后的 Mod/分组计数、逐项新增关联、缺失目标目录、定义冲突与缺失图标。`plan.json` 保存执行所需的完整合并结果及源文件哈希。

合并保留现有标签及关联，以标签 ID 为准去重。相同 ID 定义冲突优先保留当前定义，再按 BackupPath 顺序选取；不同 ID 的同名标签保留各自 ID。多个历史备份取并集，因此可能重新加入用户曾主动移除的标签，须检查预览。不重命名 Mod，不猜测启停或移动后的路径；未找到的路径仍保留关联并在预览标记。缺失图标按来源顺序补齐，现有图标不覆盖。

当前 JSON 损坏时默认停止。确认只能从备份重建后，可在生成预览时加 `-AllowDamagedCurrent`；此时不能保留损坏文件中未备份的关联，原文件仍在执行前备份。备份本身损坏、路径异常或引用缺失标签时停止，避免静默丢弃。

## 3. 按预览执行

```powershell
.\Recover-ModTags.ps1 -ApplyPlan '.\tag-preview\plan.json' -WhatIf
.\Recover-ModTags.ps1 -ApplyPlan '.\tag-preview\plan.json'
```

WhatIf 不备份、不写标签。实际执行会重新校验输入文件哈希，预览后有变化则拒绝执行。请勿编辑 plan.json；更改选项应重新生成预览。目标写入路径含目录链接或重解析点会拒绝执行，请使用真实目录。

恢复先将所有将被触及的现有文件复制到目标 Mods 下的 `.mod-tags-recovery-日期-随机值`，再逐文件写临时文件并替换；新文件使用移动。写入异常时尝试回滚本次触及文件，失败时输出原文件副本位置。`before.json` 记录哪些目标文件原先存在；原 JSON、图标副本按相对路径存放。需要手工撤销时先退出 SSMT，按 before.json 恢复原文件，移除记录为原先不存在的新增文件。不要删除整个 .mod-tags 目录。

替换是逐文件进行，不是三个 JSON 的跨文件事务；突然断电时请利用恢复前副本和 before.json 核对并恢复。报告包含本机路径及标签信息，应按个人数据保存。

## 限制与验证

旧版没有自动生成标签历史备份。此工具只能恢复磁盘上实际存在的副本，无法找回已被覆盖且无备份的数据，也不读取 Windows 回收站、卷影副本或未解压的压缩包。

自动测试：`powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Test-ModTagRecovery.ps1`。测试仅操作独立临时目录，覆盖定位、只读预览、HTML 转义、WhatIf、过期计划拒绝、合并、图标、恢复前副本、幂等、中途失败回滚、损坏保护及路径检查；兼容性已在 5.1 与 7 上验证。
