---
name: release-build-diagnostics
description: 区分 SSMT 主应用本机打包和 Native、Runtime 云端构建失败，按日志定位并执行不打包的局部验证。
---

# Release 构建排查

先读 [发布构建排查](../../docs/release-build-diagnostics.md)。

- 先确认失败的仓库、时间和步骤。Native / Runtime 最新 Actions 成功不能排除主应用本机打包失败。
- 不运行主应用发布包脚本；使用组件构建、配置解析、语法检查等最小验证。
- `tauri.conf.json > version` 错误先核对 SemVer。主应用发布使用三段正式版本；不能套用 DLL 的四段 Windows 文件版本。
- 用 `tauri inspect wix-upgrade-code` 只读复现配置解析。配置能通过不等于完整构建成功。
- 版本预检要早于签名材料读取、组件编译及资源删除；显式版本覆盖也须能修复非法旧值。
- 保留用户未提交的版本选择，不擅自发布、推送或修改远端标签。
