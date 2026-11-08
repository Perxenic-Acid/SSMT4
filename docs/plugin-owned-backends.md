# 插件自有后台与本体边界

本体只理解声明、权限、包内路径、租约与通用生命周期。插件名称、游戏设置字段、组件目录、下载地址、INI 格式及旧状态修复策略属于插件代码。将插件 ID 特判改成专用策略枚举，并不等于完成隔离。

## 当前实现

本体只保留 `backend.rs`、`backend_launch.rs`、通用能力代理、页面租约、权限与包验证。DLSS5、HoYoShade、Player Tweaks 的策略、资源下载、设置、游戏状态检查、配置组合和恢复算法均在各自 Native 后台中执行。本体已删除旧图形模块、专属 Tauri 命令、图形任务代理、专属 Vue 设置组件和固定插件路由，也不再依赖或重导出 `ssmt-plugin-graphics`。

| 插件 | 新包 | 实现 |
|---|---|---|
| Player Tweaks | `ssmt.player-tweaks/0.2.0` | `native/crates/player-tweaks-backend`，包内静态设置页 |
| HoYoShade | `ssmt.hoyoshade.bridge/0.2.0` | `native/crates/hoyoshade-backend`，包内策略与静态状态页 |
| DLSS5 | `ssmt.dlss5.integration/0.3.0` | `native/crates/dlss5-backend`、`native/dlss5-integration-ui` |
| Poser | `ssmt.poser.genshin-probe/0.1.1` | 已通过通用 `runtimePlugins` 接入，无本体业务分支 |

`native/crates/plugin-graphics` 仅由插件后台使用，负责 ReShade 组合、用户设置保留、Add-on 所有权、多文件提交与中断恢复。共享频道及其内容由插件解释，本体不知道 ReShade 的 INI 格式。

`LaunchGame.ts` 已接入通用后台启动计划，合并多个贡献的 DLL、环境变量、参数和日志。资源冲突、准备或提交失败时重新选择候选并撤回消费者。前端合并失败只取消 `launch.cancel`，保留其他已经成功的 `prepare`；整条启动失败才取消全部后台。纯净直启不执行后台。无插件时不读取插件专属文件或依赖状态。

新包均为 `publish: false`。这些源码与离线验证已经接线，不代表通过真实 WebView、真实游戏、显卡或注入验收；正式发布前仍需完成这些验证。旧包不会由本体按 ID 补成新包，使用专属旧入口的包需要升级。

本轮按隔离要求移除了 PR 中的 `graphics.*` 能力和 `graphicsProfiles`，由插件自己的后台方法替代。这些入口未作为兼容分支留在本体；依赖它们的旧页面不能直接复用。页面桥协议仍为 1，整理后的宿主契约见 [机器可读快照](plugin-api-v1-contract.json)。

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

生命周期为 `prepare`、`cancel`、`disabled`、`recover`，并支持只撤回启动贡献的 `launch.cancel`。启动失败会尝试 `cancel`；停用清理成功后才更新启用状态，失败保留可重试状态。声明 `prepare` 的后台未成功准备时，不允许其额外 DLL 注入。未安装或未启用插件不会执行后台。

### 后台启动贡献

后台还可声明 `launch.describe`、`launch.prepare`、`launch.commit`、`launch.cancel`。一项启动贡献必须声明 `launch.prepare` 与 `launch.cancel`。`launch.describe` 不修改游戏状态，返回：

```json
{"exports":{"example.shared-channel":{"pluginOwnedData":true}},"launch":{"priority":10,"resources":["example.exclusive-resource"]}}
```

`exports` 的频道名及数据格式由插件定义。没有独占启动计划时 `launch` 为 `null`；仍可贡献共享数据。宿主最多接受 16 个频道、32 项资源声明。资源名大小写不敏感；高优先级占用者胜出，最高同级冲突拒绝占用，其他无冲突插件继续工作。已输掉其他资源的候选不会额外阻塞无关插件。

`launch.prepare` 收到 `launchContext` 和 `peers`。`launchContext` 包含原游戏路径、是否允许预加载及现有额外 DLL 列表 `existingDlls`；插件据此检查自己的共存要求。宿主在提交前还会对用户额外 DLL 做通用文件名冲突校验。每个 peer 包含宿主确认的插件 ID、版本、权限、注入风险确认状态、是否被选中及其不透明 `exports`。返回形状如下：

```json
{"dlls":[{"path":"C:/PluginOwned/Library.dll","sha256":"64位十六进制哈希"}],"environment":{"EXAMPLE_PATH":"C:/PluginOwned","UNSET_EXAMPLE":null},"requiredLaunchArgs":["-example"],"excludedLaunchArgs":[],"logSources":[{"name":"Plugin log","path":"C:/PluginOwned/plugin.log"}]}
```

启动计划要求 `game.launch`；DLL 还要求 `native.inject`、`filesystem.read` 和用户风险确认。DLL 必须位于插件包、自己的后台数据/资源目录或已就绪的外部依赖中，真实文件哈希必须匹配。日志只允许这些目录中的文件。环境变量与参数还会与其他计划比较，防止大小写别名绕过冲突检查。`launch.commit` 可选，在计划校验后执行；失败必须能够由 `launch.cancel` 恢复。

任一后台准备或提交失败时，宿主排除其共享数据，取消其他已准备计划后重新准备，并重算低优先级候选。若回滚失败，拒绝整份可选计划，不能在尚未归还的资源上继续尝试回退；普通游戏启动仍可继续。普通启动失败或恢复时也调用 `launch.cancel`，不将它与较早的 `prepare` 成功标记混为一谈。使用共享 Add-on 的插件必须检查来源权限与风险确认；宿主不解读频道内部的可执行内容。

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

冷文件算法及其回归已迁入 Native 后台，不再由本体执行。

## HoYoShade 独立后台验证

```powershell
pwsh -File native/tools/build-plugin-backends.ps1 -Backends hoyoshade -ArtifactDirectory tmp/plugin-isolation/artifacts
$env:SSMT_HOYOSHADE_BACKEND = (Resolve-Path tmp/plugin-isolation/artifacts/ssmt-hoyoshade-backend.exe).Path
node --test scripts/tests/test-hoyoshade-backend.mjs
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib plugin::backend_launch
```

子进程回归使用假游戏文件与临时资源，不执行 DLL 或真实注入；覆盖多插件共享配置、Add-on 来源确认、停用贡献清理、教程与效果参数保留、外部代理和路径越界。宿主回归还覆盖失败贡献移除、消费者回滚重建和低优先级回退。真实 WebView 与游戏内效果尚未验收；启动队列的调用链由前端隔离回归验证。

## DLSS5 的任务与恢复

下载、导入、组件选择和旧安装恢复由插件自身的 `task.start/read/active/cancel` 执行。宿主的每次 JSON 请求仍有 120 秒上限；插件任务进程负责下载、进度记录、取消与 30 分钟下载超时。页面重开后可找到当前游戏的活动任务；失效进程的任务会标记失败并保留事务文件以便恢复。任务通过自身资源目录的跨进程锁串行写入；任务文件路径及游戏作用域均校验。

后台上下文提供 `hostProcessId`。任务持有宿主进程句柄，宿主退出时结束异步操作并走原有下载回滚。游戏退出恢复 helper 属于插件代码，SSMT 主程序不再识别专用恢复参数。

GIMI 的两份 AA 记录均由 DLSS5 后台处理，写注册表前持久化原值与写入值。取消或停用时仅恢复仍等于插件写入值的 AA 字段，保留其他画质及玩家后续修改；所有权在恢复成功后释放。这些行为有内存回归，不通过测试写真实游戏注册表。

```powershell
pwsh -NoProfile -File native/tools/build-plugin-backends.ps1 -Backends dlss5 -ArtifactDirectory native/dist/plugin-backend-dev
$env:SSMT_DLSS5_BACKEND = (Resolve-Path native/dist/plugin-backend-dev/ssmt-dlss5-backend.exe).Path
node --test scripts/tests/test-dlss5-backend.mjs
cargo test --offline --manifest-path native/Cargo.toml -p ssmt-dlss5-backend
node scripts/dev/verify-plugin-isolation.mjs
```

构建脚本同时构建 DLSS5 页面；可以复用主工作区依赖，独立检出 Native 时先在 `dlss5-integration-ui` 安装其声明的前端依赖。生成文件位于 `dist`，不提交到包来源。打包使用 `pack-official-plugins.ps1 -DevelopmentPluginId ssmt.dlss5.integration -PluginVersion 0.3.0 -LocalBuild` 并明确传入构建产物目录。其他 DLL 和许可证沿用固定组件构建产物，不伪造缺少的组件。

## 验证层次

独立后台、共享图形库、宿主权限及仲裁、前端启动队列分别回归。`verify-plugin-isolation.mjs` 检查本体不出现专有实现或插件图形库依赖；`verify-plugin-api-v1-contract.mjs` 校验包与协议。

完整包可在临时注册表中验证，不影响用户安装：将 `SSMT_PLUGIN_PACKAGE` 设为实际包路径（多个包使用平台路径分隔符），执行 `cargo test --offline --manifest-path src-tauri/Cargo.toml --lib installs_real_packages_in_isolated_registry -- --ignored`。

独立子进程测试只使用临时资源和假游戏程序，不下载、不执行 DLL、不注入真实游戏。真实组件回归需要相应环境变量，默认忽略；真实游戏、游戏内 NR/FPS/镜头行为、正常关闭 SSMT 后的游戏生命周期仍需实机验收。

### 2026-11-08 离线验证记录

- 本体插件单元测试 118 项通过，2 项环境依赖测试默认忽略；日志跟随测试 3 项通过。
- Native 的 DLSS5、Player Tweaks、共享图形库分别通过 61、5、14 项测试；另有 4 项真实组件或环境测试默认忽略。
- 三个独立后台的真实子进程回归通过；三个实际开发包通过本体安装器的临时注册表安装校验。
- 启动队列、设置页面与新页面桥共 58 项通过，1 项旧 Player Tweaks Vue 页面测试因页面已迁出而跳过。
- 本体及 DLSS5 页面类型检查、构建通过；19 个官方包通过契约检查，本体全部 337 个源码文件通过插件隔离检查。

开发包使用新构建的后台和页面，以及本机已有的固定 DLL 组件；这不等同于重新编译这些 DLL，也不替代实机验收。未执行发行包构建，未推送提交。
