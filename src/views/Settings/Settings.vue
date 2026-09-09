<script setup lang="ts">
import { AppStateManager, type GameInfo } from '../../store/AppStateManager'
import { ResourceManager } from '../../store/ResourceManager'
import { getGamePresetDisplayName, getGamePresetOptions } from '../../store/GamePreset'
import { convertFileSrc } from '@tauri-apps/api/core';
import {
  APP_UI_SCALE_MAX,
  APP_UI_SCALE_MIN,
  APP_UI_SCALE_STEP,
  SSMT_LOCALE_OPTIONS,
} from '../../store/AppSettings'
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath, openUrl } from '@tauri-apps/plugin-opener';
import { mkdir } from '@tauri-apps/plugin-fs';
import { getVersion } from '@tauri-apps/api/app';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ref, onMounted, computed, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  ArrowRight,
  Brush,
  ChatDotRound,
  Coffee,
  Delete,
  Document,
  Edit,
  FolderOpened,
  Key,
  Link,
  Lock,
  Monitor,
  Picture,
  Plus,
  Refresh,
  FullScreen,
  Star,
  StarFilled,
  View,
} from '@element-plus/icons-vue';
import type { OptionalPageId } from '../../store/AppSettings';
import {
  checkAndInstallAppUpdate,
  isCheckingAppUpdate,
  isInstallingAppUpdate,
} from '../../common/AppSelfUpdate';

const appSettings = AppStateManager.appSettings;
const textureMarkStyleOptions = ['Hash', 'Slot', 'SharedSlot'] as const;
const { t } = useI18n();

const languageOptions = SSMT_LOCALE_OPTIONS;

const appVersion = ref('1.0.0');
const workspaceAccessProxyPortInput = ref('');
const workspaceAccessProxyPortInvalid = computed(() => {
  const value = workspaceAccessProxyPortInput.value.trim();
  if (!value) return false;
  if (!/^\d+$/u.test(value)) return true;
  const port = Number(value);
  return !Number.isInteger(port) || port < 1 || port > 65535;
});
const applyWorkspaceAccessProxyPort = (value: string): void => {
  workspaceAccessProxyPortInput.value = value;
  const normalized = value.trim();
  if (!normalized) {
    appSettings.workspaceAccessProxyPort = 0;
  } else if (!workspaceAccessProxyPortInvalid.value) {
    appSettings.workspaceAccessProxyPort = Number(normalized);
  }
};
watch(() => appSettings.workspaceAccessProxyPort, (port) => {
  const normalized = port > 0 ? String(port) : '';
  if (!workspaceAccessProxyPortInvalid.value && workspaceAccessProxyPortInput.value !== normalized) {
    workspaceAccessProxyPortInput.value = normalized;
  }
}, { immediate: true });

const formatMaskOpacity = (value: number) => `${value.toFixed(1)}x`;
const uiScalePercent = computed({
  get: () => Math.round(appSettings.uiScale * 100),
  set: (value: number) => {
    appSettings.uiScale = value / 100;
  },
});
const formatUiScale = (value: number) => `${value}%`;
const pageVisibilityGroups = computed(() => [
  { title: t('settings.pageVisibility.modCreation'), pages: [
    { id: 'work' as OptionalPageId, label: t('titlebar.nav.work') },
    { id: 'markTexture' as OptionalPageId, label: t('titlebar.nav.markTexture') },
    { id: 'textureModMaker' as OptionalPageId, label: t('titlebar.nav.textureModMaker') },
    { id: 'uiBuilder' as OptionalPageId, label: t('titlebar.nav.uiBuilder') },
  ] },
  { title: t('settings.pageVisibility.modManagement'), pages: [
    { id: 'mods' as OptionalPageId, label: t('titlebar.nav.mods') },
  ] },
  { title: t('settings.pageVisibility.modAcquisition'), pages: [
    { id: 'gameBanana' as OptionalPageId, label: t('titlebar.nav.gameBanana') },
    { id: 'nexusMods' as OptionalPageId, label: t('titlebar.nav.nexusMods') },
  ] },
  { title: t('settings.pageVisibility.aiAssistant'), pages: [
    { id: 'xianzun' as OptionalPageId, label: t('titlebar.nav.xianzun') },
  ] },
]);

const selectCacheDir = async () => {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    title: t('settings.general.selectCacheFolderTitle')
  });

  if (selected && typeof selected === 'string') {
    appSettings.DBMTWorkFolder = selected;
  }
};

const openCacheDir = async () => {
  const cacheDir = (appSettings.DBMTWorkFolder || '').trim();
  if (!cacheDir) {
    ElMessage.warning(t('settings.messages.openCacheFolderFailed'));
    return;
  }
  try {
    // 目录可能被用户手动删除，打开前先确保它存在。
    await mkdir(cacheDir, { recursive: true });
    await openPath(cacheDir);
  } catch (error) {
    console.error('Failed to open cache folder:', error);
    ElMessage.error(t('settings.messages.openCacheFolderFailed'));
  }
};

onMounted(async () => {
  try {
    appVersion.value = await getVersion();
  } catch (e) {
    console.error('Failed to get version:', e);
  }
});

const openReleasePage = async () => {
  await openUrl('https://github.com/Perxenic-Acid/SSMT4/releases');
};

const handleCheckAndInstallAppUpdate = async () => {
  await checkAndInstallAppUpdate('manual');
};

const openUsageDocs = async () => {
  await openUrl('https://perxenic-acid.github.io/SSMT4-Documents/');
};

/* ═══════════════════════ Games management ═══════════════════════ */

const gamesList = AppStateManager.gamesList;
const gamePresetOptions = computed(() => getGamePresetOptions(t));
const gameIconPickerFilters = [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg', 'bmp', 'ico', 'avif'] }];

const toggleGameFavorite = async (game: GameInfo) => {
  try {
    await ResourceManager.setGameVisibility(game.name, !game.showSidebar);
    await AppStateManager.loadGames();
  } catch (error) {
    console.error('Failed to toggle favorite game:', error);
    ElMessage.error(t('gameLibrary.messages.addToFavoritesFailed'));
  }
};

const changeGameIcon = async (game: GameInfo) => {
  try {
    const file = await openDialog({ multiple: false, filters: gameIconPickerFilters });
    if (typeof file === 'string') {
      await ResourceManager.setGameIcon(game.name, file);
      await AppStateManager.loadGames();
    }
  } catch (error) {
    console.error('Set icon failed:', error);
    ElMessage.error(t('gameLibrary.messages.setIconFailed'));
  }
};

const deleteGameConfig = async (game: GameInfo) => {
  try {
    await ElMessageBox.confirm(
      t('gameLibrary.messages.deleteConfirmContent', { gameName: game.name }),
      t('gameLibrary.messages.deleteConfirmTitle'),
      {
        confirmButtonText: t('gameLibrary.dialog.delete'),
        cancelButtonText: t('gameLibrary.dialog.cancel'),
        type: 'warning',
      }
    );
  } catch {
    return;
  }

  try {
    await ResourceManager.deleteGameConfigFolder(game.name);
    await AppStateManager.loadGames();

    // If the deleted game was active, switch to the first available one
    if (appSettings.CurrentGameName === game.name && gamesList.length > 0) {
      await AppStateManager.selectGame(gamesList[0]);
    }
  } catch (error) {
    console.error('Failed to delete game config:', error);
    ElMessage.error(t('gameLibrary.messages.deleteConfigFailed'));
  }
};

/* ─────── Create game dialog ─────── */

const showCreateGameDialog = ref(false);
const newGameName = ref('');
const newGamePreset = ref('');
const newGameIconPath = ref('');
const newGameIconPreview = ref('');

const openCreateGameDialog = () => {
  newGameName.value = '';
  newGamePreset.value = gamePresetOptions.value[0]?.value || '';
  newGameIconPath.value = '';
  newGameIconPreview.value = '';
  showCreateGameDialog.value = true;
};

const pickNewGameIcon = async () => {
  try {
    const file = await openDialog({ multiple: false, filters: gameIconPickerFilters });
    if (typeof file === 'string') {
      newGameIconPath.value = file;
      newGameIconPreview.value = convertFileSrc(file);
    }
  } catch (error) {
    console.error('Pick icon failed:', error);
  }
};

const confirmCreateGame = async () => {
  const gameName = newGameName.value.trim();
  if (!gameName) {
    ElMessage.warning(t('gameLibrary.messages.enterConfigName'));
    return;
  }

  try {
    await ResourceManager.createNewConfig(gameName, {
      gamePreset: newGamePreset.value,
      backgroundType: 'Image',
    });

    if (newGameIconPath.value) {
      await ResourceManager.setGameIcon(gameName, newGameIconPath.value);
    }

    await AppStateManager.loadGames();
    const created = gamesList.find(g => g.name === gameName);
    if (created) {
      await AppStateManager.selectGame(created);
    }
    showCreateGameDialog.value = false;
  } catch (error) {
    console.error('Create config failed:', error);
    ElMessage.error(t('gameLibrary.messages.createConfigFailed'));
  }
};
</script>

<template>
  <div class="settings-page">
    <div class="settings-shell">
      <div class="settings-layout">
        <main class="settings-main" :aria-label="t('settings.title')">
          <section class="settings-section">
            <div class="section-heading">
              <h2>{{ t('settings.sections.general') }}</h2>
              <p>{{ t('settings.sections.generalDesc') }}</p>
            </div>
            <div class="settings-group">
              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Monitor /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.personalization.language') }}</div>
                    <div class="setting-description">{{ t('settings.hints.language') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-select
                    v-model="appSettings.locale"
                    :aria-label="t('settings.personalization.language')"
                  >
                    <el-option
                      v-for="item in languageOptions"
                      :key="item.value"
                      :label="item.label"
                      :value="item.value"
                    />
                  </el-select>
                </div>
              </div>

              <div class="setting-row setting-row-wide">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><FolderOpened /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.cacheFolder') }}</div>
                    <div class="setting-description">{{ t('settings.hints.cacheFolder') }}</div>
                  </div>
                </div>
                <div class="setting-control path-control">
                  <el-input
                    v-model="appSettings.DBMTWorkFolder"
                    :aria-label="t('settings.general.cacheFolder')"
                    :placeholder="t('settings.general.cacheFolderPlaceholder')"
                    readonly
                  />
                  <el-tooltip :content="t('settings.general.changeFolder')" placement="top" :show-after="250">
                    <el-button
                      class="path-icon-btn"
                      :aria-label="t('settings.general.changeFolder')"
                      @click="selectCacheDir"
                    >
                      <el-icon><Edit /></el-icon>
                    </el-button>
                  </el-tooltip>
                  <el-tooltip :content="t('settings.general.openFolder')" placement="top" :show-after="250">
                    <el-button
                      class="path-icon-btn"
                      :aria-label="t('settings.general.openFolder')"
                      @click="openCacheDir"
                    >
                      <el-icon><FolderOpened /></el-icon>
                    </el-button>
                  </el-tooltip>
                </div>
              </div>

              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Lock /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.githubToken') }}</div>
                    <div class="setting-description">{{ t('settings.hints.githubToken') }}</div>
                  </div>
                </div>
                <div class="setting-control">
                  <el-input
                    v-model="appSettings.githubToken"
                    :aria-label="t('settings.general.githubToken')"
                    :placeholder="t('settings.general.githubTokenPlaceholder')"
                    type="password"
                    show-password
                  />
                </div>
              </div>

              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Link /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.workspaceAccessProxyPort') }}</div>
                    <div class="setting-description">{{ t('settings.hints.workspaceAccessProxyPort') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-input
                    :model-value="workspaceAccessProxyPortInput"
                    @update:model-value="applyWorkspaceAccessProxyPort"
                    :aria-label="t('settings.general.workspaceAccessProxyPort')"
                    inputmode="numeric"
                    :class="{ 'workspace-access-proxy-input--invalid': workspaceAccessProxyPortInvalid }"
                    :placeholder="t('settings.general.workspaceAccessProxyPortPlaceholder')"
                  />
                </div>
              </div>

              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Key /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.showWindowShortcut') }}</div>
                    <div class="setting-description">{{ t('settings.hints.showWindowShortcut') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-switch
                    v-model="appSettings.showWindowShortcutEnabled"
                    :aria-label="t('settings.general.showWindowShortcut')"
                  />
                </div>
              </div>

              <div class="setting-row setting-row-wide">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Monitor /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.gameLaunchMode') }}</div>
                    <div class="setting-description">{{ t('settings.hints.gameLaunchMode') }}</div>
                  </div>
                </div>
                <div class="setting-control">
                  <el-select v-model="appSettings.gameLaunchMode" :aria-label="t('settings.general.gameLaunchMode')">
                    <el-option value="always-pure" :label="t('settings.general.gameLaunchModeAlwaysPure')" />
                    <el-option value="ctrl-pure" :label="t('settings.general.gameLaunchModeCtrlPure')" />
                    <el-option value="always-normal" :label="t('settings.general.gameLaunchModeAlwaysNormal')" />
                  </el-select>
                </div>
              </div>

              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Document /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.general.modelExtractionLogLanguage') }}</div>
                    <div class="setting-description">{{ t('settings.hints.modelExtractionLogLanguage') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-select v-model="appSettings.modelExtractionLogLanguage" :aria-label="t('settings.general.modelExtractionLogLanguage')">
                    <el-option value="zh-CN" :label="t('settings.general.modelExtractionLogChinese')" />
                    <el-option value="en" :label="t('settings.general.modelExtractionLogEnglish')" />
                  </el-select>
                </div>
              </div>
            </div>
          </section>

          <section class="settings-section">
            <div class="section-heading">
              <h2>{{ t('settings.sections.appearance') }}</h2>
              <p>{{ t('settings.sections.appearanceDesc') }}</p>
            </div>
            <div class="settings-group">
              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><Brush /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.personalization.textureMarkStyle') }}</div>
                    <div class="setting-description">{{ t('settings.hints.textureMarkStyle') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-select
                    v-model="appSettings.textureMarkStylePreference"
                    :aria-label="t('settings.personalization.textureMarkStyle')"
                  >
                    <el-option
                      v-for="item in textureMarkStyleOptions"
                      :key="item"
                      :label="item"
                      :value="item"
                    />
                  </el-select>
                </div>
              </div>

              <div class="setting-row setting-row-slider">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><FullScreen /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.personalization.uiScale') }}</div>
                    <div class="setting-description">{{ t('settings.personalization.uiScaleHint') }}</div>
                  </div>
                </div>
                <div class="setting-control ui-scale-control">
                  <el-slider
                    v-model="uiScalePercent"
                    :aria-label="t('settings.personalization.uiScale')"
                    :min="APP_UI_SCALE_MIN * 100"
                    :max="APP_UI_SCALE_MAX * 100"
                    :step="APP_UI_SCALE_STEP * 100"
                    :format-tooltip="formatUiScale"
                  />
                  <output class="scale-value">{{ formatUiScale(uiScalePercent) }}</output>
                </div>
              </div>

              <div class="setting-row setting-row-slider">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><View /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.personalization.backgroundMaskOpacity') }}</div>
                    <div class="setting-description">{{ t('settings.personalization.backgroundMaskOpacityHint') }}</div>
                  </div>
                </div>
                <div class="setting-control mask-opacity-control">
                  <el-slider
                    v-model="appSettings.globalDimMaskStrength"
                    :aria-label="t('settings.personalization.backgroundMaskOpacity')"
                    :min="0"
                    :max="4"
                    :step="0.1"
                    :format-tooltip="formatMaskOpacity"
                    show-input
                    input-size="small"
                  />
                </div>
              </div>

              <div class="setting-row">
                <div class="setting-identity">
                  <span class="setting-icon"><el-icon><View /></el-icon></span>
                  <div>
                    <div class="setting-label">{{ t('settings.personalization.revealBlurredImagesOnHover') }}</div>
                    <div class="setting-description">{{ t('settings.personalization.revealBlurredImagesOnHoverHint') }}</div>
                  </div>
                </div>
                <div class="setting-control compact-control">
                  <el-switch
                    v-model="appSettings.revealBlurredImagesOnHover"
                    :aria-label="t('settings.personalization.revealBlurredImagesOnHover')"
                  />
                </div>
              </div>
            </div>
          </section>

          <section class="settings-section">
            <div class="section-heading">
              <h2>{{ t('settings.sections.pageVisibility') }}</h2>
              <p>{{ t('settings.sections.pageVisibilityDesc') }}</p>
            </div>
            <div class="settings-group page-visibility-container">
              <div v-for="group in pageVisibilityGroups" :key="group.title" class="page-visibility-group">
                <div class="page-visibility-group-title">{{ group.title }}</div>
                <div class="page-visibility-options">
                  <label v-for="page in group.pages" :key="page.id" class="page-visibility-option">
                    <span>{{ page.label }}</span>
                    <el-switch v-model="appSettings.pageVisibility[page.id]" :aria-label="page.label" />
                  </label>
                </div>
              </div>
            </div>
          </section>

          <section class="settings-section">
            <div class="section-heading games-heading">
              <div class="games-heading-text">
                <h2>{{ t('settings.sections.games') }}</h2>
                <p>{{ t('settings.sections.gamesDesc') }}</p>
              </div>
              <el-button class="add-game-btn" :aria-label="t('settings.games.addGame')" @click="openCreateGameDialog">
                <el-icon><Plus /></el-icon>
                <span>{{ t('settings.games.addGame') }}</span>
              </el-button>
            </div>
            <div v-if="gamesList.length > 0" class="settings-group games-panel">
              <div
                v-for="game in gamesList"
                :key="game.name"
                class="game-row"
                :class="{ current: appSettings.CurrentGameName === game.name }"
              >
                <span class="game-row-icon-wrap">
                  <img
                    v-if="game.iconPath"
                    class="game-row-icon"
                    :src="game.iconPath"
                    :alt="getGamePresetDisplayName(game.name, t)"
                    loading="lazy"
                    draggable="false"
                    @error="(e) => ((e.target as HTMLImageElement).style.opacity = '0')"
                  />
                </span>
                <span class="game-row-name" :title="getGamePresetDisplayName(game.name, t)">
                  {{ getGamePresetDisplayName(game.name, t) }}
                </span>
                <span v-if="appSettings.CurrentGameName === game.name" class="game-current-badge">
                  {{ t('settings.games.current') }}
                </span>
                <div class="game-row-actions">
                  <el-tooltip
                    :content="game.showSidebar ? t('settings.games.unfavorite') : t('settings.games.favorite')"
                    placement="top"
                    :show-after="250"
                  >
                    <button
                      type="button"
                      class="game-action-btn"
                      :class="{ active: game.showSidebar }"
                      :aria-label="game.showSidebar ? t('settings.games.unfavorite') : t('settings.games.favorite')"
                      :aria-pressed="game.showSidebar"
                      @click="toggleGameFavorite(game)"
                    >
                      <el-icon><StarFilled v-if="game.showSidebar" /><Star v-else /></el-icon>
                    </button>
                  </el-tooltip>
                  <el-tooltip :content="t('settings.games.changeIcon')" placement="top" :show-after="250">
                    <button
                      type="button"
                      class="game-action-btn"
                      :aria-label="t('settings.games.changeIcon')"
                      @click="changeGameIcon(game)"
                    >
                      <el-icon><Picture /></el-icon>
                    </button>
                  </el-tooltip>
                  <el-tooltip :content="t('settings.games.deleteConfig')" placement="top" :show-after="250">
                    <button
                      type="button"
                      class="game-action-btn danger"
                      :aria-label="t('settings.games.deleteConfig')"
                      @click="deleteGameConfig(game)"
                    >
                      <el-icon><Delete /></el-icon>
                    </button>
                  </el-tooltip>
                </div>
              </div>
            </div>
          </section>

        </main>

        <aside class="settings-sidebar" :aria-label="t('settings.sections.about')">
          <div class="about-panel">
            <div class="product-lockup">
              <img src="/icon.png" class="app-logo" alt="SSMT4" />
              <div>
                <h2>SSMT4</h2>
                <span class="app-version">V{{ appVersion }}</span>
              </div>
            </div>

            <el-button
              class="update-button"
              :loading="isCheckingAppUpdate || isInstallingAppUpdate"
              :aria-busy="isCheckingAppUpdate || isInstallingAppUpdate"
              @click="handleCheckAndInstallAppUpdate"
            >
              <el-icon><Refresh /></el-icon>
              <span>{{ isInstallingAppUpdate ? t('settings.actions.installingUpdate') : t('settings.actions.checkAppUpdate') }}</span>
            </el-button>

            <div class="about-section">
              <h3>{{ t('settings.sections.resources') }}</h3>
              <button type="button" class="link-row" @click="openReleasePage">
                <el-icon><Document /></el-icon>
                <span>{{ t('settings.about.releasePage') }}</span>
                <el-icon class="link-arrow"><ArrowRight /></el-icon>
              </button>
              <button type="button" class="link-row" @click="openUsageDocs">
                <el-icon><Link /></el-icon>
                <span>{{ t('settings.about.usageDocs') }}</span>
                <el-icon class="link-arrow"><ArrowRight /></el-icon>
              </button>
            </div>

            <div class="about-section">
              <h3>{{ t('settings.sections.community') }}</h3>
              <button type="button" class="link-row" @click="openUrl('https://discord.gg/hgzbSSXXz2')">
                <el-icon><ChatDotRound /></el-icon>
                <span>{{ t('settings.about.discord') }}</span>
                <el-icon class="link-arrow"><ArrowRight /></el-icon>
              </button>
            </div>

            <div class="about-section">
              <h3>{{ t('settings.about.sponsor') }}</h3>
              <button type="button" class="link-row" @click="openUrl('https://ifdian.net/a/NicoMico666')">
                <el-icon><Coffee /></el-icon>
                <span>{{ t('settings.about.afdian') }}</span>
                <el-icon class="link-arrow"><ArrowRight /></el-icon>
              </button>
            </div>
          </div>
        </aside>
      </div>
    </div>

    <!-- Create game config dialog -->
    <el-dialog
      v-model="showCreateGameDialog"
      class="settings-game-dialog"
      :title="t('gameLibrary.dialog.createTitle')"
      width="min(440px, calc(100vw - 32px))"
    >
      <div class="create-game-body">
        <div class="create-game-icon-preview">
          <img v-if="newGameIconPreview" :src="newGameIconPreview" :alt="newGameName || t('gameLibrary.dialog.noIcon')" />
          <div v-else class="create-game-icon-placeholder">{{ t('gameLibrary.dialog.noIcon') }}</div>
          <el-button size="small" @click="pickNewGameIcon">{{ t('gameLibrary.actions.chooseIcon') }}</el-button>
        </div>

        <div class="create-game-fields">
          <div class="create-game-row">
            <label class="create-game-label" for="settings-new-game-name">{{ t('gameLibrary.dialog.configName') }}</label>
            <el-input id="settings-new-game-name" v-model="newGameName" :placeholder="t('gameLibrary.dialog.enterConfigName')" />
          </div>
          <div class="create-game-row">
            <label class="create-game-label" for="settings-new-game-preset">{{ t('gameLibrary.dialog.gamePreset') }}</label>
            <el-select
              id="settings-new-game-preset"
              class="game-preset-select"
              v-model="newGamePreset"
              popper-class="settings-game-select-popper"
              :placeholder="t('gameLibrary.dialog.selectPreset')"
              style="width: 100%"
            >
              <el-option
                v-for="opt in gamePresetOptions"
                :key="opt.value"
                :label="opt.label"
                :value="opt.value"
              />
            </el-select>
          </div>
        </div>
      </div>

      <template #footer>
        <span class="create-game-footer">
          <el-button @click="showCreateGameDialog = false">{{ t('gameLibrary.dialog.cancel') }}</el-button>
          <el-button type="primary" @click="confirmCreateGame">{{ t('gameLibrary.dialog.confirm') }}</el-button>
        </span>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.settings-page {
  width: 100%;
  min-height: 100%;
  box-sizing: border-box;
  padding: 28px 32px 40px;
  color: rgba(var(--theme-text-primary-rgb), 0.96);
}

.settings-shell {
  width: min(1180px, 100%);
  /* Left-aligned so the page stays visually anchored to the navigation pane
     instead of drifting away from it on wide windows */
  margin: 0;
}

.setting-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  color: rgba(var(--theme-text-primary-rgb), 0.94);
  background: rgba(var(--theme-surface-tint-rgb), 0.11);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
}

.product-lockup h2,
.section-heading h2,
.about-section h3 {
  margin: 0;
  letter-spacing: 0;
}

.section-heading p {
  margin: 4px 0 0;
  color: rgba(var(--theme-text-secondary-rgb), 0.78);
  font-size: 13px;
  line-height: 1.45;
}

.settings-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 276px;
  gap: 28px;
  align-items: start;
}

.settings-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 26px;
}

.settings-section {
  min-width: 0;
}

.section-heading {
  margin: 0 4px 10px;
}

.section-heading h2 {
  font-size: 14px;
  line-height: 1.3;
  font-weight: 650;
}

.settings-group,
.about-panel {
  position: relative;
  overflow: hidden;
  border: var(--t-material-border);
  border-radius: 14px;
  background: var(--t-material-bg);
  box-shadow: var(--t-material-shadow);
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
}

.settings-group::before,
.about-panel::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  z-index: 1;
  border-radius: 14px 14px 0 0;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.08), transparent);
  pointer-events: none;
}

.setting-row {
  min-height: 78px;
  display: grid;
  grid-template-columns: minmax(220px, 1fr) minmax(240px, 360px);
  gap: 24px;
  align-items: center;
  padding: 14px 18px;
  box-sizing: border-box;
}

.setting-row + .setting-row {
  border-top: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
}

.setting-row-wide {
  grid-template-columns: minmax(220px, 0.8fr) minmax(300px, 1.2fr);
}

.setting-identity {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.setting-icon {
  width: 30px;
  height: 30px;
  margin-top: 1px;
  border-radius: 7px;
  font-size: 16px;
}

.setting-label {
  color: rgba(var(--theme-text-primary-rgb), 0.96);
  font-size: 13px;
  line-height: 1.4;
  font-weight: 600;
}

.setting-description {
  max-width: 520px;
  margin-top: 3px;
  color: rgba(var(--theme-text-secondary-rgb), 0.72);
  font-size: 12px;
  line-height: 1.45;
}

.setting-control {
  min-width: 0;
  width: 100%;
}

.compact-control {
  width: min(100%, 220px);
  justify-self: end;
}

.page-visibility-container {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.page-visibility-group {
  min-width: 0;
  padding: 16px 18px;
}

.page-visibility-group:nth-child(odd) {
  border-right: 1px solid rgba(255, 255, 255, 0.08);
}

.page-visibility-group:nth-child(n + 3) {
  border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.page-visibility-group-title {
  margin-bottom: 10px;
  color: rgba(255, 255, 255, 0.56);
  font-size: 12px;
  font-weight: 650;
}

.page-visibility-options {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.page-visibility-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 34px;
  color: rgba(255, 255, 255, 0.88);
  font-size: 13px;
  cursor: pointer;
}

.workspace-access-proxy-input--invalid :deep(.el-input__wrapper) {
  background: rgba(230, 162, 60, 0.16);
  box-shadow: 0 0 0 1px rgba(230, 162, 60, 0.72) inset;
}

.workspace-access-proxy-input--invalid :deep(.el-input__inner) {
  color: rgba(255, 224, 145, 1);
}

.path-control {
  display: flex;
  align-items: center;
  gap: 10px;
}

.path-control :deep(.el-input) {
  min-width: 0;
}

.path-control :deep(.el-button) {
  flex: 0 0 auto;
}

/* The flex gap already spaces the controls, so drop Element Plus' default
   sibling margin; otherwise the second button sits 12px further away. */
.path-control :deep(.el-button + .el-button) {
  margin-left: 0;
}

/* Icon-only buttons in the cache folder row: square, same height as the input.
   The .settings-page prefix outranks the generic .settings-page .el-button
   padding rule below (equal specificity previously let that rule win and
   crushed the button content box to 6px, clipping the icon). */
.settings-page .path-control :deep(.path-icon-btn) {
  width: 34px;
  height: 34px;
  min-width: 34px;
  padding: 0;
  font-size: 15px;
}

.action-control {
  display: flex;
  justify-content: flex-end;
}

.settings-page :deep(.el-input__wrapper),
.settings-page :deep(.el-select__wrapper) {
  min-height: 34px;
  border-radius: 7px;
  color: rgba(var(--theme-text-primary-rgb), 0.96);
  background: rgba(var(--theme-surface-tint-rgb), 0.065) !important;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.15);
  box-shadow: none !important;
  transition: background-color 160ms ease, border-color 160ms ease, box-shadow 160ms ease;
}

.settings-page :deep(.el-input__wrapper:hover),
.settings-page :deep(.el-select__wrapper:hover) {
  background: rgba(var(--theme-surface-tint-rgb), 0.09) !important;
  border-color: rgba(var(--theme-surface-tint-rgb), 0.26);
}

.settings-page :deep(.el-input__wrapper.is-focus),
.settings-page :deep(.el-select__wrapper.is-focused) {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.62);
  box-shadow: 0 0 0 3px rgba(var(--theme-surface-tint-rgb), 0.10) !important;
}

.settings-page :deep(.el-input__inner),
.settings-page :deep(.el-select__selected-item),
.settings-page :deep(.el-select__placeholder) {
  color: rgba(var(--theme-text-primary-rgb), 0.94);
  font-size: 13px;
}

.settings-page :deep(.el-input__inner::placeholder) {
  color: rgba(var(--theme-text-secondary-rgb), 0.62);
}

.settings-page :deep(.el-button) {
  min-height: 34px;
  border-radius: 7px;
  padding: 7px 13px;
  font-weight: 600;
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  background: rgba(var(--theme-surface-tint-rgb), 0.09);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.18);
  transition: background-color 160ms ease, border-color 160ms ease;
}

.settings-page :deep(.el-button:hover),
.settings-page :deep(.el-button:focus-visible) {
  color: rgba(var(--theme-text-primary-rgb), 1);
  background: rgba(var(--theme-surface-tint-rgb), 0.15);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.34);
}

.mask-opacity-control :deep(.el-slider) {
  --el-slider-main-bg-color: rgba(var(--theme-text-primary-rgb), 0.88);
  --el-slider-runway-bg-color: rgba(var(--theme-surface-tint-rgb), 0.16);
  --el-slider-button-size: 16px;
}

.settings-page :deep(.el-button::before) {
  display: none;
}

.ui-scale-control {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 48px;
  align-items: center;
  gap: 12px;
}

.ui-scale-control :deep(.el-slider) {
  --el-slider-main-bg-color: rgba(var(--theme-text-primary-rgb), 0.88);
  --el-slider-runway-bg-color: rgba(var(--theme-surface-tint-rgb), 0.16);
  --el-slider-button-size: 16px;
}

.scale-value {
  color: rgba(var(--theme-text-primary-rgb), 0.9);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.mask-opacity-control :deep(.el-slider__input),
.mask-opacity-control :deep(.el-input-number) {
  width: 156px;
}

.mask-opacity-control :deep(.el-input-number__decrease),
.mask-opacity-control :deep(.el-input-number__increase) {
  color: rgba(var(--theme-text-primary-rgb), 0.84);
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
}

.settings-page {
  padding: var(--t-page-padding);
  color: var(--t-page-text);
}

.settings-group,
.about-panel {
  border: var(--t-page-panel-border);
  border-radius: var(--t-page-panel-radius);
  background: var(--t-page-panel-bg);
  box-shadow: var(--t-page-panel-shadow);
  backdrop-filter: var(--t-page-panel-blur);
  -webkit-backdrop-filter: var(--t-page-panel-blur);
}

.settings-sidebar {
  position: sticky;
  top: 0;
  min-width: 0;
}

.about-panel {
  padding: 18px;
}

.product-lockup {
  display: flex;
  align-items: center;
  gap: 12px;
}

.app-logo {
  width: 48px;
  height: 48px;
  border-radius: 8px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.3);
}

.product-lockup h2 {
  font-size: 18px;
  line-height: 1.25;
  font-weight: 700;
}

.app-version {
  display: block;
  margin-top: 2px;
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 12px;
  color: rgba(var(--theme-text-secondary-rgb), 0.72);
}

.update-button {
  width: 100%;
  margin-top: 18px;
}

.about-section {
  padding-top: 18px;
  margin-top: 18px;
  border-top: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
}

.about-section h3 {
  padding: 0 8px 7px;
  color: rgba(var(--theme-text-secondary-rgb), 0.78);
  font-size: 12px;
  line-height: 1.3;
  font-weight: 650;
}

.link-row {
  width: 100%;
  min-height: 36px;
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) 16px;
  align-items: center;
  gap: 9px;
  padding: 7px 8px;
  border: 0;
  border-radius: 7px;
  color: rgba(var(--theme-text-primary-rgb), 0.88);
  background: transparent;
  font: inherit;
  font-size: 13px;
  line-height: 1.35;
  text-align: left;
  cursor: pointer;
  transition: color 160ms ease, background-color 160ms ease;
}

.link-row span {
  min-width: 0;
  overflow-wrap: anywhere;
}

.link-row:hover,
.link-row:focus-visible {
  color: rgba(var(--theme-text-primary-rgb), 1);
  background: rgba(var(--theme-surface-tint-rgb), 0.10);
}

.link-row:focus-visible {
  outline: 2px solid rgba(var(--theme-text-primary-rgb), 0.72);
  outline-offset: 1px;
}

.link-arrow {
  color: rgba(var(--theme-text-secondary-rgb), 0.62);
}

/* ═══════════ Games management section ═══════════ */
.games-heading {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.games-heading-text {
  min-width: 0;
}

.add-game-btn {
  flex: 0 0 auto;
  margin-bottom: 2px;
}

.games-panel {
  display: flex;
  flex-direction: column;
  padding: 6px;
}

.game-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 52px;
  padding: 6px 10px;
  border-radius: 9px;
  box-sizing: border-box;
  transition: background-color 160ms ease;
}

.game-row + .game-row {
  border-top: 1px solid rgba(var(--theme-surface-tint-rgb), 0.08);
}

.game-row:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
}

.game-row.current {
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
}

.game-row-icon-wrap {
  flex: 0 0 auto;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  overflow: hidden;
  background: rgba(var(--theme-surface-tint-rgb), 0.10);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
}

.game-row-icon {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.game-row-name {
  min-width: 0;
  flex: 1 1 auto;
  color: rgba(var(--theme-text-primary-rgb), 0.94);
  font-size: 13px;
  font-weight: 600;
  line-height: 1.35;
  overflow-wrap: anywhere;
}

.game-current-badge {
  flex: 0 0 auto;
  padding: 2px 8px;
  border-radius: 999px;
  background: rgba(var(--theme-surface-tint-rgb), 0.16);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.26);
  color: rgba(var(--theme-text-primary-rgb), 0.9);
  font-size: 11px;
  font-weight: 650;
  line-height: 1.5;
}

.game-row-actions {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 6px;
}

.game-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  padding: 0;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  border-radius: 7px;
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
  color: rgba(var(--theme-text-primary-rgb), 0.78);
  font-size: 15px;
  cursor: pointer;
  transition: background-color 160ms ease, border-color 160ms ease, color 160ms ease;
}

.game-action-btn:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.14);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.30);
  color: rgba(var(--theme-text-primary-rgb), 1);
}

.game-action-btn.active {
  color: #ffd666;
  background: rgba(255, 214, 102, 0.12);
  border-color: rgba(255, 214, 102, 0.38);
}

.game-action-btn.danger:hover {
  color: #ff8a8a;
  background: rgba(255, 100, 100, 0.12);
  border-color: rgba(255, 100, 100, 0.42);
}

/* ═══════════ Create game dialog ═══════════ */
.create-game-body {
  display: flex;
  gap: 18px;
  align-items: flex-start;
}

.create-game-icon-preview {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.create-game-icon-preview img,
.create-game-icon-placeholder {
  width: 84px;
  height: 84px;
  border-radius: 14px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.18);
  object-fit: cover;
}

.create-game-icon-placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(var(--theme-surface-tint-rgb), 0.07);
  color: rgba(var(--theme-text-secondary-rgb), 0.66);
  font-size: 12px;
}

.create-game-fields {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.create-game-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.create-game-label {
  color: rgba(var(--theme-text-secondary-rgb), 0.82);
  font-size: 12px;
  font-weight: 600;
}

.create-game-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

:global(.settings-game-dialog) {
  background: var(--t-page-panel-bg);
  border: var(--t-page-panel-border);
  box-shadow: var(--t-page-panel-shadow);
  backdrop-filter: var(--t-page-panel-blur);
}

:global(.settings-game-select-popper.el-popper) {
  background: transparent;
}

/* 游戏预设名较长：选中值进入正常文档流并换行，输入框随内容自动变高，完整显示预设名 */
.game-preset-select :deep(.el-select__placeholder) {
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

.game-preset-select :deep(.el-select__wrapper) {
  height: auto;
}

@media (max-width: 960px) {
  .settings-page {
    padding: 24px 22px 36px;
  }

  .settings-layout {
    grid-template-columns: minmax(0, 1fr);
  }

  .settings-sidebar {
    position: static;
  }

  .about-panel {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 22px;
  }

  .product-lockup,
  .update-button {
    grid-column: 1 / -1;
  }
}

@media (max-width: 760px) {
  .page-visibility-container {
    grid-template-columns: 1fr;
  }

  .page-visibility-group:nth-child(odd) {
    border-right: 0;
  }

  .page-visibility-group + .page-visibility-group {
    border-top: 1px solid rgba(255, 255, 255, 0.08);
  }

  .settings-page {
    padding: 20px 14px 28px;
  }

  .setting-row,
  .setting-row-wide {
    grid-template-columns: minmax(0, 1fr);
    gap: 12px;
    padding: 16px;
  }

  .compact-control {
    width: 100%;
    justify-self: stretch;
  }

  .action-control {
    justify-content: stretch;
  }

  .action-control :deep(.el-button) {
    width: 100%;
  }

  .about-panel {
    display: block;
  }
}

@media (max-width: 480px) {
  .path-control {
    align-items: stretch;
    flex-direction: column;
  }

  .path-control :deep(.el-button) {
    width: 100%;
  }

  .setting-row-slider .mask-opacity-control :deep(.el-slider) {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 96px;
    gap: 12px;
  }

  .mask-opacity-control :deep(.el-slider__input),
  .mask-opacity-control :deep(.el-input-number) {
    width: 96px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .settings-page *,
  .settings-page *::before,
  .settings-page *::after {
    scroll-behavior: auto !important;
    transition-duration: 0.01ms !important;
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
  }
}
</style>
