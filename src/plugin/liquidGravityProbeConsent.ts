import { invoke } from '@tauri-apps/api/core'
import { notifyPluginRegistryChanged } from './pluginRegistryEvents'
import { LIQUID_GRAVITY_PROBE_PLUGIN_ID } from './liquidGravityProbe'

const CONSENT_VERSION = 1
const CONSENT_SETTING_KEY = 'disclaimer_accepted_version'
const CONSENT_STORAGE_KEY = 'ssmt4:liquid-gravity-probe:disclaimer-version'

const normalizeAcceptedVersion = (value: unknown) => {
  const version = typeof value === 'number' ? value : Number(value)
  return Number.isFinite(version) ? version : 0
}

export const hasLiquidGravityProbeConsent = async (): Promise<boolean> => {
  if (normalizeAcceptedVersion(localStorage.getItem(CONSENT_STORAGE_KEY)) >= CONSENT_VERSION) {
    return true
  }
  try {
    const stored = await invoke<unknown>('get_plugin_setting', {
      pluginId: LIQUID_GRAVITY_PROBE_PLUGIN_ID,
      key: CONSENT_SETTING_KEY,
    })
    if (normalizeAcceptedVersion(stored) >= CONSENT_VERSION) {
      localStorage.setItem(CONSENT_STORAGE_KEY, String(CONSENT_VERSION))
      return true
    }
  } catch {
    // The local fallback below keeps the prompt usable if plugin settings are unavailable.
  }
  return false
}

export const acceptLiquidGravityProbeConsent = async (): Promise<void> => {
  localStorage.setItem(CONSENT_STORAGE_KEY, String(CONSENT_VERSION))
  try {
    await invoke('set_plugin_setting', {
      pluginId: LIQUID_GRAVITY_PROBE_PLUGIN_ID,
      key: CONSENT_SETTING_KEY,
      value: CONSENT_VERSION,
    })
  } catch {
    // The accepted state remains valid for this SSMT installation.
  }
}

export const declineLiquidGravityProbeConsent = async (): Promise<void> => {
  localStorage.removeItem(CONSENT_STORAGE_KEY)
  try {
    await invoke('set_plugin_setting', {
      pluginId: LIQUID_GRAVITY_PROBE_PLUGIN_ID,
      key: CONSENT_SETTING_KEY,
      value: 0,
    })
  } catch {
    // A failed write must not prevent the plugin from being disabled.
  }
}

export const setLiquidGravityProbeEnabled = async (
  gameName: string,
  enabled: boolean,
): Promise<void> => {
  await invoke('set_plugin_enabled_for_game', {
    gameName,
    id: LIQUID_GRAVITY_PROBE_PLUGIN_ID,
    enabled,
    gameExecutable: null,
  })
  notifyPluginRegistryChanged()
}
