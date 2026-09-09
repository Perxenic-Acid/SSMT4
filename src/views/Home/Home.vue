<script setup lang="ts" >
import { computed, ref, onMounted, onUnmounted, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { AppStateManager } from '../../store/AppStateManager'

import { openPath, openUrl } from '@tauri-apps/plugin-opener'
import { exists, mkdir } from '@tauri-apps/plugin-fs'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useRouter } from 'vue-router'
import { ResourceManager, type UpdateInfo } from '../../store/ResourceManager'
import { handOffModInstallDrop } from '../../store/ModInstallDropBus'
import { join } from '@tauri-apps/api/path'
import { getVersion } from '@tauri-apps/api/app'
import { PathHelper } from '../../helper/PathHelper'
import GameSettingsModal from '../../components/GameSettingsModal.vue'
import SettingsMenu from '../../components/SettingsMenu.vue'
import ReleaseNotesMarkdown from '../../components/ReleaseNotesMarkdown.vue'

import { MigotoManager } from '../../store/MigotoManager'
import { LaunchGame } from '../../common/LaunchGame';
import { pickNewerUpdate, UPDATE_CHECK_TIMEOUT_MS } from '../../common/UpdateCheckUtils';

import { GameConfig, GameConfigManager, type D3d11Mode } from '../../store/GameConfig';
import { getGithubRepoByGamePreset } from '../../store/GamePreset';
import {
  checkAndInstallAppUpdate,
  isInstallingAppUpdate,
} from '../../common/AppSelfUpdate'

const appSettings = AppStateManager.appSettings;
const { t } = useI18n()
const router = useRouter()
let unlistenNativeDrop: UnlistenFn | null = null;
const currentGameConfig = ref<GameConfig>(GameConfigManager.defaultGameConfig())
const appVersion = ref('')

const currentGamePreset = computed(() => (currentGameConfig.value.gamePreset || '').trim().toUpperCase())
const isCurrentPresetNtemi = computed(() => currentGamePreset.value === 'NTEMI')

const loadCurrentGameConfig = async () => {
  const gameName = appSettings.CurrentGameName?.trim()
  if (!gameName || gameName === 'Default') {
    currentGameConfig.value = GameConfigManager.defaultGameConfig()
    return
  }

  try {
    currentGameConfig.value = await ResourceManager.loadGameConfig(gameName)
  } catch (error) {
    console.error('Failed to load current game config:', error)
    currentGameConfig.value = GameConfigManager.defaultGameConfig()
  }
}

const packageVersionText = computed(() => {
  const packageVersion = (currentGameConfig.value.packageVersion || '').trim()
  if (!packageVersion) {
    return ''
  }

  const gamePreset = (currentGameConfig.value.gamePreset || '').trim()
  if (!gamePreset) {
    return `Package ${packageVersion}`
  }

  const repo = getGithubRepoByGamePreset(gamePreset)
  if (!repo) {
    return `Package ${packageVersion}`
  }

  const repoName = repo.split('/').pop()?.trim()
  if (!repoName) {
    return `Package ${packageVersion}`
  }

  return `${repoName} ${packageVersion}`
})

const currentD3d11Mode = computed<D3d11Mode>(() => ResourceManager.getEffectiveD3d11Mode(currentGameConfig.value))

const getD3d11ModeLabel = (mode: D3d11Mode): string => {
  if (mode === 'play') {
    return 'Play'
  }

  if (mode === 'ssice-a') {
    return 'ssice-a'
  }

  return 'Dev'
}

const CAPPED_DEV_D3D11_PRESETS = new Set(['IDENTITYV', 'NARAKA', 'NARAKAM'])
const isCappedDevD3d11Mode = (mode: D3d11Mode, gamePreset?: string): boolean => (
  mode === 'dev' && CAPPED_DEV_D3D11_PRESETS.has((gamePreset || '').trim().toUpperCase())
)

const getStoredDllVersion = (mode: D3d11Mode, gamePreset?: string): string => {
  if (isCappedDevD3d11Mode(mode, gamePreset)) {
    return (appSettings.coreVersionIdentityVDev || '').trim()
  }
  if (mode === 'play') {
    return (appSettings.coreVersionPlay || '').trim()
  }

  if (mode === 'ssice-a') {
    return (appSettings.coreVersionSsiceA || '').trim()
  }

  return (appSettings.coreVersionDev || appSettings.coreVersion || '').trim()
}

const getStoredDllReleaseDescription = (mode: D3d11Mode, gamePreset?: string): string => {
  if (isCappedDevD3d11Mode(mode, gamePreset)) {
    return (appSettings.coreReleaseDescriptionIdentityVDev || '').trim()
  }
  if (mode === 'play') {
    return (appSettings.coreReleaseDescriptionPlay || '').trim()
  }

  if (mode === 'ssice-a') {
    return (appSettings.coreReleaseDescriptionSsiceA || '').trim()
  }

  return (appSettings.coreReleaseDescriptionDev || appSettings.coreReleaseDescription || '').trim()
}

const coreVersionText = computed(() => {
  const coreVersion = getStoredDllVersion(currentD3d11Mode.value, currentGameConfig.value.gamePreset)
  if (!coreVersion) {
    return ''
  }

  const modeLabel = getD3d11ModeLabel(currentD3d11Mode.value)
  return `${modeLabel} Core ${coreVersion}`
})

const packageReleaseDescriptionText = computed(() => {
  const description = (currentGameConfig.value.packageReleaseDescription || '').trim()
  return description || t('home.versionInfo.noReleaseNotes')
})

const coreReleaseDescriptionText = computed(() => {
  const description = getStoredDllReleaseDescription(currentD3d11Mode.value, currentGameConfig.value.gamePreset)
  return description || t('home.versionInfo.noReleaseNotes')
})

const appVersionText = computed(() => {
  const version = appVersion.value.trim()
  if (!version) {
    return ''
  }
  return `SSMT ${version}`
})

const currentDllModeText = computed(() => {
  const gameName = appSettings.CurrentGameName?.trim()
  if (!gameName || gameName === 'Default') {
    return ''
  }

  const mode = currentD3d11Mode.value
  return `${t('home.status.currentDllMode')}: ${getD3d11ModeLabel(mode)}`
})

const openAppReleasePage = async () => {
  try {
    await openUrl('https://github.com/Perxenic-Acid/SSMT4/releases')
  } catch (error) {
    ElMessage.error(t('home.messages.operationFailed', { error: String(error) }))
  }
}

const handleCheckAndInstallAppUpdate = async () => {
  await checkAndInstallAppUpdate('manual')
}

const hasVersionInfo = computed(() => Boolean(packageVersionText.value || coreVersionText.value || appVersionText.value || currentDllModeText.value))

const open3dmigotoFolder = async () => {
  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') return;

  try {
    const path = await PathHelper.GetCurrentGame3DmigotoFolderPath();

    if (path) {
      await mkdir(path, { recursive: true });
      await openPath(path);
    } else {
      console.warn('No 3Dmigoto path found and no cache dir set.');
    }
  } catch (e) {
    console.error('Failed to open 3Dmigoto folder:', e);
  }
};

const openD3dxIni = async () => {
  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') return;

  try {
    const path = await PathHelper.GetCurrentGame3DmigotoFolderPath();

    if (path) {
      await mkdir(path, { recursive: true });
      const iniPath = await join(path, 'd3dx.ini');
      await openPath(iniPath);
    }
  } catch (e) {
    console.error('Failed to open d3dx.ini:', e);
  }
};

const showSettings = ref(false);
const settingsModalRef = ref<InstanceType<typeof GameSettingsModal> | null>(null);
const isUpdatingPackage = ref(false);
const isStartButtonDisabled = computed(() => (
  isLaunching.value
  || isUpdatingPackage.value
));

type StartGameUpdateCheckResult = {
  dllUpdate: UpdateInfo | null;
  packageUpdate: UpdateInfo | null;
};

type LaunchPrecheckConfig = Pick<GameConfig, 'packageVersion' | 'gamePreset' | 'd3d11Mode' | 'allowDllUpdates' | 'checkDllUpdateBeforeLaunch' | 'check3DmigotoPackageUpdateBeforeLaunch' | 'includePrereleaseUpdates'>;

const openSettingsTo3Dmigoto = () => {
  showSettings.value = true;
  // Ensure modal is mounted before switching tabs.
  setTimeout(() => {
    settingsModalRef.value?.switchTab('3dmigoto');
  }, 100);
};

const check3DMigotoPackageUpdate = async () => {
  if (isUpdatingPackage.value) return;
  isUpdatingPackage.value = true;
  try {
    const updated = await settingsModalRef.value?.runPackageUpdate();
    if (updated) {
      await loadCurrentGameConfig()
    }
    return updated
  } finally {
    isUpdatingPackage.value = false;
  }
};

const checkD3D11DllUpdate = async () => {
  if (isUpdatingPackage.value) return;
  isUpdatingPackage.value = true;
  try {
    return await settingsModalRef.value?.runDllUpdate();
  } finally {
    isUpdatingPackage.value = false;
  }
};

const precheckStartGameUpdates = async (config: LaunchPrecheckConfig): Promise<StartGameUpdateCheckResult> => {
  const d3d11Mode = ResourceManager.getEffectiveD3d11Mode(config);
  const gamePreset = (config.gamePreset || '').trim();
  const currentCoreVersion = getStoredDllVersion(d3d11Mode, gamePreset);
  const currentPackageVersion = (config.packageVersion || '').trim();
  const shouldCheckDllUpdate = config.allowDllUpdates !== false && config.checkDllUpdateBeforeLaunch !== false;
  const shouldCheckPackageUpdate = config.check3DmigotoPackageUpdateBeforeLaunch !== false;

  const dllPromise = shouldCheckDllUpdate && currentCoreVersion
    ? pickNewerUpdate(
        () => ResourceManager.getD3d11LatestRelease(
          d3d11Mode,
          appSettings.githubToken,
          config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
          gamePreset,
        ),
        currentCoreVersion,
        UPDATE_CHECK_TIMEOUT_MS,
      )
    : Promise.resolve(null);

  const packagePromise = shouldCheckPackageUpdate && currentPackageVersion && gamePreset && getGithubRepoByGamePreset(gamePreset)
    ? pickNewerUpdate(
        () => ResourceManager.get3DMigotoLatestRelease(
          gamePreset,
          appSettings.githubToken,
          config.includePrereleaseUpdates ?? appSettings.includePrereleaseUpdates,
        ),
        currentPackageVersion,
        UPDATE_CHECK_TIMEOUT_MS,
      )
    : Promise.resolve(null);

  const [dllUpdate, packageUpdate] = await Promise.all([dllPromise, packagePromise]);

  return {
    dllUpdate,
    packageUpdate,
  };
};

const installPrecheckedUpdatesBeforeLaunch = async () => {
  if (isUpdatingPackage.value) {
    return;
  }

  const gameName = appSettings.CurrentGameName?.trim();
  if (!gameName || gameName === 'Default') {
    return;
  }

  isUpdatingPackage.value = true;
  try {
    const latestConfig = await ResourceManager.loadGameConfig(gameName);
    currentGameConfig.value = latestConfig;

    if (latestConfig.checkDllUpdateBeforeLaunch === false && latestConfig.check3DmigotoPackageUpdateBeforeLaunch === false) {
      return;
    }

    const { dllUpdate, packageUpdate } = await precheckStartGameUpdates(latestConfig);

    if (dllUpdate) {
      await settingsModalRef.value?.installDllUpdateWithInfo(dllUpdate);
    }

    if (packageUpdate) {
      const updated = await settingsModalRef.value?.installPackageUpdateWithInfo(packageUpdate);
      if (updated) {
        await loadCurrentGameConfig();
      }
    }
  } finally {
    isUpdatingPackage.value = false;
  }
};

const switchD3d11Mode = async (mode: D3d11Mode) => {
  const gameName = appSettings.CurrentGameName?.trim();
  if (!gameName || gameName === 'Default') {
    ElMessage.info(t('home.messages.selectGameConfigFirst'));
    return;
  }

  try {
    const requestedMode = ResourceManager.getEffectiveD3d11Mode({
      ...currentGameConfig.value,
      d3d11Mode: mode,
    });
    const sourceDllPath = await ResourceManager.resolveD3d11SourcePathByMode(requestedMode, currentGameConfig.value.gamePreset);
    if (!(await exists(sourceDllPath))) {
      const updated = await settingsModalRef.value?.runDllUpdate(requestedMode);
      if (!updated) {
        return;
      }
    }

    const appliedMode = await MigotoManager.switchD3d11Mode(gameName, requestedMode);
    await loadCurrentGameConfig();
    ElMessage.success(
      appliedMode === 'play'
        ? t('home.messages.switchedToPlayDll')
        : appliedMode === 'ssice-a'
          ? t('home.messages.switchedToSsiceADll')
        : t('home.messages.switchedToDevDll'),
    );
  } catch (e) {
    console.error('Failed to switch d3d11 mode:', e);
    ElMessage.error(t('home.messages.operationFailed', { error: String(e) }));
  }
};

// Start Game Logic
const isLaunching = ref(false);

const launchGame = async (event?: MouseEvent) => {
  if (isStartButtonDisabled.value) return;

  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') {
    ElMessage.info(t('home.messages.selectGameConfigFirst'));
    return;
  }

  isLaunching.value = true;
  
  try {
      await installPrecheckedUpdatesBeforeLaunch();
      await LaunchGame.launch(
        gameName,
        appSettings,
        check3DMigotoPackageUpdate,
        openSettingsTo3Dmigoto,
        checkD3D11DllUpdate,
        event?.ctrlKey ?? false
      );
  } finally {
    setTimeout(() => {
      isLaunching.value = false;
    }, 1000);
  }
}

onMounted(() => {
  void getVersion()
    .then(version => {
      appVersion.value = version
    })
    .catch(error => {
      console.error('Failed to get app version:', error)
    })

  // 主页拖入压缩包/文件夹 → 转交模组管理页安装(仅主页与模组管理页触发)。
  void listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
    if (router.currentRoute.value.path !== '/') return;
    const paths = (event.payload?.paths ?? []).map(path => path.trim()).filter(Boolean);
    if (paths.length === 0) return;
    if (!AppStateManager.hasSelectedGame()) {
      ElMessage.info(t('home.messages.selectGameConfigFirst'));
      return;
    }
    handOffModInstallDrop(paths);
    await router.push('/mods');
  }).then((unlisten) => {
    unlistenNativeDrop = unlisten;
  }).catch((error) => {
    console.error('Failed to attach Home drop listener', error);
  });
});

onUnmounted(() => {
  unlistenNativeDrop?.();
  unlistenNativeDrop = null;
});

watch(
  () => appSettings.CurrentGameName,
  () => {
    void loadCurrentGameConfig()
  },
  { immediate: true }
)
</script>

<template>
  <div class="home-container">
    <div class="content-area">



    </div>

    <div v-if="hasVersionInfo" class="version-info-panel">
      <el-popover
        v-if="appVersionText"
        trigger="hover"
        placement="top-start"
        :width="220"
        popper-class="version-info-popper version-info-popper-menu"
      >
        <template #reference>
          <div class="version-info-chip">{{ appVersionText }}</div>
        </template>
        <div class="version-popover-menu">
          <button class="version-popover-menu-item" @click="handleCheckAndInstallAppUpdate">
            {{ isInstallingAppUpdate ? t('settings.actions.installingUpdate') : t('settings.actions.checkAppUpdate') }}
          </button>
          <button class="version-popover-menu-item" @click="openAppReleasePage">
            {{ t('settings.about.releasePage') }}
          </button>
        </div>
      </el-popover>

      <el-popover
        v-if="coreVersionText"
        trigger="hover"
        placement="top-start"
        :width="420"
        popper-class="version-info-popper"
      >
        <template #reference>
          <div class="version-info-chip">{{ coreVersionText }}</div>
        </template>
        <div class="version-popover-content">
          <div class="version-popover-title">{{ t('home.versionInfo.coreTitle') }}</div>
          <div class="version-popover-version">{{ coreVersionText }}</div>
          <ReleaseNotesMarkdown
            class="version-popover-body"
            :content="coreReleaseDescriptionText"
            compact
          />
        </div>
      </el-popover>

      <el-popover
        v-if="packageVersionText"
        trigger="hover"
        placement="top-start"
        :width="420"
        popper-class="version-info-popper"
      >
        <template #reference>
          <div class="version-info-chip">{{ packageVersionText }}</div>
        </template>
        <div class="version-popover-content">
          <div class="version-popover-title">{{ t('home.versionInfo.packageTitle') }}</div>
          <div class="version-popover-version">{{ packageVersionText }}</div>
          <ReleaseNotesMarkdown
            class="version-popover-body"
            :content="packageReleaseDescriptionText"
            compact
          />
        </div>
      </el-popover>

      <div v-if="currentDllModeText" class="version-info-chip version-info-chip-mode">{{ currentDllModeText }}</div>
    </div>

    <!-- Settings Modal -->
    <GameSettingsModal ref="settingsModalRef" v-model="showSettings" :game-name="appSettings.CurrentGameName" />

    <div class="action-bar">
      <!-- Start Game Button -->
      <div class="start-game-btn" @click="launchGame" :class="{ 'disabled': isStartButtonDisabled }">
        <div class="icon-wrapper">
          <div v-if="isUpdatingPackage" class="loading-spinner"></div>
          <div v-else class="play-triangle"></div>
        </div>
        <span class="btn-text">{{ t('home.actions.startGame') }}</span>
      </div>

      <!-- Settings Menu -->
      <SettingsMenu
        :show-settings="showSettings"
        :is-current-preset-ntemi="isCurrentPresetNtemi"
        @update:show-settings="showSettings = $event"
        @switch-d3d11-mode="switchD3d11Mode"
        @open-3dmigoto-folder="open3dmigotoFolder"
        @open-d3dx-ini="openD3dxIni"
        @check-d3-d11-dll-update="checkD3D11DllUpdate"
        @check-3-d-migoto-package-update="check3DMigotoPackageUpdate"
      />
    </div>





  </div>
</template>

<style scoped>
.home-container {
  --version-panel-gap: 24px;
  --bottom-ui-gap: 40px;
  --action-bar-width: 310px;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  padding: 0;
  box-sizing: border-box;
  position: relative;
}

.content-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 40px;
  /* Restore padding here */
  position: relative;
  z-index: 1;
  /* Ensure content sits above shadow */
}

.version-info-panel {
  position: absolute;
  left: var(--version-panel-gap);
  bottom: var(--bottom-ui-gap);
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  color: rgba(255, 255, 255, 0.7);
  font-size: 12px;
  font-weight: 500;
  line-height: 1.3;
  letter-spacing: 0.02em;
  z-index: 2;
  flex-wrap: wrap;
  max-width: calc(100% - var(--action-bar-width) - (var(--version-panel-gap) * 3));
}

.version-info-chip {
  padding: 4px 14px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.02);
  border: 1px solid rgba(255, 255, 255, 0.06);
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.06);
  transition: all 0.25s ease;
  position: relative;
  cursor: default;
  white-space: nowrap;
  color: rgba(255, 255, 255, 0.75);
}

.version-info-chip::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
  border-radius: 999px 999px 0 0;
}

.version-info-chip > * {
  position: relative;
  z-index: 2;
}

.version-info-chip:hover {
  background: rgba(255, 255, 255, 0.035);
  border-color: rgba(255, 255, 255, 0.1);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.1);
  color: var(--theme-accent);
}

.version-info-chip-mode {
  color: var(--theme-accent);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.25);
}

.version-info-chip-mode:hover {
  color: var(--theme-accent);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.45);
}

:deep(.version-info-popper) {
  padding: 16px 20px !important;
}

:deep(.version-info-popper)::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
  border-radius: 14px 14px 0 0;
}

.version-popover-content {
  display: flex;
  flex-direction: column;
  gap: 10px;
  color: rgba(255, 255, 255, 0.85);
}

.version-popover-title {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: rgba(var(--theme-surface-tint-rgb), 0.6);
}

.version-popover-version {
  font-size: 14px;
  font-weight: 700;
  color: rgba(255, 255, 255, 0.96);
  letter-spacing: 0.02em;
}

.version-popover-body {
  max-height: 280px;
  overflow-y: auto;
  font-size: 12px;
  line-height: 1.55;
  color: rgba(255, 255, 255, 0.82);
  word-break: break-word;

  scrollbar-width: thin;
  scrollbar-color: rgba(255, 255, 255, 0.25) transparent;
}

.version-popover-body::-webkit-scrollbar {
  width: 6px;
}

.version-popover-body::-webkit-scrollbar-track {
  background: transparent;
}

.version-popover-body::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.25);
  border-radius: 4px;
}

.version-popover-body::-webkit-scrollbar-thumb:hover {
  background: rgba(255, 255, 255, 0.42);
}

.version-popover-menu {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.version-popover-menu-item {
  border: 0;
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
  color: rgba(var(--theme-surface-tint-rgb), 0.75);
  border-radius: 8px;
  padding: 8px 12px;
  text-align: left;
  font-size: 12px;
  line-height: 1.4;
  cursor: pointer;
  transition: all 0.2s;
}

.version-popover-menu-item:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  color: var(--theme-accent);
}

.action-bar {
  position: absolute;
  right: 40px;
  bottom: var(--bottom-ui-gap);
  display: flex;
  height: 60px;
  gap: 10px;
  z-index: 3;
}

/* --- Start Game Button (Glass Card) --- */
.start-game-btn {
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 30px;
  padding: 0 44px 0 12px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.08);
  transition: all 0.25s ease;
  position: relative;
  color: rgba(255, 255, 255, 0.9);
  display: flex;
  align-items: center;
  cursor: pointer;
  font-family: 'Microsoft YaHei', sans-serif;
}

.start-game-btn::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
  border-radius: 30px 30px 0 0;
}

.start-game-btn > * {
  position: relative;
  z-index: 2;
}

.start-game-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  border-color: rgba(255, 255, 255, 0.2);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.12);
  transform: translateY(-2px);
}

.start-game-btn:active {
  transform: translateY(0) scale(0.98);
}

.start-game-btn .btn-text {
  font-size: 18px;
  font-weight: 700;
  margin-left: 30px;
  letter-spacing: 2px;
  color: rgba(255, 255, 255, 0.85);
  transition: color 0.25s;
}

.icon-wrapper {
  width: 36px;
  height: 36px;
  background: rgba(255, 255, 255, 0.12);
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.25s cubic-bezier(0.25, 1, 0.5, 1);
}

.play-triangle {
  width: 0;
  height: 0;
  border-style: solid;
  border-width: 6px 0 6px 10px;
  border-color: transparent transparent transparent rgba(255, 255, 255, 0.7);
  margin-left: 2px;
  transition: all 0.25s cubic-bezier(0.25, 1, 0.5, 1);
}

.loading-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.1);
  border-top-color: rgba(255, 255, 255, 0.5);
  border-radius: 50%;
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.start-game-btn.disabled {
  pointer-events: none;
  opacity: 0.35;
  filter: grayscale(0.6);
}

.start-game-btn:hover .btn-text {
  color: var(--theme-accent);
}

.start-game-btn:hover .icon-wrapper {
  background: rgba(var(--theme-surface-tint-rgb), 0.15);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.3);
  transform: scale(1.06);
}

.start-game-btn:hover .play-triangle {
  border-color: transparent transparent transparent var(--theme-accent);
}

.start-game-btn:active .icon-wrapper {
  transform: scale(0.95);
}

.start-game-btn:active .play-triangle {
  border-color: transparent transparent transparent rgba(var(--theme-surface-tint-rgb), 0.5);
}

/* Settings button styles are now in SettingsMenu.vue */

@media (max-width: 980px) {
  .home-container {
    --bottom-ui-gap: 28px;
    --version-panel-gap: 18px;
  }

  .version-info-panel {
    bottom: calc(var(--bottom-ui-gap) + 72px);
    max-width: calc(100% - (var(--version-panel-gap) * 2));
  }

  .action-bar {
    right: 28px;
  }
}

@media (max-width: 720px) {
  .version-info-panel {
    right: 18px;
    max-width: none;
  }

  .action-bar {
    right: 18px;
  }

  .start-game-btn {
    padding-right: 28px;
  }

  .start-game-btn .btn-text {
    margin-left: 18px;
    font-size: 16px;
    letter-spacing: 1px;
  }
}
</style>

<style>
.version-info-popper {
  border: var(--t-material-border) !important;
  background: var(--t-material-bg) !important;
  box-shadow: var(--t-material-shadow) !important;
  border-radius: 14px !important;
  padding: 16px 20px !important;
}
</style>
