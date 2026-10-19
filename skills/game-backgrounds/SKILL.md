---
name: game-backgrounds
description: 维护 SSMT 游戏首页的官网背景获取、签名下载、动态静态列表和播放失败回退。
---

# 游戏背景维护

阅读 [背景来源与回退说明](../../docs/game-backgrounds.md)，追踪
`ResourceManager.updateGameBackground`、`AppStateManager.applyBackgroundEntry`
和 `App.vue` 的播放链路。首页列表使用已下载的本地条目。

- 鸣潮 CDN 密钥集中在后端 `WWMI_BACKGROUND_AUTH_SECRET`，不在多个调用点复制。
  换密钥时用 Node.js 摘要重新校验固定测试向量。
- 临时鉴权只在下载时生成；配置与缓存比较保留稳定来源 URL。
- 动态与静态是不同条目，但可共享海报。获取失败的视频不能成为重复静态条目。
- 保留缓存与用户偏好；下载失败和播放失败分别回退。延迟播放错误须核对当前来源。
- 自定义导入不能覆盖正在播放的文件；同步背景列表和选中项，写入失败保留旧配置。
- 媒体停帧须有超时回退；卸载时暂停、清空 src 并 load，隐藏窗口暂停但不能覆盖用户暂停。
- 原生 `setInterval` / `clearInterval` 不能直接挂到时钟对象再作为其方法调用；用包装函数保留浏览器全局接收者。Node 或普通替身不检查此约束，须覆盖默认时钟并用真实浏览器验证挂载、卸载和游戏切换。
- 切换后 CSS 悬停、原生窗口操作正常而页面点击失效时，检查动态背景挂载的 `Illegal invocation` 及 Vue 更新队列；不能仅按遮罩或网络阻塞处理。修复后刷新已中断的页面再验收。
- 运行文档中的回归与构建；实际官网请求、夹具状态验证和 WebView 解码是不同证据。
