<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { AppStateManager, BGType } from "./store/AppStateManager";
import { normalizeAppUiScale, SSMT_LOCALE_OPTIONS, type PageVisibilitySettings } from "./store/AppSettings";
import TitleBar from "./components/TitleBar.vue";
import { useI18n } from 'vue-i18n';
import { GlobalConfig } from './store/GlobalConfig';
import { ResourceManager } from './store/ResourceManager';
import type { D3d11Mode, HuntingMode } from './store/GameConfig';
import CacheFolderPicker from './components/CacheFolderPicker.vue';
import { getGamePresetDisplayName } from './store/GamePreset';

const route = useRoute();
const router = useRouter();
const appSettings = AppStateManager.appSettings;
const gameSwitchRevision = AppStateManager.gameSwitchRevision;
const { t } = useI18n();
const selectedFirstRunRole = ref<'author' | 'player' | 'both' | null>(null);
const firstRunStep = ref(0);
type FirstRunStepKey = 'language' | 'role' | 'general' | 'authorPreferences' | 'games' | 'd3d11' | 'migoto' | 'background' | 'nsfw';
const firstRunSteps = computed<FirstRunStepKey[]>(() => {
  const role = selectedFirstRunRole.value;
  return [
    'language', 'role', 'general',
    ...(role !== 'player' ? ['authorPreferences' as const] : []),
    'games', 'd3d11', 'migoto', 'background',
    ...(role !== 'author' ? ['nsfw' as const] : []),
  ];
});
const currentFirstRunStep = computed(() => firstRunSteps.value[firstRunStep.value] || 'language');
const firstRunStepCount = computed(() => firstRunSteps.value.length);
const firstRunTransitionDirection = ref<'forward' | 'backward'>('forward');
const firstRunTransitionName = computed(() => `first-run-${firstRunTransitionDirection.value}`);
const firstRunDialog = ref<HTMLElement | null>(null);
watch(firstRunStep, () => firstRunDialog.value?.scrollTo({ top: 0 }));
const selectedFirstRunGames = ref<string[]>([]);
const firstRunGamesInitialized = ref(false);
const firstRunD3d11Mode = ref<D3d11Mode>('dev');
const firstRunUseShell = ref(false);
const firstRunHuntingMode = ref<HuntingMode>('2');
const firstRunShowWarnings = ref(true);
const firstRunCheckDllUpdate = ref(true);
const firstRunCheckPackageUpdate = ref(true);
const firstRunBackgroundType = ref<'Image' | 'Video'>('Video');
const firstRunBackgroundUpdateMode = ref<'manual' | 'auto'>('auto');
// 缓存目录：默认位置（安装目录）与自定义位置必须二选一，未选择前无法继续。
const firstRunDefaultCacheDir = ref('');
const firstRunCustomCacheDir = ref('');
const firstRunCacheMode = ref<'' | 'default' | 'custom'>('');
const firstRunCacheResolved = computed(() => {
  const path = firstRunCacheMode.value === 'default'
    ? firstRunDefaultCacheDir.value
    : firstRunCacheMode.value === 'custom' ? firstRunCustomCacheDir.value : '';
  return path.trim();
});
const firstRunNextDisabled = computed(() => {
  if (currentFirstRunStep.value === 'role') return !selectedFirstRunRole.value;
  if (currentFirstRunStep.value === 'general') return !firstRunCacheResolved.value;
  return false;
});
// 未设置缓存目录时禁止使用程序（引导页之外也会强制弹窗）。
const cacheRequiredBlocked = computed(() => {
  if (AppStateManager.isFirstRunOnboardingOpen.value) return false;
  return !(appSettings.DBMTWorkFolder || '').trim();
});
const orderedFirstRunGames = computed(() => [...AppStateManager.gamesList].sort((a, b) => {
  const aSelected = selectedFirstRunGames.value.includes(a.name) ? 0 : 1;
  const bSelected = selectedFirstRunGames.value.includes(b.name) ? 0 : 1;
  return aSelected - bSelected || a.name.localeCompare(b.name);
}));
// 尽早解析“安装目录内默认缓存位置”，供引导页与强制弹窗使用。
void GlobalConfig.SSMT4DefaultCacheFolder().then((path) => {
  if (path.trim()) firstRunDefaultCacheDir.value = path.trim();
});
watch(() => AppStateManager.gamesList.map(game => game.name).join('\n'), async () => {
  if (firstRunGamesInitialized.value || AppStateManager.gamesList.length === 0) return;
  const defaults = new Set(['GIMI', 'SRMI', 'ZZMI', 'WWMI']);
  const selected: string[] = [];
  for (const game of AppStateManager.gamesList) {
    // Built-in presets are the onboarding defaults. Custom games are only
    // inherited when the user had already made them visible; sharing a
    // compatible extractor preset must not select them by itself.
    if (defaults.has(game.name.toUpperCase()) || game.showSidebar) selected.push(game.name);
  }
  selectedFirstRunGames.value = selected;
  firstRunGamesInitialized.value = true;
}, { immediate: true });
watch(() => AppStateManager.isFirstRunOnboardingOpen.value, async open => {
  if (open && !firstRunDefaultCacheDir.value.trim()) {
    firstRunDefaultCacheDir.value = (await GlobalConfig.SSMT4DefaultCacheFolder()).trim();
  }
}, { immediate: true });
const firstRunRoleVisibility: Record<'author' | 'player' | 'both', PageVisibilitySettings> = {
  author: { work: true, markTexture: true, textureModMaker: true, mods: false, gameBanana: false, nexusMods: false, xianzun: false, uiBuilder: true },
  player: { work: false, markTexture: false, textureModMaker: false, mods: true, gameBanana: true, nexusMods: true, xianzun: true, uiBuilder: false },
  both: { work: true, markTexture: true, textureModMaker: true, mods: true, gameBanana: true, nexusMods: true, xianzun: true, uiBuilder: true },
};
const firstRunPageLabels: Array<{ id: keyof PageVisibilitySettings; labelKey: string }> = [
  { id: 'work', labelKey: 'titlebar.nav.work' },
  { id: 'markTexture', labelKey: 'titlebar.nav.markTexture' },
  { id: 'textureModMaker', labelKey: 'titlebar.nav.textureModMaker' },
  { id: 'mods', labelKey: 'titlebar.nav.mods' },
  { id: 'gameBanana', labelKey: 'titlebar.nav.gameBanana' },
  { id: 'nexusMods', labelKey: 'titlebar.nav.nexusMods' },
  { id: 'xianzun', labelKey: 'titlebar.nav.xianzun' },
  { id: 'uiBuilder', labelKey: 'titlebar.nav.uiBuilder' },
];
const selectedFirstRunPages = computed(() => {
  if (!selectedFirstRunRole.value) return [t('titlebar.nav.home')];
  const visibility = firstRunRoleVisibility[selectedFirstRunRole.value];
  return [t('titlebar.nav.home'), ...firstRunPageLabels.filter(page => visibility[page.id]).map(page => t(page.labelKey))];
});
const confirmFirstRunRole = async () => {
  if (!selectedFirstRunRole.value || !firstRunCacheResolved.value) return;
  appSettings.DBMTWorkFolder = firstRunCacheResolved.value;
  localStorage.setItem('ssmt4:post-processing-preview:lighting-mode', appSettings.postProcessPreviewLightingMode);
  appSettings.sidebarGameOrder = [
    ...selectedFirstRunGames.value,
    ...AppStateManager.gamesList.map(game => game.name).filter(name => !selectedFirstRunGames.value.includes(name)),
  ];
  for (const game of AppStateManager.gamesList) {
    const selected = selectedFirstRunGames.value.includes(game.name);
    await ResourceManager.setGameVisibility(game.name, selected);
    if (!selected) continue;
    const config = await ResourceManager.loadGameConfig(game.name);
    config.d3d11Mode = selectedFirstRunRole.value === 'player' ? 'play' : firstRunD3d11Mode.value;
    config.useShell = firstRunUseShell.value;
    if (firstRunUseShell.value) {
      config.extraDll = '';
      config.extraDlls = [];
    }
    if (selectedFirstRunRole.value !== 'player') config.huntingMode = firstRunHuntingMode.value;
    config.showErrorPopup = firstRunShowWarnings.value;
    config.checkDllUpdateBeforeLaunch = firstRunCheckDllUpdate.value;
    config.check3DmigotoPackageUpdateBeforeLaunch = firstRunCheckPackageUpdate.value;
    config.backgroundType = firstRunBackgroundType.value;
    config.backgroundUpdateMode = firstRunBackgroundUpdateMode.value;
    await ResourceManager.saveGameConfig(game.name, config);
  }
  await AppStateManager.loadGames();
  await AppStateManager.completeFirstRunOnboarding(firstRunRoleVisibility[selectedFirstRunRole.value]);
};
const advanceFirstRun = () => {
  if (firstRunNextDisabled.value) return;
  if (firstRunStep.value < firstRunStepCount.value - 1) {
    firstRunTransitionDirection.value = 'forward';
    firstRunStep.value += 1;
  }
};
const confirmCacheRequiredGate = async () => {
  const cacheDir = firstRunCacheResolved.value;
  if (!cacheDir) return;
  appSettings.DBMTWorkFolder = cacheDir;
  // 立即落盘并创建目录，避免用户未等防抖保存就关闭程序。
  await AppStateManager.saveSettingsNow();
};
const toggleFirstRunGame = (name: string) => {
  selectedFirstRunGames.value = selectedFirstRunGames.value.includes(name)
    ? selectedFirstRunGames.value.filter(item => item !== name)
    : [name, ...selectedFirstRunGames.value];
};
const firstRunGameIconFallback = 'data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2230%22 height=%2230%22 viewBox=%220 0 30 30%22%3E%3Crect width=%2230%22 height=%2230%22 rx=%226%22 fill=%22black%22 fill-opacity=%22.45%22/%3E%3C/svg%3E';
const useFirstRunGameIconFallback = (event: Event) => {
  const image = event.currentTarget as HTMLImageElement;
  if (image.src !== firstRunGameIconFallback) image.src = firstRunGameIconFallback;
};
const retreatFirstRun = () => {
  if (firstRunStep.value > 0) {
    firstRunTransitionDirection.value = 'backward';
    firstRunStep.value -= 1;
  }
};
const shouldShowGlobalDimLayer = computed(() => (
  route.path === '/settings'
  || route.path === '/mods'
  || route.path === '/work'
  || route.path === '/mark-texture-full'
  || route.path === '/texture-mod-maker'
  || route.path === '/xianzun'
  || route.path === '/ui-builder'
  || route.path.startsWith('/gamebanana')
  || route.path.startsWith('/nexusmods')
));

const clamp = (value: number, min: number, max: number) => {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
};

const globalDimLayerBackground = computed(() => {
  const strength = clamp(Number(appSettings.globalDimMaskStrength ?? 1), 0, 4);
  const center = clamp(0.12 * strength, 0, 0.48).toFixed(3);
  const middle = clamp(0.24 * strength, 0, 0.86).toFixed(3);
  const edge = clamp(0.34 * strength, 0, 0.96).toFixed(3);

  return `radial-gradient(circle at 50% 50%, rgba(0, 0, 0, ${center}) 0%, rgba(0, 0, 0, ${middle}) 52%, rgba(0, 0, 0, ${edge}) 100%)`;
});

const appUiScale = computed(() => normalizeAppUiScale(appSettings.uiScale));

const normalizeStoredAppUiScale = (value: number) => {
  const normalizedScale = normalizeAppUiScale(value);
  if (appSettings.uiScale !== normalizedScale) {
    appSettings.uiScale = normalizedScale;
  }
};

watch(() => appSettings.uiScale, normalizeStoredAppUiScale, { immediate: true });

/* ─── Route-change progress feedback ───
   Shown the instant a navigation starts and hidden two frames after it
   resolves, so even heavy first-time page mounts give immediate feedback
   instead of looking frozen. */
const isNavigating = ref(false);
let navigationToken = 0;
const showRouteProgress = () => {
  navigationToken += 1;
  isNavigating.value = true;
};
const hideRouteProgressAfterPaint = () => {
  const token = navigationToken;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (token === navigationToken) isNavigating.value = false;
  }));
};
router.beforeEach((to, from) => {
  if (to.fullPath !== from.fullPath) showRouteProgress();
});
router.afterEach(hideRouteProgressAfterPaint);
// Switching the active game remounts the current page (see the router-view
// key) — give the same immediate feedback for that potentially heavy remount.
watch(gameSwitchRevision, () => {
  showRouteProgress();
  hideRouteProgressAfterPaint();
});


// Disable default right-click context menu
const preventContextMenu = (event: Event) => {
  event.preventDefault();
};

onMounted(() => {
  document.addEventListener('contextmenu', preventContextMenu);
});

onUnmounted(() => {
  document.removeEventListener('contextmenu', preventContextMenu);
});

/* bgStyle removed, handled in template */
</script>

<template>
  <!-- Background Layer -->
  <div class="bg-layer">
    <transition-group name="bg-trans">
      <!-- Image Background -->
      <div v-if="appSettings.bgType === BGType.Image && appSettings.bgImage" :key="appSettings.bgImage" class="bg-item"
        :style="{ backgroundImage: `url(${appSettings.bgImage})` }"></div>

      <!-- Video Background -->
      <video v-if="appSettings.bgType === BGType.Video && appSettings.bgVideo" :key="appSettings.bgVideo"
        :src="appSettings.bgVideo" autoplay loop muted playsinline class="bg-item"></video>
    </transition-group>
  </div>

  <!-- Home Ambient Shadow Layer -->
  <div class="home-shadow-layer" v-if="route.path === '/'"></div>

  <!-- Global Mask Layer for Game Library Page -->
  <transition name="fade">
    <div v-if="shouldShowGlobalDimLayer" class="global-dim-layer" :style="{ background: globalDimLayerBackground }">
    </div>
  </transition>

  <el-config-provider>
    <div v-if="AppStateManager.isFirstRunOnboardingOpen.value" class="first-run-overlay">
      <section ref="firstRunDialog" class="first-run-dialog" role="dialog" aria-modal="true"
        :aria-label="t('firstRun.title')">
        <header>
          <div class="first-run-head-row">
            <span class="first-run-kicker">SSMT4</span>
            <span class="first-run-step-counter">{{ t('firstRun.progress') }} · {{ firstRunStep + 1 }}/{{
              firstRunStepCount }}</span>
          </div>
          <h2>{{ t('firstRun.title') }}</h2>
          <p>{{ t(`firstRun.steps.${currentFirstRunStep}.description`) }}</p>
        </header>
        <div class="first-run-progress" :aria-label="t('firstRun.progress')">
          <span v-for="index in firstRunStepCount" :key="index"
            :class="{ 'is-active': index - 1 <= firstRunStep, 'is-current': index - 1 === firstRunStep }"></span>
        </div>
        <transition :name="firstRunTransitionName" mode="out-in">
          <div :key="currentFirstRunStep" class="first-run-body">
            <div v-if="currentFirstRunStep === 'language'" class="first-run-lang-grid">
              <button v-for="language in SSMT_LOCALE_OPTIONS" :key="language.value" type="button" class="first-run-lang"
                :class="{ 'is-selected': appSettings.locale === language.value }"
                @click="appSettings.locale = language.value">
                <span class="first-run-lang-code">{{ language.badge }}</span>
                <span class="first-run-lang-name">{{ language.label }}</span>
                <i v-if="appSettings.locale === language.value" class="first-run-check"></i>
              </button>
            </div>
            <div v-else-if="currentFirstRunStep === 'role'" class="first-run-role-grid">
              <button v-for="role in (['author', 'player', 'both'] as const)" :key="role" type="button"
                class="first-run-role" :class="{ 'is-selected': selectedFirstRunRole === role }"
                @click="selectedFirstRunRole = role">
                <strong>{{ t(`firstRun.roles.${role}`) }}</strong>
                <span>{{ t(`firstRun.roleDescriptions.${role}`) }}</span>
                <i v-if="selectedFirstRunRole === role" class="first-run-check"></i>
              </button>
              <div v-if="selectedFirstRunRole" class="first-run-preview first-run-role-preview">
                <span>{{ t('firstRun.visiblePages') }}</span>
                <div><em v-for="page in selectedFirstRunPages" :key="page">{{ page }}</em></div>
                <p>{{ t('firstRun.settingsHint') }}</p>
              </div>
            </div>
            <div v-else-if="currentFirstRunStep === 'general'" class="first-run-form">
              <p class="first-run-cache-required">{{ t('firstRun.cacheRequired') }}</p>
              <CacheFolderPicker v-model:mode="firstRunCacheMode" :default-path="firstRunDefaultCacheDir"
                v-model:custom-path="firstRunCustomCacheDir" />
              <p v-if="firstRunCacheMode && !firstRunCacheResolved" class="first-run-cache-error">{{
                t('firstRun.cacheRequiredError') }}</p>
              <label class="first-run-switch"><span>{{ t('firstRun.fields.altF') }}</span><el-switch
                  v-model="appSettings.showWindowShortcutEnabled" /></label>
            </div>
            <div v-else-if="currentFirstRunStep === 'authorPreferences'" class="first-run-form">
              <label><span>{{ t('firstRun.fields.textureMarkStyle') }}</span><el-select
                  v-model="appSettings.textureMarkStylePreference"><el-option value="Hash" label="Hash" /><el-option
                    value="Slot" label="Slot" /><el-option value="SharedSlot" label="SharedSlot" /></el-select></label>
              <label><span>{{ t('firstRun.fields.previewLighting') }}</span><el-select
                  v-model="appSettings.postProcessPreviewLightingMode"><el-option value="half-lambert"
                    :label="t('firstRun.options.halfLambert')" /><el-option value="unlit"
                    :label="t('firstRun.options.unlit')" /><el-option value="pbr" label="PBR" /></el-select></label>
            </div>
            <div v-else-if="currentFirstRunStep === 'games'">
              <div class="first-run-game-grid">
                <button v-for="game in orderedFirstRunGames" :key="game.name" type="button"
                  :class="{ 'is-selected': selectedFirstRunGames.includes(game.name) }"
                  @click="toggleFirstRunGame(game.name)"><img :src="game.iconPath" alt=""
                    @error="useFirstRunGameIconFallback" /><span>{{ getGamePresetDisplayName(game.name, t) }}</span><i
                    v-if="selectedFirstRunGames.includes(game.name)" class="first-run-check"></i></button>
              </div>
              <div class="first-run-game-summary">
                <span>{{ t('firstRun.selectedCount', { count: selectedFirstRunGames.length }) }}</span>
                <span>{{ t('firstRun.selectedMoveFront') }}</span>
              </div>
            </div>
            <div v-else-if="currentFirstRunStep === 'd3d11'" class="first-run-form">
              <label><span>{{ t('firstRun.fields.d3d11Source') }}</span><el-radio-group
                  v-model="firstRunD3d11Mode"><el-radio-button v-if="selectedFirstRunRole !== 'player'"
                    value="dev">dev</el-radio-button><el-radio-button
                    value="play">play</el-radio-button><el-radio-button v-if="selectedFirstRunRole !== 'player'"
                    value="ssice-a">ssice-a</el-radio-button></el-radio-group></label>
            </div>
            <div v-else-if="currentFirstRunStep === 'migoto'" class="first-run-form">
              <label><span>{{ t('firstRun.fields.launchMode') }}</span><el-radio-group
                  v-model="firstRunUseShell"><el-radio-button :value="false">{{ t('firstRun.options.normalLaunch')
                  }}</el-radio-button><el-radio-button :value="true">Shell</el-radio-button></el-radio-group></label>
              <label v-if="selectedFirstRunRole !== 'player'"><span>{{ t('firstRun.fields.huntingMode')
              }}</span><el-select v-model="firstRunHuntingMode"><el-option value="0"
                    :label="t('gameSettingsModal.options.huntingMode.off')" /><el-option value="1"
                    :label="t('gameSettingsModal.options.huntingMode.on')" /><el-option value="2"
                    :label="t('gameSettingsModal.options.huntingMode.toggleByNumpad0')" /></el-select></label>
              <label class="first-run-switch"><span>{{ t('firstRun.fields.showWarnings') }}</span><el-switch
                  v-model="firstRunShowWarnings" /></label>
              <label class="first-run-switch"><span>{{ t('firstRun.fields.checkDllUpdate') }}</span><el-switch
                  v-model="firstRunCheckDllUpdate" /></label>
              <label class="first-run-switch"><span>{{ t('firstRun.fields.checkPackageUpdate') }}</span><el-switch
                  v-model="firstRunCheckPackageUpdate" /></label>
            </div>
            <div v-else-if="currentFirstRunStep === 'background'" class="first-run-form">
              <label><span>{{ t('firstRun.fields.backgroundType') }}</span><el-radio-group
                  v-model="firstRunBackgroundType"><el-radio-button value="Video">{{ t('firstRun.options.dynamic')
                  }}</el-radio-button><el-radio-button value="Image">{{ t('firstRun.options.static')
                    }}</el-radio-button></el-radio-group></label>
              <label><span>{{ t('firstRun.fields.backgroundUpdate') }}</span><el-radio-group
                  v-model="firstRunBackgroundUpdateMode"><el-radio-button value="manual">{{ t('firstRun.options.manual')
                  }}</el-radio-button><el-radio-button value="auto">{{ t('firstRun.options.auto')
                    }}</el-radio-button></el-radio-group></label>
              <label><span>{{ t('settings.personalization.backgroundMaskOpacity') }}</span><el-slider
                  v-model="appSettings.globalDimMaskStrength" :min="0" :max="4" :step="0.1" show-input /></label>
            </div>
            <div v-else-if="currentFirstRunStep === 'nsfw'" class="first-run-form">
              <label><span>{{ t('firstRun.fields.modBlurMode') }}</span><el-radio-group
                  v-model="appSettings.modsManagementBlurMode"><el-radio-button value="all">{{
                    t('firstRun.options.blurAll') }}</el-radio-button><el-radio-button value="nsfw">{{
                      t('firstRun.options.blurNsfw') }}</el-radio-button><el-radio-button value="none">{{
                      t('firstRun.options.blurNone') }}</el-radio-button></el-radio-group></label>
              <label class="first-run-switch"><span>{{ t('firstRun.fields.revealOnHover') }}</span><el-switch
                  v-model="appSettings.revealBlurredImagesOnHover" /></label>
              <label><span>{{ t('firstRun.fields.showNsfwAcquisition') }}</span><el-radio-group
                  v-model="appSettings.gamebananaShowNsfw"><el-radio-button :value="true">{{ t('firstRun.options.show')
                  }}</el-radio-button><el-radio-button :value="false">{{ t('firstRun.options.hide')
                    }}</el-radio-button></el-radio-group></label>
            </div>
          </div>
        </transition>
        <footer class="first-run-actions">
          <el-button v-if="firstRunStep > 0" @click="retreatFirstRun">{{ t('firstRun.back') }}</el-button>
          <span class="first-run-actions-grow"></span>
          <el-button v-if="firstRunStep < firstRunStepCount - 1 && currentFirstRunStep !== 'role' && currentFirstRunStep !== 'general'"
            text @click="advanceFirstRun">{{ t('firstRun.skip') }}</el-button>
          <el-button v-if="firstRunStep < firstRunStepCount - 1" type="primary" :disabled="firstRunNextDisabled"
            @click="advanceFirstRun">{{ t('firstRun.next') }}</el-button>
          <el-button v-else type="primary" :disabled="!firstRunCacheResolved" @click="confirmFirstRunRole">{{
            t('firstRun.confirm') }}</el-button>
        </footer>
      </section>
    </div>

    <!-- 未设置缓存目录时强制设置：不选择与确认无法使用程序 -->
    <div v-else-if="cacheRequiredBlocked" class="first-run-overlay">
      <section class="first-run-dialog first-run-cache-gate" role="dialog" aria-modal="true"
        :aria-label="t('firstRun.cacheRequiredTitle')">
        <header>
          <div class="first-run-head-row">
            <span class="first-run-kicker">SSMT4</span>
          </div>
          <h2>{{ t('firstRun.cacheRequiredTitle') }}</h2>
          <p>{{ t('firstRun.cacheRequiredMessage') }}</p>
        </header>
        <div class="first-run-body first-run-form">
          <p class="first-run-cache-required">{{ t('firstRun.cacheRequired') }}</p>
          <CacheFolderPicker v-model:mode="firstRunCacheMode" :default-path="firstRunDefaultCacheDir"
            v-model:custom-path="firstRunCustomCacheDir" />
          <p v-if="firstRunCacheMode && !firstRunCacheResolved" class="first-run-cache-error">{{
            t('firstRun.cacheRequiredError') }}</p>
        </div>
        <footer class="first-run-actions">
          <span class="first-run-actions-grow"></span>
          <el-button type="primary" :disabled="!firstRunCacheResolved" @click="confirmCacheRequiredGate">{{
            t('firstRun.confirmCacheGate') }}</el-button>
        </footer>
      </section>
    </div>

    <div class="app-ui-scale" :style="{ '--app-ui-scale': appUiScale }">
      <!-- WinUI3 shell: full-width caption bar on top, nav pane + content below -->
      <div class="app-shell">
        <!-- Reserves the caption bar's layout space; the bar itself is
             teleported to <body> so no popup/card can ever cover it. -->
        <div class="caption-bar-spacer" aria-hidden="true"></div>

        <div class="app-shell-body">
          <main class="app-main">
            <div class="content-scroll-wrapper glass-scrollbar" :class="{ 'no-scroll': route.path === '/' }">
              <router-view v-slot="{ Component }">
                <transition name="page-blur">
                  <KeepAlive>
                    <component :is="Component"
                      :key="`${route.fullPath}:${appSettings.CurrentGameName}:${gameSwitchRevision}`" />
                  </KeepAlive>
                </transition>
              </router-view>
            </div>
          </main>
        </div>
      </div>
    </div>
  </el-config-provider>

  <!-- Caption bar lives above every layer of the app (highest z-index), so
       window dragging / minimize / maximize / close stay usable no matter
       which card, dialog or overlay is open. It is scaled with the same
       --app-ui-scale factor so it pixel-matches the in-flow layout. -->
  <Teleport to="body">
    <div class="caption-bar-host" :style="{ '--app-ui-scale': appUiScale }">
      <TitleBar />
      <div class="route-progress" :class="{ active: isNavigating }" aria-hidden="true"></div>
    </div>
  </Teleport>
</template>

<style>
/* Global Resets */
html,
body {
  margin: 0;
  padding: 0;
  height: 100%;
  font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;

  /* Disable text selection */
  user-select: none;

  /* Cyberpunk Black Fallback: Deep dark with subtle neon glows */
  background-color: #030305;
  background-image:
    radial-gradient(circle at 50% 50%, rgba(60, 20, 100, 0.2) 0%, transparent 60%),
    radial-gradient(circle at 50% 50%, rgba(0, 100, 180, 0.1) 0%, transparent 70%);

  overflow: hidden;
}

/* Re-enable selection for inputs */
input,
textarea {
  user-select: text;
}

/* Re-enable selection inside popups, dialogs, drawers, popovers, context
   menus, floating cards and other overlay-style cards. The app shell itself
   stays non-selectable for native window-drag / button interactions. */
.el-overlay,
.el-overlay-dialog,
.el-dialog,
.el-drawer,
.el-message-box,
.el-message,
.el-notification,
.el-popper,
.el-select-dropdown,
.el-dropdown-menu,
.settings-dropdown,
.settings-dropdown-inner,
.submenu-card,
.game-switcher-card,
.shell-flyout,
.games-drawer-panel,
.first-run-overlay,
.first-run-dialog,
.app-update-dialog-overlay,
.app-update-dialog,
.settings-overlay,
.settings-window,
.preset-overlay,
.preset-popover,
.key-floater,
.mod-3d-floating-overlay,
.mod-3d-floating-page,
.preview-settings-overlay,
.preview-settings-floating-page,
.preview-zoom-overlay,
.channel-preview-overlay,
.channel-preview-modal,
.xz-plan-pop,
.xz-tool-card,
.xz-usage-popover,
.xz-log-drawer,
.xz-settings-dialog,
.xz-lightbox,
.context-menu-root,
.context-menu-surface,
.custom-context-menu,
.group-context-menu,
.version-info-popper,
.glass-dialog,
[class*="popover"],
[class*="popup"],
[class*="popper"],
[class*="dialog"],
[class*="dropdown"],
[class*="flyout"],
[class*="floater"],
[class*="floating"],
[class*="modal"],
[class*="overlay"],
[class*="context-menu"],
[class*="menu-root"],
[class*="pop"] {
  user-select: text !important;
  -webkit-user-select: text !important;
  -moz-user-select: text !important;
}

#app {
  position: fixed;
  inset: 0;
  width: auto;
  height: auto;
  margin: 0;
  overflow: hidden;
}

.bg-layer {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  z-index: 0;
  overflow: hidden;
  background-color: #050505;
}

/* Background Transition Items */
.bg-item {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  background-size: cover;
  background-position: center;
  will-change: opacity;
}

/* Transition Classes */
.bg-trans-enter-active,
.bg-trans-leave-active {
  transition: opacity 0.6s ease;
  /* Smooth 0.6s fade */
}

.bg-trans-enter-from,
.bg-trans-leave-to {
  opacity: 0;
}

.home-shadow-layer {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  z-index: 0;
  /* On top of bg-layer (also 0, but later in DOM), behind app-container (1) */
  pointer-events: none;
  /* 
     Fix: Removed 'multiply' blend mode which made things look dirty.
     New Style: Clean cinematic vignette + bottom fade for UI readability.
     Keeps the center bright and clean.
  */
  background:
    /* 1. Seamless smooth fade from bottom (for potential footer text) */
    linear-gradient(to top, rgba(0, 0, 0, 0.5) 0%, transparent 25%),

    /* 2. Very subtle cinematic vignette (corners only, center is pure clean) */
    radial-gradient(circle at 50% 50%, transparent 75%, rgba(0, 0, 0, 0.4) 140%);
}

.global-dim-layer {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  z-index: 0;
  /* Above bg-layer (0 via DOM order), below App Content (1) */
  pointer-events: none;
  /* Let clicks pass through if needed, though standard bg doesn't need interactions */
}
</style>

<style scoped>
.first-run-overlay {
  position: fixed;
  inset: 0;
  z-index: 5000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: rgba(3, 6, 12, 0.68);
  backdrop-filter: blur(12px);
}

.first-run-dialog {
  width: min(680px, calc(100vw - 48px));
  height: min(700px, calc(100vh - 48px));
  padding: 26px;
  box-sizing: border-box;
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr) auto;
  border: var(--t-card-dark-border);
  border-radius: 18px;
  background: var(--t-card-dark-bg);
  box-shadow: var(--t-card-dark-shadow);
  overflow: hidden;
}

.first-run-dialog header h2 {
  margin: 5px 0 8px;
  font-size: 22px;
}

.first-run-dialog header p {
  color: rgba(255, 255, 255, .62);
  line-height: 1.55;
}

.first-run-kicker {
  color: rgba(117, 214, 187, .9);
  font-size: 11px;
  font-weight: 800;
  letter-spacing: .14em;
}

.first-run-head-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.first-run-step-counter {
  color: rgba(255, 255, 255, .42);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: .05em;
}

.first-run-role-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
  margin: 20px 0;
}

.first-run-role,
.first-run-lang {
  position: relative;
  padding: 14px;
  border: 1px solid rgba(255, 255, 255, .11);
  border-radius: 12px;
  background: rgba(255, 255, 255, .035);
  color: white;
  text-align: left;
  cursor: pointer;
  transition: border-color .18s ease, background .18s ease, transform .18s ease;
}

.first-run-role:hover,
.first-run-lang:hover {
  border-color: rgba(255, 255, 255, .26);
  background: rgba(255, 255, 255, .06);
}

.first-run-role:active,
.first-run-lang:active {
  transform: scale(.99);
}

.first-run-role {
  min-height: 94px;
}

.first-run-role strong,
.first-run-role span {
  display: block;
}

.first-run-role span {
  margin-top: 7px;
  color: rgba(255, 255, 255, .55);
  font-size: 12px;
  line-height: 1.45;
}

.first-run-role.is-selected,
.first-run-lang.is-selected {
  border-color: rgba(117, 214, 187, .65);
  background: rgba(117, 214, 187, .13);
}

.first-run-lang-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
  width: 100%;
  margin: 20px 0;
}

.first-run-lang {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  min-height: 58px;
  padding: 10px 12px;
}

.first-run-lang-code {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: 9px;
  background: rgba(255, 255, 255, .09);
  font-size: 12px;
  font-weight: 800;
}

.first-run-lang.is-selected .first-run-lang-code {
  background: rgba(117, 214, 187, .22);
  color: rgba(117, 214, 187, 1);
}

.first-run-lang-name {
  min-width: 0;
  padding-right: 16px;
  overflow: hidden;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
  text-overflow: ellipsis;
}

.first-run-check {
  position: absolute;
  top: 10px;
  right: 10px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: rgba(117, 214, 187, .92);
  box-shadow: 0 0 8px rgba(117, 214, 187, .35);
}

.first-run-check::after {
  content: '';
  position: absolute;
  left: 6px;
  top: 3px;
  width: 4px;
  height: 9px;
  border: solid #0a1410;
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
}

.first-run-preview {
  padding: 14px;
  border-radius: 12px;
  background: rgba(255, 255, 255, .035);
}

.first-run-preview>span {
  color: rgba(255, 255, 255, .56);
  font-size: 12px;
}

.first-run-preview div {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 9px;
}

.first-run-preview em {
  padding: 5px 9px;
  border-radius: 999px;
  background: rgba(255, 255, 255, .08);
  color: rgba(255, 255, 255, .9);
  font-size: 12px;
  font-style: normal;
}

.first-run-role-preview {
  grid-column: 1 / -1;
}

.first-run-role-preview p {
  margin: 10px 0 0;
  color: rgba(255, 255, 255, .55);
  font-size: 12px;
}

.first-run-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin: 20px 0;
}

.first-run-body {
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: safe center;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 4px 8px 4px 2px;
  box-sizing: border-box;
  scrollbar-width: thin;
  scrollbar-color: rgba(117, 214, 187, .42) rgba(255, 255, 255, .04);
}

.first-run-body::-webkit-scrollbar {
  width: 6px;
}

.first-run-body::-webkit-scrollbar-track {
  border-radius: 999px;
  background: rgba(255, 255, 255, .04);
}

.first-run-body::-webkit-scrollbar-thumb {
  border-radius: 999px;
  background: rgba(117, 214, 187, .42);
}

.first-run-body::-webkit-scrollbar-thumb:hover {
  background: rgba(117, 214, 187, .62);
}

.first-run-form>label {
  display: grid;
  grid-template-columns: minmax(180px, 1fr) minmax(240px, 1.25fr);
  align-items: center;
  gap: 18px;
  min-height: 38px;
}

.first-run-form>label>span {
  color: rgba(255, 255, 255, .84);
  font-size: 13px;
}

.first-run-form .first-run-switch {
  grid-template-columns: minmax(180px, 1fr) minmax(240px, 1.25fr);
}

.first-run-form .first-run-switch .el-switch {
  justify-self: start;
}

.first-run-path {
  display: flex;
  gap: 8px;
  min-width: 0;
}

.first-run-path .el-input {
  flex: 1;
}

/* 缓存目录强制设置（引导页 + 未设置拦截弹窗） */
.first-run-cache-required {
  margin: 0 0 12px;
  color: rgba(117, 214, 187, .95);
  font-size: 13px;
  font-weight: 700;
}

.first-run-cache-error {
  margin: 10px 0 0;
  color: #ff8a7a;
  font-size: 12.5px;
}

.first-run-cache-gate {
  grid-template-rows: auto minmax(0, 1fr) auto;
}

.first-run-game-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
  margin: 20px 0 12px;
}

.first-run-game-grid button {
  display: flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  min-height: 48px;
  height: auto;
  /* 游戏预设名较长时会换行，高度随内容自动增长 */
  padding: 7px 9px;
  border: 1px solid rgba(255, 255, 255, .1);
  border-radius: 9px;
  background: rgba(255, 255, 255, .03);
  color: rgba(255, 255, 255, .76);
  cursor: pointer;
  transition: border-color .18s ease, background .18s ease;
}

.first-run-game-grid button:hover {
  border-color: rgba(255, 255, 255, .24);
  background: rgba(255, 255, 255, .06);
}

.first-run-game-grid button.is-selected {
  order: -1;
  border-color: rgba(117, 214, 187, .58);
  background: rgba(117, 214, 187, .13);
  color: white;
}

.first-run-game-grid img {
  flex: 0 0 auto;
  width: 30px;
  height: 30px;
  object-fit: contain;
}

.first-run-game-grid span {
  min-width: 0;
  padding-right: 12px;
  line-height: 1.35;
  overflow-wrap: anywhere;
  white-space: normal;
  text-align: left
}

.first-run-game-grid button .first-run-check {
  top: 7px;
  right: 7px;
  width: 15px;
  height: 15px;
  box-shadow: none;
}

.first-run-game-grid button .first-run-check::after {
  left: 5px;
  top: 2px;
  width: 3px;
  height: 7px;
}

.first-run-game-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin: 0 2px 18px;
  color: rgba(255, 255, 255, .5);
  font-size: 12px;
}

.first-run-progress {
  display: flex;
  gap: 6px;
  margin-top: 18px;
}

.first-run-progress span {
  flex: 1;
  height: 3px;
  border-radius: 3px;
  background: rgba(255, 255, 255, .1);
  transition: background .25s ease, box-shadow .25s ease;
}

.first-run-progress span.is-active {
  background: rgba(117, 214, 187, .72);
}

.first-run-progress span.is-current {
  background: rgba(117, 214, 187, .95);
  box-shadow: 0 0 8px rgba(117, 214, 187, .4);
}

.first-run-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 18px;
}

.first-run-actions-grow {
  flex: 1;
}

.first-run-dialog :deep(.el-radio-group) {
  display: inline-flex;
  gap: 3px;
  padding: 3px;
  border-radius: 9px;
  background: rgba(255, 255, 255, .045);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, .07);
}

.first-run-dialog :deep(.el-radio-button__inner) {
  min-height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 12px;
  border: 0 !important;
  border-radius: 7px !important;
  background: rgba(13, 18, 25, .58) !important;
  color: rgba(235, 242, 248, .72) !important;
  font: inherit;
  font-size: 12px;
  line-height: 1;
  box-shadow: none !important;
  transition: background .16s ease, color .16s ease, box-shadow .16s ease;
}

.first-run-dialog :deep(.el-radio-button__inner:hover) {
  color: rgba(245, 250, 253, .96) !important;
  background: rgba(255, 255, 255, .075) !important;
}

.first-run-dialog :deep(.el-radio-button.is-active .el-radio-button__inner),
.first-run-dialog :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) {
  background: rgba(76, 155, 139, .28) !important;
  color: rgba(238, 255, 250, .98) !important;
  box-shadow: inset 0 0 0 1px rgba(117, 214, 187, .52) !important;
}

.first-run-forward-enter-active,
.first-run-forward-leave-active,
.first-run-backward-enter-active,
.first-run-backward-leave-active {
  transition: opacity .2s ease, transform .2s ease;
}

.first-run-forward-enter-from {
  opacity: 0;
  transform: translateX(28px);
}

.first-run-forward-leave-to {
  opacity: 0;
  transform: translateX(-28px);
}

.first-run-backward-enter-from {
  opacity: 0;
  transform: translateX(-28px);
}

.first-run-backward-leave-to {
  opacity: 0;
  transform: translateX(28px);
}

@media (max-width: 720px) {
  .first-run-lang-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 620px) {

  .first-run-role-grid,
  .first-run-game-grid {
    grid-template-columns: 1fr;
  }

  .first-run-form>label {
    grid-template-columns: 1fr;
    gap: 7px;
  }
}

@media (max-width: 440px) {
  .first-run-lang-grid {
    grid-template-columns: 1fr;
  }
}

.app-shell {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  overflow: hidden;
  position: relative;
  z-index: 1;
  /* Above bg */
}

/* Layout placeholder for the teleported caption bar (matches its 32px height) */
.caption-bar-spacer {
  flex: 0 0 auto;
  height: 32px;
}

.app-shell-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  overflow: hidden;
  position: relative;
}

.app-ui-scale {
  position: absolute;
  top: 0;
  left: 0;
  width: calc(100% / var(--app-ui-scale, 1));
  height: calc(100% / var(--app-ui-scale, 1));
  transform: scale(var(--app-ui-scale, 1));
  transform-origin: top left;
}

/* Main Content Styles — fills the whole shell width now that navigation
   lives in the caption bar tabs instead of a floating left pane.
   No dark veil here: the game background stays bright behind sub-pages. */
.app-main {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  position: relative;
  padding: 0;
  box-sizing: border-box;
  overflow: hidden;
  color: #ffffff;
}

.content-scroll-wrapper {
  position: absolute;
  inset: 0;
  overflow-y: auto;
  overflow-x: hidden;
  /* Prevent flash of horizontal bar during transitions */

  /* 
  杩欓噷娉ㄦ剰鍟婏紝scrollbar-gutter鍗冧竾涓嶈鍔燽oth-edges
  鍚﹀垯浼氬鑷?宸﹀彸涓や晶鐨勬粦鏉′綅缃?琚己鍒剁┖鍑烘潵涓€鍧楀効
  瀵艰嚧瑙嗚鏁堟灉涓嶄匠
  */
  scrollbar-gutter: stable;
  /* Reserve space to avoid scrollbar flicker */
  box-sizing: border-box;
  /* Ensures padding doesn't cause overflow */
  /* position:relative intentionally NOT set here — .app-main (parent) already has it,
     and this would create a containing block that clips absolutely-positioned
     children (dropdowns, context menus) via overflow-y:auto */
}



.no-scroll {
  overflow-y: hidden;
}

/* Glassmorphism for Element Plus Components */
:deep(.el-card) {
  background-color: rgba(30, 30, 30, 0.6);
  border: 1px solid rgba(255, 255, 255, 0.1);
  color: #ffffff;
  --el-card-bg-color: transparent;
}

:deep(.el-card__header) {
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
  color: #fff;
}

/* Form Labels */
:deep(.el-form-item__label) {
  color: #e0e0e0;
}

/* Page Transition Effects */
.page-blur-enter-active,
.page-blur-leave-active {
  transition: all 0.2s ease-out;
  /* 鎭㈠ 0.2s 杩囨浮 */
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
}

.page-blur-enter-from {
  opacity: 0;
  transform: scale(0.98);
}

.page-blur-leave-to {
  opacity: 0;
  transform: scale(1);
}

/* Teleported caption bar — always the topmost layer in the window.
   Fullscreen overlays (.preview-zoom-overlay etc.) intentionally stop at
   top:32px with near-max z-index, so the bar must take the absolute max. */
.caption-bar-host {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100% / var(--app-ui-scale, 1));
  transform: scale(var(--app-ui-scale, 1));
  transform-origin: top left;
  z-index: 2147483647;
}

/* Route-change progress hairline — a whisper of feedback while pages load */
.route-progress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 2px;
  overflow: hidden;
  opacity: 0;
  transition: opacity 0.25s ease;
  pointer-events: none;
}

.route-progress.active {
  opacity: 1;
  transition: opacity 0.08s ease;
}

.route-progress::before {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 38%;
  border-radius: 2px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.75), transparent);
  transform: translateX(-100%);
}

.route-progress.active::before {
  animation: route-progress-slide 1.1s cubic-bezier(0.4, 0, 0.4, 1) infinite;
}

/* Element width is 38% of the bar, so ~270% self-translation sweeps it
   fully off the right edge regardless of window width or UI scale. */
@keyframes route-progress-slide {
  to {
    transform: translateX(270%);
  }
}
</style>
