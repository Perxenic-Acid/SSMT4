# 插件自有后台与本体边界

本体只理解声明、权限、包内路径、租约与通用生命周期。插件名称、游戏设置字段、组件目录、下载地址、INI 格式及旧状态修复策略属于插件代码。将插件 ID 特判改成专用策略枚举，并不等于完成隔离。

## 当前进度

Player Tweaks 的设置页面、设置规范化、GIMI INI 生成、旧游戏设置迁移、SRMI 注册表事务、所有权记录和恢复算法位于 `native/crates/player-tweaks-backend` 与对应插件包；本体中的 `player_tweaks.rs`、`star_rail_tweaks.rs` 及启动时删除专用 DLL/INI 的代码已移除。

这只是迁移的一部分。DLSS5 的图形后台，以及两者在本体中的旧图形入口、图形能力代理、设置 Vue 组件、启动预检和专用恢复逻辑仍未移除，不能据此宣称本体已经不认识插件。Poser 的包与通用启动贡献仍需单独核对。

HoYoShade 的新独立后台已位于 `native/crates/hoyoshade-backend`，开发包为 `ssmt.hoyoshade.bridge/0.2.0`，自带静态状态页、资源策略和构建产物声明。它只在插件自有目录生成热注入配置，返回 DLL 哈希、环境变量和日志来源，保留原 `PluginAssets/<id>/hot/<游戏摘要>` 中的用户设置。旧游戏目录 UI 状态只读导入；外部图形代理、其他安装的 `INSTALL.BasePath` 和未知 Add-on 均拒绝覆盖。

本体新增 `backend_launch.rs`，只处理后台协议、资源仲裁、权限、文件路径、哈希和启动计划之间的冲突，不解释共享数据。新命令与新包目前尚未接入 `LaunchGame.ts`；现有启动入口仍走旧图形实现。迁移 DLSS5 后再统一切换执行入口，删除旧模块与重导出。因此开发包保持 `publish: false`，不能安装该包来替代当前实际使用的 HoYoShade 版本，也不能宣称旧业务已经从本体删除。

ReShade 配置组合、用户设置保留、Add-on 所有权和多文件提交的实现及独立回归已移入 `native/crates/plugin-graphics`，供插件后台共享。该库不依赖应用注册表，也不维护插件 ID。原 `managed_files.rs` 已从本体删除；旧图形入口暂时通过 Cargo 路径依赖和重导出调用插件侧库，这仍是待移除的迁移接线，不能作为最终隔离方案。完成图形后台迁移时必须删除本体对该库的依赖、`managed_reshade` 重导出和 `hot_reshade` 适配入口。

独立回归使用 `cargo test --offline --manifest-path native/Cargo.toml -p ssmt-plugin-graphics`，包含空组合、单贡献、多贡献、大小写不敏感的 Add-on 冲突、配置保留和中断恢复；应用注册表中的启停组合仍由本体的过渡回归覆盖。

## Plugin API 1.1

Schema 2 的 `contributions.backend` 声明包内 EXE、SHA-256、业务方法允许列表及生命周期事件。后台要求 `process.spawn`，新包应声明 `compatibility.pluginApi: >=1.1.0 <2.0.0`。包安装和每次调用都会核对后台文件哈希，不通过 shell 执行业务命令。

`plugin.backend.request` 从有效页面租约解析插件、版本和游戏，传给插件声明的方法。页面不能自行选择其他插件的后台或直接触发生命周期事件。

后台进程从 stdin 接收一个 JSON 请求，向 stdout 返回一个 JSON 响应后退出：

```json
{"protocolVersion":1,"method":"settings.read","event":false,"payload":null,"context":{"pluginId":"example.plugin","pluginVersion":"1.0.0","gameName":"游戏配置","gamePreset":"PRESET","gameExecutable":"C:/Game/Game.exe","packageRoot":"...","dataRoot":"...","externalDependencies":{},"legacySettings":{},"migrations":{}}}
```

```json
{"ok":true,"value":{}}
```

失败响应使用 `{"ok":false,"error":"具体错误"}`。请求与响应各限制为 4 MiB，调用超时 120 秒，超时或取消后终止子进程。该接口用于有界操作；长时间下载和持续观察不应伪装成短调用。

上下文中的 `legacySettings` 仅包含本插件旧命名空间；`migrations` 仅包含清单白名单字段。宿主不解释这些字段。数据位于 `PluginData/<pluginId>/backend`。原生后台以当前用户权限执行，这不是操作系统级沙箱。

生命周期为 `prepare`、`cancel`、`disabled`、`recover`。启动失败会尝试 `cancel`；停用清理成功后才更新启用状态，失败保留可重试状态。声明 `prepare` 的后台未成功准备时，不允许其额外 DLL 注入。未安装或未启用插件不会执行后台。

### 后台启动贡献

后台还可声明 `launch.describe`、`launch.prepare`、`launch.commit`、`launch.cancel`。一项启动贡献必须声明 `launch.prepare` 与 `launch.cancel`。`launch.describe` 不修改游戏状态，返回：

```json
{"exports":{"example.shared-channel":{"pluginOwnedData":true}},"launch":{"priority":10,"resources":["example.exclusive-resource"]}}
```

`exports` 的频道名及数据格式由插件定义。没有独占启动计划时 `launch` 为 `null`；仍可贡献共享数据。宿主最多接受 16 个频道、32 项资源声明。资源名大小写不敏感；高优先级占用者胜出，最高同级冲突拒绝占用，其他无冲突插件继续工作。已输掉其他资源的候选不会额外阻塞无关插件。

`launch.prepare` 收到 `launchContext` 和 `peers`。每个 peer 包含宿主确认的插件 ID、版本、权限、注入风险确认状态、是否被选中及其不透明 `exports`。返回形状如下：

```json
{"dlls":[{"path":"C:/PluginOwned/Library.dll","sha256":"64位十六进制哈希"}],"environment":{"EXAMPLE_PATH":"C:/PluginOwned","UNSET_EXAMPLE":null},"requiredLaunchArgs":["-example"],"excludedLaunchArgs":[],"logSources":[{"name":"Plugin log","path":"C:/PluginOwned/plugin.log"}]}
```

启动计划要求 `game.launch`；DLL 还要求 `native.inject`、`filesystem.read` 和用户风险确认。DLL 必须位于插件包、自己的后台数据/资源目录或已就绪的外部依赖中，真实文件哈希必须匹配。日志只允许这些目录中的文件。环境变量与参数还会与其他计划比较，防止大小写别名绕过冲突检查。`launch.commit` 可选，在计划校验后执行；失败必须能够由 `launch.cancel` 恢复。

任一后台准备或提交失败时，宿主排除其共享数据，取消其他已准备计划后重新准备，并重算低优先级候选。普通启动失败或恢复时也调用 `launch.cancel`，不将它与较早的 `prepare` 成功标记混为一谈。使用共享 Add-on 的插件必须检查来源权限与风险确认；宿主不解读频道内部的可执行内容。

HoYoShade 使用 `ssmt.reshade.v1` 频道，与插件侧图形库的 `reshade_state::Contribution` 对应。该名称和 INI 格式仅出现在插件代码中；宿主只转发频道。

后台上下文另提供自身 `assetRoot`，供插件接管历史 `PluginAssets/<id>`。宿主不会解释目录里的文件名或格式。

`injectedDlls[].executableNames` 是可选文件名过滤条件，由插件自己声明；空列表延续旧包行为。宿主只作不区分大小写的文件名比较，不维护游戏或插件名单。

## Player Tweaks 验证与剩余限制

```powershell
cargo test --offline --manifest-path native/Cargo.toml -p ssmt-player-tweaks-backend
pwsh -File native/tools/build-player-tweaks-backend.ps1 -ArtifactDirectory tmp/plugin-isolation/artifacts
$env:SSMT_PLAYER_TWEAKS_BACKEND = (Resolve-Path tmp/plugin-isolation/artifacts/ssmt-player-tweaks-backend.exe).Path
node --test scripts/tests/test-plugin-owned-backend.mjs
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib plugin::backend
```

进程回归使用临时目录和假游戏路径，检查旧设置首次迁移、逐游戏隔离、设置持久化、INI 内容、未知方法拒绝和空所有权恢复，不写真实游戏注册表。SRMI 的原始 FPS、共享客户端所有权、用户后续修改保留及 JSON 尾部检查另有内存回归。

开发包仍为 `publish: false`。已有本机 DLL 仅用于离线包结构与哈希验证，不代表本次重编或真实游戏验收。页面真实 WebView 交互、两个游戏的启动效果、旧 DLL 残留处理及真实 FPS 恢复仍待验证。

本体的完整插件回归、前端构建通过。沙箱中四项既有冷文件回归出现拒绝访问，在正常权限下单独重跑均通过。

## HoYoShade 独立后台验证

```powershell
pwsh -File native/tools/build-plugin-backends.ps1 -Backends hoyoshade -ArtifactDirectory tmp/plugin-isolation/artifacts
$env:SSMT_HOYOSHADE_BACKEND = (Resolve-Path tmp/plugin-isolation/artifacts/ssmt-hoyoshade-backend.exe).Path
node --test scripts/tests/test-hoyoshade-backend.mjs
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib plugin::backend_launch
```

子进程回归使用假游戏文件与临时资源，不执行 DLL 或真实注入；覆盖多插件共享配置、Add-on 来源确认、停用贡献清理、教程与效果参数保留、外部代理和路径越界。宿主回归还覆盖失败贡献移除、消费者回滚重建和低优先级回退。真实 WebView、完整启动调用链和游戏内效果尚未验收。
