# Blender 导入未识别面部和脖颈

Blender 的 `ui/ui_func_import_ssmt.py::_read_submesh_role` 只读取实际导入的
`TYPE_*/*.json` 中的 `SubMeshRole`，有效值是 `Face`、`Neck`、`Eye`。
物体别名和 FaceSDFMap 贴图标记不代替这个字段。

后处理页在 `WorkSpace/<游戏>/<工作空间>/MarkTextureConfig.json` 保存选择记忆；
旧版 `WorkSpace/<游戏>/MarkTextureConfig.json` 仅在没有当前工作空间记忆时作为迁移来源。
其中选择值包含 tabId 和 subMeshName，排查时应先通过工作空间
`Config/WorkPageTabs.json` 确认 tabId 所属页，再核对 `Import.json` 实际选用类型。
不要把同游戏其它工作空间的选择记忆直接迁移过来。

旧实现加载选择记忆仅恢复 UI 状态，导致 UI 标记与导入数据脱节。修复后，加载和重新激活
后处理页都会核对当前工作空间的全部类型目录：模型 JSON 中有效角色优先，旧记忆仅补齐
当前页签中实际存在且缺少角色的模型。成功写入后才恢复 UI 标记，冲突或失败会明确报错。
不能仅凭字段缺失断言历史上何时、为何丢失。

清除角色写入 `SubMeshRole: null`，阻止旧记忆将角色重新补回。Rust 可将 null 读取为
未标记，Blender 同样将其视为未标记。重新提取保留同一模型 JSON 的已有角色及明确清除记录。
移动脖颈标记时，新旧模型的全部类型目录一并更新；文件写入失败会尝试恢复原内容，UI 不提前更新。

2026-10-24 只读检查 `Водяница`：最近导入选择的以下文件均存在，但缺少角色字段。

| SubMesh | 页面记忆角色 |
| --- | --- |
| `405a3a71-10092-0` | Face |
| `5f601413-120-0` | Face |
| `66ec3e27-3894-0` | Face |
| `d5ae71a3-23994-46005` | Neck |
| `848c5980-1176-0` | Eye |

该工作空间扫描到的 38 个 JSON 均无 `SubMeshRole`。这足以解释角色识别和面颈矫正失败，
不证明几何数据本身缺失。使用修复后的开发版本重新打开该工作空间的后处理页即可自动补齐，
再导入 Blender。本次开发应用热更新后已观察到工作空间级记忆与五个角色 JSON 自动补齐，
只读核对确认角色集合一致；没有直接编辑缓存文件，尚未执行新的 Blender 导入。

验证命令：

```powershell
node --test scripts/tests/test-mark-texture-selection.mjs
bunx vue-tsc --noEmit
cargo test --manifest-path src-tauri/Cargo.toml --lib workspace::submesh_json::role_tests
```
