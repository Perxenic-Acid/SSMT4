import { invoke } from '@tauri-apps/api/core'

export const readPluginLog = (pluginId: string): Promise<string> => invoke<string>('read_plugin_log', { pluginId })
export const clearPluginLog = (pluginId: string): Promise<void> => invoke('clear_plugin_log', { pluginId })
