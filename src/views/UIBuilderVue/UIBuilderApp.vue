<script setup lang="ts">
/* ═══════════════════════════════════════════════════════════════
   UI 构造器 — 应用根组件(纯 Vue 重构版)
   ═══════════════════════════════════════════════════════════════
   由路由组件 UIBuilder.vue 渲染,替代原 iframe(public/ui-builder-*.html)
   与 v-html 注入的 bodyMarkup.html(已删除):

   - 全部面板标记均为 components/ 下的纯 Vue 组件,事件经 useUib
     代理直达引擎的 window.UIB 命名空间,无任何 HTML 文件参与;
   - 引擎(uiBuilderEngine.ts)挂载后按既定 id 接管各容器内容;
   - 原生 alert/confirm/prompt 由 dialogs.ts 替换为自绘制弹窗;
   - 保存 INI/资源/预设经 hostBridge.ts 直连工作空间。

   页面切换时组件由 KeepAlive 保活,不会被销毁,编辑器状态完整保留。
   ═══════════════════════════════════════════════════════════════ */
import { onActivated, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue'
import JSZip from 'jszip'
import { createUIBuilderEngine } from './uiBuilderEngine'
import { installGlobalDialogs, uninstallGlobalDialogs } from './dialogs'
import UiDialogs from './UiDialogs.vue'
import UibDock from './components/UibDock.vue'
import UibSettingsWindow from './components/UibSettingsWindow.vue'
import UibAnimationWindow from './components/UibAnimationWindow.vue'
import UibPropertiesWindow from './components/UibPropertiesWindow.vue'
import UibResourceWindow from './components/UibResourceWindow.vue'
import UibTextLogicWindow from './components/UibTextLogicWindow.vue'
import UibModeBar from './components/UibModeBar.vue'
import UibViewport from './components/UibViewport.vue'
import UibHierarchy from './components/UibHierarchy.vue'
import UibOutputModal from './components/UibOutputModal.vue'
import type { UIBuilderEngineHandle } from './uiBuilderTypes'

// 引擎的资源打包模块通过全局 JSZip 工作(Vue 版由 npm 依赖提供)。
;(globalThis as Record<string, unknown>).JSZip = JSZip

const rootEl = ref<HTMLElement | null>(null)
const engine = shallowRef<UIBuilderEngineHandle | null>(null)

const boot = () => {
  if (engine.value) return
  if (!rootEl.value) return
  installGlobalDialogs()
  engine.value = createUIBuilderEngine(rootEl.value)
}

const teardown = () => {
  engine.value?.destroy()
  engine.value = null
  uninstallGlobalDialogs()
}

/** 手动刷新:销毁并重建引擎(等价于原 iframe 的重新加载)。 */
const refresh = () => {
  teardown()
  boot()
}

onMounted(boot)
onActivated(() => {
  // KeepAlive 恢复显示:引擎的窗口监听常驻,无需重建;
  // 若此前被手动销毁(刷新),这里补一次启动。
  if (!engine.value) boot()
})

onBeforeUnmount(teardown)
</script>

<template>
  <div ref="rootEl" class="ui-builder-app dark-mode">
    <!-- 左侧工具窗停靠栏 -->
    <UibDock />

    <!-- 左侧停靠轨道(仅作视觉背景)。
         悬浮窗不再嵌套在 .sidebar 内:其 backdrop-filter 会形成包含块,
         使 width: calc(100% - 150px) 按 62px 计算而塌缩成一条缝。 -->
    <div class="sidebar"></div>

    <!-- 悬浮工具窗(设置 / 动画 / 属性),引擎按停靠栏切换显隐 -->
    <UibSettingsWindow />
    <UibAnimationWindow />
    <UibPropertiesWindow />

    <!-- 资源 / 文本逻辑悬浮窗 -->
    <UibResourceWindow />
    <UibTextLogicWindow />

    <!-- 工作区模式切换 + 中央画布/蓝图/运行时 -->
    <UibModeBar />
    <UibViewport />

    <!-- 右侧层级树 -->
    <UibHierarchy />

    <!-- 生成结果输出 -->
    <UibOutputModal />
  </div>
  <button type="button" class="uib-page-refresh" title="重新加载编辑器" @click="refresh">
    ⟳ 重新加载
  </button>
  <UiDialogs />
</template>

<style>
@import './styles.css';
</style>
