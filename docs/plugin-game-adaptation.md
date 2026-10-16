# 插件逐游戏适配

按顺序推进，每项分别提交。安装和离线检查通过不等于游戏内渲染验收通过。

## 清单

- [x] 原神 MMD Poser：限制为 GIMI；配置入口、后端启用校验和旧状态过滤一起修复。提交 `b9a1e0e`，插件 Rust 测试、界面回归和前端构建通过。
- [x] DLSS5 配置、下载与启动集成：保留 GIMI 专用 FSR2 热注入链，扩展以下路线。
  - [x] 崩铁 SRMI：无原生 DLSS，使用 Feeder 合成输入及运动矢量效果。
  - [x] 鸣潮 WWMI：沿用已验证的 RHI 原生 DLSS DX11 Bridge + NR consumer 路线，由 SSMT 管理下载和注入。
  - [x] 绝区零 ZZMI / ZZMIDX12：按 DX11 Bridge / DX12 原生输入分别准备，游戏中必须启用 DLSS。
  - [ ] 新的 SSMT 集成路径实机确认成功 NR 帧、画面与退出行为。不能用之前的 RHI 实测替代这一项。
- [ ] Player Tweaks：扩展崩铁，功能按游戏区分；不能把原神地址、特征或相机功能直接套到崩铁。

## 验收约束

插件未安装或未启用时不下载、不扫描专用资源、不复制 DLL、不注入。可选组件准备失败时横幅提示并继续普通启动。检查空组合、单插件、多个插件与全部已知插件；同一进程只能有一套 ReShade Host，关闭插件后不能遗留其 Add-on 贡献。

其他游戏的组件使用独立清单与安装目录，不能复用 GIMI 特有的 Bridge 或混搭 NR 链。固定下载版本、大小与 SHA-256，新目录准备完成后才切换设置；不执行下载包内脚本、不伪造 RHI 的安装记录、不覆盖其文件。

## DLSS5 实现与验证

插件市场 → 选择游戏 → 已启用的 DLSS5 → 功能设置 → 选择显卡 → 自动下载并配置。原神继续使用原有组件包；其他三款游戏使用 `dlss5_hot_assets.json` 固定的 ReShade 6.8.0、NR consumer 6.5.3、Bridge 1.4.12、Feeder 1.19.0 和配套 snippet。新增组件存放在 `PluginAssets/ssmt.dlss5.integration/hot/`，启动前合并启用的 Shade Bridge 贡献，只注入一个 Host。

已核验所有下载包及实际提取文件。六组路线 / 显卡组合均通过真实组件解压、完整性与配置检查。另用现有 Runtime Context 夹具验证 Feeder + consumer、DX11 Bridge + consumer 的热加载及 WARP Context 调用，两项退出码均为 0，ReShade 日志确认对应 Add-on 加载。该夹具没有游戏帧和 DLSS 请求，不能证明运动矢量质量、DX12 游戏兼容或 NR 已生效。

真实组件检查默认忽略，需要显式指定本地核验包目录后执行：

```powershell
$env:SSMT_DLSS5_COMPONENT_PROBE_DIR = '已下载固定组件的隔离目录'
cargo test --lib extracts_real_pinned_components_and_prepares_each_route -- --ignored --nocapture
```

目录文件名依次为 `reshade.exe`、`consumer.zip`、`bridge.addon64`、`feeder.zip`、`sr.zip`、`nr50.zip`、`nr30.zip`、`lumenite.zip`、`ReShade.fxh`、`ReShadeUI.fxh`，来源及哈希以清单为准。检查不会启动游戏或修改游戏目录。

## 依据

- 本地 `RHI` 的 `DetailPanelBuilder.NeuralRendering.cs` 和 `Renodx5AddonService.cs`。
- [DX11 DLSS Bridge](https://github.com/NIGos/dlss5-bridge)。
- [无原生 DLSS 的 Feeder](https://github.com/jlrouzies-fr/DLSS5-Feeder)。
- [绝区零原生 DLSS 公告](https://www.nvidia.com/en-us/geforce/news/zenless-zone-zero-version-dlss-4-5-ray-tracing/)。
