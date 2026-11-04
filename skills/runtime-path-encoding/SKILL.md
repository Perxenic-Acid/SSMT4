---
name: runtime-path-encoding
description: 调查 SSMT Runtime 与 XXMI 的 Unicode 文件路径、INI 编码和资源加载问题。
---

# Runtime 路径与编码调查

先核实实际 Runtime 仓库和提交；目录联接可能被主仓库显示为未初始化 submodule，不要替换它。

- 分开检查文件路径、INI 内容编码、命名空间、资源路径拼接和最终文件打开接口。
- 当前普通 INI/DDS 使用宽字符路径，INI 内容使用 UTF-8 facet；不能只凭 CJK 文件名断言失败。
- 重点审计 `GetIniString` 的 string 重载、`GetUserConfigPath`、`glob_to_regex` 的有损转换，以及 MAX_PATH 缓冲区。
- 对照上游时记录双方提交，不把本地源码等同于用户实际部署的 DLL。
- 优先用临时 MSVC 夹具复用实际转换方式验证；明确文件打开、内容解码、DDS 解码及游戏验证各自覆盖范围。
- 现有调查见 [Runtime CJK 路径记录](../../docs/runtime-cjk-paths.md)。
