<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { ElMessage } from 'element-plus'
import { FolderOpened, Refresh, Search, Setting, Document, Delete } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { clearPluginLog, readPluginLog } from '../../plugin/logs'
import { AppStateManager } from '../../store/AppStateManager'
import { ResourceManager } from '../../store/ResourceManager'
import { getGamePresetPriority } from '../../store/GamePreset'

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

interface PluginManifest {
  id: string
  name: string
  version: string
  author: string
  description?: string
  compatibility?: { games?: string[] }
  permissions: string[]
  externalDependencies: Array<{ id: string; type: string; requiredFiles: string[] }>
}

interface InstalledPlugin {
  manifest: PluginManifest
  enabled: boolean
  bundled?: boolean
  lifecycleStatus: 'installed' | 'enabled' | 'disabled' | 'update_available' | 'broken' | 'incompatible' | 'external_dependency_missing'
  externalDependencies: Record<string, DependencyState>
  official: boolean
  appScoped: boolean
}

interface CatalogEntry {
  id: string
  name: string
  description: string
  author: string
  version: string
  supportedGames: string[]
  permissions: string[]
  externalDependencies: Array<{ id: string; requiredFiles: string[] }>
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
  externalDependencies: Array<{ id: string; requiredFiles: string[] }>
}

const { t } = useI18n()
const router = useRouter()
const gamesList = AppStateManager.gamesList
// 游戏选择器按预设优先级排序,重要游戏置顶。
const orderedGamesList = computed(() =>
  [...gamesList].sort((a, b) => getGamePresetPriority(a.name) - getGamePresetPriority(b.name)),
)
const selectedGameName = ref(AppStateManager.appSettings.CurrentGameName === 'Default'
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
const detailTab = ref<'features' | 'dependencies' | 'permissions'>('features')
const searchQuery = ref('')
const selectedId = ref('')
const installed = ref<InstalledPlugin[]>([])
const installedGameName = ref('')
const loading = ref(false)
let refreshRequestId = 0
const logDialogOpen = ref(false)
const pluginLog = ref('')
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
      description: plugin.bundled && plugin.manifest.id === 'ssmt.player-tweaks'
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
  }
}

const refreshAll = async () => {
  await Promise.all([refreshInstalled(), refreshCatalog(), refreshLocalCatalog()])
}

const refreshLocalCatalog = async () => {
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
  const gameName = selectedGameName.value
  const requestId = ++refreshRequestId
  loading.value = true
  try {
    let gameExecutable: string | null = null
    if (plugin.manifest.id === 'ssmt.dlss5.integration' && plugin.enabled) {
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
  if (!selectedEntry.value || !installedById.value.has(selectedEntry.value.id)) return
  logLoading.value = true
  try {
    pluginLog.value = await readPluginLog(selectedEntry.value.id)
    logDialogOpen.value = true
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.logFailed', { error: String(error) }))
  } finally {
    logLoading.value = false
  }
}

const clearSelectedPluginLog = async () => {
  if (!selectedEntry.value) return
  logLoading.value = true
  try {
    await clearPluginLog(selectedEntry.value.id)
    pluginLog.value = ''
    ElMessage.success(t('pluginMarketplace.messages.logCleared'))
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.logFailed', { error: String(error) }))
  } finally {
    logLoading.value = false
  }
}

const openPluginSettings = () => {
  if (selectedEntry.value?.id === 'ssmt.dlss5.integration') {
    detailTab.value = 'features'
    return
  }
  if (selectedEntry.value?.id === 'ssmt.hoyoshade.bridge') {
    void router.push({ name: 'Settings', hash: '#settings-plugins' })
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
  if (!gameName || installedGameName.value !== gameName || selectedPlugin.value?.manifest.id !== 'ssmt.dlss5.integration' || !selectedPlugin.value.enabled) return
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

onMounted(async () => {
  if (!gamesList.length) await AppStateManager.loadGames()
  if (!gamesList.some(game => game.name === selectedGameName.value)) {
    selectedGameName.value = orderedGamesList.value[0]?.name ?? ''
  }
  initialized = true
  await refreshAll()
})
</script>

<template>
  <main class="plugin-marketplace">
    <header class="marketplace-toolbar">
      <div class="toolbar-left">
        <label for="plugin-game-select">{{ t('pluginMarketplace.gameSelection') }}</label>
        <el-select id="plugin-game-select" v-model="selectedGameName" size="small"
          :placeholder="t('pluginMarketplace.chooseGame')" class="game-select">
          <el-option v-for="game in orderedGamesList" :key="game.name" :label="game.name" :value="game.name" />
        </el-select>
        <span class="scope-note">{{ selectedPlugin?.appScoped
          ? t('pluginMarketplace.appScopeHint')
          : selectedGameName ? t('pluginMarketplace.gameScopeHint', { game: selectedGameName })
          : t('pluginMarketplace.noGameHint') }}</span>
      </div>
      <div class="toolbar-actions">
        <el-button size="small" :icon="Refresh" :loading="loading" @click="refreshAll">
          {{ t('pluginMarketplace.actions.refresh') }}
        </el-button>
        <el-button size="small" :icon="FolderOpened" @click="installFromFile">
          {{ t('pluginMarketplace.actions.installFromFile') }}
        </el-button>
      </div>
    </header>

    <div v-if="catalogError" class="catalog-status">{{ t('pluginMarketplace.messages.catalogFailed', { error: catalogError }) }}</div>
    <div v-if="localCatalogError" class="catalog-status">{{ t('pluginMarketplace.messages.localCatalogFailed', { error: localCatalogError }) }}</div>

    <div class="marketplace-layout">
      <aside class="marketplace-sidebar" :aria-label="t('pluginMarketplace.title')">
        <div class="sidebar-search">
          <el-input v-model="searchQuery" size="small" clearable :placeholder="t('pluginMarketplace.searchPlaceholder')">
            <template #prefix><Search class="search-icon" /></template>
          </el-input>
        </div>
        <div class="sidebar-tabs" role="tablist">
          <button v-for="tab in (['discover', 'installed', 'updates'] as const)" :key="tab" type="button"
            :class="{ active: activeTab === tab }" role="tab" :aria-selected="activeTab === tab"
            @click="activeTab = tab">
            {{ t('pluginMarketplace.tabs.' + tab) }}
            <span v-if="tab === 'installed'">{{ installedEntries.length }}</span>
            <span v-else-if="tab === 'updates'">{{ updateEntries.length }}</span>
          </button>
        </div>
        <div class="marketplace-list" aria-live="polite">
          <button v-for="entry in visibleEntries" :key="entry.id" type="button" class="plugin-row"
            :class="{ selected: selectedId === entry.id }" :aria-pressed="selectedId === entry.id"
            @click="selectedId = entry.id">
            <span class="plugin-mark" aria-hidden="true">{{ entry.name.slice(0, 1) }}</span>
            <span class="plugin-row-content">
              <span class="plugin-row-title">
                <strong>{{ entry.name }}</strong>
                <span v-if="rowLifecycleFor(entry)" class="row-state"
                  :class="{ enabled: rowLifecycleFor(entry) === 'enabled', issue: ['broken', 'incompatible', 'external_dependency_missing'].includes(rowLifecycleFor(entry) ?? '') }">
                  {{ t('pluginMarketplace.lifecycle.' + rowLifecycleFor(entry)) }}
                </span>
              </span>
              <span class="plugin-row-description">{{ entry.description }}</span>
              <span class="plugin-row-author">{{ entry.author }} · {{ sourceLabelFor(entry) }} · {{ entry.bundled ? '' : `v${entry.version}` }}</span>
            </span>
          </button>
          <div v-if="visibleEntries.length === 0" class="empty-state">
            {{ searchQuery ? t('pluginMarketplace.emptySearch') : t('pluginMarketplace.empty.' + activeTab) }}
          </div>
        </div>
      </aside>

      <section v-if="selectedEntry" class="plugin-detail" :aria-label="selectedEntry.name">
        <div class="detail-top">
          <span class="plugin-mark large" aria-hidden="true">{{ selectedEntry.name.slice(0, 1) }}</span>
          <div class="detail-identity">
            <h2>{{ selectedEntry.name }}</h2>
            <p>{{ selectedEntry.author }} · {{ sourceLabelFor(selectedEntry) }} · {{ selectedEntry.bundled ? '' : `v${selectedEntry.version}` }}</p>
            <span class="detail-id">{{ selectedEntry.id }}</span>
          </div>
        </div>
        <p class="detail-description">{{ selectedEntry.description }}</p>
        <div class="detail-actions">
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
              :disabled="(!selectedGameName && !selectedPlugin.appScoped) || loading || selectedPlugin.lifecycleStatus === 'incompatible'" :loading="loading" @click="togglePlugin(selectedPlugin)">
              {{ selectedPlugin.enabled ? t('pluginMarketplace.actions.disable') : t('pluginMarketplace.actions.enable') }}
            </el-button>
            <el-button v-if="['ssmt.hoyoshade.bridge', 'ssmt.dlss5.integration'].includes(selectedEntry.id)" size="small" :icon="Setting"
              @click="openPluginSettings">{{ t('pluginMarketplace.actions.openSettings') }}</el-button>
            <el-button size="small" :icon="Document" :loading="logLoading" @click="openPluginLog">
              {{ t('pluginMarketplace.actions.viewLog') }}
            </el-button>
            <span class="detail-state">{{ t('pluginMarketplace.lifecycle.' + lifecycleFor(selectedEntry, selectedPlugin)) }}</span>
          </template>
        </div>

        <div class="detail-tabs" role="tablist">
          <button type="button" role="tab" :aria-selected="detailTab === 'features'"
            :class="{ active: detailTab === 'features' }" @click="detailTab = 'features'">
            {{ t('pluginMarketplace.detailTabs.features') }}
          </button>
          <button type="button" role="tab" :aria-selected="detailTab === 'dependencies'"
            :class="{ active: detailTab === 'dependencies' }" @click="detailTab = 'dependencies'">
            {{ t('pluginMarketplace.detailTabs.dependencies') }}
          </button>
          <button type="button" role="tab" :aria-selected="detailTab === 'permissions'"
            :class="{ active: detailTab === 'permissions' }" @click="detailTab = 'permissions'">
            {{ t('pluginMarketplace.detailTabs.permissions') }}
          </button>
        </div>

        <div class="detail-body">
          <div class="detail-main">
            <section v-if="detailTab === 'features'" class="detail-section">
              <template v-if="selectedEntry.id === 'ssmt.dlss5.integration' && selectedPlugin">
                <p class="section-hint">{{ t('pluginMarketplace.dlss5SettingsHint') }}</p>
                <div v-if="!selectedGameName" class="muted-copy">{{ t('pluginMarketplace.dlss5SelectGame') }}</div>
                <div v-else-if="graphicsConfigLoading || installedGameName !== selectedGameName" class="muted-copy">{{ t('pluginMarketplace.dlss5LoadingGame') }}</div>
                <div v-else-if="!selectedPlugin.enabled" class="muted-copy">{{ t('pluginMarketplace.dlss5EnableFirst') }}</div>
                <div v-else-if="graphicsConfigError" class="muted-copy">{{ graphicsConfigError }}</div>
                <Dlss5GraphicsSettings v-else-if="graphicsTargetExe"
                  :game-name="selectedGameName" :target-exe-path="graphicsTargetExe" />
                <div v-else class="muted-copy">{{ t('pluginMarketplace.dlss5ConfigureTarget') }}</div>
              </template>
              <template v-else-if="selectedEntry.id === 'ssmt.player-tweaks' && selectedPlugin && selectedPlugin.lifecycleStatus !== 'incompatible'">
                <PlayerTweaksCameraSettings
                  :game-name="selectedGameName"
                  :enabled="selectedPlugin.enabled"
                />
                <PlayerTweaksGameplaySettings
                  :game-name="selectedGameName"
                  :enabled="selectedPlugin.enabled"
                />
              </template>
              <template v-else>
                <div class="muted-copy">{{ t('pluginMarketplace.noFeatures') }}</div>
              </template>
            </section>
            <section v-else-if="detailTab === 'dependencies'" class="detail-section">
              <h3>{{ t('pluginMarketplace.sections.externalDependencies') }}</h3>
              <p class="section-hint">{{ t('pluginMarketplace.dependencyHint') }}</p>
              <div v-if="selectedEntry.externalDependencies.length === 0" class="muted-copy">
                {{ t('pluginMarketplace.none') }}
              </div>
              <div v-for="dependency in selectedEntry.externalDependencies" :key="dependency.id" class="dependency-row">
                <div class="dependency-heading">
                  <strong>{{ dependency.id }}</strong>
                  <span v-if="selectedPlugin?.externalDependencies[dependency.id]" class="dependency-status"
                    :class="dependencyStatusClass(selectedPlugin.externalDependencies[dependency.id].status)">
                    {{ dependencyStatusLabel(selectedPlugin.externalDependencies[dependency.id].status) }}
                  </span>
                </div>
                <div class="dependency-files">{{ dependency.requiredFiles.join(' · ') }}</div>
                <div v-if="selectedPlugin?.externalDependencies[dependency.id]?.path" class="dependency-path">
                  {{ selectedPlugin.externalDependencies[dependency.id].path }}
                </div>
                <el-button v-if="selectedPlugin" size="small" :disabled="loading"
                  @click="configureDependency(selectedEntry.id, dependency.id)">
                  {{ t('pluginMarketplace.actions.chooseDependencyPath') }}
                </el-button>
              </div>
            </section>
            <section v-else class="detail-section">
              <h3>{{ t('pluginMarketplace.sections.permissions') }}</h3>
              <p class="section-hint">{{ t('pluginMarketplace.permissionHint') }}</p>
              <div class="permission-list">
                <span v-for="permission in selectedEntry.permissions" :key="permission" class="permission-row">
                  {{ permission }}
                </span>
              </div>
            </section>
          </div>
          <aside class="detail-meta">
            <h3>{{ t('pluginMarketplace.metadata') }}</h3>
            <dl>
              <dt>ID</dt><dd>{{ selectedEntry.id }}</dd>
              <dt>{{ t('pluginMarketplace.version') }}</dt><dd>{{ selectedEntry.bundled ? t('pluginMarketplace.bundled') : (selectedPlugin?.manifest.version ?? selectedEntry.version) }}</dd>
              <dt>{{ t('pluginMarketplace.author') }}</dt><dd>{{ selectedEntry.author }}</dd>
              <dt>{{ t('pluginMarketplace.currentGame') }}</dt><dd>{{ selectedPlugin?.appScoped ? t('pluginMarketplace.appScope') : (selectedGameName || '—') }}</dd>
            </dl>
            <h3>{{ t('pluginMarketplace.sections.supportedGames') }}</h3>
            <div class="tag-list">
              <span v-for="game in selectedEntry.supportedGames" :key="game" class="detail-tag">{{ game }}</span>
              <span v-if="selectedEntry.supportedGames.length === 0" class="muted-copy">
                {{ t('pluginMarketplace.gameCompatibilityUnspecified') }}
              </span>
            </div>
          </aside>
        </div>
      </section>
      <section v-else class="plugin-detail detail-empty">
        {{ t('pluginMarketplace.selectPlugin') }}
      </section>
    </div>

    <el-dialog v-model="logDialogOpen" :title="t('pluginMarketplace.logTitle', { name: selectedEntry?.name ?? '' })"
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
  --market-line: rgba(var(--theme-surface-tint-rgb), .14);
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 100%;
  box-sizing: border-box;
  padding: var(--t-page-padding);
  color: var(--t-page-text);
}
.marketplace-toolbar, .toolbar-left, .toolbar-actions, .detail-top, .detail-actions, .dependency-heading {
  display: flex;
  align-items: center;
}
.marketplace-toolbar { justify-content: space-between; gap: 16px; min-height: 38px; }
.catalog-status { font-size: 12px; color: #e6b46b; }
.review-detail { font: 11px/1.5 ui-monospace, Consolas, monospace; overflow-wrap: anywhere; }
.review-disclaimer { padding: 12px; border: var(--t-page-panel-border); border-radius: 6px; line-height: 1.6; }
.toolbar-left { flex: 1 1 auto; min-width: 0; gap: 10px; }
.toolbar-left label { flex: none; font-size: 12px; font-weight: 650; }
.scope-note, .section-hint, .muted-copy {
  color: rgba(var(--theme-text-secondary-rgb), .78);
  font-size: 12px;
}
.toolbar-actions { flex: none; gap: 8px; }
.toolbar-actions :deep(.el-button + .el-button) { margin-left: 0; }
.game-select { width: min(220px, 32vw); }
.scope-note { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.marketplace-layout { display: grid; grid-template-columns: minmax(255px, 310px) minmax(0, 1fr); gap: 12px; flex: 1; min-height: 550px; }
.marketplace-sidebar, .plugin-detail {
  min-width: 0;
  border: var(--t-page-panel-border);
  border-radius: var(--t-page-panel-radius);
  background: var(--t-page-panel-bg);
  box-shadow: var(--t-page-panel-shadow);
  backdrop-filter: var(--t-page-panel-blur);
  -webkit-backdrop-filter: var(--t-page-panel-blur);
}
.marketplace-sidebar { display: flex; flex-direction: column; overflow: hidden; }
.sidebar-search { padding: 10px; border-bottom: 1px solid var(--market-line); }
.search-icon { width: 15px; height: 15px; }
.sidebar-tabs { display: flex; gap: 2px; padding: 4px 8px 0; border-bottom: 1px solid var(--market-line); }
.sidebar-tabs button, .detail-tabs button {
  border: 0; border-bottom: 2px solid transparent; background: transparent;
  color: rgba(var(--theme-text-secondary-rgb), .85); font: inherit; font-size: 12px; cursor: pointer;
}
.sidebar-tabs button { flex: 1; padding: 8px 3px; }
.sidebar-tabs button span { margin-left: 3px; font-size: 10px; opacity: .72; }
.sidebar-tabs button.active, .detail-tabs button.active { border-bottom-color: var(--t-page-accent); color: var(--t-page-text); }
.marketplace-list { flex: 1; overflow-y: auto; padding: 4px 0; }
.plugin-row { display: flex; width: 100%; gap: 10px; padding: 10px 12px; border: 0; border-left: 2px solid transparent; background: transparent; color: inherit; text-align: left; cursor: pointer; }
.plugin-row:hover { background: var(--t-surface-hover); }
.plugin-row.selected { border-left-color: var(--t-page-accent); background: var(--t-surface-raised); }
.plugin-mark { display: grid; place-items: center; flex: 0 0 auto; width: 36px; height: 36px; border-radius: 7px; background: var(--t-surface-raised); color: var(--t-page-accent); font-weight: 750; font-size: 16px; }
.plugin-mark.large { width: 60px; height: 60px; font-size: 27px; }
.plugin-row-content { display: block; min-width: 0; flex: 1; }
.plugin-row-title { display: flex; align-items: center; justify-content: space-between; gap: 5px; font-size: 13px; }
.plugin-row-title strong, .plugin-row-description, .plugin-row-author { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.plugin-row-title strong { min-width: 0; }
.plugin-row-description { margin-top: 2px; font-size: 11px; color: rgba(var(--theme-text-secondary-rgb), .9); }
.plugin-row-author { margin-top: 3px; font-size: 10px; color: rgba(var(--theme-text-secondary-rgb), .7); }
.row-state { flex: none; font-size: 10px; color: rgba(var(--theme-text-secondary-rgb), .85); }
.row-state.enabled { color: var(--t-page-accent); }
.row-state.issue { color: #e6b46b; }
.plugin-detail { overflow-y: auto; padding: 22px 24px; }
.detail-top { gap: 16px; }
.detail-identity { min-width: 0; }
.detail-identity h2 { margin: 0 0 4px; font-size: 22px; }
.detail-identity p, .detail-id { margin: 0; font-size: 12px; color: rgba(var(--theme-text-secondary-rgb), .85); }
.detail-id { display: block; margin-top: 5px; font-family: ui-monospace, Consolas, monospace; overflow-wrap: anywhere; }
.detail-description { margin: 14px 0; font-size: 13px; line-height: 1.5; }
.detail-actions { gap: 8px; flex-wrap: wrap; min-height: 32px; }
.detail-actions :deep(.el-button + .el-button) { margin-left: 0; }
.detail-state { font-size: 11px; color: rgba(var(--theme-text-secondary-rgb), .85); }
.detail-tabs { display: flex; gap: 20px; margin-top: 20px; border-bottom: 1px solid var(--market-line); }
.detail-tabs button { padding: 9px 3px; }
.detail-body { display: grid; grid-template-columns: minmax(0, 1fr) minmax(170px, 225px); gap: 24px; padding-top: 18px; }
.detail-section h3, .detail-meta h3 { margin: 0 0 8px; font-size: 13px; font-weight: 650; }
.section-hint { margin: 0 0 12px; line-height: 1.45; }
.dependency-row { padding: 11px 0; border-top: 1px solid var(--market-line); }
.dependency-heading { justify-content: space-between; gap: 8px; font-size: 12px; }
.dependency-files, .dependency-path { margin: 5px 0; color: rgba(var(--theme-text-secondary-rgb), .82); font-size: 11px; line-height: 1.4; overflow-wrap: anywhere; }
.dependency-path { font-family: ui-monospace, Consolas, monospace; }
.dependency-status { flex: none; font-size: 10px; }
.dependency-status.is-ready { color: var(--t-page-accent); }
.dependency-status.is-missing { color: #e6b46b; }
.dependency-status.is-invalid { color: #e18b83; }
.detail-meta { border-left: 1px solid var(--market-line); padding-left: 18px; }
.detail-meta h3:not(:first-child) { margin-top: 22px; }
.detail-meta dl { display: grid; grid-template-columns: 58px minmax(0, 1fr); gap: 8px; margin: 0; font-size: 11px; }
.detail-meta dt { color: rgba(var(--theme-text-secondary-rgb), .8); }
.detail-meta dd { margin: 0; overflow-wrap: anywhere; }
.tag-list, .permission-list { display: flex; flex-wrap: wrap; gap: 6px; }
.detail-tag, .permission-row { padding: 5px 7px; border: 1px solid var(--market-line); border-radius: 5px; font-size: 11px; }
.permission-row { font-family: ui-monospace, Consolas, monospace; }
.empty-state, .detail-empty { padding: 24px; color: rgba(var(--theme-text-secondary-rgb), .85); font-size: 12px; }
.detail-empty { display: grid; place-items: center; }
.plugin-log-view { max-height: 58vh; overflow: auto; margin: 0; padding: 12px; background: var(--t-surface-soft); color: var(--t-page-text); font: 11px/1.5 ui-monospace, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
@media (max-width: 900px) {
  .marketplace-layout { grid-template-columns: minmax(210px, 260px) minmax(0, 1fr); }
  .detail-body { grid-template-columns: minmax(0, 1fr); }
  .detail-meta { border-left: 0; border-top: 1px solid var(--market-line); padding: 16px 0 0; }
}
@media (max-width: 660px) {
  .marketplace-toolbar { align-items: flex-start; flex-direction: column; }
  .marketplace-layout { grid-template-columns: minmax(0, 1fr); }
  .marketplace-sidebar { max-height: 300px; }
  .toolbar-left { flex-wrap: wrap; }
  .scope-note { white-space: normal; }
}
</style>
