# Example Plugin SDK

这是一个不影响正式安装目录的陌生插件示例：

- ID 不出现在 SSMT 源码中。
- 使用 `schemaVersion: 2` 和 `compatibility.pluginApi`。
- 同时提供声明式设置页和本地服务页。
- 通过宿主 ServiceLease、CapabilityBroker 和通用 UIHost 工作。
- 本地服务 iframe 通过版本化消息桥完成一次设置写入和读回，页面不自报插件 ID。

开发版测试：

```powershell
.\scripts\dev\build-plugin-sdk-fixture.ps1
```

启动开发版 SSMT4 后执行：

```powershell
node scripts/dev/verify-plugin-sdk-fixture.mjs
```

脚本会验证安装、Schema 2、能力目录、设置读写、声明式与本地服务页面、iframe 能力桥、
禁用插件拒绝、伪 Lease 拒绝、服务停止和卸载。
