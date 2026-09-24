# Mod / DLL 占用处理

Mod 和分类目录启用、禁用时，后端暂时暂停本应用的 Mods 目录监听并重试重命名，随后恢复监听。只有 Windows 访问拒绝、共享冲突、锁冲突才进入占用诊断，目标已存在等错误不触发结束进程提示。

仍无法操作时，确认框展示占用进程的名称、PID、可执行文件路径及建议。取消不会结束进程；确认后重新检查占用及进程创建时间，仅结束确认列表中仍匹配的进程，再重试原来的改名。不会远程关闭句柄或强行卸载 DLL。系统关键进程、本应用及无法核实身份的进程不能自动结束。

## 终端目录改名

PowerShell、cmd、Windows Terminal 等终端占用待改名目录时，优先提示先切换到稳定的 Mods 根目录，或关闭相关标签页，然后重试原操作。PowerShell 提示同时设置 `Set-Location` 和 `[Environment]::CurrentDirectory`：PowerShell 的工作位置与进程工作目录不是同一个概念，只运行 `Set-Location` 不一定释放原目录。手动释放后直接重试，不结束进程。× / Esc 取消操作；“仍要结束进程…”会再次尝试改名并刷新占用列表，再单独请求结束确认。

启用/禁用会改变目录名，旧 Windows Terminal 标签页保存的启动路径不会随之更新。如果用户仍确认结束 shell，确认框会显示原目录和改名后的目录，完成后提醒关闭退出的旧标签页，从新路径重新打开；不能在旧标签页一直按 Enter 重启。父目录改名时，需要保留原来工作位置的子目录层级。

已经出现“无法访问启动目录”的终端，请关闭该标签页，从文件资源管理器进入改名后的实际目录，再打开新终端。不要为了兼容旧终端创建原目录的链接，以免 Mod 再次被扫描。

参考：[Windows Terminal 启动目录](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-general#starting-directory)、[PowerShell Set-Location](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/set-location)。

## DLL 范围

主页设置菜单的“查询 / 解除 DLL 占用”分别查询当前游戏 3Dmigoto 目录中的 `d3dcompiler_47.dll`、`d3d11.dll`，展示各进程使用的具体路径。结束进程之后再次查询；若原操作是在 Windows 文件资源管理器中进行，需要返回资源管理器重试原操作。只有实际使用所选游戏目录中具体文件的进程才会列入结果；加载其他目录中同名 DLL 的进程不会被选中。结束前重新查询这些具体路径并核对 PID 与进程创建时间；已释放或身份变化的进程不会结束。QQ、VS Code、资源管理器的进程名仅用于处理建议，不作为占用或结束依据。确认结束 explorer.exe 后会尝试重新启动资源管理器。

文件及已加载 DLL 使用 Windows Restart Manager 检测；目录句柄使用 NtQueryInformationFile 检测。权限不足、目录查询失败、扫描上限或嵌套链接目录会显示扫描不完整提示。“未检测到”不代表一定可以写入；只读、权限及扫描后新出现的占用仍可能阻止操作。

API 参考：[RmRegisterResources](https://learn.microsoft.com/en-us/windows/win32/api/restartmanager/nf-restartmanager-rmregisterresources)、[NtQueryInformationFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile)。

## 验证

前端恢复流程测试（手动释放、取消、二次确认、新占用列表与 DLL 完整路径）：

```powershell
node --test scripts/test-file-lock-recovery.mjs
```

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

- PowerShell 停留在 Mod 子目录时切换启停：先按提示切换目录并重试，确认 shell 保持运行；取消时目录不改名；选择结束时必须再次确认，并按完成提示从改名后的目录打开新终端。
- 在另一个目录加载同名 `d3d11.dll`，查询当前游戏 DLL 时不得命中该进程（自动测试 `finds_loaded_dll_by_full_path` 已覆盖）。
