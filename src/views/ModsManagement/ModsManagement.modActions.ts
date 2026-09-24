import { ElMessage, ElMessageBox } from 'element-plus';
import { openPath } from '@tauri-apps/plugin-opener';
import { open } from '@tauri-apps/plugin-dialog';
import { rename } from '@tauri-apps/plugin-fs';
import { ModManager, type DisabledParentGroupInfo } from '../../store/ModManager';
import { ModTagStore } from '../../store/ModTagStore';
import { ModStateStore } from '../../store/ModStateStore';
import { migotoIniService } from './MigotoIni';
import { captureModRuntimeState, restoreModRuntimeState } from './ModStateMemory';
import { clipboardImageToPngBytes } from './ModsManagement.clipboardPreview';
import { readImage } from '@tauri-apps/plugin-clipboard-manager';
import type { Ref } from 'vue';
import type { GroupInfo, ModInfo } from './ModsManagement.types';
import {
    buildMovedModRelativePath as buildMovedModRelativePathUtil,
    buildRenamedModRelativePath as buildRenamedModRelativePathUtil,
    getModGroupFromRelativePath as getModGroupFromRelativePathUtil,
    isSameOrChildPath as isSameOrChildPathUtil,
    replacePathPrefix as replacePathPrefixUtil,
    replaceFsPathSegment as replaceFsPathSegmentUtil,
} from './ModsManagement.paths';

type Translate = (key: string, params?: Record<string, unknown>) => string;

interface UseModsManagementModActionsOptions {
    selectedGame: Ref<string>;
    selectedGroup: Ref<string>;
    mods: Ref<ModInfo[]>;
    availableGroups: Ref<GroupInfo[]>;
    currentSubGroups: Ref<GroupInfo[]>;
    t: Translate;
    suppressFsRefresh: (ms?: number) => void;
    syncTagStateAfterMutation: (opts?: { reloadAllMods?: boolean }) => Promise<void>;
    refreshAfterMutation: (opts?: { structural?: boolean; full?: boolean }) => Promise<void>;
    allModsCatalogLoadedForGame: Ref<string>;
    invalidateSubgroupPreviewByGroupId: (game: string, groupId: string) => void;
    triggerModPulse: (modId: string) => void;
    getModsByGroup: (group: string) => ModInfo[];
    // State migration helpers
    migrateSelectedGroupPrefix: (oldPrefix: string, newPrefix: string) => void;
    migrateExpandedStatePrefix: (oldPrefix: string, newPrefix: string) => void;
    migrateGroupOrderPrefix: (oldPrefix: string, newPrefix: string) => void;
    migrateManualOrderPrefix: (oldPrefix: string, newPrefix: string) => void;
    migrateSubgroupPreviewCachePrefix: (oldPrefix: string, newPrefix: string) => void;
    bumpGroupTree: () => void;
    // Key state
    modKeyLists: Record<string, any[]>;
    modKeyLoadingState: Record<string, boolean>;
    modKeyErrorState: Record<string, Record<string, string>>;
    hoveredKeyModId: Ref<string | null>;
    modKeyEditorDialog: { visible: boolean; modId: string; modRelativePath: string; modPath: string; modName: string; loading: boolean; saving: boolean; items: any[] };
}

// Re-export utilities for convenience
const buildMovedModRelativePath = (mod: ModInfo, targetGroupId: string) => buildMovedModRelativePathUtil(mod, targetGroupId);
const buildRenamedModRelativePath = (mod: ModInfo, newName: string) => buildRenamedModRelativePathUtil(mod, newName);
const getModGroupFromRelativePath = (relativePath: string) => getModGroupFromRelativePathUtil(relativePath);
const isSameOrChildPath = (value: string, prefix: string) => isSameOrChildPathUtil(value, prefix);
const replacePathPrefix = (value: string, oldPrefix: string, newPrefix: string) => replacePathPrefixUtil(value, oldPrefix, newPrefix);
const replaceFsPathSegment = (value: string, oldRelativePath: string, newRelativePath: string) => replaceFsPathSegmentUtil(value, oldRelativePath, newRelativePath);
export const useModsManagementModActions = (opts: UseModsManagementModActionsOptions) => {
    const resolveGroupPhysicalPath = (groupId: string) => {
        if (!groupId || groupId === 'Root' || groupId === 'All') {
            return 'Root';
        }

        return opts.availableGroups.value.find((group) => group.id === groupId)?.path
            || opts.currentSubGroups.value.find((group) => group.id === groupId)?.path
            || groupId;
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

            return {
                ...group,
                id: affectsIdentity ? replacePathPrefix(group.id, oldGroupId, newGroupId) : group.id,
                path: affectsPath ? replacePathPrefix(group.path, oldPhysicalPath, newPhysicalPath) : group.path,
                enabled: group.id === oldGroupId ? enabled : group.enabled,
            };
        };

        opts.availableGroups.value = opts.availableGroups.value.map(updateGroup);
        opts.currentSubGroups.value = opts.currentSubGroups.value.map(updateGroup);
        opts.mods.value = opts.mods.value.map((mod) => {
            const affectsGroup = isSameOrChildPath(mod.group, oldGroupId);
            const affectsRelativePath = isSameOrChildPath(mod.relativePath, oldPhysicalPath);
            const affectsId = isSameOrChildPath(mod.id, oldPhysicalPath);
            if (!affectsGroup && !affectsRelativePath && !affectsId) return mod;
            const nextRelativePath = affectsRelativePath ? replacePathPrefix(mod.relativePath, oldPhysicalPath, newPhysicalPath) : mod.relativePath;
            return {
                ...mod, id: affectsId ? replacePathPrefix(mod.id, oldPhysicalPath, newPhysicalPath) : mod.id,
                relativePath: nextRelativePath,
                group: affectsGroup ? replacePathPrefix(mod.group, oldGroupId, newGroupId) : mod.group,
                path: replaceFsPathSegment(mod.path, mod.relativePath, nextRelativePath),
            };
        });
        opts.migrateSelectedGroupPrefix(oldGroupId, newGroupId);
        opts.migrateExpandedStatePrefix(oldGroupId, newGroupId);
        opts.migrateGroupOrderPrefix(oldGroupId, newGroupId);
        opts.migrateManualOrderPrefix(oldGroupId, newGroupId);
        opts.migrateManualOrderPrefix(oldPhysicalPath, newPhysicalPath);
        opts.migrateSubgroupPreviewCachePrefix(oldGroupId, newGroupId);
    };

    const migrateEnabledParentGroupsLocalState = (enabledGroups: DisabledParentGroupInfo[]) => {
        if (enabledGroups.length === 0 || !opts.selectedGame.value) return;

        enabledGroups.forEach((group) => {
            migrateGroupRenameLocalState(group.enabledPath, group.enabledPath, true, {
                oldPhysicalPath: group.disabledPath,
                newPhysicalPath: group.enabledPath,
            });
            opts.invalidateSubgroupPreviewByGroupId(opts.selectedGame.value, group.disabledPath);
            opts.invalidateSubgroupPreviewByGroupId(opts.selectedGame.value, group.enabledPath);
        });

        ModManager.clearCache(opts.selectedGame.value);
        opts.bumpGroupTree();
    };

    const updateModToggleLocalState = (mod: ModInfo, nextRelativePath: string, enabled: boolean) => {
        const previousId = mod.id;
        const previousRelativePath = mod.relativePath;
        const previousPath = mod.path;
        const nextPath = replaceFsPathSegment(previousPath, previousRelativePath, nextRelativePath);
        const nextGroup = getModGroupFromRelativePath(nextRelativePath);
        if (previousId !== nextRelativePath) {
            if (Object.prototype.hasOwnProperty.call(opts.modKeyLists, previousId)) {
                opts.modKeyLists[nextRelativePath] = opts.modKeyLists[previousId];
                delete opts.modKeyLists[previousId];
            }
            if (Object.prototype.hasOwnProperty.call(opts.modKeyLoadingState, previousId)) {
                opts.modKeyLoadingState[nextRelativePath] = opts.modKeyLoadingState[previousId];
                delete opts.modKeyLoadingState[previousId];
            }
            if (Object.prototype.hasOwnProperty.call(opts.modKeyErrorState, previousId)) {
                opts.modKeyErrorState[nextRelativePath] = opts.modKeyErrorState[previousId];
                delete opts.modKeyErrorState[previousId];
            }
            if (opts.hoveredKeyModId.value === previousId) opts.hoveredKeyModId.value = nextRelativePath;
            if (opts.modKeyEditorDialog.modId === previousId) {
                opts.modKeyEditorDialog.modId = nextRelativePath;
                opts.modKeyEditorDialog.modRelativePath = nextRelativePath;
                opts.modKeyEditorDialog.modPath = nextPath;
            }
        }
        mod.enabled = enabled;
        mod.id = nextRelativePath;
        mod.relativePath = nextRelativePath;
        mod.group = nextGroup;
        mod.path = nextPath;
        mod.previewImages = mod.previewImages || [];
        const updatedMod = { ...mod, group: nextGroup, previewImages: mod.previewImages };
        const found = opts.mods.value.some((item) =>
            item === mod || item.id === previousId || item.relativePath === previousRelativePath || item.path === previousPath);
        opts.mods.value = found
            ? opts.mods.value.map((item) => {
                if (item === mod || item.id === previousId || item.relativePath === previousRelativePath || item.path === previousPath) return updatedMod;
                return item;
            })
            : [updatedMod, ...opts.mods.value];
    };

    const moveModToGroup = async (mod: ModInfo, groupName: string) => {
        try {
            const targetPhysicalPath = resolveGroupPhysicalPath(groupName);
            const nextRelativePath = buildMovedModRelativePath(mod, targetPhysicalPath);
            await ModManager.moveModToGroup(opts.selectedGame.value, mod.relativePath, targetPhysicalPath);
            opts.suppressFsRefresh(1400);
            await ModTagStore.remapModPath(opts.selectedGame.value, mod.relativePath, nextRelativePath);
            await ModStateStore.remapModPath(opts.selectedGame.value, mod.relativePath, nextRelativePath);
            await opts.syncTagStateAfterMutation({ reloadAllMods: opts.allModsCatalogLoadedForGame.value === opts.selectedGame.value });
            ElMessage.success(opts.t('modsManagement.messages.movedToGroupSuccess', {
                group: groupName === 'Root' || !groupName ? opts.t('modsManagement.actions.moveToModsRoot') : groupName,
            }));
            await opts.refreshAfterMutation({ structural: true, full: true });
            if (groupName === 'Root' || !groupName) {
                await ModManager.openGameModsFolder(opts.selectedGame.value);
            } else {
                await ModManager.openModGroupFolder(opts.selectedGame.value, targetPhysicalPath);
            }
        } catch (e: unknown) {
            const message = String(e);
            const localizedNotFound = opts.t('modManager.messages.modNotFound');
            if (message.includes(localizedNotFound) || message.includes('Mod not found') || message.includes('未找到 Mod')) {
                await opts.refreshAfterMutation({ structural: true, full: true });
                ElMessage.warning(opts.t('modsManagement.messages.modCacheRefreshedAfterNotFound'));
                return;
            }
            ElMessage.error(opts.t('modsManagement.messages.moveFailed', { error: String(e) }));
        }
    };

    const renameMod = async (mod: ModInfo) => {
        try {
            const result = await ElMessageBox.prompt(
                opts.t('modsManagement.messages.enterNewModName'), opts.t('modsManagement.dialog.renameModTitle'),
                { confirmButtonText: opts.t('modsManagement.common.confirm'), cancelButtonText: opts.t('modsManagement.common.cancel'),
                  inputValue: mod.name, inputPattern: /^[^\\/:*?"<>|]+$/, inputErrorMessage: opts.t('modsManagement.messages.nameContainsInvalidCharacters') }
            ) as { value: string };
            const newName = result.value;
            if (newName && newName !== mod.name) {
                const nextRelativePath = buildRenamedModRelativePath(mod, newName.trim());
                await ModManager.renameMod(opts.selectedGame.value, mod.relativePath, newName.trim());
                opts.suppressFsRefresh(1400);
                await ModTagStore.remapModPath(opts.selectedGame.value, mod.relativePath, nextRelativePath);
                await ModStateStore.remapModPath(opts.selectedGame.value, mod.relativePath, nextRelativePath);
                await opts.syncTagStateAfterMutation({ reloadAllMods: opts.allModsCatalogLoadedForGame.value === opts.selectedGame.value });
                ElMessage.success(opts.t('modsManagement.messages.renamedSuccessfully'));
                await opts.refreshAfterMutation();
            }
        } catch { /* cancelled */ }
    };

    const deleteMod = async (mod: ModInfo) => {
        try {
            await ElMessageBox.confirm(
                opts.t('modsManagement.messages.deleteModConfirm', { mod: mod.name }), opts.t('modsManagement.dialog.deleteModTitle'),
                { confirmButtonText: opts.t('modsManagement.common.delete'), cancelButtonText: opts.t('modsManagement.common.cancel'), type: 'warning' }
            );
            await ModManager.deleteMod(opts.selectedGame.value, mod.relativePath);
            opts.suppressFsRefresh(1400);
            await ModTagStore.deleteModMapping(opts.selectedGame.value, mod.relativePath);
            await opts.syncTagStateAfterMutation({ reloadAllMods: opts.allModsCatalogLoadedForGame.value === opts.selectedGame.value });
            ElMessage.success(opts.t('modsManagement.messages.modDeleted'));
            await opts.refreshAfterMutation();
        } catch (e: unknown) { if (e !== 'cancel') { ElMessage.error(opts.t('modsManagement.messages.deleteFailed', { error: String(e) })); } }
    };

    const toggleMod = async (mod: ModInfo) => {
        const targetState = !mod.enabled;
        let enabledParentGroups: DisabledParentGroupInfo[] = [];
        try {
            if (!targetState && opts.selectedGame.value) {
                try {
                    const analysis = await migotoIniService.load(opts.selectedGame.value, mod);
                    await captureModRuntimeState(opts.selectedGame.value, mod, analysis.modKeyList);
                } catch (memoryError) { console.warn('Failed to capture mod runtime state before disable:', memoryError); }
            }
            const disabledParentGroups = targetState
                ? await ModManager.getDisabledParentGroups(opts.selectedGame.value, mod.relativePath) : [];
            let modPathAfterParentEnable = mod.relativePath;
            if (disabledParentGroups.length > 0) {
                const categoryList = disabledParentGroups.map((group) => group.enabledPath).join('\n');
                try {
                    await ElMessageBox.confirm(
                        opts.t('modsManagement.messages.forceEnableDisabledParentCategoriesConfirm', { categories: categoryList }),
                        opts.t('modsManagement.dialog.forceEnableCategoryTitle'),
                        { confirmButtonText: opts.t('modsManagement.common.confirm'), cancelButtonText: opts.t('modsManagement.common.cancel'), type: 'warning' },
                    );
                } catch { return; }
                opts.suppressFsRefresh(3500);
                for (const group of disabledParentGroups) {
                    await ModManager.toggleGroup(opts.selectedGame.value, group.disabledPath, true);
                    enabledParentGroups.push(group);
                    await ModTagStore.remapPrefix(opts.selectedGame.value, group.disabledPath, group.enabledPath);
                    await ModTagStore.remapGroupPrefix(opts.selectedGame.value, group.disabledPath, group.enabledPath);
                    await ModStateStore.remapPrefix(opts.selectedGame.value, group.disabledPath, group.enabledPath);
                    modPathAfterParentEnable = replacePathPrefix(modPathAfterParentEnable, group.disabledPath, group.enabledPath);
                }
            }
            opts.suppressFsRefresh(2500);
            opts.invalidateSubgroupPreviewByGroupId(opts.selectedGame.value, mod.group);
            const previousRelativePath = mod.relativePath;
            const nextRelativePath = await ModManager.toggleMod(opts.selectedGame.value, modPathAfterParentEnable, targetState);
            await ModTagStore.remapModPath(opts.selectedGame.value, modPathAfterParentEnable, nextRelativePath);
            await ModStateStore.remapModPath(opts.selectedGame.value, previousRelativePath, nextRelativePath);
            if (targetState && opts.selectedGame.value) {
                try { await restoreModRuntimeState(opts.selectedGame.value, { relativePath: nextRelativePath }); }
                catch (memoryError) { console.warn('Failed to restore mod runtime state after enable:', memoryError); }
            }
            await opts.syncTagStateAfterMutation({ reloadAllMods: opts.allModsCatalogLoadedForGame.value === opts.selectedGame.value });
            updateModToggleLocalState(mod, nextRelativePath, targetState);
            migrateEnabledParentGroupsLocalState(enabledParentGroups);
            opts.triggerModPulse(mod.id);
            ElMessage.success(opts.t('modsManagement.messages.modStateChanged', {
                mod: mod.name, state: targetState ? opts.t('modsManagement.common.enabled') : opts.t('modsManagement.common.disabled'),
            }));
        } catch (error) {
            console.error('Failed to toggle mod:', error);
            if (enabledParentGroups.length > 0) {
                await opts.refreshAfterMutation({ structural: true, full: true });
            }
            ElMessage.error(opts.t('modsManagement.messages.operationFailed', { error: String(error) }));
        }
    };

    const enableModSolo = async (mod: ModInfo) => {
        const game = opts.selectedGame.value;
        if (!game) return;
        const group = mod.group;
        const siblings = opts.getModsByGroup(group).filter(m => m.id !== mod.id && m.enabled);
        if (siblings.length > 0) {
            opts.suppressFsRefresh(siblings.length * 500 + 2000);
            for (const sibling of siblings) {
                try {
                    const nextPath = await ModManager.toggleMod(game, sibling.relativePath, false);
                    await ModTagStore.remapModPath(game, sibling.relativePath, nextPath);
                    await ModStateStore.remapModPath(game, sibling.relativePath, nextPath);
                    updateModToggleLocalState(sibling, nextPath, false);
                    opts.invalidateSubgroupPreviewByGroupId(game, sibling.group);
                } catch (e) { console.error('Failed to disable sibling mod:', sibling.name, e); }
            }
        }
        if (!mod.enabled) { await toggleMod(mod); }
        await opts.syncTagStateAfterMutation({ reloadAllMods: opts.allModsCatalogLoadedForGame.value === game });
        await opts.refreshAfterMutation();
        ElMessage.success(opts.t('modsManagement.messages.enableModSoloSuccess', { mod: mod.name }));
    };

    const addPreviewImages = async (mod: ModInfo) => {
        try {
            const selected = await open({ multiple: true, filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'bmp', 'webp', 'gif'] }] });
            if (selected && Array.isArray(selected) && selected.length > 0) {
                await ModManager.addModPreviewImages(opts.selectedGame.value, mod.relativePath, selected);
                opts.invalidateSubgroupPreviewByGroupId(opts.selectedGame.value, mod.group);
                ElMessage.success(opts.t('modsManagement.messages.addedPreviewImages', { count: selected.length }));
                await opts.refreshAfterMutation();
            }
        } catch (err) { console.error('Failed to add preview images', err); ElMessage.error(opts.t('modsManagement.messages.addPreviewImagesFailed')); }
    };

    const pasteClipboardPreviewImage = async (mod: ModInfo) => {
        try {
            const image = await readImage();
            const pngBytes = await clipboardImageToPngBytes(image as any);
            const fileName = `preview_clipboard_${Date.now()}.png`;
            await ModManager.addModPreviewImageData(opts.selectedGame.value, mod.relativePath, fileName, pngBytes);
            opts.invalidateSubgroupPreviewByGroupId(opts.selectedGame.value, mod.group);
            ElMessage.success(opts.t('modsManagement.messages.pastedPreviewImage'));
            await opts.refreshAfterMutation();
        } catch (err) { console.error('Failed to paste preview image from clipboard', err); ElMessage.error(opts.t('modsManagement.messages.pastePreviewImageFailed')); }
    };

    const convertGroupToMod = async (group: GroupInfo) => {
        const game = opts.selectedGame.value;
        if (!game) return;
        try {
            const groupPhysicalPath = group.path || group.id;
            const iniFiles = await ModManager.findNestedIniFiles(game, group.id);
            if (iniFiles.length === 0) { ElMessage.warning(opts.t('modsManagement.messages.convertCategoryToModNone')); return; }
            const selectedIni = iniFiles.sort((a, b) => a.split('/').length - b.split('/').length)[0];
            try {
                await ElMessageBox.confirm(
                    opts.t('modsManagement.messages.convertCategoryToModHint', { group: group.name }) + '\n\n' + iniFiles.map(f => `  • ${f}`).join('\n'),
                    opts.t('modsManagement.messages.convertCategoryToModTitle'),
                    { confirmButtonText: opts.t('modsManagement.common.confirm'), cancelButtonText: opts.t('modsManagement.common.cancel'), type: 'info' }
                );
            } catch { return; }
            const modsDir = await ModManager.getInstallDir(game);
            const modsRoot = `${modsDir.replace(/\\/g, '/').replace(/\/+$/, '')}/Mods`;
            const groupDir = `${modsRoot}/${groupPhysicalPath}`;
            const iniFileName = selectedIni.split('/').pop() || 'mod.ini';
            const srcPath = `${groupDir}/${selectedIni}`;
            const destPath = `${groupDir}/${iniFileName}`;
            await rename(srcPath, destPath);
            opts.suppressFsRefresh(2000);
            await opts.refreshAfterMutation({ structural: true, full: true });
            ElMessage.success(opts.t('modsManagement.messages.convertCategoryToModSuccess', { group: group.name }));
        } catch (e) { console.error('Failed to convert group to mod:', e); ElMessage.error(opts.t('modsManagement.messages.operationFailed', { error: String(e) })); }
    };

    const openModFolder = async (path: string) => { try { await openPath(path); } catch (error) { console.error(error); } };
    const openGameFolder = async () => { try { await ModManager.openGameModsFolder(opts.selectedGame.value); } catch (error) { console.error(error); } };

    return {
        migrateGroupRenameLocalState, updateModToggleLocalState,
        moveModToGroup, renameMod, deleteMod,
        toggleMod, enableModSolo,
        addPreviewImages, pasteClipboardPreviewImage,
        convertGroupToMod,
        openModFolder, openGameFolder,
    };
};
