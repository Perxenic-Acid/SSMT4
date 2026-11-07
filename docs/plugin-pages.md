# 插件页面运行层

插件页面运行层把插件 UI 和 SSMT 内部组件解耦。插件清单只声明页面类型和受控需求，前端不直接导入插件源码，也不接收任意可执行路径。

## 清单版本

当前清单 Schema 为 2。新插件应显式声明 Manifest 版本和插件 API 兼容范围：

```json
{
  "schemaVersion": 2,
  "compatibility": {
    "ssmt": ">=4.2.2",
    "pluginApi": ">=1.0.0 <2.0.0",
    "platforms": ["windows-x64"],
    "games": ["EFMI"]
  }
}
```

`schemaVersion` 描述清单字段结构，`compatibility.pluginApi` 描述插件依赖的宿主能力协议，
二者独立演进。Schema 1 可继续使用 `pluginApiVersion: 1`；旧清单未声明 API 时按 1 处理。
Schema 2 必须声明 `pluginApi`，高于宿主支持范围或格式错误的声明会在清单校验阶段拒绝。
版本范围接受 `>=1.0.0, <2.0.0` 和 `>=1.0.0 <2.0.0` 两种常见写法。

Plugin API 和方法桥协议现已冻结；机器可读版本、能力与兼容规则见
[Plugin API 1 冻结契约](plugin-api-v1-contract.md)。页面宿主必须使用
`PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION` 和统一的请求/响应类型常量，不能再散落硬编码。

## 页面类型

### 声明式

适合布尔、整数、字符串和下拉框设置。值通过现有插件设置存储按
`page:<pageId>:<fieldKey>` 保存，不启动独立进程。

### 静态页面

页面来自插件包内的相对路径。宿主不再把第三方页面挂到 `asset.localhost`，而是为每次
静态页面 Lease 启动仅绑定回环地址的只读文件服务：

- 随机端口、随机路径令牌和独立浏览器 Origin；
- 只允许读取当前 `ui/` 页面目录内的普通文件；
- 拒绝目录穿越、符号链接、目录列表、非 GET/HEAD 请求和超大文件；
- 响应 CSP 禁止访问 `asset:`、`file:` 和父页面的脚本环境；
- 只有清单声明 `network` 权限时，页面 CSP 才允许外部 HTTPS 资源；不会重新开放
  `asset.localhost` 或任意本地 HTTP 地址。

宿主为静态页面创建随机 `StaticPageLease`，返回带令牌的本地 URL；iframe fragment
携带不透明 Lease ID。离开路由或组件销毁时同时释放 Lease 和静态文件服务。
静态页面的 Origin 按 Lease 隔离，`localStorage`、`sessionStorage` 和 IndexedDB
不能作为持久设置存储；需要持久化的数据必须使用插件设置或声明式数据目录能力。

### 本地服务

适合 Python、Node 或独立 EXE 编辑器。SSMT 负责：

- 校验插件已安装并启用；
- 校验 `compatibility.pluginApi`、外部依赖和 `process.spawn` 权限；
- 从当前游戏 `Config.json` 解析 Mods、游戏目录和目标 EXE；
- 分配本机随机端口与实例令牌；
- 启动清单声明的可执行文件；
- 等待健康检查；
- 把服务加入 Windows Job Object；
- 页面通过 `ServiceLease` 使用服务，服务本身不归页面所有；
- 页面每 15 秒续期 Lease，超过 120 秒没有心跳的 Lease 会被后台清理；
- 所有租约释放后保留 30 秒宽限期，再停止进程树；
- 插件禁用、更新、卸载和 SSMT 退出时回收会话。

示例：

```json
{
  "kind": "local-service",
  "id": "editor",
  "title": "流光编辑器",
  "path": "ui/index.html",
  "service": {
    "executable": "${external.python-runtime}/python.exe",
    "arguments": [
      "-u",
      "${plugin.root}/editor/server.py",
      "--mods", "${game.mods}",
      "--data", "${service.data}",
      "--port", "${service.port}",
      "--instance-token", "${service.token}",
      "--parent-pid", "${host.pid}"
    ],
    "workingDirectory": "${plugin.root}/editor",
    "dataDirectory": "SSMT_EfmiGlowStudio",
    "healthPath": "/api/health",
    "tokenHeader": "X-Editor-Token",
    "readyTimeoutMs": 45000
  }
}
```

## 模板变量

只允许：

- `${plugin.root}`
- `${external.<dependencyId>}`
- `${game.exe}`
- `${game.directory}`
- `${game.mods}`
- `${service.port}`
- `${service.token}`
- `${service.data}`
- `${host.pid}`

参数按独立数组项传给进程，不经过 Shell。可执行文件、工作目录和参数均先校验模板变量和包路径，再启动进程。

## 服务与租约

服务键由插件 ID、插件版本、页面 ID 和游戏名组成。同一服务键只启动一个
`ServiceInstance`；每次打开页面 Acquire 一个独立 `ServiceLease`。多个页面或消费者可以
共享服务，但不会共享租约所有权。

```text
UI Page ─Acquire→ ServiceLease ─→ ServiceInstance
UI Page ─Release→ ServiceLease
```

页面卸载、路由失活或标签切换只 Release Lease，不直接拥有或停止服务。所有 Lease
释放后服务进入空闲宽限；再次 Acquire 会复用进程。显式停止、插件禁用、更新、卸载
才立即回收 ServiceInstance 和进程树。

## 前端接口

- `plugin_page_catalog(gameName)`：返回当前启用插件的页面描述。
- `acquire_plugin_page_lease(pluginId, pageId, gameName)`：创建或复用 ServiceInstance，
  并返回新 Lease。
- `release_plugin_page_lease(leaseId)`：释放 Lease；最后一个 Lease 释放后进入空闲宽限。
- `touch_plugin_page_lease(leaseId)`：为当前页面续期；TTL 过期后不可复活。
- `plugin_page_session_status(sessionId)`：按 Lease 读取可用服务信息。
- `plugin_page_service_status(serviceKey)`：读取服务、Lease 数量和进程状态。
- `start_plugin_page_service(...)`：兼容入口，等价于 Acquire Lease。
- `stop_plugin_page_service(sessionId)`：显式停止整个 ServiceInstance。
- `stop_plugin_page_services_for_plugin(pluginId)`：插件级清理。
- `plugin_capability_catalog()`：返回版本化能力清单、输入输出 Schema 和权限要求。
- `plugin_capability_request_for_lease(...)`：由当前 Lease 解析插件身份并执行能力请求。
- `acquire_static_plugin_page_lease(...)` / `release_static_plugin_page_lease(...)`：
  为静态页面创建和释放页面 Lease，同时管理该页面的隔离只读文件服务。

前端只接收不透明 URL、令牌和 `sessionId`，不传 Migoto 路径、Python 路径或任意可执行文件。

## iframe 能力桥

本地服务页和静态页通过 `postMessage` 请求宿主能力。宿主同时校验以下条件：

- `event.source` 必须等于当前服务 iframe 的 `contentWindow`；
- `event.origin` 必须等于当前服务 URL 的 Origin；
- 请求必须声明桥协议版本 1、唯一 `requestId`、能力 ID 和对象型 payload；
- `leaseId` 必须仍属于当前页面会话，插件 ID 和游戏名由后端从 Lease 解析。

请求：

```js
window.parent.postMessage({
  type: 'ssmt:capability-request',
  protocolVersion: 1,
  requestId: 'request-1',
  capability: 'plugin.settings.get',
  payload: { key: 'mode' },
}, '*')
```

响应：

```js
{
  type: 'ssmt:capability-response',
  protocolVersion: 1,
  requestId: 'request-1',
  capability: 'plugin.settings.get',
  ok: true,
  value: { value: 'fast' }
}
```

页面不能直接提交 `pluginId`、`gameName` 或任意 Tauri 命令。静态页面的能力同样绑定
`StaticPageLease`；Tauri asset URL 必须保留 `index.html` 的路径分段，使 `./assets/*`
解析到同目录资源。

插件市场由全局 `<KeepAlive>` 包裹。离开插件页时通常触发 `onDeactivated`，不会触发
`onBeforeUnmount`；页面宿主必须分别处理：

- `onDeactivated`：Release Lease，由后端在最后一个 Lease 释放后开始空闲计时；
- `onActivated`：重新 Acquire Lease，宽限期内会复用原 ServiceInstance；
- `onBeforeUnmount`：应用真正卸载时再次安排清理。

## 安全边界

- 本地服务仍以当前 Windows 用户权限运行，不等同于浏览器沙箱。
- 服务只允许绑定回环地址；URL 令牌放在 fragment。
- 健康检查只访问清单声明的本地路径，不跟随公网地址。
- iframe 禁止弹窗、顶层导航和访问 SSMT DOM。
- 生命周期与会话键包含插件版本，插件升级后旧会话必须重启。
- Lease 失效、插件禁用或服务停止后，能力桥立即拒绝新请求。

## 回归

开发版启动后运行：

```powershell
node scripts/dev/verify-plugin-page-runtime.mjs
```

脚本会安装/启用开发包、通过 UI 打开页面、验证健康接口和页面资源，
截图编辑器并确认停止会话后没有残留 Python 服务进程。
