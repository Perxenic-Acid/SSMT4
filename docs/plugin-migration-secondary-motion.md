# Secondary Motion 迁移记录

P4-B 把 Secondary Motion 的预设处理、角色参数编辑和设置 UI 从主应用迁移到独立插件包。
Native DLL 注入路径保持不变，旧用户数据继续使用原目录。

## 平台新增能力

### Static PageLease

静态插件页面不需要后台进程，但同样需要受限能力访问。宿主现在为静态 iframe 创建随机
`StaticPageLease`：

- 页面由 Lease 独占的随机回环端口和路径令牌承载，只开放 `ui/` 页面目录；
- iframe fragment 携带 Lease ID；
- 能力桥校验 `event.source`、Origin 和当前有效 Lease；
- 页面每 15 秒续期，120 秒无心跳后由宿主回收；
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

Secondary Motion 3.3.4 使用静态页面：

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
  -PluginVersion 3.3.4 `
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

## 3.3.2 总开关修复

- 骨骼物理总开关默认开启。
- 切换总开关后立即写入 `runtime/config.json` 并发布到包内运行目录，不再依赖额外点击
  “应用配置”，因此切换页面后不会回滚。
- 旧配置只有 `enabled: false`、没有 `enabled_user_set` 时视为旧版默认值，一次性迁移为
  开启并递增 revision。
- 新版本用户主动关闭会写入 `enabled_user_set: true`，后续重新打开页面仍保持关闭。
- Secondary Motion 插件在 SSMT 中禁用后 `sbm.dll` 不再注入，也不会产生骨骼写入；
  重新启用时恢复已保存的总开关状态，首次安装默认开启。

## 3.3.3 管理器布局恢复

- 恢复上游 Manager 的桌面结构：左侧窄栏显示当前角色、预设、运行时状态和分区导航，
  右侧为主参数编辑区。
- 基础参数与骨骼绑定、步态、高级参数、跳跃/惯性分别使用紧凑卡片，不再把参数树铺成
  七个纵向折叠项。
- 步态参数恢复为按 idle / walk / run / sprint / zipline 排列的动态表格，并保留数据中
  出现的额外列。
- “保存角色”和“应用设置”固定在编辑区顶部；预设与 revision 在左侧栏操作。
- 原生注入免责声明收进默认折叠提示，必要风险信息仍保留。
- `3.3.2` 已安装版本保持字节不变；布局修复作为新的 `3.3.3` 版本发布。

## 3.3.4 文字可读性修复

- 辅助说明、侧栏状态、步态表头和参数路径不再使用 8–9px 的小字号。
- 正文辅助文字提升到 11–12px，参数值输入统一为 12px。
- 局部灰色文字提高对比度，同时保留面板层级和紧凑布局。
- `3.3.3` 已安装版本保持字节不变；可读性修复作为新的 `3.3.4` 版本发布。
