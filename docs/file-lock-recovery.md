# Mod / DLL 占用处理

Mod 和分类目录启用、禁用时，后端暂时暂停本应用的 Mods 目录监听并重试重命名，随后恢复监听。只有 Windows 访问拒绝、共享冲突、锁冲突才进入占用诊断，目标已存在等错误不触发结束进程提示。

仍无法操作时，确认框展示占用进程的名称、PID、可执行文件路径及建议。取消不会结束进程；确认后重新检查占用及进程创建时间，仅结束确认列表中仍匹配的进程，再重试原来的改名。不会远程关闭句柄或强行卸载 DLL。系统关键进程、本应用及无法核实身份的进程不能自动结束。

主页设置菜单的“查询 / 解除 DLL 占用”分别查询当前游戏 3Dmigoto 目录中的 `d3dcompiler_47.dll`、`d3d11.dll`，展示各进程使用的具体路径。结束进程之后再次查询；若原操作是在 Windows 文件资源管理器中进行，需要返回资源管理器重试原操作。QQ、VS Code、资源管理器均按实际路径占用检测，不依赖进程名称白名单。确认结束 explorer.exe 后会尝试重新启动资源管理器。

文件及已加载 DLL 使用 Windows Restart Manager 检测；目录句柄使用 NtQueryInformationFile 检测。权限不足、目录查询失败、扫描上限或嵌套链接目录会显示扫描不完整提示。“未检测到”不代表一定可以写入；只读、权限及扫描后新出现的占用仍可能阻止操作。

API 参考：[RmRegisterResources](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmregisterresources)、[NtQueryInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile)。

## 验证

Windows 自动测试：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::file_locks::platform::tests -- --nocapture
```

测试创建独立子进程持有临时文件、目录句柄或加载临时 DLL，验证完整路径识别、同名 DLL 隔离、进程身份校验、本应用保护，以及释放后的目录改名。被忽略的 `lock_child` 是测试子进程入口，由这些测试自动启动。

界面验收：

- 无占用时切换 Mod / 分类启停，检查目录名、状态与监听刷新。
- 用测试程序持有 Mod 内文件，切换启停；取消确认，确认进程和目录未被改变；再次操作并确认，检查原切换完成。
- 对含多级禁用父目录的 Mod 启用，在后续确认中取消，检查已完成的父目录变更及标签路径同步。
- 查询 DLL 占用，核对文件路径、进程、处理建议；进程自行退出后确认，检查不会结束其他新进程。
- 手动检查 QQ、VS Code、Explorer 的实际占用场景；结束前保存工作，验证 Explorer 恢复和处理后的再次查询。
