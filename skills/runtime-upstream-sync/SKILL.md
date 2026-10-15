---
name: runtime-upstream-sync
description: 同步 SSMT 的 3DMigoto Runtime 与 XXMI-Libs-Package 上游，处理合并和编译兼容问题，验证桥接及 DXGI 共存并更新子模块指针。
---

# Runtime 上游同步

先读 [Runtime 同步说明](../../docs/runtime-upstream-sync.md)，其中记录仓库联接、SSMT 增量和现有构建入口。

- 用 `git -C runtime rev-parse --show-toplevel` 确认实际仓库。主仓库可能把目录联接显示为未初始化子模块，不要因此替换现有目录。
- `origin` 是 SSMT，`upstream` 是 XXMI。获取远端后检查完整更新范围与回退提交，合并上游分支最新状态，保留 SSMT 历史。
- 无文本冲突时也检查 `SSMTBridge` 的编译入口和调用点、ReShade/DXGI 共存、自定义资源路径，以及构建后不复制到游戏目录的本地改动。
- MSVC 按系统代码页读取无 BOM UTF-8 源码时，中文注释也可能引发语法错误。保留 `HackerDXGI.cpp` 的 BOM，以最小编码修正解决实际编译问题。
- 编译验证使用 `runtime/release.ps1` 并显式指定部署目录，或使用 `scripts/stage-ssmt-runtime.ps1 -Build` 暂存应用资源；不要用会启动游戏的调试脚本代替编译。
- DXGI 测试先换成本次编译的 DLL，再依次启动 `plain`、`mod`、`reshade`、`both` 四个独立进程；夹具通过与真实游戏验证分别记录。
- 完成授权的拉取合并后，先记录 Runtime 合并提交，再更新主仓库指针。不要仅提交主仓库指针而留下未提交的 Runtime 修复；推送按用户授权另行处理。
