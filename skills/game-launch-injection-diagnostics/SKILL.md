---
name: game-launch-injection-diagnostics
description: 排查 SSMT 的游戏启动、官方启动器上下文、手动等待与原生 DLL 注入差异，尤其是终末地更新后的 EFMI 与 XXMI 行为对照。不用于 Poser 物理或 Mod 资源制作。
---

# 游戏启动与注入诊断

涉及 EFMI 时先读 [终末地注入说明](../../docs/efmi-injection.md)，其中包含交付脚本、验证入口和游戏实测边界。

涉及原神 DLSS5 时先读 [热注入说明](../../docs/gimi-hot-runtime.md)，核对组件 Profile、单 Host 启动计划和日志验收。

- 从 `src/common/LaunchGame.ts` 的队列、`MigotoManager.patchD3dxForLaunch` 生成的 INI，再追到 `native/3Dmigoto-Injector-V2`，核对实际使用的注入模式；界面写“手动”不意味着 INI 已清除 `launch`。
- 区分官方启动上下文与注入机制。EFMI 使用外部官方启动进程和直接注入；不能把 XXMI 的 `DIRECT` 当成 SSMT 的 CBT Hook，也不能从几项命令行参数推断父进程伪装足够。
- `useShell` 只表示 Shell 执行方式，不代表官方启动器。先核对“启动目标程序”和两条完整 EXE 路径；Shell 直接指向同一游戏时，GIMI 热注入仅切换本次执行策略，不写回用户配置。须同步 INI 的 `launch` / `launch_args` 与队列，只创建一次游戏；无热注入计划仍保留原 Shell 流程。测试夹具不得将用户提供的不同启动器路径覆盖成目标路径，否则会掩盖误判。
- 对照 XXMI 时记录本地 commit，读具体游戏默认项和 `MigotoInjector` 调用链。不要推广成所有预设都该更换注入方式。
- 原神模型/贴图异常先区分同一 INI 重复载入、不同文件内容冲突和实际缓冲绑定；重复节告警不能证明重复载入。检查动态角色分辨率的两份注册表画质记录，GIMI 的“配置游戏设置”需与 XXMI 一样关闭该项。未知结构不猜测，写入前保留原始记录；纯净启动不得修改。
- 原神 DLSS5 只允许热注入/Hook；把 DLL 或链接临时放回游戏目录再恢复仍属于冷替换。启动成功、仅有 ReShade 或 Feature 1 创建均不等于 NR 生效，应核对同次日志中的 Feature 18 成功帧及 Mod 画面。
- 游戏已退出时先查当次 Unity 日志与 `%TEMP%/mihoyocrash_*` 的 dump；`error.log` 可能是无 BOM 的 UTF-16 LE，显式指定编码。根据 dump 的模块基址计算 RVA，并核对实际 DLL、PDB 标识与指令；UPX 解包只处理隔离副本。`calls=0` 时旧 Migoto 日志不会更新，不能据此断言没有注入。
- Context 的 COM 虚表可能被 Bridge 按标准接口长度复制，Runtime 私有方法不得依赖追加的虚表尾部。内部帧分析派发使用对象成员，不能靠关闭 hunting 规避。回归 `runtime/tests/context-interop`，同时检查 Runtime 对象身份、进程退出码和完成标记；只用 GetProcAddress 或 DLL 载入夹具可能绕过实际 Runtime，完整组件未确认身份时不得计为通过。独立接口测试仍不能替代游戏内 NR 验收。
- Runtime 的成员指针为空时先核对调用者传入的对象身份。Bridge 等组件不能把首个 COM 对象的方法全局复用于系统设备、Runtime 包装设备和显卡辅助设备；所有受影响的转发都应按对象保存。最终 Release 后清理该代登记，处理地址复用及第三方再次复制虚表；不能只给 Runtime 加空指针返回。回归 `native/SSMT-Gimi-Bridge` 的不同 COM 实现和真实 Runtime/系统 WARP 混合设备，确认旧实现可复现同一故障、修复后通过。旁置组件哈希只检查损坏，不能替代完整插件包的来源校验。
- 第三方二进制的版本、导出及实际实现应与源码对应；集成包文档的 commit 也可能指向另一套后端。先核对源码历史再选择固定基线，补丁和归档哈希随构建入口维护，不能顺手切换渲染架构。插件辅助 DLL 不应伪装为 Plugin ABI runtimePlugins；随已验证插件包分发，启用后再暂存到专用副本，保留原始下载资源和无插件启动路径。
- 原神切换 FSR2 后若在 OptiScaler 空参数表崩溃，核对实际调用函数的 DX11/DX12 分支。`Dx11Upscaler=dlss_12` 只是输出后端，模板 `UseFsr2Dx11Inputs=auto` 默认仍是 DX12；准备时必须固定 FSR2 DX11 输入、关闭 Vulkan 输入，并补齐单 Host 与设备虚表兼容项。以真实组件的旧配置复现同一 RVA，再验证修复配置 Create/Dispatch/Destroy；系统设备夹具不代表 Runtime 共存或 NR 验收通过。无 Present 的夹具不能耗尽等待 Present 回收的环形缓冲。
- 热注入所需的环境必须在提权后的 Run.exe 内显式设置，再由挂起创建的目标继承；不能依赖 SSMT 环境跨 ShellExecute 提权继承。先加载 SSMT Runtime，再按计划加载 ReShade Host、Bridge、OptiScaler、额外 DLL 和可选 PluginHost，最后恢复目标线程。只为已安装且按游戏启用的插件准备资源，禁止再启动第二个 ReShade 注入器。
- 只开 Shade 与开启 DLSS5 必须分别核对实际队列，不能因组合路径用了预加载就认定单插件路径也已覆盖。直接创建目标时共用 Run.exe 的系统 D3D11 → Runtime → ReShade 顺序，不能继续用独立 inject.exe 或强制 Runtime 延迟来争抢时序。Shade 准备失败只跳过本次贡献，不回落到已知冲突的独立注入器；Shade 单独启用不修改 DLSS 输入契约。
- D3D11 挂起预加载前先在目标进程加载系统 D3D11，再加载 Runtime。Runtime 的 DllMain 仅能挂钩已存在模块；Runtime 先到可能被当作主代理，晚加载的系统入口会漏掉。验证使用不链接 d3d11.lib 的独立目标，检查系统模块顺序及实际 Context 身份；只在 main 中手动调整加载顺序不足以代表远程预加载。CBT / 预加载的最终检测均须比较实际文件身份。
- 初始 Device / Context 已包装不代表后续调用仍经过 Runtime。检查交换链 `GetDevice`、Device1 / Context1 等取回路径，并用 INI 命令改变渲染结果、回读像素；只看 DLL、私有 GUID 或 Present 不足以证明替换已执行。优先复用已有设备的 QueryInterface 与引用管理，避免临时创建另一套包装。远程夹具在恢复屏障处取得进程句柄，同时检查退出码和完成标记；共用注入器互斥量的用例须串行运行。
- `IDXGISwapChain4` 是 `IDXGISwapChain3` 的扩展接口，不能把原生 v4 指针直接交给调用方，否则会绕过包装器的 Present 虚表。让同一个交换链包装器继承 v4，并只保存底层 v4 的借用指针转发 `SetHDRMetaData`；回归应检查 v4/v3 的 `IUnknown` 身份、`Present(TEST)` 和 HDR 元数据调用。
- 图形自动回归默认明确选用 WARP，硬件测试须显式选择。ReShade 会跳过 WARP 的部分代理，不能将它当作完整硬件共存证据。测试若伴随 nvlddmkm 153 等驱动异常，关联当次时间与日志并停止重复触发；超时、联网初始化和驱动事件的因果关系应分别验证，不能直接推断指针冲突或仅归因网络。用户只要求将其作为信号时，不扩展为驱动排障任务。
- 复制组件前核对固定版本及显卡 Profile 的完整哈希，使用 SSMT 管理的副本；绝不为了复用组件包而替换其 GIMI DLL。组合配置中第三方 FX 的相对路径应基于实际资源所有者转为绝对路径，Windows canonicalize 的 verbatim 前缀不能直接与第三方正斜杠路径拼接。保留用户禁用的非必要 Add-on，插件关闭后清除它的本次贡献。
- 插件设置应能从插件市场找到。组件由设置中的显式操作自动下载，离线导入作为补充；先核验实际可下载的完整 Profile，再固化 URL、版本、大小和哈希，不能把不含 NR 的仓库 ZIP 当成完整安装。复用资源缓存，在新自有目录准备后再切换设置，失败保留旧组件；普通启动不等待下载，可选组件失败只横幅说明本次未加载，不改变保存的插件启用状态。7z 固实压缩必须读完跳过的项，只将需要的项写入固定目标。切换游戏时丢弃旧资源状态/进度，回归入口为 `scripts/tests/test-plugin-graphics-settings.mjs`。
- 目标必须匹配完整 EXE 文件身份。系统 `d3d11.dll` 与自定义 Runtime 同名，模块验证也必须匹配实际文件；枚举被拒绝不能报告 `runtime_detected`。
- DLSS5 `0.2.1+` 插件包的旧清单可能仍声明 `dlss5-swapper`、`dlss5-adapter`、`dlss5-payload`，这些仅对该插件版本及 ID 组合视为可选集成；旧冷路线版本、其他插件或依赖仍执行原预检，不能改写已安装清单。最后运行状态应从当前路线实际使用的 ReShade 日志读取；`calls=0` 的 Migoto 日志不代表未注入，Feature 18 评估记录也不等于 NR 持续生效或 Mod 画面通过。发现深度源尺寸与输出不符时，提示用户检查 ReShade Generic Depth 候选项。
- 外部创建的进程不归注入器所有。失败时不得结束游戏；远程线程超时后不能释放仍可能被读取的参数或结束远程线程。
- 保留额外 DLL 顺序与已启用 PluginHost 的启动入口；没有插件时不要求插件文件。真实插件组合与测试 Host 是不同证据。
- 运行时部署从应用资源取得 SSMT Runtime，设置页有意保留 XXMI 上游只读 Changelog，不重复撰写上游说明；阅读 Changelog 不得触发上游 DLL 安装。更新失败用非模态横幅并保留已有文件；首次安装无旧文件才停止。复制校验与 UPX 都在同目录暂存文件完成，最后一次 rename 替换，不能预先删除旧文件或随后再覆盖注入器。回归入口为 `scripts/tests/test-runtime-sync.mjs`；纯净启动跳过这些部署。
- 先按调用链区分 GIMI Core Package 的正常更新详情、XXMI 上游只读 Changelog 与 Runtime DLL 部署；上游说明中出现“XXMI Launcher”不能证明启动了旧 DLL 更新流程，不因此删除正常 Core Package 更新入口。
- 用独立 `SSMT_INJECTOR_REQUIRE_ADMIN=OFF` 的夹具构建验证真实进程注入，正式目标继续使用管理员 manifest。脚本面向 Windows PowerShell 5 时保留 UTF-8 BOM，并显式按 UTF-8 读取 JSONL。
- 没有游戏时交付可执行的原生目标、日志脚本和测试步骤；区分旧客户端通过 CLI 选择新模式与新前端的集成路径。不要把夹具加载成功当作反作弊、图形初始化或 Mod 渲染已验收。
- 主仓库与 Native 子仓库分别检查 diff；仅在用户要求提交时，先提交 Native 源码，再提交主仓库及子模块指针。不运行主应用发布包构建脚本。

只将已证实、可复用的经验留在本 skill；单次 PID、个人游戏路径、哈希和测试输出放在诊断报告或忽略的构建目录。

- 扩展 DLSS5 游戏时先区分无原生输入、DX11 原生输入与 DX12 原生输入，再按真实 `gamePreset` 和 EXE 联合选择 Feeder / DX11 Bridge / 原生 consumer。Feeder 必须配齐运动矢量 FX、公共 include 及明确的 technique 顺序；不能把 GIMI FSR2 Bridge 套到其他游戏。下载时校验归档及提取项，ReShade 安装程序仅读取固定哈希对应的内嵌 ZIP，绝不执行。共用一个热注入 Host 与启用贡献组合器，状态检查失败也必须保留重新下载入口。离线实组件提取、接口共存、游戏内成功 NR 帧分开验收。
- 同一游戏有多个 DLSS 实现时，选择应按实例持久保存，完整安装成功后再原子切换安装记录与路线。旧配置继续原路线；各实现使用各自配套组件，不能同时注入 Feeder 与 FSR2 Bridge / OptiScaler。界面展示所选实现的独立组件状态与抗锯齿要求。
- 抗锯齿输入契约应在实际采用热注入计划、通过启动方式与 DLL 冲突检查后自动配置。原神 AA 同步 graphicsData key 8 与 globalPerfData entryType 8，两者相差一；Bridge 选择 FSR2，Feeder 关闭游戏抗锯齿。复用原始记录备份与并发比较，未知格式不猜测，失败只跳过可选 DLSS5；纯净、关闭插件、缺失组件和手动 / 外部启动路径不改 AA。不要把插件输入契约绑到普通 Mod 的画质配置开关。
- ReShade 反复显示首次教程时，核对准备阶段是否每次覆写 `OVERLAY.TutorialProgress`，不能只强制跳过教程。保存上次生成的默认值，与运行后 INI 比较以保留用户状态；必要 Host 路径、输入契约和效果顺序仍由本次贡献决定。验证旧配置迁移、重复保存、用户禁用可选效果及关闭插件后的贡献移除。
- 从旧独立注入器迁移到自有配置时，先比较游戏目录与声明资源目录的 INI。独立注入器可能清空 DisabledAddons，不能把它当作用户主动启用全部 Add-on；仅迁移教程、界面、按键和截图状态，Add-on 默认贡献仍取自声明资源。可用真实资源执行离线准备检查，确认源文件不变；该检查不加载图形组件，不能代替游戏内共存验收。
- Player Tweaks 的插件兼容范围与具体功能分开声明，新增游戏不能直接注入另一款游戏的 Hook DLL。客户端设置型增强按真实游戏元信息选择区域，先备份、记录所有权，再比较当前值后写入；恢复只改仍由插件拥有的字段，保留玩家其余设置和后续改动，共用客户端在最后一位使用者关闭后才恢复。先用真实配置做只读解析 / 内存往返检查，注册表值写入成功也不能等同于游戏内 FPS 实测。
- 启用独立 ReShade Host 前检查旧安装的入口和配置优先级。ReShade 的游戏目录 `INSTALL.BasePath` 优先于环境变量，旧图形代理也可能再次加载 Host；未知所有者的文件不能自动覆盖或删除，提示通过原管理器恢复并保留普通启动。插件关闭时不执行这类专有扫描。
- 审查 RHI / WWMI 集成时分别记录历史游戏目录部署、专用脚本和 SSMT 自有目录热注入的验收。原提交不在当前祖先链时，用具体源码及 patch-id 检查是否已等价合入；另一款游戏恢复 Mod 不代表鸣潮 NR 通过。停用、完整清理、版本回退和中断恢复分别验证，`.original` 备份不能替代所有权比较与整体事务。证据与边界见 [鸣潮 DLSS5 与插件边界审查](../../docs/dlss5-wwmi-plugin-review.md)。
- 插件目录位于本体不代表完成独立插件化。通用校验、执行器和 COM 兼容修复保留在本体 / Runtime；游戏路线、组件基线和界面应有可交付的贡献契约。检查通用协调器是否按固定插件 ID / 版本处理依赖，并检查所有可选资源冲突是否进入局部回退，不能只验证下载失败分支。
- 自有配置与 Add-on 更新先完成纯组合和所有权检查，再以多文件事务提交。进程中断后先恢复再读取当前状态；恢复逐项比较原值 / 已写值，遇到玩家后续修改保留文件及记录。API 开关只覆盖实际采用计划的本次 INI，不写回玩家保存配置；验证额外 DLL 冲突回退发生在画质修改之前。
- 声明式迁移应同时验证真实包内数据、消费入口与旧包兼容。图形候选失败后由统一收集器继续选择，前端不列举插件优先级；共享效果作为贡献合并。设置组件键必须绑定已注册 provider 与所有者，不能让其他包借用键获得专有操作入口。Player Tweaks 的设置后端与游戏专用 Hook 必须分别选择，包内存在 DLL 不意味着所有支持游戏都应注入。
