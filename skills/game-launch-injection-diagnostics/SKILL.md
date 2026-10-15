---
name: game-launch-injection-diagnostics
description: 排查 SSMT 的游戏启动、官方启动器上下文、手动等待与原生 DLL 注入差异，尤其是终末地更新后的 EFMI 与 XXMI 行为对照。不用于 Poser 物理或 Mod 资源制作。
---

# 游戏启动与注入诊断

涉及 EFMI 时先读 [终末地注入说明](../../docs/efmi-injection.md)，其中包含交付脚本、验证入口和游戏实测边界。

涉及原神 DLSS5 时先读 [热注入说明](../../docs/gimi-hot-runtime.md)，核对组件 Profile、单 Host 启动计划和日志验收。

- 从 `src/common/LaunchGame.ts` 的队列、`MigotoManager.patchD3dxForLaunch` 生成的 INI，再追到 `native/3Dmigoto-Injector-V2`，核对实际使用的注入模式；界面写“手动”不意味着 INI 已清除 `launch`。
- 区分官方启动上下文与注入机制。EFMI 使用外部官方启动进程和直接注入；不能把 XXMI 的 `DIRECT` 当成 SSMT 的 CBT Hook，也不能从几项命令行参数推断父进程伪装足够。
- 对照 XXMI 时记录本地 commit，读具体游戏默认项和 `MigotoInjector` 调用链。不要推广成所有预设都该更换注入方式。
- 原神模型/贴图异常先区分同一 INI 重复载入、不同文件内容冲突和实际缓冲绑定；重复节告警不能证明重复载入。检查动态角色分辨率的两份注册表画质记录，GIMI 的“配置游戏设置”需与 XXMI 一样关闭该项。未知结构不猜测，写入前保留原始记录；纯净启动不得修改。
- 原神 DLSS5 只允许热注入/Hook；把 DLL 或链接临时放回游戏目录再恢复仍属于冷替换。启动成功、仅有 ReShade 或 Feature 1 创建均不等于 NR 生效，应核对同次日志中的 Feature 18 成功帧及 Mod 画面。
- 热注入所需的环境必须在提权后的 Run.exe 内显式设置，再由挂起创建的目标继承；不能依赖 SSMT 环境跨 ShellExecute 提权继承。先加载 SSMT Runtime，再按计划加载 ReShade Host、Bridge、OptiScaler、额外 DLL 和可选 PluginHost，最后恢复目标线程。只为已安装且按游戏启用的插件准备资源，禁止再启动第二个 ReShade 注入器。
- 复制组件前核对固定版本及显卡 Profile 的完整哈希，使用 SSMT 管理的副本；绝不为了复用组件包而替换其 GIMI DLL。组合配置中第三方 FX 的相对路径应基于实际资源所有者转为绝对路径，Windows canonicalize 的 verbatim 前缀不能直接与第三方正斜杠路径拼接。保留用户禁用的非必要 Add-on，插件关闭后清除它的本次贡献。
- 目标必须匹配完整 EXE 文件身份。系统 `d3d11.dll` 与自定义 Runtime 同名，模块验证也必须匹配实际文件；枚举被拒绝不能报告 `runtime_detected`。
- 外部创建的进程不归注入器所有。失败时不得结束游戏；远程线程超时后不能释放仍可能被读取的参数或结束远程线程。
- 保留额外 DLL 顺序与已启用 PluginHost 的启动入口；没有插件时不要求插件文件。真实插件组合与测试 Host 是不同证据。
- 运行时部署从应用资源取得 SSMT Runtime，设置页有意保留 XXMI 上游只读 Changelog，不重复撰写上游说明；阅读 Changelog 不得触发上游 DLL 安装。更新失败用非模态横幅并保留已有文件；首次安装无旧文件才停止。复制校验与 UPX 都在同目录暂存文件完成，最后一次 rename 替换，不能预先删除旧文件或随后再覆盖注入器。回归入口为 `scripts/tests/test-runtime-sync.mjs`；纯净启动跳过这些部署。
- 用独立 `SSMT_INJECTOR_REQUIRE_ADMIN=OFF` 的夹具构建验证真实进程注入，正式目标继续使用管理员 manifest。脚本面向 Windows PowerShell 5 时保留 UTF-8 BOM，并显式按 UTF-8 读取 JSONL。
- 没有游戏时交付可执行的原生目标、日志脚本和测试步骤；区分旧客户端通过 CLI 选择新模式与新前端的集成路径。不要把夹具加载成功当作反作弊、图形初始化或 Mod 渲染已验收。
- 主仓库与 Native 子仓库分别检查 diff；仅在用户要求提交时，先提交 Native 源码，再提交主仓库及子模块指针。不运行主应用发布包构建脚本。

只将已证实、可复用的经验留在本 skill；单次 PID、个人游戏路径、哈希和测试输出放在诊断报告或忽略的构建目录。
