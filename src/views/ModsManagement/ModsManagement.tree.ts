type TreeEntry = { id: string; children: TreeEntry[] };

export function createGroupTreeLoader<T extends TreeEntry>(
    getRoots: () => T[],
    getChildren: (node: T) => Promise<T[]>,
) {
    return async (node: { level: number; data: unknown }, resolve: (children: T[]) => void, reject: () => void) => {
        try {
            resolve(node.level === 0 ? getRoots() : await getChildren(node.data as T));
        } catch {
            // 失败不当作空分类缓存，允许再次点击箭头重试。
            reject();
        }
    };
}

/** 原位同步，保留 Element Plus 用于关联内部节点的对象及数组身份。 */
export function reconcileGroupTree<T extends TreeEntry>(current: T[], incoming: T[]): void {
    const existing = new Map(current.map(node => [node.id, node]));
    const next = incoming.map(source => {
        const target = existing.get(source.id);
        if (!target) return source;
        for (const key of Object.keys(source) as (keyof T)[]) {
            if (key !== 'children' && target[key] !== source[key]) target[key] = source[key];
        }
        reconcileGroupTree(target.children, source.children);
        return target;
    });
    if (current.length !== next.length || current.some((node, index) => node !== next[index])) {
        current.splice(0, current.length, ...next);
    }
}
