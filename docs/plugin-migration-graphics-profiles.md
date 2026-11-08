# 图形插件 Profile 迁移

> 历史方案：宿主注册专属 provider / graphicsProfiles 的设计已移除。当前插件通过包内后台与页面扩展应用，见 [插件自有后台](plugin-owned-backends.md)。本文仅保留迁移背景，不作为当前接口指南。

P4-D 第一部分把图形 Provider 从“插件 ID 分派”改为“策略 + 优先级 + 资源键”模型。
HoYoShade 已迁移，DLSS5 新版本清单已切换；高风险图形路线的执行行为保持不变。

## 新 Manifest

```json
{
  "schemaVersion": 2,
  "compatibility": {
    "ssmt": ">=4.2.2",
    "pluginApi": ">=1.0.0 <2.0.0",
    "platforms": ["windows-x64"],
    "games": ["GIMI", "SRMI"]
  },
  "contributions": {
    "graphicsProfiles": [
      {
        "id": "graphics",
        "strategy": "reshade-hot",
        "priority": 10,
        "config": "host/graphics.json",
        "settingsComponent": "ssmt.reshade.settings",
        "resources": ["graphics.reshade.host"]
      }
    ]
  },
  "permissions": [
    "native.inject",
    "filesystem.read",
    "filesystem.write"
  ]
}
```

当前通用策略：

| 策略 | 用途 |
|---|---|
| `reshade-hot` | 从外部 ReShade/HoYoShade 资源准备自有热 Host |
| `dlss5-hot` | 准备 Feeder/Native Bridge/原生 DLSS5 热运行计划 |

插件 ID 不构成隐藏分派键。优先级决定候选顺序，资源键声明共享 Host 或排他路线。
宿主验证策略枚举、配置路径、设置组件、资源键、权限和包内文件。

## HoYoShade 0.2.0

HoYoShade Bridge 0.2.0：

- 从 `hostExtensions` 切换到 `graphicsProfiles`；
- 使用 `reshade-hot`；
- 声明 `graphics.reshade.host`；
- 保留现有启动适配器、外部依赖和资源验证；
- 不再要求 Core 通过 HoYoShade 插件 ID 选择 Provider。

开发验收：

```powershell
node scripts/dev/verify-graphics-profiles-migration.mjs
```

脚本创建独立 HoYoShade 资源夹具，检查清单、Profile、候选计划、`ReShade64.dll`、
设置组件和资源键。脚本不启动游戏，也不加载 DLL。

## DLSS5 0.3.0

DLSS5 0.3.0 清单声明：

```json
{
  "strategy": "dlss5-hot",
  "priority": 100,
  "settingsComponent": "ssmt.dlss5.settings",
  "resources": [
    "graphics.dlss5.route",
    "graphics.reshade.host"
  ]
}
```

同时声明独立静态页面：

```json
{
  "kind": "static",
  "id": "settings",
  "title": "DLSS 5 组件",
  "path": "ui/index.html"
}
```

旧 0.2.2 / 0.2.3 包继续通过兼容映射读取，不重打已发布版本。

当前机器没有 DLSS5 0.3.0 所需 Bridge/OptiScaler 构建产物，因此没有生成伪发布包。
正式包必须由 Native CI 从固定源码构建、校验许可证与来源、写入实际哈希，并通过打包回归。

## P4-D2：设置页与后台任务

DLSS5 设置页已迁入 `native/dlss5-integration-ui`，构建产物位于插件包的 `ui/`。
静态页面只通过 PageLease 能力桥工作，不能直接调用 Tauri Command、文件选择器或任意路径：

- 只读能力：`graphics.status.read`、`graphics.routes.read`、
  `graphics.runtime.status`、`graphics.runtime.history`；
- 任务能力：`graphics.task.start/status/cancel`；
- 操作别名：`graphics.runtime.download/import/select/remove`、
  `graphics.route.install/restore`；
- 宿主 UI：`graphics.directory.select` 只允许 `runtime.import` 用途；
- 外部管理器：`graphics.external.open` 只接受插件 Manifest 已声明的管理器 ID。

后台任务返回 `taskId`，状态为 `queued/running/succeeded/failed/cancelled`，并携带
phase、字节进度、稳定错误和最终结果。任务身份来自发起页面的 Lease，状态查询会校验
插件、版本、页面和游戏；下载任务支持取消，其他破坏性操作会完成当前事务并返回结果。
同一插件/游戏同时只允许一个图形任务，全局最多 4 个活动任务；下载任务 30 分钟超时。

断网、哈希失败、页面切换或用户切换游戏不会覆盖一套已验证安装。页面重新挂载后会用
本机保存的任务 ID 尝试续接仍在运行的下载；任务表只保留一小时内的完成记录。

`GameSettingsModal` 已移除 DLSS5 专用 Vue 设置组件。新 0.3.0 包从插件市场通用
`pages` 入口打开设置；旧 0.2.x 包仍保留市场中的兼容 UI，直到旧版本退出支持范围。

开发验收：

```powershell
pwsh -NoProfile -File native/tools/build-dlss5-integration-ui.ps1
pwsh -NoProfile -File native/tools/verify-dlss5-integration-ui.ps1
node --test scripts/tests/test-dlss5-integration-ui.mjs
```

## 后续安排

图形执行器本身仍按 `dlss5-hot` / `reshade-hot` 策略调用现有后端。下一阶段可把下载、
校验、暂存、提交和回滚进一步收敛为通用资源事务，但不得破坏已发布图形路线和旧包回退。

按用户安排，DLSS5 的 Native 资产构建、正式包、实机验收和旧 UI 清理阶段已跳过。
0.3.0 继续保留为 `publish: false` 的开发快照；0.2.x 兼容路线不清理，也不进入新的
发布或测试门槛。P5 直接转向 Plugin API 1 冻结与其余插件平台兼容性。
