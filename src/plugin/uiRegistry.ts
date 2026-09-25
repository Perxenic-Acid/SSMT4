import { invoke } from '@tauri-apps/api/core'
import type { AsyncComponentLoader, Component } from 'vue'

export interface PluginUiRouteSnapshot {
  pluginId: string
  pluginVersion: string
  pageId: string
  route: string
  packagePath: string
}

/**
 * UI packages are metadata-first until a signed first-party component is
 * explicitly registered here. Manifest paths are never treated as executable
 * JavaScript or Vue modules.
 */
const firstPartyComponents = new Map<string, AsyncComponentLoader<Component>>()

export const registerFirstPartyPluginComponent = (
  pluginId: string,
  pageId: string,
  loader: AsyncComponentLoader<Component>,
): void => {
  firstPartyComponents.set(`${pluginId}:${pageId}`, loader)
}

export const loadPluginUiRoutes = async (): Promise<PluginUiRouteSnapshot[]> => {
  const routes = await invoke<PluginUiRouteSnapshot[]>('plugin_ui_routes')
  return routes.filter((route) => firstPartyComponents.has(`${route.pluginId}:${route.pageId}`))
}

export const resolveFirstPartyPluginComponent = (
  route: Pick<PluginUiRouteSnapshot, 'pluginId' | 'pageId'>,
): AsyncComponentLoader<Component> | undefined =>
  firstPartyComponents.get(`${route.pluginId}:${route.pageId}`)
