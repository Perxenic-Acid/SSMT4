---
name: mod-tag-persistence
description: 维护 SSMT Mod 标签存储、筛选与目录迁移，排查标签丢失或并发覆盖时使用。
---

阅读 [持久化约束](../../docs/mod-tag-persistence.md)，结合 src/store/ModTagStore.ts 与 src/views/ModsManagement/ModsManagement.tags.ts 检查实际调用链。

- 扫描及 SQLite 查询结果不能作为删除标签映射的依据；保留未扫描到的路径。
- 修改必须将读取、修改、保存整体纳入同一游戏队列；内部调用不重复排队。
- 读取失败和坏 JSON 必须阻止保存；不存在的文件才允许使用初始值。
- 图标回收必须依据队列内新读取的定义，避免旧快照删除新图标。
- 使用 scripts/tests/test-mod-tags.mjs 验证实际 Store 行为；涉及持久化变动时补充失败与并发场景，并运行 Vue 类型检查。测试中的模拟文件 API 不能替代 Windows 应用内验证。

恢复用户已有标签时，使用 scripts/recovery/Recover-ModTags.ps1，先读取 [恢复流程](../../docs/mod-tag-recovery.md)。只对用户选定的同一游戏备份取并集；先生成预览，再按计划恢复，不能自动混合所有发现的库。验证脚本用 scripts/tests/Test-ModTagRecovery.ps1；实际替换文件需要普通 Windows 文件系统权限，沙箱可能禁止替换。
