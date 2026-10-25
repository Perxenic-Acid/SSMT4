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
const hotRuntime = ref<{ root: string; profile: string; route?: string } | null>(null);
const hotRuntimes = ref<Record<string, { root: string; profile: string; route?: string }>>({});
interface HotHistoryEntry { root: string; profile: string; route: string; revision: string; current: boolean; usable: boolean; reason?: string | null; removable?: boolean; cleanupReason?: string | null }
const hotHistory = ref<HotHistoryEntry[]>([]);
const historySelection = ref('');
const selectedHistory = () => hotHistory.value.find(item => item.root === historySelection.value);
const hotProfile = ref('rtx50');
const downloadProgress = ref<{ phase: string; downloadedBytes: number; totalBytes: number } | null>(null);
const isGimi = () => /(?:^|[\\/])(?:YuanShen|GenshinImpact)\.exe$/i.test(props.targetExePath.trim());
const isOtherHot = () => /(?:^|[\\/])(?:StarRail|Client-Win64-Shipping|ZenlessZoneZero)\.exe$/i.test(props.targetExePath.trim());
const isHotRoute = (route: string) => route === 'gimi-hot' || route.startsWith('ssmt-');
const routeLabel = (route: string) => ({
  'gimi-hot': label('原神 · FSR2 Bridge', 'Genshin · FSR2 bridge'),
  'ssmt-feeder': label('Feeder · 合成输入', 'Feeder · synthetic input'),
  'ssmt-native-bridge': label('原生 DLSS · DX11 Bridge', 'Native DLSS · DX11 bridge'),
  'ssmt-native': label('原生 DLSS · DX12', 'Native DLSS · DX12'),
}[route] || route);
const historyRoute = (item: HotHistoryEntry) => item.route === 'gimi-hot' ? item.route : `ssmt-${item.route}`;
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
  reason: 'd3d11_interposer_race' | 'feeder_stopped' | 'bridge_stopped' | 'nr_evaluated' | 'feature18_evaluated' | 'no_dlss_create' | 'no_nr_evidence' | 'no_log' | 'log_precedes_install';
  observedAtMs: number | null;
  warning: 'depth_source_mismatch' | null;
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
const graphicsRoutesLoading = ref(false);
let graphicsRequestId = 0;
let graphicsTimer: ReturnType<typeof setTimeout> | undefined;

const invalidateGraphicsRead = () => {
  ++graphicsRequestId;
  graphicsStateLoading.value = false;
  graphicsRoutesLoading.value = false;
};

const refreshGraphicsState = async () => {
  const target = props.targetExePath.trim();
  const gameName = props.gameName;
  const apiOverride = graphicsApiOverride.value;
  const initialProfile = hotProfile.value;
  const gimi = isGimi();
  const hotGame = gimi || isOtherHot();
  const requestId = ++graphicsRequestId;
  const stillCurrent = () => requestId === graphicsRequestId && props.gameName === gameName
    && props.targetExePath.trim() === target && graphicsApiOverride.value === apiOverride;
  if (graphicsTimer) clearTimeout(graphicsTimer);
  graphicsTimer = undefined;
  dlss5State.value = null;
  dlss5LastRun.value = null;
  graphicsStatusError.value = '';
  availableGraphicsRoutes.value = [];
  graphicsRouteScan.value = null;
  hotRuntime.value = null;
  hotRuntimes.value = {};
  hotHistory.value = [];
  historySelection.value = '';
  graphicsStateLoading.value = false;
  graphicsRoutesLoading.value = false;
  if (!target) return;
  graphicsStateLoading.value = true;
  graphicsRoutesLoading.value = true;
  const read = async <T,>(command: string, args: Record<string, string>, apply: (value: T) => void) => {
    try {
      const value = await invoke<T>(command, args);
      if (stillCurrent()) apply(value);
    } catch (error) {
      // 损坏或读取失败仅影响该项，其他路线和重新下载入口仍可使用。
      if (stillCurrent()) graphicsStatusError.value = String(error);
    }
  };
  const applyHotProfile = () => {
    // 慢速校验返回时，不覆盖玩家在等待期间选择的显卡系列。
    if (hotRuntime.value && hotProfile.value === initialProfile) hotProfile.value = hotRuntime.value.profile;
  };
  // 路线、组件校验与历史互不等待；只有路线未就绪时禁用配置操作。
  const routes = read<Dlss5RouteScan>('inspect_dlss5_route_options', {
    gameName, gameExecutable: target, apiOverride,
  }, routes => {
    graphicsRouteScan.value = routes;
    availableGraphicsRoutes.value = routes.routes;
    if (!routes.routes.includes(graphicsSelectedRoute.value)) graphicsSelectedRoute.value = routes.installedRoute && routes.routes.includes(routes.installedRoute) ? routes.installedRoute : routes.routes[0] || '';
    if (gimi) hotRuntime.value = hotRuntimes.value[graphicsSelectedRoute.value] || null;
    applyHotProfile();
  }).finally(() => { if (stillCurrent()) graphicsRoutesLoading.value = false; });
  const commands = hotGame ? (gimi ? ['gimi_hot_runtime_status', 'dlss5_hot_runtime_status'] : ['dlss5_hot_runtime_status']) : [];
  const state = Promise.all([
    read<Dlss5StateSnapshot>('inspect_dlss5_game_state', { gameExecutable: target }, state => {
      dlss5State.value = state;
      if (state.state === 'broken') graphicsStatusError.value = state.reason || '';
    }),
    ...commands.map(command => read<{ root: string; profile: string; route?: string } | null>(command, { gameName }, hot => {
      if (!hot) return;
      const route = command === 'gimi_hot_runtime_status' ? 'gimi-hot' : `ssmt-${hot.route || (gimi ? 'feeder' : '')}`;
      hotRuntimes.value[route] = hot;
      if (!gimi || route === graphicsSelectedRoute.value) hotRuntime.value = hot;
      applyHotProfile();
    })),
  ]).then(async () => {
    if (stillCurrent() && (dlss5State.value?.state === 'managed' || Object.keys(hotRuntimes.value).length)) {
      await read<Dlss5LastRunStatus>('inspect_dlss5_last_run', { gameExecutable: target, gameName }, lastRun => { dlss5LastRun.value = lastRun; });
    }
  });
  const history = hotGame
    ? read<HotHistoryEntry[]>('dlss5_hot_runtime_history', { gameName }, history => {
        hotHistory.value = history;
        historySelection.value = history.find(item => item.current)?.root || history[0]?.root || '';
      })
    : Promise.resolve();
  await Promise.all([routes, state, history]);
  if (stillCurrent()) graphicsStateLoading.value = false;
};

const installSelectedDlss5Route = async (offline = false) => {
  if (graphicsActionBusy.value || graphicsRoutesLoading.value) return;
  const gameName = props.gameName;
  const gameExecutable = props.targetExePath.trim();
  const route = graphicsSelectedRoute.value;
  const profile = hotProfile.value;
  const stillCurrent = () => props.gameName === gameName && props.targetExePath.trim() === gameExecutable;
  if (!gameExecutable || !route) return;
  invalidateGraphicsRead();
  if (route === 'optiscaler') {
    try { await invoke('open_dlss5_swapper', { gameName: props.gameName }); }
    catch (error) { ElMessage.error(String(error)); }
    return;
  }
  graphicsActionBusy.value = true;
  graphicsStatusError.value = '';
  downloadProgress.value = null;
  try {
    if (isHotRoute(route)) {
      if (offline && route === 'gimi-hot') {
        const directory = await open({ directory: true, multiple: false, title: label('选择 GIMI 热注入组件包目录', 'Select the GIMI hot-injection component package') });
        if (typeof directory !== 'string' || !stillCurrent()) return;
        await invoke('import_gimi_hot_runtime', { gameName, gameExecutable, sourceDirectory: directory, profile });
      } else {
        const onProgress = new Channel<{ phase: string; downloadedBytes: number; totalBytes: number }>();
        onProgress.onmessage = event => { if (stillCurrent()) downloadProgress.value = event; };
        await invoke(route === 'gimi-hot' ? 'download_gimi_hot_runtime' : 'download_dlss5_hot_runtime', { gameName, gameExecutable, profile, route, onProgress });
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
  if (graphicsActionBusy.value) return;
  const gameExecutable = props.targetExePath.trim();
  if (!gameExecutable) return;
  invalidateGraphicsRead();
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

const manageHotHistory = async (remove = false) => {
  if (graphicsActionBusy.value || !historySelection.value) return;
  const selected = selectedHistory();
  if (!selected || (remove ? selected.removable === false : !selected.usable)) return;
  const gameName = props.gameName, gameExecutable = props.targetExePath.trim(), root = historySelection.value;
  if (!gameExecutable) return;
  invalidateGraphicsRead();
  graphicsActionBusy.value = true;
  try {
    if (remove) {
      try {
        await ElMessageBox.confirm(label(`清理这套热组件及其配置、预设和日志？\n${root}\n游戏目录和下载缓存将保留。`, `Remove this hot installation, including its settings, presets and logs?\n${root}\nGame files and download cache are retained.`), label('清理热组件', 'Remove hot components'), { type: 'warning' });
      } catch { return; }
    }
    if (props.gameName !== gameName || props.targetExePath.trim() !== gameExecutable) return;
    await invoke(remove ? 'remove_dlss5_hot_runtime' : 'select_dlss5_hot_runtime', { gameName, gameExecutable, root });
    ElMessage.success(remove ? label('热组件已清理', 'Hot components removed') : label('已选择这套组件，下次启动生效', 'Selected components apply on the next launch'));
    if (props.gameName === gameName && props.targetExePath.trim() === gameExecutable) {
      if (!remove) graphicsSelectedRoute.value = historyRoute(selected);
      await refreshGraphicsState();
    }
  } catch (error) {
    if (props.gameName === gameName) graphicsStatusError.value = String(error);
    ElMessage.error(String(error));
  } finally { graphicsActionBusy.value = false; }
};

watch(() => [props.gameName, props.targetExePath, graphicsApiOverride.value], () => {
  invalidateGraphicsRead();
  hotRuntime.value = null;
  hotRuntimes.value = {};
  hotHistory.value = [];
  historySelection.value = '';
  dlss5State.value = null;
  dlss5LastRun.value = null;
  availableGraphicsRoutes.value = [];
  graphicsRouteScan.value = null;
  graphicsSelectedRoute.value = '';
  graphicsStateLoading.value = !!props.targetExePath.trim();
  graphicsRoutesLoading.value = !!props.targetExePath.trim();
  graphicsStatusError.value = '';
  downloadProgress.value = null;
  if (graphicsTimer) clearTimeout(graphicsTimer);
  graphicsTimer = setTimeout(() => void refreshGraphicsState(), 350);
}, { immediate: true });

watch(graphicsSelectedRoute, route => {
  if (isGimi()) {
    hotRuntime.value = hotRuntimes.value[route] || null;
    if (hotRuntime.value && !graphicsStateLoading.value) hotProfile.value = hotRuntime.value.profile;
  }
});

onUnmounted(() => {
  invalidateGraphicsRead();
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
        <strong>{{ graphicsStateLoading ? label('正在读取组件状态…', 'Checking components…')
          : dlss5LastRun?.verdict === 'inactive' ? label('上次运行：神经渲染未生效', 'Last run: neural rendering inactive')
          : dlss5LastRun?.verdict === 'engaged' ? label('上次运行：已确认神经渲染执行', 'Last run: neural rendering evaluation confirmed') : hotRuntime
          ? label('热注入组件已配置', 'Hot-injection components configured') : dlss5State?.state === 'managed'
            ? t('gameSettingsModal.fields.graphicsManagedState', { route: dlss5State.route || '—', owner: dlss5State.owner || 'DLSS5-Swapper' })
            : dlss5State?.state === 'broken' ? t('gameSettingsModal.fields.graphicsBrokenState')
              : label('尚未配置组件', 'Components not configured') }}</strong>
        <p v-if="hotRuntime">{{ hotRuntime.profile === 'rtx50' ? 'RTX 50' : 'RTX 30 / 40' }} · {{ hotRuntime.route ? routeLabel(`ssmt-${hotRuntime.route}`) : routeLabel(graphicsSelectedRoute) }} · {{ label('已配置组件不代表渲染已生效', 'Configured components do not confirm rendering is active') }}</p>
        <p v-else-if="isGimi()">{{ label('选择显卡后，自动下载并部署完整组件。', 'Choose your GPU to download and deploy the complete components.') }}</p>
        <p v-else>{{ label('选择适合当前游戏的渲染路线。', 'Choose a rendering route for this game.') }}</p>
      </div>
    </div>
    <p v-if="graphicsStatusError" class="graphics-error" role="alert">{{ graphicsStatusError }}</p>

    <div class="graphics-setup">
      <div class="graphics-controls">
        <label v-if="isGimi() || isOtherHot()" class="graphics-field">
          <span>{{ label('显卡系列', 'GPU series') }}</span>
          <el-select v-model="hotProfile" :disabled="graphicsActionBusy" :aria-label="label('显卡系列', 'GPU series')">
            <el-option value="rtx50" label="RTX 50" />
            <el-option value="rtx30" :label="label('RTX 30 / 40（40 为实验支持）', 'RTX 30 / 40 (40 experimental)')" />
          </el-select>
        </label>
          <label v-if="!isGimi()" class="graphics-field">
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
              <el-option v-for="route in availableGraphicsRoutes" :key="route" :value="route" :label="routeLabel(route)" />
            </el-select>
          </label>
      </div>
      <div class="graphics-actions">
        <el-button type="primary" :icon="Download" :loading="graphicsActionBusy" :disabled="!graphicsSelectedRoute || graphicsRoutesLoading" @click="installSelectedDlss5Route()">
          {{ isHotRoute(graphicsSelectedRoute) ? label('自动下载并配置', 'Download and configure') : graphicsSelectedRoute === 'optiscaler' ? t('gameSettingsModal.actions.openSwapper') : dlss5State?.state === 'managed' && !dlss5State.managedHost ? t('gameSettingsModal.actions.migrateGraphicsRoute') : t('gameSettingsModal.actions.installGraphicsRoute') }}
        </el-button>
        <el-button v-if="graphicsSelectedRoute === 'gimi-hot'" :disabled="graphicsActionBusy || !graphicsSelectedRoute || graphicsRoutesLoading" @click="installSelectedDlss5Route(true)">
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
    <div v-if="hotHistory.length" class="graphics-startup">
      <h4>{{ label('热组件历史', 'Hot installation history') }}</h4>
      <p>{{ label('关闭游戏后可选择历史组件及对应实现，或清理当前实例的自有副本。清理正在使用的组件后，需要重新选择或配置才能启用 DLSS5。', 'Close the game to select a previous installation and its implementation, or remove an owned copy for this instance. After removing active components, select or configure another installation to use DLSS5.') }}</p>
      <div class="graphics-actions">
        <el-select v-model="historySelection" :disabled="graphicsActionBusy" :aria-label="label('热组件历史', 'Hot installation history')">
          <el-option v-for="item in hotHistory" :key="item.root" :value="item.root"
            :label="`${item.current ? '● ' : ''}${routeLabel(historyRoute(item))} · ${item.profile} · ${item.revision.slice(0, 14)} · ${item.root.split(/[\\/]/).pop()}${item.usable ? '' : label('（需修复）', ' (needs repair)')}`" />
        </el-select>
        <el-button :disabled="graphicsActionBusy || !hotHistory.some(item => item.root === historySelection && item.usable)" @click="manageHotHistory()">{{ label('使用所选组件', 'Use selected components') }}</el-button>
        <el-button :disabled="graphicsActionBusy || !historySelection || selectedHistory()?.removable === false" @click="manageHotHistory(true)">{{ label('清理所选组件', 'Remove selected components') }}</el-button>
      </div>
      <p v-if="selectedHistory()" class="graphics-path">{{ selectedHistory()?.root }}</p>
      <p v-if="selectedHistory()?.reason">{{ selectedHistory()?.reason }}</p>
      <p v-if="selectedHistory()?.cleanupReason">{{ selectedHistory()?.cleanupReason }}</p>
    </div>
    <template v-if="graphicsSelectedRoute === 'gimi-hot'">
      <p class="graphics-hint">{{ label('首次下载约 302 MiB。请先关闭游戏；下载失败保留原组件，普通游戏启动仍可使用。', 'First download: about 302 MiB. Close the game before setup. Failed downloads preserve existing components and normal game launch remains available.') }}</p>
      <div class="graphics-startup">
        <h4>{{ label('启动前', 'Before launching') }}</h4>
        <ul>
          <li>{{ label('SSMT 启动前会自动启用游戏内 FSR2；渲染精度至少 1.0。', 'SSMT automatically enables in-game FSR2 before launch; use render resolution 1.0 or higher.') }}</li>
          <li>{{ label('保持“启动目标程序”开启；使用 Shell 启动时，将启动路径设为同一个游戏程序。', 'Keep Launch target program enabled. For Shell launch, set the launch path to the same game executable.') }}</li>
        </ul>
        <p>{{ label('组件配置成功仅代表安装完成，NR 效果需进入游戏验证。', 'Configured components indicate installation only; verify NR rendering in game.') }}</p>
      </div>
      <div class="graphics-startup">
        <h4>{{ label('游戏内控制', 'In-game controls') }}</h4>
        <ul>
          <li>{{ label('Home：打开 ReShade，在附加组件中调整 DLSS5 神经渲染。F6：开关神经渲染。', 'Home: open ReShade and adjust DLSS5 Neural Rendering under Add-ons. F6: toggle Neural Rendering.') }}</li>
          <li>{{ label('Insert：打开 OptiScaler，调整超分与界面设置。请保持其中的“启用神经渲染”关闭，避免额外运行另一套 NR。', 'Insert: open OptiScaler for upscaling and interface settings. Keep its Enable Neural Rendering option off to avoid running another NR implementation.') }}</li>
        </ul>
        <p>{{ label('可展开启动浮窗中的控制台详情，查看 FSR2 Bridge 与 DLSS5 NR 的本次运行日志。', 'Expand the launch console details to view this session’s FSR2 Bridge and DLSS5 NR logs.') }}</p>
      </div>
      <details class="graphics-source">
        <summary>{{ label('组件来源与安装位置', 'Component source and installation') }}</summary>
        <p>{{ label('来自 CXP GIMI 集成项目（GitHub）及其 Runtime 包（Google Drive），按固定版本和哈希校验。第三方实验组件的兼容性仍取决于游戏与显卡。', 'From CXP GIMI integration (GitHub) and its runtime package (Google Drive), verified against pinned versions and hashes. Compatibility of these experimental components depends on the game and GPU.') }}</p>
        <p>{{ label('使用 SSMT Runtime 热注入，不向游戏目录放置代理 DLL。', 'Uses SSMT Runtime hot injection without game-local proxy DLLs.') }}</p>
        <code v-if="hotRuntime">{{ hotRuntime.root }}</code>
        <el-button link type="primary" size="small" @click="openComponentSource">{{ label('查看组件来源', 'View component source') }}</el-button>
      </details>
    </template>
    <div v-if="isOtherHot() || graphicsSelectedRoute === 'ssmt-feeder'" class="graphics-startup">
      <h4>{{ label('启动前', 'Before launching') }}</h4>
      <p>{{ graphicsSelectedRoute === 'ssmt-feeder'
        ? label('使用 Feeder 与 Lumenite 合成运动矢量，无需游戏原生 DLSS。原神启动前会自动关闭游戏抗锯齿，避免与 FSR2 重复重建；可与 FSR2 Bridge 路线分别比较画质与性能。', 'Feeder and Lumenite synthesize motion vectors without native DLSS. SSMT automatically disables in-game anti-aliasing in Genshin before launch to avoid duplicate reconstruction; compare quality and performance with the FSR2 bridge route.')
        : label('请在游戏内开启 DLSS；DX11 / DX12 必须与当前游戏预设一致。', 'Enable DLSS in game and match the DX11 / DX12 game preset.') }}</p>
      <p>{{ label('请先关闭游戏，再自动下载。组件位于 SSMT 自有目录，通过 SSMT 直接启动时热注入；若已有 Swapper 或 RHI / ReShade 安装，请先通过原管理器恢复，避免重复加载 Host。', 'Close the game before setup. Components live in SSMT storage and are hot-injected on direct launch. Restore any previous Swapper or RHI / ReShade installation through its manager to avoid duplicate hosts.') }}</p>
      <p>{{ label('组件按固定版本、大小及哈希校验，来自 ReShade、Feeder、DX11 Bridge、LumeniteFX 和 RHI 组件库。配置完成后仍需游戏内确认 NR 生效。', 'Pinned components are verified by size and hash, from ReShade, Feeder, DX11 Bridge, LumeniteFX and the RHI component repository. Verify NR rendering in game after setup.') }}</p>
      <code v-if="hotRuntime">{{ hotRuntime.root }}</code>
    </div>
    <div v-if="graphicsSelectedRoute === 'ssmt-feeder'" class="graphics-startup">
      <h4>{{ label('Feeder 游戏内控制', 'Feeder in-game controls') }}</h4>
      <p>{{ label('按 Home 打开 ReShade：在效果页启用 DLSS5_Feed，在附加组件页调整 DLSS5 Feeder 和神经渲染。先选择与主画面匹配的深度缓冲，再比较开启与关闭的画面；无需开启游戏原生 DLSS。', 'Press Home to open ReShade. Enable DLSS5_Feed under Effects and adjust DLSS5 Feeder and Neural Rendering under Add-ons. Select the depth buffer matching the main view before comparing on/off output. Native in-game DLSS is not required.') }}</p>
    </div>
    <p v-if="isGimi()" class="graphics-hint">{{ label('自动下载并配置成功后，此游戏下次启动使用所选实现；切换失败保留原路线。', 'Successful setup selects this implementation for the next launch; a failed setup keeps the previous route.') }}</p>
    <div v-if="dlss5State?.state === 'managed' || hotRuntime" class="graphics-hint"
      :class="{ 'graphics-error': dlss5LastRun?.verdict === 'inactive' }">
      {{ t('gameSettingsModal.fields.graphicsInstalledOnly') }}
      <template v-if="dlss5LastRun">
        · {{ t(`gameSettingsModal.fields.graphicsRuntime_${dlss5LastRun.reason}`) }}
        <span v-if="dlss5LastRun.observedAtMs">({{ new Date(dlss5LastRun.observedAtMs).toLocaleString() }})</span>
      </template>
    </div>
    <p v-if="dlss5LastRun?.warning === 'depth_source_mismatch'" class="graphics-error" role="status">
      {{ t('gameSettingsModal.fields.graphicsRuntime_depth_source_mismatch') }}
    </p>
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
