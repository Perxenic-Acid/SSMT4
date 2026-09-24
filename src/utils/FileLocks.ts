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

function advice(owner: LockOwner): string {
  switch (owner.name.toLowerCase()) {
    case 'explorer.exe': return t('fileLocks.explorer')
    case 'code.exe': return t('fileLocks.code')
    case 'qq.exe': return t('fileLocks.qq')
    default: return t('fileLocks.generic')
  }
}

function reportContent(report: LockReport, willTerminate: boolean) {
  return h('div', { style: 'max-height:55vh;overflow:auto;overflow-wrap:anywhere;text-align:left' }, [
    ...report.paths.map(path => h('p', path)),
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

async function confirmRelease(report: LockReport): Promise<boolean> {
  const approved = report.owners.filter(owner => owner.canTerminate)
  if (!approved.length) {
    await ElMessageBox.alert(reportContent(report, false), t('fileLocks.diagnosis'), { confirmButtonText: t('fileLocks.ok') })
    return false
  }
  try {
    await ElMessageBox.confirm(reportContent(report, true), t('fileLocks.confirmTitle'), {
      confirmButtonText: t('fileLocks.confirm'), cancelButtonText: t('fileLocks.cancel'), type: 'warning',
      closeOnClickModal: false, distinguishCancelAndClose: true,
    })
  } catch { return false }
  await invoke('terminate_file_lock_owners', { paths: report.paths, approved })
  return true
}

/** Keeps the original operation intact; confirmation never implicitly approves new owners. */
export async function renameModWithLockRecovery(source: string, destination: string, watchRoot: string) {
  const rename = () => invoke<void>('rename_mod_with_retry', { source, destination, watchRoot })
  try {
    await rename()
    return
  } catch (originalError) {
    if (!String(originalError).includes('FILE_LOCKED:')) throw originalError
    const report = await queryFileLocks([source]).catch(error => {
      throw new Error(`${originalError}\n${error}`)
    })
    if (!report.owners.length) {
      report.warnings.unshift(String(originalError))
      await confirmRelease(report)
      throw originalError
    }
    if (!await confirmRelease(report)) throw new Error(t('fileLocks.cancelled'))
    // Retry only the failed rename, not previously completed parent/group mutations.
    await rename()
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
