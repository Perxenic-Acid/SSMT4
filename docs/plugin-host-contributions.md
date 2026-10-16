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

外部依赖目录按插件 ID 与依赖 ID 共享，不绑定包版本或游戏实例。升级前的版本设置仍可读取：优先使用当前版本显式选择，再从同一插件的旧记录按语义版本寻找最近配置；之后的选择写入共享路径。新版重新验证自身的 `requiredFiles`，不继承旧版的就绪状态、启用状态或来源信任，也不借用其他插件的同名依赖。无效的显式选择保留错误，不悄悄回落到另一目录。市场在依赖行就近显示具体原因。

## 分阶段边界

第一阶段使用静态注册 provider 收敛调用点，配置与组件基线逐步进入包。静态 Rust / Vue 实现仍随本体编译，因此不能宣称插件已能独立交付任意新逻辑或界面。后续的本体扩展执行边界应单独设计，并保留来源验证、权限、启停与恢复控制；无需为此破坏 Native ABI 或允许任意 JS。

验证至少包含空插件、单个、多个及全部已知插件，声明缺失/损坏、未知 provider、条件依赖、停用和旧包兼容。

## 当前落点

- `host_graphics` 统一收集当前游戏已启用的图形 provider，按声明的优先级选取 Host；前端检查计划冲突，通过后才应用输入契约。候选失败可排除后继续收集，不假定插件数量。
- DLSS5 0.2.2 的 `host/graphics.json` 提供预设与 EXE 联合路线、API、本次参数约束以及组件清单位置。下载、校验和准备读取同一份声明。旧包使用冻结兼容清单。GIMI FSR2 Bridge 的内部配置和接口兼容仍由已注册实现控制，不能仅修改下载清单就保证新 Bridge 兼容。
- Shade 0.1.1 声明目标进程及外部资源的 Host、INI、预设和 Add-on 路径，解析后交给公共组合器；组合器不再直接查找 Shade ID。新 provider 的执行实现仍需注册。
- Player Tweaks 0.1.0 声明各游戏功能、已注册后端和包内 DLL。安装包优先于旧的随应用资源，避免同时出现两张同 ID 卡片；原神只选择专用 Hook，崩铁只执行客户端设置。旧的随应用版本保留兼容路径。
- 市场从本体校验后的 `settingsComponent` 挂载已有可信组件；未知键或未注册的所有者不会获得其他插件的组件。停用时不挂载会读取插件状态的图形设置。

上述版本均暂缓正式发布。源码与已有 Native 产物可经现有本地打包管线生成测试包；发布仍须走 Native Release 来源和目录校验。JSON 契约没有改变 Native ABI，静态 Rust / Vue provider 也仍然需要随本体更新。
