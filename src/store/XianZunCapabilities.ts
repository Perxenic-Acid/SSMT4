/* ═══════════════════════════════════════════════════════════════
   CheeseCat (芝士猫) — Full app capability layer
   ═══════════════════════════════════════════════════════════════
   Beyond the 39 Tauri commands (XianZunMcp.ts), this registry
   exposes every public function of the frontend stores / helpers
   as a callable capability, so the agent can freely combine them
   like its own arms.

   How it works:
   - registerAuto() walks a module's function properties and parses
     each function's parameter names from its source, so the agent
     can call with `{ paramName: value }` without a hardcoded schema.
   - Risk is inferred from the function name (delete/remove/install/
     save …) so destructive actions require user confirmation.
   ═══════════════════════════════════════════════════════════════ */

import type { RiskLevel } from './XianZunMcp'
import { serializeResult } from './XianZunMcp'
import { MigotoManager } from './MigotoManager'
import { PathHelper } from '../helper/PathHelper'
import { LaunchGame } from '../common/LaunchGame'
import { GlobalConfig } from './GlobalConfig'
import * as AppSelfUpdate from '../common/AppSelfUpdate'
import * as TextureMarkMemory from '../common/TextureMarkMemory'
import * as TextureMarkNameMemory from '../common/TextureMarkNameMemory'
import * as DrawIBConfig from '../common/DrawIBConfig'
import * as UpdateCheckUtils from '../common/UpdateCheckUtils'
import * as SSMTFileUtils from '../utils/SSMTFileUtils'
import * as SSMTStringUtils from '../utils/SSMTStringUtils'

export interface CapabilityTool {
  name: string
  description: string
  category: string
  risk: RiskLevel
  inputSchema: { type: 'object'; properties: Record<string, never>; required: string[] }
  execute: (args: Record<string, unknown>) => string | Promise<string>
}

/* Stores are created inside the component (active pinia), so the
   caller passes instances in. */
export interface CapabilityContext {
  resourceManager: Record<string, unknown>
  modManager: Record<string, unknown>
  modTagStore: Record<string, unknown>
  modPresetStore: Record<string, unknown>
  modStateStore: Record<string, unknown>
  gameConfig: Record<string, unknown>
}

/* ═══════════════════════════════════════════════
   Heuristics
   ═══════════════════════════════════════════════ */

const DANGER_HINTS = [
  'delete',
  'remove',
  'uninstall',
  'recycle',
  'overwrite',
  'purge',
  'destroy',
  'clear',
]

const WRITE_HINTS = [
  'save',
  'write',
  'create',
  'install',
  'update',
  'set',
  'apply',
  'move',
  'rename',
  'toggle',
  'switch',
  'patch',
  'add',
  'import',
  'export',
  'upsert',
  'copy',
  'remap',
  'enable',
  'disable',
]

const guessRisk = (fnName: string): RiskLevel => {
  const lower = fnName.toLowerCase()
  // 'preset' contains the substring 'reset' — never treat presets as destructive.
  if (!lower.includes('preset') && DANGER_HINTS.some((hint) => lower.includes(hint))) {
    return 'danger'
  }
  if (WRITE_HINTS.some((hint) => lower.includes(hint))) return 'write'
  return 'read'
}

/** Parse parameter names from a function's source (works for async,
    arrow and method shorthand; dev builds keep real names). */
const parseParamNames = (fn: (...args: unknown[]) => unknown): string[] => {
  try {
    const source = fn.toString()
    const match = source.match(/^[^(]*\(([^)]*)\)/)
    if (!match) return []
    return match[1]
      .split(',')
      .map((part) => part.trim().replace(/=.*$/, '').replace(/^\.\.\./, '').trim())
      .filter(Boolean)
  } catch {
    return []
  }
}

const registerAuto = (
  prefix: string,
  category: string,
  label: string,
  module: Record<string, unknown>,
  extraExclude: string[] = [],
): CapabilityTool[] => {
  const excluded = new Set<string>(['constructor', 'then', 'catch', ...extraExclude])
  const tools: CapabilityTool[] = []

  // Static class methods are non-enumerable, so Object.entries() misses them
  // (for example GlobalConfig.AppSettingsFilePath). Walk own property names so
  // classes, namespaces and store instances all register correctly.
  for (const key of Object.getOwnPropertyNames(module)) {
    if (excluded.has(key)) continue
    if (key.startsWith('$') || key.startsWith('_')) continue
    const value = (module as Record<string, unknown>)[key]
    if (typeof value !== 'function') continue

    const fn = value as (...args: unknown[]) => unknown
    const params = parseParamNames(fn)
    const risk = guessRisk(key)

    tools.push({
      name: `${prefix}.${key}`,
      description: [
        `${label}模块函数 ${key}(${params.join(', ') || '无参数'})。`,
        `以 { ${params.map((p) => `${p}: 值`).join(', ') || ''} } 形式传入参数(键名须与参数名一致)。`,
        `若执行报错,请根据错误信息修正参数后重试。${risk === 'danger' ? '⚠ 该操作会修改或删除数据,需要用户确认。' : ''}`,
      ].join(' '),
      category,
      risk,
      inputSchema: { type: 'object', properties: {}, required: [] },
      execute: async (args) => {
        const callArgs = params.map((name) => args[name])
        return serializeResult(await fn.apply(module, callArgs))
      },
    })
  }
  return tools
}

/* ═══════════════════════════════════════════════
   Registry builder — call once from the page setup
   ═══════════════════════════════════════════════ */

export const buildCapabilityTools = (ctx: CapabilityContext): CapabilityTool[] => {
  const tools: CapabilityTool[] = []

  // Pinia stores (instances passed in)
  tools.push(
    ...registerAuto(
      'ResourceManager',
      '资源管理',
      '游戏扫描/游戏配置/背景图/更新安装',
      ctx.resourceManager,
      [
        'getD3d11ReleaseSource',
        'findExistingFileIgnoreCase',
        'getFixedBackgroundUrl',
        'getHypBackgroundUrl',
        'resolveBackgroundDownloadUrl',
        'resolveBootDllSource',
        'resolveD3d11SourcePathByMode',
        'resolveMigotoDllSource',
        'resolveGithubReleaseAsset',
        'resolveGithubToken',
        'mapGithubReleaseToUpdateInfo',
        'normalizeD3d11Mode',
        'isDx12GamePreset',
        'isMihoyoGamePreset',
        'supportsAutoUpdateBackground',
        'getGithubRepoByGamePreset',
        'getGameD3d11Mode',
        'ensureGlobalGameIconConfigExists',
        'getMissingXXMILibsFiles',
      ],
    ),
    ...registerAuto('ModManager', '模组管理', 'Mod 扫描/开关/分组/重命名/预览图', ctx.modManager, [
      'throwIfCancelled',
      'isScanCancelled',
      'isScanCancelledError',
      'getCachedGroup',
      'makeGroupCacheKey',
      'invalidateGameCache',
      'clearCache',
      'findPhysicalGroupSegment',
      'normalizeGroupId',
      'normalizeGroupPath',
      'resolvePhysicalGroupPath',
      'resolvePhysicalModPath',
      'stripDisabledPrefix',
      'enableParentGroupsForMod',
      'getDisabledParentGroups',
    ]),
    ...registerAuto('ModTags', '模组标签', 'Mod/分组标签与图标管理', ctx.modTagStore, [
      'getStoreDir',
      'getMappingsFilePath',
      'getTagsFilePath',
      'getIconsDir',
      'getGroupMappingsFilePath',
      'hashString',
      'slugifyTagName',
      'normalizeColor',
      'normalizePath',
      'normalizeTagName',
      'pruneMappings',
      'load',
      'buildSnapshot',
      'cleanupUnusedIcons',
      'ensureStoreDir',
      'normalizeTagMappings',
      'normalizeGroupPath',
    ]),
    ...registerAuto('ModPresets', '模组预设', 'Mod 预设创建/应用/删除', ctx.modPresetStore, [
      'getPresetsPath',
      'loadPresets',
      'buildModRuntimeRestoreValues',
      'collectModRuntimeVariables',
      'generatePresetId',
      'loadIni',
      'getSectionEntries',
      'removeModRuntimeEntriesFromLines',
      'resetPresetState',
    ]),
    ...registerAuto('ModState', '模组状态', 'Mod 状态持久化/读取', ctx.modStateStore, [
      'getStoreDir',
      'getStoreFile',
      'ensureStoreDir',
      'normalizeModState',
      'normalizeModStateId',
      'normalizePath',
      'remapPrefix',
      'writeSnapshot',
      'load',
    ]),
    ...registerAuto('GameConfig', '游戏配置', '游戏配置创建/读写/删除', ctx.gameConfig, [
      'getConfigPath',
      'buildInitialConfig',
      'createNewConfig',
      'normalizeConfig',
      'normalizeD3d11Mode',
      'normalizeHuntingMode',
      'normalizeExtraDll',
      'normalizeExtraDlls',
      'normalizeLaunchProgramItem',
      'normalizeLaunchProgramList',
      'resolveGamePresetByGameName',
      'getForcedD3d11ModeByGamePreset',
      'isValidGamePreset',
      'applyMatchedGamePreset',
    ]),
  )

  // Static classes & plain modules
  tools.push(
    ...registerAuto('MigotoManager', '3Dmigoto', '3Dmigoto 集成/模式切换/IB dump', MigotoManager as unknown as Record<string, unknown>),
    ...registerAuto('PathHelper', '路径', '游戏/3Dmigoto/工作区路径查询', PathHelper as unknown as Record<string, unknown>),
    ...registerAuto('LaunchGame', '游戏启动', '游戏启动全流程(检查依赖/补丁/启动)', LaunchGame as unknown as Record<string, unknown>, [
      'formatLaunchError',
      'formatProgramsForLog',
      'formatProgramSummary',
      'getLaunchProgramGroupLabel',
      'getProcessNameFromPath',
      'isD3d11BusyError',
    ]),
    ...registerAuto('GlobalConfig', '全局配置', '全局设置读写', GlobalConfig as unknown as Record<string, unknown>),
    ...registerAuto('AppSelfUpdate', '应用更新', '检查/下载/安装应用更新', AppSelfUpdate as unknown as Record<string, unknown>),
    ...registerAuto('TextureMarkMemory', '纹理标记', '纹理标记记忆读写', TextureMarkMemory as unknown as Record<string, unknown>),
    ...registerAuto('TextureMarkNameMemory', '纹理标记', '纹理标记命名记忆', TextureMarkNameMemory as unknown as Record<string, unknown>),
    ...registerAuto('DrawIBConfig', '纹理标记', 'DrawIB 配置读写', DrawIBConfig as unknown as Record<string, unknown>),
    ...registerAuto('UpdateCheckUtils', '更新检查', '更新检查工具', UpdateCheckUtils as unknown as Record<string, unknown>),
    ...registerAuto('SSMTFileUtils', '文件工具', '路径拼接/文件 MD5/JSON 读写', SSMTFileUtils as unknown as Record<string, unknown>),
    ...registerAuto('SSMTStringUtils', '字符串工具', '字符串处理工具', SSMTStringUtils as unknown as Record<string, unknown>),
  )

  return tools
}
