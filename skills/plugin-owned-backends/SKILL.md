---
name: plugin-owned-backends
description: 在 SSMT 中将插件专用设置、后台和启动逻辑迁回插件包，维护通用后台协议与插件隔离边界。
---

先阅读 `docs/plugin-owned-backends.md`，确认当前完成范围和仍在本体的专用模块。不要把“使用 graphicsProfiles 或注册 provider”误判为已经实现业务隔离。

业务算法和旧设置字段由插件代码解释。本体的通用后台入口位于 `src-tauri/src/plugin/backend.rs`；`plugin.backend.request` 必须由有效 PageLease 解析身份，并限制到清单方法。生命周期只由宿主发起。新增操作沿用包内路径校验、真实文件哈希、有界标准输入输出、超时终止和失败回滚。

Player Tweaks 的事务代码及相关内存回归在 `native/crates/player-tweaks-backend`。恢复原值时保留用户后续修改；注册表写入前持久化所有权，恢复写入成功后才释放所有权。跨进程锁需由进程退出自动释放。

图形插件共享的 ReShade 组合、设置保留和文件事务位于 `native/crates/plugin-graphics`。不要在不同插件后台复制两套恢复实现。迁移期间本体仍依赖该库并保留旧图形入口；最终验收必须移除这条依赖及重导出，不能把源码移到 native 后继续静态编进本体称为隔离。独立执行该库测试，并保留宿主的多插件启停回归直到新执行入口接管。

构建后台使用 `native/tools/build-player-tweaks-backend.ps1`，完整包由既有 `pack-official-plugins.ps1` 从构建产物更新哈希。不要将生成 EXE/DLL 手工写入受跟踪的包来源目录，也不要将开发包标为可发布。

多个后台统一使用 `native/tools/build-plugin-backends.ps1`，旧 Player Tweaks 脚本只作为转发入口。HoYoShade 独立实现位于 `native/crates/hoyoshade-backend`；开发包尚未接入实际启动入口，不能将子进程测试通过写成整条迁移完成。

通用启动协议位于 `backend_launch.rs`：共享频道保持不透明，不能在宿主增加 ReShade、DLSS5 或固定插件 ID 的分派。任一贡献失败时先排除其共享数据，再取消并重建所有已准备消费者，最后重算回退候选。共享 Add-on 的消费方需要检查宿主附带的来源权限及风险确认，不能用消费方的一次确认替代其他来源插件的确认。

执行文档中的独立后台进程回归与相关宿主检查。区分内存测试、真实子进程协议、完整包校验、WebView 和游戏验收；前一层通过不能替代后一层。主仓库与 native 的源码变更分别报告。
