import type { ModInfo } from './ModsManagement.types';
import { normalizeModIdentity } from './ModsManagement.paths';

// 复用已显示对象，并一次提交完整结果，避免先删后加造成空帧。
export function reconcileModSnapshot(current: ModInfo[], incoming: ModInfo[], removeMissing = true): ModInfo[] {
    const byId = new Map(current.map(mod => [normalizeModIdentity(mod.relativePath || mod.id), mod]));
    const seen = new Set<string>();
    const next: ModInfo[] = [];
    for (const fresh of incoming) {
        const id = normalizeModIdentity(fresh.relativePath || fresh.id);
        if (!id || seen.has(id)) continue;
        seen.add(id);
        const existing = byId.get(id);
        if (existing) {
            Object.assign(existing, fresh);
            next.push(existing);
        } else next.push(fresh);
    }
    return removeMissing ? next : [...current, ...next.filter(mod => !byId.has(normalizeModIdentity(mod.relativePath || mod.id)))];
}
