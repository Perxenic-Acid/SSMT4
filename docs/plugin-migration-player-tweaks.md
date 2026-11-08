# Player Tweaks 插件后台迁移

Player Tweaks 的新迁移以插件自有后台为边界。旧文档中“SRMI 兼容层暂留本体”的安排已取消。

- 本体不再包含 `player_tweaks.rs`、`star_rail_tweaks.rs`、对应 Tauri Command 或启动时删除专用 DLL/INI 的逻辑。
- 页面及后台源码位于 native 仓库，后台从包内执行，通过通用 JSON 协议接收上下文。
- 游戏设置保存在插件作用域的逐游戏文件中；GIMI 在启动准备时生成 DLL 同目录 INI，SRMI 在插件后台维护注册表事务和恢复记录。
- 旧 `Config.json` 中的字段通过清单声明白名单传入，旧插件设置命名空间只传给插件自己。
- DLL 注入通过清单中的 `executableNames` 过滤条件区分游戏；本体不维护游戏名称与 DLL 的对应表。
- 新包要求 Plugin API 1.1，开发包保持不可发布，不重打已发布版本。

协议、构建命令、已完成验证、剩余游戏验收以及尚未迁移的图形插件见 [插件自有后台与本体边界](plugin-owned-backends.md)。
