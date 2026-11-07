> [!CAUTION]
> **⚠️ 安全性警告**
>
> SSMT4带有注入行为的Run.exe会100%触发杀软误报（虽然并未存在加壳行为，但是百分百会被误报，请自行注意评估使用风险，本人只能以微薄的开发者身份提供最基础的安全性承诺,由于其更新机制依赖于Github Release，唯一的安全性保证依赖于.sig签名文件，SSMT4-Alpha不会主动在 [Perxenic-Acid/SSMT4](https://github.com/Perxenic-Acid/SSMT4) 以外的地方发布，如果在其它平台看到了SSMT4-Alpha的二进制发布版本而不是直接提供源码或仓库地址，请注意辨别，并且请仔细将其与本仓库对应版本Release中.sig文件对比，以防下载到重打包的危险二次发布版本。


# SSMT4  ![GitHub Downloads (all assets, latest release)](https://img.shields.io/github/downloads/Perxenic-Acid/SSMT4/latest/total?color=blue&label=LatestDownloads)


- 配套Blender插件：https://github.com/Perxenic-Acid/TheHerta4
- 更多蓝图特性的Blender插件: https://github.com/xuhuan9102/TheHerta4
- SSMT4文档源代码：https://github.com/Perxenic-Acid/SSMT4-Documents
- SSMT4文档阅读地址：https://perxenic-acid.github.io/SSMT4-Documents/

## 终末地（EFMI）启动

先在官方启动器中启用 DirectX 11，再点击 SSMT 启动，并在 120 秒内从官方启动器开始游戏。
EFMI 会等待实际游戏进程后直接注入；Shell 模式可打开配置的官方启动器。
游戏目标路径仍需填写 `Endfield.exe`。详细步骤与诊断入口见 [终末地注入说明](docs/efmi-injection.md)。

## 本地 Native 插件开发

运行 `./build_native.ps1 -Configuration Release` 会构建 Native 工具和可选 Poser，
并把带清单的本地插件包放到 `src-tauri/resources/PluginPackages`。
可用 `-BuildPoserProbe $false` 跳过 Poser 构建。
插件市场刷新时自动发现这些包，标注“本地构建”；无需先发布到 GitHub Release。
点击“安装本地构建”检查来源和权限后安装，再按需启用。发现和安装不会自动启用插件。
本地构建包不视为已验证的 Native Release 来源；裸 DLL 和宿主测试 DLL 不直接成为市场条目。
前后端命令更新后，正在运行的 SSMT 开发进程需要重启才能使用新发现流程。

## 插件页面运行层

当前清单版本为 Schema 2，插件使用 `compatibility.pluginApi` 声明能力兼容范围，并通过
`contributions.pages` 提供声明式设置、静态页面或本地服务页面。Schema 1 与
`pluginApiVersion` 继续兼容读取。

Plugin API 1 已冻结；`1.x` 只增加兼容能力，不删除、重命名或改变已有语义。机器可读契约、
官方插件兼容矩阵和升级规则见 [Plugin API 1 冻结契约](docs/plugin-api-v1-contract.md)。
可使用 `bun run check:plugin-api-contract` 检查全部官方包和页面桥协议。

本地服务由 SSMT 分配随机端口和令牌，等待健康检查；页面仅持有 ServiceLease，最后一个
Lease 释放后进入空闲宽限，插件禁用、更新、卸载或 SSMT 退出时回收进程树。本地服务页
可通过版本化 iframe 桥请求 CapabilityBroker，调用身份由 Lease 解析，页面不能自报插件 ID。

安装、清单格式和安全边界见 [插件页面运行层](docs/plugin-pages.md)。
陌生插件开发、权限和错误规范见 [插件 SDK](docs/plugin-sdk.md)。

需要参与启动流程的插件可通过 `contributions.launchHooks` 声明受控文件动作和启动参数。
宿主负责资源冲突仲裁、SHA-256 验证、可恢复文件事务和游戏退出后的恢复；插件不能提交
任意绝对路径或任意 Shell 命令。协议与模拟验证见 [插件启动计划](docs/plugin-launch-plan.md)。

Liquid Gravity Probe 0.2.33 已迁移为 Schema 2 本地服务页面，使用通用 UIHost、
ServiceLease 和 CapabilityBroker。主应用不再包含该插件的专用 Tauri 命令、独立导航入口
或业务 Vue 页面；旧 `/plugins/liquid-gravity-probe` 地址只重定向到插件市场。迁移说明见
[Liquid Gravity Probe 迁移](docs/plugin-migration-liquid-gravity.md)。

Secondary Motion 3.3.1 已迁移为 Schema 2 静态页面。预设与角色参数存放在插件作用域
持久数据目录，宿主在启动前把数据镜像到原生 DLL 期望的包内目录；静态页面通过
PageLease 能力桥读写数据，不再依赖专用 Tauri 命令或本体 Vue 组件。迁移说明见
[Secondary Motion 迁移](docs/plugin-migration-secondary-motion.md)。

Player Tweaks 0.2.0 已把 GIMI 设置页、DLL/INI 运行配置和策略声明迁入插件包。GIMI 使用
通用作用域数据、启动镜像和注入 DLL；SRMI 保留一个临时注册表设置适配层。旧
`playerTweaksCamera` / `playerTweaksGameplay` 配置由声明式迁移能力按字段导入。迁移说明见
[Player Tweaks 迁移](docs/plugin-migration-player-tweaks.md)。

图形插件已开始迁移到通用 `graphicsProfiles`：插件声明策略、优先级、配置和资源键，
宿主不再按 HoYoShade/DLSS5 插件 ID 选择 Provider。HoYoShade 0.2.0 已通过通用
ReShade 热运行计划验收。DLSS5 后续迁移已跳过；0.3.0 只保留为不可发布开发快照。
详见 [图形 Profile 迁移](docs/plugin-migration-graphics-profiles.md)。

# 附加功能移除说明

SSMT4开源版仅包含了基础的启动器、管理器、模型提取功能，基础功能方便社区共建

其他附加功能为了满足开源社区的合规要求，已从SSMT4中删除，并放入MIMITools 咪咪工具箱中

https://github.com/StarBobis/MIMITools

没有这些附加的功能也不会影响你正常使用SSMT，请放心好了。

# LTS技术支持群(大本营)

大本营在discord，技术讨论可以去里面，为满足合规需求，暂时不开国内聊天群

Server Invite Link: https://discord.gg/aVKXuUrrM

# 特别感谢
- [Starbobis/Nicomico](https://github.com/Starbobis) 作为项目原拥有者对 SSMT 弥足轻重的贡献;
- [希尔·芙乐艾](https://github.com/xuhuan9102)对 SSMT 尤其是其 Blender 插件的大量贡献;
- [一夏抹忧](https://github.com/yixiamoyou)对 SSMT 尤其是其文档工程的大量贡献;
- 可能未提及的曾为 SSMT 项目贡献过代码的朋友;
- [H₄XeO₆](https://github.com/Perxenic-Acid) 也就是是我自己做出的一些贡献.
