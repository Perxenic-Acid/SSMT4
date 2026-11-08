---
name: plugin-owned-backends
description: 在 SSMT 中将插件专用设置、后台和启动逻辑迁回插件包，维护通用后台协议与插件隔离边界。
---

先阅读 `docs/plugin-owned-backends.md`，确认实现、协议与尚未完成的实机验证。不要把“使用 graphicsProfiles 或注册 provider”误判为已经实现业务隔离。

业务算法和旧设置字段由插件代码解释。本体的通用后台入口位于 `src-tauri/src/plugin/backend.rs`；`plugin.backend.request` 必须由有效 PageLease 解析身份，并限制到清单方法。生命周期只由宿主发起。新增操作沿用包内路径校验、真实文件哈希、有界标准输入输出、超时终止和失败回滚。

Player Tweaks 的事务代码及相关内存回归在 `native/crates/player-tweaks-backend`。恢复原值时保留用户后续修改；注册表写入前持久化所有权，恢复写入成功后才释放所有权。跨进程锁需由进程退出自动释放。

图形插件共享的 ReShade 组合、设置保留和文件事务位于 `native/crates/plugin-graphics`。不要在不同插件后台复制两套恢复实现。本体已移除对该库的依赖及重导出，禁止重新静态编入本体。独立执行该库测试，同时保留通用宿主的多插件启停与资源仲裁回归。

构建后台使用 `native/tools/build-player-tweaks-backend.ps1`，完整包由既有 `pack-official-plugins.ps1` 从构建产物更新哈希。不要将生成 EXE/DLL 手工写入受跟踪的包来源目录，也不要将开发包标为可发布。

多个后台统一使用 `native/tools/build-plugin-backends.ps1`，旧 Player Tweaks 脚本只作为转发入口。HoYoShade 独立实现位于 `native/crates/hoyoshade-backend`；开发包已接入通用启动入口，保持不可发布直到真实游戏验证通过。DLSS5 的后台与页面分别位于 `native/crates/dlss5-backend` 和 `native/dlss5-integration-ui`；长任务及其恢复属于插件，不能在本体重新建立专用任务代理。

通用启动协议位于 `backend_launch.rs`：共享频道保持不透明，不能在宿主增加 ReShade、DLSS5 或固定插件 ID 的分派。任一贡献失败时先排除其共享数据，再取消并重建所有已准备消费者，最后重算回退候选。共享 Add-on 的消费方需要检查宿主附带的来源权限及风险确认，不能用消费方的一次确认替代其他来源插件的确认。

执行文档中的独立后台进程回归与相关宿主检查。区分内存测试、真实子进程协议、完整包校验、WebView 和游戏验收；前一层通过不能替代后一层。主仓库与 native 的源码变更分别报告。

合并计划失败时只撤回 `launch.cancel`，不能清除其他已经成功的 `prepare` 状态后继续注入其 DLL。整条启动未提交时才执行完整 `cancel`。取消请求携带原启动上下文，避免配置切换后恢复错误的游戏目标。

执行 `node scripts/dev/verify-plugin-isolation.mjs` 防止插件专用路由、组件、策略枚举或依赖回流。本体只提供通用目录选择、HTTPS 打开和后台代理；包内页面不得调用旧专有 Tauri 命令。
