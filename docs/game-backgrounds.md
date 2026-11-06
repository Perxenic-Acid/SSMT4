# 游戏背景获取与回退

`ResourceManager.updateGameBackground` 下载背景并将可用资源写入游戏配置，
`AppStateManager` 将本地路径转换为界面资源，`App.vue` 负责视频播放。
首页背景列表沿用 `backgroundEntries`，不会直接播放带时效签名的远程 URL。

## 鸣潮官网资源

来源在 `src/store/ResourceManager.ts` 的 `fixedBackgroundSources.WWMI`：

- 海报：`https://mc.kurogames.com/website-preface/video/bg/bg-poster.webp`
- 视频：`https://mc.kurogames.com/website-preface/video/bg/bg-video.mp4`

视频下载前通过 `sign_wwmi_background_url` 添加 `auth_key`。后端复用现有 MD5
依赖，以 URL pathname、Unix 秒时间戳、5 字节十六进制随机数、UID `0` 和密钥
用 `-` 拼接后计算摘要。查询参数不参与摘要。

官网密钥轮换时，随 SSMT 更新维护
`src-tauri/src/commands/background.rs` 中的 `WWMI_BACKGROUND_AUTH_SECRET`。
测试中的固定摘要是与 Node.js `createHash('md5')` 对照的样例；更换密钥时同步更新。
2026-10-16 已用签名请求确认官网视频返回 HTTP 206、`video/mp4` 和 MP4 `ftyp` 标识。

配置、条目 ID 和缓存比较保留未签名的来源 URL；下载时才签名，避免时间戳和随机数
造成每次启动重复下载。静态与动态各占一个条目，共用一份本地海报。

## 失败回退

- 请求有超时限制；鸣潮视频会检查 MP4 标识，避免将鉴权错误页面保存为视频。
- 文件先写入临时文件再替换，下载或写入失败时保留原缓存。
- 鸣潮视频获取失败时保留静态条目，不加入没有视频文件的动态条目。
- 海报也不可获取时优先保留本地静态背景；全部不可用时保留原配置，由界面使用内置背景。
- 播放或解码失败时使用海报，没有海报则使用内置静态图，不改变游戏配置中的动态偏好。
- 播放错误只处理当前视频；已经离场的视频报错不能覆盖新的游戏或背景。

验证：`node --test scripts/tests/test-game-backgrounds.mjs`、
在 `src-tauri` 下执行 `cargo test --lib commands::background::tests`，以及 `bun run build`。
测试夹具覆盖请求与状态切换；真实 WebView 的解码表现仍以应用实际播放为准。
## 自定义视频的播放与资源释放

自定义背景保存到独立文件，并写入背景列表与选中项；导入成功后改为手动更新，避免官网刷新覆盖选择。
复制或配置写入失败时保留旧背景，不先删除源文件；正在播放的文件不会被原位覆盖。

`BackgroundVideo.vue` 使用 `attachBackgroundVideo` 管理播放器：15 秒无首帧、播放后 10 秒无新画面或播放拒绝时释放解码器并回退海报。
支持视频帧回调的 WebView 按解码画面判断进度，其他环境按播放时间判断；窗口隐藏时暂停，恢复时尊重用户主动暂停。
切换背景或卸载组件会清除媒体来源并调用 `load()`，迟到的错误不能覆盖新背景。

自动测试覆盖状态与资源生命周期；特定编码、超大视频的实际解码性能仍需在目标 WebView 上验收。

## 设置弹窗更新后的配置快照

设置弹窗持有的是打开时加载的 `GameConfig` 快照。自动更新背景会独立加载、修改并
保存磁盘配置；如果更新后不重新加载弹窗快照，关闭弹窗时的整文件保存会把旧背景
重新写回。2026-11-06 在正式版 `v4.2.2` 上确认，该问题会让原神和绝区零互相覆盖：
更新后的绝区零配置会重新引用 `Games/GIMI` 下的原神素材。

弹窗配置必须绑定加载时的游戏名；加载中或游戏名不匹配时禁止保存、导入和自动更新。
自动更新完成并确认目标游戏没有切换后，必须重新加载目标游戏的磁盘配置再刷新主页。
`updateGameBackground` 只采用目标游戏配置中的 preset，官网响应须按 `game.id`
匹配请求的 game ID，首页扫描还须拒绝路径不在当前游戏目录内的背景条目。

设置弹窗保存采用“相对打开时快照的嵌套字段差异”合并到最新磁盘配置，不能整文件
覆盖，并通过队列串行执行。关闭时没有字段变化就只刷新内存快照，不写磁盘；保存期间
发生的新编辑必须保留给下一次保存。主页背景切换必须先固定被点击条目，再异步保存，
不能在等待后重新读取当前游戏的响应式背景列表。

`loadGames` 使用请求代次，迟到结果不能覆盖较新的扫描结果；自动更新刷新列表时不再
递归触发第二次自动更新。背景列表选择 ID 失效时先回退到 `backgroundType` 对应条目，
而不是固定选择第 0 项。HYP 背景元数据请求有 20 秒超时。

终末地 LevelDB 发现只有在活动 `.log` 不早于最新 `.ldb` 时才优先使用 `.log`；这是
降低旧记录误判的防护，不替代真正的 LevelDB 序列解析。

## 切换游戏后悬停正常、点击失效

2026-10-19 在运行中的 WebView 捕获到 `attachBackgroundVideo` 抛出
`TypeError: Illegal invocation`。旧实现把浏览器原生 `setInterval`、`clearInterval`
直接保存为 `clock` 的方法，调用时接收者变成 `clock`，违反浏览器 API 的接收者约束。
动态背景的挂载异常打断了 Vue 更新队列，因此 CSS 悬停和原生窗口操作仍有效，
而游戏切换、设置和页面导航无法更新。已经中断的开发页面需刷新一次才能恢复队列。

默认计时器现在通过包装函数调用 `globalThis.setInterval` 和
`globalThis.clearInterval`；保留可注入时钟用于超时测试。Node 的计时器和普通替身
不会暴露此问题，因此回归另行检查接收者，并在禁用 GPU 的 Chromium 中使用真实
计时器、真实 `BackgroundVideo`、主页、标题栏与 KeepAlive 切换验证：旧代码可复现
异常和设置失效，新代码多次切换静态 / 动态背景及设置页面均正常。

