# 原神 DLSS5 热注入

## 当前交付范围

原神（`YuanShen.exe` / `GenshinImpact.exe`）使用单独的 `gimi-hot` 路线。仅在安装了 DLSS5 插件并为当前游戏启用后，SSMT 才读取它的组件配置、校验资源或准备注入。未安装、关闭插件、其他游戏和纯净直接启动均不准备这条链。

本阶段交付自动下载部署、离线导入、目标进程环境、挂起注入、Shade Bridge 共存和禁用后的恢复。组件缺失或准备失败时横幅提示，本次跳过 DLSS5 并继续普通 Mod 启动；不会改变保存的启用状态。其他游戏继续使用已有路线，不代表它们已完成适配。启动与配置验证通过尚不能证明新版本游戏里的 NR 效果。

## 准备组件与启动

1. 在插件页安装 DLSS5 **0.2.1 或更新版本**，并为原神启用。新插件包包含 SSMT 适配的 Bridge；旧包缺失该组件时横幅提醒并继续普通 Mod 启动。Shade Bridge 是可选项；DLSS5 单独启用也会准备自己的被动 ReShade Host。
2. 关闭游戏。在**插件市场 → 选择原神 → DLSS5 → 功能设置**选择显卡 Profile（RTX 50 或 RTX 30/40；RTX 40 为实验支持），点击**自动下载并配置**。首次约下载 302 MiB，包含配套的 `nvngx_dlssnr.dll`，无需玩家另找组件；后续复用已校验的缓存。游戏设置中保留相同入口。
3. 离线情况下点击“离线导入”，选择包含 `components` 的完整 GIMI 集成包根目录。组件来自 [CXP GIMI 集成项目的固定版本](https://github.com/CXP-2024/GIMI_reshade_dlss_integration/tree/cc4697187cb57bacc7c932b7f53f16be24af152a)，必须补齐所选 Profile 的 NR Runtime。SSMT 逐文件校验固定 SHA-256，不接受混用版本或另一 Profile 的 Runtime。具体路径与完整哈希以 `src-tauri/src/plugin/gimi_hot_runtime.rs` 和 `gimi_hot_assets.json` 为准。
4. 若显示旧的 Swapper 托管安装，先用原有“恢复”操作恢复该安装，再使用热注入。遇到损坏清单先处理该错误，不手动猜测删除游戏文件；热注入不会临时恢复 Swapper 代理文件。
5. 保持游戏设置中的“启动目标程序”开启。普通启动直接使用目标程序；“使用 Shell 启动”的启动路径与目标路径指向同一个游戏 EXE 时，也属于由 SSMT 直接启动。SSMT 会仅为本次热注入改由 Run.exe 创建挂起进程，保存的 Shell 设置不变。两条路径指向不同程序时，暂不支持通过另一启动程序创建游戏；不能对已经创建的目标补设环境。
6. SSMT 在实际采用热注入计划后自动配置抗锯齿：FSR2 Bridge 启用 **FSR2**，Feeder 关闭游戏抗锯齿。Bridge 的渲染精度仍至少 **1.0**。进入世界观察 Mod、贴图及 NR 效果；仅弹出 ReShade 界面不表示 DLSS5 生效。

自动下载和离线导入均部署到 SSMT 的 `PluginAssets/ssmt.dlss5.integration/gimi-hot/<唯一目录>`，之后只修改该副本。离线导入只读取选中的源目录。核心文件如下：

| 组件 | 相对路径 |
| --- | --- |
| 被动 Host | `components/ReShade/ReShade64.dll` |
| FSR2 Bridge | `components/Bridge/Dx11FsrBridge.dll` 与相邻 INI |
| 超分 | `components/OptiScaler/OptiScaler.dll`、相邻 INI、`nvngx_dlss.dll` |
| D3D12 Runtime | `components/OptiScaler/D3D12_Optiscaler/D3D12Core.dll` |
| NR | `components/DLSS5/Addons/pre-nr`（RTX 50）或 `pre-nr-rtx30`（RTX 30/40）下的 `nr-before-sr.zh-CN.addon64`、`nrchain_nvngx.dll`、`nvngx_dlssnr.dll`、`nr_before_sr.ini` |

不会导入集成包中的 GIMI Runtime、解锁器、旧日志或缓存，也不改用户原有额外 DLL 配置。反复导入会创建新副本，可在下述历史入口选择或明确确认后清理，不自动回收旧目录。插件市场中的 Swapper 外部工具依赖状态描述其管理器功能，原神热注入不要求 Swapper、adapter 或其 payload 目录就绪。

组件状态、路线和历史在后台独立读取。大型 DLL 哈希校验不占用应用交互线程；路线就绪后即可选择并配置组件，无需等待历史校验。单项读取失败仍保留修复入口；切换游戏、离开设置或开始配置后，迟到的旧读取不会覆盖当前状态，也不会改写等待期间选择的显卡系列。

启动计划使用插件包中的 SSMT Bridge 1.2.3.1，将其按构建哈希暂存到导入目录的
`state/bridge/<SHA-256>/Dx11FsrBridge.dll`，并复制当前 Bridge INI。固定上游组件保持原样，
升级插件无需重新下载 NR。插件包继续经过现有包结构与来源校验；组件的旁置哈希只用于
检查安装后损坏，不能代替发布来源验证。禁用插件时不读取、暂存或注入此组件。

## 热组件历史与清理

插件的**功能设置 → 热组件历史**统一展示当前游戏实例的 FSR2 Bridge 和 Feeder 安装。关闭游戏后点击“使用所选组件”，SSMT 会重新校验文件与兼容基线，并原子保存该安装及对应实现。界面同步切换实现；仅有一个活动标记，不会同时注入两套输入链。FSR2 Bridge 的基线指纹来自固定组件与 Profile，未知的旧基线须重新配置，不能仅凭历史中自存的哈希放行新组件组合。

新下载和导入均记录实例所有权，最多保留 32 套历史。选择、下载、导入、清理和启动准备共用操作锁；导入失败或期间关闭插件、启动游戏时，不切换旧安装，清理本次未完成的副本。关闭插件或未安装插件时不收集这些历史、不读取组件。

清理需确认完整目录，会删除该套自有副本内的组件、配置、预设与日志，保留游戏文件、离线源目录、下载缓存和其他实例的组件。先撤销待删安装的选择，再删除文件；锁定文件或中断时保留登记供重试。清理非活动实现不会改变当前路线；清理活动安装后也不会自动改用另一套实现，下次启动仅提示 DLSS5 未加载并继续普通 Mod 启动，直到手动选择或重新配置。

升级前只保存了最后一套 FSR2 Bridge 安装指针，SSMT 会将这套已知安装加入统一历史。它仍可在校验通过后选择，但缺少实例所有权标记时不允许自动删除，并在界面说明原因。不扫描认领其他旧目录，也不根据相似文件名猜测归属；此前已经丢失指针的副本无法自动恢复为该实例的历史。

## 自动下载来源与失败恢复

常规组件、配置模板和许可证从上述固定 commit 的 `raw.githubusercontent.com` 下载，逐项校验大小与 SHA-256。NR 使用[上游发布的 v1.3 完整包](https://drive.google.com/file/d/1d-aaUEo_ftRpY7mFCO7zUTugBKg7pW-j/view)，固定大小 `221201410`，归档 SHA-256 为 `95a87bc1b5c981a7afa63e9e18bb68410b1697d3f20108418ac9b7a4050388cf`。内置 7z 解码只将所选 Profile 的 NR 文件写入预定位置，再校验对应 DLL 哈希；不执行第三方安装脚本、不使用包内其他 DLL，也不按包内路径批量解压。

2026-10-15 联网核验发现上游 RTX 50 单独下载链接返回的文件不匹配公布哈希且 Authenticode 为 HashMismatch，故两种 Profile 统一从上述完整包提取；完整包内两套 NR 文件均与预期一致。不得因此放宽哈希规则。

下载复用 `PluginAssetCache`：大小/哈希匹配才复用，损坏缓存可重试下载，完整验证新文件后再替换缓存。组件在独立新目录准备，全部校验成功且游戏仍关闭、插件仍按游戏启用时才切换设置指针。网络、配额、超时、提取、校验或保存失败均保留此前安装；下载失败不阻止普通启动。下载进度只作用于原先选择的游戏，切换游戏不会把旧结果填到新游戏。未安装或未启用插件时不显示资源配置控件，也不执行下载。

## 启动与组合规则

提权后的 Run.exe 根据 `--set-env` / `--unset-env` 设置本次子进程环境，再以挂起方式创建游戏。加载顺序为：

```text
SSMT Runtime
  → 被动 ReShade Host（关闭图形 Hook）
  → Dx11FsrBridge
  → OptiScaler
  → 用户额外 DLL（保留原顺序）
  → 已启用运行时插件的 PluginHost
  → 恢复游戏主线程
```

`GIMI_HOSTED_RESHADE_DLL` 和 `GIMI_HOSTED_RESHADE_CONFIG` 指向同一套托管资源。SSMT Runtime 负责最终 Present 上的 ReShade，NR add-on 在前置链运行；OptiScaler 的后置 NR 被关闭，避免重复执行。这里的挂起加载仍为进程注入，不向游戏目录放置 `dxgi.dll` / `d3d11.dll` 或符号链接。

| 按游戏启用组合 | 行为 |
| --- | --- |
| 无图形插件 | 原有 3DMigoto 启动，不读取热注入资源 |
| 仅 Shade Bridge | 直接创建目标时同样走 Run.exe 的有序热注入，使用独立托管配置 |
| 仅 DLSS5 | 热注入三组件与 NR，不加载 Shade 的 FX/Add-on |
| DLSS5 + Shade Bridge | 同一 Host 合入 Shade 的配置、FX 与 Add-on；保留其禁用列表 |
| 上述组合 + 其他运行时插件 | 继续传入 PluginHost，不另建第二个 ReShade Host |
| 关闭 DLSS5 | 下次正常启动重新生成 INI，去掉热注入附加 DLL 与专用环境 |

Shade 的相对 FX/贴图路径按它声明的资源根目录展开。Shade 关闭时清理上次复制到热注入副本的 Add-on，避免它继续产生贡献。用户额外 DLL 与托管组件同名、或声明 `d3d11.dll` / `dxgi.dll` 时报告所有权冲突；不会靠禁用整个插件组合规避共存。

Shell 只是启动方式，不能据 `useShell=true` 推断使用了官方启动器。热注入比较规范化后的完整启动/目标路径，不能只比 EXE 文件名。同一目标的 Shell 配置会同步改写本次 INI 的 `launch` / `launch_args` 和启动队列：只由 Run.exe 创建一次游戏，保留参数及 PluginHost，并等待目标就绪。未准备热注入计划时继续使用原有 Shell 队列。

原神的 Swapper 安装与启动暂存入口均拒绝冷加载，旧安装仅允许恢复。新的热注入资源缺失、哈希错误、Profile 不匹配或准备失败时，显示 DLSS5 本次未加载的横幅并去掉本次热注入贡献，继续普通 Mod 启动；Shade 和 PluginHost 资源可用时继续加载它们，不把回退报告为 DLSS5 成功。保存的插件启用状态不变。已就绪热注入计划的额外 DLL 所有权冲突、关闭“启动目标程序”或 Shell 指向另一启动程序等情况仍须先处理，提示应明确当前配置，而非笼统要求关闭“官方启动器模式”。

### 共享预设与失败提示

源配置中的 `GENERAL.PresetPath` 指向各自原始预设，不能直接作为共享 Host 的标量冲突比较。
组合器先把两侧路径统一为托管预设，合并已读取的效果内容，最终由采用的路线指定自有目录中的绝对路径。
这不会改写 HoYoShade 源配置或向原神目录部署代理 DLL；其他标量及同名宏定义的真实冲突仍会拒绝合并。

同一插件保留多个安装版本时，启动能力、共享效果、PluginHost 和页面贡献均只选择与注册表 `find` 一致的最高语义版本。
旧包继续保留供管理，不并行贡献，不因当前版本关闭或不兼容而自动启用旧版本。
2026-10-18 的五条警告实际对应三个 DLSS5 版本、两个 Shade 版本；`PresetPath` 误判又让它们全部在准备阶段被跳过。

失败横幅列出插件名称与版本，并明确“原因”。配置冲突包含节、键、双方值及处理建议，合并贡献失败时补充来源。
同次启动中相同原因合成一条并列出受影响插件，不同原因分别提示；已失败候选不在后续回退时重新执行。
可选图形贡献失败继续普通 Mod 启动，不修改玩家保存的启用选择，也不将准备失败描述为运行时已经注入成功。

## 验证与实机验收

2026-10-18：150 项插件 Rust 回归、60 项启动链前端回归及前端构建通过。
额外使用本机完整 RTX50 Bridge 组件与 HoYoShade 源配置，在临时目录验证单独准备、合并、停用后移除效果贡献及源 INI 不变；未加载图形 DLL 或启动游戏。
该检查修复了准备阶段的失败，游戏内 DLSS5、ReShade 与 Mod 共存效果仍需实机确认。

### 2026-10-15 切换 FSR2 崩溃修复

23:35:55 的实机 dump 位于 `OptiScaler.dll + 0x1C3E57`，读取空的 NGX 参数对象。
核对实际 DLL 的指令、函数名字符串及固定源码后，确认调用的是
`ffxFsr2ContextDispatch_Dx12`。原神 Bridge 提供 DX11 FSR2 契约，但下载包的
`UseFsr2Dx11Inputs=auto` 默认选中 DX12；旧准备流程只指定 `Dx11Upscaler=dlss_12`
输出后端，没有固定输入 API。错误分支未创建参数表却返回 Create 成功，随后首次
Dispatch 触发空指针。

准备流程现在显式启用 FSR2 DX11 输入、关闭 Vulkan 输入，固定 DLSS 后端所需选项，
跳过 OptiScaler 的设备虚表 Hook，并禁止它另载 ReShade 或运行内置后置 NR。
这些选项只写入已启用插件的托管配置，每次启动重新生成，已有导入目录也会修复。
本次无需重装 0.2.1 插件、重新下载组件或重新编译 Native。

新增 `native/SSMT-Gimi-Bridge/tests/fsr2.cpp` 使用真实 Bridge 与 OptiScaler，
在 RTX 5060 Laptop 上检查 FSR2 Create、两次 Dispatch 及 Destroy。仅将输入选择
恢复为 `auto`，即可复现相同的 `OptiScaler + 0x1C3E57`；修复配置通过，日志确认
进入 DX11 输入并成功创建 DLSS Feature。该夹具使用系统 D3D11 设备，仅验证输入
契约；Runtime 共存由另一组混合设备测试覆盖，不能合称为完整游戏链通过。
没有 SwapChain / Present 的夹具只执行两次，不用于检验持续帧或 NR Feature 18。
游戏内切换、NR 及画面仍待实机验收。

显式运行实组件 Dispatch 验证：

```powershell
cmake --build native/build-gimi-bridge --config Release --target gimi-fsr2-dispatch-test
$env:SSMT_HOT_FSR2_FIXTURE = (Resolve-Path native/build-gimi-bridge/SSMT-Gimi-Bridge/Release/gimi-fsr2-dispatch-test.exe).Path
# 同时设置下文的 SSMT_HOT_TEST_SOURCE 和 Profile，再运行 ignored 测试。
```

### 2026-10-15 混合设备崩溃修复

22:32:58 和 22:42:04 的两份新 dump 均确认加载了修复后的 Runtime 1.2.2.12，
已越过前一次 Context 私有虚表崩溃。这次异常位于 `d3d11.dll + 0xC158A`：
`HackerDevice::CreateBuffer` 读取空的 `mOrigDevice1`，上层调用者是 Bridge，
后续栈中可见 NVIDIA/NvCamera 组件。

Bridge 保存第一个设备的方法，再用于其他设备；当第一个是 Runtime 包装设备、
另一个是系统或显卡辅助设备时，`this` 的布局不匹配。修复按对象转发所有受影响的
Device、Context、Factory 和 SwapChain 方法，并在最终 Release 后移除该代登记。
不通过 Runtime 空指针返回或禁用 NVIDIA 组件规避问题。

新增回归位于 `native/SSMT-Gimi-Bridge`。未修复源码的两个不同 COM 实现测试出现
1004 次错误转发；真实 WARP 辅助设备与 Runtime 包装设备交替创建 Buffer 时，
在相同 Runtime 偏移复现 `0xC0000005`。修复后两项通过，覆盖复制虚表、释放后地址
复用和不同线程的 Context 调用。Debug、Release 均编译通过；完整组件副本的
哈希、独立暂存及原始文件保留检查通过。这仍不能证明游戏内 FSR2 dispatch 或 NR
Feature 18 已成功，需继续检查实际游戏日志和画面。

Bridge 基于上游 1.2.3 的固定源码 `cbdf5c11bfea8d4cb423866dabb96a755fb8ab0b`，
完整适配补丁和构建入口在 Native 仓库。集成包文档记录的另一个源码提交已采用
不同的 FFX12 后端，不能直接据该说明重编译并替换现有 FSR2 转译组件。
构建插件包不会自动安装或启用，开发环境通过市场本地包入口更新至 0.2.1。
Bridge 本次日志位于上述 `state/bridge/<SHA-256>`，启动行应包含
`ssmt_version=1.2.3.1 com_forwarding=per_instance`。

23:35 的实机反馈已确认可以进入游戏，但切换 FSR2 后仍崩溃。新 dump 的异常模块
为 OptiScaler，前述混合设备修复作为独立阶段保留；FSR2 转译与 NR 尚未验收通过。

### 2026-10-15 启动崩溃修复

21:20:47 与 21:21:59 的两份游戏 dump 均为执行空地址的 `0xC0000005`，首个返回地址位于当时加载的 SSMT Runtime `d3d11.dll + 0xAC914`。将已部署 DLL 的副本解包后，核对本地构建的 PDB 标识与调用指令，定位为 `FrameAnalysisContext::VSSetShaderResources` 调用虚表偏移 `0x430` 的帧分析日志方法。ReShade、Bridge 与 OptiScaler 均已载入；这不是“没有注入”。Migoto 的 `calls=0`，旧 `d3d11_log.txt` 的时间不能代表本次启动。

Bridge 按 COM 接口长度复制 Context 虚表，Runtime 原先追加的三个内部方法不在副本中。修复将日志、选项与资源导出的内部派发移到对象成员，保留普通 Context 的空操作和帧分析 Context 的具体实现，不关闭 hunting，不修改 Mod 或插件启用状态。修改局限于 Runtime 内部 C++ 类，Plugin ABI 不变，需完整重编译 Runtime。

回归入口位于 `runtime/tests/context-interop`：独立 WARP 设备覆盖 hunting=0/2、原始虚表和 128/134 项副本。旧版本四项副本测试均复现空地址崩溃，新版本六项通过；真实 ReShade + Bridge 组件在旧版同样崩溃，新版可完成接口调用。既有 DXGI 桌面复制的纯系统、Runtime、ReShade、二者组合四项也通过。

完整 Bridge + OptiScaler 组合的独立夹具未能确认返回对象的 Runtime 私有身份，测试拒绝将其计为通过；它不提供游戏内 NR 结论。本次修复后尚未启动真实游戏，仍须按下方日志检查 FSR2 dispatch、Feature 18 与 Mod 画面。修复后的 DLL 部署到应用资源，由正常启动的静默同步流程更新 Migoto 目录，不向游戏目录放置代理文件。

源码验证入口：

```powershell
node --test scripts/tests/test-runtime-sync.mjs scripts/tests/test-efmi-launch.mjs scripts/tests/test-plugin-graphics-settings.mjs
cargo test --manifest-path src-tauri/Cargo.toml --lib plugin::
cargo test --manifest-path src-tauri/Cargo.toml --lib gimi_settings::
bun run build
```

带真实资源的本地配置验证（不启动游戏）：

```powershell
$env:SSMT_HOT_TEST_SOURCE = '完整组件包根目录'
$env:SSMT_HOT_TEST_PROFILE = 'rtx50' # 或 rtx30
$env:SSMT_HOT_TEST_SHADE_ROOT = '已安装的 HOYOShade 根目录' # 可选
cargo test --manifest-path src-tauri/Cargo.toml --lib prepares_real_components_without_game_directory_writes -- --ignored --nocapture
```

测试创建临时自有副本，验证完整文件哈希、中文与空格路径、最终 ReShade/OptiScaler 配置、Shade 开关、禁用 Add-on 保留以及源配置未被改写。原生注入器另用管理员要求关闭的独立 CTest 夹具验证真实进程中的预加载顺序、环境继承/删除与 PluginHost；正式 Run.exe 仍要求管理员。

自动下载实测（联网，使用隔离目录，不修改玩家配置）：

```powershell
$env:SSMT_HOT_DOWNLOAD_TEST_ROOT = '隔离测试目录'
cargo test --manifest-path src-tauri/Cargo.toml --lib downloads_and_prepares_both_real_profiles -- --ignored --nocapture
```

2026-10-15 已从空缓存完成上述联网下载验证，RTX 50 和 RTX 30/40 的组件均完成提取、哈希与配置校验；另通过 48 项前端/启动回归与原生夹具，未在本次改动后启动真实游戏。两套 Profile 都尚无本次改动后的显卡实机结果。Bridge 依赖游戏版本的特征扫描，不能据固定组件哈希宣称未来游戏版本长期兼容。

实机验收时只查看**本次启动**的日志：

- Migoto 日志应有托管 ReShade 初始化结果；只有 DLL 载入记录不够。
- Bridge 日志应有 `fsr2_translation_context_created` 及实际 dispatch；只有 FSR2 接口查询不够。
- NR 日志应有 `NR-before-SR evaluate succeeded` 的成功帧（Feature 18），并继续观察画面与稳定性；Feature 1 创建不等于 NR 生效。
- 分别测试 DLSS5 单独、DLSS5 + Shade Bridge，以及关闭两者后的普通 Mod 启动。不得使用导入前的旧成功日志作为新结果。

托管 ReShade 配置位于导入目录下的 `state/reshade-runtime`，组件日志位于它们各自的目录；通过设置中显示的导入目录核对时间戳。原神 Mod 的动态角色分辨率问题已由用户确认恢复，记录在[本次问题清单](runtime-and-graphics-fixes-20261015.md)，不以这项验收替代 DLSS5 渲染验收。
