import { computed, reactive, ref } from 'vue';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessage, ElMessageBox } from 'element-plus';
import { ModManager } from '../../store/ModManager';
import { ModTagStore, type ModTagDefinition } from '../../store/ModTagStore';
import type { GroupInfo, ModInfo, ModTagDialogState, TagManagementDialogState } from './ModsManagement.types';
import type { Ref } from 'vue';

type Translate = (key: string, params?: Record<string, unknown>) => string;

interface UseModsManagementTagsOptions {
    selectedGame: Ref<string>;
    t: Translate;
    updateAvailableGroups: (groups: GroupInfo[]) => void;
    suppressFsRefresh: (durationMs?: number) => void;
}

export const useModsManagementTags = (options: UseModsManagementTagsOptions) => {
    const tagDefinitions = ref<ModTagDefinition[]>([]);
    const modTagMappings = ref<Record<string, string[]>>({});
    const tagIconPathMap = ref<Record<string, string>>({});
    const groupTagMappings = ref<Record<string, string[]>>({});
    const activeTagIds = ref<string[]>([]);
    const allModsCatalog = ref<ModInfo[]>([]);
    const allModsCatalogLoading = ref(false);
    const allModsCatalogLoadedForGame = ref('');
    const tagIconVersion = ref(Date.now());
    let allModsCatalogScanToken = 0;

    const tagManagementDialog = reactive<TagManagementDialogState>({
        visible: false,
        editingId: '',
        name: '',
        color: '#4D7CFE',
        iconSourcePath: '',
        removeIcon: false,
        saving: false,
    });

    const modTagDialog = reactive<ModTagDialogState>({
        visible: false,
        modId: '',
        modName: '',
        selectedTagIds: [],
        saving: false,
    });

    const tagMap = computed(() => new Map(tagDefinitions.value.map((tag) => [tag.id, tag])));
    const activeTags = computed(() => activeTagIds.value
        .map((tagId) => tagMap.value.get(tagId))
        .filter((tag): tag is ModTagDefinition => !!tag));
    const hasActiveTagFilter = computed(() => activeTagIds.value.length > 0);

    const resetTagState = () => {
        tagDefinitions.value = [];
        modTagMappings.value = {};
        groupTagMappings.value = {};
        tagIconPathMap.value = {};
        activeTagIds.value = [];
        allModsCatalog.value = [];
        allModsCatalogLoadedForGame.value = '';
    };

    const syncTagIconPathMap = async (gameName: string, tags: ModTagDefinition[]) => {
        const entries = await Promise.all(tags.map(async (tag) => {
            const iconPath = await ModTagStore.getTagIconPath(gameName, tag);
            return [tag.id, iconPath] as const;
        }));

        tagIconPathMap.value = Object.fromEntries(entries.filter(([, iconPath]) => !!iconPath));
        tagIconVersion.value = Date.now();
    };

    const loadTagState = async (gameName: string) => {
        const snapshot = await ModTagStore.load(gameName);
        tagDefinitions.value = snapshot.tags;
        modTagMappings.value = snapshot.modMappings;
        groupTagMappings.value = snapshot.groupMappings;
        await ModTagStore.cleanupUnusedIcons(gameName, snapshot.tags);
        await syncTagIconPathMap(gameName, snapshot.tags);
    };

    const refreshAllModsCatalog = async (optionsArg?: { force?: boolean }) => {
        if (!options.selectedGame.value) {
            allModsCatalog.value = [];
            allModsCatalogLoadedForGame.value = '';
            return;
        }

        if (!optionsArg?.force && allModsCatalogLoadedForGame.value === options.selectedGame.value && allModsCatalog.value.length > 0) {
            return;
        }

        const gameName = options.selectedGame.value;
        const scanToken = ++allModsCatalogScanToken;
        const scanSignal = {
            isCancelled: () => scanToken !== allModsCatalogScanToken || gameName !== options.selectedGame.value,
        };
        allModsCatalogLoading.value = true;
        try {
            const result = await ModManager.scanAllMods(gameName, scanSignal);
            if (scanSignal.isCancelled()) {
                return;
            }
            allModsCatalog.value = result.mods;
            allModsCatalogLoadedForGame.value = gameName;
            options.updateAvailableGroups(result.groups);
            // 扫描缓存不代表完整磁盘状态，刷新只能读取标签，不能据此删除用户数据。
            const snapshot = await ModTagStore.load(gameName);
            if (scanSignal.isCancelled()) return;
            tagDefinitions.value = snapshot.tags;
            modTagMappings.value = snapshot.modMappings;
            groupTagMappings.value = snapshot.groupMappings;
            await syncTagIconPathMap(gameName, snapshot.tags);
        } finally {
            if (!scanSignal.isCancelled()) {
                allModsCatalogLoading.value = false;
            }
        }
    };

    const getTagIdsForMod = (modId: string) => modTagMappings.value[String(modId || '').replace(/\\/g, '/')] || [];

    const getTagsForMod = (mod: ModInfo) => getTagIdsForMod(mod.relativePath)
        .map((tagId) => tagMap.value.get(tagId))
        .filter((tag): tag is ModTagDefinition => !!tag);

    const getTagUsageCount = (tagId: string) => Object.values(modTagMappings.value)
        .reduce((count, tagIds) => count + (tagIds.includes(tagId) ? 1 : 0), 0);

    const getTagIconUrl = (tag: ModTagDefinition) => {
        const iconPath = tagIconPathMap.value[tag.id];
        if (!iconPath) {
            return '';
        }
        return `${convertFileSrc(iconPath)}?v=${tagIconVersion.value}`;
    };

    const getEditingTag = () => tagDefinitions.value.find((tag) => tag.id === tagManagementDialog.editingId) || null;

    const getDraftTagIconPreviewUrl = () => {
        if (tagManagementDialog.iconSourcePath) {
            return convertFileSrc(tagManagementDialog.iconSourcePath);
        }

        if (!tagManagementDialog.removeIcon) {
            const editingTag = getEditingTag();
            if (editingTag) {
                return getTagIconUrl(editingTag);
            }
        }

        return '';
    };

    const getTagChipStyle = (tag: ModTagDefinition) => ({
        '--tag-accent': tag.color,
    } as Record<string, string>);

    const toggleActiveTag = async (tagId: string) => {
        activeTagIds.value = activeTagIds.value.includes(tagId)
            ? activeTagIds.value.filter((item) => item !== tagId)
            : [...activeTagIds.value, tagId];

        if (activeTagIds.value.length > 0) {
            await refreshAllModsCatalog();
        }
    };

    const clearActiveTags = () => {
        activeTagIds.value = [];
    };

    const syncTagStateAfterMutation = async (mutationOptions?: { reloadAllMods?: boolean }) => {
        if (!options.selectedGame.value) return;
        await loadTagState(options.selectedGame.value);
        if (hasActiveTagFilter.value || mutationOptions?.reloadAllMods) {
            allModsCatalogLoadedForGame.value = '';
            await refreshAllModsCatalog({ force: true });
        }
    };

    const resetTagManagementForm = () => {
        tagManagementDialog.editingId = '';
        tagManagementDialog.name = '';
        tagManagementDialog.color = '#4D7CFE';
        tagManagementDialog.iconSourcePath = '';
        tagManagementDialog.removeIcon = false;
        tagManagementDialog.saving = false;
    };

    const openTagManagementDialog = () => {
        resetTagManagementForm();
    };

    const editTagDefinition = (tag: ModTagDefinition) => {
        tagManagementDialog.editingId = tag.id;
        tagManagementDialog.name = tag.name;
        tagManagementDialog.color = tag.color;
        tagManagementDialog.iconSourcePath = '';
        tagManagementDialog.removeIcon = false;
    };

    const pickTagIconSource = async () => {
        const picked = await open({
            multiple: false,
            filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp'] }],
        });
        if (picked) {
            tagManagementDialog.iconSourcePath = picked;
            tagManagementDialog.removeIcon = false;
        }
    };

    const saveTagDefinition = async () => {
        if (!options.selectedGame.value) return;
        if (!tagManagementDialog.name.trim()) {
            ElMessage.warning(options.t('modsManagement.messages.enterTagName'));
            return;
        }

        tagManagementDialog.saving = true;
        try {
            options.suppressFsRefresh(1400);
            await ModTagStore.upsertTag(
                options.selectedGame.value,
                {
                    id: tagManagementDialog.editingId || undefined,
                    name: tagManagementDialog.name.trim(),
                    color: tagManagementDialog.color,
                },
                {
                    iconSourcePath: tagManagementDialog.iconSourcePath || undefined,
                    removeIcon: tagManagementDialog.removeIcon,
                },
            );
            await syncTagStateAfterMutation({ reloadAllMods: hasActiveTagFilter.value });
            ElMessage.success(tagManagementDialog.editingId
                ? options.t('modsManagement.messages.tagUpdated')
                : options.t('modsManagement.messages.tagCreated'));
            resetTagManagementForm();
        } catch (error) {
            ElMessage.error(options.t('modsManagement.messages.saveTagFailed', { error: String(error) }));
        } finally {
            tagManagementDialog.saving = false;
        }
    };

    const deleteTagDefinition = async (tag: ModTagDefinition) => {
        if (!options.selectedGame.value) return;
        try {
            await ElMessageBox.confirm(
                options.t('modsManagement.messages.deleteTagConfirm', { tag: tag.name }),
                options.t('modsManagement.dialog.deleteTagTitle'),
                {
                    confirmButtonText: options.t('modsManagement.common.delete'),
                    cancelButtonText: options.t('modsManagement.common.cancel'),
                    type: 'warning',
                },
            );
            options.suppressFsRefresh(1400);
            await ModTagStore.deleteTag(options.selectedGame.value, tag.id);
            activeTagIds.value = activeTagIds.value.filter((tagId) => tagId !== tag.id);
            await syncTagStateAfterMutation({ reloadAllMods: true });
            ElMessage.success(options.t('modsManagement.messages.tagDeleted'));
            if (tagManagementDialog.editingId === tag.id) {
                resetTagManagementForm();
            }
        } catch (error: unknown) {
            const action = typeof error === 'object' && error !== null && 'action' in error ? (error as { action: string }).action : undefined;
            if (error !== 'cancel' && action !== 'cancel') {
                ElMessage.error(options.t('modsManagement.messages.deleteTagFailed', { error: String(error) }));
            }
        }
    };

    const openModTagDialog = (mod: ModInfo) => {
        modTagDialog.visible = true;
        modTagDialog.modId = mod.relativePath;
        modTagDialog.modName = mod.name;
        modTagDialog.selectedTagIds = [...getTagIdsForMod(mod.relativePath)];
    };

    // ---- Group tag methods ----

    const getTagIdsForGroup = (groupId: string) => groupTagMappings.value[String(groupId || '').replace(/\\/g, '/')] || [];

    const getTagsForGroup = (group: GroupInfo) => getTagIdsForGroup(group.id)
        .map((tagId) => tagMap.value.get(tagId))
        .filter((tag): tag is ModTagDefinition => !!tag);

    const isNsfwTag = (tag: ModTagDefinition) => tag.name.trim().toLowerCase() === 'nsfw';

    const ensureNsfwTag = async () => {
        const existing = tagDefinitions.value.find(isNsfwTag);
        if (existing) return existing;

        options.suppressFsRefresh(1400);
        await ModTagStore.upsertTag(options.selectedGame.value, {
            name: 'NSFW',
            color: '#C33B53',
        });
        await loadTagState(options.selectedGame.value);
        const created = tagDefinitions.value.find(isNsfwTag);
        if (!created) throw new Error('Unable to create NSFW tag');
        return created;
    };

    const toggleNsfwAssignment = async (targetId: string, scope: 'mod' | 'group') => {
        if (!options.selectedGame.value || !targetId) return false;

        const tagIds = scope === 'mod'
            ? getTagIdsForMod(targetId)
            : getTagIdsForGroup(targetId);
        const existingNsfwTag = tagDefinitions.value.find(isNsfwTag);
        const isMarked = !!existingNsfwTag && tagIds.includes(existingNsfwTag.id);
        const nsfwTag = existingNsfwTag || await ensureNsfwTag();
        const nextTagIds = isMarked
            ? tagIds.filter((tagId) => tagId !== nsfwTag.id)
            : Array.from(new Set([...tagIds, nsfwTag.id]));

        options.suppressFsRefresh(1400);
        if (scope === 'mod') {
            await ModTagStore.setModTags(options.selectedGame.value, targetId, nextTagIds);
        } else {
            await ModTagStore.setGroupTags(options.selectedGame.value, targetId, nextTagIds);
        }
        await syncTagStateAfterMutation({ reloadAllMods: hasActiveTagFilter.value });
        return !isMarked;
    };

    const toggleModNsfw = (mod: ModInfo) => toggleNsfwAssignment(mod.relativePath, 'mod');
    const toggleGroupNsfw = (group: GroupInfo) => toggleNsfwAssignment(group.id, 'group');

    const openGroupTagDialog = (group: GroupInfo) => {
        modTagDialog.visible = true;
        modTagDialog.modId = group.id;
        modTagDialog.modName = group.name;
        modTagDialog.selectedTagIds = [...getTagIdsForGroup(group.id)];
    };

    const saveGroupTagAssignments = async () => {
        if (!options.selectedGame.value || !modTagDialog.modId) return;
        modTagDialog.saving = true;
        try {
            options.suppressFsRefresh(1400);
            await ModTagStore.setGroupTags(options.selectedGame.value, modTagDialog.modId, modTagDialog.selectedTagIds);
            await syncTagStateAfterMutation({ reloadAllMods: hasActiveTagFilter.value });
            modTagDialog.visible = false;
            ElMessage.success(options.t('modsManagement.messages.modTagsSaved'));
        } catch (error) {
            ElMessage.error(options.t('modsManagement.messages.saveModTagsFailed', { error: String(error) }));
        } finally {
            modTagDialog.saving = false;
        }
    };

    const saveModTagAssignments = async () => {
        if (!options.selectedGame.value || !modTagDialog.modId) return;
        modTagDialog.saving = true;
        try {
            options.suppressFsRefresh(1400);
            await ModTagStore.setModTags(options.selectedGame.value, modTagDialog.modId, modTagDialog.selectedTagIds);
            await syncTagStateAfterMutation({ reloadAllMods: hasActiveTagFilter.value });
            modTagDialog.visible = false;
            ElMessage.success(options.t('modsManagement.messages.modTagsSaved'));
        } catch (error) {
            ElMessage.error(options.t('modsManagement.messages.saveModTagsFailed', { error: String(error) }));
        } finally {
            modTagDialog.saving = false;
        }
    };

    return {
        tagDefinitions,
        modTagMappings,
        groupTagMappings,
        activeTagIds,
        allModsCatalog,
        allModsCatalogLoading,
        allModsCatalogLoadedForGame,
        tagIconVersion,
        tagManagementDialog,
        modTagDialog,
        tagMap,
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
        getTagIdsForGroup,
        getTagsForGroup,
        isNsfwTag,
        toggleModNsfw,
        toggleGroupNsfw,
        openGroupTagDialog,
        saveGroupTagAssignments,
    };
};
