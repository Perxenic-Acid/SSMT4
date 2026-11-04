# Runtime 的 CJK 路径与 INI 编码检查

检查日期：2026-10-26。SSMT Runtime 提交 `f39924d5`；本地 XXMI-Libs-Package 提交 `a7847eae`。以下是源码调查与局部夹具结果，不代表已验证游戏中部署的任意 DLL 版本。

## 常规读取链路

- 两者 `DirectX11/IniHandler.cpp` 的 `ParseNamespacedIniFile` 均用宽字符路径打开 INI，并通过 `codecvt_utf8<wchar_t, 0x10ffff, consume_header>` 解码内容。
- 递归扫描使用 Unicode 构建下的 `FindFirstFile` / `FindNextFile`；文件名和命名空间保留为 `wstring`。
- Resource 的 `filename` 取宽字符串，`CommandList.cpp` 将其直接交给 DirectXTK DDS 加载器；`DirectXTK/Src/LoaderHelpers.h` 最终用 `CreateFileW`（其他平台分支为 `CreateFile2`）打开文件。
- 因此没有发现“常见中日韩字符出现在普通 INI/DDS 文件名就必然失败”的依据。本地 XXMI 也已具备这些接口，不能将报告直接归因于缺少 Unicode 文件接口。

## 实际限制

- INI 内容要求 UTF-8，支持 BOM。GBK 等本地编码和 UTF-16 不在这条解码链的支持范围；非 UTF-8 注释同样经过解码器，可能干扰后续解析。文件名编码与文件内容编码应分别调查。
- SSMT 改进了 Resource 完整路径拼接与长路径前缀，上游该处仍用 `wchar_t path[MAX_PATH]` 拼接。但 SSMT 的 `filename` 配置值缓冲区仍为 MAX_PATH，INI 扫描也未统一处理长路径，不能宣称所有长路径都已支持。
- 两者 `GetUserConfigPath` 经 `GetIniString(..., string*)` 逐字符窄化，再直接拓宽；自定义 `user_config` 路径含 CJK 会损坏。默认 ASCII 配置名不触发此问题。
- 两者 `glob_to_regex` 直接将宽字符模式窄化，而匹配文件名使用 UTF-8；含 CJK 的 `exclude_recursive` 规则存在不匹配风险。这通常影响排除行为，不能直接等同于 DDS 打开失败。

## 局部验证与边界

临时 MSVC C++17 夹具复用 Runtime 的文件打开及 UTF-8 facet，在中文、日文、韩文混合目录内读取包含混合 CJK 资源名的配置：UTF-8 和 UTF-8 BOM 内容精确匹配；GBK 不匹配；UTF-16 未读出有效行。`CreateFileW` 成功打开同目录下混合 CJK 名称的测试文件。夹具未验证 DDS 内容解码、GPU 资源创建、完整 INI 解析或游戏注入，也未覆盖补充平面汉字。

排查实际报告时应取得原始 INI 字节、完整绝对路径、实际加载的 Runtime 版本及日志，区分递归扫描遗漏、内容解码错误、资源路径错误与 DDS 加载失败。本轮未修改 Runtime 源码或构建部署 DLL。
