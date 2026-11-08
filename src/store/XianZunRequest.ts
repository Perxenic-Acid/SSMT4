const OPENCODE_HOST = 'opencode.ai'
const CHEESE_CAT_USER_AGENT = 'SSMT4-CheeseCat/1.0'

const isOpenCodeHost = (baseUrl: string): boolean => {
  try {
    const hostname = new URL(baseUrl.trim()).hostname.toLowerCase()
    return hostname === OPENCODE_HOST || hostname.endsWith(`.${OPENCODE_HOST}`)
  } catch {
    return false
  }
}

export const buildXianZunRequestHeaders = (options: {
  apiKey: string
  baseUrl: string
  sessionId?: string
}): Record<string, string> => {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${options.apiKey}`,
  }

  if (!isOpenCodeHost(options.baseUrl)) return headers

  headers['User-Agent'] = CHEESE_CAT_USER_AGENT
  const sessionId = options.sessionId?.trim()
  if (sessionId) headers['x-opencode-session'] = sessionId
  return headers
}
