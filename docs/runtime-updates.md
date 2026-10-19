# SSMT Runtime 的更新与锁定

游戏设置 → **SSMT Runtime** 提供“锁定当前运行时”胶囊开关，按游戏保存，默认关闭。
开启后，保留该游戏 Migoto 目录中现有的 DLL，包括没有版本资源的自定义构建；
不联网检查，不随包覆盖，也不重新进行 UPX 打包。文件缺失时才从随包资源补齐。
开关同时覆盖 DX11 / DX12，不限制 GIMI 等核心配置包及可选插件各自的更新。

核心配置包解压始终排除 `d3d11.dll` / `d3d12.dll`，防止包里夹带的旧 Runtime
绕过上述开关或覆盖 SSMT 适配版本。普通 Mod / 工作空间解压不受该过滤影响。

## 未锁定时的来源选择

DX11 正常启动通过 `resolve_ssmt_runtime_source` 访问
`Perxenic-Acid/SSMT-3DMigoto-Runtime` 的 `rolling` Release。
只接受同一发布目录下的 `SSMT-Runtime-x64.zip` 与 `SHA256SUMS.txt`，
不下载安装 SpectrumQT 的上游 DLL。设置页保留的 XXMI Changelog 仍是只读资料。

下载要求 HTTPS，核对发布资产的声明大小、归档 SHA-256，以及 GitHub 提供的资产摘要
（如果存在）。归档必须只含一个名为 `d3d11.dll` 的 x64 PE DLL；拒绝多余文件、
路径跳转、符号链接及 `0.0.0.0` 文件版本。限制归档 32 MiB，DLL 64 MiB。
网络过程合计最多等待 12 秒，同一进程每 15 分钟至多检查一次。

已验证 DLL 按内容哈希存入
`%LOCALAPPDATA%/SSMT4GlobalConfigs/Runtime/x64/`，原始下载不会被 UPX 改写。
DLL 和缓存索引先写同目录暂存文件，校验完成后 rename 提交；读取缓存时重新验证哈希。
更新失败用一次非模态横幅显示原因，继续选用可用的本地 Runtime；没有任何可用 DLL
时才停止注入。纯净启动不进入此流程，也不新增插件扫描或插件文件要求。

选择随包 DLL、已验证缓存和当前游戏 DLL 中数值文件版本最高的一份；同版本优先
随包构建。较新的现有 DLL 直接保留，不因 UPX 改变哈希而被旧安装包覆盖。
准备部署时仍使用现有的暂存、复制校验和 UPX 链，最后一次 rename 替换游戏 DLL。
DX12 目前只使用随包 DLL，不访问上述 DX11 Release；可以同样锁定现有版本。

## 区分开发资源与实际安装资源

| 文件位置 | 使用方 |
| --- | --- |
| `runtime/x64/Release/d3d11.dll` | Runtime 的 debug / release 组件脚本 |
| `src-tauri/resources/d3d11.dll` | 源码开发与后续应用构建的随包来源 |
| 实际 `ssmt4.exe` 旁的 `resources/d3d11.dll` | 当前安装版的随包来源 |
| 游戏所配置的 Migoto 目录中的 `d3d11.dll` | Run.exe 实际注入的文件 |

只运行 `scripts/stage-ssmt-runtime.ps1` 不会更新另一个安装目录。
排查“debug 正确、SSMT 仍旧”时先确认运行中 `ssmt4.exe` 的完整路径，再比较上述 DLL
的版本与哈希。UPX 前后哈希会不同，应同时核对数值文件版本，不能仅以哈希不同判断来源。
后端 `[SSMTRuntime]` 日志记录选中的版本、来源、绝对路径、目标路径和更新失败原因。

## 云端版本基线与验证

2026-10-19 的真实 GitHub 资产校验发现，fork 只有 `rolling` 标签，没有本机的
`v1.2.2` 等上游版本标签，因此旧版本生成器在云端产出 `0.0.0.0`。
Runtime 现在通过 `tools/version-baseline.json` 固定版本与祖先提交；无版本标签的
完整 checkout 也能按同一提交距离生成版本。后续发布前拒绝零版本 DLL。
客户端会拒绝此前的零版本资产并保留本地版本，直到包含该修复的新 Runtime 发布。

离线验证：

```powershell
node --test scripts/tests/test-runtime-sync.mjs scripts/tests/test-efmi-launch.mjs
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::ssmt_runtime::tests
powershell.exe -NoProfile -ExecutionPolicy Bypass -File runtime/tests/version-generation/Run-Tests.ps1
bun run build
```

真实发布检查（只下载到独立临时目录，不加载 DLL，不启动游戏）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::ssmt_runtime::tests::published_rolling_is_verified_or_falls_back_without_replacing_local_runtime -- --ignored --nocapture
```

交付验证时，先在游戏设置中开启锁定并记录游戏 DLL 的哈希，正常启动后确认哈希未变；
解除锁定后再检查一次版本和 `[SSMTRuntime]` 来源日志。断网应仅出现横幅且原有 Mod
仍能启动。组件回归与文件校验不能代替游戏内的画面及共存验收。
