# 主页游戏选择固定模式

默认保留标题栏下拉菜单。在主页菜单中点击图钉后，游戏选择固定到主页左侧；
标题栏入口通过透明度、位移和 visibility 隐藏，不移除布局占位，并通过 inert 禁止交互。
侧栏图钉执行反向切换，两种模式都将焦点转到新入口。

`AppSettings.homeGameSidebarPinned` 保存用户偏好，旧配置默认 false。
固定模式只作用于 `/`；其它路由始终显示标题栏入口且不提供图钉。
路由变化关闭已打开的下拉菜单，返回主页重新应用固定偏好。

组件职责：

- `TitleBar.vue`：标题栏入口与主页专属图钉。
- `HomeGameSidebar.vue`：主页侧栏、取消固定、滚动游戏列表。
- `GameSwitcherPin.vue`：两个入口共享的图钉按钮。
- `common/GameSwitcher.ts`：常用游戏排序，两入口复用相同收藏和用户排序。
- `Home.vue`：侧栏出现/退出动画，兼容减少动态效果偏好。

侧栏使用独立的 flex 布局列，固定时预留 96px，并与主内容宽度同步过渡。
主内容位于 `home-main` 命名容器中；新闻尺寸使用容器宽度单位，窄屏规则使用
容器查询，不能继续按整个窗口宽度判断。参见
[CSS 容器查询](https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Containment/Container_queries)。
新闻、版本信息和启动操作均以该主内容区域定位；全屏游戏设置弹窗放在容器之外。
侧栏不能绝对定位叠在内容上；自身按内容高度收缩，游戏较多时内部滚动。
侧栏整体保持垂直居中，采用透明图标列，避免整块胶囊背景抢占视觉注意力。

动画参数统一放在 `src/styles/tokens.css`：`--motion-duration-fast` 用于反馈，
`--motion-duration-standard` 用于普通切换，`--motion-duration-layout` 用于布局变化，
`--motion-delay-handoff` 用于入口交接。缓动分别使用 `--motion-ease-out` 和
`--motion-ease-layout`。其它组件可以直接在 CSS transition 中引用这些变量；
减少动态效果偏好会将公共时长和延迟归零。优先动画化 opacity、transform，
只有需要真实让出空间时才改变宽度，避免使用 transition: all。

标题栏按下时直接调用 `startDragging()`，由 Windows 原生标题栏拖动处理还原与贴靠，
不要在拖动前异步调用 `unmaximize()`。此前该调用未获权限且位于异常捕获之外，
导致最大化状态无法进入拖动。参见 [Tauri 自定义标题栏](https://v2.tauri.app/learn/window-customization/)
和 [Tao Windows 实现](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/windows/window.rs)。
组件回归检查原生拖动立即调用、最大化状态刷新和鼠标过滤；真实 Windows 拖动手势仍需实机验收。

侧栏同时保留其它游戏入口，避免标题栏隐藏后无法选择非常用游戏。
所有选择继续走 `AppStateManager.selectGame`，不创建平行的游戏状态或启动配置。

验证：

```powershell
node --test scripts/tests/test-game-switcher.mjs scripts/tests/test-launch-settings.mjs
bunx vue-tsc --noEmit
```

本次还在隔离的 Edge 组件夹具中验证真实 Vue 渲染和 CSS 动画：固定/取消固定、
标题栏布局宽度及导航位置不变、inert、侧栏滚动、非主页无图钉和路由往返。
夹具替换了 Tauri/配置服务，不代替完整应用的人工验收。

主页布局修正另使用真实 Home 模板和样式、模拟数据验证 1600×900、1280×720、
1043×603、900×600、720×600 下侧栏开关与预下载组合，共 20 种布局。
检查新闻、版本信息和启动区均在主内容范围内、相互不重叠，并检查侧栏收缩和减少动态效果。
