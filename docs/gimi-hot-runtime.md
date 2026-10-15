# 原神 DLSS5 热注入

## 当前交付范围

原神（`YuanShen.exe` / `GenshinImpact.exe`）使用单独的 `gimi-hot` 路线。仅在安装了 DLSS5 插件并为当前游戏启用后，SSMT 才读取它的组件配置、校验资源或准备注入。未安装、关闭插件、其他游戏和纯净直接启动均不准备这条链。

本阶段交付组件导入、目标进程环境、挂起注入、Shade Bridge 共存和禁用后的恢复。没有新增自动下载或发布第三方 DLL；其他游戏继续使用已有路线，不代表它们已完成适配。启动与配置验证通过尚不能证明新版本游戏里的 NR 效果。

## 准备组件与启动

1. 在插件页安装 DLSS5 插件，并为原神启用。Shade Bridge 是可选项；DLSS5 单独启用也会准备自己的被动 ReShade Host。
2. 关闭游戏。在原神设置的 DLSS5 区域选择显卡 Profile（RTX 50 或 RTX 30/40），点击“导入热注入组件”，选择包含 `components` 的完整 GIMI 集成包根目录。
3. 组件来自 [CXP GIMI 集成项目](https://github.com/CXP-2024/GIMI_reshade_dlss_integration) 的 `cc46971` 二进制组。必须补齐所选 Profile 对应的 `nvngx_dlssnr.dll`；仓库目录存在并不代表大体积 Runtime 已经下载。SSMT 逐文件校验固定 SHA-256，不接受混用版本或另一 Profile 的 Runtime。具体路径与完整哈希以 `src-tauri/src/plugin/gimi_hot_runtime.rs` 为准。
4. 若显示旧的 Swapper 托管安装，先用原有“恢复”操作恢复该安装，再使用热注入。遇到损坏清单先处理该错误，不手动猜测删除游戏文件；热注入不会临时恢复 Swapper 代理文件。
5. 由 SSMT 直接启动原神，关闭“手动启动”和“使用官方启动器”模式。本阶段需要由 Run.exe 创建挂起进程，不能对已经创建的目标补设环境。
6. 游戏内抗锯齿选择 **FSR2**，渲染精度至少 **1.0**，进入世界观察 Mod、贴图及 NR 效果。仅弹出 ReShade 界面不表示 DLSS5 生效。

导入只读取选中的源目录，复制以下文件到 SSMT 的 `PluginAssets/ssmt.dlss5.integration/gimi-hot/<唯一目录>`，之后只修改该副本：

| 组件 | 相对路径 |
| --- | --- |
| 被动 Host | `components/ReShade/ReShade64.dll` |
| FSR2 Bridge | `components/Bridge/Dx11FsrBridge.dll` 与相邻 INI |
| 超分 | `components/OptiScaler/OptiScaler.dll`、相邻 INI、`nvngx_dlss.dll` |
| D3D12 Runtime | `components/OptiScaler/D3D12_Optiscaler/D3D12Core.dll` |
| NR | `components/DLSS5/Addons/pre-nr`（RTX 50）或 `pre-nr-rtx30`（RTX 30/40）下的 `nr-before-sr.zh-CN.addon64`、`nrchain_nvngx.dll`、`nvngx_dlssnr.dll`、`nr_before_sr.ini` |

不会导入集成包中的 GIMI Runtime、解锁器、旧日志或缓存，也不改用户原有额外 DLL 配置。反复导入会创建新副本；本阶段未实现旧副本自动回收。插件市场中的 Swapper 外部工具依赖状态描述其管理器功能，原神热注入不要求 Swapper、adapter 或其 payload 目录就绪。

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

原神的 Swapper 安装与启动暂存入口均拒绝冷加载，旧安装仅允许恢复。新的热注入资源缺失、哈希错误、Profile 不匹配或游戏已运行时会在提交启动队列前报错，不静默回退为“只有 ReShade”的成功状态。

## 验证与实机验收

源码验证入口：

```powershell
node --test scripts/tests/test-runtime-sync.mjs scripts/tests/test-efmi-launch.mjs
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

2026-10-15 已通过上述配置验证（本机 RTX 50 完整资源）、启动回归与原生夹具，未在本次改动后启动真实游戏。RTX 30/40 只核对了清单与配对规则，尚无该显卡上的运行结果。Bridge 依赖游戏版本的特征扫描，不能据固定组件哈希宣称未来游戏版本长期兼容。

实机验收时只查看**本次启动**的日志：

- Migoto 日志应有托管 ReShade 初始化结果；只有 DLL 载入记录不够。
- Bridge 日志应有 `fsr2_translation_context_created` 及实际 dispatch；只有 FSR2 接口查询不够。
- NR 日志应有 `NR-before-SR evaluate succeeded` 的成功帧（Feature 18），并继续观察画面与稳定性；Feature 1 创建不等于 NR 生效。
- 分别测试 DLSS5 单独、DLSS5 + Shade Bridge，以及关闭两者后的普通 Mod 启动。不得使用导入前的旧成功日志作为新结果。

托管 ReShade 配置位于导入目录下的 `state/reshade-runtime`，组件日志位于它们各自的目录；通过设置中显示的导入目录核对时间戳。原神 Mod 的动态角色分辨率问题已由用户确认恢复，记录在[本次问题清单](runtime-and-graphics-fixes-20261015.md)，不以这项验收替代 DLSS5 渲染验收。
