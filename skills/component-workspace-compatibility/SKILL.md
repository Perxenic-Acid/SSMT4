---
name: component-workspace-compatibility
description: 修改 SSMT Component 显示、别名或工作空间映射时，核查贴图标记、云端交换和 TheHerta 导入导出的兼容性。
---

- 先区分真实提取目录、`DrawIB-Component.json` 编号和显示名称。Component 别名保存在
  各 LOD 的 `ComponentAliases.json`，键是目录名；不要用可变编号或别名改写路径。
- 标记页选中 ID 是 `{tabId, subMeshName}` 的 JSON 字符串；预览、角色标记及记忆依赖
  该 ID。同步显隐单选时展开对应 LOD / IB，只滚动左栏，避免移动根页面或重建全部预览。
- 云端有完整包和仅配置两条路径。后者通过 Portable Metadata v1 的可选
  `lods[].componentAliases` 往返；检查 Rust 预检、上传序列化、Worker 归一化、实际及
  模板 schema、导入重建。服务端或公共库 schema 未部署不能当作已上线。
- 合并别名必须跟随资源来源选择，不能把冲突中被舍弃一方的别名合入结果。
- TheHerta 是 `BlenderPlugin` 链接到的独立仓库。`WorkSpaceModel` 优先读取持久化编号，
  显示别名不参与目录解析。同时检查旧长名、短名、LOD 前缀、旧别名和带点号的别名。
- 验证命令与具体协议见 [Component 兼容说明](../../docs/component-aliases.md)。
  浏览器界面测试可隔离 3D 预览并禁用 GPU；工作空间解析测试不能宣称已完成 Blender
  内实际导入、导出验收。
