# Plugin API 1 冻结契约

Plugin API 1 从 P5 起进入维护期。宿主可以增加向后兼容的能力，但插件和宿主都不得依赖
内部 Tauri Command、Vue 组件、文件布局或插件 ID 特判。

机器可读快照位于 [plugin-api-v1-contract.json](plugin-api-v1-contract.json)，
Rust 能力目录和前端桥版本必须与快照一致。

## 版本边界

| 契约 | 当前版本 | 演进规则 |
|---|---:|---|
| Plugin API | `1.0.0` | `1.x` 只增不改；删除、重命名或改变语义必须发布 `2.0.0` |
| Capability API | `1` | 能力 ID、权限和响应结构以快照为准 |
| Manifest Schema | `2` | 与 Plugin API 分开演进；未知字段仍拒绝 |
| 页面桥协议 | `1` | 消息名、字段和来源校验改变时必须提升协议版本 |

增加新能力、新页面字段或新的可选响应字段属于兼容扩展。修改已有能力的输入含义、
权限门槛、成功/失败语义，或删除页面类型，不属于兼容扩展。

新增能力时提升 Plugin API 次版本（例如 `1.1.0`），同步更新冻结快照和能力目录；
已有声明 `>=1.0.0 <2.0.0` 的插件继续可用。插件若必须依赖新能力，可声明
`>=1.1.0 <2.0.0`，旧宿主会把它显示为不兼容。

## 升级原则

### SSMT 升级

- 不得要求插件重新编译。
- 不得把内部 Tauri Command 名暴露为公开契约。
- 新宿主必须继续读取 Schema 1 和历史 `pluginApiVersion: 1` 包。
- 声明未来 Plugin API 的包仍可安装并显示，但状态必须是 `incompatible`，且旧启用标记
  不能让它运行。
- 不兼容包保持可见、不可启用；旧的已启用状态仍可关闭。
- 页面、服务和启动贡献失败不得阻断普通 Mod 管理、游戏启动或无插件页面。

### 插件升级

- 不得要求宿主增加固定插件 ID 分支。
- 页面、服务、脚本和资源必须装在 `.ssmtpkg` 内。
- 同一 `pluginId@version` 不得重新打包不同实现；变化必须提升插件版本。
- 页面升级后会产生新的 ServiceInstance 键，旧 Lease 和旧服务会话必须回收。
- 插件只能通过 CapabilityBroker 获取宿主数据与操作，不直接访问 Tauri API。

## 当前官方兼容矩阵

| 插件 | 最新包 | Manifest | Plugin API | 页面 |
|---|---:|---:|---|---|
| DLSS5 Integration | `0.3.0` | 2 | `>=1.0.0 <2.0.0` | `static`；仍为不可发布草稿 |
| HoYoShade Bridge | `0.2.0` | 2 | `>=1.0.0 <2.0.0` | 旧设置组件兼容入口 |
| Liquid Gravity Probe | `0.2.33` | 2 | `>=1.0.0 <2.0.0` | `local-service` |
| Player Tweaks | `0.2.0` | 2 | `>=1.0.0 <2.0.0` | `static` |
| Poser | `0.1.1` | 1 | legacy `pluginApiVersion: 1` | 无通用页面 |
| Secondary Motion | `3.3.1` | 2 | `>=1.0.0 <2.0.0` | `static` |

DLSS5 后续迁移已跳过，因此其 0.3.0 仅作为开发快照保留 `publish: false`，不纳入正式
发布矩阵。

## 页面桥

请求固定为：

```js
window.parent.postMessage({
  type: 'ssmt:capability-request',
  protocolVersion: 1,
  requestId: 'request-1',
  capability: 'plugin.settings.get',
  payload: { key: 'mode' },
}, '*')
```

响应固定包含 `type`、`protocolVersion`、`requestId`、`capability` 和 `ok`。
成功时携带 `value`，失败时携带稳定 `errorCode` 与面向用户的 `errorMessage`。

宿主持续校验：

- 回复目标必须是当前 iframe；
- `event.source` 必须是当前 `contentWindow`；
- Origin 必须与服务或静态资源 URL 一致；
- `requestId` 长度、能力 ID 和 payload 对象类型合法；
- Lease 仍有效，插件、版本、页面和游戏从 Lease 解析。

## 自动回归

```powershell
bun run check:plugin-api-contract
cargo test --manifest-path src-tauri/Cargo.toml plugin::capability_broker
node --test scripts/tests/test-plugin-graphics-settings.mjs scripts/tests/test-dlss5-integration-ui.mjs
```

`check:plugin-api-contract` 会检查：

- 冻结能力 ID、权限顺序、错误码、页面类型和桥协议；
- 全部 19 个官方包版本都接受当前 Plugin API；
- 各插件最新版本、Schema、静态/本地服务页面和构建产物完整；
- 插件 UI 不导入任意 Tauri API；
- 页面宿主和所有现有插件桥使用协议版本 `1`。

需要运行真实页面、服务进程、安装/启用/卸载和伪 Lease 拒绝时，再启动开发版 SSMT，
执行 `verify-plugin-sdk-fixture.mjs` 与 `verify-plugin-page-runtime.mjs`。这些浏览器
回归不等于游戏内 DLL、渲染效果或反作弊兼容验收。

## 发布前检查

1. 版本变化必须有新插件目录；不得重打同版本。
2. `check:plugin-api-contract`、UI contracts、插件 Rust 回归和 `bun run build` 全部通过。
3. 检查 Native 打包输出中的清单、文件大小、SHA-256、许可证和 Release 目录一致。
4. 验证空插件、单插件、多插件、未来 Plugin API、缺文件和不兼容 SSMT 版本。
5. 确认不兼容插件可关闭，普通游戏启动和主页面不读取插件专有资源。
6. 个人测试分支完成验证后再询问是否推送；未得到明确同意不得推送主仓库或创建 PR。
