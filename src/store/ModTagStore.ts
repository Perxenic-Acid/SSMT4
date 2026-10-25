import { copyFile, exists, mkdir, readDir, readTextFile, rename, writeTextFile } from '@tauri-apps/plugin-fs'
import { defineStore } from 'pinia'
import { moveFileToRecycleBin } from '../utils/RecycleBin'
import { ModManager } from './ModManager'

// ---------------------------------------------------------------------------
// Public interfaces
// ---------------------------------------------------------------------------

export interface ModTagDefinition {
  id: string
  name: string
  color: string
  iconFile?: string
  createdAt: number
  updatedAt: number
}

export interface ModTagSnapshot {
  version: number
  tags: ModTagDefinition[]
  modMappings: Record<string, string[]>
  groupMappings: Record<string, string[]>
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TAG_STORE_VERSION = 1
const TAG_DIR_NAME = '.mod-tags'
const TAG_ICONS_DIR = 'icons'
const TAGS_FILE_NAME = 'tags.json'
const TAG_MAPPINGS_FILE_NAME = 'mod-mappings.json'
const GROUP_MAPPINGS_FILE_NAME = 'group-mappings.json'
const TAG_COLOR_PALETTE = ['#EF6C57', '#8EE6FF', '#7DDCFF', '#5C9E6D', '#2A9D8F', '#4D7CFE', '#7B61FF', '#C05C7E']

// ---------------------------------------------------------------------------
// Standalone helpers (pure functions — no store state)
// ---------------------------------------------------------------------------

const normalizePath = (value: string) => value.replace(/\\/g, '/').replace(/\/+/g, '/').replace(/\/$/, '')

const joinPath = (...parts: string[]) => parts
  .map((part, index) => {
    const normalized = normalizePath(part)
    if (index === 0) {
      return normalized.replace(/\/+$/, '')
    }
    return normalized.replace(/^\/+|\/+$/g, '')
  })
  .filter(Boolean)
  .join('/')

const slugifyTagName = (value: string) => {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9一-龥]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug || 'tag'
}

const hashString = (value: string) => {
  let hash = 0
  for (let index = 0; index < value.length; index += 1) {
    hash = ((hash << 5) - hash) + value.charCodeAt(index)
    hash |= 0
  }
  return Math.abs(hash)
}

const normalizeColor = (value: unknown, fallbackSeed = '') => {
  if (typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value.trim())) {
    return value.trim()
  }
  return TAG_COLOR_PALETTE[hashString(fallbackSeed || 'tag') % TAG_COLOR_PALETTE.length]
}

const normalizeTagName = (value: unknown) => {
  if (typeof value !== 'string') {
    return ''
  }
  return value.trim().slice(0, 48)
}

const normalizeTagDefinition = (value: unknown): ModTagDefinition | null => {
  if (!value || typeof value !== 'object') {
    return null
  }

  const candidate = value as Partial<ModTagDefinition>
  const name = normalizeTagName(candidate.name)
  const id = typeof candidate.id === 'string' ? candidate.id.trim() : ''
  if (!id || !name) {
    return null
  }

  const createdAt = Number.isFinite(candidate.createdAt) ? Number(candidate.createdAt) : Date.now()
  const updatedAt = Number.isFinite(candidate.updatedAt) ? Number(candidate.updatedAt) : createdAt
  const iconFile = typeof candidate.iconFile === 'string' && candidate.iconFile.trim() ? candidate.iconFile.trim() : undefined

  return {
    id,
    name,
    color: normalizeColor(candidate.color, id),
    iconFile,
    createdAt,
    updatedAt,
  }
}

const normalizeTagMappings = (value: unknown, validTagIds: Set<string>) => {
  if (!value || typeof value !== 'object') {
    return {} as Record<string, string[]>
  }

  const normalized: Record<string, string[]> = {}
  for (const [modId, tags] of Object.entries(value as Record<string, unknown>)) {
    const cleanModId = normalizePath(String(modId || ''))
    if (!cleanModId || !Array.isArray(tags)) {
      continue
    }

    const cleanTags = Array.from(new Set(tags
      .map((tagId) => String(tagId || '').trim())
      .filter((tagId) => !!tagId && validTagIds.has(tagId))))

    if (cleanTags.length > 0) {
      normalized[cleanModId] = cleanTags
    }
  }

  return normalized
}

// ---------------------------------------------------------------------------
// Internal helpers (extracted from private static methods — no store state)
// ---------------------------------------------------------------------------

async function getModsRoot(gameName: string) {
  const installDir = await ModManager.getInstallDir(gameName)
  return joinPath(installDir, 'Mods')
}

async function getStoreDir(gameName: string) {
  const modsRoot = await getModsRoot(gameName)
  return joinPath(modsRoot, TAG_DIR_NAME)
}

async function getIconsDir(gameName: string) {
  const storeDir = await getStoreDir(gameName)
  return joinPath(storeDir, TAG_ICONS_DIR)
}

async function getTagsFilePath(gameName: string) {
  const storeDir = await getStoreDir(gameName)
  return joinPath(storeDir, TAGS_FILE_NAME)
}

async function getMappingsFilePath(gameName: string) {
  const storeDir = await getStoreDir(gameName)
  return joinPath(storeDir, TAG_MAPPINGS_FILE_NAME)
}

async function getGroupMappingsFilePath(gameName: string) {
  const storeDir = await getStoreDir(gameName)
  return joinPath(storeDir, GROUP_MAPPINGS_FILE_NAME)
}

async function ensureStoreDir(gameName: string) {
  const storeDir = await getStoreDir(gameName)
  if (!(await exists(storeDir))) {
    await mkdir(storeDir, { recursive: true })
  }
  const iconsDir = await getIconsDir(gameName)
  if (!(await exists(iconsDir))) {
    await mkdir(iconsDir, { recursive: true })
  }
}

async function readJsonFile<T>(path: string, fallback: T): Promise<T> {
  try {
    if (!(await exists(path))) {
      return fallback
    }
    const raw = await readTextFile(path)
    return JSON.parse(raw) as T
  } catch (error) {
    throw new Error(`读取标签数据失败 (${path}): ${String(error)}`)
  }
}

async function writeJsonFile(path: string, value: unknown) {
  // 先完整写入临时文件，避免写入中断破坏上一份有效 JSON。
  const temporaryPath = `${path}.tmp`
  await writeTextFile(temporaryPath, JSON.stringify(value, null, 2))
  await rename(temporaryPath, path)
}

async function removeIconFile(gameName: string, iconFile?: string) {
  if (!iconFile) return
  const iconsDir = await getIconsDir(gameName)
  const iconPath = joinPath(iconsDir, iconFile)
  if (await exists(iconPath)) {
    await moveFileToRecycleBin(iconPath)
  }
}

async function storeIconFile(gameName: string, tagId: string, sourcePath: string, previousIconFile?: string) {
  await ensureStoreDir(gameName)
  const sourceName = sourcePath.split(/[\\/]/).pop() || ''
  const extMatch = sourceName.match(/\.([a-zA-Z0-9]+)$/)
  const ext = extMatch ? extMatch[1].toLowerCase() : 'png'
  const iconFile = `${tagId}.${ext}`
  const iconsDir = await getIconsDir(gameName)
  const targetPath = joinPath(iconsDir, iconFile)

  if (previousIconFile && previousIconFile !== iconFile) {
    await removeIconFile(gameName, previousIconFile)
  }

  await copyFile(sourcePath, targetPath)
  return iconFile
}

function buildSnapshot(tags: ModTagDefinition[], modMappings: Record<string, string[]>, groupMappings?: Record<string, string[]>): ModTagSnapshot {
  const validTagIds = new Set(tags.map((tag) => tag.id))
  return {
    version: TAG_STORE_VERSION,
    tags,
    modMappings: normalizeTagMappings(modMappings, validTagIds),
    groupMappings: normalizeTagMappings(groupMappings || {}, validTagIds),
  }
}

// ---------------------------------------------------------------------------
// Pinia store (setup syntax / composition API)
// ---------------------------------------------------------------------------

export const useModTagStore = defineStore('modTags', () => {
  // 同一游戏的读取和完整修改事务排队；内部调用保持不排队，避免嵌套死锁。
  const pendingOperations = new Map<string, Promise<unknown>>()
  function serialize<Args extends unknown[], Result>(operation: (gameName: string, ...args: Args) => Promise<Result>) {
    return (gameName: string, ...args: Args): Promise<Result> => {
      const previous = pendingOperations.get(gameName) || Promise.resolve()
      const result = previous.catch(() => undefined).then(() => operation(gameName, ...args))
      const settled = result.then(() => undefined, () => undefined)
      pendingOperations.set(gameName, settled)
      void settled.then(() => {
        if (pendingOperations.get(gameName) === settled) pendingOperations.delete(gameName)
      })
      return result
    }
  }

  // ---- Public API (former public static methods) ----

  async function load(gameName: string): Promise<ModTagSnapshot> {
    const tagsFile = await getTagsFilePath(gameName)
    const mappingsFile = await getMappingsFilePath(gameName)
    const groupMappingsFile = await getGroupMappingsFilePath(gameName)
    const tagsRaw = await readJsonFile<unknown>(tagsFile, [])
    const mappingsRaw = await readJsonFile<unknown>(mappingsFile, {})
    const groupMappingsRaw = await readJsonFile<unknown>(groupMappingsFile, {})

    const isMapping = (value: unknown): value is Record<string, string[]> =>
      !!value && typeof value === 'object' && !Array.isArray(value)
      && Object.values(value).every((ids) => Array.isArray(ids) && ids.every((id) => typeof id === 'string'))
    if (!Array.isArray(tagsRaw) || tagsRaw.some((tag) => normalizeTagDefinition(tag) === null)
      || !isMapping(mappingsRaw) || !isMapping(groupMappingsRaw)) {
      throw new Error('标签数据格式无效，已停止加载和保存，请检查 .mod-tags 中的数据文件')
    }

    const tags = Array.isArray(tagsRaw)
      ? tagsRaw.map(normalizeTagDefinition).filter((tag): tag is ModTagDefinition => tag !== null)
      : []

    const dedupedTags = Array.from(new Map(tags.map((tag) => [tag.id, tag])).values())
      .sort((left, right) => left.name.localeCompare(right.name, undefined, { sensitivity: 'base' }))

    return buildSnapshot(dedupedTags, mappingsRaw as Record<string, string[]>, groupMappingsRaw as Record<string, string[]>)
  }

  async function save(gameName: string, snapshot: ModTagSnapshot) {
    await ensureStoreDir(gameName)
    const normalized = buildSnapshot(snapshot.tags, snapshot.modMappings, snapshot.groupMappings)
    const tagsFile = await getTagsFilePath(gameName)
    const mappingsFile = await getMappingsFilePath(gameName)
    const groupMappingsFile = await getGroupMappingsFilePath(gameName)
    await writeJsonFile(tagsFile, normalized.tags)
    await writeJsonFile(mappingsFile, normalized.modMappings)
    await writeJsonFile(groupMappingsFile, normalized.groupMappings)
    return normalized
  }

  async function upsertTag(
    gameName: string,
    input: { id?: string; name: string; color?: string },
    options?: { iconSourcePath?: string; removeIcon?: boolean },
  ): Promise<ModTagDefinition> {
    const snapshot = await load(gameName)
    const normalizedName = normalizeTagName(input.name)
    if (!normalizedName) {
      throw new Error('Tag name is required')
    }

    const duplicate = snapshot.tags.find((tag) => tag.name.toLowerCase() === normalizedName.toLowerCase() && tag.id !== input.id)
    if (duplicate) {
      throw new Error('Tag name already exists')
    }

    const now = Date.now()
    const existing = input.id ? snapshot.tags.find((tag) => tag.id === input.id) : undefined
    const nextId = existing?.id || `${slugifyTagName(normalizedName)}-${now.toString(36)}`
    let nextIconFile = existing?.iconFile

    if (options?.removeIcon) {
      await removeIconFile(gameName, nextIconFile)
      nextIconFile = undefined
    }

    if (options?.iconSourcePath) {
      nextIconFile = await storeIconFile(gameName, nextId, options.iconSourcePath, existing?.iconFile)
    }

    const nextTag: ModTagDefinition = {
      id: nextId,
      name: normalizedName,
      color: normalizeColor(input.color, nextId),
      iconFile: nextIconFile,
      createdAt: existing?.createdAt || now,
      updatedAt: now,
    }

    const nextTags = existing
      ? snapshot.tags.map((tag) => tag.id === existing.id ? nextTag : tag)
      : [...snapshot.tags, nextTag]

    await save(gameName, {
      ...snapshot,
      tags: nextTags,
    })

    return nextTag
  }

  async function deleteTag(gameName: string, tagId: string) {
    const snapshot = await load(gameName)
    const existing = snapshot.tags.find((tag) => tag.id === tagId)
    if (!existing) {
      return snapshot
    }

    await removeIconFile(gameName, existing.iconFile)
    const nextMappings: Record<string, string[]> = {}
    Object.entries(snapshot.modMappings).forEach(([modId, tagIds]) => {
      const remaining = tagIds.filter((item) => item !== tagId)
      if (remaining.length > 0) {
        nextMappings[modId] = remaining
      }
    })

    return save(gameName, {
      ...snapshot,
      tags: snapshot.tags.filter((tag) => tag.id !== tagId),
      modMappings: nextMappings,
    })
  }

  async function setModTags(gameName: string, modId: string, tagIds: string[]) {
    const snapshot = await load(gameName)
    const validTagIds = new Set(snapshot.tags.map((tag) => tag.id))
    const cleanModId = normalizePath(modId)
    const cleanTags = Array.from(new Set(tagIds
      .map((tagId) => String(tagId || '').trim())
      .filter((tagId) => !!tagId && validTagIds.has(tagId))))

    const nextMappings = {
      ...snapshot.modMappings,
    }

    if (cleanTags.length > 0) {
      nextMappings[cleanModId] = cleanTags
    } else {
      delete nextMappings[cleanModId]
    }

    return save(gameName, {
      ...snapshot,
      modMappings: nextMappings,
    })
  }

  async function toggleModTag(gameName: string, modId: string, tagId: string) {
    const snapshot = await load(gameName)
    const current = snapshot.modMappings[normalizePath(modId)] || []
    const next = current.includes(tagId)
      ? current.filter((item) => item !== tagId)
      : [...current, tagId]
    return setModTags(gameName, modId, next)
  }

  async function remapModPath(gameName: string, oldModId: string, newModId: string) {
    const snapshot = await load(gameName)
    const cleanOldId = normalizePath(oldModId)
    const cleanNewId = normalizePath(newModId)
    if (!cleanOldId || !cleanNewId || cleanOldId === cleanNewId) {
      return snapshot
    }

    const currentTags = snapshot.modMappings[cleanOldId]
    if (!currentTags) {
      return snapshot
    }

    const nextMappings = {
      ...snapshot.modMappings,
      [cleanNewId]: Array.from(new Set([...(snapshot.modMappings[cleanNewId] || []), ...currentTags])),
    }
    delete nextMappings[cleanOldId]

    return save(gameName, {
      ...snapshot,
      modMappings: nextMappings,
    })
  }

  async function remapPrefix(gameName: string, oldPrefix: string, newPrefix: string) {
    const snapshot = await load(gameName)
    const cleanOldPrefix = normalizePath(oldPrefix)
    const cleanNewPrefix = normalizePath(newPrefix)
    if (!cleanOldPrefix || !cleanNewPrefix || cleanOldPrefix === cleanNewPrefix) {
      return snapshot
    }

    const nextMappings: Record<string, string[]> = {}
    Object.entries(snapshot.modMappings).forEach(([modId, tagIds]) => {
      const cleanModId = normalizePath(modId)
      if (cleanModId === cleanOldPrefix || cleanModId.startsWith(`${cleanOldPrefix}/`)) {
        const suffix = cleanModId.slice(cleanOldPrefix.length)
        nextMappings[`${cleanNewPrefix}${suffix}`] = Array.from(new Set([...(nextMappings[`${cleanNewPrefix}${suffix}`] || []), ...tagIds]))
      } else {
        nextMappings[cleanModId] = Array.from(new Set([...(nextMappings[cleanModId] || []), ...tagIds]))
      }
    })

    return save(gameName, {
      ...snapshot,
      modMappings: nextMappings,
    })
  }

  async function deleteModMapping(gameName: string, modId: string) {
    const snapshot = await load(gameName)
    const cleanModId = normalizePath(modId)
    if (!(cleanModId in snapshot.modMappings)) {
      return snapshot
    }
    const nextMappings = { ...snapshot.modMappings }
    delete nextMappings[cleanModId]
    return save(gameName, {
      ...snapshot,
      modMappings: nextMappings,
    })
  }

  async function deletePrefixMappings(gameName: string, prefix: string) {
    const snapshot = await load(gameName)
    const cleanPrefix = normalizePath(prefix)
    if (!cleanPrefix) {
      return snapshot
    }

    const nextMappings: Record<string, string[]> = {}
    Object.entries(snapshot.modMappings).forEach(([modId, tagIds]) => {
      const cleanModId = normalizePath(modId)
      if (cleanModId === cleanPrefix || cleanModId.startsWith(`${cleanPrefix}/`)) {
        return
      }
      nextMappings[cleanModId] = tagIds
    })

    return save(gameName, {
      ...snapshot,
      modMappings: nextMappings,
    })
  }

  async function getTagIconPath(gameName: string, tag: ModTagDefinition) {
    if (!tag.iconFile) {
      return ''
    }
    const iconsDir = await getIconsDir(gameName)
    return joinPath(iconsDir, tag.iconFile)
  }

  async function cleanupUnusedIcons(gameName: string, _tags: ModTagDefinition[]) {
    // 调用方快照可能已经过期，必须在队列内重新读取后再判断图标归属。
    const { tags } = await load(gameName)
    const iconsDir = await getIconsDir(gameName)
    if (!(await exists(iconsDir))) {
      return
    }

    const validFiles = new Set(tags.map((tag) => tag.iconFile).filter((value): value is string => !!value))
    const entries = await readDir(iconsDir)
    for (const entry of entries) {
      if (!entry.name || entry.isDirectory) {
        continue
      }
      if (!validFiles.has(entry.name)) {
        const iconPath = joinPath(iconsDir, entry.name)
        await moveFileToRecycleBin(iconPath)
      }
    }
  }

  // ---- Group tag methods ----

  async function setGroupTags(gameName: string, groupId: string, tagIds: string[]) {
    const snapshot = await load(gameName)
    const validTagIds = new Set(snapshot.tags.map((tag) => tag.id))
    const cleanGroupId = normalizePath(groupId)
    const cleanTags = Array.from(new Set(tagIds
      .map((tagId) => String(tagId || '').trim())
      .filter((tagId) => !!tagId && validTagIds.has(tagId))))

    const nextGroupMappings = { ...snapshot.groupMappings }

    if (cleanTags.length > 0) {
      nextGroupMappings[cleanGroupId] = cleanTags
    } else {
      delete nextGroupMappings[cleanGroupId]
    }

    return save(gameName, {
      ...snapshot,
      groupMappings: nextGroupMappings,
    })
  }

  async function remapGroupPath(gameName: string, oldGroupId: string, newGroupId: string) {
    const snapshot = await load(gameName)
    const cleanOldId = normalizePath(oldGroupId)
    const cleanNewId = normalizePath(newGroupId)
    if (!cleanOldId || !cleanNewId || cleanOldId === cleanNewId) {
      return snapshot
    }

    const currentTags = snapshot.groupMappings[cleanOldId]
    if (!currentTags) {
      return snapshot
    }

    const nextGroupMappings = {
      ...snapshot.groupMappings,
      [cleanNewId]: Array.from(new Set([...(snapshot.groupMappings[cleanNewId] || []), ...currentTags])),
    }
    delete nextGroupMappings[cleanOldId]

    return save(gameName, {
      ...snapshot,
      groupMappings: nextGroupMappings,
    })
  }

  async function remapGroupPrefix(gameName: string, oldPrefix: string, newPrefix: string) {
    const snapshot = await load(gameName)
    const cleanOldPrefix = normalizePath(oldPrefix)
    const cleanNewPrefix = normalizePath(newPrefix)
    if (!cleanOldPrefix || !cleanNewPrefix || cleanOldPrefix === cleanNewPrefix) {
      return snapshot
    }

    const nextGroupMappings: Record<string, string[]> = {}
    Object.entries(snapshot.groupMappings).forEach(([groupId, tagIds]) => {
      const cleanGroupId = normalizePath(groupId)
      if (cleanGroupId === cleanOldPrefix || cleanGroupId.startsWith(`${cleanOldPrefix}/`)) {
        const suffix = cleanGroupId.slice(cleanOldPrefix.length)
        nextGroupMappings[`${cleanNewPrefix}${suffix}`] = Array.from(new Set([...(nextGroupMappings[`${cleanNewPrefix}${suffix}`] || []), ...tagIds]))
      } else {
        nextGroupMappings[cleanGroupId] = Array.from(new Set([...(nextGroupMappings[cleanGroupId] || []), ...tagIds]))
      }
    })

    return save(gameName, {
      ...snapshot,
      groupMappings: nextGroupMappings,
    })
  }

  async function deleteGroupMapping(gameName: string, groupId: string) {
    const snapshot = await load(gameName)
    const cleanGroupId = normalizePath(groupId)
    if (!(cleanGroupId in snapshot.groupMappings)) {
      return snapshot
    }
    const nextGroupMappings = { ...snapshot.groupMappings }
    delete nextGroupMappings[cleanGroupId]
    return save(gameName, {
      ...snapshot,
      groupMappings: nextGroupMappings,
    })
  }

  return {
    load: serialize(load),
    save: serialize(async (gameName: string, snapshot: ModTagSnapshot) => {
      await load(gameName)
      return save(gameName, snapshot)
    }),
    upsertTag: serialize(upsertTag),
    deleteTag: serialize(deleteTag),
    setModTags: serialize(setModTags),
    toggleModTag: serialize(toggleModTag),
    remapModPath: serialize(remapModPath),
    remapPrefix: serialize(remapPrefix),
    deleteModMapping: serialize(deleteModMapping),
    deletePrefixMappings: serialize(deletePrefixMappings),
    getTagIconPath: serialize(getTagIconPath),
    cleanupUnusedIcons: serialize(cleanupUnusedIcons),
    setGroupTags: serialize(setGroupTags),
    remapGroupPath: serialize(remapGroupPath),
    remapGroupPrefix: serialize(remapGroupPrefix),
    deleteGroupMapping: serialize(deleteGroupMapping),
  }
})

// ---------------------------------------------------------------------------
// Backward-compatible wrapper — keeps `ModTagStore.load(...)` working
// ---------------------------------------------------------------------------

export const ModTagStore = {
  get load() { return useModTagStore().load },
  get save() { return useModTagStore().save },
  get upsertTag() { return useModTagStore().upsertTag },
  get deleteTag() { return useModTagStore().deleteTag },
  get setModTags() { return useModTagStore().setModTags },
  get toggleModTag() { return useModTagStore().toggleModTag },
  get remapModPath() { return useModTagStore().remapModPath },
  get remapPrefix() { return useModTagStore().remapPrefix },
  get deleteModMapping() { return useModTagStore().deleteModMapping },
  get deletePrefixMappings() { return useModTagStore().deletePrefixMappings },
  get getTagIconPath() { return useModTagStore().getTagIconPath },
  get cleanupUnusedIcons() { return useModTagStore().cleanupUnusedIcons },
  get setGroupTags() { return useModTagStore().setGroupTags },
  get remapGroupPath() { return useModTagStore().remapGroupPath },
  get remapGroupPrefix() { return useModTagStore().remapGroupPrefix },
  get deleteGroupMapping() { return useModTagStore().deleteGroupMapping },
} as const
