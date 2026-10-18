# 运行时与图形问题处理记录（2026-10-15）

按以下顺序处理；每项完成可验证修复或阶段性工作后分别提交。实机效果与构建、夹具验证分开记录。

| 顺序 | 问题与验收目标 | 状态 |
| --- | --- | --- |
| 1 | 移除 XXMI DLL 更新弹窗。SSMT Runtime 静默更新；任何更新失败仅显示横幅，继续使用已有 DLL，不阻断启动。 | 已完成源码修复与本地验证，待实机回归 |
| 2 | 改善 DLSS5 插件兼容性，优先定位原神无法加载；逐游戏描述支持边界，保持插件启停及组合可用。 | 已修复 Context 虚表、混合设备转发及 FSR2 输入 API 配置；用户已可进入游戏，切换 FSR2 的最新修复与游戏内 NR 待验收 |
| 3 | 定位原神 Mod 模型散乱、贴图消失与拉伸，检查 Runtime 上游更新、Mod 配置与图形插件组合。 | 已解决；关闭动态角色分辨率后，用户确认所有 Mod 恢复 |

## 1. 运行时静默更新

当前运行时由 SSMT 应用资源提供，随应用更新，启动时静默同步到游戏的 Migoto 目录。DX11 使用 SSMT 修改过的 `resources/d3d11.dll`，DX12 保留现有 `resources/DX12/d3d12.dll` 来源。不新增独立在线 DLL 下载器，不安装 XXMI 上游 DLL。按用户明确的设计，设置页保留 XXMI 上游的只读 Changelog：SSMT 在其基础上适配，不重复编写上游说明；阅读更新记录不等于切换 DLL 来源。

源码中旧的 XXMI DLL 更新安装流程已经移除，清理未使用的最新版本查询 API 与旧更新确认文案，保留只读发布说明入口。用户随后确认新看到的“版本详情 / Warning / Please use XXMI Launcher to install and manage GIMI”属于 GIMI Core 更新包，保留其正常更新与上游说明，不按 DLL 更新弹窗处理。

此前部署仍可能阻断启动：`Run.exe` 先删除再复制且随后重复覆盖；DLL 先移走旧文件，再替换，UPX 又在最终文件上运行。现在统一为同目录暂存、复制哈希校验、暂存文件 UPX 处理和一次重命名替换。提交前不移动或删除旧文件；复制、校验、UPX、替换或清理失败时，仅显示非模态横幅，旧文件仍在原路径并继续启动。应用资源缺失也可以使用现存文件。首次安装且没有旧文件时只显示横幅并停止，避免启动缺失运行时的注入器。纯净启动仍不读取或部署 Runtime，无插件时不要求 PluginHost。

验证入口：

```powershell
node --test scripts/tests/test-runtime-sync.mjs scripts/tests/test-efmi-launch.mjs
bun run build
```

测试使用真实临时文件、生产 TypeScript 调用链与可控的失败注入，覆盖部署成功、内容相同、源文件缺失、复制失败、哈希读取和校验失败、文件占用、清理失败、UPX 失败/缺失/已打包、首次安装无回退、DX12 路径、纯净启动及 EFMI 启动队列。另以当前构建的 SSMT Runtime 在忽略的临时目录执行真实 UPX 打包与 `-t`，均通过；未修改游戏目录。前端类型检查与 Vite 构建通过，只有现有第三方注释与包体积警告。本次未进行真实游戏注入或画面验收。

## 2. DLSS5 插件

用户实测：3DMigoto 单独启动仍有模型/贴图异常；3DMigoto + DLSS5，以及再加 Shade Bridge 的组合均没有 DLSS5，后一组合只有 ReShade。原神明确禁止任何形式的冷替换，包括启动前临时放回代理 DLL/链接再在退出后恢复；必须使用热注入与 Hook。已有 `managed_cold_files::stage` 不符合此要求，不能作为原神方案。

本次增加独立的原神热注入路线：导入固定哈希、显卡 Profile 配对的组件到 SSMT 的 PluginAssets；Run.exe 在提权后设置本次目标的环境变量，创建挂起游戏，再依次加载 SSMT Runtime、被动 ReShade Host、FSR2 Bridge、OptiScaler、用户额外 DLL 与可选 PluginHost，最后恢复游戏线程。不复制 CXP 的 GIMI DLL，不向游戏目录放置代理 DLL；原神的 Swapper 安装与暂存入口直接拒绝。Shade Bridge 开启时把其 FX 和 Add-on 合入同一 Host，保留它禁用的 Add-on；关闭后不再贡献这些资源。

已完成启动编排、真实组件哈希与配置、原生进程夹具以及已知官方插件组合验证。新增插件市场中的 DLSS5 设置入口，提供完整组件自动下载和离线导入，逐文件验证后再激活，失败保留旧安装；普通启动不等待下载，组件不可用时横幅提示并跳过本次 DLSS5。RTX 50 与 RTX 30/40 两套 Profile 均完成从空缓存的真实下载、提取与配置验证，前端/启动 48 项回归通过。组件就绪不表示新启动的游戏已产生 NR 成功帧；官方资产发布以及其他游戏的新适配未包含在此阶段。安装、测试与日志验收见[原神 DLSS5 热注入](gimi-hot-runtime.md)。

23:35 用户已可进入游戏，切换 FSR2 后崩溃。新 dump 确认为 OptiScaler 将 DX11
Bridge 输入误交给 DX12 分支，偏移 `0x1C3E57`。已补齐启动准备的输入 API 与
兼容配置；独立实组件夹具可复现旧配置崩溃，修复配置完成 DX11 Create、Dispatch
及 Destroy。此项只验证系统设备上的输入契约，完整游戏 NR 与持续帧仍待验收。

## 3. 原神 Mod 加载

后续两次启动崩溃已单独定位为 Runtime 的内部帧分析方法与 Bridge 虚表副本冲突，修复及回归见[热注入诊断记录](gimi-hot-runtime.md)。这不改变下述已由用户确认的 Mod 资源修复结论。

只读检查本机最新单 DLL 日志：当前 SSMT Runtime v1.2.2.11、GIMI Core v8.9.1，Core 延迟加载已执行；95 条 INI 加载记录对应 95 个不同文件身份，没有同一文件经 junction 被重复读取的证据。存在部分用户 Mod 内容重复与重复节告警，不能将其直接归因为全局重复加载。抽查 Blend.buf 为标准 float4 权重 + uint4 索引、stride 32，没有发现这些样本的格式错误。之前修复的跨设备资源搬运日志本次未出现。

发现确定的启动配置缺口：本机原神注册表的动态角色分辨率开启（graphicsData 的 key 21 / value 2，globalPerfData 的 entryType 21 / index 1）；XXMI GIMI 启动时在 configure_game 开启的情况下关闭它，而 SSMT 原先只实现 WWMI/ZZMI 的启动设置。现已补齐 GIMI 调用，按目标 EXE 选择国服/国际服键，同时更新两份画质记录。写入前备份原始 REG_BINARY，重复调用不写入，未知格式不修改；关闭配置开关或纯净启动不读写游戏注册表。只在用户实际启动时修改，不在本次诊断中直接修改本机设置。

Rust 测试覆盖两份记录、单侧不一致、缺失条目、未知格式、ASCII JSON/中文及代理对转义、NUL 结尾、无变化时返回和其他画质设置保留。前端测试覆盖启动前调用与配置开关，已有纯净启动测试验证无配置调用。

2026-10-15 用户补充：原先所有被替换模型都异常，不需要以 F10 重载作为解决办法。补齐启动配置后，用户确认“所有mod已恢复”。本项据此完成实机验收；修复提交为 `2be878c`。无需修改 Mod 的 Blend 数据、屏蔽用户目录或回退 Runtime 上游合并。

### 2026-10-18 贴图再次异常：新版纹理入口缺少跟踪

用户报告 FSR2 Bridge、Feeder 及纯 3DMigoto（另启用 Player Tweaks）都出现贴图丢失或拉伸。只读检查当前游戏配置，动态角色分辨率的两份记录仍为关闭；现有 Player Tweaks 仅声明镜头、帧率和队伍页功能，未找到贴图绑定修改。当前游戏目录的 Runtime 为 1.2.2.15，隔离解压后 `.text` 与应用资源一致，不能将 UPX 前后的整文件哈希差异判断为部署旧 DLL。`d3d11_log.txt` 仍是 10 月 15 日且当前关闭调用日志，不能将它当作本次启动证据。

检查到明确缺陷：Device3 的 `CreateTexture2D1` / `CreateTexture3D1` 直接交给底层设备，绕过原有资源哈希、创建描述覆盖、Hunting 记录及释放跟踪。即使 Draw、绑定与重载仍经过 Runtime，这些新建纹理也不能参与正常 TextureOverride 匹配。现使新旧入口共用既有创建流程，保留新版系统 API、`TextureLayout` 及调用方引用，不修改贴图哈希算法或 Mod 文件。

新增 WARP 纹理回读回归：修复前，旧入口能将蓝色纹理替换为绿色 DDS，新版 2D / 3D 均未替换，尺寸覆盖也失败；修复后全部通过。覆盖交替创建的匹配 / 不匹配内容、释放后重建、尺寸覆盖、验证及失败返回，并与原始系统设备比较非默认 ROW_MAJOR 布局。原 Context、COM 接口、Hunting 和 F8 回归也通过，共十一项；不启动游戏或硬件渲染测试。

当前结论仅为新版资源路径的可复现缺陷已修复，不能据此认定纯 3DMigoto 的全部异常已消除。下一次实机应先确认普通启动时贴图恢复，再观察两条 DLSS5 路线；不以 F10 作为修复步骤。如果仍有异常，需本次运行产生的新日志或 Frame Analysis 来确认实际纹理创建和替换路径，不能继续引用旧日志。

Runtime 修复提交为 `895bcc01`；本地构建版本为 `1.2.2.17`，已同步应用资源及开发运行目录，由正常启动流程更新游戏 Runtime。未直接覆盖游戏 DLL、改动 Mod 或重编 OptiScaler。

### 1.2.2.17 复测未恢复：头像等 2D 贴图

用户通过 `runtime/debug.ps1` 重新构建并排除 DLL 被覆盖后，确认仍然异常，进一步说明
主要是头像等 2D 贴图。此反馈确认新版纹理入口修复尚不足以解释全部实机异常；
保留已经独立复现的入口修复，不把模型 Blend 或 Player Tweaks 作为当前根因。

只读检查实际头像 Mod：TextureOverride 先设置 AvatarFix 标志并执行
`handling = skip`，随后 ShaderOverride 的 post 命令调用 CustomShader，读取
`vs-t0 = vb0` 与 `x0 = first_vertex` 后 `draw = from_caller`。
因此丢图既可能是匹配问题，也可能是跳过原绘制后没有正确重绘；拉伸还需检查实际
VB 布局、偏移及 UI 着色器输入，不能只验证 DDS 加载。

在忽略目录构造软件夹具，验证同一跳过 / 重绘流程、24 字节 POSITION / COLOR /
TEXCOORD 顶点、非整 stride 大小的缓冲区、交替的 IA 偏移与 BaseVertex，以及
Device3 延迟 Context 提交；像素回读均通过。这只排除了所构造的简化场景，
没有复现真实头像着色器与游戏当前的绘制链，不能据此宣称游戏侧已恢复。
未启动游戏或硬件 GPU 测试，也未修改玩家 Mod。

本次 `d3d11_log.txt` 仍停在 10 月 15 日，原因是原 `debug.ps1` 保留了关闭日志的
INI 设置。现将该开发入口默认设为 Info，并提供 `-FrameAnalysisLogOnly`：在异常页面
按一次 F8，仅采集本帧 API / Mod 命令，避免整帧资源转储。每次独立备份 INI，提供
`-LogLevel preserve` 保留开关及 `-PrepareOnly` 离线准备。Windows PowerShell 5
配置回归通过，WARP 实际采集只生成包含 Draw 的 `log.txt`。

当前尚未定位本次实机根因。下一步必须检查同次异常页面的 TextureOverride 命中、
AvatarFix post 重绘和绑定记录；不能将旧日志中的同名资源或重复 Mod 内容视为本次证据。
命令与恢复步骤见 [Runtime 调试入口](../runtime/docs/reshade-compatibility.md)。

### 两次异常采集：多设备延迟加载遗漏参数纹理

用户确认 23:12:20 的采集表现为拉伸，23:12:38 的采集表现为消失。两份日志均来自
Runtime 1.2.2.17，纹理命中后成功设置 AvatarFix 标志、替换 DDS 并执行 post 重绘。
拉伸样本使用 `4f8eee47124e933d` / CustomShaderFix2，VB stride 24、BaseVertex 3188；
消失样本使用 `f61f9bc2a15bedef` / CustomShaderFix3，VB stride 24、BaseVertex 1140。
因此不能再归因为完全未匹配、未执行重绘或缺少 GameType。

同次初始化日志显示，两个设备在加载完整 Mod 配置前创建，均跳过了空参数纹理；
完整配置随后只补建了一份 397 参数的纹理。原 `ReloadConfig` 仅更新触发重载的
设备，却用全局 CPU 数组长度判断其他设备是否已经拥有资源。辅助交换链先 Present
时，游戏设备可能始终没有 `t120`，使 AvatarFix 虽设置了 `x0=first_vertex`，
GPU 实际仍按零取数。不同 UI 中错误顶点的 UV / 尺寸计算可表现为拉伸或不绘制。
日志中的跨设备 TexFx 搬运另作线索，不将每一条搬运告警都认定为此缺陷所致。

在两个 WARP 设备与延迟 include 的同一时序下复现：旧 DLL 的游戏 Context 在
`t120` 返回空，非零 BaseVertex 重绘回读为黑色；仅保留单设备的旧测试会通过。
现增加配置代次：每台设备在下一次绘制、Dispatch 或 Mod 命令前补齐自身参数纹理，
每个已有 Context 重新绑定相应代次；不重跑 `[Constants]`，避免重置玩家状态。
参数数量不变的重载、扩容、缩容及清空也同步，普通热路径不新增分配或锁。

新增 `params-early` / `params-delayed` 回归各覆盖 30 次真实纹理替换、post 重绘与
像素回读。旧 DLL 分别在参数扩容与初始缺失处失败，新 DLL 连同原有检查共十三项
通过；x64 Release 编译通过。测试只使用 WARP，未启动游戏或硬件 GPU 测试，
未改动玩家 Mod。当前结论为与实机日志时序一致的资源同步缺陷已复现并修复；
最终画面仍待纯 3DMigoto 启动复测，再核对两条 DLSS5 路线。

Runtime 提交 `0d1f1fb9`，本地构建版本 `1.2.2.19`。已核对三处 DLL 的哈希一致并同步
应用资源、开发运行资源和 GIMI 测试目录，保留旧游戏 Runtime 备份及匹配的新 PDB。
临时采集 INI 与独立备份之间仅有 Hunting / analyse_options / log_level 的变化，
已恢复采集前原件；两份异常帧日志和原备份保留。可直接重启游戏复测原来两处界面，
无须 F10 重载。
