# 终末地官方启动器启动与直接注入

## 已确认的代码差异

对照本地 XXMI-Launcher `1be4f9a`：

- `src/xxmi_launcher/core/packages/model_importers/efmi_package.py` 的 EFMI 默认 `xxmi_dll_inject_mode = InjectMode.DIRECT`，DX11 参数为 `-force-d3d11`。
- `core/game_launcher.py` 的手动模式不创建游戏进程，注入器直接等待游戏出现。
- `core/utils/dll_injector.py` 每 100 ms 查找进程，再调用 `3dmloader.dll` 的 `Inject`。现有 Runtime 的 `InjectorLib/Injector.cpp` 可用于核对该函数：写入完整 DLL 路径并通过远程 `LoadLibraryW` 加载。
- SSMT 原先的等待模式只安装 CBT Hook；该路径不执行直接注入，也不处理额外 DLL 和 PluginHost。
- SSMT 原先只根据 `useShell` 清除 INI 中的 `launch`，忽略 `launchTargetProgram = false`。用户选择手动启动后，仍可能留下直接创建游戏的配置。
- 原等待路径在模块枚举被拒绝时会返回成功，不能据此认定 SSMT Runtime 已加载。

“游戏提示需要官方启动器”和“由官方启动器启动后不能注入”是两个可分别验证的现象。现有材料没有证明新版游戏只检查父进程，因此采用官方启动器保留完整启动上下文，再注入已创建的游戏进程。

`launch_endfield.py` 不能作为父进程校验的充分实验：它没有转发所观察到的游戏参数，也没有完整声明 ctypes Win32 函数签名；x64 下默认整数返回值可能截断 HANDLE。即使修正这些问题，仍需实机才能判断是否还涉及环境、IPC 或其他启动上下文。

## 正式 EFMI 启动流程

该方案已于 2026-10-15 由用户反馈实机验证成立；2026-10-19 起默认启动链路对齐 XXMI 的 EFMI DIRECT：SSMT 直接启动 `Endfield.exe`，进程出现后再注入。使用包含此修改的客户端时，无需单独运行诊断脚本：

1. 将游戏预设设为 EFMI，目标程序选择实际 `Endfield.exe`，并安装好 EFMI 资源。
2. 默认“启动目标程序”开启且 Shell 关闭：点击 SSMT 启动，SSMT 会先启动 `Run.exe` 等待进程，再直接启动 `Endfield.exe`，并使用配置的启动参数（默认 `-force-d3d11`）。
3. 如需由官方启动器创建游戏：开启 Shell 并填写官方启动器路径，或关闭“启动目标程序”后手动启动；官方启动器中仍需启用 DirectX 11。

EFMI 的正常注入路径使用 `--inject-mode direct --wait-for-target --process-timeout 120`：

- 默认启动由 SSMT 创建 `Endfield.exe`；`Run.exe` 只等待进程并注入。
- 直接启动由管理员权限的 `Run.exe` 从官方启动器目录解析最新的 `Games.exe`，先启动隐藏的挂起父进程，再用 `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` 创建 `Endfield.exe`，以通过官方启动器的父进程校验。
- Shell 启动先启动注入器，再打开配置的官方启动器。用户仍需在官方启动器中开始游戏。
- 关闭自动启动目标程序时，只等待游戏，不启动官方启动器或游戏，也不执行配置的前后置程序。
- 默认直接启动会把配置的 `-force-d3d11` 传给游戏；Shell/手动启动时，须在官方启动器里启用 DirectX 11。
- 新等待模式会按游戏 EXE 的完整文件身份匹配进程，按顺序加载 Runtime、配置的额外 DLL 和已启用插件需要的 PluginHost。未启用插件时不要求 PluginHost 文件。
- 游戏进程由外部创建，注入失败不会结束游戏或尝试恢复其线程。
- `runtime_detected` 只在观察到指定 DLL 文件时发出；系统目录中的同名 `d3d11.dll` 不算成功。模块枚举不可用会明确失败，不冒充已验证加载。
- 直接注入的 JSONL 调用在完成后立即退出，不受 INI 的控制台退出倒计时影响，避免 `delay=-1` 使隐藏诊断进程等待按键。

其他预设保留原注入机制；手动模式清理残留 `launch` 的修正通用于所有预设。纯净模式保留原行为，不属于这条注入路径；若直接启动遭官方启动器拦截，请直接使用官方启动器。

## 独立诊断与旧客户端回归

测试包包含 `Run.exe`、`Test-EfmiInjection.ps1`、本文和哈希文件。它只包含原生注入器，不是 SSMT 安装包，也不包含 Runtime DLL、EFMI 资源包或 Mod。

只替换 `Run.exe` 无法让旧客户端自动选择直接注入；旧客户端请使用随包脚本。正式集成需同时包含新的前端逻辑和原生注入器，正常使用按上节操作。

1. 用官方启动器确认游戏不加 Mod 可以正常进入，在官方启动器中启用 DirectX 11，然后彻底退出游戏。
2. 在 SSMT 中确认终末地的目标路径指向实际 `Endfield.exe`，3Dmigoto 目录已经安装好 EFMI 资源。检查该目录 `d3dx.ini` 中 `[Loader] target` 是游戏 EXE 的完整路径，`module` 指向实际 SSMT Runtime DLL。`target` 不能填官方启动器路径。若残留 `loader = XXMI Launcher.exe`，应先让 SSMT 准备一次自己的配置，或备份后移除这项 XXMI 专用限制。
3. 关闭 SSMT/XXMI 已运行的注入器。将测试包解压到普通目录，用**管理员 PowerShell**进入该目录，执行：

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Test-EfmiInjection.ps1 -MigotoDirectory "D:\SSMTCache\3Dmigoto\EFMI"
   ```

   将路径替换为第 2 步的实际 3Dmigoto 目录。仓库中也可执行 `scripts/diagnostics/Test-EfmiInjection.ps1`，默认使用 `src-tauri/resources/Run.exe`。需要更长准备时间时增加 `-TimeoutSeconds 300`。

4. 看到等待提示后，在官方启动器点击开始游戏。不要同时点击 SSMT 或 XXMI 的开始按钮。
5. 记录游戏能否进入、可识别的 Mod 是否生效、Hunting 是否显示，以及有无启动器提示、崩溃或黑屏。脚本报告已加载 DLL 只能证明注入和模块观察成功，不能证明 Mod 渲染兼容。
6. 回传脚本打印的整个日志目录，以及游戏版本、区服、官方启动器版本、DX11 设置和实际画面结果。日志包含 `events.jsonl`、`injector.log`、`report.json`；报告记录注入器哈希、游戏 PID、路径、命令行及父进程信息。即使失败也保留报告。

脚本不覆盖运行时目录文件、不修改 INI、不伪造父进程。它直接使用指定目录的配置；原有 `inject_dlls` 仍会生效。首次定位可以在 SSMT 中关闭额外插件，再分别验证已启用的单个和多个插件组合。默认不传 PluginHost 配置；需要测试已有启用配置时，显式使用 `-PluginHostConfig`，并通过 `-InjectorPath` 选择同目录具有 `SSMT-PluginHost.dll` 的新注入器。

从测试包运行的 `Run.exe` 不在原 3Dmigoto 目录，因此不会因那个目录中残留的 `SSMT-Player-Tweaks.dll` 自动注入该文件；INI 中明确声明的额外 DLL 不受此影响。需要验证与新版应用相同的可选 Player Tweaks 行为时，应先由新版应用部署启用状态对应的文件，再显式指定该目录中的新 `Run.exe`。

## 按日志定位

| 观察 | 下一步 |
| --- | --- |
| 等待超时，未找到目标 | 核对完整 EXE 路径、官方启动器是否真正创建游戏、准备时间和权限；不要只添加游戏命令行 |
| `OpenProcess ... win32=5` | 核对脚本及游戏权限，回传错误与进程信息；该日志不代表已注入 |
| `Remote LoadLibraryW module unavailable` | 远端系统模块未就绪或枚举不可用，回传原生日志；不要通过“假定成功”隐藏问题 |
| `CreateRemoteThread` 或远程加载失败 | 回传 Win32 错误、线程退出码、DLL 路径及启动器/游戏版本 |
| `runtime_detected`、`launch_complete` 均出现，Mod 不生效 | 已观测到指定运行时 DLL；继续核对官方启动器 DX11 设置、EFMI 包/Runtime 版本和游戏日志，属于后续初始化或渲染兼容排查 |
| 使用官方启动器仍弹同一提示 | 单独确认无注入的官方启动能否进入；此现象不能仅由注入策略解释 |

## 本地验证与维护

前端验证：

```powershell
node --test scripts/tests/test-efmi-launch.mjs
bun run build
```

原生夹具测试必须使用独立、无需管理员权限的构建目录，避免改变正式注入器的 manifest：

```powershell
cmake -S native/3Dmigoto-Injector-V2 -B native/build-injector-tests -A x64 -DSSMT_INJECTOR_REQUIRE_ADMIN=OFF -DSSMT_INJECTOR_BUILD_TESTS=ON
cmake --build native/build-injector-tests --config Debug --parallel
ctest --test-dir native/build-injector-tests -C Debug --output-on-failure
```

测试实际创建自有目标进程并注入不导出 CBTProc 的夹具 DLL，覆盖 Runtime、多个额外 DLL、测试 PluginHost、同名不同路径进程、同名系统 DLL、中文和空格路径、超时、缺失 DLL、无效 DLL、参数错误。前端测试执行真实的启动队列和 INI 修改逻辑，覆盖 EFMI 手动/Shell、插件启停、其他预设和纯净启动。

正式原生目标可单独构建，不调用主仓库发布脚本：

```powershell
cmake --build native/build --config Release --target 3Dmigoto-Injector-V2 --parallel
```

**2026-10-15 验证记录：**本地夹具测试、诊断脚本成功/失败报告和前端构建通过；用户随后反馈独立测试包方案在实机验证成立，因此将相同方案接入正式 EFMI 启动流程。开发机器没有安装游戏，未独立复现实机结果；该反馈也不代表所有游戏版本、区服或实际插件组合均已验收。Native 源码属于独立子仓库，提交时需先提交原生修改，再由主仓库记录新的子模块指针。

Windows API 约束参考：[CreateRemoteThread](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createremotethread) 的权限、线程与 DLL 初始化串行化；[CreateToolhelp32Snapshot](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot) 的架构限制及 `ERROR_BAD_LENGTH`。模块枚举是有可能失败的独立观察，不能用进程存在或线程创建成功替代。
