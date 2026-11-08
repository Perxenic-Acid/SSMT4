# Launch Plan Conflict Fixture

这两个陌生插件声明同一个游戏目录目标 `ssmt-fixture-proxy.dll`，并声明同一个资源：

```text
example.codex.launch-alpha
example.codex.launch-beta
```

两者优先级相同，因此宿主应返回明确冲突，而不能按扫描顺序随机选择。

开发版测试：

```powershell
.\scripts\dev\build-plugin-launch-plan-fixture.ps1
node scripts/dev/verify-plugin-launch-plan-fixture.mjs
```

预览命令只校验计划，不写游戏目录。夹具不会启动游戏，也不会部署 DLL。
