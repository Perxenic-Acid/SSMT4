<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch, onActivated, onBeforeUnmount } from 'vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openPath as openExternal } from '@tauri-apps/plugin-opener';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { exists, readDir, readTextFile, writeTextFile, mkdir, stat } from '@tauri-apps/plugin-fs';
import { join } from '@tauri-apps/api/path';
import { debugError, debugLog, debugWarn } from '../../utils/debugLog';
import { AppStateManager, BGType } from '../../store/AppStateManager';
import { setPendingXianZunPrompt } from '../../store/XianZunPendingPrompt';
import { PathHelper } from '../../helper/PathHelper';
import { ResourceManager } from '../../store/ResourceManager';
import { MigotoManager } from '../../store/MigotoManager';
import {
  type DrawIBConfigEntry,
  editableRowsToDrawIBConfigEntries,
  writeDrawIBConfigToWorkspace,
} from '../../common/DrawIBConfig';
import {
  open3DMigotoFolder,
  openModsFolder,
  openLatestFrameAnalysisFolder,
  openLatestFrameAnalysisLog,
  openLatestFrameAnalysisDeduped,
  openSSMT4GlobalConfigsFolder,
} from './WorkPage.OpenFolder';
import {
  runExtractModels,
  runFullExtract,
  handleTextureMenuCommand,
  type FullExtractDataTypeFilter,
} from './WorkPage.Extract';
import { generateIBSkipToMods } from './WorkPage.Generate';
import { moveFileToRecycleBin, moveDirectoryToRecycleBin } from '../../utils/RecycleBin';
import SidePanel from './SidePanel.vue';
import ExtractConfigSection from './ExtractConfigSection.vue';
import ConfigTables from './ConfigTables.vue';
import type { ModelRow, SkipRow, VSCheckRow } from './WorkPage.types';

const appSettings = AppStateManager.appSettings;
const { t } = useI18n();
const router = useRouter();
const DEFAULT_WORKSPACE_NAME = 'Default';
const WORKSPACE_ACCESS_API_URL = 'https://ssmt-workspace-api-dev.angeloyrd856.workers.dev';
const workspaceAccessProxyPort = (): number | undefined => {
  const port = Number(appSettings.workspaceAccessProxyPort);
  return Number.isInteger(port) && port >= 1 && port <= 65535 ? port : undefined;
};

type DrawIBSubmeshRange = {
  firstIndex: string;
  indexCount: string;
};

type WorkspaceTabMeta = {
  id: string;
  name: string;
};

type WorkPageTabsIndex = {
  activeTabId: string;
  tabs: WorkspaceTabMeta[];
};

type WorkPageTabConfig = {
  modelRows: ModelRow[];
  skipRows: SkipRow[];
  vsRows: VSCheckRow[];
  frameAnalysisFolderPath: string;
  selectedFrameAnalysis: string;
  extractPanelTab: string;
  fullExtractDataTypeFilter: FullExtractDataTypeFilter;
  workPageDrawerCollapsed: {
    workspace: boolean;
    workspaceSelector: boolean;
    commonFolders: boolean;
    textureExtract: boolean;
    otherFunctions: boolean;
  };
};

type WorkspaceFrameAnalysisConfig = {
  frameAnalysisFolderPath: string;
  selectedFrameAnalysis: string;
};

type WorkspaceArchiveBuildResult = {
  archivePath: string;
  sha256: string;
  size: number;
  uncompressedSize: number;
  fileCount: number;
};

type WorkspaceArchiveImportResult = {
  workspacePath: string;
  workspaceName: string;
  fileCount: number;
  totalSize: number;
};

type WorkspacePublishResult = {
  submissionId: string;
  entryId: string;
  status: string;
  metadataPath?: string;
};

type WorkspaceUploadProgress = {
  submissionId: string;
  completedBytes: number;
  totalBytes: number;
  completedParts: number;
  totalParts: number;
};

type LibraryIndexEntry = {
  entryId: string;
  workspaceName: string;
  description: string | null;
  attribution: string;
  uploadedAt: string;
  capturedAt: string | null;
  drawIB: string[];
  aliases: string[];
  metadataDownloadCount: number;
  fullPackageDownloadCount: number;
  fullDataAvailable: boolean;
  fullDataSize: number;
  availability: string;
  reviewState: string;
  metadataPath: string;
};

type LibraryIndex = { entries: LibraryIndexEntry[] };
type LibraryMetadata = {
  schemaVersion: number;
  entryId: string;
  gamePreset: string;
  workspaceName: string;
  description: string | null;
  uploadedAt: string;
  attribution: { mode: 'anonymous' | 'custom'; displayName?: string };
  workspaceAliases?: string[];
  lods: unknown[];
  fullData: { available: boolean; sha256?: string; size?: number };
};
type WorkspaceProvenance = { entryId: string; uploadedAt: string; attribution: string; aliases: string[] };

type WorkspaceDiskSpaceReport = {
  availableBytes: number | null;
  recommendedFreeBytes: number;
  belowRecommended: boolean;
};

type WorkspaceTabSaveSnapshot = {
  gameKey: string;
  workspaceName: string;
  activeTabId: string;
  tabs: WorkspaceTabMeta[];
  tabConfig: WorkPageTabConfig;
};

type WorkspaceMergeMode = 'chain' | 'zip';
type WorkspaceMergePreview = { conflictingHashes: string[] };
type WorkspaceMergeLod = {
  name: string;
  firstLodName: string | null;
  secondLodName: string | null;
};
type WorkspaceMergeResult = {
  workspaceName: string;
  lods: WorkspaceMergeLod[];
  copiedFileCount: number;
};

// Removed unused appWindow
const workspaceName = ref('');
const workspaceDraftName = ref('');

const workspaceOptions = ref<string[]>([]);
const workspaceModifiedTimes = ref<Record<string, number>>({});
const isScanningWorkspaces = ref(false);
const workspaceTabs = ref<WorkspaceTabMeta[]>([]);
const activeWorkspaceTabId = ref('');
const editingWorkspaceTabId = ref<string | null>(null);
const workspaceTabNameEditBackup = ref<Record<string, string>>({});
let workspaceTabSeed = 1;
const workspaceMergeDialog = ref(false);
const workspaceMergeOptions = ref<string[]>([]);
const workspaceMergeFirst = ref('');
const workspaceMergeSecond = ref('');
const workspaceMergeOutput = ref('');
const workspaceMergeMode = ref<WorkspaceMergeMode>('chain');
const workspaceMergeConflictingHashes = ref<string[]>([]);
const workspaceMergeHashPreferences = ref<Record<string, 'first' | 'second'>>({});
const workspaceMergeBusy = ref(false);
const workspaceMergePreviewLoading = ref(false);

const modelRows = ref<ModelRow[]>([{ drawIB: '', aliasName: '' }]);
const skipRows = ref<SkipRow[]>([{ skipIB: '', aliasName: '', indexCount: '', firstIndex: '' }]);
const vsRows = ref<VSCheckRow[]>([{ enabled: true, hash: '' }]);
const frameAnalysisOptions = ref<string[]>([]);
const selectedFrameAnalysis = ref('');
const frameAnalysisFolderPath = ref('');
const isRefreshing = ref(false);
const isExtracting = ref(false);
const latestExtractionLogPaths = ref<Record<string, string>>({});
const extractionLogWorkspaceKey = (gameName: string, workName: string) => `${gameName.trim()}\u0000${workName.trim()}`;
const currentExtractionLogWorkspaceKey = computed(() => extractionLogWorkspaceKey(appSettings.CurrentGameName, workspaceName.value));
const currentLatestExtractionLogPath = computed(() => latestExtractionLogPaths.value[currentExtractionLogWorkspaceKey.value] || '');
const isSpecificIbDumpToggling = ref(false);
const extractPanelTab = ref('drawib');
const fullExtractDataTypeFilter = ref<FullExtractDataTypeFilter>('all');
const useSpecificIbDump = ref(false);
const specificIbDumpRestoreAnalyseOptions = ref('');
const workPageDrawerCollapsed = ref({
  workspace: false,
  workspaceSelector: false,
  commonFolders: false,
  textureExtract: false,
  otherFunctions: false,
});

const FULL_EXTRACT_DATA_TYPE_FILTER_OPTIONS: Array<{ value: FullExtractDataTypeFilter; labelKey: string }> = [
  { value: 'all', labelKey: 'workPage.actions.fullExtractAllDataTypes' },
  { value: 'gpu-preskinning-only', labelKey: 'workPage.actions.fullExtractGpuPreSkinningOnly' },
  { value: 'cpu-preskinning-only', labelKey: 'workPage.actions.fullExtractCpuPreSkinningOnly' },
];

const normalizeFullExtractDataTypeFilter = (
  parsed?: Partial<WorkPageTabConfig> & { onlyGpuPreSkinning?: boolean }
): FullExtractDataTypeFilter => {
  if (parsed?.fullExtractDataTypeFilter === 'gpu-preskinning-only' || parsed?.fullExtractDataTypeFilter === 'cpu-preskinning-only') {
    return parsed.fullExtractDataTypeFilter;
  }

  if (parsed?.fullExtractDataTypeFilter === 'all') {
    return 'all';
  }

  return parsed?.onlyGpuPreSkinning === true ? 'gpu-preskinning-only' : 'all';
};

const normalizeWorkPageDrawerCollapsed = (
  parsed?: Partial<WorkPageTabConfig>['workPageDrawerCollapsed']
): WorkPageTabConfig['workPageDrawerCollapsed'] => ({
  workspace: parsed?.workspace === true,
  workspaceSelector: parsed?.workspaceSelector === true,
  commonFolders: parsed?.commonFolders === true,
  textureExtract: parsed?.textureExtract === true,
  otherFunctions: parsed?.otherFunctions === true,
});


const normalizeWorkspaceNameInput = (name: string): string => name.trim();

const findExistingWorkspaceOption = (name: string): string | undefined => {
  const normalizedName = normalizeWorkspaceNameInput(name).toLowerCase();
  if (!normalizedName) {
    return undefined;
  }

  return workspaceOptions.value.find((option) => option.trim().toLowerCase() === normalizedName);
};

const findWorkspaceMergeOption = (name: string): string | undefined => {
  const normalizedName = normalizeWorkspaceNameInput(name).toLowerCase();
  return workspaceMergeOptions.value.find((option) => option.trim().toLowerCase() === normalizedName);
};

const waitForInitialAppState = async (): Promise<void> => {
  if (AppStateManager.hasLoadedInitialState()) {
    return;
  }

  await new Promise<void>((resolve) => {
    const poll = () => {
      if (AppStateManager.hasLoadedInitialState()) {
        resolve();
        return;
      }

      window.setTimeout(poll, 50);
    };

    poll();
  });
};

// Single place that maps a game name to the workspace memory/folder key.
// 'Default' (and an empty selection) share the 'DefaultGame' bucket; names are
// trimmed so lookups never miss because of stray whitespace.
const normalizeWorkspaceGameKey = (gameName?: string): string => {
  const trimmedName = (gameName || '').trim();
  return trimmedName && trimmedName !== 'Default' ? trimmedName : 'DefaultGame';
};

const getCurrentWorkspaceMemoryGameKey = (): string => normalizeWorkspaceGameKey(appSettings.CurrentGameName);

const getRememberedWorkspaceForCurrentGame = (): string => {
  const gameKey = getCurrentWorkspaceMemoryGameKey();
  const byGame = appSettings.CurrentWorkSpaceByGame?.[gameKey] || '';
  // CurrentWorkSpace is a legacy/global mirror. Falling back to it here leaks
  // the previously selected game's workspace into a game with no memory yet.
  return normalizeWorkspaceNameInput(byGame);
};

const rememberWorkspaceForCurrentGame = (name: string): void => {
  const normalizedName = normalizeWorkspaceNameInput(name);
  const gameKey = getCurrentWorkspaceMemoryGameKey();

  appSettings.CurrentWorkSpace = normalizedName;
  appSettings.CurrentWorkSpaceByGame = {
    ...(appSettings.CurrentWorkSpaceByGame || {}),
    [gameKey]: normalizedName,
  };
};


const ensureTrailingEmptyRow = () => {
  debugLog('WorkPage', 'ensureTrailingEmptyRow - before', JSON.stringify(modelRows.value));
  if (modelRows.value.length === 0) {
    modelRows.value.push({ drawIB: '', aliasName: '' });
    debugLog('WorkPage', 'ensureTrailingEmptyRow - added initial empty row');
    return;
  }

  const last = modelRows.value[modelRows.value.length - 1];
  const lastHasContent = last.drawIB.trim() !== '' || last.aliasName.trim() !== '';

  if (lastHasContent) {
    modelRows.value.push({ drawIB: '', aliasName: '' });
    debugLog('WorkPage', 'ensureTrailingEmptyRow - appended trailing empty row');
  } else {
    while (modelRows.value.length > 1) {
      const prev = modelRows.value[modelRows.value.length - 2];
      const prevEmpty = prev.drawIB.trim() === '' && prev.aliasName.trim() === '';
      if (prevEmpty) {
        modelRows.value.splice(modelRows.value.length - 1, 1);
        debugLog('WorkPage', 'ensureTrailingEmptyRow - removed extra empty row');
      } else {
        break;
      }
    }
  }
  debugLog('WorkPage', 'ensureTrailingEmptyRow - after', JSON.stringify(modelRows.value));
};

const moveModelRow = (index: number, direction: 'up' | 'down') => {
  const rows = modelRows.value;
  const targetIndex = direction === 'up' ? index - 1 : index + 1;
  if (targetIndex < 0 || targetIndex >= rows.length) {
    return;
  }

  const current = rows[index];
  const target = rows[targetIndex];
  rows[index] = target;
  rows[targetIndex] = current;
};

const removeModelRow = (index: number) => {
  if (index < 0 || index >= modelRows.value.length) {
    return;
  }

  modelRows.value.splice(index, 1);
  ensureTrailingEmptyRow();
};

const ensureTrailingSkipRow = () => {
  const rows = skipRows.value;
  if (rows.length === 0) {
    rows.push({ skipIB: '', aliasName: '', indexCount: '', firstIndex: '' });
    return;
  }

  const last = rows[rows.length - 1];
  const lastHasContent = last.skipIB.trim() !== ''
    || last.aliasName.trim() !== ''
    || last.indexCount.trim() !== ''
    || last.firstIndex.trim() !== '';

  if (lastHasContent) {
    rows.push({ skipIB: '', aliasName: '', indexCount: '', firstIndex: '' });
  } else {
    while (rows.length > 1) {
      const prev = rows[rows.length - 2];
      const prevEmpty = prev.skipIB.trim() === ''
        && prev.aliasName.trim() === ''
        && prev.indexCount.trim() === ''
        && prev.firstIndex.trim() === '';
      if (prevEmpty) {
        rows.splice(rows.length - 1, 1);
      } else {
        break;
      }
    }
  }
};

const removeSkipRow = (index: number) => {
  if (index < 0 || index >= skipRows.value.length) {
    return;
  }

  skipRows.value.splice(index, 1);
  ensureTrailingSkipRow();
};

const moveSkipRow = (index: number, direction: 'up' | 'down') => {
  const targetIndex = direction === 'up' ? index - 1 : index + 1;
  if (index < 0 || targetIndex < 0 || targetIndex >= skipRows.value.length) return;
  const current = skipRows.value[index];
  skipRows.value[index] = skipRows.value[targetIndex];
  skipRows.value[targetIndex] = current;
};

const ensureTrailingVSRow = () => {
  const rows = vsRows.value;
  if (rows.length === 0) {
    rows.push({ enabled: true, hash: '' });
    return;
  }

  const last = rows[rows.length - 1];
  const lastHasContent = last.hash.trim() !== '';

  if (lastHasContent) {
    rows.push({ enabled: true, hash: '' });
  } else {
    while (rows.length > 1) {
      const prev = rows[rows.length - 2];
      const prevEmpty = prev.hash.trim() === '';
      if (prevEmpty) {
        rows.splice(rows.length - 1, 1);
      } else {
        break;
      }
    }
  }
};

const removeVSCheckRow = (index: number) => {
  if (index < 0 || index >= vsRows.value.length) {
    return;
  }

  vsRows.value.splice(index, 1);
  ensureTrailingVSRow();
};

const moveVSCheckRow = (index: number, direction: 'up' | 'down') => {
  const targetIndex = direction === 'up' ? index - 1 : index + 1;
  if (index < 0 || targetIndex < 0 || targetIndex >= vsRows.value.length) return;
  const current = vsRows.value[index];
  vsRows.value[index] = vsRows.value[targetIndex];
  vsRows.value[targetIndex] = current;
};

const getWorkspaceBaseDir = async (gameKey?: string) => {
  // If no valid game selected, fallback to a global placeholder so workspaces still work
  const gameName = normalizeWorkspaceGameKey(gameKey ?? appSettings.CurrentGameName);
  if (!appSettings.DBMTWorkFolder) {
    debugWarn('WorkPage', 'getWorkspaceBaseDir - cacheDir not configured');
    return undefined;
  }
  const p = await join(appSettings.DBMTWorkFolder, 'WorkSpace', gameName);
  debugLog('WorkPage', 'getWorkspaceBaseDir ->', p);
  return p;
};

const isWorkspaceLoading = ref(false);
const isSkipConfigLoading = ref(false);
const isVSConfigLoading = ref(false);
const isFrameAnalysisPathConfigLoading = ref(false);
const isWorkspaceTabConfigLoading = ref(false);
const isFrameAnalysisPathInvalid = ref(false);
const isWorkspaceTransitioning = ref(false);
const activeWorkspaceArchiveUpload = ref<{ workerUrl: string; archivePath: string } | null>(null);
const isWorkspaceArchiveUploadCancelling = ref(false);
const workspaceAccessDialog = ref<'upload' | 'download' | null>(null);
const workspaceAccessDescription = ref('');
const workspaceAccessPublishName = ref('');
const workspaceAccessAttribution = ref('');
const workspaceAccessAliases = ref('');
const workspaceAccessIncludeFullPackage = ref(true);
const workspacePublishing = ref(false);
const workspaceDownloadingEntry = ref<{ entryId: string; mode: 'metadata' | 'full' } | null>(null);
const workspaceUploadProgress = ref<WorkspaceUploadProgress | null>(null);
const unavailableWorkspaceFullPackages = ref<string[]>([]);
const workspaceAccessBusy = computed(() => workspacePublishing.value || workspaceDownloadingEntry.value !== null);
const workspaceProvenance = ref<WorkspaceProvenance | null>(null);
const workspaceLibraryQuery = ref('');
const workspaceLibrarySort = ref<'time' | 'downloads'>('time');
const workspaceLibraryEntries = ref<LibraryIndexEntry[]>([]);
const workspaceLibraryLoading = ref(false);
const workspaceUploadPercent = computed(() => {
  const progress = workspaceUploadProgress.value;
  if (!progress || progress.totalBytes <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((progress.completedBytes / progress.totalBytes) * 100)));
});
const workspaceEntryHasFullPackage = (entry: LibraryIndexEntry): boolean =>
  entry.fullDataAvailable && !unavailableWorkspaceFullPackages.value.includes(entry.entryId);
let lastInvalidFrameAnalysisWarnPath = '';

// Serial save queue with per-key coalescing. Bursts of debounced auto-saves
// (one per keystroke pause) collapse into the latest pending save per target,
// so switching workspaces never waits on a backlog of stale writes.
type WorkspaceSaveTask = {
  key: string;
  run: () => Promise<void>;
  done: () => void;
};
const pendingWorkspaceSaveTasks: WorkspaceSaveTask[] = [];
let workspaceSaveQueueDraining = false;
let unsettledWorkspaceSaveCount = 0;
const workspaceSaveIdleResolvers: Array<() => void> = [];

// Per-target save epochs. Clearing/deleting a workspace (or deleting a tab)
// bumps the epoch for that target; save tasks enqueued before the bump are
// skipped so they cannot resurrect deleted folders or stale tab configs.
const workspaceSaveEpochs = new Map<string, number>();
const workspaceSaveEpochKey = (gameKey: string, wsName: string): string => `${gameKey}\u0000${wsName}`;
const workspaceTabSaveEpochKey = (gameKey: string, wsName: string, tabId: string): string => `${gameKey}\u0000${wsName}\u0000${tabId}`;
const bumpWorkspaceSaveEpoch = (key: string): void => {
  workspaceSaveEpochs.set(key, (workspaceSaveEpochs.get(key) ?? 0) + 1);
};
const currentWorkspaceSaveEpoch = (key: string): number => workspaceSaveEpochs.get(key) ?? 0;

const WINDOWS_RESERVED_NAMES = new Set([
  'CON', 'PRN', 'AUX', 'NUL',
  'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
  'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9',
]);

const normalizeWorkspaceTabName = (name: string): string => name.trim();

const isValidWindowsFileName = (name: string): boolean => {
  if (!name) return false;
  if (name.length > 128) return false;
  if (/^[.\s]+$/.test(name)) return false;
  if (/[<>:"/\\|?*\x00-\x1F]/.test(name)) return false;
  if (name.endsWith(' ') || name.endsWith('.')) return false;
  const upper = name.toUpperCase();
  if (WINDOWS_RESERVED_NAMES.has(upper)) return false;
  return true;
};

const createWorkspaceTabId = (): string => {
  workspaceTabSeed += 1;
  return `ws-tab-${Date.now()}-${workspaceTabSeed}`;
};

const createDefaultWorkspaceTabName = (index: number): string =>
  `${t('workPage.ui.defaultTabNamePrefix')}${index}`;

const getNextWorkspaceTabIndex = (): number => {
  const prefix = t('workPage.ui.defaultTabNamePrefix').toLowerCase();
  let highestIndex = -1;

  for (const tab of workspaceTabs.value) {
    const normalizedName = normalizeWorkspaceTabName(tab.name).toLowerCase();
    if (!normalizedName.startsWith(prefix)) {
      continue;
    }

    const suffix = normalizedName.slice(prefix.length).trim();
    if (!/^\d+$/.test(suffix)) {
      continue;
    }

    highestIndex = Math.max(highestIndex, Number.parseInt(suffix, 10));
  }

  return highestIndex + 1;
};

const createDefaultWorkspaceTabMeta = (): WorkspaceTabMeta => {
  let nextIndex = getNextWorkspaceTabIndex();
  let nextName = createDefaultWorkspaceTabName(nextIndex);

  while (workspaceTabs.value.some((tab) => tab.name.toLowerCase() === nextName.toLowerCase())) {
    nextIndex += 1;
    nextName = createDefaultWorkspaceTabName(nextIndex);
  }

  return {
    id: createWorkspaceTabId(),
    name: nextName,
  };
};

const getWorkspaceDirPath = async (wsName: string, gameKey?: string): Promise<string | undefined> => {
  if (!wsName) return undefined;
  const baseDir = await getWorkspaceBaseDir(gameKey);
  if (!baseDir) return undefined;
  return join(baseDir, wsName);
};

const getWorkspaceLodDirPath = async (
  wsName: string,
  lodName: string,
  gameKey?: string,
): Promise<string | undefined> => {
  const workspaceDir = await getWorkspaceDirPath(wsName, gameKey);
  if (!workspaceDir) return undefined;
  const trimmedLodName = lodName.trim();
  if (!trimmedLodName) return undefined;
  return join(workspaceDir, trimmedLodName);
};

const getWorkspaceConfigDirPath = async (wsName: string, gameKey?: string): Promise<string | undefined> => {
  const workspaceDir = await getWorkspaceDirPath(wsName, gameKey);
  if (!workspaceDir) return undefined;
  return join(workspaceDir, 'Config');
};

const getWorkspaceTabsConfigPath = async (wsName: string, gameKey?: string): Promise<string | undefined> => {
  const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
  if (!configDir) return undefined;
  return join(configDir, 'WorkPageTabs.json');
};

const getWorkspaceTabConfigPath = async (wsName: string, tabId: string, gameKey?: string): Promise<string | undefined> => {
  const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
  if (!configDir) return undefined;
  return join(configDir, 'Tabs', `${tabId}.json`);
};

const getWorkspaceFrameAnalysisConfigPath = async (wsName: string, gameKey?: string): Promise<string | undefined> => {
  const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
  if (!configDir) return undefined;
  return join(configDir, 'FrameAnalysisPath.json');
};

const getLegacyWorkspaceFrameAnalysisConfigPath = async (wsName: string, gameKey?: string): Promise<string | undefined> => {
  const workspaceDir = await getWorkspaceDirPath(wsName, gameKey);
  if (!workspaceDir) return undefined;
  return join(workspaceDir, 'FrameAnalysisPathConfig.json');
};

const setAllConfigLoading = (value: boolean) => {
  isWorkspaceLoading.value = value;
  isSkipConfigLoading.value = value;
  isVSConfigLoading.value = value;
  isFrameAnalysisPathConfigLoading.value = value;
  isWorkspaceTabConfigLoading.value = value;
};

const movePathToRecycleBin = async (targetPath: string): Promise<void> => {
  await moveDirectoryToRecycleBin(targetPath);
};

const createInMemoryDefaultWorkspaceState = () => {
  resetLeftThreeLists();
  const defaultTab = createDefaultWorkspaceTabMeta();
  workspaceTabs.value = [defaultTab];
  activeWorkspaceTabId.value = defaultTab.id;
  editingWorkspaceTabId.value = null;
  workspaceTabNameEditBackup.value = {};
};

const resetLeftThreeLists = () => {
  modelRows.value = [{ drawIB: '', aliasName: '' }];
  skipRows.value = [{ skipIB: '', aliasName: '', indexCount: '', firstIndex: '' }];
  vsRows.value = [{ enabled: true, hash: '' }];
  frameAnalysisFolderPath.value = '';
  selectedFrameAnalysis.value = '';
  extractPanelTab.value = 'drawib';
  fullExtractDataTypeFilter.value = 'all';
  workPageDrawerCollapsed.value = normalizeWorkPageDrawerCollapsed();
  isFrameAnalysisPathInvalid.value = false;
};

const buildCurrentWorkspaceTabConfig = (): WorkPageTabConfig => ({
  modelRows: modelRows.value
    .map((row) => ({
      drawIB: row.drawIB.trim(),
      aliasName: row.aliasName.trim(),
    }))
    .filter((row) => row.drawIB !== '' || row.aliasName !== ''),
  skipRows: skipRows.value
    .map((row) => ({
      skipIB: row.skipIB.trim(),
      aliasName: row.aliasName.trim(),
      indexCount: row.indexCount.trim(),
      firstIndex: row.firstIndex.trim(),
    }))
    .filter((row) => row.skipIB !== '' || row.aliasName !== '' || row.indexCount !== '' || row.firstIndex !== ''),
  vsRows: vsRows.value
    .map((row) => ({
      enabled: row.enabled !== false,
      hash: row.hash.trim(),
    }))
    .filter((row) => row.hash !== ''),
  frameAnalysisFolderPath: frameAnalysisFolderPath.value.trim(),
  selectedFrameAnalysis: selectedFrameAnalysis.value.trim(),
  extractPanelTab: extractPanelTab.value,
  fullExtractDataTypeFilter: fullExtractDataTypeFilter.value,
  workPageDrawerCollapsed: normalizeWorkPageDrawerCollapsed(workPageDrawerCollapsed.value),
});

const cloneWorkspaceTabConfig = (config: WorkPageTabConfig): WorkPageTabConfig =>
  normalizeWorkspaceTabConfig(JSON.parse(JSON.stringify(config)) as Partial<WorkPageTabConfig>);

const buildWorkspaceTabsSnapshot = (): WorkspaceTabMeta[] =>
  workspaceTabs.value.map((tab) => ({
    id: tab.id,
    name: normalizeWorkspaceTabName(tab.name),
  }));

const createWorkspaceTabSaveSnapshot = (options?: {
  workspaceName?: string;
  activeTabId?: string;
  tabs?: WorkspaceTabMeta[];
  tabConfig?: WorkPageTabConfig;
  gameKey?: string;
}): WorkspaceTabSaveSnapshot | undefined => {
  const workspaceNameSnapshot = (options?.workspaceName ?? workspaceName.value).trim();
  const activeTabIdSnapshot = (options?.activeTabId ?? activeWorkspaceTabId.value).trim();

  if (!workspaceNameSnapshot || !activeTabIdSnapshot) {
    return undefined;
  }

  const tabsSnapshot = (options?.tabs ?? buildWorkspaceTabsSnapshot()).map((tab) => ({
    id: tab.id,
    name: normalizeWorkspaceTabName(tab.name),
  }));

  return {
    gameKey: options?.gameKey ?? getCurrentWorkspaceMemoryGameKey(),
    workspaceName: workspaceNameSnapshot,
    activeTabId: activeTabIdSnapshot,
    tabs: tabsSnapshot,
    tabConfig: cloneWorkspaceTabConfig(options?.tabConfig ?? buildCurrentWorkspaceTabConfig()),
  };
};

const settleWorkspaceSaveTask = (task: WorkspaceSaveTask): void => {
  unsettledWorkspaceSaveCount = Math.max(0, unsettledWorkspaceSaveCount - 1);
  task.done();
  if (unsettledWorkspaceSaveCount === 0) {
    const resolvers = workspaceSaveIdleResolvers.splice(0, workspaceSaveIdleResolvers.length);
    for (const resolve of resolvers) {
      resolve();
    }
  }
};

const drainWorkspaceSaveQueue = async (): Promise<void> => {
  if (workspaceSaveQueueDraining) return;
  workspaceSaveQueueDraining = true;
  try {
    while (pendingWorkspaceSaveTasks.length > 0) {
      const nextTask = pendingWorkspaceSaveTasks.shift();
      if (!nextTask) break;
      try {
        await nextTask.run();
      } catch (error) {
        console.error('Workspace save failed', error);
      } finally {
        settleWorkspaceSaveTask(nextTask);
      }
    }
  } finally {
    workspaceSaveQueueDraining = false;
  }
};

const enqueueWorkspaceSave = (key: string, task: () => Promise<void>): Promise<void> => {
  // A still-pending task with the same key writes the same files with strictly
  // older content, so the newer task supersedes it.
  const supersededIndex = pendingWorkspaceSaveTasks.findIndex((pending) => pending.key === key);
  if (supersededIndex >= 0) {
    const [superseded] = pendingWorkspaceSaveTasks.splice(supersededIndex, 1);
    settleWorkspaceSaveTask(superseded);
  }

  unsettledWorkspaceSaveCount += 1;
  return new Promise<void>((resolve) => {
    pendingWorkspaceSaveTasks.push({ key, run: task, done: resolve });
    void drainWorkspaceSaveQueue();
  });
};

// Resolves once every save enqueued so far (including coalesced replacements)
// has finished. Used by publish/export flows that must read final files.
const waitForWorkspaceSaves = async (): Promise<void> => {
  if (unsettledWorkspaceSaveCount === 0) return;
  await new Promise<void>((resolve) => {
    workspaceSaveIdleResolvers.push(resolve);
  });
};

const normalizeWorkspaceTabConfig = (
  parsed?: Partial<WorkPageTabConfig> & { onlyGpuPreSkinning?: boolean }
): WorkPageTabConfig => ({
  modelRows: Array.isArray(parsed?.modelRows)
    ? parsed!.modelRows.map((row) => ({ drawIB: row.drawIB || '', aliasName: row.aliasName || '' }))
    : [],
  skipRows: Array.isArray(parsed?.skipRows)
    ? parsed!.skipRows.map((row) => ({
      skipIB: row.skipIB || '',
      aliasName: row.aliasName || '',
      indexCount: row.indexCount || '',
      firstIndex: row.firstIndex || '',
    }))
    : [],
  vsRows: Array.isArray(parsed?.vsRows)
    ? parsed!.vsRows.map((row) => ({ enabled: row.enabled !== false, hash: row.hash || '' }))
    : [],
  frameAnalysisFolderPath: (parsed?.frameAnalysisFolderPath || '').trim(),
  selectedFrameAnalysis: (parsed?.selectedFrameAnalysis || '').trim(),
  extractPanelTab: parsed?.extractPanelTab || 'drawib',
  fullExtractDataTypeFilter: normalizeFullExtractDataTypeFilter(parsed),
  workPageDrawerCollapsed: normalizeWorkPageDrawerCollapsed(parsed?.workPageDrawerCollapsed),
});

const normalizeWorkspaceFrameAnalysisConfig = (
  parsed?: Partial<WorkspaceFrameAnalysisConfig>
): WorkspaceFrameAnalysisConfig => ({
  frameAnalysisFolderPath: (parsed?.frameAnalysisFolderPath || '').trim(),
  selectedFrameAnalysis: (parsed?.selectedFrameAnalysis || '').trim(),
});

const readWorkspaceTabsIndexBySnapshot = async (wsName: string, gameKey?: string): Promise<WorkPageTabsIndex | undefined> => {
  try {
    const indexPath = await getWorkspaceTabsConfigPath(wsName, gameKey);
    if (!indexPath) return undefined;
    const raw = await readTextFile(indexPath);
    return JSON.parse(raw) as WorkPageTabsIndex;
  } catch {
    return undefined;
  }
};

const readWorkspaceTabConfigBySnapshot = async (
  wsName: string,
  tabId: string,
  gameKey?: string,
): Promise<WorkPageTabConfig> => {
  try {
    const tabConfigPath = await getWorkspaceTabConfigPath(wsName, tabId, gameKey);
    if (!tabConfigPath) {
      return normalizeWorkspaceTabConfig();
    }

    const raw = await readTextFile(tabConfigPath);
    const parsed = JSON.parse(raw) as Partial<WorkPageTabConfig>;
    return normalizeWorkspaceTabConfig(parsed);
  } catch {
    return normalizeWorkspaceTabConfig();
  }
};

const readWorkspaceFrameAnalysisConfigBySnapshot = async (
  wsName: string,
  gameKey?: string,
): Promise<WorkspaceFrameAnalysisConfig | undefined> => {
  const configPaths = [
    await getWorkspaceFrameAnalysisConfigPath(wsName, gameKey),
    await getLegacyWorkspaceFrameAnalysisConfigPath(wsName, gameKey),
  ].filter((path): path is string => Boolean(path));

  for (const configPath of configPaths) {
    try {
      const raw = await readTextFile(configPath);
      const parsed = JSON.parse(raw) as Partial<WorkspaceFrameAnalysisConfig>;
      return normalizeWorkspaceFrameAnalysisConfig(parsed);
    } catch {
      continue;
    }
  }

  return undefined;
};

const withWorkspaceFrameAnalysisFallback = async (
  wsName: string,
  config: WorkPageTabConfig,
  gameKey?: string,
): Promise<WorkPageTabConfig> => {
  if (config.frameAnalysisFolderPath || config.selectedFrameAnalysis) {
    return config;
  }

  const frameAnalysisConfig = await readWorkspaceFrameAnalysisConfigBySnapshot(wsName, gameKey);
  if (!frameAnalysisConfig) {
    return config;
  }

  return {
    ...config,
    frameAnalysisFolderPath: frameAnalysisConfig.frameAnalysisFolderPath,
    selectedFrameAnalysis: frameAnalysisConfig.selectedFrameAnalysis,
  };
};

const mergeWorkspaceDrawIBEntries = (configs: WorkPageTabConfig[]): DrawIBConfigEntry[] => {
  const mergedEntries: DrawIBConfigEntry[] = [];
  const mergedEntryIndexMap = new Map<string, number>();

  for (const config of configs) {
    const entries = editableRowsToDrawIBConfigEntries(config.modelRows);
    for (const entry of entries) {
      const drawIB = entry.DrawIB.trim();
      if (!drawIB) continue;

      const dedupeKey = drawIB.toLowerCase();
      const existingIndex = mergedEntryIndexMap.get(dedupeKey);
      if (existingIndex !== undefined) {
        mergedEntries[existingIndex] = {
          DrawIB: drawIB,
          Alias: entry.Alias.trim(),
        };
        continue;
      }

      mergedEntryIndexMap.set(dedupeKey, mergedEntries.length);
      mergedEntries.push({
        DrawIB: drawIB,
        Alias: entry.Alias.trim(),
      });
    }
  }

  return mergedEntries;
};

const writeWorkspaceAggregatedDrawIBConfig = async (
  wsName: string,
  _targetTabConfig: WorkPageTabConfig,
  options?: {
    tabId?: string;
    tabConfig?: WorkPageTabConfig;
    tabs?: WorkspaceTabMeta[];
    gameKey?: string;
  }
): Promise<void> => {
  if (!wsName) return;

  const indexSnapshot = options?.tabs
    ? { activeTabId: options.tabId || '', tabs: options.tabs }
    : await readWorkspaceTabsIndexBySnapshot(wsName, options?.gameKey);

  const tabMetas = Array.isArray(indexSnapshot?.tabs) ? indexSnapshot!.tabs : [];
  const effectiveTabMetas = [...tabMetas];

  if (options?.tabId && !effectiveTabMetas.some((tab) => tab.id === options.tabId)) {
    effectiveTabMetas.push({ id: options.tabId, name: '' });
  }

  if (options?.tabId) {
    const currentTabIndex = effectiveTabMetas.findIndex((tab) => tab.id === options.tabId);
    if (currentTabIndex >= 0) {
      const [currentTabMeta] = effectiveTabMetas.splice(currentTabIndex, 1);
      effectiveTabMetas.push(currentTabMeta);
    }
  }

  // LOD 名称 = 目标标签页的名称
  const targetTabMeta = options?.tabId
    ? effectiveTabMetas.find((t) => t.id === options.tabId)
    : undefined;
  const targetLodName = normalizeWorkspaceTabName(targetTabMeta?.name || '');
  if (!targetLodName) return;

  const targetLodDir = await getWorkspaceLodDirPath(wsName, targetLodName, options?.gameKey);
  if (!targetLodDir) return;

  // 收集所有与目标 LOD 名称相同的标签页配置（按标签名分组）
  const configs: WorkPageTabConfig[] = [];
  for (const tab of effectiveTabMetas) {
    if (!tab.id) continue;
    const tabLodName = normalizeWorkspaceTabName(tab.name);
    if (tabLodName !== targetLodName) continue;

    if (options?.tabId && options.tabConfig && tab.id === options.tabId) {
      configs.push(options.tabConfig);
      continue;
    }

    const candidateConfig = await readWorkspaceTabConfigBySnapshot(wsName, tab.id, options?.gameKey);
    configs.push(candidateConfig);
  }

  const mergedEntries = mergeWorkspaceDrawIBEntries(configs);
  await writeDrawIBConfigToWorkspace(targetLodDir, mergedEntries);
};

const writeWorkspaceActiveTabDrawIBConfig = async (
  wsName: string,
  lodName: string,
  tabConfig: WorkPageTabConfig,
  gameKey?: string
): Promise<void> => {
  if (!wsName) return;

  const workspaceDir = await getWorkspaceLodDirPath(wsName, lodName, gameKey);
  if (!workspaceDir) return;

  await writeDrawIBConfigToWorkspace(workspaceDir, editableRowsToDrawIBConfigEntries(tabConfig.modelRows));
};

const applyWorkspaceTabConfig = async (config: WorkPageTabConfig) => {
  setAllConfigLoading(true);

  try {
    modelRows.value = config.modelRows.length > 0
      ? config.modelRows.map((row) => ({ drawIB: row.drawIB || '', aliasName: row.aliasName || '' }))
      : [{ drawIB: '', aliasName: '' }];

    skipRows.value = config.skipRows.length > 0
      ? config.skipRows.map((row) => ({
        skipIB: row.skipIB || '',
        aliasName: row.aliasName || '',
        indexCount: row.indexCount || '',
        firstIndex: row.firstIndex || '',
      }))
      : [{ skipIB: '', aliasName: '', indexCount: '', firstIndex: '' }];

    vsRows.value = config.vsRows.length > 0
      ? config.vsRows.map((row) => ({ enabled: row.enabled !== false, hash: row.hash || '' }))
      : [{ enabled: true, hash: '' }];

    frameAnalysisFolderPath.value = (config.frameAnalysisFolderPath || '').trim();
    selectedFrameAnalysis.value = (config.selectedFrameAnalysis || '').trim();
    extractPanelTab.value = config.extractPanelTab || 'drawib';
    fullExtractDataTypeFilter.value = normalizeFullExtractDataTypeFilter(config);
    workPageDrawerCollapsed.value = normalizeWorkPageDrawerCollapsed(config.workPageDrawerCollapsed);

    ensureTrailingEmptyRow();
    ensureTrailingSkipRow();
    ensureTrailingVSRow();
    await validateFrameAnalysisPath(false);
  } finally {
    setTimeout(() => {
      setAllConfigLoading(false);
    }, 120);
  }
};

const saveWorkspaceTabsIndex = async (): Promise<void> => {
  if (!workspaceName.value) return;

  const workspaceNameSnapshot = workspaceName.value;
  const activeTabIdSnapshot = activeWorkspaceTabId.value;
  const tabsSnapshot = buildWorkspaceTabsSnapshot();

  await saveWorkspaceTabsIndexBySnapshot(workspaceNameSnapshot, activeTabIdSnapshot, tabsSnapshot);
};

const saveWorkspaceTabsIndexBySnapshot = async (
  wsName: string,
  activeTabId: string,
  tabs: WorkspaceTabMeta[],
  gameKey = getCurrentWorkspaceMemoryGameKey(),
): Promise<void> => {
  if (!wsName) return;

  const epochKey = workspaceSaveEpochKey(gameKey, wsName);
  const expectedEpoch = currentWorkspaceSaveEpoch(epochKey);

  await enqueueWorkspaceSave(`tabs-index|${epochKey}`, async () => {
    // Skip saves enqueued before the workspace was cleared/deleted: running
    // them now would resurrect the folder or stale tab configs.
    if (currentWorkspaceSaveEpoch(epochKey) !== expectedEpoch) return;
    try {
      const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
      const indexPath = await getWorkspaceTabsConfigPath(wsName, gameKey);
      if (!configDir || !indexPath) return;

      await mkdir(configDir, { recursive: true });

      const payload: WorkPageTabsIndex = {
        activeTabId,
        tabs: tabs.map((tab) => ({ id: tab.id, name: normalizeWorkspaceTabName(tab.name) })),
      };

      await writeTextFile(indexPath, JSON.stringify(payload, null, 2));
    } catch (err) {
      console.error('Failed to save workspace tabs index', err);
    }
  });
};

const saveWorkspaceTabConfigSnapshot = async (snapshot: WorkspaceTabSaveSnapshot): Promise<void> => {
  await saveWorkspaceTabConfigBySnapshot(
    snapshot.workspaceName,
    snapshot.activeTabId,
    snapshot.tabConfig,
    snapshot.tabs,
    snapshot.gameKey,
  );
};

const saveWorkspaceTabConfigBySnapshot = async (
  wsName: string,
  tabId: string,
  tabConfig: WorkPageTabConfig,
  tabs?: WorkspaceTabMeta[],
  gameKey = getCurrentWorkspaceMemoryGameKey(),
): Promise<void> => {
  if (!wsName || !tabId) return;

  const configSnapshot = cloneWorkspaceTabConfig(tabConfig);
  const tabsSnapshot = tabs?.map((tab) => ({ id: tab.id, name: normalizeWorkspaceTabName(tab.name) }));
  const epochKey = workspaceSaveEpochKey(gameKey, wsName);
  const tabEpochKey = workspaceTabSaveEpochKey(gameKey, wsName, tabId);
  const expectedEpoch = currentWorkspaceSaveEpoch(epochKey);
  const expectedTabEpoch = currentWorkspaceSaveEpoch(tabEpochKey);

  await enqueueWorkspaceSave(`tab-config|${tabEpochKey}`, async () => {
    // Skip saves enqueued before the owning workspace was cleared/deleted or
    // before this tab was deleted, so they cannot recreate stale files.
    if (currentWorkspaceSaveEpoch(epochKey) !== expectedEpoch) return;
    if (currentWorkspaceSaveEpoch(tabEpochKey) !== expectedTabEpoch) return;
    try {
      const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
      const tabConfigPath = await getWorkspaceTabConfigPath(wsName, tabId, gameKey);
      const frameAnalysisConfigPath = await getWorkspaceFrameAnalysisConfigPath(wsName, gameKey);
      if (!configDir || !tabConfigPath) return;

      const tabConfigDir = await join(configDir, 'Tabs');
      await mkdir(tabConfigDir, { recursive: true });

      await writeTextFile(tabConfigPath, JSON.stringify(configSnapshot, null, 2));
      if (frameAnalysisConfigPath) {
        await writeTextFile(
          frameAnalysisConfigPath,
          JSON.stringify(
            {
              frameAnalysisFolderPath: configSnapshot.frameAnalysisFolderPath.trim(),
              selectedFrameAnalysis: configSnapshot.selectedFrameAnalysis.trim(),
            },
            null,
            2
          )
        );
      }
      // Keep a runtime copy in workspace root for existing backend commands.
      // If tabs were not provided, fall back to reading the saved index so that
      // writeLegacyWorkspaceRuntimeFiles can resolve the LOD name from the tab name.
      let resolvedTabs = tabsSnapshot;
      if (!resolvedTabs) {
        const indexSnapshot = await readWorkspaceTabsIndexBySnapshot(wsName, gameKey);
        resolvedTabs = indexSnapshot?.tabs?.map((t) => ({ id: t.id, name: t.name }));
      }
      // The tab config and frame-analysis files were already written above in
      // this same save pass; skipRewrittenConfigFiles avoids writing them twice.
      await writeLegacyWorkspaceRuntimeFiles(wsName, configSnapshot, { tabId, tabs: resolvedTabs, gameKey, skipRewrittenConfigFiles: true });
    } catch (err) {
      console.error('Failed to save workspace tab config', err);
    }
  });
};

const saveCurrentWorkspaceTabConfig = async (): Promise<void> => {
  if (isWorkspaceTransitioning.value || isWorkspaceTabConfigLoading.value) return;

  const snapshot = createWorkspaceTabSaveSnapshot();
  if (!snapshot) return;

  await saveWorkspaceTabConfigSnapshot(snapshot);
};

const loadWorkspaceTabConfig = async (
  wsName: string,
  tabId: string,
  expectedGameKey = getCurrentWorkspaceMemoryGameKey(),
): Promise<void> => {
  const contextIsCurrent = () => expectedGameKey === getCurrentWorkspaceMemoryGameKey();
  if (!contextIsCurrent()) return;
  if (!wsName || !tabId) {
    await applyWorkspaceTabConfig({
      modelRows: [],
      skipRows: [],
      vsRows: [],
      frameAnalysisFolderPath: '',
      selectedFrameAnalysis: '',
      extractPanelTab: 'drawib',
      fullExtractDataTypeFilter: 'all',
      workPageDrawerCollapsed: normalizeWorkPageDrawerCollapsed(),
    });
    return;
  }

  try {
    const tabConfigPath = await getWorkspaceTabConfigPath(wsName, tabId, expectedGameKey);
    if (!tabConfigPath) return;

    const raw = await readTextFile(tabConfigPath);
    if (!contextIsCurrent()) return;
    const parsed = JSON.parse(raw) as Partial<WorkPageTabConfig>;

    const config = await withWorkspaceFrameAnalysisFallback(wsName, normalizeWorkspaceTabConfig(parsed), expectedGameKey);
    if (!contextIsCurrent()) return;
    await applyWorkspaceTabConfig(config);
  } catch {
    if (!contextIsCurrent()) return;
    const config = await withWorkspaceFrameAnalysisFallback(wsName, normalizeWorkspaceTabConfig(), expectedGameKey);
    if (!contextIsCurrent()) return;
    await applyWorkspaceTabConfig(config);
  }
};

const ensureWorkspaceTabsInitialized = async (
  wsName: string,
  expectedGameKey = getCurrentWorkspaceMemoryGameKey(),
): Promise<void> => {
  if (!wsName) return;
  const contextIsCurrent = () => expectedGameKey === getCurrentWorkspaceMemoryGameKey();
  if (!contextIsCurrent()) return;

  const configDir = await getWorkspaceConfigDirPath(wsName, expectedGameKey);
  const indexPath = await getWorkspaceTabsConfigPath(wsName, expectedGameKey);
  if (!configDir || !indexPath) return;

  await mkdir(configDir, { recursive: true });
  await mkdir(await join(configDir, 'Tabs'), { recursive: true });

  try {
    const raw = await readTextFile(indexPath);
    if (!contextIsCurrent()) return;
    const parsed = JSON.parse(raw) as Partial<WorkPageTabsIndex>;
    const tabs = Array.isArray(parsed.tabs) ? parsed.tabs : [];

    const validTabs = tabs
      .map((tab) => ({
        id: String(tab.id || ''),
        name: normalizeWorkspaceTabName(String(tab.name || '')),
      }))
      .filter((tab) => tab.id && tab.name && isValidWindowsFileName(tab.name));

    const dedupedTabs: WorkspaceTabMeta[] = [];
    for (const tab of validTabs) {
      if (dedupedTabs.some((item) => item.name.toLowerCase() === tab.name.toLowerCase())) {
        continue;
      }
      dedupedTabs.push(tab);
    }

    workspaceTabs.value = dedupedTabs;
    if (workspaceTabs.value.length === 0) {
      const defaultTab = createDefaultWorkspaceTabMeta();
      workspaceTabs.value = [defaultTab];
      activeWorkspaceTabId.value = defaultTab.id;
      await saveWorkspaceTabsIndexBySnapshot(wsName, defaultTab.id, buildWorkspaceTabsSnapshot(), expectedGameKey);
      await saveWorkspaceTabConfigBySnapshot(wsName, defaultTab.id, normalizeWorkspaceTabConfig(), buildWorkspaceTabsSnapshot(), expectedGameKey);
      return;
    }

    const preferredActiveId = String(parsed.activeTabId || '');
    activeWorkspaceTabId.value = workspaceTabs.value.some((tab) => tab.id === preferredActiveId)
      ? preferredActiveId
      : workspaceTabs.value[0].id;
  } catch {
    if (!contextIsCurrent()) return;
    const defaultTab = createDefaultWorkspaceTabMeta();
    workspaceTabs.value = [defaultTab];
    activeWorkspaceTabId.value = defaultTab.id;
    await saveWorkspaceTabsIndexBySnapshot(wsName, defaultTab.id, buildWorkspaceTabsSnapshot(), expectedGameKey);
    await saveWorkspaceTabConfigBySnapshot(wsName, defaultTab.id, normalizeWorkspaceTabConfig(), buildWorkspaceTabsSnapshot(), expectedGameKey);
  }
};

const validateFrameAnalysisPath = async (showWarning: boolean): Promise<void> => {
  const path = frameAnalysisFolderPath.value.trim();
  if (!path) {
    isFrameAnalysisPathInvalid.value = false;
    lastInvalidFrameAnalysisWarnPath = '';
    return;
  }

  try {
    // exists() is far cheaper than readDir() here: frame analysis folders can
    // contain thousands of dumped files, and this runs on every workspace switch.
    if (!(await exists(path))) {
      throw new Error('Frame analysis path does not exist');
    }
    isFrameAnalysisPathInvalid.value = false;
    lastInvalidFrameAnalysisWarnPath = '';
  } catch {
    isFrameAnalysisPathInvalid.value = true;
    if (showWarning && lastInvalidFrameAnalysisWarnPath !== path) {
      lastInvalidFrameAnalysisWarnPath = path;
      ElMessage.warning(t('workPage.messages.frameAnalysisPathInvalid'));
    }
  }
};

// Auto-save logic
let saveTimer: ReturnType<typeof setTimeout>;
let skipSaveTimer: ReturnType<typeof setTimeout>;
let vsSaveTimer: ReturnType<typeof setTimeout>;
let frameAnalysisPathSaveTimer: ReturnType<typeof setTimeout>;
let frameAnalysisValidateTimer: ReturnType<typeof setTimeout>;
let unlistenNativeDrop: UnlistenFn | null = null;
let unlistenWorkspaceUploadProgress: UnlistenFn | null = null;
let workspaceContextRevision = 0;
let workspaceSwitchRevision = 0;

const clearPendingSaveTimers = () => {
  clearTimeout(saveTimer);
  clearTimeout(skipSaveTimer);
  clearTimeout(vsSaveTimer);
  clearTimeout(frameAnalysisPathSaveTimer);
  clearTimeout(frameAnalysisValidateTimer);
};

const flushCurrentWorkspaceTabConfig = async (snapshot?: WorkspaceTabSaveSnapshot): Promise<void> => {
  clearPendingSaveTimers();

  if (!snapshot && isWorkspaceTabConfigLoading.value) return;

  const effectiveSnapshot = snapshot ?? createWorkspaceTabSaveSnapshot();
  if (!effectiveSnapshot) return;

  await saveWorkspaceTabConfigSnapshot(effectiveSnapshot);
};

const switchWorkspace = async (targetWorkspaceName: string): Promise<void> => {
  const expectedGameKey = getCurrentWorkspaceMemoryGameKey();
  const expectedContextRevision = workspaceContextRevision;
  let expectedSwitchRevision = workspaceSwitchRevision;
  const contextIsCurrent = () => expectedGameKey === getCurrentWorkspaceMemoryGameKey()
    && expectedContextRevision === workspaceContextRevision
    && expectedSwitchRevision === workspaceSwitchRevision;
  const resolvedWorkspaceName = findExistingWorkspaceOption(targetWorkspaceName)
    ?? normalizeWorkspaceNameInput(targetWorkspaceName);
  const currentWorkspaceName = normalizeWorkspaceNameInput(workspaceName.value);

  if (!resolvedWorkspaceName) {
    workspaceDraftName.value = '';
    if (!currentWorkspaceName) {
      return;
    }

    // Cancel any in-flight switch: without this bump a pending switchWorkspace
    // would pass its context checks after we cleared the state and resurrect
    // the workspace the user just deselected.
    workspaceSwitchRevision += 1;
    clearPendingSaveTimers();
    setAllConfigLoading(true);
    isWorkspaceTransitioning.value = true;

    try {
      workspaceName.value = '';
      rememberWorkspaceForCurrentGame('');
      resetLeftThreeLists();
      workspaceTabs.value = [];
      activeWorkspaceTabId.value = '';
      editingWorkspaceTabId.value = null;
      workspaceTabNameEditBackup.value = {};
    } finally {
      setAllConfigLoading(false);
      isWorkspaceTransitioning.value = false;
    }
    return;
  }

  workspaceDraftName.value = resolvedWorkspaceName;
  if (resolvedWorkspaceName === currentWorkspaceName) {
    return;
  }
  expectedSwitchRevision = ++workspaceSwitchRevision;

  const previousSnapshot = currentWorkspaceName && activeWorkspaceTabId.value && !isWorkspaceTabConfigLoading.value
    ? createWorkspaceTabSaveSnapshot({
        workspaceName: currentWorkspaceName,
        activeTabId: activeWorkspaceTabId.value,
        tabs: buildWorkspaceTabsSnapshot(),
        tabConfig: buildCurrentWorkspaceTabConfig(),
        gameKey: expectedGameKey,
      })
    : undefined;

  clearPendingSaveTimers();
  isWorkspaceTransitioning.value = true;
  setAllConfigLoading(true);

  let shouldReleaseLoadingState = true;

  try {
    if (previousSnapshot) {
      await saveWorkspaceTabConfigSnapshot(previousSnapshot);
    }
    if (!contextIsCurrent()) return;

    resetLeftThreeLists();
    workspaceTabs.value = [];
    activeWorkspaceTabId.value = '';
    editingWorkspaceTabId.value = null;
    workspaceTabNameEditBackup.value = {};

    workspaceName.value = resolvedWorkspaceName;
    rememberWorkspaceForCurrentGame(resolvedWorkspaceName);
    await loadWorkspaceProvenance(resolvedWorkspaceName, expectedGameKey);
    if (!contextIsCurrent()) return;
    await ensureWorkspaceTabsInitialized(resolvedWorkspaceName, expectedGameKey);
    if (!contextIsCurrent()) return;
    await saveWorkspaceTabsIndexBySnapshot(
      resolvedWorkspaceName,
      activeWorkspaceTabId.value,
      buildWorkspaceTabsSnapshot(),
      expectedGameKey,
    );
    if (!contextIsCurrent()) return;
    await loadWorkspaceTabConfig(resolvedWorkspaceName, activeWorkspaceTabId.value, expectedGameKey);
    if (!contextIsCurrent()) return;
    shouldReleaseLoadingState = false;
  } finally {
    if (shouldReleaseLoadingState && contextIsCurrent()) {
      setAllConfigLoading(false);
    }
    if (contextIsCurrent()) isWorkspaceTransitioning.value = false;
  }
};

const handleWorkspaceSelectionChange = async (value: string) => {
  const normalizedValue = normalizeWorkspaceNameInput(value);
  workspaceDraftName.value = normalizedValue;

  const existingWorkspaceName = findExistingWorkspaceOption(normalizedValue);
  if (!existingWorkspaceName) {
    return;
  }

  await switchWorkspace(existingWorkspaceName);
};

// Watch for changes and auto-save
watch(modelRows, () => {
  ensureTrailingEmptyRow();

  if (isWorkspaceLoading.value || isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;

  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    void saveCurrentWorkspaceTabConfig();
  }, 800);
}, { deep: true });

watch(skipRows, () => {
  ensureTrailingSkipRow();
  if (isSkipConfigLoading.value || isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;

  clearTimeout(skipSaveTimer);
  skipSaveTimer = setTimeout(() => {
    void saveCurrentWorkspaceTabConfig();
  }, 800);
}, { deep: true });

watch(vsRows, () => {
  ensureTrailingVSRow();
  if (isVSConfigLoading.value || isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;

  clearTimeout(vsSaveTimer);
  vsSaveTimer = setTimeout(() => {
    void saveCurrentWorkspaceTabConfig();
  }, 800);
}, { deep: true });

watch(frameAnalysisFolderPath, () => {
  if (isFrameAnalysisPathConfigLoading.value || isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;

  clearTimeout(frameAnalysisPathSaveTimer);
  frameAnalysisPathSaveTimer = setTimeout(() => {
    void saveCurrentWorkspaceTabConfig();
  }, 500);

  clearTimeout(frameAnalysisValidateTimer);
  frameAnalysisValidateTimer = setTimeout(() => {
    void validateFrameAnalysisPath(true);
  }, 250);
});

watch([selectedFrameAnalysis, extractPanelTab, fullExtractDataTypeFilter], () => {
  if (isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;
  void saveCurrentWorkspaceTabConfig();
});

watch(workPageDrawerCollapsed, () => {
  if (isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;
  void saveCurrentWorkspaceTabConfig();
}, { deep: true });

watch(activeWorkspaceTabId, async (newTabId, oldTabId) => {
  if (!workspaceName.value) return;
  if (!newTabId || newTabId === oldTabId) return;
  if (isWorkspaceTabConfigLoading.value || isWorkspaceTransitioning.value) return;

  const workspaceNameSnapshot = workspaceName.value;
  const tabsSnapshot = buildWorkspaceTabsSnapshot();
  // This watcher is async: the user may switch games (or workspaces) while the
  // saves below are in flight. Capture the game key up-front and bail out the
  // moment it changes, otherwise stale writes land in another game's
  // same-named workspace and stale loads overwrite the freshly loaded config.
  const expectedGameKey = getCurrentWorkspaceMemoryGameKey();
  const contextIsCurrent = () => expectedGameKey === getCurrentWorkspaceMemoryGameKey();
  const previousSnapshot = oldTabId
    ? createWorkspaceTabSaveSnapshot({
        workspaceName: workspaceNameSnapshot,
        activeTabId: oldTabId,
        tabs: tabsSnapshot,
        tabConfig: buildCurrentWorkspaceTabConfig(),
        gameKey: expectedGameKey,
      })
    : undefined;

  clearPendingSaveTimers();
  isWorkspaceTransitioning.value = true;
  setAllConfigLoading(true);

  let shouldReleaseLoadingState = true;

  try {
    if (previousSnapshot) {
      await saveWorkspaceTabConfigSnapshot(previousSnapshot);
    }
    if (!contextIsCurrent()) return;

    await saveWorkspaceTabsIndexBySnapshot(workspaceNameSnapshot, newTabId, tabsSnapshot, expectedGameKey);
    if (!contextIsCurrent()) return;
    await loadWorkspaceTabConfig(workspaceNameSnapshot, newTabId, expectedGameKey);
    if (!contextIsCurrent()) return;
    shouldReleaseLoadingState = false;
  } finally {
    if (shouldReleaseLoadingState && contextIsCurrent()) {
      setAllConfigLoading(false);
    }
    if (contextIsCurrent()) isWorkspaceTransitioning.value = false;
  }
});

const syncFrameAnalysisFolderPathFromSelection = async () => {
  if (!selectedFrameAnalysis.value) {
    return;
  }

  const current3dmigotoFolderPath = await PathHelper.GetCurrentGame3DmigotoFolderPath();
  if (!current3dmigotoFolderPath) {
    frameAnalysisFolderPath.value = '';
    return;
  }

  const selected = selectedFrameAnalysis.value;

  // If the selected option is a LOD folder (e.g. LOD0, LOD1), find the latest
  // FrameAnalysis folder inside it and use that as the full path.
  if (/^lod\d+$/i.test(selected)) {
    const lodFolderPath = await join(current3dmigotoFolderPath, selected);
    try {
      const lodEntries = await readDir(lodFolderPath);
      const faFolders = lodEntries
        .filter((e) => e.isDirectory && e.name && e.name.startsWith('FrameAnalysis'))
        .map((e) => e.name as string)
        .sort((a, b) => b.localeCompare(a));
      if (faFolders.length > 0) {
        frameAnalysisFolderPath.value = await join(lodFolderPath, faFolders[0]);
        return;
      }
    } catch {
      // Fall through to set lodFolderPath itself
    }
    frameAnalysisFolderPath.value = lodFolderPath;
    return;
  }

  frameAnalysisFolderPath.value = await join(current3dmigotoFolderPath, selected);
};

watch(selectedFrameAnalysis, () => {
  if (isWorkspaceTabConfigLoading.value || isFrameAnalysisPathConfigLoading.value) {
    return;
  }
  void syncFrameAnalysisFolderPathFromSelection();
});

watch(() => appSettings.CurrentGameName, (_newGameName, previousGameName) => {
  // The current rows still hold the previous game's workspace state here.
  // Snapshot and flush them to the PREVIOUS game's folder before clearing —
  // otherwise edits made in the last debounce window are silently dropped.
  // The game key must be taken from previousGameName: the game has already
  // changed by the time this watcher runs.
  const previousGameKey = normalizeWorkspaceGameKey(previousGameName);
  const pendingSnapshot = createWorkspaceTabSaveSnapshot({ gameKey: previousGameKey });

  clearPendingSaveTimers();
  workspaceContextRevision += 1;
  workspaceSwitchRevision += 1; // cancel any in-flight workspace switch
  workspaceOptions.value = [];
  workspaceModifiedTimes.value = {};
  workspaceName.value = '';
  workspaceDraftName.value = '';
  if (pendingSnapshot) {
    void saveWorkspaceTabConfigSnapshot(pendingSnapshot);
  }
  void loadSpecificIbDumpState();
  void refreshWorkspaces();
});

const handleFrameAnalysisOptionClick = async (item: string) => {
  selectedFrameAnalysis.value = item;
  await syncFrameAnalysisFolderPathFromSelection();
};



const refreshFrameAnalysisFolders = async () => {
  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') {
    ElMessage.info(t('workPage.messages.selectGameConfigFirst'));
    return;
  }

  isRefreshing.value = true;
  try {
    const currentSelectedFrameAnalysis = selectedFrameAnalysis.value.trim();
    const currentFrameAnalysisFolderPath = frameAnalysisFolderPath.value.trim();

    // Get current 3Dmigoto directory
    const basePath = await PathHelper.GetCurrentGame3DmigotoFolderPath();

    if (!basePath) {
      ElMessage.warning(t('workPage.messages.migotoFolderNotFound'));
      return;
    }


    const entries = await readDir(basePath);
    const lodFolders: string[] = [];
    const directFaFolders: string[] = [];

    for (const entry of entries) {
      if (!entry.name || !entry.isDirectory) continue;
      if (/^lod\d+$/i.test(entry.name)) {
        lodFolders.push(entry.name);
      } else if (entry.name.startsWith('FrameAnalysis')) {
        directFaFolders.push(entry.name);
      }
    }

    // Prefer LOD subfolders; fall back to direct FrameAnalysis folders
    lodFolders.sort((a, b) => a.localeCompare(b));
    directFaFolders.sort((a, b) => b.localeCompare(a));
    const folders = lodFolders.length > 0 ? lodFolders : directFaFolders;

    // Sort by name descending (newest timestamp first)
    frameAnalysisOptions.value = folders;

    if (folders.length === 0) {
      if (!currentFrameAnalysisFolderPath) {
        selectedFrameAnalysis.value = '';
      }
    } else {
      // For LOD mode: match selected by LOD name found in the current frame analysis path
      const normalizedCurrentPath = currentFrameAnalysisFolderPath
        .replace(/\\/g, '/')
        .replace(/\/+$/, '')
        .toLowerCase();

      // Try to find a folder that matches the current frame analysis path or is
      // a LOD name found inside the path segments.
      let matchedFolderFromPath = '';
      if (normalizedCurrentPath) {
        const pathSegments = normalizedCurrentPath.split('/').filter(Boolean);
        for (const folder of folders) {
          const folderLower = folder.toLowerCase();
          // Direct match: the path ends with the folder name
          if (pathSegments.includes(folderLower)) {
            matchedFolderFromPath = folder;
            break;
          }
          // Full path match (direct FrameAnalysis folders)
          const candidatePath = (await join(basePath, folder))
            .replace(/\\/g, '/')
            .replace(/\/+$/, '')
            .toLowerCase();
          if (normalizedCurrentPath.startsWith(candidatePath)) {
            matchedFolderFromPath = folder;
            break;
          }
        }
      }

      if (currentSelectedFrameAnalysis && folders.includes(currentSelectedFrameAnalysis)) {
        return;
      }

      if (matchedFolderFromPath) {
        selectedFrameAnalysis.value = matchedFolderFromPath;
        return;
      }

      if (!currentSelectedFrameAnalysis && !currentFrameAnalysisFolderPath) {
        selectedFrameAnalysis.value = folders[0];
      }
    }

  } catch (err) {
    console.error('Failed to refresh FrameAnalysis folders', err);
    ElMessage.error(t('workPage.messages.refreshFailed'));
  } finally {
    isRefreshing.value = false;
  }
};

const handleExtractModels = async () => {
  const logWorkspaceKey = extractionLogWorkspaceKey(appSettings.CurrentGameName, workspaceName.value);
  if (workspaceName.value) {
    await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
      tabId: activeWorkspaceTabId.value,
      tabs: workspaceTabs.value,
      drawIBScope: 'active-tab',
    });
  }
  const activeTabName = normalizeWorkspaceTabName(
    workspaceTabs.value.find((t) => t.id === activeWorkspaceTabId.value)?.name || ''
  );
  await runExtractModels(
    isExtracting.value,
    modelRows.value,
    frameAnalysisFolderPath.value,
    activeTabName,
    getWorkspaceBaseDir,
    workspaceName.value,
    appSettings.CurrentGameName,
    activeWorkspaceTabId.value,
    appSettings.textureMarkStylePreference,
    (value) => {
      isExtracting.value = value;
    },
    openPath,
    openExtractionLog,
    (path) => {
      latestExtractionLogPaths.value = { ...latestExtractionLogPaths.value, [logWorkspaceKey]: path };
    }
  );
};

const handleFullExtract = async () => {
  const logWorkspaceKey = extractionLogWorkspaceKey(appSettings.CurrentGameName, workspaceName.value);
  if (workspaceName.value) {
    await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
      tabId: activeWorkspaceTabId.value,
      tabs: workspaceTabs.value,
      drawIBScope: 'active-tab',
    });
  }
  const activeTabName = normalizeWorkspaceTabName(
    workspaceTabs.value.find((t) => t.id === activeWorkspaceTabId.value)?.name || ''
  );
  await runFullExtract(
    isExtracting.value,
    frameAnalysisFolderPath.value,
    activeTabName,
    fullExtractDataTypeFilter.value,
    getWorkspaceBaseDir,
    workspaceName.value,
    appSettings.CurrentGameName,
    activeWorkspaceTabId.value,
    appSettings.textureMarkStylePreference,
    (value) => {
      isExtracting.value = value;
    },
    openPath,
    openExtractionLog,
    (path) => {
      latestExtractionLogPaths.value = { ...latestExtractionLogPaths.value, [logWorkspaceKey]: path };
    }
  );
};

const handleOpenLatestExtractionLog = async () => {
  if (currentLatestExtractionLogPath.value) await openExtractionLog(currentLatestExtractionLogPath.value);
};

const handleSelectLatestFrameAnalysis = async () => {
    // Force refresh first
    await refreshFrameAnalysisFolders();
    if (frameAnalysisOptions.value.length > 0) {
        // Since list is already sorted descending (newest first), pick index 0
        selectedFrameAnalysis.value = frameAnalysisOptions.value[0];
    await syncFrameAnalysisFolderPathFromSelection();
      ElMessage.success(t('workPage.messages.switchedTo', { folder: frameAnalysisOptions.value[0] }));
    } else {
      ElMessage.warning(t('workPage.messages.noFrameAnalysisFolders'));
    }
};

const handleAnalyzeMissingGameTypes = async () => {
  if (!(appSettings.xianzunApiKey || '').trim()) {
    ElMessage.warning(t('workPage.messages.xianzunUnavailable'));
    return;
  }
  setPendingXianZunPrompt(t('workPage.messages.analyzeGameTypePrompt'));
  await router.push('/xianzun');
};

const handlePickFrameAnalysisFolder = async () => {
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: t('workPage.dialog.selectFrameAnalysisFolderTitle'),
    });

    if (!selected || typeof selected !== 'string') {
      return;
    }

    selectedFrameAnalysis.value = '';
    frameAnalysisFolderPath.value = selected;
  } catch (err) {
    console.error('Failed to select FrameAnalysis folder', err);
    ElMessage.error(t('workPage.messages.selectFrameAnalysisFolderFailed'));
  }
};

const normalizeDroppedPath = (rawPath: string): string => {
  const trimmed = rawPath.trim();
  if (!trimmed) return '';

  if (trimmed.startsWith('file://')) {
    const decoded = decodeURIComponent(trimmed.replace('file://', ''));
    return decoded.replace(/^\/([a-zA-Z]:\/)/, '$1');
  }

  return trimmed;
};

const resolveDropPath = (event: DragEvent): string => {
  const dataTransfer = event.dataTransfer;
  if (!dataTransfer) return '';

  for (const file of Array.from(dataTransfer.files)) {
    const fileWithPath = file as File & { path?: string };
    if (fileWithPath.path && fileWithPath.path.trim()) {
      return normalizeDroppedPath(fileWithPath.path);
    }
  }

  const uriList = dataTransfer.getData('text/uri-list');
  if (uriList.trim()) {
    const firstLine = uriList
      .split(/\r?\n/)
      .map(line => line.trim())
      .find(line => line && !line.startsWith('#'));

    if (firstLine) {
      return normalizeDroppedPath(firstLine);
    }
  }

  const plainText = dataTransfer.getData('text/plain');
  if (plainText.trim()) {
    return normalizeDroppedPath(plainText);
  }

  return '';
};

const applyDroppedFrameAnalysisFolder = async (rawPath: string): Promise<boolean> => {
  const droppedPath = normalizeDroppedPath(rawPath);
  if (!droppedPath) {
    ElMessage.warning(t('workPage.messages.noValidPathDetected'));
    return false;
  }

  try {
    await readDir(droppedPath);
  } catch {
    ElMessage.warning(t('workPage.messages.droppedTargetNotAccessible'));
    return false;
  }

  selectedFrameAnalysis.value = '';
  frameAnalysisFolderPath.value = droppedPath;
  ElMessage.success(t('workPage.messages.folderLoadedFromDragDrop'));
  return true;
};

const handleDropFrameAnalysisFolder = async (event: DragEvent) => {
  event.preventDefault();

  try {
    const droppedPath = resolveDropPath(event);
    await applyDroppedFrameAnalysisFolder(droppedPath);
  } catch (err) {
    console.error('Failed to read dragged FrameAnalysis folder', err);
    ElMessage.error(t('workPage.messages.readDraggedFolderFailed'));
  }
};

const handleOpenFrameAnalysisFolderPath = async () => {
  const target = frameAnalysisFolderPath.value.trim();
  await openPath(target, t('workPage.messages.specifyFrameAnalysisPathFirst'));
};

const handleTextureMenu = async (cmd: unknown) => {
  debugLog('WorkPage', 'texture menu command(raw):', cmd);
  if (workspaceName.value) {
    await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
      tabId: activeWorkspaceTabId.value,
      tabs: workspaceTabs.value,
    });
  }
  const activeTabName = normalizeWorkspaceTabName(
    workspaceTabs.value.find((t) => t.id === activeWorkspaceTabId.value)?.name || ''
  );
  await handleTextureMenuCommand(cmd, {
    frameAnalysisFolderPath: frameAnalysisFolderPath.value,
    activeTabName,
    getWorkspaceBaseDir,
    workspaceName: workspaceName.value,
    currentGameName: appSettings.CurrentGameName,
    openPath,
  });
};

const loadWorkspaceProvenance = async (wsName: string, gameKey?: string): Promise<void> => {
  workspaceProvenance.value = null;
  const configDir = await getWorkspaceConfigDirPath(wsName, gameKey);
  if (!configDir) return;
  try {
    const value = JSON.parse(await readTextFile(await join(configDir, 'WorkspaceAccess.json'))) as Partial<WorkspaceProvenance>;
    if (typeof value.entryId === 'string' && typeof value.uploadedAt === 'string' && typeof value.attribution === 'string') {
      workspaceProvenance.value = {
        entryId: value.entryId,
        uploadedAt: value.uploadedAt,
        attribution: value.attribution,
        aliases: Array.isArray(value.aliases) ? value.aliases.filter((item): item is string => typeof item === 'string') : [],
      };
    }
  } catch {
    // Locally created workspaces do not have library provenance.
  }
};

const openWorkspaceUploadDialog = (): void => {
  if (!workspaceName.value) {
    ElMessage.warning(t('workPage.messages.selectOrCreateWorkspaceFirst'));
    return;
  }
  workspaceAccessDescription.value = '';
  workspaceAccessPublishName.value = workspaceName.value;
  workspaceAccessAttribution.value = appSettings.workspaceAccessAttribution;
  workspaceAccessAliases.value = '';
  workspaceAccessIncludeFullPackage.value = true;
  workspaceAccessDialog.value = 'upload';
};

const normalizedWorkspaceAliases = (): string[] => Array.from(new Set(
  workspaceAccessAliases.value.split(/[\n,]/u).map((value) => value.trim()).filter(Boolean),
));

const handlePublishWorkspaceFromDialog = async (): Promise<void> => {
  if (!workspaceName.value || workspaceAccessBusy.value) return;
  try {
    workspacePublishing.value = true;
    workspaceUploadProgress.value = null;
    await saveCurrentWorkspaceTabConfig();
    await waitForWorkspaceSaves();
    const workspacePath = await getWorkspaceDirPath(workspaceName.value);
    if (!workspacePath) throw new Error('WORKSPACE_NOT_ACCESSIBLE');
    const publishName = workspaceAccessPublishName.value.trim();
    if (!isValidWindowsFileName(publishName)) {
      ElMessage.warning(t('workPage.messages.workspacePublishNameInvalid'));
      return;
    }
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    const request = {
      workerUrl: WORKSPACE_ACCESS_API_URL,
      proxyPort: workspaceAccessProxyPort(),
      workspacePath,
      workspaceName: publishName,
      gamePreset,
      description: workspaceAccessDescription.value.trim() || undefined,
      attribution: workspaceAccessAttribution.value.trim() ? { mode: 'custom', displayName: workspaceAccessAttribution.value.trim() } : { mode: 'anonymous' },
      workspaceAliases: normalizedWorkspaceAliases(),
    };
    appSettings.workspaceAccessAttribution = workspaceAccessAttribution.value.trim();
    const command = workspaceAccessIncludeFullPackage.value ? 'workspace_access_create_and_publish' : 'workspace_access_publish';
    const result = await invoke<WorkspacePublishResult>(command, { request });
    workspaceAccessDialog.value = null;
    ElMessage.success(t('workPage.messages.workspacePublished', { entryId: result.entryId }));
    void refreshWorkspaceLibraryAfterPublish(result.entryId);
  } catch (error) {
    console.error('Workspace publish failed', error);
    ElMessage.error(t('workPage.messages.workspacePublishFailed'));
  } finally {
    workspacePublishing.value = false;
    window.setTimeout(() => {
      workspaceUploadProgress.value = null;
    }, workspaceUploadPercent.value >= 100 ? 1400 : 500);
  }
};

const workspaceLibraryEntriesSignature = (entries: LibraryIndexEntry[]): string => entries
  .map((entry) => [
    entry.entryId,
    entry.workspaceName,
    entry.description ?? '',
    entry.attribution,
    entry.uploadedAt,
    entry.fullDataAvailable,
    entry.fullDataSize,
    entry.metadataDownloadCount ?? 0,
    entry.fullPackageDownloadCount ?? 0,
    entry.metadataPath,
  ].join('\u0000'))
  .join('\u0001');

const loadWorkspaceLibraryForDialog = async ({
  silent = false,
  forceRefresh = false,
}: { silent?: boolean; forceRefresh?: boolean } = {}): Promise<void> => {
  try {
    if (!silent) workspaceLibraryLoading.value = true;
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    const index = await invoke<LibraryIndex>('workspace_access_fetch_index', {
      rawBaseUrl: null,
      gamePreset,
      proxyPort: workspaceAccessProxyPort(),
      forceRefresh,
    });
    // Do not replace the reactive list when its content is unchanged. In
    // particular, background post-publish checks must not make the visible
    // download panel flicker while the remote index is still catching up.
    if (workspaceLibraryEntriesSignature(index.entries) !== workspaceLibraryEntriesSignature(workspaceLibraryEntries.value)) {
      workspaceLibraryEntries.value = index.entries;
    }
  } catch (error) {
    console.error('Workspace library load failed', error);
    if (!silent) ElMessage.error(t('workPage.messages.workspaceLibraryFailed'));
  } finally {
    if (!silent) workspaceLibraryLoading.value = false;
  }
};

const refreshWorkspaceLibraryAfterPublish = async (entryId: string): Promise<void> => {
  // The index is rebuilt asynchronously by the public-library workflow. Poll a
  // short time after publishing so the download dialog is normally current when
  // the user opens it, rather than relying on a later manual refresh.
  for (let attempt = 0; attempt < 8; attempt += 1) {
    await loadWorkspaceLibraryForDialog({ silent: true, forceRefresh: true });
    if (workspaceLibraryEntries.value.some((entry) => entry.entryId === entryId)) return;
    await new Promise((resolve) => window.setTimeout(resolve, 2_000));
  }
};

const openWorkspaceDownloadDialog = async (): Promise<void> => {
  workspaceAccessDialog.value = 'download';
  workspaceLibraryQuery.value = '';
  await loadWorkspaceLibraryForDialog({ forceRefresh: true });
};

const filteredWorkspaceLibraryEntries = () => {
  const query = workspaceLibraryQuery.value.trim().toLocaleLowerCase();
  const filtered = workspaceLibraryEntries.value.filter((entry) => !query || [entry.workspaceName, entry.description ?? '', entry.attribution, ...entry.drawIB, ...entry.aliases].join('\n').toLocaleLowerCase().includes(query));
  return filtered.sort((left, right) => workspaceLibrarySort.value === 'downloads'
    ? ((right.metadataDownloadCount ?? 0) + (right.fullPackageDownloadCount ?? 0)) - ((left.metadataDownloadCount ?? 0) + (left.fullPackageDownloadCount ?? 0)) || right.uploadedAt.localeCompare(left.uploadedAt)
    : right.uploadedAt.localeCompare(left.uploadedAt));
};

const handleLibraryDownload = async (entry: LibraryIndexEntry, mode: 'metadata' | 'full'): Promise<void> => {
  if (workspaceAccessBusy.value) return;
  try {
    workspaceDownloadingEntry.value = { entryId: entry.entryId, mode };
    const metadata = await invoke<LibraryMetadata>('workspace_access_fetch_metadata', { rawBaseUrl: null, metadataPath: entry.metadataPath, proxyPort: workspaceAccessProxyPort() });
    const workspaceBase = await getWorkspaceBaseDir();
    if (!workspaceBase) throw new Error('WORKSPACE_BASE_NOT_FOUND');
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    let imported: WorkspaceArchiveImportResult;
    if (mode === 'full') {
      if (!metadata.fullData.available || !metadata.fullData.sha256) {
        unavailableWorkspaceFullPackages.value = Array.from(new Set([...unavailableWorkspaceFullPackages.value, entry.entryId]));
        ElMessage.warning(t('workPage.messages.workspaceFullPackageUnavailable'));
        return;
      }
      const diskSpace = await invoke<WorkspaceDiskSpaceReport>('workspace_access_check_disk_space', { path: workspaceBase });
      if (diskSpace.belowRecommended) {
        const availableGiB = ((diskSpace.availableBytes ?? 0) / (1024 ** 3)).toFixed(2);
        const proceed = await ElMessageBox.confirm(t('workPage.dialog.workspaceLibraryLowDiskSpaceMessage', { available: availableGiB }), t('workPage.dialog.workspaceLibraryTitle'), { type: 'warning' }).then(() => true).catch(() => false);
        if (!proceed) return;
      }
      try {
        imported = await invoke<WorkspaceArchiveImportResult>('workspace_access_download_and_import_entry', {
          workerUrl: WORKSPACE_ACCESS_API_URL,
          proxyPort: workspaceAccessProxyPort(),
          entryId: metadata.entryId,
          expectedSha256: metadata.fullData.sha256,
          workspaceBase,
          workspaceName: metadata.workspaceName,
          gamePreset,
        });
      } catch (error) {
        if (String(error).includes('DOWNLOAD_HTTP_404')) {
          unavailableWorkspaceFullPackages.value = Array.from(new Set([...unavailableWorkspaceFullPackages.value, entry.entryId]));
          ElMessage.warning(t('workPage.messages.workspaceFullPackageUnavailable'));
          return;
        }
        throw error;
      }
    } else {
      imported = await invoke<WorkspaceArchiveImportResult>('workspace_access_import_metadata_skeleton', { metadata, workspaceBase, workspaceName: metadata.workspaceName });
    }
    const attribution = metadata.attribution.mode === 'custom' ? metadata.attribution.displayName || 'anonymous' : 'anonymous';
    await writeTextFile(
      await join(imported.workspacePath, 'Config', 'WorkspaceAccess.json'),
      JSON.stringify({ entryId: metadata.entryId, uploadedAt: metadata.uploadedAt, attribution, aliases: metadata.workspaceAliases || [] }, null, 2),
    );
    await refreshWorkspaces();
    await handleWorkspaceSelectionChange(imported.workspaceName);
    void invoke('workspace_access_record_download', {
      workerUrl: WORKSPACE_ACCESS_API_URL,
      entryId: metadata.entryId,
      kind: mode === 'metadata' ? 'metadata' : 'fullPackage',
      proxyPort: workspaceAccessProxyPort(),
    }).catch((error) => {
      console.warn('Workspace download count was not recorded', error);
    });
    workspaceAccessDialog.value = null;
    ElMessage.success(t('workPage.messages.workspaceArchiveImported', { name: imported.workspaceName, files: imported.fileCount }));
  } catch (error) {
    console.error('Workspace library download failed', error);
    ElMessage.error(t('workPage.messages.workspaceLibraryFailed'));
  } finally {
    workspaceDownloadingEntry.value = null;
  }
};

const _legacyWorkspaceAccessActions = async (): Promise<void> => {
  if (!workspaceName.value) {
    ElMessage.warning(t('workPage.messages.selectOrCreateWorkspaceFirst'));
    return;
  }

  try {
    await saveCurrentWorkspaceTabConfig();
    await waitForWorkspaceSaves();
    const workspacePath = await getWorkspaceDirPath(workspaceName.value);
    if (!workspacePath) {
      ElMessage.warning(t('workPage.messages.selectGameAndCacheFirst'));
      return;
    }
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    const outputPath = await saveDialog({
      defaultPath: `${workspaceName.value}.ssmtws`,
      filters: [{ name: 'SSMT Workspace', extensions: ['ssmtws'] }],
    });
    if (!outputPath) return;

    if (isWorkspaceTransitioning.value) return;
    isWorkspaceTransitioning.value = true;
    let result: WorkspaceArchiveBuildResult;
    try {
      result = await invoke<WorkspaceArchiveBuildResult>('workspace_access_create_archive', {
        request: {
          workspacePath,
          workspaceName: workspaceName.value,
          gamePreset,
          outputPath,
        },
      });
    } finally {
      isWorkspaceTransitioning.value = false;
    }
    ElMessage.success(t('workPage.messages.workspaceArchiveCreated', {
      files: result.fileCount,
      size: result.size,
    }));
  } catch (error) {
    console.error('Workspace archive creation failed', error);
    ElMessage.error(t('workPage.messages.workspaceArchiveFailed'));
  }
};

const _legacyImportWorkspaceArchive = async (): Promise<void> => {
  try {
    const archivePath = await openDialog({
      multiple: false,
      filters: [{ name: 'SSMT Workspace', extensions: ['ssmtws'] }],
    });
    if (!archivePath || Array.isArray(archivePath)) return;
    const workspaceBase = await getWorkspaceBaseDir();
    if (!workspaceBase) {
      ElMessage.warning(t('workPage.messages.selectGameAndCacheFirst'));
      return;
    }
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    const prompt = await ElMessageBox.prompt(
      t('workPage.dialog.importWorkspaceNameMessage'),
      t('workPage.dialog.importWorkspaceTitle'),
      { inputPlaceholder: t('workPage.placeholders.enterWorkspaceName'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
    );
    const workspaceName = prompt.value.trim();
    const result = await invoke<WorkspaceArchiveImportResult>('workspace_access_import_archive', {
      archivePath,
      workspaceBase,
      workspaceName,
      gamePreset,
    });
    await refreshWorkspaces();
    await handleWorkspaceSelectionChange(result.workspaceName);
    ElMessage.success(t('workPage.messages.workspaceArchiveImported', { name: result.workspaceName, files: result.fileCount }));
  } catch (error) {
    if (error === 'cancel' || (error instanceof Error && /cancel|取消/.test(error.message))) return;
    console.error('Workspace archive import failed', error);
    ElMessage.error(t('workPage.messages.workspaceArchiveImportFailed'));
  }
};

const _legacyPromptWorkspacePublishDetails = async (withArchive: boolean): Promise<{
  workerUrl: string;
  description?: string;
  capturedAt?: string;
  gameBuild?: string;
  attribution: { mode: 'anonymous' | 'custom'; displayName?: string };
  archivePath?: string;
} | undefined> => {
  const workerUrl = WORKSPACE_ACCESS_API_URL;
  const descriptionInput = await ElMessageBox.prompt(
    t('workPage.dialog.workspaceDescriptionMessage'),
    t('workPage.dialog.workspacePublishTitle'),
    { inputPlaceholder: t('workPage.placeholders.workspaceDescription'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel'), inputType: 'textarea' },
  );
  const capturedAtInput = await ElMessageBox.prompt(
    t('workPage.dialog.workspaceCapturedAtMessage'),
    t('workPage.dialog.workspacePublishTitle'),
    { inputValue: new Date().toISOString(), inputPlaceholder: t('workPage.placeholders.workspaceCapturedAt'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
  );
  const gameBuildInput = await ElMessageBox.prompt(
    t('workPage.dialog.workspaceGameBuildMessage'),
    t('workPage.dialog.workspacePublishTitle'),
    { inputPlaceholder: t('workPage.placeholders.workspaceGameBuild'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
  );
  const attributionInput = await ElMessageBox.prompt(
    t('workPage.dialog.workspaceAttributionMessage'),
    t('workPage.dialog.workspacePublishTitle'),
    { inputPlaceholder: t('workPage.placeholders.workspaceAttribution'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
  );
  let archivePath: string | undefined;
  if (withArchive) {
    const selected = await openDialog({ multiple: false, filters: [{ name: 'SSMT Workspace', extensions: ['ssmtws'] }] });
    if (!selected || Array.isArray(selected)) return undefined;
    archivePath = selected;
  }
  const displayName = attributionInput.value.trim();
  return {
    workerUrl,
    description: descriptionInput.value.trim() || undefined,
    capturedAt: capturedAtInput.value.trim() || undefined,
    gameBuild: gameBuildInput.value.trim() || undefined,
    attribution: displayName ? { mode: 'custom', displayName } : { mode: 'anonymous' },
    archivePath,
  };
};

const _legacyPublishWorkspace = async (withArchive: boolean): Promise<void> => {
  if (!workspaceName.value) {
    ElMessage.warning(t('workPage.messages.selectOrCreateWorkspaceFirst'));
    return;
  }
  try {
    await saveCurrentWorkspaceTabConfig();
    await waitForWorkspaceSaves();
    const workspacePath = await getWorkspaceDirPath(workspaceName.value);
    if (!workspacePath) {
      ElMessage.warning(t('workPage.messages.selectGameAndCacheFirst'));
      return;
    }
    const details = await _legacyPromptWorkspacePublishDetails(withArchive);
    if (!details) return;
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    if (isWorkspaceTransitioning.value) return;
    isWorkspaceTransitioning.value = true;
    if (details.archivePath) {
      activeWorkspaceArchiveUpload.value = { workerUrl: details.workerUrl, archivePath: details.archivePath };
    }
    let result: WorkspacePublishResult;
    try {
      result = await invoke<WorkspacePublishResult>('workspace_access_publish', {
        request: {
          workerUrl: details.workerUrl,
          workspacePath,
          workspaceName: workspaceName.value,
          gamePreset,
          proxyPort: workspaceAccessProxyPort(),
          description: details.description,
          capturedAt: details.capturedAt,
          gameBuild: details.gameBuild,
          attribution: details.attribution,
          archivePath: details.archivePath,
        },
      });
    } finally {
      activeWorkspaceArchiveUpload.value = null;
      isWorkspaceArchiveUploadCancelling.value = false;
      isWorkspaceTransitioning.value = false;
    }
    ElMessage.success(t('workPage.messages.workspacePublished', { entryId: result.entryId }));
  } catch (error) {
    if (error === 'cancel' || (error instanceof Error && /cancel|取消/.test(error.message))) return;
    if (String(error).includes('UPLOAD_CANCELLED')) {
      ElMessage.info(t('workPage.messages.workspaceUploadCancelled'));
      return;
    }
    console.error('Workspace publish failed', error);
    ElMessage.error(t('workPage.messages.workspacePublishFailed'));
  }
};

const _legacyCancelWorkspaceArchiveUpload = async (): Promise<void> => {
  const upload = activeWorkspaceArchiveUpload.value;
  if (!upload || isWorkspaceArchiveUploadCancelling.value) return;
  isWorkspaceArchiveUploadCancelling.value = true;
  try {
    await invoke('workspace_access_cancel_upload', upload);
    ElMessage.info(t('workPage.messages.workspaceUploadCancellationRequested'));
  } catch (error) {
    console.error('Workspace archive upload cancellation failed', error);
    ElMessage.error(t('workPage.messages.workspaceUploadCancelFailed'));
    isWorkspaceArchiveUploadCancelling.value = false;
  }
};

const _legacyBrowseWorkspaceLibrary = async (): Promise<void> => {
  try {
    const gameConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
    const gamePreset = (gameConfig?.gamePreset || appSettings.CurrentGameName).trim();
    const index = await invoke<LibraryIndex>('workspace_access_fetch_index', { rawBaseUrl: null, gamePreset, proxyPort: workspaceAccessProxyPort() });
    if (!index.entries.length) {
      ElMessage.info(t('workPage.messages.workspaceLibraryEmpty'));
      return;
    }
    const queryInput = await ElMessageBox.prompt(
      t('workPage.dialog.workspaceLibrarySearchMessage'),
      t('workPage.dialog.workspaceLibraryTitle'),
      { inputPlaceholder: t('workPage.placeholders.workspaceLibrarySearch'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
    );
    const query = queryInput.value.trim().toLocaleLowerCase();
    const sortInput = await ElMessageBox.prompt(
      t('workPage.dialog.workspaceLibrarySortMessage'),
      t('workPage.dialog.workspaceLibraryTitle'),
      { inputValue: 'uploadedAt', inputPlaceholder: t('workPage.placeholders.workspaceLibrarySort'), confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') },
    );
    const sortKey = ['capturedAt', 'fullDataSize'].includes(sortInput.value.trim()) ? sortInput.value.trim() : 'uploadedAt';
    const entries = index.entries.filter((entry) => !query || [entry.workspaceName, entry.attribution, ...entry.drawIB, ...entry.aliases].join('\n').toLocaleLowerCase().includes(query));
    entries.sort((left, right) => {
      if (sortKey === 'fullDataSize') return right.fullDataSize - left.fullDataSize || right.uploadedAt.localeCompare(left.uploadedAt);
      const leftTime = Date.parse(sortKey === 'capturedAt' ? (left.capturedAt || '') : left.uploadedAt) || 0;
      const rightTime = Date.parse(sortKey === 'capturedAt' ? (right.capturedAt || '') : right.uploadedAt) || 0;
      return rightTime - leftTime || right.uploadedAt.localeCompare(left.uploadedAt);
    });
    entries.splice(50);
    if (!entries.length) {
      ElMessage.info(t('workPage.messages.workspaceLibraryNoMatches'));
      return;
    }
    await ElMessageBox.alert(entries.map((entry, index) => `${index + 1}. ${entry.workspaceName} | ${entry.attribution} (${t('workPage.dialog.workspaceLibraryAttributionUnverified')}) | ${entry.fullDataAvailable ? t('workPage.dialog.workspaceLibraryFullPackage') : t('workPage.dialog.workspaceLibraryMetadataOnly')}${entry.fullDataAvailable ? ` ${entry.fullDataSize} bytes` : ''} | ${entry.entryId}${entry.availability !== 'active' ? ` | ${entry.availability}` : ''}${entry.reviewState !== 'not_required' ? ` | ${t('workPage.dialog.workspaceLibraryReview')}: ${entry.reviewState}` : ''}`).join('\n'), t('workPage.dialog.workspaceLibraryResultsTitle'), { confirmButtonText: t('workPage.common.confirm') });
    const selectedInput = await ElMessageBox.prompt(t('workPage.dialog.workspaceLibrarySelectMessage'), t('workPage.dialog.workspaceLibraryTitle'), { inputPlaceholder: entries[0].entryId, confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') });
    const selected = entries.find((entry) => entry.entryId === selectedInput.value.trim()) || entries[Number.parseInt(selectedInput.value.trim(), 10) - 1];
    if (!selected) {
      ElMessage.warning(t('workPage.messages.workspaceLibraryInvalidSelection'));
      return;
    }
    const metadata = await invoke<LibraryMetadata>('workspace_access_fetch_metadata', { rawBaseUrl: null, metadataPath: selected.metadataPath, proxyPort: workspaceAccessProxyPort() });
    const workspaceBase = await getWorkspaceBaseDir();
    if (!workspaceBase) {
      ElMessage.warning(t('workPage.messages.selectGameAndCacheFirst'));
      return;
    }
    if (metadata.fullData.available && metadata.fullData.sha256) {
      const shouldDownload = await ElMessageBox.confirm(t('workPage.dialog.workspaceLibraryDownloadMessage'), t('workPage.dialog.workspaceLibraryTitle'), { confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel') }).then(() => true).catch(() => false);
      if (!shouldDownload) return;
      const diskSpace = await invoke<WorkspaceDiskSpaceReport>('workspace_access_check_disk_space', { path: workspaceBase });
      if (diskSpace.belowRecommended) {
        const availableGiB = ((diskSpace.availableBytes ?? 0) / (1024 ** 3)).toFixed(2);
        const continueDownload = await ElMessageBox.confirm(
          t('workPage.dialog.workspaceLibraryLowDiskSpaceMessage', { available: availableGiB }),
          t('workPage.dialog.workspaceLibraryTitle'),
          { type: 'warning', confirmButtonText: t('workPage.dialog.workspaceLibraryLowDiskSpaceContinue'), cancelButtonText: t('workPage.common.cancel') },
        ).then(() => true).catch(() => false);
        if (!continueDownload) return;
      }
      const imported = await invoke<WorkspaceArchiveImportResult>('workspace_access_download_and_import_entry', {
        workerUrl: WORKSPACE_ACCESS_API_URL,
        proxyPort: workspaceAccessProxyPort(),
        entryId: metadata.entryId,
        expectedSha256: metadata.fullData.sha256,
        workspaceBase,
        workspaceName: metadata.workspaceName,
        gamePreset,
      });
      await refreshWorkspaces();
      await handleWorkspaceSelectionChange(imported.workspaceName);
      ElMessage.success(t('workPage.messages.workspaceArchiveImported', { name: imported.workspaceName, files: imported.fileCount }));
      return;
    }
    const imported = await invoke<WorkspaceArchiveImportResult>('workspace_access_import_metadata_skeleton', { metadata, workspaceBase, workspaceName: metadata.workspaceName });
    await refreshWorkspaces();
    await handleWorkspaceSelectionChange(imported.workspaceName);
    ElMessage.success(t('workPage.messages.workspaceMetadataImported', { name: imported.workspaceName }));
  } catch (error) {
    if (error === 'cancel' || (error instanceof Error && /cancel|取消/.test(error.message))) return;
    console.error('Workspace library browse failed', error);
    ElMessage.error(t('workPage.messages.workspaceLibraryFailed'));
  }
};

void [_legacyWorkspaceAccessActions, _legacyImportWorkspaceArchive, _legacyPromptWorkspacePublishDetails, _legacyPublishWorkspace, _legacyCancelWorkspaceArchiveUpload, _legacyBrowseWorkspaceLibrary];

const collectSpecificIbDumpDrawIbs = (): string[] => Array.from(
  new Set(
    modelRows.value
      .map((row) => row.drawIB.trim())
      .filter((drawIB) => drawIB !== '')
  )
);

const loadSpecificIbDumpState = async () => {
  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') {
    useSpecificIbDump.value = false;
    specificIbDumpRestoreAnalyseOptions.value = '';
    return;
  }

  const currentConfig = await ResourceManager.loadGameConfig(gameName);
  useSpecificIbDump.value = !!currentConfig?.useSpecificIbDump;
  specificIbDumpRestoreAnalyseOptions.value = currentConfig?.specificIbDumpRestoreAnalyseOptions || '';
};

const persistSpecificIbDumpState = async (enabled: boolean, restoreAnalyseOptions: string) => {
  const gameName = appSettings.CurrentGameName;
  if (!gameName || gameName === 'Default') {
    return;
  }

  const currentConfig = await ResourceManager.loadGameConfig(gameName);
  await ResourceManager.saveGameConfig(gameName, {
    ...currentConfig,
    useSpecificIbDump: enabled,
    specificIbDumpRestoreAnalyseOptions: restoreAnalyseOptions,
  });
};

const handleSpecificIbDumpToggle = async (value: string | number | boolean) => {
  const enabled = value === true;
  if (isSpecificIbDumpToggling.value) {
    return;
  }

  const previousEnabled = !enabled;
  isSpecificIbDumpToggling.value = true;

  try {
    const gameName = appSettings.CurrentGameName;
    if (!gameName || gameName === 'Default') {
      throw new Error(t('workPage.messages.selectGameConfigFirst'));
    }

    if (workspaceName.value) {
      await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
        tabId: activeWorkspaceTabId.value,
        tabs: workspaceTabs.value,
      });
    }

    const currentConfig = await ResourceManager.loadGameConfig(gameName);

    if (enabled) {
      const legacyLogicName = (currentConfig as Record<string, unknown> | undefined)?.LogicName;
      const result = await MigotoManager.enableSpecificIbDumpMode({
        gameName,
        gamePreset: currentConfig?.gamePreset,
        logicName: currentConfig?.logicName || (typeof legacyLogicName === 'string' ? legacyLogicName : undefined),
        drawIbs: collectSpecificIbDumpDrawIbs(),
      });

      specificIbDumpRestoreAnalyseOptions.value = result.restoreAnalyseOptions;
      await persistSpecificIbDumpState(true, result.restoreAnalyseOptions);
      ElMessage.success(t('workPage.messages.specificIbDumpEnabled'));
      return;
    }

    await MigotoManager.disableSpecificIbDumpMode({
      gameName,
      restoreAnalyseOptions: specificIbDumpRestoreAnalyseOptions.value,
    });
    specificIbDumpRestoreAnalyseOptions.value = '';
    await persistSpecificIbDumpState(false, '');
    ElMessage.success(t('workPage.messages.specificIbDumpDisabled'));
  } catch (err) {
    console.error('Failed to toggle specific IB dump mode', err);
    useSpecificIbDump.value = previousEnabled;
    ElMessage.error(t('workPage.messages.specificIbDumpToggleFailed', { error: String(err) }));
  } finally {
    isSpecificIbDumpToggling.value = false;
  }
};

type CommonInvokeArgs = {
  frameAnalysisFolder: string;
  gamePreset: string;
  workspaceRootPath: string;
  lodName: string;
  lodWorkspacePath: string;
};

const resolveCommonInvokeArgs = async (): Promise<CommonInvokeArgs | undefined> => {
  const trimmedFrameAnalysisFolderPath = frameAnalysisFolderPath.value.trim();
  if (!trimmedFrameAnalysisFolderPath) {
    ElMessage.warning(t('workPage.messages.specifyFrameAnalysisPathFirst'));
    return undefined;
  }

  const workspaceBase = await getWorkspaceBaseDir();
  if (!workspaceBase) {
    ElMessage.warning(t('workPage.messages.selectGameAndCacheFirst'));
    return undefined;
  }

  if (!workspaceName.value) {
    ElMessage.warning(t('workPage.messages.selectOrCreateWorkspaceFirst'));
    return undefined;
  }

  // LOD 名称 = 当前激活标签页名称，与 FrameAnalysis 路径无关
  const lodName = normalizeWorkspaceTabName(
    workspaceTabs.value.find((t) => t.id === activeWorkspaceTabId.value)?.name || ''
  );
  if (!lodName) {
    ElMessage.warning(t('workPage.messages.selectOrCreateWorkspaceFirst'));
    return undefined;
  }

  const workspaceRootPath = await join(workspaceBase, workspaceName.value);
  const lodWorkspacePath = await join(workspaceRootPath, lodName);
  const frameAnalysisFolder = trimmedFrameAnalysisFolderPath;
  const currentConfig = await ResourceManager.loadGameConfig(appSettings.CurrentGameName);
  const gamePreset = currentConfig?.gamePreset || appSettings.CurrentGameName;

  return {
    frameAnalysisFolder,
    gamePreset,
    workspaceRootPath,
    lodName,
    lodWorkspacePath,
  };
};

const handleGenerateIBSkip = async () => {
  try {
    const normalizedSkipRows = skipRows.value.map((row) => ({
      skipIB: row.skipIB.trim(),
      aliasName: row.aliasName.trim(),
      indexCount: row.indexCount.trim(),
      firstIndex: row.firstIndex.trim(),
    }));

    const needsAutoExpand = normalizedSkipRows.some((row) =>
      row.skipIB !== '' && row.firstIndex === '' && row.indexCount === ''
    );

    let rowsToGenerate = normalizedSkipRows;
    if (needsAutoExpand) {
      const invokeArgs = await resolveCommonInvokeArgs();
      if (!invokeArgs) return;

      const drawIBSubmeshCache = new Map<string, DrawIBSubmeshRange[]>();
      const expandedRows: SkipRow[] = [];

      for (const row of normalizedSkipRows) {
        if (row.skipIB === '') {
          continue;
        }

        if (row.firstIndex !== '' || row.indexCount !== '') {
          expandedRows.push(row);
          continue;
        }

        const normalizedDrawIB = row.skipIB.toLowerCase();
        let submeshList = drawIBSubmeshCache.get(normalizedDrawIB);
        if (!submeshList) {
          submeshList = await invoke<DrawIBSubmeshRange[]>('analyze_draw_ib_submeshes', {
            frameAnalysisFolder: invokeArgs.frameAnalysisFolder,
            drawIb: row.skipIB,
          });
          drawIBSubmeshCache.set(normalizedDrawIB, submeshList);
        }

        if (submeshList.length === 0) {
          expandedRows.push(row);
          continue;
        }

        for (const submesh of submeshList) {
          expandedRows.push({
            skipIB: row.skipIB,
            aliasName: row.aliasName,
            indexCount: (submesh.indexCount || '').trim(),
            firstIndex: (submesh.firstIndex || '').trim(),
          });
        }
      }

      rowsToGenerate = expandedRows;
    }

    await generateIBSkipToMods(rowsToGenerate, workspaceName.value);
    ElMessage.success(t('workPage.messages.ibskipGenerated'));
  } catch (err) {
    console.error('Failed to generate IBSkip', err);
    ElMessage.error(t('workPage.messages.generateIBSkipFailed', { error: String(err) }));
  }
};

const handleUpdateVSCheck = async () => {
  try {
    if (workspaceName.value) {
      await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
        tabId: activeWorkspaceTabId.value,
        tabs: workspaceTabs.value,
      });
    }
    const invokeArgs = await resolveCommonInvokeArgs();
    if (!invokeArgs) return;

    await invoke('update_vscheck', {
      frameAnalysisFolder: invokeArgs.frameAnalysisFolder,
      gamePreset: invokeArgs.gamePreset,
      workspacePath: invokeArgs.lodWorkspacePath,
    });

    const vsConfigPath = await join(invokeArgs.lodWorkspacePath, 'VSCheckConfig.json');
    try {
      const raw = await readTextFile(vsConfigPath);
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        vsRows.value = parsed.map((item: { Enabled?: boolean; Hash?: string }) => ({
          enabled: item.Enabled !== false,
          hash: item.Hash || '',
        }));
        ensureTrailingVSRow();
      }
    } catch {
      // ignore parse/read failures and keep existing rows
    }

    await saveCurrentWorkspaceTabConfig();
    ElMessage.success(t('workPage.messages.vscheckUpdated'));
  } catch (err) {
    console.error('Failed to update VSCheck list', err);
    ElMessage.error(t('workPage.messages.updateVSCheckFailed', { error: String(err) }));
  }
};

const handleGenerateVSCheck = async () => {
  try {
    if (workspaceName.value) {
      await writeLegacyWorkspaceRuntimeFiles(workspaceName.value, buildCurrentWorkspaceTabConfig(), {
        tabId: activeWorkspaceTabId.value,
        tabs: workspaceTabs.value,
      });
    }
    const invokeArgs = await resolveCommonInvokeArgs();
    if (!invokeArgs) return;

    const generatedModPath = await PathHelper.GetWorkspaceGeneratedModFolderPath(workspaceName.value);
    if (!generatedModPath) {
      ElMessage.warning(t('workPage.messages.generatedModFolderPathResolveFailed'));
      return;
    }

    await invoke('generate_vscheck', {
      frameAnalysisFolder: invokeArgs.frameAnalysisFolder,
      gamePreset: invokeArgs.gamePreset,
      workspacePath: invokeArgs.lodWorkspacePath,
      generatedModFolderPath: generatedModPath,
    });

    await openPath(generatedModPath, t('workPage.messages.openGeneratedModFolderFailed'));
    ElMessage.success(t('workPage.messages.vscheckGenerationTriggered'));
  } catch (err) {
    console.error('Failed to generate VSCheck', err);
    ElMessage.error(t('workPage.messages.generateVSCheckFailed', { error: String(err) }));
  }
};

const openPath = async (path: string | undefined, emptyMsg: string) => {
  if (!path) {
    ElMessage.warning(emptyMsg);
    return;
  }

  try {
    await mkdir(path, { recursive: true });
    await openExternal(path);
  } catch (err) {
    console.error('Failed to open path', path, err);
    ElMessage.error(t('workPage.messages.openFolderFailed'));
  }
};

const openExtractionLog = async (path: string) => {
  try {
    await openExternal(path);
  } catch (err) {
    console.error('Failed to open extraction log', path, err);
    ElMessage.error(t('workPage.messages.openExtractionLogFailed'));
  }
};

const handleFolderMenu = async (cmd: string) => {
  if (cmd === 'migoto') {
    await open3DMigotoFolder();
    return;
  }

  if (cmd === 'mods') {
    await openModsFolder();
    return;
  }

  if (cmd === 'ssmt4GlobalConfigs') {
    await openSSMT4GlobalConfigsFolder();
    return;
  }

  if (cmd === 'latestFA') {
    await openLatestFrameAnalysisFolder(refreshFrameAnalysisFolders);
    return;
  }

  if (cmd === 'latestFALog') {
    await openLatestFrameAnalysisLog(refreshFrameAnalysisFolders);
    return;
  }

  if (cmd === 'latestFADeduped') {
    await openLatestFrameAnalysisDeduped(refreshFrameAnalysisFolders);
  }
};

const writeLegacyWorkspaceRuntimeFiles = async (
  wsName: string,
  tabConfig: WorkPageTabConfig,
  options?: {
    tabId?: string;
    tabs?: WorkspaceTabMeta[];
    drawIBScope?: 'aggregated' | 'active-tab';
    gameKey?: string;
    // Set when the caller already wrote the tab config and frame-analysis
    // files with identical content in the same save pass.
    skipRewrittenConfigFiles?: boolean;
  }
): Promise<void> => {
  if (!wsName) return;

  // LOD 名称 = 当前激活标签页的名称，与 FrameAnalysis 路径无关
  const lodName = options?.tabId && options?.tabs
    ? normalizeWorkspaceTabName(options.tabs.find((t) => t.id === options.tabId)?.name || '')
    : '';
  if (!lodName) return;

  const workspaceDir = await getWorkspaceDirPath(wsName, options?.gameKey);
  const lodWorkspaceDir = await getWorkspaceLodDirPath(wsName, lodName, options?.gameKey);
  if (!workspaceDir || !lodWorkspaceDir) return;

  const configDir = await getWorkspaceConfigDirPath(wsName, options?.gameKey);
  const tabsIndexPath = await getWorkspaceTabsConfigPath(wsName, options?.gameKey);
  const activeTabConfigPath = options?.tabId
    ? await getWorkspaceTabConfigPath(wsName, options.tabId, options?.gameKey)
    : undefined;

  await mkdir(workspaceDir, { recursive: true });
  await mkdir(lodWorkspaceDir, { recursive: true });

  if (configDir) {
    await mkdir(configDir, { recursive: true });
  }

  if (options?.tabId && configDir) {
    await mkdir(await join(configDir, 'Tabs'), { recursive: true });
  }

  if (tabsIndexPath && options?.tabId && options?.tabs) {
    const normalizedTabs = (options?.tabs || []).map((tab) => ({
      id: tab.id,
      name: normalizeWorkspaceTabName(tab.name),
    }));

    await writeTextFile(
      tabsIndexPath,
      JSON.stringify(
        {
          activeTabId: options.tabId,
          tabs: normalizedTabs,
        },
        null,
        2
      )
    );
  }

  if (activeTabConfigPath && !options?.skipRewrittenConfigFiles) {
    await writeTextFile(activeTabConfigPath, JSON.stringify(tabConfig, null, 2));
  }

  if (options?.drawIBScope === 'active-tab') {
    await writeWorkspaceActiveTabDrawIBConfig(wsName, lodName, tabConfig, options?.gameKey);
  } else {
    await writeWorkspaceAggregatedDrawIBConfig(wsName, tabConfig, {
      tabId: options?.tabId,
      tabConfig,
      tabs: options?.tabs,
      gameKey: options?.gameKey,
    });
  }

  const skipConfigPath = await join(lodWorkspaceDir, 'SkipIBConfig.json');
  const skipPayload = tabConfig.skipRows
    .map((row) => ({
      SkipIB: row.skipIB.trim(),
      Alias: row.aliasName.trim(),
      IndexCount: row.indexCount.trim(),
      FirstIndex: row.firstIndex.trim(),
    }))
    .filter((row) => row.SkipIB !== '' || row.Alias !== '' || row.IndexCount !== '' || row.FirstIndex !== '');
  await writeTextFile(skipConfigPath, JSON.stringify(skipPayload, null, 2));

  const vsConfigPath = await join(lodWorkspaceDir, 'VSCheckConfig.json');
  const vsPayload = tabConfig.vsRows
    .map((row) => ({ Enabled: row.enabled !== false, Hash: row.hash.trim() }))
    .filter((row) => row.Hash !== '');
  await writeTextFile(vsConfigPath, JSON.stringify(vsPayload, null, 2));

  // NOTE: gameKey must be forwarded here. Saves run through a serial queue and
  // may execute after the user switched games; resolving this path against the
  // *current* game would write this workspace's frame-analysis config into the
  // same-named workspace of a different game.
  const frameAnalysisConfigPath = await getWorkspaceFrameAnalysisConfigPath(wsName, options?.gameKey);
  if (frameAnalysisConfigPath && !options?.skipRewrittenConfigFiles) {
    await writeTextFile(
      frameAnalysisConfigPath,
      JSON.stringify(
        {
          frameAnalysisFolderPath: tabConfig.frameAnalysisFolderPath.trim(),
          selectedFrameAnalysis: tabConfig.selectedFrameAnalysis.trim(),
        },
        null,
        2
      )
    );
  }
};

const handleSelectWorkspaceTab = async (tabId: string) => {
  if (tabId === activeWorkspaceTabId.value) return;
  activeWorkspaceTabId.value = tabId;
};

const handleAddWorkspaceTab = async () => {
  const tab = createDefaultWorkspaceTabMeta();
  workspaceTabs.value.push(tab);
  activeWorkspaceTabId.value = tab.id;
  await saveWorkspaceTabsIndex();
};

const handleDeleteWorkspaceTab = async (tabId: string) => {
  if (workspaceTabs.value.length <= 1) {
    ElMessage.warning(t('workPage.messages.keepAtLeastOneTab'));
    return;
  }

  const tab = workspaceTabs.value.find((item) => item.id === tabId);
  if (!tab) return;

  try {
    await ElMessageBox.confirm(
      t('workPage.messages.confirmDeleteWorkspaceTab', { name: tab.name }),
      t('workPage.dialog.confirmDeleteWorkspaceTabTitle'),
      { confirmButtonText: t('workPage.common.delete'), cancelButtonText: t('workPage.common.cancel'), type: 'warning' }
    );
  } catch {
    return;
  }

  // Invalidate queued/in-flight saves for the tab being deleted. The
  // activeWorkspaceTabId watcher below enqueues a final save for the old tab;
  // without this bump that save would run after the recycle-bin move and
  // recreate the deleted tab's config file.
  if (workspaceName.value) {
    bumpWorkspaceSaveEpoch(workspaceTabSaveEpochKey(getCurrentWorkspaceMemoryGameKey(), workspaceName.value, tabId));
  }

  if (activeWorkspaceTabId.value === tabId) {
    await saveCurrentWorkspaceTabConfig();
  }

  const index = workspaceTabs.value.findIndex((item) => item.id === tabId);
  if (index < 0) return;
  workspaceTabs.value.splice(index, 1);

  const nextActive = workspaceTabs.value[index] || workspaceTabs.value[index - 1] || workspaceTabs.value[0];
  activeWorkspaceTabId.value = nextActive.id;

  if (workspaceName.value) {
    const tabConfigPath = await getWorkspaceTabConfigPath(workspaceName.value, tabId);
    if (tabConfigPath) {
      try {
        await moveFileToRecycleBin(tabConfigPath);
      } catch {
        // ignore if file not exists
      }
    }
  }

  editingWorkspaceTabId.value = null;
  await saveWorkspaceTabsIndex();
};

// quick menu actions removed — use explicit buttons in UI instead

const loadWorkspaceMergeConfig = async (
  sourceWorkspaceName: string,
  lodName: string | null,
  gameKey?: string
): Promise<WorkPageTabConfig> => {
  if (!lodName) return normalizeWorkspaceTabConfig();
  const index = await readWorkspaceTabsIndexBySnapshot(sourceWorkspaceName, gameKey);
  const tab = index?.tabs.find((item) => item.name.localeCompare(lodName, undefined, { sensitivity: 'accent' }) === 0);
  return tab
    ? readWorkspaceTabConfigBySnapshot(sourceWorkspaceName, tab.id, gameKey)
    : normalizeWorkspaceTabConfig();
};

const filterWorkspaceMergeConfig = (
  config: WorkPageTabConfig,
  source: 'first' | 'second'
): WorkPageTabConfig => {
  const shouldKeep = (hash: string): boolean => {
    const normalizedHash = hash.trim().toLowerCase();
    return !workspaceMergeConflictingHashes.value.includes(normalizedHash)
      || workspaceMergeHashPreferences.value[normalizedHash] === source;
  };
  const filtered = cloneWorkspaceTabConfig(config);
  filtered.modelRows = filtered.modelRows.filter((row) => shouldKeep(row.drawIB));
  return filtered;
};

const mergeWorkspaceTabConfigs = (
  firstConfig: WorkPageTabConfig,
  secondConfig: WorkPageTabConfig
): WorkPageTabConfig => {
  const first = filterWorkspaceMergeConfig(firstConfig, 'first');
  const second = filterWorkspaceMergeConfig(secondConfig, 'second');
  const mergeRows = <T>(
    firstRows: T[],
    secondRows: T[],
    hash: (row: T) => string,
    useDrawIBPreference = false
  ): T[] => {
    const result = [...firstRows];
    const indexes = new Map(result.map((row, index) => [hash(row).trim().toLowerCase(), index]));
    for (const row of secondRows) {
      const key = hash(row).trim().toLowerCase();
      const existing = indexes.get(key);
      if (existing === undefined) {
        indexes.set(key, result.length);
        result.push(row);
      } else if (useDrawIBPreference && workspaceMergeHashPreferences.value[key] === 'second') {
        result[existing] = row;
      }
    }
    return result;
  };

  return {
    ...first,
    modelRows: mergeRows(first.modelRows, second.modelRows, (row) => row.drawIB, true),
    skipRows: mergeRows(first.skipRows, second.skipRows, (row) => row.skipIB),
    vsRows: mergeRows(first.vsRows, second.vsRows, (row) => row.hash),
  };
};

const writeMergedWorkspaceConfiguration = async (result: WorkspaceMergeResult, gameKey?: string): Promise<void> => {
  const tabs = result.lods.map((lod) => ({ id: createWorkspaceTabId(), name: lod.name }));
  for (let index = 0; index < result.lods.length; index += 1) {
    const lod = result.lods[index];
    const [firstConfig, secondConfig] = await Promise.all([
      loadWorkspaceMergeConfig(workspaceMergeFirst.value, lod.firstLodName, gameKey),
      loadWorkspaceMergeConfig(workspaceMergeSecond.value, lod.secondLodName, gameKey),
    ]);
    const tabConfig = lod.firstLodName && lod.secondLodName
      ? mergeWorkspaceTabConfigs(firstConfig, secondConfig)
      : lod.firstLodName
        ? filterWorkspaceMergeConfig(firstConfig, 'first')
        : filterWorkspaceMergeConfig(secondConfig, 'second');
    await writeLegacyWorkspaceRuntimeFiles(result.workspaceName, tabConfig, {
      tabId: tabs[index].id,
      tabs,
      drawIBScope: 'active-tab',
      gameKey,
    });
  }
};

const refreshWorkspaceMergePreview = async (): Promise<void> => {
  const first = findWorkspaceMergeOption(workspaceMergeFirst.value);
  const second = findWorkspaceMergeOption(workspaceMergeSecond.value);
  workspaceMergeConflictingHashes.value = [];
  workspaceMergeHashPreferences.value = {};
  if (!first || !second || first === second) return;

  workspaceMergePreviewLoading.value = true;
  try {
    const base = await getWorkspaceBaseDir();
    if (!base) return;
    const preview = await invoke<WorkspaceMergePreview>('workspace_merge_preview', {
      workspaceBase: base,
      firstWorkspaceName: first,
      secondWorkspaceName: second,
    });
    workspaceMergeConflictingHashes.value = preview.conflictingHashes;
  } catch (error) {
    console.error('Failed to preview workspace merge', error);
    ElMessage.error(t('workPage.messages.workspaceMergePreviewFailed', { error: String(error) }));
  } finally {
    workspaceMergePreviewLoading.value = false;
  }
};

const refreshWorkspaceMergeOptions = async (): Promise<string[]> => {
  const base = await getWorkspaceBaseDir();
  if (!base) {
    workspaceMergeOptions.value = [];
    return [];
  }
  try {
    const options = (await readDir(base))
      .filter((entry) => entry.isDirectory && !!entry.name)
      .map((entry) => entry.name as string)
      .sort((left, right) => left.localeCompare(right, undefined, { numeric: true, sensitivity: 'base' }));
    workspaceMergeOptions.value = options;
    return options;
  } catch {
    workspaceMergeOptions.value = [];
    return [];
  }
};

const openWorkspaceMergeDialog = async (): Promise<void> => {
  const options = await refreshWorkspaceMergeOptions();
  if (options.length < 2) {
    ElMessage.warning(t('workPage.messages.workspaceMergeNeedsTwo'));
    return;
  }
  const first = findWorkspaceMergeOption(workspaceName.value) || options[0];
  const second = options.find((name) => name !== first) || '';
  workspaceMergeFirst.value = first;
  workspaceMergeSecond.value = second;
  workspaceMergeOutput.value = `${first}_${second}`;
  workspaceMergeMode.value = 'chain';
  workspaceMergeDialog.value = true;
  await refreshWorkspaceMergePreview();
};

const handleWorkspaceMerge = async (): Promise<void> => {
  const first = findWorkspaceMergeOption(workspaceMergeFirst.value);
  const second = findWorkspaceMergeOption(workspaceMergeSecond.value);
  const output = normalizeWorkspaceNameInput(workspaceMergeOutput.value);
  if (!first || !second || first === second) {
    ElMessage.warning(t('workPage.messages.workspaceMergeSelectTwo'));
    return;
  }
  if (!output || !isValidWindowsFileName(output)) {
    ElMessage.warning(t('workPage.messages.workspaceMergeInvalidName'));
    return;
  }
  if (findExistingWorkspaceOption(output)) {
    ElMessage.warning(t('workPage.messages.workspaceAlreadyExists'));
    return;
  }
  if (workspaceMergeConflictingHashes.value.some((hash) => !workspaceMergeHashPreferences.value[hash])) {
    ElMessage.warning(t('workPage.messages.workspaceMergeChooseConflicts'));
    return;
  }

  const base = await getWorkspaceBaseDir();
  if (!base) return;
  const sourceSnapshot = createWorkspaceTabSaveSnapshot();
  // A merge can take a while; pin every read/write to the game that was
  // selected when it started so a mid-merge game switch cannot leak files
  // into another game's workspace folder.
  const mergeGameKey = getCurrentWorkspaceMemoryGameKey();
  workspaceMergeBusy.value = true;
  try {
    if (sourceSnapshot) await flushCurrentWorkspaceTabConfig(sourceSnapshot);
    const result = await invoke<WorkspaceMergeResult>('workspace_merge', {
      workspaceBase: base,
      firstWorkspaceName: first,
      secondWorkspaceName: second,
      outputWorkspaceName: output,
      mode: workspaceMergeMode.value,
      hashPreferences: workspaceMergeHashPreferences.value,
    });
    if (mergeGameKey !== getCurrentWorkspaceMemoryGameKey()) return;
    await writeMergedWorkspaceConfiguration(result, mergeGameKey);
    await refreshWorkspaces();
    await switchWorkspace(result.workspaceName);
    workspaceMergeDialog.value = false;
    ElMessage.success(t('workPage.messages.workspaceMerged', { name: result.workspaceName, files: result.copiedFileCount }));
  } catch (error) {
    console.error('Workspace merge failed', error);
    ElMessage.error(t('workPage.messages.workspaceMergeFailed', { error: String(error) }));
  } finally {
    workspaceMergeBusy.value = false;
  }
};

watch([workspaceMergeFirst, workspaceMergeSecond], () => {
  if (workspaceMergeDialog.value) void refreshWorkspaceMergePreview();
});

watch(() => appSettings.CurrentGameName, () => {
  workspaceMergeDialog.value = false;
  workspaceMergeOptions.value = [];
});

const refreshWorkspaces = async () => {
    const scannedGameKey = getCurrentWorkspaceMemoryGameKey();
    isScanningWorkspaces.value = true;
    try {
        const baseDir = await getWorkspaceBaseDir();
        if(!baseDir) return;
    debugLog('WorkPage', 'refreshWorkspaces - baseDir', baseDir);

        // Ensure base directory exists
        await mkdir(baseDir, { recursive: true });

        const entries = await readDir(baseDir);
        const folders = entries
            .filter(e => e.isDirectory)
            .map(e => e.name);

        const modifiedTimes = Object.fromEntries(await Promise.all(
          folders.map(async (name) => {
            try {
              const metadata = await stat(await join(baseDir, name));
              return [name, metadata.mtime?.getTime() ?? 0] as const;
            } catch {
              return [name, 0] as const;
            }
          })
        ));
        if (scannedGameKey !== getCurrentWorkspaceMemoryGameKey()) return;
        workspaceModifiedTimes.value = modifiedTimes;
        workspaceOptions.value = folders;
        debugLog('WorkPage', 'refreshWorkspaces - found folders', folders);

        // Auto create Default if empty
          if (folders.length === 0) {
            debugLog('WorkPage', 'refreshWorkspaces - no workspaces found, creating Default');
            await createDefaultWorkspace();
            return;
        }

        const currentWorkspaceName = findExistingWorkspaceOption(workspaceName.value);
        const savedWorkspaceName = findExistingWorkspaceOption(getRememberedWorkspaceForCurrentGame());
        const fallbackWorkspaceName = findExistingWorkspaceOption(DEFAULT_WORKSPACE_NAME) || folders[0];
        const targetWorkspaceName = currentWorkspaceName || savedWorkspaceName || fallbackWorkspaceName;

        if (!targetWorkspaceName) {
          workspaceDraftName.value = '';
          return;
        }

        if (currentWorkspaceName === targetWorkspaceName) {
          workspaceDraftName.value = targetWorkspaceName;
          return;
        }

        debugLog('WorkPage', 'refreshWorkspaces - switching to', targetWorkspaceName);
        await switchWorkspace(targetWorkspaceName);

    } catch (err) {
        console.error('Failed to refresh workspaces', err);
    } finally {
        isScanningWorkspaces.value = false;
    }
};

const createWorkspaceDirectory = async (name: string, options?: { showMessage?: boolean; switchAfterCreate?: boolean }) => {
    const normalizedName = normalizeWorkspaceNameInput(name);
    if (!normalizedName) return;
    try {
        const baseDir = await getWorkspaceBaseDir();
        if (!baseDir) {
            ElMessage.warning(t('workPage.messages.cacheDirNotConfigured')); 
           return;
        }
        const targetPath = await join(baseDir, normalizedName);
        await mkdir(targetPath, { recursive: true });
        
        // Refresh list
        const entries = await readDir(baseDir);
         const folders = entries
            .filter(e => e.isDirectory)
            .map(e => e.name);
         workspaceModifiedTimes.value = Object.fromEntries(await Promise.all(
           folders.map(async (folderName) => {
             try {
               const metadata = await stat(await join(baseDir, folderName));
               return [folderName, metadata.mtime?.getTime() ?? 0] as const;
             } catch {
               return [folderName, 0] as const;
             }
           })
         ));
         workspaceOptions.value = folders;
         workspaceDraftName.value = normalizedName;

         if (options?.switchAfterCreate) {
           await switchWorkspace(normalizedName);
         }
         
         if (options?.showMessage !== false) {
           ElMessage.success(t('workPage.messages.workspaceReady', { name: normalizedName }));
         }
    } catch (err) {
        console.error('Failed to create workspace', err);
        ElMessage.error(t('workPage.messages.createWorkspaceFailed', { name: normalizedName }));
    }
};

const createDefaultWorkspace = async (): Promise<void> => {
  await createWorkspaceDirectory(DEFAULT_WORKSPACE_NAME, { showMessage: false, switchAfterCreate: true });
};


const handleCreateWorkspace = async () => {
  const targetWorkspaceName = normalizeWorkspaceNameInput(workspaceDraftName.value);
  if (!targetWorkspaceName) {
    ElMessage.warning(t('workPage.messages.enterWorkspaceName'));
    return;
  }

  // Confirm creation to avoid mis-clicks
  try {
    await ElMessageBox.confirm(
      t('workPage.messages.confirmCreateWorkspace', { name: targetWorkspaceName }),
      t('workPage.dialog.confirmCreationTitle'),
      { confirmButtonText: t('workPage.common.create'), cancelButtonText: t('workPage.common.cancel'), type: 'warning' }
    );
  } catch (err: unknown) {
    if (err && (err === 'cancel' || (err instanceof Error && /cancel|取消/.test(err.message)))) return;
    // otherwise continue (but it's safe to return)
    return;
  }

  // Check if duplicate
  if (findExistingWorkspaceOption(targetWorkspaceName)) {
    ElMessage.info(t('workPage.messages.workspaceAlreadyExists'));
     return;
  }

  await createWorkspaceDirectory(targetWorkspaceName, { switchAfterCreate: true });
};

let hasSeenInitialActivation = false;

onMounted(() => {
  void (async () => {
    try {
      unlistenNativeDrop = await listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
        if (router.currentRoute.value.path !== '/work') return;
        const payload = event?.payload as { paths?: string[] };
        const firstPath = (payload?.paths?.[0] || '').trim();
        if (!firstPath) {
          return;
        }

        await applyDroppedFrameAnalysisFolder(firstPath);
      });
    } catch (error) {
      debugError('WorkPage.DnD', 'Failed to attach native drop listener', error);
    }
  })();
  void (async () => {
    try {
      unlistenWorkspaceUploadProgress = await listen<WorkspaceUploadProgress>('workspace-access-upload-progress', (event) => {
        workspaceUploadProgress.value = event.payload;
        // This first event is emitted only after the Worker accepted the
        // submission and local resumable state was written.
        if (workspaceAccessDialog.value === 'upload') workspaceAccessDialog.value = null;
      });
    } catch (error) {
      debugError('WorkPage.WorkspaceAccess', 'Failed to attach upload progress listener', error);
    }
  })();

  ensureTrailingEmptyRow();
  ensureTrailingSkipRow();
  ensureTrailingVSRow();
  void (async () => {
    await waitForInitialAppState();
    await loadSpecificIbDumpState();
    await refreshWorkspaces();
  })();
  
  // If options are empty, try auto-refresh.
  if (frameAnalysisOptions.value.length === 0) {
      refreshFrameAnalysisFolders();
  }
  debugLog('WorkPage', 'onMounted - initialized');
});

onBeforeUnmount(() => {
  const snapshot = createWorkspaceTabSaveSnapshot();
  if (snapshot) {
    void flushCurrentWorkspaceTabConfig(snapshot);
  }

  if (unlistenNativeDrop) {
    unlistenNativeDrop();
    unlistenNativeDrop = null;
  }
  if (unlistenWorkspaceUploadProgress) {
    unlistenWorkspaceUploadProgress();
    unlistenWorkspaceUploadProgress = null;
  }
});

// If KeepAlive is enabled, onActivated will also fire.
onActivated(() => {
    // KeepAlive activates once immediately after mount. onMounted already owns
    // that initialization, so do not scan every workspace twice.
    if (!hasSeenInitialActivation) {
      hasSeenInitialActivation = true;
      return;
    }
    if (frameAnalysisOptions.value.length === 0) {
      refreshFrameAnalysisFolders();
    }
    void loadSpecificIbDumpState();
    // Also refresh workspace list on activation to ensure current selection and configs are re-read
    refreshWorkspaces();
    debugLog('WorkPage', 'onActivated - refreshed frame analysis and workspaces');
});


const handleCreateFromConfig = async () => {
  // Workflow: remember current DrawIB list -> create/switch to target workspace ->
  // restore remembered DrawIB list and save.
  const targetWorkspaceName = normalizeWorkspaceNameInput(workspaceDraftName.value);
  if (!targetWorkspaceName) {
    ElMessage.warning(t('workPage.messages.enterWorkspaceNameToCreate'));
    return;
  }

  const rememberedConfig = cloneWorkspaceTabConfig(buildCurrentWorkspaceTabConfig());
  const sourceSnapshot = createWorkspaceTabSaveSnapshot();

  try {
    // Confirm creation to avoid mis-clicks
    try {
      await ElMessageBox.confirm(
        t('workPage.messages.confirmCreateAndApply', { name: targetWorkspaceName }),
        t('workPage.dialog.confirmCreateAndApplyTitle'),
        { confirmButtonText: t('workPage.common.create'), cancelButtonText: t('workPage.common.cancel'), type: 'warning' }
      );
    } catch (err: unknown) {
      if (err && (err === 'cancel' || (err instanceof Error && /cancel|取消/.test(err.message)))) return;
      return;
    }

    if (findExistingWorkspaceOption(targetWorkspaceName)) {
      ElMessage.info(t('workPage.messages.workspaceAlreadyExists'));
      return;
    }

    if (sourceSnapshot) {
      await flushCurrentWorkspaceTabConfig(sourceSnapshot);
    }

    await createWorkspaceDirectory(targetWorkspaceName, { switchAfterCreate: true });

    await applyWorkspaceTabConfig(rememberedConfig);
    await saveWorkspaceTabsIndex();
    await saveCurrentWorkspaceTabConfig();
    ElMessage.success(t('workPage.messages.workspaceCreatedAndApplied', { name: targetWorkspaceName }));
  } catch (err) {
    console.error('handleCreateFromConfig failed', err);
    ElMessage.error(t('workPage.messages.createAndSwitchWorkspaceFailed'));
  }
};

const handleOpenWorkspace = async () => {
    const baseDir = await getWorkspaceBaseDir();

    // Normally no need to check workspace dir existence here because this page ensures it.
    // Kept as a defensive guard for rare manual deletions.
    if (!baseDir || !workspaceName.value) {
          ElMessage.warning(t('workPage.messages.selectValidWorkspaceFirst'));
         return;
    }
    const path = await join(baseDir, workspaceName.value);
    await mkdir(path, { recursive: true });
        openPath(path, t('workPage.messages.openFailed'));
};

const handleOpenWorkspaceGeneratedModFolder = async () => {
  if (!workspaceName.value) {
    ElMessage.warning(t('workPage.messages.selectValidWorkspaceFirst'));
    return;
  }

  const generatedModPath = await PathHelper.GetWorkspaceGeneratedModFolderPath(workspaceName.value);
  await openPath(generatedModPath, t('workPage.messages.openGeneratedModFolderFailed'));
};



const handleClearWorkspace = async () => {
  if (!workspaceName.value) return;
  const currentWorkspaceName = workspaceName.value;
  try {
    await ElMessageBox.confirm(
      t('workPage.messages.confirmClearWorkspace', { name: currentWorkspaceName }),
      t('workPage.dialog.confirmClearTitle'),
      { confirmButtonText: t('workPage.common.confirm'), cancelButtonText: t('workPage.common.cancel'), type: 'warning' }
    );

    const baseDir = await getWorkspaceBaseDir();
    if (!baseDir) return;

    const workspaceDir = await join(baseDir, currentWorkspaceName);
    await mkdir(workspaceDir, { recursive: true });

  clearPendingSaveTimers();
  // Cancel any in-flight switch and invalidate queued saves for this workspace:
  // a stale save running after the cleanup would recreate the files we are
  // about to remove.
  workspaceSwitchRevision += 1;
  bumpWorkspaceSaveEpoch(workspaceSaveEpochKey(getCurrentWorkspaceMemoryGameKey(), currentWorkspaceName));
  setAllConfigLoading(true);

    // Remove all contents in workspace directory, keep the directory itself.
    const entries = await readDir(workspaceDir);
    for (const entry of entries) {
      if (!entry.name) {
        continue;
      }

      const targetPath = await join(workspaceDir, entry.name);
      try {
        if (entry.isDirectory) {
          await moveDirectoryToRecycleBin(targetPath);
        } else {
          await moveFileToRecycleBin(targetPath);
        }
      } catch {
        // Ignore single-entry failure and continue cleanup.
      }
    }

    createInMemoryDefaultWorkspaceState();
    workspaceDraftName.value = currentWorkspaceName;
    await saveWorkspaceTabsIndexBySnapshot(currentWorkspaceName, activeWorkspaceTabId.value, buildWorkspaceTabsSnapshot());
    await saveWorkspaceTabConfigBySnapshot(
      currentWorkspaceName,
      activeWorkspaceTabId.value,
      normalizeWorkspaceTabConfig(),
      buildWorkspaceTabsSnapshot()
    );
    setAllConfigLoading(false);

    ElMessage.success(t('workPage.messages.workspaceCleared'));
  } catch (err: unknown) {
    setAllConfigLoading(false);
    // If user canceled, ignore.
    if (err && err === 'cancel') return;
    // Element Plus confirm throws an object on cancel, safe to ignore.
    if (err && err instanceof Error && /cancel|取消/.test(err.message)) return;
    ElMessage.error(t('workPage.messages.clearWorkspaceFailed'));
    console.error(err);
  }
};

const handleDeleteWorkspace = async (targetWorkspaceName = workspaceName.value) => {
   const currentWorkspaceName = findExistingWorkspaceOption(targetWorkspaceName);
   if (!currentWorkspaceName) return;
   try {
    await ElMessageBox.confirm(
      t('workPage.messages.confirmDeleteWorkspace', { name: currentWorkspaceName }),
      t('workPage.dialog.confirmDeleteTitle'),
      { confirmButtonText: t('workPage.common.delete'), cancelButtonText: t('workPage.common.cancel'), type: 'warning' }
    );

    const baseDir = await getWorkspaceBaseDir();
    if (!baseDir) return;
    const path = await join(baseDir, currentWorkspaceName);

    clearPendingSaveTimers();
    // Cancel any in-flight switch and invalidate queued saves for the deleted
    // workspace: a stale save running after the recycle-bin move would
    // recreate the deleted folder (mkdir recursive) as a ghost workspace.
    workspaceSwitchRevision += 1;
    bumpWorkspaceSaveEpoch(workspaceSaveEpochKey(getCurrentWorkspaceMemoryGameKey(), currentWorkspaceName));
    setAllConfigLoading(true);

    await movePathToRecycleBin(path);
    ElMessage.success(t('workPage.messages.workspaceDeleted', { name: currentWorkspaceName }));

    if (currentWorkspaceName === workspaceName.value) {
      workspaceName.value = '';
      workspaceDraftName.value = '';
      rememberWorkspaceForCurrentGame('');
      resetLeftThreeLists();
      workspaceTabs.value = [];
      activeWorkspaceTabId.value = '';
      editingWorkspaceTabId.value = null;
      workspaceTabNameEditBackup.value = {};
    } else if (currentWorkspaceName === workspaceDraftName.value) {
      workspaceDraftName.value = workspaceName.value;
    }

    await nextTick();

    setAllConfigLoading(false);
    await refreshWorkspaces();
   } catch (err: unknown) {
     setAllConfigLoading(false);
     if (err && err === 'cancel') return;
     if (err && err instanceof Error && /cancel|取消/.test(err.message)) return;
    ElMessage.error(t('workPage.messages.deleteFailed'));
     console.error(err);
   }
};
</script>

<template>
  <div class="work-page-container" :class="{ 'is-video-background': appSettings.bgType === BGType.Video }">
    <div class="work-layout">
      <div class="main-column">
        <div class="panel-stack glass-scrollbar">
          <section class="panel workspace-tabs-shell">
            <div class="workspace-tabs-head">
              <div class="workspace-tabs-list">
                <div
                  v-for="tab in workspaceTabs"
                  :key="tab.id"
                  class="workspace-tab-chip"
                  :class="{ 'is-active': tab.id === activeWorkspaceTabId }"
                  @click="handleSelectWorkspaceTab(tab.id)"
                >
                  <span class="workspace-tab-name">
                    {{ tab.name }}
                  </span>

                  <el-button
                    text
                    class="workspace-tab-delete"
                    @click.stop="handleDeleteWorkspaceTab(tab.id)"
                  >
                    ×
                  </el-button>
                </div>
              </div>

              <el-button plain @click="handleAddWorkspaceTab">
                {{ t('workPage.actions.addWorkspaceTab') }}
              </el-button>
            </div>

            <div class="workspace-tabs-content">
              <ExtractConfigSection
                v-model:selectedFrameAnalysis="selectedFrameAnalysis"
                v-model:frameAnalysisFolderPath="frameAnalysisFolderPath"
                v-model:extractPanelTab="extractPanelTab"
                v-model:modelRows="modelRows"
                v-model:fullExtractDataTypeFilter="fullExtractDataTypeFilter"
                :frameAnalysisOptions="frameAnalysisOptions"
                :isRefreshing="isRefreshing"
                :isFrameAnalysisPathInvalid="isFrameAnalysisPathInvalid"
                :isExtracting="isExtracting"
                :hasLatestExtractionLog="!!currentLatestExtractionLogPath"
                :fullExtractDataTypeFilterOptions="FULL_EXTRACT_DATA_TYPE_FILTER_OPTIONS"
                @refresh="refreshFrameAnalysisFolders"
                @selectLatest="handleSelectLatestFrameAnalysis"
                @analyzeMissingGameTypes="handleAnalyzeMissingGameTypes"
                @pickFolder="handlePickFrameAnalysisFolder"
                @openFolder="handleOpenFrameAnalysisFolderPath"
                @dropFolder="handleDropFrameAnalysisFolder"
                @selectFrameAnalysisOption="handleFrameAnalysisOptionClick"
                @moveModelRow="moveModelRow"
                @removeModelRow="removeModelRow"
                @extractModels="handleExtractModels"
                @fullExtract="handleFullExtract"
                @openLatestExtractionLog="handleOpenLatestExtractionLog"
              />

              <ConfigTables
                v-model:skip-rows="skipRows"
                v-model:vs-rows="vsRows"
                @removeSkipRow="removeSkipRow"
                @moveSkipRow="moveSkipRow"
                @generateIBSkip="handleGenerateIBSkip"
                @removeVSCheckRow="removeVSCheckRow"
                @moveVSCheckRow="moveVSCheckRow"
                @updateVSCheck="handleUpdateVSCheck"
                @generateVSCheck="handleGenerateVSCheck"
              />
            </div>
          </section>
        </div>
      </div>

      <SidePanel
        v-model:workspaceDraftName="workspaceDraftName"
        v-model:drawerCollapsed="workPageDrawerCollapsed"
        v-model:useSpecificIbDump="useSpecificIbDump"
        :workspaceName="workspaceName"
        :workspaceOptions="workspaceOptions"
        :workspaceModifiedTimes="workspaceModifiedTimes"
        :workspaceProvenance="workspaceProvenance"
        :isSpecificIbDumpToggling="isSpecificIbDumpToggling"
        :workspaceUploadProgress="workspaceUploadPercent"
        :workspaceUploadActive="workspaceUploadProgress !== null"
        @createWorkspace="handleCreateWorkspace"
        @createFromConfig="handleCreateFromConfig"
        @openWorkspace="handleOpenWorkspace"
        @openGeneratedMod="handleOpenWorkspaceGeneratedModFolder"
        @clearWorkspace="handleClearWorkspace"
        @deleteWorkspace="handleDeleteWorkspace"
        @selectWorkspace="handleWorkspaceSelectionChange"
        @folderMenu="handleFolderMenu"
        @textureMenu="handleTextureMenu"
        @openWorkspaceUpload="openWorkspaceUploadDialog"
        @openWorkspaceDownload="openWorkspaceDownloadDialog"
        @openWorkspaceMerge="openWorkspaceMergeDialog"
        @specificIbDumpToggle="handleSpecificIbDumpToggle"
      />

    </div>

    <el-dialog
      class="workspace-merge-dialog"
      :model-value="workspaceMergeDialog"
      width="min(680px, 94vw)"
      :close-on-click-modal="!workspaceMergeBusy"
      @update:model-value="(visible) => { if (!visible && !workspaceMergeBusy) workspaceMergeDialog = false; }"
    >
      <template #header>{{ t('workPage.dialog.workspaceMergeTitle') }}</template>
      <el-form label-position="top">
        <div class="workspace-merge-sources">
          <el-form-item :label="t('workPage.dialog.workspaceMergeFirst')">
            <el-select v-model="workspaceMergeFirst" :disabled="workspaceMergeBusy" @change="refreshWorkspaceMergePreview">
              <el-option v-for="name in workspaceMergeOptions" :key="`first-${name}`" :label="name" :value="name" />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('workPage.dialog.workspaceMergeSecond')">
            <el-select v-model="workspaceMergeSecond" :disabled="workspaceMergeBusy" @change="refreshWorkspaceMergePreview">
              <el-option v-for="name in workspaceMergeOptions" :key="`second-${name}`" :label="name" :value="name" :disabled="name === workspaceMergeFirst" />
            </el-select>
          </el-form-item>
        </div>
        <el-form-item :label="t('workPage.dialog.workspaceMergeOutput')">
          <el-input v-model="workspaceMergeOutput" :disabled="workspaceMergeBusy" />
        </el-form-item>
        <el-form-item :label="t('workPage.dialog.workspaceMergeMode')">
          <el-radio-group v-model="workspaceMergeMode" :disabled="workspaceMergeBusy">
            <el-radio value="chain">{{ t('workPage.dialog.workspaceMergeChain') }}</el-radio>
            <el-radio value="zip">{{ t('workPage.dialog.workspaceMergeZip') }}</el-radio>
          </el-radio-group>
          <p class="workspace-merge-hint">{{ workspaceMergeMode === 'chain' ? t('workPage.dialog.workspaceMergeChainHint') : t('workPage.dialog.workspaceMergeZipHint') }}</p>
        </el-form-item>
        <section v-loading="workspaceMergePreviewLoading" class="workspace-merge-conflicts">
          <p class="workspace-merge-conflicts-title">{{ t('workPage.dialog.workspaceMergeConflicts') }}</p>
          <el-empty v-if="!workspaceMergePreviewLoading && workspaceMergeConflictingHashes.length === 0" :description="t('workPage.dialog.workspaceMergeNoConflicts')" :image-size="48" />
          <div v-for="hash in workspaceMergeConflictingHashes" :key="hash" class="workspace-merge-conflict">
            <code>{{ hash }}</code>
            <el-radio-group v-model="workspaceMergeHashPreferences[hash]" :disabled="workspaceMergeBusy">
              <el-radio value="first">{{ workspaceMergeFirst }}</el-radio>
              <el-radio value="second">{{ workspaceMergeSecond }}</el-radio>
            </el-radio-group>
          </div>
        </section>
      </el-form>
      <template #footer>
        <el-button :disabled="workspaceMergeBusy" @click="workspaceMergeDialog = false">{{ t('workPage.common.cancel') }}</el-button>
        <el-button type="primary" :loading="workspaceMergeBusy" @click="handleWorkspaceMerge">{{ t('workPage.actions.mergeWorkspaces') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog class="workspace-access-dialog" :model-value="workspaceAccessDialog !== null" @update:model-value="(visible) => { if (!visible) workspaceAccessDialog = null; }" width="min(760px, 94vw)" :close-on-click-modal="!workspaceAccessBusy">
      <template #header>
        <span>{{ workspaceAccessDialog === 'upload' ? t('workPage.dialog.workspacePublishTitle') : t('workPage.dialog.workspaceLibraryTitle') }}</span>
      </template>

      <el-form v-if="workspaceAccessDialog === 'upload'" label-position="top">
        <el-form-item :label="t('workPage.dialog.workspacePublishNameMessage')">
          <el-input v-model="workspaceAccessPublishName" :maxlength="128" :placeholder="t('workPage.placeholders.workspacePublishName')" />
        </el-form-item>
        <el-form-item :label="t('workPage.dialog.workspaceDescriptionMessage')">
          <el-input v-model="workspaceAccessDescription" type="textarea" :rows="3" :placeholder="t('workPage.placeholders.workspaceDescription')" />
        </el-form-item>
        <el-form-item :label="t('workPage.dialog.workspaceAttributionMessage')">
          <el-input v-model="workspaceAccessAttribution" :placeholder="t('workPage.placeholders.workspaceAttribution')" />
        </el-form-item>
        <el-form-item :label="t('workPage.dialog.workspaceAliasesMessage')">
          <el-input v-model="workspaceAccessAliases" type="textarea" :rows="2" :placeholder="t('workPage.placeholders.workspaceAliases')" />
        </el-form-item>
        <el-form-item>
          <el-checkbox v-model="workspaceAccessIncludeFullPackage">{{ t('workPage.dialog.workspaceIncludeFullPackage') }}</el-checkbox>
        </el-form-item>
        <p class="workspace-access-hint">{{ t('workPage.dialog.workspaceTemporaryArchiveHint') }}</p>
      </el-form>

      <template v-else>
        <div class="workspace-library-toolbar">
          <el-input v-model="workspaceLibraryQuery" clearable :placeholder="t('workPage.placeholders.workspaceLibrarySearch')" />
          <div class="workspace-library-sort" role="group" :aria-label="t('workPage.dialog.workspaceLibrarySortMessage')">
            <el-button size="small" :type="workspaceLibrarySort === 'time' ? 'primary' : 'default'" @click="workspaceLibrarySort = 'time'">{{ t('workPage.actions.sortWorkspaceLibraryByTime') }}</el-button>
            <el-button size="small" :type="workspaceLibrarySort === 'downloads' ? 'primary' : 'default'" @click="workspaceLibrarySort = 'downloads'">{{ t('workPage.actions.sortWorkspaceLibraryByDownloads') }}</el-button>
          </div>
          <el-button text :loading="workspaceLibraryLoading" @click="loadWorkspaceLibraryForDialog({ forceRefresh: true })">{{ t('workPage.actions.refresh') }}</el-button>
        </div>
        <div v-loading="workspaceLibraryLoading" class="workspace-library-results glass-scrollbar">
          <article v-for="entry in filteredWorkspaceLibraryEntries()" :key="entry.entryId" class="workspace-library-entry">
            <div class="workspace-library-entry-main">
              <div class="workspace-library-entry-heading">
                <strong>{{ entry.workspaceName }}</strong>
                <el-tag size="small" effect="plain" :type="workspaceEntryHasFullPackage(entry) ? 'success' : 'info'">
                  {{ workspaceEntryHasFullPackage(entry) ? t('workPage.dialog.workspaceLibraryFullPackage') : t('workPage.dialog.workspaceLibraryMetadataOnly') }}
                </el-tag>
              </div>
              <p class="workspace-library-entry-attribution">{{ entry.attribution || 'anonymous' }}</p>
              <p v-if="entry.description" class="workspace-library-entry-description">{{ entry.description }}</p>
              <p class="workspace-library-entry-stats">
                {{ t('workPage.ui.workspaceLibraryUploadedAt', { time: new Date(entry.uploadedAt).toLocaleString() }) }} ·
                {{ t('workPage.ui.workspaceLibraryMetadataDownloads', { count: entry.metadataDownloadCount ?? 0 }) }} ·
                {{ t('workPage.ui.workspaceLibraryFullPackageDownloads', { count: entry.fullPackageDownloadCount ?? 0 }) }}
              </p>
              <p v-if="entry.drawIB.length" class="workspace-library-entry-drawib">{{ entry.drawIB.join(' · ') }}</p>
              <p v-if="entry.aliases.length" class="workspace-library-entry-aliases">{{ entry.aliases.join(' · ') }}</p>
            </div>
            <div class="workspace-library-entry-actions">
              <el-button
                size="small"
                plain
                :loading="workspaceDownloadingEntry?.entryId === entry.entryId && workspaceDownloadingEntry.mode === 'metadata'"
                :disabled="workspaceAccessBusy"
                @click="handleLibraryDownload(entry, 'metadata')"
              >{{ t('workPage.actions.importWorkspaceMetadata') }}</el-button>
              <el-button
                v-if="workspaceEntryHasFullPackage(entry)"
                size="small"
                type="primary"
                :loading="workspaceDownloadingEntry?.entryId === entry.entryId && workspaceDownloadingEntry.mode === 'full'"
                :disabled="workspaceAccessBusy"
                @click="handleLibraryDownload(entry, 'full')"
              >{{ t('workPage.actions.downloadWorkspacePackage') }}</el-button>
            </div>
          </article>
          <el-empty v-if="!workspaceLibraryLoading && filteredWorkspaceLibraryEntries().length === 0" :description="t('workPage.messages.workspaceLibraryNoMatches')" :image-size="76" />
        </div>
      </template>

      <template #footer>
        <el-button @click="workspaceAccessDialog = null" :disabled="workspaceAccessBusy">{{ t('workPage.common.cancel') }}</el-button>
        <el-button v-if="workspaceAccessDialog === 'upload'" type="primary" :loading="workspacePublishing" @click="handlePublishWorkspaceFromDialog">{{ t('workPage.actions.openWorkspaceUpload') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.work-page-container {
  --work-crystal: var(--theme-accent);
  --work-crystal-rgb: var(--theme-accent-rgb);
  --work-crystal-soft: rgba(var(--theme-surface-tint-rgb), 0.08);
  --work-crystal-border: rgba(var(--theme-surface-tint-rgb), 0.18);
  --work-crystal-text: rgba(var(--theme-text-secondary-rgb), 0.78);
  --work-danger-bg: rgba(245, 108, 108, 0.18);
  --work-danger-border: rgba(245, 108, 108, 0.42);
  --work-danger-text: rgba(255, 178, 178, 1);
  --el-color-primary: var(--theme-accent);
  --el-color-primary-light-3: rgba(var(--theme-surface-tint-rgb), 0.72);
  --el-color-primary-light-5: rgba(var(--theme-surface-tint-rgb), 0.52);
  --el-color-primary-light-7: rgba(var(--theme-surface-tint-rgb), 0.30);
  --el-color-primary-light-8: rgba(var(--theme-surface-tint-rgb), 0.20);
  --el-color-primary-light-9: rgba(var(--theme-surface-tint-rgb), 0.10);
  --el-color-primary-dark-2: #42BFF2;
  padding: 28px 18px;
  /* padding-top: 60px; Safe Area Removed */
  height: 100%;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: 16px;
  color: #e8ecf5;
  overflow: hidden;
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
}



.work-layout {
  height: 100%;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 4fr) minmax(260px, 1.15fr);
  gap: 24px;
}

.main-column {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  height: 100%;
}

.panel-stack {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 18px;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-right: 4px;
}

.workspace-tabs-shell {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.workspace-tabs-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.workspace-tabs-list {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.workspace-tab-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 28px;
  padding: 2px 6px 2px 8px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.035);
  color: rgba(var(--theme-text-secondary-rgb), 0.78);
  cursor: pointer;
  transition: all 0.25s ease;
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
}

.workspace-tab-chip.is-active {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.42);
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  color: rgba(var(--theme-text-primary-rgb), 0.98);
  box-shadow: 0 6px 18px rgba(var(--theme-surface-tint-rgb), 0.08);
}

.workspace-tab-name {
  font-size: 0.82rem;
  line-height: 1;
}

.workspace-tab-delete {
  min-width: 18px;
  height: 18px;
  padding: 0;
  border: 0 !important;
  background: transparent !important;
  box-shadow: none !important;
  color: rgba(255, 178, 178, 0.86);
}

.workspace-tab-delete::before {
  display: none !important;
}

.workspace-tab-input {
  width: 140px;
}

.workspace-tabs-content {
  display: flex;
  flex-direction: column;
  gap: 12px;
}


.workspace-tabs-shell :deep(.controls-row .el-button),
.workspace-tabs-shell :deep(.inner-panel .el-button:not(.workspace-tab-delete)),
.workspace-tabs-head .el-button {
  --el-button-bg-color: rgba(var(--work-crystal-rgb), 0.045);
  --el-button-border-color: rgba(var(--work-crystal-rgb), 0.14);
  --el-button-text-color: var(--work-crystal-text);
  --el-button-hover-bg-color: rgba(var(--work-crystal-rgb), 0.10);
  --el-button-hover-border-color: rgba(var(--work-crystal-rgb), 0.30);
  --el-button-hover-text-color: var(--work-crystal);
  --el-button-active-bg-color: rgba(var(--work-crystal-rgb), 0.08);
  --el-button-active-border-color: rgba(var(--work-crystal-rgb), 0.34);
  --el-button-active-text-color: var(--work-crystal);
  height: 34px;
  border-radius: 8px;
  border: 1px solid var(--el-button-border-color) !important;
  background: var(--el-button-bg-color) !important;
  color: var(--el-button-text-color) !important;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.3px;
  transition: all 0.2s ease;
  position: relative;
  overflow: hidden;
}
.workspace-tabs-shell :deep(.controls-row .el-button)::before,
.workspace-tabs-shell :deep(.inner-panel .el-button:not(.workspace-tab-delete))::before,
.workspace-tabs-head .el-button::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(var(--work-crystal-rgb), 0.16), transparent);
  pointer-events: none;
}
.workspace-tabs-shell :deep(.controls-row .el-button):hover,
.workspace-tabs-shell :deep(.inner-panel .el-button:not(.workspace-tab-delete)):hover,
.workspace-tabs-head .el-button:hover {
  background: var(--el-button-hover-bg-color) !important;
  border-color: var(--el-button-hover-border-color) !important;
  color: var(--el-button-hover-text-color) !important;
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(0,0,0,0.08);
}
.workspace-tabs-shell :deep(.controls-row .el-button):active,
.workspace-tabs-shell :deep(.inner-panel .el-button:not(.workspace-tab-delete)):active,
.workspace-tabs-head .el-button:active {
  transform: translateY(0) scale(0.98);
}
.workspace-tabs-shell :deep(.controls-row .el-button).is-disabled,
.workspace-tabs-shell :deep(.controls-row .el-button).is-disabled:hover,
.workspace-tabs-shell :deep(.inner-panel .el-button.is-disabled:not(.workspace-tab-delete)),
.workspace-tabs-shell :deep(.inner-panel .el-button.is-disabled:not(.workspace-tab-delete)):hover,
.workspace-tabs-head .el-button.is-disabled,
.workspace-tabs-head .el-button.is-disabled:hover {
  opacity: 0.35;
  cursor: not-allowed;
  transform: none !important;
  background: var(--el-button-bg-color) !important;
  border-color: var(--el-button-border-color) !important;
}

.workspace-tabs-shell :deep(.controls-row .el-button.el-button--primary),
.workspace-tabs-shell :deep(.inner-panel .el-button.el-button--primary:not(.workspace-tab-delete)) {
  --el-button-bg-color: rgba(var(--work-crystal-rgb), 0.07);
  --el-button-border-color: rgba(var(--work-crystal-rgb), 0.20);
  --el-button-text-color: rgba(var(--work-crystal-rgb), 0.88);
  --el-button-hover-bg-color: rgba(var(--work-crystal-rgb), 0.13);
  --el-button-hover-border-color: rgba(var(--work-crystal-rgb), 0.34);
  --el-button-hover-text-color: var(--work-crystal);
}


.workspace-tabs-shell :deep(.el-button .circular path) {
  stroke: var(--theme-accent) !important;
}

.workspace-tabs-shell :deep(.el-button.is-loading::before) {
  background-color: rgba(8, 22, 32, 0.24) !important;
}

.workspace-tabs-shell :deep(.el-loading-spinner .path),
.workspace-tabs-shell :deep(.el-loading-spinner .circular path) {
  stroke: var(--theme-accent) !important;
}

.workspace-tabs-shell :deep(.el-button--primary) {
  --el-button-bg-color: rgba(var(--theme-surface-tint-rgb), 0.07) !important;
  --el-button-border-color: rgba(var(--theme-surface-tint-rgb), 0.20) !important;
  --el-button-text-color: rgba(var(--theme-surface-tint-rgb), 0.88) !important;
  --el-button-hover-bg-color: rgba(var(--theme-surface-tint-rgb), 0.13) !important;
  --el-button-hover-border-color: rgba(var(--theme-surface-tint-rgb), 0.34) !important;
  --el-button-hover-text-color: var(--theme-accent) !important;
  --el-button-active-bg-color: rgba(var(--theme-surface-tint-rgb), 0.10) !important;
  --el-button-active-border-color: rgba(var(--theme-surface-tint-rgb), 0.40) !important;
  --el-button-active-text-color: var(--theme-accent) !important;
}

.workspace-tabs-shell :deep(.el-input__wrapper),
.workspace-tabs-shell :deep(.el-select__wrapper) {
  background: rgba(255, 255, 255, 0.045) !important;
  box-shadow: 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.10) inset !important;
}

.workspace-tabs-shell :deep(.el-input__wrapper:hover),
.workspace-tabs-shell :deep(.el-input__wrapper.is-focus),
.workspace-tabs-shell :deep(.el-select__wrapper:hover),
.workspace-tabs-shell :deep(.el-select__wrapper.is-focused) {
  box-shadow: 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.28) inset !important;
}

.workspace-tabs-shell :deep(.glass-table.el-table) {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.10) !important;
  border-radius: 8px !important;
  overflow: hidden !important;
  background: rgba(var(--theme-surface-tint-rgb), 0.022) !important;
  box-shadow: none !important;
}

.workspace-tabs-shell :deep(.glass-table .el-table__inner-wrapper),
.workspace-tabs-shell :deep(.glass-table .el-table__header-wrapper),
.workspace-tabs-shell :deep(.glass-table .el-table__body-wrapper),
.workspace-tabs-shell :deep(.glass-table .el-scrollbar),
.workspace-tabs-shell :deep(.glass-table .el-scrollbar__wrap),
.workspace-tabs-shell :deep(.glass-table .el-scrollbar__view) {
  border-radius: 8px !important;
  overflow: hidden !important;
}

.workspace-tabs-shell :deep(.glass-table.el-table th.el-table__cell) {
  background: rgba(var(--theme-surface-tint-rgb), 0.055) !important;
  color: rgba(var(--theme-text-secondary-rgb), 0.80) !important;
}

.workspace-tabs-shell :deep(.glass-table.el-table::before),
.workspace-tabs-shell :deep(.glass-table.el-table::after),
.workspace-tabs-shell :deep(.glass-table .el-table__inner-wrapper::before),
.workspace-tabs-shell :deep(.glass-table .el-table__inner-wrapper::after) {
  background-color: rgba(var(--theme-surface-tint-rgb), 0.10) !important;
}

.workspace-tabs-shell :deep(.el-checkbox__input.is-checked .el-checkbox__inner) {
  background-color: #2563eb !important;
  border-color: #60a5fa !important;
  box-shadow: 0 0 0 2px rgba(37, 99, 235, 0.18), 0 0 10px rgba(37, 99, 235, 0.28) !important;
}

.workspace-tabs-shell :deep(.el-checkbox__input.is-checked + .el-checkbox__label) {
  color: rgba(var(--theme-text-secondary-rgb), 0.90) !important;
}

:deep(.workspace-merge-dialog) {
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.22);
  border-radius: 8px;
  background: rgba(18, 24, 34, 0.98);
}

:deep(.workspace-merge-dialog .el-dialog__body) {
  padding-top: 12px;
}

.workspace-merge-sources {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.workspace-merge-sources :deep(.el-select) {
  width: 100%;
}

.workspace-merge-hint {
  width: 100%;
  margin: 8px 0 0;
  color: rgba(var(--theme-text-secondary-rgb), 0.72);
  font-size: 12px;
  line-height: 1.45;
}

.workspace-merge-conflicts {
  min-height: 80px;
  max-height: 280px;
  overflow: auto;
  padding: 12px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.035);
}

.workspace-merge-conflicts-title {
  margin: 0 0 8px;
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  font-size: 13px;
  font-weight: 650;
}

.workspace-merge-conflict {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 38px;
  padding: 6px 0;
  border-top: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
}

.workspace-merge-conflict code {
  color: var(--theme-accent);
  font-size: 13px;
}

@media (max-width: 560px) {
  .workspace-merge-sources {
    grid-template-columns: 1fr;
    gap: 0;
  }

  .workspace-merge-conflict {
    align-items: flex-start;
    flex-direction: column;
  }
}

.panel {
  /* Subtle dark backing keeps the panel readable over bright game
     backgrounds even when the global dim mask is turned down */
  background:
    linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.045), rgba(var(--theme-surface-tint-rgb), 0.018)),
    rgba(8, 12, 19, 0.38);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.09);
  border-radius: 8px;
  padding: 18px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.06);
  transition: all 0.25s ease;
  position: relative;
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
}

.panel::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(var(--theme-surface-tint-rgb), 0.14), transparent);
  pointer-events: none;
  border-radius: 8px 8px 0 0;
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.panel-head h2 {
  margin: 6px 0 4px;
  font-size: 1.35rem;
  font-weight: 600;
}

.hint {
  margin: 0;
  color: rgba(255, 255, 255, 0.58);
  font-size: 0.92rem;
}

.muted {
  border-style: dashed;
  border-color: rgba(255, 255, 255, 0.10);
  /* 更透明的 muted 背景以减少灰蓝感 */
  background: rgba(15, 18, 28, 0.45);
}

:deep(.workspace-access-dialog) {
  overflow: hidden;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.22);
  border-radius: 20px;
  background:
    radial-gradient(circle at top right, rgba(var(--theme-surface-tint-rgb), 0.16), transparent 42%),
    linear-gradient(145deg, rgba(27, 32, 47, 0.98), rgba(15, 18, 29, 0.98));
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
}

:deep(.workspace-access-dialog .el-dialog__header) {
  margin-right: 0;
  padding: 22px 24px 16px;
  border-bottom: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  color: rgba(255, 255, 255, 0.95);
  font-size: 1.08rem;
  font-weight: 650;
}

:deep(.workspace-access-dialog .el-dialog__body) {
  padding: 20px 24px;
}

:deep(.workspace-access-dialog .el-dialog__footer) {
  padding: 14px 24px 20px;
  border-top: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
}

.workspace-library-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 14px;
}

.workspace-library-toolbar :deep(.el-input) {
  flex: 1;
}

.workspace-library-sort {
  display: flex;
  flex: 0 0 auto;
  gap: 4px;
}

.workspace-library-results {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 150px;
  max-height: min(56vh, 500px);
  overflow: auto;
  padding: 2px 3px 6px;
}

.workspace-library-entry {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 15px 16px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.17);
  border-radius: 15px;
  background: linear-gradient(120deg, rgba(var(--theme-surface-tint-rgb), 0.10), rgba(255, 255, 255, 0.025));
  transition: border-color 0.2s ease, transform 0.2s ease, background 0.2s ease;
}

.workspace-library-entry:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.32);
  background: linear-gradient(120deg, rgba(var(--theme-surface-tint-rgb), 0.15), rgba(255, 255, 255, 0.045));
  transform: translateY(-1px);
}

.workspace-library-entry-main {
  min-width: 0;
}

.workspace-library-entry-heading,
.workspace-library-entry-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.workspace-library-entry-heading strong {
  overflow: hidden;
  color: rgba(255, 255, 255, 0.94);
  font-size: 1rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-library-entry-attribution,
.workspace-library-entry-drawib,
.workspace-library-entry-aliases {
  margin: 5px 0 0;
  overflow: hidden;
  color: rgba(255, 255, 255, 0.64);
  font-size: 0.82rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-library-entry-description {
  display: -webkit-box;
  margin: 7px 0 0;
  overflow: hidden;
  color: rgba(255, 255, 255, 0.78);
  font-size: 0.88rem;
  line-height: 1.45;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.workspace-library-entry-stats {
  margin: 8px 0 0;
  color: rgba(var(--theme-surface-tint-rgb), 0.82);
  font-size: 0.78rem;
  line-height: 1.35;
}

.workspace-library-entry-drawib {
  color: rgba(var(--theme-surface-tint-rgb), 0.88);
}

.workspace-library-entry-actions {
  flex: 0 0 auto;
  flex-wrap: wrap;
  justify-content: flex-end;
}

@media (max-width: 640px) {
  .workspace-library-toolbar {
    align-items: stretch;
    flex-wrap: wrap;
  }

  .workspace-library-sort {
    width: 100%;
  }

  .workspace-library-entry {
    align-items: stretch;
    flex-direction: column;
    gap: 12px;
  }

  .workspace-library-entry-actions {
    justify-content: flex-start;
  }
}

@media (max-width: 960px) {
  .work-page-container {
    overflow: auto;
  }

  .work-layout {
    grid-template-columns: 1fr;
  }

  .main-column {
    min-height: auto;
  }

  .panel-stack {
    overflow: visible;
    padding-right: 0;
  }
}
</style>
