# 插件逐游戏适配

按顺序推进，每项分别提交。安装和离线检查通过不等于游戏内渲染验收通过。

## 清单

本轮反馈按以下顺序处理：

- [x] 修复 D3D11 热注入初始化顺序及同名系统 DLL 被误报为 Runtime，补充真实 Runtime 注入回归；崩铁 Mod 画面仍待复测。
- [x] 原神增加可选择的 Feeder 实现，保留原 FSR2 Bridge 路线；组件配置及启动计划回归通过，画面待实机比较。
- [ ] 保留 ReShade 教程进度及用户配置，覆盖原神与其他热注入路线。

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

本轮崩铁日志确认 Feeder 已接收帧，但 ReShade 在 Runtime 注入之后才延迟加载系统 D3D11。Runtime 只在初始化时挂钩已加载的系统库，因此预加载现先装入系统 D3D11，再装入 Runtime、各插件并恢复目标；最终确认比较完整文件身份。隔离测试覆盖真实进程、系统入口的加载顺序及 Runtime Context 身份，另检查真实 ReShade 共存。原先的 Context 夹具在 EXE 导入阶段就加载系统 D3D11，不能覆盖这次启动顺序。已有 `calls=0` 的旧 Migoto 日志仍不能判断本次是否接管；游戏内 Mod 画面必须复测。

插件未安装或未启用时不下载、不扫描专用资源、不复制 DLL、不注入。可选组件准备失败时横幅提示并继续普通启动。检查空组合、单插件、多个插件与全部已知插件；同一进程只能有一套 ReShade Host，关闭插件后不能遗留其 Add-on 贡献。

其他游戏的组件使用独立清单与安装目录，不能复用 GIMI 特有的 Bridge 或混搭 NR 链。固定下载版本、大小与 SHA-256，新目录准备完成后才切换设置；不执行下载包内脚本、不伪造 RHI 的安装记录、不覆盖其文件。

## DLSS5 实现与验证

插件市场 → 选择游戏 → 已启用的 DLSS5 → 功能设置 → 选择显卡 → 自动下载并配置。原神继续使用原有组件包；其他三款游戏使用 `dlss5_hot_assets.json` 固定的 ReShade 6.8.0、NR consumer 6.5.3、Bridge 1.4.12、Feeder 1.19.0 和配套 snippet。新增组件存放在 `PluginAssets/ssmt.dlss5.integration/hot/`，启动前合并启用的 Shade Bridge 贡献，只注入一个 Host。

原神的“渲染路线”现在可选 FSR2 Bridge 或 Feeder。前者仍使用 CXP 配套组件；后者使用独立的 Feeder / Lumenite / consumer 配套组件，不载入 OptiScaler 或 FSR2 Bridge，不向游戏目录放置 DLL。Feeder 路线关闭游戏内 FSR2；FSR2 Bridge 路线仍选择 FSR2。所选路线在自动下载或离线导入完整成功后与安装记录一起保存，下载失败保持原路线；旧配置默认继续 FSR2 Bridge。两套资源分别保留，按游戏实例记录选择，重新打开设置时显示当前实现。

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

## 依据

- 本地 `RHI` 的 `DetailPanelBuilder.NeuralRendering.cs` 和 `Renodx5AddonService.cs`。
- [DX11 DLSS Bridge](https://github.com/NIGos/dlss5-bridge)。
- [无原生 DLSS 的 Feeder](https://github.com/jlrouzies-fr/DLSS5-Feeder)。
- [绝区零原生 DLSS 公告](https://www.nvidia.com/en-us/geforce/news/zenless-zone-zero-version-dlss-4-5-ray-tracing/)。
