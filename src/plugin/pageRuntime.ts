import { invoke } from '@tauri-apps/api/core'

export const PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION = 1
export const PLUGIN_PAGE_BRIDGE_REQUEST_TYPE = 'ssmt:capability-request'
export const PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE = 'ssmt:capability-response'

export type PluginPageKind = 'declarative' | 'static' | 'local-service'
export type DeclarativePageFieldKind = 'boolean' | 'integer' | 'string' | 'select'

export interface DeclarativePageOption {
  value: string
  label: string
}

export interface DeclarativePageField {
  key: string
  label: string
  kind: DeclarativePageFieldKind
  defaultValue?: unknown
  description?: string
  required?: boolean
  min?: number | null
  max?: number | null
  options?: DeclarativePageOption[]
}

export interface PluginPageCatalogEntry {
  pluginId: string
  pluginVersion: string
  pageId: string
  title: string
  kind: PluginPageKind
  description?: string
  pagePath?: string | null
  fields?: DeclarativePageField[]
}

export interface PluginPageServiceInfo {
  sessionId: string
  leaseId: string
  serviceKey: string
  leaseCount: number
  pluginId: string
  pluginVersion: string
  pageId: string
  gameName: string
  running: boolean
  processId: number
  url: string
  pagePath: string
  dataRoot: string
  logPath: string
}

export interface PluginPageServiceStatus {
  serviceKey: string
  leaseCount: number
  pluginId: string
  pluginVersion: string
  pageId: string
  gameName: string
  running: boolean
  processId: number
  url: string
  pagePath: string
  dataRoot: string
  logPath: string
}

export interface StaticPluginPageLease {
  leaseId: string
  pluginId: string
  pluginVersion: string
  gameName: string
  pageId: string
  pagePath: string
  url: string
}

export interface PluginCapabilityResponse {
  requestId?: string | null
  capability: string
  ok: boolean
  value?: unknown
  errorCode?: string
  errorMessage?: string
}

export const loadPluginPageCatalog = async (gameName: string): Promise<PluginPageCatalogEntry[]> =>
  invoke<PluginPageCatalogEntry[]>('plugin_page_catalog', { gameName })

export const acquirePluginPageLease = async (
  pluginId: string,
  pageId: string,
  gameName: string,
): Promise<PluginPageServiceInfo> =>
  invoke<PluginPageServiceInfo>('acquire_plugin_page_lease', {
    pluginId,
    pageId,
    gameName,
  })

export const releasePluginPageLease = async (leaseId: string): Promise<boolean> =>
  invoke<boolean>('release_plugin_page_lease', { leaseId })

export const acquireStaticPluginPageLease = async (
  pluginId: string,
  pageId: string,
  gameName: string,
): Promise<StaticPluginPageLease> =>
  invoke<StaticPluginPageLease>('acquire_static_plugin_page_lease', {
    pluginId,
    pageId,
    gameName,
  })

export const releaseStaticPluginPageLease = async (leaseId: string): Promise<boolean> =>
  invoke<boolean>('release_static_plugin_page_lease', { leaseId })

// Renew the backend lease while a plugin page remains mounted.
export const touchPluginPageLease = async (leaseId: string): Promise<boolean> =>
  invoke<boolean>('touch_plugin_page_lease', { leaseId })

export const stopPluginPageService = async (sessionId: string): Promise<boolean> =>
  invoke<boolean>('stop_plugin_page_service', { sessionId })

export const stopPluginPageServicesForPlugin = async (pluginId: string): Promise<number> =>
  invoke<number>('stop_plugin_page_services_for_plugin', { pluginId })

export const stopAllPluginPageServices = async (): Promise<number> =>
  invoke<number>('stop_all_plugin_page_services')

export const getPluginPageSessionStatus = async (
  sessionId: string,
): Promise<PluginPageServiceInfo | null> =>
  invoke<PluginPageServiceInfo | null>('plugin_page_session_status', { sessionId })

export const getPluginPageServiceStatus = async (
  serviceKey: string,
): Promise<PluginPageServiceStatus | null> =>
  invoke<PluginPageServiceStatus | null>('plugin_page_service_status', { serviceKey })

export const requestPluginCapability = async (
  pluginId: string,
  gameName: string,
  capability: string,
  payload: Record<string, unknown>,
  requestId: string = crypto.randomUUID(),
): Promise<unknown> => {
  const response = await invoke<PluginCapabilityResponse>('plugin_capability_request', {
    pluginId,
    gameName,
    capability,
    payload,
    requestId,
  })
  if (!response.ok) {
    throw new Error(`${response.errorCode || 'capability_error'}: ${response.errorMessage || 'unknown error'}`)
  }
  return response.value
}

export const requestPluginCapabilityForLease = async (
  leaseId: string,
  capability: string,
  payload: Record<string, unknown>,
  requestId: string = crypto.randomUUID(),
): Promise<PluginCapabilityResponse> =>
  invoke<PluginCapabilityResponse>('plugin_capability_request_for_lease', {
    leaseId,
    capability,
    payload,
    requestId,
  })
