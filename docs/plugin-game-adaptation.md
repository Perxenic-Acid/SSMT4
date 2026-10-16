# 插件逐游戏适配

按顺序推进，每项分别提交。安装和离线检查通过不等于游戏内渲染验收通过。

## 清单

本轮反馈按以下顺序处理：

- [x] 崩铁 3DMigoto 与 ReShade 共存：已修复交换链取回底层设备的绕过路径，并让 `IDXGISwapChain4` 保持 Runtime 包装；2026-10-16 的真实短测同时加载 SRMI Runtime、ReShade 和 DLSS5，日志确认交换链重载配置、解析 `Mods` 及加载自定义资源，用户确认成功载入。完整游戏内画面和长时间退出行为仍需单独验收。
- [x] Shade Bridge 单独启用时共用 Run.exe 预加载链，移除独立注入器竞争和强制 150 ms 延迟；保留源 Add-on 禁用项及游戏已有教程 / 按键状态。
- [x] 原神启动前按实际采用的 DLSS5 路线自动控制 FSR2，不再依赖玩家手工切换。

- [x] 修复 D3D11 热注入初始化顺序及同名系统 DLL 被误报为 Runtime，补充真实 Runtime 注入回归；崩铁 Mod 画面仍待复测。
- [x] 原神增加可选择的 Feeder 实现，保留原 FSR2 Bridge 路线；组件配置及启动计划回归通过，画面待实机比较。
- [x] 保留 ReShade 教程进度及用户配置，覆盖原神与其他热注入路线，旧配置首次迁移与重复启动回归通过。

- [x] 原神 MMD Poser：限制为 GIMI；配置入口、后端启用校验和旧状态过滤一起修复。提交 `b9a1e0e`，插件 Rust 测试、界面回归和前端构建通过。
- [x] DLSS5 配置、下载与启动集成：保留 GIMI 专用 FSR2 热注入链，扩展以下路线。
  - [x] 崩铁 SRMI：无原生 DLSS，使用 Feeder 合成输入及运动矢量效果。
  - [x] 鸣潮 WWMI：沿用已验证的 RHI 原生 DLSS DX11 Bridge + NR consumer 路线，由 SSMT 管理下载和注入。
  - [x] 绝区零 ZZMI / ZZMIDX12：按 DX11 Bridge / DX12 原生输入分别准备，游戏中必须启用 DLSS。
  - [ ] 新的 SSMT 集成路径实机确认成功 NR 帧、画面与退出行为。不能用之前的 RHI 实测替代这一项。
- [x] Player Tweaks 第一阶段：扩展崩铁 120 FPS，功能按游戏区分；不注入原神专用 DLL。
  - [x] 国服 / 国际服按实际客户端信息选取设置位置，保存恢复记录，保留其他画质字段及后续用户修改。
  - [ ] 崩铁游戏内 FPS 验收；任意帧率、相机等 Native 功能仍需单独取得对应版本证据。

## 验收约束

10 月 16 日用户进一步确认：崩铁关闭 HoYoShade Bridge 后 3DMigoto 可用，开启后无欢迎文字。14:49 的 ReShade 日志显示它从外部资源目录先注入，系统 D3D11 随后才加载。调用链检查发现此前有序预加载只覆盖 DLSS5，Shade 单独开启仍启动独立 `inject.exe`、等待后走 CBT，并强制 Runtime 延迟 150 ms。这条遗漏路径现通过 `prepare_hoyoshade_hot_runtime` 准备同一种启动计划，交给 Run.exe 挂起创建游戏，按系统 D3D11、SSMT Runtime、ReShade 的顺序加载后恢复；不再启动独立注入器。Shell 直接指向同一游戏 EXE 时也使用该策略，不改保存的配置；手动等待和不同外部启动器仍保留原路径，尚未覆盖本次修复。

Shade 单独使用的配置位于 `PluginAssets/ssmt.hoyoshade.bridge/hot/<游戏实例名哈希>/`。复用已有配置组合器及默认值记录，不添加 DLSS 组件或修改抗锯齿；原 HoYoShade 的 Add-on 禁用项保留。首次迁移只读取游戏目录旧 INI 的教程、按键、界面与截图状态，不继承旧注入器重写的 Add-on / Host 路径，也不写入游戏目录。未启用插件时不准备这些资源，准备失败仅横幅提示并继续普通 Mod 启动。DLSS5 准备或画质配置失败时，Shade 同样采用有序预加载回退，不重新启动独立注入器。

本次 121 项 Rust 插件测试、70 项前端 / 启动回归及前端构建通过；另使用本机真实 HoYoShade 资源与崩铁旧 INI 连续准备两次，确认禁用项、教程进度及源文件不变。该资源检查不加载 DLL、不启动游戏；未重复运行 GPU 夹具，不能据此认定游戏内共存已经恢复。本次修改涉及 Tauri 命令和前端，需重新启动开发应用（`bun tauri dev`）；仅重新 `build_native.ps1` 不会更新这条调用链。

复用实际资源检查时设置 `SSMT_SHADE_TEST_SOURCE` 为 HoYoShade 目录、`SSMT_SHADE_TEST_GAME` 为已退出的游戏 EXE，再执行 `cargo test --manifest-path src-tauri/Cargo.toml --lib plugin::hoyoshade::tests::real_shade_assets_prepare_without_executing_or_modifying_sources -- --ignored`。输出仅写到独立临时目录，成功后清理；不会运行其中的 EXE、DLL 或 Add-on。

10 月 16 日继续排查确认 `HackerSwapChain::GetDevice` 原先直接返回系统设备，从它重新取得的 Context 会绕开 Runtime。现改为向交换链所持有的包装设备查询接口，复用原有 Hook / 包装及引用管理策略。新增 Unity 风格 WARP 用例覆盖 BGRA、Device1 / Context1、独立交换链及从交换链取回设备；它通过 INI 的 `post clear` 命令把蓝色清屏改为红色并回读像素，修复前退出 7，修复后通过。三项原生注入回归和原有六项 Context 回归通过。该缺陷与“进程已检测到”是不同层次，仍不能把夹具通过当作崩铁实机恢复。

同日用户报告硬件测试期间出现 `nvlddmkm` 事件 153；只读核对 14:06–14:35 共 13 条，时间与真实 ReShade 交换链夹具重合。硬件组合未取得完成标记并超时，已停止重复硬件复测；驱动事件与超时的因果关系、是否涉及设备 / Context 指针冲突均未证实，不扩展为驱动排障任务。ReShade 日志停在初始化后段，不能仅按源码中的联网检查判定全部超时原因。后续自动回归明确使用 WARP，硬件必须显式选择；WARP 不覆盖 ReShade 的完整硬件代理与游戏画面。

本轮崩铁日志确认 Feeder 已接收帧，但 ReShade 在 Runtime 注入之后才延迟加载系统 D3D11。Runtime 只在初始化时挂钩已加载的系统库，因此预加载现先装入系统 D3D11，再装入 Runtime、各插件并恢复目标；最终确认比较完整文件身份。隔离测试覆盖真实进程、系统入口的加载顺序及 Runtime Context 身份，另检查真实 ReShade 共存。原先的 Context 夹具在 EXE 导入阶段就加载系统 D3D11，不能覆盖这次启动顺序。已有 `calls=0` 的旧 Migoto 日志仍不能判断本次是否接管；游戏内 Mod 画面必须复测。

10 月 16 日又核对了用户报告的“DLSS5 缺少依赖但似乎可用”：历史插件包仍声明旧 Swapper、adapter、payload 为依赖，但当前崩铁路线由 SSMT 托管热注入，不使用这三项。生命周期和启动预检现只对 DLSS5 `0.2.1` 及更新包的这三个旧依赖 ID 按可选集成处理；`0.2.0` 及更旧冷路线和其他插件 / 依赖仍按必需项检查。不会修改已安装的插件包。诊断同时读取受管 ReShade 或 SSMT 自有热注入目录中的 `ReShade.log`。Feature 18 成功记录只能证明对应效果执行过，仍需看后续帧计数；日志中的深度源尺寸若为 `5120×3200`、而画面为 `2560×1600`，应在 Generic Depth 中选择与画面匹配的候选项。用户看到的 HoYoShade `v0.1.0` 是插件包版本，SSMT 本体的启动注入修复不会改变该版本号。上述日志只证明 ReShade / 效果运行，不证明 3DMigoto 本次已接管或 Mod 画面恢复；这两项仍待游戏内复测。

插件未安装或未启用时不下载、不扫描专用资源、不复制 DLL、不注入。可选组件准备失败时横幅提示并继续普通启动。检查空组合、单插件、多个插件与全部已知插件；同一进程只能有一套 ReShade Host，关闭插件后不能遗留其 Add-on 贡献。

其他游戏的组件使用独立清单与安装目录，不能复用 GIMI 特有的 Bridge 或混搭 NR 链。固定下载版本、大小与 SHA-256，新目录准备完成后才切换设置；不执行下载包内脚本、不伪造 RHI 的安装记录、不覆盖其文件。

## DLSS5 实现与验证

原神的抗锯齿现在由启动计划自动控制：FSR2 Bridge 启用 FSR2，Feeder 关闭游戏抗锯齿。组件、启动方式及 DLL 所有权检查通过后才修改注册表，同时更新 `graphicsData` 的 key 8 和 `globalPerfData` 的 entryType 8；原始 REG_BINARY 按现有画质备份机制保存，写入前再次比较当前值。关闭插件、纯净启动、组件准备失败或不支持的启动方式均不修改抗锯齿，画质格式未知或备份 / 写入失败仅横幅说明并跳过本次 DLSS5，普通 Mod 启动继续。此操作独立于普通的“配置游戏设置”开关。编码依据为 [Collapse 原神设置模型](https://github.com/CollapseLauncher/Collapse/tree/main/CollapseLauncher/Classes/GameManagement/GameSettings/Genshin/RegistryClass)：Off / FSR2 / SMAA 的 index 为 0 / 1 / 2，graphicsData 的 value 加一。六项 Rust 画质测试与 67 项前端 / 启动回归通过，验证没有写入本机注册表。

插件市场 → 选择游戏 → 已启用的 DLSS5 → 功能设置 → 选择显卡 → 自动下载并配置。原神继续使用原有组件包；其他三款游戏使用 `dlss5_hot_assets.json` 固定的 ReShade 6.8.0、NR consumer 6.5.3、Bridge 1.4.12、Feeder 1.19.0 和配套 snippet。新增组件存放在 `PluginAssets/ssmt.dlss5.integration/hot/`，启动前合并启用的 Shade Bridge 贡献，只注入一个 Host。

原神的“渲染路线”现在可选 FSR2 Bridge 或 Feeder。前者仍使用 CXP 配套组件；后者使用独立的 Feeder / Lumenite / consumer 配套组件，不载入 OptiScaler 或 FSR2 Bridge，不向游戏目录放置 DLL。Feeder 路线关闭游戏内 FSR2；FSR2 Bridge 路线仍选择 FSR2。所选路线在自动下载或离线导入完整成功后与安装记录一起保存，下载失败保持原路线；旧配置默认继续 FSR2 Bridge。两套资源分别保留，按游戏实例记录选择，重新打开设置时显示当前实现。

热注入配置不再每次直接用模板覆盖 ReShade 的运行结果。自有配置目录中的 `ReShade.ssmt-defaults.json` 保存上次生成的默认值，启动时比较默认值与当前 INI，保留教程进度、界面、按键及仍启用效果的参数。缺少记录的旧配置会迁移已有教程与界面状态。路径、必要 Add-on、Feeder provider 和必要效果顺序由当前启动计划维护，关闭 Shade 后移除其效果、搜索路径及 DLL，保留用户额外效果和非必要禁用项。原神 FSR2 Bridge 使用 `state/reshade-runtime`，Feeder / DX11 Bridge / DX12 使用各自安装根目录；首次使用另一套实现仍有独立的 ReShade 状态。

本轮验证：117 项 Rust 插件测试、66 项前端 / 启动回归及两项原生注入 CTest 通过。原神 FSR2 Bridge 与另外六组路线 / 显卡 Profile 的完整组件提取和配置检查通过；真实 ReShade 与 Runtime 预加载顺序、Context 身份检查通过。尚未在真实游戏中验收本轮修改后的 Mod 画面、原神 Feeder 的 NR 帧或教程完成后的连续重启行为。

已有 RHI / ReShade 安装需先通过原管理器恢复。准备时检查游戏目录的 `dxgi.dll` / `d3d11.dll` 和 `ReShade.ini` 中的 `INSTALL.BasePath`，发现冲突只跳过本次 DLSS5 并提示，不覆盖原文件。ReShade 6.8.0 的游戏目录 BasePath 优先于环境变量，单设 `RESHADE_BASE_PATH_OVERRIDE` 无法隔离旧安装。运行后在自有组件目录查看 `ReShade.log`，Feeder 路线还可查看 `dlss5-feed.log`；只有成功加载日志不能认定 NR 已执行。

手动等待或通过外部官方启动器创建进程时无法继承这条热注入计划，因此本次仅跳过 DLSS5，并继续原启动方式。Shell 直接指向同一个游戏 EXE 时仍可由 Run.exe 挂起创建并继承环境；计划与 PluginHost、额外 DLL 顺序一起传递。相关启动队列回归保存在 `scripts/tests/test-efmi-launch.mjs`。

已核验所有下载包及实际提取文件。六组路线 / 显卡组合均通过真实组件解压、完整性与配置检查。另用现有 Runtime Context 夹具验证 Feeder + consumer、DX11 Bridge + consumer 的热加载及 WARP Context 调用，两项退出码均为 0，ReShade 日志确认对应 Add-on 加载。该夹具没有游戏帧和 DLSS 请求，不能证明运动矢量质量、DX12 游戏兼容或 NR 已生效。

真实组件检查默认忽略，需要显式指定本地核验包目录后执行：

```powershell
$env:SSMT_DLSS5_COMPONENT_PROBE_DIR = '已下载固定组件的隔离目录'
cargo test --lib extracts_real_pinned_components_and_prepares_each_route -- --ignored --nocapture
```

目录文件名依次为 `reshade.exe`、`consumer.zip`、`bridge.addon64`、`feeder.zip`、`sr.zip`、`nr50.zip`、`nr30.zip`、`lumenite.zip`、`ReShade.fxh`、`ReShadeUI.fxh`，来源及哈希以清单为准。检查不会启动游戏或修改游戏目录。

## Player Tweaks 实现与验证

原神保留相机、目标帧率和快速队伍页功能。崩铁当前只显示“启用 120 FPS”，在下次启动前更新客户端的 `GraphicsSettings_Model_h2986158309`；按 `StarRail_Data/app.info` 区分 `miHoYo/崩坏：星穹铁道` 和 `Cognosphere/Star Rail`，不通过注册表键是否存在来猜地区。

插件启用时才处理 FPS。原始 REG_BINARY 备份在 `GameSettingsBackups/SRMI/`，恢复所有权保存在插件设置中。关闭插件前需关闭游戏；最后一个共用客户端的实例停用时，只把仍等于插件写入值的 FPS 恢复成原值，其他画质设置保持玩家当前值。已经由玩家设置为 120 的值不认领。格式未知、资源缺失或并发修改时跳过本次增强并横幅提示，游戏继续启动。

已通过真实国服设置的只读解析与内存往返验证，未修改本机注册表。自动回归覆盖地区识别、JSON / REG_BINARY 格式边界、其他画质字段保留、用户后续修改、已有 120 FPS、多实例共享与最后所有者恢复。新启动路径尚需游戏内实测；没有新增崩铁相机或队伍页 Hook。

```powershell
$env:SSMT_STAR_RAIL_APP_INFO = '游戏目录/StarRail_Data/app.info'
cargo test --lib reads_real_settings_and_checks_roundtrip_without_registry_writes -- --ignored --nocapture
```

清单中的代码项已完成阶段提交，实机项保持未勾选；后续根据游戏日志与画面分别推进。

## 插件市场与设置界面（2026-10-16）

- [x] 修复插件升级后丢失旧依赖目录的误报，按插件与依赖 ID 复用路径并重新校验新版文件，不继承来源信任。
- [x] Player Tweaks 改用胶囊开关，按视野、场景、镜头行为、帧率与便捷功能分组，窄面板单列显示。
- [x] 依赖状态旁提供获取入口：DLSS5 复用自动下载与校验部署；HoYoShade 提供发布页、目录选择和打开目录。通用设置不再读取或挂载 HoYoShade 专用界面，旧入口定位市场依赖卡片。旧 Swapper 下载源不可用、适配器未独立发布时明确说明，不影响热注入。
- [x] 移除市场顶部标题和重复游戏选择器，跟随全局当前游戏；刷新、本地安装收在搜索框右侧图标按钮，保留提示与无障碍标签。
- [x] 市场及设置页限制在应用内容区，两侧独立滚动。左栏按自然内容取高，超出可用区域后钳制；不按完整视口估算高度，也不强制拉伸短菜单。窄窗口上下分区时仍分别滚动。

已验证 142 项 Rust 插件测试、19 项前端交互测试、真实插件打包与安装检查，以及前端类型检查和构建。真实 Vue 组件浏览器检查覆盖短 / 长菜单、右侧滚动、设置导航定位、短窗口、窄窗口及 125% 界面缩放；均未产生根容器纵向滚动。浏览器检查关闭 GPU，不启动游戏或加载渲染 DLL。这些界面与依赖修复需要更新并重启 SSMT 开发应用，仅重新构建 Native 不会更新本体。

## 依据

- 本地 `RHI` 的 `DetailPanelBuilder.NeuralRendering.cs` 和 `Renodx5AddonService.cs`。
- [DX11 DLSS Bridge](https://github.com/NIGos/dlss5-bridge)。
- [无原生 DLSS 的 Feeder](https://github.com/jlrouzies-fr/DLSS5-Feeder)。
- [绝区零原生 DLSS 公告](https://www.nvidia.com/en-us/geforce/news/zenless-zone-zero-version-dlss-4-5-ray-tracing/)。
