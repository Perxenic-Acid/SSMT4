import { i18n } from '../i18n'
import { h } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElMessageBox } from 'element-plus'

const t = i18n.global.t

interface LockOwner {
  pid: number
  name: string
  executable: string
  started: string
  canTerminate: boolean
  paths?: string[]
}
interface LockReport {
  paths: string[]
  owners: LockOwner[]
  warnings: string[]
}

export const queryFileLocks = (paths: string[]) => invoke<LockReport>('query_file_locks', { paths })

interface RenameContext {
  source: string
  destination: string
  watchRoot: string
}

function isTerminal(owner: LockOwner): boolean {
  const name = (owner.executable || owner.name).split(/[\\/]/).pop()?.toLowerCase() || ''
  return ['pwsh.exe', 'powershell.exe', 'cmd.exe', 'windowsterminal.exe',
    'openconsole.exe', 'conhost.exe', 'bash.exe', 'zsh.exe', 'fish.exe',
    'nu.exe', 'wsl.exe', 'mintty.exe'].includes(name)
}

function advice(owner: LockOwner): string {
  if (isTerminal(owner)) return t('fileLocks.terminal')
  switch (owner.name.toLowerCase()) {
    case 'explorer.exe': return t('fileLocks.explorer')
    case 'code.exe': return t('fileLocks.code')
    case 'qq.exe': return t('fileLocks.qq')
    default: return t('fileLocks.generic')
  }
}

function reportContent(report: LockReport, willTerminate: boolean, rename?: RenameContext) {
  return h('div', { style: 'max-height:55vh;overflow:auto;overflow-wrap:anywhere;text-align:left' }, [
    ...report.paths.map(path => h('p', path)),
    ...(rename ? [
      h('p', t('fileLocks.renameFrom', { path: rename.source })),
      h('p', t('fileLocks.renameTo', { path: rename.destination })),
      ...(report.owners.some(isTerminal) ? [h('p', t('fileLocks.terminalRenameWarning'))] : []),
    ] : []),
    ...report.warnings.map(warning => h('p', { style: 'color:var(--el-color-warning)' }, warning)),
    ...report.owners.map(owner => h('div', { style: 'margin:12px 0' }, [
      h('strong', `${owner.name} (PID ${owner.pid})`),
      h('div', owner.executable || t('fileLocks.unknownPath')),
      ...(owner.paths || []).map(path => h('div', path)),
      h('div', advice(owner)),
      h('div', owner.canTerminate ? (willTerminate ? t('fileLocks.willTerminate') : '') : t('fileLocks.cannotTerminate')),
    ])),
    h('p', report.owners.length
      ? t('fileLocks.holders')
      : t('fileLocks.none')),
    ...(willTerminate ? [h('p', t('fileLocks.lossWarning'))] : []),
  ])
}

async function confirmRelease(report: LockReport, rename?: RenameContext): Promise<boolean> {
  const approved = report.owners.filter(owner => owner.canTerminate)
  if (!approved.length) {
    await ElMessageBox.alert(reportContent(report, false, rename), t('fileLocks.diagnosis'), { confirmButtonText: t('fileLocks.ok') })
    return false
  }
  try {
    await ElMessageBox.confirm(reportContent(report, true, rename), t('fileLocks.confirmTitle'), {
      confirmButtonText: t('fileLocks.confirm'), cancelButtonText: t('fileLocks.cancel'), type: 'warning',
      closeOnClickModal: false, distinguishCancelAndClose: true,
    })
  } catch { return false }
  await invoke('terminate_file_lock_owners', { paths: report.paths, approved })
  return true
}

/** Give shells a chance to leave the directory without terminating their session. */
async function chooseTerminalRecovery(report: LockReport, context: RenameContext): Promise<'retry' | 'terminate'> {
  // LiteralPath plus PowerShell single-quote escaping handles spaces, brackets and apostrophes.
  const command = `Set-Location -LiteralPath '${context.watchRoot.replace(/'/g, "''")}' -ErrorAction Stop\n[Environment]::CurrentDirectory = (Get-Location).ProviderPath`
  try {
    await ElMessageBox.confirm(h('div', [
      reportContent(report, false, context),
      h('p', t('fileLocks.terminalMoveFirst')),
      h('pre', { style: 'white-space:pre-wrap;overflow-wrap:anywhere;user-select:text' }, command),
      h('p', t('fileLocks.terminalOtherShell')),
    ]), t('fileLocks.terminalTitle'), {
      confirmButtonText: t('fileLocks.retryAfterMove'),
      cancelButtonText: t('fileLocks.chooseTerminate'),
      closeOnClickModal: false, distinguishCancelAndClose: true,
      showClose: true, type: 'warning',
    })
    return 'retry'
  } catch (action) {
    // Only the explicitly labelled secondary button proceeds to termination confirmation.
    // Escape and the close icon cancel the Mod operation.
    if (action === 'cancel') return 'terminate'
    throw new Error(t('fileLocks.cancelled'))
  }
}

/** Keeps the original operation intact; confirmation never implicitly approves new owners. */
export async function renameModWithLockRecovery(source: string, destination: string, watchRoot: string) {
  const context = { source, destination, watchRoot }
  const rename = () => invoke<void>('rename_mod_with_retry', context)
  while (true) {
    try {
      await rename()
      return
    } catch (originalError) {
      if (!String(originalError).includes('FILE_LOCKED:')) throw originalError
      let report = await queryFileLocks([source]).catch(error => {
        throw new Error(`${originalError}\n${error}`)
      })
      if (!report.owners.length) {
        report.warnings.unshift(String(originalError))
        await confirmRelease(report, context)
        throw originalError
      }
      if (report.owners.some(isTerminal)) {
        if (await chooseTerminalRecovery(report, context) === 'retry') continue
        // The user may have switched directories or exited while reading the instructions.
        // Retry once before offering to kill anything; otherwise obtain a fresh owner list.
        try { await rename(); return } catch (error) {
          if (!String(error).includes('FILE_LOCKED:')) throw error
        }
        report = await queryFileLocks([source])
      }
      if (!await confirmRelease(report, context)) throw new Error(t('fileLocks.cancelled'))
      // Retry only the failed rename, not previously completed parent/group mutations.
      await rename()
      if (report.owners.some(owner => owner.canTerminate && isTerminal(owner))) {
        // Closing this informational dialog must not turn a completed rename into a failure.
        await ElMessageBox.alert(h('div', [
          h('p', t('fileLocks.terminalRenamed')),
          h('p', t('fileLocks.renameTo', { path: destination })),
        ]), t('fileLocks.afterRename'), { confirmButtonText: t('fileLocks.ok') }).catch(() => {})
      }
      return
    }
  }
}

export async function diagnoseDllLocks(directory: string) {
  const root = directory.replace(/[\\/]+$/, '')
  const paths = ['d3dcompiler_47.dll', 'd3d11.dll'].map(name => `${root}/${name}`)
  const query = async (): Promise<LockReport> => {
    const reports = await Promise.all(paths.map(path => queryFileLocks([path])))
    const owners = new Map<string, LockOwner>()
    for (const report of reports) {
      for (const owner of report.owners) {
        const key = `${owner.pid}:${owner.started}`
        const existing = owners.get(key)
        if (existing) {
          existing.paths!.push(...report.paths)
          existing.canTerminate &&= owner.canTerminate
        } else owners.set(key, { ...owner, paths: [...report.paths] })
      }
    }
    return { paths, owners: [...owners.values()], warnings: reports.flatMap(report => report.warnings) }
  }
  const report = await query()
  if (await confirmRelease(report)) {
    const updated = await query()
    await ElMessageBox.alert(reportContent(updated, false), t('fileLocks.after'), { confirmButtonText: t('fileLocks.ok') })
  }
}
