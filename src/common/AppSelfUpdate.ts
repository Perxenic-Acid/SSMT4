import { ElMessage } from 'element-plus'
import { createApp, ref } from 'vue'
import { AppStateManager } from '../store/AppStateManager'
import { ResourceManager } from '../store/ResourceManager'
import { i18n } from '../i18n'
import { debugWarn } from '../utils/debugLog'
import AppUpdateDialog from '../components/AppUpdateDialog.vue'

type AppUpdateMode = 'silent' | 'manual'

type AppUpdateHandle = {
  version: string
  downloadAndInstall: () => Promise<void>
}

type AppUpdateOutcome =
  | { kind: 'none' }
  | { kind: 'error'; error: unknown }
  | {
      kind: 'available'
      update: AppUpdateHandle
      version: string
      releaseNotes: string
      hasResolvedReleaseNotes: boolean
    }

const APP_RELEASE_NOTES_REPO = 'Perxenic-Acid/SSMT4'
// The updater endpoint is intended for packaged release builds only. Vite's
// production flag is compiled into the bundle, so `bun tauri dev` cannot
// accidentally contact GitHub while release artifacts retain the updater.
const APP_UPDATE_ENABLED = import.meta.env.PROD

const t = i18n.global.t

export const isCheckingAppUpdate = ref(false)
export const isInstallingAppUpdate = ref(false)

let activeCheckPromise: Promise<AppUpdateOutcome> | null = null
let activePromptPromise: Promise<void> | null = null
let startupSilentCheckTriggered = false

const showAppUpdateDialog = (outcome: Extract<AppUpdateOutcome, { kind: 'available' }>): Promise<boolean> => {
  return new Promise((resolve) => {
    const container = document.createElement('div')
    document.body.appendChild(container)

    let settled = false

    const finish = (confirmed: boolean) => {
      if (settled) {
        return
      }

      settled = true
      app.unmount()
      container.remove()
      resolve(confirmed)
    }

    const app = createApp(AppUpdateDialog, {
      title: t('settings.messages.appUpdateConfirmTitle'),
      message: t('settings.messages.appUpdateConfirmContent', { version: outcome.version }),
      releaseNotesTitle: t('settings.messages.appUpdateReleaseNotesTitle'),
      releaseNotes: outcome.releaseNotes,
      confirmText: t('settings.actions.updateNow'),
      cancelText: t('settings.actions.later'),
      onConfirm: () => finish(true),
      onCancel: () => finish(false),
    })

    app.mount(container)
  })
}

const normalizeVersionTag = (version: string): string => {
  return (version || '').trim().replace(/^[^\d]*/, '')
}

const resolveReleaseNotes = async (
  version: string,
): Promise<{ releaseNotes: string; hasResolvedReleaseNotes: boolean }> => {
  try {
    const release = await ResourceManager.getAppLatestRelease(
      AppStateManager.appSettings.githubToken,
      AppStateManager.appSettings.includePrereleaseUpdates,
    )

    if (normalizeVersionTag(release.version) !== normalizeVersionTag(version)) {
      return {
        releaseNotes: t('settings.messages.appUpdateNoReleaseNotes'),
        hasResolvedReleaseNotes: false,
      }
    }

    const description = (release.description || '').trim()
    return {
      releaseNotes: description || t('settings.messages.appUpdateNoReleaseNotes'),
      hasResolvedReleaseNotes: true,
    }
  } catch (error) {
    debugWarn('AppSelfUpdate', `Failed to fetch release notes for ${APP_RELEASE_NOTES_REPO}:`, error)
    return {
      releaseNotes: t('settings.messages.appUpdateNoReleaseNotes'),
      hasResolvedReleaseNotes: false,
    }
  }
}

const performUpdateCheck = async (): Promise<AppUpdateOutcome> => {
  if (activeCheckPromise) {
    return activeCheckPromise
  }

  activeCheckPromise = (async () => {
    isCheckingAppUpdate.value = true
    try {
      const updater = await import('@tauri-apps/plugin-updater')
      const rawUpdate = await updater.check()

      if (!rawUpdate) {
        return { kind: 'none' }
      }

      const update = rawUpdate as AppUpdateHandle
      const { releaseNotes, hasResolvedReleaseNotes } = await resolveReleaseNotes(update.version)

      return {
        kind: 'available',
        update,
        version: update.version,
        releaseNotes,
        hasResolvedReleaseNotes,
      }
    } catch (error) {
      return { kind: 'error', error }
    } finally {
      isCheckingAppUpdate.value = false
      activeCheckPromise = null
    }
  })()

  return activeCheckPromise
}

const promptAndInstallAppUpdate = async (outcome: Extract<AppUpdateOutcome, { kind: 'available' }>): Promise<void> => {
  if (activePromptPromise) {
    return activePromptPromise
  }

  activePromptPromise = (async () => {
    const confirmed = await showAppUpdateDialog(outcome)

    if (!confirmed) {
      return
    }

    isInstallingAppUpdate.value = true
    try {
      await outcome.update.downloadAndInstall()
      ElMessage.success(t('settings.messages.appUpdateInstalledRestart'))
    } catch (error) {
      console.error('Failed to update app', error)
      ElMessage.error(t('settings.messages.appUpdateFailed', { error: String(error) }))
    } finally {
      isInstallingAppUpdate.value = false
    }
  })().finally(() => {
    activePromptPromise = null
  })

  return activePromptPromise
}

export const checkAndInstallAppUpdate = async (mode: AppUpdateMode = 'manual'): Promise<void> => {
  if (!APP_UPDATE_ENABLED) {
    debugWarn('AppSelfUpdate', 'Skipping app update check in a development build')
    return
  }

  if (isInstallingAppUpdate.value) {
    return
  }

  const outcome = await performUpdateCheck()

  if (outcome.kind === 'error') {
    if (mode === 'manual') {
      console.error('Failed to update app', outcome.error)
      ElMessage.error(t('settings.messages.appUpdateFailed', { error: String(outcome.error) }))
    }
    return
  }

  if (outcome.kind === 'none') {
    if (mode === 'manual') {
      ElMessage.success(t('settings.messages.noAppUpdate'))
    }
    return
  }

  if (mode === 'silent' && !outcome.hasResolvedReleaseNotes) {
    return
  }

  await promptAndInstallAppUpdate(outcome)
}

export const triggerSilentStartupAppUpdateCheck = (): void => {
  if (startupSilentCheckTriggered) {
    return
  }

  startupSilentCheckTriggered = true
  void checkAndInstallAppUpdate('silent')
}
