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

