export type ComponentTreeItem = {
	value: string;
	tabId: string;
	tabName: string;
	drawIB: string;
	ibLabel: string;
	componentLabel: string;
};

export const componentIbKey = (tabId: string, drawIB: string): string => JSON.stringify([tabId, drawIB]);

export function buildComponentTree<T extends ComponentTreeItem>(items: readonly T[]) {
	const lods = new Map<string, { key: string; label: string; ibs: Map<string, { key: string; label: string; items: T[] }> }>();
	for (const item of items) {
		let lod = lods.get(item.tabId);
		if (!lod) {
			lod = { key: item.tabId, label: item.tabName, ibs: new Map() };
			lods.set(item.tabId, lod);
		}
		const key = componentIbKey(item.tabId, item.drawIB);
		let ib = lod.ibs.get(key);
		if (!ib) {
			ib = { key, label: item.ibLabel, items: [] };
			lod.ibs.set(key, ib);
		}
		ib.items.push(item);
	}
	return [...lods.values()].map(lod => ({ ...lod, ibs: [...lod.ibs.values()] }));
}
