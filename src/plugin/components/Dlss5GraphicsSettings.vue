<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';

const props = defineProps<{ gameName: string; targetExePath: string }>();
const { t, locale } = useI18n();
const label = (zh: string, en: string) => String(locale.value).startsWith('zh') ? zh : en;
const hotRuntime = ref<{ root: string; profile: string } | null>(null);
const hotProfile = ref('rtx50');
const downloadProgress = ref<{ phase: string; downloadedBytes: number; totalBytes: number } | null>(null);
const isGimi = () => /(?:^|[\\/])(?:YuanShen|GenshinImpact)\.exe$/i.test(props.targetExePath.trim());
const openComponentSource = () => openUrl('https://github.com/CXP-2024/GIMI_reshade_dlss_integration/tree/cc4697187cb57bacc7c932b7f53f16be24af152a').catch(error => ElMessage.error(String(error)));

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
  hotRuntime.value = null;
  if (!target) return;
  try {
    const state = await invoke<Dlss5StateSnapshot>('inspect_dlss5_game_state', { gameExecutable: target });
    if (requestId !== graphicsRequestId) return;
    dlss5State.value = state;
    if (isGimi()) {
      const hot = await invoke<{ root: string; profile: string } | null>('gimi_hot_runtime_status', { gameName: props.gameName });
      if (requestId !== graphicsRequestId) return;
      hotRuntime.value = hot;
      if (hot) hotProfile.value = hot.profile;
    }
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

const installSelectedDlss5Route = async (offline = false) => {
  if (graphicsActionBusy.value) return;
  const gameName = props.gameName;
  const gameExecutable = props.targetExePath.trim();
  const route = graphicsSelectedRoute.value;
  const profile = hotProfile.value;
  const stillCurrent = () => props.gameName === gameName && props.targetExePath.trim() === gameExecutable;
  if (!gameExecutable || !route) return;
  if (route === 'optiscaler') {
    try { await invoke('open_dlss5_swapper', { gameName: props.gameName }); }
    catch (error) { ElMessage.error(String(error)); }
    return;
  }
  graphicsActionBusy.value = true;
  graphicsStatusError.value = '';
  downloadProgress.value = null;
  try {
    if (route === 'gimi-hot') {
      if (offline) {
        const directory = await open({ directory: true, multiple: false, title: label('选择 GIMI 热注入组件包目录', 'Select the GIMI hot-injection component package') });
        if (typeof directory !== 'string' || !stillCurrent()) return;
        await invoke('import_gimi_hot_runtime', { gameName, gameExecutable, sourceDirectory: directory, profile });
      } else {
        const onProgress = new Channel<{ phase: string; downloadedBytes: number; totalBytes: number }>();
        onProgress.onmessage = event => { if (stillCurrent()) downloadProgress.value = event; };
        await invoke('download_gimi_hot_runtime', { gameName, gameExecutable, profile, onProgress });
      }
      ElMessage.success(label(`${gameName} 热注入组件已配置`, `${gameName} hot-injection components configured`));
      if (stillCurrent()) await refreshGraphicsState();
      return;
    }
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
    if (stillCurrent()) graphicsStatusError.value = String(error);
    ElMessage.error(String(error));
  } finally {
    graphicsActionBusy.value = false;
    downloadProgress.value = null;
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
        {{ hotRuntime ? label('热注入组件已配置 · ', 'Hot-injection components configured · ') + hotRuntime.profile : dlss5State?.state === 'managed'
          ? t('gameSettingsModal.fields.graphicsManagedState', { route: dlss5State.route || '—', owner: dlss5State.owner || 'DLSS5-Swapper' })
          : dlss5State?.state === 'broken'
            ? t('gameSettingsModal.fields.graphicsBrokenState')
            : t('gameSettingsModal.fields.graphicsNotManagedState') }}
        <span v-if="availableGraphicsRoutes.length"> · {{ t('gameSettingsModal.fields.graphicsRoutes', { routes: availableGraphicsRoutes.join(', ') }) }}</span>
        <span v-if="graphicsStatusError" style="color:var(--el-color-danger)"> · {{ graphicsStatusError }}</span>
      </span>
      <el-button size="small" :disabled="graphicsActionBusy" @click="refreshGraphicsState">{{ t('gameSettingsModal.actions.refreshGraphicsState') }}</el-button>
    </div>
    <div class="graphics-path-row graphics-controls">
      <el-select v-if="!isGimi()" v-model="graphicsApiOverride" size="small" style="width:140px" :aria-label="t('gameSettingsModal.fields.graphicsApi')">
        <el-option value="auto" :label="t('gameSettingsModal.fields.graphicsApiAuto')" />
        <el-option value="d3d11" label="DirectX 11" />
        <el-option value="d3d12" label="DirectX 12" />
      </el-select>
      <el-select v-if="isGimi()" v-model="hotProfile" size="small" style="width:140px" :disabled="graphicsActionBusy" :aria-label="label('显卡组件 Profile', 'GPU component profile')">
        <el-option value="rtx50" label="RTX 50" />
        <el-option value="rtx30" :label="label('RTX 30 / 40（40 为实验支持）', 'RTX 30 / 40 (40 experimental)')" />
      </el-select>
      <el-select v-model="graphicsSelectedRoute" size="small" style="width:150px" :aria-label="t('gameSettingsModal.fields.graphicsRoute')" :disabled="graphicsActionBusy || !availableGraphicsRoutes.length">
        <el-option v-for="route in availableGraphicsRoutes" :key="route" :value="route" :label="route" />
      </el-select>
      <el-button size="small" type="primary" :loading="graphicsActionBusy" :disabled="!graphicsSelectedRoute" @click="installSelectedDlss5Route()">
        {{ graphicsSelectedRoute === 'gimi-hot' ? label('自动下载并配置', 'Download and configure') : graphicsSelectedRoute === 'optiscaler' ? t('gameSettingsModal.actions.openSwapper') : dlss5State?.state === 'managed' && !dlss5State.managedHost ? t('gameSettingsModal.actions.migrateGraphicsRoute') : t('gameSettingsModal.actions.installGraphicsRoute') }}
      </el-button>
      <el-button v-if="graphicsSelectedRoute === 'gimi-hot'" size="small" :disabled="graphicsActionBusy" @click="installSelectedDlss5Route(true)">
        {{ label('离线导入', 'Import offline') }}
      </el-button>
      <el-button v-if="dlss5State?.state === 'managed'" size="small" :loading="graphicsActionBusy" @click="restoreDlss5Route">
        {{ t('gameSettingsModal.actions.restoreGraphicsRoute') }}
      </el-button>
    </div>
    <div v-if="downloadProgress" class="graphics-last-run" role="status" aria-live="polite">
      {{ downloadProgress.phase === 'downloading' ? label('正在下载并校验', 'Downloading and verifying') : label('正在提取并校验组件', 'Extracting and verifying components') }}
      · {{ (downloadProgress.downloadedBytes / 1048576).toFixed(1) }} / {{ (downloadProgress.totalBytes / 1048576).toFixed(1) }} MiB
      <el-progress :percentage="Math.min(100, Math.floor(downloadProgress.downloadedBytes * 100 / Math.max(1, downloadProgress.totalBytes)))" />
    </div>
    <div v-if="isGimi()" class="graphics-last-run">
      {{ label('自动下载约 302 MiB，包含所选显卡的完整 NR Runtime。下载失败保留原组件，可重试或离线导入；不影响普通游戏启动。', 'Downloads about 302 MiB including the matching NR Runtime. Failure preserves existing components; retry or import offline. Normal game launch remains available.') }}
      {{ label('来源：CXP GIMI 集成项目（GitHub）及其 Runtime 包（Google Drive）。属于第三方实验组件，校验不代表兼容性保证。', 'Source: CXP GIMI integration (GitHub) and its runtime package (Google Drive). These are experimental third-party components; verification does not guarantee compatibility.') }}
      <el-button link type="primary" size="small" @click="openComponentSource">{{ label('查看组件来源', 'View component source') }}</el-button>
      {{ label('原神使用热注入，不向游戏目录放置代理 DLL。游戏内请选择 FSR2，渲染精度至少 1.0；由 SSMT 直接启动。', 'Genshin uses hot injection without game-local proxy DLLs. Select FSR2 and render resolution 1.0 or higher in game, then launch directly through SSMT.') }}
      <span v-if="hotRuntime"> · {{ label('组件已配置，尚不代表 NR 生效：', 'Components configured; NR activation still requires verification: ') }}{{ hotRuntime.root }}</span>
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
