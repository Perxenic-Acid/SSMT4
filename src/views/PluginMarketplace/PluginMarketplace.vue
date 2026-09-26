<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { ElMessage } from 'element-plus'
import { Download, FolderOpened, Refresh, Setting, SwitchButton, Document, Delete } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { clearPluginLog, readPluginLog } from '../../plugin/logs'

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
  permissions: string[]
  externalDependencies: Array<{ id: string; type: string; requiredFiles: string[] }>
}

interface InstalledPlugin {
  manifest: PluginManifest
  enabled: boolean
  lifecycleStatus: 'installed' | 'enabled' | 'disabled' | 'update_available' | 'broken' | 'incompatible' | 'external_dependency_missing'
  externalDependencies: Record<string, DependencyState>
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
  packageSize: number
  screenshots: string[]
  changelog: string[]
}

const { t } = useI18n()
const router = useRouter()

// TODO 12 deliberately keeps the catalog first-party and local. Online catalog
// refresh and package download belong to the next marketplace iteration.
const catalog: CatalogEntry[] = [
  {
    id: 'ssmt.hoyoshade.bridge',
    name: 'HoYoShade Bridge',
    description: '启动游戏前准备 ReShade.ini，并协调 HoYoShade injector。',
    author: 'SSMT',
    version: '0.1.0',
    supportedGames: ['GIMI', 'HIMI', 'SRMI', 'ZZMI'],
    permissions: ['process.spawn', 'process.observe', 'game.launch'],
    externalDependencies: [{
      id: 'hoyoshade',
      requiredFiles: ['inject.exe', 'ReShade64.dll', 'ReShade.ini', 'LauncherResource/INIBuild.exe', 'InjectResource', 'reshade-shaders', 'Presets'],
    }],
    packageSize: 18_432,
    screenshots: [],
    changelog: [],
  },
  {
    id: 'ssmt.dlss5.integration',
    name: 'DLSS 5 Swapper Integration',
    description: '把 DLSS 5 Swapper 作为外部管理器集成到 SSMT。',
    author: 'SSMT',
    version: '0.1.0',
    supportedGames: ['GIMI', 'HIMI', 'SRMI', 'ZZMI'],
    permissions: ['process.spawn', 'filesystem.read'],
    externalDependencies: [{ id: 'dlss5-swapper', requiredFiles: ['DLSS5-Swapper.exe'] }],
    packageSize: 16_384,
    screenshots: [],
    changelog: [],
  },
]

const activeTab = ref<'discover' | 'installed' | 'updates'>('discover')
const selectedId = ref(catalog[0]?.id ?? '')
const installed = ref<InstalledPlugin[]>([])
const loading = ref(false)
const logDialogOpen = ref(false)
const pluginLog = ref('')
const logLoading = ref(false)

const selectedEntry = computed(() => catalog.find(entry => entry.id === selectedId.value) ?? catalog[0])
const installedById = computed(() => new Map(installed.value.map(plugin => [plugin.manifest.id, plugin])))
const installedEntries = computed(() => catalog.filter(entry => installedById.value.has(entry.id)))
const updateEntries = computed(() => catalog.filter(entry => {
  const current = installedById.value.get(entry.id)
  return current && compareVersions(entry.version, current.manifest.version) > 0
}))
const lifecycleFor = (entry: CatalogEntry, plugin: InstalledPlugin) => {
  if (compareVersions(entry.version, plugin.manifest.version) > 0) return 'update_available'
  return plugin.lifecycleStatus
}
const visibleEntries = computed(() => {
  if (activeTab.value === 'installed') return installedEntries.value
  if (activeTab.value === 'updates') return updateEntries.value
  return catalog
})

const formatSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

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
  loading.value = true
  try {
    installed.value = await invoke<InstalledPlugin[]>('plugin_registry_snapshot')
  } catch (error) {
    console.error('Failed to load plugin registry:', error)
    ElMessage.error(t('pluginMarketplace.messages.loadFailed', { error: String(error) }))
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
    installed.value = await invoke<InstalledPlugin[]>('install_plugin_package', { archivePath: selected })
    ElMessage.success(t('pluginMarketplace.messages.installed'))
  } catch (error) {
    console.error('Failed to install plugin package:', error)
    ElMessage.error(t('pluginMarketplace.messages.installFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const togglePlugin = async (plugin: InstalledPlugin) => {
  loading.value = true
  try {
    installed.value = await invoke<InstalledPlugin[]>('set_plugin_enabled', {
      id: plugin.manifest.id,
      enabled: !plugin.enabled,
    })
    ElMessage.success(plugin.enabled
      ? t('pluginMarketplace.messages.disabled')
      : t('pluginMarketplace.messages.enabled'))
  } catch (error) {
    console.error('Failed to update plugin state:', error)
    ElMessage.error(t('pluginMarketplace.messages.updateFailed', { error: String(error) }))
  } finally {
    loading.value = false
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
  if (selectedEntry.value?.id === 'ssmt.hoyoshade.bridge') {
    void router.push({ name: 'Settings', hash: '#settings-plugins' })
  }
}

const dependencyStatusLabel = (status: DependencyStatus) => t(`pluginMarketplace.dependencyStatus.${status}`)
const dependencyStatusClass = (status: DependencyStatus) => `is-${status}`

onMounted(refreshInstalled)
</script>

<template>
  <main class="plugin-marketplace">
    <header class="marketplace-header">
      <div>
        <p class="marketplace-kicker">SSMT PLUGIN PLATFORM</p>
        <h1>{{ t('pluginMarketplace.title') }}</h1>
        <p class="marketplace-subtitle">{{ t('pluginMarketplace.subtitle') }}</p>
      </div>
      <div class="marketplace-header-actions">
        <el-button :icon="Refresh" :loading="loading" @click="refreshInstalled">
          {{ t('pluginMarketplace.actions.refresh') }}
        </el-button>
        <el-button type="primary" :icon="FolderOpened" :loading="loading" @click="installFromFile">
          {{ t('pluginMarketplace.actions.installFromFile') }}
        </el-button>
      </div>
    </header>

    <div class="marketplace-tabs" role="tablist">
      <button v-for="tab in (['discover', 'installed', 'updates'] as const)" :key="tab" type="button"
        class="marketplace-tab" :class="{ active: activeTab === tab }" role="tab"
        :aria-selected="activeTab === tab" @click="activeTab = tab">
        {{ t(`pluginMarketplace.tabs.${tab}`) }}
        <span v-if="tab === 'installed'" class="tab-count">{{ installedEntries.length }}</span>
        <span v-else-if="tab === 'updates'" class="tab-count">{{ updateEntries.length }}</span>
      </button>
    </div>

    <section class="marketplace-layout">
      <div class="marketplace-list" aria-live="polite">
        <button v-for="entry in visibleEntries" :key="entry.id" type="button" class="plugin-row"
          :class="{ selected: selectedId === entry.id }" @click="selectedId = entry.id">
          <span class="plugin-mark" aria-hidden="true"><span>{{ entry.name.slice(0, 1) }}</span></span>
          <span class="plugin-row-content">
            <span class="plugin-row-title">
              <strong>{{ entry.name }}</strong>
              <span class="plugin-version">v{{ entry.version }}</span>
            </span>
            <span class="plugin-row-description">{{ entry.description }}</span>
            <span class="plugin-row-meta">
              <span>{{ entry.author }}</span>
              <span>{{ formatSize(entry.packageSize) }}</span>
              <span v-if="installedById.has(entry.id)" class="installed-label">
                {{ t(`pluginMarketplace.lifecycle.${lifecycleFor(entry, installedById.get(entry.id)!)}`) }}
              </span>
            </span>
          </span>
        </button>
        <div v-if="visibleEntries.length === 0" class="empty-state">
          {{ t(`pluginMarketplace.empty.${activeTab}`) }}
        </div>
      </div>

      <aside v-if="selectedEntry" class="plugin-detail">
        <div class="detail-heading">
          <span class="plugin-mark large" aria-hidden="true"><span>{{ selectedEntry.name.slice(0, 1) }}</span></span>
          <div>
            <p class="marketplace-kicker">{{ selectedEntry.id }}</p>
            <h2>{{ selectedEntry.name }}</h2>
            <p>{{ t('pluginMarketplace.byAuthor', { author: selectedEntry.author }) }} · v{{ selectedEntry.version }}</p>
          </div>
        </div>

        <p class="detail-description">{{ selectedEntry.description }}</p>

        <div class="detail-section">
          <h3>{{ t('pluginMarketplace.sections.screenshots') }}</h3>
          <div v-if="selectedEntry.screenshots.length === 0" class="muted-copy">{{ t('pluginMarketplace.notProvided') }}</div>
          <div v-else class="screenshot-list">
            <img v-for="screenshot in selectedEntry.screenshots" :key="screenshot" :src="screenshot" :alt="selectedEntry.name" />
          </div>
        </div>

        <div class="detail-section">
          <h3>{{ t('pluginMarketplace.sections.supportedGames') }}</h3>
          <div class="tag-list"><span v-for="game in selectedEntry.supportedGames" :key="game" class="detail-tag">{{ game }}</span></div>
        </div>

        <div class="detail-section">
          <h3>{{ t('pluginMarketplace.sections.permissions') }}</h3>
          <div class="permission-list"><span v-for="permission in selectedEntry.permissions" :key="permission" class="permission-row"><Setting :size="14" />{{ permission }}</span></div>
        </div>

        <div class="detail-section">
          <h3>{{ t('pluginMarketplace.sections.externalDependencies') }}</h3>
          <div v-if="selectedEntry.externalDependencies.length === 0" class="muted-copy">{{ t('pluginMarketplace.none') }}</div>
          <div v-for="dependency in selectedEntry.externalDependencies" :key="dependency.id" class="dependency-row">
            <div class="dependency-heading"><strong>{{ dependency.id }}</strong><span v-if="installedById.get(selectedEntry.id)?.externalDependencies[dependency.id]" class="dependency-status" :class="dependencyStatusClass(installedById.get(selectedEntry.id)!.externalDependencies[dependency.id].status)">{{ dependencyStatusLabel(installedById.get(selectedEntry.id)!.externalDependencies[dependency.id].status) }}</span></div>
            <span class="dependency-files">{{ dependency.requiredFiles.join(' · ') }}</span>
          </div>
        </div>

        <div class="detail-section">
          <h3>{{ t('pluginMarketplace.sections.changelog') }}</h3>
          <ul v-if="selectedEntry.changelog.length" class="changelog-list">
            <li v-for="(item, index) in selectedEntry.changelog" :key="index">{{ item }}</li>
          </ul>
          <div v-else class="muted-copy">{{ t('pluginMarketplace.notProvided') }}</div>
        </div>

        <div class="detail-footer">
          <span class="install-size"><Download :size="15" />{{ formatSize(selectedEntry.packageSize) }}</span>
          <template v-if="installedById.get(selectedEntry.id)">
            <el-button v-if="selectedEntry.id === 'ssmt.hoyoshade.bridge'" :icon="Setting" @click="openPluginSettings">
              {{ t('pluginMarketplace.actions.openSettings') }}
            </el-button>
            <el-button :icon="Document" :loading="logLoading" @click="openPluginLog">{{ t('pluginMarketplace.actions.viewLog') }}</el-button>
            <el-button :type="installedById.get(selectedEntry.id)?.enabled ? 'warning' : 'success'" :icon="SwitchButton" :loading="loading" @click="togglePlugin(installedById.get(selectedEntry.id)!)">
              {{ installedById.get(selectedEntry.id)?.enabled ? t('pluginMarketplace.actions.disable') : t('pluginMarketplace.actions.enable') }}
            </el-button>
          </template>
          <el-button v-else type="primary" :icon="FolderOpened" :loading="loading" @click="installFromFile">{{ t('pluginMarketplace.actions.installPackage') }}</el-button>
        </div>
      </aside>
    </section>

    <el-dialog v-model="logDialogOpen" :title="t('pluginMarketplace.logTitle', { name: selectedEntry?.name ?? '' })" width="min(760px, calc(100vw - 36px))">
      <pre class="plugin-log-view">{{ pluginLog || t('pluginMarketplace.logEmpty') }}</pre>
      <template #footer>
        <el-button :icon="Delete" :loading="logLoading" @click="clearSelectedPluginLog">{{ t('pluginMarketplace.actions.clearLog') }}</el-button>
        <el-button @click="logDialogOpen = false">{{ t('pluginMarketplace.actions.close') }}</el-button>
      </template>
    </el-dialog>
  </main>
</template>

<style scoped>
.plugin-marketplace {
  min-height: 100%;
  box-sizing: border-box;
  padding: 34px clamp(20px, 4vw, 56px) 48px;
  color: var(--t-page-text, #edf4f2);
}

.marketplace-header,
.marketplace-layout,
.detail-heading,
.detail-footer,
.plugin-row-title,
.plugin-row-meta,
.dependency-heading,
.marketplace-header-actions {
  display: flex;
  align-items: center;
}

.marketplace-header { justify-content: space-between; gap: 24px; margin-bottom: 26px; }
.marketplace-header h1 { margin: 4px 0 8px; font-size: clamp(24px, 3vw, 34px); font-weight: 700; letter-spacing: 0; }
.marketplace-kicker { margin: 0; color: var(--t-page-accent, #75d6bb); font-size: 11px; font-weight: 800; letter-spacing: .15em; }
.marketplace-subtitle { max-width: 680px; margin: 0; color: rgba(232, 242, 240, .62); line-height: 1.55; }
.marketplace-header-actions { gap: 10px; flex-wrap: wrap; justify-content: flex-end; }
.marketplace-tabs { display: flex; gap: 4px; margin-bottom: 18px; border-bottom: 1px solid rgba(255,255,255,.1); }
.marketplace-tab { display: inline-flex; align-items: center; gap: 8px; padding: 10px 15px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: rgba(235,242,240,.58); font: inherit; font-size: 13px; cursor: pointer; }
.marketplace-tab:hover { color: rgba(255,255,255,.9); }
.marketplace-tab.active { border-bottom-color: var(--t-page-accent, #75d6bb); color: #fff; }
.tab-count { min-width: 18px; padding: 2px 6px; border-radius: 999px; background: rgba(255,255,255,.1); font-size: 11px; text-align: center; }
.marketplace-layout { align-items: stretch; gap: 18px; min-height: 540px; }
.marketplace-list { flex: 0 1 48%; min-width: 280px; display: flex; flex-direction: column; gap: 8px; }
.plugin-row { display: flex; align-items: flex-start; gap: 13px; width: 100%; padding: 15px; border: 1px solid rgba(255,255,255,.1); border-radius: 10px; background: rgba(17,24,30,.58); color: inherit; text-align: left; cursor: pointer; transition: border-color .18s ease, background .18s ease, transform .18s ease; }
.plugin-row:hover { border-color: rgba(117,214,187,.44); background: rgba(117,214,187,.08); transform: translateY(-1px); }
.plugin-row.selected { border-color: rgba(117,214,187,.72); background: rgba(117,214,187,.12); }
.plugin-mark { flex: 0 0 auto; display: grid; place-items: center; width: 38px; height: 38px; border: 1px solid rgba(117,214,187,.42); border-radius: 10px; background: rgba(117,214,187,.13); color: #a5ebd6; font-size: 17px; font-weight: 800; }
.plugin-mark.large { width: 52px; height: 52px; font-size: 22px; }
.plugin-row-content { min-width: 0; flex: 1; }
.plugin-row-title { justify-content: space-between; gap: 8px; }
.plugin-row-title strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.plugin-version { flex: 0 0 auto; color: rgba(235,242,240,.5); font-size: 11px; }
.plugin-row-description { display: block; margin-top: 6px; color: rgba(235,242,240,.62); font-size: 12px; line-height: 1.45; }
.plugin-row-meta { flex-wrap: wrap; gap: 10px; margin-top: 10px; color: rgba(235,242,240,.42); font-size: 11px; }
.installed-label { color: #9be5cd; }
.plugin-detail { flex: 1 1 52%; min-width: 0; padding: 24px; border: 1px solid rgba(255,255,255,.1); border-radius: 12px; background: rgba(12,18,24,.7); box-shadow: 0 20px 50px rgba(0,0,0,.18); }
.detail-heading { align-items: flex-start; gap: 14px; }
.detail-heading h2 { margin: 5px 0 4px; font-size: 22px; }
.detail-heading p:last-child { margin: 0; color: rgba(235,242,240,.48); font-size: 12px; }
.detail-description { margin: 24px 0; color: rgba(235,242,240,.78); line-height: 1.6; }
.detail-section { margin-top: 22px; }
.detail-section h3 { margin: 0 0 10px; color: rgba(255,255,255,.86); font-size: 12px; font-weight: 700; letter-spacing: .04em; }
.tag-list { display: flex; flex-wrap: wrap; gap: 6px; }
.detail-tag { padding: 5px 8px; border: 1px solid rgba(255,255,255,.12); border-radius: 6px; background: rgba(255,255,255,.05); color: rgba(235,242,240,.72); font-size: 11px; }
.permission-list { display: flex; flex-wrap: wrap; gap: 7px; }
.permission-row { display: inline-flex; align-items: center; gap: 5px; padding: 5px 8px; border-radius: 6px; background: rgba(117,214,187,.1); color: #a9e9d6; font-size: 11px; }
.dependency-row { padding: 10px 0; border-top: 1px solid rgba(255,255,255,.08); }
.dependency-heading { justify-content: space-between; gap: 8px; font-size: 12px; }
.dependency-files { display: block; margin-top: 5px; color: rgba(235,242,240,.46); font-size: 11px; line-height: 1.5; overflow-wrap: anywhere; }
.dependency-status { padding: 3px 6px; border-radius: 5px; font-size: 10px; }
.dependency-status.is-ready { background: rgba(117,214,187,.14); color: #a5ebd6; }
.dependency-status.is-missing { background: rgba(240,177,92,.14); color: #f1c17e; }
.dependency-status.is-invalid { background: rgba(255,125,112,.14); color: #ff9b8e; }
.muted-copy, .empty-state { color: rgba(235,242,240,.44); font-size: 12px; }
.screenshot-list { display: flex; flex-wrap: wrap; gap: 8px; }
.screenshot-list img { max-width: 100%; max-height: 220px; object-fit: contain; border: 1px solid rgba(255,255,255,.12); border-radius: 6px; }
.changelog-list { margin: 0; padding-left: 18px; color: rgba(235,242,240,.62); font-size: 12px; line-height: 1.6; }
.plugin-log-view { max-height: 58vh; overflow: auto; margin: 0; padding: 14px; border: 1px solid rgba(255,255,255,.1); border-radius: 7px; background: rgba(0,0,0,.24); color: rgba(235,242,240,.75); font: 11px/1.55 ui-monospace, SFMono-Regular, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.empty-state { padding: 32px 18px; border: 1px dashed rgba(255,255,255,.14); border-radius: 10px; text-align: center; }
.detail-footer { justify-content: flex-end; gap: 10px; margin-top: 30px; padding-top: 18px; border-top: 1px solid rgba(255,255,255,.1); }
.install-size { display: inline-flex; align-items: center; gap: 5px; margin-right: auto; color: rgba(235,242,240,.5); font-size: 11px; }

@media (max-width: 820px) {
  .marketplace-header { align-items: flex-start; flex-direction: column; }
  .marketplace-header-actions { justify-content: flex-start; }
  .marketplace-layout { flex-direction: column; }
  .marketplace-list { flex: none; width: 100%; }
}
</style>
