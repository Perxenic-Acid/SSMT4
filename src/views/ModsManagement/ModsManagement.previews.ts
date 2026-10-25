import { ref, reactive } from 'vue';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ModManager } from '../../store/ModManager';
import type { Ref } from 'vue';
import type { GroupInfo, ModInfo } from './ModsManagement.types';
import {
    loadSubgroupPreviewCache as loadSubgroupPreviewCacheStorage,
    persistSubgroupPreviewCache as persistSubgroupPreviewCacheStorage,
    SUBGROUP_PREVIEW_MAX_IMAGES,
} from './ModsManagement.storage';
import { normalizeManualOrderId } from './ModsManagement.paths';

interface UseModsManagementPreviewsOptions {
    selectedGame: Ref<string>;
    mods: Ref<ModInfo[]>;
    modPreviewIndices?: Record<string, number>;
    subgroupPreviewMap?: Ref<Record<string, string[]>>;
    subgroupPreviewIndices?: Record<string, number>;
    groupIconVersion?: Ref<number>;
    availableGroups?: Ref<GroupInfo[]>;
}

export const useModsManagementPreviews = (opts: UseModsManagementPreviewsOptions) => {
    // Use external reactive state if provided, otherwise create internal
    const modPreviewIndices = opts.modPreviewIndices ?? reactive<Record<string, number>>({});
    const subgroupPreviewMap = opts.subgroupPreviewMap ?? ref<Record<string, string[]>>(loadSubgroupPreviewCacheStorage());
    const subgroupPreviewIndices = opts.subgroupPreviewIndices ?? reactive<Record<string, number>>({});
    let subgroupPreviewLoadToken = 0;
    const dirtyPreviewKeys = new Set<string>();
    let subgroupPreviewPersistTimer: ReturnType<typeof setTimeout> | null = null;
    const MAX_AUTO_SWITCH_MODS = 20;
    const MAX_AUTO_SWITCH_SUBGROUPS = 12;

    const getStableModUiId = (mod: ModInfo) => normalizeManualOrderId(mod.id || mod.relativePath || mod.name);

    const getPreviewIndex = (mod: ModInfo) => modPreviewIndices[getStableModUiId(mod)] || 0;

    const getPreviewUrl = (mod: ModInfo) => {
        if (mod.previewImages && mod.previewImages.length > 0) {
            const index = getPreviewIndex(mod);
            const safeIndex = index < mod.previewImages.length ? index : 0;
            return convertFileSrc(mod.previewImages[safeIndex]);
        }
        return '';
    };

    const nextPreview = (mod: ModInfo) => {
        if (!mod.previewImages?.length) return;
        const current = getPreviewIndex(mod);
        modPreviewIndices[getStableModUiId(mod)] = (current + 1) % mod.previewImages.length;
    };

    const prevPreview = (mod: ModInfo) => {
        if (!mod.previewImages?.length) return;
        const current = getPreviewIndex(mod);
        modPreviewIndices[getStableModUiId(mod)] = (current - 1 + mod.previewImages.length) % mod.previewImages.length;
    };

    const setPreviewIndex = (mod: ModInfo, index: number) => {
        modPreviewIndices[getStableModUiId(mod)] = index;
    };

    // Auto-switch intervals
    let autoSwitchInterval: ReturnType<typeof setInterval> | null = null;
    let subgroupAutoSwitchInterval: ReturnType<typeof setInterval> | null = null;

    const startAutoSwitch = (shouldRunVisualEffects: Ref<boolean>, visibleSubGroups: Ref<GroupInfo[]>) => {
        autoSwitchInterval = setInterval(() => {
            if (!shouldRunVisualEffects.value) return;
            let switched = 0;
            opts.mods.value.forEach(mod => {
                if (switched >= MAX_AUTO_SWITCH_MODS) return;
                if (mod.previewImages && mod.previewImages.length > 1) { nextPreview(mod); switched += 1; }
            });
        }, 9000);

        subgroupAutoSwitchInterval = setInterval(() => {
            if (!shouldRunVisualEffects.value) return;
            let switched = 0;
            visibleSubGroups.value.forEach(group => {
                if (switched >= MAX_AUTO_SWITCH_SUBGROUPS) return;
                const key = makeSubgroupPreviewKey(group.id);
                const images = subgroupPreviewMap.value[key] || [];
                if (images.length > 1) {
                    subgroupPreviewIndices[key] = ((subgroupPreviewIndices[key] || 0) + 1) % images.length;
                    switched += 1;
                }
            });
        }, 6000);
    };

    const stopAutoSwitch = () => {
        if (autoSwitchInterval) clearInterval(autoSwitchInterval);
        if (subgroupAutoSwitchInterval) clearInterval(subgroupAutoSwitchInterval);
    };

    // Subgroup preview management
    const persistSubgroupPreviewCache = () => { persistSubgroupPreviewCacheStorage(subgroupPreviewMap.value); };

    const schedulePersistSubgroupPreviewCache = () => {
        if (subgroupPreviewPersistTimer) clearTimeout(subgroupPreviewPersistTimer);
        subgroupPreviewPersistTimer = setTimeout(() => { persistSubgroupPreviewCache(); }, 160);
    };

    const invalidateSubgroupPreviewByGroupId = (gameName: string, groupId: string) => {
        if (!gameName || !groupId) return;
        const key = `${gameName}::${groupId}`;
        dirtyPreviewKeys.add(key);
    };

    const invalidateSubgroupPreviewByGame = (gameName: string) => {
        if (!gameName) return;
        const prefix = `${gameName}::`;
        Object.keys(subgroupPreviewMap.value).forEach((key) => {
            if (key.startsWith(prefix)) dirtyPreviewKeys.add(key);
        });
    };

    const makeSubgroupPreviewKey = (groupId: string) => `${opts.selectedGame.value}::${groupId}`;

    const getSubgroupPreviewUrl = (group: GroupInfo) => {
        const key = makeSubgroupPreviewKey(group.id);
        const images = subgroupPreviewMap.value[key] || [];
        if (!images.length) return '';
        const idx = subgroupPreviewIndices[key] || 0;
        const safeIdx = idx < images.length ? idx : 0;
        return convertFileSrc(images[safeIdx]);
    };

    const preloadSubgroupPreviewImages = async (groups: GroupInfo[]) => {
        if (!opts.selectedGame.value || groups.length === 0) return;
        const token = ++subgroupPreviewLoadToken;
        const game = opts.selectedGame.value;
        for (const group of groups) {
            const key = `${game}::${group.id}`;
            if (Object.prototype.hasOwnProperty.call(subgroupPreviewMap.value, key) && !dirtyPreviewKeys.has(key)) continue;
            try {
                const cached = ModManager.getCachedGroup(game, group.id);
                const scanned = cached ?? await ModManager.scanGroup(game, group.id);
                const images: string[] = [];
                for (const mod of scanned.mods) {
                    if (!mod.enabled) continue;
                    if (mod.previewImages?.length) images.push(...mod.previewImages.slice(0, 2));
                }
                const validImages = Array.from(new Set(images)).slice(0, SUBGROUP_PREVIEW_MAX_IMAGES);
                if (token !== subgroupPreviewLoadToken) return;
                subgroupPreviewMap.value[key] = validImages;
                dirtyPreviewKeys.delete(key);
                if (!(key in subgroupPreviewIndices)) subgroupPreviewIndices[key] = 0;
                schedulePersistSubgroupPreviewCache();
            } catch {
                if (token !== subgroupPreviewLoadToken) return;
                // 临时读取失败时保留旧预览，后续加载仍可重试。
                dirtyPreviewKeys.add(key);
            }
        }
    };

    const getGroupIcon = (groupId: string) => {
        if (!groupId || groupId === 'Root') return null;
        const groups = opts.availableGroups?.value ?? [];
        return groups.find(g => g.id === groupId)?.iconPath;
    };

    const getGroupIconUrl = (iconPath: string) => {
        const src = convertFileSrc(iconPath);
        const separator = src.includes('?') ? '&' : '?';
        const version = opts.groupIconVersion?.value ?? Date.now();
        return `${src}${separator}v=${version}`;
    };

    return {
        getStableModUiId, getPreviewIndex, getPreviewUrl,
        nextPreview, prevPreview, setPreviewIndex,
        startAutoSwitch, stopAutoSwitch,
        invalidateSubgroupPreviewByGroupId, invalidateSubgroupPreviewByGame,
        makeSubgroupPreviewKey, getSubgroupPreviewUrl,
        preloadSubgroupPreviewImages,
        getGroupIcon, getGroupIconUrl,
        schedulePersistSubgroupPreviewCache,
        subgroupPreviewPersistTimer,
        MAX_AUTO_SWITCH_MODS, MAX_AUTO_SWITCH_SUBGROUPS,
    };
};
