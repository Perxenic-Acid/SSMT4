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
