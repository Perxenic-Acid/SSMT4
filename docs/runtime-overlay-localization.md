# 3DMigoto 游戏内提示汉化

左上角 Runtime 提示来自 `runtime/DirectX11/Overlay.cpp`，不是
`pcre2-10.30/src/pcre2_error.c`。后者是正则表达式库的诊断文本，不修改该第三方库。

## 选择语言

在 SSMT 的“设置 → 通用 → 3DMigoto 游戏内提示语言”选择自动、简体中文或 English。
设置全局保存，与 SSMT 界面语言和模型提取日志语言独立。下次普通启动会写入当前游戏的 DX11 Runtime INI，包含热注入路线；纯净启动不改 INI，DX12 不写入未支持的语言选项。

Runtime 的 `d3dx.ini` 支持：

```ini
[Hunting]
overlay_language = auto
```

- `auto`：简体中文 Windows（中国大陆、新加坡）使用中文，其他系统使用英文。
- `zh_s`：使用简体中文。也接受 `zh-CN`、`zh-Hans`。
- `en`：使用英文。也接受 `en-US`。

旧 INI 不添加该项也默认使用 `auto`。手动修改 INI 后通过现有的配置重载键生效，
不改变 Hunting 是否启用、警告开关或游戏功能。无效语言值写日志并按 `auto` 处理。

当前覆盖 113 条提示格式和 15 类表达式错误原因：配置重载、用户设置清理、帧分析、
ShaderFixes 重载、文件及贴图加载失败、重复配置节 / 键 / 包含、哈希解析、
枚举、条件、资源格式及表达式错误等，并翻译 Hunting 模式标签。
未收录的 Runtime 文本、编译器原始诊断、Mod 自带文本及高级性能统计保留原文。
配置键、枚举技术值、着色器类型、哈希和文件名不翻译。

2026-10-19 的重载红字反馈确认了覆盖遗漏：本机日志中反复出现的
`Duplicate section found` 未收录，故在中文模式下仍显示英文。
现已对照 `IniHandler.cpp` 的全部 79 条静态叠加层格式和 `Override.cpp` 的 5 条
格式补齐目录，并将检查加入回归。表达式错误外层与解析器已知原因一并翻译，
原因后的变量、表达式和 `^` 指示位置参数保留；未识别的详细诊断仍显示原文。
翻译在提示入队时完成，告警级别、开关与英文日志保持原有行为。

## 系统字体与性能边界

中文通过 GDI 读取系统 **Microsoft YaHei（微软雅黑）**，验证实际选中的字体及中文字形后，
生成 GPU 字形图集，交给现有 DirectXTK SpriteFont / SpriteBatch 绘制。
不再使用原有 ASCII 图集显示中文，也不随包分发 Windows 字体文件。

正常帧只检查语言和提示代次。仅第一次显示中文，或新提示引入了未缓存的字形时，
才在 SpriteBatch 开始前更新图集；不在逐帧路径扫描系统字体、调用 GDI 或创建字库。
图集创建使用 Runtime 的原始设备入口，沿用叠加层已有的状态保存 / 恢复链。

字形缓存每个叠加层最多 4096 个 BMP 字符，纹理边长不超过 2048。
缺少微软雅黑、字库超限或创建失败时，该叠加层本次生命周期回退英文字体及英文提示，
日志记录一次原因。未包含的字符使用问号；不将 UTF-16 代理项当作独立字形。

提示在入队时分别保存英文和中文显示文本，字体失败或语言切换仍能恢复英文。
日志继续写英文原格式串。窄字符格式串保留原来的 `%s` / `%S` 语义，
使用独立 UTF-8 CRT locale 和明确的 UTF-8 解码，不改变整个游戏进程的 locale。

## 离线验证及实机验收

从 SSMT 根目录执行以下命令，仅构建 Runtime 组件并部署到独立临时目录，不启动游戏：

```powershell
New-Item -ItemType Directory -Force tmp/overlay-localization | Out-Null
./runtime/release.ps1 -TestRuntimeDir "$PWD/tmp/overlay-localization"
cmake -S runtime/tests/overlay-localization -B tmp/overlay-localization/build -A x64
cmake --build tmp/overlay-localization/build --config Release
ctest --test-dir tmp/overlay-localization/build -C Release --output-on-failure
```

测试说明见 [Runtime 回归入口](../runtime/tests/overlay-localization/README.md)。
2026-10-19 已通过 x64 Release 编译、四项本地化回归及原有二十五项 Context / 帧分析 / 贴图 WARP 回归。
软件测试已验证真实中文字形绘制、格式参数、Unicode 路径、字体失败、渲染状态恢复，
以及 32 次 Present 仅创建一次中文字库；不代表实机叠加层共存或游戏帧率已验收。
重载红字增补的 Runtime 版本为 `1.2.2.26`，另已通过同组四项回归，
其中格式契约包含三项 Python 检查；
旧翻译实现会在“重复配置节有中文”断言处失败，新实现验证了橙红色中文像素。

交付实机后检查：

1. 正常进入游戏，按现有 F10 重载键，确认中文提示清晰、无方框，路径及数字正确。
   已开启加载错误提示且配置存在问题时，核对红色 / 橙红色告警也显示中文，配置节名及
   文件路径保留；英文日志仍应能定位同一条错误。
2. 原本已启用 Hunting 时检查模式标签；按现有 F8 帧分析键，确认保存提示与文件一致。
3. 改为 `en` 再重载，确认提示恢复英文；再切回 `zh_s`，确认仍正常。
4. 检查头像 Mod、两条 DLSS5 路线与 ReShade 共存画面及帧率；字体变更不能代替此前尚待完成的渲染问题验收。

debug 脚本已显示中文、SSMT 仍旧时，检查实际安装目录的资源，而非只检查源码中的
`src-tauri/resources`。来源选择、GitHub 静默更新和按游戏锁定见 [运行时更新说明](runtime-updates.md)。
