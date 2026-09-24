<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { fetch } from '@tauri-apps/plugin-http';
import { gameBananaApiGet } from './gameBananaApi';
import { openUrl } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { appDataDir, join } from '@tauri-apps/api/path';
import { exists, mkdir, writeFile } from '@tauri-apps/plugin-fs';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import { AppStateManager } from '../../store/AppStateManager';
import { ResourceManager } from '../../store/ResourceManager';
import { ModManager } from '../../store/ModManager';
import { getGamePresetDisplayName } from '../../store/GamePreset';
import { PathHelper } from '../../helper/PathHelper';
import { gameBananaHistory, type GameBananaHistoryEntry } from './gameBananaHistory';
import { setGameBananaBlurMode, setGameBananaShowNsfw } from './gameBananaBlurSettings';

type GbImage = Record<string, unknown>;

interface GbSubmitter {
  _idRow?: number;
  _sName?: string;
  _sProfileUrl?: string;
  _sAvatarUrl?: string;
}

interface GbCategoryRef {
  _idRow?: number;
  _sName?: string;
  _sProfileUrl?: string;
  _sIconUrl?: string;
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
  _aCategory?: GbCategoryRef;
  _aRootCategory?: GbCategoryRef;
  _aSubCategory?: GbCategoryRef;
}

interface GbIndexPayload {
  _aMetadata?: { _nRecordCount?: number; _bIsComplete?: boolean };
  _aRecords?: GbRecord[];
}

interface GbCategory {
  _idRow: number;
  _sName?: string;
  _sUrl?: string;
  _sIconUrl?: string;
  _nItemCount?: number;
  _nCategoryCount?: number;
}

interface GbCategoryNode extends GbCategory {
  children: GbCategoryNode[];
  childrenLoaded: boolean;
  hasChildren: boolean | undefined;
  loadingChildren: boolean;
}

interface GbCategoryRow {
  node: GbCategoryNode;
  depth: number;
}

interface GbFile {
  _sDescription?: string;
  _idRow?: number;
  _sFile?: string;
  _nFilesize?: number;
  _tsDateAdded?: number;
  _sDownloadUrl?: string;
}

interface GbPostRecord {
  _idRow: number;
  _sText?: string;
  _tsDateAdded?: number;
  _tsDateModified?: number;
  _nReplyCount?: number;
  _nStampScore?: number;
  _aPoster?: GbSubmitter;
}

interface GbPostPayload {
  _aMetadata?: { _nRecordCount?: number; _bIsComplete?: boolean };
  _aRecords?: GbPostRecord[];
}

interface GbComment {
  id: number;
  author: string;
  authorId: number | null;
  authorUrl: string;
  avatarUrl: string;
  postedAt: number;
  updatedAt: number;
  replyCount: number;
  score: number;
  bodyHtml: string;
  depth: number;
  loadingReplies?: boolean;
  repliesLoaded?: boolean;
}

interface GbUpdateRecord {
  _idRow: number;
  _sName?: string;
  _sText?: string;
  _sVersion?: string;
  _tsDateAdded?: number;
  _sProfileUrl?: string;
}

interface GbUpdatePayload {
  _aMetadata?: { _nRecordCount?: number; _bIsComplete?: boolean };
  _aRecords: GbUpdateRecord[];
}

interface GbProfile extends GbRecord {
  _bHasUpdates?: boolean;
  _nUpdatesCount?: number;
  _sText?: string;
  _tsDateAdded?: number;
  _nDownloadCount?: number;
  _nViewCount?: number;
  _nLikeCount?: number;
  _aContentRatings?: Record<string, string>;
  _aArchivedFiles?: GbFile[];
  _aFiles?: GbFile[];
}

interface GbModCard {
  id: number;
  title: string;
  author: string;
  authorId: number | null;
  authorUrl: string;
  profileUrl: string;
  gameId: number | null;
  gameName: string;
  thumbnailUrl: string;
  description: string;
  updatedAt: number;
  categoryId: number | null;
  categoryName: string;
  categoryTrail: GbCategoryNode[];
  isNsfw: boolean;
}

interface GbModDetail extends GbModCard {
  descriptionHtml: string;
  createdAt: number;
  downloads: number;
  views: number;
  likes: number;
  screenshots: string[];
  contentRatings: string[];
  files: GbFile[];
}

interface GamebananaInstallContext {
  gameName: string;
  installDir: string;
  fallbackGameName: string;
}

interface TranslationModelOption {
  value: string;
  label: string;
}

interface InstallProgressEvent {
  gameName?: string;
  game_name?: string;
  modName?: string;
  mod_name?: string;
  stage?: string;
  current?: number;
  total?: number;
}

const { t } = useI18n();
const appSettings = AppStateManager.appSettings;
const route = useRoute();
const router = useRouter();

const GAMEBANANA_ID_BY_PRESET: Record<string, number> = {
  GIMI: 8552,
  WWMI: 20357,
  SRMI: 18366,
  ZZMI: 19567,
  HIMI: 10349,
  EFMI: 21842,
  NTEMI: 23012,
  APMI: 21772,
  IDENTITYV: 19670,
  SNOWBREAK: 19719,
  NARAKA: 17843,
  GF2: 19494,
};
const DEFAULT_GAMEBANANA_ID = GAMEBANANA_ID_BY_PRESET.GIMI;
const DEFAULT_GAMEBANANA_INSTALL_GAME = 'DefaultGame';
const PAGE_SIZE_OPTIONS = [12, 24, 36, 48];
const GAMEBANANA_ICON_CACHE_FOLDER = 'gamebanana-category-icons';
const GAMEBANANA_BLOCKLIST_STORAGE = 'gamebanana:blocklist:v1';

type GbBlockedItem = { key: string; label: string };
type GbBlocklist = { categories: GbBlockedItem[]; authors: GbBlockedItem[]; mods: GbBlockedItem[] };
type GbBlockTarget =
  | { kind: 'category'; category: GbCategoryNode }
  | { kind: 'author'; mod: GbModCard }
  | { kind: 'mod'; mod: GbModCard };

const loadGbBlocklist = (): GbBlocklist => {
  try {
    const parsed = JSON.parse(localStorage.getItem(GAMEBANANA_BLOCKLIST_STORAGE) || '{}') as Partial<GbBlocklist>;
    return {
      categories: Array.isArray(parsed.categories) ? parsed.categories : [],
      authors: Array.isArray(parsed.authors) ? parsed.authors : [],
      mods: Array.isArray(parsed.mods) ? parsed.mods : [],
    };
  } catch {
    return { categories: [], authors: [], mods: [] };
  }
};

const gameId = ref<number | null>(DEFAULT_GAMEBANANA_ID);
const gameTargetLabel = ref('');
const showGameIdInput = ref(false);
const searchQuery = ref('');
const pageSize = ref(24);
const currentPage = ref(1);
const hasMore = ref(false);
const totalRecords = ref(0);
const categoryTree = ref<GbCategoryNode[]>([]);
const expandedCategoryIds = ref(new Set<number>());
const selectedCategoryId = ref<number | null>(null);
const mods = ref<GbModCard[]>([]);
const selectedModId = ref<number | null>(null);
const detail = ref<GbModDetail | null>(null);
const gbBlocklist = reactive<GbBlocklist>(loadGbBlocklist());
const showGbBlocklist = ref(false);
const gbBlockMenu = reactive({ visible: false, x: 0, y: 0, target: null as GbBlockTarget | null });
const fallbackInstallGameName = ref('');
const comments = ref<GbComment[]>([]);
const commentsPage = ref(0);
const commentsHasMore = ref(false);
const loadingCategories = ref(false);
const loadingMods = ref(false);
const loadingDetail = ref(false);
const detailLoadFailed = ref(false);
const commentsError = ref('');
const updates = ref<(GbUpdateRecord & { bodyHtml: string })[]>([]);
const updatesVisible = ref(false);
const loadingUpdates = ref(false);
const updatesError = ref('');
const updatesPage = ref(0);
const updatesHasMore = ref(false);
const loadingComments = ref(false);
const loadingMoreComments = ref(false);
const installingFileId = ref<number | null>(null);
const installGroup = ref('');
type InstallGroupMode = 'automatic' | 'manual' | 'matching-icon';
const installGroupMode = ref<InstallGroupMode>('automatic');
const matchingIconGroups = ref<Array<{ id: string; iconPath: string }>>([]);
const loadingMatchingIconGroups = ref(false);
const installStatus = ref('');
const installPhase = ref<'idle' | 'downloading' | 'installing'>('idle');
const downloadProgress = ref(0);
const installProgress = ref(0);
const downloadedFileIds = ref(new Set<number>());
const showTranslationSettings = ref(false);
const translationStatus = ref('');
const loadingTranslationModels = ref(false);
const translationModelsStatus = ref('');
const translationModelOptions = ref<TranslationModelOption[]>([]);
const errorMessage = ref('');
const revealedImages = ref(new Set<string>());

let modsRequestId = 0;
let detailRequestId = 0;
let commentsRequestId = 0;
let updatesRequestId = 0;
let categoriesRequestId = 0;
let downloadedStateRequestId = 0;
let syncingGameTarget = false;
let gameTargetSyncRevision = 0;
let resolvedRequestedModTargetKey = '';
let hoveredTranslateElement: HTMLElement | null = null;
let hoveredTranslateText = '';
let activeGamebananaInstall: { gameName: string; modName: string } | null = null;
let unlistenDownloadProgress: UnlistenFn | null = null;
let unlistenInstallProgress: UnlistenFn | null = null;
const translationNodes = new Set<HTMLElement>();
const translationBlocksByAnchor = new Map<HTMLElement, HTMLElement>();
const translationAnchorsByBlock = new Map<HTMLElement, HTMLElement>();
const translationCache = new Map<string, string>();
const translationRequestCount = ref(0);
const translationFontStyleOptions = ['regular', 'italic', 'bold', 'bold-italic'] as const;
const translationFailureModeOptions = ['retry', 'message', 'silent'] as const;
const translationFontWarning = ref<'missing' | 'limited' | ''>('');
const translationColorWarning = ref(false);
let translationFontCheckRevision = 0;

const quotedFontFamily = (family: string): string => `"${family.replace(/["\\]/g, '\\$&')}"`;
const measureFontSample = (font: string, sample: string): number => {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) return 0;
  context.font = `32px ${font}`;
  return context.measureText(sample).width;
};

const checkTranslationFont = async () => {
  const revision = ++translationFontCheckRevision;
  const family = appSettings.gamebananaTranslationFontFamily.trim();
  if (!family || typeof document === 'undefined') {
    translationFontWarning.value = '';
    return;
  }
  await document.fonts.ready;
  if (revision !== translationFontCheckRevision) return;
  const font = quotedFontFamily(family);
  const sample = '中文測試日本語한글';
  await document.fonts.load(`16px ${font}`).catch(() => []);
  const asciiProbe = 'mmmmmmmmmmWWWWWWWWWW';
  const fallbackFamilies = ['monospace', 'serif', 'sans-serif'];
  const exists = fallbackFamilies.some((fallback) => Math.abs(
    measureFontSample(`${font}, ${fallback}`, asciiProbe) - measureFontSample(fallback, asciiProbe),
  ) >= 0.5);
  if (revision !== translationFontCheckRevision) return;
  if (!exists) {
    translationFontWarning.value = 'missing';
    return;
  }
  const fallback = 'monospace';
  const specifiedWidth = measureFontSample(`${font}, ${fallback}`, sample);
  const fallbackWidth = measureFontSample(fallback, sample);
  translationFontWarning.value = Math.abs(specifiedWidth - fallbackWidth) < 0.5 ? 'limited' : '';
};

watch(
  [() => appSettings.gamebananaTranslationFontFamily, showTranslationSettings],
  ([, isOpen]) => { if (isOpen) void checkTranslationFont(); },
  { immediate: true },
);

const normalizedTranslationColor = (value: string): string | null => {
  const normalized = value.trim();
  if (!normalized) return '#FFFFFFB3';
  if (/^#[\da-f]{3,4}$/i.test(normalized) || /^#[\da-f]{6}([\da-f]{2})?$/i.test(normalized)) return normalized;
  return null;
};

watch(() => appSettings.gamebananaTranslationColor, (value) => {
  translationColorWarning.value = normalizedTranslationColor(value) === null;
}, { immediate: true });

const applyTranslationTextStyle = (element: HTMLElement) => {
  element.style.fontFamily = appSettings.gamebananaTranslationFontFamily.trim() || 'inherit';
  element.style.fontSize = appSettings.gamebananaTranslationFontSize.trim() || 'inherit';
  element.style.color = normalizedTranslationColor(appSettings.gamebananaTranslationColor) || '#FFFFFFB3';
  element.style.fontStyle = appSettings.gamebananaTranslationFontStyle.includes('italic') ? 'italic' : 'normal';
  element.style.fontWeight = appSettings.gamebananaTranslationFontStyle.includes('bold') ? '700' : '400';
};

const allCategories = computed(() => {
  const flattened: GbCategoryNode[] = [];
  const visit = (nodes: GbCategoryNode[]) => {
    for (const node of nodes) {
      flattened.push(node);
      visit(node.children);
    }
  };
  visit(categoryTree.value);
  return flattened;
});
const categoryBlockKey = (categoryId: number) => `${gameId.value ?? 'unknown'}:${categoryId}`;
const authorBlockKey = (mod: GbModCard) => mod.authorId !== null
  ? `id:${mod.authorId}`
  : `name:${mod.author.trim().toLocaleLowerCase()}`;
const blockedCategoryKeys = computed(() => new Set(gbBlocklist.categories.map((item) => item.key)));
const blockedAuthorKeys = computed(() => new Set(gbBlocklist.authors.map((item) => item.key)));
const blockedModKeys = computed(() => new Set(gbBlocklist.mods.map((item) => item.key)));
const isCategoryBlocked = (categoryId: number) => blockedCategoryKeys.value.has(categoryBlockKey(categoryId));
const isModBlocked = (mod: GbModCard) => blockedModKeys.value.has(String(mod.id))
  || blockedAuthorKeys.value.has(authorBlockKey(mod))
  || (mod.categoryId !== null && isCategoryBlocked(mod.categoryId))
  || mod.categoryTrail.some((category) => isCategoryBlocked(category._idRow));

const categoryRows = computed<GbCategoryRow[]>(() => {
  const rows: GbCategoryRow[] = [];
  const visit = (nodes: GbCategoryNode[], depth: number) => {
    for (const node of nodes) {
      if (isCategoryBlocked(node._idRow)) continue;
      rows.push({ node, depth });
      if (expandedCategoryIds.value.has(node._idRow)) visit(node.children, depth + 1);
    }
  };
  visit(categoryTree.value, 0);
  return rows;
});
const selectedCategoryName = computed(() =>
  selectedCategoryId.value === null
    ? t('gameBanana.allCategories')
    : allCategories.value.find((item) => item._idRow === selectedCategoryId.value)?._sName || t('gameBanana.allCategories'),
);

const visibleMods = computed(() => mods.value.filter((item) => !isModBlocked(item) && (appSettings.gamebananaShowNsfw || !item.isNsfw)));
const visibleCountText = computed(() => `${visibleMods.value.length}${!appSettings.gamebananaShowNsfw ? ` / ${mods.value.length}` : ''}`);
const gameBananaBlurMode = computed({ get: () => appSettings.gamebananaBlurMode, set: (mode) => setGameBananaBlurMode(appSettings, mode) });
const currentGameName = computed(() => appSettings.CurrentGameName?.trim() || '');

const isImageRevealed = (src: string): boolean => revealedImages.value.has(src);
const shouldBlurImage = (isNsfw: boolean): boolean =>
  appSettings.gamebananaBlurMode === 'all' || (appSettings.gamebananaBlurMode === 'nsfw' && isNsfw);
const isImageBlurred = (src: string, isNsfw: boolean): boolean =>
  shouldBlurImage(isNsfw) && !!src && !revealedImages.value.has(src);
const toggleImageBlur = (src: string) => {
  if (!src) return;
  const next = new Set(revealedImages.value);
  if (next.has(src)) next.delete(src);
  else next.add(src);
  revealedImages.value = next;
};
const onModThumbClick = (event: MouseEvent, mod: GbModCard) => {
  if (!shouldBlurImage(mod.isNsfw) || !mod.thumbnailUrl) return;
  event.stopPropagation();
  toggleImageBlur(mod.thumbnailUrl);
};
const onScreenshotClick = (image: string) => {
  if (shouldBlurImage(detail.value?.isNsfw === true)) toggleImageBlur(image);
  else void openExternal(image);
};
const toggleRichTextImageBlur = (image: HTMLImageElement) => {
  const src = image.currentSrc || image.src || '';
  if (!src) return;
  toggleImageBlur(src);
  image.classList.toggle('is-blurred', shouldBlurImage(detail.value?.isNsfw === true) && !revealedImages.value.has(src));
  image.classList.toggle('can-hover-reveal', appSettings.revealBlurredImagesOnHover);
};
const syncRichTextImageBlur = () => {
  document.querySelectorAll<HTMLImageElement>('.gb-rich-text img, .gb-comment-body img').forEach((image) => {
    const src = image.currentSrc || image.src || '';
    image.classList.toggle(
      'is-blurred',
      shouldBlurImage(detail.value?.isNsfw === true) && !!src && !revealedImages.value.has(src),
    );
    image.classList.toggle('can-hover-reveal', appSettings.revealBlurredImagesOnHover);
  });
};

const queryNumber = (value: unknown): number | null => {
  const parsed = Number(Array.isArray(value) ? value[0] : value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
};
const requestedModId = computed(() => queryNumber(route.query.mod));
const requestedGameId = computed(() => queryNumber(route.query.game));
const requestedGameName = computed(() => {
  const value = Array.isArray(route.query.gameName) ? route.query.gameName[0] : route.query.gameName;
  return typeof value === 'string' ? value.trim() : '';
});
const requestedModTargetKey = (): string => requestedModId.value === null
  ? ''
  : `${requestedModId.value}:${requestedGameId.value || ''}:${requestedGameName.value}`;
const gameUrl = computed(() => gameId.value && gameId.value > 0
  ? `https://gamebanana.com/games/${gameId.value}`
  : 'https://gamebanana.com/games');
const isInstalling = computed(() => installingFileId.value !== null);

const asString = (value: unknown): string => typeof value === 'string' ? value.trim() : '';
const asNumber = (value: unknown): number => Number.isFinite(Number(value)) ? Number(value) : 0;

const normalizedGameName = (value: string): string => value
  .trim()
  .toLocaleLowerCase()
  .replace(/[^\p{L}\p{N}]+/gu, '');

const gamebananaPresetForGameId = (gameId: number | null): string | null => {
  if (gameId === null) return null;
  return Object.entries(GAMEBANANA_ID_BY_PRESET)
    .find(([, knownGameId]) => knownGameId === gameId)?.[0] || null;
};

const resolveGamebananaEnvironment = async (gameId: number | null, gameName: string) => {
  const preset = gamebananaPresetForGameId(gameId);
  const normalizedName = normalizedGameName(gameName);
  const directMatch = AppStateManager.gamesList.find((game) => {
    const normalizedLocalName = normalizedGameName(game.name);
    return (normalizedName && normalizedLocalName === normalizedName)
      || (preset !== null && game.name.trim().toUpperCase() === preset);
  });
  if (directMatch) return directMatch;
  if (!preset) return null;

  for (const game of AppStateManager.gamesList) {
    try {
      const config = await ResourceManager.loadGameConfig(game.name);
      if (asString(config?.gamePreset).toUpperCase() === preset) return game;
    } catch (error) {
      console.warn(`Unable to read the ${game.name} configuration while resolving a GameBanana game:`, error);
    }
  }
  return null;
};

const resolveGamebananaInstallContext = async (
  mod: Pick<GbModCard, 'gameId' | 'gameName'>,
  switchEnvironment = false,
): Promise<GamebananaInstallContext> => {
  const matchedGame = await resolveGamebananaEnvironment(mod.gameId, mod.gameName);
  if (matchedGame) {
    if (switchEnvironment && currentGameName.value !== matchedGame.name) {
      await AppStateManager.selectGame(matchedGame);
    }
    const installDir = await PathHelper.GetGame3DmigotoFolderPath(matchedGame.name);
    if (installDir) {
      return { gameName: matchedGame.name, installDir, fallbackGameName: '' };
    }
  }

  const installDir = await PathHelper.GetGame3DmigotoFolderPath(DEFAULT_GAMEBANANA_INSTALL_GAME);
  if (!installDir) throw new Error(t('modManager.messages.installDirNotConfigured'));
  return {
    gameName: DEFAULT_GAMEBANANA_INSTALL_GAME,
    installDir,
    fallbackGameName: mod.gameName || (mod.gameId ? `Game ${mod.gameId}` : t('gameBanana.unknownGame')),
  };
};

const toPlainText = (value: unknown): string => {
  const html = asString(value);
  if (!html) return '';
  return html
    .replace(/<br\s*\/?\s*>/gi, '\n')
    .replace(/<\/?p[^>]*>/gi, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&#39;/gi, "'")
    .replace(/&quot;/gi, '"')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
};

const isSafeHttpUrl = (value: string): boolean => {
  try {
    const url = new URL(value);
    return url.protocol === 'https:' || url.protocol === 'http:';
  } catch {
    return false;
  }
};

const richTextBlockTags = new Set([
  'P', 'BLOCKQUOTE', 'PRE', 'UL', 'OL', 'LI',
  'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HR',
  'TABLE', 'THEAD', 'TBODY', 'TFOOT', 'TR', 'TH', 'TD',
]);

const wrapLooseRichTextNodes = (root: Element, document: Document) => {
  const normalized: Node[] = [];
  let inlineNodes: Node[] = [];
  let pendingBreaks: HTMLBRElement[] = [];

  const flushInlineNodes = () => {
    const hasContent = inlineNodes.some((node) =>
      Boolean(node.textContent?.trim()) || (node instanceof Element && node.tagName !== 'BR'),
    );
    if (hasContent) {
      const paragraph = document.createElement('p');
      paragraph.append(...inlineNodes);
      normalized.push(paragraph);
    }
    inlineNodes = [];
    pendingBreaks = [];
  };

  for (const node of Array.from(root.childNodes)) {
    if (node instanceof HTMLBRElement) {
      pendingBreaks.push(node);
      continue;
    }

    const isBlock = node instanceof Element && richTextBlockTags.has(node.tagName);
    if (isBlock) {
      flushInlineNodes();
      normalized.push(node);
      continue;
    }

    if (pendingBreaks.length >= 2) {
      flushInlineNodes();
    } else if (pendingBreaks.length === 1 && inlineNodes.length) {
      inlineNodes.push(pendingBreaks[0]);
      pendingBreaks = [];
    }
    inlineNodes.push(node);
  }

  flushInlineNodes();
  root.replaceChildren(...normalized);
};

const sanitizeRichText = (value: unknown): string => {
  const source = asString(value);
  if (!source || typeof DOMParser === 'undefined') return '';

  const allowedTags = new Set([
    'P', 'BR', 'B', 'STRONG', 'I', 'EM', 'U', 'S', 'SMALL', 'CODE', 'PRE', 'BLOCKQUOTE',
    'UL', 'OL', 'LI', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HR', 'A', 'SPAN', 'IMG',
    'TABLE', 'THEAD', 'TBODY', 'TFOOT', 'TR', 'TH', 'TD',
  ]);
  const document = new DOMParser().parseFromString(`<article>${source}</article>`, 'text/html');
  const root = document.body.firstElementChild;
  if (!root) return '';

  for (const element of Array.from(root.querySelectorAll('*'))) {
    if (!allowedTags.has(element.tagName)) {
      element.replaceWith(...Array.from(element.childNodes));
      continue;
    }

    const attributes = Array.from(element.attributes);
    for (const attribute of attributes) element.removeAttribute(attribute.name);

    if (element instanceof HTMLAnchorElement) {
      const href = asString(attributes.find((attribute) => attribute.name.toLowerCase() === 'href')?.value);
      if (isSafeHttpUrl(href)) {
        element.href = href;
        element.target = '_blank';
        element.rel = 'noopener noreferrer';
      } else {
        element.replaceWith(...Array.from(element.childNodes));
      }
      continue;
    }

    if (element instanceof HTMLImageElement) {
      const src = asString(attributes.find((attribute) => attribute.name.toLowerCase() === 'src')?.value);
      const alt = asString(attributes.find((attribute) => attribute.name.toLowerCase() === 'alt')?.value);
      if (!isSafeHttpUrl(src)) {
        element.remove();
      } else {
        element.src = src;
        element.alt = alt;
        element.loading = 'lazy';
      }
    }
  }

  // GameBanana descriptions often mix top-level text/BR/CODE nodes with
  // headings or rules. Normalize every loose inline run, not only documents
  // containing no block elements, so each visible paragraph can be hovered and
  // used as an independent translation anchor.
  wrapLooseRichTextNodes(root, document);

  return root.innerHTML.trim();
};

const sanitizeTranslatedRichText = (value: string): string => sanitizeRichText(
  value
    .trim()
    .replace(/^```(?:html)?\s*/i, '')
    .replace(/\s*```$/i, ''),
);

const translatedParagraphHtml = (value: string): string => {
  const sanitized = sanitizeTranslatedRichText(value);
  const parsed = new DOMParser().parseFromString(`<article>${sanitized}</article>`, 'text/html');
  const root = parsed.body.firstElementChild;
  return root?.children.length === 1 && root.firstElementChild?.tagName === 'P'
    ? root.firstElementChild.innerHTML
    : sanitized;
};

const restoreGamebananaScreenshotVariant = (value: string): string => {
  const normalized = value.trim();
  if (!normalized) return '';

  // GameBanana's _sUrl530/_sUrl220 fields commonly point to files named
  // "530-90_original-file.jpg".  Those are thumbnail variants, whereas the
  // matching original lives at the same path without the numeric prefix.
  // Keep the old manager's two-part numeric guard so ordinary file names are
  // never altered accidentally.
  const restorePath = (path: string) => path.replace(
    /(^|\/)(?:\d+(?:-\d+)+)_([^/]+)$/,
    '$1$2',
  );
  try {
    const parsed = new URL(normalized);
    parsed.pathname = restorePath(parsed.pathname);
    return parsed.toString();
  } catch {
    return restorePath(normalized);
  }
};

const imageUrl = (image: GbImage, preferLarge = false): string => {
  const preferredUrlKeys = preferLarge
    ? ['_sUrl', '_sUrl800', '_sUrl530', '_sUrl220', '_sUrl100']
    : ['_sUrl530', '_sUrl220', '_sUrl100', '_sUrl'];
  for (const key of preferredUrlKeys) {
    const direct = asString(image[key]);
    if (direct) return preferLarge ? restoreGamebananaScreenshotVariant(direct) : direct;
  }

  const preferredFileKeys = preferLarge
    ? ['_sFile', '_sFile800', '_sFile530', '_sFile220', '_sFile100']
    : ['_sFile530', '_sFile220', '_sFile100', '_sFile'];
  const baseUrl = asString(image._sBaseUrl) || 'https://images.gamebanana.com/img/ss/mods';
  for (const key of preferredFileKeys) {
    const file = asString(image[key]);
    if (file) {
      const url = `${baseUrl.replace(/\/$/, '')}/${file.replace(/^\//, '')}`;
      return preferLarge ? restoreGamebananaScreenshotVariant(url) : url;
    }
  }
  return '';
};

const mediaUrls = (record: GbRecord, preferLarge = false): string[] => {
  const seen = new Set<string>();
  return (record._aPreviewMedia?._aImages || [])
    .map((item) => imageUrl(item, preferLarge))
    .filter((url) => {
      if (!url || seen.has(url)) return false;
      seen.add(url);
      return true;
    });
};

const categoryNodeFromReference = (reference: GbCategoryRef | undefined): GbCategoryNode | null => {
  if (!reference) return null;
  const id = asNumber(reference._idRow) || categoryIdFromUrl(asString(reference._sProfileUrl)) || 0;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
  return {
    _idRow: id,
    _sName: asString(reference._sName),
    _sUrl: asString(reference._sProfileUrl),
    _sIconUrl: asString(reference._sIconUrl),
    children: [],
    childrenLoaded: true,
    hasChildren: false,
    loadingChildren: false,
  };
};

const categoryTrailFromRecord = (record: GbRecord): GbCategoryNode[] => {
  const seen = new Set<number>();
  return [record._aRootCategory, record._aCategory, record._aSubCategory]
    .map(categoryNodeFromReference)
    .filter((item): item is GbCategoryNode => item !== null && !seen.has(item._idRow) && Boolean(seen.add(item._idRow)));
};

const recordToCard = (record: GbRecord): GbModCard => {
  const categoryTrail = categoryTrailFromRecord(record);
  const category = categoryTrail.at(-1);
  const isNsfw = record._bIsNsfw === true || record._bHasContentRatings === true;
  return {
    id: record._idRow,
    title: asString(record._sName) || `Mod #${record._idRow}`,
    author: asString(record._aSubmitter?._sName) || t('gameBanana.unknownAuthor'),
    authorId: record._aSubmitter?._idRow ?? null,
    authorUrl: asString(record._aSubmitter?._sProfileUrl),
    profileUrl: asString(record._sProfileUrl) || `https://gamebanana.com/mods/${record._idRow}`,
    gameId: record._aGame?._idRow ?? null,
    gameName: asString(record._aGame?._sName),
    thumbnailUrl: mediaUrls(record)[0] || '',
    description: toPlainText(record._sDescription),
    updatedAt: asNumber(record._tsDateUpdated || record._tsDateModified),
    categoryId: category?._idRow ?? null,
    categoryName: asString(category?._sName),
    categoryTrail,
    isNsfw,
  };
};

const mergedCategoryTrail = (profileTrail: GbCategoryNode[], sourceTrail: GbCategoryNode[]): GbCategoryNode[] => {
  if (!profileTrail.length) return sourceTrail;
  if (!sourceTrail.length) return profileTrail;
  const profileLeaf = profileTrail.at(-1)?._idRow;
  const sourceLeaf = sourceTrail.at(-1)?._idRow;
  return profileLeaf === sourceLeaf && sourceTrail.length > profileTrail.length ? sourceTrail : profileTrail;
};

const profileToDetail = (profile: GbProfile, sourceCard?: GbModCard): GbModDetail => {
  const base = recordToCard(profile);
  const categoryTrail = mergedCategoryTrail(base.categoryTrail, sourceCard?.categoryTrail || []);
  const category = categoryTrail.at(-1);
  const files = profile._aArchivedFiles || profile._aFiles || [];
  return {
    ...base,
    isNsfw: base.isNsfw
      || (profile._aContentRatings ? Object.keys(profile._aContentRatings).length > 0 : false)
      || sourceCard?.isNsfw === true,
    gameId: base.gameId ?? sourceCard?.gameId ?? null,
    gameName: base.gameName || sourceCard?.gameName || '',
    categoryTrail,
    categoryId: category?._idRow ?? base.categoryId,
    categoryName: asString(category?._sName) || base.categoryName,
    description: toPlainText(profile._sText || profile._sDescription),
    descriptionHtml: sanitizeRichText(profile._sText || profile._sDescription),
    createdAt: asNumber(profile._tsDateAdded),
    downloads: asNumber(profile._nDownloadCount),
    views: asNumber(profile._nViewCount),
    likes: asNumber(profile._nLikeCount),
    screenshots: mediaUrls(profile, true),
    contentRatings: Object.values(profile._aContentRatings || {}).map(asString).filter(Boolean),
    files,
  };
};

const postToComment = (post: GbPostRecord, depth = 0): GbComment => ({
  id: post._idRow,
  author: asString(post._aPoster?._sName) || t('gameBanana.unknownAuthor'),
  authorId: post._aPoster?._idRow ?? null,
  authorUrl: asString(post._aPoster?._sProfileUrl),
  avatarUrl: asString(post._aPoster?._sAvatarUrl),
  postedAt: asNumber(post._tsDateAdded),
  updatedAt: asNumber(post._tsDateModified),
  replyCount: asNumber(post._nReplyCount),
  score: asNumber(post._nStampScore),
  bodyHtml: sanitizeRichText(post._sText),
  depth,
});

const formatDate = (timestamp: number): string => {
  if (!timestamp) return '—';
  return new Date(timestamp * 1000).toLocaleDateString();
};

const formatNumber = (value: number): string => new Intl.NumberFormat().format(value || 0);

const formatFileSize = (bytes: number | undefined): string => {
  const size = Number(bytes || 0);
  if (!size) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  const index = Math.min(units.length - 1, Math.floor(Math.log(size) / Math.log(1024)));
  return `${(size / (1024 ** index)).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
};

let pageDisposed = false;
const apiGet = <T>(path: string, params: Record<string, string> = {}, isCurrent: () => boolean = () => true): Promise<T> =>
  gameBananaApiGet<T>(path, params, () => !pageDisposed && isCurrent());

const categoryIdFromUrl = (url: string): number | null => {
  const match = url.match(/\/cats\/(\d+)(?:[/?#]|$)/i);
  const id = match ? Number(match[1]) : NaN;
  return Number.isSafeInteger(id) && id > 0 ? id : null;
};

const categoryNodeFromApi = (record: Record<string, unknown>): GbCategoryNode | null => {
  const id = asNumber(record._idRow) || categoryIdFromUrl(asString(record._sUrl)) || 0;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
  return {
    _idRow: id,
    _sName: asString(record._sName),
    _sUrl: asString(record._sUrl),
    _sIconUrl: asString(record._sIconUrl),
    _nItemCount: record._nItemCount === undefined ? undefined : asNumber(record._nItemCount),
    _nCategoryCount: record._nCategoryCount === undefined ? undefined : asNumber(record._nCategoryCount),
    children: [],
    childrenLoaded: record._nCategoryCount === 0,
    hasChildren: record._nCategoryCount === undefined ? undefined : asNumber(record._nCategoryCount) > 0,
    loadingChildren: false,
  };
};

const fetchCategoryChildren = async (node: GbCategoryNode): Promise<GbCategoryNode[]> => {
  if (node._nCategoryCount === 0) return [];
  const records = await apiGet<Array<Record<string, unknown>>>(`/ModCategory/${node._idRow}/SubCategories`);
  return (Array.isArray(records) ? records : [])
    .map(categoryNodeFromApi)
    .filter((item): item is GbCategoryNode => item !== null);
};

const loadCategoryChildren = async (node: GbCategoryNode) => {
  if (node.childrenLoaded || node.loadingChildren) return;
  node.loadingChildren = true;
  try {
    const children = await fetchCategoryChildren(node);
    node.children = children.filter((child) => child._idRow !== node._idRow);
    node.childrenLoaded = true;
    node.hasChildren = node.children.length > 0;
  } catch (error) {
    errorMessage.value = t('gameBanana.errors.loadCategories', { error: String(error) });
  } finally {
    node.loadingChildren = false;
  }
};

const revealCategoryTrail = async (trail: GbCategoryNode[]) => {
  for (const category of trail) {
    const node = allCategories.value.find((item) => item._idRow === category._idRow);
    if (!node) continue;
    expandedCategoryIds.value = new Set([...expandedCategoryIds.value, node._idRow]);
    await loadCategoryChildren(node);
  }
};

const loadCategories = async () => {
  if (!gameId.value || gameId.value <= 0) return;
  const requestId = ++categoriesRequestId;
  loadingCategories.value = true;
  try {
    const items = await apiGet<Array<Record<string, unknown>>>('/Mod/Categories', {
      _idGameRow: String(gameId.value),
      _sSort: 'a_to_z',
      _bShowEmpty: 'true',
    });
    if (requestId !== categoriesRequestId) return;
    const roots = (Array.isArray(items) ? items : [])
      .map(categoryNodeFromApi)
      .filter((item): item is GbCategoryNode => item !== null);
    if (requestId !== categoriesRequestId) return;
    categoryTree.value = roots;
    expandedCategoryIds.value = new Set();
    if (selectedCategoryId.value !== null && !allCategories.value.some((item) => item._idRow === selectedCategoryId.value)) {
      selectedCategoryId.value = null;
    }
  } catch (error) {
    if (requestId === categoriesRequestId) errorMessage.value = t('gameBanana.errors.loadCategories', { error: String(error) });
  } finally {
    if (requestId === categoriesRequestId) loadingCategories.value = false;
  }
};

const loadMods = async (requestedPage = 1) => {
  if (!gameId.value || gameId.value <= 0) return;
  const requestId = ++modsRequestId;
  loadingMods.value = true;
  errorMessage.value = '';
  currentPage.value = Math.max(1, requestedPage);
  try {
    const params: Record<string, string> = {
      _nPage: String(currentPage.value),
      _nPerpage: String(pageSize.value),
      _sOrderBy: '_tsDateUpdated,DESC',
      '_aFilters[Generic_Game]': String(gameId.value),
    };
    if (selectedCategoryId.value !== null) params['_aFilters[Generic_Category]'] = String(selectedCategoryId.value);
    if (searchQuery.value.trim()) params['_aFilters[Generic_Name]'] = `contains,${searchQuery.value.trim()}`;

    const payload = await apiGet<GbIndexPayload>('/Mod/Index', params);
    if (requestId !== modsRequestId) return;

    mods.value = (payload._aRecords || []).map(recordToCard);
    totalRecords.value = asNumber(payload._aMetadata?._nRecordCount);
    hasMore.value = !payload._aMetadata?._bIsComplete && mods.value.length >= pageSize.value;
    selectedModId.value = null;
    detail.value = null;
    const targetKey = requestedModTargetKey();
    const shouldLocateRequestedMod = Boolean(targetKey && targetKey !== resolvedRequestedModTargetKey);
    const requestedMod = !shouldLocateRequestedMod || requestedModId.value === null
      ? null
      : visibleMods.value.find((item) => item.id === requestedModId.value) || null;
    if (requestedMod) {
      resolvedRequestedModTargetKey = targetKey;
      void selectMod(requestedMod);
    } else if (shouldLocateRequestedMod && requestedModId.value !== null) void loadRequestedMod(requestedModId.value);
    else {
      const initialMod = visibleMods.value[0] || mods.value[0];
      if (initialMod) void selectMod(initialMod);
    }
  } catch (error) {
    if (requestId === modsRequestId) {
      mods.value = [];
      detail.value = null;
      errorMessage.value = t('gameBanana.errors.loadMods', { error: String(error) });
    }
  } finally {
    if (requestId === modsRequestId) loadingMods.value = false;
  }
};

const loadUpdates = async (modId: number, page = 1, append = false) => {
  const requestId = ++updatesRequestId;
  loadingUpdates.value = true;
  updatesError.value = '';
  if (!append) {
    updates.value = [];
    updatesPage.value = 0;
    updatesHasMore.value = false;
  }
  try {
    const payload = await apiGet<GbUpdatePayload>(`/Mod/${modId}/Updates`, {
      _nPage: String(page), _nPerpage: '10',
    }, () => requestId === updatesRequestId && selectedModId.value === modId);
    if (requestId !== updatesRequestId || selectedModId.value !== modId) return;
    const next = payload._aRecords.map(update => ({ ...update, bodyHtml: sanitizeRichText(update._sText) }));
    const existing = new Set(append ? updates.value.map(update => update._idRow) : []);
    updates.value = [...(append ? updates.value : []), ...next.filter(update => !existing.has(update._idRow))];
    updatesPage.value = page;
    const total = asNumber(payload._aMetadata?._nRecordCount);
    updatesHasMore.value = next.length > 0 && (total > 0
      ? page * 10 < total
      : payload._aMetadata?._bIsComplete === false);
  } catch (error) {
    if (requestId === updatesRequestId && selectedModId.value === modId) {
      updatesError.value = t('gameBanana.loadUpdatesError', { error: String(error) });
    }
  } finally {
    if (requestId === updatesRequestId) loadingUpdates.value = false;
  }
};

const loadComments = async (modId: number, requestedPage = 1, append = false) => {
  const requestId = ++commentsRequestId;
  commentsError.value = '';
  if (append) loadingMoreComments.value = true;
  else {
    loadingComments.value = true;
    comments.value = [];
    commentsPage.value = 0;
    commentsHasMore.value = false;
    loadingMoreComments.value = false;
  }

  try {
    const payload = await apiGet<GbPostPayload>(`/Mod/${modId}/Posts`, {
      _nPage: String(requestedPage),
      _nPerpage: '15',
    }, () => requestId === commentsRequestId && selectedModId.value === modId);
    if (requestId !== commentsRequestId || selectedModId.value !== modId) return;
    const next = (payload._aRecords || []).map((post) => postToComment(post));
    comments.value = append ? [...comments.value, ...next] : next;
    commentsPage.value = requestedPage;
    const total = asNumber(payload._aMetadata?._nRecordCount);
    commentsHasMore.value = total > 0
      ? requestedPage * 15 < total
      : !payload._aMetadata?._bIsComplete;
  } catch (error) {
    if (requestId === commentsRequestId && selectedModId.value === modId) {
      commentsError.value = t('gameBanana.errors.loadComments', { error: String(error) });
    }
  } finally {
    if (requestId === commentsRequestId) {
      loadingComments.value = false;
      loadingMoreComments.value = false;
    }
  }
};

const loadReplies = async (comment: GbComment) => {
  if (comment.loadingReplies || comment.repliesLoaded || comment.replyCount <= 0) return;
  comment.loadingReplies = true;
  const modId = selectedModId.value;
  const requestId = commentsRequestId;
  try {
    const payload = await apiGet<GbPostPayload>(`/Post/${comment.id}/Posts`, {
      _nPage: '1',
      _nPerpage: '50',
    }, () => requestId === commentsRequestId && selectedModId.value === modId);
    if (requestId !== commentsRequestId || selectedModId.value !== modId) return;
    const replies = (payload._aRecords || []).map((post) => postToComment(post, comment.depth + 1));
    const index = comments.value.findIndex((item) => item.id === comment.id);
    if (index >= 0) comments.value.splice(index + 1, 0, ...replies);
    comment.repliesLoaded = true;
  } catch (error) {
    if (requestId === commentsRequestId && selectedModId.value === modId) {
      commentsError.value = t('gameBanana.errors.loadReplies', { error: String(error) });
    }
  } finally {
    comment.loadingReplies = false;
  }
};

const selectMod = async (mod: GbModCard) => {
  ++updatesRequestId;
  updates.value = [];
  updatesVisible.value = false;
  loadingUpdates.value = false;
  updatesError.value = '';
  updatesPage.value = 0;
  updatesHasMore.value = false;
  detailLoadFailed.value = false;
  errorMessage.value = '';
  commentsError.value = '';
  ++commentsRequestId;
  loadingComments.value = false;
  loadingMoreComments.value = false;
  clearTranslations();
  clearHoveredText();
  downloadedFileIds.value = new Set();
  fallbackInstallGameName.value = '';
  selectedModId.value = mod.id;
  detail.value = null;
  comments.value = [];
  commentsPage.value = 0;
  commentsHasMore.value = false;
  const requestId = ++detailRequestId;
  loadingDetail.value = true;
  try {
    const profile = await apiGet<GbProfile>(`/Mod/${mod.id}/ProfilePage`, {}, () => requestId === detailRequestId && selectedModId.value === mod.id);
    if (requestId !== detailRequestId) return;
    detail.value = profileToDetail(profile, mod);
    updatesVisible.value = profile._bHasUpdates === true || asNumber(profile._nUpdatesCount) > 0;
    if (updatesVisible.value) void loadUpdates(mod.id);
    void loadComments(mod.id);
    void refreshDownloadedFileState(detail.value);
  } catch (error) {
    if (requestId === detailRequestId && selectedModId.value === mod.id) {
      detailLoadFailed.value = true;
      detail.value = { ...mod, descriptionHtml: '', createdAt: 0, downloads: 0, views: 0, likes: 0, screenshots: [], contentRatings: [], files: [] };
      errorMessage.value = t('gameBanana.errors.loadDetail', { error: String(error) });
      void refreshDownloadedFileState(detail.value);
    }
  } finally {
    if (requestId === detailRequestId) loadingDetail.value = false;
  }
};

async function loadRequestedMod(modId: number) {
  try {
    const profile = await apiGet<GbProfile>(`/Mod/${modId}/ProfilePage`, {}, () => requestedModId.value === modId);
    if (requestedModId.value !== modId) return;
    const targetKey = requestedModTargetKey();
    if (!targetKey || targetKey === resolvedRequestedModTargetKey) return;
    const profileCard = recordToCard(profile);
    const card: GbModCard = {
      ...profileCard,
      gameId: profileCard.gameId ?? requestedGameId.value,
      gameName: profileCard.gameName || requestedGameName.value,
    };
    // The leaf category can contain only this one Mod. Keep the list useful by
    // filtering at the top-level group while still expanding the full trail.
    const targetCategoryId = card.categoryTrail[0]?._idRow ?? card.categoryId;
    if (targetCategoryId !== null && selectedCategoryId.value !== targetCategoryId) {
      await revealCategoryTrail(card.categoryTrail);
      selectedCategoryId.value = targetCategoryId;
      await loadMods(1);
      return;
    }
    if (!mods.value.some((item) => item.id === card.id)) {
      mods.value = [card, ...mods.value];
    }
    resolvedRequestedModTargetKey = targetKey;
    await selectMod(card);
  } catch (error) {
    if (!pageDisposed && requestedModId.value === modId) {
      errorMessage.value = t('gameBanana.errors.loadDetail', { error: String(error) });
    }
  }
}

const applyTarget = async () => {
  currentPage.value = 1;
  selectedCategoryId.value = null;
  if (!gameId.value || gameId.value <= 0) {
    categoryTree.value = [];
    mods.value = [];
    detail.value = null;
    selectedModId.value = null;
    totalRecords.value = 0;
    hasMore.value = false;
    return;
  }
  await loadCategories();
  await loadMods(1);
};

const selectCategory = (categoryId: number | null) => {
  if (selectedCategoryId.value === categoryId) return;
  resolvedRequestedModTargetKey = requestedModTargetKey();
  selectedCategoryId.value = categoryId;
  void loadMods(1);
};

const categoryCanExpand = (node: GbCategoryNode): boolean => node.hasChildren !== false;

const toggleCategory = async (node: GbCategoryNode) => {
  const categoryId = node._idRow;
  const next = new Set(expandedCategoryIds.value);
  if (next.has(categoryId)) {
    next.delete(categoryId);
    expandedCategoryIds.value = next;
    return;
  }
  await loadCategoryChildren(node);
  if (node.hasChildren === false) return;
  next.add(categoryId);
  expandedCategoryIds.value = next;
};

const openExternal = async (url: string) => {
  if (!url) return;
  try {
    await openUrl(url);
  } catch (error) {
    ElMessage.error(t('gameBanana.errors.openExternal', { error: String(error) }));
  }
};

const memberIdFromProfileUrl = (profileUrl: string): number | null => {
  const match = profileUrl.match(/\/members\/(\d+)(?:\/|$)/i);
  const value = match ? Number(match[1]) : NaN;
  return Number.isSafeInteger(value) && value > 0 ? value : null;
};

const openAuthorPage = (authorId: number | null, authorUrl: string, authorName: string) => {
  const memberId = authorId || memberIdFromProfileUrl(authorUrl);
  if (!memberId) {
    void openExternal(authorUrl);
    return;
  }
  void gameBananaHistory.push(router, {
    kind: 'author',
    title: authorName,
    location: { name: 'GameBananaAuthor', params: { authorId: String(memberId) } },
  });
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

const richTextClick = (event: MouseEvent) => {
  const target = event.target instanceof Element ? event.target : null;
  const image = target?.closest('img') as HTMLImageElement | null;
  if (image) {
    if (!shouldBlurImage(detail.value?.isNsfw === true)) return;
    event.preventDefault();
    toggleRichTextImageBlur(image);
    return;
  }
  const anchor = target?.closest('a[href]') as HTMLAnchorElement | null;
  if (!anchor?.href) return;
  event.preventDefault();
  void openExternal(anchor.href);
};

const sanitizeInstallName = (value: string): string => {
  const safe = value.replace(/[<>:"/\\|?*\u0000-\u001F]/g, ' ').replace(/\s+/g, ' ').trim();
  return safe || 'GameBanana Mod';
};

const sanitizeCategoryPathSegment = (value: string): string => {
  const safe = value.replace(/[<>:"/\\|?*\u0000-\u001F]/g, ' ').replace(/[. ]+$/g, '').replace(/\s+/g, ' ').trim();
  return safe || 'Uncategorized';
};

const categoryTrailFromTree = (categoryId: number | null): GbCategoryNode[] => {
  if (!categoryId) return [];
  const visit = (nodes: GbCategoryNode[], ancestors: GbCategoryNode[]): GbCategoryNode[] => {
    for (const node of nodes) {
      const path = [...ancestors, node];
      if (node._idRow === categoryId) return path;
      const nested = visit(node.children, path);
      if (nested.length) return nested;
    }
    return [];
  };
  return visit(categoryTree.value, []);
};

const gamebananaCategoryTrail = (mod: GbModCard): GbCategoryNode[] => {
  const treeTrail = categoryTrailFromTree(mod.categoryId);
  return treeTrail.length ? treeTrail : mod.categoryTrail;
};

const resolveGamebananaCategoryTrail = async (mod: GbModCard): Promise<GbCategoryNode[]> => {
  const existingTrail = categoryTrailFromTree(mod.categoryId);
  if (existingTrail.length || !mod.categoryId) return existingTrail.length ? existingTrail : mod.categoryTrail;

  const rootId = mod.categoryTrail[0]?._idRow;
  const roots = rootId
    ? categoryTree.value.filter((node) => node._idRow === rootId)
    : categoryTree.value;
  const visit = async (nodes: GbCategoryNode[], ancestors: GbCategoryNode[]): Promise<GbCategoryNode[]> => {
    for (const node of nodes) {
      const path = [...ancestors, node];
      if (node._idRow === mod.categoryId) return path;
      if (!node.childrenLoaded) await loadCategoryChildren(node);
      const nested = await visit(node.children, path);
      if (nested.length) return nested;
    }
    return [];
  };
  const loadedTrail = await visit(roots, []);
  return loadedTrail.length ? loadedTrail : mod.categoryTrail;
};

const gamebananaInstallGroupFromTrail = (trail: GbCategoryNode[], fallbackGameName = ''): string => {
  const sourceGameSegment = fallbackGameName ? sanitizeCategoryPathSegment(fallbackGameName) : '';
  return ['GameBanana', sourceGameSegment, ...trail.map((category) => sanitizeCategoryPathSegment(category._sName || `Category ${category._idRow}`))]
    .filter(Boolean)
    .join('/');
};

const gamebananaInstallGroup = (mod: GbModCard): string => gamebananaInstallGroupFromTrail(
  gamebananaCategoryTrail(mod),
  fallbackInstallGameName.value || (currentGameName.value === 'Default' ? mod.gameName : ''),
);

const resolveGamebananaInstallTarget = async (mod: GbModCard, fallbackGameName = '') => {
  const categoryTrail = await resolveGamebananaCategoryTrail(mod);
  const automaticGroup = gamebananaInstallGroupFromTrail(categoryTrail, fallbackGameName);
  const targetGroup = installGroupMode.value === 'automatic'
    ? automaticGroup
    : installGroupMode.value === 'manual'
      ? (installGroup.value.trim() || 'Root')
      : (installGroup.value.trim() || automaticGroup);
  return {
    categoryTrail,
    automaticGroup,
    targetGroup,
    targetName: sanitizeInstallName(mod.title),
  };
};

const isFileDownloaded = (file: GbFile): boolean => downloadedFileIds.value.has(file._idRow ?? -1);

const refreshDownloadedFileState = async (mod: GbModDetail | null = detail.value) => {
  const requestId = ++downloadedStateRequestId;
  if (!mod?.files.length) {
    downloadedFileIds.value = new Set();
    return;
  }

  try {
    const context = await resolveGamebananaInstallContext(mod);
    if (requestId === downloadedStateRequestId && detail.value?.id === mod.id) {
      fallbackInstallGameName.value = context.fallbackGameName;
    }
    const target = await resolveGamebananaInstallTarget(mod, context.fallbackGameName);
    const installed = await invoke<boolean>('mod_install_target_exists', {
      installDir: context.installDir,
      targetName: target.targetName,
      targetGroup: target.targetGroup,
    });
    if (requestId !== downloadedStateRequestId || detail.value?.id !== mod.id) return;
    downloadedFileIds.value = installed
      ? new Set(mod.files.map((file) => file._idRow ?? -1))
      : new Set();
  } catch (error) {
    if (requestId === downloadedStateRequestId) {
      console.warn('Unable to check the GameBanana install target:', error);
      downloadedFileIds.value = new Set();
    }
  }
};

const cacheGamebananaCategoryIcon = async (category: GbCategoryNode): Promise<string> => {
  const iconUrl = asString(category._sIconUrl);
  if (!isSafeHttpUrl(iconUrl)) return '';
  const cacheDir = await join(await appDataDir(), GAMEBANANA_ICON_CACHE_FOLDER);
  const cachedIcon = await join(cacheDir, `category-${category._idRow}.png`);
  if (await exists(cachedIcon)) return cachedIcon;

  const response = await fetch(iconUrl, { method: 'GET' });
  if (!response.ok) throw new Error(`GameBanana HTTP ${response.status}`);
  const data = new Uint8Array(await response.arrayBuffer());
  await mkdir(cacheDir, { recursive: true });
  await writeFile(cachedIcon, data);
  return cachedIcon;
};

const applyGamebananaCategoryIcons = async (gameName: string, trail: GbCategoryNode[]) => {
  for (let index = 0; index < trail.length; index += 1) {
    const category = trail[index];
    try {
      const cachedIcon = await cacheGamebananaCategoryIcon(category);
      if (!cachedIcon) continue;
      const categoryGroupPath = ['GameBanana', ...trail.slice(0, index + 1).map((node) => sanitizeCategoryPathSegment(node._sName || `Category ${node._idRow}`))].join('/');
      await ModManager.setModGroupIcon(gameName, categoryGroupPath, cachedIcon);
    } catch (error) {
      console.warn(`Unable to assign GameBanana category icon for ${category._idRow}:`, error);
    }
  }
};

const progressRatio = (current: number | undefined, total: number | undefined, unknownTotalBytes = 220 * 1024 * 1024): number => {
  const normalizedCurrent = Math.max(0, Number(current) || 0);
  const normalizedTotal = Math.max(0, Number(total) || 0);
  if (normalizedTotal > 0) return Math.min(100, (normalizedCurrent / normalizedTotal) * 100);
  return Math.min(95, (normalizedCurrent / unknownTotalBytes) * 100);
};

const isActiveInstallEvent = (payload: InstallProgressEvent) => {
  if (!activeGamebananaInstall) return false;
  const eventGame = payload.gameName || payload.game_name || '';
  const eventMod = payload.modName || payload.mod_name || '';
  return eventGame === activeGamebananaInstall.gameName && eventMod === activeGamebananaInstall.modName;
};

const updateDownloadProgress = (payload: InstallProgressEvent) => {
  if (!isActiveInstallEvent(payload)) return;
  installPhase.value = 'downloading';
  installStatus.value = t('gameBanana.installDownloading');
  downloadProgress.value = progressRatio(payload.current, payload.total);
};

const updateInstallProgress = (payload: InstallProgressEvent) => {
  if (!isActiveInstallEvent(payload)) return;
  const stage = String(payload.stage || '').toLowerCase();
  if (stage === 'done') {
    installPhase.value = 'installing';
    installProgress.value = 100;
    return;
  }
  installPhase.value = 'installing';
  installStatus.value = t('gameBanana.installInstalling');
  const stageBase = stage === 'analyzing' ? 2 : 6;
  installProgress.value = stage === 'analyzing'
    ? stageBase
    : Math.min(98, stageBase + progressRatio(payload.current, payload.total) * (100 - stageBase) / 100);
};

const installButtonStyle = (file: GbFile) => {
  const active = installingFileId.value === (file._idRow ?? -1);
  return {
    '--gb-download-progress': `${active ? downloadProgress.value : 0}%`,
    '--gb-install-progress': `${active ? Math.min(downloadProgress.value, installProgress.value) : 0}%`,
  };
};

const installButtonClass = (file: GbFile) => ({
  'is-progress': installingFileId.value === (file._idRow ?? -1),
  'is-downloaded': installingFileId.value !== (file._idRow ?? -1) && isFileDownloaded(file),
});

const isCancellableDownload = (file: GbFile) => installingFileId.value === (file._idRow ?? -1)
  && installPhase.value === 'downloading';

const isFileActionDisabled = (file: GbFile) => (isInstalling.value && !isCancellableDownload(file))
  || (installGroupMode.value === 'matching-icon' && !installGroup.value);

const installButtonLabel = (file: GbFile) => isCancellableDownload(file)
  ? installStatus.value === t('gameBanana.cancellingDownload')
    ? installStatus.value
    : t('gameBanana.cancelDownload')
  : installingFileId.value === (file._idRow ?? -1)
    ? installStatus.value || t('gameBanana.installing')
  : isFileDownloaded(file) ? t('gameBanana.downloaded') : t('gameBanana.install');

const listenForInstallProgress = async () => {
  const [downloadUnlisten, installUnlisten] = await Promise.all([
    listen<InstallProgressEvent>('gamebanana-install-progress', (event) => updateDownloadProgress(event.payload)),
    listen<InstallProgressEvent>('mod-install-progress', (event) => updateInstallProgress(event.payload)),
  ]);
  unlistenDownloadProgress = downloadUnlisten;
  unlistenInstallProgress = installUnlisten;
};

const cancelDownloadAndInstall = async (file: GbFile) => {
  if (!isCancellableDownload(file) || !activeGamebananaInstall) return;
  installStatus.value = t('gameBanana.cancellingDownload');
  try {
    await invoke('cancel_gamebanana_download_and_install_mod', {
      gameName: activeGamebananaInstall.gameName,
      targetName: activeGamebananaInstall.modName,
    });
  } catch (error) {
    // The transfer itself remains active if the cancellation signal could not
    // reach the backend, so leave the same button available for another try.
    installStatus.value = t('gameBanana.cancelDownload');
    ElMessage.error(t('gameBanana.errors.installFailed', { error: String(error) }));
  }
};

const handleInstallAction = async (file: GbFile) => {
  if (isCancellableDownload(file)) {
    await cancelDownloadAndInstall(file);
    return;
  }
  await downloadAndInstall(file);
};

const downloadAndInstall = async (file: GbFile) => {
  if (!detail.value || isInstalling.value) return;
  const downloadUrl = asString(file._sDownloadUrl);
  if (!downloadUrl) {
    ElMessage.error(t('gameBanana.errors.installNoDownload'));
    return;
  }

  if (isFileDownloaded(file)) {
    try {
      await ElMessageBox.confirm(
        t('gameBanana.downloadAgainConfirm', { name: detail.value.title }),
        t('gameBanana.downloaded'),
        {
          confirmButtonText: t('gameBanana.downloadAgain'),
          cancelButtonText: t('gameBanana.cancel'),
          type: 'warning',
        },
      );
    } catch {
      return;
    }
  }

  const fileId = file._idRow ?? -1;
  installingFileId.value = fileId;
  installStatus.value = t('gameBanana.installDownloading');
  installPhase.value = 'downloading';
  downloadProgress.value = 0;
  installProgress.value = 0;
  try {
    const context = await resolveGamebananaInstallContext(detail.value, true);
    const target = await resolveGamebananaInstallTarget(detail.value, context.fallbackGameName);
    const previewUrls = Array.from(new Set([
      ...detail.value.screenshots,
      detail.value.thumbnailUrl,
    ].map((url) => url.trim()).filter(Boolean)));
    activeGamebananaInstall = { gameName: context.gameName, modName: target.targetName };
    await invoke('gamebanana_download_and_install_mod', {
      gameName: context.gameName,
      installDir: context.installDir,
      downloadUrl,
      archiveName: file._sFile || '',
      targetName: target.targetName,
      targetGroup: target.targetGroup,
      password: null,
      backupExisting: true,
      previewUrls,
    });
    if (!context.fallbackGameName && installGroupMode.value === 'automatic' && target.targetGroup === target.automaticGroup) {
      await applyGamebananaCategoryIcons(context.gameName, target.categoryTrail);
    }
    downloadedFileIds.value = new Set(detail.value.files.map((item) => item._idRow ?? -1));
    installStatus.value = t('gameBanana.installComplete');
    ElMessage.success(t('gameBanana.installComplete'));
  } catch (error) {
    if (String(error).includes('GameBanana download cancelled')) {
      installStatus.value = t('gameBanana.downloadCancelled');
      ElMessage.info(installStatus.value);
      return;
    }
    installStatus.value = t('gameBanana.errors.installFailed', { error: String(error) });
    ElMessage.error(installStatus.value);
  } finally {
    activeGamebananaInstall = null;
    installingFileId.value = null;
    installPhase.value = 'idle';
    downloadProgress.value = 0;
    installProgress.value = 0;
  }
};

const clearTranslations = () => {
  for (const node of translationNodes) node.remove();
  translationNodes.clear();
  translationBlocksByAnchor.clear();
  translationAnchorsByBlock.clear();
};

const clearHoveredText = () => {
  hoveredTranslateElement = null;
  hoveredTranslateText = '';
};

const richTextBlockSelector = 'p, li, blockquote, pre, h1, h2, h3, h4, h5, h6, td, th';

const translationAnchorFromElement = (target: Element): HTMLElement | null => {
  if (target.closest('.gb-inline-translation')) return null;

  const richTextContainer = target.closest('.gb-rich-text, .gb-comment-body') as HTMLElement | null;
  if (richTextContainer) {
    const block = target.closest(richTextBlockSelector) as HTMLElement | null;
    return block && richTextContainer.contains(block) ? block : richTextContainer;
  }

  return target.closest('[data-gb-translate]') as HTMLElement | null;
};

const rememberHoveredText = (event: MouseEvent) => {
  const eventTarget = event.target;
  const target = eventTarget instanceof Element
    ? eventTarget
    : eventTarget instanceof Node
      ? eventTarget.parentElement
      : null;
  const element = target ? translationAnchorFromElement(target) : null;
  if (!element) {
    clearHoveredText();
    return;
  }
  const text = element?.innerText?.trim() || '';
  if (!element || !text) {
    clearHoveredText();
    return;
  }
  hoveredTranslateElement = element;
  hoveredTranslateText = text;
};

const matchesTranslationShortcut = (event: KeyboardEvent): boolean => {
  const shortcut = appSettings.gamebananaTranslationShortcut.trim().toLowerCase() || 'ctrl';
  const tokens = shortcut.split('+').map((item) => item.trim()).filter(Boolean);
  const key = tokens.find((item) => !['ctrl', 'control', 'shift', 'alt', 'meta', 'cmd', 'command'].includes(item));
  const expectsCtrl = tokens.includes('ctrl') || tokens.includes('control');
  const expectsMeta = tokens.includes('meta') || tokens.includes('cmd') || tokens.includes('command');
  if (!key) {
    return expectsCtrl
      && event.key.toLowerCase() === 'control'
      && event.ctrlKey
      && !event.metaKey
      && !event.shiftKey
      && !event.altKey;
  }
  if (event.key.toLowerCase() !== key) return false;
  return (expectsCtrl ? event.ctrlKey : !event.ctrlKey)
    && (expectsMeta ? event.metaKey : !event.metaKey)
    && event.shiftKey === tokens.includes('shift')
    && event.altKey === tokens.includes('alt');
};

const removeTranslationBlock = (block: HTMLElement) => {
  const anchor = translationAnchorsByBlock.get(block);
  block.remove();
  translationNodes.delete(block);
  translationAnchorsByBlock.delete(block);
  if (anchor) translationBlocksByAnchor.delete(anchor);
};

const removeTranslationForAnchor = (anchor: HTMLElement) => {
  const block = translationBlocksByAnchor.get(anchor);
  if (block) removeTranslationBlock(block);
};

const registerTranslationBlock = (anchor: HTMLElement, block: HTMLElement) => {
  translationNodes.add(block);
  translationBlocksByAnchor.set(anchor, block);
  translationAnchorsByBlock.set(block, anchor);
};

const appendTranslation = (anchor: HTMLElement, translation: string, richText: boolean) => {
  const block = document.createElement('p');
  block.className = 'gb-inline-translation';
  applyTranslationTextStyle(block);
  if (richText) {
    block.innerHTML = translatedParagraphHtml(translation);
  } else {
    block.textContent = translation;
  }
  anchor.insertAdjacentElement('afterend', block);
  registerTranslationBlock(anchor, block);
  return block;
};

const appendTranslationLoading = (anchor: HTMLElement) => {
  const block = document.createElement('p');
  block.className = 'gb-inline-translation is-loading';
  applyTranslationTextStyle(block);
  const spinner = document.createElement('i');
  spinner.className = 'gb-translation-spinner';
  spinner.setAttribute('aria-hidden', 'true');
  const label = document.createElement('span');
  label.textContent = t('gameBanana.translationWorking');
  block.append(spinner, label);
  anchor.insertAdjacentElement('afterend', block);
  registerTranslationBlock(anchor, block);
  return block;
};

const resolveTranslationLoading = (block: HTMLElement, translation: string, richText: boolean) => {
  if (!block.isConnected) return;
  block.classList.remove('is-loading');
  if (richText) {
    block.innerHTML = translatedParagraphHtml(translation);
  } else {
    block.textContent = translation;
  }
};

const loadMatchingIconGroups = async () => {
  const mod = detail.value;
  if (!mod || !currentGameName.value || currentGameName.value === 'Default') {
    matchingIconGroups.value = [];
    return;
  }
  loadingMatchingIconGroups.value = true;
  matchingIconGroups.value = [];
  try {
    const trail = await resolveGamebananaCategoryTrail(mod);
    const leaf = trail.at(-1);
    if (!leaf) return;
    const targetIcon = await cacheGamebananaCategoryIcon(leaf);
    if (!targetIcon) return;
    const targetHash = await invoke<string>('file_md5', { path: targetIcon });
    const { groups } = await ModManager.scanAllMods(currentGameName.value);
    const candidates = groups.filter((group) => !!group.iconPath && group.id !== 'Root' && group.id !== 'All');
    const matches = await Promise.all(candidates.map(async (group) => {
      try {
        const hash = await invoke<string>('file_md5', { path: group.iconPath as string });
        return hash === targetHash ? { id: group.id, iconPath: group.iconPath as string } : null;
      } catch { return null; }
    }));
    matchingIconGroups.value = matches.filter((item): item is { id: string; iconPath: string } => item !== null);
    installGroup.value = matchingIconGroups.value[0]?.id || '';
  } catch (error) {
    console.warn('Unable to find groups with matching GameBanana category icon:', error);
    ElMessage.warning(t('gameBanana.matchingIconPathsFailed', { error: String(error) }));
  } finally {
    loadingMatchingIconGroups.value = false;
  }
};

const TRANSLATION_PROVIDER_PRESETS: Record<string, { url: string; model: string }> = {
  openai: { url: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  compatible: { url: '', model: '' },
  claude: { url: 'https://api.anthropic.com/v1', model: 'claude-3-5-haiku-latest' },
  deepseek: { url: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  gemini: { url: 'https://generativelanguage.googleapis.com/v1beta', model: 'gemini-2.0-flash' },
  google: { url: 'https://translation.googleapis.com/language/translate/v2', model: '' },
};

const applyTranslationProviderPreset = () => {
  const preset = TRANSLATION_PROVIDER_PRESETS[appSettings.gamebananaTranslationProvider];
  if (!preset) return;
  appSettings.gamebananaTranslationApiUrl = preset.url;
  appSettings.gamebananaTranslationModel = preset.model;
  translationModelOptions.value = preset.model
    ? [{ value: preset.model, label: preset.model }]
    : [{ value: 'nmt', label: 'Google NMT' }];
};

const providerEndpoint = (baseUrl: string, suffix: string): string => {
  const normalized = baseUrl.replace(/\/$/, '');
  if (normalized.toLowerCase().endsWith(suffix.toLowerCase())) return normalized;
  return normalized.replace(/\/(?:chat\/completions|messages|generateContent)$/i, '') + suffix;
};

const readApiError = (payload: unknown, fallback: string): string => {
  if (!payload || typeof payload !== 'object') return fallback;
  const data = payload as { error?: { message?: unknown } | unknown; message?: unknown };
  if (typeof data.error === 'string') return data.error;
  if (data.error && typeof data.error === 'object' && typeof (data.error as { message?: unknown }).message === 'string') {
    return (data.error as { message: string }).message;
  }
  return typeof data.message === 'string' ? data.message : fallback;
};

const uniqueModelOptions = (options: TranslationModelOption[]): TranslationModelOption[] => {
  const seen = new Set<string>();
  return options
    .filter((option) => option.value && !seen.has(option.value) && Boolean(seen.add(option.value)))
    .sort((left, right) => left.label.localeCompare(right.label));
};

const fetchTranslationModels = async () => {
  const provider = appSettings.gamebananaTranslationProvider;
  const apiKey = appSettings.gamebananaTranslationApiKey.trim();
  const baseUrl = appSettings.gamebananaTranslationApiUrl.trim().replace(/\/$/, '');
  if (!apiKey || !baseUrl) {
    translationModelsStatus.value = t('gameBanana.translationModelsMissingConfig');
    return;
  }

  loadingTranslationModels.value = true;
  translationModelsStatus.value = t('gameBanana.translationModelsLoading');
  try {
    let options: TranslationModelOption[] = [];
    if (provider === 'google') {
      const endpoint = providerEndpoint(baseUrl, '/languages');
      const response = await fetch(`${endpoint}?key=${encodeURIComponent(apiKey)}`, { method: 'GET' });
      const payload = await response.json() as { data?: { languages?: unknown[] } };
      if (!response.ok) throw new Error(readApiError(payload, `HTTP ${response.status}`));
      options = [{ value: 'nmt', label: 'Google NMT (default)' }];
    } else if (provider === 'gemini') {
      const endpoint = providerEndpoint(baseUrl, '/models');
      const response = await fetch(`${endpoint}?pageSize=1000&key=${encodeURIComponent(apiKey)}`, { method: 'GET' });
      const payload = await response.json() as {
        models?: Array<{ name?: unknown; displayName?: unknown; supportedGenerationMethods?: unknown }>;
        error?: { message?: string };
      };
      if (!response.ok) throw new Error(readApiError(payload, `HTTP ${response.status}`));
      options = (payload.models || [])
        .filter((model) => !Array.isArray(model.supportedGenerationMethods) || model.supportedGenerationMethods.includes('generateContent'))
        .map((model) => {
          const name = asString(model.name).replace(/^models\//i, '');
          const displayName = asString(model.displayName);
          return { value: name, label: displayName ? `${displayName} (${name})` : name };
        });
    } else {
      const endpoint = providerEndpoint(baseUrl, '/models');
      const response = await fetch(endpoint, {
        method: 'GET',
        headers: provider === 'claude'
          ? { 'x-api-key': apiKey, 'anthropic-version': '2023-06-01' }
          : { Authorization: `Bearer ${apiKey}` },
      });
      const payload = await response.json() as {
        data?: Array<{ id?: unknown; display_name?: unknown; displayName?: unknown }>;
        error?: { message?: string };
      };
      if (!response.ok) throw new Error(readApiError(payload, `HTTP ${response.status}`));
      options = (payload.data || []).map((model) => {
        const id = asString(model.id);
        const label = asString(model.display_name) || asString(model.displayName);
        return { value: id, label: label && label !== id ? `${label} (${id})` : id };
      });
    }

    const currentModel = appSettings.gamebananaTranslationModel.trim();
    if (currentModel && !options.some((option) => option.value === currentModel)) {
      options.push({ value: currentModel, label: `${currentModel} (${t('gameBanana.translationModelCustom')})` });
    }
    translationModelOptions.value = uniqueModelOptions(options);
    if (!appSettings.gamebananaTranslationModel && translationModelOptions.value.length) {
      appSettings.gamebananaTranslationModel = translationModelOptions.value[0].value;
    }
    translationModelsStatus.value = t('gameBanana.translationModelsLoaded', { count: translationModelOptions.value.length });
  } catch (error) {
    translationModelsStatus.value = t('gameBanana.errors.translationModelsFailed', { error: String(error) });
  } finally {
    loadingTranslationModels.value = false;
  }
};

const onTranslationProviderChange = () => {
  applyTranslationProviderPreset();
  void fetchTranslationModels();
};

const translateWithConfiguredProvider = async (
  source: string,
  targetLanguage: string,
  richText: boolean,
  context = '',
): Promise<string> => {
  const provider = appSettings.gamebananaTranslationProvider;
  const apiKey = appSettings.gamebananaTranslationApiKey.trim();
  const baseUrl = appSettings.gamebananaTranslationApiUrl.trim().replace(/\/$/, '');
  const model = appSettings.gamebananaTranslationModel.trim() || 'gpt-4o-mini';
  const translationInstruction = richText
    ? `Translate only the TARGET GameBanana HTML fragment into ${targetLanguage}. Use the FULL CONTENT only as context. Preserve the target's meaningful HTML structure, links, and inline formatting. Return only the translated TARGET as valid HTML, with no markers, Markdown fences, explanation, or translation of other paragraphs.`
    : `Translate only the TARGET GameBanana paragraph into ${targetLanguage}. Use the FULL CONTENT only as context. Return only the translated TARGET, with no markers, explanation, or translation of other paragraphs.`;
  const contextualSource = context
    ? `<FULL_CONTENT>\n${context}\n</FULL_CONTENT>\n\n<TARGET>\n${source}\n</TARGET>`
    : source;

  if (provider === 'claude') {
    const endpoint = /\/messages$/i.test(baseUrl) ? baseUrl : `${baseUrl}/messages`;
    const response = await fetch(endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': apiKey,
        'anthropic-version': '2023-06-01',
      },
      body: JSON.stringify({
        model,
        max_tokens: 2048,
        system: translationInstruction,
        messages: [{ role: 'user', content: contextualSource }],
      }),
    });
    const payload = await response.json() as { content?: Array<{ text?: string }>; error?: { message?: string } };
    if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
    return asString(payload.content?.[0]?.text);
  }

  if (provider === 'gemini') {
    const endpointBase = /\/models$/i.test(baseUrl) ? baseUrl : `${baseUrl}/models`;
    const response = await fetch(`${endpointBase}/${encodeURIComponent(model)}:generateContent?key=${encodeURIComponent(apiKey)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        contents: [{ parts: [{ text: `${translationInstruction}\n\n${contextualSource}` }] }],
        generationConfig: { temperature: 0.2 },
      }),
    });
    const payload = await response.json() as { candidates?: Array<{ content?: { parts?: Array<{ text?: string }> } }>; error?: { message?: string } };
    if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
    return asString(payload.candidates?.[0]?.content?.parts?.[0]?.text);
  }

  if (provider === 'google') {
    const endpoint = /\/language\/translate\/v2$/i.test(baseUrl) ? baseUrl : `${baseUrl}/language/translate/v2`;
    const response = await fetch(`${endpoint}?key=${encodeURIComponent(apiKey)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ q: source, target: targetLanguage, format: richText ? 'html' : 'text', model: model || 'nmt' }),
    });
    const payload = await response.json() as { data?: { translations?: Array<{ translatedText?: string }> }; error?: { message?: string } };
    if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
    const translated = asString(payload.data?.translations?.[0]?.translatedText);
    return richText ? translated : toPlainText(translated);
  }

  const endpoint = /\/chat\/completions$/i.test(baseUrl) ? baseUrl : `${baseUrl}/chat/completions`;
  const response = await fetch(endpoint, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      temperature: 0.2,
      messages: [
        { role: 'system', content: translationInstruction },
        { role: 'user', content: contextualSource },
      ],
    }),
  });
  const payload = await response.json() as { choices?: Array<{ message?: { content?: string } }>; error?: { message?: string } };
  if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
  return asString(payload.choices?.[0]?.message?.content);
};

const isRichTextContainer = (element: HTMLElement) => element.matches('.gb-rich-text, .gb-comment-body');

const translationContextForContainer = (container: HTMLElement, richText: boolean): string => {
  const clone = container.cloneNode(true) as HTMLElement;
  clone.querySelectorAll('.gb-inline-translation').forEach((node) => node.remove());
  return richText ? clone.innerHTML.trim() : clone.innerText.trim();
};

const translationTargetsForAnchor = (anchor: HTMLElement): HTMLElement[] => {
  if (!isRichTextContainer(anchor)) return [anchor];
  const candidates = Array.from(anchor.querySelectorAll(richTextBlockSelector))
    .filter((block): block is HTMLElement => block instanceof HTMLElement && Boolean(block.innerText.trim()));
  // Prefer leaf blocks: list items and table cells may themselves contain a
  // paragraph, and translating both would duplicate the same source text.
  const blocks = candidates.filter((block) => !candidates.some((other) => other !== block && block.contains(other)));
  return blocks.length ? blocks : [anchor];
};

const translateAnchor = async (anchor: HTMLElement) => {
  const richText = appSettings.gamebananaTranslationRichText
    && Boolean(anchor.closest('.gb-rich-text, .gb-comment-body'));
  const source = richText
    ? anchor.innerHTML.trim()
    : anchor.innerText.trim();
  const contextContainer = anchor.closest('.gb-rich-text, .gb-comment-body') as HTMLElement | null;
  const fullContext = appSettings.gamebananaTranslationUseContext && contextContainer
    ? translationContextForContainer(contextContainer, richText)
    : '';
  if (!source || !anchor.isConnected) {
    return;
  }
  if (translationBlocksByAnchor.has(anchor)) {
    removeTranslationForAnchor(anchor);
    return;
  }
  const apiKey = appSettings.gamebananaTranslationApiKey.trim();
  const baseUrl = appSettings.gamebananaTranslationApiUrl.trim().replace(/\/$/, '');
  if (!apiKey || !baseUrl) {
    ElMessage.warning(t('gameBanana.translationMissingConfig'));
    showTranslationSettings.value = true;
    return;
  }

  const targetLanguage = appSettings.gamebananaTranslationTargetLanguage.trim() || '简体中文';
  const model = appSettings.gamebananaTranslationModel.trim() || 'gpt-4o-mini';
  const cacheKey = `${appSettings.gamebananaTranslationProvider}|${baseUrl}|${model}|${targetLanguage}|${richText ? 'html' : 'text'}|${fullContext}|${source}`;
  const cached = translationCache.get(cacheKey);
  if (cached) {
    appendTranslation(anchor, cached, richText);
    return;
  }

  translationRequestCount.value += 1;
  translationStatus.value = t('gameBanana.translationWorking');
  const loadingBlock = appendTranslationLoading(anchor);
  try {
    const maxAttempts = appSettings.gamebananaTranslationFailureMode === 'retry' ? 5 : 1;
    let translated = '';
    let lastError: unknown;
    for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
      try {
        translated = await translateWithConfiguredProvider(
          source.slice(0, 12000), targetLanguage, richText,
          appSettings.gamebananaTranslationProvider === 'google' ? '' : fullContext.slice(0, 24000),
        );
        if (!translated) throw new Error(t('gameBanana.errors.translationEmpty'));
        break;
      } catch (error) {
        lastError = error;
      }
    }
    if (!translated) throw lastError || new Error(t('gameBanana.errors.translationEmpty'));
    translationCache.set(cacheKey, translated);
    resolveTranslationLoading(loadingBlock, translated, richText);
  } catch (error) {
    if (appSettings.gamebananaTranslationFailureMode === 'message') {
      loadingBlock.classList.remove('is-loading');
      loadingBlock.textContent = t('gameBanana.translationFailedInline');
    } else {
      removeTranslationBlock(loadingBlock);
    }
    translationStatus.value = t('gameBanana.errors.translationFailed', { error: String(error) });
    ElMessage.error(translationStatus.value);
  } finally {
    translationRequestCount.value = Math.max(0, translationRequestCount.value - 1);
    if (translationRequestCount.value === 0 && !translationStatus.value.startsWith(t('gameBanana.errors.translationFailed', { error: '' }))) {
      translationStatus.value = '';
    }
  }
};

const translateHoveredParagraph = async () => {
  if (!appSettings.gamebananaTranslationEnabled) return;
  const anchor = hoveredTranslateElement;
  if (!anchor?.isConnected || !hoveredTranslateText.trim()) {
    ElMessage.info(t('gameBanana.translationNoParagraph'));
    return;
  }

  const targets = translationTargetsForAnchor(anchor);
  // When the pointer lands directly on a rich-text container, retain the
  // paragraph boundary in both directions: one hotkey creates one result per
  // source block; pressing again removes that same set instead of appending a
  // combined translation after the entire article.
  if (targets.length > 1 && targets.some((target) => translationBlocksByAnchor.has(target))) {
    targets.forEach(removeTranslationForAnchor);
    return;
  }
  await Promise.all(targets.map((target) => translateAnchor(target)));
};

// 翻译快捷键「点按 vs 按住」判定:只有快速的点按(按下-松开在
// TAP_WINDOW_MS 内)才视为一次有意的翻译调用;按住不放(例如 Ctrl
// 配合游戏内操作或其它快捷键)绝不会弹出「请先悬停」提示。
const TAP_WINDOW_MS = 300;
let translationKeyDownAt = 0;

const onGlobalKeydown = (event: KeyboardEvent) => {
  const target = event.target as HTMLElement | null;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target?.isContentEditable) return;
  if (!matchesTranslationShortcut(event)) return;
  // 按住产生的系统重复 keydown 一律忽略,避免重复翻译/重复提示。
  if (event.repeat) return;
  event.preventDefault();
  if (!appSettings.gamebananaTranslationEnabled) return;

  const anchor = hoveredTranslateElement;
  if (anchor?.isConnected && hoveredTranslateText.trim()) {
    // 有悬停文本:立即翻译;本次按键在松开时不再弹提示。
    translationKeyDownAt = 0;
    void translateHoveredParagraph();
    return;
  }

  // 没有悬停文本:先记录按下时间,是否提示推迟到松开时决定。
  translationKeyDownAt = Date.now();
};

const shortcutModifierKey = (event: KeyboardEvent): boolean => {
  const shortcut = appSettings.gamebananaTranslationShortcut.trim().toLowerCase() || 'ctrl';
  const tokens = shortcut.split('+').map((item) => item.trim().toLowerCase()).filter(Boolean);
  const key = event.key.trim().toLowerCase();
  if (tokens.includes(key)) return true;
  if (key === 'control' && (tokens.includes('ctrl') || tokens.includes('control'))) return true;
  if (key === 'meta' && (tokens.includes('meta') || tokens.includes('cmd') || tokens.includes('command'))) return true;
  return false;
};

const onGlobalKeyup = (event: KeyboardEvent) => {
  if (!shortcutModifierKey(event)) return;
  if (!translationKeyDownAt) return;
  const heldMs = Date.now() - translationKeyDownAt;
  translationKeyDownAt = 0;
  // 按住超过点按窗口 → 视为「按住」,不打扰用户。
  if (heldMs > TAP_WINDOW_MS) return;
  const anchor = hoveredTranslateElement;
  if (anchor?.isConnected && hoveredTranslateText.trim()) return;
  ElMessage.info(t('gameBanana.translationNoParagraph'));
};

const onGlobalBlur = () => {
  // 窗口失焦时清掉未完成的点按记录,防止稍后误弹提示。
  translationKeyDownAt = 0;
};

const syncGameTargetOnce = async () => {
  let gameName = currentGameName.value;
  const routeGameId = requestedGameId.value;
  if (routeGameId !== null) {
    const matchedGame = await resolveGamebananaEnvironment(routeGameId, requestedGameName.value);
    if (matchedGame && matchedGame.name !== gameName) {
      await AppStateManager.selectGame(matchedGame);
      gameName = matchedGame.name;
    }
  }

  const storedKey = `gamebanana:game-id:${gameName || 'default'}`;
  const storedId = Number(localStorage.getItem(storedKey));
  gameTargetLabel.value = gameName ? getGamePresetDisplayName(gameName, t) : t('gameBanana.noGameSelected');
  showGameIdInput.value = true;
  gameId.value = Number.isInteger(storedId) && storedId > 0 ? storedId : null;

  if (routeGameId !== null) {
    gameId.value = routeGameId;
    const preset = gamebananaPresetForGameId(routeGameId);
    gameTargetLabel.value = gameName && gameName !== 'Default'
      ? `${getGamePresetDisplayName(gameName, t)} · ${preset ? getGamePresetDisplayName(preset, t) : routeGameId}`
      : requestedGameName.value || `${t('gameBanana.game')} · ${routeGameId}`;
    await applyTarget();
    return;
  }

  if (gameName && gameName !== 'Default') {
    try {
      const config = await ResourceManager.loadGameConfig(gameName);
      const preset = asString(config?.gamePreset).toUpperCase();
      const presetGameId = GAMEBANANA_ID_BY_PRESET[preset];
      if (presetGameId) {
        gameId.value = presetGameId;
        showGameIdInput.value = false;
        gameTargetLabel.value = `${getGamePresetDisplayName(gameName, t)} · ${getGamePresetDisplayName(preset, t)}`;
      }
    } catch (error) {
      console.warn('Unable to load the current game configuration for GameBanana:', error);
    }
  }

  await applyTarget();
};

const syncGameTarget = async () => {
  gameTargetSyncRevision += 1;
  if (syncingGameTarget) return;
  syncingGameTarget = true;
  try {
    let completedRevision = -1;
    while (completedRevision !== gameTargetSyncRevision) {
      completedRevision = gameTargetSyncRevision;
      await syncGameTargetOnce();
    }
  } finally {
    syncingGameTarget = false;
  }
};

watch(() => route.fullPath, () => gameBananaHistory.observe(router, route), { immediate: true });

watch(gameId, (value) => {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) return;
  const normalized = Math.floor(parsed);
  if (value !== normalized) {
    gameId.value = normalized;
    return;
  }
  if (requestedGameId.value === normalized) return;
  localStorage.setItem(`gamebanana:game-id:${currentGameName.value || 'default'}`, String(normalized));
});

watch([() => appSettings.gamebananaBlurMode, () => appSettings.revealBlurredImagesOnHover], () => {
  void nextTick(syncRichTextImageBlur);
});

watch([detail, () => comments.value.length, () => updates.value], () => {
  void nextTick(syncRichTextImageBlur);
});

watch(() => appSettings.CurrentGameName, () => {
  if (!syncingGameTarget) void syncGameTarget();
});

watch(() => [route.query.mod, route.query.game, route.query.gameName], () => {
  resolvedRequestedModTargetKey = '';
  void syncGameTarget();
});

watch([() => detail.value?.id, installGroup, currentGameName], () => {
  void refreshDownloadedFileState();
});

watch(() => detail.value?.id, () => {
  installGroupMode.value = 'automatic';
  installGroup.value = '';
  matchingIconGroups.value = [];
});

watch(installGroupMode, (mode) => {
  installGroup.value = '';
  if (mode === 'matching-icon') void loadMatchingIconGroups();
});

watch(showTranslationSettings, (visible) => {
  if (visible) void fetchTranslationModels();
});

type GbResizablePanel = 'categories' | 'results';

const GB_COLUMN_WIDTHS_STORAGE = 'gamebanana-column-widths-v2';
const GB_DEFAULT_PANEL_WIDTHS: Record<GbResizablePanel, number> = { categories: 230, results: 320 };
const GB_MIN_PANEL_WIDTHS: Record<GbResizablePanel, number> = { categories: 160, results: 320 };
const GB_MAX_PANEL_WIDTHS: Record<GbResizablePanel, number> = { categories: 420, results: 960 };
const GB_MIN_DETAIL_WIDTH = 270;
const GB_COLUMN_RESIZERS_WIDTH = 20;
const gbUsesStackedLayout = () => window.matchMedia('(max-width: 70em)').matches;

const clampGbPanelWidth = (panel: GbResizablePanel, value: unknown) => {
  const parsed = Number(value);
  const fallback = GB_DEFAULT_PANEL_WIDTHS[panel];
  const cssMinimum = panel === 'results' && gbLayoutRef.value
    ? (Number.parseFloat(getComputedStyle(gbLayoutRef.value).fontSize) || 16) * 20
    : GB_MIN_PANEL_WIDTHS[panel];
  return Math.round(Math.min(GB_MAX_PANEL_WIDTHS[panel], Math.max(cssMinimum, Number.isFinite(parsed) ? parsed : fallback)));
};

const loadGbPanelWidths = (): Record<GbResizablePanel, number> => {
  try {
    const stored = JSON.parse(localStorage.getItem(GB_COLUMN_WIDTHS_STORAGE) || '{}') as Partial<Record<GbResizablePanel, number>>;
    return {
      categories: clampGbPanelWidth('categories', stored.categories),
      results: clampGbPanelWidth('results', stored.results),
    };
  } catch {
    return { ...GB_DEFAULT_PANEL_WIDTHS };
  }
};

const gbLayoutRef = ref<HTMLElement | null>(null);
const gbPanelWidths = reactive(loadGbPanelWidths());
const activeGbPanelResize = ref<GbResizablePanel | null>(null);
const gbLayoutColumnStyle = computed(() => ({
  '--gb-categories-width': `${gbPanelWidths.categories}px`,
  '--gb-results-width': `${gbPanelWidths.results}px`,
}));

let gbPanelResizeObserver: ResizeObserver | null = null;
let gbPanelResizePointerId: number | null = null;
let gbPanelResizeStartX = 0;
let gbPanelResizeStartWidth = 0;

const getGbAvailableSidePanelWidth = () => {
  const layoutWidth = gbLayoutRef.value?.clientWidth || window.innerWidth;
  return Math.max(
    GB_MIN_PANEL_WIDTHS.categories + GB_MIN_PANEL_WIDTHS.results,
    layoutWidth - GB_COLUMN_RESIZERS_WIDTH - GB_MIN_DETAIL_WIDTH,
  );
};

const constrainGbPanelWidths = () => {
  if (gbUsesStackedLayout()) return;

  gbPanelWidths.categories = clampGbPanelWidth('categories', gbPanelWidths.categories);
  gbPanelWidths.results = clampGbPanelWidth('results', gbPanelWidths.results);

  const available = getGbAvailableSidePanelWidth();
  const currentTotal = gbPanelWidths.categories + gbPanelWidths.results;
  if (currentTotal <= available) return;

  const categoriesExtra = gbPanelWidths.categories - GB_MIN_PANEL_WIDTHS.categories;
  const resultsExtra = gbPanelWidths.results - GB_MIN_PANEL_WIDTHS.results;
  const availableExtra = Math.max(0, available - GB_MIN_PANEL_WIDTHS.categories - GB_MIN_PANEL_WIDTHS.results);
  const extraTotal = categoriesExtra + resultsExtra;
  const scale = extraTotal > 0 ? Math.min(1, availableExtra / extraTotal) : 0;
  gbPanelWidths.categories = Math.round(GB_MIN_PANEL_WIDTHS.categories + categoriesExtra * scale);
  gbPanelWidths.results = Math.round(GB_MIN_PANEL_WIDTHS.results + resultsExtra * scale);
};

const persistGbPanelWidths = () => {
  localStorage.setItem(GB_COLUMN_WIDTHS_STORAGE, JSON.stringify({
    categories: gbPanelWidths.categories,
    results: gbPanelWidths.results,
  }));
};

const setGbPanelWidth = (panel: GbResizablePanel, requestedWidth: number) => {
  const otherPanel: GbResizablePanel = panel === 'categories' ? 'results' : 'categories';
  const availableForPanel = Math.max(GB_MIN_PANEL_WIDTHS[panel], getGbAvailableSidePanelWidth() - gbPanelWidths[otherPanel]);
  gbPanelWidths[panel] = Math.min(clampGbPanelWidth(panel, requestedWidth), availableForPanel);
};

const startGbPanelResize = (panel: GbResizablePanel, event: PointerEvent) => {
  if (event.button !== 0 || gbUsesStackedLayout()) return;
  event.preventDefault();
  activeGbPanelResize.value = panel;
  gbPanelResizePointerId = event.pointerId;
  gbPanelResizeStartX = event.clientX;
  gbPanelResizeStartWidth = gbPanelWidths[panel];
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
};

const moveGbPanelResize = (event: PointerEvent) => {
  const panel = activeGbPanelResize.value;
  if (!panel || event.pointerId !== gbPanelResizePointerId) return;
  const pointerDelta = event.clientX - gbPanelResizeStartX;
  setGbPanelWidth(panel, gbPanelResizeStartWidth + pointerDelta);
};

const stopGbPanelResize = (event?: PointerEvent) => {
  if (!activeGbPanelResize.value) return;
  const target = event?.currentTarget as HTMLElement | undefined;
  const pointerId = gbPanelResizePointerId;
  activeGbPanelResize.value = null;
  gbPanelResizePointerId = null;
  if (target && pointerId !== null && target.hasPointerCapture(pointerId)) {
    target.releasePointerCapture(pointerId);
  }
  persistGbPanelWidths();
};

const resetGbPanelWidth = (panel: GbResizablePanel) => {
  setGbPanelWidth(panel, GB_DEFAULT_PANEL_WIDTHS[panel]);
  persistGbPanelWidths();
};

const onGbPanelResizeKeydown = (panel: GbResizablePanel, event: KeyboardEvent) => {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
  event.preventDefault();
  const direction = event.key === 'ArrowRight' ? 1 : -1;
  setGbPanelWidth(panel, gbPanelWidths[panel] + direction * 16);
  persistGbPanelWidths();
};

const persistGbBlocklist = () => {
  localStorage.setItem(GAMEBANANA_BLOCKLIST_STORAGE, JSON.stringify(gbBlocklist));
};

const closeGbBlockMenu = () => {
  gbBlockMenu.visible = false;
  gbBlockMenu.target = null;
};

const openGbBlockMenu = (event: MouseEvent, target: GbBlockTarget) => {
  event.preventDefault();
  event.stopPropagation();
  gbBlockMenu.target = target;
  gbBlockMenu.x = Math.min(event.clientX, window.innerWidth - 190);
  gbBlockMenu.y = Math.min(event.clientY, window.innerHeight - 52);
  gbBlockMenu.visible = true;
};

const addGbBlockedItem = (collection: GbBlockedItem[], item: GbBlockedItem) => {
  if (!collection.some((existing) => existing.key === item.key)) collection.push(item);
  persistGbBlocklist();
};

const applyGbBlockTarget = () => {
  const target = gbBlockMenu.target;
  if (!target) return;
  if (target.kind === 'category') {
    addGbBlockedItem(gbBlocklist.categories, {
      key: categoryBlockKey(target.category._idRow),
      label: target.category._sName || `#${target.category._idRow}`,
    });
    if (selectedCategoryId.value === target.category._idRow) selectCategory(null);
  } else if (target.kind === 'author') {
    addGbBlockedItem(gbBlocklist.authors, { key: authorBlockKey(target.mod), label: target.mod.author });
  } else {
    addGbBlockedItem(gbBlocklist.mods, { key: String(target.mod.id), label: target.mod.title });
  }
  if (target.kind !== 'category' && selectedModId.value === target.mod.id) {
    selectedModId.value = null;
    detail.value = null;
  }
  ElMessage.success(t('gameBanana.blocked'));
  closeGbBlockMenu();
};

const removeGbBlockedItem = (kind: keyof GbBlocklist, key: string) => {
  const collection = gbBlocklist[kind];
  const index = collection.findIndex((item) => item.key === key);
  if (index >= 0) collection.splice(index, 1);
  persistGbBlocklist();
};

const gbBlockedCount = computed(() => gbBlocklist.categories.length + gbBlocklist.authors.length + gbBlocklist.mods.length);
const gbBlocklistKinds: Array<keyof GbBlocklist> = ['categories', 'authors', 'mods'];
const closeGbTransientMenus = () => closeGbBlockMenu();

onMounted(() => {
  window.addEventListener('keydown', onGlobalKeydown);
  window.addEventListener('keyup', onGlobalKeyup);
  window.addEventListener('blur', onGlobalBlur);
  window.addEventListener('pointerdown', closeGbTransientMenus);
  gbPanelResizeObserver = new ResizeObserver(constrainGbPanelWidths);
  if (gbLayoutRef.value) gbPanelResizeObserver.observe(gbLayoutRef.value);
  constrainGbPanelWidths();
  void listenForInstallProgress();
  void syncGameTarget();
  if (appSettings.gamebananaTranslationApiKey.trim() && appSettings.gamebananaTranslationApiUrl.trim()) {
    void fetchTranslationModels();
  }
});

onBeforeUnmount(() => {
  pageDisposed = true;
  ++updatesRequestId;
  ++detailRequestId;
  ++commentsRequestId;
  ++modsRequestId;
  ++categoriesRequestId;
  window.removeEventListener('keydown', onGlobalKeydown);
  window.removeEventListener('keyup', onGlobalKeyup);
  window.removeEventListener('blur', onGlobalBlur);
  window.removeEventListener('pointerdown', closeGbTransientMenus);
  unlistenDownloadProgress?.();
  unlistenInstallProgress?.();
  gbPanelResizeObserver?.disconnect();
  gbPanelResizeObserver = null;
  stopGbPanelResize();
  clearTranslations();
});
</script>

<template>
  <div class="page-container gamebanana-page" @pointerover="rememberHoveredText" @pointermove="rememberHoveredText" @mouseleave="clearHoveredText">
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
    <section class="gb-controls glass-panel">
      <label class="gb-field gb-search-field">
        <span>{{ t('gameBanana.search') }}</span>
        <input v-model="searchQuery" type="search" :placeholder="t('gameBanana.searchPlaceholder')" @keyup.enter="loadMods(1)" />
      </label>
      <button type="button" class="gb-button gb-button--primary" :disabled="loadingMods" @click="loadMods(1)">
        {{ loadingMods ? t('gameBanana.loading') : t('gameBanana.searchAction') }}
      </button>
      <label v-if="showGameIdInput" class="gb-field gb-id-field">
        <span>{{ t('gameBanana.gameId') }}</span>
        <input v-model.number="gameId" type="number" min="1" :placeholder="t('gameBanana.gameId')" @keyup.enter="applyTarget" />
      </label>
      <label class="gb-field gb-size-field">
        <span>{{ t('gameBanana.perPage') }}</span>
        <el-select v-model="pageSize" class="gb-select" popper-class="gamebanana-select-popper" @change="loadMods(1)">
          <el-option v-for="size in PAGE_SIZE_OPTIONS" :key="size" :label="String(size)" :value="size" />
        </el-select>
      </label>
      <label class="gb-field gb-nsfw-field" :title="t('gameBanana.nsfwImageBlur')">
        <span>{{ t('gameBanana.nsfwImageBlur') }}</span>
        <el-radio-group v-model="gameBananaBlurMode" class="gb-nsfw-mode">
          <el-radio-button value="all">{{ t('gameBanana.blurAllImages') }}</el-radio-button>
          <el-radio-button value="nsfw" :disabled="!appSettings.gamebananaShowNsfw">{{ t('gameBanana.blurNsfwImages') }}</el-radio-button>
          <el-radio-button value="none">{{ t('gameBanana.blurNoImages') }}</el-radio-button>
        </el-radio-group>
      </label>
      <label class="gb-field gb-show-nsfw-field">
        <span>{{ t('gameBanana.nsfwShown') }}</span>
        <el-radio-group :model-value="appSettings.gamebananaShowNsfw" class="gb-nsfw-visibility" @change="(value: string | number | boolean | undefined) => setGameBananaShowNsfw(appSettings, value === true)">
          <el-radio-button :value="true">{{ t('gameBanana.nsfwShow') }}</el-radio-button>
          <el-radio-button :value="false">{{ t('gameBanana.nsfwHide') }}</el-radio-button>
        </el-radio-group>
      </label>
      <button type="button" class="gb-button" :disabled="loadingCategories || loadingMods" @click="applyTarget">
        {{ t('gameBanana.refresh') }}
      </button>
      <button type="button" class="gb-button" @click="openExternal(gameUrl)">{{ t('gameBanana.openGamePage') }}</button>
      <button type="button" class="gb-button" :class="{ active: showTranslationSettings }" @click="showTranslationSettings = !showTranslationSettings">
        {{ t('gameBanana.translationSettings') }}
      </button>
      <button type="button" class="gb-button" @click="showGbBlocklist = true">
        {{ t('gameBanana.blocklist') }}<span v-if="gbBlockedCount"> · {{ gbBlockedCount }}</span>
      </button>
    </section>

    <el-dialog v-model="showTranslationSettings" class="glass-dialog gb-translation-dialog" :title="t('gameBanana.translationSettings')" width="620px" align-center>
    <div class="gb-translation-settings">
      <div class="gb-translation-settings-head">
        <strong>{{ t('gameBanana.translationSettings') }}</strong>
        <span>{{ t('gameBanana.translationHint', { shortcut: appSettings.gamebananaTranslationShortcut || 'Ctrl' }) }}</span>
      </div>
      <label class="gb-field gb-translation-enabled">
        <span>{{ t('gameBanana.translationEnabled') }}</span>
        <el-switch v-model="appSettings.gamebananaTranslationEnabled" />
      </label>
      <label class="gb-field gb-translation-enabled" :title="t('gameBanana.translationRichTextHint')">
        <span>{{ t('gameBanana.translationRichText') }}</span>
        <el-switch v-model="appSettings.gamebananaTranslationRichText" />
      </label>
      <label class="gb-field gb-translation-enabled">
        <span>{{ t('gameBanana.translationUseContext') }}</span>
        <el-switch v-model="appSettings.gamebananaTranslationUseContext" />
      </label>
      <label class="gb-field" :class="{ 'has-font-warning': translationFontWarning }">
        <span>{{ t('gameBanana.translationFontFamily') }}</span>
        <input v-model="appSettings.gamebananaTranslationFontFamily" type="text" :placeholder="t('gameBanana.translationDefault')" />
        <small v-if="translationFontWarning">{{ t(`gameBanana.translationFontWarnings.${translationFontWarning}`) }}</small>
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationFontSize') }}</span>
        <input v-model="appSettings.gamebananaTranslationFontSize" type="text" :placeholder="t('gameBanana.translationDefault')" />
      </label>
      <label class="gb-field" :class="{ 'has-font-warning': translationColorWarning }">
        <span>{{ t('gameBanana.translationColor') }}</span>
        <input v-model="appSettings.gamebananaTranslationColor" type="text" placeholder="#FFFFFFB3" spellcheck="false" />
        <small v-if="translationColorWarning">{{ t('gameBanana.translationColorInvalid') }}</small>
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationFontStyle') }}</span>
        <el-select v-model="appSettings.gamebananaTranslationFontStyle" class="gb-select">
          <el-option v-for="style in translationFontStyleOptions" :key="style" :value="style" :label="t(`gameBanana.translationFontStyles.${style}`)" />
        </el-select>
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationFailureMode') }}</span>
        <el-select v-model="appSettings.gamebananaTranslationFailureMode" class="gb-select">
          <el-option v-for="mode in translationFailureModeOptions" :key="mode" :value="mode" :label="t(`gameBanana.translationFailureModes.${mode}`)" />
        </el-select>
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationProvider') }}</span>
        <el-select v-model="appSettings.gamebananaTranslationProvider" class="gb-select" popper-class="gamebanana-select-popper" @change="onTranslationProviderChange">
          <el-option label="OpenAI" value="openai" />
          <el-option :label="t('gameBanana.translationProviderCompatible')" value="compatible" />
          <el-option label="Claude" value="claude" />
          <el-option label="DeepSeek" value="deepseek" />
          <el-option label="Gemini" value="gemini" />
          <el-option label="Google Cloud Translate" value="google" />
        </el-select>
      </label>
      <label class="gb-field gb-translation-url">
        <span>{{ t('gameBanana.translationApiUrl') }}</span>
        <input v-model="appSettings.gamebananaTranslationApiUrl" type="url" placeholder="https://api.openai.com/v1" @change="fetchTranslationModels" />
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationApiKey') }}</span>
        <input v-model="appSettings.gamebananaTranslationApiKey" type="password" autocomplete="off" @change="fetchTranslationModels" />
      </label>
      <label class="gb-field gb-translation-model">
        <span>{{ t('gameBanana.translationModel') }}</span>
        <span class="gb-model-control">
          <el-select
            v-model="appSettings.gamebananaTranslationModel"
            class="gb-select"
            popper-class="gamebanana-select-popper"
            filterable
            allow-create
            default-first-option
            :loading="loadingTranslationModels"
            :placeholder="t('gameBanana.translationModel')"
          >
            <el-option
              v-if="appSettings.gamebananaTranslationModel && !translationModelOptions.some((option) => option.value === appSettings.gamebananaTranslationModel)"
              :label="appSettings.gamebananaTranslationModel"
              :value="appSettings.gamebananaTranslationModel"
            />
            <el-option v-for="option in translationModelOptions" :key="option.value" :label="option.label" :value="option.value" />
          </el-select>
          <button type="button" class="gb-model-refresh" :disabled="loadingTranslationModels" :title="t('gameBanana.translationModelsRefresh')" @click="fetchTranslationModels">↻</button>
        </span>
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationTarget') }}</span>
        <input v-model="appSettings.gamebananaTranslationTargetLanguage" type="text" />
      </label>
      <label class="gb-field">
        <span>{{ t('gameBanana.translationShortcut') }}</span>
        <input v-model="appSettings.gamebananaTranslationShortcut" type="text" placeholder="Ctrl" />
      </label>
      <span v-if="translationStatus || translationModelsStatus" class="gb-translation-status">{{ translationStatus || translationModelsStatus }}</span>
    </div>
    <template #footer><el-button type="primary" @click="showTranslationSettings = false">{{ t('gameBanana.done') }}</el-button></template>
    </el-dialog>

    <el-dialog v-model="showGbBlocklist" class="glass-dialog gb-blocklist-dialog" :title="t('gameBanana.blocklist')" width="560px" align-center>
      <div class="gb-blocklist-groups">
        <section v-for="kind in gbBlocklistKinds" :key="kind">
          <h3>{{ t(`gameBanana.blocklistKinds.${kind}`) }} · {{ gbBlocklist[kind].length }}</h3>
          <div v-if="gbBlocklist[kind].length" class="gb-blocklist-items">
            <span v-for="item in gbBlocklist[kind]" :key="item.key">
              <b>{{ item.label }}</b>
              <button type="button" :title="t('gameBanana.unblock')" @click="removeGbBlockedItem(kind, item.key)">×</button>
            </span>
          </div>
          <p v-else>{{ t('gameBanana.blocklistEmpty') }}</p>
        </section>
      </div>
      <template #footer><el-button type="primary" @click="showGbBlocklist = false">{{ t('gameBanana.done') }}</el-button></template>
    </el-dialog>

    <Teleport to="body">
      <div
        v-if="gbBlockMenu.visible && gbBlockMenu.target"
        class="gb-block-context-menu glass-context-menu"
        :style="{ left: `${gbBlockMenu.x}px`, top: `${gbBlockMenu.y}px` }"
        @pointerdown.stop
      >
        <button type="button" @click="applyGbBlockTarget">
          {{ t(`gameBanana.blockActions.${gbBlockMenu.target.kind}`) }}
        </button>
      </div>
    </Teleport>

    <p v-if="errorMessage" class="gb-error">{{ errorMessage }}</p>

    <main
      ref="gbLayoutRef"
      class="gb-layout"
      :class="{
        'is-resizing-columns': activeGbPanelResize,
      }"
      :style="gbLayoutColumnStyle"
    >
      <aside class="gb-panel gb-categories glass-panel">
        <div class="gb-panel-title">
          <span>{{ t('gameBanana.categories') }}</span>
          <small>{{ loadingCategories ? t('gameBanana.loading') : allCategories.length }}</small>
        </div>
        <div class="gb-category-list">
          <button type="button" class="gb-category" :class="{ active: selectedCategoryId === null }" @click="selectCategory(null)">
            {{ t('gameBanana.allCategories') }}
          </button>
          <div v-for="row in categoryRows" :key="row.node._idRow" class="gb-category-row" :style="{ '--category-depth': String(row.depth) }">
            <button
              v-if="categoryCanExpand(row.node)"
              type="button"
              class="gb-category-toggle"
              :class="{ expanded: expandedCategoryIds.has(row.node._idRow), loading: row.node.loadingChildren }"
              :aria-label="expandedCategoryIds.has(row.node._idRow) ? t('gameBanana.collapseCategory') : t('gameBanana.expandCategory')"
              @click="toggleCategory(row.node)"
            >›</button>
            <span v-else class="gb-category-toggle-spacer" aria-hidden="true" />
            <button
              type="button"
              class="gb-category"
              :class="{ active: selectedCategoryId === row.node._idRow }"
              data-gb-translate
              @click="selectCategory(row.node._idRow)"
              @contextmenu="openGbBlockMenu($event, { kind: 'category', category: row.node })"
            >
              <img v-if="row.node._sIconUrl" class="gb-category-icon" :src="row.node._sIconUrl" alt="" loading="lazy" />
              <span>{{ row.node._sName || `#${row.node._idRow}` }}</span>
            </button>
          </div>
        </div>
      </aside>

      <div
        class="gb-column-resizer gb-categories-resizer"
        :class="{ active: activeGbPanelResize === 'categories' }"
        role="separator"
        tabindex="0"
        aria-orientation="vertical"
        :aria-label="`${t('gameBanana.categories')} / ${t('gameBanana.results')}`"
        :aria-valuemin="GB_MIN_PANEL_WIDTHS.categories"
        :aria-valuemax="GB_MAX_PANEL_WIDTHS.categories"
        :aria-valuenow="gbPanelWidths.categories"
        @pointerdown.stop="startGbPanelResize('categories', $event)"
        @pointermove.stop="moveGbPanelResize"
        @pointerup.stop="stopGbPanelResize"
        @pointercancel.stop="stopGbPanelResize"
        @lostpointercapture.stop="stopGbPanelResize"
        @dblclick.stop="resetGbPanelWidth('categories')"
        @keydown="onGbPanelResizeKeydown('categories', $event)"
      ></div>

      <section class="gb-panel gb-results glass-panel">
        <div class="gb-panel-title gb-results-title">
          <div>
            <span data-gb-translate>{{ selectedCategoryName }}</span>
            <small>{{ t('gameBanana.resultCount', { count: visibleCountText, total: formatNumber(totalRecords) }) }}</small>
          </div>
          <span>{{ t('gameBanana.page', { current: currentPage }) }}</span>
        </div>

        <div v-if="loadingMods" class="gb-empty">{{ t('gameBanana.loading') }}</div>
        <div v-else-if="visibleMods.length" class="gb-mod-list">
          <article
            v-for="mod in visibleMods"
            :key="mod.id"
            class="gb-mod-card"
            :class="{ active: selectedModId === mod.id }"
            role="button"
            tabindex="0"
            @click="selectMod(mod)"
            @contextmenu="openGbBlockMenu($event, { kind: 'mod', mod })"
            @keydown.enter.prevent="selectMod(mod)"
            @keydown.space.prevent="selectMod(mod)"
          >
            <div
              class="gb-mod-thumb"
              :class="{
                'is-image-blurred': isImageBlurred(mod.thumbnailUrl, mod.isNsfw),
                'can-hover-reveal': appSettings.revealBlurredImagesOnHover,
                revealed: isImageRevealed(mod.thumbnailUrl),
              }"
              @click="onModThumbClick($event, mod)"
            >
              <img v-if="mod.thumbnailUrl" :src="mod.thumbnailUrl" :alt="mod.title" loading="lazy" />
              <span v-else>{{ mod.title.slice(0, 1) }}</span>
              <span v-if="mod.isNsfw" class="gb-nsfw-badge">NSFW</span>
              <button
                v-if="shouldBlurImage(mod.isNsfw) && mod.thumbnailUrl"
                type="button"
                class="gb-img-toggle"
                :title="isImageRevealed(mod.thumbnailUrl) ? t('gameBanana.nsfwHideImage') : t('gameBanana.nsfwRevealImage')"
                :aria-label="isImageRevealed(mod.thumbnailUrl) ? t('gameBanana.nsfwHideImage') : t('gameBanana.nsfwRevealImage')"
                @click.stop="toggleImageBlur(mod.thumbnailUrl)"
              >
                <svg class="gb-eye gb-eye-open" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                <svg class="gb-eye gb-eye-closed" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
              </button>
            </div>
            <span class="gb-mod-copy">
              <strong :title="mod.title" data-gb-translate>{{ mod.title }}</strong>
              <small>
                  <button type="button" class="gb-author-link" :disabled="!mod.authorId && !mod.authorUrl" data-gb-translate @click.stop="openAuthorPage(mod.authorId, mod.authorUrl, mod.author)" @contextmenu="openGbBlockMenu($event, { kind: 'author', mod })">{{ mod.author }}</button>
              </small>
              <em v-if="mod.categoryName" data-gb-translate>{{ mod.categoryName }}</em>
              <p data-gb-translate>{{ mod.description || t('gameBanana.noDescription') }}</p>
              <time>{{ t('gameBanana.updated') }} · {{ formatDate(mod.updatedAt) }}</time>
            </span>
          </article>
        </div>
        <div v-else class="gb-empty">{{ t('gameBanana.empty') }}</div>

        <div class="gb-pagination">
          <button type="button" class="gb-button" :disabled="loadingMods || currentPage <= 1" @click="loadMods(currentPage - 1)">
            {{ t('gameBanana.previous') }}
          </button>
          <button type="button" class="gb-button" :disabled="loadingMods || !hasMore" @click="loadMods(currentPage + 1)">
            {{ t('gameBanana.next') }}
          </button>
        </div>
      </section>

      <div
        class="gb-column-resizer gb-detail-resizer"
        :class="{ active: activeGbPanelResize === 'results' }"
        role="separator"
        tabindex="0"
        aria-orientation="vertical"
        :aria-label="`${t('gameBanana.results')} / ${t('gameBanana.detail')}`"
        :aria-valuemin="GB_MIN_PANEL_WIDTHS.results"
        :aria-valuemax="GB_MAX_PANEL_WIDTHS.results"
        :aria-valuenow="gbPanelWidths.results"
        @pointerdown.stop="startGbPanelResize('results', $event)"
        @pointermove.stop="moveGbPanelResize"
        @pointerup.stop="stopGbPanelResize"
        @pointercancel.stop="stopGbPanelResize"
        @lostpointercapture.stop="stopGbPanelResize"
        @dblclick.stop="resetGbPanelWidth('results')"
        @keydown="onGbPanelResizeKeydown('results', $event)"
      ></div>

      <aside class="gb-panel gb-detail glass-panel">
        <div v-if="loadingDetail" class="gb-empty">{{ t('gameBanana.loading') }}</div>
        <template v-else-if="detail">
          <button v-if="detailLoadFailed" type="button" class="gb-button" @click="selectMod(detail)">{{ t('gameBanana.refresh') }}</button>
          <div class="gb-detail-head">
            <div>
              <h2 data-gb-translate>{{ detail.title }}</h2>
              <button type="button" class="gb-author-link" :disabled="!detail.authorId && !detail.authorUrl" data-gb-translate @click="openAuthorPage(detail.authorId, detail.authorUrl, detail.author)" @contextmenu="openGbBlockMenu($event, { kind: 'author', mod: detail })">
                {{ detail.author }}
              </button>
            </div>
            <span v-if="detail.isNsfw" class="gb-nsfw-badge">NSFW</span>
          </div>

          <div v-if="detail.screenshots.length" class="gb-screenshots">
            <button
              v-for="(image, index) in detail.screenshots.slice(0, 4)"
              :key="image"
              type="button"
              :class="{
                'gb-blur-active': shouldBlurImage(detail.isNsfw),
                'is-image-blurred': isImageBlurred(image, detail.isNsfw),
                'can-hover-reveal': appSettings.revealBlurredImagesOnHover,
                revealed: isImageRevealed(image),
              }"
              @click="onScreenshotClick(image)"
            >
              <img :src="image" :alt="`${detail.title} ${index + 1}`" loading="lazy" />
              <span v-if="shouldBlurImage(detail.isNsfw)" class="gb-img-eye" aria-hidden="true">
                <svg class="gb-eye gb-eye-open" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                <svg class="gb-eye gb-eye-closed" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
              </span>
            </button>
          </div>

          <div class="gb-stats">
            <span>{{ t('gameBanana.downloads') }}<strong>{{ formatNumber(detail.downloads) }}</strong></span>
            <span>{{ t('gameBanana.views') }}<strong>{{ formatNumber(detail.views) }}</strong></span>
            <span>{{ t('gameBanana.likes') }}<strong>{{ formatNumber(detail.likes) }}</strong></span>
          </div>

          <section v-if="updatesVisible" class="gb-updates">
            <div class="gb-comments-head">
              <h3>{{ t('gameBanana.updates') }}</h3>
              <button type="button" class="gb-link-button" :disabled="loadingUpdates" @click="loadUpdates(detail.id)">{{ t('gameBanana.refresh') }}</button>
            </div>
            <p v-if="updatesError" class="gb-error">{{ updatesError }}</p>
            <article v-for="update in updates" :key="update._idRow" class="gb-update">
              <header>
                <button type="button" class="gb-link-button" data-gb-translate @click="openExternal(`https://gamebanana.com/updates/${update._idRow}`)">{{ update._sName || t('gameBanana.updates') }}</button>
                <span v-if="update._sVersion">{{ update._sVersion }}</span>
                <time>{{ formatDate(asNumber(update._tsDateAdded)) }}</time>
              </header>
              <div class="gb-rich-text" @click="richTextClick" v-html="update.bodyHtml" />
            </article>
            <p v-if="loadingUpdates" class="gb-comments-empty">{{ t('gameBanana.loading') }}</p>
            <p v-else-if="!updates.length && !updatesError" class="gb-comments-empty">{{ t('gameBanana.noUpdates') }}</p>
            <button v-if="updatesHasMore" type="button" class="gb-button" :disabled="loadingUpdates" @click="loadUpdates(detail.id, updatesPage + 1, true)">{{ t('gameBanana.moreUpdates') }}</button>
          </section>

          <div v-if="detail.files.length" class="gb-files">
            <h3>{{ t('gameBanana.files') }}</h3>
            <div class="gb-install-target">
              <span class="gb-install-target-label">{{ t('gameBanana.installGroup') }}</span>
              <el-radio-group v-model="installGroupMode" class="gb-install-group-modes" size="small">
                <el-radio-button value="automatic">{{ t('gameBanana.installGroupAutomatic') }}</el-radio-button>
                <el-radio-button value="manual">{{ t('gameBanana.installGroupManual') }}</el-radio-button>
                <el-radio-button value="matching-icon">{{ t('gameBanana.installGroupMatchingIcon') }}</el-radio-button>
              </el-radio-group>
              <label v-if="installGroupMode === 'automatic'" class="gb-install-group">
                <input :value="gamebananaInstallGroup(detail)" type="text" readonly />
              </label>
              <label v-else-if="installGroupMode === 'manual'" class="gb-install-group">
                <input v-model="installGroup" type="text" :placeholder="t('gameBanana.installGroupManualPlaceholder')" />
              </label>
              <label v-else class="gb-install-group">
                <el-select
                  v-model="installGroup"
                  class="gb-install-path-select"
                  popper-class="gamebanana-select-popper"
                  :loading="loadingMatchingIconGroups"
                  :disabled="loadingMatchingIconGroups || matchingIconGroups.length === 0"
                  :placeholder="loadingMatchingIconGroups ? t('gameBanana.loadingMatchingIconPaths') : t('gameBanana.noMatchingIconPaths')"
                  :no-data-text="t('gameBanana.noMatchingIconPaths')"
                >
                  <el-option v-for="group in matchingIconGroups" :key="group.id" :label="group.id" :value="group.id" />
                </el-select>
                <button type="button" class="gb-install-path-refresh" :disabled="loadingMatchingIconGroups" @click="loadMatchingIconGroups">↻</button>
              </label>
            </div>
            <div v-for="file in detail.files" :key="file._idRow || file._sFile" class="gb-file">
              <span>
                <strong data-gb-translate>{{ file._sFile || t('gameBanana.download') }}</strong>
                <span v-if="file._sDescription" class="gb-file-description" data-gb-translate>{{ file._sDescription }}</span>
                <small>{{ formatFileSize(file._nFilesize) }} · {{ formatDate(asNumber(file._tsDateAdded)) }}</small>
              </span>
              <span class="gb-file-actions">
                <button
                  type="button"
                  class="gb-file-action gb-file-install-action"
                  :class="installButtonClass(file)"
                  :style="installButtonStyle(file)"
                  :disabled="isFileActionDisabled(file)"
                  @click="handleInstallAction(file)"
                >
                  <span>{{ installButtonLabel(file) }}</span>
                </button>
                <button type="button" class="gb-file-action" @click="openExternal(file._sDownloadUrl || detail.profileUrl)">{{ t('gameBanana.openDownload') }}</button>
              </span>
            </div>
          </div>

          <p v-if="detail.contentRatings.length" class="gb-ratings" data-gb-translate>{{ detail.contentRatings.join(' · ') }}</p>
          <div
            v-if="detail.descriptionHtml"
            class="gb-rich-text gb-description"
            @click="richTextClick"
            v-html="detail.descriptionHtml"
          />
          <p v-else class="gb-description" data-gb-translate>{{ detail.description || t('gameBanana.noDescription') }}</p>

          <section class="gb-comments">
            <div class="gb-comments-head">
              <h3>{{ t('gameBanana.comments') }}</h3>
              <button type="button" class="gb-link-button" :disabled="loadingComments" @click="loadComments(detail.id)">{{ t('gameBanana.refresh') }}</button>
            </div>
            <p v-if="commentsError" class="gb-error">{{ commentsError }}</p>
            <p v-if="loadingComments" class="gb-comments-empty">{{ t('gameBanana.loading') }}</p>
            <div v-else-if="comments.length" class="gb-comment-list">
              <article v-for="comment in comments" :key="comment.id" class="gb-comment" :class="{ reply: comment.depth > 0 }" :style="{ '--comment-depth': String(Math.min(comment.depth, 3)) }">
                <header>
                  <img v-if="comment.avatarUrl" :src="comment.avatarUrl" :alt="comment.author" />
                  <button type="button" class="gb-author-link" data-gb-translate @click="openAuthorPage(comment.authorId, comment.authorUrl, comment.author)">{{ comment.author }}</button>
                  <time>{{ formatDate(comment.postedAt) }}</time>
                  <span v-if="comment.score" class="gb-comment-score">+{{ comment.score }}</span>
                </header>
                <div class="gb-rich-text gb-comment-body" @click="richTextClick" v-html="comment.bodyHtml" />
                <button v-if="comment.replyCount > 0 && !comment.repliesLoaded" type="button" class="gb-comment-replies" :disabled="comment.loadingReplies" @click="loadReplies(comment)">
                  {{ comment.loadingReplies ? t('gameBanana.loading') : t('gameBanana.loadReplies', { count: comment.replyCount }) }}
                </button>
              </article>
            </div>
            <p v-else-if="!commentsError" class="gb-comments-empty">{{ t('gameBanana.noComments') }}</p>
            <button v-if="commentsHasMore" type="button" class="gb-button gb-comments-more" :disabled="loadingMoreComments" @click="loadComments(detail.id, commentsPage + 1, true)">
              {{ loadingMoreComments ? t('gameBanana.loading') : t('gameBanana.loadMoreComments') }}
            </button>
          </section>

          <div class="gb-detail-actions">
            <button type="button" class="gb-button gb-button--primary" @click="openExternal(detail.profileUrl)">
              {{ t('gameBanana.openModPage') }}
            </button>
          </div>
        </template>
        <div v-else class="gb-empty">{{ t('gameBanana.selectMod') }}</div>
      </aside>
    </main>
  </div>
</template>

<style scoped>
.gamebanana-page {
  position: relative;
  height: 100%;
  min-height: 0;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: var(--t-page-gap);
  overflow: hidden;
  padding: var(--t-page-padding);
  color: var(--t-page-text);
}

.gb-history-nav { position:absolute; top:8px; right:24px; left:24px; z-index:2; display:flex; align-items:center; gap:6px; min-width:0; }.gb-history-trail { display:flex; flex:1 1 auto; align-items:center; min-width:0; height:32px; overflow-x:auto; scrollbar-width:none; white-space:nowrap; border:1px solid rgba(255,255,255,.1); border-radius:7px; background:rgba(15,18,31,.32); backdrop-filter:blur(10px); -webkit-backdrop-filter:blur(10px); }.gb-history-trail::-webkit-scrollbar { display:none; }.gb-history-entry { display:block; flex:0 1 auto; max-width:220px; overflow:hidden; padding:0 8px; border:0; background:transparent; color:rgba(var(--theme-text-secondary-rgb),.72); font:inherit; font-size:11px; line-height:30px; text-align:left; text-overflow:ellipsis; white-space:nowrap; cursor:pointer; }.gb-history-entry:hover { color:rgba(var(--theme-text-primary-rgb),.96); }.gb-history-entry.is-current { color:rgba(var(--theme-surface-tint-rgb),.96); font-weight:700; }.gb-history-separator { flex:0 0 auto; color:rgba(var(--theme-text-secondary-rgb),.38); font-size:12px; }.gb-history-button { display:grid; flex:0 0 auto; width:32px; height:32px; place-items:center; padding:0; border:1px solid rgba(255,255,255,.13); border-radius:7px; background:rgba(15,18,31,.42); color:rgba(var(--theme-text-primary-rgb),.88); cursor:pointer; backdrop-filter:blur(10px); -webkit-backdrop-filter:blur(10px); }.gb-history-button:hover:not(:disabled) { border-color:rgba(var(--theme-surface-tint-rgb),.42); background:rgba(var(--theme-surface-tint-rgb),.18); }.gb-history-button:disabled { opacity:.4; cursor:default; }.gb-history-button svg { width:17px; height:17px; fill:none; stroke:currentColor; stroke-linecap:round; stroke-linejoin:round; stroke-width:2; }

.glass-panel {
  background: var(--t-page-panel-bg);
  border: var(--t-page-panel-border);
  box-shadow: var(--t-page-panel-shadow);
  backdrop-filter: var(--t-page-panel-blur);
  -webkit-backdrop-filter: var(--t-page-panel-blur);
}

.gb-controls,
.gb-layout,
.gb-panel,
.gb-results,
.gb-detail {
  min-height: 0;
}

.gb-detail h2,
.gb-files h3 {
  margin: 0;
}

.gb-link-button,
.gb-author-link {
  appearance: none;
  border: 0;
  padding: 0;
  background: transparent;
  color: rgba(var(--theme-surface-tint-rgb), 0.9);
  font: inherit;
  cursor: pointer;
}

.gb-link-button:hover,
.gb-author-link:hover { color: rgba(255,255,255,0.96); text-decoration: underline; }
.gb-author-link:disabled { color: rgba(var(--theme-text-secondary-rgb), 0.68); cursor: default; text-decoration: none; }

.gb-controls {
  display: flex;
  align-items: end;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 12px;
}
.gb-title-block { display: grid; flex: 0 0 auto; gap: 2px; min-width: 124px; }
.gb-title-block strong { color: rgba(var(--theme-text-primary-rgb), 0.94); font-size: 16px; letter-spacing: .02em; }
.gb-title-block span { max-width: 150px; overflow: hidden; color: rgba(var(--theme-text-secondary-rgb), .6); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }

.gb-field { display: grid; gap: 4px; min-width: 90px; }
.gb-field span { color: rgba(var(--theme-text-secondary-rgb), 0.64); font-size: 10px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase; }
.gb-search-field { flex: 1 1 260px; }
.gb-id-field { width: 104px; }
.gb-size-field { width: 78px; }
.gb-field input,
.gb-field select,
.gb-field :deep(.el-select__wrapper) {
  width: 100%;
  box-sizing: border-box;
  min-height: 30px;
  padding: 0 9px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  border-radius: 7px;
  outline: none;
  background: rgba(var(--theme-surface-tint-rgb), 0.055);
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  font: inherit;
  font-size: 12px;
}
.gb-field input:focus,
.gb-field select:focus,
.gb-field :deep(.el-select__wrapper.is-focused) { border-color: rgba(var(--theme-surface-tint-rgb), 0.62); box-shadow: 0 0 0 2px rgba(var(--theme-surface-tint-rgb), 0.11); }
.gb-field :deep(.el-select) { width: 100%; }
.gb-field :deep(.el-select__selected-item),
.gb-field :deep(.el-select__placeholder) { color: rgba(var(--theme-text-primary-rgb), .92); font-size: 12px; }
.gb-nsfw-field { min-width: 188px; }
.gb-show-nsfw-field { min-width: 116px; }
.gb-nsfw-visibility { display:flex; min-height:30px; overflow:hidden; border-radius:7px; }
.gb-nsfw-visibility :deep(.el-radio-button) { flex:1 1 0; min-width:0; }
.gb-nsfw-visibility :deep(.el-radio-button__inner) { display:flex; align-items:center; justify-content:center; width:100%; min-height:30px; padding:0 10px; border-color:rgba(255,255,255,.13); background:rgba(255,255,255,.055); color:rgba(var(--theme-text-primary-rgb),.86); font:inherit; font-size:12px; box-shadow:none; transition:background .16s ease,border-color .16s ease,color .16s ease; }
.gb-nsfw-visibility :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) { background:rgba(var(--theme-surface-tint-rgb),.20); border-color:rgba(var(--theme-surface-tint-rgb),.42); color:rgba(var(--theme-text-primary-rgb),.98); box-shadow:-1px 0 0 rgba(var(--theme-surface-tint-rgb),.42); }
.gb-nsfw-mode { display: flex; min-height: 30px; overflow: hidden; border-radius: 7px; }
.gb-nsfw-mode :deep(.el-radio-button) { flex: 1 1 0; min-width: 0; }
.gb-nsfw-mode :deep(.el-radio-button__inner) {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
	box-sizing: border-box;
  min-height: 30px;
  padding: 0 10px;
  border-color: rgba(255,255,255,.13);
  background: rgba(255,255,255,.055);
  color: rgba(var(--theme-text-primary-rgb),.86);
  font: inherit;
  font-size: 12px;
  line-height: 1;
  box-shadow: none;
  transition: background .16s ease, border-color .16s ease, color .16s ease;
}
.gb-nsfw-mode :deep(.el-radio-button:first-child .el-radio-button__inner) { border-radius: 7px 0 0 7px; }
.gb-nsfw-mode :deep(.el-radio-button:last-child .el-radio-button__inner) { border-radius: 0 7px 7px 0; }
.gb-nsfw-mode :deep(.el-radio-button__inner:hover) { background: rgba(var(--theme-surface-tint-rgb),.16); border-color: rgba(var(--theme-surface-tint-rgb),.38); color: rgba(var(--theme-text-primary-rgb),.96); }
.gb-nsfw-mode :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) { background: rgba(var(--theme-surface-tint-rgb),.20); border-color: rgba(var(--theme-surface-tint-rgb),.42); color: rgba(var(--theme-text-primary-rgb),.98); box-shadow: -1px 0 0 0 rgba(var(--theme-surface-tint-rgb),.42); }
.gb-nsfw-mode :deep(.el-radio-button.is-disabled .el-radio-button__inner) { opacity:.35; background:rgba(255,255,255,.025); color:rgba(var(--theme-text-secondary-rgb),.45); cursor:not-allowed; }
.gb-nsfw-mode :deep(.el-radio-button__inner),.gb-nsfw-visibility :deep(.el-radio-button__inner) { border:none!important; outline:none!important; box-shadow:none!important; }
.gb-nsfw-mode :deep(.el-radio-button.is-active .el-radio-button__inner),.gb-nsfw-visibility :deep(.el-radio-button.is-active .el-radio-button__inner) { box-shadow:none!important; }

.gb-translation-settings { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); align-items:end; gap:12px; }
.gb-translation-settings-head { grid-column:1 / -1; display:flex; align-items:baseline; gap:10px; color:rgba(var(--theme-text-primary-rgb),.88); font-size:12px; }
.gb-translation-settings-head span, .gb-translation-status { color: rgba(var(--theme-text-secondary-rgb),.62); font-size: 10px; }
.gb-translation-enabled { width: 86px; }
.gb-translation-url { min-width: 190px; }
.gb-translation-model { min-width: 145px; }
.gb-field.has-font-warning>span,.gb-field.has-font-warning>small { color:#e6a23c; }
.gb-field.has-font-warning input { border-color:rgba(230,162,60,.72); background:rgba(230,162,60,.08); box-shadow:0 0 0 1px rgba(230,162,60,.16); }
.gb-field>small { margin-top:1px; color:rgba(var(--theme-text-secondary-rgb),.58); font-size:9px; line-height:1.3; }
:global(.gb-translation-dialog.el-dialog) { overflow:hidden; border:1px solid rgba(var(--theme-surface-tint-rgb),.18); border-radius:14px; background:rgba(24,28,38,.88); box-shadow:0 22px 60px rgba(0,0,0,.42); backdrop-filter:blur(24px) saturate(1.25); -webkit-backdrop-filter:blur(24px) saturate(1.25); }
:global(.gb-translation-dialog .el-dialog__body) { max-height:min(68vh,650px); overflow:auto; padding-top:8px; }
.gb-blocklist-groups { display:grid; gap:14px; }
.gb-blocklist-groups section { display:grid; gap:7px; }
.gb-blocklist-groups h3 { margin:0; color:rgba(var(--theme-text-primary-rgb),.88); font-size:12px; }
.gb-blocklist-groups p { margin:0; color:rgba(var(--theme-text-secondary-rgb),.52); font-size:11px; }
.gb-blocklist-items { display:flex; flex-wrap:wrap; gap:6px; }
.gb-blocklist-items>span { display:flex; align-items:center; gap:7px; max-width:100%; padding:5px 6px 5px 9px; border:1px solid rgba(var(--theme-surface-tint-rgb),.16); border-radius:7px; background:rgba(var(--theme-surface-tint-rgb),.06); }
.gb-blocklist-items b { overflow:hidden; color:rgba(var(--theme-text-primary-rgb),.78); font-size:11px; font-weight:500; text-overflow:ellipsis; white-space:nowrap; }
.gb-blocklist-items button { width:20px; height:20px; padding:0; border:0; border-radius:5px; background:rgba(255,255,255,.06); color:rgba(var(--theme-text-secondary-rgb),.72); cursor:pointer; }
.gb-blocklist-items button:hover { background:rgba(var(--theme-danger-rgb),.16); color:rgba(var(--theme-text-primary-rgb),.96); }
:global(.gb-block-context-menu) { position:fixed; z-index:5000; min-width:178px; padding:5px; }
:global(.gb-block-context-menu button) { width:100%; padding:8px 10px; border:0; border-radius:6px; background:transparent; color:rgba(var(--theme-text-primary-rgb),.88); font:inherit; font-size:12px; text-align:left; cursor:pointer; }
:global(.gb-block-context-menu button:hover) { background:rgba(var(--theme-danger-rgb),.14); color:rgba(var(--theme-text-primary-rgb),.98); }
.gb-model-control { display: flex; align-items: stretch; gap: 5px; min-width: 0; text-transform: none; }
.gb-model-refresh { width: 30px; flex: 0 0 auto; border: 1px solid rgba(255,255,255,.13); border-radius: 7px; background: rgba(255,255,255,.055); color: rgba(var(--theme-text-primary-rgb),.82); font: inherit; font-size: 16px; line-height: 1; cursor: pointer; }.gb-model-refresh:hover:not(:disabled) { background: rgba(var(--theme-surface-tint-rgb),.16); border-color: rgba(var(--theme-surface-tint-rgb),.38); }.gb-model-refresh:disabled { opacity:.45; cursor: default; }

.gb-button {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	height: 30px;
  min-height: 30px;
  padding: 0 11px;
  border: 1px solid rgba(255,255,255,0.13);
  border-radius: 7px;
  background: rgba(255,255,255,0.055);
  color: rgba(var(--theme-text-primary-rgb), 0.86);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  transition: background .16s ease, border-color .16s ease, transform .16s ease;
}
.gb-button:hover:not(:disabled) { background: rgba(var(--theme-surface-tint-rgb), 0.16); border-color: rgba(var(--theme-surface-tint-rgb), 0.38); transform: translateY(-1px); }
.gb-button:disabled { opacity: 0.45; cursor: default; }
.gb-button--primary { background: rgba(var(--theme-surface-tint-rgb), 0.2); border-color: rgba(var(--theme-surface-tint-rgb), 0.42); }
.gb-nsfw-toggle-button { display: inline-flex; align-items: center; gap: 6px; }
.gb-nsfw-toggle-button .gb-nsfw-toggle-icon { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 2; }
.gb-nsfw-toggle-button.active { background: rgba(var(--theme-surface-tint-rgb), 0.2); border-color: rgba(var(--theme-surface-tint-rgb), 0.42); color: var(--theme-warning, #e6a23c); }

.gb-error { margin: -3px 2px 0; color: #ffabab; font-size: 12px; }

.gb-layout {
  flex: 1;
  display: grid;
  grid-template-areas: "categories categories-resizer results detail-resizer detail";
  grid-template-columns: var(--gb-categories-width, 230px) 10px minmax(20em, var(--gb-results-width, 320px)) 10px minmax(270px, 1fr);
  column-gap: 0;
  overflow: hidden;
}

.gb-panel { border-radius: var(--t-page-panel-radius); overflow: hidden; }
.gb-categories { grid-area: categories; }
.gb-results { grid-area: results; }
.gb-detail { grid-area: detail; }
.gb-categories,
.gb-results,
.gb-detail { display: flex; flex-direction: column; }

.gb-column-resizer {
  position: relative;
  z-index: 8;
  width: 10px;
  min-width: 10px;
  min-height: 0;
  padding: 0;
  border: 0;
  outline: none;
  background: transparent;
  cursor: col-resize;
  touch-action: none;
}
.gb-categories-resizer { grid-area: categories-resizer; }
.gb-detail-resizer { grid-area: detail-resizer; }
.gb-column-resizer::before {
  content: '';
  position: absolute;
  top: 12px;
  bottom: 12px;
  left: 4px;
  width: 2px;
  border-radius: 999px;
  background: rgba(255,255,255,.10);
  transition: background .16s ease, box-shadow .16s ease, transform .16s ease;
}
.gb-column-resizer:hover::before,
.gb-column-resizer:focus-visible::before,
.gb-column-resizer.active::before {
  background: rgba(var(--theme-surface-tint-rgb),.78);
  box-shadow: 0 0 10px rgba(var(--theme-surface-tint-rgb),.42);
  transform: scaleX(1.5);
}
.gb-column-resizer:focus-visible { border-radius: 6px; box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb),.48); }
.gb-layout.is-resizing-columns,
.gb-layout.is-resizing-columns * { cursor: col-resize !important; user-select: none !important; }

.gb-panel-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex: 0 0 auto;
  padding: 12px 13px 9px;
  border-bottom: 1px solid rgba(255,255,255,0.07);
  color: rgba(var(--theme-text-primary-rgb), 0.88);
  font-size: 13px;
  font-weight: 700;
}
.gb-panel-title small,
.gb-results-title span:last-child { color: rgba(var(--theme-text-secondary-rgb), 0.58); font-size: 11px; font-weight: 500; }
.gb-results-title > div { display: grid; gap: 2px; }

.gb-category-list,
.gb-mod-list,
.gb-detail { overflow: auto; scrollbar-color: rgba(255,255,255,0.2) transparent; }
.gb-category-list { padding: 6px; }
.gb-category-row { display: flex; flex-wrap: wrap; align-items: stretch; min-width: 0; padding-left: calc(var(--category-depth) * 14px); }
.gb-category-toggle,
.gb-category-toggle-spacer { width: 18px; flex: 0 0 18px; align-self: stretch; }
.gb-category-toggle { border: 0; background: transparent; color: rgba(var(--theme-text-secondary-rgb), .65); font: inherit; font-size: 18px; line-height: 1; cursor: pointer; transition: color .15s ease, transform .15s ease; }
.gb-category-toggle:hover { color: rgba(var(--theme-text-primary-rgb), .96); }.gb-category-toggle.expanded { transform: rotate(90deg); }.gb-category-toggle.loading { opacity: .55; animation: gb-category-toggle-spin .8s linear infinite; pointer-events: none; }
@keyframes gb-category-toggle-spin { to { transform: rotate(360deg); } }
.gb-category {
  display: flex;
  align-items: center;
  width: calc(100% - 18px);
  min-width: 0;
  gap: 7px;
  padding: 8px 7px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: rgba(var(--theme-text-primary-rgb), 0.72);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}
.gb-category span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.gb-category-icon { width: 17px; height: 17px; flex: 0 0 auto; object-fit: contain; filter: drop-shadow(0 1px 2px rgba(0,0,0,.45)); }
.gb-category-row :global(.gb-inline-translation) { flex: 0 0 calc(100% - 18px); margin-left: 18px; }
.gb-category:hover { background: rgba(255,255,255,0.06); color: rgba(var(--theme-text-primary-rgb), 0.94); }
.gb-category.active { background: rgba(var(--theme-surface-tint-rgb), 0.16); color: rgba(var(--theme-surface-tint-rgb), 0.98); font-weight: 700; }

.gb-mod-list {
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
  flex: 1;
  overflow-x: hidden;
  padding: 7px;
}
.gb-mod-card {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  width: 100%;
  max-width: 100%;
  min-width: 0;
  box-sizing: border-box;
  gap: 10px;
  padding: 8px;
  border: 1px solid transparent;
  border-radius: 9px;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}
.gb-mod-card:focus-visible { outline: 2px solid rgba(var(--theme-surface-tint-rgb), .7); outline-offset: 2px; }
.gb-mod-card + .gb-mod-card { margin-top: 3px; }
.gb-mod-card:hover { background: rgba(255,255,255,0.055); }
.gb-mod-card.active { background: rgba(var(--theme-surface-tint-rgb), 0.12); border-color: rgba(var(--theme-surface-tint-rgb), 0.32); }
.gb-mod-thumb { position: relative; aspect-ratio: 16 / 9; overflow: hidden; border-radius: 6px; background: rgba(0,0,0,0.22); display: grid; place-items: center; color: rgba(var(--theme-surface-tint-rgb), 0.7); font-size: 24px; font-weight: 800; }
.gb-mod-thumb img { width: 100%; height: 100%; object-fit: cover; transition: filter .22s ease, transform .22s ease; }
.gb-mod-card:hover .gb-mod-thumb img { transform: scale(1.04); }
.gb-mod-thumb.is-nsfw-blurred img { filter: blur(18px) saturate(.75); transform: scale(1.16); }
.gb-mod-thumb.is-nsfw-blurred::after { content: 'NSFW'; position: absolute; inset: 0; z-index: 2; display: grid; place-items: center; background: rgba(8, 9, 14, .24); color: rgba(255,255,255,.9); font-size: 11px; letter-spacing: .16em; text-shadow: 0 1px 7px rgba(0,0,0,.95); }
.gb-nsfw-badge { position: absolute; top: 5px; right: 5px; padding: 2px 5px; border-radius: 4px; background: rgba(138, 27, 58, 0.84); color: #fff; font-size: 9px; font-weight: 800; letter-spacing: .06em; }
.gb-mod-copy { min-width: 0; display: grid; align-content: start; gap: 2px; }
.gb-mod-copy strong { overflow: hidden; color: rgba(var(--theme-text-primary-rgb), 0.9); font-size: 13px; line-height: 1.3; text-overflow: ellipsis; white-space: nowrap; }
.gb-mod-copy small,
.gb-mod-copy em,
.gb-mod-copy time { color: rgba(var(--theme-text-secondary-rgb), 0.62); font-size: 10px; font-style: normal; }
.gb-mod-copy em { color: rgba(var(--theme-surface-tint-rgb), 0.72); }
.gb-mod-copy p { display: -webkit-box; margin: 3px 0 0; overflow: hidden; color: rgba(var(--theme-text-secondary-rgb), 0.65); font-size: 11px; line-height: 1.35; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
.gb-pagination { display: flex; justify-content: flex-end; gap: 7px; flex: 0 0 auto; padding: 9px 12px; border-top: 1px solid rgba(255,255,255,0.07); }

.gb-detail { padding: 14px; gap: 13px; }
.gb-detail-head { display: flex; align-items: start; justify-content: space-between; gap: 10px; }
.gb-detail h2 { color: rgba(var(--theme-text-primary-rgb), 0.94); font-size: 17px; line-height: 1.25; }
.gb-detail-head .gb-author-link { margin-top: 4px; font-size: 12px; }
.gb-detail-head .gb-nsfw-badge { position: static; flex: 0 0 auto; }
.gb-screenshots { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px; }
.gb-screenshots button { position: relative; aspect-ratio: 16 / 9; overflow: hidden; padding: 0; border: 1px solid rgba(255,255,255,0.08); border-radius: 6px; background: rgba(0,0,0,0.2); cursor: zoom-in; }
.gb-screenshots img { width: 100%; height: 100%; object-fit: cover; transition: filter .22s ease, transform .22s ease; }
.gb-screenshots button:hover img { transform: scale(1.05); }
.gb-screenshots button.is-nsfw-blurred img { filter: blur(18px) saturate(.75); transform: scale(1.16); }
.gb-screenshots button.is-nsfw-blurred::after { content: 'NSFW'; position: absolute; inset: 0; z-index: 1; display: grid; place-items: center; background: rgba(8,9,14,.26); color: rgba(255,255,255,.9); font-size: 10px; font-weight: 800; letter-spacing: .14em; text-shadow: 0 1px 6px rgba(0,0,0,.9); }
.gb-mod-thumb.is-image-blurred img,
.gb-screenshots button.is-image-blurred img { filter: blur(18px) saturate(.75); transform: scale(1.16); }
.gb-mod-thumb.is-image-blurred::after,
.gb-screenshots button.is-image-blurred::after { content: ''; position: absolute; inset: 0; z-index: 2; display: grid; place-items: center; background: rgba(8,9,14,.24); color: rgba(255,255,255,.9); font-size: 11px; letter-spacing: .16em; text-shadow: 0 1px 7px rgba(0,0,0,.95); pointer-events: none; transition: opacity .22s ease; }
.gb-mod-thumb.is-image-blurred.can-hover-reveal:hover img,
.gb-screenshots button.is-image-blurred.can-hover-reveal:hover img { filter: blur(0) saturate(1); transform: scale(1.04); }
.gb-mod-thumb.is-image-blurred.can-hover-reveal:hover::after,
.gb-screenshots button.is-image-blurred.can-hover-reveal:hover::after { opacity: 0; }
.gb-img-toggle { position: absolute; right: 8px; bottom: 8px; z-index: 3; width: 30px; height: 30px; display: inline-flex; align-items: center; justify-content: center; padding: 0; border-radius: 50%; border: 1px solid rgba(255,255,255,.28); background: rgba(12,12,20,.62); color: rgba(255,255,255,.94); box-shadow: 0 4px 14px rgba(0,0,0,.35), inset 0 1px 0 rgba(255,255,255,.12); /* 性能：mod 列表每个 NSFW 缩略图一个该按钮，逐项模糊层会在滚动时引发闪烁，背景已足够不透明 */ cursor: pointer; transition: transform 160ms ease, background-color 160ms ease, border-color 160ms ease; }
.gb-img-toggle:hover { transform: scale(1.08); background: rgba(24,24,38,.78); border-color: rgba(255,255,255,.45); color: #fff; }
.gb-img-toggle:active { transform: scale(.96); }
.gb-eye { width: 16px; height: 16px; display: block; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 2; }
.gb-eye-closed { display: none; }
.gb-mod-thumb.revealed .gb-eye-open { display: none; }
.gb-mod-thumb.revealed .gb-eye-closed { display: block; }
.gb-screenshots button.gb-blur-active { cursor: pointer; }
.gb-screenshots button .gb-img-eye { position: absolute; right: 8px; bottom: 8px; z-index: 3; width: 30px; height: 30px; display: none; align-items: center; justify-content: center; border-radius: 50%; border: 1px solid rgba(255,255,255,.28); background: rgba(12,12,20,.62); color: rgba(255,255,255,.94); box-shadow: 0 4px 14px rgba(0,0,0,.35), inset 0 1px 0 rgba(255,255,255,.12); /* 性能：逐截图元素不加 backdrop-filter */ pointer-events: none; }
.gb-screenshots button.gb-blur-active .gb-img-eye { display: inline-flex; }
.gb-screenshots button.revealed .gb-eye-open { display: none; }
.gb-screenshots button.revealed .gb-eye-closed { display: block; }
.gb-rich-text :deep(img), .gb-comment-body :deep(img) { transition: filter .22s ease, transform .22s ease; }
.gb-rich-text :deep(img.is-blurred), .gb-comment-body :deep(img.is-blurred) { filter: blur(14px) saturate(.8); transform: scale(1.04); cursor: zoom-in; }
.gb-rich-text :deep(img.is-blurred.can-hover-reveal:hover), .gb-comment-body :deep(img.is-blurred.can-hover-reveal:hover) { filter: blur(0) saturate(1); transform: scale(1); }
.gb-stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; }
.gb-stats span { display: grid; gap: 3px; padding: 7px; border-radius: 6px; background: rgba(255,255,255,0.045); color: rgba(var(--theme-text-secondary-rgb), 0.6); font-size: 10px; }
.gb-stats strong { color: rgba(var(--theme-text-primary-rgb), 0.9); font-size: 13px; }
.gb-ratings { margin: -6px 0 0; color: #f1bc9b; font-size: 11px; }
.gb-description { margin: 0; color: rgba(var(--theme-text-primary-rgb), 0.74); font-size: 12px; line-height: 1.56; }
.gb-rich-text { overflow-wrap: anywhere; }
.gb-rich-text :deep(p), .gb-rich-text :deep(li), .gb-rich-text :deep(blockquote), .gb-rich-text :deep(pre) { margin: 0 0 9px; }
.gb-rich-text :deep(h1), .gb-rich-text :deep(h2), .gb-rich-text :deep(h3), .gb-rich-text :deep(h4) { margin: 11px 0 6px; color: rgba(var(--theme-text-primary-rgb),.9); line-height: 1.28; }
.gb-rich-text :deep(h1) { font-size: 18px; }.gb-rich-text :deep(h2) { font-size: 16px; }.gb-rich-text :deep(h3), .gb-rich-text :deep(h4) { font-size: 14px; }
.gb-rich-text :deep(ul), .gb-rich-text :deep(ol) { margin: 0 0 9px; padding-left: 20px; }
.gb-rich-text :deep(blockquote) { padding: 7px 10px; border-left: 2px solid rgba(var(--theme-surface-tint-rgb),.56); background: rgba(255,255,255,.035); }
.gb-rich-text :deep(pre), .gb-rich-text :deep(code) { font-family: Consolas, 'Cascadia Code', monospace; }.gb-rich-text :deep(pre) { padding: 8px; overflow: auto; border-radius: 6px; background: rgba(0,0,0,.22); white-space: pre-wrap; }
.gb-rich-text :deep(a) { color: rgba(var(--theme-surface-tint-rgb),.95); text-decoration: underline; cursor: pointer; }.gb-rich-text :deep(img) { max-width: 100%; height: auto; border-radius: 6px; }
.gb-rich-text :deep(table) { width: 100%; margin: 0 0 9px; border-collapse: collapse; font-size: 11px; }.gb-rich-text :deep(th), .gb-rich-text :deep(td) { padding: 5px 6px; border: 1px solid rgba(255,255,255,.12); text-align: left; }.gb-rich-text :deep(th) { background: rgba(255,255,255,.06); }
.gb-rich-text :deep(p:hover), .gb-rich-text :deep(li:hover), .gb-rich-text :deep(blockquote:hover), .gb-rich-text :deep(pre:hover), .gb-rich-text :deep(h1:hover), .gb-rich-text :deep(h2:hover), .gb-rich-text :deep(h3:hover), .gb-rich-text :deep(h4:hover), .gb-rich-text :deep(h5:hover), .gb-rich-text :deep(h6:hover), .gb-rich-text :deep(td:hover), .gb-rich-text :deep(th:hover) { outline: 1px dashed rgba(var(--theme-surface-tint-rgb),.36); outline-offset: 3px; }
.gb-files { display: grid; gap: 6px; }
.gb-files h3 { color: rgba(var(--theme-text-primary-rgb), 0.8); font-size: 12px; }
.gb-install-target { display:grid; gap:6px; padding:7px; border:1px solid rgba(255,255,255,.08); border-radius:7px; background:rgba(255,255,255,.025); }.gb-install-target-label { color:rgba(var(--theme-text-secondary-rgb),.66); font-size:10px; }.gb-install-group-modes { display:flex; min-height:27px; overflow:hidden; border-radius:6px; }.gb-install-group-modes :deep(.el-radio-button) { flex:1 1 0; min-width:0; }.gb-install-group-modes :deep(.el-radio-button__inner) { display:flex; align-items:center; justify-content:center; box-sizing:border-box; width:100%; min-height:27px; padding:0 7px; border:none!important; outline:none!important; background:rgba(255,255,255,.055); color:rgba(var(--theme-text-primary-rgb),.78); font:inherit; font-size:10px; line-height:1; box-shadow:none!important; transition:background .16s ease,color .16s ease; }.gb-install-group-modes :deep(.el-radio-button__inner:hover) { background:rgba(var(--theme-surface-tint-rgb),.16); color:rgba(var(--theme-text-primary-rgb),.96); }.gb-install-group-modes :deep(.el-radio-button.is-active .el-radio-button__inner) { background:rgba(var(--theme-surface-tint-rgb),.22); color:rgba(var(--theme-text-primary-rgb),.98); box-shadow:none!important; }.gb-install-group { display:flex; align-items:center; gap:6px; color:rgba(var(--theme-text-secondary-rgb),.66); font-size:10px; }.gb-install-group input { min-width:0; flex:1; height:27px; box-sizing:border-box; padding:0 8px; border:1px solid rgba(var(--theme-surface-tint-rgb),.14); border-radius:5px; outline:none; background:rgba(var(--theme-surface-tint-rgb),.055); color:rgba(var(--theme-text-primary-rgb),.86); font:inherit; }.gb-install-group input[readonly] { color:rgba(var(--theme-text-secondary-rgb),.72); cursor:default; }.gb-install-path-select { min-width:0; flex:1; }.gb-install-path-select :deep(.el-select__wrapper) { min-height:27px; padding:0 8px; border:1px solid rgba(var(--theme-surface-tint-rgb),.14); border-radius:5px; background:rgba(var(--theme-surface-tint-rgb),.055); box-shadow:none!important; }.gb-install-path-select :deep(.el-select__selected-item),.gb-install-path-select :deep(.el-select__placeholder) { color:rgba(var(--theme-text-primary-rgb),.86); font-size:10px; }.gb-install-path-select :deep(.el-select__wrapper.is-focused) { border-color:rgba(var(--theme-surface-tint-rgb),.48); box-shadow:0 0 0 2px rgba(var(--theme-surface-tint-rgb),.10)!important; }.gb-install-path-refresh { width:27px; height:27px; border:1px solid rgba(var(--theme-surface-tint-rgb),.14); border-radius:5px; background:rgba(var(--theme-surface-tint-rgb),.07); color:rgba(var(--theme-text-primary-rgb),.82); cursor:pointer; }.gb-install-path-refresh:hover:not(:disabled) { background:rgba(var(--theme-surface-tint-rgb),.16); }.gb-install-path-refresh:disabled { opacity:.45; cursor:default; }
.gb-updates { display: grid; gap: 8px; }
.gb-updates h3 { margin: 0; font-size: 13px; }
.gb-update { padding: 10px; border: 1px solid rgba(255,255,255,.08); border-radius: 7px; background: rgba(255,255,255,.025); overflow-wrap: anywhere; }
.gb-update header { display: flex; align-items: baseline; flex-wrap: wrap; gap: 8px; margin-bottom: 8px; }
.gb-update header button { text-align: left; }
.gb-update time { margin-left: auto; font-size: 10px; color: rgba(var(--theme-text-secondary-rgb),.56); }
.gb-file-description { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 11px; line-height: 1.5; color: rgba(var(--theme-text-primary-rgb),.75); }
.gb-file { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 7px 8px; border: 1px solid rgba(255,255,255,0.08); border-radius: 6px; background: rgba(255,255,255,0.035); color: inherit; text-align: left; }
.gb-file:hover { background: rgba(var(--theme-surface-tint-rgb), 0.1); border-color: rgba(var(--theme-surface-tint-rgb), 0.25); }
.gb-file > span:first-child { display: grid; min-width: 0; gap: 2px; }
.gb-file strong { overflow: hidden; color: rgba(var(--theme-text-primary-rgb), 0.82); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.gb-file small { color: rgba(var(--theme-text-secondary-rgb), 0.56); font-size: 10px; }
.gb-file-actions { display: flex; flex: 0 0 auto; gap: 5px; }
.gb-file-action { min-height: 24px; padding: 0 6px; border: 1px solid rgba(var(--theme-surface-tint-rgb),.25); border-radius: 5px; background: rgba(var(--theme-surface-tint-rgb),.09); color: rgba(var(--theme-surface-tint-rgb), .95); font: inherit; font-size: 10px; cursor: pointer; }
.gb-file-action:disabled { opacity: .72; cursor: default; }
.gb-file-install-action { position: relative; isolation: isolate; min-width: 78px; overflow: hidden; }
.gb-file-install-action::before, .gb-file-install-action::after { position: absolute; inset: 0 auto 0 0; width: 0; content: ''; opacity: 0; transition: width .16s linear; }
.gb-file-install-action::before { z-index: 0; background: rgba(var(--theme-surface-tint-rgb),.045); }
.gb-file-install-action::after { z-index: 1; background: rgba(var(--theme-surface-tint-rgb),.09); }
.gb-file-install-action.is-progress { background: rgba(var(--theme-surface-tint-rgb),.009); }
.gb-file-install-action.is-progress::before { width: var(--gb-download-progress); opacity: 1; }
.gb-file-install-action.is-progress::after { width: var(--gb-install-progress); opacity: 1; }
.gb-file-install-action.is-downloaded { border-color: rgba(255,255,255,.16); background: rgba(255,255,255,.09); color: rgba(var(--theme-text-secondary-rgb),.68); }
.gb-file-install-action > span { position: relative; z-index: 2; }
.gb-comments { display: grid; gap: 8px; padding-top: 3px; }.gb-comments-head { display: flex; align-items: center; justify-content: space-between; }.gb-comments h3 { margin: 0; color: rgba(var(--theme-text-primary-rgb),.84); font-size: 13px; }.gb-comments-empty { margin: 0; color: rgba(var(--theme-text-secondary-rgb),.56); font-size: 11px; }
.gb-comment-list { display: grid; gap: 7px; }.gb-comment { margin-left: calc(var(--comment-depth) * 11px); padding: 8px; border: 1px solid rgba(255,255,255,.07); border-radius: 7px; background: rgba(255,255,255,.025); }.gb-comment header { display: flex; align-items: center; gap: 6px; margin-bottom: 6px; }.gb-comment header img { width: 20px; height: 20px; border-radius: 50%; object-fit: cover; }.gb-comment time { margin-left: auto; color: rgba(var(--theme-text-secondary-rgb),.5); font-size: 10px; }.gb-comment-score { color: #a5eabf; font-size: 10px; }.gb-comment-body { color: rgba(var(--theme-text-primary-rgb),.72); font-size: 11px; line-height: 1.5; }.gb-comment-body :deep(p:last-child) { margin-bottom: 0; }.gb-comment-replies { margin-top: 5px; padding: 0; border: 0; background: none; color: rgba(var(--theme-surface-tint-rgb),.9); font: inherit; font-size: 10px; cursor: pointer; }.gb-comments-more { justify-self: start; }
:global(p.gb-inline-translation) { margin:-3px 0 9px!important; padding:0; border:0; background:none; color:rgba(var(--theme-surface-tint-rgb),.86); font:inherit; line-height:inherit; white-space:pre-wrap; }
:global(p.gb-inline-translation.is-loading) { display:flex; align-items:center; gap:7px; color:rgba(var(--theme-text-secondary-rgb),.62); }
:global(p.gb-inline-translation p) { margin:0; }
:global(p.gb-inline-translation ul),:global(p.gb-inline-translation ol) { margin:4px 0; padding-left:18px; }
:global(p.gb-inline-translation a) { color:inherit; text-decoration:underline; }
:global(.gb-translation-spinner) { width: 12px; height: 12px; flex: 0 0 auto; border: 2px solid rgba(var(--theme-surface-tint-rgb), .2); border-top-color: rgba(var(--theme-surface-tint-rgb), .95); border-radius: 50%; animation: gb-translation-spin .7s linear infinite; }
@keyframes gb-translation-spin { to { transform: rotate(360deg); } }
.gb-detail-actions { padding-top: 2px; }
.gb-empty { display: grid; flex: 1; place-items: center; padding: 24px; color: rgba(var(--theme-text-secondary-rgb), 0.58); font-size: 12px; text-align: center; }

:global(.gamebanana-select-popper.el-select__popper.el-popper) { overflow: hidden; border: 1px solid rgba(var(--theme-surface-tint-rgb),.18); border-radius: 9px; background: rgba(24,28,38,.72); box-shadow: 0 14px 32px rgba(0,0,0,.34); backdrop-filter: blur(18px) saturate(1.2); -webkit-backdrop-filter: blur(18px) saturate(1.2); }
:global(.gamebanana-select-popper .el-select-dropdown) { background: transparent; }
:global(.gamebanana-select-popper .el-select-dropdown__item) { color: rgba(255,255,255,.82); }
:global(.gamebanana-select-popper .el-select-dropdown__item:hover),
:global(.gamebanana-select-popper .el-select-dropdown__item.is-hovering) { background: rgba(var(--theme-surface-tint-rgb),.16); color: rgba(255,255,255,.96); }
:global(.gamebanana-select-popper .el-select-dropdown__item.is-selected) { background: rgba(var(--theme-surface-tint-rgb),.24); color: rgba(255,255,255,.98); }
:global(.gamebanana-select-popper .el-select-dropdown__empty) { color: rgba(255,255,255,.52); }

@media (max-width: 70em) {
  .gb-layout {
    grid-template-areas: "categories results" "detail detail";
    grid-template-columns: minmax(145px, .7fr) minmax(330px, 1.45fr);
    grid-template-rows: minmax(240px, 1fr) minmax(320px, 1fr);
    gap: var(--t-page-gap);
    overflow: auto;
  }
  .gb-column-resizer { display: none; }
  .gb-detail { min-height: 0; }
}

@media (max-width: 45em) {
  .gamebanana-page { overflow: hidden; padding: var(--t-page-padding-compact); }
  .gb-history-nav { right:10px; left:10px; }
  .gb-history-entry { max-width:150px; }
  .gb-controls { align-items: end; flex-wrap: wrap; }
  .gb-search-field { flex-basis: 100%; }
  .gb-translation-settings { grid-template-columns: repeat(2, minmax(0, 1fr)); }.gb-translation-settings-head { grid-column: 1 / -1; flex-direction: column; gap: 3px; }.gb-translation-url { min-width: 0; }
  .gb-layout { display: flex; flex-direction: column; overflow: auto; }
  .gb-categories { max-height: 190px; }
  .gb-results { min-height: 470px; }
  .gb-detail { min-height: 360px; }
}
</style>
