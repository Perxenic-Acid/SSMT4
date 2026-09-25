import { reactive, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { ElMessage, ElMessageBox } from 'element-plus'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { currentMonitor, getCurrentWindow, LogicalSize, primaryMonitor } from '@tauri-apps/api/window'
import { getVersion } from '@tauri-apps/api/app'
import { AppSettings, BGType, DEFAULT_PAGE_VISIBILITY, SSMTLocale, type PageVisibilitySettings } from './AppSettings'
import { applyAppThemeColors } from './AppTheme'
import { GlobalConfig } from './GlobalConfig'
import { AUTO_UPDATE_SUPPORTED_PRESET_SET } from './GamePreset'
import { ResourceManager } from './ResourceManager'
import { i18n } from '../i18n'

const t = i18n.global.t

const DEFAULT_BG_IMAGE = '/Background.png'
const DEFAULT_WINDOW_WIDTH = 1280
const DEFAULT_WINDOW_HEIGHT = 720
const MIN_WINDOW_WIDTH = 1043
const MIN_WINDOW_HEIGHT = 603
const WINDOW_SCREEN_MARGIN = 48

export interface GameInfo {
  name: string
  iconPath: string
  bgPath: string
  bgVideoPath?: string
  bgThemePath?: string
  bgImages?: string[]
  bgVideos?: string[]
  backgroundEntries?: Array<{
    id: string
    type: BGType
    imagePath?: string
    videoPath?: string
    themePath?: string
    imageUrl?: string
    videoUrl?: string
    themeUrl?: string
  }>
  selectedBackgroundId?: string
  bgIndex?: number
  bgType: BGType
  showSidebar: boolean
}

// Reactive state at module level — NOT returned from store to avoid Pinia ref-unwrapping.
// External code accesses these directly via AppStateManager.isDrawerOpen / .value etc.
const isDrawerOpen = ref(false)
const appSettings = reactive(new AppSettings())
const gamesList = reactive<GameInfo[]>([])
const gamesDir = ref<string>('')
const isFirstRunOnboardingOpen = ref(false)
const gameSwitchRevision = ref(0)

export const useAppStateStore = defineStore('appState', () => {
  const autoUpdateInFlight = new Set<string>()

  let initialStateLoaded = false
  let isInitialized = false
  let sizeListenerRegistered = false
  let saveDebounceTimer: ReturnType<typeof setTimeout> | null = null

  function scheduleSettingsSave(settings: AppSettings) {
    if (saveDebounceTimer) {
      clearTimeout(saveDebounceTimer)
    }

    const snapshot = new AppSettings({ ...settings })
    saveDebounceTimer = setTimeout(() => {
      saveDebounceTimer = null
      void GlobalConfig.SaveConfig(snapshot).catch((error) => {
        console.error('Failed to save settings:', error)
      })
    }, 250)
  }

  async function resolveSafeWindowBounds(width: number, height: number) {
    const monitor = (await currentMonitor()) ?? (await primaryMonitor())

    if (!monitor) {
      return {
        width: Math.max(Math.round(width), MIN_WINDOW_WIDTH),
        height: Math.max(Math.round(height), MIN_WINDOW_HEIGHT),
      }
    }

    const scaleFactor = monitor.scaleFactor || 1
    const workAreaWidth = monitor.workArea.size.width / scaleFactor
    const workAreaHeight = monitor.workArea.size.height / scaleFactor
    const maxWidth = Math.max(320, Math.floor(workAreaWidth - WINDOW_SCREEN_MARGIN))
    const maxHeight = Math.max(240, Math.floor(workAreaHeight - WINDOW_SCREEN_MARGIN))

    const normalizedWidth = Math.max(Math.round(width), MIN_WINDOW_WIDTH)
    const normalizedHeight = Math.max(Math.round(height), MIN_WINDOW_HEIGHT)

    return {
      width: maxWidth >= MIN_WINDOW_WIDTH ? Math.min(normalizedWidth, maxWidth) : maxWidth,
      height: maxHeight >= MIN_WINDOW_HEIGHT ? Math.min(normalizedHeight, maxHeight) : maxHeight,
    }
  }

  async function syncVersionNumberFromTauri() {
    try {
      const versionText = await getVersion()
      const versionNumber = AppSettings.parseVersionStringToNumber(versionText)
      if (versionNumber > 0) {
        AppSettings.setCurrentVersionNumber(versionNumber)
        appSettings.VersionNumber = versionNumber
      }
    } catch (err) {
      console.warn('Failed to read app version from Tauri API', err)
    }
  }

  async function registerWindowSizeListener() {
    if (sizeListenerRegistered) return
    try {
      const win = getCurrentWindow()
      await win.onResized(async ({ payload }) => {
        try {
          const factor = await win.scaleFactor()
          appSettings.windowWidth = payload.width / factor
          appSettings.windowHeight = payload.height / factor
        } catch (err) {
          console.warn('Failed to capture window size change', err)
        }
      })
      sizeListenerRegistered = true
    } catch (err) {
      console.warn('Failed to register window size listener', err)
    }
  }

  async function restoreWindowBounds(width: number, height: number) {
    try {
      const win = getCurrentWindow()
      const safeBounds = await resolveSafeWindowBounds(width, height)
      await win.setSize(new LogicalSize(safeBounds.width, safeBounds.height))
      await win.center()
    } catch (err) {
      console.warn('Failed to restore window bounds from settings', err)
    }
  }

  function reconcileSidebarGameOrder(
    games: readonly GameInfo[],
    preferredOrder: readonly string[] = appSettings.sidebarGameOrder,
  ): string[] {
    const availableNames = new Set(games.map(game => game.name))
    const seen = new Set<string>()
    const normalizedOrder: string[] = []

    for (const name of preferredOrder) {
      if (!availableNames.has(name) || seen.has(name)) {
        continue
      }

      seen.add(name)
      normalizedOrder.push(name)
    }

    for (const game of games) {
      if (seen.has(game.name)) {
        continue
      }

      seen.add(game.name)
      normalizedOrder.push(game.name)
    }

    return normalizedOrder
  }

  function setSidebarGameOrder(preferredOrder: readonly string[]) {
    appSettings.sidebarGameOrder = reconcileSidebarGameOrder(gamesList, preferredOrder)
  }

  async function prepareWindowForDisplay() {
    const savedWidth = appSettings.windowWidth
    const savedHeight = appSettings.windowHeight
    const width = savedWidth ?? DEFAULT_WINDOW_WIDTH
    const height = savedHeight ?? DEFAULT_WINDOW_HEIGHT

    await restoreWindowBounds(width, height)
  }

  async function restoreWindowBoundsFromSettings() {
    const savedWidth = appSettings.windowWidth
    const savedHeight = appSettings.windowHeight
    if (!savedWidth || !savedHeight) {
      return
    }

    const width = Math.max(savedWidth, MIN_WINDOW_WIDTH)
    const height = Math.max(savedHeight, MIN_WINDOW_HEIGHT)
    await restoreWindowBounds(width, height)
  }

  async function loadSettings() {
    try {
      await syncVersionNumberFromTauri()
      const loaded = await GlobalConfig.ReadConfig()
      console.log('Loaded settings from disk:', loaded)
      Object.assign(appSettings, loaded)
      appSettings.VersionNumber = AppSettings.getCurrentVersionNumber()
      applyAppThemeColors()
      syncShowWindowShortcut()
      
      void registerWindowSizeListener()
      setTimeout(() => {
        isInitialized = true
      }, 100)
    } catch (e) {
      console.error('Failed to load settings:', e)
      await ElMessageBox.alert(t('appState.messages.loadSettingsFailed', { error: String(e) }), t('appState.common.errorTitle'), {
        confirmButtonText: t('appState.common.confirm'),
        type: 'error'
      })
    }
  }

  async function saveSettingsNow() {
    if (saveDebounceTimer) {
      clearTimeout(saveDebounceTimer)
      saveDebounceTimer = null
    }
    await GlobalConfig.SaveConfig(new AppSettings({ ...appSettings }))
  }

  function syncShowWindowShortcut() {
    void invoke('set_show_window_shortcut_enabled', {
      enabled: appSettings.showWindowShortcutEnabled,
    }).catch((error) => {
      console.warn('Failed to sync show-window shortcut state:', error)
    })
  }

  async function loadGames() {
    try {
      const games = await ResourceManager.scanGames()

      const visibleGames = games

      console.log('Available games:', visibleGames)

      const processed = visibleGames.map(g => {
        const timestamp = Date.now()
        const icon = g.icon_path ? convertFileSrc(g.icon_path) + `?t=${timestamp}` : ''
        const bg = g.bg_path ? convertFileSrc(g.bg_path) + `?t=${timestamp}` : undefined
        const bgVideo = g.bg_video_path ? convertFileSrc(g.bg_video_path) + `?t=${timestamp}` : undefined
        const bgTheme = g.bg_theme_path ? convertFileSrc(g.bg_theme_path) + `?t=${timestamp}` : undefined

        return {
          name: g.name,
          iconPath: icon,
          bgPath: bg,
          bgVideoPath: bgVideo,
          bgThemePath: bgTheme,
          bgImages: (g.bg_images || []).map(path => convertFileSrc(path) + `?t=${timestamp}`),
          bgVideos: (g.bg_videos || []).map(path => convertFileSrc(path) + `?t=${timestamp}`),
          backgroundEntries: (g.background_entries || []).map(entry => ({
            ...entry,
            imagePath: entry.imagePath ? convertFileSrc(entry.imagePath) + `?t=${timestamp}` : undefined,
            videoPath: entry.videoPath ? convertFileSrc(entry.videoPath) + `?t=${timestamp}` : undefined,
            themePath: entry.themePath ? convertFileSrc(entry.themePath) + `?t=${timestamp}` : undefined,
          })),
          selectedBackgroundId: g.selected_background_id,
          bgIndex: g.selected_background_id
            ? Math.max(0, (g.background_entries || []).findIndex(entry => entry.id === g.selected_background_id))
            : 0,
          bgType: g.bg_type || BGType.Image,
          showSidebar: g.show_sidebar,
        } as GameInfo
      })

      appSettings.sidebarGameOrder = reconcileSidebarGameOrder(processed)

      gamesList.splice(0, gamesList.length, ...processed)

      if (appSettings.CurrentGameName) {
        const current = gamesList.find(g => g.name === appSettings.CurrentGameName)
        if (current) {
          await selectGame(current, { persist: false })
        } else {
          appSettings.CurrentGameName = 'Default'
          appSettings.bgType = BGType.Image
          appSettings.bgImage = DEFAULT_BG_IMAGE
          appSettings.bgVideo = ''
          appSettings.bgTheme = ''
        }
      } else {
        appSettings.bgType = BGType.Image
        appSettings.bgImage = DEFAULT_BG_IMAGE
        appSettings.bgVideo = ''
        appSettings.bgTheme = ''
      }
    } catch (e) {
      console.error('Failed to scan games:', e)
    }
  }

  async function selectGame(game: GameInfo, options: { persist?: boolean } = {}) {
    await ResourceManager.ensureGameConfigExists(game.name)
    switchToGame(game)

    // A game determines all subsequent per-game paths. Persist the switch
    // synchronously so closing the app cannot restore the previous game.
    if (options.persist !== false) {
      await saveSettingsNow()
    }
  }

  function switchToGame(game: GameInfo) {
    appSettings.CurrentGameName = game.name
    // Keep the legacy/global mirror aligned with the selected game. Consumers
    // that still read CurrentWorkSpace must never observe the previous game's
    // workspace during a game switch.
    // The lookup key must be normalized exactly like the writers do
    // (trimmed, 'Default' -> 'DefaultGame'), otherwise the mirror is cleared
    // for the Default game or for names with stray whitespace and downstream
    // pages resolve the wrong workspace.
    const workspaceMemoryKey = (() => {
      const trimmedName = (game.name || '').trim()
      return trimmedName && trimmedName !== 'Default' ? trimmedName : 'DefaultGame'
    })()
    appSettings.CurrentWorkSpace = appSettings.CurrentWorkSpaceByGame?.[workspaceMemoryKey] || ''
    const useVideo = game.bgType === BGType.Video

    const selectedEntry = game.backgroundEntries?.[game.bgIndex || 0]
    if (selectedEntry) {
      applyBackgroundEntry(game, game.bgIndex || 0)
      void autoUpdateBackgroundIfNeeded(game)
      return
    }

    if (useVideo && game.bgVideoPath) {
      const newVideo = game.bgVideoPath || ''
      appSettings.bgTheme = game.bgThemePath || ''
      if (appSettings.bgType !== BGType.Video) {
        appSettings.bgType = BGType.Video
        if (appSettings.bgVideo !== newVideo) appSettings.bgVideo = newVideo
        if (appSettings.bgImage !== '') appSettings.bgImage = ''
      } else if (appSettings.bgVideo !== newVideo) {
        appSettings.bgVideo = newVideo
      }
    } else {
      const newImage = game.bgPath || ''
      const resolvedImage = newImage || DEFAULT_BG_IMAGE
      if (appSettings.bgType !== BGType.Image) {
        appSettings.bgType = BGType.Image
        if (appSettings.bgImage !== resolvedImage) appSettings.bgImage = resolvedImage
        if (appSettings.bgVideo !== '') appSettings.bgVideo = ''
        appSettings.bgTheme = ''
      } else if (appSettings.bgImage !== resolvedImage) {
        appSettings.bgImage = resolvedImage
      }
    }

    void autoUpdateBackgroundIfNeeded(game)
  }

  function applyBackgroundEntry(game: GameInfo, index: number): void {
    const entries = game.backgroundEntries || []
    if (!entries.length) return
    const normalizedIndex = ((index % entries.length) + entries.length) % entries.length
    const entry = entries[normalizedIndex]
    game.bgIndex = normalizedIndex
    if (entry.type === BGType.Video && entry.videoPath) {
      appSettings.bgType = BGType.Video
      appSettings.bgVideo = entry.videoPath
      appSettings.bgImage = entry.imagePath || ''
      appSettings.bgTheme = entry.themePath || ''
      return
    }

    appSettings.bgType = BGType.Image
    appSettings.bgImage = entry.imagePath || entry.videoPath || DEFAULT_BG_IMAGE
    appSettings.bgVideo = ''
    appSettings.bgTheme = ''
  }

  function refreshCurrentPage() {
    gameSwitchRevision.value += 1
  }

  function switchToDefaultGame() {
    appSettings.CurrentGameName = 'Default'
    // The Default pseudo-game shares the 'DefaultGame' workspace bucket.
    appSettings.CurrentWorkSpace = appSettings.CurrentWorkSpaceByGame?.['DefaultGame'] || ''
    appSettings.bgType = BGType.Image
    appSettings.bgImage = DEFAULT_BG_IMAGE
    appSettings.bgVideo = ''
    appSettings.bgTheme = ''
  }

  function hasSelectedGame(): boolean {
    const currentGameName = appSettings.CurrentGameName.trim()
    if (!currentGameName || currentGameName === 'Default') {
      return false
    }

    if (gamesList.length === 0) {
      return true
    }

    return gamesList.some(game => game.name === currentGameName)
  }

  function hasLoadedInitialState(): boolean {
    return initialStateLoaded
  }

  async function autoUpdateBackgroundIfNeeded(game: GameInfo) {
    if (!game) return
    if (autoUpdateInFlight.has(game.name)) return

    autoUpdateInFlight.add(game.name)
    try {
      const config = await ResourceManager.loadGameConfig(game.name)
      const preset: string | undefined = config?.gamePreset

      // Only auto-update if mode is 'auto' and the preset supports it
      if (config?.backgroundUpdateMode !== 'auto') return
      if (!preset || !AUTO_UPDATE_SUPPORTED_PRESET_SET.has(preset)) return

      try {
        const result = await ResourceManager.updateGameBackground(
          game.name, preset, (config.backgroundType as BGType) || BGType.Image,
          config.lastBackgroundUrl,
        )
        await loadGames()

        if (result.changed) {
          ElMessage({
            message: t('appState.messages.autoUpdatedImageBackground', { game: game.name }),
            type: 'success',
            duration: 3000,
            showClose: true,
          })
        }
      } catch (err) {
        console.warn('Auto background update failed:', err)
      }
    } finally {
      autoUpdateInFlight.delete(game.name)
    }
  }

  async function initAppState() {
    await loadSettings()
    if (await GlobalConfig.EnsureFirstRunMarker()) {
      appSettings.pageVisibility = Object.fromEntries(
        Object.keys(DEFAULT_PAGE_VISIBILITY).map(key => [key, false]),
      ) as PageVisibilitySettings
      isFirstRunOnboardingOpen.value = true
    }
    // 所有游戏配置均复制到全局配置，全部功能开放。
    gamesDir.value = await ResourceManager.CopyGamesToGlobalConfig(true)
    await loadGames()
    initialStateLoaded = true
  }

  async function completeFirstRunOnboarding(pageVisibility: PageVisibilitySettings) {
    appSettings.pageVisibility = { ...pageVisibility }
    isFirstRunOnboardingOpen.value = false
    await GlobalConfig.SaveConfig(new AppSettings({ ...appSettings }))
  }

  // Static init — run once on store creation
  watch(appSettings, async (newVal) => {
    if (!isInitialized) {
      console.log('Skipping save because store is not yet initialized')
      return
    }
    console.log('Saving settings:', newVal)
    scheduleSettingsSave(new AppSettings({ ...newVal }))
  }, { deep: true })

  watch(() => appSettings.showWindowShortcutEnabled, () => {
    syncShowWindowShortcut()
  })

  return {
    saveSettingsNow,
    setSidebarGameOrder,
    prepareWindowForDisplay,
    restoreWindowBoundsFromSettings,
    loadGames,
    selectGame,
    switchToGame,
    applyBackgroundEntry,
    refreshCurrentPage,
    switchToDefaultGame,
    hasSelectedGame,
    hasLoadedInitialState,
    initAppState,
    completeFirstRunOnboarding,
  }
})

// Backward-compatible wrapper
export const AppStateManager = {
  isDrawerOpen,
  appSettings,
  gamesList,
  gamesDir,
  isFirstRunOnboardingOpen,
  gameSwitchRevision,
  get saveSettingsNow() { return useAppStateStore().saveSettingsNow },
  get setSidebarGameOrder() { return useAppStateStore().setSidebarGameOrder },
  get prepareWindowForDisplay() { return useAppStateStore().prepareWindowForDisplay },
  get restoreWindowBoundsFromSettings() { return useAppStateStore().restoreWindowBoundsFromSettings },
  get loadGames() { return useAppStateStore().loadGames },
  get selectGame() { return useAppStateStore().selectGame },
  get switchToGame() { return useAppStateStore().switchToGame },
  get applyBackgroundEntry() { return useAppStateStore().applyBackgroundEntry },
  get refreshCurrentPage() { return useAppStateStore().refreshCurrentPage },
  get switchToDefaultGame() { return useAppStateStore().switchToDefaultGame },
  get hasSelectedGame() { return useAppStateStore().hasSelectedGame },
  get hasLoadedInitialState() { return useAppStateStore().hasLoadedInitialState },
  get initAppState() { return useAppStateStore().initAppState },
  get completeFirstRunOnboarding() { return useAppStateStore().completeFirstRunOnboarding },
}

export { BGType }
export { SSMTLocale }
