import { join } from '@tauri-apps/api/path'
import { exists, remove, writeTextFile, copyFile, rename } from '@tauri-apps/plugin-fs'
import { D3dxIniManager } from './D3dxIniManager'
import { ResourceManager } from './ResourceManager'
import { PathHelper } from '../helper/PathHelper'
import { i18n } from '../i18n'
import type { GameConfig, HuntingMode } from './GameConfig'

const t = i18n.global.t

const ANALYSE_ENABLE = 'deferred_ctx_immediate dump_rt dump_cb dump_vb dump_ib buf txt dds dump_tex dds symlink'
const ANALYSE_DISABLE = 'deferred_ctx_immediate dump_rt dump_cb dump_vb dump_ib buf txt dds dump_tex dds'
const SPECIFIC_IB_DUMP_FILE_NAME = 'DumpIBListConfig.ini'

const SPECIFIC_IB_ROOT_SHADER_MAP: Readonly<Record<string, readonly string[]>> = {
  SRMI: [
    '1c932707d4d8df41',
    'fee307b98a965c16',
    '4d9c23fd387846c7',
    'd50694eedd2a8595',
    'c9f2b46571d22858',
  ],
  GIMI: ['653c63ba4a73ca8b'],
  ZZMI: ['e8425f64cfb887cd', 'a0b21a8e787c5a98', '9684c4091fc9e35a'],
  ZZMIDX12: ['e8425f64cfb887cd', 'a0b21a8e787c5a98', '9684c4091fc9e35a'],
}

const SPECIFIC_IB_LOGIC_WITHOUT_ROOT_SHADER: ReadonlySet<string> = new Set(['CTXMC', 'IdentityV', 'EFMI', 'WWMI'])

const SPECIFIC_IB_PRESET_TO_LOGIC_MAP: Readonly<Record<string, string>> = {
  GIMI: 'GIMI',
  SRMI: 'SRMI',
  ZZMI: 'ZZMI',
  ZZMIDX12: 'ZZMIDX12',
  IDENTITYV: 'IdentityV',
}

const normalizeHuntingMode = (value: unknown): HuntingMode => {
  if (value === '0' || value === '1' || value === '2') {
    return value
  }
  return '2'
}

export interface ThreeDMigotoConfig {
  gamePreset?: string
  installDir?: string
  targetExePath?: string
  launcherExePath?: string
  launchArgs?: string
  useShell?: boolean
  showErrorPopup?: boolean
  autoSetAnalyseOptions?: boolean
  huntingMode?: HuntingMode
  delay?: number
  extraDll?: string
  extraDlls?: string[]
  autoExitSeconds?: number
  useUpx?: boolean
}

export interface SpecificIbDumpOptions {
  gameName: string
  gamePreset?: string
  logicName?: string
  drawIbs: string[]
  restoreAnalyseOptions?: string
}

export interface SpecificIbDumpResult {
  restoreAnalyseOptions: string
  logicName: string
}

const normalizeSpecificIbDumpLogicName = (logicName: unknown, gamePreset: unknown): string => {
  const normalizedLogicName = typeof logicName === 'string' ? logicName.trim().toUpperCase() : ''
  if (normalizedLogicName) {
    return normalizedLogicName
  }

  const normalizedPreset = typeof gamePreset === 'string' ? gamePreset.trim().toUpperCase() : ''
  return SPECIFIC_IB_PRESET_TO_LOGIC_MAP[normalizedPreset] || normalizedPreset
}

const normalizeSpecificIbDumpDrawIbs = (drawIbs: string[]): string[] => {
  const normalized: string[] = []
  const seen = new Set<string>()

  drawIbs.forEach(drawIb => {
    const trimmed = drawIb.trim()
    if (!trimmed || seen.has(trimmed)) {
      return
    }

    seen.add(trimmed)
    normalized.push(trimmed)
  })

  return normalized
}

const buildSpecificIbDumpConfigContent = (drawIbs: string[], logicName: string, analyseOptions: string): string => {
  const normalizedDrawIbs = normalizeSpecificIbDumpDrawIbs(drawIbs)
  if (normalizedDrawIbs.length === 0) {
    throw new Error(t('migotoManager.messages.specificIbDumpRequiresDrawIbs'))
  }

  const rootShaders = SPECIFIC_IB_ROOT_SHADER_MAP[logicName] || []
  const logicWithoutRootShader = SPECIFIC_IB_LOGIC_WITHOUT_ROOT_SHADER.has(logicName)
  if (!logicWithoutRootShader && rootShaders.length === 0) {
    throw new Error(t('migotoManager.messages.specificIbDumpUnsupportedLogic', { logicName }))
  }

  const lines: string[] = []

  normalizedDrawIbs.forEach(drawIb => {
    lines.push(`[TextureOverride_IB_Dump_${drawIb}]`)
    lines.push(`hash = ${drawIb}`)
    lines.push(`analyse_options = ${analyseOptions}`)
    lines.push('')
  })

  rootShaders.forEach(rootShader => {
    lines.push(`[ShaderOverride_Dump_${rootShader}]`)
    lines.push(`hash = ${rootShader}`)
    lines.push(`analyse_options = ${analyseOptions}`)
    lines.push('')
  })

  return lines.join('\n')
}

export class MigotoManager {
  static async resolveMigotoDir(_gameName: string, _cfg: ThreeDMigotoConfig): Promise<string> {
    const resolved = await PathHelper.GetCurrentGame3DmigotoFolderPath()
    if (resolved && resolved.trim()) return resolved
    throw new Error(t('migotoManager.messages.installDirNotConfigured'))
  }

  static async check3DmigotoIntegrity(gameName: string): Promise<boolean> {
    const gameConfig = await ResourceManager.loadGameConfig(gameName)
    const cfg = gameConfig || {}
    const migotoDir = await this.resolveMigotoDir(gameName, cfg)
    const d3dxPath = await join(migotoDir, 'd3dx.ini')
    return exists(d3dxPath)
  }

  static async applySSMTRuntimeToMigotoDir(gameName: string, migotoDir: string, config?: GameConfig): Promise<void> {
    const effectiveConfig = config || await ResourceManager.loadGameConfig(gameName)
    const dllSource = await ResourceManager.resolveMigotoDllSource(effectiveConfig)
    const sourcePath = dllSource.sourcePath
    const destPath = await join(migotoDir, dllSource.targetFileName)

    if (!(await exists(sourcePath))) {
      throw new Error(t('migotoManager.messages.ssmtRuntimeSourceMissing', { path: sourcePath }))
    }

    const stagingPath = `${destPath}.ssmt-staging-${Date.now()}-${Math.random().toString(16).slice(2)}`
    const backupPath = `${destPath}.ssmt-backup-${Date.now()}-${Math.random().toString(16).slice(2)}`
    let backupCreated = false

    try {
      // Stage beside the destination so the final rename stays on the same
      // volume. Keep the existing DLL available until the staged copy is ready.
      await copyFile(sourcePath, stagingPath)

      if (await exists(destPath)) {
        await rename(destPath, backupPath)
        backupCreated = true
      }

      await rename(stagingPath, destPath)

      if (backupCreated && await exists(backupPath)) {
        await remove(backupPath)
      }
    } catch (error) {
      try {
        if (await exists(stagingPath)) {
          await remove(stagingPath)
        }
        if (backupCreated && !(await exists(destPath)) && await exists(backupPath)) {
          await rename(backupPath, destPath)
        }
      } catch (restoreError) {
        throw new Error(`${String(error)}; failed to restore the previous ${dllSource.label}: ${String(restoreError)}`)
      }

      throw error
    }

  }

  /**
   * 将当前配置写入d3dx.ini
   * @param gameName 
   */
  static async patchD3dxForLaunch(
    gameName: string,
    options: { dllInitializationDelay?: number } = {},
  ): Promise<void> {
    const gameConfig = await ResourceManager.loadGameConfig(gameName)
    const cfg = gameConfig || {}
    const migotoDir = await this.resolveMigotoDir(gameName, cfg)
    const iniPath = await join(migotoDir, 'd3dx.ini')
    if (!(await exists(iniPath))) throw new Error(t('migotoManager.messages.iniNotFoundAt', { path: iniPath }))

    let lines = await D3dxIniManager.loadIni(iniPath)
    const dllSource = await ResourceManager.resolveMigotoDllSource(cfg)
    lines = D3dxIniManager.setIniValue(lines, 'Loader', 'module', dllSource.targetFileName)

    // Upstream packages use `loader` as an XXMI-only allowlist. SSMT validates
    // the exact target path and does not require that external launcher.
    lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'loader')

    if (cfg.targetExePath && cfg.targetExePath.trim()) {
      lines = D3dxIniManager.setIniValue(lines, 'Loader', 'target', cfg.targetExePath)
    }

    const runShell = !!cfg.useShell
    if (runShell) {
      lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'launch')
      lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'launch_args')
    } else {
      const launchTarget = (cfg.targetExePath && cfg.targetExePath.trim())
        ? cfg.targetExePath.trim()
        : ''

      // Extra DLL injection in Run.exe only happens on the launch-and-inject path.
      // If `launch` is empty, Run.exe falls back to wait mode and skips inject_dlls.
      if (launchTarget) {
        lines = D3dxIniManager.setIniValue(lines, 'Loader', 'launch', launchTarget)
      } else {
        lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'launch')
      }

      if (launchTarget && cfg.launchArgs && cfg.launchArgs.trim()) {
        lines = D3dxIniManager.setIniValue(lines, 'Loader', 'launch_args', cfg.launchArgs)
      } else {
        lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'launch_args')
      }
    }

    const showWarnings = !!cfg.showErrorPopup
    lines = D3dxIniManager.setIniValue(lines, 'Logging', 'show_warnings', showWarnings ? '1' : '0')

    if (cfg.autoSetAnalyseOptions) {
      lines = D3dxIniManager.setIniValue(
        lines,
        'Hunting',
        'analyse_options',
        'deferred_ctx_immediate dump_rt dump_cb dump_vb dump_ib buf txt dds dump_tex dds symlink'
      )
    }

    const dllInitializationDelay = typeof options.dllInitializationDelay === 'number'
      ? options.dllInitializationDelay
      : cfg.delay
    if (typeof dllInitializationDelay === 'number') {
      lines = D3dxIniManager.setIniValue(
        lines,
        'System',
        'dll_initialization_delay',
        String(dllInitializationDelay),
      )
    }

    lines = D3dxIniManager.setIniValue(lines, 'Hunting', 'hunting', normalizeHuntingMode(cfg.huntingMode))
    lines = D3dxIniManager.setIniValue(lines, 'Hunting', 'marking_actions', 'clipboard asm hlsl')

    if (typeof cfg.autoExitSeconds === 'number') {
      lines = D3dxIniManager.setIniValue(lines, 'Loader', 'delay', String(cfg.autoExitSeconds))
    }

    const extraDlls = Array.isArray(cfg.extraDlls)
      ? cfg.extraDlls.filter((dll): dll is string => typeof dll === 'string').map(dll => dll.trim()).filter(Boolean)
      : []
    const legacyExtraDll = typeof cfg.extraDll === 'string' ? cfg.extraDll.trim() : ''
    const dllsToInject = extraDlls.length > 0 ? extraDlls : (legacyExtraDll ? [legacyExtraDll] : [])

    if (dllsToInject.length > 0) {
      lines = D3dxIniManager.setIniValue(lines, 'Loader', 'inject_dlls', dllsToInject.join('|'))
      lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'inject_dll')
    } else {
      lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'inject_dlls')
      lines = D3dxIniManager.removeIniKey(lines, 'Loader', 'inject_dll')
    }

    await D3dxIniManager.saveIni(iniPath, lines)
  }

  static async toggleSymlinkOption(gameName: string, enable: boolean): Promise<void> {
    const gameConfig = await ResourceManager.loadGameConfig(gameName)
    const cfg = gameConfig || {}
    const migotoDir = await this.resolveMigotoDir(gameName, cfg)
    const iniPath = await join(migotoDir, 'd3dx.ini')
    if (!(await exists(iniPath))) throw new Error(t('migotoManager.messages.iniNotFoundAt', { path: iniPath }))

    let lines = await D3dxIniManager.loadIni(iniPath)
    const val = enable ? ANALYSE_ENABLE : ANALYSE_DISABLE
    lines = D3dxIniManager.setIniValue(lines, 'hunting', 'analyse_options', val)
    await D3dxIniManager.saveIni(iniPath, lines)
  }

  static async enableSpecificIbDumpMode(options: SpecificIbDumpOptions): Promise<SpecificIbDumpResult> {
    const gameConfig = await ResourceManager.loadGameConfig(options.gameName)
    const cfg = gameConfig || {}
    const migotoDir = await this.resolveMigotoDir(options.gameName, cfg)
    const iniPath = await join(migotoDir, 'd3dx.ini')
    const dumpConfigPath = await join(migotoDir, SPECIFIC_IB_DUMP_FILE_NAME)
    if (!(await exists(iniPath))) throw new Error(t('migotoManager.messages.iniNotFoundAt', { path: iniPath }))

    const resolvedLogicName = normalizeSpecificIbDumpLogicName(options.logicName, options.gamePreset)
    const lines = await D3dxIniManager.loadIni(iniPath)
    const currentAnalyseOptions = D3dxIniManager.getIniValue(lines, 'hunting', 'analyse_options') || ''
    const specificAnalyseOptions = currentAnalyseOptions.trim() || ANALYSE_DISABLE
    const dumpConfigContent = buildSpecificIbDumpConfigContent(options.drawIbs, resolvedLogicName, specificAnalyseOptions)

    await writeTextFile(dumpConfigPath, dumpConfigContent)

    let nextLines = D3dxIniManager.setSectionLineEnabled(lines, 'include', 'include_recursive = Mods', false)
    nextLines = D3dxIniManager.setSectionLineEnabled(nextLines, 'include', `include = ${SPECIFIC_IB_DUMP_FILE_NAME}`, true)
    nextLines = D3dxIniManager.setIniValue(nextLines, 'hunting', 'analyse_options', '')
    await D3dxIniManager.saveIni(iniPath, nextLines)

    return {
      restoreAnalyseOptions: currentAnalyseOptions,
      logicName: resolvedLogicName,
    }
  }

  static async disableSpecificIbDumpMode(options: Pick<SpecificIbDumpOptions, 'gameName' | 'restoreAnalyseOptions'>): Promise<void> {
    const gameConfig = await ResourceManager.loadGameConfig(options.gameName)
    const cfg = gameConfig || {}
    const migotoDir = await this.resolveMigotoDir(options.gameName, cfg)
    const iniPath = await join(migotoDir, 'd3dx.ini')
    const dumpConfigPath = await join(migotoDir, SPECIFIC_IB_DUMP_FILE_NAME)
    if (!(await exists(iniPath))) throw new Error(t('migotoManager.messages.iniNotFoundAt', { path: iniPath }))

    let lines = await D3dxIniManager.loadIni(iniPath)
    lines = D3dxIniManager.setIniValue(lines, 'hunting', 'analyse_options', options.restoreAnalyseOptions || '')
    lines = D3dxIniManager.setSectionLineEnabled(lines, 'include', 'include_recursive = Mods', true)
    lines = D3dxIniManager.setSectionLineEnabled(lines, 'include', `include = ${SPECIFIC_IB_DUMP_FILE_NAME}`, false)
    await D3dxIniManager.saveIni(iniPath, lines)

    if (await exists(dumpConfigPath)) {
      await remove(dumpConfigPath)
    }
  }
}
