<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';

const props = defineProps<{ gameName: string; targetExePath: string }>();
const { t } = useI18n();

interface Dlss5StateSnapshot {
  state: 'not_managed' | 'managed' | 'broken';
  route: string | null;
  managedHost: boolean | null;
  owner: string | null;
  reason: string | null;
}

interface Dlss5RouteScan {
  routes: string[];
  api: string;
  apiLabel: string;
  installedRoute: string | null;
  managedHost: boolean;
  owner: string | null;
  antiCheatWarning: boolean;
}

interface Dlss5LastRunStatus {
  verdict: 'inactive' | 'engaged' | 'unknown';
  reason: 'd3d11_interposer_race' | 'feeder_stopped' | 'nr_evaluated' | 'no_dlss_create' | 'no_nr_evidence' | 'no_log' | 'log_precedes_install';
  observedAtMs: number | null;
}

const dlss5State = ref<Dlss5StateSnapshot | null>(null);
const dlss5LastRun = ref<Dlss5LastRunStatus | null>(null);
const graphicsStatusError = ref('');
const availableGraphicsRoutes = ref<string[]>([]);
const graphicsRouteScan = ref<Dlss5RouteScan | null>(null);
const graphicsApiOverride = ref('auto');
const graphicsSelectedRoute = ref('');
const graphicsActionBusy = ref(false);
let graphicsRequestId = 0;
let graphicsTimer: ReturnType<typeof setTimeout> | undefined;

const refreshGraphicsState = async () => {
  const target = props.targetExePath.trim();
  const requestId = ++graphicsRequestId;
  dlss5State.value = null;
  dlss5LastRun.value = null;
  graphicsStatusError.value = '';
  availableGraphicsRoutes.value = [];
  graphicsRouteScan.value = null;
  if (!target) return;
  try {
    const state = await invoke<Dlss5StateSnapshot>('inspect_dlss5_game_state', { gameExecutable: target });
    if (requestId !== graphicsRequestId) return;
    dlss5State.value = state;
    if (state.state === 'broken') graphicsStatusError.value = state.reason || '';
    if (state.state === 'managed') {
      try {
        const lastRun = await invoke<Dlss5LastRunStatus>('inspect_dlss5_last_run', { gameExecutable: target });
        if (requestId === graphicsRequestId) dlss5LastRun.value = lastRun;
      } catch (error) {
        if (requestId === graphicsRequestId) graphicsStatusError.value = String(error);
      }
    }
    try {
      const routes = await invoke<Dlss5RouteScan>('inspect_dlss5_route_options', {
        gameName: props.gameName, gameExecutable: target, apiOverride: graphicsApiOverride.value,
      });
      if (requestId === graphicsRequestId) {
        graphicsRouteScan.value = routes;
        availableGraphicsRoutes.value = routes.routes;
        if (!routes.routes.includes(graphicsSelectedRoute.value)) graphicsSelectedRoute.value = routes.routes[0] || '';
      }
    } catch (error) {
      if (requestId === graphicsRequestId) graphicsStatusError.value = String(error);
    }
  } catch (error) {
    if (requestId === graphicsRequestId) graphicsStatusError.value = String(error);
  }
};

const installSelectedDlss5Route = async () => {
  const gameExecutable = props.targetExePath.trim();
  const route = graphicsSelectedRoute.value;
  if (!gameExecutable || !route) return;
  if (route === 'optiscaler') {
    try { await invoke('open_dlss5_swapper', { gameName: props.gameName }); }
    catch (error) { ElMessage.error(String(error)); }
    return;
  }
  graphicsActionBusy.value = true;
  try {
    let antiCheatAcknowledged = false;
    if (graphicsRouteScan.value?.antiCheatWarning) {
      try {
        await ElMessageBox.confirm(t('gameSettingsModal.messages.graphicsAntiCheatConfirm'),
          t('gameSettingsModal.fields.graphicsStack'), { type: 'warning' });
      } catch { return; }
      antiCheatAcknowledged = true;
    }
    await invoke('install_managed_dlss5_route', {
      gameName: props.gameName, gameExecutable, route, apiOverride: graphicsApiOverride.value, antiCheatAcknowledged,
    });
    ElMessage.success(t('gameSettingsModal.messages.graphicsInstalled', { route }));
    await refreshGraphicsState();
  } catch (error) {
    graphicsStatusError.value = String(error);
    ElMessage.error(String(error));
  } finally {
    graphicsActionBusy.value = false;
  }
};

const restoreDlss5Route = async () => {
  const gameExecutable = props.targetExePath.trim();
  if (!gameExecutable) return;
  graphicsActionBusy.value = true;
  try {
    await invoke('restore_dlss5_install', { gameName: props.gameName, gameExecutable });
    ElMessage.success(t('gameSettingsModal.messages.graphicsRestored'));
    await refreshGraphicsState();
  } catch (error) {
    graphicsStatusError.value = String(error);
    ElMessage.error(String(error));
  } finally {
    graphicsActionBusy.value = false;
  }
};

watch(() => [props.gameName, props.targetExePath, graphicsApiOverride.value], () => {
  if (graphicsTimer) clearTimeout(graphicsTimer);
  graphicsTimer = setTimeout(() => void refreshGraphicsState(), 350);
}, { immediate: true });

onUnmounted(() => {
  ++graphicsRequestId;
  if (graphicsTimer) clearTimeout(graphicsTimer);
});
</script>

<template>
  <div class="dlss5-graphics-settings">
    <div class="graphics-field-label">{{ t('gameSettingsModal.fields.graphicsStack') }}</div>
    <div class="graphics-path-row">
      <span class="graphics-path-input">
        {{ dlss5State?.state === 'managed'
          ? t('gameSettingsModal.fields.graphicsManagedState', { route: dlss5State.route || '—', owner: dlss5State.owner || 'DLSS5-Swapper' })
          : dlss5State?.state === 'broken'
            ? t('gameSettingsModal.fields.graphicsBrokenState')
            : t('gameSettingsModal.fields.graphicsNotManagedState') }}
        <span v-if="availableGraphicsRoutes.length"> · {{ t('gameSettingsModal.fields.graphicsRoutes', { routes: availableGraphicsRoutes.join(', ') }) }}</span>
        <span v-if="graphicsStatusError" style="color:var(--el-color-danger)"> · {{ graphicsStatusError }}</span>
      </span>
      <el-button size="small" @click="refreshGraphicsState">{{ t('gameSettingsModal.actions.refreshGraphicsState') }}</el-button>
    </div>
    <div class="graphics-path-row graphics-controls">
      <el-select v-model="graphicsApiOverride" size="small" style="width:140px" :aria-label="t('gameSettingsModal.fields.graphicsApi')">
        <el-option value="auto" :label="t('gameSettingsModal.fields.graphicsApiAuto')" />
        <el-option value="d3d11" label="DirectX 11" />
        <el-option value="d3d12" label="DirectX 12" />
      </el-select>
      <el-select v-model="graphicsSelectedRoute" size="small" style="width:150px" :aria-label="t('gameSettingsModal.fields.graphicsRoute')" :disabled="!availableGraphicsRoutes.length">
        <el-option v-for="route in availableGraphicsRoutes" :key="route" :value="route" :label="route" />
      </el-select>
      <el-button size="small" type="primary" :loading="graphicsActionBusy" :disabled="!graphicsSelectedRoute" @click="installSelectedDlss5Route">
        {{ graphicsSelectedRoute === 'optiscaler' ? t('gameSettingsModal.actions.openSwapper') : dlss5State?.state === 'managed' && !dlss5State.managedHost ? t('gameSettingsModal.actions.migrateGraphicsRoute') : t('gameSettingsModal.actions.installGraphicsRoute') }}
      </el-button>
      <el-button v-if="dlss5State?.state === 'managed'" size="small" :loading="graphicsActionBusy" @click="restoreDlss5Route">
        {{ t('gameSettingsModal.actions.restoreGraphicsRoute') }}
      </el-button>
    </div>
    <div v-if="dlss5State?.state === 'managed'" class="graphics-last-run"
      :class="dlss5LastRun?.verdict === 'inactive' ? 'graphics-last-run-error' : ''">
      {{ t('gameSettingsModal.fields.graphicsInstalledOnly') }}
      <template v-if="dlss5LastRun">
        · {{ t(`gameSettingsModal.fields.graphicsRuntime_${dlss5LastRun.reason}`) }}
        <span v-if="dlss5LastRun.observedAtMs">({{ new Date(dlss5LastRun.observedAtMs).toLocaleString() }})</span>
      </template>
    </div>
  </div>
</template>

<style scoped>
.dlss5-graphics-settings { display: flex; flex-direction: column; gap: 12px; }
.graphics-field-label { font-size: 12px; font-weight: 500; color: rgba(255, 255, 255, 0.6); letter-spacing: 0.02em; }
.graphics-path-row { display: flex; gap: 6px; }
.graphics-path-input { flex: 1; min-width: 0; min-height: 34px; box-sizing: border-box; background: rgba(0, 0, 0, 0.3); border: 1px solid rgba(255, 255, 255, 0.08); border-radius: 6px; padding: 7px 10px; color: rgba(255, 255, 255, 0.85); font-size: 13px; overflow-wrap: anywhere; }
.graphics-controls { flex-wrap: wrap; }
.graphics-last-run { color: rgba(255, 255, 255, 0.68); font-size: 12px; }
.graphics-last-run-error { color: var(--el-color-danger); }
</style>
