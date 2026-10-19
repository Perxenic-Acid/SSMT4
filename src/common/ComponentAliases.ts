import { join } from '@tauri-apps/api/path';
import { exists, readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';

export const COMPONENT_ALIASES_FILE = 'ComponentAliases.json';
export const COMPONENT_ALIAS_MAX_LENGTH = 128;
export type ComponentAliases = Record<string, string>;

// 键是提取目录的稳定名称，不是可能重新编号的 Component 序号；别名只参与显示。
export const isComponentKey = (key: string): boolean => /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(key);
export const isComponentAlias = (alias: string): boolean =>
	Array.from(alias).length <= COMPONENT_ALIAS_MAX_LENGTH && !/[\u0000-\u001f\u007f]/.test(alias);

export function parseComponentAliases(content: string): ComponentAliases {
	const value: unknown = JSON.parse(content);
	if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid ComponentAliases.json');
	const entries = Object.entries(value);
	if (entries.length > 4096 || entries.some(([key, alias]) => !isComponentKey(key) || typeof alias !== 'string' || !isComponentAlias(alias))) {
		throw new Error('Invalid ComponentAliases.json');
	}
	return Object.fromEntries(entries.map(([key, alias]) => [key, (alias as string).trim()]).filter(([, alias]) => alias));
}

export async function readComponentAliases(workspaceLodPath: string): Promise<ComponentAliases> {
	const path = await join(workspaceLodPath, COMPONENT_ALIASES_FILE);
	return await exists(path) ? parseComponentAliases(await readTextFile(path)) : {};
}

export async function writeComponentAlias(workspaceLodPath: string, subMeshName: string, alias: string): Promise<ComponentAliases> {
	if (!isComponentKey(subMeshName) || !isComponentAlias(alias)) throw new Error('Invalid Component alias');
	// 重新读取，保留其他 Component 的别名；损坏时不覆盖原文件。
	const aliases = await readComponentAliases(workspaceLodPath);
	const normalized = alias.trim();
	if (normalized) aliases[subMeshName] = normalized;
	else delete aliases[subMeshName];
	if (Object.keys(aliases).length > 4096) throw new Error('Too many Component aliases');
	await writeTextFile(await join(workspaceLodPath, COMPONENT_ALIASES_FILE), JSON.stringify(aliases, null, 2));
	return aliases;
}
