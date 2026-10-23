# 启动浮窗与控制台

SSMT 启动队列中的 Console PE、`.cmd` 和 `.bat` 程序会在启动浮窗中显示标准输出、标准错误和退出码，支持按行发送文本。
点击开始游戏即展开浮窗，在等待事件监听与更新检查之前显示。可查看程序输出和启动事件、切换来源、复制回显或打开日志；切换页面不会丢失会话。
启动队列结束且注入器确认 `launch_complete` 后，控制台直接隐藏。只发现游戏进程不算注入完成，也不等待 Run.exe 成功后的退出倒计时或游戏日志跟随结束。失败保留展开详情与首次错误，取消或前置检查未通过不当成成功。
隐藏浮窗不结束进程，也不会因迟到的普通输出自动展开；当前启动的错误会展开仍可见的面板。
折叠面板在鼠标未悬停时半透明，悬停或键盘焦点进入时恢复清晰；展开详情保持清晰。无悬停能力的触屏保留清晰状态。
关闭按钮在启动流程、任意代次的控制台程序或只读日志跟随仍活动时禁用，悬停会解释原因；结束后可关闭。不能仅以所选来源或短命 Run.exe 的退出判断空闲。
成功启动后不保留阶段提示；失败的启动会持续保留面板。手动关闭只隐藏界面，保留有界历史与日志。迟到输出不重新打开，下一次启动重新展开面板。
GUI 程序沿用原来的 Shell 启动路径。没有可选插件时，同样使用原有游戏启动计划，不额外准备插件资源。

设置 → 通用中可录制“打开启动控制台”全局快捷键，默认 `Ctrl+Shift+L`，并提供禁用开关和直接打开按钮。没有历史或手动关闭过时也能唤起；空白控制台不会启动 Shell 或其他程序。
快捷键注册成功才保存设置；占用、无效组合或保留的 Alt+F 会在原位提示，原快捷键继续有效。打开控制台会恢复并聚焦 SSMT，和 Alt+F 的显示 / 最小化切换独立。两类注册命令都异步调度，避免插件内部锁与窗口线程调度发生互等。

## 启动期间的窗口响应

插件图形检查、组件准备、PluginHost 配置生成、Player Tweaks 资源校验与原神画质设置均由异步 Tauri 命令转入 `spawn_blocking`。同步命令会占住窗口线程；仅把函数声明成 async 仍会阻塞调度线程，不能直接执行大 DLL 的哈希、复制或等待外部程序。
UPX 等外部工具、GUI Shell 启动和进程枚举也移到 blocking pool，队列中的延时继续使用异步 sleep。只等进程名时不再逐个读取 EXE 路径。
控制台握手仍须在提交后确认，防止重复启动；它的等待与游戏生命周期分开，不能用取消握手或提前放行伪装界面响应正常。状态监听不可用或程序退出但没有完成事件时保留“启动状态未确认”与原因，不把缺少完成事件当成成功收起。

## Windows Terminal 的集成边界

`wt.exe` 是 Windows Terminal 的应用入口，官方命令行管理的是窗口、标签和面板，没有向 WebView 提供内嵌控件接口。[Windows Terminal 命令行文档](https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments)

真正复用 Windows Terminal 渲染控件，需要原生 TerminalControl / XAML 宿主和 ConPTY 连接，再处理焦点、DPI、输入、提权与部署。微软的 [UWP 控件 NuGet 打包 PR](https://github.com/microsoft/terminal/pull/20488) 在 2026-10-19 核查时仍为 Draft，说明当前支持范围是 C++ UWP 与 C++ UWP XAML Islands。由此判断，它不能直接作为当前 Vue / Tauri 控件替换。
当前实现继续提供标准流与日志浮窗，没有内嵌 WT；不能把启动 cmd / pwsh 或自行绘制终端界面称为嵌入 Windows Terminal。后续若采用 TerminalControl，应作为独立的可选原生宿主方案验证，不改动正常游戏启动与无插件路径。

## 通道与生命周期

`launch_programs` 在后台线程为控制台建立单次使用的本机回环连接，然后启动同一 SSMT 可执行文件的 `--ssmt-launch-console` 模式。
该模式在初始化 Tauri 与单实例插件之前分流，不创建第二个应用窗口。
需要管理员权限时通过 UAC 启动 helper，在同级权限内建立工具的 stdio 管道，避免依赖 ShellExecute 继承重定向句柄。
绑定仅使用 `127.0.0.1` 的随机端口，连接需验证本次随机凭据；不会保存可复用的提权请求文件。

Windows 上接受的连接会继承监听端非阻塞状态，必须显式调用 `set_nonblocking(false)` 后再进行带超时的握手读取，否则可能返回 Winsock 10035。
握手结束后，同时清除原始流与 `BufReader` 所持克隆句柄的读取超时；空闲控制台和等待输入不是断线。日志记录读取句柄的超时状态与底层通信错误。
命令已发送但启动确认丢失时，只提示控制台状态未确认，不能把已启动的游戏当成失败并回滚配置或再次启动。
确定的进程创建失败与用户取消 UAC 才按启动错误处理。

界面每个程序最多保留 128 Ki 个字符，后端每 100ms 合并回显，超量显示可省略，但日志记录限流前的数据。
通信报文和输入有大小上限。进程退出后最多排空两秒管道，避免后代继承 stdout 导致会话挂住。
关闭应用会断开输入并停止本体日志写入，helper 继续等待工具退出、排空输出，不主动结束工具或游戏。

### 游戏与 SSMT 的独立生命周期

游戏成功启动后，关闭 SSMT 应只结束界面、输入连接和本体日志跟随；游戏及其进程内 Runtime / 插件继续运行。需要常驻的辅助程序按游戏生命周期退出，图形文件恢复 helper 继续等待游戏结束再还原文件。启动尚未完成时仍可能依赖 SSMT 协调恢复屏障，不能把此阶段当成已经交接。

Windows 的父子进程关系本身不会让父进程退出时自动结束子进程，但子进程默认继承父进程的 Job。上级工具关闭带 `KILL_ON_JOB_CLOSE` 的 Job 时，会结束其中尚存的进程；`CREATE_NO_WINDOW` 和 `DETACHED_PROCESS` 都不能替代 Job 脱离。[Windows 进程退出说明](https://learn.microsoft.com/en-us/windows/win32/procthread/terminating-a-process)、[Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)

`bun tauri dev` 的 Cargo 启动链尤其需要检查这个边界；Cargo 在 Windows 上创建允许显式脱离的 Job，用于清理整棵启动树。[Cargo Job 实现](https://doc.rust-lang.org/nightly/nightly-rustc/src/cargo/util/job.rs.html)
控制台 helper 的 PowerShell 启动入口、GUI Shell 启动入口，以及图形恢复 helper 统一通过 `utils::independent_process` 创建，使用 `CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW`。在启动链最外层脱离后，Run.exe 与游戏仍沿用原有参数、工作目录、UAC 和注入顺序。仅移除本体的结束进程代码、或只验证 TCP 断开不杀工具，都不足以验证这个边界。

某些受管环境禁止脱离 Job。仅在进程创建返回访问被拒绝时按原方式重试，并写明无法脱离的诊断信息，避免阻断启动或重复创建已成功启动的目标；这种降级下仍可能受上级 Job 清理影响，不承诺完全独立。

这是标准流控制台，不能接管已独立运行的外部控制台，也不提供完整 ConPTY/VT 终端或 `ReadConsole` 等 Console API 交互。
输出按流处理 UTF-8；出现非法 UTF-8 时尝试 GBK，跨分片的多字节字符会保留。文本输入使用 UTF-8。
`Run.exe --machine-readable` 的 JSONL 会复用现有启动事件总线，普通回显仍保留在控制台与日志中。

## ReShade 日志

已启用的图形插件贡献实际采用的 ReShade 日志路径。普通启动没有这些来源；不会为关闭的插件扫描或创建日志。启动前建立文件基线，确认游戏 PID 后按只读来源跟随；Run.exe 退出不影响游戏日志跟随。

ReShade 是 DLL，没有可供接管的独立控制台。浮窗展示它实际写出的 `ReShade.log`，不向它发送标准输入。读取允许源文件重写、轮转和删除，保留跨分片中文，不重放上次运行的内容；输出也写入 SSMT 的独立控制台日志。游戏退出时有界排空，来源无法访问时仅停止该来源并保留具体错误，不阻止游戏运行。

## 回显日志

日志保存在当前缓存根目录的 `Logs/YYYYMMDDTHHMMSS.mmm-launch-console-<会话>.log`。
记录启动目标、工作目录、提权标志、PID、stdout/stderr、通信错误与退出码；不主动记录用户输入、参数或内部连接凭据。
工具自身回显的内容会如实记录。每份日志约 4 MiB 时轮转，仅保留当前文件与同名 `.previous.log`，写日志失败不会阻止启动。
浮窗的“打开日志”打开当前会话文件；清空浮窗不会清空日志。

## 验证

- `node --test scripts/tests/test-launch-monitor.mjs scripts/tests/test-launch-settings.mjs scripts/tests/test-efmi-launch.mjs scripts/tests/test-runtime-sync.mjs`：立即展开、事件与队列完成顺序、成功收起 / 失败保留、快捷键空白唤起、配置持久化，以及多会话、无控制台、无插件与组合启动。
- 在 `src-tauri` 执行 `cargo test --lib commands::launch_shortcut::tests` 与 `cargo test --lib commands::game_launcher::tests`：快捷键冲突保留与回滚、外部程序等待期间的调度响应。不注册真实系统快捷键，不启动游戏。
- 在 `src-tauri` 执行 `cargo test --lib commands::launch_console::tests -- --test-threads=1`：真实 Winsock 延迟读取、标准流子进程、无换行提示、中文输入、大段输出、退出码、断开不杀进程、日志轮转。
- `cargo test --lib utils::independent_process::tests -- --test-threads=1`：在隔离夹具内创建真实 `KILL_ON_JOB_CLOSE` Job，对照原路径被结束、新路径继续完成，以及禁止脱离时的启动降级。只关闭测试自建的 Job，不启动游戏。
- `cargo test --lib commands::launch_logs::tests -- --test-threads=1`：跳过旧日志、延迟创建、追加、重写、轮转、跨分片中文和有界大段输出。
- `bun run build` 与 `node scripts/check-render-perf.mjs`。

实机验收：启动游戏后查看 Run.exe 会话；确认 `launch_complete` 或运行时检测事件与实际游戏状态一致。
收起并重新展开浮窗，检查输出连续性；切换 ReShade 来源，确认没有输入框，Run.exe 退出后仍收到游戏日志，游戏退出后显示跟随结束。检查日志中的 PID、注入事件与退出码。
负载活动期间检查关闭按钮禁用及原因提示；所有来源退出后面板仍保留，可以手动关闭。折叠后移开鼠标，检查半透明及悬停 / 键盘聚焦恢复，窄窗口中关闭按钮和底部操作均应留在边界内。
提权确认、反作弊环境和游戏自身运行结果仍须单独实机确认，离线子进程测试不能替代这些证据。
成功进入游戏后正常关闭 SSMT，确认游戏及 Mod 保持运行；游戏退出后确认图形恢复完成。开发模式与正式版、Console 与 GUI 启动路径分别验收；不要用任务管理器的“结束进程树”代替正常关闭。
