# Runtime 上游同步

`runtime` 是独立仓库，`origin` 为 SSMT Runtime，`upstream` 为 XXMI-Libs-Package。当前工作区通过目录联接访问它；主仓库的 `git submodule status` 可能显示未初始化，但 `git -C runtime rev-parse --show-toplevel` 能找到实际仓库时，应在现有仓库同步，不要为消除这个标记重新检出或替换目录联接。

## 同步与检查

先分别检查主仓库和 Runtime 的工作区，再获取两个远端。上游更新应合并 `upstream/master`，不是把 SSMT 分支替换成上游：

```powershell
git -C runtime fetch --multiple origin upstream
git -C runtime log --oneline HEAD..upstream/master
git -C runtime diff --stat HEAD...upstream/master
git -C runtime merge --no-commit --no-ff upstream/master
```

合并后对照 `git -C runtime diff upstream/master -- DirectX11`，确认 SSMT 的增量仍在：

- `SSMTBridge` 的设备就绪与 Present 通知，以及项目文件中的编译入口。
- DXGI 设备解包、交换链代理及 `skip_swapchain_wrap` 配置。
- 自定义资源的长路径处理，以及无交换链时绘制 Overlay 的保护。
- 构建后不再调用上游的 `CopyToGames.bat`，避免构建影响其他游戏目录。

无文本冲突不代表无需编译。MSVC 未指定 `/utf-8` 时，会按系统代码页解释无 BOM 文件；中文注释也可能导致解析错误。`HackerDXGI.cpp` 已添加 UTF-8 BOM，后续修改应保留，避免仅改注释却使编译失败。不要为修正单个文件而统一重写上游文件编码。

## 验证与应用资源

只构建 Runtime，并将产物部署到明确指定的目录。不要调用会启动游戏的 `runtime/debug.ps1` 来完成普通编译验证。主仓库已有暂存入口：

```powershell
./scripts/stage-ssmt-runtime.ps1 -Build
```

该入口使用 `runtime/release.ps1` 构建 x64 Release，将 DLL 放入应用资源目录；它不会调用主应用的发布包构建脚本。若只做隔离验证，可创建忽略的 `runtime/build/upstream-validation` 目录，再将其作为 `runtime/release.ps1 -TestRuntimeDir` 的参数。

DXGI 共存回归使用 [Runtime 测试说明](../runtime/tests/dxgi-interop/README.md)。必须先将本次构建的 DLL 放到测试目录，再依次运行 `plain`、`mod`、`reshade` 和 `both`；每项都使用独立进程。此测试创建桌面复制接口但不取得桌面帧，不能替代真实游戏、Mod 或实际插件组合的验收。

验证通过后完成 Runtime 合并提交，再在主仓库记录 `runtime` 的新指针。推送须按用户授权单独处理；需要推送时，先推送 Runtime 提交，避免主仓库指向远端不存在的对象。

## 2026-10-15 同步记录

- 从 SSMT `1c4de869` 合并上游 `a7847eae`（`v1.2.2`），包含 54 个尚未合入的上游提交。
- 上游主要增加槽位/资源池范围及资源数据缓存，修正空间哈希偏移、常量缓冲区步长、资源视图复用与着色器错误对象释放。
- 合并的是上游最新状态，保留了 `v1.2.2` 对条件批处理的回退。
- 无文本冲突；额外修正 `HackerDXGI.cpp` 的 UTF-8 标记，并清除上游新增注释的一处行尾空白。
- x64 Release 构建通过；四种 DXGI 测试均返回 `DuplicateOutput=00000000`、退出码 0。未进行真实游戏验证。
