# 原神 DLSS5 热注入

## 当前交付范围

原神（`YuanShen.exe` / `GenshinImpact.exe`）使用单独的 `gimi-hot` 路线。仅在安装了 DLSS5 插件并为当前游戏启用后，SSMT 才读取它的组件配置、校验资源或准备注入。未安装、关闭插件、其他游戏和纯净直接启动均不准备这条链。

本阶段交付自动下载部署、离线导入、目标进程环境、挂起注入、Shade Bridge 共存和禁用后的恢复。组件缺失或准备失败时横幅提示，本次跳过 DLSS5 并继续普通 Mod 启动；不会改变保存的启用状态。其他游戏继续使用已有路线，不代表它们已完成适配。启动与配置验证通过尚不能证明新版本游戏里的 NR 效果。

## 准备组件与启动

1. 在插件页安装 DLSS5 插件，并为原神启用。Shade Bridge 是可选项；DLSS5 单独启用也会准备自己的被动 ReShade Host。
2. 关闭游戏。在**插件市场 → 选择原神 → DLSS5 → 功能设置**选择显卡 Profile（RTX 50 或 RTX 30/40；RTX 40 为实验支持），点击**自动下载并配置**。首次约下载 302 MiB，包含配套的 `nvngx_dlssnr.dll`，无需玩家另找组件；后续复用已校验的缓存。游戏设置中保留相同入口。
3. 离线情况下点击“离线导入”，选择包含 `components` 的完整 GIMI 集成包根目录。组件来自 [CXP GIMI 集成项目的固定版本](https://github.com/CXP-2024/GIMI_reshade_dlss_integration/tree/cc4697187cb57bacc7c932b7f53f16be24af152a)，必须补齐所选 Profile 的 NR Runtime。SSMT 逐文件校验固定 SHA-256，不接受混用版本或另一 Profile 的 Runtime。具体路径与完整哈希以 `src-tauri/src/plugin/gimi_hot_runtime.rs` 和 `gimi_hot_assets.json` 为准。
4. 若显示旧的 Swapper 托管安装，先用原有“恢复”操作恢复该安装，再使用热注入。遇到损坏清单先处理该错误，不手动猜测删除游戏文件；热注入不会临时恢复 Swapper 代理文件。
5. 由 SSMT 直接启动原神，关闭“手动启动”和“使用官方启动器”模式。本阶段需要由 Run.exe 创建挂起进程，不能对已经创建的目标补设环境。
6. 游戏内抗锯齿选择 **FSR2**，渲染精度至少 **1.0**，进入世界观察 Mod、贴图及 NR 效果。仅弹出 ReShade 界面不表示 DLSS5 生效。

自动下载和离线导入均部署到 SSMT 的 `PluginAssets/ssmt.dlss5.integration/gimi-hot/<唯一目录>`，之后只修改该副本。离线导入只读取选中的源目录。核心文件如下：

| 组件 | 相对路径 |
| --- | --- |
| 被动 Host | `components/ReShade/ReShade64.dll` |
| FSR2 Bridge | `components/Bridge/Dx11FsrBridge.dll` 与相邻 INI |
| 超分 | `components/OptiScaler/OptiScaler.dll`、相邻 INI、`nvngx_dlss.dll` |
| D3D12 Runtime | `components/OptiScaler/D3D12_Optiscaler/D3D12Core.dll` |
| NR | `components/DLSS5/Addons/pre-nr`（RTX 50）或 `pre-nr-rtx30`（RTX 30/40）下的 `nr-before-sr.zh-CN.addon64`、`nrchain_nvngx.dll`、`nvngx_dlssnr.dll`、`nr_before_sr.ini` |

不会导入集成包中的 GIMI Runtime、解锁器、旧日志或缓存，也不改用户原有额外 DLL 配置。反复导入会创建新副本；本阶段未实现旧副本自动回收。插件市场中的 Swapper 外部工具依赖状态描述其管理器功能，原神热注入不要求 Swapper、adapter 或其 payload 目录就绪。

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
| 仅 Shade Bridge | 原有 Shade 注入路线 |
| 仅 DLSS5 | 热注入三组件与 NR，不加载 Shade 的 FX/Add-on |
| DLSS5 + Shade Bridge | 同一 Host 合入 Shade 的配置、FX 与 Add-on；保留其禁用列表 |
| 上述组合 + 其他运行时插件 | 继续传入 PluginHost，不另建第二个 ReShade Host |
| 关闭 DLSS5 | 下次正常启动重新生成 INI，去掉热注入附加 DLL 与专用环境 |

Shade 的相对 FX/贴图路径按它声明的资源根目录展开。Shade 关闭时清理上次复制到热注入副本的 Add-on，避免它继续产生贡献。用户额外 DLL 与托管组件同名、或声明 `d3d11.dll` / `dxgi.dll` 时报告所有权冲突；不会靠禁用整个插件组合规避共存。

原神的 Swapper 安装与启动暂存入口均拒绝冷加载，旧安装仅允许恢复。新的热注入资源缺失、哈希错误、Profile 不匹配或准备失败时，显示 DLSS5 本次未加载的横幅并去掉本次热注入贡献，继续普通 Mod 启动；Shade 和 PluginHost 资源可用时继续加载它们，不把回退报告为 DLSS5 成功。保存的插件启用状态不变。已就绪热注入计划的额外 DLL 所有权冲突及不支持的手动/官方启动器模式仍须先处理。

## 验证与实机验收

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
