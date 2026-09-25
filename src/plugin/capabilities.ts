import { invoke } from '@tauri-apps/api/core'

export type PluginCapability =
  | 'filesystem.read'
  | 'filesystem.write'
  | 'game.read'
  | 'plugin.settings'
  | 'launch.start'
  | 'launch.observe'
  | 'process.spawn'
  | 'process.observe'
  | 'network'

export interface PluginCapabilitiesSnapshot {
  pluginId: string
  capabilities: PluginCapability[]
}

/** Capability gate for first-party UI pages. Keep raw Tauri invoke calls in core code. */
export const createPluginCapabilityClient = async (pluginId: string) => {
  const snapshot = await invoke<PluginCapabilitiesSnapshot>('plugin_capabilities', { pluginId })
  const allowed = new Set<PluginCapability>(snapshot.capabilities)

  return {
    pluginId: snapshot.pluginId,
    has: (capability: PluginCapability): boolean => allowed.has(capability),
    require: (capability: PluginCapability): void => {
      if (!allowed.has(capability)) {
        throw new Error(`Plugin ${snapshot.pluginId} lacks capability: ${capability}`)
      }
    },
  }
}
