import type { GroupInfo, ModInfo } from './ModsManagement.types';

export const ROOT_PARENT_ID = '__ROOT__';

export const getGroupParent = (id: string) => {
    const parts = id.split('/');
    if (parts.length <= 1) return ROOT_PARENT_ID;
    return parts.slice(0, -1).join('/');
};

export const isSameOrChildPath = (value: string, prefix: string) => {
    const normalizedValue = String(value || '').replace(/\\/g, '/');
    const normalizedPrefix = String(prefix || '').replace(/\\/g, '/');
    if (!normalizedValue || !normalizedPrefix) return false;
    return normalizedValue === normalizedPrefix || normalizedValue.startsWith(`${normalizedPrefix}/`);
};

export const replacePathPrefix = (value: string, oldPrefix: string, newPrefix: string) => {
    const normalizedValue = String(value || '').replace(/\\/g, '/');
    const normalizedOld = String(oldPrefix || '').replace(/\\/g, '/');
    const normalizedNew = String(newPrefix || '').replace(/\\/g, '/');

    if (!normalizedOld || !isSameOrChildPath(normalizedValue, normalizedOld)) {
        return normalizedValue;
    }

    if (normalizedValue === normalizedOld) {
        return normalizedNew;
    }

    return `${normalizedNew}/${normalizedValue.slice(normalizedOld.length + 1)}`;
};

export const replaceFsPathSegment = (value: string, oldRelativePath: string, newRelativePath: string) => {
    const normalizedValue = String(value || '').replace(/\\/g, '/');
    const normalizedOld = String(oldRelativePath || '').replace(/\\/g, '/');
    const normalizedNew = String(newRelativePath || '').replace(/\\/g, '/');
    if (!normalizedValue || !normalizedOld) return normalizedValue;

    const exactSuffix = `/${normalizedOld}`;
    const childSuffix = `/${normalizedOld}/`;

    if (normalizedValue.endsWith(exactSuffix)) {
        return `${normalizedValue.slice(0, -exactSuffix.length)}/${normalizedNew}`;
    }

    const childIndex = normalizedValue.indexOf(childSuffix);
    if (childIndex !== -1) {
        return `${normalizedValue.slice(0, childIndex)}/${normalizedNew}/${normalizedValue.slice(childIndex + childSuffix.length)}`;
    }

    return normalizedValue;
};

export const renameKeyedRecord = <T,>(source: Record<string, T>, oldPrefix: string, newPrefix: string) => {
    const renamed: Record<string, T> = {};
    Object.entries(source).forEach(([key, value]) => {
        renamed[replacePathPrefix(key, oldPrefix, newPrefix)] = value;
    });
    return renamed;
};

export const getGroupAncestors = (id: string) => {
    if (!id || id === 'All' || id === 'Root') return [] as string[];
    const parts = id.split('/');
    const ancestors: string[] = [];
    for (let i = 1; i < parts.length; i += 1) {
        ancestors.push(parts.slice(0, i).join('/'));
    }
    return ancestors;
};

export const stripDisabledFolderName = (folderName: string) => {
    const upper = folderName.toUpperCase();
    if (upper.startsWith('DISABLED_')) {
        return { cleanName: folderName.substring(9), hasUnderscore: true, disabled: true };
    }
    if (upper.startsWith('DISABLED')) {
        return { cleanName: folderName.substring(8), hasUnderscore: false, disabled: true };
    }
    return { cleanName: folderName, hasUnderscore: false, disabled: false };
};

export const normalizeGroupId = (groupId: string) => {
    const normalized = String(groupId || '').replace(/\\/g, '/').replace(/^\/+|\/+$/g, '');
    if (!normalized || normalized === 'Root' || normalized === 'All') return normalized;
    return normalized
        .split('/')
        .filter(Boolean)
        .map((segment) => {
            const { cleanName } = stripDisabledFolderName(segment);
            return cleanName || segment;
        })
        .join('/');
};

export const normalizeModIdentity = (relativePath: string) => String(relativePath || '')
    .replace(/\\/g, '/')
    .replace(/^\/+|\/+$/g, '')
    .split('/')
    .filter(Boolean)
    .map((segment) => {
        const { cleanName } = stripDisabledFolderName(segment);
        return (cleanName || segment).toLowerCase();
    })
    .join('/');

export const buildRenamedModRelativePath = (mod: ModInfo, newName: string) => {
    const parts = mod.relativePath.split('/');
    const currentFolderName = parts.pop() || '';
    const parentPath = parts.join('/');
    const { disabled, hasUnderscore } = stripDisabledFolderName(currentFolderName);
    const nextFolderName = disabled
        ? hasUnderscore
            ? `DISABLED_${newName}`
            : `DISABLED${newName}`
        : newName;
    return [parentPath, nextFolderName].filter(Boolean).join('/');
};

export const buildMovedModRelativePath = (mod: ModInfo, targetGroupId: string) => {
    const folderName = mod.relativePath.split('/').pop() || '';
    const targetParent = targetGroupId === 'Root' || !targetGroupId ? '' : targetGroupId;
    return [targetParent, folderName].filter(Boolean).join('/');
};

export const getModGroupFromRelativePath = (relativePath: string) => {
    const parts = String(relativePath || '').replace(/\\/g, '/').split('/').filter(Boolean);
    parts.pop();
    const groupParts = parts
        .map((segment) => {
            const { cleanName } = stripDisabledFolderName(segment);
            return cleanName || segment;
        })
        .filter(Boolean);
    return groupParts.length > 0 ? groupParts.join('/') : 'Root';
};

export const updateGroupsWithPrefixRename = (sourceGroups: GroupInfo[], oldPrefix: string, newPrefix: string, enabled?: boolean) => {
    return sourceGroups.map((group) => {
        if (!isSameOrChildPath(group.id, oldPrefix)) {
            return group;
        }

        const isExact = group.id === oldPrefix;
        return {
            ...group,
            id: replacePathPrefix(group.id, oldPrefix, newPrefix),
            path: replacePathPrefix(group.path, oldPrefix, newPrefix),
            enabled: isExact && typeof enabled === 'boolean' ? enabled : group.enabled,
        };
    });
};

export const normalizeManualOrderId = (id: string) => {
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

export const isDisabledGroup = (group: GroupInfo) => {
    const name = (group.name || '').toLowerCase();
    const idTail = (group.id || '').split('/').pop()?.toLowerCase() || '';
    return !group.enabled || name.startsWith('disabled') || idTail.startsWith('disabled');
};

export const dedupeGroupsById = (groups: GroupInfo[]) => {
    const deduped = new Map<string, GroupInfo>();

    for (const group of groups) {
        if (!group?.id) continue;
        const existing = deduped.get(group.id);
        if (!existing) {
            deduped.set(group.id, { ...group });
            continue;
        }

        const preferIncoming = group.enabled && !existing.enabled;
        const preferred = preferIncoming ? group : existing;
        const fallback = preferIncoming ? existing : group;
        const modCount = group.enabled === existing.enabled
            ? Math.max(existing.modCount ?? 0, group.modCount ?? 0)
            : (preferred.modCount ?? 0);
        deduped.set(group.id, {
            ...fallback,
            ...preferred,
            iconPath: preferred.iconPath ?? fallback.iconPath,
            modCount,
            isDirectoryLink: Boolean(existing.isDirectoryLink || group.isDirectoryLink),
        });
    }

    return Array.from(deduped.values());
};
