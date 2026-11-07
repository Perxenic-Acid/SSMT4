# Secondary Motion 3.3.1 迁移记录

P4-B 把 Secondary Motion 的预设处理、角色参数编辑和设置 UI 从主应用迁移到独立插件包。
Native DLL 注入路径保持不变，旧用户数据继续使用原目录。

## 平台新增能力

### Static PageLease

静态插件页面不需要后台进程，但同样需要受限能力访问。宿主现在为静态 iframe 创建随机
`StaticPageLease`：

- iframe fragment 携带 Lease ID；
- 能力桥校验 `event.source`、Origin 和当前有效 Lease；
- 离开路由立即释放；
- 插件无法自报 `pluginId` 或 `gameName`。

### 插件作用域数据目录

Manifest 使用：

```json
{
  "dataDirectories": [
    {
      "id": "SecondaryMotion",
      "packagePath": "SecondaryMotion",
      "seedFromPackage": true,
      "mirrorOnLaunch": true
    }
  ]
}
```

持久路径：

```text
PluginData/ssmt.secondary-motion/SecondaryMotion
```

该路径与旧版宿主使用的位置一致，因此已有 `runtime/`、`presets/`、`data/` 和
`surface/` 数据无需迁移。

启动前，宿主把所有已启用插件中声明 `mirrorOnLaunch` 的目录复制到包内
`packagePath`。这样现有 `sbm.dll` 继续读取包内 `SecondaryMotion/runtime/config.json`
和状态文件，同时用户数据不会因插件升级被包内容覆盖。

## 插件页面

Secondary Motion 3.3.1 使用静态页面：

```json
{
  "pages": [
    {
      "kind": "static",
      "id": "settings",
      "title": "Secondary Motion",
      "path": "ui/index.html"
    }
  ]
}
```

页面通过以下通用能力工作：

- `plugin.data.list`
- `plugin.data.read`
- `plugin.data.runtime.read`
- `plugin.data.write`
- `plugin.data.remove`
- `plugin.data.publish`
- `game.process.read`

页面自己负责：

- 读取和合并角色默认参数；
- 修改预设并写入 `.json.bak`；
- 更新 `runtime/config.json` revision；
- 提交后镜像到包内运行目录；
- 显示 DLL 写出的 `runtime_status.json`；
- 中英文界面、风险提示和维护说明。

## 主应用删除项

- `src-tauri/src/plugin/secondary_motion.rs`
- `secondary_motion_snapshot`
- `update_secondary_motion_settings`
- `update_secondary_motion_character`
- `src/plugin/components/SecondaryMotionSettings.vue`
- Marketplace 中 `ssmt.secondary-motion.settings` 专用分支
- 专属原项目链接映射

`plugin_launch_injected_dlls` 已迁为通用模块 `injected_dlls.rs`，不再包含任何
Secondary Motion 特判。

## 验证

```powershell
pwsh -File native/tools/build-secondary-motion-ui.ps1
pwsh -File native/tools/pack-official-plugins.ps1 `
  -ReleaseTag local-build `
  -DevelopmentPluginId ssmt.secondary-motion `
  -PluginVersion 3.3.1 `
  -LocalBuild `
  -NativeArtifactDirectory native/dist/Release `
  -OutputDirectory build/plugin-packages
node scripts/dev/verify-secondary-motion-migration.mjs
```

验收覆盖：

- 静态页面和 PageLease；
- 插件作用域数据读写与发布；
- 旧持久数据目录；
- 前端实际渲染和暗色主题；
- 插件升级后 PageHost 按版本重新挂载；
- 启动镜像通用化，不依赖 Secondary Motion ID。
