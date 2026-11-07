# SSMT 插件 SDK（Plugin API 1）

本文描述 P2 阶段已经落地的第三方插件契约。目标是让一个从未出现在 SSMT 源码中的
插件，通过 `.ssmtpkg` 独立完成安装、页面展示、服务启动、授权数据读写和卸载，而不修改
SSMT4 本体。

当前契约是已冻结的 Plugin API 1。`1.x` 只允许兼容扩展；删除、重命名或改变现有能力、
页面和解桥语义时必须提升主版本。插件通过 `compatibility.pluginApi` 协商兼容性。

机器可读快照和升级规则见 [Plugin API 1 冻结契约](plugin-api-v1-contract.md)。

## 1. Manifest

新插件使用 Schema 2：

```json
{
  "schemaVersion": 2,
  "id": "example.codex.page-sdk",
  "name": "Example Plugin SDK",
  "version": "1.0.1",
  "author": "Example",
  "compatibility": {
    "ssmt": ">=4.2.2",
    "pluginApi": ">=1.0.0 <2.0.0",
    "platforms": ["windows-x64"],
    "games": ["EFMI"]
  },
  "contributions": {
    "pages": []
  },
  "permissions": []
}
```

规则：

- `schemaVersion` 与 `pluginApi` 分开管理；Schema 描述清单结构，Plugin API 描述运行契约。
- Schema 2 必须声明 `compatibility.pluginApi`。
- Plugin API 版本范围接受逗号或空格分隔，例如 `>=1.0.0, <2.0.0` 和
  `>=1.0.0 <2.0.0`。
- Schema 1 的 `pluginApiVersion: 1` 继续兼容读取；新插件不应继续使用该字段表达能力范围。
- 插件 ID 不得依赖宿主白名单；官方前缀不自动获得额外权限。

## 2. 页面

`contributions.pages` 支持三类页面：

| 类型 | 用途 | 能力桥 |
|---|---|---|
| `declarative` | 布尔、整数、字符串和下拉框设置 | 宿主直接使用 CapabilityBroker |
| `static` | 包内静态 HTML 展示 | 支持 PageLease 能力桥 |
| `local-service` | 插件自带 Python、Node 或 EXE UI/服务 | 支持 Lease 绑定消息桥 |

需要复杂编辑器、框架 UI 或数据读写时，使用 `local-service`。服务程序、UI 构建资源、
业务逻辑和业务参数都应留在插件包内。

需要参与游戏启动的插件使用 `contributions.launchHooks`。宿主不会执行插件提供的脚本，
只解释经过校验的文件动作和启动参数；详细契约见
[插件启动计划](plugin-launch-plan.md)。

## 3. CapabilityBroker

宿主提供 `plugin_capability_catalog()`，返回当前能力 API 版本、插件 API 版本、能力 ID、
所需权限以及输入输出 JSON Schema。

Plugin API 1 当前能力：

| 能力 | 权限 | 说明 |
|---|---|---|
| `plugin.settings.get` | 无 | 读取插件设置命名空间 |
| `plugin.settings.set` | 无 | 写入插件设置命名空间 |
| `plugin.settings.remove` | 无 | 删除插件设置键 |
| `plugin.storage.get` | `filesystem.read` | 读取插件作用域存储 |
| `plugin.storage.set` | `filesystem.write` | 写入插件作用域存储 |
| `plugin.storage.remove` | `filesystem.write` | 删除插件作用域存储 |
| `game.context.read` | 无 | 读取经过筛选的游戏上下文 |
| `plugin.data.list/read/write/remove` | 文件读写权限 | 访问声明的插件作用域数据目录 |
| `plugin.data.runtime.read` | `filesystem.read` | 读取包内运行时镜像文件 |
| `plugin.data.publish` | `filesystem.write` | 把持久数据镜像到包内运行目录 |
| `game.process.read` | `process.observe` | 判断配置的游戏进程是否运行 |
| `plugin.migration.read` | `filesystem.read` | 只读取清单声明的旧 GameConfig 顶层字段 |
| `graphics.status.read/routes.read` | `filesystem.read` | 读取图形状态、上次运行证据和可选路线 |
| `graphics.runtime.status/history` | `filesystem.read` | 读取已校验运行时和实例自有历史 |
| `graphics.task.start/status/cancel` | 按操作声明 | 启动、轮询或取消宿主图形任务 |
| `graphics.runtime.download/import/select/remove` | 文件读写；下载另需 `network` | 通过任务代理管理图形运行时 |
| `graphics.route.install/restore` | 文件读写、`native.inject`、`process.spawn` | 通过任务代理安装或恢复图形路线 |
| `graphics.directory.select` | `filesystem.read` | 仅允许为 `runtime.import` 打开宿主目录选择器 |
| `graphics.external.open` | `process.spawn` | 只打开 Manifest 已声明的外部管理器 |

插件不能通过能力 ID 执行任意 Shell、写任意绝对路径、注入任意进程或调用任意 Tauri
Command。新增高危能力必须单独定义 Schema、权限、错误码、超时和负向测试。

## 4. 请求身份

受信宿主页面可以调用 `plugin_capability_request`。插件 iframe 不直接调用 Tauri，
而是通过 `PluginPageHost` 的版本化消息桥调用
`plugin_capability_request_for_lease`。

```text
Plugin iframe
  └─ postMessage(capability, payload, requestId)
       └─ UIHost 校验 origin + contentWindow + active lease
            └─ CapabilityBroker 从 lease 解析 pluginId/gameName
                 └─ 权限策略 + 插件身份 + API 兼容性
                      └─ 能力实现
```

插件页面不能自行提交插件 ID。Lease 失效、服务停止、插件禁用或卸载后，新的能力请求
返回 `lease_invalid` 或相应策略错误。

消息协议和示例见 [插件页面运行层](plugin-pages.md#iframe-能力桥)。

## 5. 数据作用域

- 设置键以 `pluginId` 分区，禁止空键、控制字符和路径分隔符。
- 通用页面字段使用 `page:<pageId>:<fieldKey>` 保存。
- `plugin.storage.*` 使用同一插件分区中的 `storage:<key>` 键。
- 卸载插件时清理该插件的设置命名空间。
- 插件业务服务自己的数据目录由清单声明，宿主创建并在页面上提供打开入口。

`plugin.storage.*` 是小型键值存储。复杂预设和角色数据使用
`contributions.dataDirectories` 声明的插件作用域目录：持久文件位于
`PluginData/<pluginId>/<dataId>`，插件升级不会覆盖；需要兼容旧原生运行时的目录可在
启动前镜像回插件包。宿主记录每次镜像拥有的文件清单；持久目录中已删除的文件会在
下一次发布时从镜像中同步删除，镜像之外的原生种子文件不会被清理。

旧版宿主把插件设置保存在 `GameConfig` 时，插件可声明迁移字段：

```json
{
  "migrations": [
    {
      "id": "legacy-game-config",
      "source": "game-config",
      "keys": ["playerTweaksCamera", "playerTweaksGameplay"]
    }
  ]
}
```

宿主只返回这些顶层字段，不返回完整配置、路径、密钥或其他插件数据。

图形插件通过 `contributions.graphicsProfiles` 声明平台策略：

```json
{
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
}
```

当前通用策略为 `reshade-hot` 和 `dlss5-hot`。插件 ID 不构成隐藏分派键；优先级决定候选
顺序，资源键用于声明共享 Host 或排他路线。策略执行器只接受插件包内配置和已校验资源。

复杂图形设置 UI 应作为 Manifest `contributions.pages` 中的 `static` 页面分发。
页面通过能力桥启动后台任务并轮询 `taskId`，不直接调用专用 Tauri Command；这样插件更新
UI 不会要求宿主重新编译，宿主升级也不会把内部 Command 名称变成插件 ABI。

## 6. 错误规范

能力响应不依赖异常文本判断结果：

```json
{
  "requestId": "request-1",
  "capability": "plugin.storage.set",
  "ok": false,
  "errorCode": "capability_denied",
  "errorMessage": "plugin is missing permission: FilesystemWrite"
}
```

当前稳定错误码：

| 错误码 | 含义 |
|---|---|
| `plugin_not_installed` | 插件身份不存在 |
| `plugin_disabled` | 插件未在当前游戏启用 |
| `plugin_api_incompatible` | 插件 API 范围与宿主不兼容 |
| `lease_invalid` | 页面 Lease 缺失、过期或已停止 |
| `capability_not_supported` | 宿主不认识该能力 |
| `capability_denied` | 插件缺少所需权限 |
| `invalid_payload` | 请求缺少字段或格式错误 |
| `storage_unavailable` / `storage_error` | 设置存储不可用或 I/O 失败 |
| `game_context_unavailable` / `game_context_invalid` | 游戏上下文不存在或无法解析 |
| `plugin_registry_unavailable` | 插件注册表不可用 |
| `bridge_error` | 宿主桥内部错误 |

插件应记录 `requestId` 和 `errorCode`，不要把面向用户的错误判断建立在英文
`errorMessage` 上。

## 7. 开发与验证

完整陌生插件示例位于 `examples/plugin-sdk-fixture/`。开发流程：

```powershell
.\scripts\dev\build-plugin-sdk-fixture.ps1
bun run check:plugin-api-contract
```

启动开发版 SSMT4 后：

```powershell
node scripts/dev/verify-plugin-sdk-fixture.mjs
```

该回归验证：

- 陌生 ID 本地发现、安装和启用；
- Schema 2 与空格分隔 `pluginApi`；
- Capability 目录与设置读写；
- 声明式页面和本地服务页面注册；
- 服务健康、iframe 实际加载与业务资源；
- iframe 经 Lease 桥完成设置写入和读回；
- 禁用插件请求被拒绝；
- 不存在或伪造 Lease 被拒绝；
- 服务显式停止、进程回收和插件卸载。

插件内容变化必须提升 `version`。同一 `pluginId@version` 不应重新打包不同实现；
升级后的新版本使用新的 ServiceInstance 键并清理旧服务会话。

## 8. 安全边界

- Manifest 权限是宿主能力策略输入，不是操作系统沙箱。
- 本地服务仍以当前 Windows 用户权限运行。
- iframe 只能加载插件包声明的页面；服务只绑定回环地址。
- 静态页面只能通过当前租约对应的随机回环端口和路径令牌加载，不能直接访问全局
  `asset.localhost` 或其他插件目录。
- 静态页面的浏览器 Origin 不保证跨挂载保持不变，不能把 `localStorage`、Cookie 或
  IndexedDB 当作插件配置的权威存储。
- token 放在 URL fragment，健康检查和业务 API 使用实例令牌。
- 消息桥同时校验来源 Origin、`contentWindow` 和有效 Lease。
- 静态页面能力绑定随机 `StaticPageLease`；不得通过放宽 iframe sandbox 绕过来源和
  `contentWindow` 校验。
- 页面 Lease 需要持续续期；WebView 崩溃或强制重载后，后端会在 TTL 到期时清理遗留的
  静态页服务和本地服务进程。
- Windows Job Object 负责进程树生命周期，不负责文件系统和账户隔离。
