# Player Tweaks 0.2.0 迁移记录

P4-C 把 Player Tweaks 的 GIMI 设置 UI、功能策略、DLL 位置和运行 INI 生成迁入插件包。
SRMI 的注册表 FPS 操作暂时保留一个明确的兼容适配层。

## 已完成

- Player Tweaks 0.2.0 使用 Schema 2 静态页面。
- GIMI 设置保存在插件作用域目录：

```text
PluginData/ssmt.player-tweaks/PlayerTweaks
```

- 插件 UI 生成 `SSMT-Player-Tweaks.ini` 并发布到包内 `runtime/`。
- 启动前宿主通用镜像持久数据，随后通过通用 `injectedDlls` 加载包内 DLL。
- 主应用不再把 Player Tweaks DLL/INI 复制到 3Dmigoto 根目录。
- 旧根目录中的 `SSMT-Player-Tweaks.dll` 和 `.ini` 会作为迁移残留清理。
- 功能策略不再由主应用 `host/features.json` 和 Provider 解释。
- Camera/Gameplay 专用 Vue 组件、TS 配置模块和 Marketplace 分支已删除。

## 旧设置迁移

旧版本将设置保存在当前游戏的 `Config.json`：

```json
{
  "playerTweaksCamera": {},
  "playerTweaksGameplay": {}
}
```

新插件声明：

```json
{
  "migrations": [
    {
      "id": "legacy-game-config",
      "source": "game-config",
      "keys": [
        "playerTweaksCamera",
        "playerTweaksGameplay"
      ]
    }
  ]
}
```

`plugin.migration.read` 只返回这两个字段。插件首次打开时写入新的游戏作用域设置文件，
并生成 GIMI INI；旧 `Config.json` 字段不删除，便于回退。

## GIMI 路径

```text
Player Tweaks static UI
  → plugin.data.write
  → PluginData/.../games/<game>/settings.json
  → SSMT-Player-Tweaks.ini
  → plugin.data.publish
  → package runtime/
  → plugin_launch_injected_dlls
  → Run.exe extra DLL
```

插件 DLL 继续读取自身目录旁的 INI，因此现有 Native 代码无需修改配置格式。

## SRMI 兼容层

SRMI 仍由 `star_rail_tweaks.rs` 修改客户端画质设置中的 FPS。该适配层已经：

- 只读取插件的 `star-rail.json` 设置，不再读取 GameConfig；
- 保留 CN/Global 客户端识别；
- 保留原始 FPS、所有权、备份文件和关闭游戏恢复；
- 只恢复本插件写入的 120 FPS，不覆盖玩家后续修改；
- 在插件禁用时释放对应游戏的设置所有权。

后续应把这部分抽象成通用 `client.settings.transaction` 能力，插件只提交结构化设置事务，
宿主解释允许的注册表值、所有者和恢复策略，从而删除最后的 Player Tweaks 专用 Rust 实现。

## 验证

```powershell
pwsh -File native/tools/build-player-tweaks-ui.ps1
pwsh -File native/tools/pack-official-plugins.ps1 `
  -ReleaseTag local-build `
  -DevelopmentPluginId ssmt.player-tweaks `
  -PluginVersion 0.2.0 `
  -LocalBuild `
  -NativeArtifactDirectory native/dist/Release `
  -OutputDirectory build/plugin-packages
node scripts/dev/verify-player-tweaks-migration.mjs
```

验收覆盖：

- GIMI 和 SRMI 静态页面的实际渲染；
- 游戏切换和 PageHost 重新挂载；
- 旧 GameConfig 字段白名单迁移；
- GIMI INI 生成和包内镜像；
- 包内 DLL 通用注入路径；
- SRMI 设置文件初始化；
- 无专用 Camera/Gameplay Vue 组件依赖。
