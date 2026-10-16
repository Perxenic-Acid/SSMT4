<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { openPath, openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage } from 'element-plus'
import { FolderOpened, Refresh, Search, Setting, Document, Delete, Box, InfoFilled } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { clearPluginLog, readPluginLog } from '../../plugin/logs'
import { AppStateManager } from '../../store/AppStateManager'
import { ResourceManager } from '../../store/ResourceManager'

const PlayerTweaksCameraSettings = defineAsyncComponent(
  () => import('../../plugin/components/PlayerTweaksCameraSettings.vue'),
)
const PlayerTweaksGameplaySettings = defineAsyncComponent(
  () => import('../../plugin/components/PlayerTweaksGameplaySettings.vue'),
)
const Dlss5GraphicsSettings = defineAsyncComponent(
  () => import('../../plugin/components/Dlss5GraphicsSettings.vue'),
)

type DependencyStatus = 'missing' | 'invalid' | 'ready'

interface DependencyState {
  status: DependencyStatus
  path?: string | null
  reason?: string | null
}

interface ExternalDependency {
  id: string
  requiredFiles: string[]
  requiredFor?: string[] | null
  downloadUrl?: string | null
  downloadHint?: string | null
}

interface PluginManifest {
  id: string
  name: string
  version: string
  author: string
  description?: string
  compatibility?: { games?: string[] }
  permissions: string[]
  externalDependencies: ExternalDependency[]
  contributions?: { hostExtensions?: Array<{ id: string; settingsComponent?: string | null }> }
}

interface InstalledPlugin {
  manifest: PluginManifest
  enabled: boolean
  bundled?: boolean
  lifecycleStatus: 'installed' | 'enabled' | 'disabled' | 'update_available' | 'broken' | 'incompatible' | 'external_dependency_missing'
  externalDependencies: Record<string, DependencyState>
  official: boolean
  appScoped: boolean
  features?: string[]
}

interface CatalogEntry {
  id: string
  name: string
  description: string
  author: string
  version: string
  supportedGames: string[]
  permissions: string[]
  externalDependencies: ExternalDependency[]
  bundled?: boolean
  official?: boolean
  localArchive?: string
}

interface PackageInspection {
  manifest: PluginManifest
  sha256: string
  fileCount: number
  unpackedSize: number
}

interface LocalPluginPackage {
  archivePath: string
  inspection: PackageInspection
}

interface OfficialCatalogEntry {
  id: string
  name: string
  description: string
  author: string
  version: string
  supportedGames: string[]
  permissions: string[]
  externalDependencies: ExternalDependency[]
}

const { t } = useI18n()
const route = useRoute()
const selectedGameName = computed(() => AppStateManager.appSettings.CurrentGameName === 'Default'
  ? '' : AppStateManager.appSettings.CurrentGameName)

const catalog = ref<CatalogEntry[]>([])
const localCatalog = ref<CatalogEntry[]>([])
const catalogError = ref('')
const localCatalogError = ref('')
const reviewOpen = ref(false)
const reviewAccepted = ref(false)
const reviewArchive = ref('')
const reviewLocalId = ref('')
const reviewInspection = ref<PackageInspection | null>(null)

const activeTab = ref<'discover' | 'installed' | 'updates'>('discover')
const detailTab = ref<'features' | 'dependencies' | 'permissions' | 'details'>(route.query.tab === 'dependencies' ? 'dependencies' : 'features')
const searchQuery = ref('')
const selectedId = ref(typeof route.query.plugin === 'string' ? route.query.plugin : '')
const installed = ref<InstalledPlugin[]>([])
const installedGameName = ref('')
const loading = ref(false)
const catalogLoading = ref(false)
const localCatalogLoading = ref(false)
const refreshing = computed(() => loading.value || catalogLoading.value || localCatalogLoading.value)
let refreshRequestId = 0
const logDialogOpen = ref(false)
const pluginLog = ref('')
const logPlugin = ref<{ id: string; name: string } | null>(null)
const logLoading = ref(false)

const selectedEntry = computed(() => visibleEntries.value.find(entry => entry.id === selectedId.value))
const installedById = computed(() => {
  const byId = new Map<string, InstalledPlugin>()
  for (const plugin of installed.value) {
    const current = byId.get(plugin.manifest.id)
    if (!current || compareVersions(plugin.manifest.version, current.manifest.version) > 0) {
      byId.set(plugin.manifest.id, plugin)
    }
  }
  return byId
})
const selectedPlugin = computed(() => selectedEntry.value ? installedById.value.get(selectedEntry.value.id) : undefined)
const hasSettings = (plugin: InstalledPlugin | undefined, component: string) =>
  plugin?.manifest.contributions?.hostExtensions?.some(extension => extension.settingsComponent === component) ?? false
const selectedSettings = computed(() => selectedPlugin.value?.manifest.contributions?.hostExtensions?.map(extension => extension.settingsComponent) ?? [])
// 已安装版本可能与目录中的待更新版本不同，操作必须使用实际安装的依赖声明。
const selectedDependencies = computed(() => selectedPlugin.value?.manifest.externalDependencies ?? selectedEntry.value?.externalDependencies ?? [])
const dependencyActionsEnabled = computed(() => !!selectedPlugin.value?.enabled && !loading.value
  && selectedPlugin.value.lifecycleStatus !== 'incompatible'
  && (selectedPlugin.value.appScoped || (!!selectedGameName.value && installedGameName.value === selectedGameName.value)))
const optionalDependency = (dependency: ExternalDependency) => Array.isArray(dependency.requiredFor) && !dependency.requiredFor.includes('launch')
const dependencyDownloadUrl = (dependency: ExternalDependency) => {
  const raw = dependency.downloadUrl
  if (!raw || raw.trim() !== raw || /[\u0000-\u001f\u007f]/.test(raw)) return null
  try {
    const url = new URL(raw)
    return url.protocol === 'https:' && url.hostname && !url.username && !url.password ? url : null
  } catch { return null }
}
const dependencyDownloadSource = (dependency: ExternalDependency) => dependencyDownloadUrl(dependency)?.hostname ?? ''
const openDependencyDownload = async (dependencyId: string) => {
  if (!dependencyActionsEnabled.value) return
  const dependency = selectedDependencies.value.find(item => item.id === dependencyId)
  const url = dependency && dependencyDownloadUrl(dependency)
  if (!url) return
  try { await openUrl(url.href) }
  catch (error) { ElMessage.error(t('pluginMarketplace.dependencyAcquisition.openFailed', { error: String(error) })) }
}
const openDependencySettings = () => {
  if (dependencyActionsEnabled.value) openPluginSettings()
}
const openDependencyDirectory = async (dependencyId: string) => {
  if (!dependencyActionsEnabled.value) return
  const dependency = selectedPlugin.value?.externalDependencies[dependencyId]
  if (dependency?.status !== 'ready' || !dependency.path) return
  try { await openPath(dependency.path) }
  catch (error) { ElMessage.error(t('pluginMarketplace.dependencyAcquisition.openFolderFailed', { error: String(error) })) }
}
const graphicsTargetExe = ref('')
const graphicsConfigLoading = ref(false)
const graphicsConfigError = ref('')
let graphicsConfigRequestId = 0
const availableCatalog = computed(() => {
  const byId = new Map<string, CatalogEntry>()
  for (const entry of [...catalog.value, ...localCatalog.value]) {
    const current = byId.get(entry.id)
    if (!current || compareVersions(entry.version, current.version) >= 0) byId.set(entry.id, entry)
  }
  return [...byId.values()]
})
const allEntries = computed<CatalogEntry[]>(() => {
  const entries = [...availableCatalog.value]
  for (const plugin of installedById.value.values()) {
    if (entries.some(entry => entry.id === plugin.manifest.id)) continue
    entries.push({
      id: plugin.manifest.id,
      name: plugin.manifest.name,
      description: plugin.bundled && hasSettings(plugin, 'ssmt.player-tweaks.settings')
        ? t('pluginMarketplace.playerTweaksDescription')
        : plugin.manifest.description || t('pluginMarketplace.installedPackage'),
      author: plugin.manifest.author,
      version: plugin.manifest.version,
      supportedGames: plugin.manifest.compatibility?.games ?? [],
      permissions: plugin.manifest.permissions,
      externalDependencies: plugin.manifest.externalDependencies,
      bundled: plugin.bundled,
    })
  }
  return entries
})
const installedEntries = computed(() => allEntries.value.filter(entry => installedById.value.has(entry.id)))
const updateEntries = computed(() => availableCatalog.value.filter(entry => {
  const current = installedById.value.get(entry.id)
  return current && compareVersions(entry.version, current.manifest.version) > 0
}))
const lifecycleFor = (entry: CatalogEntry, plugin: InstalledPlugin) => {
  if (plugin.lifecycleStatus === 'incompatible') return 'incompatible'
  if (compareVersions(entry.version, plugin.manifest.version) > 0) return 'update_available'
  return plugin.lifecycleStatus
}
const rowLifecycleFor = (entry: CatalogEntry) => {
  const plugin = installedById.value.get(entry.id)
  return plugin ? lifecycleFor(entry, plugin) : undefined
}
const sourceLabelFor = (entry: CatalogEntry) => entry.localArchive
  ? t('pluginMarketplace.localBuild')
  : entry.bundled
  ? t('pluginMarketplace.bundled')
  : installedById.value.has(entry.id)
    ? t(installedById.value.get(entry.id)?.official ? 'pluginMarketplace.official' : 'pluginMarketplace.thirdParty')
    : t(entry.official ? 'pluginMarketplace.official' : 'pluginMarketplace.thirdParty')
const sourceKindFor = (entry: CatalogEntry) => entry.localArchive ? 'local'
  : entry.bundled ? 'bundled'
  : (installedById.value.get(entry.id)?.official ?? entry.official) ? 'official' : 'thirdParty'
const monogramFor = (entry: CatalogEntry) => {
  const name = entry.name.replace(/^SSMT\s+/i, '').trim()
  const words = name.split(/[\s-]+/)
  return (words.length > 1 ? words.slice(0, 2).map(word => Array.from(word)[0]).join('') : Array.from(name).slice(0, 2).join('')).toLocaleUpperCase()
}
const countFor = (tab: typeof activeTab.value) => tab === 'installed' ? installedEntries.value.length
  : tab === 'updates' ? updateEntries.value.length : allEntries.value.length
const visibleEntries = computed(() => {
  const entries = activeTab.value === 'installed' ? installedEntries.value
    : activeTab.value === 'updates' ? updateEntries.value : allEntries.value
  const query = searchQuery.value.trim().toLocaleLowerCase()
  return query ? entries.filter(entry =>
    [entry.name, entry.id, entry.author, entry.description].some(value => value.toLocaleLowerCase().includes(query))) : entries
})

const compareVersions = (left: string, right: string) => {
  const parse = (value: string) => value.split('.').map(part => Number.parseInt(part, 10) || 0)
  const a = parse(left)
  const b = parse(right)
  for (let index = 0; index < 3; index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) return (a[index] ?? 0) - (b[index] ?? 0)
  }
  return 0
}

const refreshInstalled = async () => {
  const requestId = ++refreshRequestId
  const gameName = selectedGameName.value
  loading.value = true
  try {
    const snapshot = await invoke<InstalledPlugin[]>('plugin_registry_snapshot_for_game', {
      gameName,
    })
    if (requestId === refreshRequestId) {
      installed.value = snapshot
      installedGameName.value = gameName
    }
  } catch (error) {
    if (requestId === refreshRequestId) {
      console.error('Failed to load plugin registry:', error)
      ElMessage.error(t('pluginMarketplace.messages.loadFailed', { error: String(error) }))
    }
  } finally {
    if (requestId === refreshRequestId) loading.value = false
  }
}

const refreshCatalog = async () => {
  catalogLoading.value = true
  try {
    const entries = await invoke<OfficialCatalogEntry[]>('official_plugin_catalog')
    catalog.value = entries.map(entry => ({
      id: entry.id,
      name: entry.name,
      description: entry.description,
      author: entry.author,
      version: entry.version,
      supportedGames: entry.supportedGames,
      permissions: entry.permissions,
      externalDependencies: entry.externalDependencies,
      official: true,
    }))
    catalogError.value = ''
  } catch (error) {
    catalogError.value = String(error)
  } finally {
    catalogLoading.value = false
  }
}

const refreshAll = async () => {
  if (refreshing.value) return
  await Promise.all([refreshInstalled(), refreshCatalog(), refreshLocalCatalog()])
}

const refreshLocalCatalog = async () => {
  localCatalogLoading.value = true
  try {
    const packages = await invoke<LocalPluginPackage[]>('local_plugin_packages')
    localCatalog.value = packages.map(({ archivePath, inspection }) => ({
      id: inspection.manifest.id,
      name: inspection.manifest.name,
      description: inspection.manifest.description || inspection.manifest.name,
      author: inspection.manifest.author,
      version: inspection.manifest.version,
      supportedGames: inspection.manifest.compatibility?.games ?? [],
      permissions: inspection.manifest.permissions,
      externalDependencies: inspection.manifest.externalDependencies,
      localArchive: archivePath,
    }))
    localCatalogError.value = ''
  } catch (error) {
    localCatalogError.value = String(error)
  } finally {
    localCatalogLoading.value = false
  }
}

const installLocal = async (entry: CatalogEntry) => {
  loading.value = true
  try {
    const packages = await invoke<LocalPluginPackage[]>('local_plugin_packages')
    const current = packages.find(item => item.inspection.manifest.id === entry.id)
    if (!current) throw new Error(t('pluginMarketplace.messages.localUnavailable'))
    reviewInspection.value = current.inspection
    reviewArchive.value = current.archivePath
    reviewLocalId.value = entry.id
    reviewAccepted.value = false
    reviewOpen.value = true
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.installFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const installFromFile = async () => {
  const selected = await openDialog({
    multiple: false,
    filters: [{ name: 'SSMT Plugin Package', extensions: ['ssmtpkg'] }],
    title: t('pluginMarketplace.actions.choosePackage'),
  })
  if (typeof selected !== 'string') return

  loading.value = true
  try {
    reviewInspection.value = await invoke<PackageInspection>('inspect_plugin_package', { archivePath: selected })
    reviewArchive.value = selected
    reviewLocalId.value = ''
    reviewAccepted.value = false
    reviewOpen.value = true
  } catch (error) {
    console.error('Failed to install plugin package:', error)
    ElMessage.error(t('pluginMarketplace.messages.installFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const installReviewed = async () => {
  if (!reviewAccepted.value || !reviewInspection.value) return
  loading.value = true
  try {
    await invoke(reviewLocalId.value ? 'install_local_plugin' : 'install_plugin_package', {
      ...(reviewLocalId.value ? { id: reviewLocalId.value } : { archivePath: reviewArchive.value }),
      reviewedSha256: reviewInspection.value.sha256,
      disclaimerAccepted: true,
    })
    reviewOpen.value = false
    await refreshInstalled()
    ElMessage.success(t('pluginMarketplace.messages.installed'))
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.installFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const installOfficial = async (entry: CatalogEntry) => {
  loading.value = true
  try {
    await invoke('install_official_plugin', { id: entry.id, version: entry.version })
    await refreshInstalled()
    ElMessage.success(t('pluginMarketplace.messages.installed'))
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.installFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const configureDependency = async (pluginId: string, dependencyId: string) => {
  const selected = await openDialog({ directory: true, multiple: false })
  if (typeof selected !== 'string') return
  loading.value = true
  try {
    await invoke('set_plugin_external_dependency_path', {
      pluginId, dependencyId, path: selected,
    })
    await refreshInstalled()
    ElMessage.success(t('pluginMarketplace.messages.dependencySaved'))
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.dependencySaveFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const togglePlugin = async (plugin: InstalledPlugin) => {
  if (!selectedGameName.value && !plugin.appScoped) return
  if (!plugin.appScoped && installedGameName.value !== selectedGameName.value) return
  if (!plugin.enabled && plugin.lifecycleStatus === 'incompatible') return
  const gameName = selectedGameName.value
  const requestId = ++refreshRequestId
  loading.value = true
  try {
    let gameExecutable: string | null = null
    if (plugin.enabled && !plugin.appScoped) {
      try {
        gameExecutable = (await ResourceManager.loadGameConfig(gameName)).targetExePath?.trim() || null
      } catch {
        // 后端仍可使用安装时记录的游戏路径完成恢复。
      }
    }
    const snapshot = await invoke<InstalledPlugin[]>('set_plugin_enabled_for_game', {
      gameName,
      id: plugin.manifest.id,
      enabled: !plugin.enabled,
      gameExecutable,
    })
    if (requestId === refreshRequestId) {
      installed.value = snapshot
      installedGameName.value = gameName
    }
    ElMessage.success(plugin.enabled
      ? t('pluginMarketplace.messages.disabled')
      : t('pluginMarketplace.messages.enabled'))
  } catch (error) {
    console.error('Failed to update plugin state:', error)
    ElMessage.error(t('pluginMarketplace.messages.updateFailed', { error: String(error) }))
  } finally {
    if (requestId === refreshRequestId) loading.value = false
  }
}

const openPluginLog = async () => {
  if (!selectedEntry.value || !installedById.value.has(selectedEntry.value.id) || logLoading.value) return
  const { id, name } = selectedEntry.value
  logPlugin.value = { id, name }
  logLoading.value = true
  try {
    pluginLog.value = await readPluginLog(id)
    logDialogOpen.value = true
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.logFailed', { error: String(error) }))
  } finally {
    logLoading.value = false
  }
}

const clearSelectedPluginLog = async () => {
  if (!logPlugin.value || logLoading.value) return
  logLoading.value = true
  try {
    await clearPluginLog(logPlugin.value.id)
    pluginLog.value = ''
    ElMessage.success(t('pluginMarketplace.messages.logCleared'))
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.logFailed', { error: String(error) }))
  } finally {
    logLoading.value = false
  }
}

const openPluginSettings = () => {
  if (selectedSettings.value.includes('ssmt.dlss5.settings') || selectedSettings.value.includes('ssmt.player-tweaks.settings')) {
    detailTab.value = 'features'
    return
  }
  if (selectedSettings.value.includes('ssmt.reshade.settings')) {
    detailTab.value = 'dependencies'
  }
}

const dependencyStatusLabel = (status: DependencyStatus) => t(`pluginMarketplace.dependencyStatus.${status}`)
const dependencyStatusClass = (status: DependencyStatus) => `is-${status}`

let initialized = false
watch(() => [selectedGameName.value, installedGameName.value, selectedPlugin.value?.manifest.id, selectedPlugin.value?.enabled], async () => {
  const requestId = ++graphicsConfigRequestId
  const gameName = selectedGameName.value
  graphicsTargetExe.value = ''
  graphicsConfigError.value = ''
  graphicsConfigLoading.value = false
  if (!gameName || installedGameName.value !== gameName || !selectedSettings.value.includes('ssmt.dlss5.settings') || !selectedPlugin.value?.enabled) return
  graphicsConfigLoading.value = true
  try {
    const config = await ResourceManager.loadGameConfig(gameName)
    if (requestId === graphicsConfigRequestId) graphicsTargetExe.value = config.targetExePath?.trim() || ''
  } catch (error) {
    if (requestId === graphicsConfigRequestId) graphicsConfigError.value = String(error)
  } finally {
    if (requestId === graphicsConfigRequestId) graphicsConfigLoading.value = false
  }
}, { immediate: true })
watch(selectedGameName, () => {
  if (initialized) void refreshInstalled()
})
watch(visibleEntries, (entries) => {
  if (!entries.some(entry => entry.id === selectedId.value)) selectedId.value = entries[0]?.id ?? ''
})
watch(selectedId, () => { detailTab.value = 'features' })

onMounted(async () => {
  initialized = true
  await refreshAll()
})
</script>

<template>
  <main class="plugin-marketplace">
    <details v-if="catalogError || localCatalogError" class="catalog-notice">
      <summary><InfoFilled aria-hidden="true" /><span>{{ t('pluginMarketplace.catalogUnavailable') }}</span><span class="notice-details">{{ t('pluginMarketplace.showDetails') }}</span></summary>
      <p v-if="catalogError">{{ t('pluginMarketplace.messages.catalogFailed', { error: catalogError }) }}</p>
      <p v-if="localCatalogError">{{ t('pluginMarketplace.messages.localCatalogFailed', { error: localCatalogError }) }}</p>
    </details>

    <div class="marketplace-layout">
      <aside class="marketplace-sidebar" :aria-label="t('pluginMarketplace.title')">
        <div class="sidebar-search">
          <el-input v-model="searchQuery" clearable :placeholder="t('pluginMarketplace.searchPlaceholder')"
            :aria-label="t('pluginMarketplace.searchPlaceholder')" @keydown.esc="searchQuery = ''">
            <template #prefix><Search class="search-icon" /></template>
          </el-input>
          <el-button class="search-action" :icon="Refresh" :loading="refreshing"
            :title="t('pluginMarketplace.actions.refresh')" :aria-label="t('pluginMarketplace.actions.refresh')"
            @click="refreshAll" />
          <el-button class="search-action" :icon="FolderOpened" :disabled="loading"
            :title="t('pluginMarketplace.actions.installFromFile')" :aria-label="t('pluginMarketplace.actions.installFromFile')"
            @click="installFromFile" />
        </div>
        <div class="sidebar-tabs" role="group" :aria-label="t('pluginMarketplace.browsePlugins')">
          <button v-for="tab in (['discover', 'installed', 'updates'] as const)" :key="tab" type="button"
            :class="{ active: activeTab === tab }" :aria-pressed="activeTab === tab"
            @click="activeTab = tab">
            {{ t('pluginMarketplace.tabs.' + tab) }}
            <span :class="{ 'has-updates': tab === 'updates' && updateEntries.length > 0 }">{{ countFor(tab) }}</span>
          </button>
        </div>
        <div class="list-caption">{{ t('pluginMarketplace.resultsCount', { count: visibleEntries.length }) }}</div>
        <div class="marketplace-list" :aria-busy="refreshing">
          <button v-for="entry in visibleEntries" :key="entry.id" type="button" class="plugin-row"
            :class="{ selected: selectedId === entry.id }" :aria-pressed="selectedId === entry.id"
            @click="selectedId = entry.id">
            <span class="plugin-mark" aria-hidden="true">{{ monogramFor(entry) }}</span>
            <span class="plugin-row-content">
              <span class="plugin-row-title">
                <strong :title="entry.name">{{ entry.name }}</strong>
              </span>
              <span class="plugin-row-description">{{ entry.description }}</span>
              <span class="plugin-row-footer">
                <span class="source-badge" :class="'source-' + sourceKindFor(entry)">{{ t('pluginMarketplace.sourceBadges.' + sourceKindFor(entry)) }}</span>
                <span class="plugin-row-version" v-if="!entry.bundled">v{{ entry.version }}</span>
                <span v-if="rowLifecycleFor(entry)" class="row-state"
                  :class="{ enabled: rowLifecycleFor(entry) === 'enabled', issue: ['broken', 'incompatible', 'external_dependency_missing'].includes(rowLifecycleFor(entry) ?? '') }">
                  {{ t('pluginMarketplace.lifecycle.' + rowLifecycleFor(entry)) }}
                </span>
              </span>
            </span>
          </button>
          <div v-if="visibleEntries.length === 0 && refreshing" class="list-loading" role="status">
            <span>{{ t('pluginMarketplace.loadingPlugins') }}</span>
            <div v-for="row in 3" :key="row" class="loading-row" aria-hidden="true" />
          </div>
          <div v-else-if="visibleEntries.length === 0" class="empty-state" role="status">
            <Search v-if="searchQuery.trim()" aria-hidden="true" /><Box v-else aria-hidden="true" />
            <p>{{ searchQuery.trim() ? t('pluginMarketplace.emptySearch') : t('pluginMarketplace.empty.' + activeTab) }}</p>
            <el-button v-if="searchQuery.trim()" size="small" @click="searchQuery = ''">{{ t('pluginMarketplace.actions.clearSearch') }}</el-button>
          </div>
        </div>
      </aside>

      <section v-if="selectedEntry" class="plugin-detail" :aria-label="selectedEntry.name">
        <header class="detail-top">
          <span class="plugin-mark large" aria-hidden="true">{{ monogramFor(selectedEntry) }}</span>
          <div class="detail-identity">
            <h2>{{ selectedEntry.name }}</h2>
            <div class="detail-byline">
              <span>{{ selectedEntry.author }}</span>
              <span class="source-badge" :class="'source-' + sourceKindFor(selectedEntry)">{{ t('pluginMarketplace.sourceBadges.' + sourceKindFor(selectedEntry)) }}</span>
              <span v-if="!selectedEntry.bundled">v{{ selectedPlugin?.manifest.version ?? selectedEntry.version }}</span>
            </div>
          </div>
          <span v-if="selectedPlugin" class="detail-state" :class="{ 'is-enabled': selectedPlugin.enabled }">
            <span class="state-dot" aria-hidden="true" />{{ t('pluginMarketplace.status.' + (selectedPlugin.enabled ? 'enabled' : 'disabled')) }}
          </span>
        </header>
        <p class="detail-description">{{ selectedEntry.description }}</p>
        <div class="detail-actions" :aria-busy="loading">
          <el-button v-if="selectedEntry.localArchive && (!selectedPlugin || compareVersions(selectedEntry.version, selectedPlugin.manifest.version) > 0)"
            type="primary" size="small" :loading="loading" @click="installLocal(selectedEntry)">
            {{ t('pluginMarketplace.actions.installLocal') }}
          </el-button>
          <el-button v-else-if="!selectedPlugin && selectedEntry.official" type="primary" size="small"
            :loading="loading" @click="installOfficial(selectedEntry)">
            {{ t('pluginMarketplace.actions.installOfficial') }}
          </el-button>
          <el-button v-else-if="!selectedPlugin" type="primary" size="small" :icon="FolderOpened"
            :loading="loading" @click="installFromFile">{{ t('pluginMarketplace.actions.installPackage') }}</el-button>
          <template v-else>
            <el-button v-if="selectedEntry.official && compareVersions(selectedEntry.version, selectedPlugin.manifest.version) > 0"
              size="small" type="primary" :loading="loading" @click="installOfficial(selectedEntry)">
              {{ t('pluginMarketplace.actions.updateOfficial') }}
            </el-button>
            <el-button size="small" :type="selectedPlugin.enabled ? undefined : 'primary'"
              :disabled="(!selectedGameName && !selectedPlugin.appScoped) || (!selectedPlugin.appScoped && installedGameName !== selectedGameName) || loading || (!selectedPlugin.enabled && selectedPlugin.lifecycleStatus === 'incompatible')" :loading="loading" @click="togglePlugin(selectedPlugin)">
              {{ selectedPlugin.enabled ? t('pluginMarketplace.actions.disable') : t('pluginMarketplace.actions.enable') }}
            </el-button>
            <el-button v-if="selectedSettings.some(Boolean)" size="small" :icon="Setting"
              @click="openPluginSettings">{{ t('pluginMarketplace.actions.openSettings') }}</el-button>
            <el-button size="small" :icon="Document" :loading="logLoading" @click="openPluginLog">
              {{ t('pluginMarketplace.actions.viewLog') }}
            </el-button>
            <span v-if="['broken', 'incompatible', 'external_dependency_missing', 'update_available'].includes(lifecycleFor(selectedEntry, selectedPlugin))" class="detail-issue">
              {{ t('pluginMarketplace.lifecycle.' + lifecycleFor(selectedEntry, selectedPlugin)) }}
            </span>
          </template>
        </div>

        <div class="detail-tabs" role="group" :aria-label="t('pluginMarketplace.pluginContent')">
          <button type="button" :aria-pressed="detailTab === 'features'"
            :class="{ active: detailTab === 'features' }" @click="detailTab = 'features'">
            {{ t('pluginMarketplace.detailTabs.features') }}
          </button>
          <button type="button" :aria-pressed="detailTab === 'dependencies'"
            :class="{ active: detailTab === 'dependencies' }" @click="detailTab = 'dependencies'">
            {{ t('pluginMarketplace.detailTabs.dependencies') }}
          </button>
          <button type="button" :aria-pressed="detailTab === 'permissions'"
            :class="{ active: detailTab === 'permissions' }" @click="detailTab = 'permissions'">
            {{ t('pluginMarketplace.detailTabs.permissions') }}
          </button>
          <button type="button" :aria-pressed="detailTab === 'details'"
            :class="{ active: detailTab === 'details' }" @click="detailTab = 'details'">
            {{ t('pluginMarketplace.detailTabs.details') }}
          </button>
        </div>

        <div class="detail-body">
          <div class="detail-main">
            <section v-if="detailTab === 'features'" class="detail-section">
              <template v-if="selectedSettings.includes('ssmt.dlss5.settings') && selectedPlugin">
                <p class="section-hint">{{ t('pluginMarketplace.dlss5SettingsHint') }}</p>
                <div v-if="!selectedGameName" class="muted-copy">{{ t('pluginMarketplace.dlss5SelectGame') }}</div>
                <div v-else-if="graphicsConfigLoading || installedGameName !== selectedGameName" class="muted-copy">{{ t('pluginMarketplace.dlss5LoadingGame') }}</div>
                <div v-else-if="!selectedPlugin.enabled" class="muted-copy">{{ t('pluginMarketplace.dlss5EnableFirst') }}</div>
                <div v-else-if="graphicsConfigError" class="muted-copy">{{ graphicsConfigError }}</div>
                <Dlss5GraphicsSettings v-else-if="graphicsTargetExe"
                  :game-name="selectedGameName" :target-exe-path="graphicsTargetExe" />
                <div v-else class="muted-copy">{{ t('pluginMarketplace.dlss5ConfigureTarget') }}</div>
              </template>
              <template v-else-if="selectedSettings.includes('ssmt.player-tweaks.settings') && selectedPlugin && installedGameName === selectedGameName && selectedPlugin.lifecycleStatus !== 'incompatible'">
                <PlayerTweaksCameraSettings
                  v-if="selectedPlugin.features?.includes('camera')"
                  :game-name="selectedGameName"
                  :enabled="selectedPlugin.enabled"
                />
                <PlayerTweaksGameplaySettings
                  :game-name="selectedGameName"
                  :enabled="selectedPlugin.enabled"
                  :registry-fps="selectedPlugin.features?.includes('fps-120') || false"
                />
              </template>
              <template v-else>
                <div class="muted-copy">{{ t('pluginMarketplace.noFeatures') }}</div>
              </template>
            </section>
            <section v-else-if="detailTab === 'dependencies'" class="detail-section">
              <h3>{{ t('pluginMarketplace.sections.externalDependencies') }}</h3>
              <p class="section-hint">{{ t('pluginMarketplace.dependencyHint') }}</p>
              <div v-if="selectedSettings.includes('ssmt.dlss5.settings')" class="dependency-row dependency-managed">
                <strong>{{ t('pluginMarketplace.dependencyAcquisition.hotComponents') }}</strong>
                <p class="section-hint">{{ t('pluginMarketplace.dependencyAcquisition.hotHint') }}</p>
                <el-button type="primary" size="small" :disabled="!dependencyActionsEnabled" @click="openDependencySettings">
                  {{ t('pluginMarketplace.dependencyAcquisition.configureDownload') }}
                </el-button>
              </div>
              <p v-if="!dependencyActionsEnabled && (selectedDependencies.length || selectedSettings.includes('ssmt.dlss5.settings'))" class="section-hint">
                {{ t('pluginMarketplace.dependencyAcquisition.enableFirst') }}
              </p>
              <div v-if="selectedDependencies.length === 0 && !selectedSettings.includes('ssmt.dlss5.settings')" class="muted-copy">
                {{ t('pluginMarketplace.none') }}
              </div>
              <div v-for="dependency in selectedDependencies" :key="dependency.id" class="dependency-row">
                <div class="dependency-heading">
                  <strong>{{ dependency.id }}</strong>
                  <span v-if="optionalDependency(dependency)" class="dependency-optional">
                    {{ t('pluginMarketplace.dependencyAcquisition.optional') }}
                  </span>
                  <span v-if="selectedPlugin?.externalDependencies[dependency.id]" class="dependency-status"
                    :class="dependencyStatusClass(selectedPlugin.externalDependencies[dependency.id].status)">
                    {{ dependencyStatusLabel(selectedPlugin.externalDependencies[dependency.id].status) }}
                  </span>
                </div>
                <div class="dependency-files">{{ dependency.requiredFiles.join(' · ') }}</div>
                <div v-if="selectedPlugin?.externalDependencies[dependency.id]?.path" class="dependency-path">
                  {{ selectedPlugin.externalDependencies[dependency.id].path }}
                </div>
                <p v-if="selectedPlugin?.externalDependencies[dependency.id]?.reason" class="dependency-reason">
                  {{ selectedPlugin.externalDependencies[dependency.id].reason }}
                </p>
                <p class="section-hint">{{ dependency.downloadHint || (!dependencyDownloadSource(dependency) ? t('pluginMarketplace.dependencyAcquisition.unavailable') : '') }}</p>
                <div class="dependency-actions">
                  <el-button v-if="dependencyDownloadSource(dependency)" size="small" :disabled="!dependencyActionsEnabled"
                    @click="openDependencyDownload(dependency.id)">
                    {{ t('pluginMarketplace.dependencyAcquisition.downloadPage') }}
                  </el-button>
                  <el-button v-if="selectedPlugin" size="small" :disabled="loading"
                    @click="configureDependency(selectedEntry.id, dependency.id)">
                    {{ t('pluginMarketplace.actions.chooseDependencyPath') }}
                  </el-button>
                  <el-button v-if="selectedPlugin?.externalDependencies[dependency.id]?.path" size="small"
                    :disabled="!dependencyActionsEnabled || selectedPlugin.externalDependencies[dependency.id].status !== 'ready'"
                    @click="openDependencyDirectory(dependency.id)">
                    {{ t('pluginMarketplace.dependencyAcquisition.openFolder') }}
                  </el-button>
                  <span v-if="dependencyDownloadSource(dependency)" class="dependency-source">
                    {{ t('pluginMarketplace.dependencyAcquisition.source', { host: dependencyDownloadSource(dependency) }) }}
                  </span>
                </div>
              </div>
            </section>
            <section v-else-if="detailTab === 'permissions'" class="detail-section">
              <h3>{{ t('pluginMarketplace.sections.permissions') }}</h3>
              <p class="section-hint">{{ t('pluginMarketplace.permissionHint') }}</p>
              <div class="permission-list">
                <span v-for="permission in selectedEntry.permissions" :key="permission" class="permission-row">
                  {{ permission }}
                </span>
                <span v-if="selectedEntry.permissions.length === 0" class="muted-copy">{{ t('pluginMarketplace.none') }}</span>
              </div>
            </section>
            <section v-else class="detail-section detail-meta">
              <h3>{{ t('pluginMarketplace.metadata') }}</h3>
              <dl>
                <dt>ID</dt><dd>{{ selectedEntry.id }}</dd>
                <dt>{{ t('pluginMarketplace.version') }}</dt><dd>{{ selectedEntry.bundled ? t('pluginMarketplace.bundled') : (selectedPlugin?.manifest.version ?? selectedEntry.version) }}</dd>
                <dt>{{ t('pluginMarketplace.author') }}</dt><dd>{{ selectedEntry.author }}</dd>
                <dt>{{ t('pluginMarketplace.source') }}</dt><dd>{{ sourceLabelFor(selectedEntry) }}</dd>
                <dt>{{ t('pluginMarketplace.currentGame') }}</dt><dd>{{ selectedPlugin?.appScoped ? t('pluginMarketplace.appScope') : (selectedGameName || '—') }}</dd>
              </dl>
              <h3>{{ t('pluginMarketplace.sections.supportedGames') }}</h3>
              <div class="tag-list">
                <span v-for="game in selectedEntry.supportedGames" :key="game" class="detail-tag">{{ game }}</span>
                <span v-if="selectedEntry.supportedGames.length === 0" class="muted-copy">
                  {{ t('pluginMarketplace.gameCompatibilityUnspecified') }}
                </span>
              </div>
            </section>
          </div>
        </div>
      </section>
      <section v-else class="plugin-detail detail-empty">
        <div><Box aria-hidden="true" /><h2>{{ t('pluginMarketplace.title') }}</h2><p>{{ t('pluginMarketplace.selectPlugin') }}</p></div>
      </section>
    </div>

    <el-dialog v-model="logDialogOpen" :title="t('pluginMarketplace.logTitle', { name: logPlugin?.name ?? '' })"
      width="min(760px, calc(100vw - 36px))">
      <pre class="plugin-log-view">{{ pluginLog || t('pluginMarketplace.logEmpty') }}</pre>
      <template #footer>
        <el-button :icon="Delete" :loading="logLoading" @click="clearSelectedPluginLog">{{ t('pluginMarketplace.actions.clearLog') }}</el-button>
        <el-button @click="logDialogOpen = false">{{ t('pluginMarketplace.actions.close') }}</el-button>
      </template>
    </el-dialog>
    <el-dialog v-model="reviewOpen" :title="t('pluginMarketplace.review.title')" width="min(640px, calc(100vw - 36px))">
      <template v-if="reviewInspection">
        <p><strong>{{ reviewInspection.manifest.name }}</strong> · {{ reviewInspection.manifest.author }} · v{{ reviewInspection.manifest.version }}</p>
        <p class="review-detail">{{ reviewInspection.manifest.id }} · {{ reviewInspection.fileCount }} files · SHA-256: {{ reviewInspection.sha256 }}</p>
        <p class="review-detail">{{ t('pluginMarketplace.review.source') }} {{ reviewArchive }}</p>
        <p class="review-detail">{{ t('pluginMarketplace.review.permissions') }} {{ reviewInspection.manifest.permissions.join(', ') || t('pluginMarketplace.none') }}</p>
        <p class="review-disclaimer">{{ t('pluginMarketplace.review.disclaimer') }}</p>
        <el-checkbox v-model="reviewAccepted">{{ t('pluginMarketplace.review.accept') }}</el-checkbox>
      </template>
      <template #footer>
        <el-button @click="reviewOpen = false">{{ t('pluginMarketplace.review.cancel') }}</el-button>
        <el-button type="primary" :disabled="!reviewAccepted" :loading="loading" @click="installReviewed">
          {{ t('pluginMarketplace.actions.installPackage') }}
        </el-button>
      </template>
    </el-dialog>
  </main>
</template>

<style scoped>
.plugin-marketplace {
  --market-line: rgba(var(--theme-surface-tint-rgb), .10);
  --market-muted: rgba(var(--theme-text-secondary-rgb), .82);
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 100%;
  box-sizing: border-box;
  padding: var(--t-page-padding);
  color: var(--t-page-text);
}
.detail-top, .detail-actions, .dependency-heading {
  display: flex;
  align-items: center;
}
.catalog-notice { border: 1px solid var(--t-warning-border); border-radius: 8px; background: var(--t-warning-bg); font-size: 12px; line-height: 1.6; }
.catalog-notice summary { display: flex; align-items: center; gap: 9px; padding: 10px 14px; cursor: pointer; list-style: none; color: var(--t-warning-text); }
.catalog-notice summary::-webkit-details-marker { display: none; }
.catalog-notice summary > svg { flex: none; width: 16px; height: 16px; }
.notice-details { margin-left: auto; white-space: nowrap; color: var(--market-muted); font-size: 11px; }
.catalog-notice p { padding: 0 14px 12px 39px; margin: 0; overflow-wrap: anywhere; color: var(--market-muted); }
.review-detail { font: 11px/1.5 ui-monospace, Consolas, monospace; overflow-wrap: anywhere; }
.review-disclaimer { padding: 12px; border: var(--t-page-panel-border); border-radius: 6px; line-height: 1.6; }
.section-hint, .muted-copy {
  color: var(--market-muted);
  font-size: 12px;
  line-height: 1.7;
}
.marketplace-layout { display: grid; grid-template-columns: minmax(270px, 310px) minmax(0, 1fr); gap: 16px; flex: 1; align-items: start; min-height: 500px; }
.marketplace-sidebar, .plugin-detail {
  min-width: 0;
  border: var(--t-page-panel-border);
  border-radius: var(--t-page-panel-radius);
  background: var(--t-page-panel-bg);
  box-shadow: var(--t-page-panel-shadow);
  backdrop-filter: var(--t-page-panel-blur);
  -webkit-backdrop-filter: var(--t-page-panel-blur);
}
.marketplace-sidebar { display: flex; flex-direction: column; overflow: hidden; position: sticky; top: 12px; max-height: calc(100vh - 72px); }
.sidebar-search { display: flex; align-items: center; gap: 6px; padding: 14px 14px 10px; }
.sidebar-search .el-input { flex: 1; min-width: 0; }
.sidebar-search .search-action { flex: none; width: 32px; height: 32px; margin: 0; padding: 0; }
.search-icon { width: 15px; height: 15px; }
.sidebar-tabs { display: flex; gap: 3px; padding: 4px; margin: 0 14px; border: 1px solid var(--market-line); border-radius: 8px; background: var(--t-surface-subtle); }
.sidebar-tabs button, .detail-tabs button {
  border: 0; background: transparent;
  color: var(--market-muted); font: inherit; font-size: 12px; cursor: pointer;
  transition: background .15s, color .15s;
}
.sidebar-tabs button { flex: 1; min-width: 0; padding: 7px 0; border-radius: 5px; white-space: nowrap; }
.sidebar-tabs button span { margin-left: 4px; font-size: 10px; color: var(--t-page-text-muted); font-variant-numeric: tabular-nums; }
.sidebar-tabs button.active { background: var(--t-surface-raised); color: var(--t-page-text); box-shadow: 0 1px 3px #0002; }
.sidebar-tabs .has-updates { color: var(--t-warning-text); }
.sidebar-tabs button:hover, .detail-tabs button:hover { color: var(--t-page-text); }
.list-caption { padding: 14px 17px 6px; color: var(--t-page-text-muted); font-size: 11px; }
.marketplace-list { flex: 1; overflow-y: auto; overscroll-behavior: contain; padding: 0 7px 7px; }
.plugin-row { display: flex; width: 100%; gap: 11px; padding: 13px 10px; margin: 3px 0; border: 1px solid transparent; border-radius: 8px; background: transparent; color: inherit; text-align: left; cursor: pointer; transition: background .15s, border-color .15s; }
.plugin-row:hover { background: var(--t-surface-hover); }
.plugin-row.selected { border-color: rgba(var(--theme-surface-tint-rgb), .22); background: var(--t-surface-raised); }
.plugin-mark { display: grid; place-items: center; flex: 0 0 auto; width: 37px; height: 37px; border: 1px solid var(--market-line); border-radius: 10px; background: linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), .12), rgba(var(--theme-surface-tint-rgb), .03)); color: var(--t-page-text); font-weight: 650; font-size: 13px; letter-spacing: .5px; }
.plugin-mark.large { width: 56px; height: 56px; border-radius: 14px; font-size: 20px; }
.plugin-row-content { display: block; min-width: 0; flex: 1; }
.plugin-row-title strong { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 600; line-height: 1.5; }
.plugin-row-description { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; overflow: hidden; margin-top: 5px; font-size: 11px; line-height: 1.7; color: var(--market-muted); }
.plugin-row-footer { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; margin-top: 9px; font-size: 10px; }
.plugin-row-version { color: var(--t-page-text-muted); font-variant-numeric: tabular-nums; }
.source-badge { display: inline-flex; align-items: center; flex: none; padding: 1px 6px; border: 1px solid var(--market-line); border-radius: 4px; color: var(--market-muted); font-size: 10px; font-weight: 400; line-height: 1.7; }
.row-state { margin-left: auto; font-size: 10px; color: var(--market-muted); }
.row-state.enabled { color: var(--t-page-text); }
.row-state.issue, .detail-issue { color: var(--t-warning-text); }
.plugin-detail { padding: 24px; min-height: 500px; }
.detail-top { gap: 14px; flex-wrap: wrap; }
.detail-identity { min-width: 0; flex: 1 1 200px; }
.detail-identity h2 { margin: 0 0 9px; font-size: 21px; line-height: 1.3; font-weight: 650; overflow-wrap: anywhere; letter-spacing: -.3px; }
.detail-byline { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; color: var(--market-muted); font-size: 11px; }
.detail-description { margin: 14px 0 12px; font-size: 13px; line-height: 1.8; color: var(--market-muted); }
.detail-actions { gap: 8px; flex-wrap: wrap; min-height: 32px; margin-top: 12px; }
.detail-actions :deep(.el-button) { min-height: 30px; padding: 7px 12px; }
.detail-actions :deep(.el-button + .el-button) { margin-left: 0; }
.detail-state { display: inline-flex; align-items: center; gap: 6px; font-size: 11px; color: var(--market-muted); white-space: nowrap; }
.state-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--t-page-text-muted); }
.is-enabled .state-dot { background: var(--theme-success); box-shadow: 0 0 0 3px var(--t-success-bg); }
.detail-issue { font-size: 11px; }
.detail-tabs { display: flex; gap: 22px; margin-top: 18px; border-bottom: 1px solid var(--market-line); }
.detail-tabs button { border-bottom: 2px solid transparent; padding: 10px 0; white-space: nowrap; }
.detail-tabs button.active { border-bottom-color: var(--t-page-accent); color: var(--t-page-text); }
.detail-body { padding-top: 22px; }
.detail-main { min-width: 0; }
.detail-section { min-width: 0; }
.detail-section h3, .detail-meta h3 { margin: 0 0 8px; font-size: 13px; font-weight: 650; }
.section-hint { margin: 0 0 18px; }
.dependency-row { padding: 16px; margin: 12px 0; border: 1px solid var(--market-line); border-radius: 8px; background: var(--t-surface-subtle); }
.dependency-heading { justify-content: space-between; gap: 8px; flex-wrap: wrap; font-size: 12px; }
.dependency-heading strong { overflow-wrap: anywhere; min-width: 0; }
.dependency-files, .dependency-path { margin: 5px 0; color: rgba(var(--theme-text-secondary-rgb), .82); font-size: 11px; line-height: 1.4; overflow-wrap: anywhere; }
.dependency-files { margin: 9px 0; }
.dependency-path { font-family: ui-monospace, Consolas, monospace; padding: 8px 10px; background: var(--t-surface-soft); border-radius: 6px; }
.dependency-reason { margin: 8px 0; color: var(--t-warning-text); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
.dependency-status { flex: none; font-size: 10px; padding: 2px 7px; border-radius: 4px; background: var(--t-surface-raised); }
.dependency-status.is-ready { color: var(--t-success-text); }
.dependency-status.is-missing { color: var(--t-warning-text); }
.dependency-status.is-invalid { color: var(--t-danger-text); }
.dependency-heading .dependency-optional { margin-right: auto; color: var(--t-page-text-muted); font-size: 11px; }
.dependency-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.dependency-actions .el-button + .el-button { margin-left: 0; }
.dependency-source { color: var(--t-page-text-muted); font-size: 11px; overflow-wrap: anywhere; }
.dependency-managed { border-color: color-mix(in srgb, var(--theme-success) 45%, var(--market-line)); }
.detail-meta h3:not(:first-child) { margin-top: 22px; }
.detail-meta dl { display: grid; grid-template-columns: minmax(65px, max-content) minmax(0, 1fr); gap: 0 20px; margin: 0; font-size: 12px; line-height: 1.7; }
.detail-meta dt, .detail-meta dd { padding: 10px 0; border-bottom: 1px solid var(--market-line); }
.detail-meta dt { color: var(--market-muted); }
.detail-meta dd { margin: 0; overflow-wrap: anywhere; user-select: text; }
.tag-list, .permission-list { display: flex; flex-wrap: wrap; gap: 6px; }
.detail-tag, .permission-row { padding: 6px 10px; border: 1px solid var(--market-line); border-radius: 5px; font-size: 11px; }
.permission-row { font-family: ui-monospace, Consolas, monospace; overflow-wrap: anywhere; min-width: 0; }
.empty-state, .detail-empty { padding: 32px 18px; color: var(--market-muted); font-size: 12px; text-align: center; line-height: 1.7; }
.empty-state > svg, .detail-empty svg { width: 30px; height: 30px; opacity: .45; }
.detail-empty { display: grid; place-items: center; }
.detail-empty h2 { margin: 14px 0 6px; font-size: 16px; font-weight: 500; color: var(--t-page-text); }
.detail-empty p { max-width: 260px; margin: 0; }
.list-loading { padding: 16px 10px; color: var(--market-muted); font-size: 12px; }
.loading-row { height: 58px; border-radius: 7px; margin-top: 12px; background: linear-gradient(100deg, var(--t-surface-soft) 25%, var(--t-surface-hover) 50%, var(--t-surface-soft) 75%); background-size: 200% 100%; animation: market-loading 1.5s linear infinite; }
@keyframes market-loading { to { background-position: -200% 0; } }
.plugin-marketplace button:focus-visible, .catalog-notice summary:focus-visible { outline: 2px solid var(--t-page-accent); outline-offset: 3px; }
.plugin-log-view { max-height: 58vh; overflow: auto; margin: 0; padding: 12px; background: var(--t-surface-soft); color: var(--t-page-text); font: 11px/1.5 ui-monospace, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
@media (max-width: 1080px) {
  .marketplace-layout { grid-template-columns: minmax(250px, 280px) minmax(0, 1fr); }
  .plugin-detail { padding: 20px; }
  .detail-identity h2 { font-size: 19px; }
}
@media (max-width: 760px) {
  .marketplace-layout { grid-template-columns: minmax(0, 1fr); }
  .marketplace-sidebar { position: static; max-height: 340px; }
  .plugin-detail { min-height: 360px; }
  .detail-tabs { gap: 20px; }
}
@media (prefers-reduced-motion: reduce) {
  .plugin-marketplace * { animation: none; transition: none; }
}
</style>
