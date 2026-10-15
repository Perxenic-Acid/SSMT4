> [!CAUTION]
> **⚠️ 安全性警告**
>
> SSMT4带有注入行为的Run.exe会100%触发杀软误报（虽然并未存在加壳行为，但是百分百会被误报，请自行注意评估使用风险，本人只能以微薄的开发者身份提供最基础的安全性承诺,由于其更新机制依赖于Github Release，唯一的安全性保证依赖于.sig签名文件，SSMT4-Alpha不会主动在 [Perxenic-Acid/SSMT4](https://github.com/Perxenic-Acid/SSMT4) 以外的地方发布，如果在其它平台看到了SSMT4-Alpha的二进制发布版本而不是直接提供源码或仓库地址，请注意辨别，并且请仔细将其与本仓库对应版本Release中.sig文件对比，以防下载到重打包的危险二次发布版本。


# SSMT4  ![GitHub Downloads (all assets, latest release)](https://img.shields.io/github/downloads/Perxenic-Acid/SSMT4/latest/total?color=blue&label=LatestDownloads)


- 配套Blender插件：https://github.com/Perxenic-Acid/TheHerta4
- 更多蓝图特性的Blender插件: https://github.com/xuhuan9102/TheHerta4
- SSMT4文档源代码：https://github.com/Perxenic-Acid/SSMT4-Documents
- SSMT4文档阅读地址：https://perxenic-acid.github.io/SSMT4-Documents/

## 本地 Native 插件开发

运行 `./build_native.ps1 -Configuration Release` 会构建 Native 工具和可选 Poser，
并把带清单的本地插件包放到 `src-tauri/resources/PluginPackages`。
可用 `-BuildPoserProbe $false` 跳过 Poser 构建。
插件市场刷新时自动发现这些包，标注“本地构建”；无需先发布到 GitHub Release。
点击“安装本地构建”检查来源和权限后安装，再按需启用。发现和安装不会自动启用插件。
本地构建包不视为已验证的 Native Release 来源；裸 DLL 和宿主测试 DLL 不直接成为市场条目。
前后端命令更新后，正在运行的 SSMT 开发进程需要重启才能使用新发现流程。

# 附加功能移除说明

SSMT4开源版仅包含了基础的启动器、管理器、模型提取功能，基础功能方便社区共建

其他附加功能为了满足开源社区的合规要求，已从SSMT4中删除，并放入MIMITools 咪咪工具箱中

https://github.com/StarBobis/MIMITools

没有这些附加的功能也不会影响你正常使用SSMT，请放心好了。

# LTS技术支持群(大本营)

大本营在discord，技术讨论可以去里面，为满足合规需求，暂时不开国内聊天群

Server Invite Link: https://discord.gg/aVKXuUrrM

# 特别感谢
- [Starbobis/Nicomico](https://github.com/Starbobis) 作为项目原拥有者对 SSMT 弥足轻重的贡献;
- [希尔·芙乐艾](https://github.com/xuhuan9102)对 SSMT 尤其是其 Blender 插件的大量贡献;
- [一夏抹忧](https://github.com/yixiamoyou)对 SSMT 尤其是其文档工程的大量贡献;
- 可能未提及的曾为 SSMT 项目贡献过代码的朋友;
- [H₄XeO₆](https://github.com/Perxenic-Acid) 也就是是我自己做出的一些贡献.
