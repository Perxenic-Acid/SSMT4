import type {
  AnthropicMessagesCompat,
  ModelInputModality,
  OpenAICompletionsCompat,
  OpenAIResponsesCompat,
  ThinkingLevelMap,
} from '@earendil-works/pi-ai'

export type XianZunPiApi =
  | 'openai-completions'
  | 'openai-responses'
  | 'anthropic-messages'

export type XianZunPiCompat =
  | OpenAICompletionsCompat
  | OpenAIResponsesCompat
  | AnthropicMessagesCompat

export interface XianZunPiToolCallData {
  id: string
  name: string
  arguments: string
}

export type XianZunPiAssistantContent =
  | {
      type: 'text'
      text: string
      textSignature?: string
    }
  | {
      type: 'thinking'
      thinking: string
      thinkingSignature?: string
      redacted?: boolean
    }
  | {
      type: 'toolCall'
      id: string
      name: string
      arguments: unknown
      thoughtSignature?: string
      namespace?: string
      argumentsError?: string
    }

export interface XianZunPiAssistantUsage {
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
  cacheWrite1h?: number
  reasoning?: number
  totalTokens: number
  cost: {
    input: number
    output: number
    cacheRead: number
    cacheWrite: number
    total: number
  }
}

export interface XianZunPiAssistantSnapshot {
  role: 'assistant'
  content: XianZunPiAssistantContent[]
  api: XianZunPiApi
  provider: string
  providerIdentity?: string
  model: string
  responseModel?: string
  responseId?: string
  providerThinkingLevel?: string
  thinkingLevel?: string
  usage: XianZunPiAssistantUsage
  stopReason: 'pending' | 'stop' | 'length' | 'toolUse' | 'error' | 'aborted' | 'deferred'
  errorMessage?: string
  rawStopReason?: string
  endTurn?: boolean
  timestamp: number
  durationMs?: number
}

export interface XianZunPiMessage {
  role: 'system' | 'user' | 'assistant' | 'tool'
  content: string
  assistantMessage?: XianZunPiAssistantSnapshot
  toolCalls?: XianZunPiToolCallData[]
  toolCallId?: string
  toolName?: string
  isError?: boolean
}

export interface XianZunPiTool {
  type: string
  function: {
    name: string
    description: string
    parameters: unknown
  }
}

export interface XianZunPiModel {
  id: string
  name: string
  api: XianZunPiApi
  baseUrl: string
  input: ModelInputModality[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  thinkingLevelMap?: ThinkingLevelMap
  compat?: XianZunPiCompat
}

export interface XianZunPiUsage {
  promptTokens: number
  completionTokens: number
  totalTokens: number
  cacheHitTokens: number
  cacheMissTokens: number
}

export interface XianZunPiToolCall {
  id: string
  name: string
  arguments: string
  index: number
}

export interface XianZunPiStreamResult {
  content: string
  reasoning: string
  toolCalls: XianZunPiToolCall[]
  usage: XianZunPiUsage | null
  assistantMessage?: XianZunPiAssistantSnapshot
}

export interface XianZunPiStreamOptions {
  api: XianZunPiApi
  baseUrl: string
  apiKey?: string
  model: Omit<XianZunPiModel, 'api' | 'baseUrl'>
  messages: XianZunPiMessage[]
  tools?: XianZunPiTool[]
  sessionId?: string
  headers?: Record<string, string>
  reasoningEffort?: string
  temperature?: number
  maxTokens?: number
  signal: AbortSignal
  providerIdentity?: string
  fetchImpl?: typeof globalThis.fetch
  onChunk: (chunk: {
    content?: string
    reasoning?: string
    toolCall?: {
      id?: string
      index: number
      name?: string
      argumentsDelta?: string
    }
  }) => void
}
