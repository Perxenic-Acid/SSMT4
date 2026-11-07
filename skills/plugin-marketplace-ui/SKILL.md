---
name: plugin-marketplace-ui
description: 修改 SSMT 插件市场的列表、详情、设置入口与异步交互，验证主题、窗口适配和可选插件隔离。不用于原生注入或插件包来源验证的实现。
---

# 插件市场界面维护

兼容范围来自清单并按实际 `gamePreset` 比较；同时检查后端启用与启动贡献过滤，不能只禁用界面开关。旧不兼容状态仍应允许关闭，更新提示不能遮盖不兼容提示。

先读 [界面与验证说明](../../docs/plugin-marketplace-ui.md)，追踪市场、设置组件和实际 IPC 调用。

- 保留共享玻璃主题变量、预设游戏排序及本体/游戏插件作用域。不要为了布局更换安装、来源验证或第三方包确认流程。
- 常用设置使用详情区完整宽度；来源、版本、权限与依赖仍须可找到。短来源标签不能取代后端验证。
- 插件未安装或未按作用域启用时，不因展示界面扫描、配置或下载插件资源。用 IPC 调用记录验证无插件、禁用和切换游戏路径。
- 并行刷新应等待所有来源，失败保留已有结果。日志等延迟操作捕获原对象身份，不能用返回时的新选择执行清空等动作。
- 快照替换不等于插件状态改变；用独立的身份、版本和启用状态监听，避免数组 getter 因对象替换反复卸载设置。持续闪烁先区分重复 IPC / DOM 更新与合成问题；无游戏也可能有模糊层，市场的无模糊材质须覆盖静态、视频和空状态，不能仅挂在视频背景 class 下。慢目录占位动画仅使用透明度或位移，禁止无限动画 `background-position`；用生产模式真实组件检查空闲更新和 Paint，不以清空列表、持续刷新或禁用全局 GPU 加速掩盖问题，浏览器结果不能代替目标 WebView2 验收。
- 正式版仅从 Native Release 按需安装官方插件；Debug 才发现 `build/plugin-packages`，不能把开发包放进应用资源。Player Tweaks 不保留虚拟内置条目或应用 DLL 回退，旧启用状态不能代替安装；独立安装包保留。分别检查注册表、远程目录与开发目录，不先认定漏打包或 DLL 冲突。跨电脑闪退检查事件模块 / 异常代码与实际 Release 的 CPU 特性；禁止 `target-cpu=native`。压缩包解压和哈希移出交互线程；使用 `plugin_marketplace_startup` 回归，不运行发布包构建。
- 官方条目全部消失时核对真实发布目录的解析错误；PowerShell 缺省数组可能成为 `[null]`，单个坏条目会影响整个目录。多版本目录按 ID 展示最新版，发现与安装均不隐式启用或部署外部组件。实际云端包验证复用说明文档中的隔离集成测试，区分界面夹具、真实下载安装和游戏内验收。
- 安装旧插件却没有更新按钮时，同时核对已安装清单、GitHub 最新目录的包版本和实际 DLL；主应用或 Native Release 标签变了，不代表插件版本变了。同一 `ID@版本` 不得重打新实现，安装器也不会覆盖同版目录。Poser 的 DLL 查询版本读取包清单，正式发布的源码变更检查见 Native 的 `tools/check-plugin-release-versions.ps1`；本地提交或 Actions Artifact 不能代替发布新目录。
- 切换游戏或渲染 API 时立即失效旧状态与可安装路线，递增请求标识。延迟扫描期间不能沿用上一对象的路线，旧结果与进度不得覆盖新对象。
- 运行 `scripts/tests/test-plugin-graphics-settings.mjs` 中的行为回归及 `bun run build`。不为纯样式编写镜像测试，不运行主应用发布包脚本。
- Plugin API 1 已冻结。修改能力、页面桥或官方包版本后先运行
  `bun run check:plugin-api-contract`；任何能力删除/重命名、权限语义变化或页面协议
  破坏都必须停止并改为新主版本，不能直接更新冻结快照。契约见
  [Plugin API 1 冻结契约](../../docs/plugin-api-v1-contract.md)。
- 浏览器使用真实组件、共享样式和临时 IPC 夹具；检查默认/最小窗口、中英文、窄窗口、空列表、断网和本体插件。确认无横向溢出、下载操作可找到、键盘焦点可见。
- 高度钳制须检查实际盒模型：`height: 100%` / `max-height: 100%` 的容器要将内边距与边框计入可用高度。测量左右底边、滚动后的边界及缩放，不以根容器无滚动条代替验收；`overflow: hidden` 可能只把越界内容裁掉。夹具不要额外注入全局 CSS reset，避免替真实组件修掉缺失约束。
- 新插件页面从 `plugin_page_catalog` 读取，不在市场中继续增加固定插件 ID 分支。声明式、静态页和本地服务共用 `PluginPageHost`；本地服务只能由后端解析游戏配置、模板变量、端口和令牌。
- Schema 2 使用 `compatibility.pluginApi`，Schema 1 的 `pluginApiVersion` 只作兼容读取；市场展示“设置入口”时必须同时计算 `hostExtensions` 和通用 `pages`，不能把陌生插件页面误报为无设置。
- 插件自带的本地服务页只能通过版本化 `postMessage` 桥请求能力。验证来源 Origin、`contentWindow`、请求 ID 和有效 Lease；后端必须从 Lease 解析插件身份，禁止信任页面自报的插件 ID。回归入口为 `scripts/dev/verify-plugin-sdk-fixture.mjs`。
- 声明 `launchHooks` 的插件必须展示通用启动资源冲突提示。调用 `plugin_launch_plan_preview` 读取仲裁结果，不在前端按插件 ID 决定优先级；同优先级争抢同一资源时必须明确阻断，高优先级唯一归属才可生成可执行计划。回归入口为 `scripts/dev/verify-plugin-launch-plan-fixture.mjs`。
- 已完成迁移的官方插件应从主应用删除专用 Nav、路由组件、配置面板、免责声明和 Tauri 命令；只允许保留旧地址到市场的重定向。Liquid Gravity Probe 0.2.33 必须通过 `pages[]` 本地服务页加载，并保持旧数据目录 `Mods/SSMT_LiquidGravityProbe`。回归入口为 `scripts/dev/verify-liquid-gravity-probe-migration.mjs`。
- 静态插件页必须通过 `StaticPageLease` 使用能力桥；Tauri asset URL 只能拆分最后一个 `%2F` 文件名分隔符，否则 `./assets/*` 会错误解析到 asset 根目录。插件升级后 `PluginPageHost` 的 key 必须包含版本。Secondary Motion 3.3.1 的持久数据位于 `PluginData/ssmt.secondary-motion/SecondaryMotion`，启动前由宿主通用镜像到包内目录；回归入口为 `scripts/dev/verify-secondary-motion-migration.mjs`。
- Player Tweaks 0.2.0 的 GIMI DLL、INI 和设置均由插件包持有，通用注入只接收包内 DLL 路径。旧 GameConfig 设置只能通过 `migrations.keys` 白名单迁移，不能读取完整配置。SRMI 注册表事务仍是临时兼容层，修改时必须保留所有权 Journal、只恢复本插件写入值和关闭游戏后恢复。回归入口为 `scripts/dev/verify-player-tweaks-migration.mjs`。
- 图形插件必须使用 `graphicsProfiles.strategy/priority/resources`，不能在 Core 新增插件 ID 到 Provider 的分派。HoYoShade 0.2.0 的 `reshade-hot` 已纳入通用计划；DLSS5 后续迁移已跳过，0.3.0 保持 `publish: false` 开发快照，不构建正式包。回归入口为 `scripts/dev/verify-graphics-profiles-migration.mjs`。
- iframe 的 `load` 事件只表示文档开始加载，不等于编辑器可用。必须同时验证健康接口、页面 HTML/构建资源、子页面完成状态和截图；旧 session URL 在服务停止后会显示连接拒绝，刷新页面或清理会话后才能判断 UI 回归。
- 本地服务按 `pluginId + version + pageId + game` 组成 ServiceInstance，页面只持有 ServiceLease。离开页面必须先 Release Lease，不能直接拥有或终止服务；最后一个 Lease 释放后由后端执行空闲宽限。插件禁用、更新、卸载和 SSMT 退出必须立即清理，进程使用 Windows Job Object。完整 ABI 与回归见 [插件页面运行层](../../docs/plugin-pages.md)。
- 全局页面容器使用 `<KeepAlive>`，切路由只触发组件 `onDeactivated`，不能用 `onBeforeUnmount` 代替空闲回收。`onActivated` 必须取消待停止计时并查询 session；session 已失效才重新启动。回归入口为 `scripts/dev/verify-plugin-page-idle-cleanup.mjs`。
- 一次性预览、截图、采集脚本及回传诊断包留在忽略目录，不为单次排查新增正式工具和测试入口。区分界面夹具、真实下载、原生注入和实际渲染的验证证据，不将后几项视为截图验收的结果。
