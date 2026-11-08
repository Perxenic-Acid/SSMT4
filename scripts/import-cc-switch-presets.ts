import { createHash } from 'node:crypto'
import { mkdir } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const workspaceRoot = resolve(import.meta.dir, '..')
const sourceRoot = resolve(
  workspaceRoot,
  process.env.CC_SWITCH_SOURCE ?? '../cc-switch-source',
)
const outputPath = resolve(workspaceRoot, 'src/data/xianzun/cc-switch-presets.json')

const moduleSpecs = [
  ['pi', 'src/config/piProviderPresets.ts', 'piProviderPresets'],
  ['claude', 'src/config/claudeProviderPresets.ts', 'providerPresets'],
  ['claudeDesktop', 'src/config/claudeDesktopProviderPresets.ts', 'claudeDesktopProviderPresets'],
  ['codex', 'src/config/codexProviderPresets.ts', 'codexProviderPresets'],
  ['gemini', 'src/config/geminiProviderPresets.ts', 'geminiProviderPresets'],
  ['opencode', 'src/config/opencodeProviderPresets.ts', 'opencodeProviderPresets'],
  ['openclaw', 'src/config/openclawProviderPresets.ts', 'openclawProviderPresets'],
  ['hermes', 'src/config/hermesProviderPresets.ts', 'hermesProviderPresets'],
  ['dsh', 'src/config/dshProviderPresets.ts', 'dshProviderPresets'],
] as const

type LooseRecord = Record<string, any>

const ID_OVERRIDES: Record<string, string> = {
  'DeepSeek': 'deepseek',
  'Kimi': 'moonshot',
  'OpenCode Go': 'opencode-go',
  'OpenRouter': 'openrouter',
  'StepFun': 'stepfun',
  'Zhipu GLM': 'zhipu',
  '千问AI平台': 'qwen',
  'MiniMax': 'minimax',
  'SiliconFlow': 'siliconflow',
}

const DEFAULT_MODEL_OVERRIDES: Record<string, string> = {
  'OpenCode Go': 'deepseek-v4.1-flash',
}

const SPECIAL_AUTH_REASONS: Record<string, string> = {
  'AWS Bedrock': '当前 WebView 运行时不支持 AWS Bedrock SigV4 凭据链。',
  'AWS Bedrock (AKSK)': '需要 AWS Access Key/Secret 与区域签名，当前版本暂不支持。',
  'AWS Bedrock (API Key)': '需要 AWS Bedrock API Key 与区域签名，当前版本暂不支持。',
  'Azure OpenAI': '需要 Azure 资源地址、部署名和 API 版本，当前预设暂不自动配置。',
  'Claude Official': 'Claude 官方订阅预设需要 OAuth 登录，当前版本暂不支持。',
  'Claude Desktop Official': 'Claude Desktop 官方订阅预设需要专用 OAuth，当前版本暂不支持。',
  'Codex': 'Codex 订阅预设需要 ChatGPT OAuth，当前版本暂不支持。',
  'Gemini Native': 'Gemini 原生协议尚未接入，请使用 OpenAI 兼容的 Gemini 预设。',
  'GitHub Copilot': 'GitHub Copilot 需要设备 OAuth，当前版本暂不支持。',
  'Google Official': 'Google 官方 OAuth 登录尚未接入；请使用 API Key 型 Gemini 预设。',
  'Oh My OpenCode': '该预设描述 OpenCode 包装器而非可直接访问的 API 端点。',
  'Oh My OpenCode Slim': '该预设描述 OpenCode 包装器而非可直接访问的 API 端点。',
  'OpenAI Official': 'OpenAI 官方订阅预设需要 ChatGPT OAuth，当前版本暂不支持。',
  'xAI (Grok) OAuth': 'xAI OAuth 登录尚未接入；请使用 API Key 型 xAI (Grok) 预设。',
}

const slugify = (value: string): string => {
  const ascii = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  if (ascii) return ascii
  return createHash('sha1').update(value).digest('hex').slice(0, 12)
}

const stableId = (preset: LooseRecord): string => {
  if (ID_OVERRIDES[preset.name]) return ID_OVERRIDES[preset.name]
  if (preset.providerKey) return `cc-switch-${slugify(String(preset.providerKey).replace(/^cc-switch-/, ''))}`
  return `cc-switch-${slugify(preset.name)}`
}

const protocolFor = (api: string): 'openai' | 'anthropic' =>
  api === 'anthropic-messages' ? 'anthropic' : 'openai'

const groupFor = (category: unknown, name: string): string => {
  if (category === 'cn_official') return 'china'
  if (category === 'aggregator' || category === 'third_party') return 'aggregator'
  if (category === 'cloud_provider') return 'cloud'
  if (category === 'local') return 'local'
  if (/OpenAI|Anthropic|Claude|Google|Gemini|xAI|Mistral|Meta|Nvidia/i.test(name)) {
    return 'international'
  }
  return 'aggregator'
}

const apiFromFormat = (format: unknown, wireApi: string | undefined, source: string): string => {
  if (format === 'anthropic') return 'anthropic-messages'
  if (format === 'openai_responses') return 'openai-responses'
  if (format === 'openai_chat') return 'openai-completions'
  if (wireApi === 'responses') return 'openai-responses'
  if (wireApi === 'chat') return 'openai-completions'
  if (source === 'claude' || source === 'claudeDesktop') return 'anthropic-messages'
  if (source === 'codex') return 'openai-responses'
  if (source === 'gemini') return 'gemini-native'
  return 'openai-completions'
}

const modelProfile = (
  id: string,
  raw: LooseRecord = {},
): LooseRecord => ({
  id,
  name: typeof raw.name === 'string' && raw.name.trim() ? raw.name.trim() : id,
  input: Array.isArray(raw.input) && raw.input.length > 0 ? raw.input : ['text'],
  contextWindow: Number.isFinite(raw.contextWindow) && raw.contextWindow > 0
    ? Math.floor(raw.contextWindow)
    : 1_000_000,
  maxTokens: Number.isFinite(raw.maxTokens) && raw.maxTokens > 0
    ? Math.floor(raw.maxTokens)
    : 128_000,
  reasoning: raw.reasoning === true,
  ...(typeof raw.reasoning === 'boolean' ? {} : { reasoningKnown: false }),
  ...(raw.compat && typeof raw.compat === 'object' ? { compat: { ...raw.compat } } : {}),
  ...(raw.thinkingLevelMap && typeof raw.thinkingLevelMap === 'object'
    ? { thinkingLevelMap: { ...raw.thinkingLevelMap } }
    : {}),
})

const modelProfilesFrom = (preset: LooseRecord): Record<string, LooseRecord> => {
  const settings = preset.settingsConfig ?? {}
  const candidates = [
    settings.models,
    preset.models,
    preset.modelCatalog?.map((row: LooseRecord) => ({ ...row, id: row.model ?? row.id })),
  ]
  const profiles: Record<string, LooseRecord> = {}
  for (const candidate of candidates) {
    if (Array.isArray(candidate)) {
      for (const raw of candidate) {
        const item = raw && typeof raw === 'object' ? raw : {}
        const id = String(item.id ?? item.model ?? item.name ?? '').trim()
        if (id && !profiles[id]) profiles[id] = modelProfile(id, item)
      }
    } else if (candidate && typeof candidate === 'object') {
      for (const [key, raw] of Object.entries(candidate)) {
        const item = raw && typeof raw === 'object' ? raw as LooseRecord : {}
        const id = String(item.id ?? item.model ?? key).trim()
        if (id && !profiles[id]) profiles[id] = modelProfile(id, item)
      }
    }
    if (Object.keys(profiles).length > 0) break
  }
  return profiles
}

const inferredModel = (preset: LooseRecord, profiles: Record<string, LooseRecord>): string => {
  const settings = preset.settingsConfig ?? {}
  return String(
    settings.models?.[0]?.id
    ?? settings.models?.[0]?.model
    ?? settings.env?.ANTHROPIC_MODEL
    ?? settings.env?.GEMINI_MODEL
    ?? settings.env?.OPENAI_MODEL
    ?? preset.model
    ?? Object.keys(profiles)[0]
    ?? '',
  ).trim()
}

const inferredBaseUrl = (preset: LooseRecord): string => {
  const settings = preset.settingsConfig ?? {}
  const codexConfig = typeof preset.config === 'string' ? preset.config : ''
  return String(
    settings.env?.ANTHROPIC_BASE_URL
    ?? settings.env?.GOOGLE_GEMINI_BASE_URL
    ?? settings.env?.OPENAI_BASE_URL
    ?? settings.options?.baseURL
    ?? settings.provider?.base_url
    ?? settings.base_url
    ?? settings.baseUrl
    ?? preset.baseUrl
    ?? preset.baseURL
    ?? codexConfig.match(/base_url\s*=\s*"([^"]+)"/)?.[1]
    ?? '',
  ).trim()
}

const inferredWireApi = (preset: LooseRecord): string | undefined => {
  if (typeof preset.apiFormat === 'string') return preset.apiFormat
  const codexConfig = typeof preset.config === 'string' ? preset.config : ''
  return codexConfig.match(/wire_api\s*=\s*"([^"]+)"/)?.[1]
}

const normalizeBaseUrlForApi = (api: string, baseUrl: string): string => {
  const value = baseUrl.trim().replace(/[?#].*$/, '').replace(/\/+$/, '')
  if (!value) return value
  try {
    const url = new URL(value)
    if (api === 'openai-responses'
      && url.hostname === 'api.openai.com'
      && (url.pathname === '' || url.pathname === '/')) {
      url.pathname = '/v1'
      return url.toString().replace(/\/$/, '')
    }
  } catch {
    return value
  }
  return value
}

const anthropicAuthFor = (preset: LooseRecord): 'bearer' | 'x-api-key' | undefined => {
  const env = preset.settingsConfig?.env ?? {}
  if (env.ANTHROPIC_API_KEY !== undefined || preset.apiKeyField === 'ANTHROPIC_API_KEY') {
    return 'x-api-key'
  }
  if (env.ANTHROPIC_AUTH_TOKEN !== undefined || preset.apiKeyField === 'ANTHROPIC_AUTH_TOKEN') {
    return 'bearer'
  }
  try {
    return new URL(inferredBaseUrl(preset)).hostname === 'api.anthropic.com'
      ? 'x-api-key'
      : undefined
  } catch {
    return undefined
  }
}

const unsupportedReasonFor = (
  preset: LooseRecord,
  api: string,
  baseUrl: string,
  models: string[],
): string | undefined => {
  if (SPECIAL_AUTH_REASONS[preset.name]) return SPECIAL_AUTH_REASONS[preset.name]
  if (preset.requiresOAuth || /oauth|copilot/i.test(String(preset.providerType ?? ''))) {
    return '该预设需要 OAuth 或专用托管账号登录，当前版本暂不支持。'
  }
  if (api === 'bedrock-converse-stream') {
    return '当前 WebView 运行时不支持 AWS Bedrock。'
  }
  if (!['openai-completions', 'openai-responses', 'anthropic-messages'].includes(api)) {
    return `协议 ${api || '(未声明)'} 尚未接入。`
  }
  if (!baseUrl || /{{|}}|\$\{|\bYOUR_[A-Z0-9_]+\b|<YOUR_[A-Z0-9_]+>/i.test(baseUrl)) {
    return '该预设需要供应商专用配置变量，暂不能直接创建 API Key 路由。'
  }
  if (models.length === 0) return '该预设没有可直接使用的模型目录。'
  return undefined
}

const convertPreset = (preset: LooseRecord, source: string): LooseRecord => {
  const profiles = modelProfilesFrom(preset)
  const defaultModel = DEFAULT_MODEL_OVERRIDES[String(preset.name)] ?? inferredModel(preset, profiles)
  if (defaultModel && !profiles[defaultModel]) profiles[defaultModel] = modelProfile(defaultModel)
  const models = Object.keys(profiles)
  const api = String(
    preset.settingsConfig?.api
    ?? apiFromFormat(preset.apiFormat, inferredWireApi(preset), source),
  )
  const baseUrl = normalizeBaseUrlForApi(api, inferredBaseUrl(preset))
  const unsupportedReason = unsupportedReasonFor(preset, api, baseUrl, models)
  const anthropicAuth = api === 'anthropic-messages' ? anthropicAuthFor(preset) : undefined
  return {
    id: stableId(preset),
    name: String(preset.name),
    group: groupFor(preset.category, String(preset.name)),
    protocol: protocolFor(api),
    api,
    baseUrl,
    defaultModel: defaultModel || models[0] || '',
    models,
    modelProfiles: profiles,
    ...(preset.settingsConfig?.headers && typeof preset.settingsConfig.headers === 'object'
      ? { headers: { ...preset.settingsConfig.headers } }
      : {}),
    ...(preset.settingsConfig?.compat && typeof preset.settingsConfig.compat === 'object'
      ? { compat: { ...preset.settingsConfig.compat } }
      : {}),
    ...(preset.websiteUrl ? { sourceUrl: preset.websiteUrl } : {}),
    ...(anthropicAuth ? { anthropicAuth } : {}),
    ...(unsupportedReason ? { unsupportedReason } : {}),
    ...(/opencode\.ai/i.test(baseUrl) ? { sendsOpencodeSessionHeader: true } : {}),
    source,
  }
}

const routeVariant = (route: LooseRecord): LooseRecord => {
  const routeHash = createHash('sha1')
    .update(`${route.source}|${route.api}|${route.baseUrl}|${route.anthropicAuth ?? ''}`)
    .digest('hex')
    .slice(0, 8)
  const sourceLabels: Record<string, string> = {
    pi: 'Pi',
    claude: 'Claude Code',
    claudeDesktop: 'Claude Desktop',
    codex: 'Codex',
    gemini: 'Gemini',
    opencode: 'OpenCode',
    openclaw: 'OpenClaw',
    hermes: 'Hermes',
    dsh: 'DSH',
  }
  const apiLabels: Record<string, string> = {
    'openai-completions': 'OpenAI Chat',
    'openai-responses': 'OpenAI Responses',
    'anthropic-messages': 'Anthropic Messages',
  }
  return {
    id: `${route.id}-${slugify(String(route.source || 'route'))}-${slugify(String(route.api))}-${routeHash}`,
    label: `${sourceLabels[String(route.source)] ?? route.source} · ${apiLabels[String(route.api)] ?? route.api}`,
    source: route.source,
    protocol: route.protocol,
    api: route.api,
    baseUrl: route.baseUrl,
    defaultModel: route.defaultModel,
    models: route.models,
    modelProfiles: route.modelProfiles,
    ...(route.headers ? { headers: route.headers } : {}),
    ...(route.compat ? { compat: route.compat } : {}),
    ...(route.anthropicAuth ? { anthropicAuth: route.anthropicAuth } : {}),
    ...(route.unsupportedReason ? { unsupportedReason: route.unsupportedReason } : {}),
    ...(route.sourceUrl ? { sourceUrl: route.sourceUrl } : {}),
    ...(route.sendsOpencodeSessionHeader ? { sendsOpencodeSessionHeader: true } : {}),
  }
}

const mergePresetRoutes = (routes: LooseRecord[]): LooseRecord => {
  const unique: LooseRecord[] = []
  const seenRoutes = new Set<string>()
  for (const route of routes) {
    const key = `${route.api}|${route.baseUrl}|${route.anthropicAuth ?? ''}`
    if (seenRoutes.has(key)) continue
    seenRoutes.add(key)
    unique.push(route)
  }
  const primary = unique.find(route => !route.unsupportedReason) ?? unique[0]
  const variants = unique.map(routeVariant)
  const primaryRouteId = variants[unique.indexOf(primary)]?.id
  return {
    ...primary,
    ...(primaryRouteId ? { primaryRouteId } : {}),
    routes: variants,
  }
}

const gitHead = Bun.spawnSync(['git', '-C', sourceRoot, 'rev-parse', 'HEAD'])
if (gitHead.exitCode !== 0) throw new Error('Cannot resolve CC Switch Git commit')
const sourceCommit = gitHead.stdout.toString().trim()

const importedGroups = new Map<string, LooseRecord[]>()
for (const [source, relativePath, exportName] of moduleSpecs) {
  const module = await import(pathToFileURL(resolve(sourceRoot, relativePath)).href) as LooseRecord
  const presets = Array.isArray(module[exportName]) ? module[exportName] as LooseRecord[] : []
  for (const preset of presets) {
    const key = String(preset.name).trim().toLowerCase()
    if (!key) continue
    const converted = convertPreset(preset, source)
    const group = importedGroups.get(key)
    if (group) group.push(converted)
    else importedGroups.set(key, [converted])
  }
}

const imported = Array.from(importedGroups.values()).map(mergePresetRoutes)
imported.sort((left, right) => left.name.localeCompare(right.name, 'zh-CN'))
const catalog = {
  source: {
    repository: 'https://github.com/farion1231/cc-switch',
    commit: sourceCommit,
    license: 'MIT',
    generator: 'scripts/import-cc-switch-presets.ts',
  },
  stats: {
    providers: imported.length,
    piProviders: 117,
    piModels: 421,
    extraProviders: imported.length - 117,
  },
  presets: imported,
}

await mkdir(dirname(outputPath), { recursive: true })
await Bun.write(outputPath, `${JSON.stringify(catalog, null, 2)}\n`)
console.log(`Wrote ${imported.length} presets to ${outputPath}`)
