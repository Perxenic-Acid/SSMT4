import type { XianZunProvider } from '../../store/XianZunProviders.ts'
import { resolveXianZunProviderModel } from '../../store/XianZunProviders.ts'
import { streamXianZunPiAi, toTextFallbackMessages } from './piAiAdapter.ts'
import type {
  XianZunPiMessage,
  XianZunPiStreamResult,
  XianZunPiTool,
} from './types.ts'

const OPENCODE_USER_AGENT = 'SSMT4-CheeseCat/1.0'

const normalizedProviderEndpoint = (baseUrl: string): string => {
  try {
    const url = new URL(baseUrl.trim())
    url.hash = ''
    return url.toString().replace(/\/+$/, '')
  } catch {
    return baseUrl.trim().replace(/\/+$/, '')
  }
}

export const xianZunProviderIdentity = (provider: XianZunProvider): string =>
  JSON.stringify([
    provider.id,
    provider.api,
    normalizedProviderEndpoint(provider.baseUrl),
  ])

const isOpenCodeEndpoint = (baseUrl: string): boolean => {
  try {
    const hostname = new URL(baseUrl).hostname.toLowerCase()
    return hostname === 'opencode.ai' || hostname.endsWith('.opencode.ai')
  } catch {
    return false
  }
}

const removeHeader = (headers: Record<string, string>, name: string) => {
  const target = name.toLowerCase()
  for (const key of Object.keys(headers)) {
    if (key.toLowerCase() === target) delete headers[key]
  }
}

export const buildProviderTransportHeaders = (
  provider: XianZunProvider,
  sessionId: string,
): Record<string, string> | undefined => {
  const headers: Record<string, string> = { ...(provider.headers ?? {}) }
  if (provider.api === 'anthropic-messages') {
    removeHeader(headers, 'authorization')
    removeHeader(headers, 'x-api-key')
    headers['anthropic-version'] = '2023-06-01'
    headers['anthropic-dangerous-direct-browser-access'] = 'true'
    if (provider.anthropicAuth === 'bearer') {
      headers.Authorization = `Bearer ${provider.apiKey.trim()}`
    }
  }
  if (provider.sendsOpencodeSessionHeader || isOpenCodeEndpoint(provider.baseUrl)) {
    if (sessionId.trim()) headers['x-opencode-session'] = sessionId.trim()
    const hasUserAgent = Object.keys(headers).some(name => name.toLowerCase() === 'user-agent')
    if (!hasUserAgent) headers['User-Agent'] = OPENCODE_USER_AGENT
  }
  return Object.keys(headers).length > 0 ? headers : undefined
}

const isToolCompatibilityError = (error: unknown): boolean => {
  const message = error instanceof Error ? error.message : String(error)
  return /tool|function|unsupported|unknown field|not supported|invalid request/i.test(message)
}

export type XianZunProviderStreamResult = XianZunPiStreamResult & {
  toolMode: 'native' | 'text'
}

export const streamXianZunProvider = async (
  options: {
    provider: XianZunProvider
    model: string
    messages: XianZunPiMessage[]
    tools?: XianZunPiTool[]
    sessionId: string
    reasoningEffort?: string
    temperature?: number
    maxTokens?: number
    signal: AbortSignal
    fetchImpl: typeof globalThis.fetch
    onChunk: Parameters<typeof streamXianZunPiAi>[0]['onChunk']
  },
): Promise<XianZunProviderStreamResult> => {
  const profile = resolveXianZunProviderModel(
    options.provider,
    options.model,
    options.reasoningEffort,
  )
  const headers = buildProviderTransportHeaders(options.provider, options.sessionId)
  const bearerAuthOwnedByHeader = options.provider.api === 'anthropic-messages'
    && options.provider.anthropicAuth === 'bearer'

  const request = (
    tools: typeof options.tools,
    onChunk: typeof options.onChunk,
    messages = options.messages,
  ) =>
    streamXianZunPiAi({
      api: options.provider.api,
      baseUrl: options.provider.baseUrl,
      apiKey: bearerAuthOwnedByHeader ? undefined : options.provider.apiKey,
      model: profile,
      messages,
      tools,
      sessionId: options.sessionId,
      headers,
      reasoningEffort: options.reasoningEffort,
      temperature: options.temperature,
      maxTokens: options.maxTokens,
      signal: options.signal,
      providerIdentity: xianZunProviderIdentity(options.provider),
      fetchImpl: options.fetchImpl,
      onChunk,
    })

  let emitted = false
  let firstError: unknown
  try {
    const result = await request(options.tools, (chunk) => {
      emitted = true
      options.onChunk(chunk)
    })
    return {
      ...result,
      toolMode: options.tools?.length ? 'native' : 'text',
    }
  } catch (error) {
    firstError = error
  }

  if (!emitted && options.tools?.length && isToolCompatibilityError(firstError)) {
    const result = await request(
      undefined,
      options.onChunk,
      toTextFallbackMessages(options.messages),
    )
    return { ...result, toolMode: 'text' }
  }
  throw firstError
}
