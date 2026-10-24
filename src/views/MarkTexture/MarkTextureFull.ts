import { join } from '@tauri-apps/api/path'
import { copyFile, exists, readDir, readTextFile, remove, writeTextFile } from '@tauri-apps/plugin-fs'
import { applyTextureMemoryToItems, loadTextureMemoryByPSHash } from '../../common/TextureMarkMemory'
import { SSMTStringUtils } from '../../utils/SSMTStringUtils'

export type MarkStyle = 'Hash' | 'Slot' | 'SharedSlot'
export type SubMeshRole = 'Face' | 'Neck' | 'Eye'

export type TextureItemForApply = {
	name: string
	slot: string
	markName: string
	markStyle: MarkStyle
	faceNormalChannel?: 'R' | 'G' | 'B' | 'A'
	suffix: string
}

type TrianglelistDedupedTextureProperty = {
	FALogDedupedFileName?: string
	FADataDedupedFileName?: string
	Format?: string
}

type TrianglelistDedupedFileNameJson = Record<string, TrianglelistDedupedTextureProperty>

type TextureMarkUpInfo = {
	MarkName: string
	MarkHash: string
	MarkSlot: string
	MarkType: MarkStyle
	MarkFileName: string
	MarkDedupedFileName?: string
	FaceSDFChannel?: 'R' | 'G' | 'B' | 'A'
}

const buildTextureIdentityKey = (dedupedFileName: string, markHash: string, markSlot: string): string => {
	const normalizedDedupedFileName = dedupedFileName.trim().toLowerCase()
	if (normalizedDedupedFileName) {
		return `deduped::${normalizedDedupedFileName}`
	}

	const normalizedHash = markHash.trim().toLowerCase()
	if (normalizedHash) {
		return `hash::${normalizedHash}`
	}

	const normalizedSlot = markSlot.trim().toLowerCase()
	if (normalizedSlot) {
		return `slot::${normalizedSlot}`
	}

	return ''
}

type SubMeshJsonObject = {
	GamePreset?: string
	WorkGameType?: string
	IndexBufferList?: unknown
	CategoryBufferList?: unknown
	TextureMarkUpInfoList?: unknown
	DiffuseMap?: string[]
	SubMeshRole?: SubMeshRole | null
	[key: string]: unknown
}

type SubMeshTargetEntry = {
	folderPath: string
	jsonPath: string
}

const SUBMESH_TEXTURE_MARKUP_LIST_KEY = 'TextureMarkUpInfoList'
const SUBMESH_DIFFUSE_MAP_LIST_KEY = 'DiffuseMap'
const SUBMESH_ROLE_KEY = 'SubMeshRole'

export type AppliedSubMeshTextureMark = {
	markName: string
	markHash: string
	markSlot: string
	markStyle: MarkStyle
	markFileName: string
	markDedupedFileName: string
	faceNormalChannel: 'R' | 'G' | 'B' | 'A'
	folderPath: string
	textureFilePath: string
	jsonPath: string
}

type TextureItemForAutoApply = TextureItemForApply & {
	psHash: string
	render: boolean
}

export type AutoApplyTextureMarksForTabResult = {
	scannedSubMeshCount: number
	appliedSubMeshCount: number
	matchedDrawCallCount: number
	appliedTextureCount: number
	targetFolderCount: number
}

export type ExportHashStyleTextureModTemplateResult = {
	exportedTextureCount: number
	skippedSlotStyleCount: number
	generatedFolderPath: string
	generatedIniPath: string
	drawIB: string
}

export type ExportSlotStyleTextureModTemplateResult = {
	exportedTextureCount: number
	skippedHashStyleCount: number
	generatedFolderPath: string
	generatedIniPath: string
	subMesh: string
}

export const getMarkTextureComponentJsonPath = async (
	workspacePath: string
): Promise<string> => {
	return join(workspacePath, 'ComponentName_DrawCallIndexList.json')
}

export const getMarkTextureTrianglelistJsonPath = async (
	workspacePath: string
): Promise<string> => {
	return join(workspacePath, 'TrianglelistDedupedFileName.json')
}

export const getMarkTextureDrawIBComponentJsonPath = async (
	workspacePath: string
): Promise<string> => {
	return join(workspacePath, 'DrawIB-Component.json')
}

const readTrianglelistDedupedDict = async (
	workspacePath: string
): Promise<TrianglelistDedupedFileNameJson> => {
	const trianglelistJsonPath = await getMarkTextureTrianglelistJsonPath(workspacePath)
	if (!(await exists(trianglelistJsonPath))) {
		return {}
	}
	const content = await readTextFile(trianglelistJsonPath)
	return JSON.parse(content) as TrianglelistDedupedFileNameJson
}

const isSubMeshJsonObject = (value: unknown): value is SubMeshJsonObject => {
	if (!value || typeof value !== 'object') {
		return false
	}

	const parsed = value as Record<string, unknown>
	return (
		typeof parsed.GamePreset === 'string' &&
		typeof parsed.WorkGameType === 'string' &&
		Array.isArray(parsed.IndexBufferList) &&
		Array.isArray(parsed.CategoryBufferList)
	)
}

const readSubMeshJsonFile = async (jsonPath: string): Promise<SubMeshJsonObject | undefined> => {
	if (!(await exists(jsonPath))) {
		return undefined
	}

	try {
		const content = await readTextFile(jsonPath)
		const parsed = JSON.parse(content) as unknown
		if (!isSubMeshJsonObject(parsed)) {
			return undefined
		}
		return parsed
	} catch {
		return undefined
	}
}

const getSubMeshTargetEntries = async (
	workspacePath: string,
	subMesh: string,
	strict = false
): Promise<SubMeshTargetEntry[]> => {
	const subMeshFolder = await join(workspacePath, subMesh)
	if (!(await exists(subMeshFolder))) {
		return []
	}

	const entries = await readDir(subMeshFolder)
	const targetEntries: SubMeshTargetEntry[] = []

	for (const entry of entries) {
		if (!entry.isDirectory || !entry.name) {
			continue
		}

		const folderPath = await join(subMeshFolder, entry.name)
		const folderEntries = await readDir(folderPath)

		for (const folderEntry of folderEntries) {
			if (folderEntry.isDirectory || !folderEntry.name || !folderEntry.name.endsWith('.json')) {
				continue
			}

			const jsonPath = await join(folderPath, folderEntry.name)
			const parsed = await readSubMeshJsonFile(jsonPath)
			if (!parsed) {
				if (strict && folderEntry.name === `${subMesh}.json`) throw new Error(`无效的 SubMesh 元数据: ${jsonPath}`)
				continue
			}

			targetEntries.push({
				folderPath,
				jsonPath,
			})
			break
		}
	}

	return targetEntries
}

const copyTextureToTargets = async (
	sourcePath: string,
	targetFileName: string,
	targetFolders: string[]
): Promise<void> => {
	for (const targetFolder of targetFolders) {
		const targetPath = await join(targetFolder, targetFileName)
		if (await exists(targetPath)) {
			await remove(targetPath)
		}
		await copyFile(sourcePath, targetPath)
	}
}

const copyTextureToFolder = async (
	sourcePath: string,
	targetFolder: string,
	targetFileName: string
): Promise<void> => {
	const targetPath = await join(targetFolder, targetFileName)
	if (await exists(targetPath)) {
		await remove(targetPath)
	}
	await copyFile(sourcePath, targetPath)
}

const syncDiffuseMapMetadata = (
	parsed: SubMeshJsonObject,
	markupList: TextureMarkUpInfo[]
): void => {
	// Entries are relative to this JSON file's parent folder, so Blender can
	// resolve them without knowing the workspace root.
	const diffuseMapFiles = markupList
		.filter(item => item.MarkName.trim().toLowerCase() === 'diffusemap')
		.map(item => item.MarkFileName.trim())
		.filter((fileName, index, files) => !!fileName && files.indexOf(fileName) === index)
	if (diffuseMapFiles.length > 0) {
		parsed[SUBMESH_DIFFUSE_MAP_LIST_KEY] = diffuseMapFiles
	} else {
		delete parsed[SUBMESH_DIFFUSE_MAP_LIST_KEY]
	}
}

const writeMarkupToSubMeshJson = async (
	targetJsonPath: string,
	markupList: TextureMarkUpInfo[]
): Promise<void> => {
	const parsed = await readSubMeshJsonFile(targetJsonPath)
	if (!parsed) {
		return
	}

	parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY] = markupList
	syncDiffuseMapMetadata(parsed, markupList)
	await writeTextFile(targetJsonPath, JSON.stringify(parsed, null, 2))
}

/**
 * Persists the semantic role selected in the post-process page beside the
 * extracted SubMesh metadata. Blender importers can consume this directly,
 * without depending on SSMT's page-local MarkTextureConfig.json.
 */
export type SubMeshRoleChange = {
	workspacePath: string
	subMesh: string
	role?: SubMeshRole
}

export const writeSubMeshRoleMetadata = async (args: SubMeshRoleChange): Promise<{ updatedFileCount: number }> =>
	writeSubMeshRoleMetadataBatch([args])

/** 先准备完整变更；任何类型目录写入失败时恢复已尝试写入的文件。 */
export const writeSubMeshRoleMetadataBatch = async (changes: SubMeshRoleChange[]): Promise<{ updatedFileCount: number }> => {
	const writes: { path: string; before: string; after: string }[] = []
	let updatedFileCount = 0
	for (const { workspacePath, subMesh, role } of changes) {
		const entries = await getSubMeshTargetEntries(workspacePath, subMesh, true)
		if (entries.length === 0) throw new Error(`没有可写的 SubMesh 角色元数据: ${subMesh}`)
		for (const entry of entries) {
			const before = await readTextFile(entry.jsonPath)
			const parsed = JSON.parse(before) as SubMeshJsonObject
			if (!isSubMeshJsonObject(parsed)) throw new Error(`无效的 SubMesh 元数据: ${entry.jsonPath}`)
			updatedFileCount += 1
			if (parsed[SUBMESH_ROLE_KEY] === (role ?? null)) continue
			// null 表示用户明确清除，防止旧页面记忆将它重新补回。
			parsed[SUBMESH_ROLE_KEY] = role ?? null
			writes.push({ path: entry.jsonPath, before, after: JSON.stringify(parsed, null, 2) })
		}
	}
	const attempted: typeof writes = []
	try {
		for (const write of writes) {
			attempted.push(write)
			await writeTextFile(write.path, write.after)
		}
	} catch (error) {
		const rollbackFailures: string[] = []
		for (const write of attempted.reverse()) {
			try { await writeTextFile(write.path, write.before) }
			catch { rollbackFailures.push(write.path) }
		}
		throw new Error(`写入 SubMesh 角色失败: ${String(error)}${rollbackFailures.length ? `；恢复失败: ${rollbackFailures.join(', ')}` : ''}`)
	}
	return { updatedFileCount }
}

/** 模型 JSON 优先；旧页面记忆仅补齐缺失的角色，并同步全部类型目录。 */
export const restoreSubMeshRoleMetadata = async (args: {
	workspacePath: string
	subMesh: string
	rememberedRole?: SubMeshRole
}): Promise<SubMeshRole | undefined> => {
	const entries = await getSubMeshTargetEntries(args.workspacePath, args.subMesh, true)
	if (entries.length === 0) throw new Error(`没有可读的 SubMesh 角色元数据: ${args.subMesh}`)
	const roles = new Set<SubMeshRole | undefined>()
	for (const entry of entries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		if (!parsed) throw new Error(`无法读取 SubMesh 角色元数据: ${entry.jsonPath}`)
		const role = parsed[SUBMESH_ROLE_KEY]
		if (role === 'Face' || role === 'Neck' || role === 'Eye') roles.add(role)
		else if (role === null) roles.add(undefined)
		else if (role !== undefined) throw new Error(`无效的 SubMesh 角色: ${entry.jsonPath}`)
	}
	if (roles.size > 1) throw new Error(`SubMesh 不同类型目录的角色标记冲突: ${args.subMesh}`)
	if (roles.size === 0 && args.rememberedRole === undefined) return undefined
	const role = roles.size ? roles.values().next().value : args.rememberedRole
	await writeSubMeshRoleMetadata({ ...args, role })
	return role
}

const normalizeMarkupList = (value: unknown): TextureMarkUpInfo[] => {
	if (!Array.isArray(value)) {
		return []
	}

	return value
		.filter((item): item is Record<string, unknown> => !!item && typeof item === 'object')
		.map(item => ({
			MarkName: typeof item.MarkName === 'string' ? item.MarkName : '',
			MarkHash: typeof item.MarkHash === 'string' ? item.MarkHash : '',
			MarkSlot: typeof item.MarkSlot === 'string' ? item.MarkSlot : '',
			MarkType: item.MarkType === 'Slot' ? 'Slot' : item.MarkType === 'SharedSlot' ? 'SharedSlot' : 'Hash',
			MarkFileName: typeof item.MarkFileName === 'string' ? item.MarkFileName : '',
			MarkDedupedFileName: typeof item.MarkDedupedFileName === 'string' ? item.MarkDedupedFileName : '',
			FaceSDFChannel: typeof item.MarkName === 'string' && item.MarkName.trim().toLowerCase() === 'facesdfmap'
				? item.FaceSDFChannel === 'G' || item.FaceSDFChannel === 'B' || item.FaceSDFChannel === 'A'
					? item.FaceSDFChannel
					: 'R'
				: undefined,
		}))
}

const dedupeMarkupListByTextureIdentity = (markupList: TextureMarkUpInfo[]): TextureMarkUpInfo[] => {
	const markupMap = new Map<string, TextureMarkUpInfo>()

	for (const item of markupList) {
		if (!item.MarkName.trim()) {
			continue
		}

		const identityKey = buildTextureIdentityKey(
			item.MarkDedupedFileName || '',
			item.MarkHash,
			item.MarkSlot
		)
		if (!identityKey) {
			continue
		}

		markupMap.set(identityKey, item)
	}

	return Array.from(markupMap.values())
}

export const readAppliedSubMeshTextureMarks = async (args: {
	workspacePath: string
	subMesh: string
}): Promise<AppliedSubMeshTextureMark[]> => {
	const { workspacePath, subMesh } = args
	if (!workspacePath || !subMesh) {
		return []
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	const result: AppliedSubMeshTextureMark[] = []
	const seenKeys = new Set<string>()

	for (const entry of targetEntries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		if (!parsed) {
			continue
		}

		const normalizedList = dedupeMarkupListByTextureIdentity(
			normalizeMarkupList(parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY])
		)
		const copiedFileNameCounts = new Map<string, number>()
		for (const item of normalizedList) {
			const key = item.MarkFileName.trim().toLowerCase()
			if (key) copiedFileNameCounts.set(key, (copiedFileNameCounts.get(key) || 0) + 1)
		}
		for (const item of normalizedList) {
			if (!item.MarkName.trim() || !item.MarkFileName.trim()) {
				continue
			}

			const copiedTextureFilePath = await join(entry.folderPath, item.MarkFileName)
			// Legacy workspaces gave all same-semantic maps one destination name,
			// so each subsequent copy overwrote the preceding texture. Their
			// per-texture source remains in DedupedTextures.
			const copiedFileIsShared = (copiedFileNameCounts.get(item.MarkFileName.trim().toLowerCase()) || 0) > 1
			const dedupedTextureFilePath = copiedFileIsShared && item.MarkDedupedFileName
				? await join(workspacePath, 'DedupedTextures', item.MarkDedupedFileName)
				: ''
			const textureFilePath = dedupedTextureFilePath && await exists(dedupedTextureFilePath)
				? dedupedTextureFilePath
				: copiedTextureFilePath
			if (!(await exists(textureFilePath))) {
				continue
			}

			const markKey = [
				item.MarkName.trim().toLowerCase(),
				item.MarkType,
				item.MarkFileName.trim().toLowerCase(),
				item.MarkHash.trim().toLowerCase(),
				item.MarkSlot.trim().toLowerCase(),
			].join('::')
			if (seenKeys.has(markKey)) {
				continue
			}
			seenKeys.add(markKey)

			result.push({
				markName: item.MarkName,
				markHash: item.MarkHash,
				markSlot: item.MarkSlot,
				markStyle: item.MarkType,
				markFileName: item.MarkFileName,
				markDedupedFileName: item.MarkDedupedFileName || '',
				faceNormalChannel: item.FaceSDFChannel || 'R',
				folderPath: entry.folderPath,
				textureFilePath,
				jsonPath: entry.jsonPath,
			})
		}
	}

	return result
}

type DrawIBComponentJson = Record<string, Record<string, string>>

export const readPersistedSubMeshDrawCallIndexes = async (args: {
	workspacePath: string
	subMesh: string
}): Promise<string[]> => {
	const { workspacePath, subMesh } = args
	if (!workspacePath || !subMesh) {
		return []
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	const drawCalls = new Set<string>()
	for (const entry of targetEntries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		const values = parsed?.DrawCallIndexList
		if (!Array.isArray(values)) {
			continue
		}
		for (const value of values) {
			if (typeof value === 'string' && value.trim()) {
				drawCalls.add(value.trim())
			}
		}
	}

	return [...drawCalls].sort((left, right) => left.localeCompare(right, undefined, { numeric: true }))
}

const getPixelSlotFromTextureFileName = (textureFileName: string): string => {
	const startPos = textureFileName.indexOf('-')
	const endPos = textureFileName.indexOf('=')
	if (startPos < 0 || endPos < 0 || endPos <= startPos + 1) {
		return ''
	}

	let pixelSlot = textureFileName.slice(startPos + 1, endPos)
	if (pixelSlot.includes('-vs')) {
		const lastDash = pixelSlot.lastIndexOf('-')
		if (lastDash > 0) {
			pixelSlot = pixelSlot.slice(0, lastDash)
		}
	}

	return pixelSlot
}

const getDrawCallIndexFromTextureFileName = (textureFileName: string): string => {
	const match = /^(\d{6})-ps-t/i.exec(textureFileName.trim())
	return match?.[1] || ''
}

/**
 * Compatibility path for workspaces produced before per-submesh draw-call
 * metadata existed.  A stored mark carries the original deduped texture name;
 * resolve that identity back through TrianglelistDedupedFileName.json instead
 * of borrowing another component's draw-call list.
 */
export const readDrawCallIndexesMatchedByPersistedSubMeshMarks = async (args: {
	workspacePath: string
	subMesh: string
	trianglelistDedupedDict: TrianglelistDedupedFileNameJson
}): Promise<string[]> => {
	const { workspacePath, subMesh, trianglelistDedupedDict } = args
	if (!workspacePath || !subMesh) {
		return []
	}

	const marks = await readAppliedSubMeshTextureMarks({ workspacePath, subMesh })
	if (marks.length === 0) {
		return []
	}

	const drawCalls = new Set<string>()
	for (const mark of marks) {
		const markDedupedFileName = mark.markDedupedFileName.trim().toLowerCase()
		const markHash = mark.markHash.trim().toLowerCase()
		const markSlot = mark.markSlot.trim().toLowerCase()

		for (const [textureFileName, textureProperty] of Object.entries(trianglelistDedupedDict)) {
			const dedupedFileName = (textureProperty?.FALogDedupedFileName || '').trim().toLowerCase()
			const textureHash = SSMTStringUtils.getFileHashFromFileName(textureFileName).trim().toLowerCase()
			const textureSlot = getPixelSlotFromTextureFileName(textureFileName).trim().toLowerCase()
			const matchedByDedupedFileName = !!markDedupedFileName && dedupedFileName === markDedupedFileName
			const matchedByHashAndSlot = !!markHash && textureHash === markHash && (!markSlot || textureSlot === markSlot)
			if (!matchedByDedupedFileName && !matchedByHashAndSlot) {
				continue
			}

			const drawCall = getDrawCallIndexFromTextureFileName(textureFileName)
			if (drawCall) {
				drawCalls.add(drawCall)
			}
		}
	}

	return [...drawCalls].sort((left, right) => left.localeCompare(right, undefined, { numeric: true }))
}

const getTextureSuffixFromFileName = (textureFileName: string): string => {
	const equalIndex = textureFileName.lastIndexOf('=')
	const lastDot = textureFileName.lastIndexOf('.')
	if (lastDot < 0) {
		return ''
	}
	if (equalIndex >= 0 && lastDot < equalIndex) {
		return ''
	}
	return textureFileName.slice(lastDot)
}

const readComponentDrawCallMap = async (
	workspacePath: string
): Promise<Record<string, string[]>> => {
	const componentJsonPath = await getMarkTextureComponentJsonPath(workspacePath)
	if (!(await exists(componentJsonPath))) {
		return {}
	}
	const content = await readTextFile(componentJsonPath)
	const parsed = JSON.parse(content) as Record<string, unknown>
	const result: Record<string, string[]> = {}

	for (const [subMeshName, drawCalls] of Object.entries(parsed)) {
		if (!subMeshName.trim() || !Array.isArray(drawCalls)) {
			continue
		}

		result[subMeshName] = drawCalls
			.filter((item): item is string => typeof item === 'string')
			.map(item => item.trim())
			.filter(item => item.length > 0)
	}

	return result
}

const readMappedSubMeshNames = async (workspacePath: string): Promise<string[]> => {
	const drawIBComponentJsonPath = await getMarkTextureDrawIBComponentJsonPath(workspacePath)
	if (!(await exists(drawIBComponentJsonPath))) {
		return []
	}

	try {
		const drawIBComponentContent = await readTextFile(drawIBComponentJsonPath)
		const parsed = JSON.parse(drawIBComponentContent) as DrawIBComponentJson
		const subMeshes: string[] = []

		for (const componentMap of Object.values(parsed)) {
			if (!componentMap || typeof componentMap !== 'object') {
				continue
			}
			for (const subMesh of Object.values(componentMap)) {
				if (typeof subMesh === 'string' && subMesh.trim()) {
					subMeshes.push(subMesh.trim())
				}
			}
		}

		return [...new Set(subMeshes)].sort((left, right) => left.localeCompare(right, undefined, { numeric: true }))
	} catch {
		return []
	}
}

const buildTextureListForDrawCall = (
	drawCall: string,
	trianglelistDedupedDict: TrianglelistDedupedFileNameJson,
	defaultMarkStyle: MarkStyle
): TextureItemForAutoApply[] => {
	const matchedEntries = Object.entries(trianglelistDedupedDict)
		.filter(([textureFileName]) => textureFileName.startsWith(drawCall))
		.sort((a, b) => a[0].localeCompare(b[0]))

	return matchedEntries.map(([textureFileName, textureProperty]) => ({
		name: textureFileName,
		slot: getPixelSlotFromTextureFileName(textureFileName),
		psHash: SSMTStringUtils.getPSHashFromFileName(textureFileName),
		render: (textureProperty?.FADataDedupedFileName || '').trim().length > 0,
		suffix: getTextureSuffixFromFileName(textureFileName),
		markName: '',
		markStyle: defaultMarkStyle,
	}))
}

export const autoApplyTextureMarksFromHistoryForTab = async (args: {
	workspacePath: string
	tabId: string
	currentGameName: string
	defaultMarkStyle?: MarkStyle
}): Promise<AutoApplyTextureMarksForTabResult> => {
	const { workspacePath, tabId, currentGameName } = args
	const defaultMarkStyle: MarkStyle = args.defaultMarkStyle === 'Slot' ? 'Slot' : args.defaultMarkStyle === 'SharedSlot' ? 'SharedSlot' : 'Hash'
	const result: AutoApplyTextureMarksForTabResult = {
		scannedSubMeshCount: 0,
		appliedSubMeshCount: 0,
		matchedDrawCallCount: 0,
		appliedTextureCount: 0,
		targetFolderCount: 0,
	}

	if (!workspacePath || !tabId) {
		return result
	}

	const [legacyComponentDrawCallMap, trianglelistDedupedDict, mappedSubMeshNames] = await Promise.all([
		readComponentDrawCallMap(workspacePath),
		readTrianglelistDedupedDict(workspacePath),
		readMappedSubMeshNames(workspacePath),
	])

	const componentDrawCallMap: Record<string, string[]> = {}
	const subMeshNamesToResolve = mappedSubMeshNames.length > 0
		? mappedSubMeshNames
		: Object.keys(legacyComponentDrawCallMap)
	for (const subMesh of subMeshNamesToResolve) {
		const persistedDrawCalls = await readPersistedSubMeshDrawCallIndexes({ workspacePath, subMesh })
		const directLegacyDrawCalls = legacyComponentDrawCallMap[subMesh] ?? []
		const drawCallsFromPersistedMarks = persistedDrawCalls.length === 0 && directLegacyDrawCalls.length === 0
			? await readDrawCallIndexesMatchedByPersistedSubMeshMarks({
				workspacePath,
				subMesh,
				trianglelistDedupedDict,
			})
			: []
		componentDrawCallMap[subMesh] = persistedDrawCalls.length > 0
			? persistedDrawCalls
			: directLegacyDrawCalls.length > 0
				? directLegacyDrawCalls
				: drawCallsFromPersistedMarks
	}

	const subMeshNames = Object.keys(componentDrawCallMap).sort((a, b) => a.localeCompare(b))

	for (const subMesh of subMeshNames) {
		result.scannedSubMeshCount += 1
		const drawCalls = componentDrawCallMap[subMesh] ?? []

		for (const drawCall of drawCalls) {
			const textureList = buildTextureListForDrawCall(drawCall, trianglelistDedupedDict, defaultMarkStyle)
			if (textureList.length === 0) {
				continue
			}

			const psHash = textureList[0]?.psHash?.trim() || ''
			if (!psHash) {
				continue
			}

			const memoryConfig = await loadTextureMemoryByPSHash(currentGameName, psHash)
			if (!memoryConfig) {
				continue
			}

			const textureListWithMemory = applyTextureMemoryToItems(textureList, memoryConfig)
			if (!textureListWithMemory.some(item => item.markName.trim())) {
				continue
			}

			const applyResult = await applyTextureMarkForCurrentSubMesh({
				workspacePath,
				tabId,
				subMesh,
				textureList: textureListWithMemory,
			})

			if (applyResult.appliedCount <= 0) {
				continue
			}

			result.matchedDrawCallCount += 1
			result.appliedSubMeshCount += 1
			result.appliedTextureCount += applyResult.appliedCount
			result.targetFolderCount += applyResult.targetFolderCount
			break
		}
	}

	return result
}

export const applyTextureMarkForCurrentSubMesh = async (args: {
	workspacePath: string
	tabId: string
	subMesh: string
	textureList: TextureItemForApply[]
}): Promise<{ appliedCount: number; targetFolderCount: number }> => {
	const { workspacePath, tabId, subMesh, textureList } = args
	if (!workspacePath || !tabId || !subMesh) {
		return { appliedCount: 0, targetFolderCount: 0 }
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	if (targetEntries.length === 0) {
		return { appliedCount: 0, targetFolderCount: 0 }
	}

	const dedupedDict = await readTrianglelistDedupedDict(workspacePath)
	const markupMap = new Map<string, { markup: TextureMarkUpInfo; sourcePath: string; dedupedFileName: string }>()
	const markNameCounts = new Map<string, number>()
	for (const item of textureList) {
		const markName = item.markName?.trim().toLowerCase()
		if (markName) markNameCounts.set(markName, (markNameCounts.get(markName) || 0) + 1)
	}
	let appliedCount = 0

	for (const item of textureList) {
		if (!item.markName?.trim()) {
			continue
		}

		const deduped = dedupedDict[item.name]?.FALogDedupedFileName?.trim() || ''
		if (!deduped) {
			continue
		}

		const sourcePath = await join(workspacePath, 'DedupedTextures', deduped)
		if (!(await exists(sourcePath))) {
			continue
		}

		const textureHash = SSMTStringUtils.getFileHashFromFileName(item.name)
		const normalizedMarkName = item.markName.trim().toLowerCase()
		const needsUniqueFileName = (markNameCounts.get(normalizedMarkName) || 0) > 1
		const targetFileName = needsUniqueFileName
			? `${subMesh}-${item.markName}-${textureHash || item.slot.replace(/[^a-z0-9]+/gi, '_')}${item.suffix || '.dds'}`
			: `${subMesh}-${item.markName}${item.suffix || '.dds'}`
		const identityKey = buildTextureIdentityKey(deduped, textureHash, item.slot)
		if (!identityKey) {
			continue
		}

		markupMap.set(identityKey, {
			markup: {
				MarkName: item.markName,
				MarkHash: textureHash,
				MarkSlot: item.slot,
				MarkType: item.markStyle,
				MarkFileName: targetFileName,
				MarkDedupedFileName: deduped,
				FaceSDFChannel: item.markName.trim().toLowerCase() === 'facesdfmap'
					? item.faceNormalChannel || 'R'
					: undefined,
			},
			sourcePath,
			dedupedFileName: deduped,
		})
	}

	const markupEntries = Array.from(markupMap.values())
	const markupList = markupEntries.map(item => item.markup)
	const targetFolders = targetEntries.map(item => item.folderPath)

	for (const { markup, sourcePath } of markupEntries) {
		await copyTextureToTargets(sourcePath, markup.MarkFileName, targetFolders)

		appliedCount += 1
	}

	for (const entry of targetEntries) {
		await writeMarkupToSubMeshJson(entry.jsonPath, markupList)
	}

	return {
		appliedCount,
		targetFolderCount: targetEntries.length,
	}
}

export const clearCurrentSubMeshTextureMarkup = async (args: {
	workspacePath: string
	subMesh: string
}): Promise<{ clearedFolderCount: number }> => {
	const { workspacePath, subMesh } = args
	if (!workspacePath || !subMesh) {
		return { clearedFolderCount: 0 }
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	let clearedFolderCount = 0

	for (const entry of targetEntries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		if (!parsed) {
			continue
		}

		parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY] = []
		syncDiffuseMapMetadata(parsed, [])
		await writeTextFile(entry.jsonPath, JSON.stringify(parsed, null, 2))
		clearedFolderCount += 1
	}

	return { clearedFolderCount }
}

export const hasExistingSubMeshTextureMarks = async (
	workspacePath: string,
	subMesh: string
): Promise<boolean> => {
	if (!workspacePath || !subMesh) {
		return false
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	for (const entry of targetEntries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		if (!parsed) {
			continue
		}

		const normalizedList = normalizeMarkupList(parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY])
		if (normalizedList.length > 0) {
			return true
		}
	}

	return false
}

export const updateCurrentSubMeshTextureMarkupStyle = async (args: {
	workspacePath: string
	subMesh: string
	markStyle: MarkStyle
}): Promise<{ updatedFolderCount: number; updatedMarkupCount: number }> => {
	const { workspacePath, subMesh, markStyle } = args
	if (!workspacePath || !subMesh) {
		return { updatedFolderCount: 0, updatedMarkupCount: 0 }
	}

	const targetEntries = await getSubMeshTargetEntries(workspacePath, subMesh)
	let updatedFolderCount = 0
	let updatedMarkupCount = 0

	for (const entry of targetEntries) {
		const parsed = await readSubMeshJsonFile(entry.jsonPath)
		if (!parsed) {
			continue
		}

		const normalizedList = normalizeMarkupList(parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY])
		let fileChanged = false

		if (normalizedList.length === 0) {
			continue
		}

		const nextList = normalizedList.map(item => {
			const nextItem: TextureMarkUpInfo = {
				...item,
				MarkType: markStyle,
			}
			const changed = item.MarkType !== markStyle
			if (changed) {
				updatedMarkupCount += 1
				fileChanged = true
			}

			return nextItem
		})

		if (!fileChanged) {
			continue
		}

		parsed[SUBMESH_TEXTURE_MARKUP_LIST_KEY] = nextList
		syncDiffuseMapMetadata(parsed, nextList)
		await writeTextFile(entry.jsonPath, JSON.stringify(parsed, null, 2))
		updatedFolderCount += 1
	}

	return { updatedFolderCount, updatedMarkupCount }
}

export const exportHashStyleTextureModTemplateForCurrentSelection = async (args: {
	workspacePath: string
	generatedModFolderPath: string
	tabId: string
	drawIB: string
	textureList: TextureItemForApply[]
	currentGameName?: string
}): Promise<ExportHashStyleTextureModTemplateResult> => {
	const {
		workspacePath,
		generatedModFolderPath,
		tabId,
		drawIB,
		textureList,
		currentGameName,
	} = args
	const result: ExportHashStyleTextureModTemplateResult = {
		exportedTextureCount: 0,
		skippedSlotStyleCount: 0,
		generatedFolderPath: generatedModFolderPath,
		generatedIniPath: '',
		drawIB,
	}

	if (!workspacePath || !generatedModFolderPath || !tabId || !drawIB) {
		return result
	}

	const dedupedDict = await readTrianglelistDedupedDict(workspacePath)
	const textureModIniLines: string[] = []
	const writtenTextureHashSet = new Set<string>()
	const writtenPixelSlotSet = new Set<string>()
	const normalizedGameName = (currentGameName || '').trim().toUpperCase()
	const isZZZLikeGame = normalizedGameName === 'ZZMI' || normalizedGameName === 'ZZMIDX12'

	textureModIniLines.push('[TextureOverride_IB_SlotCheck]')
	textureModIniLines.push(`hash = ${drawIB}`)
	textureModIniLines.push('match_priority = 0')

	if (isZZZLikeGame) {
		textureModIniLines.push('run = CommandListSkinTexture')
		textureModIniLines.push('')
	} else {
		for (const item of textureList) {
			if (!item.markName?.trim()) {
				continue
			}

			if (item.markStyle !== 'Hash') {
				result.skippedSlotStyleCount += 1
				continue
			}

			const pixelSlot = item.slot.trim()
			if (!pixelSlot || writtenPixelSlotSet.has(pixelSlot)) {
				continue
			}

			writtenPixelSlotSet.add(pixelSlot)
			textureModIniLines.push(`checktextureoverride = ${pixelSlot}`)
		}
		textureModIniLines.push('')
	}

	for (const item of textureList) {
		if (!item.markName?.trim()) {
			continue
		}

		if (item.markStyle !== 'Hash') {
			continue
		}

		const deduped = dedupedDict[item.name]?.FALogDedupedFileName?.trim() || ''
		if (!deduped) {
			continue
		}

		const sourcePath = await join(workspacePath, 'DedupedTextures', deduped)
		if (!(await exists(sourcePath))) {
			continue
		}

		const textureHash = SSMTStringUtils.getFileHashFromFileName(item.name)
		if (!textureHash || writtenTextureHashSet.has(textureHash)) {
			continue
		}

		writtenTextureHashSet.add(textureHash)

		const targetImageFileName = `${item.markName.trim()}_${deduped}`
		textureModIniLines.push(`[TextureOverride_Texture_${textureHash}]`)
		textureModIniLines.push(`hash = ${textureHash}`)
		textureModIniLines.push(`this = ResourceTexture_${textureHash}`)
		textureModIniLines.push('')
		textureModIniLines.push(`[ResourceTexture_${textureHash}]`)
		textureModIniLines.push(`filename = ${targetImageFileName}`)
		textureModIniLines.push('')

		await copyTextureToFolder(sourcePath, generatedModFolderPath, targetImageFileName)
		result.exportedTextureCount += 1
	}

	if (result.exportedTextureCount <= 0) {
		return result
	}

	const textureModIniFileName = `${drawIB}_TextureMod.ini`
	const generatedIniPath = await join(generatedModFolderPath, textureModIniFileName)
	await writeTextFile(generatedIniPath, textureModIniLines.join('\n'))
	result.generatedIniPath = generatedIniPath

	return result
}

const parseSlotTemplateSubMeshInfo = (
	subMesh: string
): { hash: string; matchIndexCount: string; matchFirstIndex: string } | undefined => {
	const parts = subMesh
		.split('-')
		.map(part => part.trim())
		.filter(part => part.length > 0)

	if (parts.length < 3) {
		return undefined
	}

	return {
		hash: parts[0],
		matchIndexCount: parts[1],
		matchFirstIndex: parts[2],
	}
}

export const exportSlotStyleTextureModTemplateForCurrentSelection = async (args: {
	workspacePath: string
	generatedModFolderPath: string
	tabId: string
	subMesh: string
	textureList: TextureItemForApply[]
	gamePreset?: string
}): Promise<ExportSlotStyleTextureModTemplateResult> => {
	const {
		workspacePath,
		generatedModFolderPath,
		tabId,
		subMesh,
		textureList,
		gamePreset,
	} = args
	const result: ExportSlotStyleTextureModTemplateResult = {
		exportedTextureCount: 0,
		skippedHashStyleCount: 0,
		generatedFolderPath: generatedModFolderPath,
		generatedIniPath: '',
		subMesh,
	}

	if (!workspacePath || !generatedModFolderPath || !tabId || !subMesh) {
		return result
	}

	const subMeshInfo = parseSlotTemplateSubMeshInfo(subMesh)
	if (!subMeshInfo) {
		return result
	}

	const dedupedDict = await readTrianglelistDedupedDict(workspacePath)
	const overrideLines: string[] = []
	const resourceLines: string[] = []
	const writtenPixelSlotSet = new Set<string>()
	const normalizedPreset = (gamePreset || '').trim().toUpperCase()

	overrideLines.push(';MARK:TextureOverrideIB')
	overrideLines.push(
		`[TextureOverride_${subMeshInfo.hash}_${subMeshInfo.matchIndexCount}_${subMeshInfo.matchFirstIndex}]`
	)
	overrideLines.push(`hash = ${subMeshInfo.hash}`)
	overrideLines.push(`match_first_index = ${subMeshInfo.matchFirstIndex}`)
	overrideLines.push(`match_index_count = ${subMeshInfo.matchIndexCount}`)
	if (normalizedPreset === 'EFMI') {
		overrideLines.push('run = CommandList\\EFMIv1\\OverrideTextures')
	}

	for (const item of textureList) {
		if (!item.markName?.trim()) {
			continue
		}

		if (item.markStyle !== 'Slot' && item.markStyle !== 'SharedSlot') {
			result.skippedHashStyleCount += 1
			continue
		}

		const pixelSlot = item.slot.trim()
		if (!pixelSlot || writtenPixelSlotSet.has(pixelSlot)) {
			continue
		}

		const deduped = dedupedDict[item.name]?.FALogDedupedFileName?.trim() || ''
		if (!deduped) {
			continue
		}

		const sourcePath = await join(workspacePath, 'DedupedTextures', deduped)
		if (!(await exists(sourcePath))) {
			continue
		}

		writtenPixelSlotSet.add(pixelSlot)

		const resourceName = `Resource-${subMeshInfo.hash}-${subMeshInfo.matchFirstIndex}-${item.markName.trim()}`
		const targetImageFileName = `${subMeshInfo.hash}-${subMeshInfo.matchIndexCount}-${subMeshInfo.matchFirstIndex}-${item.markName.trim()}${item.suffix || '.dds'}`

		overrideLines.push(`${pixelSlot} = ${resourceName}`)
		resourceLines.push(`[${resourceName}]`)
		resourceLines.push(`filename = ${targetImageFileName}`)
		resourceLines.push('')

		await copyTextureToFolder(sourcePath, generatedModFolderPath, targetImageFileName)
		result.exportedTextureCount += 1
	}

	if (result.exportedTextureCount <= 0) {
		return result
	}

	overrideLines.push('')
	overrideLines.push(';MARK:ResourceTexture')
	overrideLines.push(...resourceLines)

	const textureModIniFileName = `${subMesh}_SlotTextureMod.ini`
	const generatedIniPath = await join(generatedModFolderPath, textureModIniFileName)
	await writeTextFile(generatedIniPath, overrideLines.join('\n'))
	result.generatedIniPath = generatedIniPath

	return result
}
