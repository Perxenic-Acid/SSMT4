---
name: ssmt4-headless-workspace
description: 用已安装的 SSMT4 无界面命令从现有 FrameAnalysis 创建黑塔4工作空间，按 LOD 和哈希提取模型及骨骼源数据，并可标记贴图。适用于黑塔4导入导出测试的前置准备。
---

# SSMT4 无界面工作空间

有磁盘上的 FrameAnalysis 时，直接运行 `ssmt4-workspace.exe <request.json>`，无需启动桌面窗口。请求结构和示例见 [request.md](../../docs/headless-workspace.md)。从安装目录运行命令，或按文档从源码运行。已安装的命令从同级 resources 读取数据类型。

- 用独立目录准备测试工作空间。每个 LOD 显式设置自己的帧目录和 DrawIB 列表。按哈希提取时使用 `draw_ibs`；全量提取使用 `full_filter`。
- 可按哈希和槽位指定贴图标记。出现多个不同候选文件时，用提取输出的 `TrianglelistDedupedFileName.json` 锁定 `capture_name`。
- 根据退出码确认执行结果。检查各 LOD 的 `DrawIB-Component.json` 和 `Config/Tabs` 对应的帧路径。若使用含 ZZMI 骨骼源提取功能的 SSMT4 版本并且测试依赖骨骼融合，检查各 LOD 的 `ZZMISkeletonSource`。若指定了贴图标记，检查子网格 JSON 和贴图文件。
- 已离线验证两个 LOD 分别使用不同已有抓帧，按指定哈希提取，并按哈希和槽位标记 DDS。没有可用抓帧时，游戏内抓帧由用户完成。
