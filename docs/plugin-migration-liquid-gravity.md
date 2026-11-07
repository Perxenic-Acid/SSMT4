# Liquid Gravity Probe 0.2.33 迁移记录

本阶段把 Liquid Gravity Probe 从“主应用内建官方插件”迁移为独立插件包。主应用只保留
通用插件平台能力；插件 UI、Python 编辑器、业务配置和免责声明策略由插件包自己持有。

## 迁移前

- 主应用注册 `liquid_gravity_probe_detect`、启动和停止三个专用 Tauri 命令；
- 主应用维护专用 `LiquidGravityProbeProcess`、Python 发现、端口、令牌、日志和进程回收；
- 主应用有一个常驻导航入口和 `/plugins/liquid-gravity-probe` 专用 Vue 路由；
- 插件市场包含 Liquid Gravity 专用免责声明、启用流程和详情内容；
- 插件包只声明 `uiPages`，实际 UI 源码仍编译进主应用。

## 迁移后

插件版本 `0.2.33` 使用 Schema 2：

```json
{
  "schemaVersion": 2,
  "compatibility": {
    "ssmt": ">=4.2.2",
    "pluginApi": ">=1.0.0 <2.0.0",
    "platforms": ["windows-x64"],
    "games": ["EFMI"]
  },
  "contributions": {
    "pages": [
      {
        "kind": "local-service",
        "id": "editor",
        "title": "Liquid Gravity Probe",
        "path": "ui/index.html",
        "service": {
          "executable": "${external.python-runtime}/python.exe",
          "arguments": [
            "-u",
            "${plugin.root}/editor/server.py",
            "--mods",
            "${game.mods}",
            "--probe",
            "${service.data}",
            "--output",
            "${service.data}/output",
            "--port",
            "${service.port}",
            "--instance-token",
            "${service.token}",
            "--parent-pid",
            "${host.pid}"
          ],
          "workingDirectory": "${plugin.root}/editor",
          "dataDirectory": "SSMT_LiquidGravityProbe",
          "healthPath": "/api/health",
          "tokenHeader": "X-Editor-Token"
        }
      }
    ]
  },
  "permissions": [
    "process.spawn",
    "filesystem.read",
    "filesystem.write"
  ]
}
```

通用平台负责：

- Python 外部依赖解析；
- 随机端口和实例令牌；
- 服务健康门控；
- Windows Job Object 进程树回收；
- ServiceLease 和 30 秒空闲宽限；
- 日志、数据目录和插件版本化会话键；
- iframe 到 CapabilityBroker 的受限能力桥。

插件包负责：

- 编辑器 UI 源码和独立 Vite 构建；
- Python `server.py` 与运行时资源；
- Mod 扫描、喷口配置、效果参数和按键绑定；
- 免责声明内容与同意状态；
- 具体业务错误和写入逻辑。

## 数据兼容

新服务仍使用：

```text
Mods/SSMT_LiquidGravityProbe
```

因此旧的 `Probe.ini`、`surface/`、角色配置和 `output/` 不需要迁移。旧版保存的同意状态
键 `disclaimer_accepted_version` 继续读取；未同意的用户会在插件页面内看到插件自己的
免责声明。

## 入口兼容

旧地址：

```text
/plugins/liquid-gravity-probe
```

会重定向到：

```text
/plugins?plugin=ssmt.liquid-gravity-probe
```

这条重定向是临时兼容桥，不包含插件业务实现。独立 Nav 入口已删除，插件页面统一从
插件市场进入。

## 源码边界

主应用不再包含：

- `src/views/LiquidGravityProbe/`；
- Liquid Gravity 专用 Vue 主机与 3D 视口；
- 专用免责声明组件；
- 专用同意和启用辅助模块；
- `liquid-gravity-probe.html`；
- `src-tauri/src/commands/liquid_gravity_probe.rs`；
- 三个专用 Tauri 命令。

插件 UI 源码位于 Native 仓库：

```text
native/liquid-gravity-probe-ui/
```

构建目标：

```text
native/official-plugin-packages/ssmt.liquid-gravity-probe/0.2.33/ui/
```

## 验证

构建插件 UI：

```powershell
pwsh -File native/tools/build-liquid-gravity-probe-ui.ps1
```

构建本地插件包：

```powershell
pwsh -File native/tools/pack-official-plugins.ps1 `
  -ReleaseTag local-build `
  -DevelopmentPluginId ssmt.liquid-gravity-probe `
  -PluginVersion 0.2.33 `
  -LocalBuild `
  -OutputDirectory build/plugin-packages
```

启动开发版 SSMT4 后：

```powershell
node scripts/dev/verify-liquid-gravity-probe-migration.mjs
```

验收覆盖：

- Schema 2 和 `pages[]` 被正式包校验器接受；
- 通用 ServiceLease 能启动 Python 编辑器并通过健康检查；
- iframe 加载插件自带 UI；
- 旧数据目录保持不变；
- 旧地址重定向到市场；
- 独立 Nav 不存在；
- 服务停止后无专用命令依赖。

## 回退

迁移保留旧版本目录 `0.2.32` 和旧数据目录。若 0.2.33 需要回退，可重新安装已锁定的旧包；
但旧包依赖的主应用专用命令已在 P4 删除，因此生产回退应回退到迁移前的 SSMT 主程序版本，
而不是只降级插件包。正式发布前应把 0.2.33 与目标 SSMT 版本作为一组兼容组合验收。
