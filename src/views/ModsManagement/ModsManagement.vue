<script setup lang="ts">
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, computed, watch, reactive, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { openPath } from '@tauri-apps/plugin-opener';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AppStateManager } from '../../store/AppStateManager';
import { takePendingModInstallDrop } from '../../store/ModInstallDropBus';
import { open } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { ModManager, type DisabledParentGroupInfo } from '../../store/ModManager';
import { ModTagStore } from '../../store/ModTagStore';
import { ModStateStore } from '../../store/ModStateStore';
import { ModPresetStore } from '../../store/ModPresetStore';
import type { ModPresetsFile } from '../../store/ModPresetStore';
import { migotoIniService } from './MigotoIni';
import { captureModRuntimeState, restoreModRuntimeState } from './ModStateMemory';
import { Back, Folder, CircleClose, Top, Right } from '@element-plus/icons-vue';
import { ElMessage, ElMessageBox } from 'element-plus';
import { debugLog, debugWarn } from '../../utils/debugLog';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import type { GroupInfo, ModAnalysisResult, ModInfo } from './ModsManagement.types';
import { useModsManagementModKeys } from './ModsManagement.modKeys';
import { useModsManagementInstall } from './ModsManagement.install';
import { useModsManagementTags } from './ModsManagement.tags';
import { useModsManagementDialogs } from './ModsManagement.dialogs';
import { useModsManagementPreviews } from './ModsManagement.previews';
import EditSwitchKeyList from './EditSwitchKeyList.vue';
import InlineModKeysDialog from './InlineModKeysDialog.vue';
import { applyInlineModKeys, hasInlineModKeyBackup, inspectInlineModKeys, restoreInlineModKeys, type InlineKeyChoice } from './InlineModKeys';
import TagManagement from './TagManagement.vue';
import ContextMenu from './ContextMenu.vue';
import GroupContextMenu from './GroupContextMenu.vue';
import GameBananaCategoryIconDialog from './GameBananaCategoryIconDialog.vue';
import InstallModDialog from './InstallModDialog.vue';
import ModCard from './ModCard.vue';
import Mod3DPreview from './Mod3DPreview.vue';
import ModPresetPopover from './ModPresetPopover.vue';
import { getModDynamicStyle, getPreviewFileNameLower } from './ModsManagement.preview';
import { clipboardImageToPngBytes } from './ModsManagement.clipboardPreview';
import { buildModSortComparator, compareModsByDefault, type SortCriterion, type SortOrder } from './ModsManagement.sort';
import { mergeGroupOrder } from './ModsManagement.order';
import { createMutationReconciler, type MutationImpact } from './ModsManagement.mutations';
import { readImage } from '@tauri-apps/plugin-clipboard-manager';
import {
    buildMovedModRelativePath as buildMovedModRelativePathUtil,
    buildRenamedModRelativePath as buildRenamedModRelativePathUtil,
    getModGroupFromRelativePath as getModGroupFromRelativePathUtil,
    getGroupAncestors as getGroupAncestorsUtil,
    getGroupParent as getGroupParentUtil,
    isDisabledGroup as isDisabledGroupUtil,
    isSameOrChildPath as isSameOrChildPathUtil,
    renameKeyedRecord as renameKeyedRecordUtil,
    replaceFsPathSegment as replaceFsPathSegmentUtil,
    replacePathPrefix as replacePathPrefixUtil,
    ROOT_PARENT_ID,
    normalizeGroupId as normalizeGroupIdUtil,
    normalizeModIdentity,
    stripDisabledFolderName as stripDisabledFolderNameUtil,
} from './ModsManagement.paths';
import {
    loadExpandedState as loadExpandedStateStorage,
    loadGroupOrders as loadGroupOrdersStorage,
    loadManualOrders as loadManualOrdersStorage,
    loadSelectedGroupState as loadSelectedGroupStateStorage,
    loadSubgroupPreviewCache as loadSubgroupPreviewCacheStorage,
    persistExpandedState as persistExpandedStateStorage,
    persistGroupOrders as persistGroupOrdersStorage,
    persistManualOrders as persistManualOrdersStorage,
    persistSelectedGroupState as persistSelectedGroupStateStorage,
} from './ModsManagement.storage';


const gamesList = AppStateManager.gamesList;
const appSettings = AppStateManager.appSettings;
const router = useRouter();
const { t } = useI18n();

const isNsfwMod = (mod: ModInfo): boolean => {
    const taggedGroups = [mod.group, ...getGroupAncestors(mod.group)]
        .filter((groupId, index, values) => !!groupId && values.indexOf(groupId) === index)
        .flatMap((groupId) => getTagsForGroup({ id: groupId, name: groupId, path: groupId, enabled: true } as GroupInfo).map((tag) => tag.name));
    const text = [
        mod.name,
        mod.group,
        ...getTagsForMod(mod).map((tag) => tag.name),
        ...taggedGroups,
    ].join(' ').toLowerCase();
    return /\bnsfw\b|\br[- ]?18\b|\b18\+|\badult\b|\bhentai\b|\bnud(?:e|ity)\b|\bnaked\b|\bsex(?:ual)?\b|成人|色情|裸露|无码/.test(text);
};

const isNsfwGroup = (group: GroupInfo): boolean => {
    const taggedGroups = [group.id, ...getGroupAncestors(group.id)]
        .filter((groupId, index, values) => !!groupId && values.indexOf(groupId) === index)
        .flatMap((groupId) => getTagsForGroup({ id: groupId, name: groupId, path: groupId, enabled: true } as GroupInfo).map((tag) => tag.name));
    const text = [group.name, group.id, group.path, ...taggedGroups].join(' ').toLowerCase();
    return /\bnsfw\b|\br[- ]?18\b|\b18\+|\badult\b|\bhentai\b|\bnud(?:e|ity)\b|\bnaked\b|\bsex(?:ual)?\b|成人|色情|裸露|无码/.test(text);
};

type ElTreeNode = { key?: string; level: number; expanded: boolean; data: GroupInfo; childNodes: ElTreeNode[]; expand: () => void; collapse: () => void; setExpandedKeys?: (keys: string[]) => void }

const modPulseState = reactive<Record<string, boolean>>({});
const windowEffectsActive = ref(true);

type ResizablePanel = 'groups' | 'tags';

const MOD_MANAGER_COLUMN_WIDTHS_STORAGE = 'mod-management-column-widths-v1';
const DEFAULT_PANEL_WIDTHS: Record<ResizablePanel, number> = { groups: 250, tags: 290 };
const MIN_PANEL_WIDTHS: Record<ResizablePanel, number> = { groups: 200, tags: 240 };
const MAX_PANEL_WIDTHS: Record<ResizablePanel, number> = { groups: 420, tags: 460 };
const MIN_MODS_PANEL_WIDTH = 320;
const WORKSPACE_HORIZONTAL_PADDING = 20;
const COLUMN_RESIZERS_WIDTH = 20;

const clampPanelWidth = (panel: ResizablePanel, value: unknown) => {
    const parsed = Number(value);
    const fallback = DEFAULT_PANEL_WIDTHS[panel];
    return Math.round(Math.min(MAX_PANEL_WIDTHS[panel], Math.max(MIN_PANEL_WIDTHS[panel], Number.isFinite(parsed) ? parsed : fallback)));
};

const loadPanelWidths = (): Record<ResizablePanel, number> => {
    try {
        const stored = JSON.parse(localStorage.getItem(MOD_MANAGER_COLUMN_WIDTHS_STORAGE) || '{}') as Partial<Record<ResizablePanel, number>>;
        return {
            groups: clampPanelWidth('groups', stored.groups),
            tags: clampPanelWidth('tags', stored.tags),
        };
    } catch {
        return { ...DEFAULT_PANEL_WIDTHS };
    }
};

const workspaceRef = ref<HTMLDivElement | null>(null);
const panelWidths = reactive(loadPanelWidths());
const activePanelResize = ref<ResizablePanel | null>(null);
const workspaceColumnStyle = computed(() => ({
    '--groups-panel-width': `${panelWidths.groups}px`,
    '--tags-panel-width': `${panelWidths.tags}px`,
}));

let panelResizeObserver: ResizeObserver | null = null;
let panelResizePointerId: number | null = null;
let panelResizeStartX = 0;
let panelResizeStartWidth = 0;

const getAvailableSidePanelWidth = () => {
    const workspaceWidth = workspaceRef.value?.clientWidth || window.innerWidth;
    return Math.max(
        MIN_PANEL_WIDTHS.groups + MIN_PANEL_WIDTHS.tags,
        workspaceWidth - WORKSPACE_HORIZONTAL_PADDING - COLUMN_RESIZERS_WIDTH - MIN_MODS_PANEL_WIDTH,
    );
};

const constrainPanelWidths = () => {
    if (window.innerWidth <= 900) return;

    panelWidths.groups = clampPanelWidth('groups', panelWidths.groups);
    panelWidths.tags = clampPanelWidth('tags', panelWidths.tags);

    const available = getAvailableSidePanelWidth();
    const currentTotal = panelWidths.groups + panelWidths.tags;
    if (currentTotal <= available) return;

    const groupsExtra = panelWidths.groups - MIN_PANEL_WIDTHS.groups;
    const tagsExtra = panelWidths.tags - MIN_PANEL_WIDTHS.tags;
    const availableExtra = Math.max(0, available - MIN_PANEL_WIDTHS.groups - MIN_PANEL_WIDTHS.tags);
    const extraTotal = groupsExtra + tagsExtra;
    const scale = extraTotal > 0 ? Math.min(1, availableExtra / extraTotal) : 0;

    panelWidths.groups = Math.round(MIN_PANEL_WIDTHS.groups + groupsExtra * scale);
    panelWidths.tags = Math.round(MIN_PANEL_WIDTHS.tags + tagsExtra * scale);
};

const persistPanelWidths = () => {
    localStorage.setItem(MOD_MANAGER_COLUMN_WIDTHS_STORAGE, JSON.stringify({
        groups: panelWidths.groups,
        tags: panelWidths.tags,
    }));
};

const setPanelWidth = (panel: ResizablePanel, requestedWidth: number) => {
    const otherPanel: ResizablePanel = panel === 'groups' ? 'tags' : 'groups';
    const availableForPanel = Math.max(MIN_PANEL_WIDTHS[panel], getAvailableSidePanelWidth() - panelWidths[otherPanel]);
    panelWidths[panel] = Math.min(clampPanelWidth(panel, requestedWidth), availableForPanel);
};

const startPanelResize = (panel: ResizablePanel, event: PointerEvent) => {
    if (event.button !== 0 || window.innerWidth <= 900) return;
    event.preventDefault();
    activePanelResize.value = panel;
    panelResizePointerId = event.pointerId;
    panelResizeStartX = event.clientX;
    panelResizeStartWidth = panelWidths[panel];
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
};

const movePanelResize = (event: PointerEvent) => {
    const panel = activePanelResize.value;
    if (!panel || event.pointerId !== panelResizePointerId) return;
    const pointerDelta = event.clientX - panelResizeStartX;
    setPanelWidth(panel, panelResizeStartWidth + (panel === 'groups' ? pointerDelta : -pointerDelta));
};

const stopPanelResize = (event?: PointerEvent) => {
    if (!activePanelResize.value) return;
    const target = event?.currentTarget as HTMLElement | undefined;
    const pointerId = panelResizePointerId;
    activePanelResize.value = null;
    panelResizePointerId = null;
    if (target && pointerId !== null && target.hasPointerCapture(pointerId)) {
        target.releasePointerCapture(pointerId);
    }
    persistPanelWidths();
};

const resetPanelWidth = (panel: ResizablePanel) => {
    setPanelWidth(panel, DEFAULT_PANEL_WIDTHS[panel]);
    persistPanelWidths();
};

const onPanelResizeKeydown = (panel: ResizablePanel, event: KeyboardEvent) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const direction = event.key === 'ArrowRight' ? 1 : -1;
    setPanelWidth(panel, panelWidths[panel] + direction * (panel === 'groups' ? 16 : -16));
    persistPanelWidths();
};

onMounted(() => {
    panelResizeObserver = new ResizeObserver(constrainPanelWidths);
    if (workspaceRef.value) panelResizeObserver.observe(workspaceRef.value);
    constrainPanelWidths();
});

onUnmounted(() => {
    panelResizeObserver?.disconnect();
    panelResizeObserver = null;
    stopPanelResize();
});

// ============================================================
// Utility functions
// ============================================================
const tryGetCurrentWindow = () => {
    try { return getCurrentWindow(); }
    catch (error) { console.warn('Failed to access current window for effect throttling:', error); return null; }
};

const refreshWindowEffectsState = async () => {
    const visibleDocument = !document.hidden;
    const focusedDocument = typeof document.hasFocus === 'function' ? document.hasFocus() : true;
    let isVisible = true; let isMinimized = false;
    const appWindow = tryGetCurrentWindow();
    if (appWindow) {
        try { [isVisible, isMinimized] = await Promise.all([appWindow.isVisible(), appWindow.isMinimized()]); }
        catch (error) { console.warn('Failed to read current window state for effect throttling:', error); }
    }
    windowEffectsActive.value = visibleDocument && focusedDocument && isVisible && !isMinimized;
};

const shouldRunVisualEffects = computed(() => windowEffectsActive.value);

const buildRenamedModRelativePath = (mod: ModInfo, newName: string) => buildRenamedModRelativePathUtil(mod, newName);
const buildMovedModRelativePath = (mod: ModInfo, targetGroupId: string) => buildMovedModRelativePathUtil(mod, targetGroupId);
const getModGroupFromRelativePath = (relativePath: string) => getModGroupFromRelativePathUtil(relativePath);
const isDisabledGroup = (group: GroupInfo) => isDisabledGroupUtil(group);
const MODS_TREE_ROOT_ID = '__MODS_TREE_ROOT__';

const onModCardMouseMove = (e: MouseEvent) => {
    const card = e.currentTarget as HTMLElement | null; if (!card) return;
    const rect = card.getBoundingClientRect(); if (rect.width <= 0 || rect.height <= 0) return;
    card.style.setProperty('--mx', `${(Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width)) * 100).toFixed(2)}%`);
    card.style.setProperty('--my', `${(Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height)) * 100).toFixed(2)}%`);
};

const onModCardMouseLeave = (e: MouseEvent) => {
    const card = e.currentTarget as HTMLElement | null; if (!card) return;
    card.style.setProperty('--mx', '50%'); card.style.setProperty('--my', '18%');
};

const triggerModPulse = (modId: string) => {
    modPulseState[modId] = true;
    setTimeout(() => { delete modPulseState[modId]; }, 540);
};

// ============================================================
// Mod key hover / floater state
// ============================================================
const hoveredKeyModId = ref<string | null>(null);
const keyListPos = ref({ x: 0, y: 0 });
let keyListHideTimer: ReturnType<typeof setTimeout> | null = null;

const openModKeyEditorForCurrentHover = () => {
    if (!hoveredKeyModId.value) return;
    const mod = mods.value.find((m) => m.id === hoveredKeyModId.value);
    if (mod) { hoveredKeyModId.value = null; openModKeyEditor(mod); }
};

const showKeyFloater = (mod: ModInfo, e: MouseEvent) => {
    const badge = (e.currentTarget as HTMLElement);
    const rect = badge.getBoundingClientRect();
    const panelW = 300; const panelH = 380;
    let x = rect.right + 8; let y = rect.top;
    if (x + panelW > window.innerWidth - 8) x = rect.left - panelW - 8;
    if (y + panelH > window.innerHeight - 8) y = Math.max(8, window.innerHeight - 8 - panelH);
    keyListPos.value = { x, y };
    if (hoveredKeyModId.value !== mod.id) { hoveredKeyModId.value = mod.id; loadModKeyList(mod); }
    if (keyListHideTimer) { clearTimeout(keyListHideTimer); keyListHideTimer = null; }
};

const hideKeyFloater = () => { keyListHideTimer = setTimeout(() => { hoveredKeyModId.value = null; }, 250); };
const keepKeyFloater = () => { if (keyListHideTimer) { clearTimeout(keyListHideTimer); keyListHideTimer = null; } };

// Close modals on Escape key
onMounted(() => {
    const handler = (e: KeyboardEvent) => {
        if (e.key === 'Escape') {
            if (modKeyListDialog.visible) {
                closeModKeyList();
            } else if (modTagDialog.visible) {
                modTagDialog.visible = false;
            }
        }
    };
    window.addEventListener('keydown', handler);
    onUnmounted(() => window.removeEventListener('keydown', handler));
});



const bumpGroupTree = () => {
    groupTreeRenderKey.value += 1;
    nextTick(() => applyExpandedToTree());
};

const groupIconVersion = ref(Date.now());
let mutationReconciler: ReturnType<typeof createMutationReconciler> | null = null;

const refreshAfterMutation = async (impact: MutationImpact = 'content') => {
    if (!mutationReconciler) {
        throw new Error('Mutation reconciler is not initialized');
    }
    await mutationReconciler.reconcile(impact);
};

// Group Management
const createNewGroup = async () => {
    try {
        const result = await ElMessageBox.prompt(t('modsManagement.messages.enterNewCategoryName'), t('modsManagement.dialog.createCategoryTitle'), {
            confirmButtonText: t('modsManagement.common.create'),
            cancelButtonText: t('modsManagement.common.cancel'),
            inputPattern: /^[^\\/:*?"<>|]+$/,
            inputErrorMessage: t('modsManagement.messages.nameContainsInvalidCharacters'),
        }) as { value: string };
        
        const value = result.value.trim();
        
        if (value) {
            await ModManager.createModGroup(selectedGame.value, value);
            ElMessage.success(t('modsManagement.messages.categoryCreated'));
            await refreshAfterMutation('structure');
        }
    } catch (e: unknown) {
        const action = typeof e === 'object' && e !== null && 'action' in e ? (e as { action: string }).action : undefined;
        if (e !== 'cancel' && e !== 'close' && action !== 'cancel' && action !== 'close') {
            ElMessage.error(t('modsManagement.messages.createFailed', { error: String(e) }));
        }
    }
};

const subGroupDialog = reactive({
    visible: false,
    parentId: '',
    name: '',
    icon: ''
});

const openSubGroupDialog = (parentId: string) => {
    subGroupDialog.visible = true;
    subGroupDialog.parentId = parentId;
    subGroupDialog.name = '';
    subGroupDialog.icon = '';
};

const pickSubGroupIcon = async () => {
    const picked = await open({
        multiple: false,
        filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp'] }]
    });
    if (picked) {
        subGroupDialog.icon = picked;
    }
};

const confirmSubGroup = async () => {
    if (!subGroupDialog.name) {
        ElMessage.warning(t('modsManagement.messages.enterSubcategoryName'));
        return;
    }
    const newGroupPath = subGroupDialog.parentId ? `${subGroupDialog.parentId}/${subGroupDialog.name}` : subGroupDialog.name;
    try {
        await ModManager.createModGroup(selectedGame.value, newGroupPath);
        if (subGroupDialog.icon) {
            try {
                await ModManager.setModGroupIcon(selectedGame.value, newGroupPath, subGroupDialog.icon);
            } catch (e: unknown) {
                ElMessage.warning(t('modsManagement.messages.subcategoryIconSetFailed', { error: String(e) }));
            }
        }
        ElMessage.success(t('modsManagement.messages.subcategoryCreated'));
        subGroupDialog.visible = false;
        await refreshAfterMutation('structure');
    } catch (e: unknown) {
        ElMessage.error(t('modsManagement.messages.createFailed', { error: String(e) }));
    }
};

const toggleGroup = async (group: GroupInfo) => {
    // Check if parent group is disabled when enabling
    if (!group.enabled) {
        const parentId = getGroupParent(group.id);
        if (parentId && parentId !== 'Root' && parentId !== 'All') {
            const parentGroup = groups.value.find(g => g.id === parentId);
            if (parentGroup && !parentGroup.enabled) {
                ElMessage.warning(t('modsManagement.messages.cannotEnableCategoryWithDisabledParent', {
                    category: group.name,
                    parentCategory: parentGroup.name,
                }));
                return;
            }
        }
    }

    try {
        const nextEnabled = !group.enabled;
        const groupId = group.id;
        const oldPhysicalPath = group.path || group.id;
        suppressFsRefresh(2500);
        const newPhysicalPath = await ModManager.toggleGroup(selectedGame.value, oldPhysicalPath, nextEnabled);
        await ModTagStore.remapPrefix(selectedGame.value, oldPhysicalPath, newPhysicalPath);
        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });

        applyGroupToggleLocalState(groupId, oldPhysicalPath, newPhysicalPath, nextEnabled);
        group.enabled = nextEnabled;
        group.path = newPhysicalPath;

        await refreshAfterMutation('structure');

        ElMessage.success(nextEnabled ? t('modsManagement.messages.categoryEnabled') : t('modsManagement.messages.categoryDisabled'));
    } catch (e: unknown) {
        ElMessage.error(t('modsManagement.messages.toggleStatusFailed', { error: String(e) }));
        await refreshAfterMutation('structure');
    }
};

const renameGroup = async (group: GroupInfo) => {
    const oldGroupId = group.id;
    const oldPhysicalPath = group.path || group.id;
    const physicalParts = oldPhysicalPath.split('/');
    const oldPhysicalName = physicalParts[physicalParts.length - 1] || group.name;
    const physicalParent = physicalParts.length > 1 ? physicalParts.slice(0, -1).join('/') : '';
    const { cleanName, disabled, hasUnderscore } = stripDisabledFolderName(oldPhysicalName);
    const currentName = group.name || cleanName || oldPhysicalName;
    const stableParent = getGroupParent(oldGroupId);

    try {
        const result = await ElMessageBox.prompt(t('modsManagement.messages.enterNewCategoryName'), t('modsManagement.dialog.renameCategoryTitle'), {
            confirmButtonText: t('modsManagement.common.confirm'),
            cancelButtonText: t('modsManagement.common.cancel'),
            inputValue: currentName,
            inputPattern: /^[^\\/:*?"<>|]+$/,
            inputErrorMessage: t('modsManagement.messages.nameContainsInvalidCharacters')
        }) as { value: string };
        
        const newName = result.value;
        
        if (newName && newName !== currentName) {
            const trimmedName = newName.trim();
            const newPhysicalName = disabled
                ? hasUnderscore ? `DISABLED_${trimmedName}` : `DISABLED${trimmedName}`
                : trimmedName;
            const newPhysicalPath = physicalParent ? `${physicalParent}/${newPhysicalName}` : newPhysicalName;
            const newGroupId = stableParent && stableParent !== ROOT_PARENT_ID
                ? `${stableParent}/${trimmedName}`
                : trimmedName;

            await ModManager.renameGroup(selectedGame.value, oldPhysicalPath, newPhysicalPath);
            suppressFsRefresh(1400);
            await ModTagStore.remapPrefix(selectedGame.value, oldPhysicalPath, newPhysicalPath);
            await ModTagStore.remapGroupPrefix(selectedGame.value, oldGroupId, newGroupId);
            await ModStateStore.remapPrefix(selectedGame.value, oldPhysicalPath, newPhysicalPath);
            await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
            ElMessage.success(t('modsManagement.messages.categoryRenamed'));
            migrateGroupRenameLocalState(oldGroupId, newGroupId, group.enabled, { oldPhysicalPath, newPhysicalPath });
            group.id = newGroupId;
            group.name = trimmedName;
            group.path = newPhysicalPath;
            await refreshAfterMutation('structure');
        }
    } catch (e: unknown) {
        const action = typeof e === 'object' && e !== null && 'action' in e ? (e as { action: string }).action : undefined;
        if (e !== 'cancel' && action !== 'cancel') {
            ElMessage.error(t('modsManagement.messages.renameFailed', { error: String(e) }));
        }
    }
};

const deleteGroup = async (group: GroupInfo) => {
    try {
        await ElMessageBox.confirm(
            t('modsManagement.messages.deleteCategoryConfirm', { category: group.name }),
            t('modsManagement.dialog.deleteCategoryTitle'),
            {
                confirmButtonText: t('modsManagement.common.delete'),
                cancelButtonText: t('modsManagement.common.cancel'),
                type: 'warning',
            }
        )
        
        const groupId = group.id;
        const physicalPath = group.path || group.id;
        await ModManager.deleteGroup(selectedGame.value, physicalPath);
        suppressFsRefresh(1400);
        await ModTagStore.deletePrefixMappings(selectedGame.value, physicalPath);
        await ModTagStore.deleteGroupMapping(selectedGame.value, groupId);
        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
        
        ElMessage.success(t('modsManagement.messages.categoryDeleted'));
        if (selectedGroup.value === groupId) {
            selectedGroup.value = ROOT_GROUP_ID;
        }
        if (sidebarSelectedGroup.value === groupId) {
            sidebarSelectedGroup.value = ROOT_GROUP_ID;
        }
        await refreshAfterMutation('structure');
    } catch (e: unknown) {
        if (e !== 'cancel') {
             ElMessage.error(t('modsManagement.messages.deleteFailed', { error: String(e) }));
        }
    }
};

const moveModToGroup = async (mod: ModInfo, groupName: string) => {
    try {
        const targetPhysicalPath = resolveGroupPhysicalPath(groupName);
        const nextRelativePath = buildMovedModRelativePath(mod, targetPhysicalPath);
        await ModManager.moveModToGroup(selectedGame.value, mod.relativePath, targetPhysicalPath);
        suppressFsRefresh(1400);
        await ModTagStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
        await ModStateStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
        ElMessage.success(t('modsManagement.messages.movedToGroupSuccess', {
            group: groupName === 'Root' || !groupName ? t('modsManagement.actions.moveToModsRoot') : groupName,
        }));
        await refreshAfterMutation('structure');
        if (groupName === 'Root' || !groupName) {
            await ModManager.openGameModsFolder(selectedGame.value);
        } else {
            await ModManager.openModGroupFolder(selectedGame.value, targetPhysicalPath);
        }
    } catch (e: unknown) {
        const message = String(e);
        const localizedNotFound = t('modManager.messages.modNotFound');
        if (message.includes(localizedNotFound) || message.includes('Mod not found') || message.includes('未找到 Mod')) {
            await refreshAfterMutation('structure');
            ElMessage.warning(t('modsManagement.messages.modCacheRefreshedAfterNotFound'));
            return;
        }
        ElMessage.error(t('modsManagement.messages.moveFailed', { error: String(e) }));
    }
};

const renameMod = async (mod: ModInfo) => {
    try {
        const result = await ElMessageBox.prompt(t('modsManagement.messages.enterNewModName'), t('modsManagement.dialog.renameModTitle'), {
            confirmButtonText: t('modsManagement.common.confirm'),
            cancelButtonText: t('modsManagement.common.cancel'),
            inputValue: mod.name,
            inputPattern: /^[^\\/:*?"<>|]+$/,
            inputErrorMessage: t('modsManagement.messages.nameContainsInvalidCharacters')
        }) as { value: string };
        
        const newName = result.value;

        if (newName && newName !== mod.name) {
            const nextRelativePath = buildRenamedModRelativePath(mod, newName.trim());
            await ModManager.renameMod(selectedGame.value, mod.relativePath, newName.trim());
            suppressFsRefresh(1400);
            await ModTagStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
            await ModStateStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
            await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
            ElMessage.success(t('modsManagement.messages.renamedSuccessfully'));
            await refreshAfterMutation('content');
        }
    } catch {
        // User cancelled
    }
};

const addPreviewImages = async (mod: ModInfo) => {
    try {
        const selected = await open({
            multiple: true,
            filters: [{
                name: 'Images',
                extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'gif']
            }]
        });

        if (selected && Array.isArray(selected) && selected.length > 0) {
            await ModManager.addModPreviewImages(selectedGame.value, mod.relativePath, selected);
            invalidateSubgroupPreviewByGroupId(selectedGame.value, mod.group);
            ElMessage.success(t('modsManagement.messages.addedPreviewImages', { count: selected.length }));
            await refreshAfterMutation('content');
        }
    } catch (err) {
        console.error('Failed to add preview images', err);
        ElMessage.error(t('modsManagement.messages.addPreviewImagesFailed'));
    }
};

const pasteClipboardPreviewImage = async (mod: ModInfo) => {
    try {
        const image = await readImage();
        if (!image) {
            ElMessage.warning(t('modsManagement.messages.noImageOnClipboard'));
            return;
        }

        const pngBytes = await clipboardImageToPngBytes(image);
        const fileName = `preview_clipboard_${Date.now()}.png`;

        await ModManager.addModPreviewImageData(
            selectedGame.value,
            mod.relativePath,
            fileName,
            pngBytes
        );

        invalidateSubgroupPreviewByGroupId(selectedGame.value, mod.group);
        ElMessage.success(t('modsManagement.messages.pastedPreviewImage'));
        await refreshAfterMutation('content');
    } catch (err) {
        console.error('Failed to paste preview image from clipboard', err);
        ElMessage.error(t('modsManagement.messages.pastePreviewImageFailed'));
    }
};

const deleteMod = async (mod: ModInfo) => {
    try {
        await ElMessageBox.confirm(
            t('modsManagement.messages.deleteModConfirm', { mod: mod.name }),
            t('modsManagement.dialog.deleteModTitle'),
            {
                confirmButtonText: t('modsManagement.common.delete'),
                cancelButtonText: t('modsManagement.common.cancel'),
                type: 'warning',
            }
        )
        
        await ModManager.deleteMod(selectedGame.value, mod.relativePath);
        suppressFsRefresh(1400);
        await ModTagStore.deleteModMapping(selectedGame.value, mod.relativePath);
        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
        
        ElMessage.success(t('modsManagement.messages.modDeleted'));
        await refreshAfterMutation('structure');
    } catch (e: unknown) {
        if (e !== 'cancel') {
             ElMessage.error(t('modsManagement.messages.deleteFailed', { error: String(e) }));
        }
    }
};

const loading = ref(false);
const mods = ref<ModInfo[]>([]);
const currentSubGroups = ref<GroupInfo[]>([]);
const availableGroups = ref<GroupInfo[]>([]);
const selectedGame = ref('');
const searchQuery = ref('');
const floating3DPreviewMod = ref<ModInfo>();
const floating3DPreviewOpen = ref(false);
const floating3DPreviewCache = ref<ModInfo[]>([]);
const floating3DPreviewPage = ref<HTMLElement>();

// Hiding the floating page must not destroy its renderer and decoded model.
// Reopening the same Mod can then reuse the complete resident scene.
const closeFloating3DPreview = () => { floating3DPreviewOpen.value = false; };
const dismissFloating3DPreview = () => {
    floating3DPreviewOpen.value = false;
    floating3DPreviewMod.value = undefined;
    // The preview and its zoom view are teleported to <body>. Destroy cached
    // instances when leaving the route so nested overlays and WebGL canvases
    // cannot outlive the Mods page.
    floating3DPreviewCache.value = [];
};
const openFloating3DPreview = (mod: ModInfo) => {
    const cacheKey = mod.path.toLowerCase();
    floating3DPreviewCache.value = [
        ...floating3DPreviewCache.value.filter(item => item.path.toLowerCase() !== cacheKey),
        mod,
    ].slice(-10);
    floating3DPreviewMod.value = mod;
    floating3DPreviewOpen.value = true;
    void nextTick(() => floating3DPreviewPage.value?.focus());
};
const searchMatchedModPaths = ref<Set<string>>(new Set());
const searchLoading = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let searchToken = 0;

// View mode & sort state
type ViewMode = 'grid' | 'list';
const viewMode = ref<ViewMode>('grid');
const sortBy = ref<SortCriterion>('manual');
const sortOrder = ref<SortOrder>('asc');
const sortTagId = ref<string>('');

let subgroupPreviewPreloadTimer: ReturnType<typeof setTimeout> | null = null;

const ROOT_GROUP_ID = 'Root';
const selectedGroup = ref(ROOT_GROUP_ID);
const sidebarSelectedGroup = ref(ROOT_GROUP_ID);
const pendingExternalFileDropGroup = ref<string | null>(null);
const groupTreeRenderKey = ref(0);
const groupNavigationHistory = ref<string[]>([]);
const groupNavigationForwardHistory = ref<string[]>([]);
const isApplyingGroupHistory = ref(false);
const canNavigateGroupBack = computed(() => groupNavigationHistory.value.length > 0);
const canNavigateGroupForward = computed(() => groupNavigationForwardHistory.value.length > 0);
const canNavigateGroupUp = computed(() => {
    const current = selectedGroup.value;
    return !!current && current !== 'All' && current !== ROOT_GROUP_ID;
});

// Breadcrumb segments for the toolbar path navigator
interface BreadcrumbSegment { label: string; groupId: string; isLast: boolean }
const groupBreadcrumbs = computed<BreadcrumbSegment[]>(() => {
    const current = selectedGroup.value;
    if (!current || current === 'All' || current === ROOT_GROUP_ID) {
        return [{ label: t('modsManagement.common.root'), groupId: ROOT_GROUP_ID, isLast: true }];
    }
    const parts = current.split('/').filter(Boolean);
    if (parts.length === 0) {
        return [{ label: t('modsManagement.common.root'), groupId: ROOT_GROUP_ID, isLast: true }];
    }
    const segments: BreadcrumbSegment[] = [
        { label: t('modsManagement.common.root'), groupId: ROOT_GROUP_ID, isLast: false },
    ];
    for (let i = 0; i < parts.length; i++) {
        const groupId = parts.slice(0, i + 1).join('/');
        segments.push({ label: parts[i], groupId, isLast: i === parts.length - 1 });
    }
    return segments;
});

const navigateGroupBreadcrumb = (groupId: string) => {
    if (groupId === selectedGroup.value) return;
    if (groupId === ROOT_GROUP_ID) {
        handleGroupClick({ id: ROOT_GROUP_ID, name: 'Root', path: ROOT_GROUP_ID, enabled: true } as GroupInfo, { ensureExpanded: false, syncSidebar: true });
        return;
    }
    handleGroupClick({ id: groupId, name: groupId.split('/').pop() || groupId, path: groupId, enabled: true } as GroupInfo, { ensureExpanded: false, syncSidebar: true });
};

// ============================================================
// Dialogs composable — context menus, export, reverse preset
// ============================================================
const {
    contextMenu, groupContextMenu,
    closeContextMenu, closeGroupContextMenu,
    showModContextMenu: showModContextMenuBase, showGroupContextMenu,
    archiveExportFormats, exportArchiveDialog,
    openExportArchiveDialog, cancelExportArchive, chooseExportOutputDir, confirmExportArchive,
    modKeyListDialog, closeModKeyList,
} = useModsManagementDialogs({ selectedGame, t });

const inlineKeyBackupState = reactive<Record<string, boolean>>({});
const showModContextMenu = (event: MouseEvent, mod: ModInfo) => {
    showModContextMenuBase(event, mod);
    inlineKeyBackupState[mod.id] = false;
    if (!selectedGame.value) return;
    void hasInlineModKeyBackup(selectedGame.value, mod.relativePath)
        .then(hasBackup => { inlineKeyBackupState[mod.id] = hasBackup; })
        .catch(() => { inlineKeyBackupState[mod.id] = false; });
};

let selectedGroupTimer: ReturnType<typeof setTimeout> | null = null;
const persistSelectedGroupState = () => {
    if (selectedGroupTimer) clearTimeout(selectedGroupTimer);
    selectedGroupTimer = setTimeout(() => persistSelectedGroupStateStorage(selectedGroupState.value), 200);
};
const loadSubgroupPreviewCache = () => loadSubgroupPreviewCacheStorage();

const selectedGroupState = ref<Record<string, string>>(loadSelectedGroupStateStorage());

// ============================================================
// Preset state
// ============================================================
const presetPopoverVisible = ref(false);
const presetPopoverX = ref(0);
const presetPopoverY = ref(0);
const presetTargetMod = ref<ModInfo | null>(null);
const presetDataMap = ref<Record<string, ModPresetsFile>>({});

const getPresetData = (mod: ModInfo): ModPresetsFile => {
    return presetDataMap.value[mod.id] || { version: 1, presets: [] };
};

const getActivePresetName = (mod: ModInfo): string | null => {
    const data = getPresetData(mod);
    const active = data.presets.find(p => p.active);
    return active ? active.name : null;
};

const loadModPresets = async (mod: ModInfo) => {
    try {
        const data = await ModPresetStore.loadPresets(mod.path);
        presetDataMap.value = { ...presetDataMap.value, [mod.id]: data };
    } catch { /* ignore */ }
};

const openPresetPopover = (mod: ModInfo, e: MouseEvent) => {
    closeContextMenu();
    closeGroupContextMenu();
    presetTargetMod.value = mod;
    presetPopoverX.value = e.clientX;
    presetPopoverY.value = e.clientY;
    presetPopoverVisible.value = true;
    loadModPresets(mod);
};

const closePresetPopover = () => {
    presetPopoverVisible.value = false;
    presetTargetMod.value = null;
};

const onApplyPreset = async (presetId: string) => {
    const mod = presetTargetMod.value;
    if (!mod || !selectedGame.value) return;
    try {
        await ModPresetStore.applyPreset(selectedGame.value, mod.relativePath, mod.path, presetId);
        triggerModPulse(mod.id);
        await loadModPresets(mod);
    } catch (e: unknown) {
        ElMessage.error(String(e));
    }
    closePresetPopover();
};

const onSaveCurrentAsPreset = async (name: string) => {
    const mod = presetTargetMod.value;
    if (!mod || !selectedGame.value) return;
    try {
        const preset = await ModPresetStore.captureAsPreset(selectedGame.value, mod.relativePath, mod.path, name);
        if (!preset) {
            // captureAsPreset returns null when the mod has no active key
            // switches recorded in d3dx_user.ini — meaning there is nothing
            // to save as a preset yet.
            ElMessage.warning(t('modsManagement.messages.noKeyStateToSave'));
            return;
        }
        triggerModPulse(mod.id);
        await loadModPresets(mod);
        ElMessage.success(t('modsManagement.messages.presetSaved', { name }));
    } catch (e: unknown) {
        ElMessage.error(String(e));
    }
};

const onDeletePreset = async (presetId: string) => {
    const mod = presetTargetMod.value;
    if (!mod) return;
    try {
        await ModPresetStore.deletePreset(mod.path, presetId);
        await loadModPresets(mod);
    } catch (e: unknown) {
        ElMessage.error(String(e));
    }
};

const onRenamePreset = async (presetId: string, newName: string) => {
    const mod = presetTargetMod.value;
    if (!mod) return;
    try {
        await ModPresetStore.renamePreset(mod.path, presetId, newName);
        await loadModPresets(mod);
    } catch (e: unknown) {
        ElMessage.error(String(e));
    }
};

const onResetPresetState = async () => {
    const mod = presetTargetMod.value;
    if (!mod || !selectedGame.value) return;
    try {
        await ModPresetStore.resetPresetState(selectedGame.value, mod.relativePath, mod.path);
        triggerModPulse(mod.id);
        await loadModPresets(mod);
        ElMessage.success(t('modsManagement.messages.presetStateReset'));
    } catch (e: unknown) {
        ElMessage.error(String(e));
    }
};

const manualOrders = ref<Record<string, Record<string, string[]>>>(loadManualOrdersStorage());
const draggingOrderId = ref<string | null>(null);
const dragOverId = ref<string | null>(null);
const manualSortState = reactive({
    active: false,
    startX: 0,
    startY: 0,
    hasMoved: false,
    mod: null as ModInfo | null,
});
let manualSortGroupHover: HTMLElement | null = null;

// Group manual order state
const groupOrders = ref<Record<string, Record<string, string[]>>>(loadGroupOrdersStorage());
const groupDragState = reactive({
    active: false,
    startX: 0,
    startY: 0,
    hasMoved: false,
    sourceId: '' as string,
    targetId: null as string | null,
    sourceParent: '' as string,
});
const groupHoverId = ref<string | null>(null);
const expandedState = ref<Record<string, string[]>>(loadExpandedStateStorage());
const expandedKeys = ref<string[]>([]);
const canCollapseSelectedGroup = computed(() => {
    const currentGroup = String(sidebarSelectedGroup.value || '');
    if (!currentGroup || currentGroup === 'All' || currentGroup === 'Root') {
        return false;
    }
    return expandedKeys.value.some((key) => isSameOrChildPath(key, currentGroup));
});
const groupListRef = ref<HTMLElement | null>(null);
const groupTreeRef = ref();
const switchingGroup = ref(false);
const loadedModsCount = ref(0);
const totalModsCount = ref(0);
const MOD_RENDER_BATCH_SIZE = 24;
const modHydrationDepth = ref(0);
const isHydratingMods = computed(() => modHydrationDepth.value > 0);
const orderPersistencePauseDepth = ref(0);
const isOrderPersistencePaused = computed(() => orderPersistencePauseDepth.value > 0);
let groupLoadToken = 0;
let startWatchingToken = 0;
let refreshModsToken = 0;
let needsReactivationRescan = false;
let reactivationRescanRunning = false;
const isModsPageActive = ref(true);
let watchedGameName = '';

const makeScanSignal = (getToken: () => number, token: number) => ({
    isCancelled: () => getToken() !== token,
});

const resolvePreferredGroup = async (gameName: string, preferredGroup?: string) => {
    const normalizedGroup = normalizeGroupId(String(preferredGroup || '').trim());
    if (!normalizedGroup || normalizedGroup === 'All' || normalizedGroup === ROOT_GROUP_ID) {
        return ROOT_GROUP_ID;
    }

    try {
        return await ModManager.groupExists(gameName, normalizedGroup) ? normalizedGroup : ROOT_GROUP_ID;
    } catch (error) {
        console.warn('Failed to resolve preferred group, fallback to Root:', error);
        return ROOT_GROUP_ID;
    }
};

const withOrderPersistencePaused = async <T>(task: () => Promise<T>): Promise<T> => {
    orderPersistencePauseDepth.value += 1;
    try {
        return await task();
    } finally {
        orderPersistencePauseDepth.value = Math.max(0, orderPersistencePauseDepth.value - 1);
    }
};

// Deferred persistence: avoids blocking the main thread with synchronous
// JSON.stringify + localStorage.setItem during Vue reactive updates.
let manualOrdersTimer: ReturnType<typeof setTimeout> | null = null;
let groupOrdersTimer: ReturnType<typeof setTimeout> | null = null;
let expandedStateTimer: ReturnType<typeof setTimeout> | null = null;

const persistManualOrders = () => {
    if (manualOrdersTimer) clearTimeout(manualOrdersTimer);
    manualOrdersTimer = setTimeout(() => persistManualOrdersStorage(manualOrders.value), 200);
};

const persistGroupOrders = () => {
    if (groupOrdersTimer) clearTimeout(groupOrdersTimer);
    groupOrdersTimer = setTimeout(() => {
        groupOrdersTimer = null;
        persistGroupOrdersStorage(groupOrders.value);
    }, 200);
};

const flushGroupOrders = () => {
    if (groupOrdersTimer) {
        clearTimeout(groupOrdersTimer);
        groupOrdersTimer = null;
    }
    persistGroupOrdersStorage(groupOrders.value);
};

const persistExpandedState = () => {
    if (expandedStateTimer) clearTimeout(expandedStateTimer);
    expandedStateTimer = setTimeout(() => persistExpandedStateStorage(expandedState.value), 200);
};

const getGroupParent = (id: string) => {
    return getGroupParentUtil(id);
};

const isSameOrChildPath = (value: string, prefix: string) => {
    return isSameOrChildPathUtil(value, prefix);
};

const replacePathPrefix = (value: string, oldPrefix: string, newPrefix: string) => {
    return replacePathPrefixUtil(value, oldPrefix, newPrefix);
};

const stripDisabledFolderName = (folderName: string) => {
    return stripDisabledFolderNameUtil(folderName);
};

const normalizeGroupId = (groupId: string) => {
    return normalizeGroupIdUtil(groupId);
};

const replaceFsPathSegment = (value: string, oldRelativePath: string, newRelativePath: string) => {
    return replaceFsPathSegmentUtil(value, oldRelativePath, newRelativePath);
};

const resolveGroupPhysicalPath = (groupId: string) => {
    if (!groupId || groupId === 'Root' || groupId === 'All' || groupId === MODS_TREE_ROOT_ID) {
        return 'Root';
    }

    const group = availableGroups.value.find((item) => item.id === groupId)
        || currentSubGroups.value.find((item) => item.id === groupId)
        || groups.value.find((item) => item.id === groupId);
    return group?.path || groupId;
};

const renameKeyedRecord = <T,>(source: Record<string, T>, oldPrefix: string, newPrefix: string) => {
    return renameKeyedRecordUtil(source, oldPrefix, newPrefix);
};

const migrateExpandedStatePrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game) return;

    const renameExpandedEntries = (entries: string[]) => Array.from(new Set(entries.map((entry) => replacePathPrefix(entry, oldPrefix, newPrefix))));

    expandedKeys.value = renameExpandedEntries(expandedKeys.value);
    expandedState.value[game] = renameExpandedEntries(expandedState.value[game] || []);
    persistExpandedState();
};

const migrateGroupOrderPrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game || !groupOrders.value[game]) return;

    const next: Record<string, string[]> = {};
    Object.entries(groupOrders.value[game]).forEach(([parentId, children]) => {
        const nextParentId = replacePathPrefix(parentId, oldPrefix, newPrefix);
        const nextChildren = Array.from(new Set(children.map((childId) => replacePathPrefix(childId, oldPrefix, newPrefix))));
        next[nextParentId] = nextChildren;
    });
    groupOrders.value[game] = next;
    flushGroupOrders();
};

const migrateManualOrderPrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game || !manualOrders.value[game]) return;

    const next: Record<string, string[]> = {};
    Object.entries(manualOrders.value[game]).forEach(([groupId, order]) => {
        const nextGroupId = replacePathPrefix(groupId, oldPrefix, newPrefix);
        next[nextGroupId] = Array.from(new Set(order.map((id) => replacePathPrefix(id, oldPrefix, newPrefix))));
    });
    manualOrders.value[game] = next;
    persistManualOrders();
};

const migrateSelectedGroupPrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game) return;

    if (isSameOrChildPath(selectedGroup.value, oldPrefix)) {
        selectedGroup.value = replacePathPrefix(selectedGroup.value, oldPrefix, newPrefix);
    }

    if (isSameOrChildPath(sidebarSelectedGroup.value, oldPrefix)) {
        sidebarSelectedGroup.value = replacePathPrefix(sidebarSelectedGroup.value, oldPrefix, newPrefix);
    }

    const remembered = selectedGroupState.value[game];
    if (remembered && isSameOrChildPath(remembered, oldPrefix)) {
        selectedGroupState.value[game] = replacePathPrefix(remembered, oldPrefix, newPrefix);
        persistSelectedGroupState();
    }
};

const migrateSubgroupPreviewCachePrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game) return;

    const scopedPrefix = `${game}::`;
    const nextMap: Record<string, string[]> = {};
    Object.entries(subgroupPreviewMap.value).forEach(([key, images]) => {
        if (!key.startsWith(scopedPrefix)) {
            nextMap[key] = images;
            return;
        }

        const groupId = key.slice(scopedPrefix.length);
        const nextGroupId = replacePathPrefix(groupId, oldPrefix, newPrefix);
        nextMap[`${scopedPrefix}${nextGroupId}`] = images;
    });
    subgroupPreviewMap.value = nextMap;

    const nextIndices = renameKeyedRecord(subgroupPreviewIndices, `${scopedPrefix}${oldPrefix}`, `${scopedPrefix}${newPrefix}`);
    Object.keys(subgroupPreviewIndices).forEach((key) => delete subgroupPreviewIndices[key]);
    Object.entries(nextIndices).forEach(([key, value]) => {
        subgroupPreviewIndices[key] = value;
    });
    schedulePersistSubgroupPreviewCache();
};

const migrateGroupRenameLocalState = (
    oldGroupId: string,
    newGroupId: string,
    enabled: boolean,
    options?: { oldPhysicalPath?: string; newPhysicalPath?: string },
) => {
    const oldPhysicalPath = options?.oldPhysicalPath || oldGroupId;
    const newPhysicalPath = options?.newPhysicalPath || newGroupId;
    const updateGroup = (group: GroupInfo) => {
        const affectsIdentity = isSameOrChildPath(group.id, oldGroupId);
        const affectsPath = isSameOrChildPath(group.path, oldPhysicalPath);
        if (!affectsIdentity && !affectsPath) return group;

        const nextId = affectsIdentity ? replacePathPrefix(group.id, oldGroupId, newGroupId) : group.id;
        return {
            ...group,
            id: nextId,
            path: affectsPath ? replacePathPrefix(group.path, oldPhysicalPath, newPhysicalPath) : group.path,
            enabled: group.id === oldGroupId ? enabled : group.enabled,
        };
    };

    availableGroups.value = availableGroups.value.map(updateGroup);
    currentSubGroups.value = currentSubGroups.value.map(updateGroup);

    mods.value = mods.value.map((mod) => {
        const affectsGroup = isSameOrChildPath(mod.group, oldGroupId);
        const affectsRelativePath = isSameOrChildPath(mod.relativePath, oldPhysicalPath);
        const affectsId = isSameOrChildPath(mod.id, oldPhysicalPath);
        if (!affectsGroup && !affectsRelativePath && !affectsId) {
            return mod;
        }

        const nextRelativePath = affectsRelativePath ? replacePathPrefix(mod.relativePath, oldPhysicalPath, newPhysicalPath) : mod.relativePath;
        return {
            ...mod,
            id: affectsId ? replacePathPrefix(mod.id, oldPhysicalPath, newPhysicalPath) : mod.id,
            relativePath: nextRelativePath,
            group: affectsGroup ? replacePathPrefix(mod.group, oldGroupId, newGroupId) : mod.group,
            path: replaceFsPathSegment(mod.path, mod.relativePath, nextRelativePath),
        };
    });

    migrateSelectedGroupPrefix(oldGroupId, newGroupId);
    migrateExpandedStatePrefix(oldGroupId, newGroupId);
    migrateGroupOrderPrefix(oldGroupId, newGroupId);
    migrateManualOrderPrefix(oldGroupId, newGroupId);
    migrateManualOrderItemPrefix(oldPhysicalPath, newPhysicalPath);
    remapModKeyStatePrefix(oldPhysicalPath, newPhysicalPath);
    migrateSubgroupPreviewCachePrefix(oldGroupId, newGroupId);
};

const remapModKeyStatePrefix = (oldPrefix: string, newPrefix: string) => {
    const nextLists = renameKeyedRecord(modKeyLists, oldPrefix, newPrefix);
    Object.keys(modKeyLists).forEach((key) => delete modKeyLists[key]);
    Object.entries(nextLists).forEach(([key, value]) => {
        modKeyLists[key] = value;
    });

    const nextLoading = renameKeyedRecord(modKeyLoadingState, oldPrefix, newPrefix);
    Object.keys(modKeyLoadingState).forEach((key) => delete modKeyLoadingState[key]);
    Object.entries(nextLoading).forEach(([key, value]) => {
        modKeyLoadingState[key] = value;
    });

    const nextErrors = renameKeyedRecord(modKeyErrorState, oldPrefix, newPrefix);
    Object.keys(modKeyErrorState).forEach((key) => delete modKeyErrorState[key]);
    Object.entries(nextErrors).forEach(([key, value]) => {
        modKeyErrorState[key] = value;
    });

    if (hoveredKeyModId.value && isSameOrChildPath(hoveredKeyModId.value, oldPrefix)) {
        hoveredKeyModId.value = replacePathPrefix(hoveredKeyModId.value, oldPrefix, newPrefix);
    }
    if (modKeyEditorDialog.modId && isSameOrChildPath(modKeyEditorDialog.modId, oldPrefix)) {
        modKeyEditorDialog.modId = replacePathPrefix(modKeyEditorDialog.modId, oldPrefix, newPrefix);
        modKeyEditorDialog.modRelativePath = replacePathPrefix(modKeyEditorDialog.modRelativePath, oldPrefix, newPrefix);
        modKeyEditorDialog.modPath = replaceFsPathSegment(modKeyEditorDialog.modPath, oldPrefix, newPrefix);
    }
};

const migrateManualOrderItemPrefix = (oldPrefix: string, newPrefix: string) => {
    const game = selectedGame.value;
    if (!game || !manualOrders.value[game]) return;

    let changed = false;
    const nextForGame: Record<string, string[]> = {};
    Object.entries(manualOrders.value[game]).forEach(([groupId, order]) => {
        const nextOrder = order.map((id) => replacePathPrefix(id, oldPrefix, newPrefix));
        if (nextOrder.some((id, index) => id !== order[index])) {
            changed = true;
        }
        nextForGame[groupId] = nextOrder;
    });

    if (!changed) return;
    manualOrders.value[game] = nextForGame;
    persistManualOrders();
};

const updateGroupsWithPhysicalPathPrefix = (sourceGroups: GroupInfo[], groupId: string, oldPhysicalPath: string, newPhysicalPath: string, enabled: boolean) => {
    return sourceGroups.map((item) => {
        const affectsIdentity = isSameOrChildPath(item.id, groupId);
        const affectsPath = isSameOrChildPath(item.path, oldPhysicalPath);
        if (!affectsIdentity && !affectsPath) return item;

        return {
            ...item,
            path: affectsPath ? replacePathPrefix(item.path, oldPhysicalPath, newPhysicalPath) : item.path,
            enabled: item.id === groupId ? enabled : item.enabled,
        };
    });
};

const applyGroupToggleLocalState = (groupId: string, oldPhysicalPath: string, newPhysicalPath: string, enabled: boolean) => {
    availableGroups.value = updateGroupsWithPhysicalPathPrefix(availableGroups.value, groupId, oldPhysicalPath, newPhysicalPath, enabled);
    currentSubGroups.value = updateGroupsWithPhysicalPathPrefix(currentSubGroups.value, groupId, oldPhysicalPath, newPhysicalPath, enabled);

    mods.value = mods.value.map((mod) => {
        const affectsRelativePath = isSameOrChildPath(mod.relativePath, oldPhysicalPath);
        const affectsId = isSameOrChildPath(mod.id, oldPhysicalPath);
        if (!affectsRelativePath && !affectsId) return mod;

        const nextRelativePath = affectsRelativePath ? replacePathPrefix(mod.relativePath, oldPhysicalPath, newPhysicalPath) : mod.relativePath;
        return {
            ...mod,
            id: affectsId ? replacePathPrefix(mod.id, oldPhysicalPath, newPhysicalPath) : mod.id,
            relativePath: nextRelativePath,
            group: getModGroupFromRelativePath(nextRelativePath),
            path: replaceFsPathSegment(mod.path, mod.relativePath, nextRelativePath),
        };
    });

    remapModKeyStatePrefix(oldPhysicalPath, newPhysicalPath);
    migrateManualOrderItemPrefix(oldPhysicalPath, newPhysicalPath);
};

const migrateEnabledParentGroupsLocalState = async (enabledGroups: DisabledParentGroupInfo[]) => {
    if (enabledGroups.length === 0 || !selectedGame.value) return;

    const game = selectedGame.value;

    enabledGroups.forEach((group) => {
        applyGroupToggleLocalState(group.enabledPath, group.disabledPath, group.enabledPath, true);
        invalidateSubgroupPreviewByGroupId(game, group.disabledPath);
        invalidateSubgroupPreviewByGroupId(game, group.enabledPath);
    });
};

const updateModToggleLocalState = (mod: ModInfo, nextRelativePath: string, enabled: boolean) => {
    const previousId = mod.id;
    const previousRelativePath = mod.relativePath;
    const previousPath = mod.path;
    const nextPath = replaceFsPathSegment(previousPath, previousRelativePath, nextRelativePath);
    const nextGroup = getModGroupFromRelativePath(nextRelativePath);

    if (previousId !== nextRelativePath) {
        if (Object.prototype.hasOwnProperty.call(modKeyLists, previousId)) {
            modKeyLists[nextRelativePath] = modKeyLists[previousId];
            delete modKeyLists[previousId];
        }
        if (Object.prototype.hasOwnProperty.call(modKeyLoadingState, previousId)) {
            modKeyLoadingState[nextRelativePath] = modKeyLoadingState[previousId];
            delete modKeyLoadingState[previousId];
        }
        if (Object.prototype.hasOwnProperty.call(modKeyErrorState, previousId)) {
            modKeyErrorState[nextRelativePath] = modKeyErrorState[previousId];
            delete modKeyErrorState[previousId];
        }
        if (hoveredKeyModId.value === previousId) {
            hoveredKeyModId.value = nextRelativePath;
        }
        if (modKeyEditorDialog.modId === previousId) {
            modKeyEditorDialog.modId = nextRelativePath;
            modKeyEditorDialog.modRelativePath = nextRelativePath;
            modKeyEditorDialog.modPath = nextPath;
        }
    }

    mod.enabled = enabled;
    mod.id = nextRelativePath;
    mod.relativePath = nextRelativePath;
    mod.group = nextGroup;
    mod.path = nextPath;
    mod.previewImages = mod.previewImages || [];

    const updatedMod = { ...mod, group: nextGroup, previewImages: mod.previewImages };
    const found = mods.value.some((item) =>
        item === mod || item.id === previousId || item.relativePath === previousRelativePath || item.path === previousPath
    );

    mods.value = found
        ? mods.value.map((item) => {
            if (item === mod || item.id === previousId || item.relativePath === previousRelativePath || item.path === previousPath) {
                return updatedMod;
            }
            return item;
        })
        : [updatedMod, ...mods.value];
};

const getGroupAncestors = (id: string) => {
    return getGroupAncestorsUtil(id);
};

const expandGroupPath = async (groupId: string) => {
    const game = selectedGame.value;
    if (!game || !groupId || groupId === 'All') return;

    const ancestors = getGroupAncestors(groupId);
    const keysToExpand = groupId === 'Root'
        ? []
        : [...ancestors, groupId];

    const set = new Set(expandedKeys.value);
    let changed = false;
    keysToExpand.forEach((id) => {
        if (!set.has(id)) {
            set.add(id);
            changed = true;
        }
    });

    if (!changed) return;

    expandedKeys.value = Array.from(set);
    expandedState.value[game] = [...expandedKeys.value];
    persistExpandedState();
    await nextTick();
    applyExpandedToTree();
};

const sanitizeGroupOrder = (game: string, parentId: string, childrenIds: string[]) => {
    if (!groupOrders.value[game]) groupOrders.value[game] = {};
    const existing = groupOrders.value[game][parentId] || [];

    // When order persistence is paused (e.g. during a full refresh),
    // a temporarily empty childrenIds list means the group tree is
    // being rebuilt — don't wipe the in-memory order or it will be
    // reconstructed alphabetically when the tree refills.
    if (isOrderPersistencePaused.value && childrenIds.length === 0) {
        return [...existing];
    }

    const { order: next, changed } = mergeGroupOrder(existing, childrenIds, {
        preserveUnknown: isOrderPersistencePaused.value,
    });
    if (!changed) return next;
    groupOrders.value[game][parentId] = next;
    if (!isOrderPersistencePaused.value) {
        persistGroupOrders();
    }
    return next;
};

const sortGroupsByOrder = (game: string, parentId: string, sourceGroups: GroupInfo[]) => {
    const ids = sourceGroups.map(g => g.id);
    const orderList = sanitizeGroupOrder(game, parentId, ids);
    const idxMap = new Map(orderList.map((id, idx) => [id, idx]));
    return [...sourceGroups].sort((a, b) => {
        const ia = idxMap.get(a.id) ?? Number.MAX_SAFE_INTEGER;
        const ib = idxMap.get(b.id) ?? Number.MAX_SAFE_INTEGER;
        if (ia !== ib) return ia - ib;
        return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
    });
};

const sanitizeExpanded = (game: string) => {
    const current = expandedState.value[game] || [];
    const deduped = Array.from(new Set(current)).filter((id) => id !== MODS_TREE_ROOT_ID && id !== 'Root');
    if (!expandedState.value[game]) {
        expandedState.value[game] = deduped;
        persistExpandedState();
        return [...deduped];
    }

    const changed = deduped.length !== current.length;
    if (changed) {
        expandedState.value[game] = deduped;
        persistExpandedState();
    }
    return [...(expandedState.value[game] || [])];
};

interface ModReconcileOptions {
    removeMissing?: boolean;
    refreshAnalysis?: boolean;
}

const appendModsIncrementally = async (
    incomingMods: ModInfo[],
    token: number,
    options: ModReconcileOptions = {},
) => {
    const removeMissing = options.removeMissing !== false;
    const refreshAnalysis = options.refreshAnalysis !== false;
    modHydrationDepth.value += 1;
    try {
        totalModsCount.value = incomingMods.length;
        if (refreshAnalysis) clearModAnalysisSubscriptions();

        const existingByIdentity = new Map(
            mods.value.map((mod) => [normalizeModIdentity(mod.relativePath || mod.id), mod]),
        );
        const retained: ModInfo[] = [];
        const additions: ModInfo[] = [];
        const incomingIdentities = new Set<string>();

        for (const fresh of incomingMods) {
            const identity = normalizeModIdentity(fresh.relativePath || fresh.id);
            if (!identity || incomingIdentities.has(identity)) continue;
            incomingIdentities.add(identity);
            const existing = existingByIdentity.get(identity);
            if (existing) {
                Object.assign(existing, fresh);
                retained.push(existing);
            } else {
                additions.push(fresh);
            }
        }

        if (removeMissing) {
            // Keep existing item references for Vue's keyed FLIP transition, so
            // only vanished folders leave instead of rebuilding the whole grid.
            mods.value = retained;
            loadedModsCount.value = retained.length;
            await nextTick();
        }

        for (let i = 0; i < additions.length; i += MOD_RENDER_BATCH_SIZE) {
            if (token !== groupLoadToken) return;
            const batch = additions.slice(i, i + MOD_RENDER_BATCH_SIZE);
            mods.value = [...mods.value, ...batch];
            loadedModsCount.value = mods.value.length;
            if (i + MOD_RENDER_BATCH_SIZE < additions.length) {
                await new Promise((resolve) => requestAnimationFrame(resolve));
            }
        }

        if (!removeMissing) {
            loadedModsCount.value = mods.value.length;
        }
        if (refreshAnalysis && incomingMods.length > 0) {
            queueVisibleModAnalysis(incomingMods);
        }
    } finally {
        modHydrationDepth.value = Math.max(0, modHydrationDepth.value - 1);
    }
};

const reconcileCurrentSubGroups = (incomingGroups: GroupInfo[], removeMissing = true) => {
    const existingById = new Map(currentSubGroups.value.map((group) => [group.id, group]));
    const next: GroupInfo[] = [];
    const additions: GroupInfo[] = [];
    const seen = new Set<string>();

    for (const fresh of incomingGroups) {
        if (!fresh.id || seen.has(fresh.id)) continue;
        seen.add(fresh.id);
        const existing = existingById.get(fresh.id);
        if (existing) {
            Object.assign(existing, fresh);
            next.push(existing);
        } else {
            additions.push(fresh);
        }
    }

    currentSubGroups.value = removeMissing
        ? [...next, ...additions]
        : [...currentSubGroups.value, ...additions];
};

const hasVisibleContent = () => mods.value.length > 0 || currentSubGroups.value.length > 0 || availableGroups.value.length > 0;

const applyCachedSnapshot = async (gameName: string) => {
    const selected = selectedGroup.value && selectedGroup.value !== 'All' ? selectedGroup.value : 'Root';
    const rootCached = ModManager.getCachedGroup(gameName, 'Root');
    if (rootCached) {
        updateAvailableGroups(rootCached.groups);
    }

    const targetCached = selected === 'Root'
        ? rootCached
        : ModManager.getCachedGroup(gameName, selected);

    if (!targetCached) {
        return false;
    }

    reconcileCurrentSubGroups(targetCached.groups);
    const token = ++groupLoadToken;
    await appendModsIncrementally(targetCached.mods, token);
    return true;
};

const applyExpandedToTree = async () => {
    await nextTick();
    try {
        const tree = groupTreeRef.value as {
            getNode?: (key: string) => ElTreeNode | null;
            store?: { root?: ElTreeNode & { expanded?: boolean; expand?: () => void }; getNode?: (key: string) => ElTreeNode | null };
        } | null;
        const root = tree?.store?.root;
        if (root && !root.expanded && typeof root.expand === 'function') {
            root.expand();
            await nextTick();
        }

        for (const key of expandedKeys.value) {
            const node = tree?.getNode?.(key) || tree?.store?.getNode?.(key);
            if (node && !node.expanded && typeof node.expand === 'function') {
                node.expand();
                await nextTick();
            }
        }
    } catch (e) {
        console.warn('Failed to apply expanded keys', e);
    }
};

const scrollGroupIntoView = async (groupId: string) => {
    if (!groupId) return;
    await nextTick();
    const container = groupListRef.value;
    if (!container) return;
    const target = Array.from(container.querySelectorAll<HTMLElement>('[data-group-id]')).find(
        (element) => element.dataset.groupId === groupId,
    );
    target?.scrollIntoView({ block: 'center', inline: 'nearest' });
};

// const applyGroupOrder = (parentId: string, nodes: any[]) => {
//     const game = selectedGame.value;
//     if (!game) return nodes;
//     const ids = nodes.map(n => n.id);
//     const orderList = sanitizeGroupOrder(game, parentId, ids);
//     const idxMap = new Map(orderList.map((id, idx) => [id, idx]));
//     return [...nodes].sort((a, b) => {
//         const ia = idxMap.get(a.id) ?? Number.MAX_SAFE_INTEGER;
//         const ib = idxMap.get(b.id) ?? Number.MAX_SAFE_INTEGER;
//         return ia - ib;
//     });
// };

const getOrderContext = () => {
    if (!selectedGame.value) return null;
    return { game: selectedGame.value, group: selectedGroup.value || 'All' };
};

const getModsByGroup = (group: string) => {
    if (group === 'All') return mods.value;
    if (group === 'Root') return mods.value.filter(m => m.group === 'Root');
    return mods.value.filter(m => m.group === group);
};

const compareDefault = compareModsByDefault;

const normalizeManualOrderId = (id: string) => {
    const normalized = String(id || '').replace(/\\/g, '/');
    if (!normalized) return '';
    const parts = normalized.split('/');
    const tail = parts.pop() || '';
    const upper = tail.toUpperCase();
    let cleanTail = tail;
    if (upper.startsWith('DISABLED_')) {
        cleanTail = tail.substring(9);
    } else if (upper.startsWith('DISABLED')) {
        cleanTail = tail.substring(8);
    }
    return [...parts, cleanTail].filter(Boolean).join('/');
};

const getManualOrderId = (mod: ModInfo) => normalizeManualOrderId(mod.id);

const sanitizeOrderForContext = (game: string, group: string) => {
    const groupMods = getModsByGroup(group);
    const existing = manualOrders.value[game]?.[group] || [];
    const validIds = new Set(groupMods.map(getManualOrderId));
    const filteredExisting = Array.from(new Set(existing.map(normalizeManualOrderId)))
        .filter(id => validIds.has(id));
    const existingSet = new Set(filteredExisting);
    const missing = groupMods
        .filter(m => !existingSet.has(getManualOrderId(m)))
        .sort(compareDefault)
        .map(getManualOrderId);
    const next = [...filteredExisting, ...missing];

    if (isOrderPersistencePaused.value) {
        return next;
    }

    if (groupMods.length === 0 && existing.length > 0 && (loading.value || switchingGroup.value || isHydratingMods.value)) {
        return existing;
    }

    if (!manualOrders.value[game]) manualOrders.value[game] = {};
    const prev = manualOrders.value[game][group] || [];
    const changed = prev.length !== next.length || prev.some((id, idx) => id !== next[idx]);
    if (changed) {
        manualOrders.value[game][group] = next;
        persistManualOrders();
    }
    return next;
};

const getCurrentOrderList = () => {
    const ctx = getOrderContext();
    if (!ctx) return [] as string[];
    return sanitizeOrderForContext(ctx.game, ctx.group);
};

const applyManualReorder = (dragId: string, targetId: string) => {
    const ctx = getOrderContext();
    if (!ctx) return;
    sanitizeOrderForContext(ctx.game, ctx.group);
    const order = manualOrders.value[ctx.game][ctx.group] || [];
    const normalizedDragId = normalizeManualOrderId(dragId);
    const normalizedTargetId = normalizeManualOrderId(targetId);
    const from = order.indexOf(normalizedDragId);
    const to = order.indexOf(normalizedTargetId);
    if (from === -1 || to === -1) return;
    const next = [...order];
    next.splice(from, 1);
    next.splice(to, 0, normalizedDragId);
    manualOrders.value[ctx.game][ctx.group] = next;
    persistManualOrders();
};

const {
    showInstallDialog,
    installForm,
    installPreview,
    isInstalling,
    installProgress,
    handleFileDrop,
    batchInstallFromPaths,
    pickInstallArchive,
    pickInstallFolder,
    cancelInstall,
    confirmInstall,
} = useModsManagementInstall({
    selectedGame,
    selectedGroup,
    loading,
    t,
    pendingDropGroup: pendingExternalFileDropGroup,
    reconcileInstalledMods: () => refreshAfterMutation('structure'),
});

// 主页(/)拖入后跳转到 /mods 的交接兜底:无论事件与路由谁先到达,
// 待安装路径只在最终到达模组管理页时被消费一次。
const drainPendingModInstallDrop = async () => {
    const handedOff = takePendingModInstallDrop();
    if (handedOff.length === 0) return;
    if (handedOff.length === 1) {
        await handleFileDrop(handedOff[0]);
    } else {
        await batchInstallFromPaths(handedOff);
    }
};

watch(
    () => router.currentRoute.value.path,
    (path) => {
        if (path === '/mods') void drainPendingModInstallDrop();
    },
);

// Native DnD (kept for completeness) + Manual fallback
const draggingMod = ref<ModInfo | null>(null);

// Global debug listeners (removed on unmount)
let globalDragOverLogger: ((e: DragEvent) => void) | null = null;
let globalDragEnterLogger: ((e: DragEvent) => void) | null = null;

const onCardMouseDownWrapper = (e: MouseEvent, mod: ModInfo) => {
    onManualSortMouseDown(e, mod);
};

const onDragEnter = (e: DragEvent) => {
    debugLog('DragEnter', e.currentTarget);
    const target = (e.currentTarget as HTMLElement);
    target.classList.add('drag-over');
};

const onDragOver = (e: DragEvent) => {
    e.preventDefault(); // Necessary to allow dropping
    
    if (e.dataTransfer) {
        e.dataTransfer.dropEffect = 'move';
    }
    
    const target = (e.currentTarget as HTMLElement);
    if (!target.classList.contains('drag-over')) {
        target.classList.add('drag-over');
    }
};

const onDragLeave = (e: DragEvent) => {
    const target = (e.currentTarget as HTMLElement);
    // Fix: Only remove class if we are actually leaving the element, 
    // not just entering a child element (like the text span or icon)
    const related = e.relatedTarget as Node | null;
    if (target.contains(related)) {
        return;
    }
    target.classList.remove('drag-over');
};

const onDrop = async (e: DragEvent, targetGroupId: string) => {
    e.preventDefault();
    const target = (e.currentTarget as HTMLElement);
    target.classList.remove('drag-over');
    const normalizedTargetGroupId = targetGroupId === MODS_TREE_ROOT_ID ? 'Root' : targetGroupId;

    const rawData = e.dataTransfer?.getData('text/plain');
    const mod = draggingMod.value;

    debugLog('Drop', { targetGroupId: normalizedTargetGroupId, rawData, modId: mod?.id });

    if (mod && (mod.id === rawData || !rawData)) {
        const modId = mod.id;
        
        if (mod.group === normalizedTargetGroupId) return;
        if (normalizedTargetGroupId === 'All') {
            return; 
        }
    
        try {
            const targetPhysicalPath = resolveGroupPhysicalPath(normalizedTargetGroupId);
            const nextRelativePath = buildMovedModRelativePath(mod, targetPhysicalPath);
            await ModManager.moveModToGroup(selectedGame.value, modId, targetPhysicalPath);
            suppressFsRefresh(1400);
            await ModTagStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
            await ModStateStore.remapModPath(selectedGame.value, mod.relativePath, nextRelativePath);
            await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
            
            ElMessage.success({
                message: t('modsManagement.messages.movedToGroup', {
                    group: normalizedTargetGroupId === 'Root' ? t('modsManagement.ui.uncategorizedRoot') : normalizedTargetGroupId,
                }),
                offset: 48 
            });
            await refreshAfterMutation('structure');
        } catch (e: unknown) {
                 ElMessage.error({
                     message: t('modsManagement.messages.moveFailed', { error: String(e) }),
                offset: 48
            });
        } finally {
            draggingMod.value = null;
            document.body.style.userSelect = '';
        }
    } else {
        // Check if external files were dropped on this tree node
        if (e.dataTransfer?.types.includes('Files')) {
            pendingExternalFileDropGroup.value = normalizedTargetGroupId;
            return;
        }
        debugWarn('Drop', 'No mod captured or ID mismatch', { rawData, dragging: mod?.id });
    }
};

const onManualSortMouseDown = (e: MouseEvent, mod: ModInfo) => {
    if (e.button !== 0) return;
    manualSortState.active = true;
    manualSortState.startX = e.clientX;
    manualSortState.startY = e.clientY;
    manualSortState.hasMoved = false;
    manualSortState.mod = mod;
    draggingOrderId.value = mod.id;
    document.addEventListener('mousemove', onManualSortMouseMove);
    document.addEventListener('mouseup', onManualSortMouseUp);
};

const setManualSortGroupHover = (el: HTMLElement | null) => {
    if (manualSortGroupHover && manualSortGroupHover !== el) {
        manualSortGroupHover.classList.remove('drag-over');
    }
    if (el && manualSortGroupHover !== el) {
        el.classList.add('drag-over');
    }
    manualSortGroupHover = el;
};

const onManualSortMouseMove = (e: MouseEvent) => {
    if (!manualSortState.active || !manualSortState.mod) return;
    const dx = e.clientX - manualSortState.startX;
    const dy = e.clientY - manualSortState.startY;
    if (!manualSortState.hasMoved && Math.hypot(dx, dy) > 3) {
        manualSortState.hasMoved = true;
        document.body.style.userSelect = 'none';
    }

    if (manualSortState.hasMoved) {
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const groupEl = el?.closest?.('[data-group-id]') as HTMLElement | null;
        if (groupEl) {
            setManualSortGroupHover(groupEl);
            dragOverId.value = null; // don't show card hover while over sidebar
        } else {
            setManualSortGroupHover(null);
            const card = el?.closest?.('.mod-card') as HTMLElement | null;
            const targetId = card?.dataset?.modId || null;
            dragOverId.value = targetId;
        }
    }
};

function resetManualSortState() {
    manualSortState.active = false;
    manualSortState.hasMoved = false;
    manualSortState.mod = null;
    draggingOrderId.value = null;
    dragOverId.value = null;
    setManualSortGroupHover(null);
    document.body.style.userSelect = '';
}

const onManualSortMouseUp = (e: MouseEvent) => {
    document.removeEventListener('mousemove', onManualSortMouseMove);
    document.removeEventListener('mouseup', onManualSortMouseUp);

    if (!manualSortState.active || !manualSortState.mod) {
        resetManualSortState();
        return;
    }

    if (manualSortState.hasMoved) {
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const groupEl = el?.closest?.('[data-group-id]') as HTMLElement | null;
        const targetGroupId = groupEl?.dataset.groupId;

        if (targetGroupId && targetGroupId !== 'All' && manualSortState.mod.group !== targetGroupId) {
            moveModToGroup(manualSortState.mod, targetGroupId);
            resetManualSortState();
            return;
        }

        const card = el?.closest?.('.mod-card') as HTMLElement | null;
        const targetId = card?.dataset?.modId || null;
        if (targetId && targetId !== manualSortState.mod.id) {
            applyManualReorder(manualSortState.mod.id, targetId);
        }
    }

    resetManualSortState();
};

// Watcher cleanup
let unlistenFileChange: UnlistenFn | null = null;
let unlistenDrop: UnlistenFn | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let unlistenInstallProgress: UnlistenFn | null = null;
let unlistenWindowFocus: UnlistenFn | null = null;
let unlistenWindowResize: UnlistenFn | null = null;

const onVisibilityChange = () => {
    void refreshWindowEffectsState();
};

let suppressFsRefreshUntil = 0;

const suppressFsRefresh = (durationMs = 1200) => {
    suppressFsRefreshUntil = Math.max(suppressFsRefreshUntil, Date.now() + durationMs);
};

// Initialize selected game from store if possible
onMounted(async () => {
    startAutoSwitch(shouldRunVisualEffects, visibleSubGroups);

    void refreshWindowEffectsState();

    const appWindow = tryGetCurrentWindow();
    if (appWindow) {
        try {
            unlistenWindowFocus = await appWindow.onFocusChanged(() => {
                void refreshWindowEffectsState();
            });
            unlistenWindowResize = await appWindow.onResized(() => {
                void refreshWindowEffectsState();
            });
        } catch (error) {
            console.warn('Failed to attach window activity listeners for effect throttling:', error);
        }
    }

    // ... existing init code ...
    if (appSettings.CurrentGameName && gamesList.find(g => g.name === appSettings.CurrentGameName)) {
        selectedGame.value = appSettings.CurrentGameName;
    } else if (gamesList.length > 0) {
        selectedGame.value = gamesList[0].name;
    }
    
    // Listen for file drops
    unlistenDrop = await listen<{ paths: string[] }>('tauri://drag-drop', async (event) => {
        // 主页(/)拖入转交的路径优先处理——即使事件到来时路由尚未切换完成。
        const handedOff = takePendingModInstallDrop();
        if (handedOff.length > 0) {
            if (handedOff.length === 1) {
                await handleFileDrop(handedOff[0]);
            } else {
                await batchInstallFromPaths(handedOff);
            }
            return;
        }
        if (router.currentRoute.value.path !== '/mods') return;
        const payload = event.payload;
        if (payload.paths && payload.paths.length > 0) {
            if (payload.paths.length === 1) {
                handleFileDrop(payload.paths[0]);
            } else {
                await batchInstallFromPaths(payload.paths);
            }
        }
    });

    // Install progress from backend
    unlistenInstallProgress = await listen<{ percent: number; stage: string }>('mod-install-progress', (event) => {
        const payload = event.payload as { gameName?: string; modName?: string; stage?: string; current?: number; total?: number };
        if (!payload) return;
        if (payload.gameName && payload.gameName !== selectedGame.value) return;
        const total = payload.total ?? 0;
        const current = payload.current ?? 0;
        const stage = payload.stage || 'extracting';
        installProgress.visible = true;
        installProgress.stage = stage;
        if (total > 0) {
            installProgress.percent = Math.min(100, Math.round((current / total) * 100));
        } else {
            installProgress.percent = stage === 'done' ? 100 : 10;
        }
        if (stage === 'done') {
            setTimeout(() => {
                installProgress.visible = false;
            }, 600);
        }
    });

    unlistenFileChange = await listen<string[]>('mod-library-files-changed', () => {
        // Keep-alive views remain subscribed while hidden.  Do not start an
        // expensive filesystem/index refresh in the background: record the
        // change and perform the one deliberate rescan when this page is
        // brought back into view.
        if (!isModsPageActive.value) {
            needsReactivationRescan = true;
            return;
        }
        if (debounceTimer) clearTimeout(debounceTimer);
        debounceTimer = setTimeout(async () => {
            if (!selectedGame.value) return;
            if (Date.now() < suppressFsRefreshUntil) return;
            await refreshAfterMutation('structure');
        }, 600);
    });

    // Debug: log global dragenter/dragover to see if events fire anywhere
    globalDragOverLogger = (e: DragEvent) => {
        // Only log for our page container to reduce noise
        const t = e.target as HTMLElement | null;
        if (t && t.closest && t.closest('.mod-manager')) {
            debugLog('Global dragover', t.className);
        }
    };
    globalDragEnterLogger = (e: DragEvent) => {
        const t = e.target as HTMLElement | null;
        if (t && t.closest && t.closest('.mod-manager')) {
            debugLog('Global dragenter', t.className);
        }
    };
    document.addEventListener('dragover', globalDragOverLogger);
    document.addEventListener('dragenter', globalDragEnterLogger);
    document.addEventListener('visibilitychange', onVisibilityChange);
    window.addEventListener('focus', onVisibilityChange);
    window.addEventListener('blur', onVisibilityChange);
    window.addEventListener('mousedown', onMouseSideBack, true);
    window.addEventListener('mouseup', onMouseSideBack, true);
});

onUnmounted(() => {
    stopAutoSwitch();
    if (unlistenFileChange) unlistenFileChange();
    if (unlistenDrop) unlistenDrop();
    if (unlistenInstallProgress) unlistenInstallProgress();
    if (unlistenWindowFocus) unlistenWindowFocus();
    if (unlistenWindowResize) unlistenWindowResize();
    if (subgroupPreviewPreloadTimer) clearTimeout(subgroupPreviewPreloadTimer);
    if (searchTimer) clearTimeout(searchTimer);
    if (subgroupPreviewPersistTimer) clearTimeout(subgroupPreviewPersistTimer);
    if (debounceTimer) clearTimeout(debounceTimer);
    clearModAnalysisSubscriptions();
    // Flush any deferred persistence writes immediately
    [manualOrdersTimer, expandedStateTimer, selectedGroupTimer].forEach(t => {
        if (t) { clearTimeout(t); }
    });
    persistManualOrdersStorage(manualOrders.value);
    flushGroupOrders();
    persistExpandedStateStorage(expandedState.value);
    persistSelectedGroupStateStorage(selectedGroupState.value);
    invoke('unwatch_mod_library').catch(e => console.error(e));

    if (globalDragOverLogger) document.removeEventListener('dragover', globalDragOverLogger);
    if (globalDragEnterLogger) document.removeEventListener('dragenter', globalDragEnterLogger);
    document.removeEventListener('visibilitychange', onVisibilityChange);
    window.removeEventListener('focus', onVisibilityChange);
    window.removeEventListener('blur', onVisibilityChange);
    window.removeEventListener('mousedown', onMouseSideBack, true);
    window.removeEventListener('mouseup', onMouseSideBack, true);
});

watch(selectedGame, (newVal) => {
    if (newVal) {
        void (async () => {
            if (!isModsPageActive.value) {
                needsReactivationRescan = true;
                return;
            }
            const remembered = selectedGroupState.value[newVal];
            const nextGroup = await resolvePreferredGroup(newVal, remembered);
            if (selectedGame.value !== newVal) {
                return;
            }

            resetModKeyPopoverState();
            selectedGroup.value = nextGroup;
            sidebarSelectedGroup.value = nextGroup;
            expandedKeys.value = [...sanitizeExpanded(newVal)];
            applyExpandedToTree();
            await startWatching(newVal);
        })();
    }
});

watch(selectedGame, async (newVal) => {
    resetTagState();
    if (!newVal) return;
    await loadTagState(newVal);
});

watch(selectedGroup, (newVal) => {
    const game = selectedGame.value;
    if (!game || !newVal) return;
    if (selectedGroupState.value[game] !== newVal) {
        selectedGroupState.value[game] = newVal;
        persistSelectedGroupState();
    }
});

watch(() => appSettings.CurrentGameName, (newVal) => {
    if (newVal && gamesList.find(g => g.name === newVal)) {
        selectedGame.value = newVal;
    }
}, { immediate: true });

watch([mods, selectedGame, selectedGroup], () => {
    if (loading.value || switchingGroup.value || isHydratingMods.value || isOrderPersistencePaused.value) {
        return;
    }
    const ctx = getOrderContext();
    if (ctx) sanitizeOrderForContext(ctx.game, ctx.group);
}, { immediate: true });

watch(
    () => [selectedGame.value, currentSubGroups.value.map(g => g.id).join('|'), searchQuery.value],
    () => {
        const query = searchQuery.value.trim().toLowerCase();
        const visibleGroups = query
            ? currentSubGroups.value.filter(g => g.name.toLowerCase().includes(query) || g.id.toLowerCase().includes(query))
            : [...currentSubGroups.value];

        if (subgroupPreviewPreloadTimer) {
            clearTimeout(subgroupPreviewPreloadTimer);
        }
        subgroupPreviewPreloadTimer = setTimeout(() => {
            void preloadSubgroupPreviewImages(visibleGroups);
        }, 120);
    },
    { immediate: true }
);

watch(availableGroups, () => {
    const game = selectedGame.value;
    if (!game) return;
    const buckets: Record<string, string[]> = {};
    availableGroups.value.forEach(g => {
        const parent = getGroupParent(g.id);
        if (!buckets[parent]) buckets[parent] = [];
        buckets[parent].push(g.id);
    });
    Object.entries(buckets).forEach(([parent, ids]) => {
        sanitizeGroupOrder(game, parent, ids);
    });
}, { immediate: true });

const startWatching = async (gameName: string) => {
    const currentToken = ++startWatchingToken;
    const hadContent = hasVisibleContent();
    try {
        await withOrderPersistencePaused(async () => {
            const usedCache = await applyCachedSnapshot(gameName);
            if (!hadContent && !usedCache) {
                loading.value = true;
            }

            // Initial load only Root; keep UI content while refreshing.
            await refreshMods(gameName, { preserveVisible: hadContent || usedCache });
        });

        const installDir = await ModManager.getInstallDir(gameName);
        await invoke('watch_mod_library', { installDir });
        watchedGameName = gameName;
    } catch (error) {
        console.error('Failed to start watching:', error);
    } finally {
        if (currentToken === startWatchingToken) {
            loading.value = false;
        }
    }
};

const refreshCurrentMutationContent = async () => {
    if (!selectedGame.value) return;
    const currentRefreshToken = ++refreshModsToken;
    const signal = makeScanSignal(() => refreshModsToken, currentRefreshToken);
    await withOrderPersistencePaused(async () => {
        // `refresh: true` is essential: clearing the Pinia cache alone still
        // allows the Rust SQLite index to return the pre-mutation snapshot.
        await loadGroupMods(selectedGroup.value, { showProgress: false, signal, refresh: true });
    });
};

const refreshMods = async (gameName: string, options?: { preserveVisible?: boolean }) => {
    const currentRefreshToken = ++refreshModsToken;
    const preserveVisible = !!options?.preserveVisible;
    const signal = makeScanSignal(() => refreshModsToken, currentRefreshToken);
    try {
        await withOrderPersistencePaused(async () => {
            // Reset state only for hard reload; preserve existing cards for fast perceived load.
            if (!preserveVisible) {
                availableGroups.value = [];
                mods.value = [];
                loadedModsCount.value = 0;
                totalModsCount.value = 0;
            }

            // Try streaming scan first for progressive rendering.
            // Falls back to cached SQLite query if already indexed.
            const streamed = await loadGroupModsStreaming(gameName, 'Root', currentRefreshToken);
            if (signal.isCancelled()) {
                return;
            }

            const nextGroup = await resolvePreferredGroup(gameName, selectedGroup.value);
            if (selectedGroup.value !== nextGroup) {
                selectedGroup.value = nextGroup;
            }
            if (sidebarSelectedGroup.value !== nextGroup) {
                sidebarSelectedGroup.value = nextGroup;
            }

            // If selectedGroup is specific, try to load it?
            if (!streamed && selectedGroup.value && selectedGroup.value !== 'All' && selectedGroup.value !== 'Root') {
                await loadGroupMods(selectedGroup.value, { showProgress: !preserveVisible, signal });
            }
        });
        if (signal.isCancelled()) {
            return;
        }
        await loadTagState(gameName);
        allModsCatalogLoadedForGame.value = '';
        if (hasActiveTagFilter.value) {
            await refreshAllModsCatalog({ force: true });
        }
    } catch (error) {
        if (ModManager.isScanCancelled(error)) {
            return;
        }
        console.error('Failed to scan mods:', error);
        ElMessage.error(t('modsManagement.messages.scanFailed', { error: String(error) }));
        // Keep the last known-good cards visible. A transient scan failure is
        // not proof that the user's library is empty.
    }
};

mutationReconciler = createMutationReconciler({
    getGameName: () => selectedGame.value,
    invalidateClientState: (gameName) => {
        resetModKeyPopoverState();
        ModManager.clearCache(gameName);
        migotoIniService.clearGame(gameName);
        invalidateSubgroupPreviewByGame(gameName);
        allModsCatalogLoadedForGame.value = '';
        groupIconVersion.value = Date.now();
    },
    refreshCurrentContent: async (gameName) => {
        await refreshCurrentMutationContent();
        await loadTagState(gameName);
        if (hasActiveTagFilter.value) {
            await refreshAllModsCatalog({ force: true });
        }
    },
    refreshStructure: async (gameName) => {
        // Rebuild the persistent Rust index first, then hydrate the current
        // view and lazy tree from that fresh source of truth.
        await ModManager.refreshLibrary(gameName);
        await refreshMods(gameName, { preserveVisible: true });
        bumpGroupTree();
    },
    onError: (error, impact) => {
        console.error(`Failed to reconcile ${impact} mutation`, error);
        ElMessage.error(t('modsManagement.messages.scanFailed', { error: String(error) }));
    },
});

const rescanAfterReactivation = async () => {
    if (!needsReactivationRescan || reactivationRescanRunning || !selectedGame.value) return;
    reactivationRescanRunning = true;
    try {
        if (watchedGameName !== selectedGame.value) {
            await startWatching(selectedGame.value);
        }
        // A kept-alive page can miss watcher events while another view is active.
        // Rebuild the persistent index before showing it again so paths always
        // reflect the actual Mods directory, not a stale cached snapshot.
        await refreshAfterMutation('structure');
    } catch (error) {
        console.error('Failed to rescan Mods after reactivation:', error);
    } finally {
        needsReactivationRescan = false;
        reactivationRescanRunning = false;
    }
};

onDeactivated(() => {
    isModsPageActive.value = false;
    // Do NOT mark needsReactivationRescan here: the file-change listener and
    // the game watcher already set it when something actually changed while
    // this page was hidden. Flagging it unconditionally forced a full
    // structure rescan on every tab switch, which made the kept-alive page
    // feel like it reloaded from scratch each time.
    dismissFloating3DPreview();
});

onActivated(() => {
    isModsPageActive.value = true;
    void drainPendingModInstallDrop();
    void rescanAfterReactivation();
});

const updateAvailableGroups = (newGroups: GroupInfo[]) => {
    const existingMap = new Map(availableGroups.value.map(g => [g.id, g]));
    let changed = false;
    const merged = availableGroups.value.map(g => {
        const fresh = newGroups.find(ng => ng.id === g.id);
        if (fresh) {
            // Update existing entry with fresh scan data so fields like
            // enabled, iconPath, and modCount stay in sync with disk.
            if (
                g.enabled !== fresh.enabled
                || g.iconPath !== fresh.iconPath
                || g.modCount !== fresh.modCount
                || g.path !== fresh.path
                || g.name !== fresh.name
            ) {
                changed = true;
                return {
                    ...g,
                    name: fresh.name,
                    path: fresh.path,
                    enabled: fresh.enabled,
                    iconPath: fresh.iconPath,
                    modCount: fresh.modCount,
                };
            }
        }
        return g;
    });
    const toAdd = newGroups.filter(g => !existingMap.has(g.id));
    if (toAdd.length > 0) {
        changed = true;
        availableGroups.value = [...merged, ...toAdd];
    } else if (changed) {
        availableGroups.value = merged;
    }
};

const {
    tagDefinitions,
    activeTagIds,
    allModsCatalog,
    allModsCatalogLoadedForGame,
    tagManagementDialog,
    modTagDialog,
    activeTags,
    hasActiveTagFilter,
    resetTagState,
    loadTagState,
    refreshAllModsCatalog,
    getTagIdsForMod,
    getTagsForMod,
    getTagUsageCount,
    getTagIconUrl,
    getDraftTagIconPreviewUrl,
    getTagChipStyle,
    toggleActiveTag,
    clearActiveTags,
    syncTagStateAfterMutation,
    resetTagManagementForm,
    openTagManagementDialog,
    editTagDefinition,
    pickTagIconSource,
    getEditingTag,
    saveTagDefinition,
    deleteTagDefinition,
    openModTagDialog,
    saveModTagAssignments,
    getTagsForGroup,
    isNsfwTag,
    toggleModNsfw,
    toggleGroupNsfw,
    openGroupTagDialog,
    saveGroupTagAssignments,
} = useModsManagementTags({
    selectedGame,
    t,
    updateAvailableGroups,
    suppressFsRefresh,
});

const currentTagTargetIsGroup = ref(false);

const openGroupTagDialogWrapped = (group: GroupInfo) => {
    currentTagTargetIsGroup.value = true;
    openGroupTagDialog(group);
};

const openModTagDialogWrapped = (mod: ModInfo) => {
    currentTagTargetIsGroup.value = false;
    openModTagDialog(mod);
};

const isNsfwGroupMarked = (group: GroupInfo) => getTagsForGroup(group).some(isNsfwTag);

const toggleModNsfwMarked = async (mod: ModInfo) => {
    try {
        const isNowMarked = await toggleModNsfw(mod);
        ElMessage.success(t(isNowMarked ? 'modsManagement.messages.nsfwMarked' : 'modsManagement.messages.nsfwUnmarked'));
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.nsfwMarkFailed', { error: String(error) }));
    }
};

const toggleGroupNsfwMarked = async (group: GroupInfo) => {
    try {
        const isNowMarked = await toggleGroupNsfw(group);
        ElMessage.success(t(isNowMarked ? 'modsManagement.messages.nsfwMarked' : 'modsManagement.messages.nsfwUnmarked'));
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.nsfwMarkFailed', { error: String(error) }));
    }
};

const saveTagAssignmentsForCurrentTarget = async () => {
    if (currentTagTargetIsGroup.value) {
        await saveGroupTagAssignments();
    } else {
        await saveModTagAssignments();
    }
    currentTagTargetIsGroup.value = false;
};

// Replaces fetchMods
const fetchMods = async () => {
    if (selectedGame.value) {
        ModManager.clearCache(selectedGame.value);
        migotoIniService.clearGame(selectedGame.value);
        resetModKeyPopoverState();
        invalidateSubgroupPreviewByGame(selectedGame.value);
        loading.value = true;
        try {
            await ModManager.refreshLibrary(selectedGame.value);
            await startWatching(selectedGame.value);
        } catch (error) {
            console.error('Manual mod library refresh failed:', error);
            ElMessage.error(t('modsManagement.messages.scanFailed', { error: String(error) }));
        } finally {
            loading.value = false;
        }
    }
};

const remapModMetadataBestEffort = async (
    game: string,
    tagSourcePath: string,
    stateSourcePath: string,
    nextRelativePath: string,
) => {
    try {
        await ModTagStore.remapModPath(game, tagSourcePath, nextRelativePath);
    } catch (error) {
        debugWarn('ModsManagement.Toggle', 'Failed to remap mod tags after folder rename', { error, tagSourcePath, nextRelativePath });
    }

    try {
        await ModStateStore.remapModPath(game, stateSourcePath, nextRelativePath);
    } catch (error) {
        debugWarn('ModsManagement.Toggle', 'Failed to remap remembered mod state after folder rename', { error, stateSourcePath, nextRelativePath });
    }
};

const toggleMod = async (mod: ModInfo) => {
    const targetState = !mod.enabled;
    let enabledParentGroups: DisabledParentGroupInfo[] = [];
    
    try {
        if (!targetState && selectedGame.value) {
            try {
                const analysis = await migotoIniService.load(selectedGame.value, mod);
                await captureModRuntimeState(selectedGame.value, mod, analysis.modKeyList);
            } catch (memoryError) {
                console.warn('Failed to capture mod runtime state before disable:', memoryError);
            }
        }

        const disabledParentGroups = targetState
            ? await ModManager.getDisabledParentGroups(selectedGame.value, mod.relativePath)
            : [];
        let modPathAfterParentEnable = mod.relativePath;

        if (disabledParentGroups.length > 0) {
            const categoryList = disabledParentGroups.map((group) => group.enabledPath).join('\n');
            try {
                await ElMessageBox.confirm(
                    t('modsManagement.messages.forceEnableDisabledParentCategoriesConfirm', {
                        categories: categoryList,
                    }),
                    t('modsManagement.dialog.forceEnableCategoryTitle'),
                    {
                        confirmButtonText: t('modsManagement.common.confirm'),
                        cancelButtonText: t('modsManagement.common.cancel'),
                        type: 'warning',
                    },
                );
            } catch {
                return;
            }

            suppressFsRefresh(3500);
            for (const group of disabledParentGroups) {
                await ModManager.toggleGroup(selectedGame.value, group.disabledPath, true);
                enabledParentGroups.push(group);
                await ModTagStore.remapPrefix(selectedGame.value, group.disabledPath, group.enabledPath);
                await ModTagStore.remapGroupPrefix(selectedGame.value, group.disabledPath, group.enabledPath);
                await ModStateStore.remapPrefix(selectedGame.value, group.disabledPath, group.enabledPath);
                modPathAfterParentEnable = replacePathPrefix(modPathAfterParentEnable, group.disabledPath, group.enabledPath);
            }
        }

        // Prevent watcher-triggered full refresh from interrupting local toggle UX.
        suppressFsRefresh(2500);
        invalidateSubgroupPreviewByGroupId(selectedGame.value, mod.group);
        const previousRelativePath = mod.relativePath;
        const nextRelativePath = await ModManager.toggleMod(selectedGame.value, modPathAfterParentEnable, targetState);
        updateModToggleLocalState(mod, nextRelativePath, targetState);
        await remapModMetadataBestEffort(
            selectedGame.value,
            modPathAfterParentEnable,
            previousRelativePath,
            nextRelativePath,
        );

        if (targetState && selectedGame.value) {
            try {
                await restoreModRuntimeState(selectedGame.value, { relativePath: nextRelativePath });
            } catch (memoryError) {
                console.warn('Failed to restore mod runtime state after enable:', memoryError);
            }
        }

        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === selectedGame.value });
        await migrateEnabledParentGroupsLocalState(enabledParentGroups);
        await refreshAfterMutation(enabledParentGroups.length > 0 ? 'structure' : 'content');
        triggerModPulse(mod.id);
        ElMessage.success(t('modsManagement.messages.modStateChanged', {
            mod: mod.name,
            state: targetState ? t('modsManagement.common.enabled') : t('modsManagement.common.disabled'),
        }));
    } catch (error) {
        console.error('Failed to toggle mod:', error);
        if (enabledParentGroups.length > 0) {
            await refreshAfterMutation('structure');
        }
        ElMessage.error(t('modsManagement.messages.operationFailed', { error: String(error) }));
    }
};

const enableModSolo = async (mod: ModInfo) => {
    const game = selectedGame.value;
    if (!game) return;

    const group = mod.group;
    const targetIdentity = normalizeModIdentity(mod.relativePath);

    try {
        const freshGroup = await ModManager.scanGroup(game, group, undefined, { refresh: true });
        const diskTarget = freshGroup.mods.find((item) => normalizeModIdentity(item.relativePath) === targetIdentity);
        if (!diskTarget) {
            throw new Error(t('modManager.messages.modDirectoryNotFound'));
        }

        const enabledSiblings = freshGroup.mods.filter((item) =>
            item.enabled && normalizeModIdentity(item.relativePath) !== targetIdentity
        );
        suppressFsRefresh((enabledSiblings.length + 1) * 700 + 2500);

        for (const sibling of enabledSiblings) {
            const previousRelativePath = sibling.relativePath;
            const nextPath = await ModManager.toggleMod(game, previousRelativePath, false);
            updateModToggleLocalState(sibling, nextPath, false);
            await remapModMetadataBestEffort(game, previousRelativePath, previousRelativePath, nextPath);
            invalidateSubgroupPreviewByGroupId(game, sibling.group);
        }

        let targetPath = diskTarget.relativePath;
        if (!diskTarget.enabled) {
            const previousRelativePath = diskTarget.relativePath;
            targetPath = await ModManager.toggleMod(game, previousRelativePath, true);
            await remapModMetadataBestEffort(game, previousRelativePath, previousRelativePath, targetPath);
        }

        updateModToggleLocalState(mod, targetPath, true);
        invalidateSubgroupPreviewByGroupId(game, group);
        triggerModPulse(mod.id);

        await syncTagStateAfterMutation({ reloadAllMods: allModsCatalogLoadedForGame.value === game });
        await refreshAfterMutation('content');
        ElMessage.success(t('modsManagement.messages.enableModSoloSuccess', { mod: mod.name }));
    } catch (error) {
        console.error('Failed to enable mod exclusively:', mod.name, error);
        await refreshAfterMutation('content');
        ElMessage.error(t('modsManagement.messages.operationFailed', { error: String(error) }));
    }
};

const convertGroupToMod = async (group: GroupInfo) => {
    const game = selectedGame.value;
    if (!game) return;

    try {
        const modsDir = await ModManager.getInstallDir(game);
        const modsRoot = `${modsDir.replace(/\\/g, '/').replace(/\/+$/, '')}/Mods`;
        const groupDir = `${modsRoot}/${group.path || group.id}`;
        const markerPath = `${groupDir}/thisisa.mod`;

        try {
            await ElMessageBox.confirm(
                t('modsManagement.messages.convertCategoryToModHint', { group: group.name }),
                t('modsManagement.messages.convertCategoryToModTitle'),
                { confirmButtonText: t('modsManagement.common.confirm'), cancelButtonText: t('modsManagement.common.cancel'), type: 'info' }
            );
        } catch {
            return; // user cancelled
        }

        // Create a thisisa.mod marker file — the Rust scanner treats
        // any directory containing this file as a leaf mod, no .ini required.
        await writeTextFile(markerPath, '# SSMT Mod Marker\n');
        suppressFsRefresh(2000);
        await refreshAfterMutation('structure');

        ElMessage.success(t('modsManagement.messages.convertCategoryToModSuccess', { group: group.name }));
    } catch (e) {
        console.error('Failed to convert group to mod:', e);
        ElMessage.error(t('modsManagement.messages.operationFailed', { error: String(e) }));
    }
};

const openModFolder = async (path: string) => {
    try {
        await openPath(path);
    } catch (error) {
        console.error(error);
    }
};

const openGameFolder = async () => {
    try {
        await ModManager.openGameModsFolder(selectedGame.value);
    } catch (error) {
        console.error(error);
    }
}

const openCurrentGroupFolder = async () => {
    const group = selectedGroup.value;
    if (!group || group === 'All' || group === ROOT_GROUP_ID) {
        await openGameFolder();
    } else {
        try {
            await ModManager.openModGroupFolder(selectedGame.value, group);
        } catch (error) {
            console.error(error);
        }
    }
}

// Computed Properties
const groups = computed(() => {
    // Map of groupID -> GroupInfo
    const map = new Map<string, GroupInfo>();
    
    // Add known groups from backend
    availableGroups.value.forEach(g => {
        map.set(g.id, g);
    });

    // Add implicit groups from mods
    mods.value.forEach(m => {
        if (m.group && m.group !== "Root" && !map.has(m.group)) {
            // Split slash name if we want friendly name for implicit groups
            // ModInfo.group is the full path ID now
            const parts = m.group.split('/');
            const name = parts[parts.length - 1];
            map.set(m.group, { 
                id: m.group, 
                name: name,
                path: m.group, // Fallback path
                enabled: true, // Fallback enabled state
                iconPath: undefined,
                modCount: 0,
            });
        }
    });

    // Sort by ID is usually fine for hierarchy
    const list = Array.from(map.values()).sort((a, b) => a.id.localeCompare(b.id));

    return [{ id: 'All', name: t('modsManagement.ui.all'), path: '', enabled: true }, ...list];
});

// Lazy load node for Tree
const loadNode = async (node: { level: number; data: GroupInfo }, resolve: (data: Array<GroupInfo & { label: string; leaf: boolean }>) => void) => {
    if (!selectedGame.value) return resolve([]);

    // Root level
    if (node.level === 0) {
        try {
            const cached = ModManager.getCachedGroup(selectedGame.value, 'Root');
            const groups = cached ? cached.groups : (await ModManager.scanGroup(selectedGame.value, 'Root')).groups;
            updateAvailableGroups(groups);
            const orderedGroups = sortGroupsByOrder(selectedGame.value, ROOT_PARENT_ID, groups);
            const nodes = orderedGroups.map(g => ({
                id: g.id,
                label: g.name,
                leaf: false,
                icon: g.iconPath,
                path: g.path,
                enabled: g.enabled,
                isDirectoryLink: g.isDirectoryLink,
                name: g.name,
                count: g.modCount ?? 0,
            }));
            return resolve(nodes as Array<GroupInfo & { label: string; leaf: boolean }>);
        } catch (e) {
            console.error(e);
            return resolve([]);
        }
    }

    // Child level
    if (node.data) {
        const groupPath = node.data.id === MODS_TREE_ROOT_ID ? 'Root' : node.data.id;
        // Check cache first — avoids redundant IPC calls
        const cached = ModManager.getCachedGroup(selectedGame.value, groupPath);
        let groups: GroupInfo[];
        if (cached) {
            groups = cached.groups;
        } else {
            const result = await ModManager.scanGroup(selectedGame.value, groupPath);
            groups = result.groups;
        }
        updateAvailableGroups(groups);
        const orderedGroups = sortGroupsByOrder(selectedGame.value, String(node.data.id || ''), groups);
        const nodes = orderedGroups.map(g => ({
            id: g.id,
            label: g.name,
            leaf: false,
            icon: g.iconPath,
            path: g.path,
            enabled: g.enabled,
            isDirectoryLink: g.isDirectoryLink,
            name: g.name,
            count: g.modCount ?? 0,
        }));
        resolve(nodes as Array<GroupInfo & { label: string; leaf: boolean }>);
    }
};

const syncTreeCurrentSelection = (groupId: string) => {
    sidebarSelectedGroup.value = groupId;
    const tree = groupTreeRef.value as { setCurrentKey: (key?: string) => void; getCurrentKey: () => string | null; getNode: (key: string) => ElTreeNode | null; store: { getNode: (key: string) => ElTreeNode | null } };
    if (!tree || typeof tree.setCurrentKey !== 'function') return;
    tree.setCurrentKey(groupId === 'Root' ? undefined : groupId);
};

const rememberGroupNavigation = (fromGroup: string, toGroup: string) => {
    if (isApplyingGroupHistory.value || !fromGroup || fromGroup === toGroup) return;
    groupNavigationHistory.value = [...groupNavigationHistory.value.slice(-49), fromGroup];
    groupNavigationForwardHistory.value = [];
};

const handleGroupClick = async (data: GroupInfo, options?: { ensureExpanded?: boolean; syncSidebar?: boolean }) => {
    if (!data?.id) return;
    const ensureExpanded = options?.ensureExpanded ?? true;
    const syncSidebar = options?.syncSidebar ?? true;
    const nextGroup = data.id === MODS_TREE_ROOT_ID ? 'Root' : data.id;
    rememberGroupNavigation(selectedGroup.value, nextGroup);
    selectedGroup.value = nextGroup;
    if (syncSidebar) {
        syncTreeCurrentSelection(selectedGroup.value);
    }
    if (syncSidebar && ensureExpanded) {
        await expandGroupPath(selectedGroup.value);
    }
    // Load mods for this group
    await loadGroupMods(selectedGroup.value);
};

const handleTreeNodeClick = async (data: GroupInfo, _node?: unknown, _tree?: unknown, event?: MouseEvent) => {
    const target = event?.target as HTMLElement | null;
    if (target?.closest('.el-tree-node__expand-icon')) {
        return;
    }
    await handleGroupClick(data, { ensureExpanded: false });
};

const handleTreeCurrentChange = async (data: GroupInfo) => {
    if (!data?.id) return;
    // Fallback: if tree current node changes but main click handler wasn't triggered,
    // ensure the right panel always follows the selected node.
    if (data.id === sidebarSelectedGroup.value && data.id === selectedGroup.value) return;
    await handleGroupClick(data, { ensureExpanded: false });
};

const navigateToParentGroup = async () => {
    const current = selectedGroup.value;
    if (!current || current === 'All' || current === 'Root') {
        return false;
    }

    const parent = getGroupParent(current);
    const targetGroup = parent === ROOT_PARENT_ID ? 'Root' : parent;
    rememberGroupNavigation(current, targetGroup);
    selectedGroup.value = targetGroup;
    syncTreeCurrentSelection(targetGroup);
    await expandGroupPath(targetGroup);
    await loadGroupMods(targetGroup, { showProgress: false });
    return true;
};

const navigateBackGroup = async () => {
    const targetGroup = groupNavigationHistory.value.pop();
    if (!targetGroup) return;

    isApplyingGroupHistory.value = true;
    try {
        const currentGroup = selectedGroup.value;
        if (currentGroup && currentGroup !== targetGroup) {
            groupNavigationForwardHistory.value = [...groupNavigationForwardHistory.value.slice(-49), currentGroup];
        }
        selectedGroup.value = targetGroup;
        syncTreeCurrentSelection(targetGroup);
        await expandGroupPath(targetGroup);
        await loadGroupMods(targetGroup, { showProgress: false });
    } finally {
        isApplyingGroupHistory.value = false;
    }
};

const navigateForwardGroup = async () => {
    const targetGroup = groupNavigationForwardHistory.value.pop();
    if (!targetGroup) return;

    isApplyingGroupHistory.value = true;
    try {
        const currentGroup = selectedGroup.value;
        if (currentGroup && currentGroup !== targetGroup) {
            groupNavigationHistory.value = [...groupNavigationHistory.value.slice(-49), currentGroup];
        }
        selectedGroup.value = targetGroup;
        syncTreeCurrentSelection(targetGroup);
        await expandGroupPath(targetGroup);
        await loadGroupMods(targetGroup, { showProgress: false });
    } finally {
        isApplyingGroupHistory.value = false;
    }
};

const onMouseSideBack = (e: MouseEvent) => {
    // button 3 is Browser Back (XButton1) on most mice.
    if (e.button !== 3) return;
    const target = e.target as HTMLElement | null;
    if (!target?.closest('.mod-manager')) return;

    // Consume event to prevent route/browser history back.
    e.preventDefault();
    e.stopPropagation();
    void navigateBackGroup();
};

const loadGroupMods = async (groupId: String, options?: { showProgress?: boolean; signal?: { isCancelled?: () => boolean }; refresh?: boolean }) => {
    if (!selectedGame.value) return;
    const token = ++groupLoadToken;
    const showProgress = options?.showProgress !== false;
    const isAllGroup = groupId === 'All';
    const path = groupId === 'Root' ? 'Root' : groupId;
    const signal = {
        isCancelled: () => token !== groupLoadToken || !!options?.signal?.isCancelled?.(),
    };

    // Show cached data immediately for instant perceived response
    if (!options?.refresh && !isAllGroup) {
        const cached = ModManager.getCachedGroup(selectedGame.value, String(path));
        if (cached) {
            updateAvailableGroups(cached.groups);
            reconcileCurrentSubGroups(cached.groups);
            void preloadSubgroupPreviewImages(cached.groups);
            await appendModsIncrementally(cached.mods, token);
            if (token !== groupLoadToken) return;
        }
    }

    if (showProgress) {
        switchingGroup.value = true;
    }
    try {
        const { mods: newMods, groups: newGroups } = isAllGroup
            // "All" is a view change, not a request to probe the Mods folder
            // again.  The persistent index is rebuilt only at startup,
            // reactivation, a known local mutation, or a filesystem-change
            // notification.
            ? await ModManager.scanAllMods(selectedGame.value, signal)
            : await ModManager.scanGroup(selectedGame.value, path as string, signal, { refresh: !!options?.refresh });
        if (token !== groupLoadToken) return;
        if (signal?.isCancelled?.()) return;
        updateAvailableGroups(newGroups);
        reconcileCurrentSubGroups(isAllGroup ? [] : newGroups);
        if (!isAllGroup) {
            void preloadSubgroupPreviewImages(newGroups);
        }
        await appendModsIncrementally(newMods, token);
    } catch (e) {
        if (token !== groupLoadToken) return;
        if (ModManager.isScanCancelled(e)) return;
        currentSubGroups.value = [];
        mods.value = [];
        loadedModsCount.value = 0;
        totalModsCount.value = 0;
        ElMessage.error(t('modsManagement.messages.loadModFailed', { error: String(e) }));
    } finally {
        if (showProgress && token === groupLoadToken) {
            switchingGroup.value = false;
        }
    }
}


interface ScanChunkEvent {
    phase: 'start' | 'chunk' | 'done';
    total?: number;
    mods?: ModInfo[];
    groups?: GroupInfo[];
    totalMods?: number;
    totalGroups?: number;
}

/**
 * Load mods using the streaming scan command.
 * When the SQLite cache is populated, data arrives instantly (via cache).
 * On first visit, mods/groups arrive in chunks for progressive rendering.
 * Returns true if data was streamed progressively, false if it was instant (cached).
 */
const loadGroupModsStreaming = async (gameName: string, groupPath: string, refreshToken: number): Promise<boolean> => {
    const relativePath = (groupPath === 'Root' || groupPath === 'All') ? '' : groupPath;
    const installDir = await ModManager.getInstallDir(gameName);

    let streamedChunks = false;
    const accumulatedMods: ModInfo[] = [];
    const accumulatedGroups: GroupInfo[] = [];
    let chunkReconcile = Promise.resolve();

    const unlistenChunk = await listen<ScanChunkEvent>('mod-library-scan-chunk', (event) => {
        const payload = event.payload;
        if (!payload) return;
        chunkReconcile = chunkReconcile.then(async () => {
            switch (payload.phase) {
                case 'start':
                    totalModsCount.value = payload.total ?? 0;
                    accumulatedMods.length = 0;
                    accumulatedGroups.length = 0;
                    // Preserve cards until the final scan result is available; this
                    // avoids an empty, disorienting page during a background rescan.
                    break;
                case 'chunk': {
                    streamedChunks = true;
                    if (payload.mods && payload.mods.length > 0) {
                        accumulatedMods.push(...payload.mods);
                        await appendModsIncrementally(accumulatedMods, groupLoadToken, {
                            removeMissing: false,
                            refreshAnalysis: false,
                        });
                    }
                    if (payload.groups && payload.groups.length > 0) {
                        accumulatedGroups.push(...payload.groups);
                        updateAvailableGroups(payload.groups);
                        reconcileCurrentSubGroups(accumulatedGroups, false);
                    }
                    break;
                }
                case 'done':
                    totalModsCount.value = payload.totalMods ?? accumulatedMods.length;
                    break;
            }
        });
    });

    try {
        // Fire the streaming scan (don't await — events arrive during invocation)
        let result = await invoke<{ mods: ModInfo[]; groups: GroupInfo[] }>('mod_library_stream_scan', {
            gameName,
            installDir,
            groupPath: relativePath || 'Root',
        });

        await chunkReconcile;

        if (refreshModsToken !== refreshToken) return streamedChunks;

        // An empty cached root is cheap to verify and too risky to trust: an
        // earlier transient read failure may have persisted it as a valid
        // snapshot. A genuinely empty library simply scans empty again.
        if (result && result.mods.length === 0 && result.groups.length === 0) {
            result = await ModManager.scanGroup(gameName, groupPath, undefined, { refresh: true });
            if (refreshModsToken !== refreshToken) return streamedChunks;
        }

        // Finish with the command result so cached and streamed paths converge to
        // the same state even if no group chunk was emitted.
        if (result) {
            updateAvailableGroups(result.groups);
            reconcileCurrentSubGroups(result.groups);
            await appendModsIncrementally(result.mods, groupLoadToken);
            totalModsCount.value = result.mods.length;
        }
    } catch (e) {
        if (ModManager.isScanCancelled(e)) {
            return streamedChunks;
        }
        console.error('Stream scan failed:', e);
        // The streaming command normally prefers SQLite. If that path fails,
        // force a direct disk refresh so a manual refresh can always escape a
        // stale or damaged cached snapshot.
        if (refreshModsToken === refreshToken) {
            const result = await ModManager.scanGroup(gameName, groupPath, undefined, { refresh: true });
            if (refreshModsToken !== refreshToken) return streamedChunks;
            updateAvailableGroups(result.groups);
            reconcileCurrentSubGroups(result.groups);
            await appendModsIncrementally(result.mods, groupLoadToken);
            totalModsCount.value = result.mods.length;
        }
    } finally {
        unlistenChunk();
    }

    return streamedChunks;
};

const setGroupIcon = async (group: GroupInfo) => {
     try {
        const selected = await open({
            multiple: false,
            filters: [{
                name: 'Image',
                extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp']
            }]
        });

        if (selected) {
            await ModManager.setModGroupIcon(selectedGame.value, group.path || group.id, selected);
            ElMessage.success(t('modsManagement.messages.iconSetSuccessfully'));
            await refreshAfterMutation('structure');
        }
    } catch (e: unknown) {
        ElMessage.error(t('modsManagement.messages.setIconFailed', { error: String(e) }));
    }
};

const reorderGroup = (sourceId: string, targetId: string) => {
    const game = selectedGame.value;
    if (!game) return;
    if (sourceId === targetId) return;
    const sourceParent = getGroupParent(sourceId);
    const targetParent = getGroupParent(targetId);
    if (sourceParent !== targetParent) return; // forbid cross-level moves
    const siblings = availableGroups.value
        .filter(g => getGroupParent(g.id) === sourceParent)
        .map(g => g.id);
    sanitizeGroupOrder(game, sourceParent, siblings);
    const order = groupOrders.value[game][sourceParent] || [];
    const from = order.indexOf(sourceId);
    const to = order.indexOf(targetId);
    if (from === -1 || to === -1) return;
    const next = [...order];
    next.splice(from, 1);
    next.splice(to, 0, sourceId);
    groupOrders.value[game][sourceParent] = next;
    flushGroupOrders();
    bumpGroupTree();
};

const onGroupExpand = async (data: GroupInfo, node?: ElTreeNode) => {
    const game = selectedGame.value;
    if (!game) return;
    const id = String(node?.key ?? data?.id ?? '');
    if (!id) return;
    const set = new Set(sanitizeExpanded(game));
    if (!set.has(id)) {
        set.add(id);
        expandedKeys.value = Array.from(set);
        expandedState.value[game] = [...expandedKeys.value];
        debugLog('GroupExpanded', 'Expanded', id, '->', expandedKeys.value);
        persistExpandedState();
    }
    await handleGroupClick(data, { ensureExpanded: false });
};

const onGroupCollapse = async (data: GroupInfo, node?: ElTreeNode) => {
    const game = selectedGame.value;
    if (!game) return;
    const id = String(node?.key ?? data?.id ?? '');
    if (!id) return;
    const set = new Set(sanitizeExpanded(game));
    if (set.has(id)) {
        set.delete(id);
        expandedKeys.value = Array.from(set);
        expandedState.value[game] = [...expandedKeys.value];
        debugLog('GroupExpanded', 'Collapsed', id, '->', expandedKeys.value);
        persistExpandedState();
    }
    await handleGroupClick(data, { ensureExpanded: false });
};

const collapseSelectedGroup = async () => {
    const game = selectedGame.value;
    const currentGroup = String(sidebarSelectedGroup.value || '');
    if (!game || !currentGroup || currentGroup === 'All' || currentGroup === 'Root') return;
    const nextExpandedKeys = expandedKeys.value.filter((key) => !isSameOrChildPath(key, currentGroup));
    if (nextExpandedKeys.length === expandedKeys.value.length) return;
    expandedKeys.value = nextExpandedKeys;
    expandedState.value[game] = [...nextExpandedKeys];
    persistExpandedState();
    await applyExpandedToTree();
    await scrollGroupIntoView(currentGroup);
};

const onGroupMouseDown = (e: MouseEvent, groupId: string) => {
    if (groupId === 'All' || groupId === 'Root' || groupId === MODS_TREE_ROOT_ID) return;
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest('.el-tree-node__expand-icon')) return;
    groupDragState.active = true;
    groupDragState.startX = e.clientX;
    groupDragState.startY = e.clientY;
    groupDragState.hasMoved = false;
    groupDragState.sourceId = groupId;
    groupDragState.sourceParent = getGroupParent(groupId);
    groupDragState.targetId = null;
    document.addEventListener('mousemove', onGroupMouseMove);
    document.addEventListener('mouseup', onGroupMouseUp);
};

const onGroupMouseMove = (e: MouseEvent) => {
    if (!groupDragState.active) return;
    const dx = e.clientX - groupDragState.startX;
    const dy = e.clientY - groupDragState.startY;
    if (!groupDragState.hasMoved && Math.hypot(dx, dy) > 3) {
        groupDragState.hasMoved = true;
        document.body.style.userSelect = 'none';
    }
    if (groupDragState.hasMoved) {
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const node = el?.closest?.('[data-group-id]') as HTMLElement | null;
        const targetId = node?.dataset.groupId || null;
        const targetParent = node?.dataset.parentId || null;
        if (targetId && targetParent === groupDragState.sourceParent) {
            groupHoverId.value = targetId;
            groupDragState.targetId = targetId;
        } else {
            groupHoverId.value = null;
            groupDragState.targetId = null;
        }
    }
};

const onGroupMouseUp = () => {
    document.removeEventListener('mousemove', onGroupMouseMove);
    document.removeEventListener('mouseup', onGroupMouseUp);

    if (groupDragState.hasMoved && groupDragState.targetId) {
        reorderGroup(groupDragState.sourceId, groupDragState.targetId);
    }
    resetGroupDrag();
};

const resetGroupDrag = () => {
    groupDragState.active = false;
    groupDragState.hasMoved = false;
    groupDragState.sourceId = '';
    groupDragState.targetId = null;
    groupDragState.sourceParent = '';
    groupHoverId.value = null;
    document.body.style.userSelect = '';
};

const openModGroupFolder = async (group: GroupInfo) => {
    try {
        await ModManager.openModGroupFolder(selectedGame.value, group.path || group.id);
    } catch (e: unknown) {
        ElMessage.error(t('modsManagement.messages.openFolderFailed', { error: String(e) }));
    }
};

const buildSortComparator = (): ((a: ModInfo, b: ModInfo) => number) => {
    return buildModSortComparator({
        criterion: sortBy.value,
        order: sortOrder.value,
        sortTagId: sortTagId.value,
        manualOrderIds: sortBy.value === 'manual' ? getCurrentOrderList() : [],
        getManualOrderId,
        getTagIds: (mod) => getTagIdsForMod(mod.relativePath),
        getTagNames: (mod) => getTagsForMod(mod).map((tag) => tag.name),
    });
};

const filteredMods = computed(() => {
    const searching = !!searchQuery.value.trim();
    let result = searching || hasActiveTagFilter.value ? [...allModsCatalog.value] : [...mods.value];

    if (!searching && !hasActiveTagFilter.value && selectedGroup.value !== 'All') {
        result = result.filter(m => m.group === selectedGroup.value);
    }

    if (hasActiveTagFilter.value) {
        result = result.filter((mod) => activeTagIds.value.every((tagId) => getTagIdsForMod(mod.relativePath).includes(tagId)));
    }

    if (searching) {
        result = result.filter((mod) => searchMatchedModPaths.value.has(mod.relativePath));
    }

    return [...result].sort(buildSortComparator());
});

watch(
    () => router.currentRoute.value.path,
    (path) => {
        // Run before KeepAlive pauses this component. Teleported descendants
        // otherwise remain visible after the route's own DOM is deactivated.
        if (path !== '/mods') dismissFloating3DPreview();
    },
);

const emptyModsDescription = computed(() => {
    const query = searchQuery.value.trim();
    if (query) {
        return t('modsManagement.ui.noMatchingModForSearch', { query });
    }
    return hasActiveTagFilter.value
        ? t('modsManagement.ui.noMatchingMod')
        : t('modsManagement.ui.noContentYet');
});

const visibleSubGroups = computed(() => {
    if (hasActiveTagFilter.value || searchQuery.value.trim()) {
        return [] as GroupInfo[];
    }
    let result = [...currentSubGroups.value];
    if (searchQuery.value) {
        const query = searchQuery.value.toLowerCase();
        result = result.filter(g => g.name.toLowerCase().includes(query) || g.id.toLowerCase().includes(query));
    }
    const parentId = selectedGroup.value === 'Root' ? ROOT_PARENT_ID : selectedGroup.value;
    return sortGroupsByOrder(selectedGame.value, parentId, result);
});

watch(searchQuery, (rawQuery) => {
    const query = rawQuery.trim();
    const token = ++searchToken;
    if (searchTimer) clearTimeout(searchTimer);
    if (!query) {
        searchMatchedModPaths.value = new Set();
        searchLoading.value = false;
        return;
    }
    searchLoading.value = true;
    searchTimer = setTimeout(async () => {
        try {
            await refreshAllModsCatalog();
            const matches = await ModManager.searchMods(selectedGame.value, query);
            if (token === searchToken) searchMatchedModPaths.value = new Set(matches);
        } catch (error) {
            if (token === searchToken) {
                searchMatchedModPaths.value = new Set();
                console.error('Failed to search Mods:', error);
            }
        } finally {
            if (token === searchToken) searchLoading.value = false;
        }
    }, 250);
});

const navigateToModGroup = async (mod: ModInfo) => {
    searchQuery.value = '';
    const groupId = mod.group || ROOT_GROUP_ID;
    await handleGroupClick({ id: groupId, name: groupId.split('/').pop() || groupId, path: groupId, enabled: true } as GroupInfo);
};

const gameBananaIconDialog = reactive({ visible: false, group: null as GroupInfo | null, mode: 'auto' as 'auto' | 'manual' });
const openGameBananaIconDialog = (group: GroupInfo, mode: 'auto' | 'manual') => {
    gameBananaIconDialog.group = group;
    gameBananaIconDialog.mode = mode;
    gameBananaIconDialog.visible = true;
};
const applyGameBananaGroupIcon = async (group: GroupInfo, iconPath: string) => {
    try {
        await ModManager.setModGroupIcon(selectedGame.value, group.path || group.id, iconPath);
        gameBananaIconDialog.visible = false;
        ElMessage.success(t('modsManagement.messages.gameBananaIconApplied'));
        await refreshAfterMutation('structure');
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.setIconFailed', { error: String(error) }));
    }
};

const modPreviewIndices = reactive<Record<string, number>>({});
const subgroupPreviewMap = ref<Record<string, string[]>>(loadSubgroupPreviewCache());
const subgroupPreviewIndices = reactive<Record<string, number>>({});
const modAnalysisUnsubscribers = new Map<string, () => void>();

const clearModAnalysisSubscriptions = () => {
    modAnalysisUnsubscribers.forEach((unsubscribe) => unsubscribe());
    modAnalysisUnsubscribers.clear();
};

const applyAnalysisResultToMod = (mod: ModInfo, modKey: string, result: ModAnalysisResult) => {
    const excludedSet = new Set(result.excludedPreviewFileNames);
    const excludedStems = new Set(
        result.excludedPreviewFileNames
            .map((f) => f.replace(/\.[^.]+$/, ''))
            .filter((s) => s.length > 0)
    );
    const filteredPreviewImages = mod.previewImages.filter((imagePath) => {
        const fileNameLower = getPreviewFileNameLower(imagePath);
        if (excludedSet.has(fileNameLower)) return false;
        const stem = fileNameLower.replace(/\.[^.]+$/, '');
        return !excludedStems.has(stem);
    });
    const previewChanged = filteredPreviewImages.length !== mod.previewImages.length
        || filteredPreviewImages.some((imagePath, index) => imagePath !== mod.previewImages[index]);

    if (previewChanged) {
        mod.previewImages = filteredPreviewImages;
        const currentIndex = modPreviewIndices[mod.id] || 0;
        if (currentIndex >= filteredPreviewImages.length) {
            modPreviewIndices[mod.id] = 0;
        }
        if (selectedGame.value) {
            invalidateSubgroupPreviewByGroupId(selectedGame.value, mod.group);
        }
    }

    if (modKeyLoadingState[mod.id] || Object.prototype.hasOwnProperty.call(modKeyLists, mod.id) || result.modKeyList.length > 0) {
        modKeyLists[mod.id] = result.modKeyList;
        delete modKeyErrorState[mod.id];
        modKeyLoadingState[mod.id] = false;
    }

    if (!mods.value.some((item) => item.id === mod.id)) {
        const unsubscribe = modAnalysisUnsubscribers.get(modKey);
        if (unsubscribe) {
            unsubscribe();
            modAnalysisUnsubscribers.delete(modKey);
        }
    }
};

const {
    modKeyLists,
    modKeyLoadingState,
    modKeyErrorState,
    modKeyEditorDialog,
    resetModKeyPopoverState,
    getModKeyItems,
    getModKeyDisplayName,
    getModKeySectionTitle,
    addBindingInput,
    removeBindingInput,
    loadModKeyList,
    openModKeyEditor,
    saveModKeyEditor,
} = useModsManagementModKeys({
    selectedGame,
    mods,
    suppressFsRefresh,
    applyAnalysisResultToMod,
    t,
});

const inlineKeysDialog = reactive({
    visible: false,
    loading: false,
    applying: false,
    mod: null as ModInfo | null,
    choices: [] as InlineKeyChoice[],
    hasBackup: false,
});

const openInlineModKeys = async (mod: ModInfo) => {
    if (!selectedGame.value) return;
    inlineKeysDialog.visible = true;
    inlineKeysDialog.loading = true;
    inlineKeysDialog.mod = mod;
    inlineKeysDialog.choices = [];
    try {
        migotoIniService.invalidate(selectedGame.value, mod.relativePath);
        const analysis = await migotoIniService.load(selectedGame.value, mod);
        const result = await inspectInlineModKeys(selectedGame.value, mod, analysis.modKeyList);
        inlineKeysDialog.choices = result.choices;
        inlineKeysDialog.hasBackup = result.hasBackup;
        inlineKeyBackupState[mod.id] = result.hasBackup;
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.inspectInlineModKeysFailed', { error: String(error) }));
        inlineKeysDialog.visible = false;
    } finally {
        inlineKeysDialog.loading = false;
    }
};

const applyInlineKeysFromDialog = async (selected: Record<string, string>) => {
    const mod = inlineKeysDialog.mod;
    if (!mod || !selectedGame.value) return;
    inlineKeysDialog.applying = true;
    try {
        suppressFsRefresh(2200);
        await applyInlineModKeys(selectedGame.value, mod, inlineKeysDialog.choices, selected);
        migotoIniService.invalidate(selectedGame.value, mod.relativePath);
        delete modKeyLists[mod.id];
        inlineKeysDialog.hasBackup = true;
        inlineKeyBackupState[mod.id] = true;
        ElMessage.success(t('modsManagement.messages.inlineModKeysApplied'));
        inlineKeysDialog.visible = false;
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.inlineModKeysFailed', { error: String(error) }));
    } finally {
        inlineKeysDialog.applying = false;
    }
};

const restoreInlineKeysFromDialog = async () => {
    const mod = inlineKeysDialog.mod;
    if (!mod || !selectedGame.value) return;
    inlineKeysDialog.applying = true;
    try {
        suppressFsRefresh(2200);
        await restoreInlineModKeys(selectedGame.value, mod);
        migotoIniService.invalidate(selectedGame.value, mod.relativePath);
        delete modKeyLists[mod.id];
        inlineKeysDialog.hasBackup = false;
        inlineKeyBackupState[mod.id] = false;
        ElMessage.success(t('modsManagement.messages.inlineModKeysRestored'));
        inlineKeysDialog.visible = false;
    } catch (error) {
        ElMessage.error(t('modsManagement.messages.restoreInlineModKeysFailed', { error: String(error) }));
    } finally {
        inlineKeysDialog.applying = false;
    }
};

const queueVisibleModAnalysis = (modBatch: ModInfo[]) => {
    if (!selectedGame.value || modBatch.length === 0) return;
    const gameName = selectedGame.value;

    modBatch.forEach((mod) => {
        const modKey = `${gameName}:${mod.relativePath}`;
        if (!modAnalysisUnsubscribers.has(modKey)) {
            const unsubscribe = migotoIniService.subscribe(gameName, mod.relativePath, (snapshot) => {
                const liveMod = mods.value.find((item) => item.relativePath === mod.relativePath);
                if (!liveMod) {
                    return;
                }

                if (snapshot.status === 'ready' && snapshot.result) {
                    applyAnalysisResultToMod(liveMod, modKey, snapshot.result);
                    return;
                }

                if (snapshot.status === 'error') {
                    if (modKeyLoadingState[liveMod.id]) {
                        modKeyErrorState[liveMod.id] = snapshot.error || 'Mod ini analysis failed';
                        modKeyLoadingState[liveMod.id] = false;
                    }
                    console.warn('Mod ini analysis failed:', liveMod.relativePath, snapshot.error);
                }
            });

            modAnalysisUnsubscribers.set(modKey, unsubscribe);
        }
    });

    migotoIniService.prefetch(gameName, modBatch);
};

// ============================================================
// Previews composable — preview indices, subgroup cache, auto-switch
// ============================================================
const {
    getStableModUiId, getPreviewIndex, getPreviewUrl,
    nextPreview, prevPreview, setPreviewIndex,
    startAutoSwitch, stopAutoSwitch,
    invalidateSubgroupPreviewByGroupId, invalidateSubgroupPreviewByGame,
    getSubgroupPreviewUrl, preloadSubgroupPreviewImages,
    getGroupIcon, getGroupIconUrl,
    schedulePersistSubgroupPreviewCache,
    subgroupPreviewPersistTimer,
} = useModsManagementPreviews({
    selectedGame, mods,
    modPreviewIndices, subgroupPreviewMap, subgroupPreviewIndices,
    groupIconVersion, availableGroups,
});

</script>

<template>
    <div class="page-container mod-manager" :class="{ 'effects-paused': !shouldRunVisualEffects }">
    <div
        ref="workspaceRef"
        class="mod-manager-workspace"
        :class="{ 'is-resizing-columns': activePanelResize }"
        :style="workspaceColumnStyle"
        @click="closeContextMenu(); closeGroupContextMenu()"
        @contextmenu="closeContextMenu(); closeGroupContextMenu()"
    >
        <section class="mod-manager-fixed-panel mod-manager-toolbar-panel">
        <div class="tb-bar">
            <!-- Ambient glow -->
            <div class="tb-glow"></div>

            <div class="tb-inner">
                <div class="tb-left">
                    <div class="tb-search">
                        <svg class="tb-search-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="11" cy="11" r="8"/>
                            <line x1="21" y1="21" x2="16.65" y2="16.65"/>
                        </svg>
                        <input
                            v-model="searchQuery"
                            :placeholder="t('modsManagement.ui.searchModPlaceholder')"
                            class="tb-search-input"
                        />
                        <button v-if="searchQuery" class="tb-search-clear" @click="searchQuery = ''">
                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                <line x1="18" y1="6" x2="6" y2="18"/>
                                <line x1="6" y1="6" x2="18" y2="18"/>
                            </svg>
                        </button>
                    </div>
                </div>

                <!-- Breadcrumb path navigator -->
                <div class="tb-center">
                    <nav class="tb-breadcrumb" :aria-label="t('modsManagement.ui.upToParentCategory')">
                        <template v-for="(seg, idx) in groupBreadcrumbs" :key="seg.groupId">
                            <span v-if="idx > 0" class="tb-breadcrumb-sep">›</span>
                            <button
                                type="button"
                                class="tb-breadcrumb-item"
                                :class="{ active: seg.isLast }"
                                :disabled="seg.isLast"
                                @click="navigateGroupBreadcrumb(seg.groupId)"
                                :title="seg.isLast ? '' : seg.label"
                            >
                                <svg v-if="idx === 0" class="tb-breadcrumb-home" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>
                                    <polyline points="9 22 9 12 15 12 15 22"/>
                                </svg>
                                <span>{{ seg.label }}</span>
                            </button>
                        </template>
                    </nav>
                </div>

                <div class="tb-right">
                    <!-- View mode toggle -->
                    <div class="tb-segmented-group">
                        <button
                            type="button"
                            class="tb-seg-btn"
                            :class="{ active: viewMode === 'grid' }"
                            :title="t('modsManagement.ui.gridView')"
                            :aria-label="t('modsManagement.ui.gridView')"
                            @click="viewMode = 'grid'"
                        >
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/>
                            </svg>
                        </button>
                        <button
                            type="button"
                            class="tb-seg-btn"
                            :class="{ active: viewMode === 'list' }"
                            :title="t('modsManagement.ui.listView')"
                            :aria-label="t('modsManagement.ui.listView')"
                            @click="viewMode = 'list'"
                        >
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>
                            </svg>
                        </button>
                    </div>
                    <div class="tb-divider"></div>

                    <!-- Sort controls -->
                    <el-tooltip :content="t('modsManagement.ui.sortBy')" placement="bottom">
                        <el-select v-model="sortBy" class="tb-select" popper-class="gamebanana-select-popper" :aria-label="t('modsManagement.ui.sortBy')">
                            <el-option value="manual" :label="t('modsManagement.ui.sortManual')" />
                            <el-option value="name" :label="t('modsManagement.ui.sortByName')" />
                            <el-option value="modified" :label="t('modsManagement.ui.sortByModified')" />
                            <el-option value="tag" :label="t('modsManagement.ui.sortByTag')" />
                        </el-select>
                    </el-tooltip>
                    <el-tooltip v-if="sortBy === 'tag'" :content="t('modsManagement.ui.selectSortTag')" placement="bottom">
                        <el-select v-model="sortTagId" class="tb-select" popper-class="gamebanana-select-popper" :aria-label="t('modsManagement.ui.selectSortTag')">
                            <el-option value="" :label="t('modsManagement.ui.selectSortTag')" />
                            <el-option v-for="tag in tagDefinitions" :key="tag.id" :value="tag.id" :label="tag.name" />
                        </el-select>
                    </el-tooltip>
                    <button
                        type="button"
                        class="tb-btn tb-btn--icon tb-sort-order-btn"
                        :title="sortOrder === 'asc' ? t('modsManagement.ui.sortAscending') : t('modsManagement.ui.sortDescending')"
                        :aria-label="sortOrder === 'asc' ? t('modsManagement.ui.sortAscending') : t('modsManagement.ui.sortDescending')"
                        @click="sortOrder = sortOrder === 'asc' ? 'desc' : 'asc'"
                        v-show="sortBy !== 'manual'"
                    >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round">
                            <line v-if="sortOrder === 'asc'" x1="12" y1="5" x2="12" y2="19"/><polyline v-if="sortOrder === 'asc'" points="19 12 12 19 5 12"/>
                            <line v-if="sortOrder === 'desc'" x1="12" y1="19" x2="12" y2="5"/><polyline v-if="sortOrder === 'desc'" points="5 12 12 5 19 12"/>
                        </svg>
                    </button>
                    <div class="tb-divider"></div>
                    <el-radio-group v-model="appSettings.modsManagementBlurMode" class="tb-blur-mode" :aria-label="t('modsManagement.ui.blurNsfwPreviews')">
                        <el-radio-button value="all">{{ t('modsManagement.ui.blurAllImages') }}</el-radio-button>
                        <el-radio-button value="nsfw">{{ t('modsManagement.ui.blurNsfwImages') }}</el-radio-button>
                        <el-radio-button value="none">{{ t('modsManagement.ui.blurNoImages') }}</el-radio-button>
                    </el-radio-group>
                    <div class="tb-divider"></div>
                    <button
                        type="button"
                        class="tb-btn tb-btn--primary tb-btn--icon"
                        @click="pickInstallArchive"
                        :title="t('modsManagement.actions.importArchive')"
                        :aria-label="t('modsManagement.actions.importArchive')"
                    >
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="12" y1="5" x2="12" y2="19"/>
                            <line x1="5" y1="12" x2="19" y2="12"/>
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="tb-btn tb-btn--icon"
                        @click="pickInstallFolder"
                        :title="t('modsManagement.actions.importFolder')"
                        :aria-label="t('modsManagement.actions.importFolder')"
                    >
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                            <line x1="12" y1="11" x2="12" y2="17"/>
                            <line x1="9" y1="14" x2="15" y2="14"/>
                        </svg>
                    </button>
                    <div class="tb-divider"></div>
                    <button
                        type="button"
                        class="tb-btn tb-btn--icon"
                        @click="openCurrentGroupFolder"
                        :title="t('modsManagement.ui.openFolder')"
                        :aria-label="t('modsManagement.ui.openFolder')"
                    >
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="tb-btn tb-btn--icon"
                        :disabled="loading"
                        @click="fetchMods"
                        :title="t('modsManagement.ui.refreshMods')"
                    >
                        <svg
                            width="16" height="16" viewBox="0 0 24 24" fill="none"
                            stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
                            :class="{ 'tb-spin': loading }"
                        >
                            <polyline points="23 4 23 10 17 10"/>
                            <polyline points="1 20 1 14 7 14"/>
                            <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
                        </svg>
                    </button>
                </div>
            </div>
        </div>
        </section>

        <aside class="mod-manager-fixed-panel mod-manager-groups-panel">
        <div class="mod-manager-layout-content">
           <div class="sidebar"
               @click="closeContextMenu(); closeGroupContextMenu()"
               @dragenter.stop.prevent
               @dragover.stop.prevent
               @drop.stop.prevent>
            <!-- Ambient glow -->
            <div class="sd-glow"></div>

            <div class="sd-header">
                <div class="sd-header-left">
                    <span class="sd-title">{{ t('modsManagement.ui.categories') }}</span>
                </div>
                <div class="sd-header-actions">
                    <button
                        v-if="canCollapseSelectedGroup"
                        type="button"
                        class="sd-btn"
                        @click.stop="collapseSelectedGroup"
                        :title="t('modsManagement.actions.collapseSelected')"
                    >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="6 9 12 15 18 9"/>
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="sd-btn"
                        @click.stop="createNewGroup"
                        :title="t('modsManagement.actions.createCategory')"
                    >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="12" y1="5" x2="12" y2="19"/>
                            <line x1="5" y1="12" x2="19" y2="12"/>
                        </svg>
                    </button>
                </div>
            </div>
            <div ref="groupListRef" class="group-list glass-scrollbar">
                <el-tree
                    :key="groupTreeRenderKey"
                    ref="groupTreeRef"
                    lazy
                    :load="loadNode as any"
                    node-key="id"
                    :props="{ label: 'label', isLeaf: 'leaf' }"
                    :expand-on-click-node="false"
                    :current-node-key="sidebarSelectedGroup !== 'Root' && sidebarSelectedGroup !== 'All' ? sidebarSelectedGroup : undefined"
                    highlight-current
                    @node-click="handleTreeNodeClick"
                    @current-change="handleTreeCurrentChange"
                    @node-expand="onGroupExpand as any"
                    @node-collapse="onGroupCollapse as any"
                    class="group-tree"
                >
                    <template #default="{ node, data }">
                        <div class="custom-tree-node"
                            @contextmenu.prevent.stop="data.id !== MODS_TREE_ROOT_ID && showGroupContextMenu($event, data)"
                            @dragenter.stop.prevent="onDragEnter"
                            @dragover.stop.prevent="onDragOver"
                            @dragleave.stop="onDragLeave"
                            @drop.stop.prevent="onDrop($event, data.id)"
                            :data-group-id="data.id"
                            :data-parent-id="data.id === MODS_TREE_ROOT_ID ? '' : getGroupParent(data.id)"
                            :class="{ 'reorder-hover': groupHoverId === data.id, 'is-disabled': !data.enabled, 'is-directory-link': data.isDirectoryLink }"
                            @mousedown.stop="onGroupMouseDown($event, data.id)"
                        >
                            <div class="node-content">
                                <img v-if="data.icon" :src="getGroupIconUrl(data.icon)" class="tree-icon" />
                                <el-icon v-else class="tree-icon-placeholder"><Folder /></el-icon>
                                <span class="node-label" :title="node.label">{{ node.label }}</span>
                                  <el-tooltip :content="t('modsManagement.ui.categoryDisabledTooltip')" placement="right" :show-after="500" v-if="!data.enabled">
                                     <el-icon class="disabled-icon"><CircleClose /></el-icon>
                                </el-tooltip>
                            </div>
                            <!-- Count is lazily loaded or hidden -->
                            <span class="count" v-if="data.count !== undefined && data.count > 0">{{ data.count }}</span>
                        </div>
                    </template>
                </el-tree>
            </div>
        </div>
        </div>
        </aside>

        <div
            class="mod-manager-column-resizer mod-manager-groups-resizer"
            :class="{ active: activePanelResize === 'groups' }"
            role="separator"
            tabindex="0"
            aria-orientation="vertical"
            :aria-label="`${t('modsManagement.ui.categories')} / ${t('modsManagement.ui.mods')}`"
            :aria-valuemin="MIN_PANEL_WIDTHS.groups"
            :aria-valuemax="MAX_PANEL_WIDTHS.groups"
            :aria-valuenow="panelWidths.groups"
            @pointerdown.stop="startPanelResize('groups', $event)"
            @pointermove.stop="movePanelResize"
            @pointerup.stop="stopPanelResize"
            @pointercancel.stop="stopPanelResize"
            @lostpointercapture.stop="stopPanelResize"
            @dblclick.stop="resetPanelWidth('groups')"
            @keydown="onPanelResizeKeydown('groups', $event)"
        ></div>

        <main class="mod-manager-fixed-panel mod-manager-mods-panel">
        <div class="mod-manager-layout-content mod-manager-mods-content">
        <div class="mod-manager-panel-header">
            <div class="mod-manager-panel-title">
                <span>{{ t('modsManagement.ui.mods') }}</span>
                <span class="mod-manager-panel-count">{{ filteredMods.length }}</span>
            </div>
            <div class="mod-manager-panel-actions">
                <button
                    type="button"
                    class="mod-manager-panel-icon-btn"
                    :disabled="!canNavigateGroupBack"
                    :title="t('modsManagement.ui.backToPreviousCategory')"
                    :aria-label="t('modsManagement.ui.backToPreviousCategory')"
                    @click.stop="navigateBackGroup"
                >
                    <el-icon><Back /></el-icon>
                </button>
                <button
                    type="button"
                    class="mod-manager-panel-icon-btn"
                    :disabled="!canNavigateGroupForward"
                    :title="t('modsManagement.ui.forwardToNextCategory')"
                    :aria-label="t('modsManagement.ui.forwardToNextCategory')"
                    @click.stop="navigateForwardGroup"
                >
                    <el-icon><Right /></el-icon>
                </button>
                <button
                    type="button"
                    class="mod-manager-panel-icon-btn"
                    :disabled="!canNavigateGroupUp"
                    :title="t('modsManagement.ui.upToParentCategory')"
                    :aria-label="t('modsManagement.ui.upToParentCategory')"
                    @click.stop="navigateToParentGroup"
                >
                    <el-icon><Top /></el-icon>
                </button>
            </div>
        </div>
        <div class="mod-grid-container glass-scrollbar">
            <div v-if="switchingGroup" class="group-loading-hint glass-panel">
                {{ t('modsManagement.ui.loadingCurrentCategory') }}: {{ loadedModsCount }}/{{ totalModsCount || '?' }}
            </div>

            <div v-if="filteredMods.length === 0 && visibleSubGroups.length === 0" class="empty-state">
                <el-empty :description="emptyModsDescription">
                    <div class="empty-state-actions">
                        <button type="button" class="tb-btn tb-btn--primary tb-btn--icon" @click="pickInstallArchive" :title="t('modsManagement.actions.importArchive')" :aria-label="t('modsManagement.actions.importArchive')">
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
                        </button>
                        <button type="button" class="tb-btn tb-btn--icon" @click="pickInstallFolder" :title="t('modsManagement.actions.importFolder')" :aria-label="t('modsManagement.actions.importFolder')">
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/><line x1="12" y1="11" x2="12" y2="17"/><line x1="9" y1="14" x2="15" y2="14"/></svg>
                        </button>
                    </div>
                    <button type="button" class="tb-btn" @click="openCurrentGroupFolder">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                        {{ t('modsManagement.ui.openFolder') }}
                    </button>
                </el-empty>
            </div>
            
            <div v-else class="mod-content-sections">
                <!-- Subgroups: grid cards -->
                <TransitionGroup name="mod-entry" tag="div" class="mod-grid" v-if="viewMode === 'grid' && visibleSubGroups.length > 0">
                <div
                    v-for="group in visibleSubGroups"
                    :key="`group-${group.id}`"
                    class="subgroup-card glass-panel"
                    :class="{ 'is-disabled': isDisabledGroup(group), 'is-directory-link': group.isDirectoryLink, 'reorder-hover': groupHoverId === group.id }"
                    :data-group-id="group.id"
                    :data-parent-id="getGroupParent(group.id)"
                    @click="handleGroupClick({ id: group.id, name: group.name, path: group.path, enabled: group.enabled ?? true } as GroupInfo, { ensureExpanded: false, syncSidebar: false })"
                    @contextmenu.prevent.stop="showGroupContextMenu($event, group)"
                    @mousedown.stop="onGroupMouseDown($event, group.id)"
                >
                    <transition name="subgroup-bg-slide">
                        <div
                            v-if="getSubgroupPreviewUrl(group)"
                            :key="getSubgroupPreviewUrl(group)"
                            class="subgroup-bg-preview"
                            :class="{
                                'is-nsfw-blurred': appSettings.modsManagementBlurMode === 'all' || (appSettings.modsManagementBlurMode === 'nsfw' && isNsfwGroup(group)),
                                'can-hover-reveal': appSettings.revealBlurredImagesOnHover,
                            }"
                            :style="{ backgroundImage: `url(${getSubgroupPreviewUrl(group)})` }"
                        ></div>
                    </transition>
                    <div class="subgroup-header">
                        <div class="subgroup-icon-wrap">
                            <img v-if="group.iconPath" :src="getGroupIconUrl(group.iconPath)" class="subgroup-icon" />
                            <el-icon v-else class="subgroup-icon-placeholder"><Folder /></el-icon>
                        </div>
                        <el-switch
                            class="subgroup-toggle"
                            :model-value="group.enabled"
                            @change="toggleGroup(group)"
                            @click.stop
                            @mousedown.stop
                            inline-prompt
                            :active-text="t('modsManagement.common.on')"
                            :inactive-text="t('modsManagement.common.off')"
                            style="--el-switch-on-color: var(--el-color-success); --el-switch-off-color: var(--el-color-danger);"
                        />
                    </div>
                    <div class="subgroup-name" :title="group.name">
                        {{ group.name }}
                        <span v-if="isDisabledGroup(group)" class="disabled-badge">{{ t('modsManagement.ui.disabled') }}</span>
                    </div>
                    <div class="subgroup-path">{{ group.id }}</div>
                    <div class="subgroup-count">{{ t('modsManagement.ui.modCount') }}: {{ group.modCount ?? 0 }}</div>
                    <div v-if="getTagsForGroup(group).length > 0" class="subgroup-tags">
                        <span
                            v-for="tag in getTagsForGroup(group)"
                            :key="tag.id"
                            class="mod-tag-chip mod-tag-chip--small"
                            :style="getTagChipStyle(tag)"
                        >{{ tag.name }}</span>
                    </div>
                </div>
                </TransitionGroup>

                <!-- Grid view: mod cards -->
                <TransitionGroup v-if="viewMode === 'grid'" name="mod-entry" tag="div" class="mod-grid">
                <ModCard
                    v-for="mod in filteredMods"
                    :key="`mod-${getStableModUiId(mod)}`"
                    :mod="mod"
                    :is-drag-over="dragOverId === mod.id"
                    :is-pulsing="!!modPulseState[mod.id]"
                    :dynamic-style="getModDynamicStyle(mod)"
                    :preview-url="getPreviewUrl(mod)"
                    :preview-index="getPreviewIndex(mod)"
                    :has-multiple-images="!!(mod.previewImages && mod.previewImages.length > 1)"
                    :tags="getTagsForMod(mod)"
                    :key-items="getModKeyItems(mod.id)"
                    :group-icon-url="(mod.group !== 'Root' && getGroupIcon(mod.group)) ? getGroupIconUrl((getGroupIcon(mod.group) as string)) : ''"
                    :group-display-name="mod.group !== 'Root' ? (searchQuery.trim() ? mod.group : (mod.group.split('/').pop() ?? '')) : ''"
                    :is-root-group="mod.group === 'Root'"
                    :blur-nsfw-preview="appSettings.modsManagementBlurMode === 'all' || (appSettings.modsManagementBlurMode === 'nsfw' && isNsfwMod(mod))"
                    @contextmenu="showModContextMenu($event, mod)"
                    @card-mousedown="onCardMouseDownWrapper($event, mod)"
                    @mousemove="onModCardMouseMove"
                    @mouseleave="onModCardMouseLeave"
                    @dblclick="openModFolder(mod.path)"
                    @open-tag-dialog="openModTagDialog(mod)"
                    @show-key-floater="showKeyFloater(mod, $event)"
                    @hide-key-floater="hideKeyFloater"
                    @edit-keys="openModKeyEditor(mod)"
                    @prev-preview="prevPreview(mod)"
                    @next-preview="nextPreview(mod)"
                    @set-preview-index="(idx: number) => setPreviewIndex(mod, idx)"
                    @toggle="toggleMod(mod)"
                    @open-presets="openPresetPopover(mod, $event)"
                    :preset-name="getActivePresetName(mod)"
                />
                </TransitionGroup>

                <!-- List view -->
                <div v-else class="mod-list glass-panel">
                <div class="mod-list-header">
                    <span class="mod-list-hd mod-list-hd--preview"></span>
                    <span class="mod-list-hd mod-list-hd--name">{{ t('modsManagement.fields.modName') }}</span>
                    <span class="mod-list-hd mod-list-hd--status">{{ t('modsManagement.common.on') }} / {{ t('modsManagement.common.off') }}</span>
                    <span class="mod-list-hd mod-list-hd--tags">{{ t('modsManagement.ui.tags') }}</span>
                    <span class="mod-list-hd mod-list-hd--group">{{ t('modsManagement.fields.groupCharacter') }}</span>
                    <span class="mod-list-hd mod-list-hd--modified">{{ t('modsManagement.ui.sortByModified') }}</span>
                </div>

                <!-- List view: subgroup rows -->
                <TransitionGroup name="mod-entry" tag="div" class="mod-list-transition">
                <div
                    v-for="group in visibleSubGroups"
                    :key="`group-list-${group.id}`"
                    class="mod-list-row mod-list-row--group"
                    :class="{ 'is-disabled': isDisabledGroup(group), 'is-directory-link': group.isDirectoryLink }"
                    :data-group-id="group.id"
                    @click="handleGroupClick({ id: group.id, name: group.name, path: group.path, enabled: group.enabled ?? true } as GroupInfo, { ensureExpanded: false, syncSidebar: false })"
                    @contextmenu.prevent.stop="showGroupContextMenu($event, group)"
                >
                    <span class="mod-list-cell mod-list-cell--preview">
                        <div class="mod-list-thumb mod-list-thumb--group" :class="{ active: group.enabled }">
                            <img v-if="group.iconPath" :src="getGroupIconUrl(group.iconPath)" class="mod-list-thumb-img" />
                            <el-icon v-else class="mod-list-thumb-fallback"><Folder /></el-icon>
                        </div>
                    </span>
                    <span class="mod-list-cell mod-list-cell--name" :title="group.name">
                        <span class="mod-list-group-name">{{ group.name }}</span>
                        <span v-if="isDisabledGroup(group)" class="disabled-badge">{{ t('modsManagement.ui.disabled') }}</span>
                    </span>
                    <span class="mod-list-cell mod-list-cell--status" @click.stop>
                        <el-switch
                            :model-value="group.enabled"
                            @change="toggleGroup(group)"
                            size="small"
                            inline-prompt
                            :active-text="t('modsManagement.common.on')"
                            :inactive-text="t('modsManagement.common.off')"
                            style="--el-switch-on-color: var(--el-color-success); --el-switch-off-color: var(--el-color-danger);"
                        />
                    </span>
                    <span class="mod-list-cell mod-list-cell--tags">
                        <span
                            v-for="tag in getTagsForGroup(group)"
                            :key="tag.id"
                            class="mod-tag-chip mod-tag-chip--small"
                            :style="getTagChipStyle(tag)"
                        >{{ tag.name }}</span>
                    </span>
                    <span class="mod-list-cell mod-list-cell--group">{{ t('modsManagement.ui.modCount') }}: {{ group.modCount ?? 0 }}</span>
                    <span class="mod-list-cell mod-list-cell--modified">—</span>
                </div>
                </TransitionGroup>

                <!-- List view: mod rows -->
                <TransitionGroup name="mod-entry" tag="div" class="mod-list-transition">
                <div
                    v-for="mod in filteredMods"
                    :key="`mod-list-${getStableModUiId(mod)}`"
                    class="mod-list-row"
                    :class="{ 'is-disabled': !mod.enabled, 'is-directory-link': mod.isDirectoryLink, 'reorder-hover': dragOverId === mod.id, 'state-pulse': !!modPulseState[mod.id] }"
                    :data-mod-id="mod.id"
                    @contextmenu.prevent.stop="showModContextMenu($event, mod)"
                    @dblclick.stop="openModFolder(mod.path)"
                    @mousedown="onCardMouseDownWrapper($event, mod)"
                >
                    <!-- Preview thumbnail -->
                    <span class="mod-list-cell mod-list-cell--preview">
                        <div class="mod-list-thumb" :class="{ active: mod.enabled }">
                            <el-image
                                v-if="getPreviewUrl(mod)"
                                :src="getPreviewUrl(mod)"
                                fit="cover"
                                loading="lazy"
                                class="mod-list-thumb-img"
                                :class="{ 'is-nsfw-blurred': appSettings.modsManagementBlurMode === 'all' || (appSettings.modsManagementBlurMode === 'nsfw' && isNsfwMod(mod)), 'can-hover-reveal': appSettings.revealBlurredImagesOnHover }"
                            >
                                <template #error>
                                    <span class="mod-list-thumb-fallback">{{ mod.name.charAt(0) }}</span>
                                </template>
                            </el-image>
                            <span v-else class="mod-list-thumb-fallback">{{ mod.name.charAt(0) }}</span>
                            <span v-if="appSettings.modsManagementBlurMode === 'nsfw' && isNsfwMod(mod)" class="mod-list-nsfw-shield">NSFW</span>
                        </div>
                    </span>
                    <!-- Name -->
                    <span class="mod-list-cell mod-list-cell--name" :title="mod.name">
                        {{ mod.name }}
                    </span>
                    <!-- Enabled toggle -->
                    <span class="mod-list-cell mod-list-cell--status" @click.stop>
                        <el-switch
                            :model-value="mod.enabled"
                            @change="toggleMod(mod)"
                            size="small"
                            inline-prompt
                            :active-text="t('modsManagement.common.on')"
                            :inactive-text="t('modsManagement.common.off')"
                            style="--el-switch-on-color: var(--el-color-success); --el-switch-off-color: var(--el-color-danger);"
                        />
                    </span>
                    <!-- Tags -->
                    <span class="mod-list-cell mod-list-cell--tags">
                        <span
                            v-for="tag in getTagsForMod(mod)"
                            :key="tag.id"
                            class="mod-tag-chip mod-tag-chip--small"
                            :style="getTagChipStyle(tag)"
                        >{{ tag.name }}</span>
                        <button
                            type="button"
                            class="mod-list-tag-btn"
                            :title="t('modsManagement.actions.editModTags')"
                            @click.stop="openModTagDialog(mod)"
                        >
                            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
                        </button>
                    </span>
                    <!-- Group -->
                    <span class="mod-list-cell mod-list-cell--group" :title="mod.group">
                        {{ mod.group !== 'Root' ? (searchQuery.trim() ? mod.group : (mod.group.split('/').pop() ?? mod.group)) : '—' }}
                    </span>
                    <!-- Last modified -->
                    <span class="mod-list-cell mod-list-cell--modified">
                        {{ mod.lastModified ? new Date(mod.lastModified * 1000).toLocaleDateString() : '—' }}
                    </span>
                </div>
                </TransitionGroup>
                </div>
            </div>
        </div>
        </div>
        </main>

        <div
            class="mod-manager-column-resizer mod-manager-tags-resizer"
            :class="{ active: activePanelResize === 'tags' }"
            role="separator"
            tabindex="0"
            aria-orientation="vertical"
            :aria-label="`${t('modsManagement.ui.mods')} / ${t('modsManagement.ui.tags')}`"
            :aria-valuemin="MIN_PANEL_WIDTHS.tags"
            :aria-valuemax="MAX_PANEL_WIDTHS.tags"
            :aria-valuenow="panelWidths.tags"
            @pointerdown.stop="startPanelResize('tags', $event)"
            @pointermove.stop="movePanelResize"
            @pointerup.stop="stopPanelResize"
            @pointercancel.stop="stopPanelResize"
            @lostpointercapture.stop="stopPanelResize"
            @dblclick.stop="resetPanelWidth('tags')"
            @keydown="onPanelResizeKeydown('tags', $event)"
        ></div>

        <aside class="mod-manager-fixed-panel mod-manager-tags-panel">
        <div class="mod-manager-tags-layout-content">
        <TagManagement
          :tag-definitions="tagDefinitions"
          :active-tag-ids="activeTagIds"
          :active-tags="activeTags"
          :has-active-tag-filter="hasActiveTagFilter"
          :tag-management-dialog="tagManagementDialog"
          :mod-tag-dialog="modTagDialog"
          :get-tag-icon-url="getTagIconUrl"
          :get-tag-chip-style="getTagChipStyle"
          :get-tag-usage-count="getTagUsageCount"
          :get-draft-tag-icon-preview-url="getDraftTagIconPreviewUrl"
          :get-editing-tag="getEditingTag"
          @toggle-active-tag="toggleActiveTag"
          @clear-active-tags="clearActiveTags"
          @save-tag-definition="saveTagDefinition"
          @edit-tag-definition="editTagDefinition"
          @delete-tag-definition="deleteTagDefinition"
          @reset-tag-management-form="resetTagManagementForm"
          @pick-tag-icon-source="pickTagIconSource"
          @open-mod-tag-dialog="(mod?: ModInfo) => { if (mod) openModTagDialogWrapped(mod) }"
          @save-mod-tag-assignments="saveTagAssignmentsForCurrentTarget"
          @open-tag-management-from-mod-tag="() => { openTagManagementDialog(); modTagDialog.visible = false; }"
          @close-mod-tag-dialog="modTagDialog.visible = false"
        />
        </div>
        </aside>
    </div>

    <!-- Edit Switch Key List -->
    <EditSwitchKeyList
      :visible="modKeyEditorDialog.visible"
      :loading="modKeyEditorDialog.loading"
      :saving="modKeyEditorDialog.saving"
      :mod-name="modKeyEditorDialog.modName"
      :items="modKeyEditorDialog.items"
      :get-mod-key-section-title="getModKeySectionTitle"
      @close="modKeyEditorDialog.visible = false"
      @save="saveModKeyEditor"
      @add-binding-input="addBindingInput"
      @remove-binding-input="removeBindingInput"
      @add-back-binding-input="(values: string[]) => addBindingInput(values)"
      @remove-back-binding-input="(values: string[], index: number) => removeBindingInput(values, index)"
    />

    <!-- Key Floater (floating card next to K badge) -->
    <Teleport to="body">
      <Transition name="kf-fade">
        <div
          v-if="hoveredKeyModId && getModKeyItems(hoveredKeyModId).length"
          class="key-floater"
          :style="{ left: keyListPos.x + 'px', top: keyListPos.y + 'px' }"
          @mouseenter="keepKeyFloater"
          @mouseleave="hideKeyFloater"
        >
          <div class="kf-header">
            <span class="kf-title">{{ t('modsManagement.ui.modKeyListTitle') }}</span>
          </div>
          <div class="kf-body glass-scrollbar--thin">
            <div
              v-for="(item, kidx) in getModKeyItems(hoveredKeyModId)"
              :key="kidx"
              class="kf-item"
            >
              <div class="kf-item-info">
                <div class="kf-item-name">{{ getModKeyDisplayName(item) }}</div>
                <div class="kf-item-desc">{{ getModKeySectionTitle(item) }}</div>
              </div>
              <div v-if="item.keyType" class="kf-type">{{ item.keyType }}</div>
            </div>
          </div>
          <div class="kf-footer">
            <button class="kf-edit-btn" @click.stop="openModKeyEditorForCurrentHover">
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 5px;">
                <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
                <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
              </svg>
              {{ t('modsManagement.actions.editModKeys') }}
            </button>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Install Dialog -->
    <InstallModDialog
      :visible="showInstallDialog"
      :form="installForm"
      :preview="installPreview"
      :progress="installProgress"
      :installing="isInstalling"
      :group-options="groups.map(g => g.id)"
      :tag-definitions="tagDefinitions"
      @cancel="cancelInstall"
      @confirm="confirmInstall"
    />

    <el-dialog
        v-model="exportArchiveDialog.visible"
        :title="t('modsManagement.dialog.exportArchiveTitle')"
        width="460px"
        align-center
        append-to-body
        custom-class="export-archive-dialog"
        :close-on-click-modal="!exportArchiveDialog.exporting"
        :close-on-press-escape="!exportArchiveDialog.exporting"
        :show-close="!exportArchiveDialog.exporting"
    >
        <el-form label-width="110px" :model="exportArchiveDialog">
            <el-form-item :label="t('modsManagement.fields.modName')">
                <el-input :model-value="exportArchiveDialog.modName" disabled />
            </el-form-item>
            <el-form-item :label="t('modsManagement.fields.archiveName')">
                <el-input
                    v-model="exportArchiveDialog.archiveName"
                    :placeholder="t('modsManagement.placeholders.archiveName')"
                    :disabled="exportArchiveDialog.exporting"
                />
            </el-form-item>
            <el-form-item :label="t('modsManagement.fields.outputFolder')">
                <div class="export-output-row">
                    <el-input
                        v-model="exportArchiveDialog.outputDir"
                        :placeholder="t('modsManagement.placeholders.outputFolder')"
                        readonly
                        :disabled="exportArchiveDialog.exporting"
                    />
                    <button type="button" class="tb-btn" @click="chooseExportOutputDir" :disabled="exportArchiveDialog.exporting">
                        {{ t('modsManagement.actions.chooseFolder') }}
                    </button>
                </div>
            </el-form-item>
            <el-form-item :label="t('modsManagement.fields.archivePassword')">
                <el-input
                    v-model="exportArchiveDialog.password"
                    type="password"
                    show-password
                    :placeholder="t('modsManagement.placeholders.archivePasswordOptional')"
                    :disabled="exportArchiveDialog.exporting"
                />
            </el-form-item>
            <el-form-item :label="t('modsManagement.fields.archiveFormat')">
                <el-select v-model="exportArchiveDialog.format" :disabled="exportArchiveDialog.exporting" style="width: 100%">
                    <el-option
                        v-for="format in archiveExportFormats"
                        :key="format"
                        :label="format.toUpperCase()"
                        :value="format"
                    />
                </el-select>
            </el-form-item>
        </el-form>
        <template #footer>
            <span class="dialog-footer">
                <button type="button" class="tb-btn" @click="cancelExportArchive" :disabled="exportArchiveDialog.exporting">{{ t('modsManagement.common.cancel') }}</button>
                <button type="button" class="tb-btn tb-btn--primary" @click="confirmExportArchive" :disabled="exportArchiveDialog.exporting">
                    {{ exportArchiveDialog.exporting ? t('modsManagement.progress.exporting') : t('modsManagement.common.confirm') }}
                </button>
            </span>
        </template>
    </el-dialog>

    <!-- Sub Group Dialog -->
    <el-dialog v-model="subGroupDialog.visible" :title="t('modsManagement.dialog.createSubcategoryTitle')" width="420px" align-center custom-class="glass-dialog">
        <el-form label-width="90px">
            <el-form-item :label="t('modsManagement.fields.name')">
                <el-input v-model="subGroupDialog.name" :placeholder="t('modsManagement.placeholders.subcategoryName')" />
            </el-form-item>
            <el-form-item :label="t('modsManagement.fields.iconOptional')">
                <div class="subgroup-icon-row">
                    <el-input v-model="subGroupDialog.icon" :placeholder="t('modsManagement.ui.notSelected')" readonly />
                    <button type="button" class="tb-btn" @click="pickSubGroupIcon">{{ t('modsManagement.actions.chooseIcon') }}</button>
                </div>
            </el-form-item>
        </el-form>
        <template #footer>
            <span class="dialog-footer">
                <button type="button" class="tb-btn" @click="subGroupDialog.visible = false">{{ t('modsManagement.common.cancel') }}</button>
                <button type="button" class="tb-btn tb-btn--primary" @click="confirmSubGroup">{{ t('modsManagement.common.confirm') }}</button>
            </span>
        </template>
    </el-dialog>



    <InlineModKeysDialog
      :visible="inlineKeysDialog.visible"
      :loading="inlineKeysDialog.loading"
      :applying="inlineKeysDialog.applying"
      :mod-name="inlineKeysDialog.mod?.name || ''"
      :choices="inlineKeysDialog.choices"
      :has-backup="inlineKeysDialog.hasBackup"
      @close="inlineKeysDialog.visible = false"
      @apply="applyInlineKeysFromDialog"
      @restore="restoreInlineKeysFromDialog"
    />

    <Teleport to="body">
      <Transition name="mod-3d-floating-fade">
        <div v-if="floating3DPreviewMod" v-show="floating3DPreviewOpen" class="mod-3d-floating-overlay" @click.self="closeFloating3DPreview">
          <section ref="floating3DPreviewPage" class="mod-3d-floating-page" role="dialog" aria-modal="true" tabindex="-1" @keydown.esc="closeFloating3DPreview">
            <header class="mod-3d-floating-header">
              <div>
                <strong>3D 预览</strong>
                <span>{{ floating3DPreviewMod.name }}</span>
              </div>
              <button type="button" title="关闭" @click="closeFloating3DPreview">×</button>
            </header>
            <Mod3DPreview
              v-for="cachedMod in floating3DPreviewCache"
              v-show="cachedMod.path.toLowerCase() === floating3DPreviewMod.path.toLowerCase()"
              :key="cachedMod.path.toLowerCase()"
              :mod="cachedMod"
            />
          </section>
        </div>
      </Transition>
    </Teleport>

    <!-- Custom Context Menu -->
    <ContextMenu
      :visible="contextMenu.visible"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :target="contextMenu.target"
      :groups="groups"
      :is-nsfw="contextMenu.target ? getTagsForMod(contextMenu.target).some(isNsfwTag) : false"
      :current-group="selectedGroup"
      :has-mod-keys="contextMenu.target ? getModKeyItems(contextMenu.target.id).length > 0 : false"
      :has-inline-key-backup="contextMenu.target ? !!inlineKeyBackupState[contextMenu.target.id] : false"
      @close="closeContextMenu"
      @open-3d-preview="(mod: ModInfo) => { closeContextMenu(); openFloating3DPreview(mod); }"
      @open-mod-folder="(path: string) => { closeContextMenu(); openModFolder(path); }"
      @move-mod-to-group="(mod: ModInfo, groupId: string) => { closeContextMenu(); moveModToGroup(mod, groupId); }"
      @create-new-group="closeContextMenu(); createNewGroup()"
      @rename-mod="(mod: ModInfo) => { closeContextMenu(); renameMod(mod); }"
      @export-mod-archive="(mod: ModInfo) => { closeContextMenu(); openExportArchiveDialog(mod); }"
      @open-mod-tag-dialog="(mod: ModInfo) => { closeContextMenu(); openModTagDialogWrapped(mod); }"
      @edit-mod-keys="(mod: ModInfo) => { closeContextMenu(); openModKeyEditor(mod); }"
      @inline-mod-keys="(mod: ModInfo) => { closeContextMenu(); void openInlineModKeys(mod); }"
      @restore-inline-mod-keys="(mod: ModInfo) => { closeContextMenu(); void openInlineModKeys(mod); }"
      @toggle-nsfw="(mod: ModInfo) => { closeContextMenu(); void toggleModNsfwMarked(mod); }"
      @add-preview-images="(mod: ModInfo) => { closeContextMenu(); addPreviewImages(mod); }"
      @paste-clipboard-preview-image="(mod: ModInfo) => { closeContextMenu(); pasteClipboardPreviewImage(mod); }"
      @enable-mod-solo="(mod: ModInfo) => { closeContextMenu(); enableModSolo(mod); }"
      @delete-mod="(mod: ModInfo) => { closeContextMenu(); deleteMod(mod); }"
      @navigate-to-mod-group="(mod: ModInfo) => { closeContextMenu(); void navigateToModGroup(mod); }"
    />

    <!-- Group Context Menu -->
    <GroupContextMenu
      :visible="groupContextMenu.visible"
      :x="groupContextMenu.x"
      :y="groupContextMenu.y"
      :target="groupContextMenu.target"
      :is-nsfw="groupContextMenu.target ? isNsfwGroupMarked(groupContextMenu.target) : false"
      @close="closeGroupContextMenu"
      @toggle-group="(group: GroupInfo) => { closeGroupContextMenu(); toggleGroup(group); }"
      @open-mod-group-folder="(group: GroupInfo) => { closeGroupContextMenu(); openModGroupFolder(group); }"
      @open-sub-group-dialog="(group: GroupInfo) => { closeGroupContextMenu(); openSubGroupDialog(group.path || group.id); }"
      @set-group-icon="(group: GroupInfo) => { closeGroupContextMenu(); setGroupIcon(group); }"
      @auto-match-gamebanana-icon="(group: GroupInfo) => { closeGroupContextMenu(); openGameBananaIconDialog(group, 'auto'); }"
      @bind-gamebanana-category="(group: GroupInfo) => { closeGroupContextMenu(); openGameBananaIconDialog(group, 'manual'); }"
      @rename-group="(group: GroupInfo) => { closeGroupContextMenu(); renameGroup(group); }"
      @delete-group="(group: GroupInfo) => { closeGroupContextMenu(); deleteGroup(group); }"
      @edit-group-tags="(group: GroupInfo) => { closeGroupContextMenu(); openGroupTagDialogWrapped(group); }"
      @toggle-nsfw="(group: GroupInfo) => { closeGroupContextMenu(); void toggleGroupNsfwMarked(group); }"
      @convert-group-to-mod="(group: GroupInfo) => { closeGroupContextMenu(); convertGroupToMod(group); }"
    />

    <GameBananaCategoryIconDialog
      :visible="gameBananaIconDialog.visible"
      :group="gameBananaIconDialog.group"
      :game-name="selectedGame"
      :mode="gameBananaIconDialog.mode"
      @close="gameBananaIconDialog.visible = false"
      @apply="applyGameBananaGroupIcon"
    />

    <ModPresetPopover
      :visible="presetPopoverVisible"
      :x="presetPopoverX"
      :y="presetPopoverY"
      :presets="presetTargetMod ? getPresetData(presetTargetMod) : { version: 1, presets: [] }"
      :mod-name="presetTargetMod?.name ?? ''"
      :game-name="selectedGame"
      :mod-relative-path="presetTargetMod?.relativePath ?? ''"
      :mod-path="presetTargetMod?.path ?? ''"
      @close="closePresetPopover"
      @apply-preset="onApplyPreset"
      @save-current="onSaveCurrentAsPreset"
      @delete-preset="onDeletePreset"
      @rename-preset="onRenamePreset"
      @reset-current="onResetPresetState"
    />
  </div>
</template>

<style scoped>
.page-container.mod-manager {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 0;
    overflow: hidden;
    -webkit-app-region: no-drag; /* Ensure content is not treated as window drag area */
}

.mod-manager-workspace {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    display: grid;
    grid-template-areas:
        "toolbar toolbar toolbar toolbar toolbar"
        "groups groups-resizer mods tags-resizer tags";
    grid-template-columns: var(--groups-panel-width, 250px) 10px minmax(0, 1fr) 10px var(--tags-panel-width, 290px);
    grid-template-rows: auto minmax(0, 1fr);
    column-gap: 0;
    row-gap: 10px;
    padding: 10px;
    box-sizing: border-box;
    overflow: hidden;
    background:
        radial-gradient(circle at 1px 1px, rgba(255,255,255,0.055) 1px, transparent 0) 0 0 / 24px 24px,
        rgba(3, 6, 11, 0.32);
    font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
}

.mod-manager-fixed-panel {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border-radius: 10px;
}

.mod-manager-toolbar-panel {
    grid-area: toolbar;
    overflow: visible;
}

.mod-manager-groups-panel {
    grid-area: groups;
}

.mod-manager-mods-panel {
    grid-area: mods;
}

.mod-manager-tags-panel {
    grid-area: tags;
}

.mod-manager-column-resizer {
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

.mod-manager-groups-resizer {
    grid-area: groups-resizer;
}

.mod-manager-tags-resizer {
    grid-area: tags-resizer;
}

.mod-manager-column-resizer::before {
    content: '';
    position: absolute;
    top: 12px;
    bottom: 12px;
    left: 4px;
    width: 2px;
    border-radius: 999px;
    background: rgba(255,255,255,0.10);
    transition: background 0.16s ease, box-shadow 0.16s ease, transform 0.16s ease;
}

.mod-manager-column-resizer:hover::before,
.mod-manager-column-resizer:focus-visible::before,
.mod-manager-column-resizer.active::before {
    background: rgba(var(--theme-surface-tint-rgb), 0.78);
    box-shadow: 0 0 10px rgba(var(--theme-surface-tint-rgb), 0.42);
    transform: scaleX(1.5);
}

.mod-manager-column-resizer:focus-visible {
    border-radius: 6px;
    box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.48);
}

.mod-manager-workspace.is-resizing-columns,
.mod-manager-workspace.is-resizing-columns * {
    cursor: col-resize !important;
    user-select: none !important;
}

.mod-manager-layout-content {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden !important;
    box-sizing: border-box;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.12);
    background: rgba(255,255,255,0.045);
    box-shadow: 0 8px 24px rgba(0,0,0,0.12), 0 0 0 1px rgba(255,255,255,0.04) inset;
}

.mod-manager-workspace button,
.mod-manager-workspace input,
.mod-manager-workspace textarea,
.mod-manager-workspace select,
.mod-manager-workspace code {
    font-family: inherit;
}

.mod-manager-workspace :deep(.el-button),
.mod-manager-workspace :deep(.el-input__inner),
.mod-manager-workspace :deep(.el-select__selected-item),
.mod-manager-workspace :deep(.el-select__placeholder),
.mod-manager-workspace :deep(.el-tag),
.mod-manager-workspace :deep(.el-empty__description) {
    font-family: inherit;
}

.mod-manager-toolbar-panel .tb-bar {
    border: 1px solid rgba(255,255,255,0.12);
    border-radius: 10px;
}

.mod-manager-panel-header {
    position: relative;
    z-index: 4;
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-bottom: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.035);
    color: rgba(255,255,255,0.80);
    font-size: 12px;
    font-weight: 700;
}

.mod-manager-panel-title {
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
}

.mod-manager-panel-count {
    min-width: 26px;
    height: 20px;
    padding: 0 7px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background: rgba(255,255,255,0.08);
    color: rgba(255,255,255,0.72);
    font-size: 11px;
}

.mod-manager-panel-actions {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
}

.mod-manager-panel-icon-btn {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.08);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.45);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.2s ease;
}

.mod-manager-panel-icon-btn:hover:not(:disabled) {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.18);
    color: rgba(255,255,255,0.80);
    transform: scale(1.06);
}

.mod-manager-panel-icon-btn:active:not(:disabled) {
    transform: scale(0.96);
}

.mod-manager-panel-icon-btn:disabled {
    cursor: default;
    opacity: 0.36;
}

.mod-manager-panel-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
}

.mod-manager-tags-layout-content {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden !important;
}

.mod-manager-layout-content .sidebar {
    width: 100%;
    flex: 1;
    min-height: 0;
    border-right: 0;
    border-radius: 8px;
}

.mod-manager-mods-content .mod-grid-container {
    flex: 1;
    min-height: 0;
}

.empty-state-actions {
    display: flex;
    gap: 12px;
    justify-content: center;
    margin-bottom: 12px;
    flex-wrap: wrap;
}

/* Glass Panel Utility �� White Sci-Fi Glass */
.glass-panel {
    background: rgba(255,255,255,0.06);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    border: 1px solid rgba(255,255,255,0.12);
    box-shadow: 0 4px 12px rgba(0,0,0,0.15), 0 0 0 1px rgba(255,255,255,0.06) inset;
}

/* Toolbar — White Sci-Fi Glass (SwitchKeyList style) */
.toolbar-stack {
    position: relative;
    z-index: 24;
    flex-shrink: 0;
}

.tb-bar {
    position: relative;
    overflow: hidden;
    background: rgba(255,255,255,0.06);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    border-bottom: 1px solid rgba(255,255,255,0.12);
    box-shadow: 0 4px 12px rgba(0,0,0,0.15), 0 0 0 1px rgba(255,255,255,0.06) inset;
}

/* Top hairline */
.tb-bar::before {
    content: ''; position: absolute;
    top: 0; left: 0; right: 0; height: 1px;
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.18), transparent);
    pointer-events: none; z-index: 1;
}

/* Dot pattern overlay */
.tb-bar::after {
    content: ''; position: absolute; inset: 0;
    pointer-events: none; z-index: 0;
    background-image: radial-gradient(circle, rgba(255,255,255,0.04) 1px, transparent 1px);
    background-size: 20px 20px;
}

/* Ambient glow */
.tb-glow {
    position: absolute;
    top: -60%; right: -10%; width: 40%; height: 120%;
    background: radial-gradient(ellipse, rgba(255,255,255,0.10), transparent 70%);
    pointer-events: none; z-index: 0;
}

.tb-inner {
    position: relative; z-index: 1;
    padding: 10px clamp(18px, 2vw, 28px);
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 24px;
}

.tb-left, .tb-center, .tb-right {
    display: flex;
    align-items: center;
}

.tb-left {
    flex: 0 0 auto;
    min-width: 0;
}

.tb-center {
    flex: 1 1 auto;
    min-width: 0;
    justify-content: flex-start;
    overflow: hidden;
}

.tb-right {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 4px;
}

/* ---- Search Box ---- */
.tb-search {
    position: relative;
    display: flex;
    align-items: center;
    width: 200px;
}
.tb-search-icon {
    position: absolute; left: 12px;
    color: rgba(255,255,255,0.30);
    pointer-events: none; flex-shrink: 0;
}
.tb-search-input {
    width: 100%; height: 34px;
    padding: 0 34px 0 36px;
    border-radius: 10px;
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.80);
    font-size: 13px;
    outline: none;
    transition: all 0.2s ease;
}
.tb-search-input::placeholder { color: rgba(255,255,255,0.28); }
.tb-search-input:focus {
    border-color: rgba(255,255,255,0.25);
    background: rgba(255,255,255,0.07);
    box-shadow: 0 0 0 3px rgba(255,255,255,0.04);
}
.tb-search-clear {
    position: absolute; right: 6px;
    width: 24px; height: 24px;
    border: none; border-radius: 6px;
    background: transparent;
    color: rgba(255,255,255,0.25);
    display: flex; align-items: center; justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
    z-index: 1;
}
.tb-search-clear:hover {
    background: rgba(255,255,255,0.08);
    color: rgba(255,255,255,0.60);
}

/* ---- Breadcrumb Path Navigator ---- */
.tb-breadcrumb {
    display: flex;
    align-items: center;
    gap: 2px;
    overflow: hidden;
    white-space: nowrap;
}
.tb-breadcrumb-sep {
    color: rgba(255,255,255,0.20);
    font-size: 15px;
    font-weight: 300;
    flex-shrink: 0;
    margin: 0 1px;
    user-select: none;
}
.tb-breadcrumb-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 9px;
    border-radius: 8px;
    border: none;
    background: transparent;
    color: rgba(255,255,255,0.50);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
    flex-shrink: 0;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.tb-breadcrumb-item:hover:not(:disabled) {
    background: rgba(255,255,255,0.08);
    color: rgba(255,255,255,0.85);
}
.tb-breadcrumb-item.active {
    color: rgba(255,255,255,0.90);
    font-weight: 600;
    cursor: default;
}
.tb-breadcrumb-item:disabled {
    cursor: default;
    opacity: 1;
}
.tb-breadcrumb-home {
    flex-shrink: 0;
    opacity: 0.6;
}

/* ---- Toolbar Buttons ---- */
.tb-btn {
    height: 36px; border-radius: 10px;
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.70);
    font-size: 12px; font-weight: 600; letter-spacing: 0.3px;
    display: inline-flex; align-items: center; gap: 6px;
    padding: 0 14px;
    cursor: pointer;
    transition: all 0.2s ease;
    white-space: nowrap;
    position: relative;
    overflow: hidden;
}
.tb-btn::before {
    content: ''; position: absolute;
    top: 0; left: 0; right: 0; height: 1px;
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.12), transparent);
    pointer-events: none;
}
.tb-btn:hover {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.20);
    color: rgba(255,255,255,0.90);
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(0,0,0,0.08);
}
.tb-btn:active {
    transform: translateY(0) scale(0.98);
}
.tb-btn svg { flex-shrink: 0; }

.tb-btn--icon::before {
    display: none;
}

.tb-action-group {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px;
    border-radius: 14px;
    background: rgba(255,255,255,0.035);
    border: 1px solid rgba(255,255,255,0.08);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.05);
}

.tb-filter-btn {
    display: none;
}

@media (max-width: 980px) {
    .tb-inner {
        flex-wrap: wrap;
        gap: 10px;
    }

    .tb-left {
        flex: 1 1 auto;
    }

    .tb-center {
        flex: 1 1 100%;
        order: 3;
    }

    .tb-right {
        margin-left: auto;
    }

    .tb-search {
        width: 160px;
    }
}

@media (max-width: 900px) {
    .mod-manager-workspace {
        grid-template-areas:
            "toolbar toolbar"
            "groups mods"
            "tags tags";
        grid-template-columns: minmax(200px, 240px) minmax(0, 1fr);
        grid-template-rows: auto minmax(420px, 1fr) 300px;
        gap: 10px;
        overflow: auto;
    }

    .mod-manager-column-resizer {
        display: none;
    }
}

@media (max-width: 680px) {
    .mod-manager-workspace {
        grid-template-areas:
            "toolbar"
            "groups"
            "mods"
            "tags";
        grid-template-columns: minmax(0, 1fr);
        grid-template-rows: auto 260px minmax(520px, auto) 320px;
    }

    .tb-left,
    .tb-right,
    .tb-center {
        width: 100%;
    }

    .tb-right {
        flex-wrap: wrap;
        margin-left: 0;
    }

    .tb-search {
        width: 100%;
    }
}

/* Primary action button */
.tb-btn--primary {
    background: rgba(70, 200, 120, 0.08);
    border-color: rgba(70, 200, 120, 0.18);
    color: rgba(100, 230, 150, 0.85);
}
.tb-btn--primary:hover {
    background: rgba(70, 200, 120, 0.14);
    border-color: rgba(70, 200, 120, 0.30);
    color: rgba(100, 230, 150, 1);
    box-shadow: 0 4px 16px rgba(70, 200, 120, 0.08);
}

/* Icon-only button */
.tb-btn--icon {
    width: 36px; padding: 0;
    display: flex; align-items: center; justify-content: center;
}
.tb-btn--icon:disabled {
    opacity: 0.35; cursor: not-allowed;
    transform: none !important;
}
/* Divider */
.tb-divider {
    width: 1px; height: 22px;
    background: rgba(255,255,255,0.10);
    flex-shrink: 0;
}

/* Segmented button group (view mode toggle) */
.tb-segmented-group {
    display: inline-flex;
    align-items: center;
    border-radius: 10px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.10);
    overflow: hidden;
}
.tb-seg-btn {
    width: 32px; height: 30px;
    display: flex; align-items: center; justify-content: center;
    border: none; background: transparent;
    color: rgba(255,255,255,0.40);
    cursor: pointer;
    transition: all 0.2s ease;
}
.tb-seg-btn:first-child { border-radius: 9px 0 0 9px; }
.tb-seg-btn:last-child { border-radius: 0 9px 9px 0; }
.tb-seg-btn:hover {
    color: rgba(255,255,255,0.70);
    background: rgba(255,255,255,0.06);
}
.tb-seg-btn.active {
    color: rgba(255,255,255,0.95);
    background: rgba(255,255,255,0.12);
    box-shadow: 0 1px 3px rgba(0,0,0,0.15);
}

/* Toolbar select dropdowns */
.tb-select {
    height: 30px; border-radius: 9px;
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.70);
    font-size: 12px; font-weight: 500;
    padding: 0 24px 0 10px;
    cursor: pointer;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='8' height='5' viewBox='0 0 8 5'%3E%3Cpath d='M1 1l3 3 3-3' stroke='rgba(255,255,255,0.5)' stroke-width='1.2' fill='none' stroke-linecap='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 8px center;
    transition: all 0.2s ease;
    max-width: 110px;
}
.tb-select:hover {
    border-color: rgba(255,255,255,0.22);
    background-color: rgba(255,255,255,0.07);
}
.tb-select:focus {
    border-color: rgba(120,200,255,0.35);
}
.tb-select option {
    background: var(--t-material-bg);
    color: rgba(255,255,255,0.90);
}

/* Sort order toggle button */
.tb-sort-order-btn {
    width: 32px !important; height: 30px;
}

/* Refresh spin */
.tb-spin {
    animation: tbSpin 0.8s linear infinite;
}
@keyframes tbSpin {
    to { transform: rotate(360deg); }
}

/* Tag CSS now in TagManagement.vue */

/* ===== Mod Key Editor - Glass Sci-Fi Aesthetic ===== */

.divider-vertical {
    width: 1px;
    height: 24px;
    background: rgba(255, 255, 255, 0.2);
    margin: 0 8px;
}

/* Main Layout */
.main-content {
    flex: 1;
    display: flex;
    overflow: hidden;
}

/* Sidebar — White Sci-Fi Glass (SwitchKeyList style) */
.sidebar {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    position: relative;
    background: rgba(255,255,255,0.06);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    border-right: 1px solid rgba(255,255,255,0.12);
    box-shadow: 0 4px 12px rgba(0,0,0,0.15), 0 0 0 1px rgba(255,255,255,0.06) inset;
}

/* Top hairline */
.sidebar::before {
    content: ''; position: absolute;
    top: 0; left: 0; right: 0; height: 1px;
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.18), transparent);
    pointer-events: none; z-index: 1;
}

/* Dot pattern overlay */
.sidebar::after {
    content: ''; position: absolute; inset: 0;
    pointer-events: none; z-index: 0;
    background-image: radial-gradient(circle, rgba(255,255,255,0.04) 1px, transparent 1px);
    background-size: 20px 20px;
}

/* Ambient glow */
.sd-glow {
    position: absolute;
    top: -40%; right: -20%; width: 60%; height: 80%;
    background: radial-gradient(ellipse, rgba(255,255,255,0.08), transparent 70%);
    pointer-events: none; z-index: 0;
}

/* ---- Sidebar Header ---- */
.sd-header {
    position: relative; z-index: 2;
    display: flex; align-items: center; justify-content: space-between;
    padding: 16px 16px 10px; flex-shrink: 0;
}
.sd-header::after {
    content: '';
    position: absolute; bottom: 0; left: 16px; right: 16px; height: 1px;
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.12), transparent);
}
.sd-header-left { display: flex; align-items: baseline; gap: 10px; min-width: 0; }
.sd-header-actions { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }

.sd-title {
    font-size: 14px; font-weight: 700; letter-spacing: 1px;
    text-transform: uppercase; color: rgba(255,255,255,0.88);
}

/* Sidebar header buttons */
.sd-btn {
    width: 28px; height: 28px; border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.08);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.45);
    display: inline-flex; align-items: center; justify-content: center;
    cursor: pointer; position: relative; z-index: 2;
    transition: all 0.2s ease;
}
.sd-btn:hover {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.18);
    color: rgba(255,255,255,0.80);
    transform: scale(1.06);
}
.sd-btn:active { transform: scale(0.96); }

/* ---- Group List ---- */
.group-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    position: relative; z-index: 2;
    padding: 8px 10px 12px;
}

.sidebar-resizer {
    position: absolute;
    top: 0;
    right: 0;
    width: 4px;
    height: 100%;
    cursor: col-resize;
    background: transparent;
    z-index: 10;
    transition: background 0.2s;
}

.sidebar-resizer:hover {
    background: rgba(var(--theme-surface-tint-rgb), 0.34);
}

/* Drag Styling */
.group-item.drag-over,
.custom-tree-node.drag-over {
    background: rgba(var(--theme-surface-tint-rgb), 0.14) !important;
    border-radius: 4px;
    outline: 1px dashed rgba(var(--theme-surface-tint-rgb), 0.55);
}

.group-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    margin-bottom: 4px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s;
    color: #a0a0a0;
}

.group-item:hover {
    background: rgba(255, 255, 255, 0.05);
    color: #fff;
}

.group-item.active {
    background: rgba(var(--theme-surface-tint-rgb), 0.14);
    color: var(--theme-accent);
    font-weight: 500;
}

.group-icon {
    width: 20px;
    height: 20px;
    margin-right: 6px;
    border-radius: 4px;
    overflow: hidden;
    flex-shrink: 0;
}
.icon-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.group-name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.count {
    font-size: 11px;
    background: rgba(255,255,255,0.06);
    padding: 1px 8px;
    border-radius: 999px;
    color: rgba(255,255,255,0.40);
    font-weight: 600;
}

/* Mod Grid */
.mod-grid-container {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    /* Custom Scrollbar */
}

.group-loading-hint {
    padding: 10px 16px;
    margin-bottom: 14px;
    border-radius: 12px;
    font-size: 12px;
    color: rgba(255,255,255,0.70);
    background: rgba(255,255,255,0.04);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    border: 1px solid rgba(255,255,255,0.10);
    box-shadow: 0 4px 12px rgba(0,0,0,0.08), 0 0 0 1px rgba(255,255,255,0.04) inset;
}

/* active-filter-summary moved to TagManagement.vue */

.subgroup-icon-row {
    display: flex;
    gap: 8px;
}



.mod-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, 240px);
    gap: 20px;
    justify-content: flex-start;
}

.mod-content-sections {
    display: flex;
    flex-direction: column;
    gap: 28px;
}

.mod-entry-enter-active,
.mod-entry-leave-active,
.mod-entry-move {
    transition: opacity .2s ease, transform .24s cubic-bezier(.2, .7, .2, 1);
}

.mod-entry-enter-from,
.mod-entry-leave-to {
    opacity: 0;
    transform: scale(.975) translateY(8px);
}

.mod-list-transition {
    display: contents;
}

/* ===== List View ===== */
.mod-list {
    display: flex;
    flex-direction: column;
    border-radius: 12px;
    overflow: hidden;
}
.mod-list-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    height: 36px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
    background: rgba(255,255,255,0.04);
}
.mod-list-hd {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(255,255,255,0.40);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.mod-list-hd--preview { width: 48px; flex-shrink: 0; }
.mod-list-hd--name { flex: 1; min-width: 0; }
.mod-list-hd--status { width: 72px; flex-shrink: 0; text-align: center; }
.mod-list-hd--tags { width: 160px; flex-shrink: 0; }
.mod-list-hd--group { width: 100px; flex-shrink: 0; }
.mod-list-hd--modified { width: 90px; flex-shrink: 0; }

.mod-list-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    height: 48px;
    border-bottom: 1px solid rgba(255,255,255,0.04);
    cursor: pointer;
    transition: background 0.15s ease;
}
.mod-list-row:hover {
    background: rgba(255,255,255,0.05);
}
.mod-list-row.is-selected {
    background: color-mix(in srgb, var(--theme-accent) 14%, transparent);
    box-shadow: inset 3px 0 color-mix(in srgb, var(--theme-accent) 82%, white 8%);
}
.mod-list-row.is-disabled {
    opacity: 0.55;
}
.mod-list-row--group {
    background: rgba(255,255,255,0.025);
    font-weight: 500;
}
.mod-list-row--group:hover {
    background: rgba(255,255,255,0.07);
}
.mod-list-row.state-pulse {
    animation: modListPulse 0.5s ease;
}
@keyframes modListPulse {
    0% { background: rgba(var(--theme-surface-tint-rgb), 0.18); }
    100% { background: transparent; }
}

.mod-list-cell {
    font-size: 13px;
    color: rgba(255,255,255,0.80);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.mod-list-cell--preview { width: 48px; flex-shrink: 0; }
.mod-list-cell--name { flex: 1; min-width: 0; font-weight: 500; }
.mod-list-cell--status {
    width: 72px; flex-shrink: 0;
    display: flex; justify-content: center;
}
.mod-list-cell--tags {
    width: 160px; flex-shrink: 0;
    display: flex; align-items: center; gap: 3px;
    overflow: hidden;
}
.mod-list-cell--group { width: 100px; flex-shrink: 0; color: rgba(255,255,255,0.50); }
.mod-list-cell--modified { width: 90px; flex-shrink: 0; color: rgba(255,255,255,0.45); font-size: 12px; }

.mod-list-thumb {
    position: relative;
    width: 36px; height: 36px;
    border-radius: 8px;
    overflow: hidden;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.10);
    display: flex; align-items: center; justify-content: center;
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
}
.mod-list-thumb.active {
    border-color: rgba(var(--theme-surface-tint-rgb), 0.40);
    box-shadow: 0 0 8px rgba(var(--theme-surface-tint-rgb), 0.18);
}
.mod-list-thumb-img {
    width: 100%; height: 100%;
}
.mod-list-thumb-img.is-nsfw-blurred {
    filter: blur(12px) saturate(0.82);
    transform: scale(1.14);
    transition: filter 220ms ease, transform 220ms ease;
}
.mod-list-thumb:hover .mod-list-thumb-img.is-nsfw-blurred.can-hover-reveal {
    filter: blur(0) saturate(1);
    transform: scale(1.04);
}
.mod-list-nsfw-shield {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: grid;
    place-items: center;
    color: rgba(255,255,255,0.9);
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.12em;
    background: rgba(6, 8, 12, 0.18);
    text-shadow: 0 1px 6px rgba(0,0,0,0.9);
    pointer-events: none;
    transition: opacity 220ms ease;
}
.mod-list-thumb:has(.can-hover-reveal):hover .mod-list-nsfw-shield { opacity: 0; }

.tb-blur-mode { display:flex; min-height:30px; }
.tb-blur-mode :deep(.el-radio-button) { flex:1 1 0; }
.tb-blur-mode :deep(.el-radio-button__inner) { display:flex; align-items:center; justify-content:center; box-sizing:border-box; min-height:30px; padding:0 8px; border-color:rgba(255,255,255,.13); background:rgba(255,255,255,.055); color:rgba(var(--theme-text-primary-rgb),.78); font-size:11px; line-height:1; box-shadow:none; transition:background .18s ease,border-color .18s ease,color .18s ease; }
.tb-blur-mode :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner) { background:rgba(var(--theme-surface-tint-rgb),.20); border-color:rgba(var(--theme-surface-tint-rgb),.42); color:rgba(var(--theme-text-primary-rgb),.98); box-shadow:-1px 0 0 rgba(var(--theme-surface-tint-rgb),.42); }
.tb-blur-mode :deep(.el-radio-button__inner) { border:none!important; outline:none!important; box-shadow:none!important; }
.tb-blur-mode :deep(.el-radio-button.is-active .el-radio-button__inner) { box-shadow:none!important; }
.mod-list-thumb-fallback {
    font-size: 16px; font-weight: 700;
    color: rgba(255,255,255,0.30);
    text-transform: uppercase;
}
.mod-list-thumb--group {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.16);
}
.mod-list-thumb--group .mod-list-thumb-fallback {
    font-size: 18px;
    color: rgba(255,255,255,0.40);
}

.mod-list-group-name {
    color: rgba(255,255,255,0.90);
}

.mod-list-tag-btn {
    width: 18px; height: 18px;
    border-radius: 5px;
    border: 1px solid rgba(255,255,255,0.10);
    background: rgba(255,255,255,0.04);
    color: rgba(255,255,255,0.35);
    display: flex; align-items: center; justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
    transition: all 0.15s ease;
}
.mod-list-tag-btn:hover {
    color: rgba(255,255,255,0.75);
    background: rgba(255,255,255,0.10);
    border-color: rgba(255,255,255,0.22);
}

.subgroup-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
}

.mod-manager.effects-paused *,
.mod-manager.effects-paused *::before,
.mod-manager.effects-paused *::after {
    animation-play-state: paused !important;
}

.subgroup-card {
    min-height: 120px;
    padding: 14px;
    border-radius: 14px;
    cursor: pointer;
    transition: transform 0.22s ease, box-shadow 0.22s ease;
    display: flex;
    flex-direction: column;
    gap: 8px;
    position: relative;
    overflow: hidden;
    background: rgba(255,255,255,0.08);
    /* 性能：子组卡片逐个出现在网格中，必须无 backdrop-filter（覆盖 .glass-panel 的模糊），否则滚动时合成层爆炸 */
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border: 1px solid rgba(255,255,255,0.20);
    box-shadow:
        0 0 16px rgba(var(--theme-surface-tint-rgb), 0.14),
        0 8px 24px rgba(0,0,0,0.12);
}

/* Top hairline */
.subgroup-card::before {
    display: none;
}

/* Sheen sweep — always active */
.subgroup-card::after {
    content: "";
    position: absolute;
    top: 0; left: -160%;
    width: 160%; height: 100%;
    background: linear-gradient(110deg,
        transparent 38%,
        rgba(255,255,255,0.06) 46%,
        rgba(255,255,255,0.22) 50%,
        rgba(255,255,255,0.06) 54%,
        transparent 62%);
    transform: skewX(-18deg);
    pointer-events: none;
    opacity: 1;
    animation: subgroupSheen 3.4s ease-in-out infinite;
    z-index: 4;
}

.subgroup-card > * {
    position: relative;
    z-index: 3;
}

.subgroup-bg-preview {
    position: absolute;
    inset: 0;
    z-index: 1;
    background-size: cover;
    background-position: center;
    background-repeat: no-repeat;
    opacity: 0.80;
    filter: brightness(1.10) saturate(1.10) contrast(1.02);
    transition: opacity 0.28s ease, filter 0.28s ease;
}

.subgroup-bg-preview::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to bottom,
        rgba(10, 14, 22, 0.10) 0%,
        rgba(10, 14, 22, 0.30) 56%,
        rgba(10, 14, 22, 0.55) 100%);
    transition: opacity 0.28s ease;
}

.subgroup-card:hover {
    transform: translateY(-3px);
    border-color: rgba(255,255,255,0.26);
    box-shadow:
        0 0 32px rgba(var(--theme-surface-tint-rgb), 0.22),
        0 0 48px rgba(142, 230, 255, 0.10),
        0 14px 36px rgba(0,0,0,0.16);
}
.subgroup-card:hover .subgroup-bg-preview {
    opacity: 0.85;
    filter: brightness(1.12) saturate(1.12) contrast(1.04);
}
.subgroup-card:hover .subgroup-bg-preview::after {
    background: linear-gradient(to bottom,
        rgba(10, 14, 22, 0.02) 0%,
        rgba(10, 14, 22, 0.08) 56%,
        rgba(10, 14, 22, 0.20) 100%);
}

.subgroup-card.reorder-hover {
    outline: 2px solid rgba(var(--theme-surface-tint-rgb), 0.55);
    outline-offset: 1px;
    transform: translateY(-3px);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.45);
}

.subgroup-card.is-disabled {
    opacity: 1;
    filter: none;
    border-color: rgba(255,255,255,0.14);
}
.subgroup-card.is-disabled::before,
.subgroup-card.is-disabled::after {
    display: none;
}
.subgroup-card.is-disabled .subgroup-bg-preview {
    opacity: 0.80;
    filter: none;
}

.subgroup-card .subgroup-bg-preview.is-nsfw-blurred,
.subgroup-card:hover .subgroup-bg-preview.is-nsfw-blurred {
    filter: blur(18px) brightness(0.82) saturate(0.75);
    transform: scale(1.16);
}

.subgroup-card:hover .subgroup-bg-preview.is-nsfw-blurred.can-hover-reveal {
    filter: brightness(1.12) saturate(1.12) contrast(1.04);
    transform: scale(1.04);
}

.subgroup-bg-slide-enter-active,
.subgroup-bg-slide-leave-active {
    transition: transform 0.32s ease, opacity 0.22s ease;
}

.subgroup-bg-slide-enter-from {
    transform: translateX(14%);
    opacity: 0.25;
}

.subgroup-bg-slide-leave-to {
    transform: translateX(-14%);
    opacity: 0.25;
}

/* 位移改用 translateX（合成器属性）：left 动画每帧触发布局+重绘。
   换算：left -160%→145% = 305% 父宽；元素宽 160% 父宽 → 305/160 = 190.625% 自身宽 */
@keyframes subgroupSheen {
    0%   { transform: translateX(0) skewX(-18deg);        opacity: 0.3; }
    42%  { transform: translateX(190.625%) skewX(-18deg); opacity: 0.7; }
    100% { transform: translateX(190.625%) skewX(-18deg); opacity: 0.3; }
}

.subgroup-card.is-disabled .subgroup-name,
.subgroup-card.is-disabled .subgroup-path,
.subgroup-card.is-disabled .subgroup-count {
    color: inherit;
}

.subgroup-icon-wrap {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    justify-content: center;
}

.subgroup-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
}

.subgroup-toggle {
    flex-shrink: 0;
    position: relative;
    z-index: 3;
}

.subgroup-icon {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 10px;
}

.subgroup-icon-placeholder {
    font-size: 20px;
    color: rgba(var(--theme-surface-tint-rgb), 0.72);
}

.subgroup-name {
    font-size: 15px;
    font-weight: 600;
    color: rgba(255,255,255,0.88);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: 0.02em;
}

.disabled-badge {
    margin-left: 6px;
    font-size: 11px;
    color: rgba(255,255,255,0.55);
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.12);
    border-radius: 999px;
    padding: 1px 8px;
    vertical-align: middle;
    font-weight: 600;
}

.subgroup-path {
    font-size: 12px;
    color: rgba(255,255,255,0.45);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.subgroup-count {
    font-size: 12px;
    color: rgba(100, 230, 150, 0.80);
    font-weight: 500;
}

.subgroup-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
}

@keyframes subgroupBorderSheen {
    0% {
        left: -160%;
        opacity: 0.26;
    }
    42% {
        left: 145%;
        opacity: 0.62;
    }
    100% {
        left: 145%;
        opacity: 0.26;
    }
}

.mod-card {
    --mx: 50%;
    --my: 18%;
    --phase-a: 0s;
    --phase-b: 0s;
    --breath-duration: 4.2s;
    --sheen-duration: 5.2s;
    --rotate-duration: 8.5s;
    border-radius: 12px;
    overflow: hidden;
    transition: all 0.3s cubic-bezier(0.25, 0.8, 0.25, 1);
    display: flex;
    flex-direction: column;
    width: 240px;
    position: relative;
    user-select: none;
    cursor: grab;
    z-index: 1;
    background: rgba(255,255,255,0.10);
    border: 1px solid rgba(255,255,255,0.22);
    box-shadow:
        0 0 20px rgba(var(--theme-surface-tint-rgb), 0.16),
        0 12px 32px rgba(0,0,0,0.15),
        0 0 0 1px rgba(255,255,255,0.10) inset;
    animation: none;
    /* Free virtual scrolling: browser skips rendering off-screen cards */
    content-visibility: auto;
    contain-intrinsic-size: 240px 300px;
}

.mod-card-main {
    display: flex;
    flex-direction: column;
    width: 100%;
    overflow: hidden;
    border-radius: 12px;
}

.mod-card:not(.is-disabled) {
    /* 呼吸基态（原 keyframes 0%/100% 的值，动画不再覆盖 border/box-shadow，hover 阴影恢复生效） */
    border-color: rgba(var(--theme-surface-tint-rgb), 0.24);
    box-shadow:
        0 0 0 1px rgba(255, 255, 255, 0.08) inset,
        0 0 24px rgba(var(--theme-surface-tint-rgb), 0.18),
        0 12px 26px rgba(0, 0, 0, 0.38);
}

/* 呼吸脉冲层：只动 opacity，合成器混合完成，每张卡片每帧零重绘。
   原实现直接动画 box-shadow/border-color，所有可见卡片每帧都在主线程重绘。 */
.mod-card:not(.is-disabled)::before {
    content: "";
    position: absolute;
    /* 不能用 inset:-1px：卡片 overflow:hidden 会把伪元素裁到 padding-box，边框环会被裁没 */
    inset: 0;
    border-radius: 12px;
    border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.48);
    box-shadow:
        0 0 18px rgba(var(--theme-surface-tint-rgb), 0.20) inset,
        0 0 0 1px rgba(255, 255, 255, 0.04) inset;
    opacity: 0;
    pointer-events: none;
    animation: modEnabledBreath var(--breath-duration) ease-in-out infinite;
    animation-delay: var(--phase-a);
}

.mod-card.state-pulse::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 12px;
    pointer-events: none;
    border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.70);
    box-shadow: 0 0 0 rgba(var(--theme-surface-tint-rgb), 0.45);
    animation: modStatePulse 0.54s ease-out 1;
    z-index: 7;
}

.mod-card[draggable="true"] { cursor: move; }
.mod-card[draggable="false"] { cursor: grab; }

.mod-card.reorder-hover {
    outline: 2px dashed rgba(var(--theme-surface-tint-rgb), 0.60);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.40);
}

.mod-card:active {
    cursor: grabbing;
}

.mod-card:hover {
    transform: translateY(-4px);
    box-shadow:
        0 0 40px rgba(var(--theme-surface-tint-rgb), 0.28),
        0 0 60px rgba(142, 230, 255, 0.12),
        0 18px 48px rgba(0,0,0,0.20),
        0 0 0 1px rgba(255,255,255,0.12) inset;
    border-color: rgba(255,255,255,0.28);
    z-index: 2;
}

/* Disabled State Visuals */
.mod-card.is-disabled {
    opacity: 1;
    filter: none;
    border-color: rgba(255,255,255,0.12);
    box-shadow:
        0 0 0 1px rgba(255,255,255,0.08) inset,
        0 4px 18px rgba(0,0,0,0.15);
}

.mod-card.is-disabled:hover {
    transform: translateY(-3px);
    border-color: rgba(255,255,255,0.24);
    box-shadow:
        0 12px 32px rgba(0,0,0,0.18),
        0 0 0 1px rgba(255,255,255,0.10) inset;
}

.card-preview {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    flex-shrink: 0;
    background: rgba(255,255,255,0.02);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
}

/* Crystal wrapper — mouse-follow glow + sheen */
.mod-crystal-wrapper {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: radial-gradient(circle at var(--mx) var(--my),
            rgba(255, 255, 255, 0.25) 0%,
            rgba(255, 255, 255, 0.08) 40%,
            transparent 100%);
}

.mod-crystal-wrapper::after {
    content: "";
    position: absolute;
    top: 0;
    left: -150%;
    width: 200%;
    height: 100%;
    background: linear-gradient(115deg,
        transparent 40%,
        rgba(255, 255, 255, 0.08) 45%,
        rgba(255, 255, 255, 0.35) 50%,
        rgba(255, 255, 255, 0.08) 55%,
        transparent 60%);
    transform: skewX(-20deg);
    pointer-events: none;
    z-index: 5;
    opacity: 0.5;
}

.preview-nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 24px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    z-index: 10;
    background: rgba(0,0,0,0.3); 
    color: rgba(255,255,255,0.7);
    opacity: 0; 
    transition: opacity 0.3s, background 0.3s;
    border-radius: 4px;
    margin: 0 4px;
}

.mod-card:hover .preview-nav {
    opacity: 1; 
}

.preview-nav.prev { left: 0; }
.preview-nav.next { right: 0; }

.preview-nav:hover { 
    background: rgba(0,0,0,0.6); 
    color: #fff;
}

.mod-card-actions {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 22;
    display: flex;
    align-items: center;
    gap: 8px;
}

/* ═══ Key Floater (floating card next to K badge) ═══ */
.kf-fade-enter-active,
.kf-fade-leave-active { transition: opacity 0.15s ease, transform 0.15s ease; }
.kf-fade-enter-from,
.kf-fade-leave-to { opacity: 0; transform: translateY(-4px) scale(0.96); }

.key-floater {
    position: fixed;
    z-index: 11000;
    width: 300px;
    max-height: 380px;
    display: flex;
    flex-direction: column;
    background: rgba(255,255,255,0.06);
    backdrop-filter: blur(28px) saturate(1.6);
    -webkit-backdrop-filter: blur(28px) saturate(1.6);
    border: 1px solid rgba(255,255,255,0.18);
    border-radius: 14px;
    box-shadow: 0 24px 64px rgba(0,0,0,0.30), 0 0 0 1px rgba(255,255,255,0.06) inset;
    overflow: hidden;
}

.kf-header {
    flex-shrink: 0;
    padding: 10px 14px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}
.kf-title {
    font-size: 11px; font-weight: 700; letter-spacing: 0.8px;
    text-transform: uppercase; color: rgba(255,255,255,0.65);
}

.kf-body {
    flex: 1; overflow-y: auto;
    padding: 8px 10px;
    display: flex; flex-direction: column; gap: 5px;
}

.kf-item {
    display: flex; align-items: center; justify-content: space-between;
    gap: 8px; padding: 7px 10px;
    border-radius: 8px;
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.07);
}
.kf-item:hover {
    background: rgba(255,255,255,0.06);
    border-color: rgba(255,255,255,0.14);
}
.kf-item-info { flex: 1; min-width: 0; }
.kf-item-name {
    font-size: 11px; font-weight: 600; color: rgba(255,255,255,0.85);
    word-break: break-word;
}
.kf-item-desc {
    margin-top: 1px; font-size: 9px; color: rgba(255,255,255,0.35);
    word-break: break-word; font-family: 'Consolas', 'Courier New', monospace;
}
.kf-type {
    flex-shrink: 0;
    padding: 2px 8px; border-radius: 999px;
    background: rgba(var(--theme-surface-tint-rgb),0.08);
    border: 1px solid rgba(var(--theme-surface-tint-rgb),0.12);
    font-size: 9px; font-weight: 700; letter-spacing: 0.5px;
    text-transform: uppercase; color: rgba(var(--theme-surface-tint-rgb),0.65);
}

.kf-footer {
    flex-shrink: 0;
    padding: 8px 10px;
    border-top: 1px solid rgba(255,255,255,0.08);
}
.kf-edit-btn {
    width: 100%; height: 28px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.12);
    background: rgba(255,255,255,0.05);
    color: rgba(255,255,255,0.65);
    font-size: 11px; font-weight: 600;
    cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    transition: all 0.15s;
}
.kf-edit-btn:hover {
    background: rgba(255,255,255,0.09);
    border-color: rgba(255,255,255,0.22);
    color: var(--theme-accent);
}

.mod-key-badge,
.mod-tag-badge {
    width: 28px;
    height: 28px;
    border: 1px solid rgba(255,255,255,0.12);
    border-radius: 999px;
    background: rgba(255,255,255,0.06);
    /* 性能：逐卡片元素不加 backdrop-filter */
    box-shadow: 0 4px 12px rgba(0,0,0,0.12), 0 0 0 1px rgba(255,255,255,0.04) inset;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.2s ease;
}

/* Key badge �� crystal accent */
.mod-key-badge {
    color: rgba(var(--theme-surface-tint-rgb),0.75);
}

/* Tag badge �� white glass */
.mod-tag-badge {
    width: auto;
    min-width: 44px;
    padding: 0 10px;
    border-radius: 999px;
    color: rgba(255,255,255,0.60);
}

.mod-key-badge:hover {
    transform: scale(1.08);
    background: rgba(var(--theme-surface-tint-rgb),0.10);
    border-color: rgba(var(--theme-surface-tint-rgb),0.30);
    color: var(--theme-accent);
    box-shadow: 0 4px 14px rgba(0,0,0,0.18), 0 0 12px rgba(var(--theme-surface-tint-rgb),0.06);
}

.mod-tag-badge:hover {
    transform: translateY(-1px) scale(1.05);
    background: rgba(255,255,255,0.10);
    border-color: rgba(255,255,255,0.28);
    color: rgba(255,255,255,0.90);
    box-shadow: 0 6px 20px rgba(0,0,0,0.20), 0 0 0 1px rgba(255,255,255,0.06) inset;
}

.mod-tag-badge:active {
    transform: scale(0.95);
}

.mod-key-badge-text {
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.5px;
    line-height: 1;
}

.mod-tag-badge-text {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.4px;
    line-height: 1;
}

.mod-tag-badge:hover .mod-tag-badge-text {
    color: var(--theme-accent);
}



.preview-indicators {
    position: absolute;
    bottom: 8px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: center;
    gap: 6px;
    z-index: 15;
    pointer-events: none;
}

.indicator-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.4);
    transition: background 0.3s, transform 0.3s;
    pointer-events: auto;
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0,0,0,0.3);
}

.indicator-dot:hover {
    background: rgba(255, 255, 255, 0.7);
    transform: scale(1.1);
}

.indicator-dot.active {
    background: #fff;
    transform: scale(1.2);
    box-shadow: 0 0 4px rgba(255,255,255,0.4);
}

.slide-item {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
}

/* Slide Transition for Preview Images */
.preview-slide-enter-active,
.preview-slide-leave-active {
  transition: transform 0.4s cubic-bezier(0.25, 1, 0.5, 1), opacity 0.4s ease;
}

.preview-slide-enter-from {
  transform: translateX(100%);
  opacity: 0.8;
}

.preview-slide-leave-to {
  transform: translateX(-100%);
  opacity: 0.8;
}

/* Ensure container can handle absolute children properly */
.image-wrapper {
    width: 100%;
    height: 100%;
    position: relative;
    overflow: hidden; /* Clip sliding images */
}

.image-wrapper::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to bottom, rgba(var(--theme-surface-tint-rgb), 0.08), rgba(0, 0, 0, 0) 34%, rgba(5, 8, 14, 0.24) 100%);
    z-index: 1;
    pointer-events: none;
}

/* Zoom effect on the internal image */
.zoom-image {
    width: 100%;
    height: 100%;
    display: block;
    transition: transform 0.5s ease;
    position: relative;
    z-index: 0;
}

.mod-card:hover .zoom-image {
    transform: scale(1.05);
}

.mod-card.is-disabled .mod-name {
    color: rgba(255,255,255,0.88);
}

.mod-card.is-disabled .mod-group {
    color: rgba(255,255,255,0.56);
}

.mod-card.is-disabled .card-info {
    background: linear-gradient(to top, rgba(7, 10, 16, 0.78), rgba(7, 10, 16, 0.36));
}

@keyframes modEnabledBreath {
    0%,
    100% {
        opacity: 0;
    }
    50% {
        opacity: 1;
    }
}

@keyframes modStatePulse {
    0% {
        opacity: 0.85;
        transform: scale(1);
        box-shadow: 0 0 0 rgba(var(--theme-surface-tint-rgb), 0.42);
    }
    100% {
        opacity: 0;
        transform: scale(1.03);
        box-shadow: 0 0 26px rgba(var(--theme-surface-tint-rgb), 0);
    }
}

/* Prevent image dragging interfering with card dragging */
:deep(.mod-card img) {
    -webkit-user-drag: none;
    pointer-events: none;
}

.image-placeholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background:
        linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), 0.08), rgba(var(--theme-surface-tint-rgb), 0.025)),
        rgba(255, 255, 255, 0.03);
    color: rgba(var(--theme-surface-tint-rgb), 0.28);
    font-size: 48px;
    font-weight: 800;
}
.preview-info {
    font-size: 13px;
    color: #ccc;
    background: rgba(0,0,0,0.2);
    padding: 10px;
    border-radius: 4px;
}
.preview-info p {
    margin: 4px 0;
}

.char-avatar {
    text-transform: uppercase;
}

/* Hover Action Overlay */
.card-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.2s;
    pointer-events: none; /* Let clicks pass through to draggable card */
}

/* Allow interaction with buttons inside overlay */
.card-overlay .el-button {
    pointer-events: auto;
}

.mod-card:hover .card-overlay {
    opacity: 1;
}

/* Footer Info Area — Glass */
.card-info {
    padding: 12px 14px 14px;
    background: rgba(255,255,255,0.03);
    /* 性能：逐卡片元素不加 backdrop-filter */
    border-top: 1px solid rgba(255,255,255,0.06);
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-height: 84px;
    position: relative;
    z-index: 2;
}

.header-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 10px;
}

.text-content {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
}

.mod-name {
    font-weight: 600;
    color: rgba(255,255,255,0.88);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 14px;
    letter-spacing: 0.3px;
}

.mod-group {
    font-size: 11px;
    color: rgba(255,255,255,0.45);
    display: flex;
    align-items: center;
    gap: 4px;
}
/*.mod-group::before {
    content: '';
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: #666;
}*/
.mini-group-icon {
    width: 14px;
    height: 14px;
    border-radius: 2px;
    object-fit: cover;
}

.mod-tag-line {
    min-height: 16px;
    font-size: 11px;
    line-height: 1.45;
    color: rgba(255,255,255,0.55);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.mod-tag-line::before {
    content: '\26AC ';
    color: rgba(var(--theme-surface-tint-rgb),0.50);
    font-size: 9px;
    margin-right: 2px;
}

/* Active dot color if mod is enabled? Could be cool */
/*
.mod-card:not(.is-disabled) .mod-group::before {
    background-color: #67C23A;
    box-shadow: 0 0 6px rgba(103, 194, 58, 0.5);
}
*/


/* Switch styling tweak */
:deep(.el-switch__core) {
    background-color: rgba(255,255,255,0.1);
    border-color: transparent;
}

/* Tree Styles — Glass Sci-Fi */
.group-tree {
    background: transparent; 
    color: rgba(255,255,255,0.78);
    min-height: 100%;
    padding: 0 4px 8px;
    box-sizing: border-box;
}
:deep(.el-tree-node > .el-tree-node__children) {
    display: block;
    margin-left: 4px;
}

:deep(.el-tree-node.is-expanded > .el-tree-node__children) {
    display: block;
    max-height: none;
    overflow: visible;
    padding-right: 0;
    margin: 2px 0 6px 6px;
    border-left: 1px solid rgba(255, 255, 255, 0.06);
}

:deep(.el-tree-node.is-expanded > .el-tree-node__children > .el-tree-node) {
    min-width: 0;
}

:deep(.el-tree-node__content) {
    height: 34px;
    border-radius: 8px;
    margin-bottom: 2px;
    transition: all 0.15s ease;
}
:deep(.el-tree-node__content:hover) {
    background-color: rgba(255, 255, 255, 0.06) !important;
}
:deep(.el-tree--highlight-current .el-tree-node.is-current > .el-tree-node__content) {
    background: rgba(255,255,255,0.08) !important;
    box-shadow: inset 0 0 0 1px rgba(255,255,255,0.08);
    color: rgba(255,255,255,0.92);
}
:deep(.el-tree-node__expand-icon) {
    color: rgba(255, 255, 255, 0.30);
    font-size: 12px;
    transition: transform 0.2s ease;
}
:deep(.el-tree-node__expand-icon.is-leaf) {
    color: transparent;
}
:deep(.el-tree-node__expand-icon.expanded) {
    transform: rotate(90deg);
}

.custom-tree-node {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 30px;
    padding: 4px 9px;
    margin: 2px 3px;
    box-sizing: border-box;
    border-radius: 7px;
    overflow: hidden;
}

.custom-tree-node.is-disabled {
    opacity: 1;
    color: rgba(255,255,255,0.78);
}

.disabled-icon {
    margin-left: 4px;
    color: #F56C6C;
    font-size: 14px;
}

.custom-tree-node.reorder-hover {
    background: rgba(var(--theme-surface-tint-rgb), 0.12);
    border-radius: 6px;
    outline: 1px dashed rgba(var(--theme-surface-tint-rgb), 0.55);
}

.node-content {
    display: flex;
    align-items: center;
    gap: 8px;
    overflow: hidden;
}

.node-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0.02em;
}

.tree-icon {
    width: 20px;
    height: 20px;
    object-fit: cover;
    border-radius: 6px;
    border: 1px solid rgba(255,255,255,0.08);
}

.tree-icon-placeholder {
    font-size: 16px;
    color: rgba(255,255,255,0.30);
}
:deep(.el-switch.is-checked .el-switch__core) {
    background-color: #67C23A;
}

/* Context Menu — Glass Sci-Fi */
.custom-context-menu {
    position: fixed;
    z-index: 9999;
    background: rgba(255,255,255,0.06);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    border: 1px solid rgba(255,255,255,0.15);
    border-radius: 12px;
    padding: 6px 0;
    min-width: 170px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.25), 0 0 0 1px rgba(255,255,255,0.06) inset;
    overflow: visible;
}
.custom-context-menu::before {
    content: ''; position: absolute;
    top: 0; left: 0; right: 0; height: 1px;
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.12), transparent);
    pointer-events: none;
}

.menu-item {
    padding: 9px 16px;
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    transition: all 0.15s ease;
    color: rgba(255,255,255,0.72);
    font-size: 0.9em;
    font-weight: 500;
    letter-spacing: 0.02em;
    position: relative;
    z-index: 1;
}

.menu-item:hover {
    background: rgba(255,255,255,0.08);
    color: rgba(255,255,255,0.92);
}

.menu-divider {
    height: 1px;
    background: rgba(255, 255, 255, 0.1);
    margin: 4px 0;
}

.has-submenu .submenu {
    position: absolute;
    left: 100%;
    top: 0;
    margin-left: 4px;
    /* Reuse base styles */
    background:
        linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.10), rgba(var(--theme-surface-tint-rgb), 0.04)),
        rgba(24, 21, 14, 0.92);
    border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.16);
    border-radius: 8px;
    padding: 4px 0;
    min-width: 260px;
    max-width: min(420px, calc(100vw - 40px));
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.38), 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.04) inset;
    backdrop-filter: blur(12px) saturate(1.25);
    -webkit-backdrop-filter: blur(12px) saturate(1.25);
    opacity: 0;
    visibility: hidden;
    transition:
        opacity 0.12s ease 0.18s,
        visibility 0s linear 0.18s;
}

.has-submenu::after {
    content: '';
    position: absolute;
    top: -8px;
    left: 100%;
    width: 14px;
    height: calc(100% + 16px);
}

.has-submenu:hover .submenu,
.has-submenu .submenu:hover {
    opacity: 1;
    visibility: visible;
    transition-delay: 0s;
}

.has-submenu .submenu .menu-item {
    white-space: nowrap;
}

.has-submenu .submenu .menu-item span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
}

.arrow-right {
    margin-left: auto;
    font-size: 0.8em;
    opacity: 0.7;
}

/* sidebar-header moved to scoped CSS (sd-header etc.) */

.inline-progress {
    margin-top: 16px;
    padding: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
    background: rgba(255,255,255,0.03);
}

.inline-progress-header {
    font-size: 13px;
    color: #fff;
    margin-bottom: 8px;
}

/* Crystal glass theme overrides for this page.
   Keep these late in the scoped style so old local white-glass rules cannot win. */
.mod-manager .glass-panel,
.mod-manager .tb-bar,
.mod-manager .sidebar,
.mod-manager .subgroup-card,
.mod-manager .mod-card,
.mod-manager .key-floater,
.mod-manager .custom-context-menu,
.mod-manager .inline-progress {
    background: var(--t-page-panel-bg);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.12);
    box-shadow: var(--t-page-panel-shadow);
}
.tb-select.el-select {
    width: 110px;
    padding: 0;
    border: 0;
    background: transparent;
    background-image: none;
}
.tb-select.el-select :deep(.el-select__wrapper) {
    min-height: 30px;
    padding: 0 9px;
    border: 1px solid rgba(255,255,255,0.10);
    border-radius: 9px;
    background: rgba(255,255,255,0.04);
    box-shadow: none;
}
.tb-select.el-select :deep(.el-select__selected-item) {
    color: rgba(255,255,255,0.70);
    font-size: 12px;
    font-weight: 500;
}

/* One physical edge per main panel; inner highlights made these borders look doubled. */
.mod-manager .mod-manager-layout-content,
.mod-manager .tb-bar,
.mod-manager .mod-card {
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.18);
}

.mod-manager .tb-bar::before {
    display: none;
}

.mod-manager .tb-bar {
    border-bottom-color: rgba(var(--theme-surface-tint-rgb), 0.14);
}

.mod-manager .sidebar {
    border-right-color: rgba(var(--theme-surface-tint-rgb), 0.14);
}

.mod-manager .tb-bar::before,
.mod-manager .sidebar::before,
.mod-manager .subgroup-card::before,
.mod-manager .custom-context-menu::before {
    background: linear-gradient(90deg, transparent, rgba(var(--theme-surface-tint-rgb), 0.22), rgba(255, 255, 255, 0.10), transparent);
}

.mod-manager .tb-bar::after,
.mod-manager .sidebar::after {
    background-image: radial-gradient(circle, rgba(var(--theme-surface-tint-rgb), 0.055) 1px, transparent 1px);
}

.mod-manager .tb-glow,
.mod-manager .sd-glow {
    background: radial-gradient(ellipse, rgba(var(--theme-surface-tint-rgb), 0.12), transparent 70%);
}

.mod-manager .tb-search-input,
.mod-manager .tb-btn,
.mod-manager .sd-btn,
.mod-manager .mod-manager-panel-icon-btn,
.mod-manager .group-loading-hint,
.mod-manager .kf-item,
.mod-manager .kf-edit-btn,
.mod-manager :deep(.mod-key-badge),
.mod-manager :deep(.mod-tag-badge),
.mod-manager .preview-info {
    background: rgba(var(--theme-surface-tint-rgb), 0.045);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.14);
    color: rgba(var(--theme-text-secondary-rgb), 0.78);
}

.mod-manager .tb-search-input::placeholder {
    color: rgba(var(--theme-text-secondary-rgb), 0.34);
}

.mod-manager .tb-search-icon,
.mod-manager .tb-search-clear,
.mod-manager .sd-btn,
.mod-manager .tree-icon-placeholder {
    color: rgba(var(--theme-surface-tint-rgb), 0.52);
}

.mod-manager .tb-search-input:focus,
.mod-manager .tb-btn:hover,
.mod-manager .sd-btn:hover,
.mod-manager .mod-manager-panel-icon-btn:hover:not(:disabled),
.mod-manager .kf-item:hover,
.mod-manager .kf-edit-btn:hover,
.mod-manager :deep(.mod-tag-badge):hover {
    background: rgba(var(--theme-surface-tint-rgb), 0.085);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.28);
    color: rgba(var(--theme-text-primary-rgb), 0.96);
    box-shadow: 0 6px 18px rgba(var(--theme-surface-tint-rgb), 0.10);
}

.mod-manager .tb-btn::before,
.mod-manager .sd-header::after,
.mod-manager .subgroup-card::after,
.mod-manager .kf-header,
.mod-manager .kf-footer,
.mod-manager :deep(.card-info),
.mod-manager :deep(.menu-divider) {
    border-color: rgba(var(--theme-surface-tint-rgb), 0.10);
}

.mod-manager .tb-btn::before {
    background: linear-gradient(90deg, transparent, rgba(var(--theme-surface-tint-rgb), 0.16), transparent);
}

.mod-manager .tb-divider,
.mod-manager .divider-vertical {
    background: rgba(var(--theme-surface-tint-rgb), 0.14);
}

.mod-manager .tb-btn--primary {
    background: rgba(var(--theme-surface-tint-rgb), 0.08);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.20);
    color: rgba(var(--theme-surface-tint-rgb), 0.88);
}

.mod-manager .tb-btn--primary:hover {
    background: rgba(var(--theme-surface-tint-rgb), 0.14);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.34);
    color: var(--theme-accent);
    box-shadow: 0 4px 16px rgba(var(--theme-surface-tint-rgb), 0.12);
}

.mod-manager .sd-title,
.mod-manager .subgroup-name,
.mod-manager :deep(.mod-name),
.mod-manager .inline-progress-header {
    color: rgba(var(--theme-text-primary-rgb), 0.94);
}

.mod-manager .count,
.mod-manager .subgroup-path,
.mod-manager :deep(.mod-group),
.mod-manager :deep(.mod-tag-line),
.mod-manager .kf-title,
.mod-manager .kf-item-desc {
    color: rgba(var(--theme-text-secondary-rgb), 0.52);
}

.mod-manager .group-item:hover,
.mod-manager :deep(.el-tree-node__content:hover),
.mod-manager :deep(.menu-item):hover {
    background: rgba(var(--theme-surface-tint-rgb), 0.075) !important;
    color: rgba(var(--theme-text-primary-rgb), 0.96);
}

.mod-manager :deep(.el-tree--highlight-current .el-tree-node.is-current > .el-tree-node__content) {
    background: rgba(var(--theme-surface-tint-rgb), 0.12) !important;
    box-shadow: inset 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.18);
    color: rgba(var(--theme-text-primary-rgb), 0.96);
}

.mod-manager :deep(.el-tree-node.is-expanded > .el-tree-node__children) {
    border-left-color: rgba(var(--theme-surface-tint-rgb), 0.12);
}

.mod-manager .subgroup-card {
    background:
        linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.075), rgba(var(--theme-surface-tint-rgb), 0.026)),
        rgba(255, 255, 255, 0.035);
}

.mod-manager .subgroup-card:hover,
.mod-manager .mod-card:hover {
    border-color: rgba(var(--theme-surface-tint-rgb), 0.32);
    box-shadow:
        0 0 32px rgba(var(--theme-surface-tint-rgb), 0.20),
        0 14px 36px rgba(0, 0, 0, 0.18);
}

.mod-manager .subgroup-bg-preview::after {
    background: linear-gradient(to bottom,
        rgba(var(--theme-surface-tint-rgb), 0.045) 0%,
        rgba(18, 15, 10, 0.24) 58%,
        rgba(8, 7, 5, 0.50) 100%);
}

.mod-manager .mod-card {
    background:
        linear-gradient(145deg, rgba(var(--theme-surface-tint-rgb), 0.075), rgba(var(--theme-surface-tint-rgb), 0.028)),
        rgba(255, 255, 255, 0.035);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.18);
}

.mod-manager .subgroup-card.is-directory-link,
.mod-manager .mod-list-row.is-directory-link,
.mod-manager :deep(.mod-card.is-directory-link) {
    background:
        linear-gradient(145deg, rgba(85, 174, 104, 0.16), rgba(85, 174, 104, 0.045)),
        rgba(255, 255, 255, 0.035);
    border-color: rgba(112, 208, 124, 0.38);
}

.mod-manager .custom-tree-node.is-directory-link {
    background: rgba(85, 174, 104, 0.12);
    border-radius: 6px;
    box-shadow: inset 0 0 0 1px rgba(112, 208, 124, 0.24);
}

.mod-manager .subgroup-card.is-directory-link:hover,
.mod-manager .mod-list-row.is-directory-link:hover,
.mod-manager :deep(.mod-card.is-directory-link:hover) {
    border-color: rgba(142, 230, 152, 0.58);
}

.mod-manager :deep(.card-preview),
.mod-manager :deep(.card-info) {
    background: rgba(var(--theme-surface-tint-rgb), 0.035);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.10);
}

.mod-manager :deep(.mod-crystal-wrapper) {
    background: radial-gradient(circle at var(--mx) var(--my),
        rgba(var(--theme-surface-tint-rgb), 0.20) 0%,
        rgba(var(--theme-surface-tint-rgb), 0.07) 42%,
        transparent 100%);
}

.mod-manager :deep(.mod-crystal-wrapper::after) {
    background: linear-gradient(110deg,
        transparent 38%,
        rgba(var(--theme-surface-tint-rgb), 0.055) 46%,
        rgba(var(--theme-text-primary-rgb), 0.24) 50%,
        rgba(var(--theme-surface-tint-rgb), 0.055) 54%,
        transparent 62%);
}

.mod-manager .subgroup-card::after {
    background: linear-gradient(110deg,
        transparent 38%,
        rgba(var(--theme-surface-tint-rgb), 0.055) 46%,
        rgba(var(--theme-text-primary-rgb), 0.24) 50%,
        rgba(var(--theme-surface-tint-rgb), 0.055) 54%,
        transparent 62%);
}

.mod-manager :deep(.indicator-dot) {
    background: rgba(var(--theme-surface-tint-rgb), 0.38);
}

.mod-manager :deep(.indicator-dot):hover,
.mod-manager :deep(.indicator-dot).active {
    background: var(--theme-accent);
    box-shadow: 0 0 8px rgba(var(--theme-surface-tint-rgb), 0.34);
}

.mod-manager .custom-context-menu,
.mod-manager :deep(.has-submenu .submenu) {
    background: var(--t-material-bg);
    border-color: rgba(var(--theme-surface-tint-rgb), 0.10);
}

.mod-manager .inline-progress {
    border-color: rgba(var(--theme-surface-tint-rgb), 0.12);
}

:global(.export-archive-dialog) {
  z-index: 10020;
  background: var(--t-material-bg);
  border: var(--t-material-border);
  box-shadow: var(--t-material-shadow);
}

:global(.export-archive-dialog .el-dialog__header),
:global(.export-archive-dialog .el-dialog__body),
:global(.export-archive-dialog .el-dialog__footer) {
    background: transparent;
}

.export-output-row {
    display: flex;
    width: 100%;
    gap: 8px;
}

.export-output-row .el-input {
    flex: 1;
    min-width: 0;
}
</style>

<style>
.mod-3d-floating-overlay {
    position: fixed;
    top: 32px;
    right: 0;
    bottom: 0;
    left: 0;
    z-index: 2147482000;
    display: grid;
    place-items: center;
    padding: 20px;
    background: rgba(3, 6, 12, 0.72);
    backdrop-filter: blur(4px);
}

.mod-3d-floating-page {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    width: min(1380px, calc(100vw - 40px));
    height: min(900px, calc(100vh - 72px));
    min-height: 0;
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 16px;
    outline: none;
    background: rgba(10, 14, 22, 0.97);
    box-shadow: 0 28px 80px rgba(0, 0, 0, 0.58);
}

.mod-3d-floating-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 48px;
    padding: 0 15px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
}

.mod-3d-floating-header > div { display: flex; align-items: baseline; gap: 10px; min-width: 0; }
.mod-3d-floating-header strong { color: rgba(245, 248, 255, 0.94); font-size: 14px; }
.mod-3d-floating-header span { overflow: hidden; color: rgba(220, 229, 245, 0.58); text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
.mod-3d-floating-header button { width: 30px; height: 30px; border: 1px solid rgba(255,255,255,.12); border-radius: 7px; background: rgba(255,255,255,.05); color: #eef4ff; font-size: 20px; cursor: pointer; }

.mod-3d-floating-page > .mod-3d-preview {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(300px, 360px);
    grid-template-rows: auto minmax(0, 1fr);
    column-gap: 12px;
    height: 100%;
    min-height: 0;
    padding: 12px;
}

.mod-3d-floating-page > .mod-3d-preview > .mod-3d-preview__head { grid-column: 1 / -1; }
.mod-3d-floating-page > .mod-3d-preview > .mod-3d-preview__viewport { min-height: 0; }
.mod-3d-floating-page > .mod-3d-preview > .mod-3d-preview__controls {
    align-self: stretch;
    align-content: start;
    grid-auto-rows: max-content;
    min-height: 0;
    max-height: none;
    padding: 0 5px 8px 2px;
    overflow-x: hidden;
    overflow-y: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
}

.mod-3d-floating-fade-enter-active,
.mod-3d-floating-fade-leave-active { transition: opacity .16s ease; }
.mod-3d-floating-fade-enter-active .mod-3d-floating-page,
.mod-3d-floating-fade-leave-active .mod-3d-floating-page { transition: transform .16s ease, opacity .16s ease; }
.mod-3d-floating-fade-enter-from,
.mod-3d-floating-fade-leave-to { opacity: 0; }
.mod-3d-floating-fade-enter-from .mod-3d-floating-page,
.mod-3d-floating-fade-leave-to .mod-3d-floating-page { opacity: 0; transform: translateY(10px) scale(.985); }
</style>
