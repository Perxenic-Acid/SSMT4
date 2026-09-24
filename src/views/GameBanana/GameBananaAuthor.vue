<script setup lang="ts">
import { GAMEBANANA_SORTS, readGameBananaSort, saveGameBananaSort } from './gameBananaSort';
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { fetch } from '@tauri-apps/plugin-http';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ElMessage } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { AppStateManager } from '../../store/AppStateManager';
import { gameBananaHistory, type GameBananaHistoryEntry } from './gameBananaHistory';
import { setGameBananaBlurMode, setGameBananaShowNsfw } from './gameBananaBlurSettings';

type GbImage = Record<string, unknown>;

interface GbSubmitter {
  _idRow?: number;
  _sName?: string;
  _sProfileUrl?: string;
}

interface GbGameRef {
  _idRow?: number;
  _sName?: string;
  _sProfileUrl?: string;
  _sIconUrl?: string;
}

interface GbRecord {
  _idRow: number;
  _sName?: string;
  _sDescription?: string;
  _sProfileUrl?: string;
  _tsDateUpdated?: number;
  _tsDateModified?: number;
  _bIsNsfw?: boolean;
  _bHasContentRatings?: boolean;
  _aSubmitter?: GbSubmitter;
  _aGame?: GbGameRef;
  _aPreviewMedia?: { _aImages?: GbImage[] };
}

interface GbIndexPayload {
  _aMetadata?: { _nRecordCount?: number; _bIsComplete?: boolean };
  _aRecords?: GbRecord[];
}

interface GbAuthorProfile {
  _idRow: number;
  _sName?: string;
  _sProfileUrl?: string;
  _sAvatarUrl?: string;
  _sUserTitle?: string;
  _sHonoraryTitle?: string;
  _sLocation?: string;
  _tsJoinDate?: number;
  _nSubscriberCount?: number;
  _nPostCount?: number;
}

interface AuthorModCard {
  id: number;
  title: string;
  profileUrl: string;
  gameId: number | null;
  gameName: string;
  gameIconUrl: string;
  description: string;
  thumbnailUrl: string;
  updatedAt: number;
  isNsfw: boolean;
}

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const appSettings = AppStateManager.appSettings;
const API_BASE = 'https://gamebanana.com/apiv11';

const profile = ref<GbAuthorProfile | null>(null);
const mods = ref<AuthorModCard[]>([]);
const page = ref(1);
const hasMore = ref(false);
const loadingProfile = ref(false);
const loadingMods = ref(false);
const errorMessage = ref('');
let profileRequestId = 0;
let modsRequestId = 0;

const authorId = computed(() => {
  const value = Number(route.params.authorId);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
});
const visibleMods = computed(() => appSettings.gamebananaShowNsfw ? mods.value : mods.value.filter((item) => !item.isNsfw));
const gameBananaBlurMode = computed({ get: () => appSettings.gamebananaBlurMode, set: (mode) => setGameBananaBlurMode(appSettings, mode) });
const shouldBlurImage = (isNsfw: boolean): boolean => appSettings.gamebananaBlurMode === 'all'
  || (appSettings.gamebananaBlurMode === 'nsfw' && isNsfw);

const asString = (value: unknown): string => typeof value === 'string' ? value.trim() : '';
const asNumber = (value: unknown): number => Number.isFinite(Number(value)) ? Number(value) : 0;
const toPlainText = (value: unknown): string => asString(value)
  .replace(/<br\s*\/?\s*>/gi, '\n')
  .replace(/<[^>]+>/g, '')
  .replace(/&nbsp;/gi, ' ')
  .replace(/&amp;/gi, '&')
  .replace(/\s+/g, ' ')
  .trim();

const gamebananaUrl = (value: unknown, fallbackPath: string): string => {
  const raw = asString(value);
  try {
    return new URL(raw || fallbackPath, 'https://gamebanana.com').toString();
  } catch {
    return `https://gamebanana.com${fallbackPath}`;
  }
};

const imageUrl = (image: GbImage): string => {
  for (const key of ['_sUrl530', '_sUrl220', '_sUrl100', '_sUrl']) {
    const url = asString(image[key]);
    if (url) return url;
  }
  const baseUrl = asString(image._sBaseUrl) || 'https://images.gamebanana.com/img/ss/mods';
  for (const key of ['_sFile530', '_sFile220', '_sFile100', '_sFile']) {
    const file = asString(image[key]);
    if (file) return `${baseUrl.replace(/\/$/, '')}/${file.replace(/^\//, '')}`;
  }
  return '';
};

const apiGet = async <T>(path: string, params: Record<string, string> = {}): Promise<T> => {
  const response = await fetch(`${API_BASE}${path}?${new URLSearchParams(params).toString()}`, { method: 'GET' });
  if (!response.ok) throw new Error(`GameBanana HTTP ${response.status}`);
  return await response.json() as T;
};

const cardFromRecord = (record: GbRecord): AuthorModCard => ({
  id: record._idRow,
  title: asString(record._sName) || `Mod #${record._idRow}`,
  profileUrl: gamebananaUrl(record._sProfileUrl, `/mods/${record._idRow}`),
  gameId: record._aGame?._idRow ?? null,
  gameName: asString(record._aGame?._sName),
  gameIconUrl: asString(record._aGame?._sIconUrl),
  description: toPlainText(record._sDescription),
  thumbnailUrl: (record._aPreviewMedia?._aImages || []).map(imageUrl).find(Boolean) || '',
  updatedAt: asNumber(record._tsDateUpdated || record._tsDateModified),
  isNsfw: record._bIsNsfw === true || record._bHasContentRatings === true,
});

const formatDate = (timestamp: number): string => timestamp ? new Date(timestamp * 1000).toLocaleDateString() : '—';
const formatNumber = (value: number): string => new Intl.NumberFormat().format(value || 0);

const openExternal = async (url: string) => {
  if (!url) return;
  try {
    await openUrl(url);
  } catch (error) {
    ElMessage.error(t('gameBanana.errors.openExternal', { error: String(error) }));
  }
};

const restoreGameEnvironment = async (gameName: string) => {
  const targetGameName = gameName.trim();
  const currentGameName = appSettings.CurrentGameName?.trim() || 'Default';
  if (!targetGameName || targetGameName === currentGameName) return;

  if (targetGameName === 'Default') {
    AppStateManager.switchToDefaultGame();
    return;
  }

  const game = AppStateManager.gamesList.find((item) => item.name === targetGameName);
  if (game) await AppStateManager.selectGame(game);
};

const openModPage = async (mod: AuthorModCard) => {
  const returnGame = appSettings.CurrentGameName?.trim() || 'Default';
  await gameBananaHistory.replace(router, {
    kind: 'author',
    title: profile.value?._sName || `#${route.params.authorId}`,
    location: {
      name: 'GameBananaAuthor',
      params: { authorId: String(route.params.authorId) },
      query: { ...route.query, returnGame },
    },
  });

  const query: Record<string, string> = { mod: String(mod.id) };
  if (mod.gameId) query.game = String(mod.gameId);
  if (mod.gameName) query.gameName = mod.gameName;
  await gameBananaHistory.push(router, { kind: 'mod', title: mod.title, location: { name: 'GameBanana', query } });
};

const goBack = () => { void gameBananaHistory.go(router, -1); };
const goForward = () => { void gameBananaHistory.go(router, 1); };
const goParent = () => { void gameBananaHistory.parent(router); };
const goHome = () => { void gameBananaHistory.home(router); };
const jumpToHistory = (entryIndex: number) => { void gameBananaHistory.jump(router, entryIndex); };
const historyLabel = (entry: GameBananaHistoryEntry): string => {
  if (entry.kind === 'home') return t('gameBanana.historyInitial');
  if (entry.kind === 'author') return t('gameBanana.historyAuthor', { name: entry.title });
  return t('gameBanana.historyMod', { name: entry.title });
};

const loadAuthor = async () => {
  const id = authorId.value;
  if (!id) {
    errorMessage.value = t('gameBanana.authorInvalid');
    return;
  }
  const currentRequest = ++profileRequestId;
  loadingProfile.value = true;
  errorMessage.value = '';
  try {
    const result = await apiGet<GbAuthorProfile>(`/Member/${id}/ProfilePage`);
    if (currentRequest === profileRequestId) profile.value = result;
  } catch (error) {
    if (currentRequest === profileRequestId) errorMessage.value = t('gameBanana.errors.loadAuthor', { error: String(error) });
  } finally {
    if (currentRequest === profileRequestId) loadingProfile.value = false;
  }
};

const sortMode = ref(readGameBananaSort());
const changeSort = () => {
  saveGameBananaSort(sortMode.value);
  void loadMods(1);
};

const loadMods = async (requestedPage = 1) => {
  const id = authorId.value;
  if (!id) return;
  const currentRequest = ++modsRequestId;
  page.value = Math.max(1, requestedPage);
  loadingMods.value = true;
  try {
    const payload = await apiGet<GbIndexPayload>('/Mod/Index', {
      _nPage: String(page.value),
      _nPerpage: '24',
      _sSort: sortMode.value,
      '_aFilters[Generic_Submitter]': String(id),
    });
    if (currentRequest !== modsRequestId) return;
    mods.value = (payload._aRecords || []).map(cardFromRecord);
    hasMore.value = !payload._aMetadata?._bIsComplete && mods.value.length >= 24;
  } catch (error) {
    if (currentRequest === modsRequestId) errorMessage.value = t('gameBanana.errors.loadAuthorMods', { error: String(error) });
  } finally {
    if (currentRequest === modsRequestId) loadingMods.value = false;
  }
};

const refresh = async () => {
  await Promise.all([loadAuthor(), loadMods(1)]);
};

watch(authorId, () => { void refresh(); });
watch(() => route.fullPath, () => gameBananaHistory.observe(router, route), { immediate: true });
watch(
  () => route.name === 'GameBananaAuthor' ? route.query.returnGame : undefined,
  (value) => {
    const gameName = Array.isArray(value) ? value[0] : value;
    if (typeof gameName === 'string') void restoreGameEnvironment(gameName);
  },
  { immediate: true },
);
onMounted(() => { void refresh(); });
</script>

<template>
  <div class="page-container gb-author-page">
    <nav class="gb-history-nav" :aria-label="t('gameBanana.historyNavigation')">
      <div class="gb-history-trail">
        <template v-for="(entry, entryIndex) in gameBananaHistory.items" :key="`${entryIndex}:${entry.title}`">
          <span v-if="entryIndex" class="gb-history-separator" aria-hidden="true">/</span>
          <button type="button" class="gb-history-entry" :class="{ 'is-current': entryIndex === gameBananaHistory.currentIndex }" :title="historyLabel(entry)" @click="jumpToHistory(entryIndex)">{{ historyLabel(entry) }}</button>
        </template>
      </div>
      <button type="button" class="gb-history-button" :title="t('gameBanana.historyBack')" :aria-label="t('gameBanana.historyBack')" :disabled="!gameBananaHistory.canGoBack" @click="goBack">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 18-6-6 6-6" /></svg>
      </button>
      <button type="button" class="gb-history-button" :title="t('gameBanana.historyForward')" :aria-label="t('gameBanana.historyForward')" :disabled="!gameBananaHistory.canGoForward" @click="goForward">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 18 6-6-6-6" /></svg>
      </button>
      <button type="button" class="gb-history-button" :title="t('gameBanana.historyParent')" :aria-label="t('gameBanana.historyParent')" :disabled="!gameBananaHistory.canGoParent" @click="goParent">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20V5" /><path d="m6 11 6-6 6 6" /></svg>
      </button>
      <button type="button" class="gb-history-button" :title="t('gameBanana.historyHome')" :aria-label="t('gameBanana.historyHome')" :disabled="gameBananaHistory.isHome" @click="goHome">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m3 10 9-7 9 7" /><path d="M5 9.5V21h14V9.5" /><path d="M9 21v-7h6v7" /></svg>
      </button>
    </nav>
    <section class="gb-author-toolbar glass-panel">
      <span class="gb-author-toolbar-title">{{ t('gameBanana.authorPage') }}</span>
      <label class="gb-author-sort-field">
        <span>{{ t('gameBanana.sort') }}</span>
        <el-select v-model="sortMode" class="gb-select" popper-class="gamebanana-select-popper" :aria-label="t('gameBanana.sort')" @change="changeSort">
          <el-option v-for="sort in GAMEBANANA_SORTS" :key="sort" :label="t(`gameBanana.sortOptions.${sort}`)" :value="sort" />
        </el-select>
      </label>
      <label class="gb-author-nsfw-field" :title="t('gameBanana.nsfwShown')">
        <span>{{ t('gameBanana.nsfwShown') }}</span>
        <el-radio-group v-model="gameBananaBlurMode" class="gb-author-nsfw-mode">
          <el-radio-button value="all">{{ t('gameBanana.blurAllImages') }}</el-radio-button>
          <el-radio-button value="nsfw" :disabled="!appSettings.gamebananaShowNsfw">{{ t('gameBanana.blurNsfwImages') }}</el-radio-button>
          <el-radio-button value="none">{{ t('gameBanana.blurNoImages') }}</el-radio-button>
        </el-radio-group>
      </label>
      <label class="gb-author-show-field"><span>{{ t('gameBanana.nsfwShown') }}</span><el-radio-group :model-value="appSettings.gamebananaShowNsfw" class="gb-author-visibility" @change="(value: string | number | boolean | undefined) => setGameBananaShowNsfw(appSettings, value === true)"><el-radio-button :value="true">{{ t('gameBanana.nsfwShow') }}</el-radio-button><el-radio-button :value="false">{{ t('gameBanana.nsfwHide') }}</el-radio-button></el-radio-group></label>
      <button type="button" class="gb-button" :disabled="loadingProfile || loadingMods" @click="refresh">{{ t('gameBanana.refresh') }}</button>
    </section>

    <p v-if="errorMessage" class="gb-author-error">{{ errorMessage }}</p>
    <main class="gb-author-layout">
      <section class="gb-author-profile glass-panel">
        <div v-if="loadingProfile" class="gb-empty">{{ t('gameBanana.loading') }}</div>
        <template v-else-if="profile">
          <img v-if="profile._sAvatarUrl" class="gb-author-avatar" :src="profile._sAvatarUrl" :alt="profile._sName" />
          <div class="gb-author-profile-copy">
            <h1>{{ profile._sName || `#${profile._idRow}` }}</h1>
            <p v-if="profile._sUserTitle || profile._sHonoraryTitle">{{ profile._sUserTitle || profile._sHonoraryTitle }}</p>
            <p v-if="profile._sLocation">{{ toPlainText(profile._sLocation) }}</p>
            <div class="gb-author-stats">
              <span>{{ t('gameBanana.subscribers') }}<strong>{{ formatNumber(asNumber(profile._nSubscriberCount)) }}</strong></span>
              <span>{{ t('gameBanana.posts') }}<strong>{{ formatNumber(asNumber(profile._nPostCount)) }}</strong></span>
              <span>{{ t('gameBanana.joined') }}<strong>{{ formatDate(asNumber(profile._tsJoinDate)) }}</strong></span>
            </div>
            <button type="button" class="gb-button gb-button--primary" @click="openExternal(profile._sProfileUrl || '')">{{ t('gameBanana.openAuthorPage') }}</button>
          </div>
        </template>
      </section>

      <section class="gb-author-mods glass-panel">
        <header><strong>{{ t('gameBanana.authorMods') }}</strong><span>{{ visibleMods.length }}</span></header>
        <div v-if="loadingMods" class="gb-empty">{{ t('gameBanana.loading') }}</div>
        <div v-else-if="visibleMods.length" class="gb-author-mod-grid">
          <button v-for="mod in visibleMods" :key="mod.id" type="button" class="gb-author-card" @click="openModPage(mod)">
            <div class="gb-author-thumb" :class="{ 'is-nsfw-blurred': shouldBlurImage(mod.isNsfw), 'can-hover-reveal': appSettings.revealBlurredImagesOnHover }">
              <img v-if="mod.thumbnailUrl" :src="mod.thumbnailUrl" :alt="mod.title" loading="lazy" />
              <span v-else>{{ mod.title.slice(0, 1) }}</span>
              <b v-if="mod.isNsfw">NSFW</b>
            </div>
            <strong>{{ mod.title }}</strong>
            <span class="gb-author-game" :title="`${t('gameBanana.game')}: ${mod.gameName || t('gameBanana.unknownGame')}`">
              <img v-if="mod.gameIconUrl" :src="mod.gameIconUrl" alt="" loading="lazy" />
              <span>{{ t('gameBanana.game') }}: {{ mod.gameName || t('gameBanana.unknownGame') }}</span>
            </span>
            <p>{{ mod.description || t('gameBanana.noDescription') }}</p>
            <time>{{ formatDate(mod.updatedAt) }}</time>
          </button>
        </div>
        <div v-else class="gb-empty">{{ t('gameBanana.empty') }}</div>
        <footer>
          <button type="button" class="gb-button" :disabled="page <= 1 || loadingMods" @click="loadMods(page - 1)">{{ t('gameBanana.previous') }}</button>
          <span>{{ t('gameBanana.page', { current: page }) }}</span>
          <button type="button" class="gb-button" :disabled="!hasMore || loadingMods" @click="loadMods(page + 1)">{{ t('gameBanana.next') }}</button>
        </footer>
      </section>
    </main>
  </div>
</template>

<style scoped>
.gb-author-sort-field { display: grid; gap: 4px; width: 172px; flex-shrink: 0; }
.gb-author-sort-field > span { font-size: 10px; color: rgba(var(--theme-text-secondary-rgb),.64); }
.gb-author-page { position:relative; height: 100%; min-height: 0; box-sizing:border-box; display: flex; flex-direction: column; gap: var(--t-page-gap); overflow:hidden; padding: var(--t-page-padding); color: var(--t-page-text); }
.glass-panel { background:var(--t-page-panel-bg); border:var(--t-page-panel-border); box-shadow:var(--t-page-panel-shadow); backdrop-filter:var(--t-page-panel-blur); -webkit-backdrop-filter:var(--t-page-panel-blur); }
.gb-author-toolbar { display: flex; flex-wrap: wrap; align-items: end; gap: 9px; padding: 9px 12px; border-radius: 12px; }.gb-author-toolbar-title { align-self:center; flex: 1; color: rgba(var(--theme-text-primary-rgb),.9); font-size: 14px; font-weight: 700; }.gb-author-nsfw-field { display:grid; min-width:188px; gap:4px; }.gb-author-nsfw-field>span { color:rgba(var(--theme-text-secondary-rgb),.64); font-size:10px; font-weight:700; letter-spacing:.04em; text-transform:uppercase; }.gb-author-nsfw-mode { display:flex; min-height:30px; }.gb-author-nsfw-mode :deep(.el-radio-button) { flex:1 1 0; }.gb-author-nsfw-mode :deep(.el-radio-button__inner) { display:flex; align-items:center; justify-content:center; width:100%; min-height:30px; padding:0 10px; border-color:rgba(255,255,255,.13); background:rgba(255,255,255,.055); color:rgba(var(--theme-text-primary-rgb),.86); font:inherit; font-size:12px; line-height:1; box-shadow:none; transition:background .16s ease,border-color .16s ease,color .16s ease; }.gb-author-nsfw-mode :deep(.el-radio-button:first-child .el-radio-button__inner) { border-radius:7px 0 0 7px; }.gb-author-nsfw-mode :deep(.el-radio-button:last-child .el-radio-button__inner) { border-radius:0 7px 7px 0; }.gb-author-nsfw-mode :deep(.el-radio-button__inner:hover) { background:rgba(var(--theme-surface-tint-rgb),.16); border-color:rgba(var(--theme-surface-tint-rgb),.38); color:rgba(var(--theme-text-primary-rgb),.96); }.gb-author-nsfw-mode :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) { background:rgba(var(--theme-surface-tint-rgb),.20); border-color:rgba(var(--theme-surface-tint-rgb),.42); color:rgba(var(--theme-text-primary-rgb),.98); box-shadow:-1px 0 0 0 rgba(var(--theme-surface-tint-rgb),.42); }
.gb-author-show-field { display:grid; min-width:116px; gap:4px; }.gb-author-show-field>span { color:rgba(var(--theme-text-secondary-rgb),.64); font-size:10px; font-weight:700; text-transform:uppercase; }.gb-author-visibility { display:flex; min-height:30px; }.gb-author-visibility :deep(.el-radio-button) { flex:1 1 0; }.gb-author-visibility :deep(.el-radio-button__inner) { display:flex; align-items:center; justify-content:center; width:100%; min-height:30px; padding:0 10px; border-color:rgba(255,255,255,.13); background:rgba(255,255,255,.055); color:rgba(var(--theme-text-primary-rgb),.86); font-size:12px; box-shadow:none; }.gb-author-visibility :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) { background:rgba(var(--theme-surface-tint-rgb),.20); border-color:rgba(var(--theme-surface-tint-rgb),.42); color:rgba(var(--theme-text-primary-rgb),.98); }.gb-author-nsfw-mode :deep(.el-radio-button.is-disabled .el-radio-button__inner) { opacity:.35; cursor:not-allowed; }
.gb-button { min-height: 30px; padding: 0 11px; border: 1px solid rgba(255,255,255,.13); border-radius: 7px; background: rgba(255,255,255,.055); color: rgba(var(--theme-text-primary-rgb),.86); font: inherit; font-size: 12px; cursor: pointer; }.gb-button:hover:not(:disabled) { background: rgba(var(--theme-surface-tint-rgb),.16); border-color: rgba(var(--theme-surface-tint-rgb),.38); }.gb-button:disabled { opacity: .45; cursor: default; }.gb-button--primary { background: rgba(var(--theme-surface-tint-rgb),.2); border-color: rgba(var(--theme-surface-tint-rgb),.42); }.gb-history-nav { position:absolute; top:8px; right:24px; left:24px; z-index:2; display:flex; align-items:center; gap:6px; min-width:0; }.gb-history-trail { display:flex; flex:1 1 auto; align-items:center; min-width:0; height:32px; overflow-x:auto; scrollbar-width:none; white-space:nowrap; border:1px solid rgba(255,255,255,.1); border-radius:7px; background:rgba(15,18,31,.32); backdrop-filter:blur(10px); -webkit-backdrop-filter:blur(10px); }.gb-history-trail::-webkit-scrollbar { display:none; }.gb-history-entry { display:block; flex:0 1 auto; max-width:220px; overflow:hidden; padding:0 8px; border:0; background:transparent; color:rgba(var(--theme-text-secondary-rgb),.72); font:inherit; font-size:11px; line-height:30px; text-align:left; text-overflow:ellipsis; white-space:nowrap; cursor:pointer; }.gb-history-entry:hover { color:rgba(var(--theme-text-primary-rgb),.96); }.gb-history-entry.is-current { color:rgba(var(--theme-surface-tint-rgb),.96); font-weight:700; }.gb-history-separator { flex:0 0 auto; color:rgba(var(--theme-text-secondary-rgb),.38); font-size:12px; }.gb-history-button { display:grid; flex:0 0 auto; width:32px; height:32px; place-items:center; padding:0; border:1px solid rgba(255,255,255,.13); border-radius:7px; background:rgba(15,18,31,.42); color:rgba(var(--theme-text-primary-rgb),.88); cursor:pointer; backdrop-filter:blur(10px); -webkit-backdrop-filter:blur(10px); }.gb-history-button:hover:not(:disabled) { border-color:rgba(var(--theme-surface-tint-rgb),.42); background:rgba(var(--theme-surface-tint-rgb),.18); }.gb-history-button:disabled { opacity:.4; cursor:default; }.gb-history-button svg { width:17px; height:17px; fill:none; stroke:currentColor; stroke-linecap:round; stroke-linejoin:round; stroke-width:2; }
.gb-author-error { margin: -4px 1px 0; color:#ffabab; font-size:12px; }.gb-author-layout { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(245px,.85fr) minmax(0,2fr); gap:12px; overflow:hidden; }.gb-author-profile,.gb-author-mods { min-height:0; border-radius:12px; overflow:auto; }.gb-author-profile { display:flex; align-items:flex-start; gap:14px; padding:18px; }.gb-author-avatar { width:76px; height:76px; flex:0 0 auto; border-radius:50%; object-fit:cover; border:1px solid rgba(var(--theme-surface-tint-rgb),.35); }.gb-author-profile-copy { display:grid; gap:7px; min-width:0; }.gb-author-profile-copy h1,.gb-author-profile-copy p { margin:0; }.gb-author-profile-copy h1 { font-size:19px; }.gb-author-profile-copy p { color:rgba(var(--theme-text-secondary-rgb),.65); font-size:12px; line-height:1.4; }.gb-author-stats { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:5px; }.gb-author-stats span { display:grid; gap:2px; padding:6px; border-radius:6px; background:rgba(255,255,255,.045); color:rgba(var(--theme-text-secondary-rgb),.58); font-size:10px; }.gb-author-stats strong { color:rgba(var(--theme-text-primary-rgb),.88); font-size:12px; }
.gb-author-mods { display:flex; flex-direction:column; }.gb-author-mods>header,.gb-author-mods>footer { display:flex; align-items:center; justify-content:space-between; gap:8px; flex:0 0 auto; padding:10px 12px; border-bottom:1px solid rgba(255,255,255,.07); font-size:12px; }.gb-author-mods>header span,.gb-author-mods>footer span { color:rgba(var(--theme-text-secondary-rgb),.58); font-size:11px; }.gb-author-mods>footer { border-top:1px solid rgba(255,255,255,.07); border-bottom:0; justify-content:flex-end; }.gb-author-mod-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(175px,1fr)); gap:9px; padding:10px; overflow:auto; }.gb-author-card { min-width:0; padding:7px; border:1px solid rgba(255,255,255,.07); border-radius:8px; background:rgba(255,255,255,.025); color:inherit; text-align:left; cursor:pointer; }.gb-author-card:hover { border-color:rgba(var(--theme-surface-tint-rgb),.35); background:rgba(var(--theme-surface-tint-rgb),.08); }.gb-author-thumb { position:relative; display:grid; place-items:center; aspect-ratio:16/9; overflow:hidden; margin-bottom:7px; border-radius:5px; background:rgba(0,0,0,.22); color:rgba(var(--theme-surface-tint-rgb),.7); font-size:22px; font-weight:800; }.gb-author-thumb img { width:100%; height:100%; object-fit:cover; }.gb-author-thumb b { position:absolute; top:4px; right:4px; padding:2px 4px; border-radius:4px; background:rgba(138,27,58,.84); color:#fff; font-size:9px; }.gb-author-thumb.is-nsfw-blurred img { filter:blur(18px) saturate(.75); transform:scale(1.16); }.gb-author-thumb.is-nsfw-blurred::after { content:'NSFW'; position:absolute; inset:0; display:grid; place-items:center; background:rgba(8,9,14,.26); color:#fff; font-size:10px; font-weight:800; letter-spacing:.14em; text-shadow:0 1px 6px rgba(0,0,0,.9); }.gb-author-thumb.is-nsfw-blurred:hover img { filter:none; transform:scale(1.04); }.gb-author-thumb.is-nsfw-blurred:hover::after { opacity:0; }.gb-author-card>strong { display:block; overflow:hidden; color:rgba(var(--theme-text-primary-rgb),.88); font-size:12px; text-overflow:ellipsis; white-space:nowrap; }.gb-author-card p { display:-webkit-box; min-height:30px; margin:4px 0; overflow:hidden; color:rgba(var(--theme-text-secondary-rgb),.62); font-size:10px; line-height:1.45; -webkit-box-orient:vertical; -webkit-line-clamp:2; }.gb-author-card time { color:rgba(var(--theme-text-secondary-rgb),.5); font-size:10px; }.gb-empty { display:grid; flex:1; place-items:center; padding:24px; color:rgba(var(--theme-text-secondary-rgb),.58); font-size:12px; text-align:center; }
.gb-author-game { display:flex; align-items:center; gap:4px; min-width:0; margin-top:4px; color:rgba(var(--theme-surface-tint-rgb),.82); font-size:10px; line-height:1.25; }.gb-author-game img { width:14px; height:14px; flex:0 0 auto; border-radius:3px; object-fit:cover; }.gb-author-game span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
@media (max-width:720px) { .gb-author-page { overflow:hidden; padding:42px 10px 10px; }.gb-history-nav { right:10px; left:10px; }.gb-history-entry { max-width:150px; }.gb-author-toolbar { flex-wrap:wrap; }.gb-author-toolbar-title { min-width:100%; order:-1; }.gb-author-layout { display:flex; flex-direction:column; overflow:auto; }.gb-author-profile { min-height:170px; }.gb-author-mods { min-height:450px; } }
.gb-author-thumb img { transition: filter .22s ease, transform .22s ease; }
.gb-author-thumb.is-nsfw-blurred:not(.can-hover-reveal):hover img { filter: blur(18px) saturate(.75); transform: scale(1.16); }
.gb-author-thumb.is-nsfw-blurred:not(.can-hover-reveal):hover::after { opacity: 1; }
.gb-author-thumb.is-nsfw-blurred.can-hover-reveal:hover img { filter: blur(0) saturate(1); transform: scale(1.04); }
.gb-author-thumb.is-nsfw-blurred::after { transition: opacity .22s ease; }
.gb-author-thumb.is-nsfw-blurred::after { content: ''; }
.gb-author-nsfw-mode :deep(.el-radio-button__inner),.gb-author-visibility :deep(.el-radio-button__inner) { border:none!important; outline:none!important; box-shadow:none!important; }
.gb-author-nsfw-mode :deep(.el-radio-button.is-active .el-radio-button__inner),.gb-author-visibility :deep(.el-radio-button.is-active .el-radio-button__inner) { box-shadow:none!important; }
</style>
