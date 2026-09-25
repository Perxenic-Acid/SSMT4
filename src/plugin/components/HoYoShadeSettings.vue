<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { openPath, openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage } from 'element-plus'
import { Download, Edit, FolderOpened, Link } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'

const pluginId = 'ssmt.hoyoshade.bridge'
const dependencyId = 'hoyoshade'
const officialReleasesUrl = 'https://github.com/DuolaD/HoYoShade/releases'
const { t } = useI18n()

type DependencyStatus = 'missing' | 'invalid' | 'ready'
interface DependencyState {
  status: DependencyStatus
  path?: string | null
  reason?: string | null
}
interface PluginSnapshot {
  manifest: { id: string }
  externalDependencies: Record<string, DependencyState>
}

const snapshot = ref<PluginSnapshot | undefined>()
const loading = ref(false)
const dependency = computed(() => snapshot.value?.externalDependencies[dependencyId])

const refresh = async () => {
  try {
    const plugins = await invoke<PluginSnapshot[]>('plugin_registry_snapshot')
    snapshot.value = plugins.find(plugin => plugin.manifest.id === pluginId)
  } catch (error) {
    console.error('Failed to load HoYoShade plugin settings:', error)
  }
}

const chooseDirectory = async () => {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    title: t('settings.plugins.hoyoshade.chooseFolderTitle'),
  })
  if (typeof selected !== 'string' || !selected.trim()) return

  loading.value = true
  try {
    const plugins = await invoke<PluginSnapshot[]>('set_plugin_external_dependency_path', {
      pluginId,
      dependencyId,
      path: selected,
    })
    snapshot.value = plugins.find(plugin => plugin.manifest.id === pluginId)
    ElMessage.success(t('settings.plugins.hoyoshade.pathSaved'))
  } catch (error) {
    ElMessage.error(t('settings.plugins.hoyoshade.pathSaveFailed', { error: String(error) }))
  } finally {
    loading.value = false
  }
}

const openDirectory = async () => {
  if (!dependency.value?.path) return
  try {
    await openPath(dependency.value.path)
  } catch (error) {
    ElMessage.error(t('settings.plugins.hoyoshade.openFolderFailed'))
    console.error(error)
  }
}

onMounted(refresh)
</script>

<template>
  <main class="plugin-settings-page">
    <header class="plugin-settings-header">
      <div class="plugin-settings-icon"><Link /></div>
      <div>
        <p class="plugin-settings-kicker">SSMT PLUGIN</p>
        <h1>HoYoShade Bridge</h1>
        <p>{{ t('settings.plugins.hoyoshade.description') }}</p>
      </div>
    </header>

    <section class="plugin-settings-card">
      <div class="plugin-settings-card-heading">
        <div>
          <h2>{{ t('settings.plugins.hoyoshade.path') }}</h2>
          <p>{{ t('settings.plugins.hoyoshade.downloadHint') }}</p>
        </div>
        <span class="plugin-settings-status" :class="`is-${dependency?.status ?? 'missing'}`">
          {{ t(`settings.plugins.hoyoshade.status.${dependency?.status ?? 'missing'}`) }}
        </span>
      </div>

      <div v-if="!snapshot" class="plugin-settings-missing">
        {{ t('settings.plugins.hoyoshade.notInstalled') }}
      </div>
      <template v-else>
        <div class="plugin-settings-path-row">
          <el-input :model-value="dependency?.path ?? ''" readonly :placeholder="t('settings.plugins.hoyoshade.pathPlaceholder')" />
          <el-tooltip :content="t('settings.plugins.hoyoshade.chooseFolder')" placement="top">
            <el-button :icon="Edit" :loading="loading" :aria-label="t('settings.plugins.hoyoshade.chooseFolder')" @click="chooseDirectory" />
          </el-tooltip>
          <el-tooltip :content="t('settings.plugins.hoyoshade.openFolder')" placement="top">
            <el-button :icon="FolderOpened" :disabled="!dependency?.path" :aria-label="t('settings.plugins.hoyoshade.openFolder')" @click="openDirectory" />
          </el-tooltip>
        </div>
        <p v-if="dependency?.reason" class="plugin-settings-reason">{{ dependency.reason }}</p>
      </template>

      <div class="plugin-settings-actions">
        <el-button :icon="Download" @click="openUrl(officialReleasesUrl)">
          {{ t('settings.plugins.hoyoshade.download') }}
        </el-button>
      </div>
    </section>
  </main>
</template>

<style scoped>
.plugin-settings-page { width: min(920px, 100%); margin: 0; padding: 34px clamp(20px, 4vw, 56px) 48px; color: var(--t-page-text, #edf4f2); }
.plugin-settings-header { display: flex; align-items: flex-start; gap: 16px; margin-bottom: 26px; }
.plugin-settings-icon { display: grid; place-items: center; width: 48px; height: 48px; border: 1px solid rgba(117,214,187,.35); border-radius: 10px; color: #a5ebd6; background: rgba(117,214,187,.12); }
.plugin-settings-kicker { margin: 0; color: #75d6bb; font-size: 11px; font-weight: 800; letter-spacing: .14em; }
h1 { margin: 4px 0 7px; font-size: 28px; }
.plugin-settings-header p:last-child, .plugin-settings-card-heading p { margin: 0; color: rgba(235,242,240,.62); line-height: 1.5; }
.plugin-settings-card { padding: 22px; border: 1px solid rgba(255,255,255,.1); border-radius: 10px; background: rgba(12,18,24,.7); }
.plugin-settings-card-heading { display: flex; justify-content: space-between; gap: 16px; align-items: flex-start; }
h2 { margin: 0 0 6px; font-size: 16px; }
.plugin-settings-status { padding: 4px 8px; border-radius: 6px; font-size: 11px; }
.plugin-settings-status.is-ready { color: #a5ebd6; background: rgba(117,214,187,.14); }
.plugin-settings-status.is-missing { color: #f1c17e; background: rgba(240,177,92,.14); }
.plugin-settings-status.is-invalid { color: #ff9b8e; background: rgba(255,125,112,.14); }
.plugin-settings-path-row { display: flex; gap: 8px; margin-top: 20px; }
.plugin-settings-path-row .el-input { flex: 1; min-width: 0; }
.plugin-settings-reason, .plugin-settings-missing { margin: 10px 0 0; color: rgba(235,242,240,.58); font-size: 12px; }
.plugin-settings-actions { display: flex; justify-content: flex-end; margin-top: 20px; padding-top: 16px; border-top: 1px solid rgba(255,255,255,.1); }
@media (max-width: 640px) { .plugin-settings-card-heading { flex-direction: column; } .plugin-settings-path-row { flex-wrap: wrap; } .plugin-settings-path-row .el-input { flex-basis: 100%; } }
</style>
