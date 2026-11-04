<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  nextTick,
  onMounted,
  ref,
  watch,
} from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { openPath, openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  Box,
  Delete,
  Document,
  FolderOpened,
  InfoFilled,
  Refresh,
  Search,
  Setting,
} from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { clearPluginLog, readPluginLog } from '../../plugin/logs'
import { notifyPluginRegistryChanged } from '../../plugin/pluginRegistryEvents'
import { LIQUID_GRAVITY_PROBE_PLUGIN_ID } from '../../plugin/liquidGravityProbe'
import {
  acceptLiquidGravityProbeConsent,
  declineLiquidGravityProbeConsent,
  hasLiquidGravityProbeConsent,
  setLiquidGravityProbeEnabled,
} from '../../plugin/liquidGravityProbeConsent'
import { AppStateManager } from '../../store/AppStateManager'
import { getGamePresetDisplayName } from '../../store/GamePreset'
import { ResourceManager } from '../../store/ResourceManager'
import PluginCompatibilityPanel from './PluginCompatibilityPanel.vue'
import PluginPermissionList from './PluginPermissionList.vue'
import LiquidGravityProbeDisclaimer from '../../components/LiquidGravityProbeDisclaimer.vue'
import {
  orderedPermissions,
  permissionDefinitionFor,
  type PluginCompatibilityItem,
  versionRequirementState,
} from './pluginMarketplaceModel'

const PlayerTweaksCameraSettings = defineAsyncComponent(
  () => import('../../plugin/components/PlayerTweaksCameraSettings.vue'),
)
const PlayerTweaksGameplaySettings = defineAsyncComponent(
  () => import('../../plugin/components/PlayerTweaksGameplaySettings.vue'),
)
const Dlss5GraphicsSettings = defineAsyncComponent(
  () => import('../../plugin/components/Dlss5GraphicsSettings.vue'),
)
const SecondaryMotionSettings = defineAsyncComponent(
  () => import('../../plugin/components/SecondaryMotionSettings.vue'),
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
  compatibility?: { ssmt?: string; platforms?: string[]; games?: string[] }
  permissions: string[]
  externalDependencies: ExternalDependency[]
  contributions?: {
    hostExtensions?: Array<{ id: string; settingsComponent?: string | null }>
    runtimePlugins?: Array<{ path: string }>
    injectedDlls?: Array<{ path: string }>
  }
}

interface InstalledPlugin {
  manifest: PluginManifest
  enabled: boolean
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
  supportedPlatforms?: string[]
  minimumSsmtVersion?: string | null
  maximumSsmtVersion?: string | null
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
  supportedPlatforms: string[]
  minimumSsmtVersion?: string | null
  maximumSsmtVersion?: string | null
  permissions: string[]
  externalDependencies: ExternalDependency[]
}

const { t } = useI18n()
const route = useRoute()
const selectedGameName = computed(() => AppStateManager.appSettings.CurrentGameName === 'Default'
  ? '' : AppStateManager.appSettings.CurrentGameName)
const selectedGameLabel = computed(() => selectedGameName.value
  ? getGamePresetDisplayName(selectedGameName.value, t)
  : t('pluginMarketplace.hero.noGame'))
const appVersion = ref('')
const compatibilityCheckedAt = ref('')
const isModernUi = computed(() => AppStateManager.appSettings.uiMode === 'modern')

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
const officialOnly = ref(false)
const compatibleOnly = ref(false)
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
const riskDialogOpen = ref(false)
const riskAccepted = ref(false)
const riskPlugin = ref<InstalledPlugin | null>(null)
const liquidDisclaimerOpen = ref(false)
const liquidDisclaimerBusy = ref(false)
const liquidDisclaimerPlugin = ref<InstalledPlugin | null>(null)
const NATIVE_INJECTION_RISK_ACK_KEY = 'native_injection_risk_ack_v1'
const settingsSectionRef = ref<HTMLElement | null>(null)
const settingsAttention = ref(false)

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
const selectedSettings = computed(() => selectedPlugin.value?.manifest.contributions?.hostExtensions?.map(extension => extension.settingsComponent) ?? [])
const selectedInjectedDlls = computed(() => selectedPlugin.value?.manifest.contributions?.injectedDlls ?? [])
const selectedRuntimePlugins = computed(() => selectedPlugin.value?.manifest.contributions?.runtimePlugins ?? [])
const selectedInjectionComponents = computed(() => [...selectedRuntimePlugins.value, ...selectedInjectedDlls.value])
const selectedPermissions = computed(() => orderedPermissions([
  ...(selectedPlugin.value?.manifest.permissions ?? []),
  ...(selectedEntry.value?.permissions ?? []),
]))
const originalProjectUrls: Record<string, string> = {
  'ssmt.secondary-motion': 'https://github.com/Sp1cHless/Arknights-Endfield-Plugin-Secondary-bodyphysics',
}
const selectedProjectUrl = computed(() => selectedEntry.value
  ? originalProjectUrls[selectedEntry.value.id] ?? ''
  : '')
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
      description: plugin.manifest.description || t('pluginMarketplace.installedPackage'),
      author: plugin.manifest.author,
      version: plugin.manifest.version,
      supportedGames: plugin.manifest.compatibility?.games ?? [],
      supportedPlatforms: plugin.manifest.compatibility?.platforms ?? [],
      minimumSsmtVersion: plugin.manifest.compatibility?.ssmt,
      permissions: plugin.manifest.permissions,
      externalDependencies: plugin.manifest.externalDependencies,
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
  : installedById.value.has(entry.id)
    ? t(installedById.value.get(entry.id)?.official ? 'pluginMarketplace.official' : 'pluginMarketplace.thirdParty')
    : t(entry.official ? 'pluginMarketplace.official' : 'pluginMarketplace.thirdParty')
const sourceKindFor = (entry: CatalogEntry) => entry.localArchive ? 'local'
  : (installedById.value.get(entry.id)?.official ?? entry.official) ? 'official' : 'thirdParty'
const monogramFor = (entry: CatalogEntry) => {
  const name = entry.name.replace(/^SSMT\s+/i, '').trim()
  const words = name.split(/[\s-]+/)
  return (words.length > 1 ? words.slice(0, 2).map(word => Array.from(word)[0]).join('') : Array.from(name).slice(0, 2).join('')).toLocaleUpperCase()
}
const entrySupportsGame = (entry: CatalogEntry) => {
  const plugin = installedById.value.get(entry.id)
  if (plugin?.appScoped) return true
  if (!selectedGameName.value) return false
  return entry.supportedGames.length === 0 || entry.supportedGames.includes(selectedGameName.value)
}
const entrySupportsPlatform = (entry: CatalogEntry) =>
  !entry.supportedPlatforms?.length || entry.supportedPlatforms.includes('windows-x64')
const entryHasBlockingLifecycle = (entry: CatalogEntry) => {
  const lifecycle = rowLifecycleFor(entry)
  return ['broken', 'incompatible', 'external_dependency_missing'].includes(lifecycle ?? '')
}
const entryIsCompatible = (entry: CatalogEntry) =>
  !entryHasBlockingLifecycle(entry) && entrySupportsGame(entry) && entrySupportsPlatform(entry)
const entryIsOfficial = (entry: CatalogEntry) =>
  installedById.value.get(entry.id)?.official ?? entry.official ?? false
const rowStateFor = (entry: CatalogEntry) => {
  const lifecycle = rowLifecycleFor(entry)
  if (lifecycle) return t(`pluginMarketplace.lifecycle.${lifecycle}`)
  if (!entrySupportsPlatform(entry)) return t('pluginMarketplace.rowStates.platformUnsupported')
  if (!entrySupportsGame(entry) && selectedGameName.value) return t('pluginMarketplace.rowStates.gameUnsupported')
  return t('pluginMarketplace.status.notInstalled')
}
const rowStateTone = (entry: CatalogEntry) => {
  const lifecycle = rowLifecycleFor(entry)
  if (['broken', 'incompatible', 'external_dependency_missing'].includes(lifecycle ?? '')) return 'issue'
  if (lifecycle === 'enabled') return 'enabled'
  if (!entrySupportsPlatform(entry) || (!entrySupportsGame(entry) && selectedGameName.value)) return 'issue'
  return ''
}
const listIsFiltered = computed(() => !!searchQuery.value.trim() || officialOnly.value || compatibleOnly.value)
const countFor = (tab: typeof activeTab.value) => tab === 'installed' ? installedEntries.value.length
  : tab === 'updates' ? updateEntries.value.length : allEntries.value.length
const visibleEntries = computed(() => {
  const entries = activeTab.value === 'installed' ? installedEntries.value
    : activeTab.value === 'updates' ? updateEntries.value : allEntries.value
  const query = searchQuery.value.trim().toLocaleLowerCase()
  return entries
    .filter(entry => !officialOnly.value || entryIsOfficial(entry))
    .filter(entry => !compatibleOnly.value || entryIsCompatible(entry))
    .filter(entry => !query || [entry.name, entry.id, entry.author, entry.description]
      .some(value => value.toLocaleLowerCase().includes(query)))
})
const selectedLifecycle = computed(() => selectedEntry.value && selectedPlugin.value
  ? lifecycleFor(selectedEntry.value, selectedPlugin.value)
  : undefined)
const selectedHasIssue = computed(() => ['broken', 'incompatible', 'external_dependency_missing', 'update_available']
  .includes(selectedLifecycle.value ?? ''))
const selectedIssueHint = computed(() => selectedLifecycle.value
  ? t(`pluginMarketplace.issueHints.${selectedLifecycle.value}`)
  : '')
const selectedDependencyIssue = computed(() => selectedPlugin.value
  ? Object.values(selectedPlugin.value.externalDependencies).find(item => item.status !== 'ready')?.reason ?? ''
  : '')
const selectedStatusLabel = computed(() => !selectedPlugin.value
  ? t('pluginMarketplace.status.notInstalled')
  : selectedPlugin.value.enabled
    ? t('pluginMarketplace.status.enabled')
    : t(`pluginMarketplace.lifecycle.${selectedLifecycle.value ?? 'disabled'}`))
const selectedStatusClass = computed(() => {
  if (!selectedPlugin.value) return 'is-uninstalled'
  if (selectedHasIssue.value) return 'is-issue'
  return selectedPlugin.value.enabled ? 'is-enabled' : 'is-disabled'
})
const selectedInjectionValue = computed(() => {
  if (selectedInjectionComponents.value.length) {
    return t('pluginMarketplace.impact.injectionCount', { count: selectedInjectionComponents.value.length })
  }
  if (!selectedPlugin.value && selectedEntry.value?.permissions.includes('native.inject')) {
    return t('pluginMarketplace.impact.injectionDeclared')
  }
  return t('pluginMarketplace.impact.injectionNone')
})
const selectedInjectionHint = computed(() => {
  if (selectedInjectionComponents.value.length) return t('pluginMarketplace.impact.injectionHint')
  if (!selectedPlugin.value && selectedEntry.value?.permissions.includes('native.inject')) {
    return t('pluginMarketplace.impact.injectionDeclaredHint')
  }
  return t('pluginMarketplace.impact.injectionNoneHint')
})
const selectedDependencyValue = computed(() => selectedDependencies.value.length
  ? t('pluginMarketplace.impact.dependencyCount', { count: selectedDependencies.value.length })
  : t('pluginMarketplace.impact.none'))
const dependencyStates = computed(() => selectedDependencies.value.map(dependency =>
  selectedPlugin.value?.externalDependencies[dependency.id]?.status))
const selectedRequiredDependencies = computed(() => selectedDependencies.value.filter(dependency => !optionalDependency(dependency)))
const selectedMissingRequiredDependencies = computed(() => selectedRequiredDependencies.value.filter(dependency => {
  const status = selectedPlugin.value?.externalDependencies[dependency.id]?.status
  return status && status !== 'ready'
}))
const selectedDependencyHint = computed(() => {
  if (!selectedDependencies.value.length) return t('pluginMarketplace.impact.noDependencies')
  if (!selectedPlugin.value) return t('pluginMarketplace.impact.dependencyUnverified')
  const ready = dependencyStates.value.filter(status => status === 'ready').length
  const missing = dependencyStates.value.filter(status => status === 'missing' || status === 'invalid').length
  if (selectedMissingRequiredDependencies.value.length) {
    return t('pluginMarketplace.impact.dependencyMissing', { count: selectedMissingRequiredDependencies.value.length })
  }
  if (missing) return t('pluginMarketplace.impact.dependencyOptionalMissing', { count: missing })
  return t('pluginMarketplace.impact.dependencyReady', { ready, total: selectedDependencies.value.length })
})
const selectedGameCompatible = computed(() => !!selectedEntry.value && entrySupportsGame(selectedEntry.value))
const selectedPlatformCompatible = computed(() => !!selectedEntry.value && entrySupportsPlatform(selectedEntry.value))
const selectedVersionCompatibility = computed(() => versionRequirementState(
  appVersion.value,
  selectedEntry.value?.minimumSsmtVersion,
  selectedEntry.value?.maximumSsmtVersion,
))
const selectedVersionRequirementText = computed(() => {
  const requirements = [
    selectedEntry.value?.minimumSsmtVersion,
    selectedEntry.value?.maximumSsmtVersion
      ? t('pluginMarketplace.compatibility.values.maximumVersion', {
        version: selectedEntry.value.maximumSsmtVersion,
      })
      : '',
  ].filter(Boolean)
  return requirements.length
    ? requirements.join(' · ')
    : t('pluginMarketplace.compatibility.values.unspecified')
})
const selectedCompatibilityStatus = computed<'ready' | 'limited' | 'blocked'>(() => {
  if (!selectedEntry.value) return 'limited'
  if (selectedHasIssue.value && selectedLifecycle.value !== 'update_available') return 'blocked'
  if (!selectedGameCompatible.value
    || !selectedPlatformCompatible.value
    || selectedVersionCompatibility.value === 'blocked') return 'blocked'
  if (!selectedPlugin.value || selectedLifecycle.value === 'update_available' || !selectedGameName.value) return 'limited'
  if (selectedMissingRequiredDependencies.value.length) return 'blocked'
  return 'ready'
})
const selectedCompatibilitySummary = computed(() => {
  if (!selectedEntry.value) return ''
  if (!selectedPlatformCompatible.value) return t('pluginMarketplace.compatibility.summary.unsupportedPlatform')
  if (selectedVersionCompatibility.value === 'blocked') return t('pluginMarketplace.compatibility.summary.versionMismatch', {
    version: appVersion.value || t('pluginMarketplace.compatibility.values.unknownVersion'),
    range: selectedVersionRequirementText.value,
  })
  if (selectedGameName.value && !selectedGameCompatible.value) return t('pluginMarketplace.compatibility.summary.unsupportedGame', {
    game: selectedGameLabel.value,
  })
  if (!selectedPlugin.value) return t('pluginMarketplace.compatibility.summary.notInstalled')
  if (!selectedPlugin.value.appScoped && !selectedGameName.value) return t('pluginMarketplace.compatibility.summary.noGame')
  if (selectedMissingRequiredDependencies.value.length) {
    return selectedDependencyIssue.value || t('pluginMarketplace.compatibility.summary.dependenciesMissing', {
      count: selectedMissingRequiredDependencies.value.length,
    })
  }
  if (selectedHasIssue.value) return selectedIssueHint.value
  return t('pluginMarketplace.compatibility.summary.ready')
})
const selectedCompatibilityItems = computed<PluginCompatibilityItem[]>(() => {
  const entry = selectedEntry.value
  if (!entry) return []

  const supportedGames = entry.supportedGames.map(game => getGamePresetDisplayName(game, t)).join(' · ')
  const supportedPlatforms = entry.supportedPlatforms?.join(' · ') || t('pluginMarketplace.compatibility.values.unspecified')
  const dependencyState = !selectedDependencies.value.length
    ? 'supported'
    : !selectedPlugin.value ? 'unknown'
      : selectedMissingRequiredDependencies.value.length ? 'blocked' : 'supported'
  const hasDeclaredInjection = selectedPermissions.value.includes('native.inject')
  const injectionState = selectedInjectionComponents.value.length
    ? 'supported'
    : hasDeclaredInjection ? 'limited' : 'supported'

  return [
    {
      id: 'game',
      label: t('pluginMarketplace.compatibility.rows.game'),
      value: selectedPlugin.value?.appScoped
        ? t('pluginMarketplace.appScope')
        : selectedGameLabel.value,
      hint: selectedPlugin.value?.appScoped
        ? t('pluginMarketplace.compatibility.values.allGames')
        : supportedGames || t('pluginMarketplace.compatibility.values.allGames'),
      state: selectedGameCompatible.value ? 'supported' : 'blocked',
    },
    {
      id: 'platform',
      label: t('pluginMarketplace.compatibility.rows.platform'),
      value: 'Windows · x64',
      hint: supportedPlatforms,
      state: selectedPlatformCompatible.value ? 'supported' : 'blocked',
    },
    {
      id: 'injection',
      label: t('pluginMarketplace.compatibility.rows.injection'),
      value: selectedInjectionValue.value,
      hint: selectedInjectionHint.value,
      state: injectionState,
    },
    {
      id: 'dependencies',
      label: t('pluginMarketplace.compatibility.rows.dependencies'),
      value: selectedDependencyValue.value,
      hint: selectedDependencyHint.value,
      state: dependencyState,
    },
    {
      id: 'settings',
      label: t('pluginMarketplace.compatibility.rows.settings'),
      value: selectedSettings.value.some(Boolean)
        ? t('pluginMarketplace.compatibility.values.settingsAvailable')
        : t('pluginMarketplace.compatibility.values.settingsUnavailable'),
      state: 'supported',
    },
    {
      id: 'ssmt-version',
      label: t('pluginMarketplace.compatibility.rows.version'),
      value: appVersion.value ? `SSMT ${appVersion.value}` : t('pluginMarketplace.compatibility.values.unspecified'),
      hint: selectedVersionRequirementText.value,
      state: selectedVersionCompatibility.value,
    },
  ]
})
const selectedEnableBlockedReason = computed(() => {
  if (!selectedPlugin.value) return ''
  if (selectedCompatibilityStatus.value === 'blocked') return selectedCompatibilitySummary.value
  if (!selectedPlugin.value.appScoped && !selectedGameName.value) return t('pluginMarketplace.compatibility.summary.noGame')
  if (!selectedPlugin.value.appScoped && installedGameName.value !== selectedGameName.value) {
    return t('pluginMarketplace.compatibility.summary.refreshRequired')
  }
  return ''
})
const selectedCanEnable = computed(() => !!selectedPlugin.value
  && !loading.value
  && (!selectedPlugin.value.enabled && selectedCompatibilityStatus.value !== 'blocked'
    || selectedPlugin.value.enabled)
  && (selectedPlugin.value.appScoped || !!selectedGameName.value)
  && (selectedPlugin.value.appScoped || installedGameName.value === selectedGameName.value))

const compareVersions = (left: string, right: string) => {
  const parse = (value: string) => value.split('.').map(part => Number.parseInt(part, 10) || 0)
  const a = parse(left)
  const b = parse(right)
  for (let index = 0; index < 3; index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) return (a[index] ?? 0) - (b[index] ?? 0)
  }
  return 0
}

const markCompatibilityChecked = () => {
  compatibilityCheckedAt.value = new Date().toLocaleString([], {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const refreshInstalled = async (): Promise<boolean> => {
  const requestId = ++refreshRequestId
  const gameName = selectedGameName.value
  let succeeded = false
  loading.value = true
  try {
    const snapshot = await invoke<InstalledPlugin[]>('plugin_registry_snapshot_for_game', {
      gameName,
    })
    if (requestId === refreshRequestId) {
      installed.value = snapshot
      installedGameName.value = gameName
      succeeded = true
      notifyPluginRegistryChanged()
    }
  } catch (error) {
    if (requestId === refreshRequestId) {
      console.error('Failed to load plugin registry:', error)
      ElMessage.error(t('pluginMarketplace.messages.loadFailed', { error: String(error) }))
    }
  } finally {
    if (requestId === refreshRequestId) loading.value = false
  }
  return succeeded
}

const refreshCatalog = async (): Promise<boolean> => {
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
      supportedPlatforms: entry.supportedPlatforms,
      minimumSsmtVersion: entry.minimumSsmtVersion,
      maximumSsmtVersion: entry.maximumSsmtVersion,
      permissions: entry.permissions,
      externalDependencies: entry.externalDependencies,
      official: true,
    }))
    catalogError.value = ''
    return true
  } catch (error) {
    catalogError.value = String(error)
    return false
  } finally {
    catalogLoading.value = false
  }
}

const refreshAll = async () => {
  if (refreshing.value) return
  const [installedReady] = await Promise.all([refreshInstalled(), refreshCatalog(), refreshLocalCatalog()])
  if (installedReady) markCompatibilityChecked()
}

const refreshLocalCatalog = async (): Promise<boolean> => {
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
      supportedPlatforms: inspection.manifest.compatibility?.platforms ?? [],
      minimumSsmtVersion: inspection.manifest.compatibility?.ssmt,
      permissions: inspection.manifest.permissions,
      externalDependencies: inspection.manifest.externalDependencies,
      localArchive: archivePath,
    }))
    localCatalogError.value = ''
    return true
  } catch (error) {
    localCatalogError.value = String(error)
    return false
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

const applyPluginToggle = async (plugin: InstalledPlugin) => {
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
      notifyPluginRegistryChanged()
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

const togglePlugin = async (plugin: InstalledPlugin) => {
  if (
    !plugin.enabled
    && plugin.manifest.id === LIQUID_GRAVITY_PROBE_PLUGIN_ID
    && !await hasLiquidGravityProbeConsent()
  ) {
    liquidDisclaimerPlugin.value = plugin
    liquidDisclaimerOpen.value = true
    return
  }
  if (!plugin.enabled && plugin.manifest.permissions.includes('native.inject')) {
    riskPlugin.value = plugin
    riskAccepted.value = false
    riskDialogOpen.value = true
    return
  }
  await applyPluginToggle(plugin)
}

const acceptLiquidDisclaimer = async () => {
  const plugin = liquidDisclaimerPlugin.value
  if (!plugin || liquidDisclaimerBusy.value) return
  liquidDisclaimerBusy.value = true
  try {
    await acceptLiquidGravityProbeConsent()
    liquidDisclaimerOpen.value = false
    await applyPluginToggle(plugin)
  } catch (error) {
    ElMessage.error(String(error))
  } finally {
    liquidDisclaimerBusy.value = false
    liquidDisclaimerPlugin.value = null
  }
}

const declineLiquidDisclaimer = async () => {
  if (liquidDisclaimerBusy.value) return
  liquidDisclaimerBusy.value = true
  try {
    await declineLiquidGravityProbeConsent()
    if (selectedGameName.value) {
      await setLiquidGravityProbeEnabled(selectedGameName.value, false)
      await refreshInstalled()
    }
    liquidDisclaimerOpen.value = false
    ElMessage.info('已停用 Liquid Gravity Probe')
  } catch (error) {
    ElMessage.error(String(error))
  } finally {
    liquidDisclaimerBusy.value = false
    liquidDisclaimerPlugin.value = null
  }
}

const uninstallPlugin = async () => {
  const plugin = selectedPlugin.value
  if (!plugin || loading.value) return
  if (plugin.enabled) {
    ElMessage.warning(t('pluginMarketplace.messages.uninstallDisabled'))
    return
  }
  try {
    await ElMessageBox.confirm(
      t('pluginMarketplace.messages.uninstallConfirm', { name: plugin.manifest.name }),
      t('pluginMarketplace.messages.uninstallTitle'),
      {
        type: 'warning',
        confirmButtonText: t('pluginMarketplace.actions.uninstall'),
        cancelButtonText: t('pluginMarketplace.actions.close'),
        distinguishCancelAndClose: true,
      },
    )
  } catch {
    return
  }
  const requestId = ++refreshRequestId
  loading.value = true
  try {
    const snapshot = await invoke<InstalledPlugin[]>('uninstall_plugin', {
      pluginId: plugin.manifest.id,
      gameName: selectedGameName.value || null,
    })
    if (requestId === refreshRequestId) {
      installed.value = snapshot
      installedGameName.value = selectedGameName.value
      notifyPluginRegistryChanged()
      if (logPlugin.value?.id === plugin.manifest.id) {
        logDialogOpen.value = false
        logPlugin.value = null
        pluginLog.value = ''
      }
    }
    ElMessage.success(t('pluginMarketplace.messages.uninstalled'))
  } catch (error) {
    console.error('Failed to uninstall plugin:', error)
    ElMessage.error(t('pluginMarketplace.messages.uninstallFailed', { error: String(error) }))
  } finally {
    if (requestId === refreshRequestId) loading.value = false
  }
}

const confirmRiskyEnable = async () => {
  if (!riskAccepted.value || !riskPlugin.value) return
  loading.value = true
  try {
    await invoke('set_plugin_setting', {
      pluginId: riskPlugin.value.manifest.id,
      key: NATIVE_INJECTION_RISK_ACK_KEY,
      value: {
        accepted: true,
        pluginVersion: riskPlugin.value.manifest.version,
        acceptedAt: new Date().toISOString(),
      },
    })
    const plugin = riskPlugin.value
    riskDialogOpen.value = false
    await applyPluginToggle(plugin)
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.risk.saveFailed', { error: String(error) }))
  } finally {
    loading.value = false
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

const openPluginSettings = async () => {
  if (selectedSettings.value.includes('ssmt.dlss5.settings')
    || selectedSettings.value.includes('ssmt.player-tweaks.settings')
    || selectedSettings.value.includes('ssmt.secondary-motion.settings')) {
    detailTab.value = 'features'
    settingsAttention.value = false
    await nextTick()
    settingsSectionRef.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    settingsAttention.value = true
    setTimeout(() => {
      settingsAttention.value = false
    }, 1200)
    return
  }
  if (selectedSettings.value.includes('ssmt.reshade.settings')) {
    detailTab.value = 'dependencies'
  }
}

const openOriginalProject = async () => {
  if (!selectedProjectUrl.value) return
  try {
    await openUrl(selectedProjectUrl.value)
  } catch (error) {
    ElMessage.error(t('pluginMarketplace.messages.projectLinkFailed', { error: String(error) }))
  }
}

const dependencyStatusLabel = (status: DependencyStatus) => t(`pluginMarketplace.dependencyStatus.${status}`)
const dependencyStatusClass = (status: DependencyStatus) => `is-${status}`

let initialized = false
// 目录和快照会替换对象；按实际身份与状态监听，避免相同内容触发设置组件卸载和重扫。
watch([
  selectedGameName, installedGameName,
  () => selectedPlugin.value?.manifest.id,
  () => selectedPlugin.value?.manifest.version,
  () => selectedPlugin.value?.enabled,
  () => selectedSettings.value.includes('ssmt.dlss5.settings'),
], async () => {
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
watch(selectedGameName, async () => {
  if (!initialized) return
  if (await refreshInstalled()) markCompatibilityChecked()
})
watch(visibleEntries, (entries) => {
  if (!entries.some(entry => entry.id === selectedId.value)) selectedId.value = entries[0]?.id ?? ''
})
watch(selectedId, () => { detailTab.value = 'features' })

onMounted(async () => {
  initialized = true
  try {
    appVersion.value = await getVersion()
  } catch (error) {
    console.warn('Failed to read app version for plugin marketplace', error)
  }
  await refreshAll()
})
</script>

<template>
  <main class="plugin-marketplace" :class="{ 'is-classic-ui': !isModernUi }">
    <section class="marketplace-shell">
      <header v-if="isModernUi" class="marketplace-heading">
        <div class="marketplace-heading-copy">
          <h1>{{ t('pluginMarketplace.title') }}</h1>
          <div class="marketplace-brandline">
            <span class="brandline-dot" aria-hidden="true" />
            <span>SSMT{{ appVersion ? ` ${appVersion}` : '' }}</span>
            <span aria-hidden="true">·</span>
            <span>{{ selectedGameLabel }}</span>
          </div>
        </div>
        <span class="marketplace-platform">Windows x64</span>
      </header>

      <div v-if="isModernUi" class="marketplace-toolbar">
        <el-input v-model="searchQuery" clearable :placeholder="t('pluginMarketplace.searchPlaceholder')"
          :aria-label="t('pluginMarketplace.searchPlaceholder')" class="marketplace-search"
          @keydown.esc="searchQuery = ''">
          <template #prefix><Search class="search-icon" /></template>
        </el-input>
        <el-button class="toolbar-action" :icon="Refresh" :loading="refreshing"
          :title="t('pluginMarketplace.actions.refresh')" :aria-label="t('pluginMarketplace.actions.refresh')"
          @click="refreshAll" />
        <el-button class="install-action" :icon="FolderOpened" :disabled="loading" @click="installFromFile">
          {{ t('pluginMarketplace.actions.installFromFile') }}
        </el-button>
      </div>

      <div v-if="isModernUi" class="marketplace-browsebar">
        <div class="sidebar-tabs" role="group" :aria-label="t('pluginMarketplace.browsePlugins')">
          <button v-for="tab in (['discover', 'installed', 'updates'] as const)" :key="tab" type="button"
            :class="{ active: activeTab === tab }" :aria-pressed="activeTab === tab"
            @click="activeTab = tab">
            {{ t('pluginMarketplace.tabs.' + tab) }}
            <span :class="{ 'has-updates': tab === 'updates' && updateEntries.length > 0 }">{{ countFor(tab) }}</span>
          </button>
        </div>
        <div class="marketplace-filters" role="group" :aria-label="t('pluginMarketplace.filters.title')">
          <button type="button" :class="{ active: officialOnly }" :aria-pressed="officialOnly"
            @click="officialOnly = !officialOnly">{{ t('pluginMarketplace.filters.official') }}</button>
          <button type="button" :class="{ active: compatibleOnly }" :aria-pressed="compatibleOnly"
            @click="compatibleOnly = !compatibleOnly">{{ t('pluginMarketplace.filters.compatible') }}</button>
        </div>
        <div class="list-caption">{{ t('pluginMarketplace.resultsCount', { count: visibleEntries.length }) }}</div>
      </div>

      <details v-if="catalogError || localCatalogError" class="catalog-notice">
        <summary><InfoFilled aria-hidden="true" /><span>{{ t('pluginMarketplace.catalogUnavailable') }}</span><span class="notice-details">{{ t('pluginMarketplace.showDetails') }}</span></summary>
        <p v-if="catalogError">{{ t('pluginMarketplace.messages.catalogFailed', { error: catalogError }) }}</p>
        <p v-if="localCatalogError">{{ t('pluginMarketplace.messages.localCatalogFailed', { error: localCatalogError }) }}</p>
      </details>

      <div class="marketplace-layout">
        <aside class="marketplace-sidebar" :aria-label="t('pluginMarketplace.title')">
        <div v-if="!isModernUi" class="legacy-sidebar-search">
          <el-input v-model="searchQuery" clearable :placeholder="t('pluginMarketplace.searchPlaceholder')"
            :aria-label="t('pluginMarketplace.searchPlaceholder')" @keydown.esc="searchQuery = ''">
            <template #prefix><Search class="search-icon" /></template>
          </el-input>
          <el-button :icon="Refresh" :loading="refreshing"
            :title="t('pluginMarketplace.actions.refresh')" :aria-label="t('pluginMarketplace.actions.refresh')"
            @click="refreshAll" />
          <el-button :icon="FolderOpened" :disabled="loading"
            :title="t('pluginMarketplace.actions.installFromFile')"
            :aria-label="t('pluginMarketplace.actions.installFromFile')"
            @click="installFromFile" />
        </div>
        <div v-if="!isModernUi" class="sidebar-tabs legacy-sidebar-tabs"
          role="group" :aria-label="t('pluginMarketplace.browsePlugins')">
          <button v-for="tab in (['discover', 'installed', 'updates'] as const)" :key="tab" type="button"
            :class="{ active: activeTab === tab }" :aria-pressed="activeTab === tab"
            @click="activeTab = tab">
            {{ t('pluginMarketplace.tabs.' + tab) }}
            <span :class="{ 'has-updates': tab === 'updates' && updateEntries.length > 0 }">{{ countFor(tab) }}</span>
          </button>
        </div>
        <div v-if="!isModernUi" class="legacy-list-caption">
          {{ t('pluginMarketplace.resultsCount', { count: visibleEntries.length }) }}
        </div>
        <div class="marketplace-list" :aria-busy="refreshing">
          <button v-for="entry in visibleEntries" :key="entry.id" type="button" class="plugin-row"
            :class="{ selected: selectedId === entry.id }" :aria-pressed="selectedId === entry.id"
            @click="selectedId = entry.id">
            <span class="plugin-mark" aria-hidden="true">{{ monogramFor(entry) }}</span>
            <span class="plugin-row-content">
              <span class="plugin-row-title">
                <strong :title="entry.name">{{ entry.name }}</strong>
              </span>
              <span v-if="!isModernUi" class="plugin-row-description">{{ entry.description }}</span>
              <span class="plugin-row-footer">
                <span class="source-badge" :class="'source-' + sourceKindFor(entry)">{{ t('pluginMarketplace.sourceBadges.' + sourceKindFor(entry)) }}</span>
                <span class="plugin-row-version">v{{ entry.version }}</span>
                <span class="row-state" :class="rowStateTone(entry)">{{ rowStateFor(entry) }}</span>
              </span>
            </span>
          </button>
          <div v-if="visibleEntries.length === 0 && refreshing" class="list-loading" role="status">
            <span>{{ t('pluginMarketplace.loadingPlugins') }}</span>
            <div v-for="row in 3" :key="row" class="loading-row" aria-hidden="true" />
          </div>
          <div v-else-if="visibleEntries.length === 0" class="empty-state" role="status">
            <Search v-if="listIsFiltered" aria-hidden="true" /><Box v-else aria-hidden="true" />
            <p>{{ listIsFiltered ? t('pluginMarketplace.emptyFiltered') : t('pluginMarketplace.empty.' + activeTab) }}</p>
            <el-button v-if="listIsFiltered" size="small"
              @click="searchQuery = ''; officialOnly = false; compatibleOnly = false">
              {{ t('pluginMarketplace.actions.clearFilters') }}
            </el-button>
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
              <span>v{{ selectedPlugin?.manifest.version ?? selectedEntry.version }}</span>
            </div>
          </div>
          <span class="detail-state" :class="selectedStatusClass">
            <span class="state-dot" aria-hidden="true" />{{ selectedStatusLabel }}
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
              :disabled="!selectedCanEnable" :title="selectedEnableBlockedReason || undefined"
              :loading="loading" @click="togglePlugin(selectedPlugin)">
              {{ selectedPlugin.enabled
                ? t('pluginMarketplace.actions.disable')
                : selectedCanEnable ? t('pluginMarketplace.actions.enable') : t('pluginMarketplace.actions.cannotEnable') }}
            </el-button>
            <el-button v-if="selectedSettings.some(Boolean)" size="small" :icon="Setting"
              @click="openPluginSettings">{{ t('pluginMarketplace.actions.openSettings') }}</el-button>
            <el-button size="small" :icon="Document" :loading="logLoading" @click="openPluginLog">
              {{ t('pluginMarketplace.actions.viewLog') }}
            </el-button>
            <el-button size="small" type="danger" plain :icon="Delete"
              :disabled="loading || selectedPlugin.enabled"
              :title="selectedPlugin.enabled ? t('pluginMarketplace.messages.uninstallDisabled') : t('pluginMarketplace.actions.uninstall')"
              @click="uninstallPlugin">
              {{ t('pluginMarketplace.actions.uninstall') }}
            </el-button>
          </template>
        </div>

        <PluginCompatibilityPanel v-if="isModernUi" :status="selectedCompatibilityStatus"
          :title="t('pluginMarketplace.compatibility.title')"
          :summary="selectedCompatibilitySummary" :items="selectedCompatibilityItems"
          :checked-at="compatibilityCheckedAt"
          :source-note="t('pluginMarketplace.compatibility.sourceNote')" />

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
            <section
              v-if="detailTab === 'features'"
              ref="settingsSectionRef"
              class="detail-section"
              :class="{ 'settings-attention': settingsAttention }"
            >
              <template v-if="selectedEntry.id === LIQUID_GRAVITY_PROBE_PLUGIN_ID">
                <section class="liquid-market-disclaimer">
                  <header class="liquid-market-disclaimer-header">
                    <span class="liquid-market-disclaimer-icon" aria-hidden="true"><InfoFilled /></span>
                    <div>
                      <h3>使用风险与免责声明</h3>
                      <p>启用前请确认以下风险。详细声明也会在首次启动插件时弹出并等待选择。</p>
                    </div>
                  </header>
                  <ul>
                    <li>插件会修改对应 Mod 的 INI、配置和运行时资源。使用前请自行备份原始 Mod；本插件不保留持久化备份。</li>
                    <li>错误配置、游戏更新、第三方 Mod 冲突或性能不足可能导致效果异常、卡顿、闪退或配置损坏，请先在单人、离线环境测试。</li>
                    <li>在线游戏、竞技环境和成人内容须自行遵守当地法律、游戏条款及平台规则；账号处罚、封禁和内容责任由使用者承担。</li>
                    <li>本项目为随缘维护，不承诺固定更新频率、问题响应、兼容性修复或长期支持。</li>
                  </ul>
                  <details class="liquid-market-disclaimer-details">
                    <summary>查看完整免责声明</summary>
                    <div>
                      <p>本插件是非官方社区工具，与游戏开发商、发行商、平台运营方及第三方 Mod 作者无隶属、授权或背书关系。</p>
                      <p>插件不保证第三方模型、贴图、音频或成人内容的合法性、安全性、版权状态和兼容性。使用者应自行确认来源与授权范围。</p>
                      <p>插件按“现状”提供，不作持续兼容、无错误或不中断运行的担保。在适用法律允许的范围内，作者与贡献者不对数据丢失、设备损坏、账号处罚、内容损失或间接损失承担责任。</p>
                    </div>
                  </details>
                </section>
              </template>
              <template v-else-if="selectedSettings.includes('ssmt.dlss5.settings') && selectedPlugin">
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
              <template v-else-if="selectedSettings.includes('ssmt.secondary-motion.settings') && selectedPlugin && installedGameName === selectedGameName && selectedPlugin.lifecycleStatus !== 'incompatible'">
                <SecondaryMotionSettings
                  :plugin-id="selectedPlugin.manifest.id"
                  :game-name="selectedGameName"
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
              <PluginPermissionList v-if="isModernUi" :permissions="selectedPermissions" />
              <div v-else class="permission-list">
                <span v-for="permission in selectedPermissions" :key="permission" class="permission-row">
                  {{ permission }}
                </span>
                <span v-if="selectedPermissions.length === 0" class="muted-copy">{{ t('pluginMarketplace.none') }}</span>
              </div>
            </section>
            <section v-else class="detail-section detail-meta">
              <h3>{{ t('pluginMarketplace.metadata') }}</h3>
              <dl>
                <dt>ID</dt><dd>{{ selectedEntry.id }}</dd>
                <dt>{{ t('pluginMarketplace.version') }}</dt><dd>{{ selectedPlugin?.manifest.version ?? selectedEntry.version }}</dd>
                <dt>{{ t('pluginMarketplace.author') }}</dt><dd>{{ selectedEntry.author }}</dd>
                <dt>{{ t('pluginMarketplace.source') }}</dt><dd>{{ sourceLabelFor(selectedEntry) }}</dd>
                <dt>{{ t('pluginMarketplace.currentGame') }}</dt><dd>{{ selectedPlugin?.appScoped ? t('pluginMarketplace.appScope') : (selectedGameName || '—') }}</dd>
                <template v-if="selectedInjectionComponents.length">
                  <dt>{{ t('pluginMarketplace.injectionComponents') }}</dt>
                  <dd>{{ selectedInjectionComponents.map(item => item.path).join(' · ') }}</dd>
                </template>
                <template v-if="selectedProjectUrl">
                  <dt>{{ t('pluginMarketplace.originalProject') }}</dt>
                  <dd>
                    <el-button link type="primary" @click="openOriginalProject">
                      Sp1cHless / Arknights-Endfield-Plugin-Secondary-bodyphysics ↗
                    </el-button>
                  </dd>
                  <dt>{{ t('pluginMarketplace.maintenance') }}</dt>
                  <dd>{{ t('pluginMarketplace.maintenanceNote') }}</dd>
                </template>
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
    </section>

    <el-dialog v-model="riskDialogOpen" :title="t('pluginMarketplace.risk.title')"
      width="min(680px, calc(100vw - 36px))" :close-on-click-modal="false">
      <div class="native-risk-dialog">
        <p><strong>{{ riskPlugin?.manifest.name }} v{{ riskPlugin?.manifest.version }}</strong></p>
        <p>{{ t('pluginMarketplace.risk.summary') }}</p>
        <ul>
          <li>{{ t('pluginMarketplace.risk.account') }}</li>
          <li>{{ t('pluginMarketplace.risk.stability') }}</li>
          <li>{{ t('pluginMarketplace.risk.native') }}</li>
          <li>{{ t('pluginMarketplace.risk.update') }}</li>
        </ul>
        <p>{{ t('pluginMarketplace.risk.warranty') }}</p>
        <el-checkbox v-model="riskAccepted">{{ t('pluginMarketplace.risk.accept') }}</el-checkbox>
      </div>
      <template #footer>
        <el-button @click="riskDialogOpen = false">{{ t('pluginMarketplace.risk.cancel') }}</el-button>
        <el-button type="danger" :disabled="!riskAccepted" :loading="loading" @click="confirmRiskyEnable">
          {{ t('pluginMarketplace.risk.enable') }}
        </el-button>
      </template>
    </el-dialog>

    <LiquidGravityProbeDisclaimer
      :open="liquidDisclaimerOpen"
      :busy="liquidDisclaimerBusy"
      @accept="acceptLiquidDisclaimer"
      @decline="declineLiquidDisclaimer"
    />

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
        <p class="review-detail">{{ t('pluginMarketplace.review.permissions') }} {{
          reviewInspection.manifest.permissions
            .map(permission => t(permissionDefinitionFor(permission).labelKey))
            .join(', ') || t('pluginMarketplace.none')
        }}</p>
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
  --market-accent: var(--ssmt-brand-accent);
  --market-accent-rgb: var(--ssmt-brand-accent-rgb);
  --market-accent-ink: var(--ssmt-brand-accent-ink);
  display: flex;
  flex-direction: column;
  container: plugin-marketplace / inline-size;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  box-sizing: border-box;
  padding: var(--t-page-padding);
  color: var(--t-page-text);
}
.marketplace-shell {
  display: flex;
  flex-direction: column;
  gap: 11px;
  box-sizing: border-box;
  min-width: 0;
  min-height: 0;
  height: 100%;
  padding: 16px;
  overflow: hidden;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), .10);
  border-radius: 18px;
  background:
    radial-gradient(circle at 88% 0%, rgba(var(--market-accent-rgb), .075), transparent 28%),
    rgba(10, 11, 13, .965);
  box-shadow: 0 18px 48px rgba(0, 0, 0, .22);
}
.marketplace-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  flex: none;
  min-width: 0;
  padding: 1px 2px 0;
}
.marketplace-heading-copy { min-width: 0; }
.marketplace-heading h1 {
  margin: 0;
  font-size: 22px;
  font-weight: 650;
  line-height: 1.25;
  letter-spacing: -.35px;
}
.marketplace-brandline {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 4px;
  color: var(--t-page-text-muted);
  font-size: 9px;
}
.brandline-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--market-accent);
  box-shadow: 0 0 0 3px rgba(var(--market-accent-rgb), .12);
}
.marketplace-platform {
  flex: none;
  padding: 6px 11px;
  border: 1px solid var(--market-line);
  border-radius: 999px;
  background: rgba(var(--theme-surface-tint-rgb), .045);
  color: var(--market-muted);
  font-size: 9px;
}
.marketplace-toolbar {
  display: flex;
  align-items: center;
  gap: 7px;
  flex: none;
}
.marketplace-search {
  flex: 1;
  min-width: 0;
}
.marketplace-search :deep(.el-input__wrapper) {
  min-height: 38px;
  border-radius: 10px;
  background: rgba(var(--theme-surface-tint-rgb), .045);
  box-shadow: 0 0 0 1px var(--market-line) inset;
}
.marketplace-search :deep(.el-input__wrapper.is-focus) {
  box-shadow: 0 0 0 1px rgba(var(--market-accent-rgb), .62) inset;
}
.toolbar-action,
.install-action {
  flex: none;
  min-height: 38px;
  margin: 0 !important;
  border-color: var(--market-line) !important;
  background: rgba(var(--theme-surface-tint-rgb), .045) !important;
}
.toolbar-action {
  width: 38px;
  padding: 0 !important;
}
.install-action { padding-inline: 13px !important; }
.marketplace-browsebar {
  display: flex;
  align-items: center;
  gap: 14px;
  flex: none;
}
.marketplace-filters {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}
.marketplace-filters button {
  padding: 4px 8px;
  border: 1px solid var(--market-line);
  border-radius: 999px;
  background: transparent;
  color: var(--market-muted);
  font: inherit;
  font-size: 9px;
  cursor: pointer;
}
.marketplace-filters button:hover {
  color: var(--t-page-text);
  background: var(--t-surface-hover);
}
.marketplace-filters button.active {
  border-color: rgba(var(--market-accent-rgb), .42);
  background: rgba(var(--market-accent-rgb), .10);
  color: var(--t-page-text);
}
.detail-top, .detail-actions, .dependency-heading {
  display: flex;
  align-items: center;
}
.catalog-notice {
  flex: none;
  max-height: 30%;
  overflow-y: auto;
  border: 1px solid color-mix(in srgb, var(--t-warning-border) 72%, rgba(255, 255, 255, .16));
  border-radius: 8px;
  background:
    linear-gradient(135deg, rgba(var(--theme-warning-rgb), .16), rgba(var(--theme-warning-rgb), .06)),
    rgba(7, 11, 18, .94);
  box-shadow: 0 8px 28px rgba(0, 0, 0, .22);
  font-size: 12px;
  line-height: 1.6;
}
.catalog-notice summary { display: flex; align-items: center; gap: 9px; padding: 10px 14px; cursor: pointer; list-style: none; color: var(--t-warning-text); font-weight: 600; }
.catalog-notice summary::-webkit-details-marker { display: none; }
.catalog-notice summary > svg { flex: none; width: 16px; height: 16px; }
.notice-details { margin-left: auto; white-space: nowrap; color: color-mix(in srgb, var(--t-page-text) 72%, transparent); font-size: 11px; font-weight: 400; }
.catalog-notice p {
  padding: 0 14px 12px 39px;
  margin: 0;
  overflow-wrap: anywhere;
  color: rgba(var(--theme-text-primary-rgb), .92);
  font: 11px/1.65 ui-monospace, Consolas, monospace;
  user-select: text;
}
.review-detail { font: 11px/1.5 ui-monospace, Consolas, monospace; overflow-wrap: anywhere; }
.review-disclaimer { padding: 12px; border: var(--t-page-panel-border); border-radius: 6px; line-height: 1.6; }
.native-risk-dialog { color: var(--t-page-text); font-size: 12px; line-height: 1.75; }
.native-risk-dialog p { margin: 8px 0; }
.native-risk-dialog ul { padding: 10px 12px 10px 30px; margin: 10px 0; border: 1px solid var(--t-warning-border); border-radius: 8px; background: var(--t-warning-bg); color: var(--t-warning-text); }
.native-risk-dialog li + li { margin-top: 5px; }
.liquid-market-disclaimer {
  display: grid;
  gap: 12px;
  padding: 14px;
  border: 1px solid rgba(var(--market-accent-rgb), .28);
  border-radius: 8px;
  background:
    linear-gradient(135deg, rgba(var(--market-accent-rgb), .075), transparent 48%),
    rgba(8, 14, 17, .72);
}
.liquid-market-disclaimer-header {
  display: flex;
  align-items: flex-start;
  gap: 11px;
}
.liquid-market-disclaimer-icon {
  display: grid;
  flex: 0 0 30px;
  width: 30px;
  height: 30px;
  place-items: center;
  border: 1px solid rgba(var(--market-accent-rgb), .36);
  border-radius: 6px;
  background: rgba(var(--market-accent-rgb), .1);
  color: var(--market-accent);
}
.liquid-market-disclaimer-icon svg { width: 16px; height: 16px; }
.liquid-market-disclaimer-header h3 {
  margin: 0;
  color: #eef4d7;
  font-size: 13px;
  letter-spacing: 0;
}
.liquid-market-disclaimer-header p {
  margin: 4px 0 0;
  color: rgba(var(--theme-text-secondary-rgb), .86);
  font-size: 11px;
  line-height: 1.6;
}
.liquid-market-disclaimer > ul {
  display: grid;
  gap: 6px;
  margin: 0;
  padding-left: 18px;
  color: rgba(var(--theme-text-secondary-rgb), .9);
  font-size: 11px;
  line-height: 1.65;
}
.liquid-market-disclaimer > ul li::marker { color: var(--market-accent); }
.liquid-market-disclaimer-details {
  padding-top: 10px;
  border-top: 1px solid rgba(255, 255, 255, .075);
}
.liquid-market-disclaimer-details summary {
  width: max-content;
  color: var(--market-accent);
  font-size: 11px;
  font-weight: 650;
  cursor: pointer;
}
.liquid-market-disclaimer-details > div {
  display: grid;
  gap: 7px;
  margin-top: 9px;
  color: rgba(var(--theme-text-secondary-rgb), .84);
  font-size: 11px;
  line-height: 1.7;
}
.liquid-market-disclaimer-details p { margin: 0; }
.section-hint, .muted-copy {
  color: var(--market-muted);
  font-size: 12px;
  line-height: 1.7;
}
.marketplace-layout { display: grid; grid-template-columns: minmax(280px, 330px) minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); gap: 12px; flex: 1; align-items: stretch; min-height: 0; }
.marketplace-sidebar, .plugin-detail {
  /* 百分比高度包含边框和内边距，保持底边与所在网格对齐。 */
  box-sizing: border-box;
  min-width: 0;
  border: var(--t-page-panel-border);
  border-radius: 14px;
  background: linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), .035), transparent), rgba(13, 14, 17, .94);
  box-shadow: 0 10px 28px rgba(0, 0, 0, .13);
}
/* 静态背景和未配置游戏时也会遇到合成闪烁；市场及其设置统一使用无背景模糊的材质。 */
.plugin-marketplace,
.plugin-marketplace :deep(*) {
  backdrop-filter: none !important;
  -webkit-backdrop-filter: none !important;
}
/* 内容少时保持自然高度，超出本列可用高度后只收缩并滚动列表。 */
.marketplace-sidebar { display: flex; flex-direction: column; min-height: 0; max-height: 100%; overflow: hidden; }
.search-icon { width: 15px; height: 15px; }
.sidebar-tabs { display: flex; gap: 3px; padding: 3px; border: 1px solid var(--market-line); border-radius: 9px; background: rgba(var(--theme-surface-tint-rgb), .035); }
.sidebar-tabs button, .detail-tabs button {
  border: 0; background: transparent;
  color: var(--market-muted); font: inherit; font-size: 12px; cursor: pointer;
  transition: background .15s, color .15s;
}
.sidebar-tabs button { min-width: 82px; padding: 7px 12px; border-radius: 6px; white-space: nowrap; }
.sidebar-tabs button span { margin-left: 4px; font-size: 10px; color: var(--t-page-text-muted); font-variant-numeric: tabular-nums; }
.sidebar-tabs button.active { background: rgba(var(--theme-surface-tint-rgb), .10); color: var(--t-page-text); box-shadow: 0 1px 3px #0002; }
.sidebar-tabs .has-updates { color: var(--t-warning-text); }
.sidebar-tabs button:hover, .detail-tabs button:hover { color: var(--t-page-text); }
.list-caption { color: var(--t-page-text-muted); font-size: 10px; white-space: nowrap; }
.marketplace-list { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; padding: 7px; }
.plugin-row { display: flex; width: 100%; gap: 10px; padding: 10px; margin: 2px 0; border: 1px solid transparent; border-radius: 9px; background: transparent; color: inherit; text-align: left; cursor: pointer; transition: background .15s, border-color .15s; }
.plugin-row { position: relative; }
.plugin-row:hover { background: var(--t-surface-hover); }
.plugin-row.selected {
  border-color: rgba(var(--market-accent-rgb), .18);
  background: linear-gradient(90deg, rgba(var(--market-accent-rgb), .09), rgba(var(--theme-surface-tint-rgb), .055));
}
.plugin-row.selected::before {
  position: absolute;
  top: 13px;
  bottom: 13px;
  left: -1px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: var(--market-accent);
  content: '';
}
.plugin-mark { display: grid; place-items: center; flex: 0 0 auto; width: 37px; height: 37px; border: 1px solid var(--market-line); border-radius: 10px; background: linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), .12), rgba(var(--theme-surface-tint-rgb), .03)); color: var(--t-page-text); font-weight: 650; font-size: 13px; letter-spacing: .5px; }
.plugin-row.selected .plugin-mark {
  border-color: rgba(var(--market-accent-rgb), .34);
  background: var(--market-accent);
  color: var(--market-accent-ink);
}
.plugin-mark.large { width: 56px; height: 56px; border-radius: 14px; font-size: 20px; }
.plugin-detail .plugin-mark.large {
  border-color: rgba(var(--market-accent-rgb), .34);
  background:
    linear-gradient(145deg, rgba(255, 255, 255, .30), transparent 46%),
    var(--market-accent);
  color: var(--market-accent-ink);
  box-shadow: 0 8px 24px rgba(var(--market-accent-rgb), .10);
}
.plugin-row-content { display: block; min-width: 0; flex: 1; }
.plugin-row-title strong { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 600; line-height: 1.5; }
.plugin-row-footer { display: flex; align-items: center; gap: 7px; flex-wrap: wrap; margin-top: 6px; font-size: 10px; }
.plugin-row-version { color: var(--t-page-text-muted); font-variant-numeric: tabular-nums; }
.source-badge { display: inline-flex; align-items: center; flex: none; padding: 1px 6px; border: 1px solid var(--market-line); border-radius: 4px; color: var(--market-muted); font-size: 10px; font-weight: 400; line-height: 1.7; }
.row-state { margin-left: auto; font-size: 10px; color: var(--market-muted); }
.row-state.enabled { color: var(--t-page-text); }
.row-state.issue, .detail-issue { color: var(--t-warning-text); }
.plugin-detail { padding: 18px; height: 100%; min-height: 0; overflow-y: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }
.detail-top { gap: 14px; flex-wrap: wrap; }
.detail-identity { min-width: 0; flex: 1 1 200px; }
.detail-identity h2 { margin: 0 0 9px; font-size: 21px; line-height: 1.3; font-weight: 650; overflow-wrap: anywhere; letter-spacing: -.3px; }
.detail-byline { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; color: var(--market-muted); font-size: 11px; }
.detail-description { margin: 14px 0 12px; font-size: 13px; line-height: 1.8; color: var(--market-muted); }
.detail-actions { gap: 8px; flex-wrap: wrap; min-height: 32px; margin-top: 12px; }
.detail-actions :deep(.el-button) { min-height: 30px; padding: 7px 12px; }
.detail-actions :deep(.el-button + .el-button) { margin-left: 0; }
.plugin-marketplace :deep(.el-button--primary) {
  --el-button-bg-color: var(--market-accent);
  --el-button-border-color: var(--market-accent);
  --el-button-text-color: var(--market-accent-ink);
  --el-button-hover-bg-color: color-mix(in srgb, var(--market-accent) 88%, white);
  --el-button-hover-border-color: color-mix(in srgb, var(--market-accent) 88%, white);
  --el-button-hover-text-color: var(--market-accent-ink);
  --el-button-active-bg-color: color-mix(in srgb, var(--market-accent) 82%, white);
  --el-button-active-border-color: color-mix(in srgb, var(--market-accent) 82%, white);
  --el-button-active-text-color: var(--market-accent-ink);
}
.detail-state {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 9px;
  border: 1px solid var(--market-line);
  border-radius: 999px;
  background: var(--t-surface-subtle);
  color: var(--market-muted);
  font-size: 10px;
  white-space: nowrap;
}
.state-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--t-page-text-muted); }
.is-enabled .state-dot { background: var(--theme-success); box-shadow: 0 0 0 3px var(--t-success-bg); }
.detail-state.is-enabled {
  border-color: var(--t-success-border);
  background: var(--t-success-bg);
  color: var(--t-success-text);
}
.detail-state.is-issue {
  border-color: var(--t-warning-border);
  background: var(--t-warning-bg);
  color: var(--t-warning-text);
}
.detail-state.is-issue .state-dot { background: var(--theme-warning); }
.detail-issue { font-size: 11px; }
.detail-tabs { display: flex; gap: 22px; margin-top: 15px; border-bottom: 1px solid var(--market-line); }
.detail-tabs button { border-bottom: 2px solid transparent; padding: 10px 0; white-space: nowrap; }
.detail-tabs button.active { border-bottom-color: var(--market-accent); color: var(--t-page-text); }
.detail-body { padding-top: 17px; }
.detail-main { min-width: 0; }
.detail-section { min-width: 0; }
.detail-section.settings-attention { animation: settings-attention 1.15s ease-out; }
@keyframes settings-attention {
  0% { filter: brightness(1.35); box-shadow: 0 0 0 2px var(--theme-accent); }
  100% { filter: brightness(1); box-shadow: 0 0 0 2px transparent; }
}
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
.loading-row { height: 58px; border-radius: 7px; margin-top: 12px; background: var(--t-surface-hover); animation: market-loading 1.5s ease-in-out infinite; }
@keyframes market-loading { 0%, 100% { opacity: .4; } 50% { opacity: 1; } }
.plugin-marketplace button:focus-visible, .catalog-notice summary:focus-visible { outline: 2px solid var(--theme-accent); outline-offset: 3px; }
.plugin-log-view { max-height: 58vh; overflow: auto; margin: 0; padding: 12px; background: var(--t-surface-soft); color: var(--t-page-text); font: 11px/1.5 ui-monospace, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }

/* Official v4.2.2 marketplace structure */
.plugin-marketplace.is-classic-ui {
  padding: var(--t-page-padding);
}

.is-classic-ui .marketplace-shell {
  gap: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
  overflow: visible;
}

.is-classic-ui .marketplace-layout {
  gap: 16px;
}

.is-classic-ui .marketplace-sidebar,
.is-classic-ui .plugin-detail {
  border-radius: var(--t-page-panel-radius);
  background: linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), .05), transparent), rgba(8, 12, 19, .94);
  box-shadow: var(--t-page-panel-shadow);
}

.legacy-sidebar-search {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
  padding: 14px 14px 10px;
}

.legacy-sidebar-search .el-input {
  flex: 1;
  min-width: 0;
}

.legacy-sidebar-search .el-button {
  flex: none;
  width: 32px;
  height: 32px;
  margin: 0;
  padding: 0;
}

.legacy-sidebar-tabs {
  display: flex;
  flex: 0 0 auto;
  gap: 3px;
  margin: 0 14px;
  padding: 4px;
  border-radius: 8px;
}

.legacy-sidebar-tabs button {
  flex: 1;
  min-width: 0;
  padding: 7px 0;
  border-radius: 5px;
}

.legacy-list-caption {
  flex: 0 0 auto;
  padding: 14px 17px 6px;
  color: var(--t-page-text-muted);
  font-size: 11px;
}

.is-classic-ui .marketplace-list {
  flex: 0 1 auto;
  padding: 0 7px 7px;
}

.is-classic-ui .plugin-row {
  gap: 11px;
  margin: 3px 0;
  padding: 13px 10px;
  border-radius: 8px;
}

.plugin-row-description {
  display: -webkit-box;
  overflow: hidden;
  margin-top: 5px;
  color: var(--market-muted);
  font-size: 11px;
  line-height: 1.7;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

@container plugin-marketplace (max-width: 1080px) {
  .marketplace-layout { grid-template-columns: minmax(250px, 280px) minmax(0, 1fr); }
  .plugin-detail { padding: 16px; }
  .detail-identity h2 { font-size: 19px; }
}
@container plugin-marketplace (max-width: 760px) {
  .marketplace-shell { padding: 12px; }
  .marketplace-toolbar { flex-wrap: wrap; }
  .marketplace-search { flex-basis: calc(100% - 45px); }
  .install-action { flex: 1; }
  .marketplace-browsebar { align-items: flex-start; flex-direction: column; gap: 7px; }
  .marketplace-filters { margin-left: 0; }
  .list-caption { margin-left: 0; }
  .marketplace-layout { grid-template-columns: minmax(0, 1fr); grid-template-rows: fit-content(35%) minmax(0, 1fr); }
  .detail-tabs { gap: 20px; }
}
@container plugin-marketplace (max-width: 520px) {
  .marketplace-shell { padding: 9px; border-radius: 14px; }
  .marketplace-platform { display: none; }
  .marketplace-search { flex-basis: 100%; }
  .toolbar-action { width: 100%; }
  .install-action { width: 100%; }
  .detail-top { gap: 10px; }
  .plugin-mark.large { width: 48px; height: 48px; }
  .detail-tabs { overflow-x: auto; }
}
@media (prefers-reduced-motion: reduce) {
  .plugin-marketplace * { animation: none; transition: none; }
}
</style>
