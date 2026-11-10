import { fetch } from '@tauri-apps/plugin-http'
import type { XianZunAnthropicAuth, XianZunProviderProtocol } from './XianZunProviders'

export interface XianZunModelOption {
  id: string
  label: string
}

type ModelRecord = Record<string, unknown>

const asRecord = (value: unknown): ModelRecord =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as ModelRecord : {}

const asText = (value: unknown): string => typeof value === 'string' ? value.trim() : ''

/**
 * Accept a provider root, /v1 root, or a fully-qualified resource URL.
 * OpenAI-compatible gateways commonly show all three forms in their docs.
 */
export const normalizeXianZunBaseUrl = (baseUrl: string): string => {
  let value = baseUrl.trim().replace(/[?#].*$/, '').replace(/\/+$/, '')
  value = value.replace(/\/(?:chat\/completions|messages|models|responses)$/i, '')
  return value
}

export const xianZunEndpoint = (
  baseUrl: string,
  resource: 'chat/completions' | 'messages' | 'models',
  protocol: XianZunProviderProtocol = 'openai',
): string => {
  const raw = baseUrl.trim().replace(/[?#].*$/, '').replace(/\/+$/, '')
  const hasResourceSuffix = new RegExp(`/${resource.replace('/', '\\/')}$`, 'i').test(raw)
  // Anthropic's public API always scopes Messages/Models under /v1. If a
  // user enters only https://host/messages, retain the existing /v1 behavior.
  if (hasResourceSuffix && (protocol !== 'anthropic' || /\/v\d+(?:beta)?\//i.test(raw))) return raw
  let base = normalizeXianZunBaseUrl(raw)
  if (protocol === 'anthropic' && !/\/v1$/i.test(base)) base += '/v1'
  return `${base}/${resource}`
}

const modelIdFrom = (value: unknown): string => {
  if (typeof value === 'string') return value.trim()
  const record = asRecord(value)
  return asText(record.id) || asText(record.name) || asText(record.model) || asText(record.model_name) || asText(record.slug)
}

const modelLabelFrom = (value: unknown, id: string): string => {
  const record = asRecord(value)
  const label = asText(record.display_name) || asText(record.displayName) || asText(record.name)
  return label && label !== id ? `${label} (${id})` : id
}

export const parseXianZunModelPayload = (payload: unknown): XianZunModelOption[] => {
  const root = asRecord(payload)
  const nestedData = asRecord(root.data)
  const candidates: unknown[] = [
    root.data,
    root.models,
    root.items,
    nestedData.models,
    nestedData.data,
    payload,
  ]
  const values = candidates.find(Array.isArray) as unknown[] | undefined
  if (!values) return []

  const seen = new Set<string>()
  const options: XianZunModelOption[] = []
  for (const value of values) {
    const id = modelIdFrom(value)
    if (!id || seen.has(id)) continue
    seen.add(id)
    options.push({ id, label: modelLabelFrom(value, id) })
  }
  return options
}

const readPayload = async (response: Response): Promise<unknown> => {
  const raw = await response.text().catch(() => '')
  if (!raw) return null
  try {
    return JSON.parse(raw) as unknown
  } catch {
    return raw
  }
}

export const apiErrorMessage = (payload: unknown, fallback: string): string => {
  const root = asRecord(payload)
  const error = root.error
  if (typeof error === 'string' && error.trim()) return error.trim()
  const errorRecord = asRecord(error)
  return asText(errorRecord.message) || asText(root.message) || fallback
}

const removeHeader = (headers: Record<string, string>, name: string) => {
  const target = name.toLowerCase()
  for (const key of Object.keys(headers)) {
    if (key.toLowerCase() === target) delete headers[key]
  }
}

export const buildXianZunRequestHeaders = (options: {
  protocol: XianZunProviderProtocol
  apiKey: string
  auth: XianZunAnthropicAuth
  headers?: Record<string, string>
}): Record<string, string> => {
  const headers: Record<string, string> = { ...(options.headers ?? {}) }
  removeHeader(headers, 'authorization')
  removeHeader(headers, 'x-api-key')
  if (options.protocol === 'anthropic') {
    headers['anthropic-version'] = '2023-06-01'
    headers['anthropic-dangerous-direct-browser-access'] = 'true'
    if (options.auth === 'x-api-key') {
      headers['x-api-key'] = options.apiKey
    } else {
      headers.Authorization = `Bearer ${options.apiKey}`
    }
  } else {
    headers.Authorization = `Bearer ${options.apiKey}`
  }
  return headers
}

export const fetchXianZunModels = async (options: {
  protocol: XianZunProviderProtocol
  baseUrl: string
  apiKey: string
  auth: XianZunAnthropicAuth
  headers?: Record<string, string>
  fetchImpl?: typeof globalThis.fetch
}): Promise<XianZunModelOption[]> => {
  const endpoint = xianZunEndpoint(options.baseUrl, 'models', options.protocol)
  const request = options.fetchImpl ?? fetch
  const response = await request(endpoint, {
    method: 'GET',
    headers: buildXianZunRequestHeaders(options),
    signal: AbortSignal.timeout(15000),
  })
  const payload = await readPayload(response)
  if (!response.ok) throw new Error(`HTTP ${response.status}${payload ? ` - ${apiErrorMessage(payload, String(payload))}` : ''}`)
  return parseXianZunModelPayload(payload)
}

export const mergeXianZunModels = (
  existingModels: string[],
  discoveredModels: string[],
  currentModel = '',
): string[] => {
  const merged = [...existingModels]
  if (currentModel.trim()) merged.push(currentModel.trim())
  merged.push(...discoveredModels)
  return Array.from(new Set(merged.filter(Boolean)))
}
