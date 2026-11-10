import type {
  XianZunPiApi,
  XianZunPiModel,
  XianZunPiTool,
} from '../services/xianzun/types'
import ccSwitchPresetCatalog from '../data/xianzun/cc-switch-presets.json' with { type: 'json' }

export type XianZunProviderProtocol = 'openai' | 'anthropic'
export type XianZunAnthropicAuth = 'bearer' | 'x-api-key'
export type XianZunApiFormat = XianZunPiApi
export type XianZunProviderCompat = Record<string, unknown>

export interface XianZunModelProfile {
  id: string
  name: string
  input: ('text' | 'image')[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  reasoningKnown?: boolean
  compat?: XianZunProviderCompat
  thinkingLevelMap?: Record<string, string | null>
}

export interface XianZunProvider {
  id: string
  name: string
  protocol: XianZunProviderProtocol
  api: XianZunApiFormat
  baseUrl: string
  apiKey: string
  model: string
  models: string[]
  modelProfiles?: Record<string, XianZunModelProfile>
  headers?: Record<string, string>
  compat?: XianZunProviderCompat
  sendsOpencodeSessionHeader?: boolean
  anthropicAuth: XianZunAnthropicAuth
  presetId?: string
  createdAt: number
  updatedAt: number
}

export interface XianZunProviderPreset {
  id: string
  name: string
  group: 'international' | 'china' | 'aggregator' | 'third_party' | 'cloud' | 'local' | 'custom'
  protocol: XianZunProviderProtocol
  api?: XianZunApiFormat
  baseUrl: string
  defaultModel: string
  models: string[]
  modelProfiles?: Record<string, XianZunModelProfile>
  headers?: Record<string, string>
  compat?: XianZunProviderCompat
  unsupportedReason?: string
  sourceUrl?: string
  sendsOpencodeSessionHeader?: boolean
  hint?: string
  primaryRouteId?: string
  variants?: XianZunProviderPresetVariant[]
}

export interface XianZunProviderPresetVariant {
  id: string
  label: string
  source?: string
  protocol: XianZunProviderProtocol
  api?: XianZunApiFormat
  baseUrl: string
  defaultModel: string
  models: string[]
  modelProfiles?: Record<string, XianZunModelProfile>
  headers?: Record<string, string>
  compat?: XianZunProviderCompat
  anthropicAuth?: XianZunAnthropicAuth
  unsupportedReason?: string
  sourceUrl?: string
  sendsOpencodeSessionHeader?: boolean
  hint?: string
}

export const XIANZUN_PROVIDER_GROUP_LABELS: Record<XianZunProviderPreset['group'], string> = {
  international: '国际厂商',
  china: '国产厂商',
  aggregator: '聚合 / 中转',
  third_party: '第三方服务',
  cloud: '云平台',
  local: '本地 / 自托管',
  custom: '自定义',
}

const LEGACY_XIANZUN_PROVIDER_PRESETS: XianZunProviderPreset[] = [
  { id: 'deepseek', name: 'DeepSeek 深度求索', group: 'china', protocol: 'openai', baseUrl: 'https://api.deepseek.com', defaultModel: 'deepseek-v4-flash', models: ['deepseek-v4-flash', 'deepseek-v4-pro', 'deepseek-chat', 'deepseek-reasoner'] },
  { id: 'openai', name: 'OpenAI / ChatGPT', group: 'international', protocol: 'openai', baseUrl: 'https://api.openai.com/v1', defaultModel: 'gpt-5', models: ['gpt-5', 'gpt-5-mini', 'gpt-5-nano', 'gpt-4o', 'gpt-4o-mini', 'o4-mini'] },
  { id: 'anthropic', name: 'Anthropic Claude', group: 'international', protocol: 'anthropic', baseUrl: 'https://api.anthropic.com', defaultModel: 'claude-sonnet-4-5', models: ['claude-sonnet-4-5', 'claude-opus-4-5', 'claude-haiku-4-5'], hint: 'Claude Code 中转站同样适用，Bearer 与 x-api-key 鉴权均可。' },
  { id: 'gemini', name: 'Google Gemini', group: 'international', protocol: 'openai', baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai', defaultModel: 'gemini-2.5-flash', models: ['gemini-2.5-flash', 'gemini-2.5-pro', 'gemini-2.5-flash-lite', 'gemini-2.0-flash'], hint: 'OpenAI 兼容端点，API Key 在 Google AI Studio 获取（gemini-2.5-flash 免费额度充足）。' },
  { id: 'moonshot', name: 'Kimi / Moonshot 月之暗面', group: 'china', protocol: 'openai', baseUrl: 'https://api.moonshot.cn/v1', defaultModel: 'kimi-k2-0905-preview', models: ['kimi-k2-0905-preview', 'kimi-k2-turbo-preview', 'kimi-k2-thinking', 'moonshot-v1-128k'] },
  { id: 'zhipu', name: '智谱 GLM', group: 'china', protocol: 'openai', baseUrl: 'https://open.bigmodel.cn/api/paas/v4', defaultModel: 'glm-4.6', models: ['glm-4.6', 'glm-4.5', 'glm-4.5-air', 'glm-4-flash'] },
  { id: 'qwen', name: '通义千问 / 阿里百炼', group: 'china', protocol: 'openai', baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', defaultModel: 'qwen3-max', models: ['qwen3-max', 'qwen-max', 'qwen-plus', 'qwen-turbo', 'qwq-plus'] },
  { id: 'doubao', name: '豆包 / 火山方舟', group: 'china', protocol: 'openai', baseUrl: 'https://ark.cn-beijing.volces.com/api/v3', defaultModel: 'doubao-seed-1-6-250615', models: ['doubao-seed-1-6-250615', 'doubao-seed-1-6-flash-250828', 'doubao-seed-1-6-thinking-250715'] },
  { id: 'siliconflow', name: 'SiliconFlow 硅基流动', group: 'aggregator', protocol: 'openai', baseUrl: 'https://api.siliconflow.cn/v1', defaultModel: 'deepseek-ai/DeepSeek-V3.2-Exp', models: ['deepseek-ai/DeepSeek-V3.2-Exp', 'Qwen/Qwen3-235B-A22B', 'moonshotai/Kimi-K2-Instruct', 'zai-org/GLM-4.6'] },
  { id: 'qianfan', name: '百度千帆', group: 'china', protocol: 'openai', baseUrl: 'https://qianfan.baidubce.com/v2', defaultModel: 'ernie-4.5-turbo-128k', models: ['ernie-4.5-turbo-128k', 'ernie-x1-turbo-32k', 'deepseek-v3', 'deepseek-r1'] },
  { id: 'minimax', name: 'MiniMax', group: 'china', protocol: 'openai', baseUrl: 'https://api.minimaxi.com/v1', defaultModel: 'MiniMax-M2', models: ['MiniMax-M2', 'MiniMax-Text-01'] },
  { id: 'spark', name: '讯飞星火', group: 'china', protocol: 'openai', baseUrl: 'https://spark-api-open.xf-yun.com/v1', defaultModel: '4.0Ultra', models: ['4.0Ultra', 'generalv3.5', 'max-32k', 'lite'] },
  { id: 'hunyuan', name: '腾讯混元', group: 'china', protocol: 'openai', baseUrl: 'https://api.hunyuan.cloud.tencent.com/v1', defaultModel: 'hunyuan-turbos-latest', models: ['hunyuan-turbos-latest', 'hunyuan-t1-latest', 'hunyuan-lite'] },
  { id: 'stepfun', name: '阶跃星辰 StepFun', group: 'china', protocol: 'openai', baseUrl: 'https://api.stepfun.com/v1', defaultModel: 'step-2-16k', models: ['step-2-16k', 'step-1-128k', 'step-1-8k'] },
  {
    id: 'opencode-go',
    name: 'OpenCode Go',
    group: 'aggregator',
    protocol: 'openai',
    baseUrl: 'https://opencode.ai/inference/go/openai/v1',
    defaultModel: 'deepseek-v4.1-flash',
    models: [
      'deepseek-v4.1-flash',
      'deepseek-v4-pro',
      'glm-5.3-flash',
      'glm-5.3',
      'glm-5.2',
      'kimi-k3.1-pro',
      'kimi-k3',
      'kimi-k2.7-code',
      'longcat-2.0',
      'step-5-preview',
      'mimo-v2.6-flash',
      'mimo-v2.6-pro',
      'minimax-m3.1-flash',
      'minimax-m3',
      'qwen3.8-max',
      'qwen3.8-flash',
      'qwen3.7-plus',
      'hy4',
      'hy4-preview',
      'hy3',
      'grok-4.7',
      'gpt-6-luna',
      'gpt-5.6-luna',
      'claude-haiku-5-5',
      'gemini-4-argon',
      'muse-spark-1.3-contributor',
      'muse-spark-1.2-contributor',
      'crouton-alpha-free',
      'beam-free',
      'iceman-contributor-free',
    ],
    hint: '需要 OpenCode Go 订阅与 API Key；芝士猫会自动发送 OpenCode 会话请求头。',
  },
  { id: 'openrouter', name: 'OpenRouter', group: 'aggregator', protocol: 'openai', baseUrl: 'https://openrouter.ai/api/v1', defaultModel: 'anthropic/claude-sonnet-4.5', models: ['anthropic/claude-sonnet-4.5', 'openai/gpt-5', 'deepseek/deepseek-chat-v3-0324', 'moonshotai/kimi-k2'] },
  { id: 'custom', name: '自定义来源', group: 'custom', protocol: 'openai', baseUrl: '', defaultModel: '', models: [], hint: '支持 OpenAI Chat Completions 或 Anthropic Messages 兼容端点。' },
]

const legacyPresetById = new Map(
  LEGACY_XIANZUN_PROVIDER_PRESETS.map(preset => [preset.id, preset]),
)

const normalizePresetGroup = (value: unknown): XianZunProviderPreset['group'] => {
  if (value === 'international'
    || value === 'china'
    || value === 'aggregator'
    || value === 'third_party'
    || value === 'cloud'
    || value === 'local'
    || value === 'custom') {
    return value
  }
  return 'aggregator'
}

const importedXianZunProviderPresets: XianZunProviderPreset[] =
  ccSwitchPresetCatalog.presets.map(raw => {
    const legacy = legacyPresetById.get(raw.id)
    const api = isApiFormat(raw.api) ? raw.api : undefined
    return {
      ...legacy,
      id: raw.id,
      name: raw.name,
      group: normalizePresetGroup(raw.group),
      protocol: raw.protocol === 'anthropic' ? 'anthropic' : 'openai',
      api,
      baseUrl: raw.baseUrl,
      defaultModel: raw.defaultModel,
      models: [...raw.models],
      modelProfiles: normalizeModelProfiles(raw.modelProfiles),
      headers: recordOfStrings(raw.headers),
      compat: recordOfUnknowns((raw as { compat?: unknown }).compat),
      unsupportedReason: raw.unsupportedReason,
      sourceUrl: raw.sourceUrl,
      sendsOpencodeSessionHeader: raw.sendsOpencodeSessionHeader === true,
      hint: raw.unsupportedReason ?? legacy?.hint,
      primaryRouteId: typeof raw.primaryRouteId === 'string' ? raw.primaryRouteId : undefined,
      variants: Array.isArray((raw as { routes?: unknown }).routes)
        ? ((raw as { routes: XianZunProviderPresetVariant[] }).routes).map((variant): XianZunProviderPresetVariant => ({
            id: variant.id,
            label: variant.label || variant.source || variant.id,
            source: variant.source,
            protocol: variant.protocol === 'anthropic' ? 'anthropic' : 'openai',
            api: isApiFormat(variant.api) ? variant.api : undefined,
            baseUrl: variant.baseUrl,
            defaultModel: variant.defaultModel,
            models: [...variant.models],
            modelProfiles: normalizeModelProfiles(variant.modelProfiles),
            headers: recordOfStrings(variant.headers),
            compat: recordOfUnknowns(variant.compat),
            anthropicAuth: variant.anthropicAuth === 'x-api-key' ? 'x-api-key' : 'bearer',
            unsupportedReason: variant.unsupportedReason,
            sourceUrl: variant.sourceUrl,
            sendsOpencodeSessionHeader: variant.sendsOpencodeSessionHeader === true,
            hint: variant.hint,
          }))
        : undefined,
    }
  })

export const XIANZUN_PROVIDER_PRESETS: XianZunProviderPreset[] = [
  ...importedXianZunProviderPresets,
  {
    id: 'custom',
    name: '自定义来源',
    group: 'custom',
    protocol: 'openai',
    api: 'openai-completions',
    baseUrl: '',
    defaultModel: '',
    models: [],
    hint: '支持 OpenAI Chat Completions、OpenAI Responses 或 Anthropic Messages 兼容端点。',
  },
]

const providerId = (): string => `xz-provider-${Date.now()}-${Math.floor(Math.random() * 1e6)}`

const apiFromProtocol = (protocol: XianZunProviderProtocol): XianZunApiFormat =>
  protocol === 'anthropic' ? 'anthropic-messages' : 'openai-completions'

function isApiFormat(value: unknown): value is XianZunApiFormat {
  return value === 'openai-completions'
    || value === 'openai-responses'
    || value === 'anthropic-messages'
}

function recordOfStrings(value: unknown): Record<string, string> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const entries = Object.entries(value)
    .filter((entry): entry is [string, string] => typeof entry[1] === 'string')
  return entries.length > 0 ? Object.fromEntries(entries) : undefined
}

function recordOfUnknowns(value: unknown): Record<string, unknown> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  return { ...value as Record<string, unknown> }
}

function normalizeModelProfile(raw: unknown, fallbackId = ''): XianZunModelProfile | null {
  if (!raw || typeof raw !== 'object') return null
  const value = raw as Partial<XianZunModelProfile>
  const id = typeof value.id === 'string' && value.id.trim() ? value.id.trim() : fallbackId.trim()
  if (!id) return null
  const input = Array.isArray(value.input)
    ? value.input.filter((item): item is 'text' | 'image' => item === 'text' || item === 'image')
    : []
  const compat = recordOfUnknowns(value.compat)
  const thinkingLevelMap = value.thinkingLevelMap && typeof value.thinkingLevelMap === 'object'
    ? { ...value.thinkingLevelMap }
    : undefined
  return {
    id,
    name: typeof value.name === 'string' && value.name.trim() ? value.name.trim() : id,
    input: input.length > 0 ? Array.from(new Set(input)) : ['text'],
    contextWindow: typeof value.contextWindow === 'number' && value.contextWindow > 0
      ? Math.floor(value.contextWindow)
      : 1_000_000,
    maxTokens: typeof value.maxTokens === 'number' && value.maxTokens > 0
      ? Math.floor(value.maxTokens)
      : 128_000,
    reasoning: value.reasoning === true,
    reasoningKnown: value.reasoningKnown === false
      ? false
      : typeof value.reasoning === 'boolean',
    ...compat ? { compat } : {},
    ...thinkingLevelMap ? { thinkingLevelMap } : {},
  }
}

function normalizeModelProfiles(value: unknown): Record<string, XianZunModelProfile> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const profiles: Record<string, XianZunModelProfile> = {}
  for (const [id, raw] of Object.entries(value)) {
    const profile = normalizeModelProfile(raw, id)
    if (profile) profiles[id] = profile
  }
  return Object.keys(profiles).length > 0 ? profiles : undefined
}

export const createXianZunProvider = (
  preset: XianZunProviderPreset,
  variantId?: string,
): XianZunProvider => {
  const variant = preset.variants?.find(item => item.id === variantId)
    ?? preset.variants?.find(item => item.id === preset.primaryRouteId)
    ?? preset.variants?.[0]
  const route = variant ?? preset
  const now = Date.now()
  return {
    id: providerId(),
    name: preset.name,
    protocol: route.protocol,
    api: route.api ?? apiFromProtocol(route.protocol),
    baseUrl: route.baseUrl,
    apiKey: '',
    model: route.defaultModel,
    models: [...route.models],
    modelProfiles: normalizeModelProfiles(route.modelProfiles),
    headers: route.headers ? { ...route.headers } : undefined,
    compat: route.compat ? { ...route.compat } : undefined,
    sendsOpencodeSessionHeader: route.sendsOpencodeSessionHeader === true,
    anthropicAuth: variant?.anthropicAuth
      ?? (preset.id === 'anthropic' ? 'x-api-key' : 'bearer'),
    presetId: preset.id,
    createdAt: now,
    updatedAt: now,
  }
}

export const normalizeXianZunProvider = (raw: unknown): XianZunProvider | null => {
  if (!raw || typeof raw !== 'object') return null
  const value = raw as Partial<XianZunProvider>
  const now = Date.now()
  const protocol = value.protocol === 'anthropic' ? 'anthropic' : 'openai'
  const api = isApiFormat(value.api) ? value.api : apiFromProtocol(protocol)
  return {
    id: typeof value.id === 'string' && value.id.trim() ? value.id : providerId(),
    name: typeof value.name === 'string' && value.name.trim() ? value.name.trim() : '未命名来源',
    protocol: api === 'anthropic-messages' ? 'anthropic' : protocol,
    api,
    baseUrl: typeof value.baseUrl === 'string' ? value.baseUrl.trim() : '',
    apiKey: typeof value.apiKey === 'string' ? value.apiKey : '',
    model: typeof value.model === 'string' ? value.model.trim() : '',
    models: Array.isArray(value.models) ? value.models.filter((model): model is string => typeof model === 'string' && !!model.trim()) : [],
    modelProfiles: normalizeModelProfiles(value.modelProfiles),
    headers: recordOfStrings(value.headers),
    compat: recordOfUnknowns(value.compat),
    sendsOpencodeSessionHeader: value.sendsOpencodeSessionHeader === true,
    anthropicAuth: value.anthropicAuth === 'x-api-key' ? 'x-api-key' : 'bearer',
    presetId: typeof value.presetId === 'string' ? value.presetId : undefined,
    createdAt: typeof value.createdAt === 'number' ? value.createdAt : now,
    updatedAt: typeof value.updatedAt === 'number' ? value.updatedAt : now,
  }
}

export const migrateLegacyXianZunProvider = (apiUrl: string, apiKey: string, model: string): XianZunProvider => ({
  id: 'xz-provider-legacy',
  name: 'DeepSeek 深度求索',
  protocol: 'openai',
  api: 'openai-completions',
  baseUrl: apiUrl.trim() || 'https://api.deepseek.com/v1',
  apiKey,
  model: model.trim() || 'deepseek-v4-flash',
  models: ['deepseek-v4-flash', 'deepseek-v4-pro', 'deepseek-chat', 'deepseek-reasoner'],
  anthropicAuth: 'bearer',
  presetId: 'deepseek',
  createdAt: Date.now(),
  updatedAt: Date.now(),
})

export const resolveXianZunProviderModel = (
  provider: XianZunProvider,
  modelId: string,
  reasoningEffort?: string,
): XianZunModelProfile => {
  const profile = normalizeModelProfile(provider.modelProfiles?.[modelId], modelId)
    ?? {
      id: modelId,
      name: modelId,
      input: ['text'],
      contextWindow: 128_000,
      maxTokens: 8_192,
      reasoning: false,
      reasoningKnown: false,
    }
  const requestedReasoning = Boolean(reasoningEffort && reasoningEffort !== 'auto')
  return {
    ...profile,
    reasoning: profile.reasoning
      || (profile.reasoningKnown === false && requestedReasoning),
    compat: {
      ...(provider.compat ?? {}),
      ...(profile.compat ?? {}),
    },
  }
}

export const toXianZunPiModel = (
  provider: XianZunProvider,
  modelId: string,
): XianZunPiModel => {
  const profile = resolveXianZunProviderModel(provider, modelId)
  return {
    ...profile,
    api: provider.api,
    baseUrl: provider.baseUrl,
  }
}

export const snapshotXianZunProvider = (provider: XianZunProvider): XianZunProvider =>
  normalizeXianZunProvider(JSON.parse(JSON.stringify(provider))) as XianZunProvider

export type {
  XianZunPiModel,
  XianZunPiTool,
}
