import { renameModWithLockRecovery } from '../utils/FileLocks'
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { exists, mkdir, remove, copyFile, rename, writeFile, readDir } from '@tauri-apps/plugin-fs'
import { openPath } from '@tauri-apps/plugin-opener'
import { invoke } from '@tauri-apps/api/core'
import { PathHelper } from '../helper/PathHelper'
import { i18n } from '../i18n'
import { moveDirectoryToRecycleBin } from '../utils/RecycleBin'

const t = i18n.global.t

// ---------------------------------------------------------------------------
// Public interfaces
// ---------------------------------------------------------------------------

export interface ModScanSignal {
  isCancelled?: () => boolean
}

class ModScanCancelledError extends Error {
  constructor() {
    super('Mod scan cancelled')
    this.name = 'ModScanCancelledError'
  }
}

export interface GroupInfo {
  id: string
  name: string
  iconPath?: string
  path: string
  enabled: boolean
  modCount?: number
  isDirectoryLink?: boolean
}

export interface ModInfo {
  id: string
  name: string
  enabled: boolean
  path: string
  relativePath: string
  previewImages: string[]
  group: string
  isDir: boolean
  lastModified: number
  isDirectoryLink?: boolean
}

export interface DisabledParentGroupInfo {
  disabledPath: string
  enabledPath: string
  name: string
}

export interface ModKeyInfo {
  id: string
  sourceIni: string
  sourceIniPath: string
  sectionName: string
  sectionIndex: number
  keyName: string
  backName: string
  keyType: string
  keys: string[]
  backs: string[]
  valueSummary: string
  conditionSummary: string
  activeValues: ModKeyScalarValue[]
  cycleValues: ModKeyCycleValue[]
  extraProperties: ModKeyProperty[]
}

export interface ModKeyConstantBinding {
  bindingKey: string
  name: string
  initialValue: string
  declarationSourceIni: string
  declarationSourceIniPath: string
  declarationSectionName: string
  declarationModifiers: string[]
  createdByEditor: boolean
}

export interface ModKeyScalarValue {
  name: string
  value: string
  constantBinding: ModKeyConstantBinding
}

export interface ModKeyCycleValue {
  name: string
  values: string[]
  constantBinding: ModKeyConstantBinding
}

export interface ModKeyProperty {
  id: string
  key: string
  value: string
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Standalone helpers (stateless, extracted from the former class)
// ---------------------------------------------------------------------------

const throwIfCancelled = (signal?: ModScanSignal) => {
  if (signal?.isCancelled?.()) {
    throw new ModScanCancelledError()
  }
}

const isScanCancelledError = (error: unknown) => error instanceof ModScanCancelledError

function joinPath(...parts: string[]): string {
  return parts
    .map((p, i) => {
      const normalized = p.replace(/\\/g, '/')
      if (i === 0) return normalized.replace(/\/+$|^\/+/, '')
      return normalized.replace(/^\/+|\/+$/g, '')
    })
    .filter(Boolean)
    .join('/')
}

function stripDisabledPrefix(name: string) {
  const upper = name.toUpperCase()
  if (upper.startsWith('DISABLED_')) return { clean: name.substring(9), disabled: true }
  if (upper.startsWith('DISABLED')) return { clean: name.substring(8), disabled: true }
  return { clean: name, disabled: false }
}

function normalizeGroupPath(groupPath: string): string {
  const normalized = String(groupPath || '').replace(/\\/g, '/').replace(/^\/+|\/+$/g, '')
  return normalized === 'Root' || normalized === 'All' ? '' : normalized
}

function normalizeGroupId(groupPath: string): string {
  return normalizeGroupPath(groupPath)
    .split('/')
    .filter(Boolean)
    .map((segment) => {
      const { clean } = stripDisabledPrefix(segment)
      return clean || segment
    })
    .join('/')
}

function makeGroupCacheKey(gameName: string, groupPath: string): string {
  const groupId = normalizeGroupId(groupPath)
  return `${gameName}:${groupId || 'Root'}`
}

// ---------------------------------------------------------------------------
// Pinia store
// ---------------------------------------------------------------------------

export const useModManagerStore = defineStore('modManager', () => {
  // -- reactive state -------------------------------------------------------
  const modCache = ref(new Map<string, { mods: ModInfo[]; groups: GroupInfo[]; timestamp: number }>())
  const modKeyCache = ref(new Map<string, ModKeyInfo[]>())

  // -- private helpers (store-scoped because they access state or other store methods) --

  function invalidateGameCache(gameName: string) {
    clearCache(gameName)
  }

  async function getModsDir(gameName: string): Promise<string> {
    const installDir = await getInstallDir(gameName)
    const normalizedInstallDir = installDir.replace(/\\/g, '/').replace(/\/+$/g, '')
    const installDirName = normalizedInstallDir.split('/').pop() || ''
    const modsDir = installDirName.toLowerCase() === 'mods'
      ? normalizedInstallDir
      : joinPath(normalizedInstallDir, 'Mods')
    if (!(await exists(modsDir))) {
      await mkdir(modsDir, { recursive: true })
    }
    return modsDir
  }

  async function findPhysicalGroupSegment(parentPath: string, logicalSegment: string): Promise<string | null> {
    try {
      const entries = await readDir(parentPath)
      let disabledMatch: string | null = null

      for (const entry of entries) {
        const name = entry.name || ''
        if (!name || name.startsWith('.') || name.startsWith('$') || !entry.isDirectory) {
          continue
        }

        const { clean, disabled } = stripDisabledPrefix(name)
        if ((clean || name).toLowerCase() !== logicalSegment.toLowerCase()) {
          continue
        }

        if (!disabled) return name
        if (!disabledMatch) disabledMatch = name
      }

      return disabledMatch
    } catch {
      return null
    }
  }

  async function resolvePhysicalGroupPath(
    gameName: string,
    groupPath: string,
    options?: { allowMissing?: boolean },
  ): Promise<string | null> {
    const groupId = normalizeGroupId(groupPath)
    if (!groupId) return ''

    const modsDir = await getModsDir(gameName)
    let parentPath = modsDir
    const physicalParts: string[] = []

    for (const segment of groupId.split('/').filter(Boolean)) {
      const physicalSegment = await findPhysicalGroupSegment(parentPath, segment)
      if (!physicalSegment) {
        if (!options?.allowMissing) return null
        physicalParts.push(segment)
        parentPath = joinPath(parentPath, segment)
        continue
      }

      physicalParts.push(physicalSegment)
      parentPath = joinPath(parentPath, physicalSegment)
    }

    return physicalParts.join('/')
  }

  async function resolvePhysicalModPath(gameName: string, modRelativePath: string): Promise<string | null> {
    const normalized = String(modRelativePath || '').replace(/\\/g, '/').replace(/^\/+|\/+$/g, '')
    if (!normalized) return null

    const modsDir = await getModsDir(gameName)
    const directPath = joinPath(modsDir, normalized)
    if (await exists(directPath)) return normalized

    const parts = normalized.split('/').filter(Boolean)
    const requestedName = parts.pop()
    if (!requestedName) return null

    const logicalParent = normalizeGroupId(parts.join('/'))
    const physicalParent = await resolvePhysicalGroupPath(gameName, logicalParent)
    if (physicalParent === null) return null

    const parentPath = physicalParent ? joinPath(modsDir, physicalParent) : modsDir
    const requestedCleanName = stripDisabledPrefix(requestedName).clean.toLowerCase()
    try {
      const entries = await readDir(parentPath)
      const matches = entries
        .filter((entry) => entry.isDirectory && !!entry.name)
        .map((entry) => entry.name || '')
        .filter((name) => stripDisabledPrefix(name).clean.toLowerCase() === requestedCleanName)

      if (matches.length !== 1) return null
      return [physicalParent, matches[0]].filter(Boolean).join('/')
    } catch {
      return null
    }
  }

  // -- public methods -------------------------------------------------------

  async function getInstallDir(_gameName: string): Promise<string> {
    const resolved = await PathHelper.GetCurrentGame3DmigotoFolderPath()
    if (!resolved) throw new Error(t('modManager.messages.installDirNotConfigured'))
    return resolved.replace(/\\/g, '/').replace(/\/\/+/, '/')
  }

  function getCachedGroup(gameName: string, groupPath: string): { mods: ModInfo[]; groups: GroupInfo[] } | null {
    const cacheKey = makeGroupCacheKey(gameName, groupPath)
    const cached = modCache.value.get(cacheKey)
    if (!cached) return null
    return {
      mods: [...cached.mods],
      groups: [...cached.groups],
    }
  }

  function clearCache(gameName?: string) {
    if (gameName) {
      for (const key of modCache.value.keys()) {
        if (key.startsWith(`${gameName}:`)) {
          modCache.value.delete(key)
        }
      }
      for (const key of modKeyCache.value.keys()) {
        if (key.startsWith(`${gameName}:`)) {
          modKeyCache.value.delete(key)
        }
      }
    } else {
      modCache.value.clear()
      modKeyCache.value.clear()
    }
  }

  async function getModKeyList(gameName: string, modRelativePath: string): Promise<ModKeyInfo[]> {
    const cacheKey = `${gameName}:${modRelativePath}`
    const cached = modKeyCache.value.get(cacheKey)
    if (cached) {
      return [...cached]
    }

    const installDir = await getInstallDir(gameName)
    const result = await invoke<ModKeyInfo[]>('get_mod_key_list', {
      installDir,
      modRelativePath,
    })

    modKeyCache.value.set(cacheKey, result)
    return [...result]
  }

  function isScanCancelled(error: unknown) {
    return isScanCancelledError(error)
  }

  async function scanGroup(gameName: string, groupPath: string, signal?: ModScanSignal, options?: { refresh?: boolean }): Promise<{ mods: ModInfo[]; groups: GroupInfo[] }> {
    const groupId = normalizeGroupId(groupPath)
    const cacheKey = makeGroupCacheKey(gameName, groupId)
    if (!options?.refresh && modCache.value.has(cacheKey)) {
      return modCache.value.get(cacheKey)!
    }

    const relativePath = groupId
    const installDir = await getInstallDir(gameName)

    try {
      throwIfCancelled(signal)

      const result = await invoke<{ mods: ModInfo[]; groups: GroupInfo[] }>(options?.refresh ? 'mod_library_refresh_group' : 'mod_library_scan_group', {
        gameName,
        installDir,
        groupPath: relativePath || 'Root',
      })
      throwIfCancelled(signal)

      modCache.value.set(cacheKey, { ...result, timestamp: Date.now() })
      return result
    } catch (e) {
      if (isScanCancelledError(e)) {
        throw e
      }
      console.error('Scan error', e)
      // Never turn an I/O/backend failure into a successful empty scan. Empty
      // results are cached, so swallowing the error makes subsequent refreshes
      // keep serving a poisoned snapshot.
      throw e
    }
  }

  /** Calls the streaming scan command — emits `mod-library-scan-chunk` events during the scan.
   *  Returns the installDir so callers can coordinate with the event listener. */
  async function streamScanGroup(gameName: string, groupPath: string): Promise<{ installDir: string; relativePath: string }> {
    const groupId = normalizeGroupId(groupPath)
    const relativePath = groupId
    const installDir = await getInstallDir(gameName)
    // Don't await — events fire during invocation; the promise resolves when done.
    // Caller listens for mod-library-scan-chunk events in parallel.
    invoke<{ mods: ModInfo[]; groups: GroupInfo[] }>('mod_library_stream_scan', {
      gameName,
      installDir,
      groupPath: relativePath || 'Root',
    }).then((result) => {
      // Cache the final result so subsequent lookups are instant
      const cacheKey = makeGroupCacheKey(gameName, groupId)
      modCache.value.set(cacheKey, { ...result, timestamp: Date.now() })
      return result
    }).catch((e) => {
      console.error('Stream scan error:', e)
    })
    return { installDir, relativePath }
  }

  async function scanMods(gameName: string): Promise<{ mods: ModInfo[]; groups: GroupInfo[] }> {
    return scanGroup(gameName, 'Root')
  }

  async function scanAllMods(gameName: string, signal?: ModScanSignal): Promise<{ mods: ModInfo[]; groups: GroupInfo[] }> {
    throwIfCancelled(signal)
    const installDir = await getInstallDir(gameName)
    const result = await invoke<{ mods: ModInfo[]; groups: GroupInfo[] }>('mod_library_all_mods', {
      gameName,
      installDir,
    })
    throwIfCancelled(signal)
    return result
  }

  async function refreshLibrary(gameName: string, signal?: ModScanSignal): Promise<{ mods: ModInfo[]; groups: GroupInfo[] }> {
    throwIfCancelled(signal)
    const installDir = await getInstallDir(gameName)
    const result = await invoke<{ mods: ModInfo[]; groups: GroupInfo[] }>('mod_library_refresh_all', {
      gameName,
      installDir,
    })
    throwIfCancelled(signal)
    clearCache(gameName)
    return result
  }

  async function createModGroup(gameName: string, groupPath: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const target = joinPath(modsDir, groupPath)

    if (await exists(target)) {
      throw new Error(t('modManager.messages.groupAlreadyExists'))
    }

    await mkdir(target, { recursive: true })
    invalidateGameCache(gameName)
  }

  async function groupExists(gameName: string, groupPath: string): Promise<boolean> {
    const modsDir = await getModsDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupPath)
    if (physicalGroupPath === null) return false
    const target = physicalGroupPath ? joinPath(modsDir, physicalGroupPath) : modsDir

    return await exists(target)
  }

  async function toggleGroup(gameName: string, groupPath: string, enable: boolean): Promise<string> {
    const modsDir = await getModsDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupPath)

    if (physicalGroupPath === null) {
      throw new Error(t('modManager.messages.groupDirectoryNotFound'))
    }

    const currentFullPath = joinPath(modsDir, physicalGroupPath)

    if (!(await exists(currentFullPath))) {
      throw new Error(t('modManager.messages.groupDirectoryNotFound'))
    }

    const parts = currentFullPath.split('/')
    const dirname = parts.pop() || ''
    const parent = parts.join('/')

    const upper = dirname.toUpperCase()

    const newDirname = enable
      ? upper.startsWith('DISABLED_')
        ? dirname.substring(9)
        : upper === 'DISABLED'
          ? dirname
          : upper.startsWith('DISABLED')
            ? dirname.substring(8)
            : dirname
      : upper.startsWith('DISABLED')
        ? dirname
        : `DISABLED_${dirname}`

    if (newDirname === dirname) {
      return physicalGroupPath
    }

    const newFullPath = joinPath(parent, newDirname)

    if (await exists(newFullPath)) {
      throw new Error(t('modManager.messages.groupWithTargetNameAlreadyExists'))
    }

    await renameModWithLockRecovery(currentFullPath, newFullPath, modsDir)
    invalidateGameCache(gameName)
    return newFullPath.replace(`${modsDir.replace(/\\/g, '/')}/`, '')
  }

  async function renameGroup(gameName: string, oldGroup: string, newGroup: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const physicalOldGroup = await resolvePhysicalGroupPath(gameName, oldGroup)

    if (physicalOldGroup === null) {
      throw new Error(t('modManager.messages.oldGroupDoesNotExist'))
    }

    const oldPath = joinPath(modsDir, physicalOldGroup)
    const newPath = joinPath(modsDir, newGroup)

    if (!(await exists(oldPath))) {
      throw new Error(t('modManager.messages.oldGroupDoesNotExist'))
    }
    if (await exists(newPath)) {
      throw new Error(t('modManager.messages.newGroupNameAlreadyTaken'))
    }

    await rename(oldPath, newPath)
    invalidateGameCache(gameName)
  }

  async function renameMod(gameName: string, modPath: string, newName: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const oldFullPath = joinPath(modsDir, modPath)

    if (!(await exists(oldFullPath))) {
      throw new Error(t('modManager.messages.modDoesNotExist'))
    }

    const parts = oldFullPath.split('/')
    const oldFolderName = parts.pop() || ''
    const parent = parts.join('/')
    const isDisabled = oldFolderName.toUpperCase().startsWith('DISABLED')

    const finalNewName = isDisabled
      ? newName.toUpperCase().startsWith('DISABLED')
        ? newName
        : oldFolderName.toUpperCase().startsWith('DISABLED_')
          ? `DISABLED_${newName}`
          : `DISABLED${newName}`
      : newName

    const newFullPath = joinPath(parent, finalNewName)

    if (await exists(newFullPath)) {
      throw new Error(t('modManager.messages.modWithSameNameAlreadyExists'))
    }

    await rename(oldFullPath, newFullPath)
    invalidateGameCache(gameName)
  }

  async function addModPreviewImages(gameName: string, modPath: string, imagePaths: string[]): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const targetModDir = joinPath(modsDir, modPath)

    if (!(await exists(targetModDir))) {
      throw new Error(t('modManager.messages.modDirectoryDoesNotExist'))
    }

    for (const imgPath of imagePaths) {
      try {
        const fileName = imgPath.split(/[\\/]/).pop() || ''
        if (!fileName) continue

        const destPath = joinPath(targetModDir, fileName)
        await copyFile(imgPath, destPath)
      } catch (e) {
        throw new Error(t('modManager.messages.copyImageFailed', { imagePath: imgPath, error: String(e) }))
      }
    }

    invalidateGameCache(gameName)
  }

  async function addModPreviewImageData(gameName: string, modPath: string, fileName: string, data: Uint8Array): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const targetModDir = joinPath(modsDir, modPath)

    if (!(await exists(targetModDir))) {
      throw new Error(t('modManager.messages.modDirectoryDoesNotExist'))
    }

    if (!fileName.trim()) {
      throw new Error(t('modManager.messages.invalidModPath'))
    }

    const destPath = joinPath(targetModDir, fileName)
    await writeFile(destPath, data)
    invalidateGameCache(gameName)
  }

  async function moveModToGroup(gameName: string, modRelativePath: string, newGroup: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const srcPath = joinPath(modsDir, modRelativePath)

    if (!(await exists(srcPath))) {
      throw new Error(t('modManager.messages.modNotFound'))
    }

    const modName = srcPath.split('/').pop() || ''
    if (!modName) throw new Error(t('modManager.messages.invalidModPath'))

    const targetGroup = await resolvePhysicalGroupPath(gameName, newGroup, { allowMissing: true }) || ''
    const destParent = targetGroup ? joinPath(modsDir, targetGroup) : modsDir

    if (!(await exists(destParent))) {
      await mkdir(destParent, { recursive: true })
    }

    const destPath = joinPath(destParent, modName)

    if (await exists(destPath)) {
      throw new Error(t('modManager.messages.modWithSameNameExistsInTargetGroup'))
    }

    await rename(srcPath, destPath)
    invalidateGameCache(gameName)
  }

  async function deleteGroup(gameName: string, groupName: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupName)

    if (physicalGroupPath === null) {
      throw new Error(t('modManager.messages.groupDoesNotExist'))
    }

    const target = joinPath(modsDir, physicalGroupPath)

    if (!(await exists(target))) {
      throw new Error(t('modManager.messages.groupDoesNotExist'))
    }

    await moveDirectoryToRecycleBin(target)
    invalidateGameCache(gameName)
  }

  async function deleteMod(gameName: string, modRelativePath: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const target = joinPath(modsDir, modRelativePath)

    if (!(await exists(target))) {
      throw new Error(t('modManager.messages.modPathDoesNotExist'))
    }

    await moveDirectoryToRecycleBin(target)
    invalidateGameCache(gameName)
  }

  async function toggleMod(gameName: string, modRelativePath: string, enable: boolean): Promise<string> {
    const modsDir = await getModsDir(gameName)
    const physicalModPath = await resolvePhysicalModPath(gameName, modRelativePath)
    if (!physicalModPath) {
      throw new Error(t('modManager.messages.modDirectoryNotFound'))
    }
    const currentFullPath = joinPath(modsDir, physicalModPath)

    const parts = currentFullPath.split('/')
    const dirname = parts.pop() || ''
    const parent = parts.join('/')

    const { clean: cleanName, disabled: wasDisabled } = stripDisabledPrefix(dirname)

    const newDirname = enable
      ? wasDisabled
        ? cleanName
        : dirname
      : dirname.toUpperCase().startsWith('DISABLED')
        ? dirname
        : `DISABLED_${dirname}`

    if (newDirname === dirname) {
      return physicalModPath
    }

    const newFullPath = joinPath(parent, newDirname)
    if (await exists(newFullPath)) {
      throw new Error(t('modManager.messages.modWithSameNameAlreadyExists'))
    }
    await renameModWithLockRecovery(currentFullPath, newFullPath, modsDir)
    invalidateGameCache(gameName)
    return newFullPath.replace(`${modsDir.replace(/\\/g, '/')}/`, '')
  }

  async function getDisabledParentGroups(_gameName: string, modRelativePath: string): Promise<DisabledParentGroupInfo[]> {
    const groupPath = normalizeGroupPath(modRelativePath).split('/').slice(0, -1)
    const disabledGroups: DisabledParentGroupInfo[] = []
    const currentPathParts: string[] = []

    for (const segment of groupPath) {
      const { clean, disabled } = stripDisabledPrefix(segment)
      const disabledPathParts = [...currentPathParts, segment]
      const enabledPathParts = [...currentPathParts, clean]

      if (disabled) {
        disabledGroups.push({
          disabledPath: disabledPathParts.join('/'),
          enabledPath: enabledPathParts.join('/'),
          name: clean || segment,
        })
      }

      currentPathParts.push(clean)
    }

    return disabledGroups
  }

  async function enableParentGroupsForMod(gameName: string, modRelativePath: string): Promise<DisabledParentGroupInfo[]> {
    const disabledGroups = await getDisabledParentGroups(gameName, modRelativePath)
    if (disabledGroups.length === 0) {
      return []
    }

    for (const group of disabledGroups) {
      await toggleGroup(gameName, group.disabledPath, true)
    }

    invalidateGameCache(gameName)
    return disabledGroups
  }

  async function setModGroupIcon(gameName: string, groupPath: string, iconPath: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupPath)

    if (physicalGroupPath === null) {
      throw new Error(t('modManager.messages.groupDirectoryNotFound'))
    }

    const destDir = joinPath(modsDir, physicalGroupPath)
    if (!(await exists(destDir))) {
      throw new Error(t('modManager.messages.groupDirectoryNotFound'))
    }

    const commonNames = ['folder.jpg', 'folder.png', 'icon.jpg', 'icon.png', 'cover.jpg', 'cover.png']
    for (const name of commonNames) {
      const p = joinPath(destDir, name)
      if (await exists(p)) {
        try { await remove(p) } catch {}
      }
    }

    const destPath = joinPath(destDir, 'icon.png')
    await copyFile(iconPath, destPath)
    invalidateGameCache(gameName)
  }

  async function openGameModsFolder(gameName: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    await openPath(modsDir)
  }

  async function openModGroupFolder(gameName: string, groupPath: string): Promise<void> {
    const modsDir = await getModsDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupPath)
    const destDir = physicalGroupPath === null ? joinPath(modsDir, groupPath) : joinPath(modsDir, physicalGroupPath)
    await openPath(destDir)
  }

  async function findNestedIniFiles(gameName: string, groupPath: string): Promise<string[]> {
    const installDir = await getInstallDir(gameName)
    const physicalGroupPath = await resolvePhysicalGroupPath(gameName, groupPath)
    if (physicalGroupPath === null) {
      throw new Error(t('modManager.messages.groupDirectoryNotFound'))
    }
    return invoke<string[]>('find_nested_ini_files', { installDir, groupPath: physicalGroupPath })
  }

  async function searchMods(gameName: string, query: string): Promise<string[]> {
    const installDir = await getInstallDir(gameName)
    return invoke<string[]>('mod_library_search', { gameName, installDir, query })
  }

  // -- return public API ----------------------------------------------------
  return {
    getInstallDir,
    getCachedGroup,
    clearCache,
    getModKeyList,
    isScanCancelled,
    scanGroup,
    streamScanGroup,
    scanMods,
    scanAllMods,
    refreshLibrary,
    createModGroup,
    groupExists,
    toggleGroup,
    renameGroup,
    renameMod,
    addModPreviewImages,
    addModPreviewImageData,
    moveModToGroup,
    deleteGroup,
    deleteMod,
    toggleMod,
    getDisabledParentGroups,
    enableParentGroupsForMod,
    setModGroupIcon,
    openGameModsFolder,
    openModGroupFolder,
    findNestedIniFiles,
    searchMods,
  }
})

/**
 * Backward-compatible wrapper that delegates to the Pinia store.
 * Existing callers using `ModManager.methodName(...)` continue to work.
 * @deprecated Prefer `useModManagerStore()` in new code.
 */
export const ModManager = {
  get getInstallDir() { return useModManagerStore().getInstallDir },
  get getCachedGroup() { return useModManagerStore().getCachedGroup },
  get clearCache() { return useModManagerStore().clearCache },
  get getModKeyList() { return useModManagerStore().getModKeyList },
  get isScanCancelled() { return useModManagerStore().isScanCancelled },
  get scanGroup() { return useModManagerStore().scanGroup },
  get streamScanGroup() { return useModManagerStore().streamScanGroup },
  get scanMods() { return useModManagerStore().scanMods },
  get scanAllMods() { return useModManagerStore().scanAllMods },
  get refreshLibrary() { return useModManagerStore().refreshLibrary },
  get createModGroup() { return useModManagerStore().createModGroup },
  get groupExists() { return useModManagerStore().groupExists },
  get toggleGroup() { return useModManagerStore().toggleGroup },
  get renameGroup() { return useModManagerStore().renameGroup },
  get renameMod() { return useModManagerStore().renameMod },
  get addModPreviewImages() { return useModManagerStore().addModPreviewImages },
  get addModPreviewImageData() { return useModManagerStore().addModPreviewImageData },
  get moveModToGroup() { return useModManagerStore().moveModToGroup },
  get deleteGroup() { return useModManagerStore().deleteGroup },
  get deleteMod() { return useModManagerStore().deleteMod },
  get toggleMod() { return useModManagerStore().toggleMod },
  get getDisabledParentGroups() { return useModManagerStore().getDisabledParentGroups },
  get enableParentGroupsForMod() { return useModManagerStore().enableParentGroupsForMod },
  get setModGroupIcon() { return useModManagerStore().setModGroupIcon },
  get openGameModsFolder() { return useModManagerStore().openGameModsFolder },
  get searchMods() { return useModManagerStore().searchMods },
  get openModGroupFolder() { return useModManagerStore().openModGroupFolder },
  get findNestedIniFiles() { return useModManagerStore().findNestedIniFiles },
}
