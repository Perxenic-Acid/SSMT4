import { createRouter, createWebHistory } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { defineAsyncComponent, type AsyncComponentLoader } from 'vue'
import { ElMessage } from 'element-plus'
import { exists } from '@tauri-apps/plugin-fs'
import Home from '../views/Home/Home.vue'
import PageLoading from '../components/PageLoading.vue'
import { PathHelper } from '../helper/PathHelper'
import { i18n } from '../i18n'

const t = i18n.global.t

/**
 * Every sub-page is an async component with an immediate loading indicator:
 * the route switches right away and shows a lightweight spinner while the
 * chunk loads / the page mounts, instead of freezing on the old page.
 */
const asyncPage = (loader: AsyncComponentLoader) =>
  defineAsyncComponent({
    loader,
    loadingComponent: PageLoading,
    // Avoid flashing the spinner for chunks that resolve almost instantly
    delay: 120,
  })

const loadWorkPage = () => import('../views/WorkPage/WorkPage.vue')
const loadModsManagement = () => import('../views/ModsManagement/ModsManagement.vue')
const loadGameBanana = () => import('../views/GameBanana/GameBanana.vue')
const loadGameBananaAuthor = () => import('../views/GameBanana/GameBananaAuthor.vue')
const loadNexusMods = () => import('../views/NexusMods/NexusMods.vue')
const loadMarkTextureFull = () => import('../views/MarkTexture/MarkTextureFull.vue')
const loadSettings = () => import('../views/Settings/Settings.vue')
const loadXianZun = () => import('../views/XianZun/XianZun.vue')
const loadUIBuilder = () => import('../views/UIBuilder/UIBuilder.vue')
const loadTextureModMaker = () => import('../views/TextureModMaker/TextureModMaker.vue')
const loadPluginMarketplace = () => import('../views/PluginMarketplace/PluginMarketplace.vue')
const loadHoYoShadeSettings = () => import('../plugin/components/HoYoShadeSettings.vue')

const WorkPage = asyncPage(loadWorkPage)
const ModsManagement = asyncPage(loadModsManagement)
const GameBanana = asyncPage(loadGameBanana)
const GameBananaAuthor = asyncPage(loadGameBananaAuthor)
const NexusMods = asyncPage(loadNexusMods)
const MarkTextureFull = asyncPage(loadMarkTextureFull)
const Settings = asyncPage(loadSettings)
const XianZun = asyncPage(loadXianZun)
const UIBuilder = asyncPage(loadUIBuilder)
const TextureModMaker = asyncPage(loadTextureModMaker)
const PluginMarketplace = asyncPage(loadPluginMarketplace)
const HoYoShadeSettings = asyncPage(loadHoYoShadeSettings)

/**
 * Warm the route chunks in the background once the app has started, so the
 * first visit to a heavy page only pays the mount cost, never the load cost.
 */
export const prefetchRouteComponents = () => {
  const loaders = [
    loadSettings,
    loadModsManagement,
    loadWorkPage,
    loadGameBanana,
    loadNexusMods,
    loadMarkTextureFull,
    loadTextureModMaker,
    loadXianZun,
    loadGameBananaAuthor,
    loadUIBuilder,
    loadPluginMarketplace,
    loadHoYoShadeSettings,
  ]
  loaders.forEach((load, index) => {
    window.setTimeout(() => {
      void load().catch(() => {
        /* Prefetch is best-effort — a later real navigation retries. */
      })
    }, 600 + index * 320)
  })
}

const routes = [
  { path: '/', name: 'Home', component: Home, meta: { title: 'Home', requiresGame: false } },
  { path: '/mods', name: 'ModsManagement', component: ModsManagement, meta: { title: 'Mods Management', requiresGame: true } },
  { path: '/gamebanana', name: 'GameBanana', component: GameBanana, meta: { title: 'GameBanana', requiresGame: false } },
  { path: '/gamebanana/author/:authorId', name: 'GameBananaAuthor', component: GameBananaAuthor, meta: { title: 'GameBanana Author', requiresGame: false } },
  { path: '/nexusmods', name: 'NexusMods', component: NexusMods, meta: { title: 'Nexus Mods', requiresGame: false } },
  { path: '/work', name: 'WorkPage', component: WorkPage, meta: { title: 'Work Page', requiresGame: true } },
  { path: '/mark-texture-full', name: 'MarkTextureFull', component: MarkTextureFull, meta: { title: 'Texture Marking', requiresGame: true } },
  { path: '/settings', name: 'Settings', component: Settings, meta: { title: 'Settings', requiresGame: false } },
  { path: '/xianzun', name: 'XianZun', component: XianZun, meta: { title: '芝士猫', requiresGame: false } },
  { path: '/ui-builder', name: 'UIBuilder', component: UIBuilder, meta: { title: 'UI Builder', requiresGame: false } },
  { path: '/texture-mod-maker', name: 'TextureModMaker', component: TextureModMaker, meta: { title: 'Texture Mod Maker', requiresGame: true } },
  { path: '/plugins', name: 'PluginMarketplace', component: PluginMarketplace, meta: { title: 'Plugin Marketplace', requiresGame: false } },
  {
    path: '/plugins/hoyoshade',
    name: 'HoYoShadeSettings',
    component: HoYoShadeSettings,
    meta: { title: 'HoYoShade Bridge', requiresGame: false },
    beforeEnter: async () => {
      try {
        const routes = await invoke<Array<{ pluginId: string; route: string }>>('plugin_ui_routes')
        return routes.some(route => route.pluginId === 'ssmt.hoyoshade.bridge' && route.route === '/plugins/hoyoshade')
          ? true
          : { name: 'PluginMarketplace' }
      } catch {
        return { name: 'PluginMarketplace' }
      }
    },
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach(async (to) => {
  const needMigotoPath = new Set(['WorkPage', 'MarkTextureFull', 'TextureModMaker'])
  if (!needMigotoPath.has(String(to.name ?? ''))) {
    return true
  }

  const current3DmigotoFolderPath = await PathHelper.GetCurrentGame3DmigotoFolderPath()
  if (current3DmigotoFolderPath && (await exists(current3DmigotoFolderPath))) {
    return true
  }

  ElMessage.warning(t('router.messages.configureMigotoPathFirst'))
  return { name: 'Home' }
})

export default router
