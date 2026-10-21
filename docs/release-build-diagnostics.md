# Release 构建排查

先区分主应用本机打包与 Native / Runtime 的 GitHub Actions。云端组件构建成功不代表主应用配置能够通过解析；不要为排查重新运行主应用发布包脚本。

## 版本配置解析失败

`tauri.conf.json > version must be a semver string` 表示应用版本格式不合法，不是 C++ 或 Rust 编译失败。主应用发布脚本使用三段正式版本 `x.y.z`，例如 `4.2.1`；`4.2.0.001` 这样的四段版本及带前导零的版本分量不合法。Runtime DLL 的四段 Windows 文件版本与这里的应用版本用途不同。

发布脚本在读取签名材料、编译组件和暂存资源前检查版本。显式 `-Version` 也会替换原来的非法版本，而不是仅匹配已经合法的旧值。不要擅自修改玩家或维护者手动选择的发布版本。

只读验证 Tauri 配置，无需编译或签名：

```powershell
node .\node_modules\@tauri-apps\cli\tauri.js inspect wix-upgrade-code
```

该命令只读取配置并计算 MSI 标识，也可用于当前使用 NSIS 的项目。若报版本解析错误，先修复版本再重试；若输出标识，则已通过这一步配置解析，不代表整个发布构建通过。

## 开发启动缺少二进制资源

`resource path resources/d3d11.dll doesn't exist` 表示 Tauri 的资源声明指向了不存在的文件，
不是日志前面的 `CXX = None` 或 DirectXTex 链接设置出错。`d3d11.dll` 和 `Run.exe` 是被
Git 忽略的构建产物，新检出、资源清理或发布暂存失败后可能缺失，不能靠拉取源码恢复。
发布脚本在组件构建前清理这两项暂存文件，构建失败时也会清理；因此一次失败的发布
准备可能导致之后的开发启动缺少资源，即使组件目录中仍有可用产物。

`bun tauri dev` 现在会在启动 Vite / Cargo 前补齐缺失的核心资源：

- `d3d11.dll` 来自 `runtime/x64/Release/d3d11.dll`。
- `Run.exe` 和缺失的 `SSMT-PluginHost.dll` 来自 `native/dist/Release`，没有时用 `Debug`。
- 已存在的文件保持原样，保留开发者手工选择的 Runtime；可选插件不会被暂存或安装。

没有本地构建产物时一次性报告缺项和构建命令，不联网下载，也不自动触发完整构建。
首次配置开发环境可分别运行组件脚本，之后重试 `bun tauri dev`：

```powershell
.\build_native.ps1 -Configuration Release
.\scripts\stage-ssmt-runtime.ps1 -Build
```

直接调用 `bunx tauri dev` 或 `cargo check` 会绕过该包装脚本，需要先自行准备上述资源。
只有已经编译好的 Runtime 缺少暂存时，运行 `.\scripts\stage-ssmt-runtime.ps1` 即可。
本地资源与实际安装版、游戏目录中的 Runtime 并非同一份，见 [Runtime 来源说明](runtime-updates.md)。

验证资源准备逻辑时执行 `node --test scripts/tests/test-dev-resources.mjs`；验证实际构建可用
`cargo check --manifest-path src-tauri/Cargo.toml`。不要为此运行主应用发布包脚本。
