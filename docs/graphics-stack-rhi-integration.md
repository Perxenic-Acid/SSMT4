# 图形插件组合与 SSMT 资源管理

## 实施进度（2026-10-03）

- 官方插件包及其 SHA-256 目录由 Native 仓库的 Release 资产提供；Native 仓库新增打包脚本与 Release 工作流。当前该仓库尚无 Release，SSMT 会显示空官方目录，本地第三方插件安装仍可使用。
- 第三方 `.ssmtpkg` 先做压缩包结构、路径、大小、清单与贡献文件检查，展示声明权限和免责声明；确认后按检查时的 SHA-256 安装。该检查不等于恶意代码审计。
- SSMT 已有通用 HTTPS、大小、SHA-256 校验与缓存的资源下载基础，官方包下载使用该基础。旧 DLSS5 Swapper 桥接包暂不发布为官方资产，避免误认为 SSMT 已管理 DLSS5 DLL。
- 还未实现 DLSS5 DLL 的可信资源清单、逐游戏安装事务、更新/卸载和 CXP GIMI Runtime 移植；因此当前不能宣称 DLSS5 集成完成，也未做新的游戏热测。
- 启动准备现在核对 DLSS5 安装记录声明的文件；若已安装路线缺文件，调用现有 Swapper 同路线安装入口修复并复查。本机原神曾出现 Feeder 两个 add-on 缺失而记录仍在；已用配置的 payload 实际重装并核对哈希。此验证只覆盖安装文件，不代表 DLSS5 渲染热测通过。

## 已核对的现状

### 2026-10-05 原神单项注入与游戏内踢出

10 月 4 日 23:36 的隔离测试使用当前 SSMT Runtime `d3d11.dll`（SHA-256 `DEC7708F054FA8A8633EBAD727447156AB01FD3D69F15DC9B8F8B65BC69F5E07`）、当前 Native `Run.exe`、Player Tweaks，以及 SSMT3 的 GIMI Core/ShaderFixes 和唯一指向 `D:\GIMods` 的 `Mods` 链接；没有 ReShade 或 DLSS5。`Run.exe` 事件与 `d3d11_log.txt` 证明 Runtime 已加载，Player Tweaks 日志记录了 FPS `60 -> 120`。用户进入世界，开门加载明显慢，随后遇到游戏内“非法工具”踢出；虽然当时没记下弹窗错误码，原神 `LocalLog.log` 在 23:41:13 同样记录了 `Disconnect ... reason=10612`。不能把此结果称为 Player Tweaks 单独运行，因为 3DMigoto Runtime 同时存在。

10 月 5 日 09:15 的对照只移除 Player Tweaks，保留同一当前 Runtime、Core/ShaderFixes、Mod 和启动器；`runtime_detected` 事件与持续写入的 `d3d11_log.txt` 证明 3DMigoto 确实注入。用户在游戏内遇到 `10612-4001`；原神自己的 `LocalLog.log` 于 09:18:46 记录 `Disconnect ... reason=10612`，Runtime 日志最后写入于 09:19:15，客户端于 09:19:51 正常退出，没有同次 Windows 应用崩溃或 Unity 转储。故 Player Tweaks 不是这次断连的必要条件；日志不能证明触发判定的具体函数、Mod 或组件。游戏内弹窗与先前独立窗口 `(2,0,4)` 也不能合并为一种故障。Hook 掉提示无法恢复已经断开的游戏会话，不作为修复或验收方式。

09:27 的对照只把 Runtime 换成 SSMT3 已用 `d3d11.dll`（SHA-256 `5A43BD93EB8932ED1220F6C2005B8A7772A9941F76335823F01B17BD478B4B3D`），其余文件保持相同；游戏约半分钟后闪退。该次 Unity 转储定位到 `NvPresent64.dll` 空指针读取 `0xc0000005`，与 10 月 4 日 23:10 的 Ctrl 纯净启动崩溃具有同一指令位置和异常类型。因此旧 Runtime 对照没有存活到可比较游戏内踢出的时间窗口，不能据此认定旧 Runtime 不触发 `10612-4001`，也不能把此 NVIDIA Present 崩溃归因于 SSMT 注入。测试使用隔离目录，没有覆盖游戏安装目录的 DLL。

09:38 再保持当前 Runtime、Run.exe、Core/ShaderFixes，仅将隔离目录的 `Mods` 换为空目录，未触碰 `D:\GIMods`。游戏于 09:38:35 在 `NvPresent64.dll` 同一指令位置空指针读取 `0xc0000005`，Runtime 日志最后停在交换链 `ResizeBuffers` 后；没有进入可比较 `10612` 的运行区间。这个结果说明在当前机器上，早期 NVIDIA Present 崩溃会干扰 Mod 内容的差分测试，不能把本轮没有 `10612` 解读为“清空 Mod 修复了踢出”。

逐个解析 23:10 Ctrl 纯净、09:27 SSMT3 Runtime、09:38 当前 Runtime 且空 Mod 的 Unity `crash.dmp`，三次异常地址均为 `NvPresent64.dll` 模块内 RVA `0x3EA24`；纯净启动转储的模块表没有 SSMT/3DMigoto DLL。此共同崩溃点发生于不同注入条件，不能仅凭崩溃模块判定 NVIDIA 自身、游戏传入对象或其他显示层谁应修复，但足以说明“只有 SSMT 注入才会发生这次早期访问冲突”的假设不成立。

为核对无托管插件的代码回归，将 Runtime 托管 ReShade 改动前的提交 `e6144973` 在独立 worktree 构建为 x64 Release；当前 `bf0993aa` 与该构建分别运行 `tests/dxgi-interop` 的 `mod` 用例，均得到 `CreateDevice=0`、`DuplicateOutput=0` 和退出码 0。当前源码的 Hosted ReShade 初始化由环境变量控制，以上单项游戏测试没有设置该环境变量，日志也未进入新增的辅助设备包装路径。此桌面复制测试不执行游戏交换链、网络连接或 `10612` 判定，只能证明基础 D3D11/DXGI 接口在两版中可用。

09:51 用户从 SSMT 正常点击启动（未按 Ctrl）；游戏进程运行约 57 秒后由客户端记录 `Call QuitApplication` 正常退出，没有同次崩溃转储。GIMI `d3dx.ini` 在 09:51:54 更新，但 `d3d11_log.txt` 仍停在前一晚 23:09，NUMPAD0 无反应；本轮不能视为 3DMigoto 或 Player Tweaks 已加载。当前设置虽为 `ctrl-pure`，用户明确没有按 Ctrl，因此纯净分支不能解释这个结果。SSMT 的 `launch_programs` 只检查 PowerShell 的 `Start-Process` 是否成功，并在发现游戏进程后返回成功，没有核实 Run.exe 的 `runtime_detected` 或退出码；这是可证实的注入成功状态误报，不足以单独确定注入失败的原因。

09:59 用户再次从 SSMT 启动；`YuanShen.exe` 的父进程确认为 GIMI 目录的 `Run.exe`，`d3d11_log.txt` 于 09:59:26 写入本轮初始化与 Runtime 路径，Player Tweaks 日志同期记录 FPS `60 -> 120`，证明这次两个 DLL 都已加载。用户主动在登录阶段关闭游戏，客户端 09:59:45 记录 `Call QuitApplication`；尚未进入 5 分钟 `10612` 观察窗口。用户未看到 Mod 或 NUMPAD0 效果；本次 Runtime 日志没有 `ReloadConfig` 或 Core 载入记录，而当前 `skip_early_includes_load=1` 会将 Core/Mod 加载推迟到帧处理，因此不能将 DLL 加载等同于 Mod 功能已就绪。两轮启动不能混作同一条注入链。

10:12 的 SSMT 原配置对照使用资源目录中的旧 `Run.exe` 和旧 Runtime，Player Tweaks 确实记下 FPS Hook；用户进入世界并运行超过七分钟，原神日志持续有帧计数，没有新的 `10612`。但 `d3d11_log.txt` 一直没有 `ReloadConfig` 或 Core 载入，用户判断 3DMigoto 未正确工作，因此此轮不能作为正常 3DMigoto 的稳定性证明。10:20 的隔离短测只把旧 `Run.exe` 搭配当前 Runtime，未加载 Player Tweaks。旧 Run 输出 `Module snapshot failed: 299`、`Module verification denied; assuming success`，却仍发送 `runtime_detected`/`launch_complete`；Runtime 自己的日志确实证明该轮 DLL 已进入进程，但短测内也没有 `ReloadConfig`，游戏尚未进入世界，不能用于判断 Mod 是否生效。按用户要求，后续非游戏内弹窗诊断只记录约 30 秒；这轮由 agent 启动的 PID 31032 已单独结束，未触碰游戏安装目录。当前 Native 源码新构建的 `Run.exe` 位于 `native/build/3Dmigoto-Injector-V2/Release/Run.exe`，尚未部署；SSMT 资源目录的旧版不能代表现有 Native 源码的状态报告行为。

10:29 的源码追查指出具体的无插件帧处理绕过：`HackerSwapChain::QueryInterface(IDXGISwapChain3)` 只在托管 ReShade 启用时返回包装器，其他情况下向原神交出原生 v3 指针。10:12 的旧 Runtime 日志明确出现这个 v3 查询并记录“without wrapper”；游戏持续渲染，但包装器没有 Present，因此 `skip_early_includes_load=1` 的 Core/Mod 重载永远不执行。当前 Runtime 改为对受支持的 v3 总是保留包装器；v2 仍保持既有兼容条件。10:29 的首轮隔离日志随即出现 `Reloading d3dx.ini`，证明帧处理恢复，但读取完整 Mod 时在 10:29:09 发生 WER `BEX64` / `0xc0000409`，不能把这一半修复当成验收。

对崩溃配置作了仅用于定位的目录对照：空 Mods、仅单份 Monsoon `merged.ini`、整个 Monsoon 目录、Discord 的其余顶层目录分别都能完成 Core 加载并持续 Present；完整 Discord 或仅保留原有 `Mods\Discord\[monsoon]...` 逻辑目录层级都会在同一条 `MondstadtNpcShortsHeadDiffuse.dds` 资源处失败。该文件的规范化逻辑路径为 259 字符，原 `ParseResourceSection` 把包含 `\.\` 的原始路径连续 `wcscat` 到 `wchar_t path[MAX_PATH]`，长度越过 260 字符缓冲区。源码已改为动态字符串拼接、`GetFullPathNameW` 规范化，以及长路径的 `\\?\` 前缀；DLL 自身路径读取也改为动态缓冲。修复后的同层级 Monsoon 和完整 `D:\GIMods` 都完成配置解析并持续 Present 超过 30 秒，完整 Mod 日志包含 Discord 与 GameBanana 大量资源，没有新的 WER 崩溃。最终源码的 x64 Release 编译通过，完整 Mod 短测再次通过。此结果只证实配置和帧处理链；贴图画面、快捷键及 `10612` 的长期结果仍需进世界核对。

当前 SSMT 前端的 `dev` DLL 更新来源仍是上游 `SpectrumQT/XXMI-Libs-Package`。本机 Tauri debug 资源中的 `d3d11.dev.dll` 与 SSMT3 GIMI 安装目录的 `d3d11.dll` SHA-256 均为 `5A43BD93EB8932ED1220F6C2005B8A7772A9941F76335823F01B17BD478B4B3D`；`src-tauri/resources/Run.exe`、Tauri debug 资源及 SSMT3 安装目录的 `Run.exe` SHA-256 均为 `6AC37E40ADDD33CCCBB4C9AF75A17AD36E84C48F652187ABAF842A25C5839C25`，早于隔离测试使用的当前 Native `Run.exe`。因此上述隔离结果不能直接描述用户从现有 SSMT 界面启动时加载的二进制。切换默认更新源到 SSMT Runtime 前，须先解决其单独注入时的游戏内踢出，并建立可核对的发布资产和运行时哈希。下一步须比较同一注入方式下可稳定运行的基线；不能用屏蔽弹窗或禁用某个组合充当共存修复。

10:52 的完整 Mod 实机对照中，用户确认 NUMPAD0 恢复，但按贴图哈希替换的 Mod 再次部分消失、部分拉伸，并由用户主动关闭游戏。日志显示 1048 次 `Device mismatch, transferring`。进一步追查发现 `RegisterFrameActionOwner` 只在 Hosted ReShade 启用时才把辅助交换链的帧动作交给游戏主设备；本轮不加载 ReShade，虽然 v3 包装恢复了 Present 和 Core 加载，资源仍在辅助设备上初始化。头像贴图有 12 次先查根目录 `Sides` 失败，但紧接着回退到各自的 `Mods` 路径成功加载，这些告警不能解释整体贴图异常。现已让设备归属绑定不依赖 Hosted ReShade，x64 Release 重新编译通过。10:58 新隔离运行的日志确认辅助链使用游戏主交换链，并完成 `Reloading d3dx.ini`；截至 11:00 跨设备资源搬运为零。仍需用户进世界确认贴图画面。

### 原神当前热测的效果判定（2026-10-03）

本机原神最近一次 `ReShade.log` 确认 HoYoShade 加载了 `dlss5-feed.addon64` 与 RenoDX 插件，但 Feeder 随后报告：DX11 设备不支持当前要求的多线程保护，且 Present 路径已有插层，因此主动停止。游戏中的 RenoDX 状态区显示 `NO DLSS CREATE SEEN`、零 DLSS 创建/评估及零成功 NR 帧。恢复 DLL 和补齐 add-on 只修复了安装完整性，**此次神经渲染未生效**。SSMT 设置页现将安装记录与最近一次运行日志的判定分开显示；只有日志确认持续的 NR 成功帧才能验收实际效果。

RHI 的 `DLSS5 Tool + DX11 Bridge` 面向已有原生 DLSS 的 DX11/Vulkan 游戏；用户确认鸣潮测试使用了 DX11 桥接，但具体 NR 方法未确认。原神没有原生 DLSS，不能把鸣潮的 Bridge 路线直接当成原神方案。CXP 的 GIMI 专用链以 FSR2 carrier、Dx11FsrBridge 和修改后的 OptiScaler 补出 DLSS 调用，再由纯 add-on `nr-before-sr.zh-CN.addon64` 执行 Feature 18；该模式不使用 `DLSS5_Feed.fx`。它还依赖修改后的 GIMI、Hosted ReShade 和匹配显卡的 NR Runtime。其 3DMigoto 补丁与 SSMT Runtime 当前 `HackerDXGI.cpp` 不可直接套用，须按现有 Present/PluginHost 结构移植并完成游戏热测。

### 2026-10-04 原神 `(2,0,4)` 再现

本次 08:24—08:26 的运行仍是 SSMT 托管的 Swapper `feeder` 路线：游戏安装记录为 `route=feeder`；`ReShade.log` 显示 HoYoShade 的 ReShade 6.8 加载 `dlss5-feed.addon64` 与 RenoDX v4.7，之后才编译 `DLSS5_Feed.fx`。`dlss5-feed.log` 显示插件实际开始送帧、创建 DX12/NGX 会话，接着因 DX11 多线程保护不可用且进程内有 NVIDIA Smooth Motion Present 插层而停止。Windows Application Error 在 08:26:13 记录 `YuanShen.exe` 异常 `0xc0000409`；这与用户看到的 `(2,0,4)` 弹窗同次发生，但日志不能证明 Feeder 停止就是客户端完整性弹窗的直接原因。

08:24 的运行**不是 CXP 的 GIMI 集成版**。其普通版 README 把 `(2,0,4)` 和 `(6,4,2502)` 明确列为仍可能遇到的错误并要求排查旧注入组件、组件混装、权限、系统、驱动等；GIMI 版 README 给出了不同的预加载链和 Feature 18 验证方式，但没有保证任意本机配置都能免于该弹窗。两者不能只凭名字或“插件已加载”视作等效。本机现有 Swapper NR Runtime 哈希 `8270B350…CC206` 与该项目 RTX50 已验证哈希 `E16BCF15…6E1FC8E` 不同，不能用前者的失败作为对已验证整合的反证。

GIMI 集成版 README 单独提供的 Google Drive Runtime 文件取得 165,840,496 字节，SHA-256 却为 `E67DEE209320CDAFE0E93E45675D7AA34323A53ACC57A72B2E40A181581C989A`，与 README 宣称的 `E16BCF15E16E13F527491CDF7845B2FE6521A738D8F7C9C721866A8496E1FC8E` 不同；本机 Authenticode 检查报 `HashMismatch`，未部署。随后从普通版 README 的 v1.3 完整压缩包取得与其公开校验值一致的 RTX50 Runtime：压缩包 SHA-256 `95A87BC1B5C981A7AFA63E9E18BB68410B1697D3F20108418AC9B7A4050388CF`，解出的 `nvngx_dlssnr.dll` SHA-256 `E16BCF15E16E13F527491CDF7845B2FE6521A738D8F7C9C721866A8496E1FC8E`、Authenticode `Valid`。按 GIMI 项目脚本严格校验并安装到隔离实验目录；未覆盖 SSMT 的 DLSS5 文件。

08:45 与 08:54 两次独立 CXP GIMI 启动均在进入游戏前崩溃。第二次使用隔离的 GIMI 目录，其 `Mods` 是指向 `D:\GIMods` 的目录链接，真实 Mod 目录 473 个 INI 的启动前后 SHA-256 均一致。为排除原游戏目录 HoYoShade `ReShade.ini` 抢占 CXP 配置，第二次仅在启动期间将游戏目录的 `ReShade.ini` 换为 CXP 托管配置，退出后按原始哈希恢复。`ReShade.log` 证实加载了 CXP 中文 `nr-before-sr.zh-CN.addon64`；Bridge 完成 FSR2 查询及 DX11 Device Hook，OptiScaler 捕获 D3D11 Device，GIMI 日志读取了 Mod 配置。但没有 Feature 1 Create/Evaluate、Feature 18 成功帧或最终 Present。两次 `mihoyocrash/error.log` 均记录 `d3d11.dll` 访问冲突 `0xc0000005`，在 08:45:46 和 08:55:17 出现；解析对应 `crash.dmp` 的模块表，两个异常地址分别落在 `C:\Windows\System32\d3d11.dll` 的 RVA `0x15D684`、`0x15D67D`，而不是 CXP GIMI 的同名 DLL。本机为 RTX 5060 Laptop / 驱动 617.14；上游公开的通过记录为 RTX 5080 / 616.56。故该本机独立对照**尚未验证 CXP 方案能稳定进入游戏，更不能证明其已解决 `(2,0,4)`**。崩溃发生于 NR 实际运行前；异常模块位置不足以判断是谁传入了无效对象，也不能推断是游戏完整性检测。后续须先定位这次早期崩溃，再谈 SSMT 移植或组合热测。

09:05 为定位早期崩溃，保留同一 CXP GIMI DLL 与 `D:\GIMods`，只在该次诊断启动时去掉 ReShade/Bridge/OptiScaler 的预加载项；这不是产品组合限制，也不是验收方案。游戏仍在进入画面前于交换链创建附近退出。新的 `crash.dmp` 指向 NVIDIA 驱动 `NvPresent64.dll` RVA `0x4898B` 的空指针读取，GIMI 日志停在 `CreateSwapChainForHwnd`；这提示 GIMI 包装器与本机 NVIDIA Present 插层之间也有兼容问题，但单凭转储不能确定修复责任。此次结束后启动器配置、游戏 `ReShade.ini` 均恢复，无测试游戏或 UnlockFPS 进程残留。

09:10 又用原始 `GIMI-Pure` 的 `d3d11.dll`（SHA-256 `516ECD5BE843D8E3499FC83DBE3E2A2C695549BE0F4094E9338CE73AA625DFF0`）和同一 `D:\GIMods` 做对照，未预加载 CXP ReShade/Bridge/OptiScaler。仍在进入游戏前崩溃，转储同样落在 `NvPresent64.dll` RVA `0x4898B`，并为同样的空指针读取。这使“CXP 修改版 GIMI 单独引发该 `NvPresent64` 崩溃”的解释不成立；可能还涉及本机当前驱动插层、旧版 GIMI 与当前游戏的兼容性或启动器环境，需进一步隔离。用户看到的 Windows “Minimized to tray” 通知只表明某辅助程序缩入托盘，不能据此判断游戏已进入。测试配置与 Mod 文件均未留下修改。

随后以 CXP 固定的 bo3b/3Dmigoto 源码和两份 GIMI 补丁构建隔离 DLL，并修复 SDR 交换链 `IDXGISwapChain3` 查询不返回原生接口的问题。改版日志确认查询成功，原先交换链初始化处的早期崩溃不再出现。此改动目前只在 `tmp/cxp-gimi-source-archive` 的忽略目录，尚未移植到 SSMT Runtime。带 CXP FPS Unlocker 的单 GIMI 对照在窗口出现约 77 秒后仍曾发生一次空地址写入崩溃；不能仅凭此归因 FPS Hook。

为分开验证启动编排和帧率 Hook，SSMT Native 注入器新增显式 `--preload-runtime`：先在挂起目标中加载 GIMI，再顺序加载声明的附加 DLL。09:56 的隔离完整链按 GIMI、ReShade、Bridge、OptiScaler 顺序完成注入，原神窗口持续 180 秒，Bridge 的 Draw Hook 与 OptiScaler Present 均有日志。此时没有注入 SSMT Player Tweaks 或 PluginHost；NR add-on 只记录初始化，没有 Feature 1/18 创建或成功帧。OptiScaler 的前 9052 次 Present 返回 0，之后 7376 次返回 Windows SDK 定义的 `DXGI_STATUS_OCCLUDED`（`0x087A0001`），故不能把 180 秒窗口存活等同于持续可见渲染，更不能宣称 DLSS5 生效。

10:09 的隔离完整链再加入 SSMT 自带 `SSMT-Player-Tweaks.dll`。首次尝试在注入 GIMI 时失败，发生于 Player Tweaks 加载前：`NtAllocateVirtualMemory` 把请求长度向上调整到 4096 字节，Native 注入器误将调整后的长度用作 DLL 路径写入长度，导致 `NtWriteVirtualMemory` 返回 `0x8000000D`、写入 0 字节。已在 Native 注入器中将分配长度和原始字符串长度分开，并重新编译。重测按 GIMI、ReShade、Bridge、OptiScaler、Player Tweaks 顺序注入；Tweaks 日志确认 FPS Hook 初始化和 `60 -> 120` 调整，OptiScaler 前几千次 Present 返回 0，游戏窗口持续 180 秒后由测试脚本关闭，未观察到该次崩溃。该次同样尚无 NR 成功帧；进游戏场景的延长验证仍在进行。CXP FPS Unlocker 同时拥有帧率功能，不能与 SSMT Tweaks 的 Hook 共存性仅凭本测试推断；SSMT 启动器可以承担其 DLL 预加载职责，避免两个启动器同时操作同一进程。

10:12 起的延长测试终于进入游戏场景。用户实测指出大量 IB 少渲、错渲；本次使用的是在隔离目录由 bo3b/3Dmigoto 固定源码和 CXP 两份补丁编译出的 GIMI DLL，**不是**继承 XXMI 的 SSMT Runtime。该次隔离配置仅递归加载 `Mods`，遗漏了 SSMT GIMI `Core\GIMI\main.ini`，因此不能把 IB 异常单独归因于 DLL。10:16 已停止测试；脚本恢复隔离 GIMI DLL、INI、原神 `ReShade.ini` 和测试用 Player Tweaks DLL，确认没有游戏进程。后续必须以 SSMT Runtime 为基底移植必需的 Hosted ReShade、原生 Device/Context 桥接，并实测 IB 与 NR 同时正确；不能把这份 CXP/bo3b DLL 发给 SSMT 用户。

11:12 使用 CXP/bo3b 补丁 DLL、完整 SSMT GIMI Core 配置、同一 `D:\GIMods`，再加入 ReShade、Bridge、OptiScaler 与 Player Tweaks 对照。用户报告游戏逻辑继续但画面卡死。OptiScaler 日志显示第 1429 帧后进入全屏状态和 `ResizeBuffers` 变更，第二次 `ResizeBuffers` 返回 `0x887A0001`，随后第一次 Present 返回 `0x887A0005`（`DXGI_ERROR_DEVICE_REMOVED`），该错误继续出现至自动结束。没有 Feature 1/18 成功帧。故完整 Core 配置没有使这条组合链可用，也不能用进程存活宣称渲染成功。测试结束后隔离 DLL/INI、游戏 `ReShade.ini` 均按原始 SHA-256 恢复。

11:19 再单独运行同一 CXP/bo3b DLL 与完整 SSMT Core，不注入 ReShade、Bridge、OptiScaler、Player Tweaks。隔离目录仅有一条 `Mods` junction 指向 `D:\GIMods`，其下 473 个 INI 且无额外目录链接；`d3dx.ini` 包含 `Core\GIMI\main.ini`，Core 文件存在。用户仍观察到 Mod 像重复、快捷键似乎未生效。这些文件核对只能证明配置路径与文件存在，尚不能证明 Core 逻辑在该 DLL 中正确执行，也不能证明重复是 Mod 被读取两次。游戏在观察上限前退出，未找到同次崩溃转储或 Windows Application Error；该对照未通过 IB/快捷键验收。

11:23 使用已知正常的 SSMT3/XXMI Runtime DLL（SHA-256 `5A43BD93…84B4B3D`）与相同 SSMT Core、`D:\GIMods` 做单独对照。日志中的角色根顶点着色器哈希 `653c63ba4a73ca8b` 替换来自隔离目录当时残留的 CXP `ShaderFixes`，不能证明 Core 已执行；用户随后确认画面像纯净游戏、无 Mod 和快捷键。SSMT Runtime 的 Hosted ReShade 与原生 Device/Context 桥接已局部移植并能编译，但与当前 OptiScaler 同场的隔离热测在首帧附近于 NVIDIA 驱动 `nvwgf2umx.dll` RVA `0x5B62EF` 崩溃；未修改的 SSMT3 Runtime 与相同 OptiScaler 对照也在该 RVA 崩溃，因此不能把此崩溃归因于移植补丁。只运行 SSMT3 Runtime 与 Player Tweaks 则保持响应。当前不存在同时通过 IB、Present 和 NR 成功帧的本机原神结果。

12:19 的新隔离对照使用 SSMT Runtime、与 SSMT3 哈希相同的 GIMI Core、原版 `ShaderFixes`、持久配置和单一 `D:\GIMods` 链接；为排除 ReShade，只启用交换链诊断包装而故意指定不存在的 ReShade DLL。此前同源配置的热测中，用户仍看到按贴图哈希替换的 Mod 消失或拉伸；这已在无 ReShade 情况下重现，不能归因于 HoYoShade。另一方面，修复托管 ReShade 配置的 `EffectSearchPaths` 等路径后，日志确认 HoYoShade FX 被编译，用户确认完整 FX 列表已出现。

交换链日志显示两个 D3D11 设备：游戏原先包装的主设备先创建，但其交换链后创建；NVIDIA Present 插层另建的设备生成辅助交换链，并由它执行实际的 `Present`。此前直接在辅助交换链运行帧动作，首次延迟 `ReloadConfig` 随之使用辅助设备，这是贴图资源设备归属错误的强嫌疑。现在运行时在创建交换链时识别游戏主设备，把辅助链的帧动作转给主链。12:19 日志确认辅助链 `000001F1F4857E50` 绑定主链 `000001F1F4858260`，首帧重载配置前 `GetHackerContext` 返回主设备 context `000001F1F48B26C0`；没有出现设备移除错误。日志里仍有 269 条用户 Mod 的重复节告警；已知正常的 SSMT3 08:24 日志也有 268 条，单凭这些告警无法解释此次贴图异常。测试在 100 秒上限结束，临时 DLL 和配置已恢复。该轮用户不在电脑前，尚无进入世界后的贴图画面复核，因此只能判定资源初始化设备链路已修正，不能判定贴图 Mod 最终正确。

13:03 再用修正后的 Runtime、SSMT3 原版 Core/ShaderFixes 和单一 `D:\GIMods` 运行，不加载 ReShade/Bridge/OptiScaler；用户进入世界后确认贴图哈希 Mod 正常。13:07 加入托管 ReShade 后，用户确认贴图等其他效果正常；日志有 1,756 条 HoYoShade 着色器编译结果。该轮 CXP 托管配置仍让 NR add-on 注册，尽管 `DisabledAddons` 列出了它；因此不能把这次称为纯 ReShade 对照。未加载 Bridge/OptiScaler 时，插件界面显示等待兼容的 DLSS 帧，也确实没有 NR 效果。两轮隔离测试结束后 DLL 和配置均按原始哈希恢复。

13:13 首次完整链（3DMigoto、ReShade、Dx11FsrBridge、OptiScaler、NR add-on）持续 Present，但 Bridge 只有 FSR2 六个接口的查询，没有创建 FSR2 转译上下文；NR 插件等待 DLSS 帧。用户随后确认当时未把游戏图像设置为 FSR2 抗锯齿且渲染精度低于 1。13:17 重开完整链并调整游戏设置后，Bridge 于 13:19:33 记录 `fsr2_translation_context_created render=1280x800 output=2560x1600 detoured=1` 和 dispatch 成功；NR add-on 至少记录 `NR-before-SR evaluate succeeded` 第 1、2、3 次，原始 DLSS 超分继续被评估，用户确认效果出现。日志只明确给出前三次 NR 成功，不能据此宣称更长时段的持续成功率。该次进程退出后隔离文件已恢复。此结果证明本机原神的完整技术链可以工作，但仍依赖手工准备的本机资源、隔离启动脚本与游戏 FSR2 设置，不能当作 SSMT 插件市场安装、启停和卸载流程已经交付。

- `CXP-2024/dlss5_for_genshinimpact` 的普通包自带一整套运行组件；需要 GIMI/3DMigoto 的用户被引导到独立的 GIMI 集成项目。后者通过 GIMI 持有 DX11 包装器和最终 Present，并在同一链路中托管 ReShade、DLSS SR 和 NR。这是针对原神 DX11 与修改过的 GIMI/ReShade/OptiScaler 的实现，不能作为任意游戏的现成组合规则。
- RHI 的 Feeder 安装涉及 ReShade、Feeder add-on、NR consumer、`nvngx_dlss.dll`、`nvngx_dlssnr.dll`、shader 和预设。RHI 用 `rhi_install.txt` 及内部记录跟踪组件和共享文件所有权。
- 当前 RHI `App.OnLaunched` 接受 add-on 文件、`--nxm`、`--launch`、`--minimized`，没有供 SSMT 调用的安装、检查、卸载命令行协议。Feeder 的完整编排在 `DetailPanelBuilder.NeuralRendering.cs` 的 UI 回调中。
- SSMT 的 `scripts/start-wuwa-dlss5-mod.ps1` 假设 RHI 已将 ReShade 和两个 add-on 放进鸣潮游戏目录，只负责启动前复制代理并设置 WWMI 注入。它不能替代安装器，也不能安全地切换纯净游戏、ReShade、DLSS5、3DMigoto 等组合。
- SSMT 当前 `managed_stack::install_route` 把 HoYoShade 作为所有托管 DLSS5 Route 的前提。RHI/鸣潮路线不能复用这个前提；图形组件应分别声明所需宿主和资源。

## 两个候选项目的评估（2026-10-03）

### `dlss5_for_genshinimpact`：原神专用链路的参考实现

普通版的 v1.1 是输出分辨率后置 NR，v1.3 是默认前置 NR 再由 DLSS SR 放大，同时保留后置模式。其 `configure_and_start.ps1` 按固定顺序交给 FPS Unlocker 加载 ReShade、`Dx11FsrBridge.dll`、OptiScaler，并写入游戏目录 `ReShade.ini` 及包内 `OptiScaler.ini`、`fps_config.json`。这是有状态的整套启动方案，不是一个可以直接挂到现有 SSMT Run.exe 后面的 Add-on。普通版 README 明确要求 GIMI/3DMigoto 用户使用另一个 `GIMI_reshade_dlss_integration` 仓库，两个包不能混装。

GIMI 集成版值得重点研究其资源与 Hook 所有权设计：它让修改过的 GIMI 持有 DX11 包装器和最终 Present，使用修改过的 Hosted ReShade、OptiScaler 与 FPS Unlocker。作者公布了在特定 RTX 5080/驱动环境下 GIMI Mod、ReShade 和 Feature 18 持续运行的记录；这不能直接证明当前 SSMT Runtime 在本机国服原神上的兼容性。移植前要逐项比较其 GIMI/ReShade/OptiScaler 改动与 SSMT Runtime，确认 ABI、加载顺序、HDR 和生命周期。仓库本身也不包含完整的大型 NGX Runtime，RTX Profile 的文件需要成套管理。验收以 Feature 18 持续成功计数和原神游戏日志为准，不能只看 Feature 1 或窗口出现。

结论：把 GIMI 集成版作为**原神专用可选方案的移植基线**。SSMT 自有 Runtime 可以吸收其 GIMI 侧补丁，不应部署或覆盖另一套 GIMI DLL；SSMT 的启动器可实现其预加载顺序，不必依赖对方的 FPS Unlocker。用户已说明普通方案会遇到其 README 所列的拦截/报错；本次仅作源码与文档评估，没有进行新的原神热测，不能把作者的成功记录写成本机验证结果。

### RHI：鸣潮效果管理与资源安装的优先候选

RHI 对单游戏管理 ReShade 版本、shader、Add-on、预设及多种 Neural Rendering 路线，适合让玩家在 GUI 中调效果。以 Feeder 为例，安装会部署 ReShade、Feeder、NR consumer、`nvngx_dlss.dll`、`nvngx_dlssnr.dll`、`DLSS5_Feed.fx` 等文件；其他路线所需资源不同。`RhiInstallManifest` 记录组件及共享文件所有者，DLL 使用 `.original` 备份/哨兵。用户已在另一台有鸣潮的机器上验证 RHI 稳定运行；当前本机没有鸣潮，本文不把静态检查称作新的鸣潮实机验证，也不假定鸣潮一定使用 Feeder。现有 WWMI 测试脚本实际检查的是 `dlss5-bridge.addon64` 与 `renodx-dlss5.addon64`，最终应以那台机器的 RHI 安装记录和运行日志确认路线。

当前限制是 RHI 只有 GUI 安装入口：`App.OnLaunched` 识别 `--launch`、`--minimized` 等参数，Feeder 的 ReShade 准备、安装、卸载在 `DetailPanelBuilder.NeuralRendering.cs` 的按钮回调中。SSMT 现有 `start-wuwa-dlss5-mod.ps1` 只检查 RHI 预装文件、同步代理并修改 WWMI `d3dx.ini`，不能替代 RHI 安装器。直接由 SSMT 伪造 `rhi_install.txt` 或照搬文件复制会和 RHI 的更新、共享文件所有权冲突。RHI 目前的记录写入会记录错误后继续，未知 DLL 的旧 `.original` 也可能被替换；无窗口接口要提供明确失败结果和可恢复事务，不能把这些现有内部行为当成原子提交保证。

结论：鸣潮可借鉴 RHI 已验证的渲染路线和 GUI 调参方式，但最终由 **SSMT 持有 DLSS5 资源与环境的完整生命周期**。RHI 可提供效果配置界面或算法/资源参考；即使用户不安装 RHI GUI，也应能在 SSMT 完成下载、安装、调整与卸载。对于已经由 RHI 管理的目录，先检测其记录和文件所有权，再提供迁移或保留外部管理的明确路径，不能直接接管并覆盖。未安装或未启用插件时，SSMT 不读取 RHI 状态、不触碰游戏目录。RHI 与 SSMT 都是 GPL-3.0，但复制或分发第三方组件仍须分别核对其来源和许可。

## SSMT 自主管理的实施范围与难度

SSMT 的工作不止是调用一个现成安装按钮。插件市场现已支持从 Native Release 下载经哈希核验的官方 `.ssmtpkg`，并允许第三方本地包在预检和免责声明确认后安装；这些检查不能证明第三方代码无害。当前第三方包的 UI 页面仍需本体预先注册可信组件，安装任意第三方包不代表它可以直接执行自带的 Vue/JavaScript 页面。`managed_stack::install_route` 仍要求用户事先提供 HoYoShade、Swapper CLI 和 DLSS5 payload 目录，再委托外部 CLI 安装，因此还不能满足“由 SSMT 完成 DLSS5 相关 DLL 与环境的下载安装调整卸载”。下一步需要为可选图形组件建立独立的资源清单、下载和所有权事务，不能把插件包下载等同于图形环境安装。

| 工作 | 当前可复用部分 | 仍需实现、难度来源 |
| --- | --- | --- |
| 资源获取 | 插件包 SHA-256 校验、外部依赖状态 | 按组件/版本/架构/GPU Profile 维护可信下载地址、大小与哈希；断点或重试下载、临时文件、校验后入缓存；部分约 165 MB 的 `nvngx_dlssnr.dll` 不在 CXP Git 仓库，必须先有可直接获取且允许分发的来源。此项是发布前置条件，不能用“请用户去 RHI 手工下载”代替。 |
| 安装与更新 | `managed_reshade_journal` 的配置备份、哈希与恢复思路 | 将记录扩展到 DLL、add-on、shader、代理与共享文件；识别已有文件所有者和版本；先构建计划，验证目标游戏未运行，再原子写入或可回滚；切换 Profile 时严格检查 `nrchain` 与 NR Runtime 配对。工作量高。 |
| 原神运行链 | SSMT Runtime 已有 Present/PluginHost 桥接，SSMT Native 已有启动注入 | CXP 的 GIMI 补丁涉及 `HackerDXGI` 的 Hosted ReShade 生命周期、`HackerDevice`/`HackerContext` 原始对象接口、HDR 和 SwapChain、ReShade Add-on 逐帧事件；OptiScaler 的 DX11-on-DX12、NGX 参数与色彩格式仍须维护兼容构建。SSMT Runtime 与对方基线不同，应逐块移植和验证，不能直接 `git apply`。 |
| 启动编排与效果调整 | 按游戏插件启用状态、现有启动计划 | 把 CXP 的 GIMI 预加载、Bridge/OptiScaler 后加载顺序放入 SSMT 启动器；将 RHI 的游戏内效果配置能力接入可选插件界面或仅作为外部调参工具，同时由 SSMT 管理实际资源；保留用户预设和模式。 |
| 卸载与接管 | 现有 ReShade 配置日志 | 只移除 SSMT 拥有且哈希仍匹配的文件，恢复原有 DLL/INI，并处理 RHI/手工安装留下的文件；共享依赖由引用关系决定是否保留。不能把“删除整个目录”当作卸载。 |

实现顺序建议为：先建立与游戏无关的资源清单、下载缓存和所有权事务；再做 RHI/手工安装状态识别；随后接入鸣潮的具体渲染路线与原神的 Runtime 移植；最后按游戏日志验收。资源清单应描述每个组件的来源、许可证、版本、哈希、依赖与安装目标，运行计划从当前游戏和已启用插件的贡献计算，不写死三个插件的组合。第三方文件能否由 SSMT 自动下载/再分发，需要逐项确认；没有合法且可自动化的获取渠道时，应如实显示该组件不可一键安装，不能把整个插件功能伪装为已完成。

根据 2026-10-03 的任务优先级，DLSS5 方案尚会调整，下面的全组合热测作为后续验收条件保留，当前先确定宿主接口和文件所有权；不把禁用某个组合视为最终解决办法。

## 用户流程

用户在 SSMT 插件页安装并启用需要的插件，为游戏选择图形组件。SSMT 自动取得有可用来源的资源，显示下载、安装、调整或卸载计划及冲突，并执行计划；用户不必在 RHI GUI 中反复安装/卸载。启动前只对配置漂移做幂等修复。关闭某插件时只撤销该插件拥有的内容；没有下载或启用图形插件时，启动和普通页面不查询图形资源状态，也不修改游戏目录。

## 接口边界

SSMT 自己应提供独立于窗口的稳定资源入口；下列 JSON 形态也可供未来可选插件使用，至少包含：

| 操作 | 输入 | 输出 |
| --- | --- | --- |
| `inspect` | 游戏目录、可执行文件 | 渲染 API、位数、RHI 组件、文件所有者、版本、可用资源、冲突 |
| `plan` | 游戏、目标组件集合、已有组件状态 | 待安装/更新/撤销的文件与配置、备份和冲突；不写入 |
| `apply` | 经确认的计划及版本号 | 操作结果、安装记录、可恢复点、错误 |
| `restore` | 游戏、指定组件或事务 | 仅恢复由该组件拥有的内容，保留其他组件和用户文件 |

这个入口由 SSMT 的资源事务实现，不依赖 RHI GUI。对已经由 RHI GUI 安装的游戏，读取其 `RhiInstallManifest` 仅作外部所有权识别；迁移必须得到可验证的完整文件清单及哈希，不能伪造 RHI 数据库或仅按文件名删除。写入前确认游戏未运行；文件变更与记录变更要一起成功或可回滚；未知或后来被用户改写的文件先保留并报告。

SSMT 负责插件启用状态、游戏目标组件集合、资源下载与所有权、启动编排和插件页展示。RHI 只负责用户过去用它安装的资源；若后续使用 RHI GUI 调参，应避免让它改写 SSMT 已拥有的 DLL。3DMigoto 及其他插件分别声明自己的资源要求。组合规划按当前实际贡献计算，不能把固定的 `DLSS5 + ReShade + 3DMigoto` 三元组写死；允许未来插件声明共享宿主、加载顺序、代理 DLL 文件名、配置键、资源路径和互斥所有权。

同一运行链只应有一个最终 Present/代理所有者。CXP 的 GIMI 集成说明这一点，但 RHI/WWMI 的实际加载顺序和渲染结果仍须实机验证。安装成功与渲染生效也应分开判定：既核对文件/记录，也检查 ReShade、Feeder、NR 的运行日志和持续成功帧，而不是只依据安装按钮或 UI 文本。

## 验收

### 原神玩家目标（2026-10-04）

玩家只在 SSMT 中安装所选插件、为原神启用并启动游戏；不再要求手工提供 Swapper、HoYoShade、Bridge、OptiScaler 或大型 Runtime 的本机目录，也不要求运行隔离脚本。SSMT 根据该游戏当前启用的贡献下载并校验资源，按所有权记录安装及备份，建立唯一的 3DMigoto/Present/ReShade 宿主与有序注入计划。原神 DLSS 路线必须确认游戏使用 FSR2 抗锯齿和低于 1 的渲染精度；目前这一步需要游戏内手工设置，尚未满足“启用后直接开玩”。任何自动调整游戏设置的实现都必须可恢复原值，并在游戏版本变化时先验证格式。

禁用插件后，SSMT 不再加载其界面、后台能力、DLL、add-on 或 FX，也不再改动普通启动；属于该插件的游戏文件只按安装记录与当前哈希撤销，其他插件共享的资源保留，玩家原有文件按备份恢复。游戏运行时先延后变更，下一次启动按当前启用集合重算计划。无插件和空组合不得依赖 DLSS5 资源或外部管理器状态。

本机隔离热测已经验证 3DMigoto 贴图 Mod、HoYoShade 效果列表与原神 FSR2→DLSS SR→NR 技术链可以分别及共同运行；仍未交付可公开获取的完整资源清单、可信自动下载、逐游戏文件事务、启动计划接线、插件启停恢复和无插件回归。当前官方 `ssmt.dlss5.integration` 包仍声明 `dlss5-swapper`、`dlss5-adapter`、`dlss5-payload` 三个用户提供的目录依赖；它不满足上述玩家目标，不能在市场里标为“一键可用”。尤其不能把这次本机隔离脚本和未核对再分发条件的大型 DLL 直接打进官方插件包。

1. 在无插件、仅 ReShade、仅 DLSS5、仅 3DMigoto、任意双插件、全部已知插件组合间反复切换；原文件内容及其他插件状态保持正确。新增插件后扩展组合用例，不设置“禁止某组合”作为验收捷径。
2. 覆盖 RHI 已安装、未安装、已有用户/其他工具代理 DLL、共享 `nvngx_dlssnr.dll`、中途失败、游戏运行中拒绝修改、版本更新、配置漂移及恢复。
3. 插件未下载、禁用、RHI 缺失或 RHI 状态损坏时，普通启动流程仍然工作。
4. 本机原神、崩铁可做安装和组合回归；鸣潮的 WWMI/RHI/Feeder 实机图形与日志验收必须在有鸣潮的机器上完成。本机静态检查不能代替该项。

## 来源

- <https://github.com/CXP-2024/dlss5_for_genshinimpact>
- <https://github.com/CXP-2024/GIMI_reshade_dlss_integration>
- <https://github.com/RankFTW/RHI>
