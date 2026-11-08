---
name: mods-category-tree-layout
description: 排查 Mod 管理左栏分类树行高不一致、展开空白和切换分类时的间距跳动。
---

- 当前生效的树模板与 scoped 样式在 `src/views/ModsManagement/ModsManagement.vue`；同目录独立 CSS 文件存在相似规则，修改前确认引用，避免只改未加载的副本。
- 分类按需加载，空分类也可能保留展开状态。检查 `.el-tree-node__children` 的纵向 margin，不要把展开状态当作存在可见子项的证明。
- 默认全部收起，启动不读取持久化展开记录，切换游戏清空 expandedKeys 和该游戏的展开记录。区分初始化与当前浏览过程中的状态保留；不要因结构更新重建树而丢失用户刚手动展开的分支。
- 跨游戏列表隔离必须同步清空旧数据并使旧任务失效，不能沿用同游戏 preserveVisible。全局流式事件必须携带并校验 scanId；只判断当前 selectedGame 无法阻止新监听器接收旧扫描事件。懒加载额外使用游戏切换代次，防止 A→B→A 的旧任务重新获得写入资格。
- 按需加载使用树的 `lazy` / `load` 和 `leaf` 标记。非懒加载模式会直接按 children 数量判断叶子，导致必须先选中并扫描才能展开。展开与选中分离；失败调用 reject 允许重试。不要手动展开虚拟根，避免重复加载根节点；恢复深层节点依赖默认展开键。
- 未加载不代表有子分类。箭头依据后端 `hasChildGroups`，不要用 modCount 或通用 has_subdirs 代替。覆盖磁盘、流式和 SQLite 缓存路径；缓存数据刷新后同步节点 isLeafByUser / updateLeafState，懒加载树不会自动重新读取 leaf 属性。Rust 回归：`cargo test --manifest-path src-tauri/Cargo.toml --lib group_dedupe_tests --offline`。
- 核对行高与自定义节点的完整盒模型，包含图片边框、padding 和 margin。行间距由外层行统一控制；内部节点不得溢出行框。
- 分类切换不应调用 `bumpGroupTree()`。Element Plus 即使设置 node-key，收到新根数组也会重建内部节点；使用 `ModsManagement.tree.ts` 原位同步根数组、children 数组及节点对象，保留展开与选中状态。数据同步后运行 `node --test scripts/tests/test-mod-snapshot.mjs`，其中直接使用实际安装的 TreeStore 验证状态保留。
- 不覆盖 Element Plus 控制的展开高度、overflow 和显示状态；悬停、选中避免过渡布局属性。
- 检查空分类、真实子分类、图标和无图标节点、键盘选择及连续切换；构建通过不代表实际布局已验证。

维护说明见 [界面布局维护记录](../../docs/ui-layout-maintenance.md)。
