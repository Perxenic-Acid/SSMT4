# 鸣潮 DLSS5 与插件边界审查

审查日期：2026-10-16。范围为当前工作树，包括接续开发留下的未提交修改；本轮不修改渲染实现、不启动游戏，也不把其他游戏的成功结果当作鸣潮验收。

## 结论

- 鸣潮已接入 SSMT 的 DLSS5 自动下载、配置和热注入路线，但当前 SSMT 集成路径尚未完成游戏内验收，不能标记为稳定发布能力。
- RHI 方案已有短时实机成功记录，并有对应 Runtime 修复；不能因此保证当前 SSMT 路线、长时间运行或升级后的任意组件组合都稳定。
- SSMT 热注入具有较好的停用隔离和下载失败保护；完整回退、清理和中断恢复尚未闭环。RHI 的备份/卸载也不等于事务性还原。
- 插件基础设施放在本体合理，但当前本体还承载大量插件专有策略与界面，是尚未完成的插件架构；不能只靠移动目录解决。

## 证据与实际路线

检查了用户指定的接续会话、当前源码、鸣潮回传包 `SSMT4-鸣潮修复-20261002.zip`，以及本地 RHI `0e4b20e` 的部署与恢复代码。回传包中有说明和增量 Git bundle，没有原始游戏日志或转储；以下实机结果引用其记录，本轮没有重做实测。

| 路线 | 已有证据 | 证据边界 |
| --- | --- | --- |
| RHI 已部署游戏目录组件，专用脚本预加载 ReShade | 2026-10-02 记录 Mod、ReShade、DLSS5 共存；修复后反复切换角色模型无同类卡死/闪退，NR 持续超过五分钟 | 未验证数小时；有游戏目录代理和脚本改写，不是当前插件热注入路线 |
| 当前 SSMT 鸣潮插件路线 | WWMI + `Client-Win64-Shipping.exe` 选择 `native-bridge` / D3D11；组件、路线和启动队列回归通过 | 未取得这条集成路径的鸣潮成功 NR 帧、画面和退出验收 |
| 接续会话中的崩铁修复 | 用户确认 Mod 恢复；Runtime 补充 `IDXGISwapChain4` 包装转发 | 属于 SRMI 实测，不能替代 WWMI 验收 |

鸣潮当前路径是原生 DLSS 输入 → DX11 Bridge → NR consumer。组件固定为 ReShade 6.8.0、consumer 6.5.3、Bridge 1.4.12，加所选显卡 Profile 的 NR DLL；来源、大小和哈希见 [组件清单](../src-tauri/src/plugin/dlss5_hot_assets.json)。SSMT 直接管理组件，不调用 RHI.exe，也不使用 GIMI 的 FSR2 Bridge。

组件写到 SSMT 的 `PluginAssets/ssmt.dlss5.integration/hot/`，生成独立配置，通过 Run.exe 在恢复游戏主线程前预加载 Runtime 和 ReShade。路线选择见 [dlss5_hot_runtime.rs](../src-tauri/src/plugin/dlss5_hot_runtime.rs)，启动合成见 [LaunchGame.ts](../src/common/LaunchGame.ts)。

回传的 Runtime 修复没有丢失：原提交 `fded60e7` 以 `e6144973` 合入当前历史，两者对 `HackerDevice.cpp/.h` 的 stable patch-id 相同。修复限制系统 DXGI 查询经过 ReShade 时取得原始对象，避免 `DuplicateOutput` 接收到代理适配器；当前源码仍保留该分支。不能仅凭原提交不是 HEAD 的祖先判断补丁缺失。复现与实测边界见 [Runtime 共存说明](../runtime/docs/reshade-compatibility.md)。

## 健壮性与可逆性

### 当前已有保护

- 只在插件已安装且当前游戏启用时准备热注入资源；无插件和纯净启动保留原路径。
- 下载在新自有目录完成，校验归档及提取文件后才切换安装记录和路线；失败的新目录被清理，旧安装记录保留。
- 发现游戏目录 `dxgi.dll`、`d3d11.dll` 或 ReShade 的外部 `INSTALL.BasePath` 时拒绝叠加 Host，不覆盖或删除原管理器的文件。
- 组件缺失、损坏和大部分准备错误只提示本次跳过 DLSS5，继续普通启动。
- 热注入不把 DLL 复制到游戏目录。停用插件后，下次正常生成启动计划时不再贡献这些 DLL 和环境；已经运行的进程需要退出，不承诺在线卸载。

### 尚未闭环的事项

1. **缺少当前 WWMI 集成路线的实机验收。** 至少核对同次日志成功 NR 帧、Mod 替换、角色界面切换、退出与重复启动，再单独安排长时间运行。加载 Add-on、创建普通 DLSS Feature 或短时 WARP 测试都不能替代此项。
2. **输入契约仍依赖玩家。** 当前路线按预设选择 D3D11，界面提示开启游戏原生 DLSS；热注入准备没有核验实际 API 或自动控制鸣潮原生 DLSS。历史脚本显式设置 `-dx11 -krqlv=hd`，新路线没有在这里复用这一步。不能仅凭预设名称认定实际输入已满足。
3. **启停不等于完整还原。** 已有自有目录隔离，但未发现热组件历史版本选择、完整清理和整套状态回退的统一入口。旧 RHI 安装需要通过原管理器恢复；当前检测只能阻止已知冲突，不能自动完成所有旧安装迁移。
4. **配置准备不是整体事务。** `hot_reshade::compose` 在活动目录复制/删除 Add-on，`save_config` 逐文件暂存并 rename；中途失败仍可能留下部分新配置或资源。已有单文件替换保护，但缺少整组配置、Add-on 和安装记录的恢复日志。
5. **部分可选冲突仍会阻断启动。** `LaunchGame.ts` 的 `collectHotRuntimeDlls` 检测额外 DLL 冲突后抛错，两处调用未进入局部可选插件回退，最终进入启动错误处理。需要明确资源所有权的解决方式，并补齐此分支的非模态处理与回归；不能宣称所有 DLSS5 错误都已做到继续启动。

### RHI 的恢复机制不应原样当作保证

本地 RHI 的 `DetailPanelBuilder.NeuralRendering.cs` 中，`DeployWithSentinel` 为原有 DLSS DLL 建立 `.original` 备份，为新文件建立空哨兵，卸载时恢复或删除；共享文件也记录使用者。这些是有效的正常路径恢复措施。

但安装依次部署 ReShade、consumer、DLSS DLL 和 Bridge，外层失败处理未整体撤销已完成步骤；部分部署/恢复异常仅写日志。恢复也没有先比较目标文件是否仍等于本次部署版本，因此游戏更新或其他管理器改写后，旧备份可能覆盖较新的文件。固定版本下载失败还存在回落 Latest 的分支，会改变请求的组合。故只能认定“提供恢复能力”，不能认定“任何失败和后续修改下都能精确还原”。

历史 [鸣潮专用脚本](../scripts/start-wuwa-dlss5-mod.ps1) 会复制游戏目录的 `dxgi.dll` 为 `ReShade64.dll`，备份并改写 `d3dx.ini`；没有对应自动撤销全部修改的步骤。它是特定环境交付入口，不应继续扩展为通用插件安装器。

## 本体与插件的职责

| 留在本体/公共 Runtime 的能力 | 应由插件拥有或声明的内容 |
| --- | --- |
| 包来源验证、权限、路径校验、下载缓存与哈希工具 | 组件来源、版本、Profile、支持游戏与输入要求 |
| 进程创建、注入执行、生命周期、日志及错误展示 | 启动前检查和计划贡献、游戏专有配置策略 |
| 通用资源所有权、冲突检查、共享 Host、事务与恢复机制 | Add-on、效果、环境和恢复意图的具体贡献 |
| 页面/命令注册、隔离和启停框架 | 插件设置、页面及命令内容 |
| 通用 COM 包装兼容性修复 | 无需将这些修复搬到某个插件 |

当前问题有直接证据：

- [PluginContributions](../src-tauri/src/plugin/mod.rs) 只有 runtimePlugins、launcherAdapters、externalManagers、uiPages，尚不能表达实际使用的本体侧准备、资源与启动计划贡献。
- [DLSS5 0.2.1 清单](../native/official-plugin-packages/ssmt.dlss5.integration/0.2.1/ssmt-plugin.json) 的 runtimePlugins、launcherAdapters、uiPages 均为空，主要仍声明 Swapper 管理器与依赖；实际热注入能力由本体识别插件 ID 后提供。
- 游戏路线及 `include_str!` 内嵌组件清单编译进 Rust，本体 `lib.rs` 永久注册插件专用 IPC，`LaunchGame.ts` 显式安排 DLSS5 / Shade 分支和优先级。
- [插件市场](../src/views/PluginMarketplace/PluginMarketplace.vue) 按特定 ID 挂载 DLSS5 / Player Tweaks 界面；UI 清单不能独立交付这些设置。
- 新的“旧 Swapper 依赖可选”修复需要在通用 `launch_coordinator` 中按 DLSS5 ID、版本和三个依赖 ID 特判，说明清单的依赖语义落后于实际路线。

因此当前更接近“由插件安装和启用状态控制的一组内置功能”。关闭隔离已有基础，但插件更新不能独立更新其组件基线、游戏适配策略和界面；插件版本、依赖状态与实际能力也容易脱节。`src-tauri/src/plugin/` 的目录划分有助于维护，却没有形成独立交付边界。

## 建议的推进顺序

1. **补 WWMI 集成验收与失败恢复。** 固定当前组件组合；补齐 API / 原生 DLSS 输入检查、可选冲突回退、配置中断恢复与停用验证，再扩大适配范围。
2. **补齐通用贡献契约。** 由插件声明资源、路线、条件依赖、页面/命令及准备/恢复操作，本体统一收集、检查、执行。移除前端对固定插件数量和执行顺序的假设；保留旧包兼容适配。
3. **逐个迁移专有策略。** 先 DLSS5，再 Shade、Player Tweaks。第一阶段可用静态注册的 provider 收敛调用点；最终还需明确包内资源/声明和受控本体扩展的交付边界，不能把换文件夹或新建本体 crate 当作完成插件化，也不应为此直接执行清单中的任意 JS。

回归集合至少覆盖空组合、单插件、多插件、全部已知插件；停用/移除、下载失败、错误哈希、配置中断、重复启动、外部 RHI 冲突和用户后续改动。不能以禁用某个正常插件组合代替解决共存。

## 本轮验证

- `cargo test --manifest-path src-tauri/Cargo.toml --lib plugin::dlss5_hot_runtime::tests -- --nocapture`：4 项通过；1 项真实归档提取检查按原有设置忽略，未作为本轮通过项。
- `node --test scripts/tests/test-plugin-graphics-settings.mjs scripts/tests/test-efmi-launch.mjs`：32 项通过，使用模拟调用，不启动游戏或加载 DLL。
- 本轮未运行图形夹具、鸣潮实机、长时间稳定性或安装/卸载实测，也未修改其他 AI 的源码和二进制。

## 后续实施清单

- [x] 可选 DLL 冲突进入局部提示与普通启动回退，覆盖 DLSS5 和 Shade。
- [x] 鸣潮热路线仅本次约束 DX11，保留其他参数与保存配置；停用后恢复原启动参数。
- [x] 自有配置和 Add-on 通过带内容比较的多文件事务提交；失败或中断时恢复，后续用户修改保留。
- [x] Add-on 贡献记录实际所有权，组合准备不提前复制/删除文件；关闭贡献只清理仍属于 SSMT 的副本。
- [x] 准备 [鸣潮实机验收步骤](wwmi-dlss5-validation.md)，按用户安排交由其他机器测试。
- [ ] 鸣潮真实原生 DLSS 配置样本、成功 NR 帧、重复启动与长时间验收。
- [ ] 热组件版本回退和完整清理入口。
- [ ] 通用贡献契约及 DLSS5、Shade、Player Tweaks 的逐个迁移。

以上不将待实机部分标记为完成；插件架构工作可在离线验证后继续推进。
