<script setup lang="ts">
import { h, ref, watch, reactive, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessage, ElMessageBox } from 'element-plus';
import { join } from '@tauri-apps/api/path';
import { AppStateManager } from '../store/AppStateManager';
import { ResourceManager, type UpdateInfo } from '../store/ResourceManager';
import { PathHelper } from '../helper/PathHelper';
import { GlobalConfig } from '../store/GlobalConfig';
import { AUTO_UPDATE_SUPPORTED_PRESET_SET, getGamePresetDisplayName, getGamePresetOptions, getGithubRepoByGamePreset } from '../store/GamePreset';
import { GameConfig, GameConfigManager, getDefaultUseUpxForGamePreset, type D3d11Mode, type LaunchProgramConfig } from '../store/GameConfig';
import { openPath, openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { exists, mkdir } from '@tauri-apps/plugin-fs';
import { useI18n } from 'vue-i18n';
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { MigotoManager } from '../store/MigotoManager';
import { debugLog } from '../utils/debugLog';
import ReleaseNotesMarkdown from './ReleaseNotesMarkdown.vue';

const appSettings = AppStateManager.appSettings;
const gamesList = AppStateManager.gamesList;
const loadGames = AppStateManager.loadGames.bind(AppStateManager);
const { t } = useI18n();

const props = defineProps<{
  modelValue: boolean;
  gameName: string;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void;
}>();

// Config State
const config = reactive<GameConfig>(GameConfigManager.defaultGameConfig());
let loadConfigRequestId = 0;


const isLoading = ref(false);
const isDllReleaseLoading = ref(false);
const dllReleaseList = ref<UpdateInfo[]>([]);
const dllReleaseListError = ref('');
const dllReleaseListLoaded = ref(false);
const installingDllVersion = ref('');
const isPackageReleaseLoading = ref(false);
const autoMatchingExeField = ref<'targetExePath' | 'launcherExePath' | null>(null);
const packageReleaseList = ref<UpdateInfo[]>([]);
const packageReleaseListError = ref('');
const packageReleaseListLoaded = ref(false);
const installingPackageVersion = ref('');
const DLL_RELEASE_PAGE_SIZE = 10;
const dllVisibleCount = ref(DLL_RELEASE_PAGE_SIZE);
const PACKAGE_RELEASE_PAGE_SIZE = 10;
const RELEASE_NOTES_MARKER = '__SSMT_RELEASE_NOTES_MARKER__';

const buildReleaseNotesConfirmMessage = (messageKey: string, version: string, description: string) => {
  const message = t(messageKey, {
    version,
    description: RELEASE_NOTES_MARKER,
  });
  const markerIndex = message.indexOf(RELEASE_NOTES_MARKER);

  if (markerIndex < 0) {
    return h(ReleaseNotesMarkdown, { content: message, compact: true });
  }

  const before = message.slice(0, markerIndex).trimEnd();
  const after = message.slice(markerIndex + RELEASE_NOTES_MARKER.length).trimStart();
  const children = [];

  if (before) {
    children.push(h('div', { class: 'release-notes-confirm-text' }, before));
  }

  children.push(h(ReleaseNotesMarkdown, {
    content: description,
    compact: true,
  }));

  if (after) {
    children.push(h('div', { class: 'release-notes-confirm-text' }, after));
  }

  return h('div', { class: 'release-notes-confirm' }, children);
};
const packageVisibleCount = ref(PACKAGE_RELEASE_PAGE_SIZE);

// Tabs
const activeTab = ref('basic');
const tabs = computed(() => [
  { id: 'basic', label: t('gameSettingsModal.tabs.basic') },
  { id: '3dmigoto', label: t('gameSettingsModal.tabs.migoto') },
  { id: 'dllUpdate', label: t('gameSettingsModal.tabs.dllUpdate') },
  { id: 'packageUpdate', label: t('gameSettingsModal.tabs.packageUpdate') },
  { id: 'other', label: t('gameSettingsModal.tabs.other') },
]);

const isForcedSsiceADllMode = computed(() => (config.gamePreset || '').trim().toUpperCase() === 'NTEMI');
const isWWMIPreset = computed(() => (config.gamePreset || '').trim().toUpperCase() === 'WWMI');
const currentDllMode = computed<D3d11Mode>(() => ResourceManager.getEffectiveD3d11Mode(config));
const CAPPED_DEV_D3D11_PRESETS = new Set(['IDENTITYV', 'NARAKA', 'NARAKAM']);
const isCappedDevD3d11Mode = (mode: D3d11Mode): boolean => (
  mode === 'dev' && CAPPED_DEV_D3D11_PRESETS.has((config.gamePreset || '').trim().toUpperCase())
);
const isCappedDevD3d11ModeRestricted = computed(() => isCappedDevD3d11Mode(currentDllMode.value));
const getStoredDllVersion = (mode: D3d11Mode): string => {
  if (isCappedDevD3d11Mode(mode)) {
    return (appSettings.coreVersionIdentityVDev || '').trim();
  }
  if (mode === 'play') {
    return (appSettings.coreVersionPlay || '').trim();
  }

  if (mode === 'ssice-a') {
    return (appSettings.coreVersionSsiceA || '').trim();
  }

  return (appSettings.coreVersionDev || appSettings.coreVersion || '').trim();
};

const setStoredDllVersion = (mode: D3d11Mode, info: UpdateInfo) => {
  if (isCappedDevD3d11Mode(mode)) {
    appSettings.coreVersionIdentityVDev = info.version;
    appSettings.coreReleaseDescriptionIdentityVDev = info.description;
  } else if (mode === 'play') {
    appSettings.coreVersionPlay = info.version;
    appSettings.coreReleaseDescriptionPlay = info.description;
  } else if (mode === 'ssice-a') {
    appSettings.coreVersionSsiceA = info.version;
    appSettings.coreReleaseDescriptionSsiceA = info.description;
  } else {
    appSettings.coreVersionDev = info.version;
    appSettings.coreReleaseDescriptionDev = info.description;
  }

  // These legacy fields mirror the shared DLL selection. Capped presets own a
  // separate cache and must not change the version seen by other games.
  if (!isCappedDevD3d11Mode(mode)) {
    appSettings.coreVersion = info.version;
    appSettings.coreReleaseDescription = info.description;
  }
};

const currentCoreVersion = computed(() => getStoredDllVersion(currentDllMode.value));
const dllSourceOptions = computed(() => {
  const options = [
    { value: 'dev' as const, label: t('gameSettingsModal.options.d3d11Source.dev') },
    { value: 'play' as const, label: t('gameSettingsModal.options.d3d11Source.play') },
    { value: 'ssice-a' as const, label: t('gameSettingsModal.options.d3d11Source.ssiceA') },
  ];

  return isForcedSsiceADllMode.value
    ? options.filter(item => item.value === 'ssice-a')
    : options;
});
const visibleDllReleaseList = computed(() => dllReleaseList.value.slice(0, dllVisibleCount.value));
const hasMoreDllReleases = computed(() => dllReleaseList.value.length > dllVisibleCount.value);
const currentPackageVersion = computed(() => (config.packageVersion || '').trim());
const visiblePackageReleaseList = computed(() => packageReleaseList.value.slice(0, packageVisibleCount.value));
const hasMorePackageReleases = computed(() => packageReleaseList.value.length > packageVisibleCount.value);
const currentPackageRepo = computed(() => getGithubRepoByGamePreset(config.gamePreset || '') || '');
const currentPackageRepoUrl = computed(() => currentPackageRepo.value ? `https://github.com/${currentPackageRepo.value}` : '');

const launchMode = computed({
  get: (): 'normal' | 'shell' => {
    return config.useShell ? 'shell' : 'normal';
  },
  set: (val: 'normal' | 'shell') => {
    if (val === 'shell') {
      config.useShell = true;
      config.extraDll = '';
      config.extraDlls = [];
    } else {
      config.useShell = false;
    }
    saveConfig();
  },
});

const launchModeOptions = computed(() => [
  { value: 'normal' as const, label: t('gameSettingsModal.fields.launchModeNormal') },
  { value: 'shell' as const, label: t('gameSettingsModal.fields.launchModeShell') },
]);

const presetOptions = computed(() => getGamePresetOptions(t));
const huntingModeOptions = computed(() => [
  { value: '0', label: t('gameSettingsModal.options.huntingMode.off') },
  { value: '1', label: t('gameSettingsModal.options.huntingMode.on') },
  { value: '2', label: t('gameSettingsModal.options.huntingMode.toggleByNumpad0') },
]);

// Load/Save Logic
const loadConfig = async () => {
  const gameName = props.gameName?.trim();
  const requestId = ++loadConfigRequestId;

  if (!gameName) {
    Object.assign(config, GameConfigManager.defaultGameConfig());
    return;
  }

  isLoading.value = true;
  try {
    const data = await ResourceManager.loadGameConfig(gameName);
    if (requestId !== loadConfigRequestId || gameName !== props.gameName?.trim()) {
      return;
    }

    Object.assign(config, data);
  } catch (e) {
    console.error('Failed to load game config:', e);
  } finally {
    if (requestId === loadConfigRequestId) {
      isLoading.value = false;
    }
  }
};

const saveConfig = async () => {
  if (!props.gameName || isLoading.value) return; // Prevent saving if loading isn't complete
  debugLog('GameSettingsModal', `Starting saveConfig for: ${props.gameName}`);
  debugLog('GameSettingsModal', 'Current config state:', JSON.parse(JSON.stringify(config)));

  try {
    const configToSave: GameConfig = JSON.parse(JSON.stringify(config));
    await ResourceManager.saveGameConfig(props.gameName, configToSave);
    debugLog('GameSettingsModal', 'Current config saved.');
  } catch (e) {
    console.error('Failed to save current config:', e);
  }
};

const handleDllPackingChange = async () => {
  config.useUpxManuallySet = true;
  await saveConfig();
};

const syncShowTopLeftWarningsToIni = async (): Promise<void> => {
  const gameName = props.gameName?.trim();
  if (!gameName) {
    return;
  }

  await saveConfig();

  try {
    const migotoDir = await PathHelper.GetCurrentGame3DmigotoFolderPath();
    if (!migotoDir) {
      ElMessage.warning(t('gameSettingsModal.messages.update3DmigotoPackageBecauseD3dxIniMissing'));
      return;
    }

    const iniPath = await join(migotoDir, 'd3dx.ini');
    if (!(await exists(iniPath))) {
      ElMessage.warning(t('gameSettingsModal.messages.update3DmigotoPackageBecauseD3dxIniMissing'));
      return;
    }

    await MigotoManager.patchD3dxForLaunch(gameName);
    ElMessage.success(t('gameSettingsModal.messages.showTopLeftWarningsAppliedRefreshWithF10'));
  } catch (e) {
    console.error('Failed to sync show_warnings to d3dx.ini:', e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
  }
};

const handleBgTypeChange = async () => {
  await saveConfig();
  // Refresh global state if this is the active game
  if (appSettings.CurrentGameName === props.gameName) {
    await loadGames();
  }
};

const selectIcon = async () => {
  try {
    const file = await open({
      multiple: false,
      filters: [{ name: 'Images', extensions: ['png'] }]
    });

    if (file) {
      await ResourceManager.setGameIcon(props.gameName, file);
      await loadGames();
    }
  } catch (e) {
    console.error(e);
  }
};

const selectBackground = async () => {
  try {
    const isVideo = config.backgroundType === 'Video';
    const filters = isVideo
      ? [{ name: 'Videos', extensions: ['mp4', 'webm', 'ogg', 'mov'] }]
      : [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg', 'bmp', 'ico', 'avif'] }];

    const file = await open({
      multiple: false,
      filters
    });

    if (file) {
      await ResourceManager.setGameBackground(props.gameName, file, (config.backgroundType as "Image" | "Video") || 'Image');
      await loadGames(); // Refresh
    }
  } catch (e) {
    console.error(e);
  }
};

const revealBackgroundFile = async () => {
  try {
    const backgroundPath = await ResourceManager.findGameBackgroundPath(
      props.gameName,
      (config.backgroundType as 'Image' | 'Video') || 'Image',
    );

    if (!backgroundPath || !(await exists(backgroundPath))) {
      ElMessage.warning(t('gameSettingsModal.messages.backgroundFileNotFound'));
      return;
    }

    await revealItemInDir(backgroundPath);
  } catch (e) {
    console.error('Failed to reveal background file:', e);
    ElMessage.error(t('gameSettingsModal.messages.revealBackgroundFileFailed', { error: String(e) }));
  }
};

const canAutoUpdate = computed(() => AUTO_UPDATE_SUPPORTED_PRESET_SET.has(config.gamePreset || ''));

const handleUpdateModeChange = async (mode: string) => {
  await saveConfig();
  // Trigger immediate background update when switching to Auto
  if (mode === 'auto' && canAutoUpdate.value) {
    await autoUpdateBackground();
  }
};

const autoUpdateBackground = async () => {
  try {
    isLoading.value = true;
    await ResourceManager.updateGameBackground(props.gameName, config.gamePreset || '', (config.backgroundType as "Image" | "Video") || 'Image');
    await loadGames();
    ElMessage.success(t('gameSettingsModal.messages.backgroundUpdated'));

    if (appSettings.CurrentGameName === props.gameName) {
      // Force refresh UI if active
      const current = gamesList.find(g => g.name === props.gameName);
      if (current) await AppStateManager.selectGame(current);
    }

  } catch (e) {
    console.error(e);

    ElMessage.error(t('gameSettingsModal.messages.updateFailed', { error: String(e) }));
  } finally {
    isLoading.value = false;
  }
};

// 3Dmigoto Helper Functions
const pick3dmigotoDir = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('gameSettingsModal.actions.select3dmigotoDirectory')
    });
    if (selected && typeof selected === 'string') {
      config.installDir = selected;
    }
  } catch (e) { console.error(e); }
};

const set3dmigotoDirToDefault = async () => {
  const gameName = props.gameName?.trim();
  if (!gameName) {
    return;
  }

  try {
    const cacheRoot = await GlobalConfig.SSMT4CustomCacheFolder();
    config.installDir = await join(cacheRoot, '3Dmigoto', gameName);
    await saveConfig();
  } catch (e) {
    console.error('Failed to set default 3dmigoto path:', e);
  }
};

const open3dmigotoDir = async () => {
  const gameName = props.gameName?.trim();
  const preferredDir = gameName
    ? await PathHelper.GetGame3DmigotoFolderPath(gameName)
    : undefined;

  if (preferredDir) {
    try {
      await mkdir(preferredDir, { recursive: true });
      await openPath(preferredDir);
    } catch (e) {
      console.error('Open directory failed:', e);
    }
  }
};

const openCurrentPackageRepo = async () => {
  if (!currentPackageRepoUrl.value) {
    ElMessage.info(t('gameSettingsModal.messages.presetNotSupported'));
    return;
  }

  try {
    await openUrl(currentPackageRepoUrl.value);
  } catch (e) {
    console.error('Open package repository failed:', e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
  }
};

// ── Blur notification helper ──
/** Show taskbar flash + web notification when window is blurred while a confirm dialog is pending */
const setupBlurNotification = (message: string): (() => void) => {
  let unlisten: (() => void) | null = null;

  const showNotification = () => {
    try { getCurrentWindow().requestUserAttention(UserAttentionType.Informational); } catch { /* ignore */ }

    try {
      if ('Notification' in window) {
        if (Notification.permission === 'granted') {
          new Notification('SSMT', { body: message, icon: '/favicon.ico' });
        } else if (Notification.permission !== 'denied') {
          Notification.requestPermission().then(p => {
            if (p === 'granted') new Notification('SSMT', { body: message, icon: '/favicon.ico' });
          });
        }
      }
    } catch { /* web notification not available */ }
  };

  try {
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) showNotification();
    }).then(fn => { unlisten = fn; });
  } catch { /* window focus listener not available */ }

  return () => {
    if (unlisten) { try { unlisten(); } catch { /* ignore */ } unlisten = null; }
  };
};

const check3DMigotoPackageUpdate = async (): Promise<boolean> => {
  // 1. Initial Confirmation
  const unlisten = setupBlurNotification(t('gameSettingsModal.messages.confirmCheckUpdate', { preset: getGamePresetDisplayName(config.gamePreset, t) }));
  try {
    await ElMessageBox.confirm(
      t('gameSettingsModal.messages.confirmCheckUpdate', { preset: getGamePresetDisplayName(config.gamePreset, t) }),
      t('gameSettingsModal.messages.checkForUpdatesTitle'),
      {
        confirmButtonText: t('gameSettingsModal.common.confirm'),
        cancelButtonText: t('gameSettingsModal.common.cancel'),
        type: 'info'
      }
    );
  } catch {
      return false;
  } finally {
    unlisten();
  }

  try {
    isLoading.value = true;

    // 2. Fetch Info
    const info = await ResourceManager.get3DMigotoLatestRelease(
      config.gamePreset || '',
      appSettings.githubToken,
      config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
    );

    if (!info) {
      ElMessage.info(t('gameSettingsModal.messages.presetNotSupported'));
      return false;
    }

    return await installPackageUpdateFromInfo(info);

  } catch (e) {
    console.error(e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
    return false;
  } finally {
    isLoading.value = false;
  }
};

const resetDllReleaseListState = () => {
  dllReleaseList.value = [];
  dllReleaseListError.value = '';
  dllReleaseListLoaded.value = false;
  dllVisibleCount.value = DLL_RELEASE_PAGE_SIZE;
};

const resetPackageReleaseListState = () => {
  packageReleaseList.value = [];
  packageReleaseListError.value = '';
  packageReleaseListLoaded.value = false;
  packageVisibleCount.value = PACKAGE_RELEASE_PAGE_SIZE;
};

const checkD3D11DllUpdate = async (mode: D3d11Mode = currentDllMode.value): Promise<boolean> => {
  if (config.allowDllUpdates === false) {
    ElMessage.warning(t('gameSettingsModal.messages.dllUpdatesRejected'));
    return false;
  }
  const unlisten = setupBlurNotification(t('gameSettingsModal.messages.confirmCheckDllUpdate'));
  try {
    await ElMessageBox.confirm(
      t('gameSettingsModal.messages.confirmCheckDllUpdate'),
      t('gameSettingsModal.messages.checkForUpdatesTitle'),
      {
        confirmButtonText: t('gameSettingsModal.common.confirm'),
        cancelButtonText: t('gameSettingsModal.common.cancel'),
        type: 'info'
      }
    );
  } catch {
    return false;
  } finally {
    unlisten();
  }

  try {
    isLoading.value = true;

    const info = await ResourceManager.getD3d11LatestRelease(
      mode,
      appSettings.githubToken,
      config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
      config.gamePreset,
    );

    return await installDllUpdateFromInfo(info, mode);
  } catch (e) {
    console.error(e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
    return false;
  } finally {
    isLoading.value = false;
  }
};

const loadDllReleaseList = async (force = false, mode: D3d11Mode = currentDllMode.value) => {
  if (dllReleaseListLoaded.value && !force) {
    return;
  }

  isDllReleaseLoading.value = true;
  dllReleaseListError.value = '';

  try {
    const releases = await ResourceManager.getD3d11ReleaseList(
      mode,
      appSettings.githubToken,
      config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
      config.gamePreset,
    );

    dllReleaseList.value = releases;
    dllReleaseListLoaded.value = true;
    dllVisibleCount.value = DLL_RELEASE_PAGE_SIZE;
  } catch (e) {
    console.error('Failed to load d3d11.dll release list:', e);
    dllReleaseListError.value = String(e);
  } finally {
    isDllReleaseLoading.value = false;
  }
};

const refreshDllReleaseList = async () => {
  await loadDllReleaseList(true, currentDllMode.value);
};

const loadMoreDllReleases = () => {
  dllVisibleCount.value += DLL_RELEASE_PAGE_SIZE;
};

const loadPackageReleaseList = async (force = false) => {
  if (packageReleaseListLoaded.value && !force) {
    return;
  }

  isPackageReleaseLoading.value = true;
  packageReleaseListError.value = '';

  try {
    const releases = await ResourceManager.get3DMigotoReleaseList(
      config.gamePreset || '',
      appSettings.githubToken,
      config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
    );

    if (!releases) {
      packageReleaseList.value = [];
      packageReleaseListLoaded.value = true;
      packageReleaseListError.value = t('gameSettingsModal.messages.presetNotSupported');
      return;
    }

    packageReleaseList.value = releases;
    packageReleaseListLoaded.value = true;
    packageVisibleCount.value = PACKAGE_RELEASE_PAGE_SIZE;
  } catch (e) {
    console.error('Failed to load 3Dmigoto Package release list:', e);
    packageReleaseListError.value = String(e);
  } finally {
    isPackageReleaseLoading.value = false;
  }
};

const refreshPackageReleaseList = async () => {
  await loadPackageReleaseList(true);
};

const loadMorePackageReleases = () => {
  packageVisibleCount.value += PACKAGE_RELEASE_PAGE_SIZE;
};

const installPackageUpdateFromInfo = async (info: UpdateInfo): Promise<boolean> => {
  isLoading.value = false;

  const msg = t('gameSettingsModal.messages.newVersionFound', { version: info.version });
  const unlisten = setupBlurNotification(msg);
  try {
    await ElMessageBox.confirm(
      buildReleaseNotesConfirmMessage('gameSettingsModal.messages.newVersionFound', info.version, info.description),
      t('gameSettingsModal.messages.versionDetailsTitle'),
      {
        confirmButtonText: t('gameSettingsModal.common.update'),
        cancelButtonText: t('gameSettingsModal.common.cancel'),
        type: 'info'
      }
    );
  } catch {
    return false;
  } finally {
    unlisten();
  }

  try {
    isLoading.value = true;
    await ResourceManager.install3DMigotoUpdate(
      props.gameName,
      info.download_url,
      appSettings.DBMTWorkFolder,
      config.installDir,
    );
    config.packageVersion = info.version;
    config.packageReleaseDescription = info.description;
    await ResourceManager.saveGameConfig(props.gameName, JSON.parse(JSON.stringify(config)) as GameConfig);

    ElMessage.success(t('gameSettingsModal.messages.updateSuccess', { version: info.version }));
    return true;
  } catch (e) {
    console.error(e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
    return false;
  } finally {
    isLoading.value = false;
  }
};

const installDllUpdateFromInfo = async (info: UpdateInfo, mode: D3d11Mode = currentDllMode.value): Promise<boolean> => {
  if (config.allowDllUpdates === false) {
    ElMessage.warning(t('gameSettingsModal.messages.dllUpdatesRejected'));
    return false;
  }

  if (!ResourceManager.isD3d11VersionAllowedForMode(info.version, mode, config.gamePreset)) {
    ElMessage.warning(
      t('gameSettingsModal.messages.identityVDllVersionBlocked', {
        version: info.version,
        maxVersion: ResourceManager.getCappedDevD3d11MaxVersion(),
      }),
    );
    return false;
  }
  isLoading.value = false;

  const msg = t('gameSettingsModal.messages.newDllVersionFound', { version: info.version });
  const unlisten = setupBlurNotification(msg);
  try {
    await ElMessageBox.confirm(
      buildReleaseNotesConfirmMessage('gameSettingsModal.messages.newDllVersionFound', info.version, info.description),
      t('gameSettingsModal.messages.dllVersionDetailsTitle'),
      {
        confirmButtonText: t('gameSettingsModal.common.update'),
        cancelButtonText: t('gameSettingsModal.common.cancel'),
        type: 'info'
      }
    );
  } catch {
    return false;
  } finally {
    unlisten();
  }

  try {
    isLoading.value = true;
    await ResourceManager.installD3d11Update(mode, info.download_url, config.gamePreset, info.version);
    setStoredDllVersion(mode, info);

    ElMessage.success(t('gameSettingsModal.messages.dllUpdateSuccess', { version: info.version }));
    return true;
  } catch (e) {
    console.error(e);
    ElMessage.error(t('gameSettingsModal.messages.operationFailed', { error: String(e) }));
    return false;
  } finally {
    isLoading.value = false;
  }
};

const installSelectedDllVersion = async (info: UpdateInfo): Promise<boolean> => {
  installingDllVersion.value = info.version;
  try {
    const updated = await installDllUpdateFromInfo(info);
    if (updated) {
      dllReleaseListError.value = '';
    }
    return updated;
  } finally {
    if (installingDllVersion.value === info.version) {
      installingDllVersion.value = '';
    }
  }
};

const installSelectedPackageVersion = async (info: UpdateInfo): Promise<boolean> => {
  installingPackageVersion.value = info.version;
  try {
    const updated = await installPackageUpdateFromInfo(info);
    if (updated) {
      packageReleaseListError.value = '';
    }
    return updated;
  } finally {
    if (installingPackageVersion.value === info.version) {
      installingPackageVersion.value = '';
    }
  }
};

type LaunchProgramField = 'preLaunchPrograms' | 'postLaunchPrograms';

const ensureLaunchProgramList = (field: LaunchProgramField): LaunchProgramConfig[] => {
  const list = config[field];
  if (!Array.isArray(list)) {
    config[field] = [];
  }
  return config[field] as LaunchProgramConfig[];
};

const addLaunchProgram = (field: LaunchProgramField) => {
  ensureLaunchProgramList(field).push({
    exePath: '',
    args: '',
  });
};

const removeLaunchProgram = (field: LaunchProgramField, index: number) => {
  ensureLaunchProgramList(field).splice(index, 1);
};

const pickLaunchProgramExe = async (field: LaunchProgramField, index: number) => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{
        name: 'Executables',
        extensions: ['exe']
      }],
      title: t('gameSettingsModal.actions.selectExecutable')
    });

    if (selected && typeof selected === 'string') {
      const list = ensureLaunchProgramList(field);
      if (list[index]) {
        list[index].exePath = selected;
      }
    }
  } catch (e) {
    console.error(e);
  }
};

const openLaunchProgramDir = async (field: LaunchProgramField, index: number) => {
  const path = ensureLaunchProgramList(field)[index]?.exePath;
  if (!path) {
    return;
  }

  try {
    const lastSlash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    if (lastSlash > -1) {
      const dir = path.substring(0, lastSlash);
      await mkdir(dir, { recursive: true });
      await openPath(dir);
    }
  } catch (e) {
    console.error(e);
  }
};

const pickExe = async (field: 'targetExePath' | 'launcherExePath') => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ 
        name: 'Executables', 
        extensions: ['exe'] 
      }],
      title: t('gameSettingsModal.actions.selectExecutable')
    });
    if (selected && typeof selected === 'string') {
      config[field] = selected;
    }
  } catch (e) { console.error(e); }
};

const openExeDir = async (field: 'targetExePath' | 'launcherExePath') => {
  const path = config[field];
  if (path) {
    try {
      const lastSlash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
      if (lastSlash > -1) {
        const dir = path.substring(0, lastSlash);
        await mkdir(dir, { recursive: true });
        await openPath(dir);
      }
    } catch (e) { console.error(e); }
  }
};

type DiscoveredGamePaths = {
  targetExePath: string;
  launcherExePath: string;
};

const autoMatchExe = async (field: 'targetExePath' | 'launcherExePath') => {
  const gamePreset = (config.gamePreset || '').trim().toUpperCase();
  if (!gamePreset) {
    ElMessage.warning(t('gameSettingsModal.messages.selectPresetBeforeAutoMatch'));
    return;
  }

  autoMatchingExeField.value = field;
  try {
    const discovered = await invoke<DiscoveredGamePaths | null>('find_game_executable', { gamePreset });
    if (!discovered) {
      ElMessage.warning(t('gameSettingsModal.messages.autoMatchExecutableNotFound'));
      return;
    }

    const matchedPath = field === 'targetExePath'
      ? discovered.targetExePath
      : discovered.launcherExePath;
    config[field] = matchedPath;
    await saveConfig();
    ElMessage.info(t(
      field === 'targetExePath'
        ? 'gameSettingsModal.messages.autoMatchedTargetPath'
        : 'gameSettingsModal.messages.autoMatchedLauncherPath',
      { path: matchedPath }
    ));
  } catch (error) {
    console.error('Failed to auto-match executable:', error);
    ElMessage.error(t('gameSettingsModal.messages.autoMatchExecutableFailed', { error: String(error) }));
  } finally {
    autoMatchingExeField.value = null;
  }
};

const ensureExtraDlls = (): string[] => {
  if (!Array.isArray(config.extraDlls)) {
    config.extraDlls = [];
  }
  return config.extraDlls;
};

const syncPrimaryExtraDll = () => {
  const firstDll = ensureExtraDlls()
    .map(item => (item || '').trim())
    .find(Boolean);
  config.extraDll = firstDll || '';
};

const addExtraDll = (dllPath = '') => {
  ensureExtraDlls().push(dllPath);
  syncPrimaryExtraDll();
};

const pickDllForIndex = async (index: number) => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ 
        name: 'DLL Files', 
        extensions: ['dll'] 
      }],
      title: t('gameSettingsModal.actions.selectDllFile')
    });
    if (selected && typeof selected === 'string') {
      const dlls = ensureExtraDlls();
      while (dlls.length <= index) {
        dlls.push('');
      }
      dlls[index] = selected;
      syncPrimaryExtraDll();
    }
  } catch (e) { console.error(e); }
};

const setDefaultDll = async () => {
  const preferredDir = await PathHelper.GetCurrentGame3DmigotoFolderPath();

  if (preferredDir) {
    try {
      const dllPath = await join(preferredDir, 'd3d11.dll');
      addExtraDll(dllPath);
    } catch (e) {
      console.error('Failed to join path:', e);
    }
  }
};

const removeExtraDll = (index: number) => {
  ensureExtraDlls().splice(index, 1);
  syncPrimaryExtraDll();
};

const moveExtraDll = (index: number, direction: -1 | 1) => {
  const dlls = ensureExtraDlls();
  const nextIndex = index + direction;
  if (nextIndex < 0 || nextIndex >= dlls.length) return;
  const current = dlls[index];
  dlls[index] = dlls[nextIndex];
  dlls[nextIndex] = current;
  syncPrimaryExtraDll();
};

// Open/Close
watch(() => props.modelValue, (val) => {
  if (val) {
    activeTab.value = 'basic'; // Reset to first tab
    // Release candidates are game-specific (IdentityV/Naraka/NarakaM in particular cap the
    // Dev DLL at v0.9.2), so never reuse another game's cached list.
    resetDllReleaseListState();
    resetPackageReleaseListState();
    void loadConfig();
  } else {
    // When closing, save
    void saveConfig();
  }
});

watch(activeTab, (tabId) => {
  if (tabId === 'dllUpdate') {
    void loadDllReleaseList(false, currentDllMode.value);
  } else if (tabId === 'packageUpdate') {
    void loadPackageReleaseList(false);
  }
});

watch(
  () => props.gameName,
  (newGameName, oldGameName) => {
    if ((newGameName || '').trim() === (oldGameName || '').trim()) {
      return;
    }

    resetDllReleaseListState();
    resetPackageReleaseListState();
    void loadConfig();
  }
);

watch(
  () => config.gamePreset,
  (_newPreset, oldPreset) => {
    const normalizedMode = ResourceManager.getEffectiveD3d11Mode(config);
    if (config.d3d11Mode !== normalizedMode) {
      config.d3d11Mode = normalizedMode;
    }

    // GIMI defaults to UPX until the user has explicitly picked a DLL packing option.
    if (getDefaultUseUpxForGamePreset(_newPreset) && !config.useUpxManuallySet) {
      config.useUpx = true;
    }

    if (oldPreset !== undefined) {
      resetDllReleaseListState();
      resetPackageReleaseListState();

      if (props.modelValue && activeTab.value === 'dllUpdate') {
        void loadDllReleaseList(true, normalizedMode);
      }
      if (props.modelValue && activeTab.value === 'packageUpdate') {
        void loadPackageReleaseList(true);
      }
    }
  }
);

watch(
  () => config.d3d11Mode,
  (newMode, oldMode) => {
    const normalizedNewMode = ResourceManager.getEffectiveD3d11Mode({
      ...config,
      d3d11Mode: newMode,
    });
    const normalizedOldMode = ResourceManager.getEffectiveD3d11Mode({
      ...config,
      d3d11Mode: oldMode,
    });

    if (config.d3d11Mode !== normalizedNewMode) {
      config.d3d11Mode = normalizedNewMode;
      return;
    }

    if (normalizedNewMode === normalizedOldMode) {
      return;
    }

    resetDllReleaseListState();
    void saveConfig();

    if (props.modelValue && activeTab.value === 'dllUpdate') {
      void loadDllReleaseList(true, normalizedNewMode);
    }
  }
);

const close = () => {
  emit('update:modelValue', false);
};

// Expose methods to parent
defineExpose({
  switchTab: (tabId: string) => {
    activeTab.value = tabId;
  },
  runDllUpdate: async (mode?: D3d11Mode) => {
    activeTab.value = 'dllUpdate';
    await loadConfig();
    return checkD3D11DllUpdate(mode);
  },
  runPackageUpdate: async () => {
    // Ensure we are on the right tab visually
    activeTab.value = '3dmigoto';
    await loadConfig();
    // Run the update check
    return check3DMigotoPackageUpdate();
  },
  installDllUpdateWithInfo: async (info: UpdateInfo) => {
    activeTab.value = 'dllUpdate';
    await loadConfig();
    return installDllUpdateFromInfo(info);
  },
  installPackageUpdateWithInfo: async (info: UpdateInfo) => {
    activeTab.value = '3dmigoto';
    await loadConfig();
    return installPackageUpdateFromInfo(info);
  },
  runPackageUpdateDirect: async () => {
    await loadConfig();
    const dllUpdated = await checkD3D11DllUpdate();
    if (!dllUpdated) {
      return false;
    }
    return check3DMigotoPackageUpdate();
  }
});
</script>

<template>
  <transition name="modal-fade">
    <div v-if="modelValue" class="settings-overlay">
      <div class="settings-window">
        <!-- Loading Overlay -->
        <div v-if="isLoading" class="loading-overlay">
          <div class="spinner"></div>
          <div class="loading-text">{{ t('gameSettingsModal.common.processing') }}</div>
        </div>

        <!-- Sidebar -->
        <div class="settings-sidebar">
          <div class="sidebar-title">{{ t('gameSettingsModal.title') }}</div>

          <div v-for="tab in tabs" :key="tab.id" class="sidebar-item" :class="{ active: activeTab === tab.id }"
            @click="activeTab = tab.id">
            <svg v-if="tab.id === 'basic'" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="3"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/>
            </svg>
            <svg v-if="tab.id === '3dmigoto'" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polygon points="12 2 22 8.5 22 15.5 12 22 2 15.5 2 8.5"/>
              <line x1="12" y1="22" x2="12" y2="15.5"/><polyline points="22 8.5 12 15.5 2 8.5"/>
            </svg>
            <svg v-if="tab.id === 'dllUpdate'" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
            </svg>
            <svg v-if="tab.id === 'packageUpdate'" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
              <path d="M12 22.08V12"/>
              <path d="m3.27 6.96 8.73 5.05 8.73-5.05"/>
            </svg>
            <svg v-if="tab.id === 'other'" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
            </svg>
            {{ tab.label }}
          </div>
        </div>

        <!-- Content Area -->
        <div class="settings-content">
          <div class="content-header">
            <span class="header-title">{{tabs.find(t => t.id === activeTab)?.label}}</span>
            <div class="close-btn" @click="close">
              <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="18" y1="6" x2="6" y2="18"></line>
                <line x1="6" y1="6" x2="18" y2="18"></line>
              </svg>
            </div>
          </div>

          <div class="scroll-content glass-scrollbar">
            <!-- Basic Settings -->
            <div v-if="activeTab === 'basic'" class="tab-pane">
              <!-- Preset + Icon row -->
              <div class="settings-row">
                <div class="settings-card">
                  <div class="settings-card-header">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/>
                    </svg>
                    <span>{{ t('gameSettingsModal.fields.gamePreset') }}</span>
                  </div>
                  <el-select v-model="config.gamePreset" :placeholder="t('gameSettingsModal.common.select')" class="custom-select" @change="saveConfig">
                    <el-option v-for="item in presetOptions" :key="item.value" :label="item.label" :value="item.value" />
                  </el-select>
                </div>

                <div class="settings-card">
                  <div class="settings-card-header">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
                    </svg>
                    <span>{{ t('gameSettingsModal.fields.gameIcon') }}</span>
                  </div>
                  <button class="settings-icon-btn" @click="selectIcon">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                    </svg>
                    {{ t('gameSettingsModal.actions.selectIconPng') }}
                  </button>
                </div>
              </div>

              <!-- Background card -->
              <div class="settings-card settings-card-full">
                <div class="settings-card-header">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.backgroundSettings') }}</span>
                  <button type="button" class="settings-header-icon-btn" :title="t('gameSettingsModal.actions.revealBackgroundFile')" :aria-label="t('gameSettingsModal.actions.revealBackgroundFile')" @click="revealBackgroundFile">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2z"/>
                      <path d="M14 13h6"/>
                      <path d="m17 10 3 3-3 3"/>
                    </svg>
                  </button>
                </div>

                <!-- Background Type -->
                <div class="settings-toggle-row-inline">
                  <div class="settings-toggle-row-info">
                    <div class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.backgroundType') }}</div>
                  </div>
                  <el-select v-model="config.backgroundType" class="custom-select" @change="handleBgTypeChange" style="width:130px">
                    <el-option value="Image" :label="t('gameSettingsModal.fields.staticBackground')" />
                    <el-option value="Video" :label="t('gameSettingsModal.fields.dynamicBackground')" />
                  </el-select>
                </div>

                <!-- Update Mode -->
                <div class="settings-toggle-row-inline">
                  <div class="settings-toggle-row-info">
                    <div class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.backgroundUpdateMode') }}</div>
                  </div>
                  <el-select v-model="config.backgroundUpdateMode" class="custom-select" @change="handleUpdateModeChange" style="width:130px">
                    <el-option value="manual" :label="t('gameSettingsModal.options.backgroundUpdateMode.manual')" />
                    <el-option value="auto" :label="t('gameSettingsModal.options.backgroundUpdateMode.auto')" />
                  </el-select>
                </div>

                <!-- Actions -->
                <div class="settings-bg-actions">
                  <button class="settings-action-btn" @click="selectBackground">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                    </svg>
                    {{ config.backgroundType === 'Video' ? t('gameSettingsModal.actions.selectBackgroundVideo') : t('gameSettingsModal.actions.selectBackgroundImage') }}
                  </button>
                  <button v-if="canAutoUpdate" class="settings-action-btn highlight" @click="autoUpdateBackground">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                    </svg>
                    {{ t('gameSettingsModal.actions.autoUpdateBackground') }}
                  </button>
                </div>
              </div>

            </div>

            <!-- 3Dmigoto Settings -->
            <div v-if="activeTab === '3dmigoto'" class="tab-pane">

              <!-- Card 1: Path Settings -->
              <div class="settings-card settings-card-full">
                <div class="settings-card-header">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.migotoDirectory') }}</span>
                </div>
                <div class="settings-path-row">
                  <input v-model="config.installDir" type="text" class="settings-path-input"
                    :placeholder="t('gameSettingsModal.placeholders.selectOrEnterDirectory')" />
                  <el-tooltip :content="t('gameSettingsModal.actions.selectFolder')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" @click="pick3dmigotoDir">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.setDefaultCacheMigotoFolder')" placement="top" :show-after="250">
                  <button class="settings-sm-btn folder" @click="set3dmigotoDirToDefault">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.openFolder')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" @click="open3dmigotoDir">
                    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                    </svg>
                  </button>
                  </el-tooltip>
                </div>

                <div class="settings-field-label">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.targetProcessPath') }}</span>
                </div>
                <div class="settings-path-row">
                  <input v-model="config.targetExePath" type="text" class="settings-path-input"
                    :placeholder="t('gameSettingsModal.placeholders.selectGameExecutable')" />
                  <el-tooltip :content="t('gameSettingsModal.actions.selectFile')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" @click="pickExe('targetExePath')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.openLocation')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" @click="openExeDir('targetExePath')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.autoMatchPath')" placement="top" :show-after="250">
                    <button class="settings-sm-btn highlight" :disabled="autoMatchingExeField !== null" @click="autoMatchExe('targetExePath')">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16.65" y2="16.65"/><path d="M11 8v6M8 11h6"/>
                      </svg>
                    </button>
                  </el-tooltip>
                </div>

                <div class="settings-field-label">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M5 17H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-1"/>
                    <polygon points="12 15 17 21 7 21 12 15"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.launcherPath') }}</span>
                </div>
                <div class="settings-toggle-row-inline">
                  <div class="settings-toggle-row-info">
                    <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.launchTargetProgram') }}</span>
                    <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.launchTargetProgramHint') }}</span>
                  </div>
                  <el-checkbox v-model="config.launchTargetProgram" @change="saveConfig">
                    {{ t('gameSettingsModal.fields.launchTargetProgramCheckbox') }}
                  </el-checkbox>
                </div>
                <div class="settings-path-row">
                  <input v-model="config.launcherExePath" type="text" class="settings-path-input" :disabled="config.launchTargetProgram === false"
                    :placeholder="t('gameSettingsModal.placeholders.selectLauncherOptional')" />
                  <el-tooltip :content="t('gameSettingsModal.actions.selectFile')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" :disabled="config.launchTargetProgram === false" @click="pickExe('launcherExePath')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.openLocation')" placement="top" :show-after="250">
                  <button class="settings-sm-btn" :disabled="config.launchTargetProgram === false" @click="openExeDir('launcherExePath')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                    </svg>
                  </button>
                  </el-tooltip>
                  <el-tooltip :content="t('gameSettingsModal.actions.autoMatchPath')" placement="top" :show-after="250">
                    <button class="settings-sm-btn highlight" :disabled="config.launchTargetProgram === false || autoMatchingExeField !== null" @click="autoMatchExe('launcherExePath')">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16.65" y2="16.65"/><path d="M11 8v6M8 11h6"/>
                      </svg>
                    </button>
                  </el-tooltip>
                </div>
              </div>

              <!-- Card 2: Launch Configuration -->
              <div class="settings-card settings-card-full">
                <div class="settings-card-header">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.launchArguments') }}</span>
                </div>
                <input v-model="config.launchArgs" type="text" class="custom-input"
                  :placeholder="t('gameSettingsModal.placeholders.launchArgumentsExample')" />

                <div class="settings-flex-row">
                  <div>
                    <div class="settings-field-label">{{ t('gameSettingsModal.fields.launchMode') }}</div>
                    <el-select v-model="launchMode" class="custom-select" style="margin-top:6px">
                      <el-option v-for="item in launchModeOptions" :key="item.value" :label="item.label" :value="item.value" />
                    </el-select>
                  </div>
                  <div>
                    <div class="settings-field-label">{{ t('gameSettingsModal.fields.huntingMode') }}</div>
                    <el-select v-model="config.huntingMode" :placeholder="t('gameSettingsModal.common.select')" class="custom-select" @change="saveConfig" style="margin-top:6px">
                      <el-option v-for="item in huntingModeOptions" :key="item.value" :label="item.label" :value="item.value" />
                    </el-select>
                  </div>
                </div>

                <div class="settings-flex-row">
                  <div>
                    <div class="settings-field-label">{{ t('gameSettingsModal.fields.d3d11Delay') }}</div>
                    <input v-model.number="config.delay" type="number" class="settings-num-input" style="margin-top:6px" />
                  </div>
                  <div>
                    <div class="settings-field-label">{{ t('gameSettingsModal.fields.autoExitSeconds') }}</div>
                    <input v-model.number="config.autoExitSeconds" type="number" class="settings-num-input" style="margin-top:6px" />
                  </div>
                </div>

                <div v-if="!config.useShell">
                  <div class="settings-field-label" style="margin-bottom:6px">{{ t('gameSettingsModal.fields.extraInjectedDll') }}</div>
                  <div v-for="(_dll, index) in config.extraDlls" :key="index" class="settings-path-row" style="margin-bottom:6px">
                    <input v-model="config.extraDlls![index]" type="text" class="settings-path-input" :placeholder="t('gameSettingsModal.placeholders.selectOrLeaveEmpty')" @input="syncPrimaryExtraDll" />
                    <el-tooltip :content="t('gameSettingsModal.actions.selectFile')" placement="top" :show-after="250"><button class="settings-sm-btn" @click="pickDllForIndex(index)">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                      </svg>
                    </button></el-tooltip>
                    <el-tooltip :content="t('gameSettingsModal.actions.moveUp')" placement="top" :show-after="250"><button class="settings-sm-btn" @click="moveExtraDll(index, -1)" :disabled="index === 0">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m18 15-6-6-6 6"/>
                      </svg>
                    </button></el-tooltip>
                    <el-tooltip :content="t('gameSettingsModal.actions.moveDown')" placement="top" :show-after="250"><button class="settings-sm-btn" @click="moveExtraDll(index, 1)" :disabled="index === (config.extraDlls?.length || 0) - 1">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m6 9 6 6 6-6"/>
                      </svg>
                    </button></el-tooltip>
                    <el-tooltip :content="t('gameSettingsModal.actions.remove')" placement="top" :show-after="250"><button class="settings-sm-btn danger" @click="removeExtraDll(index)">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M19 6l-1 14H6L5 6"/>
                      </svg>
                    </button></el-tooltip>
                  </div>
                  <div class="settings-path-row">
                    <el-tooltip :content="t('gameSettingsModal.actions.addDll')" placement="top" :show-after="250"><button class="settings-sm-btn highlight" @click="addExtraDll()">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M12 5v14"/><path d="M5 12h14"/>
                      </svg>
                    </button></el-tooltip>
                    <el-tooltip :content="t('gameSettingsModal.actions.selectFile')" placement="top" :show-after="250"><button class="settings-sm-btn" @click="pickDllForIndex(config.extraDlls?.length || 0)">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                      </svg>
                    </button></el-tooltip>
                    <el-tooltip :content="t('gameSettingsModal.actions.setCurrentD3d11Dll')" placement="top" :show-after="250"><button class="settings-sm-btn" @click="setDefaultDll">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                      </svg>
                    </button></el-tooltip>
                  </div>
                </div>
              </div>

              <!-- Card 3: Options -->
              <div class="settings-card settings-card-full">
                <div class="settings-card-header">
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.fields.options') }}</span>
                </div>
                <label class="settings-toggle-label">
                  <input type="checkbox" v-model="config.showErrorPopup" @change="syncShowTopLeftWarningsToIni" />
                  {{ t('gameSettingsModal.fields.showTopLeftWarnings') }}
                </label>
                <label class="settings-toggle-label">
                  <input type="checkbox" v-model="config.autoSetAnalyseOptions" />
                  {{ t('gameSettingsModal.fields.autoSetAnalyseOptions') }}
                </label>
                <label class="settings-toggle-label">
                  <input type="checkbox" v-model="config.checkDllUpdateBeforeLaunch" @change="saveConfig" />
                  {{ t('gameSettingsModal.fields.checkDllUpdateBeforeLaunch') }}
                </label>
                <label class="settings-toggle-label">
                  <input type="checkbox" v-model="config.check3DmigotoPackageUpdateBeforeLaunch" @change="saveConfig" />
                  {{ t('gameSettingsModal.fields.check3DmigotoPackageUpdateBeforeLaunch') }}
                </label>

                <div style="border-top: 1px solid rgba(255,255,255,0.04); margin: 6px 0 2px;"></div>

                <div class="settings-toggle-row-inline">
                  <div class="settings-toggle-row-info">
                    <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.includePrereleaseUpdates') }}</span>
                    <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.includePrereleaseUpdatesHint') }}</span>
                  </div>
                  <el-switch v-model="config.includePrereleaseUpdates" @change="saveConfig" />
                </div>

                <div style="border-top: 1px solid rgba(255,255,255,0.04); margin: 6px 0 2px;"></div>

                <template v-if="isWWMIPreset">
                  <div class="settings-toggle-row-inline">
                    <div class="settings-toggle-row-info">
                      <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.wwmiConfigureGame') }}</span>
                      <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.wwmiConfigureGameHint') }}</span>
                    </div>
                    <el-switch v-model="config.configureGame" @change="saveConfig" />
                  </div>

                  <div class="settings-toggle-row-inline">
                    <div class="settings-toggle-row-info">
                      <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.wwmiApplyPerfTweaks') }}</span>
                      <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.wwmiApplyPerfTweaksHint') }}</span>
                    </div>
                    <el-switch v-model="config.applyPerfTweaks" @change="saveConfig" />
                  </div>

                  <div class="settings-toggle-row-inline">
                    <div class="settings-toggle-row-info">
                      <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.wwmiUnlockFps') }}</span>
                      <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.wwmiUnlockFpsHint') }}</span>
                    </div>
                    <el-switch v-model="config.unlockFps" @change="saveConfig" />
                  </div>

                  <div class="settings-toggle-row-inline">
                    <div class="settings-toggle-row-info">
                      <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.wwmiForceMaxLodBias') }}</span>
                      <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.wwmiForceMaxLodBiasHint') }}</span>
                    </div>
                    <el-switch v-model="config.forceMaxLodBias" @change="saveConfig" />
                  </div>

                  <div class="settings-toggle-row-inline">
                    <div class="settings-toggle-row-info">
                      <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.wwmiDisableWoundedFx') }}</span>
                      <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.wwmiDisableWoundedFxHint') }}</span>
                    </div>
                    <el-switch v-model="config.disableWoundedFx" @change="saveConfig" />
                  </div>

                  <div style="border-top: 1px solid rgba(255,255,255,0.04); margin: 6px 0 2px;"></div>
                </template>

                <div class="settings-toggle-row-inline">
                  <div class="settings-toggle-row-info">
                    <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.dllPacking') }}</span>
                  </div>
                  <el-select v-model="config.useUpx" class="custom-select" @change="handleDllPackingChange" style="width:130px">
                    <el-option :value="false" :label="t('gameSettingsModal.options.dllPacking.none')" />
                    <el-option :value="true" :label="t('gameSettingsModal.options.dllPacking.upx')" />
                  </el-select>
                </div>
              </div>

            </div>

            <div v-if="activeTab === 'dllUpdate'" class="tab-pane">
              <div class="settings-toggle-row-inline dll-update-permission">
                <div class="settings-toggle-row-info">
                  <span class="settings-toggle-row-title">{{ t('gameSettingsModal.fields.allowDllUpdates') }}</span>
                  <span class="settings-toggle-row-hint">{{ t('gameSettingsModal.fields.allowDllUpdatesHint') }}</span>
                </div>
                <el-checkbox v-model="config.allowDllUpdates" @change="saveConfig">
                  {{ t('gameSettingsModal.fields.allowDllUpdatesCheckbox') }}
                </el-checkbox>
              </div>

              <!-- Capped dev d3d11.dll version cap notice -->
              <div v-if="isCappedDevD3d11ModeRestricted" class="dll-cap-hint">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10"/>
                  <line x1="12" y1="8" x2="12" y2="12"/>
                  <line x1="12" y1="16" x2="12.01" y2="16"/>
                </svg>
                <span>
                  {{ t('gameSettingsModal.fields.identityVDllVersionCapHint', {
                    maxVersion: ResourceManager.getCappedDevD3d11MaxVersion(),
                  }) }}
                </span>
              </div>

              <!-- Top info bar: source selector + current version -->
              <div class="dll-top-bar">
                <div class="dll-source-group">
                  <div class="dll-source-label-row">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
                    </svg>
                    <span class="dll-source-label">{{ t('gameSettingsModal.fields.dllSource') }}</span>
                  </div>
                  <el-select v-model="config.d3d11Mode" :disabled="isForcedSsiceADllMode" class="custom-select" @change="saveConfig">
                    <el-option v-for="item in dllSourceOptions" :key="item.value" :label="item.label" :value="item.value" />
                  </el-select>
                  <div class="dll-source-hint">{{ t('gameSettingsModal.fields.dllSourceHint') }}</div>
                </div>

                <div class="dll-version-badge">
                  <div class="dll-version-badge-inner">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 6 13.5 15.5 8.5 10.5 1 18"/>
                      <polyline points="17 6 23 6 23 12"/>
                    </svg>
                    <div class="dll-version-badge-info">
                      <span class="dll-version-badge-label">{{ t('gameSettingsModal.fields.currentInstalledDllVersion') }}</span>
                      <span class="dll-version-badge-value">{{ currentCoreVersion || t('gameSettingsModal.fields.noDllVersionInstalled') }}</span>
                    </div>
                  </div>
                </div>
              </div>

              <!-- Release list section -->
              <div class="dll-release-section">
                <div class="dll-release-section-header">
                  <div class="dll-release-section-title-row">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>
                    </svg>
                    <span class="dll-release-section-title">{{ t('gameSettingsModal.fields.availableDllVersions') }}</span>
                  </div>
                  <span class="dll-release-section-subtitle">{{ t('gameSettingsModal.fields.availableDllVersionsHint') }}</span>
                </div>

                <div class="dll-release-toolbar">
                  <button class="dll-refresh-btn" @click="refreshDllReleaseList" :disabled="isDllReleaseLoading || isLoading">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10"/>
                      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                    </svg>
                    {{ isDllReleaseLoading ? t('gameSettingsModal.actions.loadingVersions') : t('gameSettingsModal.actions.refreshVersionList') }}
                  </button>
                </div>

                <!-- Loading state -->
                <div v-if="isDllReleaseLoading" class="dll-release-state dll-release-state-loading">
                  <div class="dll-state-spinner"></div>
                  <span>{{ t('gameSettingsModal.messages.loadingDllVersions') }}</span>
                </div>

                <!-- Error state -->
                <div v-else-if="dllReleaseListError" class="dll-release-state dll-release-state-error">
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
                  </svg>
                  <div>
                    <div class="dll-state-title">{{ t('gameSettingsModal.messages.dllVersionListLoadFailed', { error: '' }) }}</div>
                    <div class="dll-state-detail">{{ dllReleaseListError }}</div>
                  </div>
                </div>

                <!-- Empty state -->
                <div v-else-if="!dllReleaseList.length" class="dll-release-state dll-release-state-empty">
                  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                    <polyline points="17 8 12 3 7 8"/>
                    <line x1="12" y1="3" x2="12" y2="15"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.messages.noDllVersionsAvailable') }}</span>
                </div>

                <!-- Release cards -->
                <div v-else class="dll-release-list">
                  <div v-for="item in visibleDllReleaseList" :key="item.version" class="dll-release-card"
                    :class="{ 'dll-release-card-installed': item.version === currentCoreVersion }">
                    <div class="dll-release-card-top">
                      <div class="dll-release-card-info">
                        <div class="dll-release-version-row">
                          <span class="dll-release-version">{{ item.version }}</span>
                          <span v-if="item.is_latest" class="dll-release-badge dll-release-badge-latest">
                            {{ t('gameSettingsModal.fields.latestLabel') }}
                          </span>
                          <span v-if="item.is_prerelease" class="dll-release-badge dll-release-badge-prerelease">
                            {{ t('gameSettingsModal.fields.prereleaseLabel') }}
                          </span>
                        </div>
                        <span v-if="item.version === currentCoreVersion" class="dll-release-installed-tag">
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="20 6 9 17 4 12"/>
                          </svg>
                          {{ t('gameSettingsModal.fields.currentInstalledLabel') }}
                        </span>
                      </div>
                      <ReleaseNotesMarkdown
                        class="dll-release-description"
                        :content="item.description"
                        compact
                      />
                    </div>
                    <div class="dll-release-card-actions">
                      <button
                        class="dll-install-btn"
                        @click="installSelectedDllVersion(item)"
                        :disabled="config.allowDllUpdates === false || isLoading || isDllReleaseLoading || installingDllVersion === item.version"
                      >
                        <svg v-if="installingDllVersion !== item.version" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                          <polyline points="7 10 12 15 17 10"/>
                          <line x1="12" y1="15" x2="12" y2="3"/>
                        </svg>
                        <span v-if="installingDllVersion === item.version" class="dll-install-spinner"></span>
                        {{ installingDllVersion === item.version ? t('gameSettingsModal.actions.installingVersion') : t('gameSettingsModal.actions.installThisVersion') }}
                      </button>
                    </div>
                  </div>
                </div>

                <button v-if="hasMoreDllReleases" class="dll-load-more-btn" @click="loadMoreDllReleases">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="7 13 12 18 17 13"/>
                    <polyline points="7 6 12 11 17 6"/>
                  </svg>
                  {{ t('gameSettingsModal.actions.loadMoreVersions') }}
                </button>
              </div>
            </div>

            <div v-if="activeTab === 'packageUpdate'" class="tab-pane">
              <div class="dll-top-bar">
                <div class="dll-source-group">
                  <div class="dll-source-label-row">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                      <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
                      <line x1="12" y1="22.08" x2="12" y2="12"/>
                    </svg>
                    <span class="dll-source-label">{{ t('gameSettingsModal.fields.packageSource') }}</span>
                  </div>
                  <div class="package-source-row">
                    <div class="package-source-value">
                      {{ currentPackageRepo || t('gameSettingsModal.fields.noPackagePresetSelected') }}
                    </div>
                    <el-tooltip :content="t('gameSettingsModal.actions.openPackageRepository')" placement="bottom">
                      <button
                        class="package-source-open-btn"
                        :disabled="!currentPackageRepoUrl"
                        @click="openCurrentPackageRepo"
                      >
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                          stroke-linecap="round" stroke-linejoin="round">
                          <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
                          <polyline points="15 3 21 3 21 9"/>
                          <line x1="10" y1="14" x2="21" y2="3"/>
                        </svg>
                      </button>
                    </el-tooltip>
                  </div>
                  <div class="dll-source-hint">{{ t('gameSettingsModal.fields.packageSourceHint') }}</div>
                </div>

                <div class="dll-version-badge">
                  <div class="dll-version-badge-inner">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M20 7h-9"/>
                      <path d="M14 17H5"/>
                      <circle cx="17" cy="17" r="3"/>
                      <circle cx="7" cy="7" r="3"/>
                    </svg>
                    <div class="dll-version-badge-info">
                      <span class="dll-version-badge-label">{{ t('gameSettingsModal.fields.currentInstalledPackageVersion') }}</span>
                      <span class="dll-version-badge-value">{{ currentPackageVersion || t('gameSettingsModal.fields.noPackageVersionInstalled') }}</span>
                    </div>
                  </div>
                </div>
              </div>

              <div class="dll-release-section">
                <div class="dll-release-section-header">
                  <div class="dll-release-section-title-row">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M16 16v1a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h1"/>
                      <path d="M12 3h6v6"/>
                      <path d="M8 13 18 3"/>
                    </svg>
                    <span class="dll-release-section-title">{{ t('gameSettingsModal.fields.availablePackageVersions') }}</span>
                  </div>
                  <span class="dll-release-section-subtitle">{{ t('gameSettingsModal.fields.availablePackageVersionsHint') }}</span>
                </div>

                <div class="dll-release-toolbar">
                  <button class="dll-refresh-btn" @click="refreshPackageReleaseList" :disabled="isPackageReleaseLoading || isLoading">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10"/>
                      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                    </svg>
                    {{ isPackageReleaseLoading ? t('gameSettingsModal.actions.loadingVersions') : t('gameSettingsModal.actions.refreshVersionList') }}
                  </button>
                </div>

                <div v-if="isPackageReleaseLoading" class="dll-release-state dll-release-state-loading">
                  <div class="dll-state-spinner"></div>
                  <span>{{ t('gameSettingsModal.messages.loadingPackageVersions') }}</span>
                </div>

                <div v-else-if="packageReleaseListError" class="dll-release-state dll-release-state-error">
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
                  </svg>
                  <div>
                    <div class="dll-state-title">{{ t('gameSettingsModal.messages.packageVersionListLoadFailed', { error: '' }) }}</div>
                    <div class="dll-state-detail">{{ packageReleaseListError }}</div>
                  </div>
                </div>

                <div v-else-if="!packageReleaseList.length" class="dll-release-state dll-release-state-empty">
                  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                    <polyline points="17 8 12 3 7 8"/>
                    <line x1="12" y1="3" x2="12" y2="15"/>
                  </svg>
                  <span>{{ t('gameSettingsModal.messages.noPackageVersionsAvailable') }}</span>
                </div>

                <div v-else class="dll-release-list">
                  <div v-for="item in visiblePackageReleaseList" :key="item.version" class="dll-release-card"
                    :class="{ 'dll-release-card-installed': item.version === currentPackageVersion }">
                    <div class="dll-release-card-top">
                      <div class="dll-release-card-info">
                        <div class="dll-release-version-row">
                          <span class="dll-release-version">{{ item.version }}</span>
                          <span v-if="item.is_latest" class="dll-release-badge dll-release-badge-latest">
                            {{ t('gameSettingsModal.fields.latestLabel') }}
                          </span>
                          <span v-if="item.is_prerelease" class="dll-release-badge dll-release-badge-prerelease">
                            {{ t('gameSettingsModal.fields.prereleaseLabel') }}
                          </span>
                        </div>
                        <span v-if="item.version === currentPackageVersion" class="dll-release-installed-tag">
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="20 6 9 17 4 12"/>
                          </svg>
                          {{ t('gameSettingsModal.fields.currentInstalledLabel') }}
                        </span>
                      </div>
                      <ReleaseNotesMarkdown
                        class="dll-release-description"
                        :content="item.description"
                        compact
                      />
                    </div>
                    <div class="dll-release-card-actions">
                      <button
                        class="dll-install-btn"
                        @click="installSelectedPackageVersion(item)"
                        :disabled="isLoading || isPackageReleaseLoading || installingPackageVersion === item.version"
                      >
                        <svg v-if="installingPackageVersion !== item.version" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                          <polyline points="7 10 12 15 17 10"/>
                          <line x1="12" y1="15" x2="12" y2="3"/>
                        </svg>
                        <span v-if="installingPackageVersion === item.version" class="dll-install-spinner"></span>
                        {{ installingPackageVersion === item.version ? t('gameSettingsModal.actions.installingVersion') : t('gameSettingsModal.actions.installThisVersion') }}
                      </button>
                    </div>
                  </div>
                </div>

                <button v-if="hasMorePackageReleases" class="dll-load-more-btn" @click="loadMorePackageReleases">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="7 13 12 18 17 13"/>
                    <polyline points="7 6 12 11 17 6"/>
                  </svg>
                  {{ t('gameSettingsModal.actions.loadMoreVersions') }}
                </button>
              </div>
            </div>

            <!-- Other Settings -->
            <div v-if="activeTab === 'other'" class="tab-pane">
              <div class="launch-lists-grid">
                <!-- Pre-launch Programs -->
                <div class="launch-list-column">
                  <div class="launch-list-column-header">
                    <div class="launch-list-column-title-row">
                      <svg class="launch-list-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polygon points="5 3 19 12 5 21 5 3"/>
                      </svg>
                      <span class="launch-list-column-title">{{ t('gameSettingsModal.fields.preLaunchPrograms') }}</span>
                    </div>
                    <div class="launch-list-column-help">{{ t('gameSettingsModal.fields.preLaunchProgramsHelp') }}</div>
                  </div>

                  <div class="launch-program-list">
                    <div v-if="!config.preLaunchPrograms?.length" class="launch-program-empty">
                      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                        <polyline points="14 2 14 8 20 8"/>
                        <line x1="12" y1="18" x2="12" y2="12"/>
                        <line x1="9" y1="15" x2="15" y2="15"/>
                      </svg>
                      <span>{{ t('gameSettingsModal.messages.noLaunchProgramsConfigured') }}</span>
                    </div>
                    <div v-for="(program, index) in config.preLaunchPrograms" :key="`pre-${index}`" class="launch-program-card">
                      <div class="launch-program-card-header">
                        <span class="launch-program-card-badge">{{ t('gameSettingsModal.fields.programEntryTitle', { index: index + 1 }) }}</span>
                        <button class="launch-program-remove-btn" @click="removeLaunchProgram('preLaunchPrograms', index)" :title="t('gameSettingsModal.actions.removeProgram')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                          </svg>
                        </button>
                      </div>
                      <div class="launch-program-exe-row">
                        <input v-model="program.exePath" type="text" class="launch-program-input"
                          :placeholder="t('gameSettingsModal.placeholders.selectProgramExecutable')" />
                        <button class="launch-program-sm-btn" @click="pickLaunchProgramExe('preLaunchPrograms', index)" :title="t('gameSettingsModal.actions.selectFile')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                          </svg>
                        </button>
                        <button class="launch-program-sm-btn" @click="openLaunchProgramDir('preLaunchPrograms', index)" :title="t('gameSettingsModal.actions.openLocation')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                          </svg>
                        </button>
                      </div>
                      <div class="launch-program-args-row">
                        <svg class="launch-program-args-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>
                        </svg>
                        <input v-model="program.args" type="text" class="launch-program-input launch-program-args-input"
                          :placeholder="t('gameSettingsModal.placeholders.launchArgumentsExample')" />
                      </div>
                    </div>
                  </div>
                  <button class="launch-program-add-btn" @click="addLaunchProgram('preLaunchPrograms')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                    </svg>
                    {{ t('gameSettingsModal.actions.addProgram') }}
                  </button>
                </div>

                <!-- Post-launch Programs -->
                <div class="launch-list-column">
                  <div class="launch-list-column-header">
                    <div class="launch-list-column-title-row">
                      <svg class="launch-list-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="23 7 16 12 23 17"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/>
                      </svg>
                      <span class="launch-list-column-title">{{ t('gameSettingsModal.fields.postLaunchPrograms') }}</span>
                    </div>
                    <div class="launch-list-column-help">{{ t('gameSettingsModal.fields.postLaunchProgramsHelp') }}</div>
                  </div>

                  <div class="launch-program-list">
                    <div v-if="!config.postLaunchPrograms?.length" class="launch-program-empty">
                      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                        <polyline points="14 2 14 8 20 8"/>
                        <line x1="12" y1="18" x2="12" y2="12"/>
                        <line x1="9" y1="15" x2="15" y2="15"/>
                      </svg>
                      <span>{{ t('gameSettingsModal.messages.noLaunchProgramsConfigured') }}</span>
                    </div>
                    <div v-for="(program, index) in config.postLaunchPrograms" :key="`post-${index}`" class="launch-program-card">
                      <div class="launch-program-card-header">
                        <span class="launch-program-card-badge">{{ t('gameSettingsModal.fields.programEntryTitle', { index: index + 1 }) }}</span>
                        <button class="launch-program-remove-btn" @click="removeLaunchProgram('postLaunchPrograms', index)" :title="t('gameSettingsModal.actions.removeProgram')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                          </svg>
                        </button>
                      </div>
                      <div class="launch-program-exe-row">
                        <input v-model="program.exePath" type="text" class="launch-program-input"
                          :placeholder="t('gameSettingsModal.placeholders.selectProgramExecutable')" />
                        <button class="launch-program-sm-btn" @click="pickLaunchProgramExe('postLaunchPrograms', index)" :title="t('gameSettingsModal.actions.selectFile')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/>
                          </svg>
                        </button>
                        <button class="launch-program-sm-btn" @click="openLaunchProgramDir('postLaunchPrograms', index)" :title="t('gameSettingsModal.actions.openLocation')">
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                          </svg>
                        </button>
                      </div>
                      <div class="launch-program-args-row">
                        <svg class="launch-program-args-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>
                        </svg>
                        <input v-model="program.args" type="text" class="launch-program-input launch-program-args-input"
                          :placeholder="t('gameSettingsModal.placeholders.launchArgumentsExample')" />
                      </div>
                    </div>
                  </div>
                  <button class="launch-program-add-btn" @click="addLaunchProgram('postLaunchPrograms')">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                    </svg>
                    {{ t('gameSettingsModal.actions.addProgram') }}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </transition>
</template>

<style scoped>
.settings-overlay {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  background-color: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
  z-index: 2000;
}

.settings-window {
  position: absolute;
  top: 50px;
  bottom: 60px;
  left: 100px;
  right: 100px;
  background: var(--t-material-bg);
  border: var(--t-material-border);
  box-shadow: var(--t-material-shadow);
  border-radius: 14px;
  display: flex;
  overflow: hidden;
  animation: settingsSlideUp 0.35s cubic-bezier(0.16, 1, 0.3, 1);
}

.settings-window::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
  z-index: 1;
}

@keyframes settingsSlideUp {
  from {
    opacity: 0;
    transform: translateY(24px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

.setting-checkbox-row {
  margin-bottom: 12px;
}

.setting-help-text {
  margin-top: 8px;
  padding: 8px 12px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px solid rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.45);
  font-size: 12px;
  line-height: 1.5;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  color: white;
  cursor: pointer;
  user-select: none;
}

.flex-row {
  display: flex;
  gap: 16px;
}

.half-width {
  flex: 1;
}

/* ===== Launch Programs - Dual Column Layout ===== */
.launch-lists-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 20px;
  align-items: start;
}

.launch-list-column {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}

.launch-list-column-header {
  margin-bottom: 2px;
}

.launch-list-column-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

.launch-list-icon {
  flex-shrink: 0;
  color: var(--theme-accent);
  opacity: 0.85;
}

.launch-list-column-title {
  font-size: 15px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.92);
}

.launch-list-column-help {
  font-size: 12px;
  line-height: 1.5;
  color: rgba(255, 255, 255, 0.5);
  padding-left: 26px;
}

.launch-program-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.launch-program-empty {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 18px 16px;
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px dashed rgba(255, 255, 255, 0.12);
  color: rgba(255, 255, 255, 0.4);
  font-size: 12px;
  line-height: 1.4;
}

.launch-program-empty svg {
  flex-shrink: 0;
  opacity: 0.5;
}

.launch-program-card {
  padding: 10px 12px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.07);
  transition: border-color 0.2s, background 0.2s, box-shadow 0.2s;
}

.launch-program-card:hover {
  border-color: rgba(255, 255, 255, 0.14);
  background: rgba(255, 255, 255, 0.055);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}

.launch-program-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.launch-program-card-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 999px;
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.25);
  color: var(--theme-accent);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.launch-program-remove-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  background: rgba(232, 17, 35, 0.15);
  color: #ff6b6b;
  cursor: pointer;
  transition: all 0.2s;
  flex-shrink: 0;
}

.launch-program-remove-btn:hover {
  background: rgba(232, 17, 35, 0.35);
  color: #ff4444;
}

.launch-program-exe-row {
  display: flex;
  gap: 6px;
  margin-bottom: 6px;
}

.launch-program-input {
  flex: 1;
  min-width: 0;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.35);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 6px;
  padding: 6px 10px;
  color: rgba(255, 255, 255, 0.88);
  font-size: 13px;
  outline: none;
  transition: border-color 0.2s;
}

.launch-program-input:focus {
  border-color: var(--theme-accent);
}

.launch-program-input::placeholder {
  color: rgba(255, 255, 255, 0.25);
}

.launch-program-sm-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.07);
  color: rgba(255, 255, 255, 0.5);
  cursor: pointer;
  transition: all 0.2s;
  flex-shrink: 0;
}

.launch-program-sm-btn:hover {
  background: rgba(255, 255, 255, 0.14);
  color: rgba(255, 255, 255, 0.85);
}

.launch-program-args-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.launch-program-args-icon {
  flex-shrink: 0;
  color: rgba(255, 255, 255, 0.3);
  margin-left: 2px;
}

.launch-program-args-input {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.65);
  padding: 5px 10px;
}

.launch-program-add-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  border: none;
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.25);
  color: var(--theme-accent);
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;
  align-self: flex-start;
}

.launch-program-add-btn:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.22);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.4);
}

.launch-program-add-btn svg {
  flex-shrink: 0;
}

/* ===== DLL Update Tab ===== */

/* Top info bar: source + version side by side */
.dll-top-bar {
  display: flex;
  gap: 14px;
  margin-bottom: 18px;
  align-items: stretch;
}

.dll-source-group {
  flex: 1;
  min-width: 0;
  padding: 14px 16px;
  border-radius: 10px;
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.075), rgba(var(--theme-surface-tint-rgb), 0.025)),
    rgba(255, 255, 255, 0.028);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.13);
  box-shadow: 0 10px 26px rgba(0, 0, 0, 0.14), 0 0 0 1px rgba(var(--theme-text-primary-rgb), 0.025) inset;
  display: flex;
  flex-direction: column;
  gap: 10px;
  position: relative;
}

.dll-source-label-row {
  display: flex;
  align-items: center;
  gap: 7px;
  color: rgba(var(--theme-text-primary-rgb), 0.76);
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.dll-source-label-row svg {
  flex-shrink: 0;
  color: var(--theme-accent);
  opacity: 0.8;
}

.dll-source-hint {
  font-size: 11px;
  color: rgba(var(--theme-text-secondary-rgb), 0.46);
  line-height: 1.4;
  letter-spacing: 0;
}

.package-source-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.package-source-value {
  display: inline-flex;
  align-items: center;
  max-width: min(100%, 420px);
  min-height: 30px;
  padding: 6px 12px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.18);
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.package-source-open-btn {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.20);
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  cursor: pointer;
  transition: transform 0.18s ease, background 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
}

.package-source-open-btn:hover {
  transform: translateY(-1px);
  background: rgba(var(--theme-surface-tint-rgb), 0.16);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.36);
  box-shadow: 0 6px 14px rgba(var(--theme-surface-tint-rgb), 0.12);
}

.package-source-open-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}

/* Current version badge */
.dll-version-badge {
  flex: 0 0 240px;
  display: flex;
  align-items: center;
}

.dll-version-badge-inner {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 16px;
  border-radius: 10px;
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.13), rgba(var(--theme-surface-tint-rgb), 0.045)),
    rgba(255, 255, 255, 0.03);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.18);
  box-shadow: 0 10px 26px rgba(0, 0, 0, 0.14), 0 0 0 1px rgba(var(--theme-text-primary-rgb), 0.025) inset;
}

.dll-version-badge-inner svg {
  flex-shrink: 0;
  color: var(--theme-accent);
  opacity: 0.7;
}

.dll-version-badge-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.dll-version-badge-label {
  font-size: 11px;
  color: rgba(var(--theme-text-secondary-rgb), 0.56);
  white-space: nowrap;
}

.dll-version-badge-value {
  font-size: 15px;
  font-weight: 700;
  color: var(--theme-accent);
  letter-spacing: 0.01em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Capped dev d3d11.dll version cap notice */
.dll-cap-hint {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 12px 14px;
  margin-bottom: 14px;
  border-radius: 8px;
  background: rgba(255, 186, 88, 0.09);
  border: 1px solid rgba(255, 186, 88, 0.26);
  color: rgba(var(--theme-text-primary-rgb), 0.86);
  font-size: 12px;
  line-height: 1.5;
}

.dll-cap-hint svg {
  flex-shrink: 0;
  margin-top: 1px;
  color: #ffb95c;
}

/* Release section */
.dll-release-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
  border-radius: 10px;
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.055), rgba(var(--theme-surface-tint-rgb), 0.018)),
    rgba(255, 255, 255, 0.022);
}

.dll-release-section-header {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dll-release-section-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.dll-release-section-title-row svg {
  flex-shrink: 0;
  color: var(--theme-accent);
  opacity: 0.7;
}

.dll-release-section-title {
  font-size: 15px;
  font-weight: 600;
  color: rgba(var(--theme-text-primary-rgb), 0.92);
}

.dll-release-section-subtitle {
  font-size: 12px;
  color: rgba(var(--theme-text-secondary-rgb), 0.50);
  padding-left: 24px;
  line-height: 1.4;
}

.dll-release-toolbar {
  display: flex;
  justify-content: flex-end;
}

.dll-refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 12px;
  border: none;
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
  color: rgba(var(--theme-text-secondary-rgb), 0.72);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.2s;
}

.dll-refresh-btn:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.14);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.28);
  color: rgba(var(--theme-text-primary-rgb), 0.94);
}

.dll-refresh-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* State messages */
.dll-release-state {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 18px 20px;
  border-radius: 8px;
  font-size: 13px;
  line-height: 1.5;
}

.dll-release-state-loading {
  background: rgba(var(--theme-surface-tint-rgb), 0.045);
  border: 1px dashed rgba(var(--theme-surface-tint-rgb), 0.16);
  color: rgba(var(--theme-text-secondary-rgb), 0.58);
}

.dll-release-state-error {
  background: rgba(232, 17, 35, 0.1);
  border: 1px solid rgba(232, 17, 35, 0.22);
  color: rgba(255, 166, 166, 0.9);
}

.dll-release-state-error svg {
  flex-shrink: 0;
  color: #ff6b6b;
}

.dll-state-title {
  font-weight: 600;
  margin-bottom: 2px;
}

.dll-state-detail {
  font-size: 12px;
  opacity: 0.7;
  word-break: break-all;
}

.dll-release-state-empty {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 24px 20px;
  border-radius: 10px;
  background: rgba(255, 255, 255, 0.03);
  border: 1px dashed rgba(255, 255, 255, 0.12);
  color: rgba(255, 255, 255, 0.4);
  font-size: 13px;
  justify-content: center;
}

.dll-release-state-empty svg {
  flex-shrink: 0;
  opacity: 0.5;
}

.dll-state-spinner {
  width: 18px;
  height: 18px;
  border: 2px solid rgba(255, 255, 255, 0.08);
  border-top-color: var(--theme-accent);
  border-radius: 50%;
  animation: dll-spin 0.8s linear infinite;
  flex-shrink: 0;
}

@keyframes dll-spin {
  to { transform: rotate(360deg); }
}

/* Release cards */
.dll-release-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dll-release-card {
  display: flex;
  align-items: stretch;
  gap: 0;
  min-height: 110px;
  border-radius: 8px;
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.045), rgba(var(--theme-surface-tint-rgb), 0.014)),
    rgba(255, 255, 255, 0.025);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
  overflow: hidden;
  transition: border-color 0.2s, background 0.2s, box-shadow 0.2s;
}

.dll-release-card:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.22);
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.075), rgba(var(--theme-surface-tint-rgb), 0.022)),
    rgba(255, 255, 255, 0.035);
  box-shadow: 0 8px 22px rgba(0, 0, 0, 0.14);
}

.dll-release-card-installed {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.2);
  background: rgba(var(--theme-surface-tint-rgb), 0.04);
}

.dll-release-card-installed:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.35);
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
}

.dll-release-card-top {
  flex: 1;
  min-width: 0;
  padding: 13px 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.dll-release-card-info {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.dll-release-version-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.dll-release-version {
  font-size: 15px;
  font-weight: 700;
  color: rgba(var(--theme-text-primary-rgb), 0.94);
  letter-spacing: 0.01em;
}

.dll-release-badge {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.dll-release-badge-latest {
  background: rgba(43, 196, 99, 0.14);
  border: 1px solid rgba(43, 196, 99, 0.3);
  color: #59d98c;
}

.dll-release-badge-prerelease {
  background: rgba(var(--theme-surface-tint-rgb), 0.14);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.28);
  color: var(--theme-accent);
}

.dll-release-installed-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 9px;
  border-radius: 999px;
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.22);
  color: var(--theme-accent);
  font-size: 11px;
  font-weight: 600;
  flex-shrink: 0;
}

.dll-release-installed-tag svg {
  flex-shrink: 0;
}

.dll-release-description {
  color: rgba(var(--theme-text-secondary-rgb), 0.58);
  font-size: 12px;
  line-height: 1.55;
  word-break: break-word;
  max-height: 160px;
  overflow-y: auto;
  padding-right: 4px;
}

:global(.release-notes-confirm) {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: min(58vh, 520px);
  overflow-y: auto;
  padding-right: 4px;
  text-align: left;
}

:global(.release-notes-confirm-text) {
  white-space: pre-wrap;
  color: var(--el-text-color-regular);
  line-height: 1.65;
}

.dll-release-description::-webkit-scrollbar {
  width: 6px;
}

.dll-release-description::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.12);
  border-radius: 999px;
}

.dll-release-description::-webkit-scrollbar-track {
  background: rgba(255, 255, 255, 0.03);
  border-radius: 999px;
}

/* Card right side - install button area */
.dll-release-card-actions {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 136px;
  padding: 14px;
  border-left: 1px solid rgba(var(--theme-surface-tint-rgb), 0.08);
  background: rgba(0, 0, 0, 0.08);
  flex-shrink: 0;
}

.dll-install-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 108px;
  min-height: 34px;
  padding: 8px 10px;
  border: none;
  border-radius: 8px;
  background: linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), 0.20) 0%, rgba(var(--theme-surface-tint-rgb), 0.10) 100%);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.30);
  color: rgba(var(--theme-text-primary-rgb), 0.96);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s;
  white-space: nowrap;
}

.dll-install-btn:hover {
  background: linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), 0.30) 0%, rgba(var(--theme-surface-tint-rgb), 0.16) 100%);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.46);
  box-shadow: 0 4px 12px rgba(var(--theme-surface-tint-rgb), 0.14);
}

.dll-install-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
  box-shadow: none;
}

.dll-install-btn svg {
  flex-shrink: 0;
}

.dll-install-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid rgba(127, 200, 255, 0.2);
  border-top-color: #7fc8ff;
  border-radius: 50%;
  animation: dll-spin 0.8s linear infinite;
}

/* Load more button */
.dll-load-more-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 10px 20px;
  border: none;
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.1);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.2);
  color: var(--theme-accent);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s;
  align-self: center;
  margin-top: 4px;
}

.dll-load-more-btn:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.18);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.35);
}

.dll-load-more-btn svg {
  flex-shrink: 0;
}

/* Sidebar */
.settings-sidebar {
  width: 220px;
  background: rgba(0, 0, 0, 0.25);
  border-right: 1px solid rgba(255, 255, 255, 0.04);
  display: flex;
  flex-direction: column;
  padding: 24px 0 16px;
  flex-shrink: 0;
}

.sidebar-title {
  font-size: 15px;
  font-weight: 700;
  color: rgba(255, 255, 255, 0.9);
  padding: 0 20px 18px 20px;
  margin: 0 12px 12px 12px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  letter-spacing: 0.02em;
}

.sidebar-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 11px 20px;
  margin: 1px 8px;
  border-radius: 8px;
  color: rgba(255, 255, 255, 0.5);
  cursor: pointer;
  transition: all 0.2s;
  font-size: 13px;
  font-weight: 500;
  position: relative;
  min-height: 40px;
  line-height: 1.25;
}

.sidebar-item:hover {
  background: rgba(255, 255, 255, 0.05);
  color: rgba(255, 255, 255, 0.8);
}

.sidebar-item.active {
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: var(--theme-accent);
  font-weight: 600;
}

.sidebar-item.active::before {
  content: '';
  position: absolute;
  left: -6px;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 18px;
  border-radius: 0 3px 3px 0;
  background: var(--theme-accent);
  box-shadow: 0 0 8px rgba(var(--theme-surface-tint-rgb), 0.3);
}

.sidebar-item svg {
  flex-shrink: 0;
  opacity: 0.6;
  transition: opacity 0.2s;
}

.sidebar-item:hover svg {
  opacity: 0.9;
}

.sidebar-item.active svg {
  opacity: 1;
  color: var(--theme-accent);
}

/* Content */
.settings-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.content-header {
  height: 60px;
  padding: 0 30px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  flex-shrink: 0;
}

.header-title {
  font-size: 17px;
  font-weight: 700;
  color: #fff;
  letter-spacing: 0.01em;
}

.close-btn {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  cursor: pointer;
  color: rgba(255, 255, 255, 0.35);
  transition: all 0.2s;
}

.close-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
}

.scroll-content {
  flex: 1;
  padding: 28px 32px;
  overflow-y: auto;
}



.setting-group {
  margin-bottom: 24px;
}

.setting-label {
  display: block;
  font-size: 14px;
  color: rgba(255, 255, 255, 0.8);
  margin-bottom: 8px;
}

.custom-input {
  width: 100%;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 4px;
  padding: 8px 12px;
  color: #fff;
  font-size: 14px;
  outline: none;
  transition: border-color 0.2s;
}

.custom-input:focus {
  border-color: var(--theme-accent);
}

.button-row {
  display: flex;
  gap: 12px;
  margin-top: 12px;
}

.action-btn {
  padding: 8px 16px;
  border: none;
  border-radius: 4px;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;
  flex: 1;
  color: #fff;
  background: rgba(255, 255, 255, 0.1);
}

.action-btn:hover {
  background: rgba(255, 255, 255, 0.2);
}

.action-btn:disabled {
  cursor: not-allowed;
  opacity: 0.6;
}

.action-btn.create {
  background: rgba(var(--theme-surface-tint-rgb), 0.2);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.4);
  color: var(--theme-accent);
}

.action-btn.create:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.3);
}

.action-btn.highlight {
  background: rgba(0, 122, 204, 0.3);
  border: 1px solid rgba(0, 122, 204, 0.5);
  color: #61afef;
}

.action-btn.highlight:hover {
  background: rgba(0, 122, 204, 0.5);
}

.action-btn.delete {
  background: rgba(232, 17, 35, 0.2);
  border: 1px solid rgba(232, 17, 35, 0.4);
  color: #ff6b6b;
}

.action-btn.delete:hover {
  background: rgba(232, 17, 35, 0.3);
}

.empty-state {
  color: rgba(255, 255, 255, 0.3);
  text-align: center;
  margin-top: 40px;
}

/* ===== Settings - Card Layout ===== */
.settings-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  margin-bottom: 14px;
}

.settings-card {
  padding: 16px 18px;
  border-radius: 12px;
  background: var(--t-surface-subtle);
  border: 1px solid rgba(255, 255, 255, 0.04);
  box-shadow: none;
  transition: all 0.25s ease;
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.settings-card::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
  border-radius: 12px 12px 0 0;
}

.settings-card-full {
  margin-bottom: 14px;
}

.settings-card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.8);
  padding-bottom: 8px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  letter-spacing: 0.01em;
}

/* 游戏预设名较长（如 "Zenless Zone Zero(ZZMI)"）：选中值进入正常文档流并换行，
   输入框随内容自动变高，完整显示预设名而不是单行省略 */
.custom-select :deep(.el-select__placeholder) {
  position: relative;
  top: auto;
  transform: none;
  flex: 1;
  min-width: 0;
  white-space: normal;
  overflow-wrap: anywhere;
  text-overflow: clip;
  overflow: visible;
  line-height: 1.3;
}

.custom-select :deep(.el-select__wrapper) {
  height: auto;
}

.settings-card-header svg {
  flex-shrink: 0;
  color: var(--theme-accent);
  opacity: 0.75;
}

.settings-card-header span {
  min-width: 0;
}

.settings-header-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  margin-left: auto;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.22);
  border-radius: 6px;
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: var(--theme-accent);
  cursor: pointer;
  transition: all 0.2s;
}

.settings-header-icon-btn:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.42);
  background: rgba(var(--theme-surface-tint-rgb), 0.16);
  color: #EAF8FF;
}

.settings-header-icon-btn svg {
  color: currentColor;
  opacity: 0.9;
}

/* Inline path row (input + small icon buttons) */
.settings-path-row {
  display: flex;
  gap: 6px;
}

.settings-path-row .settings-path-input {
  flex: 1;
  min-width: 0;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 6px;
  padding: 7px 10px;
  color: rgba(255, 255, 255, 0.85);
  font-size: 13px;
  outline: none;
  transition: border-color 0.2s;
}

.settings-path-row .settings-path-input:focus {
  border-color: var(--theme-accent);
}

.settings-path-row .settings-path-input::placeholder {
  color: rgba(255, 255, 255, 0.25);
}

.settings-sm-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.45);
  cursor: pointer;
  transition: all 0.2s;
  flex-shrink: 0;
}

.settings-sm-btn:hover {
  background: rgba(255, 255, 255, 0.14);
  color: rgba(255, 255, 255, 0.85);
}

.settings-sm-btn.highlight {
  background: rgba(0, 122, 204, 0.2);
  border: 1px solid rgba(0, 122, 204, 0.3);
  color: #61afef;
}

.settings-sm-btn.highlight:hover {
  background: rgba(0, 122, 204, 0.35);
}

.settings-sm-btn.folder {
  color: var(--theme-accent);
  opacity: 0.7;
}

.settings-sm-btn.folder:hover {
  opacity: 1;
}

/* Label with icon row for compact fields */
.settings-field-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.6);
  letter-spacing: 0.02em;
}

.settings-field-label svg {
  flex-shrink: 0;
  opacity: 0.5;
}

/* Inline checkbox toggle row */
.settings-toggle-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 0;
}

.settings-toggle-row + .settings-toggle-row {
  border-top: 1px solid rgba(255, 255, 255, 0.04);
  padding-top: 10px;
}

.settings-toggle-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: rgba(255, 255, 255, 0.75);
  cursor: pointer;
  user-select: none;
}

.settings-toggle-label input[type="checkbox"] {
  accent-color: var(--theme-accent);
  width: 15px;
  height: 15px;
  cursor: pointer;
}

/* Inline toggle row (label left, switch right) */
.settings-toggle-row-inline {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 0 4px;
}

.settings-toggle-row-info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.settings-toggle-row-title {
  font-size: 13px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.8);
}

.settings-toggle-row-hint {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.35);
  line-height: 1.4;
}

.settings-toggle-row-inline:hover .settings-toggle-row-title {
  color: rgba(255, 255, 255, 0.95);
}

.settings-toggle-row-inline:hover .settings-toggle-row-hint {
  color: rgba(255, 255, 255, 0.45);
}

/* Crystal accent for switch */
.settings-toggle-row-inline :deep(.el-switch.is-checked .el-switch__core) {
  border-color: var(--theme-success);
  background-color: var(--theme-success);
}

/* Numeric input small */
.settings-num-input {
  width: 100%;
  box-sizing: border-box;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 6px;
  padding: 7px 10px;
  color: var(--theme-accent);
  font-size: 13px;
  font-weight: 600;
  outline: none;
  transition: border-color 0.2s;
}

.settings-num-input:focus {
  border-color: var(--theme-accent);
}

/* Two-column flex */
.settings-flex-row {
  display: flex;
  gap: 12px;
}

.settings-flex-row > * {
  flex: 1;
}

/* Icon button for actions inside cards */
.settings-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 7px 14px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.7);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.2s;
}

.settings-action-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: rgba(255, 255, 255, 0.9);
}

.settings-action-btn svg {
  flex-shrink: 0;
}

.settings-action-btn.highlight {
  background: rgba(0, 122, 204, 0.2);
  border: 1px solid rgba(0, 122, 204, 0.35);
  color: #61afef;
}

.settings-action-btn.highlight:hover {
  background: rgba(0, 122, 204, 0.35);
}

.settings-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.7);
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s;
  align-self: flex-start;
}

.settings-icon-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: rgba(255, 255, 255, 0.9);
}

.settings-icon-btn svg {
  flex-shrink: 0;
}

.settings-bg-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  padding-top: 4px;
}

.settings-bg-actions .settings-action-btn {
  flex: 0 0 auto;
}

/* Transitions */
.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.25s ease;
}

.modal-fade-enter-active .settings-window,
.modal-fade-leave-active .settings-window {
  transition: transform 0.25s cubic-bezier(0.16, 1, 0.3, 1);
}

.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}

.modal-fade-enter-from .settings-window {
  transform: translateY(16px) scale(0.98);
}

.modal-fade-leave-to .settings-window {
  transform: translateY(8px) scale(0.99);
}

.loading-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.65);
  z-index: 100;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
}

.spinner {
  width: 36px;
  height: 36px;
  border: 3px solid rgba(255, 255, 255, 0.06);
  border-top-color: var(--theme-accent);
  border-radius: 50%;
  animation: settingsSpin 0.8s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  margin-bottom: 14px;
}

.loading-text {
  color: var(--theme-accent);
  font-size: 14px;
  font-weight: 500;
  letter-spacing: 0.02em;
}

@keyframes settingsSpin {
  to {
    transform: rotate(360deg);
  }
}
</style>
