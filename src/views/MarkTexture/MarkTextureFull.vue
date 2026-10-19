<script setup lang="ts">
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { dirname, join } from '@tauri-apps/api/path';
import { exists, mkdir, readDir, readFile, readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';
import { openPath as openExternal, revealItemInDir } from '@tauri-apps/plugin-opener';
import { moveFileToRecycleBin } from '../../utils/RecycleBin';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ArrowRight, Close, Delete, Edit, Grid, View } from '@element-plus/icons-vue';
import { useI18n } from 'vue-i18n';
import { AppStateManager } from '../../store/AppStateManager';
import { GlobalConfig } from '../../store/GlobalConfig';
import { ResourceManager } from '../../store/ResourceManager';
import { PathHelper } from '../../helper/PathHelper';
import { SSMTStringUtils } from '../../utils/SSMTStringUtils';
import { calculateContextMenuPosition } from '../../utils/ContextMenuPosition';
import { debugError } from '../../utils/debugLog';
import {
	applyTextureMemoryToItems,
	deleteTextureMemoryByPSHash,
	loadTextureMemoryByPSHash,
	saveTextureMemoryByPSHash,
} from '../../common/TextureMarkMemory';
import {
	addCustomTextureMarkName,
	getCustomTextureMarkNames,
	removeCustomTextureMarkName,
	type TextureMarkNameMemoryItem,
} from '../../common/TextureMarkNameMemory';
import { readDrawIBConfigFromWorkspace } from '../../common/DrawIBConfig';
import { readComponentAliases, writeComponentAlias, isComponentAlias, type ComponentAliases } from '../../common/ComponentAliases';
import { buildComponentTree, componentIbKey, type ComponentTreeItem } from './ComponentTree';
import SubmeshPostProcessPreview from './SubmeshPostProcessPreview.vue';
import {
	applyTextureMarkForCurrentSubMesh,
	clearCurrentSubMeshTextureMarkup,
	exportHashStyleTextureModTemplateForCurrentSelection,
	exportSlotStyleTextureModTemplateForCurrentSelection,
	getMarkTextureComponentJsonPath,
	getMarkTextureDrawIBComponentJsonPath,
	getMarkTextureTrianglelistJsonPath,
	hasExistingSubMeshTextureMarks,
	readAppliedSubMeshTextureMarks,
	readDrawCallIndexesMatchedByPersistedSubMeshMarks,
	readPersistedSubMeshDrawCallIndexes,
	updateCurrentSubMeshTextureMarkupStyle,
	writeSubMeshRoleMetadata,
	type SubMeshRole,
} from './MarkTextureFull';

type MarkStyle = 'Hash' | 'Slot' | 'SharedSlot';

type TextureChannelKey = 'R' | 'G' | 'B' | 'A';

type TextureChannelPreview = {
	key: TextureChannelKey;
	label: TextureChannelKey;
};

type TextureItem = {
	id: string;
	name: string;
	dedupedFileName: string;
	slot: string;
	ps_hash: string;
	render: boolean;
	suffix: string;
	size: string;
	format?: string;
	preview: string;
	previewPath: string;
	ddsPath: string;
	channelPreviews: TextureChannelPreview[];
	markName: string;
	markStyle: MarkStyle;
	faceNormalChannel: TextureChannelKey;
};

type PreviewTextureOption = {
	id: string;
	label: string;
	url: string;
	markName: string;
	ddsPath: string;
	faceNormalChannel?: TextureChannelKey;
};

type PreviewSubMeshTarget = {
	id: string;
	workspacePath: string;
	subMeshName: string;
	diffuseUrl?: string;
	diffuseUrls?: string[];
	normalUrl?: string;
	faceNormalUrl?: string;
	lightMapUrl?: string;
	rampMapUrl?: string;
	metalMapUrl?: string;
	diffuseDdsPath?: string;
	diffuseDdsPaths?: string[];
	normalDdsPath?: string;
	faceNormalDdsPath?: string;
	faceNormalChannel?: TextureChannelKey;
	lightMapDdsPath?: string;
	rampMapDdsPath?: string;
	metalMapDdsPath?: string;
};

type SubMeshMarkedTextureSummary = {
	id: string;
	drawCallSelectionValue: string;
	preview: string;
	previewKey: string;
	textureName: string;
	dedupedFileName: string;
	slot: string;
	markName: string;
	markStyle: MarkStyle;
	faceNormalChannel: TextureChannelKey;
	status: 'applied' | 'pending';
	textureHash: string;
	ddsPath: string;
	previewFolderPath?: string;
};

type DecodedDdsPreview = { width: number; height: number; pixels: Uint8Array };

type SubMeshDrawerItem = ComponentTreeItem & {
	value: string;
	label: string;
	markCount: number;
	isSelected: boolean;
	markedTextures: SubMeshMarkedTextureSummary[];
	role?: 'face' | 'neck' | 'eye';
};

type TrianglelistDedupedTextureProperty = {
	FALogDedupedFileName?: string;
	FADataDedupedFileName?: string;
	Format?: string;
};

type TrianglelistDedupedFileNameJson = Record<string, TrianglelistDedupedTextureProperty>;
type SubMeshTrianglelistDedupedFileNameJson = Record<string, TrianglelistDedupedFileNameJson>;

type MarkTextureSelectionMemory = {
	subMesh?: string;
	drawCall?: string;
	drawCallBySubMesh?: Record<string, string>;
	faceSubMesh?: string;
	faceSubMeshes?: string[];
	neckSubMesh?: string;
	eyeSubMeshes?: string[];
	faceNeckAlignmentEnabled?: boolean;
};

type MarkTextureGlobalUiConfig = {
	showUnrenderedTextures?: boolean;
	rgbaPreviewCardSize?: number;
};

type WorkspaceMarkTextureSource = {
	tabId: string;
	tabName: string;
	workspacePath: string;
	subMeshDrawCallMap: Record<string, string[]>;
	trianglelistDedupedData: TrianglelistDedupedFileNameJson;
	subMeshTrianglelistDedupedData: SubMeshTrianglelistDedupedFileNameJson;
	componentAliases: ComponentAliases;
	drawIBAliasMap: Record<string, string>;
	drawIBOrderMap: Record<string, number>;
	drawIBKeysByLengthDesc: string[];
	drawIBComponentMap: Record<string, { drawIB: string; componentIndex: string }>;
};

type WorkPageTabMeta = {
	id: string;
	name: string;
};

type WorkPageTabsIndex = {
	activeTabId: string;
	tabs: WorkPageTabMeta[];
};

const appSettings = AppStateManager.appSettings;
const { t } = useI18n();

const logPrefix = '[MarkTextureFull]';
const SUBMESH_TASK_CONCURRENCY = 7;

const mapWithConcurrency = async <T, R>(
	items: readonly T[],
	limit: number,
	mapper: (item: T, index: number) => Promise<R>,
	shouldContinue: () => boolean = () => true
): Promise<R[]> => {
	const results = new Array<R>(items.length);
	let nextIndex = 0;
	const workers = Array.from({ length: Math.min(Math.max(1, limit), items.length) }, async () => {
		while (nextIndex < items.length && shouldContinue()) {
			const index = nextIndex++;
			results[index] = await mapper(items[index]!, index);
		}
	});
	await Promise.all(workers);
	return results;
};

const workspaceSources = ref<WorkspaceMarkTextureSource[]>([]);
const subMeshOptions = ref<string[]>([]);
const drawCallOptions = ref<string[]>([]);

const subMeshDrawerList = ref<HTMLElement>();
const collapsedLods = ref<string[]>([]);
const collapsedIBs = ref<string[]>([]);
const renamingComponent = ref(false);
const selectedSubMesh = ref('');
const selectedDrawCall = ref('');
const previewSyncSelectedSubMesh = ref(false);
const mutedPreviewSubMeshMap = ref<Record<string, boolean>>({});
const soloPreviewSubMeshMap = ref<Record<string, boolean>>({});
const previewReviewSubMeshMap = ref<Record<string, boolean>>({});
const faceSubMeshes = ref<string[]>([]);
const neckSubMesh = ref('');
const eyeSubMeshes = ref<string[]>([]);
const faceNeckAlignmentEnabled = ref(false);
const subMeshContextMenu = ref<{ subMesh: string; x: number; y: number } | undefined>();

const markStyleOptions: MarkStyle[] = ['Hash', 'Slot', 'SharedSlot'];
const textureChannelKeys: TextureChannelKey[] = ['R', 'G', 'B', 'A'];

const presetMarkNameOptions: Record<string, string[]> = {
	DEFAULT: ['DiffuseMap', 'NormalMap', 'LightMap', 'HighLightMap', 'RampMap', 'MaterialMap', 'StockingMap'],
	SRMI: ['DiffuseMap', 'NormalMap', 'MaterialMap', 'MaskMap', 'LightMap', 'EmissionMap', 'RampMap'],
	GIMI: ['DiffuseMap', 'NormalMap', 'FaceSDFMap', 'FaceShadow', 'LightMap', 'HighLightMap', 'MetalMap', 'MaterialMap', 'StockingMap'],
	HIMI: ['DiffuseMap', 'NormalMap', 'LightMap', 'FaceMap', 'HairMap', 'MaskMap', 'RampMap'],
	ZZMI: ['DiffuseMap', 'NormalMap', 'LightMap', 'BodyMaskMap', 'HairLightMap', 'MaterialMap', 'RampMap'],
	ZZMIDX12: ['DiffuseMap', 'NormalMap', 'LightMap', 'BodyMaskMap', 'HairLightMap', 'MaterialMap', 'RampMap'],
};

const gamePreset = ref('DEFAULT');
const customMarkNameOptions = ref<TextureMarkNameMemoryItem[]>([]);

const textureList = ref<TextureItem[]>([]);
const subMeshMarkedTextureMap = ref<Record<string, SubMeshMarkedTextureSummary[]>>({});
const drawCallSelectionBySubMesh = ref<Record<string, string>>({});
const activeChannelPreviewItem = ref<TextureItem | null>(null);
const showUnrenderedTextures = ref(true);
const rgbaPreviewCardSize = ref(0);
const isApplyingTextureMemory = ref(false);
const isApplyingAllDrawIBTextureMark = ref(false);
const isClearingCurrentDrawIBTextureMark = ref(false);
const isUpdatingCurrentDrawIBMarkStyle = ref(false);
const isDeletingCurrentTextureConfig = ref(false);
const isDeletingAllTextureConfigs = ref(false);
const isExportingHashTextureModTemplate = ref(false);
const isExportingSlotTextureModTemplate = ref(false);
const pendingRestoreSubMesh = ref('');
const pendingRestoreDrawCall = ref('');
const pendingDrawerDrawCallSelection = ref('');
let textureMemorySaveTimer: ReturnType<typeof setTimeout> | undefined;
let selectionMemorySaveTimer: ReturnType<typeof setTimeout> | undefined;
let workspaceUiConfigSaveTimer: ReturnType<typeof setTimeout> | undefined;
let workspaceReloadTimer: ReturnType<typeof setTimeout> | undefined;
let rgbaPreviewResizeCleanup: (() => void) | undefined;
let channelPreviewRenderToken = 0;
let textureListLoadToken = 0;
let subMeshOptionsLoadToken = 0;
let markedTextureSummaryLoadToken = 0;
let markedTexturePreviewCacheSeed = 0;
let lastValidSubMeshSelection = '';
let lastValidDrawCallSelection = '';

const RGBA_PREVIEW_CARD_MIN_SIZE = 220;
const RGBA_PREVIEW_CARD_INITIAL_RATIO = 0.6;
const RGBA_PREVIEW_CARD_VIEWPORT_MARGIN = 32;
const RGBA_PREVIEW_MODAL_HEADER_HEIGHT = 44;
const RGBA_PREVIEW_RENDER_MAX_DIMENSION = 1024;
const DDS_THUMBNAIL_MAX_DIMENSION = 256;
const DDS_THUMBNAIL_MEMORY_CACHE_LIMIT = 256;
const ddsThumbnailMemoryCache = new Map<string, Promise<string>>();
const channelPreviewCanvas = ref<HTMLCanvasElement | null>(null);
const enabledPreviewChannels = ref<Record<TextureChannelKey, boolean>>({ R: true, G: true, B: true, A: true });
const channelPreviewScale = ref(1);
const channelPreviewOffset = ref({ x: 0, y: 0 });
let channelPreviewDragCleanup: (() => void) | undefined;
const channelPreviewCombinationCache = new Map<number, HTMLCanvasElement>();

const currentGameName = computed(() => appSettings.CurrentGameName || 'Default');

const getPresetMarkNameList = (preset: string): string[] => {
	return [...(presetMarkNameOptions[preset] || presetMarkNameOptions.DEFAULT)];
};

const presetMarkNameList = computed(() => getPresetMarkNameList(gamePreset.value));

const normalizeMarkNameKey = (name: string): string => name.trim().toLocaleLowerCase();

const isPresetMarkName = (name: string): boolean => {
	const key = normalizeMarkNameKey(name);
	return presetMarkNameList.value.some(presetName => normalizeMarkNameKey(presetName) === key);
};

const refreshCustomMarkNameOptions = () => {
	customMarkNameOptions.value = getCustomTextureMarkNames(gamePreset.value)
		.filter(item => !isPresetMarkName(item.name));
};

const hasCustomMarkNameOptions = computed(() => customMarkNameOptions.value.length > 0);

const getRgbaPreviewCardMaxSize = (): number => {
	if (typeof window === 'undefined') {
		return 480;
	}

	return Math.max(
		RGBA_PREVIEW_CARD_MIN_SIZE,
		Math.min(
			window.innerWidth - RGBA_PREVIEW_CARD_VIEWPORT_MARGIN,
			window.innerHeight - RGBA_PREVIEW_CARD_VIEWPORT_MARGIN - RGBA_PREVIEW_MODAL_HEADER_HEIGHT
		)
	);
};

const clampRgbaPreviewCardSize = (size: number): number => {
	if (!Number.isFinite(size)) {
		return Math.round(getRgbaPreviewCardMaxSize() * RGBA_PREVIEW_CARD_INITIAL_RATIO);
	}

	return Math.round(
		Math.min(
			getRgbaPreviewCardMaxSize(),
			Math.max(RGBA_PREVIEW_CARD_MIN_SIZE, size)
		)
	);
};

const ensureRgbaPreviewCardSize = () => {
	if (rgbaPreviewCardSize.value <= 0) {
		rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(
			getRgbaPreviewCardMaxSize() * RGBA_PREVIEW_CARD_INITIAL_RATIO
		);
		return;
	}

	rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(rgbaPreviewCardSize.value);
};

const serializeSubMeshSelection = (tabId: string, subMeshName: string): string => {
	return JSON.stringify({ tabId, subMeshName });
};

const parseSubMeshSelection = (
	selectionValue: string
): { tabId: string; subMeshName: string } | undefined => {
	if (!selectionValue) {
		return undefined;
	}

	try {
		const parsed = JSON.parse(selectionValue) as {
			tabId?: string;
			subMeshName?: string;
		};
		if (!parsed.tabId || !parsed.subMeshName) {
			return undefined;
		}

		return {
			tabId: parsed.tabId,
			subMeshName: parsed.subMeshName,
		};
	} catch {
		return undefined;
	}
};

const serializeDrawCallSelection = (
	tabId: string,
	subMeshName: string,
	drawCall: string
): string => {
	return JSON.stringify({ tabId, subMeshName, drawCall });
};

const parseDrawCallSelection = (
	selectionValue: string
): { tabId: string; subMeshName: string; drawCall: string } | undefined => {
	if (!selectionValue) {
		return undefined;
	}

	try {
		const parsed = JSON.parse(selectionValue) as {
			tabId?: string;
			subMeshName?: string;
			drawCall?: string;
		};
		if (!parsed.tabId || !parsed.subMeshName || !parsed.drawCall) {
			return undefined;
		}

		return {
			tabId: parsed.tabId,
			subMeshName: parsed.subMeshName,
			drawCall: parsed.drawCall,
		};
	} catch {
		return undefined;
	}
};

const resolveDrawCallSelectionValue = (
	selectionValue: string,
	options: string[]
): string | undefined => {
	if (!selectionValue) {
		return undefined;
	}

	if (options.includes(selectionValue)) {
		return selectionValue;
	}

	return options.find(option => parseDrawCallSelection(option)?.drawCall === selectionValue);
};

const rememberDrawCallSelectionForSubMesh = (
	subMeshSelectionValue: string,
	drawCallSelectionValue: string
) => {
	if (!subMeshSelectionValue || !drawCallSelectionValue) {
		return;
	}

	drawCallSelectionBySubMesh.value = {
		...drawCallSelectionBySubMesh.value,
		[subMeshSelectionValue]: drawCallSelectionValue,
	};
};

const rememberCurrentDrawCallSelection = () => {
	if (!selectedSubMesh.value || !selectedDrawCall.value || !drawCallOptions.value.includes(selectedDrawCall.value)) {
		return;
	}

	rememberDrawCallSelectionForSubMesh(selectedSubMesh.value, selectedDrawCall.value);
};

const getRememberedDrawCallSelectionForSubMesh = (
	subMeshSelectionValue: string,
	options: string[]
): string | undefined => {
	const remembered = drawCallSelectionBySubMesh.value[subMeshSelectionValue] || '';
	return resolveDrawCallSelectionValue(remembered, options);
};

const getWorkspaceBaseDir = async (): Promise<string | undefined> => {
	console.log(`${logPrefix} getWorkspaceBaseDir input`, {
		currentGameName: appSettings.CurrentGameName,
		dbmtWorkFolder: appSettings.DBMTWorkFolder,
	});

	let gameName = appSettings.CurrentGameName;
	if (!gameName || gameName === 'Default') {
		gameName = 'DefaultGame';
		console.log(`${logPrefix} CurrentGameName invalid, fallback to DefaultGame`);
	}

	if (!appSettings.DBMTWorkFolder) {
		console.warn(`${logPrefix} DBMTWorkFolder is empty, skip DrawIB loading`);
		return undefined;
	}

	return join(appSettings.DBMTWorkFolder, 'WorkSpace', gameName);
};

const getWorkspacePath = async (workspaceName?: string): Promise<string | undefined> => {
	const workspaceBaseDir = await getWorkspaceBaseDir();
	if (!workspaceBaseDir) {
		return undefined;
	}

	const resolvedWorkspaceName = (workspaceName ?? appSettings.CurrentWorkSpace ?? '').trim();
	if (!resolvedWorkspaceName) {
		console.warn(`${logPrefix} workspaceName is empty, skip DrawIB loading`);
		return undefined;
	}

	const workspacePath = await join(workspaceBaseDir, resolvedWorkspaceName);
	console.log(`${logPrefix} resolved workspacePath`, workspacePath);
	return workspacePath;
};

const resolveWorkspaceLodPathForTab = async (
	workspacePath: string,
	tabName: string
): Promise<string> => {
	const trimmedTabName = tabName.trim();
	if (!trimmedTabName) {
		return workspacePath;
	}
	return join(workspacePath, trimmedTabName);
};

const getSelectedWorkspaceSource = (): WorkspaceMarkTextureSource | undefined => {
	const parsed = parseSubMeshSelection(selectedSubMesh.value);
	if (!parsed) {
		return undefined;
	}

	return workspaceSources.value.find(source => source.tabId === parsed.tabId);
};

const getSelectedSubMeshName = (): string => {
	return parseSubMeshSelection(selectedSubMesh.value)?.subMeshName || '';
};

const canBuildSlotTemplateFromSubMesh = (
	subMeshName: string
): { hash: string; matchIndexCount: string; matchFirstIndex: string } | undefined => {
	const parts = subMeshName
		.split('-')
		.map(part => part.trim())
		.filter(part => part.length > 0);

	if (parts.length < 3) {
		return undefined;
	}

	return {
		hash: parts[0],
		matchIndexCount: parts[1],
		matchFirstIndex: parts[2],
	};
};

const resolveDrawIBAlias = (source: WorkspaceMarkTextureSource, subMeshName: string): string => {
	const exactAlias = source.drawIBAliasMap[subMeshName];
	if (exactAlias) {
		return exactAlias;
	}

	const drawIBPrefix = subMeshName.split('-')[0]?.trim() || '';
	if (drawIBPrefix) {
		const prefixAlias = source.drawIBAliasMap[drawIBPrefix];
		if (prefixAlias) {
			return prefixAlias;
		}
	}

	for (const drawIB of source.drawIBKeysByLengthDesc) {
		if (subMeshName.startsWith(`${drawIB}-`)) {
			return source.drawIBAliasMap[drawIB] || '';
		}
	}

	return '';
};

const resolveSubMeshDrawIB = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string
): string => {
	if (source.drawIBOrderMap[subMeshName] !== undefined) {
		return subMeshName;
	}

	const drawIBPrefix = subMeshName.split('-')[0]?.trim() || '';
	if (drawIBPrefix && source.drawIBOrderMap[drawIBPrefix] !== undefined) {
		return drawIBPrefix;
	}

	for (const drawIB of source.drawIBKeysByLengthDesc) {
		if (subMeshName.startsWith(`${drawIB}-`)) {
			return drawIB;
		}
	}

	return '';
};

const getSubMeshSortOrder = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string
): number => {
	const drawIB = resolveSubMeshDrawIB(source, subMeshName);
	if (!drawIB) {
		return Number.MAX_SAFE_INTEGER;
	}

	return source.drawIBOrderMap[drawIB] ?? Number.MAX_SAFE_INTEGER;
};

const formatSubMeshLabel = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string
): string => {
	const componentInfo = source.drawIBComponentMap[subMeshName];
	if (componentInfo) {
		const alias = source.componentAliases[subMeshName] || source.drawIBAliasMap[componentInfo.drawIB] || '';
		const aliasSuffix = alias ? `.${alias}` : '';
		return `[${source.tabName}] ${componentInfo.drawIB}-${componentInfo.componentIndex}${aliasSuffix}`;
	}

	// 回退：DrawIB-Component.json 无此映射，使用旧格式
	const alias = source.componentAliases[subMeshName] || resolveDrawIBAlias(source, subMeshName);
	return alias
		? `[${source.tabName}] ${subMeshName} (${alias})`
		: `[${source.tabName}] ${subMeshName}`;
};

const subMeshOptionItems = computed(() => {
	return subMeshOptions.value.flatMap(selectionValue => {
		const parsed = parseSubMeshSelection(selectionValue);
		const source = parsed && workspaceSources.value.find(item => item.tabId === parsed.tabId);
		if (!parsed || !source) return [];
		const component = source.drawIBComponentMap[parsed.subMeshName];
		const drawIB = component?.drawIB || resolveSubMeshDrawIB(source, parsed.subMeshName) || parsed.subMeshName.split('-')[0] || parsed.subMeshName;
		const ibAlias = source.drawIBAliasMap[drawIB];
		const defaultLabel = component ? `Component ${component.componentIndex}` : parsed.subMeshName;
		return [{
			value: selectionValue,
			label: formatSubMeshLabel(source, parsed.subMeshName),
			tabId: source.tabId,
			tabName: source.tabName || t('markTexture.tree.legacyLod'),
			drawIB,
			ibLabel: ibAlias ? `${drawIB} · ${ibAlias}` : drawIB,
			componentLabel: source.componentAliases[parsed.subMeshName] || defaultLabel,
		}];
	});
});

const drawCallOptionItems = computed(() => {
	return drawCallOptions.value
		.map(selectionValue => {
			const parsed = parseDrawCallSelection(selectionValue);
			if (!parsed) {
				return undefined;
			}

			return {
				value: selectionValue,
				label: parsed.drawCall,
			};
		})
		.filter((item): item is { value: string; label: string } => !!item);
});

const subMeshDrawerItems = computed<SubMeshDrawerItem[]>(() => {
	return subMeshOptionItems.value.map(item => {
		const markedTextures = subMeshMarkedTextureMap.value[item.value] ?? [];
		return {
			...item,
			markCount: markedTextures.length,
			isSelected: selectedSubMesh.value === item.value,
			markedTextures,
			role: faceSubMeshes.value.includes(item.value) ? 'face' : neckSubMesh.value === item.value ? 'neck' : eyeSubMeshes.value.includes(item.value) ? 'eye' : undefined,
		};
	});
});

const subMeshTree = computed(() => buildComponentTree(subMeshDrawerItems.value));
const toggleCollapsed = (groups: string[], key: string): string[] =>
	groups.includes(key) ? groups.filter(value => value !== key) : [...groups, key];

const focusSubMeshInDrawer = async (selectionValue: string) => {
	const item = subMeshOptionItems.value.find(item => item.value === selectionValue);
	if (!item) return;
	collapsedLods.value = collapsedLods.value.filter(key => key !== item.tabId);
	collapsedIBs.value = collapsedIBs.value.filter(key => key !== componentIbKey(item.tabId, item.drawIB));
	await nextTick();
	// 只滚动左栏，不让 scrollIntoView 连带移动页面和 3D 预览。
	const container = subMeshDrawerList.value;
	const button = Array.from(container?.querySelectorAll<HTMLButtonElement>('[data-submesh]') || [])
		.find(button => button.dataset.submesh === selectionValue);
	if (!container || !button || selectedSubMesh.value !== selectionValue) return;
	button.focus({ preventScroll: true });
	const bounds = container.getBoundingClientRect();
	const target = button.getBoundingClientRect();
	if (target.top < bounds.top || target.bottom > bounds.bottom) {
		container.scrollTo({
			top: container.scrollTop + target.top - bounds.top - container.clientHeight / 2 + target.height / 2,
			behavior: 'instant',
		});
	}
};

const selectPreviewSubMesh = (selectionValue: string) => {
	selectSubMeshFromDrawer(selectionValue);
	void focusSubMeshInDrawer(selectionValue);
};

const renameComponent = async (selectionValue: string) => {
	if (renamingComponent.value) return;
	const parsed = parseSubMeshSelection(selectionValue);
	const source = parsed && workspaceSources.value.find(item => item.tabId === parsed.tabId);
	if (!parsed || !source) return;
	renamingComponent.value = true;
	try {
		const { value } = await ElMessageBox.prompt(t('markTexture.tree.aliasHint'), t('markTexture.tree.renameComponent'), {
			inputValue: source.componentAliases[parsed.subMeshName] || '',
			inputValidator: value => isComponentAlias(value) || t('markTexture.tree.invalidAlias'),
			confirmButtonText: t('markTexture.tree.saveAlias'), cancelButtonText: t('markTexture.tree.cancelAlias'),
		});
		try {
			source.componentAliases = await writeComponentAlias(source.workspacePath, parsed.subMeshName, value);
		} catch (error) {
			ElMessage.error(t('markTexture.tree.aliasSaveFailed', { reason: String(error) }));
		}
	} catch {
		// 取消编辑不改变已保存的别名。
	} finally {
		renamingComponent.value = false;
	}
};

const previewSubMeshTargets = computed<PreviewSubMeshTarget[]>(() => {
	const selectedOnly = previewSyncSelectedSubMesh.value
		? subMeshDrawerItems.value.filter(item => item.value === selectedSubMesh.value)
		: (() => {
			const soloItems = subMeshDrawerItems.value.filter(item => soloPreviewSubMeshMap.value[item.value]);
			return soloItems.length > 0
				? soloItems
				: subMeshDrawerItems.value.filter(item => !mutedPreviewSubMeshMap.value[item.value]);
		})();

	return selectedOnly.flatMap(item => {
		const parsed = parseSubMeshSelection(item.value);
		const source = parsed
			? workspaceSources.value.find(candidate => candidate.tabId === parsed.tabId)
			: undefined;
		if (!parsed || !source) {
			return [];
		}
		const markedTextures = subMeshMarkedTextureMap.value[item.value] ?? [];
		const isFaceSdfMark = (markName: string): boolean => {
			const normalized = markName.trim().toLowerCase();
			return normalized === 'facesdfmap';
		};
		const isFaceShadowMark = (markName: string): boolean => markName.trim().toLowerCase() === 'faceshadow';
		const isLightMapMark = (markName: string): boolean => {
			const normalized = markName.trim().toLowerCase();
			return normalized === 'lightmap' || isFaceShadowMark(normalized);
		};
		const findMarkedPreview = (markName: string): string | undefined => {
			return markedTextures.find(summary => (
				(isFaceSdfMark(markName)
					? isFaceSdfMark(summary.markName)
					: markName.trim().toLowerCase() === 'lightmap'
						? isLightMapMark(summary.markName)
						: summary.markName.trim().toLowerCase() === markName.toLowerCase()) && !!summary.preview
			))?.preview;
		};
		const findMarkedDdsPath = (markName: string): string | undefined => markedTextures.find(summary => (
			isFaceSdfMark(markName)
				? isFaceSdfMark(summary.markName)
				: markName.trim().toLowerCase() === 'lightmap'
					? isLightMapMark(summary.markName)
					: summary.markName.trim().toLowerCase() === markName.toLowerCase()
		))?.ddsPath;
		const findMarkedPreviews = (markName: string): string[] => markedTextures
		.filter(summary => (markName.trim().toLowerCase() === 'lightmap'
			? isLightMapMark(summary.markName)
			: summary.markName.trim().toLowerCase() === markName.toLowerCase()) && !!summary.preview)
			.map(summary => summary.preview);
		const findMarkedDdsPaths = (markName: string): string[] => markedTextures
		.filter(summary => (markName.trim().toLowerCase() === 'lightmap'
			? isLightMapMark(summary.markName)
			: summary.markName.trim().toLowerCase() === markName.toLowerCase()) && !!summary.ddsPath)
			.map(summary => summary.ddsPath);
		const diffuseUrls = findMarkedPreviews('DiffuseMap');
		const diffuseDdsPaths = findMarkedDdsPaths('DiffuseMap');
		return [{
			id: item.value,
			workspacePath: source.workspacePath,
			subMeshName: parsed.subMeshName,
			diffuseUrl: findMarkedPreview('DiffuseMap'),
			diffuseUrls,
			normalUrl: findMarkedPreview('NormalMap'),
			faceNormalUrl: findMarkedPreview('FaceSDFMap'),
			lightMapUrl: findMarkedPreview('LightMap'),
			rampMapUrl: findMarkedPreview('RampMap'),
			metalMapUrl: findMarkedPreview('MetalMap'),
			diffuseDdsPath: findMarkedDdsPath('DiffuseMap'),
			diffuseDdsPaths,
			normalDdsPath: findMarkedDdsPath('NormalMap'),
			faceNormalDdsPath: findMarkedDdsPath('FaceSDFMap'),
			faceNormalChannel: markedTextures.find(summary => isFaceSdfMark(summary.markName))?.faceNormalChannel || 'R',
			lightMapDdsPath: findMarkedDdsPath('LightMap'),
			rampMapDdsPath: findMarkedDdsPath('RampMap'),
			metalMapDdsPath: findMarkedDdsPath('MetalMap'),
		}];
	});
});

const previewTextureOptions = computed<PreviewTextureOption[]>(() => {
	const options = new Map<string, PreviewTextureOption>();
	const appliedOrPendingMarks = subMeshMarkedTextureMap.value[selectedSubMesh.value] ?? [];

	for (const mark of appliedOrPendingMarks) {
		if (!mark.ddsPath) {
			continue;
		}
		options.set(`mark:${mark.id}`, {
			id: `mark:${mark.id}`,
			label: `${mark.markName || t('markTexture.preview.unmarkedTexture')} · ${mark.textureName}`,
			url: mark.preview,
			markName: mark.markName,
			ddsPath: mark.ddsPath,
			faceNormalChannel: mark.faceNormalChannel,
		});
	}

	for (const item of textureList.value) {
		if (!item.ddsPath) {
			continue;
		}
		options.set(`current:${item.id}`, {
			id: `current:${item.id}`,
			label: `${item.markName || t('markTexture.preview.unmarkedTexture')} · ${item.name}`,
			url: item.preview,
			markName: item.markName,
			ddsPath: item.ddsPath,
			faceNormalChannel: item.faceNormalChannel,
		});
	}

	return Array.from(options.values());
});

const getCurrentGameFolderName = (): string => {
	const gameName = (appSettings.CurrentGameName || '').trim();
	if (!gameName || gameName === 'Default') {
		return 'DefaultGame';
	}
	return gameName;
};

const getSelectionMemoryConfigPath = async (): Promise<string | undefined> => {
	const workspaceBaseDir = await getWorkspaceBaseDir();
	if (!workspaceBaseDir) {
		return undefined;
	}

	return join(workspaceBaseDir, 'MarkTextureConfig.json');
};

const getGlobalUiConfigPath = async (): Promise<string> => {
	const globalFolder = await GlobalConfig.SSMT4GlobalConfigsFolder();
	return join(globalFolder, 'MarkTexturePageDisplayConfig.json');
};

const loadSelectionMemory = async () => {
	try {
		const configPath = await getSelectionMemoryConfigPath();
		if (!configPath) {
			return;
		}
		const content = await readTextFile(configPath);
		const parsed = JSON.parse(content) as MarkTextureSelectionMemory;

		if (typeof parsed.subMesh === 'string') {
			pendingRestoreSubMesh.value = parsed.subMesh;
		}
		if (typeof parsed.drawCall === 'string') {
			pendingRestoreDrawCall.value = parsed.drawCall;
		}
		if (typeof parsed.subMesh === 'string' && typeof parsed.drawCall === 'string') {
			rememberDrawCallSelectionForSubMesh(parsed.subMesh, parsed.drawCall);
		}
		if (parsed.drawCallBySubMesh && typeof parsed.drawCallBySubMesh === 'object') {
			const nextDrawCallSelectionBySubMesh: Record<string, string> = {};
			for (const [subMesh, drawCall] of Object.entries(parsed.drawCallBySubMesh)) {
				if (typeof subMesh === 'string' && typeof drawCall === 'string') {
					nextDrawCallSelectionBySubMesh[subMesh] = drawCall;
				}
			}
			drawCallSelectionBySubMesh.value = {
				...nextDrawCallSelectionBySubMesh,
				...drawCallSelectionBySubMesh.value,
			};
		}
		if (Array.isArray(parsed.faceSubMeshes)) {
			faceSubMeshes.value = parsed.faceSubMeshes.filter((value): value is string => typeof value === 'string');
		} else if (typeof parsed.faceSubMesh === 'string') {
			faceSubMeshes.value = [parsed.faceSubMesh];
		}
		if (typeof parsed.neckSubMesh === 'string') neckSubMesh.value = parsed.neckSubMesh;
		if (Array.isArray(parsed.eyeSubMeshes)) eyeSubMeshes.value = parsed.eyeSubMeshes.filter((value): value is string => typeof value === 'string');
		if (typeof parsed.faceNeckAlignmentEnabled === 'boolean') {
			faceNeckAlignmentEnabled.value = parsed.faceNeckAlignmentEnabled;
		}

	} catch {
		// No config yet or invalid JSON; keep defaults silently.
	}
};

const loadGlobalUiConfig = async () => {
	try {
		const configPath = await getGlobalUiConfigPath();
		const content = await readTextFile(configPath);
		const parsed = JSON.parse(content) as MarkTextureGlobalUiConfig;
		if (typeof parsed.showUnrenderedTextures === 'boolean') {
			showUnrenderedTextures.value = parsed.showUnrenderedTextures;
		}
		if (typeof parsed.rgbaPreviewCardSize === 'number') {
			rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(parsed.rgbaPreviewCardSize);
		}
	} catch {
		// No global config yet or invalid JSON; keep current value.
	}

	ensureRgbaPreviewCardSize();
};

const loadWorkspaceMarkTextureSource = async (
	workspacePath: string,
	tab: WorkPageTabMeta
): Promise<WorkspaceMarkTextureSource | undefined> => {
	const componentJsonPath = await getMarkTextureComponentJsonPath(workspacePath);
	const trianglelistJsonPath = await getMarkTextureTrianglelistJsonPath(workspacePath);
	const drawIBComponentJsonPath = await getMarkTextureDrawIBComponentJsonPath(workspacePath);
	const subMeshTrianglelistJsonPath = await join(workspacePath, 'SubMeshTrianglelistDedupedFileName.json');

	try {
		// Repair mappings produced by older builds from the extracted folders.
		// This is intentionally independent of Blender's Import.json selections.
		if (!(await exists(drawIBComponentJsonPath))) {
			await invoke('regenerate_draw_ib_component_json', { lodWorkspacePath: workspacePath })
				.catch(error => console.warn(`${logPrefix} failed to rebuild component map`, error));
		}

		const [componentContent, trianglelistContent, subMeshTrianglelistContent, drawIBConfigEntries, componentAliases] = await Promise.all([
			readTextFile(componentJsonPath),
			readTextFile(trianglelistJsonPath),
			await exists(subMeshTrianglelistJsonPath) ? readTextFile(subMeshTrianglelistJsonPath) : '{}',
			readDrawIBConfigFromWorkspace(workspacePath).catch(() => []),
			readComponentAliases(workspacePath).catch(error => {
				console.warn(`${logPrefix} Component aliases unreadable`, workspacePath, error);
				ElMessage.warning(t('markTexture.tree.aliasLoadFailed', { lod: tab.name || 'LOD0' }));
				return {};
			}),
		]);

		const parsedComponent = JSON.parse(componentContent) as Record<string, unknown>;
		const parsedTrianglelist = JSON.parse(trianglelistContent) as TrianglelistDedupedFileNameJson;
		const parsedSubMeshTrianglelist = JSON.parse(subMeshTrianglelistContent) as SubMeshTrianglelistDedupedFileNameJson;
		const rawSubMeshDrawCallMap: Record<string, string[]> = {};

		for (const [subMeshName, drawCalls] of Object.entries(parsedComponent)) {
			if (!subMeshName.trim() || !Array.isArray(drawCalls)) {
				continue;
			}

			rawSubMeshDrawCallMap[subMeshName] = drawCalls
				.filter((item): item is string => typeof item === 'string')
				.map(item => item.trim())
				.filter(item => item.length > 0);
		}

		const drawIBAliasMap: Record<string, string> = {};
		const drawIBOrderMap: Record<string, number> = {};
		for (const entry of drawIBConfigEntries) {
			const drawIB = (entry.DrawIB || '').trim();
			const alias = (entry.Alias || '').trim();
			if (!drawIB || !alias) {
				if (!drawIB) {
					continue;
				}
			}
			if (drawIBOrderMap[drawIB] === undefined) {
				drawIBOrderMap[drawIB] = Object.keys(drawIBOrderMap).length;
			}
			drawIBAliasMap[drawIB] = alias;
		}

		// DrawIB-Component.json is the complete extracted-component mapping.
		// Import.json belongs to Blender's selected data-type state and must not
		// control the post-process list or the importable component set.
		const drawIBComponentMap: Record<string, { drawIB: string; componentIndex: string }> = {};
		try {
			if (await exists(drawIBComponentJsonPath)) {
				const drawIBComponentContent = await readTextFile(drawIBComponentJsonPath);
				const parsedDrawIBComponent = JSON.parse(drawIBComponentContent) as Record<string, Record<string, string>>;
				for (const [drawIB, componentDict] of Object.entries(parsedDrawIBComponent)) {
					if (!drawIB || typeof componentDict !== 'object') {
						continue;
					}
					for (const [componentIndex, submeshFolderName] of Object.entries(componentDict)) {
						if (!submeshFolderName || typeof submeshFolderName !== 'string') {
							continue;
						}
						drawIBComponentMap[submeshFolderName] = { drawIB, componentIndex };
					}
				}
			}
		} catch {
			// DrawIB-Component.json missing or invalid — leave map empty, fall back to legacy labels
		}

		const subMeshDrawCallMap: Record<string, string[]> = {};
		if (Object.keys(drawIBComponentMap).length > 0) {
			for (const subMeshName of Object.keys(drawIBComponentMap)) {
				// Publish the complete Submesh structure immediately. Per-Submesh
				// persisted draw calls are hydrated through the bounded background queue.
				subMeshDrawCallMap[subMeshName] = rawSubMeshDrawCallMap[subMeshName] ?? [];
			}
		} else {
			Object.assign(subMeshDrawCallMap, rawSubMeshDrawCallMap);
		}

		if (Object.keys(subMeshDrawCallMap).length === 0) {
			return undefined;
		}

		return {
			tabId: tab.id,
			tabName: tab.name,
			workspacePath,
			subMeshDrawCallMap,
			trianglelistDedupedData: parsedTrianglelist,
			subMeshTrianglelistDedupedData: parsedSubMeshTrianglelist,
			componentAliases,
			drawIBAliasMap,
			drawIBOrderMap,
			drawIBKeysByLengthDesc: Object.keys(drawIBOrderMap).sort((a, b) => b.length - a.length),
			drawIBComponentMap,
		};
	} catch (error) {
		console.warn(`${logPrefix} skip workspace mark texture source`, {
			tabId: tab.id,
			tabName: tab.name,
			workspacePath,
			error,
		});
		return undefined;
	}
};

const saveSelectionMemory = async () => {
	try {
		const configPath = await getSelectionMemoryConfigPath();
		if (!configPath) {
			return;
		}
		const configDir = await dirname(configPath);
		await mkdir(configDir, { recursive: true });
		const payload: MarkTextureSelectionMemory = {
			subMesh: selectedSubMesh.value,
			drawCall: selectedDrawCall.value,
			drawCallBySubMesh: drawCallSelectionBySubMesh.value,
			faceSubMeshes: faceSubMeshes.value,
			neckSubMesh: neckSubMesh.value,
			eyeSubMeshes: eyeSubMeshes.value,
			faceNeckAlignmentEnabled: faceNeckAlignmentEnabled.value,
		};
		await writeTextFile(configPath, JSON.stringify(payload, null, 2));
	} catch {
		// Silent failure: do not affect interaction flow.
	}
};

const saveGlobalUiConfig = async () => {
	try {
		const configPath = await getGlobalUiConfigPath();
		const configDir = await dirname(configPath);
		await mkdir(configDir, { recursive: true });
		const payload: MarkTextureGlobalUiConfig = {
			showUnrenderedTextures: showUnrenderedTextures.value,
			rgbaPreviewCardSize: clampRgbaPreviewCardSize(rgbaPreviewCardSize.value),
		};
		await writeTextFile(configPath, JSON.stringify(payload, null, 2));
	} catch {
		// Silent failure: do not affect interaction flow.
	}
};

const scheduleSaveSelectionMemory = () => {
	if (selectionMemorySaveTimer) {
		clearTimeout(selectionMemorySaveTimer);
	}

	selectionMemorySaveTimer = setTimeout(() => {
		void saveSelectionMemory();
	}, 200);
};

const scheduleSaveGlobalUiConfig = () => {
	if (workspaceUiConfigSaveTimer) {
		clearTimeout(workspaceUiConfigSaveTimer);
	}

	workspaceUiConfigSaveTimer = setTimeout(() => {
		void saveGlobalUiConfig();
	}, 200);
};

const loadPresetMarkOptions = async () => {
	try {
		const config = await ResourceManager.loadGameConfig(currentGameName.value);
		const preset = (config?.gamePreset || 'DEFAULT').toUpperCase();
		gamePreset.value = preset;
	} catch {
		gamePreset.value = 'DEFAULT';
	}
	refreshCustomMarkNameOptions();
};

const loadSubMeshOptions = async () => {
	const loadToken = ++subMeshOptionsLoadToken;
	// Invalidate child work immediately. Serialized selection ids can be equal
	// across games, in which case Vue watchers would otherwise keep old data.
	markedTextureSummaryLoadToken += 1;
	textureListLoadToken += 1;
	console.log(`${logPrefix} loadSubMeshOptions start`);
	try {
		const workspacePath = await getWorkspacePath();
		if (loadToken !== subMeshOptionsLoadToken) return;
		if (!workspacePath) {
			console.warn(`${logPrefix} workspacePath unavailable, clear SubMesh options`);
			workspaceSources.value = [];
			subMeshOptions.value = [];
			drawCallOptions.value = [];
			selectedSubMesh.value = '';
			selectedDrawCall.value = '';
			textureList.value = [];
			ElMessage.info(t('markTexture.messages.noCurrentWorkspaceDetected'));
			return;
		}

		const tabsIndexPath = await join(workspacePath, 'Config', 'WorkPageTabs.json');
		let tabs: WorkPageTabMeta[];
		if (await exists(tabsIndexPath)) {
			const tabsIndex = JSON.parse(await readTextFile(tabsIndexPath)) as WorkPageTabsIndex;
			tabs = Array.isArray(tabsIndex.tabs) ? tabsIndex.tabs
				.filter(tab => !!tab && typeof tab.id === 'string' && typeof tab.name === 'string')
				.map(tab => ({ id: tab.id.trim(), name: tab.name.trim() || tab.id.trim() }))
				.filter(tab => tab.id.length > 0) : [];
		} else {
			// 旧工作空间可以只有 LOD 目录，或直接在根目录保存提取结果。
			const entries = await readDir(workspacePath);
			tabs = entries.filter(entry => entry.isDirectory && /^LOD\d+$/i.test(entry.name))
				.sort((a, b) => naturalCompare(a.name, b.name)).map(entry => ({ id: entry.name, name: entry.name }));
			if (tabs.length === 0) tabs = [{ id: 'legacy-root', name: '' }];
		}
		if (loadToken !== subMeshOptionsLoadToken) return;

		const sources = (
			await mapWithConcurrency(
				tabs, SUBMESH_TASK_CONCURRENCY, async tab => {
					const workspaceLodPath = await resolveWorkspaceLodPathForTab(workspacePath, tab.name);
					return loadWorkspaceMarkTextureSource(workspaceLodPath, tab);
				},
				() => loadToken === subMeshOptionsLoadToken
			)
		).filter((item): item is WorkspaceMarkTextureSource => !!item);
		if (loadToken !== subMeshOptionsLoadToken) return;

		workspaceSources.value = sources;
		void hydrateWorkspaceSourceDrawCalls(sources, loadToken);
		const nextSubMeshOptions = sources
			.flatMap(source => {
				return Object.keys(source.subMeshDrawCallMap)
					.sort((a, b) => {
						const orderCompare = getSubMeshSortOrder(source, a) - getSubMeshSortOrder(source, b);
						if (orderCompare !== 0) {
							return orderCompare;
						}

						// 使用 DrawIB-Component.json 的 ComponentIndex 数值排序
						const compA = source.drawIBComponentMap[a];
						const compB = source.drawIBComponentMap[b];
						if (compA && compB) {
							const drawIBCompare = naturalCompare(compA.drawIB, compB.drawIB);
							if (drawIBCompare !== 0) {
								return drawIBCompare;
							}
							return parseInt(compA.componentIndex, 10) - parseInt(compB.componentIndex, 10);
						}

						return naturalCompare(a, b);
					})
					.map(subMeshName => serializeSubMeshSelection(source.tabId, subMeshName));
			});

		subMeshOptions.value = nextSubMeshOptions;
		const validSubMeshSelections = new Set(nextSubMeshOptions);
		mutedPreviewSubMeshMap.value = Object.fromEntries(
			Object.entries(mutedPreviewSubMeshMap.value).filter(([selectionValue]) => validSubMeshSelections.has(selectionValue))
		);
		soloPreviewSubMeshMap.value = Object.fromEntries(
			Object.entries(soloPreviewSubMeshMap.value).filter(([selectionValue]) => validSubMeshSelections.has(selectionValue))
		);
		previewReviewSubMeshMap.value = Object.fromEntries(
			Object.entries(previewReviewSubMeshMap.value).filter(([selectionValue]) => validSubMeshSelections.has(selectionValue))
		);

		if (
			pendingRestoreSubMesh.value &&
			subMeshOptions.value.includes(pendingRestoreSubMesh.value)
		) {
			selectedSubMesh.value = pendingRestoreSubMesh.value;
			pendingRestoreSubMesh.value = '';
		} else if (!selectedSubMesh.value || !subMeshOptions.value.includes(selectedSubMesh.value)) {
			selectedSubMesh.value = subMeshOptions.value[0] ?? '';
		}

		syncDrawCallOptionsBySubMesh();
		const restoredDrawCall = resolveDrawCallSelectionValue(
			pendingRestoreDrawCall.value,
			drawCallOptions.value
		);
		if (restoredDrawCall) {
			selectedDrawCall.value = restoredDrawCall;
			rememberCurrentDrawCallSelection();
		}
		pendingRestoreDrawCall.value = '';

		console.log(`${logPrefix} submeshes loaded`, {
			count: subMeshOptions.value.length,
			selectedSubMesh: selectedSubMesh.value,
		});

		if (subMeshOptions.value.length === 0) {
			ElMessage.warning(t('markTexture.messages.noSubMeshConfigEntries'));
		}
		// Let Vue paint Tier/Submesh controls before texture discovery, mark
		// previews and 3D geometry begin consuming the background queues.
		await nextTick();
		if (loadToken !== subMeshOptionsLoadToken) return;

		// Reload explicitly even when the new game happens to use the same tab,
		// submesh and draw-call ids as the previous game.
		await loadTextureListByDrawCall(selectedDrawCall.value);
		if (loadToken !== subMeshOptionsLoadToken) return;
		await refreshSubMeshMarkedTextureSummary(undefined, loadToken);
	} catch (error) {
		if (loadToken !== subMeshOptionsLoadToken) return;
		workspaceSources.value = [];
		subMeshOptions.value = [];
		drawCallOptions.value = [];
		subMeshMarkedTextureMap.value = {};
		selectedSubMesh.value = '';
		selectedDrawCall.value = '';
		textureList.value = [];
		ElMessage.warning(t('markTexture.messages.loadSubMeshListFailed'));
		console.error(`${logPrefix} loadSubMeshOptions failed`, error);
	}
};

const getPixelSlotFromTextureFileName = (textureFileName: string): string => {
	const startPos = textureFileName.indexOf('-');
	const endPos = textureFileName.indexOf('=');
	if (startPos < 0 || endPos < 0 || endPos <= startPos + 1) {
		return '';
	}

	let pixelSlot = textureFileName.slice(startPos + 1, endPos);
	if (pixelSlot.includes('-vs')) {
		const lastDash = pixelSlot.lastIndexOf('-');
		if (lastDash > 0) {
			pixelSlot = pixelSlot.slice(0, lastDash);
		}
	}

	return pixelSlot;
};

const getImageSize = (previewUrl: string): Promise<string> => {
	if (!previewUrl) {
		return Promise.resolve('-');
	}

	return new Promise(resolve => {
		const img = new Image();
		img.onload = () => resolve(`${img.naturalWidth}x${img.naturalHeight}`);
		img.onerror = () => resolve('-');
		img.src = previewUrl;
	});
};

const buildTexturePreviewUrl = async (
	workspacePath: string,
	fileName: string,
	_cacheBustToken: number
): Promise<string> => {
	const filePath = await findTexturePreviewPath(workspacePath, fileName);
	return filePath ? renderDdsThumbnail(filePath) : '';
};

const hydrateWorkspaceSourceDrawCalls = async (
	sources: WorkspaceMarkTextureSource[],
	ownerLoadToken: number
) => {
	const tasks = sources.flatMap(source => Object.keys(source.subMeshDrawCallMap).map(subMeshName => ({ source, subMeshName })));
	await mapWithConcurrency(
		tasks,
		SUBMESH_TASK_CONCURRENCY,
		async ({ source, subMeshName }) => {
			const persisted = await readPersistedSubMeshDrawCallIndexes({
				workspacePath: source.workspacePath,
				subMesh: subMeshName,
			});
			if (ownerLoadToken !== subMeshOptionsLoadToken) return;
			const direct = source.subMeshDrawCallMap[subMeshName] ?? [];
			const matched = persisted.length === 0 && direct.length === 0
				? await readDrawCallIndexesMatchedByPersistedSubMeshMarks({
					workspacePath: source.workspacePath,
					subMesh: subMeshName,
					trianglelistDedupedDict: source.trianglelistDedupedData,
				})
				: [];
			if (ownerLoadToken !== subMeshOptionsLoadToken) return;
			source.subMeshDrawCallMap[subMeshName] = persisted.length > 0 ? persisted : direct.length > 0 ? direct : matched;
			workspaceSources.value = [...workspaceSources.value];
			if (parseSubMeshSelection(selectedSubMesh.value)?.subMeshName === subMeshName) {
				syncDrawCallOptionsBySubMesh();
			}
		},
		() => ownerLoadToken === subMeshOptionsLoadToken
	);
};

const renderDdsThumbnail = async (filePath: string): Promise<string> => {
	try {
		const preparedPath = await invoke<string>('prepare_dds_webgl_preview', {
			sourcePath: filePath,
			maxDimension: DDS_THUMBNAIL_MAX_DIMENSION,
		});
		const cached = ddsThumbnailMemoryCache.get(preparedPath);
		if (cached) return await cached;

		const rendering = (async () => {
			const decoded = decodeRgbaDdsPreview(await readFile(preparedPath));
			const canvas = document.createElement('canvas');
			renderChannelPreview(canvas, decoded, 15);
			return canvas.toDataURL('image/webp');
		})();
		ddsThumbnailMemoryCache.set(preparedPath, rendering);
		if (ddsThumbnailMemoryCache.size > DDS_THUMBNAIL_MEMORY_CACHE_LIMIT) {
			const oldestKey = ddsThumbnailMemoryCache.keys().next().value;
			if (oldestKey) ddsThumbnailMemoryCache.delete(oldestKey);
		}
		try {
			return await rendering;
		} catch (error) {
			// A failed or incomplete parse must never become reusable cache state.
			if (ddsThumbnailMemoryCache.get(preparedPath) === rendering) {
				ddsThumbnailMemoryCache.delete(preparedPath);
			}
			throw error;
		}
	} catch (error) {
		console.warn('Failed to render DDS thumbnail', filePath, error);
		return '';
	}
};

const findTexturePreviewPath = async (workspacePath: string, fileName: string): Promise<string> => {
	if (!fileName) {
		return '';
	}

	const ddsFileName = fileName.replace(/\.(?:jpe?g|png)$/i, '.dds');
	const filePath = await join(workspacePath, 'DedupedTextures', ddsFileName);
	return await exists(filePath) ? filePath : '';
};

const createEmptyChannelPreviews = (): TextureChannelPreview[] => {
	return textureChannelKeys.map(key => ({
		key,
		label: key,
	}));
};

const nextMarkedTexturePreviewCacheBustToken = (): number => {
	markedTexturePreviewCacheSeed += 1;
	return Date.now() + markedTexturePreviewCacheSeed;
};

const getTextureEntriesForDrawCall = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	drawCall: string
): Array<[string, TrianglelistDedupedTextureProperty]> => {
	const trianglelistData = source.subMeshTrianglelistDedupedData[subMeshName] ?? source.trianglelistDedupedData;
	return Object.entries(trianglelistData)
		.filter(([textureFileName]) => textureFileName.startsWith(drawCall))
		.sort((a, b) => naturalCompare(a[0], b[0]));
};

const getVisibleTextureEntriesForDrawCall = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	drawCall: string
): Array<[string, TrianglelistDedupedTextureProperty]> => {
	const entries = getTextureEntriesForDrawCall(source, subMeshName, drawCall);
	if (showUnrenderedTextures.value) {
		return entries;
	}

	return entries.filter(([, textureProperty]) => (textureProperty?.FADataDedupedFileName || '').trim().length > 0);
};

const getAppliedTexturePreviewUrl = async (
	folderPath: string,
	markFileName: string,
	_cacheBustToken: number
): Promise<string> => {
	if (!folderPath || !markFileName) {
		return '';
	}
	const filePath = await join(folderPath, markFileName);
	return await exists(filePath) ? renderDdsThumbnail(filePath) : '';
};

const findAppliedTextureSourceEntry = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	markDedupedFileName: string,
	markHash: string,
	markSlot: string
): [string, TrianglelistDedupedTextureProperty] | undefined => {
	const drawCalls = source.subMeshDrawCallMap[subMeshName] ?? [];
	const normalizedMarkDedupedFileName = markDedupedFileName.trim().toLowerCase();
	const normalizedMarkHash = markHash.trim().toLowerCase();
	const normalizedMarkSlot = markSlot.trim().toLowerCase();

	for (const drawCall of drawCalls) {
		const entries = getTextureEntriesForDrawCall(source, subMeshName, drawCall);
		const matchedByDedupedFileName = entries.find(([, textureProperty]) => {
			const dedupedFileName = (textureProperty?.FALogDedupedFileName || '').trim().toLowerCase();
			return !!normalizedMarkDedupedFileName && dedupedFileName === normalizedMarkDedupedFileName;
		});
		if (matchedByDedupedFileName) {
			return matchedByDedupedFileName;
		}

		const matchedByHash = entries.find(([textureFileName]) => {
			const fileHash = SSMTStringUtils.getFileHashFromFileName(textureFileName).trim().toLowerCase();
			return !!normalizedMarkHash && fileHash === normalizedMarkHash;
		});
		if (matchedByHash) {
			return matchedByHash;
		}

		const matchedBySlot = entries.find(([textureFileName]) => {
			const slot = getPixelSlotFromTextureFileName(textureFileName).trim().toLowerCase();
			return !!normalizedMarkSlot && slot === normalizedMarkSlot;
		});
		if (matchedBySlot) {
			return matchedBySlot;
		}
	}

	return undefined;
};

const getAppliedTextureDedupedPreviewUrl = async (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	markDedupedFileName: string,
	markHash: string,
	markSlot: string,
	cacheBustToken: number
): Promise<string> => {
	const directDedupedFileName = markDedupedFileName.trim();
	if (directDedupedFileName) {
		return buildTexturePreviewUrl(
			source.workspacePath,
			directDedupedFileName,
			cacheBustToken
		);
	}

	const sourceEntry = findAppliedTextureSourceEntry(
		source,
		subMeshName,
		markDedupedFileName,
		markHash,
		markSlot
	);
	const dedupedFileName = (sourceEntry?.[1]?.FALogDedupedFileName || '').trim();
	if (!dedupedFileName) {
		return '';
	}

	return buildTexturePreviewUrl(
		source.workspacePath,
		dedupedFileName,
		cacheBustToken
	);
};

const resolveDrawCallSelectionForAppliedMark = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	markDedupedFileName: string,
	markHash: string,
	markSlot: string
): string => {
	const drawCalls = source.subMeshDrawCallMap[subMeshName] ?? [];
	const normalizedMarkDedupedFileName = markDedupedFileName.trim().toLowerCase();
	const normalizedMarkHash = markHash.trim().toLowerCase();
	const normalizedMarkSlot = markSlot.trim().toLowerCase();

	for (const drawCall of drawCalls) {
		const entries = getTextureEntriesForDrawCall(source, subMeshName, drawCall);
		const matched = entries.some(([textureFileName, textureProperty]) => {
			const dedupedFileName = (textureProperty?.FALogDedupedFileName || '').trim().toLowerCase();
			const fileHash = SSMTStringUtils.getFileHashFromFileName(textureFileName).trim().toLowerCase();
			const slot = getPixelSlotFromTextureFileName(textureFileName).trim().toLowerCase();
			return (
				(!!normalizedMarkDedupedFileName && dedupedFileName === normalizedMarkDedupedFileName) ||
				(!!normalizedMarkHash && fileHash === normalizedMarkHash) ||
				(!!normalizedMarkSlot && slot === normalizedMarkSlot)
			);
		});

		if (matched) {
			return serializeDrawCallSelection(source.tabId, subMeshName, drawCall);
		}
	}

	return drawCalls[0]
		? serializeDrawCallSelection(source.tabId, subMeshName, drawCalls[0])
		: '';
};

const buildMarkedTextureSummaryForSubMesh = async (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	cacheBustToken: number
): Promise<SubMeshMarkedTextureSummary[]> => {
	const appliedMarks = await readAppliedSubMeshTextureMarks({
		workspacePath: source.workspacePath,
		subMesh: subMeshName,
	});

	return mapWithConcurrency(
		appliedMarks, SUBMESH_TASK_CONCURRENCY, async mark => {
			const ddsPath = mark.textureFilePath;
			const id = [
				source.tabId,
				subMeshName,
				mark.markFileName,
				mark.markName,
				mark.markStyle,
				mark.markHash,
				mark.markSlot,
			].join('-');
			return {
				id,
				drawCallSelectionValue: resolveDrawCallSelectionForAppliedMark(
					source,
					subMeshName,
					mark.markDedupedFileName,
					mark.markHash,
					mark.markSlot
				),
				preview: '',
				previewKey: `${id}-${cacheBustToken}`,
				textureName: mark.markFileName,
				dedupedFileName: mark.markDedupedFileName,
				slot: mark.markSlot,
				markName: mark.markName,
				markStyle: mark.markStyle,
				faceNormalChannel: mark.faceNormalChannel,
				status: 'applied',
				textureHash: mark.markHash,
				ddsPath,
				previewFolderPath: mark.folderPath,
			};
		}
	);
};

const populateAppliedMarkedTexturePreviews = async (
	targetValues: string[],
	summaryLoadToken: number,
	ownerLoadToken: number,
	cacheBustToken: number,
	sourceSnapshot: WorkspaceMarkTextureSource[]
) => {
	const tasks = targetValues.flatMap(value => {
		const parsed = parseSubMeshSelection(value);
		const source = parsed ? sourceSnapshot.find(item => item.tabId === parsed.tabId) : undefined;
		return !parsed || !source ? [] : (subMeshMarkedTextureMap.value[value] ?? [])
			.filter(summary => summary.status === 'applied')
			.map(summary => ({ value, parsed, source, summary }));
	});
	await mapWithConcurrency(tasks, SUBMESH_TASK_CONCURRENCY, async ({ value, parsed, source, summary }) => {
		const sourcePreview = await getAppliedTextureDedupedPreviewUrl(
			source,
			parsed.subMeshName,
			summary.dedupedFileName,
			summary.textureHash,
			summary.slot,
			cacheBustToken
		);
		const copiedPreview = sourcePreview ? '' : await getAppliedTexturePreviewUrl(
			summary.previewFolderPath || '',
			summary.textureName,
			cacheBustToken
		);
		if (summaryLoadToken !== markedTextureSummaryLoadToken || ownerLoadToken !== subMeshOptionsLoadToken) return;
		const liveSummary = (subMeshMarkedTextureMap.value[value] ?? []).find(item => item.id === summary.id);
		if (!liveSummary) return;
		liveSummary.preview = sourcePreview || copiedPreview;
		liveSummary.previewKey = `${summary.id}-${cacheBustToken}`;
	}, () => summaryLoadToken === markedTextureSummaryLoadToken && ownerLoadToken === subMeshOptionsLoadToken);
};

const buildPendingTextureSummaryForCurrentSubMesh = (): SubMeshMarkedTextureSummary[] => {
	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	const drawCallSelectionValue = selectedDrawCall.value;
	if (!source || !subMeshName || !selectedSubMesh.value || !drawCallSelectionValue) {
		return [];
	}

	return textureList.value
		.filter(item => item.markName.trim())
		.map(item => ({
			id: [
				source.tabId,
				subMeshName,
				selectedDrawCall.value,
				item.name,
				item.markName,
				item.markStyle,
				'pending',
			].join('-'),
			drawCallSelectionValue,
			preview: item.preview,
			previewKey: `${item.id}-${item.preview}`,
			textureName: item.name,
			dedupedFileName: item.dedupedFileName,
			slot: item.slot,
			markName: item.markName,
			markStyle: item.markStyle,
			faceNormalChannel: item.faceNormalChannel,
			status: 'pending' as const,
			textureHash: SSMTStringUtils.getFileHashFromFileName(item.name),
			ddsPath: item.ddsPath,
		}));
};

const dedupeTextureSummaryByIdentity = (items: SubMeshMarkedTextureSummary[]): SubMeshMarkedTextureSummary[] => {
	const summaryMap = new Map<string, SubMeshMarkedTextureSummary>()

	for (const item of items) {
		const dedupedKey = item.dedupedFileName.trim().toLowerCase()
		const hashKey = item.textureHash.trim().toLowerCase()
		const slotKey = item.slot.trim().toLowerCase()
		const identityKey = dedupedKey
			? `deduped::${dedupedKey}`
			: hashKey
				? `hash::${hashKey}`
				: slotKey
					? `slot::${slotKey}`
					: ''
		if (!identityKey) {
			continue
		}

		summaryMap.set(identityKey, item)
	}

	return Array.from(summaryMap.values())
};

const syncPendingTextureSummaryForCurrentSubMesh = () => {
	if (!selectedSubMesh.value) {
		return;
	}

	const existing = subMeshMarkedTextureMap.value[selectedSubMesh.value] ?? [];
	const applied = existing.filter(summary => summary.status === 'applied');
	const normalizedMarkName = (markName: string): string => {
		const normalized = markName.trim().toLowerCase();
		return normalized;
	};
	const summaryIdentity = (summary: Pick<SubMeshMarkedTextureSummary, 'dedupedFileName' | 'textureHash' | 'slot'>): string => {
		const dedupedKey = summary.dedupedFileName.trim().toLowerCase();
		const hashKey = summary.textureHash.trim().toLowerCase();
		const slotKey = summary.slot.trim().toLowerCase();
		return dedupedKey
			? `deduped::${dedupedKey}`
			: hashKey
				? `hash::${hashKey}`
				: slotKey
					? `slot::${slotKey}`
					: '';
	};
	const pending = buildPendingTextureSummaryForCurrentSubMesh();
	const pendingByMarkIdentity = new Map(
		pending.map(summary => [
			`${normalizedMarkName(summary.markName)}::${summaryIdentity(summary)}`,
			summary,
		])
	);
	// A channel edit changes no texture identity, so it used to be discarded
	// when an already-applied summary won the dedupe. Keep the persisted summary
	// as the source of truth for the file, but mirror the live edit for preview.
	const syncedApplied = applied.map(summary => {
		const key = `${normalizedMarkName(summary.markName)}::${summaryIdentity(summary)}`;
		const live = pendingByMarkIdentity.get(key);
		const isFaceShadow = normalizedMarkName(summary.markName) === 'facesdfmap';
		return live && isFaceShadow
			? { ...summary, faceNormalChannel: live.faceNormalChannel }
			: summary;
	});
	const appliedKeys = new Set(
		syncedApplied.flatMap(summary => [
			`${normalizedMarkName(summary.markName)}::${summaryIdentity(summary)}`,
		])
	);
	const pendingOnly = pending.filter(summary => {
		const key = `${normalizedMarkName(summary.markName)}::${summaryIdentity(summary)}`;
		return !appliedKeys.has(key);
	});

	subMeshMarkedTextureMap.value = {
		...subMeshMarkedTextureMap.value,
		[selectedSubMesh.value]: dedupeTextureSummaryByIdentity([...syncedApplied, ...pendingOnly]),
	};
};

const refreshSubMeshMarkedTextureSummary = async (
	selectionValue?: string,
	ownerLoadToken = subMeshOptionsLoadToken
) => {
	const summaryLoadToken = ++markedTextureSummaryLoadToken;
	const sourceSnapshot = workspaceSources.value;
	const nextMap = selectionValue
		? { ...subMeshMarkedTextureMap.value }
		: {};
	const targetValues = selectionValue ? [selectionValue] : subMeshOptions.value;
	const cacheBustToken = nextMarkedTexturePreviewCacheBustToken();

	await mapWithConcurrency(
		targetValues, SUBMESH_TASK_CONCURRENCY, async value => {
			const parsed = parseSubMeshSelection(value);
			const source = parsed
				? sourceSnapshot.find(item => item.tabId === parsed.tabId)
				: undefined;

			if (!parsed || !source) {
				delete nextMap[value];
				return;
			}

			nextMap[value] = await buildMarkedTextureSummaryForSubMesh(
				source,
				parsed.subMeshName,
				cacheBustToken
			);
			if (summaryLoadToken === markedTextureSummaryLoadToken && ownerLoadToken === subMeshOptionsLoadToken) {
				subMeshMarkedTextureMap.value = { ...nextMap };
			}
		},
		() => summaryLoadToken === markedTextureSummaryLoadToken && ownerLoadToken === subMeshOptionsLoadToken
	);
	if (
		summaryLoadToken !== markedTextureSummaryLoadToken ||
		ownerLoadToken !== subMeshOptionsLoadToken
	) return;

	subMeshMarkedTextureMap.value = nextMap;
	syncPendingTextureSummaryForCurrentSubMesh();
	void populateAppliedMarkedTexturePreviews(
		targetValues,
		summaryLoadToken,
		ownerLoadToken,
		cacheBustToken,
		sourceSnapshot
	);
};

const invalidateSubMeshMarkedTextureSummaryCache = (selectionValue: string) => {
	if (!selectionValue) {
		return;
	}

	subMeshMarkedTextureMap.value = {
		...subMeshMarkedTextureMap.value,
		[selectionValue]: [],
	};
};

const naturalCompare = (left: string, right: string): number => {
	return left.localeCompare(right, undefined, {
		numeric: true,
		sensitivity: 'base',
	});
};

const setPreviewImageVisibility = (event: Event, visible: boolean) => {
	const target = event.target;
	if (!(target instanceof HTMLImageElement)) {
		return;
	}

	target.style.opacity = visible ? '1' : '0';
};

const handlePreviewImageLoad = (event: Event) => {
	setPreviewImageVisibility(event, true);
};

const handlePreviewImageError = (event: Event) => {
	setPreviewImageVisibility(event, false);
};

const handleTextureMarkChanged = () => {
	scheduleSaveTextureMemory();
	syncPendingTextureSummaryForCurrentSubMesh();
};

const rememberCustomMarkName = (markName: string) => {
	const normalizedName = markName.trim();
	if (!normalizedName || isPresetMarkName(normalizedName)) {
		return;
	}

	addCustomTextureMarkName(gamePreset.value, normalizedName);
	refreshCustomMarkNameOptions();
};

const handleTextureMarkNameChanged = (markName: string) => {
	rememberCustomMarkName(markName);
	handleTextureMarkChanged();
};

const isFaceNormalMap = (item: TextureItem): boolean => {
	const name = item.markName.trim().toLowerCase();
	return name === 'facesdfmap';
};

const handleClearTextureMarkName = (item: TextureItem) => {
	item.markName = '';
	handleTextureMarkChanged();
};

const handleQuickTextureMarkName = (item: TextureItem, markName: string) => {
	item.markName = markName;
	handleTextureMarkNameChanged(markName);
};

const handleRemoveCustomMarkName = (event: MouseEvent, markName: string) => {
	event.preventDefault();
	event.stopPropagation();

	if (removeCustomTextureMarkName(gamePreset.value, markName)) {
		refreshCustomMarkNameOptions();
		ElMessage.success(t('markTexture.messages.customMarkNameDeleted'));
	}
};

const openChannelPreviewCard = (item: TextureItem) => {
	ensureRgbaPreviewCardSize();
	channelPreviewCombinationCache.clear();
	enabledPreviewChannels.value = { R: true, G: true, B: true, A: true };
	channelPreviewScale.value = 1;
	channelPreviewOffset.value = { x: 0, y: 0 };
	activeChannelPreviewItem.value = item;
};

const openMarkedTexturePreview = (summary: SubMeshMarkedTextureSummary) => {
	openChannelPreviewCard({
		id: summary.id,
		name: summary.textureName,
		dedupedFileName: summary.dedupedFileName,
		slot: summary.slot,
		ps_hash: summary.textureHash,
		render: true,
		suffix: '',
		size: '-',
		preview: summary.preview,
		previewPath: '',
		ddsPath: summary.ddsPath,
		channelPreviews: createEmptyChannelPreviews(),
		markName: summary.markName,
		markStyle: summary.markStyle,
		faceNormalChannel: summary.faceNormalChannel,
	});
};

const closeChannelPreviewCard = () => {
	channelPreviewRenderToken += 1;
	channelPreviewCombinationCache.clear();
	activeChannelPreviewItem.value = null;
};

const setChannelPreviewCanvas = (element: unknown) => {
	channelPreviewCanvas.value = element instanceof HTMLCanvasElement ? element : null;
};

const renderChannelPreviewWithCanvas = (canvas: HTMLCanvasElement, image: DecodedDdsPreview, mask: number) => {
	const sourceWidth = image.width;
	const sourceHeight = image.height;
	if (sourceWidth <= 0 || sourceHeight <= 0) {
		return;
	}

	const width = sourceWidth;
	const height = sourceHeight;
	canvas.width = width;
	canvas.height = height;

	const context = canvas.getContext('2d', { willReadFrequently: true });
	if (!context) {
		return;
	}
	const imageData = new ImageData(new Uint8ClampedArray(image.pixels), width, height);
	for (let index = 0; index < imageData.data.length; index += 4) {
		if ((mask & 15) === 8) {
			const alpha = imageData.data[index + 3];
			imageData.data[index] = alpha;
			imageData.data[index + 1] = alpha;
			imageData.data[index + 2] = alpha;
			imageData.data[index + 3] = 255;
		} else {
			if (!(mask & 1)) imageData.data[index] = 0;
			if (!(mask & 2)) imageData.data[index + 1] = 0;
			if (!(mask & 4)) imageData.data[index + 2] = 0;
			if (!(mask & 8)) imageData.data[index + 3] = 255;
		}
	}
	context.putImageData(imageData, 0, 0);
};

const compileChannelPreviewShader = (
	context: WebGLRenderingContext,
	type: number,
	source: string
): WebGLShader | undefined => {
	const shader = context.createShader(type);
	if (!shader) {
		return undefined;
	}
	context.shaderSource(shader, source);
	context.compileShader(shader);
	if (context.getShaderParameter(shader, context.COMPILE_STATUS)) {
		return shader;
	}
	context.deleteShader(shader);
	return undefined;
};

const renderChannelPreview = (canvas: HTMLCanvasElement, image: DecodedDdsPreview, mask: number) => {
	const sourceWidth = image.width;
	const sourceHeight = image.height;
	if (sourceWidth <= 0 || sourceHeight <= 0) {
		return;
	}

	const scale = Math.min(1, RGBA_PREVIEW_RENDER_MAX_DIMENSION / Math.max(sourceWidth, sourceHeight));
	const width = Math.max(1, Math.round(sourceWidth * scale));
	const height = Math.max(1, Math.round(sourceHeight * scale));
	canvas.width = width;
	canvas.height = height;

	const context = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: false });
	if (!context) {
		renderChannelPreviewWithCanvas(canvas, image, mask);
		return;
	}

	const vertexShader = compileChannelPreviewShader(
		context,
		context.VERTEX_SHADER,
		'attribute vec2 position; varying vec2 uv; void main() { uv = (position + 1.0) * 0.5; gl_Position = vec4(position, 0.0, 1.0); }'
	);
	const fragmentShader = compileChannelPreviewShader(
		context,
		context.FRAGMENT_SHADER,
		'precision highp float; varying vec2 uv; uniform sampler2D sourceTexture; uniform vec4 channelMask; void main() { vec4 source = texture2D(sourceTexture, uv); bool alphaOnly = channelMask.a > 0.5 && dot(channelMask.rgb, vec3(1.0)) < 0.5; vec3 color = alphaOnly ? vec3(source.a) : source.rgb * channelMask.rgb; float alpha = alphaOnly ? 1.0 : (channelMask.a > 0.5 ? source.a : 1.0); gl_FragColor = vec4(color, alpha); }'
	);
	if (!vertexShader || !fragmentShader) {
		renderChannelPreviewWithCanvas(canvas, image, mask);
		return;
	}

	const program = context.createProgram();
	const buffer = context.createBuffer();
	const texture = context.createTexture();
	if (!program || !buffer || !texture) {
		context.deleteShader(vertexShader);
		context.deleteShader(fragmentShader);
		renderChannelPreviewWithCanvas(canvas, image, mask);
		return;
	}

	context.attachShader(program, vertexShader);
	context.attachShader(program, fragmentShader);
	context.linkProgram(program);
	if (!context.getProgramParameter(program, context.LINK_STATUS)) {
		context.deleteTexture(texture);
		context.deleteBuffer(buffer);
		context.deleteProgram(program);
		context.deleteShader(vertexShader);
		context.deleteShader(fragmentShader);
		renderChannelPreviewWithCanvas(canvas, image, mask);
		return;
	}

	context.useProgram(program);
	context.bindBuffer(context.ARRAY_BUFFER, buffer);
	context.bufferData(context.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]), context.STATIC_DRAW);
	const positionLocation = context.getAttribLocation(program, 'position');
	context.enableVertexAttribArray(positionLocation);
	context.vertexAttribPointer(positionLocation, 2, context.FLOAT, false, 0, 0);
	context.activeTexture(context.TEXTURE0);
	context.bindTexture(context.TEXTURE_2D, texture);
	context.pixelStorei(context.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
	context.pixelStorei(context.UNPACK_FLIP_Y_WEBGL, true);
	context.texParameteri(context.TEXTURE_2D, context.TEXTURE_MIN_FILTER, context.NEAREST);
	context.texParameteri(context.TEXTURE_2D, context.TEXTURE_MAG_FILTER, context.NEAREST);
	context.texParameteri(context.TEXTURE_2D, context.TEXTURE_WRAP_S, context.CLAMP_TO_EDGE);
	context.texParameteri(context.TEXTURE_2D, context.TEXTURE_WRAP_T, context.CLAMP_TO_EDGE);
	context.texImage2D(context.TEXTURE_2D, 0, context.RGBA, sourceWidth, sourceHeight, 0, context.RGBA, context.UNSIGNED_BYTE, image.pixels);
	context.uniform1i(context.getUniformLocation(program, 'sourceTexture'), 0);
	const channelMask = [mask & 1 ? 1 : 0, mask & 2 ? 1 : 0, mask & 4 ? 1 : 0, mask & 8 ? 1 : 0];
	context.uniform4fv(context.getUniformLocation(program, 'channelMask'), channelMask);
	context.viewport(0, 0, width, height);
	context.drawArrays(context.TRIANGLES, 0, 6);
	context.deleteTexture(texture);
	context.deleteBuffer(buffer);
	context.deleteProgram(program);
	context.deleteShader(vertexShader);
	context.deleteShader(fragmentShader);
};

const getEnabledPreviewChannelMask = (): number => {
	return (enabledPreviewChannels.value.R ? 1 : 0)
		| (enabledPreviewChannels.value.G ? 2 : 0)
		| (enabledPreviewChannels.value.B ? 4 : 0)
		| (enabledPreviewChannels.value.A ? 8 : 0);
};

const decodeRgbaDdsPreview = (bytes: Uint8Array): DecodedDdsPreview => {
	if (bytes.byteLength < 148 || String.fromCharCode(...bytes.subarray(0, 4)) !== 'DDS ') {
		throw new Error('Invalid DDS preview cache');
	}
	const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
	const height = view.getUint32(12, true);
	const width = view.getUint32(16, true);
	const fourCc = view.getUint32(84, true);
	const hasDx10Header = fourCc === 0x30315844;
	const dataOffset = hasDx10Header ? 148 : 128;
	if (hasDx10Header) {
		const dxgiFormat = view.getUint32(128, true);
		if (dxgiFormat !== 28 && dxgiFormat !== 29) throw new Error(`Unexpected DDS preview format: ${dxgiFormat}`);
	} else {
		const rgbBitCount = view.getUint32(88, true);
		const masks = [92, 96, 100, 104].map(offset => view.getUint32(offset, true));
		if (rgbBitCount !== 32 || masks.some((mask, index) => mask !== [0xff, 0xff00, 0xff0000, 0xff000000][index])) {
			throw new Error('Unexpected legacy RGBA DDS preview layout');
		}
	}
	const byteLength = width * height * 4;
	if (!width || !height || dataOffset + byteLength > bytes.byteLength) throw new Error('DDS preview data is incomplete');
	return { width, height, pixels: bytes.slice(dataOffset, dataOffset + byteLength) };
};

const displayCachedChannelPreview = () => {
	const cached = channelPreviewCombinationCache.get(getEnabledPreviewChannelMask());
	const canvas = channelPreviewCanvas.value;
	if (!cached || !canvas) return;
	canvas.width = cached.width;
	canvas.height = cached.height;
	canvas.getContext('2d')?.drawImage(cached, 0, 0);
};

const renderActiveChannelPreviews = async () => {
	const item = activeChannelPreviewItem.value;
	const renderToken = ++channelPreviewRenderToken;
	if (!item?.ddsPath) {
		return;
	}

	let image: DecodedDdsPreview;
	try {
		const preparedPath = await invoke<string>('prepare_dds_webgl_preview', { sourcePath: item.ddsPath });
		image = decodeRgbaDdsPreview(await readFile(preparedPath));
	} catch {
		return;
	}
	if (renderToken !== channelPreviewRenderToken || activeChannelPreviewItem.value?.id !== item.id) {
		return;
	}

	// Calculate the initially visible result first, then fill the remaining combinations while idle.
	const masks = [15, 14, 13, 11, 7, 1, 2, 4, 8, 0, 3, 5, 6, 9, 10, 12];
	for (const mask of masks) {
		if (renderToken !== channelPreviewRenderToken || activeChannelPreviewItem.value?.id !== item.id) {
			return;
		}
		const resultCanvas = document.createElement('canvas');
		renderChannelPreview(resultCanvas, image, mask);
		channelPreviewCombinationCache.set(mask, resultCanvas);
		displayCachedChannelPreview();
		await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
	}
};

const togglePreviewChannel = (key: TextureChannelKey) => {
	enabledPreviewChannels.value = { ...enabledPreviewChannels.value, [key]: !enabledPreviewChannels.value[key] };
	displayCachedChannelPreview();
};

const handleChannelPreviewWheel = (event: WheelEvent) => {
	event.preventDefault();
	event.stopPropagation();
	const factor = event.deltaY < 0 ? 1.12 : 1 / 1.12;
	channelPreviewScale.value = Math.min(12, Math.max(0.2, channelPreviewScale.value * factor));
};

const stopChannelPreviewDrag = () => {
	channelPreviewDragCleanup?.();
	channelPreviewDragCleanup = undefined;
};

const startChannelPreviewDrag = (event: PointerEvent) => {
	event.preventDefault();
	const start = { ...channelPreviewOffset.value };
	const move = (moveEvent: PointerEvent) => {
		channelPreviewOffset.value = {
			x: start.x + moveEvent.clientX - event.clientX,
			y: start.y + moveEvent.clientY - event.clientY,
		};
	};
	const end = () => stopChannelPreviewDrag();
	window.addEventListener('pointermove', move);
	window.addEventListener('pointerup', end);
	window.addEventListener('pointercancel', end);
	channelPreviewDragCleanup = () => {
		window.removeEventListener('pointermove', move);
		window.removeEventListener('pointerup', end);
		window.removeEventListener('pointercancel', end);
	};
};

const channelPreviewTransform = computed(() => ({
	transform: `translate(${channelPreviewOffset.value.x}px, ${channelPreviewOffset.value.y}px) scale(${channelPreviewScale.value})`,
}));

const stopRgbaPreviewResize = () => {
	rgbaPreviewResizeCleanup?.();
	rgbaPreviewResizeCleanup = undefined;
};

const startRgbaPreviewResize = (event: PointerEvent) => {
	event.preventDefault();
	event.stopPropagation();

	ensureRgbaPreviewCardSize();
	const startPointerX = event.clientX;
	const startPointerY = event.clientY;
	const startSize = rgbaPreviewCardSize.value;

	const handlePointerMove = (moveEvent: PointerEvent) => {
		const deltaX = moveEvent.clientX - startPointerX;
		const deltaY = moveEvent.clientY - startPointerY;
		const nextSize = startSize + Math.max(deltaX, deltaY);
		rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(nextSize);
	};

	const handlePointerUp = () => {
		stopRgbaPreviewResize();
		scheduleSaveGlobalUiConfig();
	};

	window.addEventListener('pointermove', handlePointerMove);
	window.addEventListener('pointerup', handlePointerUp);
	window.addEventListener('pointercancel', handlePointerUp);

	rgbaPreviewResizeCleanup = () => {
		window.removeEventListener('pointermove', handlePointerMove);
		window.removeEventListener('pointerup', handlePointerUp);
		window.removeEventListener('pointercancel', handlePointerUp);
	};
};

const handleRgbaPreviewWheel = (event: WheelEvent) => {
	event.preventDefault();
	event.stopPropagation();

	ensureRgbaPreviewCardSize();
	const wheelDelta = event.deltaY;
	if (!Number.isFinite(wheelDelta) || wheelDelta === 0) {
		return;
	}

	const resizeStep = Math.max(24, Math.round(rgbaPreviewCardSize.value * 0.06));
	const nextSize = wheelDelta < 0
		? rgbaPreviewCardSize.value + resizeStep
		: rgbaPreviewCardSize.value - resizeStep;

	rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(nextSize);
	scheduleSaveGlobalUiConfig();
};

// Retained for compatibility with persisted preview sizing and possible legacy callers.
void startRgbaPreviewResize;
void handleRgbaPreviewWheel;

const syncRgbaPreviewCardSizeToViewport = () => {
	rgbaPreviewCardSize.value = clampRgbaPreviewCardSize(rgbaPreviewCardSize.value);
};

const getTextureSuffixFromFileName = (textureFileName: string): string => {
	const lastDot = textureFileName.lastIndexOf('.');
	if (lastDot < 0) {
		return '';
	}
	return textureFileName.slice(lastDot);
};

const loadTextureMemoryForCurrentList = async (loadToken = textureListLoadToken) => {
	if (textureList.value.length === 0) {
		return;
	}

	const psHash = textureList.value[0].ps_hash;
	if (!psHash) {
		return;
	}

	try {
		console.log(`${logPrefix} trying load texture memory`, { psHash });
		const parsed = await loadTextureMemoryByPSHash(currentGameName.value, psHash);
		if (!parsed) {
			return;
		}

		isApplyingTextureMemory.value = true;
		if (loadToken !== textureListLoadToken) {
			return;
		}
		textureList.value = applyTextureMemoryToItems(textureList.value, parsed);
		console.log(`${logPrefix} texture memory loaded`, { psHash, count: textureList.value.length });
	} catch (error) {
		console.log(`${logPrefix} texture memory not found or invalid, skip`, { psHash, error });
	} finally {
		setTimeout(() => {
			isApplyingTextureMemory.value = false;
		}, 0);
	}
};

const saveTextureMemoryForCurrentList = async () => {
	if (textureList.value.length === 0) {
		return;
	}

	const psHash = textureList.value[0].ps_hash;
	if (!psHash) {
		return;
	}

	try {
		await saveTextureMemoryByPSHash(currentGameName.value, psHash, textureList.value);
		console.log(`${logPrefix} texture memory saved`, { psHash, count: textureList.value.length });
	} catch (error) {
		console.warn(`${logPrefix} failed to save texture memory`, { psHash, error });
	}
};

const resetCurrentTextureListMarks = () => {
	const defaultMarkStyle: MarkStyle = appSettings.textureMarkStylePreference === 'Slot' ? 'Slot' : 'Hash';
	textureList.value = textureList.value.map(item => ({
		...item,
		markName: '',
		markStyle: defaultMarkStyle,
		faceNormalChannel: 'R',
	}));
};

const scheduleSaveTextureMemory = () => {
	if (isApplyingTextureMemory.value) {
		return;
	}
	if (textureList.value.length === 0) return;
	const gameName = currentGameName.value;
	const psHash = textureList.value[0]?.ps_hash || '';
	const items = textureList.value.map(item => ({
		slot: item.slot,
		markName: item.markName,
		markStyle: item.markStyle,
		faceNormalChannel: item.faceNormalChannel,
		render: item.render,
		suffix: item.suffix,
	}));
	if (!psHash) return;

	if (textureMemorySaveTimer) {
		clearTimeout(textureMemorySaveTimer);
	}

	textureMemorySaveTimer = setTimeout(() => {
		// Keep the target and payload from the edit that scheduled this save.
		// Switching games must not redirect it into the new game's configuration.
		void saveTextureMemoryByPSHash(gameName, psHash, items);
	}, 250);
};

const loadTextureListByDrawCall = async (drawCallSelectionValue: string) => {
	const loadToken = ++textureListLoadToken;
	const drawCallSelection = parseDrawCallSelection(drawCallSelectionValue);
	const drawCall = drawCallSelection?.drawCall || '';
	const selectedSubMeshAtStart = selectedSubMesh.value;
	console.log(`${logPrefix} loadTextureListByDrawCall start`, {
		subMesh: selectedSubMeshAtStart,
		drawCallSelectionValue,
		drawCall,
	});

	if (!selectedSubMeshAtStart || !drawCallSelection || !drawCall) {
		textureList.value = [];
		return;
	}

	try {
		const source = getSelectedWorkspaceSource();
		const subMeshName = getSelectedSubMeshName();
		if (
			!source ||
			!subMeshName ||
			drawCallSelection.tabId !== source.tabId ||
			drawCallSelection.subMeshName !== subMeshName
		) {
			if (loadToken !== textureListLoadToken) {
				return;
			}
			textureList.value = [];
			return;
		}

		const matchedEntries = getVisibleTextureEntriesForDrawCall(source, subMeshName, drawCall);
		const nextTextureList: TextureItem[] = await mapWithConcurrency(
			matchedEntries, SUBMESH_TASK_CONCURRENCY, async ([textureFileName, textureProperty], index) => {
				const defaultMarkStyle: MarkStyle =
					appSettings.textureMarkStylePreference === 'Slot' ? 'Slot' : 'Hash';
				const faLogDedupedFileName = (textureProperty?.FALogDedupedFileName || '').trim();
				const render = (textureProperty?.FADataDedupedFileName || '').trim().length > 0;
				const dedupedBaseName = faLogDedupedFileName
					? faLogDedupedFileName.replace(/\.[^.]+$/, '')
					: '';

				const preview = '';
				let previewPath = '';
				const size = '-';
				const channelPreviews = createEmptyChannelPreviews();
				if (dedupedBaseName) {
					previewPath = await findTexturePreviewPath(
						source.workspacePath,
						faLogDedupedFileName
					);
				}

				return {
					id: `${source.tabId}-${subMeshName}-${drawCall}-${index}`,
					name: textureFileName,
					dedupedFileName: faLogDedupedFileName,
					slot: getPixelSlotFromTextureFileName(textureFileName),
					ps_hash: SSMTStringUtils.getPSHashFromFileName(textureFileName),
					render,
					suffix: getTextureSuffixFromFileName(textureFileName),
					size,
					format: textureProperty?.Format,
					preview,
					previewPath,
					ddsPath: await join(source.workspacePath, 'DedupedTextures', faLogDedupedFileName),
					channelPreviews,
					markName: '',
					markStyle: defaultMarkStyle,
					faceNormalChannel: 'R',
				};
			}
		);

		if (
			loadToken !== textureListLoadToken ||
			selectedSubMesh.value !== selectedSubMeshAtStart ||
			selectedDrawCall.value !== drawCallSelectionValue
		) {
			return;
		}
			
		textureList.value = nextTextureList;
		await loadTextureMemoryForCurrentList(loadToken);
		if (loadToken !== textureListLoadToken) {
			return;
		}
		void populateTexturePreviews(textureList.value, loadToken);
		console.log(`${logPrefix} texture list loaded`, {
			count: nextTextureList.length,
			drawCall,
		});
	} catch (error) {
		if (loadToken !== textureListLoadToken) {
			return;
		}
		console.warn(`${logPrefix} failed to load TrianglelistDedupedFileName.json`, error);
		textureList.value = [];
	}
};

const getDrawCallTextureItemCount = (
	source: WorkspaceMarkTextureSource,
	subMeshName: string,
	drawCall: string
): number => {
	if (!drawCall) {
		return 0;
	}

	return getTextureEntriesForDrawCall(source, subMeshName, drawCall).length;
};

const getPreferredDrawCallSelectionValue = (
	source: WorkspaceMarkTextureSource | undefined,
	subMeshName: string,
	options: string[]
): string | undefined => {
	if (!source || !subMeshName || options.length === 0) {
		return undefined;
	}

	let preferredSelectionValue = options[0];
	let maxTextureItemCount = -1;

	for (const selectionValue of options) {
		const parsed = parseDrawCallSelection(selectionValue);
		if (!parsed || parsed.tabId !== source.tabId || parsed.subMeshName !== subMeshName) {
			continue;
		}

		const textureItemCount = getDrawCallTextureItemCount(source, subMeshName, parsed.drawCall);
		if (textureItemCount > maxTextureItemCount) {
			maxTextureItemCount = textureItemCount;
			preferredSelectionValue = selectionValue;
		}
	}

	return preferredSelectionValue;
};

const syncDrawCallOptionsBySubMesh = (options?: { preferCurrentSelection?: boolean }) => {
	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	const drawCalls = source && subMeshName ? source.subMeshDrawCallMap[subMeshName] ?? [] : [];
	const nextDrawCallOptions = source && subMeshName
		? drawCalls
			.filter(drawCall => getDrawCallTextureItemCount(source, subMeshName, drawCall) > 0)
			.map(drawCall => serializeDrawCallSelection(source.tabId, subMeshName, drawCall))
		: [];
	drawCallOptions.value = nextDrawCallOptions;

	const shouldPreferCurrentSelection = options?.preferCurrentSelection !== false;
	const nextSelectedDrawCall = shouldPreferCurrentSelection
		? resolveDrawCallSelectionValue(selectedDrawCall.value, nextDrawCallOptions)
		: undefined;

	const rememberedSelection = getRememberedDrawCallSelectionForSubMesh(
		selectedSubMesh.value,
		nextDrawCallOptions
	);

	if (!nextSelectedDrawCall) {
		selectedDrawCall.value = rememberedSelection
			?? getPreferredDrawCallSelectionValue(source, subMeshName, nextDrawCallOptions)
			?? '';
	} else if (selectedDrawCall.value !== nextSelectedDrawCall) {
		selectedDrawCall.value = nextSelectedDrawCall;
	}

	rememberCurrentDrawCallSelection();

	console.log(`${logPrefix} drawcalls synced`, {
		subMesh: subMeshName,
		drawCallCount: drawCallOptions.value.length,
		selectedDrawCall: parseDrawCallSelection(selectedDrawCall.value)?.drawCall || '',
	});
};

const switchDrawCallByWheel = (event: WheelEvent) => {
	if (drawCallOptions.value.length <= 1) {
		return;
	}

	const currentIndex = Math.max(drawCallOptions.value.indexOf(selectedDrawCall.value), 0);
	const step = event.deltaY > 0 ? 1 : -1;

	let nextIndex = currentIndex + step;
	if (nextIndex < 0) {
		nextIndex = drawCallOptions.value.length - 1;
	} else if (nextIndex >= drawCallOptions.value.length) {
		nextIndex = 0;
	}

	selectedDrawCall.value = drawCallOptions.value[nextIndex] ?? selectedDrawCall.value;
};

const selectSubMeshFromDrawer = (selectionValue: string) => {
	rememberCurrentDrawCallSelection();
	selectedSubMesh.value = selectionValue;
};

const openSubMeshContextMenu = (event: MouseEvent, selectionValue: string): void => {
	event.preventDefault();
	event.stopPropagation();
	const position = calculateContextMenuPosition({
		clientX: event.clientX,
		clientY: event.clientY,
		menuWidth: 210,
		menuHeight: 172,
	});
	subMeshContextMenu.value = { subMesh: selectionValue, x: position.x, y: position.y };
};

const persistSubMeshRoleMetadata = async (
	selectionValue: string,
	role?: SubMeshRole,
): Promise<void> => {
	const selection = parseSubMeshSelection(selectionValue);
	const source = selection
		? workspaceSources.value.find(candidate => candidate.tabId === selection.tabId)
		: undefined;
	if (!selection || !source) return;

	await writeSubMeshRoleMetadata({
		workspacePath: source.workspacePath,
		subMesh: selection.subMeshName,
		role,
	});
};

const markSubMeshRole = async (role: 'face' | 'neck' | 'eye'): Promise<void> => {
	const selectionValue = subMeshContextMenu.value?.subMesh;
	if (!selectionValue) return;
	if (role === 'face') {
		if (!faceSubMeshes.value.includes(selectionValue)) {
			faceSubMeshes.value = [...faceSubMeshes.value, selectionValue];
		}
		if (neckSubMesh.value === selectionValue) neckSubMesh.value = '';
		eyeSubMeshes.value = eyeSubMeshes.value.filter(value => value !== selectionValue);
	} else if (role === 'eye') {
		if (!eyeSubMeshes.value.includes(selectionValue)) eyeSubMeshes.value = [...eyeSubMeshes.value, selectionValue];
		faceSubMeshes.value = faceSubMeshes.value.filter(value => value !== selectionValue);
		if (neckSubMesh.value === selectionValue) neckSubMesh.value = '';
	} else {
		neckSubMesh.value = selectionValue;
		faceSubMeshes.value = faceSubMeshes.value.filter(value => value !== selectionValue);
		eyeSubMeshes.value = eyeSubMeshes.value.filter(value => value !== selectionValue);
	}
	subMeshContextMenu.value = undefined;
	scheduleSaveSelectionMemory();
	try {
		await persistSubMeshRoleMetadata(selectionValue, role === 'face' ? 'Face' : role === 'neck' ? 'Neck' : 'Eye');
	} catch (error) {
		debugError('MarkTextureFull', 'failed to persist SubMeshRole metadata', error);
		ElMessage.error('写入 SubMesh 角色元数据失败');
	}
};

const clearSubMeshRole = async (): Promise<void> => {
	const selectionValue = subMeshContextMenu.value?.subMesh;
	if (!selectionValue) return;
	faceSubMeshes.value = faceSubMeshes.value.filter(value => value !== selectionValue);
	if (neckSubMesh.value === selectionValue) neckSubMesh.value = '';
	eyeSubMeshes.value = eyeSubMeshes.value.filter(value => value !== selectionValue);
	subMeshContextMenu.value = undefined;
	scheduleSaveSelectionMemory();
	try {
		await persistSubMeshRoleMetadata(selectionValue);
	} catch (error) {
		debugError('MarkTextureFull', 'failed to clear SubMeshRole metadata', error);
		ElMessage.error('清除 SubMesh 角色元数据失败');
	}
};

const subMeshRoleLabel = (selectionValue: string): string => {
	if (faceSubMeshes.value.includes(selectionValue)) return 'Face';
	if (neckSubMesh.value === selectionValue) return 'Neck';
	if (eyeSubMeshes.value.includes(selectionValue)) return 'Eye';
	return 'Unmarked';
};

const togglePreviewSubMeshMuted = (selectionValue: string) => {
	if (previewSyncSelectedSubMesh.value) {
		return;
	}
	mutedPreviewSubMeshMap.value = {
		...mutedPreviewSubMeshMap.value,
		[selectionValue]: !mutedPreviewSubMeshMap.value[selectionValue],
	};
};

const togglePreviewSubMeshSolo = (selectionValue: string) => {
	if (previewSyncSelectedSubMesh.value) {
		return;
	}
	soloPreviewSubMeshMap.value = {
		...soloPreviewSubMeshMap.value,
		[selectionValue]: !soloPreviewSubMeshMap.value[selectionValue],
	};
};

const updatePreviewReviewSubMeshes = (targetIds: string[]) => {
	previewReviewSubMeshMap.value = Object.fromEntries(
		targetIds.map(targetId => [targetId, true])
	);
};

const selectMarkedTextureSummary = (
	subMeshSelectionValue: string,
	summary: SubMeshMarkedTextureSummary
) => {
	rememberCurrentDrawCallSelection();
	pendingDrawerDrawCallSelection.value = summary.drawCallSelectionValue;
	selectedSubMesh.value = subMeshSelectionValue;
	selectedDrawCall.value = summary.drawCallSelectionValue;
	rememberDrawCallSelectionForSubMesh(subMeshSelectionValue, summary.drawCallSelectionValue);
};

const initializeMarkTexturePage = async () => {
	console.log(`${logPrefix} initializeMarkTexturePage start`);
	await Promise.all([loadSelectionMemory(), loadGlobalUiConfig()]);
	await Promise.all([loadPresetMarkOptions(), loadSubMeshOptions()]);
	console.log(`${logPrefix} initializeMarkTexturePage done`);
};

const yieldForTexturePreview = (): Promise<void> => new Promise(resolve => {
	if ('requestIdleCallback' in window) {
		window.requestIdleCallback(() => resolve(), { timeout: 120 });
		return;
	}
	globalThis.setTimeout(resolve, 16);
});

const populateTexturePreviews = async (items: TextureItem[], loadToken: number) => {
	for (const item of items) {
		if (loadToken !== textureListLoadToken) return;
		await yieldForTexturePreview();
		if (loadToken !== textureListLoadToken) return;
		if (!item.previewPath) continue;
		const preview = await renderDdsThumbnail(item.previewPath);
		if (loadToken !== textureListLoadToken) return;
		// Texture memory application may replace the reactive array. Resolve the
		// current item instead of mutating an object that Vue no longer renders.
		const liveItem = textureList.value.find(candidate => candidate.id === item.id);
		if (!liveItem) continue;
		liveItem.preview = preview;
		liveItem.size = await getImageSize(preview);
	}
};

const revealTextureInDedupedFolder = async (item: TextureItem) => {
	const workspacePath = getSelectedWorkspaceSource()?.workspacePath;
	if (!workspacePath) {
		ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
		return;
	}

	const dedupedFileName = item.dedupedFileName.trim();
	if (!dedupedFileName) {
		ElMessage.warning(t('markTexture.messages.currentEntryMissingDedupedFileName'));
		return;
	}

	const dedupedFolderPath = await join(workspacePath, 'DedupedTextures');
	const targetFilePath = await join(dedupedFolderPath, dedupedFileName);

	try {
		await revealItemInDir(targetFilePath);
	} catch (error) {
		console.error(`${logPrefix} reveal deduped texture failed`, {
			targetFilePath,
			error,
		});
		try {
			await openExternal(dedupedFolderPath);
		} catch {
			// Ignore fallback error and show unified feedback below.
		}
		ElMessage.error(t('markTexture.messages.revealTextureFailed'));
	}
};

const applyTextureMark = async () => {
	if (isApplyingAllDrawIBTextureMark.value) {
		return;
	}

	if (!selectedSubMesh.value) {
		ElMessage.warning(t('markTexture.messages.selectSubMeshFirst'));
		return;
	}

	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	if (!source || !subMeshName) {
		ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
		return;
	}

	isApplyingAllDrawIBTextureMark.value = true;
	try {
		const result = await applyTextureMarkForCurrentSubMesh({
			workspacePath: source.workspacePath,
			tabId: source.tabId,
			subMesh: subMeshName,
			textureList: textureList.value,
		});

		if (result.appliedCount === 0) {
			ElMessage.warning(t('markTexture.messages.noApplicableTextureMarks'));
			return;
		}

		invalidateSubMeshMarkedTextureSummaryCache(selectedSubMesh.value);
		await refreshSubMeshMarkedTextureSummary(selectedSubMesh.value);
		ElMessage.success(
			t('markTexture.messages.applyCompleted', {
				count: result.appliedCount,
				subMesh: `[${source.tabName}] ${subMeshName}`,
				folders: result.targetFolderCount,
			})
		);
	} catch (error) {
		debugError('MarkTextureFull', 'applyTextureMark failed', error);
		ElMessage.error(t('markTexture.messages.applyTextureMarksFailed'));
	} finally {
		isApplyingAllDrawIBTextureMark.value = false;
	}
};

const cancelTextureMark = () => {
	if (isClearingCurrentDrawIBTextureMark.value) {
		return;
	}

	void (async () => {
		if (!selectedSubMesh.value) {
			ElMessage.warning(t('markTexture.messages.selectSubMeshFirst'));
			return;
		}

		const source = getSelectedWorkspaceSource();
		const subMeshName = getSelectedSubMeshName();
		if (!source || !subMeshName) {
			ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
			return;
		}

		isClearingCurrentDrawIBTextureMark.value = true;
		try {
			const result = await clearCurrentSubMeshTextureMarkup({
				workspacePath: source.workspacePath,
				subMesh: subMeshName,
			});

			if (result.clearedFolderCount === 0) {
				ElMessage.warning(t('markTexture.messages.noImportJsonTargetDirectories'));
				return;
			}

			await refreshSubMeshMarkedTextureSummary(selectedSubMesh.value);
			ElMessage.success(t('markTexture.messages.clearedTextureMarks', { folders: result.clearedFolderCount }));
		} catch (error) {
			debugError('MarkTextureFull', 'cancelTextureMark failed', error);
			ElMessage.error(t('markTexture.messages.clearTextureMarksFailed'));
		} finally {
			isClearingCurrentDrawIBTextureMark.value = false;
		}
	})();
};

const deleteCurrentTextureMemory = async () => {
	if (isDeletingCurrentTextureConfig.value) {
		return;
	}

	const psHash = textureList.value[0]?.ps_hash?.trim() || '';
	if (!psHash) {
		ElMessage.warning(t('markTexture.messages.currentPsHashNotDetected'));
		return;
	}

	try {
		await ElMessageBox.confirm(
			t('markTexture.messages.confirmDeleteCurrentTextureConfig'),
			t('markTexture.dialog.deleteCurrentTextureConfigTitle'),
			{
				confirmButtonText: t('markTexture.common.confirm'),
				cancelButtonText: t('markTexture.common.cancel'),
				type: 'warning',
			}
		);
	} catch {
		return;
	}

	isDeletingCurrentTextureConfig.value = true;
	try {
		const deleted = await deleteTextureMemoryByPSHash(currentGameName.value, psHash);
		if (!deleted) {
			ElMessage.warning(t('markTexture.messages.textureConfigNotFoundForCurrentPsHash'));
			return;
		}

		resetCurrentTextureListMarks();
		ElMessage.success(t('markTexture.messages.deletedCurrentTextureConfig'));
	} catch (error) {
		debugError('MarkTextureFull', 'deleteCurrentTextureMemory failed', error);
		ElMessage.error(t('markTexture.messages.deleteCurrentTextureConfigFailed'));
	} finally {
		isDeletingCurrentTextureConfig.value = false;
	}
};

const deleteAllTextureConfigs = async () => {
	if (isDeletingAllTextureConfigs.value) {
		return;
	}

	const gameFolderName = getCurrentGameFolderName();
	let gameFolder: string;
	try {
		gameFolder = await GlobalConfig.TextureConfigsGameFolder(gameFolderName);
	} catch {
		ElMessage.error(t('markTexture.messages.unableToLocateTextureConfigFolder'));
		return;
	}

	try {
		await ElMessageBox.confirm(
			t('markTexture.messages.confirmDeleteAllTextureConfigs'),
			t('markTexture.dialog.deleteAllTextureConfigsTitle'),
			{
				confirmButtonText: t('markTexture.common.confirm'),
				cancelButtonText: t('markTexture.common.cancel'),
				type: 'warning',
			}
		);
	} catch {
		return;
	}

	isDeletingAllTextureConfigs.value = true;
	try {
		let entries: { name?: string }[];
		try {
			entries = await readDir(gameFolder);
		} catch {
			ElMessage.success(t('markTexture.messages.deletedAllTextureConfigs', { count: 0 }));
			return;
		}

		const jsonFiles = entries.filter(e => e.name?.endsWith('.json') && e.name);
		let deletedCount = 0;

		for (const entry of jsonFiles) {
			try {
				const filePath = await join(gameFolder, entry.name!);
				await moveFileToRecycleBin(filePath);
				deletedCount += 1;
			} catch {
				// skip individual file removal errors
			}
		}

		ElMessage.success(t('markTexture.messages.deletedAllTextureConfigs', { count: deletedCount }));
	} catch (error) {
		debugError('MarkTextureFull', 'deleteAllTextureConfigs failed', error);
		ElMessage.error(t('markTexture.messages.deleteAllTextureConfigsFailed'));
	} finally {
		isDeletingAllTextureConfigs.value = false;
	}
};

const updateCurrentDrawIBMarkStyle = async (markStyle: MarkStyle) => {
	if (isApplyingAllDrawIBTextureMark.value || isClearingCurrentDrawIBTextureMark.value || isUpdatingCurrentDrawIBMarkStyle.value) {
		return;
	}

	if (!selectedSubMesh.value) {
		ElMessage.warning(t('markTexture.messages.selectSubMeshFirst'));
		return;
	}

	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	if (!source || !subMeshName) {
		ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
		return;
	}

	isUpdatingCurrentDrawIBMarkStyle.value = true;
	try {
		// Step 1: Update all items in the current list to the chosen style
		textureList.value = textureList.value.map(item => ({
			...item,
			markStyle,
		}));
		await saveTextureMemoryForCurrentList();

		// Step 2: Only apply to JSON files if the submesh already has existing marks
		const hasExistingMarks = await hasExistingSubMeshTextureMarks(
			source.workspacePath,
			subMeshName
		);

		if (hasExistingMarks) {
			await updateCurrentSubMeshTextureMarkupStyle({
				workspacePath: source.workspacePath,
				subMesh: subMeshName,
				markStyle,
			});
			await refreshSubMeshMarkedTextureSummary(selectedSubMesh.value);
		}

		ElMessage.success(
			t('markTexture.messages.updatedTextureMarksStyle', {
				style: markStyle,
				count: textureList.value.length,
			})
		);
	} catch (error) {
		debugError('MarkTextureFull', 'updateCurrentDrawIBMarkStyle failed', error);
		ElMessage.error(t('markTexture.messages.batchUpdateTextureMarkStyleFailed'));
	} finally {
		isUpdatingCurrentDrawIBMarkStyle.value = false;
	}
};

const exportHashStyleTextureModTemplate = async () => {
	if (
		isApplyingAllDrawIBTextureMark.value ||
		isClearingCurrentDrawIBTextureMark.value ||
		isUpdatingCurrentDrawIBMarkStyle.value ||
		isExportingHashTextureModTemplate.value ||
		isExportingSlotTextureModTemplate.value
	) {
		return;
	}

	if (!selectedSubMesh.value) {
		ElMessage.warning(t('markTexture.messages.selectSubMeshFirst'));
		return;
	}

	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	const drawCall = parseDrawCallSelection(selectedDrawCall.value)?.drawCall || '';
	if (!source || !subMeshName || !drawCall) {
		ElMessage.warning(t('markTexture.messages.selectDrawCallFirst'));
		return;
	}

	const drawIB = resolveDrawIBAlias(source, subMeshName) || subMeshName.split('-')[0]?.trim() || '';
	if (!drawIB) {
		ElMessage.warning(t('markTexture.messages.drawIBNotDetectedForCurrentSubMesh'));
		return;
	}

	const workspaceName = (appSettings.CurrentWorkSpace || '').trim();
	if (!workspaceName) {
		ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
		return;
	}

	isExportingHashTextureModTemplate.value = true;
	try {
		const generatedModFolderPath = await PathHelper.GetWorkspaceGeneratedModFolderPath(workspaceName);
		if (!generatedModFolderPath) {
			ElMessage.error(t('markTexture.messages.generatedModFolderPathResolveFailed'));
			return;
		}

		const result = await exportHashStyleTextureModTemplateForCurrentSelection({
			workspacePath: source.workspacePath,
			generatedModFolderPath,
			tabId: source.tabId,
			drawIB,
			textureList: textureList.value,
			currentGameName: currentGameName.value,
		});

		if (result.exportedTextureCount <= 0) {
			ElMessage.warning(t('markTexture.messages.noHashTextureMarksToExport'));
			return;
		}

		await openExternal(result.generatedFolderPath);
		ElMessage.success(
			t('markTexture.messages.hashTextureModTemplateExported', {
				count: result.exportedTextureCount,
				drawIB: result.drawIB,
				skipped: result.skippedSlotStyleCount,
			})
		);
	} catch (error) {
		debugError('MarkTextureFull', 'exportHashStyleTextureModTemplate failed', error);
		ElMessage.error(t('markTexture.messages.exportHashTextureModTemplateFailed'));
	} finally {
		isExportingHashTextureModTemplate.value = false;
	}
};

const exportSlotStyleTextureModTemplate = async () => {
	if (
		isApplyingAllDrawIBTextureMark.value ||
		isClearingCurrentDrawIBTextureMark.value ||
		isUpdatingCurrentDrawIBMarkStyle.value ||
		isExportingHashTextureModTemplate.value ||
		isExportingSlotTextureModTemplate.value
	) {
		return;
	}

	if (!selectedSubMesh.value) {
		ElMessage.warning(t('markTexture.messages.selectSubMeshFirst'));
		return;
	}

	const source = getSelectedWorkspaceSource();
	const subMeshName = getSelectedSubMeshName();
	const drawCall = parseDrawCallSelection(selectedDrawCall.value)?.drawCall || '';
	if (!source || !subMeshName || !drawCall) {
		ElMessage.warning(t('markTexture.messages.selectDrawCallFirst'));
		return;
	}

	if (!canBuildSlotTemplateFromSubMesh(subMeshName)) {
		ElMessage.warning(t('markTexture.messages.invalidCurrentSubMeshForSlotTemplate'));
		return;
	}

	const workspaceName = (appSettings.CurrentWorkSpace || '').trim();
	if (!workspaceName) {
		ElMessage.warning(t('markTexture.messages.currentWorkspaceFolderNotDetected'));
		return;
	}

	isExportingSlotTextureModTemplate.value = true;
	try {
		const generatedModFolderPath = await PathHelper.GetWorkspaceGeneratedModFolderPath(workspaceName);
		if (!generatedModFolderPath) {
			ElMessage.error(t('markTexture.messages.generatedModFolderPathResolveFailed'));
			return;
		}

		const result = await exportSlotStyleTextureModTemplateForCurrentSelection({
			workspacePath: source.workspacePath,
			generatedModFolderPath,
			tabId: source.tabId,
			subMesh: subMeshName,
			textureList: textureList.value,
			gamePreset: gamePreset.value,
		});

		if (result.exportedTextureCount <= 0) {
			ElMessage.warning(t('markTexture.messages.noSlotTextureMarksToExport'));
			return;
		}

		await openExternal(result.generatedFolderPath);
		ElMessage.success(
			t('markTexture.messages.slotTextureModTemplateExported', {
				count: result.exportedTextureCount,
				subMesh: result.subMesh,
				skipped: result.skippedHashStyleCount,
			})
		);
	} catch (error) {
		debugError('MarkTextureFull', 'exportSlotStyleTextureModTemplate failed', error);
		ElMessage.error(t('markTexture.messages.exportSlotTextureModTemplateFailed'));
	} finally {
		isExportingSlotTextureModTemplate.value = false;
	}
};

onMounted(initializeMarkTexturePage);
onMounted(() => {
	ensureRgbaPreviewCardSize();
	window.addEventListener('resize', syncRgbaPreviewCardSizeToViewport);
});
let hasSeenInitialActivation = false;
let isMarkTexturePageActive = false;
const cancelMarkTextureBackgroundWork = () => {
	if (workspaceReloadTimer) {
		clearTimeout(workspaceReloadTimer);
		workspaceReloadTimer = undefined;
	}
	subMeshOptionsLoadToken += 1;
	markedTextureSummaryLoadToken += 1;
	textureListLoadToken += 1;
	channelPreviewRenderToken += 1;
};

onActivated(() => {
	isMarkTexturePageActive = true;
	if (!hasSeenInitialActivation) {
		hasSeenInitialActivation = true;
		return;
	}
	void loadSubMeshOptions();
});

onDeactivated(() => {
	isMarkTexturePageActive = false;
	cancelMarkTextureBackgroundWork();
});
onBeforeUnmount(() => {
	cancelMarkTextureBackgroundWork();
	stopRgbaPreviewResize();
	stopChannelPreviewDrag();
	channelPreviewCombinationCache.clear();
	window.removeEventListener('resize', syncRgbaPreviewCardSizeToViewport);
	/* Clear pending save timers */
	if (textureMemorySaveTimer) {
		clearTimeout(textureMemorySaveTimer);
		textureMemorySaveTimer = undefined;
	}
	if (selectionMemorySaveTimer) {
		clearTimeout(selectionMemorySaveTimer);
		selectionMemorySaveTimer = undefined;
	}
	if (workspaceUiConfigSaveTimer) {
		clearTimeout(workspaceUiConfigSaveTimer);
		workspaceUiConfigSaveTimer = undefined;
	}
});

watch(
	() => [appSettings.DBMTWorkFolder, appSettings.CurrentGameName, appSettings.CurrentWorkSpace],
	() => {
		// Cancel ownership immediately; the debounce only delays starting new work.
		subMeshOptionsLoadToken += 1;
		markedTextureSummaryLoadToken += 1;
		textureListLoadToken += 1;
		// KeepAlive retains this page while the user visits GameBanana and other
		// routes. Game switches must not start its loaders or surface its messages.
		if (!isMarkTexturePageActive) return;
		console.log(`${logPrefix} workspace-related setting changed, reload SubMesh options`, {
			dbmtWorkFolder: appSettings.DBMTWorkFolder,
			currentGameName: appSettings.CurrentGameName,
			currentWorkSpace: appSettings.CurrentWorkSpace,
		});
		if (workspaceReloadTimer) clearTimeout(workspaceReloadTimer);
		workspaceReloadTimer = setTimeout(() => {
			workspaceReloadTimer = undefined;
			void Promise.all([loadPresetMarkOptions(), loadSelectionMemory()])
				.then(() => loadSubMeshOptions());
		}, 80);
	},
	{ immediate: false }
);

watch(
	() => selectedSubMesh.value,
	(newSubMesh) => {
		if (lastValidSubMeshSelection && lastValidDrawCallSelection) {
			rememberDrawCallSelectionForSubMesh(lastValidSubMeshSelection, lastValidDrawCallSelection);
		}
		lastValidSubMeshSelection = newSubMesh || '';
		lastValidDrawCallSelection = '';
		textureListLoadToken += 1;
		textureList.value = [];
		selectedDrawCall.value = '';
		syncDrawCallOptionsBySubMesh({ preferCurrentSelection: false });
		if (
			pendingDrawerDrawCallSelection.value &&
			drawCallOptions.value.includes(pendingDrawerDrawCallSelection.value)
		) {
			selectedDrawCall.value = pendingDrawerDrawCallSelection.value;
			rememberCurrentDrawCallSelection();
		}
		pendingDrawerDrawCallSelection.value = '';
	}
);

watch(
	() => selectedDrawCall.value,
	(newDrawCall) => {
		rememberCurrentDrawCallSelection();
		if (newDrawCall && selectedSubMesh.value && drawCallOptions.value.includes(newDrawCall)) {
			lastValidSubMeshSelection = selectedSubMesh.value;
			lastValidDrawCallSelection = newDrawCall;
		}
		void loadTextureListByDrawCall(newDrawCall);
	}
);

watch(
	() => textureList.value.map(item => `${item.id}:${item.markName}:${item.markStyle}:${item.faceNormalChannel}`).join('|'),
	() => {
		syncPendingTextureSummaryForCurrentSubMesh();
	}
);

watch(
	() => [selectedSubMesh.value, selectedDrawCall.value],
	() => {
		scheduleSaveSelectionMemory();
	}
);

watch(
	() => [faceSubMeshes.value.join('|'), neckSubMesh.value, eyeSubMeshes.value.join('|'), faceNeckAlignmentEnabled.value],
	() => {
		scheduleSaveSelectionMemory();
	}
);

watch(
	() => showUnrenderedTextures.value,
	() => {
		scheduleSaveGlobalUiConfig();
		syncDrawCallOptionsBySubMesh();
		void loadTextureListByDrawCall(selectedDrawCall.value);
	}
);

watch(activeChannelPreviewItem, (item) => {
	if (!item) {
		channelPreviewRenderToken += 1;
		return;
	}
	void nextTick(renderActiveChannelPreviews);
});

</script>

<template>
	<div class="mark-texture-page">
		<div class="mark-layout">
			<section class="left-card">
				<div class="left-workspace">
					<div class="submesh-side-column">
						<nav ref="subMeshDrawerList" class="submesh-drawer-list" :aria-label="t('markTexture.tree.title')">
							<section v-for="lod in subMeshTree" :key="lod.key" class="submesh-lod-group">
								<button type="button" class="submesh-group-toggle lod-toggle" :aria-expanded="!collapsedLods.includes(lod.key)" @click="collapsedLods = toggleCollapsed(collapsedLods, lod.key)">
									<el-icon :class="{ expanded: !collapsedLods.includes(lod.key) }"><ArrowRight /></el-icon><span>{{ lod.label }}</span><small>{{ lod.ibs.length }} IB</small>
								</button>
								<div v-if="!collapsedLods.includes(lod.key)" class="submesh-lod-content">
									<section v-for="ib in lod.ibs" :key="ib.key" class="submesh-ib-group">
										<button type="button" class="submesh-group-toggle ib-toggle" :aria-expanded="!collapsedIBs.includes(ib.key)" @click="collapsedIBs = toggleCollapsed(collapsedIBs, ib.key)">
											<el-icon :class="{ expanded: !collapsedIBs.includes(ib.key) }"><ArrowRight /></el-icon><span :title="ib.label">{{ ib.label }}</span><small>{{ ib.items.length }}</small>
										</button>
										<div v-if="!collapsedIBs.includes(ib.key)" class="submesh-ib-content">
											<section
												v-for="subMesh in ib.items"
												:key="subMesh.value"
												class="submesh-drawer-item"
												:class="{ 'is-selected': subMesh.isSelected }"
											>
												<div class="submesh-drawer-row">
													<button
														class="submesh-select-btn"
														type="button"
														:title="subMesh.label"
														:data-submesh="subMesh.value"
														:aria-current="subMesh.isSelected ? 'true' : undefined"
														@click="selectSubMeshFromDrawer(subMesh.value)"
														@contextmenu="openSubMeshContextMenu($event, subMesh.value)"
													>
														<span class="submesh-label">{{ subMesh.componentLabel }}</span>
														<span v-if="subMesh.role" class="submesh-role-badge">{{ subMesh.role === 'face' ? 'F' : subMesh.role === 'neck' ? 'N' : 'E' }}</span>
														<span class="submesh-count">{{ subMesh.markCount }}</span>
													</button>
													<button type="button" class="component-alias-button" :disabled="renamingComponent" :title="t('markTexture.tree.renameComponent')" :aria-label="`${t('markTexture.tree.renameComponent')}: ${subMesh.label}`" @click="renameComponent(subMesh.value)"><el-icon><Edit /></el-icon></button>
												</div>

												<div v-if="subMesh.markedTextures.length" class="submesh-marked-list">
													<button
														v-for="summary in subMesh.markedTextures"
														:key="summary.id"
														class="submesh-marked-texture"
														type="button"
														:title="summary.textureName"
														@click="selectMarkedTextureSummary(subMesh.value, summary)"
													>
														<span class="submesh-mark-preview" @click.stop="openMarkedTexturePreview(summary)">
															<img
																:key="summary.previewKey"
																:src="summary.preview"
																:alt="summary.markName"
																:style="{ opacity: summary.preview ? 1 : 0 }"
																@load="handlePreviewImageLoad"
																@error="handlePreviewImageError"
															/>
														</span>
														<span class="submesh-mark-meta">
															<span class="submesh-mark-meta-row">
																<span class="submesh-mark-name">{{ summary.markName }}</span>
																<span
																	class="submesh-mark-status"
																	:class="summary.status === 'applied' ? 'is-applied' : 'is-pending'"
																>
																	{{ summary.status === 'applied' ? '已应用' : '未应用' }}
																</span>
															</span>
															<span class="submesh-mark-style">{{ summary.markStyle }}</span>
														</span>
													</button>
												</div>
											</section>
										</div>
									</section>
								</div>
							</section>
						</nav>

					</div>

					<div class="texture-editor-pane">
						<div class="texture-editor-toolbar">
							<div class="texture-drawcall-select" @wheel.prevent="switchDrawCallByWheel">
								<el-select
									v-model="selectedDrawCall"
									:placeholder="t('markTexture.ui.drawCall')"
									filterable
									popper-class="mark-texture-select-popper mark-texture-nav-popper"
								>
									<el-option
										v-for="item in drawCallOptionItems"
										:key="item.value"
										:label="item.label"
										:value="item.value"
									/>
								</el-select>
							</div>
						</div>

						<div class="texture-list">
							<article
								class="texture-item"
								v-for="item in textureList"
								:key="item.id"
							>
								<div class="texture-item-actions">
									<el-tooltip
										:content="t('markTexture.ui.viewRgbaChannels')"
										placement="top"
										:show-after="250"
									>
										<button
											type="button"
											class="texture-item-action-btn"
											:aria-label="t('markTexture.ui.viewRgbaChannels')"
											@click.stop="openChannelPreviewCard(item)"
										>
											<el-icon><Grid /></el-icon>
										</button>
									</el-tooltip>
									<el-tooltip
										:content="t('markTexture.ui.locateFileInDedupedTextures')"
										placement="top"
										:show-after="250"
									>
										<button
											type="button"
											class="texture-item-action-btn"
											:aria-label="t('markTexture.ui.locateFileInDedupedTextures')"
											@click.stop="revealTextureInDedupedFolder(item)"
										>
											<el-icon><View /></el-icon>
										</button>
									</el-tooltip>
								</div>
								<div class="preview-wrap">
									<img
										:src="item.preview"
										:alt="item.name"
										:style="{ opacity: item.preview ? 1 : 0 }"
										@load="handlePreviewImageLoad"
										@error="handlePreviewImageError"
									/>
								</div>

								<div class="texture-meta">
									<div class="meta-row name-row">
										<span class="label">{{ t('markTexture.ui.name') }}</span>
										<span class="value">{{ item.name }}</span>
									</div>

									<div class="meta-row">
										<span class="label">{{ t('markTexture.ui.slotSize') }}</span>
										<span class="value">{{ item.slot }} / {{ item.format || '-' }} / {{ item.size }} / {{ t('markTexture.ui.render') }}: {{ item.render }}</span>
									</div>

									<div class="meta-row select-row">
										<span class="label">{{ t('markTexture.ui.markName') }}</span>
						<div class="mark-name-select-wrap">
											<el-select
												v-model="item.markName"
												class="mark-name-main-select"
												@change="handleTextureMarkNameChanged(item.markName)"
												filterable
												allow-create
												default-first-option
												popper-class="mark-texture-select-popper mark-name-select-popper"
												:placeholder="item.markName ? t('markTexture.placeholders.selectOrEnterMarkName') : ''"
											>
												<el-option-group :label="t('markTexture.markNameGroups.preset')">
													<el-option
														v-for="name in presetMarkNameList"
														:key="`preset-${name}`"
														:label="name"
														:value="name"
													/>
												</el-option-group>
												<el-option-group
													v-if="hasCustomMarkNameOptions"
													:label="t('markTexture.markNameGroups.custom')"
												>
													<el-option
														v-for="customName in customMarkNameOptions"
														:key="`custom-${customName.name}`"
														:label="customName.name"
														:value="customName.name"
													>
														<div class="custom-mark-name-option">
															<span class="custom-mark-name-text">{{ customName.name }}</span>
															<button
																type="button"
																class="custom-mark-name-delete-button"
																:title="t('markTexture.actions.deleteCustomMarkName')"
																:aria-label="t('markTexture.actions.deleteCustomMarkName')"
																@mousedown.stop.prevent
																@click="handleRemoveCustomMarkName($event, customName.name)"
															>
																<el-icon><Delete /></el-icon>
															</button>
														</div>
													</el-option>
												</el-option-group>
											</el-select>
											<div
												v-if="!item.markName.trim()"
												class="mark-name-quick-actions"
												@mousedown.stop.prevent
											>
												<button type="button" title="DiffuseMap" aria-label="DiffuseMap" @click.stop="handleQuickTextureMarkName(item, 'DiffuseMap')">D</button>
												<button type="button" title="LightMap" aria-label="LightMap" @click.stop="handleQuickTextureMarkName(item, 'LightMap')">L</button>
												<button type="button" title="NormalMap" aria-label="NormalMap" @click.stop="handleQuickTextureMarkName(item, 'NormalMap')">N</button>
											</div>
							<div v-if="isFaceNormalMap(item)" class="face-normal-channel-control">
								<el-tooltip content="Face SDF channel" placement="top" :show-after="250">
									<el-select
										v-model="item.faceNormalChannel"
													class="face-normal-channel-select"
													size="small"
													aria-label="Face SDF channel"
													popper-class="mark-texture-select-popper face-normal-channel-popper"
													@change="handleTextureMarkChanged"
									>
																		<el-option v-for="channel in textureChannelKeys" :key="channel" :label="channel" :value="channel" />
																	</el-select>
																</el-tooltip>
																<span class="face-normal-channel-value" aria-hidden="true">{{ item.faceNormalChannel }}</span>
							</div>
							<button
								v-if="item.markName.trim()"
												type="button"
												class="mark-name-clear-button"
												:title="t('markTexture.actions.clearTextureMarkName')"
												:aria-label="t('markTexture.actions.clearTextureMarkName')"
												@mousedown.stop.prevent
												@click.stop="handleClearTextureMarkName(item)"
											>
												<el-icon><Delete /></el-icon>
											</button>
										</div>
									</div>

									<div class="meta-row select-row">
										<span class="label">{{ t('markTexture.ui.markStyle') }}</span>
										<div class="mark-style-select-wrap">
											<el-select
												v-model="item.markStyle"
												popper-class="mark-texture-select-popper"
												@change="handleTextureMarkChanged"
												:placeholder="t('markTexture.placeholders.selectMarkStyle')"
											>
												<el-option
													v-for="style in markStyleOptions"
													:key="style"
													:label="style"
													:value="style"
												/>
											</el-select>
											<span class="mark-style-selected-value">
												{{ item.markStyle || t('markTexture.placeholders.selectMarkStyle') }}
											</span>
										</div>
									</div>
								</div>
							</article>
						</div>
					</div>
				</div>
			</section>

			<div
				v-if="activeChannelPreviewItem"
				class="channel-preview-overlay"
				role="dialog"
				aria-modal="true"
				:aria-label="t('markTexture.ui.rgbaChannelPreview')"
				@click.self="closeChannelPreviewCard"
			>
				<div
					class="channel-preview-modal"
					@click.stop
				>
					<header class="channel-preview-modal-header">
						<h3>{{ activeChannelPreviewItem.name }}</h3>
						<div class="channel-preview-toolbar">
							<button
								v-for="channel in activeChannelPreviewItem.channelPreviews"
								:key="channel.key"
								type="button"
								class="channel-preview-toggle"
								:class="{ 'is-active': enabledPreviewChannels[channel.key] }"
								@click="togglePreviewChannel(channel.key)"
							>{{ channel.label }}</button>
							<button
							class="channel-preview-close-btn"
							type="button"
							:aria-label="t('markTexture.common.close')"
							@click="closeChannelPreviewCard"
						>
							<el-icon><Close /></el-icon>
						</button>
						</div>
					</header>

					<div class="channel-preview-stage" @wheel="handleChannelPreviewWheel" @pointerdown="startChannelPreviewDrag">
						<canvas
							:ref="setChannelPreviewCanvas"
							:style="channelPreviewTransform"
							:aria-label="t('markTexture.ui.rgbaChannelPreview')"
						/>
					</div>
				</div>
			</div>

			<aside class="right-card glass-scrollbar">
				<div class="menu-stack">
					<div class="filter-toggle-row">
						<span class="filter-toggle-label">{{ t('markTexture.ui.showUnrenderedTextures') }}</span>
						<el-switch
							v-model="showUnrenderedTextures"
						/>
					</div>

					<el-button
						type="primary"
						:loading="isApplyingAllDrawIBTextureMark"
						:disabled="isClearingCurrentDrawIBTextureMark"
						@click="applyTextureMark"
					>
							{{ t('markTexture.actions.applySubMeshMark') }}
					</el-button>
					<el-button
						:loading="isClearingCurrentDrawIBTextureMark"
						:disabled="isApplyingAllDrawIBTextureMark || isUpdatingCurrentDrawIBMarkStyle"
						@click="cancelTextureMark"
					>
							{{ t('markTexture.actions.clearSubMeshMark') }}
					</el-button>
					<el-button
						type="danger"
						plain
						:loading="isDeletingCurrentTextureConfig"
						:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark || isUpdatingCurrentDrawIBMarkStyle"
						@click="deleteCurrentTextureMemory"
					>
							{{ t('markTexture.actions.deleteCurrentTextureConfig') }}
					</el-button>
					<el-button
						type="danger"
						:loading="isDeletingAllTextureConfigs"
						:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark || isUpdatingCurrentDrawIBMarkStyle"
						@click="deleteAllTextureConfigs"
					>
							{{ t('markTexture.actions.deleteAllTextureConfigs') }}
					</el-button>

					<div class="menu-divider" />

					<div class="template-export-row">
						<el-button
							:loading="isExportingHashTextureModTemplate"
							:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark || isUpdatingCurrentDrawIBMarkStyle || isExportingSlotTextureModTemplate"
							@click="exportHashStyleTextureModTemplate"
						>
							{{ t('markTexture.actions.exportHashTextureModTemplate') }}
						</el-button>
						<el-button
							:loading="isExportingSlotTextureModTemplate"
							:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark || isUpdatingCurrentDrawIBMarkStyle || isExportingHashTextureModTemplate"
							@click="exportSlotStyleTextureModTemplate"
						>
							{{ t('markTexture.actions.exportSlotTextureModTemplate') }}
						</el-button>
					</div>

					<div class="mark-style-action-row">
						<el-button
							:loading="isUpdatingCurrentDrawIBMarkStyle"
							:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark"
							:title="t('markTexture.actions.setAllToHashStyle')"
							@click="updateCurrentDrawIBMarkStyle('Hash')"
						>
							Hash
						</el-button>
						<el-button
							:loading="isUpdatingCurrentDrawIBMarkStyle"
							:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark"
							:title="t('markTexture.actions.setAllToSlotStyle')"
							@click="updateCurrentDrawIBMarkStyle('Slot')"
						>
							Slot
						</el-button>
						<el-button
							:loading="isUpdatingCurrentDrawIBMarkStyle"
							:disabled="isApplyingAllDrawIBTextureMark || isClearingCurrentDrawIBTextureMark"
							:title="t('markTexture.actions.setAllToSharedSlotStyle')"
							@click="updateCurrentDrawIBMarkStyle('SharedSlot')"
						>
							SharedSlot
						</el-button>
					</div>

					<div class="right-preview-area">
						<nav class="preview-visibility-matrix" :aria-label="t('markTexture.preview.visibilityControls')">
							<button
								type="button"
								class="preview-visibility-sync"
								:class="{ 'is-active': previewSyncSelectedSubMesh }"
								:aria-pressed="previewSyncSelectedSubMesh"
								:title="t('markTexture.preview.syncVisibility')"
								:aria-label="t('markTexture.preview.syncVisibility')"
								@click="previewSyncSelectedSubMesh = !previewSyncSelectedSubMesh"
							>
								⇆
							</button>
							<div class="preview-visibility-rows" :class="{ 'is-synced': previewSyncSelectedSubMesh }" :role="previewSyncSelectedSubMesh ? 'radiogroup' : 'group'" :aria-label="t('markTexture.preview.visibilityControls')">
							<template v-for="subMesh in subMeshDrawerItems" :key="subMesh.value">
								<button v-if="previewSyncSelectedSubMesh" type="button" role="radio" class="preview-visibility-button preview-visibility-select" :class="{ 'is-current': subMesh.isSelected, 'needs-review': previewReviewSubMeshMap[subMesh.value] }" :aria-checked="subMesh.isSelected" :title="subMesh.label" :aria-label="subMesh.label" @click="selectPreviewSubMesh(subMesh.value)">{{ subMesh.isSelected ? '●' : '○' }}</button>
								<template v-else>
								<button
									type="button"
									class="preview-visibility-button preview-visibility-mute"
									:class="{
										'is-active': mutedPreviewSubMeshMap[subMesh.value],
										'is-current': subMesh.isSelected,
										'needs-review': previewReviewSubMeshMap[subMesh.value],
									}"
																		:title="`${t('markTexture.preview.muteSubmesh')}: ${subMesh.label}`"
									:aria-label="`${t('markTexture.preview.muteSubmesh')}: ${subMesh.label}`"
									@click="togglePreviewSubMeshMuted(subMesh.value)"
								>
									M
								</button>
								<button
									type="button"
									class="preview-visibility-button preview-visibility-solo"
									:class="{
										'is-active': soloPreviewSubMeshMap[subMesh.value],
										'is-current': subMesh.isSelected,
										'needs-review': previewReviewSubMeshMap[subMesh.value],
									}"
																		:title="`${t('markTexture.preview.soloSubmesh')}: ${subMesh.label}`"
									:aria-label="`${t('markTexture.preview.soloSubmesh')}: ${subMesh.label}`"
									@click="togglePreviewSubMeshSolo(subMesh.value)"
								>
									S
								</button>
								</template>
							</template>
							</div>
						</nav>
						<SubmeshPostProcessPreview
							:workspace-path="getSelectedWorkspaceSource()?.workspacePath || ''"
							:sub-mesh-name="getSelectedSubMeshName()"
							:visible-sub-mesh-targets="previewSubMeshTargets"
							:texture-options="previewTextureOptions"
							:face-sub-mesh-ids="faceSubMeshes"
							:neck-sub-mesh-id="neckSubMesh"
							:eye-sub-mesh-ids="eyeSubMeshes"
							:face-neck-alignment-enabled="faceNeckAlignmentEnabled"
							@data-type-changed="refreshSubMeshMarkedTextureSummary(selectedSubMesh)"
							@review-targets-changed="updatePreviewReviewSubMeshes"
							@face-neck-alignment-enabled-changed="faceNeckAlignmentEnabled = $event"
						/>
					</div>
				</div>
				<Teleport to="body">
					<div v-if="subMeshContextMenu" class="submesh-context-menu-layer" @pointerdown.self="subMeshContextMenu = undefined">
						<div
							class="submesh-context-menu"
							:style="{ left: `${subMeshContextMenu.x}px`, top: `${subMeshContextMenu.y}px` }"
						>
							<div class="submesh-context-menu-heading">Submesh role</div>
							<div class="submesh-context-menu-status">Current: {{ subMeshRoleLabel(subMeshContextMenu.subMesh) }}</div>
							<button type="button" :class="{ 'is-active': faceSubMeshes.includes(subMeshContextMenu.subMesh) }" @click="markSubMeshRole('face')">
								<span>标记为面部</span><span v-if="faceSubMeshes.includes(subMeshContextMenu.subMesh)" class="menu-check">✓</span>
							</button>
							<button type="button" :class="{ 'is-active': neckSubMesh === subMeshContextMenu.subMesh }" @click="markSubMeshRole('neck')">
								<span>标记为含脖颈部</span><span v-if="neckSubMesh === subMeshContextMenu.subMesh" class="menu-check">✓</span>
							</button>
							<button type="button" :class="{ 'is-active': eyeSubMeshes.includes(subMeshContextMenu.subMesh) }" @click="markSubMeshRole('eye')">
								<span>标记为眼部</span><span v-if="eyeSubMeshes.includes(subMeshContextMenu.subMesh)" class="menu-check">✓</span>
							</button>
							<button type="button" class="is-clear" @click="clearSubMeshRole">取消标记</button>
						</div>
					</div>
				</Teleport>
			</aside>
		</div>
	</div>
</template>

<style scoped>
.mark-texture-page {
	height: 100%;
	padding: 28px 18px;
	box-sizing: border-box;
	overflow: hidden;
	color: #e8ecf5;
	backdrop-filter: blur(6px);
	-webkit-backdrop-filter: blur(6px);
}

.mark-layout {
	height: 100%;
	max-height: 100%;
	display: flex;
	gap: 16px;
	align-items: stretch;
	min-height: 0;
	overflow: hidden;
}

.left-card,
.right-card {
	box-sizing: border-box;
	height: 100%;
	align-self: stretch;
	background: var(--t-card-dark-bg);
	border: var(--t-card-dark-border);
	border-radius: 14px;
	box-shadow: var(--t-card-dark-shadow);
}

.left-card {
	flex: 8;
	min-width: 0;
	display: flex;
	flex-direction: column;
	height: 100%;
	max-height: 100%;
	min-height: 0;
	overflow: hidden;
	padding: 16px;
}

.left-workspace {
	flex: 1;
	height: 100%;
	min-height: 0;
	min-width: 0;
	display: grid;
	grid-template-columns: minmax(270px, 34%) minmax(0, 1fr);
	gap: 14px;
}

.submesh-side-column {
	min-width: 0;
	min-height: 0;
	display: flex;
}

.submesh-drawer-list {
	flex: 1;
	min-height: 0;
	min-width: 0;
	/* Keep the outer post-processing page fixed, while the potentially long
	 * Submesh list remains independently scrollable. */
	overflow-y: auto;
	overflow-x: hidden;
	display: flex;
	flex-direction: column;
	gap: 8px;
	padding-right: 4px;
	scrollbar-width: thin;
	scrollbar-color: rgba(var(--theme-surface-tint-rgb), 0.36) transparent;
}

.submesh-drawer-list::-webkit-scrollbar {
	width: 8px;
}

.texture-list::-webkit-scrollbar {
	width: 0;
	height: 0;
}

.submesh-drawer-list::-webkit-scrollbar-track {
	background: transparent;
}

.submesh-drawer-list::-webkit-scrollbar-thumb {
	background: rgba(var(--theme-surface-tint-rgb), 0.28);
	border-radius: 999px;
}

.submesh-drawer-list::-webkit-scrollbar-thumb:hover {
	background: rgba(var(--theme-surface-tint-rgb), 0.44);
}

.submesh-drawer-item {
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.09);
	border-radius: 10px;
	background: rgba(var(--theme-surface-tint-rgb), 0.035);
	overflow: visible;
}

.submesh-drawer-item.is-selected {
	border-color: rgba(var(--theme-surface-tint-rgb), 0.34);
	background: rgba(var(--theme-surface-tint-rgb), 0.075);
	box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.05);
}

.submesh-drawer-row { display: flex; align-items: center; min-height: 38px; }
.submesh-drawer-row .submesh-select-btn { flex: 1; }
.submesh-lod-content, .submesh-ib-content { display: flex; flex-direction: column; gap: 6px; margin-left: 8px; padding-left: 6px; border-left: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12); }
.submesh-group-toggle { width: 100%; display: flex; align-items: center; gap: 6px; min-width: 0; padding: 9px 4px; border: 0; color: inherit; background: transparent; text-align: left; cursor: pointer; }
.submesh-group-toggle:hover { background: rgba(var(--theme-surface-tint-rgb), 0.06); }
.submesh-group-toggle span { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.submesh-group-toggle small { opacity: 0.55; }
.submesh-group-toggle .el-icon { flex: none; transition: transform 120ms ease; }
.submesh-group-toggle .expanded { transform: rotate(90deg); }
.lod-toggle { font-size: 13px; font-weight: 650; }
.ib-toggle { font-size: 12px; }
.component-alias-button { flex: none; width: 28px; height: 30px; display: grid; place-items: center; margin-right: 3px; padding: 0; border: 0; border-radius: 5px; background: transparent; color: inherit; opacity: 0.5; cursor: pointer; }
.component-alias-button:hover, .component-alias-button:focus-visible { opacity: 1; background: rgba(var(--theme-surface-tint-rgb), 0.12); }
.preview-visibility-rows { flex: none; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); grid-auto-rows: 29px; }
.preview-visibility-rows.is-synced { grid-template-columns: minmax(0, 1fr); }
.preview-visibility-select { grid-column: 1 / -1; font-size: 15px !important; }

.submesh-select-btn,
.submesh-marked-texture {
	border: 0;
	background: transparent;
	color: inherit;
	font: inherit;
	cursor: pointer;
}

.submesh-select-btn {
	width: 100%;
	min-height: 38px;
	min-width: 0;
	display: grid;
	grid-template-columns: minmax(0, 1fr) auto auto;
	align-items: center;
	gap: 8px;
	padding: 0 9px;
	text-align: left;
}

.submesh-select-btn:hover,
.submesh-marked-texture:hover {
	background: rgba(255, 255, 255, 0.045);
}

.submesh-label {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	color: rgba(244, 247, 255, 0.88);
	font-size: 12px;
	font-weight: 600;
}

.submesh-role-badge {
	min-width: 18px;
	height: 18px;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	border: 1px solid rgba(255, 214, 135, 0.42);
	border-radius: 4px;
	color: rgba(255, 225, 169, 0.96);
	background: rgba(198, 132, 45, 0.12);
	font-size: 10px;
	font-weight: 800;
}

.submesh-count {
	min-width: 24px;
	height: 20px;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	padding: 0 6px;
	border-radius: 999px;
	background: rgba(var(--theme-surface-tint-rgb), 0.11);
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
	color: rgba(232, 236, 245, 0.78);
	font-size: 11px;
	font-weight: 700;
}

.submesh-context-menu-layer {
	position: fixed;
	inset: 0;
	z-index: 1000100;
}

.submesh-context-menu {
	position: fixed;
	width: 224px;
	padding: 7px;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.26);
	border-radius: 6px;
	background: rgba(20, 26, 35, 0.98);
	box-shadow: 0 12px 36px rgba(0, 0, 0, 0.32);
}

.submesh-context-menu-heading {
	padding: 2px 9px 1px;
	color: rgba(245, 248, 255, 0.9);
	font-size: 11px;
	font-weight: 700;
}

.submesh-context-menu-status {
	padding: 0 9px 6px;
	color: rgba(220, 227, 240, 0.52);
	font-size: 10px;
}

.submesh-context-menu button {
	width: 100%;
	min-height: 32px;
	border: 0;
	border-radius: 4px;
	padding: 0 10px;
	background: transparent;
	color: rgba(242, 246, 252, 0.9);
	font: inherit;
	font-size: 12px;
	text-align: left;
	cursor: pointer;
}

.submesh-context-menu button:hover {
	background: rgba(var(--theme-surface-tint-rgb), 0.14);
	color: #fff;
}

.submesh-context-menu button.is-active {
	background: rgba(var(--theme-surface-tint-rgb), 0.16);
	color: #fff;
}

.submesh-context-menu button.is-clear {
	margin-top: 3px;
	border-top: 1px solid rgba(255, 255, 255, 0.08);
	border-radius: 0 0 4px 4px;
	color: rgba(255, 185, 185, 0.86);
}

.menu-check {
	color: rgb(117, 214, 187);
	font-weight: 800;
}

.submesh-marked-list {
	display: flex;
	flex-direction: column;
	gap: 6px;
	padding: 0 8px 8px;
}

.submesh-marked-texture {
	min-width: 0;
	display: grid;
	grid-template-columns: 38px minmax(0, 1fr);
	align-items: center;
	gap: 8px;
	padding: 6px;
	border-radius: 8px;
	text-align: left;
	background: rgba(255, 255, 255, 0.026);
}

.submesh-mark-preview {
	width: 38px;
	height: 38px;
	border-radius: 6px;
	overflow: hidden;
	border: 1px solid rgba(255, 255, 255, 0.10);
	background: rgba(0, 0, 0, 0.16);
}

.submesh-mark-preview img {
	width: 100%;
	height: 100%;
	object-fit: cover;
	display: block;
	transition: opacity 0.18s ease;
}

.submesh-mark-meta {
	min-width: 0;
	display: flex;
	flex-direction: column;
	gap: 4px;
}

.submesh-mark-meta-row {
	min-width: 0;
	display: flex;
	align-items: center;
	gap: 6px;
}

.submesh-mark-name,
.submesh-mark-style {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

.submesh-mark-name {
	color: #f4f7ff;
	font-size: 12px;
	font-weight: 700;
	white-space: normal;
	text-overflow: clip;
	overflow-wrap: anywhere;
	line-height: 1.25;
}

.submesh-mark-style {
	color: rgba(232, 236, 245, 0.62);
	font-size: 11px;
}

.submesh-mark-status {
	flex-shrink: 0;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	min-width: 42px;
	height: 16px;
	padding: 0 6px;
	border-radius: 999px;
	font-size: 10px;
	font-weight: 800;
	line-height: 1;
	white-space: nowrap;
	box-sizing: border-box;
}

.submesh-mark-status.is-applied {
	color: rgba(194, 255, 211, 0.96);
	background: rgba(38, 196, 101, 0.20);
	border: 1px solid rgba(72, 236, 126, 0.44);
}

.submesh-mark-status.is-pending {
	color: rgba(255, 205, 205, 0.98);
	background: rgba(239, 68, 68, 0.20);
	border: 1px solid rgba(255, 112, 112, 0.42);
}

.submesh-empty-mark {
	padding: 7px 8px;
	color: rgba(232, 236, 245, 0.34);
	font-size: 12px;
	text-align: center;
}

.texture-editor-pane {
	--texture-list-padding: 4px;
	min-height: 0;
	min-width: 0;
	display: flex;
	flex-direction: column;
	gap: 10px;
}

.texture-editor-toolbar {
	display: grid;
	grid-template-columns: minmax(0, 1fr);
	align-items: center;
	gap: 10px;
	min-width: 0;
	padding-right: var(--texture-list-padding);
	box-sizing: border-box;
}

.texture-drawcall-select,
.texture-drawcall-select .el-select {
	min-width: 0;
	width: 100%;
}

.texture-drawcall-select :deep(.el-select__wrapper) {
	width: 100%;
	min-width: 0;
	min-height: 34px;
	border-radius: 10px;
}

.right-card {
	flex: 3;
	min-width: 360px;
	display: flex;
	flex-direction: column;
	padding: 16px;
	max-height: 100%;
	min-height: 0;
	overflow-y: auto;
	overflow-x: hidden;
}

.texture-list {
	flex: 1;
	min-height: 0;
	overflow-y: auto;
	overflow-x: hidden;
	display: flex;
	flex-direction: column;
	gap: 12px;
	padding-right: var(--texture-list-padding);
	scrollbar-width: none;
	scrollbar-color: transparent transparent;
}

.texture-item {
	--meta-row-height: 30px;
	--meta-row-gap: 6px;
	--preview-size: calc(var(--meta-row-height) * 3 + var(--meta-row-gap) * 2);
	position: relative;
	display: grid;
	grid-template-columns: auto minmax(0, 1fr);
	align-items: start;
	gap: 14px;
	background: rgba(var(--theme-surface-tint-rgb), 0.045);
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
	border-radius: 12px;
	padding: 10px;
}

.texture-item-actions {
	position: absolute;
	top: 8px;
	right: 8px;
	z-index: 2;
	display: flex;
	align-items: center;
	gap: 6px;
	opacity: 0;
	transform: translateY(-2px);
	pointer-events: none;
	transition: opacity 0.16s ease, transform 0.16s ease;
}

.texture-item:hover .texture-item-actions,
.texture-item:focus-within .texture-item-actions {
	opacity: 1;
	transform: translateY(0);
	pointer-events: auto;
}

.texture-item-action-btn {
	width: 28px;
	height: 28px;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	padding: 0;
	color: rgba(244, 247, 255, 0.78);
	background:
		linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.18), rgba(var(--theme-surface-tint-rgb), 0.07)),
		rgba(8, 11, 18, 0.78);
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
	border-radius: 7px;
	box-shadow: 0 8px 18px rgba(0, 0, 0, 0.24), inset 0 1px 0 rgba(255, 255, 255, 0.06);
	/* 性能：该按钮逐个出现在纹理列表项中，不加 backdrop-filter（背景已是不透明 rgba(8,11,18,0.78)，模糊无视觉收益） */
	cursor: pointer;
	transition: color 0.16s ease, background 0.16s ease, border-color 0.16s ease, transform 0.16s ease;
}

.texture-item-action-btn:hover,
.texture-item-action-btn:focus-visible {
	color: #ffffff;
	background:
		linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.28), rgba(var(--theme-surface-tint-rgb), 0.12)),
		rgba(14, 18, 28, 0.94);
	border-color: rgba(var(--theme-surface-tint-rgb), 0.30);
	transform: translateY(-1px);
	outline: none;
}

.texture-item-action-btn :deep(.el-icon) {
	font-size: 16px;
}

.channel-preview-overlay {
	position: fixed;
	inset: 0;
	z-index: 3000;
	display: flex;
	align-items: center;
	justify-content: center;
	padding: 16px;
	background: rgba(5, 9, 16, 0.42);
	backdrop-filter: blur(6px);
	-webkit-backdrop-filter: blur(6px);
}

.channel-preview-modal {
	position: relative;
	width: min(82vw, 1100px);
	height: min(82vh, 850px);
	max-width: calc(100vw - 32px);
	max-height: calc(100vh - 32px);
	overflow: hidden;
	display: flex;
	flex-direction: column;
	border: var(--t-card-dark-border);
	border-radius: 14px;
	background: var(--t-card-dark-bg);
	box-shadow: var(--t-card-dark-shadow);
	backdrop-filter: blur(12px) saturate(1.15);
	-webkit-backdrop-filter: blur(12px) saturate(1.15);
	user-select: none;
}

.channel-preview-modal-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 12px;
	min-height: 44px;
	padding: 0 10px 0 14px;
	border-bottom: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
	background: rgba(var(--theme-surface-tint-rgb), 0.035);
}

.channel-preview-modal-header h3 {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	margin: 0;
	color: rgba(var(--theme-text-primary-rgb), 0.94);
	font-size: 13px;
	font-weight: 650;
	line-height: 1.2;
}

.channel-preview-toolbar {
	display: flex;
	align-items: center;
	gap: 5px;
	flex: 0 0 auto;
}

.channel-preview-toggle {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	width: 28px;
	height: 28px;
	padding: 0;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
	border-radius: 6px;
	background: rgba(255, 255, 255, 0.035);
	color: rgba(var(--theme-text-primary-rgb), 0.38);
	font-size: 12px;
	font-weight: 750;
	cursor: pointer;
}

.channel-preview-toggle.is-active {
	border-color: rgba(117, 214, 187, 0.5);
	background: rgba(117, 214, 187, 0.16);
	color: rgba(224, 255, 247, 0.96);
}

.channel-preview-stage {
	position: relative;
	flex: 1 1 auto;
	min-height: 0;
	overflow: hidden;
	display: flex;
	align-items: center;
	justify-content: center;
	background-color: #242832;
	background-image:
		linear-gradient(45deg, #343946 25%, transparent 25%),
		linear-gradient(-45deg, #343946 25%, transparent 25%),
		linear-gradient(45deg, transparent 75%, #343946 75%),
		linear-gradient(-45deg, transparent 75%, #343946 75%);
	background-position: 0 0, 0 8px, 8px -8px, -8px 0;
	background-size: 16px 16px;
	cursor: grab;
	touch-action: none;
}

.channel-preview-stage:active {
	cursor: grabbing;
}

.channel-preview-stage canvas {
	display: block;
	image-rendering: pixelated;
	max-width: 92%;
	max-height: 92%;
	object-fit: contain;
	transform-origin: center;
	will-change: transform;
}

.channel-preview-close-btn {
	width: 28px;
	height: 28px;
	display: inline-flex;
	align-items: center;
	justify-content: center;
	padding: 0;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
	border-radius: 6px;
	background: rgba(255, 255, 255, 0.045);
	color: rgba(var(--theme-text-primary-rgb), 0.8);
	cursor: pointer;
	transition: color 0.16s ease, background 0.16s ease, border-color 0.16s ease;
}

.channel-preview-close-btn:hover {
	background: rgba(239, 68, 68, 0.16);
	border-color: rgba(239, 68, 68, 0.4);
	color: rgba(var(--theme-text-primary-rgb), 0.98);
}

.channel-preview-close-btn:focus-visible {
	outline: 2px solid rgba(255, 160, 160, 0.45);
	outline-offset: 2px;
}

.channel-preview-modal-grid {
	display: grid;
	grid-template-columns: repeat(2, minmax(0, 1fr));
	grid-template-rows: repeat(2, minmax(0, 1fr));
	gap: 8px;
	aspect-ratio: 1 / 1;
	box-sizing: border-box;
	flex: 0 0 auto;
	width: 100%;
	padding: 10px;
	position: relative;
	z-index: 1;
}

.channel-modal-card {
	position: relative;
	min-width: 0;
	min-height: 0;
	aspect-ratio: 1 / 1;
	overflow: hidden;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
	border-radius: 8px;
	background: rgba(0, 0, 0, 0.14);
	box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.025);
}

.channel-modal-card-label {
	position: absolute;
	top: 10px;
	left: 10px;
	z-index: 2;
	padding: 4px 7px;
	border-radius: 5px;
	background: rgba(var(--theme-surface-tint-rgb), 0.12);
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.20);
	color: rgba(244, 247, 255, 0.92);
	font-size: 12px;
	font-weight: 700;
	line-height: 1;
	letter-spacing: 0.04em;
	pointer-events: none;
	user-select: none;
}

.channel-modal-card-preview {
	position: absolute;
	inset: 0;
}

.channel-preview-resize-handle {
	position: absolute;
	right: 6px;
	bottom: 6px;
	z-index: 2;
	width: 18px;
	height: 18px;
	border-right: 2px solid rgba(255, 255, 255, 0.32);
	border-bottom: 2px solid rgba(255, 255, 255, 0.32);
	border-bottom-right-radius: 4px;
	cursor: nwse-resize;
	opacity: 0.9;
	transition: opacity 0.2s ease, border-color 0.2s ease;
}

.channel-preview-resize-handle:hover {
	opacity: 1;
	border-color: rgba(117, 214, 187, 0.82);
}

.channel-modal-card-preview canvas {
	position: absolute;
	inset: 0;
	width: 100%;
	height: 100%;
	display: block;
	image-rendering: pixelated;
}

.preview-wrap {
	width: var(--preview-size);
	height: var(--preview-size);
	aspect-ratio: 1 / 1;
	align-self: flex-start;
	border-radius: 8px;
	overflow: hidden;
	border: 1px solid rgba(255, 255, 255, 0.1);
}

.preview-wrap img {
	width: 100%;
	height: 100%;
	object-fit: cover;
	display: block;
	image-rendering: pixelated;
	transition: opacity 0.18s ease;
}

.channel-card {
	display: flex;
	flex-direction: column;
	gap: 6px;
}

.channel-card-label {
	font-size: 0.76rem;
	font-weight: 700;
	letter-spacing: 0.06em;
	color: rgba(232, 236, 245, 0.78);
	text-align: center;
}

.channel-card-preview {
	aspect-ratio: 1 / 1;
	border-radius: 10px;
	overflow: hidden;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
	background:
		linear-gradient(135deg, rgba(255, 255, 255, 0.02), rgba(255, 255, 255, 0.01)),
		linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), 0.07), rgba(10, 12, 18, 0.72));
	box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.025);
}

.channel-card-preview img {
	width: 100%;
	height: 100%;
	object-fit: cover;
	display: block;
	image-rendering: pixelated;
	transition: opacity 0.18s ease;
}

.texture-meta {
	display: flex;
	flex-direction: column;
	gap: 6px;
	min-width: 0;
}

.meta-row {
	display: grid;
	grid-template-columns: 78px minmax(0, 1fr);
	align-items: center;
	min-height: var(--meta-row-height);
	gap: 8px;
	min-width: 0;
}

.label {
	color: rgba(232, 236, 245, 0.8);
	font-size: 0.8rem;
}

.value {
	color: #f4f7ff;
	font-size: 0.84rem;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

.name-row {
	align-items: start;
	min-height: auto;
}

.name-row .label {
	padding-top: 1px;
}

.name-row .value {
	display: block;
	max-width: 100%;
	font-weight: 600;
	font-size: 0.88rem;
	white-space: normal;
	overflow: hidden;
	text-overflow: clip;
	overflow-wrap: anywhere;
	word-break: break-all;
	line-height: 1.35;
}

.left-card :deep(.el-input__inner),
.right-card :deep(.el-input__inner) {
	font-size: 12px;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

.left-card :deep(.el-select__selected-item),
.right-card :deep(.el-select__selected-item) {
	display: flex;
	align-items: center;
	min-width: 0;
	max-width: 100%;
	font-size: 12px;
	color: rgba(255, 255, 255, 0.88) !important;
	line-height: 24px;
}

.left-card :deep(.el-select__selection),
.right-card :deep(.el-select__selection) {
	min-width: 0;
	overflow: hidden;
	position: relative;
}

.left-card :deep(.el-select__placeholder),
.right-card :deep(.el-select__placeholder) {
	display: block;
	width: 100%;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
	pointer-events: none;
}

.left-card :deep(.el-select__placeholder > span),
.right-card :deep(.el-select__placeholder > span) {
	display: block;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

.left-card :deep(.el-select__placeholder:not(.is-transparent)),
.right-card :deep(.el-select__placeholder:not(.is-transparent)),
.left-card :deep(.el-select__placeholder:not(.is-transparent) > span),
.right-card :deep(.el-select__placeholder:not(.is-transparent) > span) {
	color: rgba(255, 255, 255, 0.88) !important;
}

.left-card :deep(.el-select__placeholder:not(.is-transparent)),
.right-card :deep(.el-select__placeholder:not(.is-transparent)) {
	z-index: 2;
}

.left-card :deep(.el-select__placeholder.is-transparent),
.right-card :deep(.el-select__placeholder.is-transparent) {
	color: rgba(255, 255, 255, 0.38) !important;
}

.left-card :deep(.el-select__input-wrapper:not(.is-hidden)),
.right-card :deep(.el-select__input-wrapper:not(.is-hidden)) {
	z-index: 3;
}

/* Ensure el-select fills the grid column inside meta-row */
.left-card :deep(.meta-row.select-row .el-select) {
	width: 100%;
	min-width: 0;
}

.left-card :deep(.meta-row.select-row .el-select__wrapper) {
	width: 100%;
	min-width: 0;
}

.left-card :deep(.meta-row.select-row .mark-name-select-wrap .mark-name-main-select) {
	flex: 1 1 0;
	width: 0;
	min-width: 0;
}

.mark-name-select-wrap {
	position: relative;
	display: flex;
	align-items: center;
	flex-wrap: nowrap;
	gap: 8px;
	width: 100%;
	min-width: 0;
}

.mark-name-select-wrap :deep(.mark-name-main-select) {
	flex: 1 1 0;
	width: 0;
	min-width: 0;
}

.mark-name-select-wrap :deep(.mark-name-main-select .el-select__wrapper) {
	width: 100%;
	min-width: 0;
}

.face-normal-channel-control {
	display: flex;
	flex: 0 0 86px;
	width: 86px;
	min-width: 86px;
	align-items: center;
}

.face-normal-channel-control :deep(.el-tooltip__trigger) {
	display: flex;
	width: 100%;
}

.face-normal-channel-control :deep(.face-normal-channel-select) {
	flex: 1 1 auto;
	width: 100%;
	min-width: 0;
}

.face-normal-channel-control :deep(.face-normal-channel-select .el-select__wrapper) {
	width: 100%;
	min-height: 32px;
	height: 32px;
	padding: 0 8px;
	box-sizing: border-box;
}

/* Element Plus may hide the selected label while its internal filter input is
 * active. The visible value belongs to the control, so keep a stable display
 * layer above that implementation detail. */
.face-normal-channel-value {
	position: absolute;
	left: 9px;
	top: 50%;
	z-index: 2;
	transform: translateY(-50%);
	color: rgba(255, 255, 255, 0.9);
	font-size: 12px;
	line-height: 1;
	pointer-events: none;
}

.face-normal-channel-control {
	position: relative;
}

.mark-name-quick-actions {
	position: absolute;
	left: 12px;
	top: 50%;
	z-index: 4;
	display: inline-flex;
	align-items: center;
	transform: translateY(-50%);
	color: rgba(255, 255, 255, 0.68);
}

.mark-name-quick-actions button {
	position: relative;
	min-width: 25px;
	height: 24px;
	padding: 0 8px;
	border: 0;
	background: transparent;
	color: inherit;
	font: inherit;
	font-size: 12px;
	font-weight: 700;
	cursor: pointer;
	transition: color 0.16s ease;
}

.mark-name-quick-actions button + button::before {
	position: absolute;
	left: 0;
	top: 5px;
	bottom: 5px;
	width: 1px;
	background: rgba(255, 255, 255, 0.24);
	content: '';
}

.mark-name-quick-actions button:hover {
	color: rgba(255, 255, 255, 1);
}

.mark-name-clear-button {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	flex: 0 0 32px;
	width: 32px;
	height: 32px;
	padding: 0;
	border: 1px solid rgba(248, 113, 113, 0.28);
	border-radius: 8px;
	background: rgba(239, 68, 68, 0.08);
	color: rgba(248, 113, 113, 0.82);
	cursor: pointer;
	transition: border-color 0.16s ease, background-color 0.16s ease, color 0.16s ease;
}

.mark-name-clear-button:hover {
	border-color: rgba(248, 113, 113, 0.58);
	background: rgba(239, 68, 68, 0.18);
	color: rgba(254, 202, 202, 1);
}

.mark-style-select-wrap {
	position: relative;
	width: 100%;
	min-width: 0;
}

.mark-style-select-wrap :deep(.el-select) {
	width: 100%;
}

.mark-style-selected-value {
	position: absolute;
	inset: 3px 30px 3px 12px;
	z-index: 4;
	display: flex;
	align-items: center;
	min-width: 0;
	overflow: hidden;
	color: rgba(255, 255, 255, 0.9);
	font-size: 12px;
	line-height: 24px;
	text-overflow: ellipsis;
	white-space: nowrap;
	pointer-events: none;
}

.mark-style-select-wrap :deep(.el-select__placeholder) {
	color: transparent !important;
}

.menu-stack {
	display: flex;
	flex-direction: column;
	gap: 10px;
	flex: 0 0 auto;
	min-height: max-content;
	min-width: 0;
	overflow: visible;
}

.right-preview-area {
	flex: 0 0 auto;
	min-height: max-content;
	display: grid;
	grid-template-columns: 54px minmax(0, 1fr);
	gap: 8px;
	margin-top: 4px;
}

.right-preview-area :deep(.submesh-preview-panel) {
	min-width: 0;
	min-height: max-content;
	height: max-content;
	overflow: visible;
}

.preview-visibility-matrix {
	min-height: 0;
	display: flex;
	flex-direction: column;
	overflow-y: auto;
	overflow-x: hidden;
	border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.2);
	border-radius: 9px;
	background: rgba(var(--theme-surface-tint-rgb), 0.045);
	box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.018);
	scrollbar-width: thin;
	scrollbar-color: rgba(var(--theme-surface-tint-rgb), 0.32) transparent;
}

.preview-visibility-matrix button {
	min-width: 0;
	margin: 0;
	border: 0;
	border-right: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
	border-bottom: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
	color: rgba(237, 242, 252, 0.7);
	background: transparent;
	font-size: 10px;
	font-weight: 800;
	line-height: 1;
	cursor: pointer;
	transition: color 0.14s ease, background 0.14s ease;
}

.preview-visibility-solo {
	border-right: 0 !important;
}

.preview-visibility-matrix button:hover:not(:disabled) {
	color: rgba(255, 255, 255, 0.96);
	background: rgba(var(--theme-surface-tint-rgb), 0.12);
}

.preview-visibility-matrix button.is-active {
	color: rgba(184, 239, 255, 1);
	background: rgba(var(--theme-surface-tint-rgb), 0.2);
}

.preview-visibility-matrix button.is-current {
	box-shadow: inset 0 0 0 2px rgba(var(--theme-surface-tint-rgb), 0.68);
}

.preview-visibility-matrix button.needs-review {
	color: rgba(255, 234, 234, 0.96);
	background: rgba(239, 68, 68, 0.5);
}

.preview-visibility-matrix button.needs-review:hover:not(:disabled) {
	background: rgba(239, 68, 68, 0.66);
}

.preview-visibility-matrix button:disabled {
	color: rgba(232, 236, 245, 0.3);
	cursor: default;
}

.preview-visibility-sync {
	min-height: 29px;
	flex: none;
	grid-column: 1 / -1;
	border-right: 0 !important;
	font-size: 16px !important;
}

.menu-stack > div,
.menu-stack .el-dropdown {
	min-width: 0;
	width: 100%;
}

.menu-stack .el-select {
	width: 100%;
	min-width: 0;
}

.menu-stack :deep(.el-select__wrapper) {
	width: 100%;
	min-width: 0;
}

:global(.mark-texture-select-popper.el-select__popper.el-popper) {
	max-width: min(460px, calc(100vw - 24px)) !important;
}

:global(.mark-name-select-popper.el-select__popper.el-popper) {
	width: min(460px, calc(100vw - 24px)) !important;
	min-width: min(460px, calc(100vw - 24px)) !important;
	overflow: hidden;
}

:global(.mark-name-select-popper .el-select-dropdown),
:global(.mark-name-select-popper .el-select-dropdown__wrap),
:global(.mark-name-select-popper .el-scrollbar),
:global(.mark-name-select-popper .el-scrollbar__wrap),
:global(.mark-name-select-popper .el-select-dropdown__list),
:global(.mark-name-select-popper .el-select-group),
:global(.mark-name-select-popper .el-select-group__wrap) {
	width: 100% !important;
	min-width: 0 !important;
	max-width: 100% !important;
	box-sizing: border-box;
	overflow-x: hidden;
}

:global(.mark-texture-nav-popper.el-select__popper.el-popper) {
	max-width: min(520px, calc(100vw - 24px)) !important;
}

:global(.mark-texture-select-popper .el-select-dropdown__item) {
	max-width: 100%;
	padding-right: 28px;
}

:global(.mark-name-select-popper .el-select-group__title) {
	color: rgba(255,255,255,0.52);
	font-size: 11px;
	font-weight: 700;
	letter-spacing: 0;
}

:global(.mark-name-select-popper .el-select-dropdown__item) {
	width: 100%;
	min-width: 0;
	max-width: 100%;
	box-sizing: border-box;
	height: 32px;
	line-height: 32px;
	padding-right: 8px;
}

:global(.custom-mark-name-option) {
	display: flex;
	align-items: center;
	gap: 8px;
	width: 100%;
	min-width: 0;
}

:global(.custom-mark-name-text) {
	flex: 1 1 auto;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}

:global(.custom-mark-name-delete-button) {
	display: inline-flex;
	align-items: center;
	justify-content: center;
	flex: 0 0 22px;
	width: 22px;
	height: 22px;
	padding: 0;
	border: 0;
	border-radius: 5px;
	background: transparent;
	color: rgba(255,255,255,0.42);
	cursor: pointer;
	opacity: 0.72;
	transition: background-color 0.16s ease, color 0.16s ease, opacity 0.16s ease;
}

:global(.custom-mark-name-delete-button:hover) {
	background: rgba(255,255,255,0.10);
	color: rgba(255,255,255,0.92);
	opacity: 1;
}

.menu-stack .el-button {
	margin-left: 0;
	width: 100%;
	height: 34px;
	border-radius: 10px;
	border: 1px solid rgba(255,255,255,0.10);
	background: rgba(255,255,255,0.04);
	color: rgba(255,255,255,0.70);
	font-size: 12px;
	font-weight: 600;
	letter-spacing: 0.3px;
	transition: all 0.2s ease;
	position: relative;
	overflow: hidden;
}
.menu-stack .el-button::before {
	display: none;
}
.menu-stack .el-button:hover {
	background: rgba(255,255,255,0.08);
	border-color: rgba(255,255,255,0.20);
	color: rgba(255,255,255,0.90);
	transform: translateY(-1px);
	box-shadow: 0 4px 12px rgba(0,0,0,0.08);
}
.menu-stack .el-button:active {
	transform: translateY(0) scale(0.98);
}
.menu-stack .el-button.is-disabled,
.menu-stack .el-button.is-disabled:hover {
	opacity: 0.35;
	cursor: not-allowed;
	transform: none !important;
	background: rgba(255,255,255,0.04);
	border-color: rgba(255,255,255,0.10);
}
.menu-stack .el-button.el-button--primary {
	background: rgba(var(--theme-surface-tint-rgb), 0.08);
	border-color: rgba(var(--theme-surface-tint-rgb), 0.18);
	color: rgba(var(--theme-surface-tint-rgb), 0.85);
}
.menu-stack .el-button.el-button--primary:hover {
	background: rgba(var(--theme-surface-tint-rgb), 0.14);
	border-color: rgba(var(--theme-surface-tint-rgb), 0.30);
	color: rgba(166, 232, 255, 1);
	box-shadow: 0 4px 16px rgba(var(--theme-surface-tint-rgb), 0.08);
}
.menu-stack .el-button.el-button--danger {
	background: rgba(220, 80, 80, 0.08);
	border-color: rgba(220, 80, 80, 0.18);
	color: rgba(255, 130, 130, 0.85);
}
.menu-stack .el-button.el-button--danger:hover {
	background: rgba(220, 80, 80, 0.14);
	border-color: rgba(220, 80, 80, 0.30);
	color: rgba(255, 150, 150, 1);
	box-shadow: 0 4px 16px rgba(220, 80, 80, 0.08);
}

.filter-toggle-row {
	display: grid;
	grid-template-columns: minmax(0, 1fr) auto;
	align-items: center;
	gap: 12px;
	width: 100%;
	box-sizing: border-box;
	min-height: 34px;
	padding: 7px 10px;
	border-radius: 10px;
	background: rgba(255, 255, 255, 0.04);
	border: 1px solid rgba(255, 255, 255, 0.06);
}

.filter-toggle-label {
	min-width: 0;
	font-size: 12px;
	line-height: 1.35;
	color: rgba(232, 236, 245, 0.88);
	overflow-wrap: anywhere;
	white-space: normal;
}

.filter-toggle-row :deep(.el-switch) {
	justify-self: end;
	flex-shrink: 0;
}

.template-export-row {
	display: grid;
	grid-template-columns: minmax(0, 1fr);
	gap: 10px;
}

.template-export-row .el-button {
	margin-left: 0;
	width: 100%;
}

.mark-style-action-row {
	display: grid;
	grid-template-columns: minmax(0, 1fr);
	gap: 8px;
}

.mark-style-action-row .el-button {
	min-width: 0;
	width: 100%;
	margin-left: 0;
	padding-left: 6px;
	padding-right: 6px;
}

.menu-bottom {
	margin-top: auto;
	display: flex;
	flex-direction: column;
	gap: 10px;
}

.menu-divider {
	height: 1px;
	background: rgba(255, 255, 255, 0.12);
}

.folder-dropdown-btn {
	justify-content: flex-start;
}

.folder-dropdown-icon {
	margin-right: 6px;
}

@media (max-width: 960px) {
	.mark-texture-page {
		overflow: auto;
	}

	.mark-layout {
		flex-direction: column;
		height: auto;
	}

	.left-card,
	.right-card {
		min-width: 0;
		position: static;
		max-height: none;
		overflow: visible;
	}

	.left-card {
		min-height: 420px;
	}

	.left-workspace {
		grid-template-columns: 1fr;
	}

	.submesh-drawer-list {
		max-height: 360px;
	}

	.right-preview-area {
		min-height: 460px;
	}

	.texture-editor-toolbar {
		grid-template-columns: 1fr;
	}

	.texture-item {
		grid-template-columns: 1fr;
	}

	.channel-preview-overlay {
		padding: 16px;
	}

	.channel-preview-modal {
		max-width: calc(100vw - 32px);
		max-height: calc(100vh - 32px);
		padding: 10px;
	}

	.channel-preview-modal-grid {
		grid-template-columns: repeat(2, minmax(0, 1fr));
		grid-template-rows: repeat(2, minmax(0, 1fr));
		gap: 8px;
	}

	.preview-wrap {
		width: 100%;
		height: 180px;
	}

	.template-export-row {
		grid-template-columns: 1fr;
	}
}
</style>
