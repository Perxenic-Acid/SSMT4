<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Check, Download, Monitor, Refresh } from '@element-plus/icons-vue';

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
const graphicsStateLoading = ref(false);
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
  graphicsStateLoading.value = false;
  if (!target) return;
  graphicsStateLoading.value = true;
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
  } finally {
    if (requestId === graphicsRequestId) graphicsStateLoading.value = false;
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
  ++graphicsRequestId;
  hotRuntime.value = null;
  dlss5State.value = null;
  dlss5LastRun.value = null;
  availableGraphicsRoutes.value = [];
  graphicsRouteScan.value = null;
  graphicsSelectedRoute.value = '';
  graphicsStateLoading.value = !!props.targetExePath.trim();
  graphicsStatusError.value = '';
  downloadProgress.value = null;
  if (graphicsTimer) clearTimeout(graphicsTimer);
  graphicsTimer = setTimeout(() => void refreshGraphicsState(), 350);
}, { immediate: true });

onUnmounted(() => {
  ++graphicsRequestId;
  if (graphicsTimer) clearTimeout(graphicsTimer);
});
</script>

<template>
  <section class="dlss5-graphics-settings" :aria-label="label('DLSS5 组件设置', 'DLSS5 component settings')">
    <div class="graphics-heading">
      <h3>{{ label('DLSS5 组件', 'DLSS5 components') }}</h3>
      <el-button size="small" :icon="Refresh" :disabled="graphicsActionBusy" :loading="graphicsStateLoading" @click="refreshGraphicsState">
        {{ t('gameSettingsModal.actions.refreshGraphicsState') }}
      </el-button>
    </div>
    <div class="graphics-status" :class="{ 'is-configured': hotRuntime || dlss5State?.state === 'managed' }" role="status">
      <span class="graphics-status-icon"><Check v-if="hotRuntime || dlss5State?.state === 'managed'" /><Monitor v-else /></span>
      <div>
        <strong>{{ graphicsStateLoading ? label('正在读取组件状态…', 'Checking components…') : hotRuntime
          ? label('热注入组件已配置', 'Hot-injection components configured') : dlss5State?.state === 'managed'
            ? t('gameSettingsModal.fields.graphicsManagedState', { route: dlss5State.route || '—', owner: dlss5State.owner || 'DLSS5-Swapper' })
            : dlss5State?.state === 'broken' ? t('gameSettingsModal.fields.graphicsBrokenState')
              : label('尚未配置组件', 'Components not configured') }}</strong>
        <p v-if="hotRuntime">{{ hotRuntime.profile === 'rtx50' ? 'RTX 50' : 'RTX 30 / 40' }} · {{ label('可重新下载或切换显卡组件', 'Reinstall or choose another GPU profile') }}</p>
        <p v-else-if="isGimi()">{{ label('选择显卡后，自动下载并部署完整组件。', 'Choose your GPU to download and deploy the complete components.') }}</p>
        <p v-else>{{ label('选择适合当前游戏的渲染路线。', 'Choose a rendering route for this game.') }}</p>
      </div>
    </div>
    <p v-if="graphicsStatusError" class="graphics-error" role="alert">{{ graphicsStatusError }}</p>

    <div class="graphics-setup">
      <div class="graphics-controls">
        <label v-if="isGimi()" class="graphics-field">
          <span>{{ label('显卡系列', 'GPU series') }}</span>
          <el-select v-model="hotProfile" :disabled="graphicsActionBusy" :aria-label="label('显卡系列', 'GPU series')">
            <el-option value="rtx50" label="RTX 50" />
            <el-option value="rtx30" :label="label('RTX 30 / 40（40 为实验支持）', 'RTX 30 / 40 (40 experimental)')" />
          </el-select>
        </label>
        <template v-else>
          <label class="graphics-field">
            <span>{{ t('gameSettingsModal.fields.graphicsApi') }}</span>
            <el-select v-model="graphicsApiOverride" :disabled="graphicsActionBusy" :aria-label="t('gameSettingsModal.fields.graphicsApi')">
              <el-option value="auto" :label="t('gameSettingsModal.fields.graphicsApiAuto')" />
              <el-option value="d3d11" label="DirectX 11" />
              <el-option value="d3d12" label="DirectX 12" />
            </el-select>
          </label>
          <label class="graphics-field">
            <span>{{ t('gameSettingsModal.fields.graphicsRoute') }}</span>
            <el-select v-model="graphicsSelectedRoute" :aria-label="t('gameSettingsModal.fields.graphicsRoute')" :disabled="graphicsActionBusy || !availableGraphicsRoutes.length">
              <el-option v-for="route in availableGraphicsRoutes" :key="route" :value="route" :label="route" />
            </el-select>
          </label>
        </template>
      </div>
      <div class="graphics-actions">
        <el-button type="primary" :icon="Download" :loading="graphicsActionBusy" :disabled="!graphicsSelectedRoute || graphicsStateLoading" @click="installSelectedDlss5Route()">
          {{ graphicsSelectedRoute === 'gimi-hot' ? label('自动下载并配置', 'Download and configure') : graphicsSelectedRoute === 'optiscaler' ? t('gameSettingsModal.actions.openSwapper') : dlss5State?.state === 'managed' && !dlss5State.managedHost ? t('gameSettingsModal.actions.migrateGraphicsRoute') : t('gameSettingsModal.actions.installGraphicsRoute') }}
        </el-button>
        <el-button v-if="isGimi()" :disabled="graphicsActionBusy || !graphicsSelectedRoute || graphicsStateLoading" @click="installSelectedDlss5Route(true)">
          {{ label('离线导入', 'Import offline') }}
        </el-button>
        <el-button v-if="dlss5State?.state === 'managed'" :disabled="graphicsActionBusy" @click="restoreDlss5Route">
          {{ t('gameSettingsModal.actions.restoreGraphicsRoute') }}
        </el-button>
      </div>
    </div>
    <div v-if="downloadProgress" class="graphics-download" role="status" aria-live="polite">
      <div class="graphics-download-heading">
        <strong>{{ downloadProgress.phase === 'downloading' ? label('正在下载并校验', 'Downloading and verifying') : label('正在提取并校验组件', 'Extracting and verifying components') }}</strong>
        <span>{{ (downloadProgress.downloadedBytes / 1048576).toFixed(1) }} / {{ (downloadProgress.totalBytes / 1048576).toFixed(1) }} MiB</span>
      </div>
      <el-progress :percentage="Math.min(100, Math.floor(downloadProgress.downloadedBytes * 100 / Math.max(1, downloadProgress.totalBytes)))" />
    </div>
    <template v-if="isGimi()">
      <p class="graphics-hint">{{ label('首次下载约 302 MiB。请先关闭游戏；下载失败保留原组件，普通游戏启动仍可使用。', 'First download: about 302 MiB. Close the game before setup. Failed downloads preserve existing components and normal game launch remains available.') }}</p>
      <div class="graphics-startup">
        <h4>{{ label('启动前', 'Before launching') }}</h4>
        <ul>
          <li>{{ label('游戏内抗锯齿选择 FSR2，渲染精度至少 1.0。', 'Select FSR2 and render resolution 1.0 or higher in game.') }}</li>
          <li>{{ label('由 SSMT 直接启动游戏，关闭手动启动和官方启动器模式。', 'Launch directly through SSMT; turn off manual and official-launcher modes.') }}</li>
        </ul>
        <p>{{ label('组件配置成功仅代表安装完成，NR 效果需进入游戏验证。', 'Configured components indicate installation only; verify NR rendering in game.') }}</p>
      </div>
      <details class="graphics-source">
        <summary>{{ label('组件来源与安装位置', 'Component source and installation') }}</summary>
        <p>{{ label('来自 CXP GIMI 集成项目（GitHub）及其 Runtime 包（Google Drive），按固定版本和哈希校验。第三方实验组件的兼容性仍取决于游戏与显卡。', 'From CXP GIMI integration (GitHub) and its runtime package (Google Drive), verified against pinned versions and hashes. Compatibility of these experimental components depends on the game and GPU.') }}</p>
        <p>{{ label('使用 SSMT Runtime 热注入，不向游戏目录放置代理 DLL。', 'Uses SSMT Runtime hot injection without game-local proxy DLLs.') }}</p>
        <code v-if="hotRuntime">{{ hotRuntime.root }}</code>
        <el-button link type="primary" size="small" @click="openComponentSource">{{ label('查看组件来源', 'View component source') }}</el-button>
      </details>
    </template>
    <div v-if="dlss5State?.state === 'managed'" class="graphics-hint"
      :class="{ 'graphics-error': dlss5LastRun?.verdict === 'inactive' }">
      {{ t('gameSettingsModal.fields.graphicsInstalledOnly') }}
      <template v-if="dlss5LastRun">
        · {{ t(`gameSettingsModal.fields.graphicsRuntime_${dlss5LastRun.reason}`) }}
        <span v-if="dlss5LastRun.observedAtMs">({{ new Date(dlss5LastRun.observedAtMs).toLocaleString() }})</span>
      </template>
    </div>
  </section>
</template>

<style scoped>
.dlss5-graphics-settings { display: flex; flex-direction: column; gap: 16px; color: var(--t-page-text); min-width: 0; }
.graphics-heading, .graphics-status, .graphics-actions, .graphics-download-heading { display: flex; align-items: center; gap: 10px; }
.graphics-heading { justify-content: space-between; }
.graphics-heading h3 { margin: 0; font-size: 14px; font-weight: 600; }
.graphics-status { padding: 16px; border: 1px solid rgba(var(--theme-surface-tint-rgb), .10); border-radius: 9px; background: var(--t-surface-soft); }
.graphics-status > div { min-width: 0; }
.graphics-status strong { font-size: 13px; font-weight: 600; line-height: 1.6; overflow-wrap: anywhere; }
.graphics-status p { margin: 5px 0 0; color: var(--t-page-text-muted); font-size: 12px; line-height: 1.6; }
.graphics-status-icon { display: grid; place-items: center; flex: none; width: 34px; height: 34px; border-radius: 8px; background: var(--t-surface-raised); color: var(--t-page-text-muted); }
.graphics-status-icon svg { width: 18px; height: 18px; }
.is-configured .graphics-status-icon { color: var(--t-success-text); background: var(--t-success-bg); }
.graphics-setup { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 16px; }
.graphics-controls { display: flex; flex-wrap: wrap; gap: 14px; flex: 1 1 230px; }
.graphics-field { display: flex; flex-direction: column; gap: 8px; flex: 1 1 180px; max-width: 300px; min-width: 0; font-size: 12px; color: rgba(var(--theme-text-secondary-rgb), .85); }
.graphics-field :deep(.el-select) { width: 100%; }
.graphics-actions { flex-wrap: wrap; }
.graphics-actions :deep(.el-button + .el-button) { margin-left: 0; }
.graphics-hint, .graphics-source, .graphics-startup { font-size: 12px; line-height: 1.8; color: rgba(var(--theme-text-secondary-rgb), .82); }
.graphics-hint { margin: 0; }
.graphics-startup { padding: 14px 16px; border-left: 2px solid rgba(var(--theme-surface-tint-rgb), .20); background: var(--t-surface-subtle); border-radius: 0 6px 6px 0; }
.graphics-startup h4 { margin: 0 0 6px; font-size: 12px; font-weight: 600; color: var(--t-page-text); }
.graphics-startup ul { padding-left: 17px; margin: 0; }
.graphics-startup li + li { margin-top: 3px; }
.graphics-startup p { margin: 8px 0 0; color: var(--t-page-text-muted); font-size: 11px; }
.graphics-source { border-top: 1px solid rgba(var(--theme-surface-tint-rgb), .10); padding-top: 12px; }
.graphics-source summary { cursor: pointer; width: fit-content; }
.graphics-source summary:focus-visible { outline: 2px solid var(--t-page-accent); outline-offset: 3px; }
.graphics-source p { margin: 10px 0; }
.graphics-source code { display: block; margin-bottom: 10px; overflow-wrap: anywhere; user-select: text; color: var(--t-page-text-muted); font: 11px/1.7 ui-monospace, Consolas, monospace; }
.graphics-error { margin: 0; padding: 10px 12px; border: 1px solid var(--t-warning-border); border-radius: 6px; background: var(--t-warning-bg); color: var(--t-warning-text); font-size: 12px; line-height: 1.7; overflow-wrap: anywhere; }
.graphics-download { padding: 14px; border: 1px solid rgba(var(--theme-surface-tint-rgb), .12); border-radius: 8px; background: var(--t-surface-soft); }
.graphics-download-heading { justify-content: space-between; flex-wrap: wrap; margin-bottom: 10px; font-size: 11px; color: var(--t-page-text-muted); font-variant-numeric: tabular-nums; }
.graphics-download-heading strong { color: var(--t-page-text); font-weight: 500; font-size: 12px; }
</style>
