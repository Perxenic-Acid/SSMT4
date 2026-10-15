---
name: game-launch-injection-diagnostics
description: 排查 SSMT 的游戏启动、官方启动器上下文、手动等待与原生 DLL 注入差异，尤其是终末地更新后的 EFMI 与 XXMI 行为对照。不用于 Poser 物理或 Mod 资源制作。
---

# 游戏启动与注入诊断

涉及 EFMI 时先读 [终末地注入说明](../../docs/efmi-injection.md)，其中包含交付脚本、验证入口和游戏实测边界。

- 从 `src/common/LaunchGame.ts` 的队列、`MigotoManager.patchD3dxForLaunch` 生成的 INI，再追到 `native/3Dmigoto-Injector-V2`，核对实际使用的注入模式；界面写“手动”不意味着 INI 已清除 `launch`。
- 区分官方启动上下文与注入机制。EFMI 使用外部官方启动进程和直接注入；不能把 XXMI 的 `DIRECT` 当成 SSMT 的 CBT Hook，也不能从几项命令行参数推断父进程伪装足够。
- 对照 XXMI 时记录本地 commit，读具体游戏默认项和 `MigotoInjector` 调用链。不要推广成所有预设都该更换注入方式。
- 目标必须匹配完整 EXE 文件身份。系统 `d3d11.dll` 与自定义 Runtime 同名，模块验证也必须匹配实际文件；枚举被拒绝不能报告 `runtime_detected`。
- 外部创建的进程不归注入器所有。失败时不得结束游戏；远程线程超时后不能释放仍可能被读取的参数或结束远程线程。
- 保留额外 DLL 顺序与已启用 PluginHost 的启动入口；没有插件时不要求插件文件。真实插件组合与测试 Host 是不同证据。
- 用独立 `SSMT_INJECTOR_REQUIRE_ADMIN=OFF` 的夹具构建验证真实进程注入，正式目标继续使用管理员 manifest。脚本面向 Windows PowerShell 5 时保留 UTF-8 BOM，并显式按 UTF-8 读取 JSONL。
- 没有游戏时交付可执行的原生目标、日志脚本和测试步骤；区分旧客户端通过 CLI 选择新模式与新前端的集成路径。不要把夹具加载成功当作反作弊、图形初始化或 Mod 渲染已验收。
- 主仓库与 Native 子仓库分别检查 diff；仅在用户要求提交时，先提交 Native 源码，再提交主仓库及子模块指针。不运行主应用发布包构建脚本。

只将已证实、可复用的经验留在本 skill；单次 PID、个人游戏路径、哈希和测试输出放在诊断报告或忽略的构建目录。
