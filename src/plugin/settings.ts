import { invoke } from '@tauri-apps/api/core'

export const createPluginSettingsClient = (pluginId: string) => ({
  get: <T>(key: string): Promise<T | null> =>
    invoke<T | null>('get_plugin_setting', { pluginId, key }),
  set: (key: string, value: unknown): Promise<void> =>
    invoke('set_plugin_setting', { pluginId, key, value }),
})
