# 本体侧插件贡献契约

此契约扩展 JSON 插件清单，不修改 Native Plugin ABI。字段默认缺省，旧包继续通过兼容适配运行。

## 声明

`contributions.hostExtensions` 声明本体侧已注册的 provider：

```json
{
  "id": "graphics",
  "provider": "ssmt.graphics.dlss5",
  "priority": 100,
  "config": "host/graphics.json",
  "settingsComponent": "ssmt.dlss5.settings"
}
```

`id` 在插件内唯一。`config` 是包内相对路径，安装时检查文件存在；禁止绝对路径、上级目录和 Windows 路径别名。配置为数据，不作为 JS、DLL 或命令执行。未知 provider 可以随包安装，执行前需要本体已有对应实现；不能因名字相似自动执行代码。

`priority` 用于提供同类 Host 的候选排序；共享效果应作为贡献合并，不能通过选中一个 Host 丢失其余插件的效果。文件、环境和配置合并仍须检查所有权与冲突。`settingsComponent` 只匹配可信 UI 注册表中的组件键，不把清单路径当成可执行前端模块。

依赖可声明 `requiredFor`：

```json
{
  "id": "manager",
  "type": "directory",
  "requiredFiles": ["Manager.exe"],
  "requiredFor": ["external-manager"]
}
```

缺省时保持旧依赖必需语义；`["launch"]` 表示普通启动必需；仅声明其他功能时不因未配置工具目录阻断普通启动。空数组表示按手动使用操作检查。显式调用管理器仍验证它实际需要的目录和文件。旧 DLSS5 包的版本兼容逻辑集中在清单适配入口，通用生命周期和启动协调器只询问依赖是否为当前功能所需。

## 分阶段边界

第一阶段使用静态注册 provider 收敛调用点，配置与组件基线逐步进入包。静态 Rust / Vue 实现仍随本体编译，因此不能宣称插件已能独立交付任意新逻辑或界面。后续的本体扩展执行边界应单独设计，并保留来源验证、权限、启停与恢复控制；无需为此破坏 Native ABI 或允许任意 JS。

验证至少包含空插件、单个、多个及全部已知插件，声明缺失/损坏、未知 provider、条件依赖、停用和旧包兼容。
