# Component 别名与工作空间兼容

贴图标记左栏按 LOD → IB → Component 展示。LOD 和 IB 可折叠；每个 Component
行右侧的铅笔按钮可修改别名，留空恢复默认名称。预览左侧开启同步显隐后，M/S
变成单选，选择会展开并立即定位左栏对应项。关闭同步后恢复此前的 M/S 状态。

## 存储与稳定标识

每个 LOD 保存可选文件 `ComponentAliases.json`：

```json
{
  "848c5980-16884-0": "面部.高光",
  "848c5980-25830-16884": "衣服"
}
```

键是实际提取目录名。它独立于 `DrawIB-Component.json` 内的显示编号，不因新增
Component、重新生成映射或合并工作空间而错配。值是显示文本，最多 128 个 Unicode
字符，不允许控制字符。每个 LOD 最多保存 4096 项，留空的条目不持久化。

别名不改变提取目录、Buffer 文件、DrawCall 映射、选中项 ID、贴图标记或 Mod 引用。
旧工作空间没有此文件时，SSMT 使用默认 Component 名称，Blender 继续使用原有 IB
别名。SSMT 也支持没有 WorkPageTabs 索引的 LOD 目录及直接保存于根目录的旧结构。
文件损坏时，标记页提示并显示默认名称，保留原文件；改名不会用空映射覆盖损坏文件。

## 云端与合并

- 完整 `.ssmtws` 包包含 LOD 中的别名文件，受现有大小和 SHA-256 校验保护。
- Portable Metadata v1 的每个 LOD 增加可选 `componentAliases` 字典，格式与文件相同。
  未提供时默认空字典，不提升 schemaVersion。上传预检、服务端归一化、JSON Schema、
  完整导入和“仅配置”导入均处理此字段，旧发布仍能导入。
- 合并时别名跟随实际选中的资源来源。同一 IB 冲突选择第二个工作空间时，不能保留
  第一个来源的别名；被选来源没有别名时，恢复默认名称。不同来源的独有项合并保留。

上线时先将 `services/workspace-access-api/schema/v1/metadata.schema.json` 同步到公共
元信息库的同名位置，再部署更新后的 Workspace API，最后发布客户端。模板 schema
也已更新。旧服务端会剔除不认识的可选字段，因此只发布新客户端不能让“仅配置”
下载保留别名。无需迁移 D1 或修改已有公共工作空间条目；本次未部署线上服务。

## Blender

配套 TheHerta 提交：`f2d8276`（独立仓库，需同步更新插件）。

TheHerta 的 `WorkSpaceModel` 按 LOD 读取别名，优先沿用 SSMT 已持久化的
`DrawIB-Component.json` 编号；缺失或无效条目按原有目录扫描方式补齐。新导入物体
和刷新后的蓝图列表显示 `LOD.DrawIB-编号.别名`，Component 别名优先于 IB 别名。

解析仍根据 LOD、IB 和编号定位目录，忽略别名后缀。旧长目录名、没有后缀的短名、
旧别名及带点号的新别名均可识别，不主动重命名现有 `.blend` 中的对象或节点。
蓝图明确选择的 LOD Submesh 优先于物体显示名。

## 离线验证

```powershell
node --test scripts/tests/test-component-aliases.mjs scripts/tests/test-mark-texture-selection.mjs
cargo test --manifest-path src-tauri/Cargo.toml --lib workspace_ -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --lib component_aliases
node --test services/workspace-access-api/test/*.test.js
python BlenderPlugin/tools/test_component_aliases.py
bun run build
```

另用禁用 GPU 的浏览器验证真实 Vue 页面上的折叠、单选定位、改名、M/S 恢复与
小窗口布局。Blender 的测试隔离注册和绘图依赖，验证实际工作空间解析代码；
不替代 Blender 内的模型导入及导出操作验收。
