export type PluginPermissionRisk = 'low' | 'medium' | 'high'

export interface PluginPermissionDefinition {
  id: string
  labelKey: string
  descriptionKey: string
  risk: PluginPermissionRisk
}

export type PluginCompatibilityState = 'supported' | 'limited' | 'blocked' | 'unknown'

export interface PluginCompatibilityItem {
  id: string
  label: string
  value: string
  hint?: string
  state: PluginCompatibilityState
}

type ParsedVersion = [number, number, number]

const parseVersion = (value: string): ParsedVersion | null => {
  const match = value.trim().replace(/^[vV]/, '').match(/^(\d+)(?:\.(\d+|x|\*))?(?:\.(\d+|x|\*))?/)
  if (!match) return null
  const part = (raw: string | undefined) => raw && /^\d+$/.test(raw) ? Number(raw) : 0
  return [Number(match[1]), part(match[2]), part(match[3])]
}

const compareParsedVersions = (left: ParsedVersion, right: ParsedVersion): number => {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index]
  }
  return 0
}

const tupleWith = (version: ParsedVersion, major: number, minor: number, patch: number): ParsedVersion => [
  version[0] + major,
  version[1] + minor,
  version[2] + patch,
]

const satisfiesComparator = (version: ParsedVersion, rawComparator: string): boolean | null => {
  const match = rawComparator.match(/^(\^|~|>=|<=|>|<|=)?\s*(\d+(?:\.(?:\d+|x|\*)){0,2})$/)
  if (!match) return null
  const operator = match[1] ?? '='
  const expected = parseVersion(match[2])
  if (!expected) return null
  const comparison = compareParsedVersions(version, expected)

  switch (operator) {
    case '>=': return comparison >= 0
    case '<=': return comparison <= 0
    case '>': return comparison > 0
    case '<': return comparison < 0
    case '=': return comparison === 0
    case '~': {
      const upper = tupleWith(expected, 0, 1, 0)
      return comparison >= 0 && compareParsedVersions(version, upper) < 0
    }
    case '^': {
      const upper = expected[0] > 0
        ? tupleWith(expected, 1, 0, 0)
        : expected[1] > 0
          ? tupleWith(expected, 0, 1, 0)
          : tupleWith(expected, 0, 0, 1)
      return comparison >= 0 && compareParsedVersions(version, upper) < 0
    }
    default: return null
  }
}

const satisfiesRange = (versionText: string, range: string): boolean | null => {
  const version = parseVersion(versionText)
  if (!version) return null
  const comparators = range.trim().split(/[,\s]+/).filter(Boolean)
  if (!comparators.length) return null
  let matched = false
  for (const comparator of comparators) {
    const result = satisfiesComparator(version, comparator)
    if (result === null) return null
    if (!result) return false
    matched = true
  }
  return matched
}

export const versionRequirementState = (
  version: string,
  minimum?: string | null,
  maximum?: string | null,
): PluginCompatibilityState => {
  if (!version.trim()) return 'unknown'
  if (minimum) {
    const matches = satisfiesRange(version, minimum)
    if (matches === null) return 'unknown'
    if (!matches) return 'blocked'
  }
  if (maximum) {
    const normalizedMaximum = /^[<>=^~]/.test(maximum.trim()) ? maximum.trim() : `<=${maximum.trim()}`
    const matches = satisfiesRange(version, normalizedMaximum)
    if (matches === null) return 'unknown'
    if (!matches) return 'blocked'
  }
  return 'supported'
}

const PERMISSION_ORDER = [
  'native.inject',
  'process.spawn',
  'game.launch',
  'process.observe',
  'filesystem.write',
  'filesystem.read',
  'network',
]

const PERMISSION_DEFINITIONS: Record<string, PluginPermissionDefinition> = {
  'native.inject': {
    id: 'native.inject',
    labelKey: 'pluginMarketplace.permissionNames.nativeInject.label',
    descriptionKey: 'pluginMarketplace.permissionNames.nativeInject.description',
    risk: 'high',
  },
  'process.spawn': {
    id: 'process.spawn',
    labelKey: 'pluginMarketplace.permissionNames.processSpawn.label',
    descriptionKey: 'pluginMarketplace.permissionNames.processSpawn.description',
    risk: 'high',
  },
  'game.launch': {
    id: 'game.launch',
    labelKey: 'pluginMarketplace.permissionNames.gameLaunch.label',
    descriptionKey: 'pluginMarketplace.permissionNames.gameLaunch.description',
    risk: 'medium',
  },
  'process.observe': {
    id: 'process.observe',
    labelKey: 'pluginMarketplace.permissionNames.processObserve.label',
    descriptionKey: 'pluginMarketplace.permissionNames.processObserve.description',
    risk: 'medium',
  },
  'filesystem.write': {
    id: 'filesystem.write',
    labelKey: 'pluginMarketplace.permissionNames.filesystemWrite.label',
    descriptionKey: 'pluginMarketplace.permissionNames.filesystemWrite.description',
    risk: 'medium',
  },
  'filesystem.read': {
    id: 'filesystem.read',
    labelKey: 'pluginMarketplace.permissionNames.filesystemRead.label',
    descriptionKey: 'pluginMarketplace.permissionNames.filesystemRead.description',
    risk: 'medium',
  },
  network: {
    id: 'network',
    labelKey: 'pluginMarketplace.permissionNames.network.label',
    descriptionKey: 'pluginMarketplace.permissionNames.network.description',
    risk: 'medium',
  },
}

export const permissionDefinitionFor = (permission: string): PluginPermissionDefinition => (
  PERMISSION_DEFINITIONS[permission] ?? {
    id: permission,
    labelKey: permission,
    descriptionKey: '',
    risk: 'medium',
  }
)

export const orderedPermissions = (permissions: readonly string[]): string[] =>
  [...new Set(permissions)].sort((left, right) => {
    const leftIndex = PERMISSION_ORDER.indexOf(left)
    const rightIndex = PERMISSION_ORDER.indexOf(right)
    const leftRank = leftIndex === -1 ? PERMISSION_ORDER.length : leftIndex
    const rightRank = rightIndex === -1 ? PERMISSION_ORDER.length : rightIndex
    return leftRank - rightRank || left.localeCompare(right)
  })
