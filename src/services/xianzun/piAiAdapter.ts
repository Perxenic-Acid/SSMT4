import type {
  Api,
  AssistantMessage,
  AssistantMessageEvent,
  Context as PiAiContext,
  JsonObject,
  Message as PiAiMessage,
  Model as PiAiModel,
  ProviderStreams,
  SimpleStreamOptions,
  TextContent,
  Tool as PiAiTool,
  ToolCall as PiAiToolCall,
} from '@earendil-works/pi-ai'
import { anthropicMessagesApi } from '@earendil-works/pi-ai/api/anthropic-messages.lazy'
import { openAICompletionsApi } from '@earendil-works/pi-ai/api/openai-completions.lazy'
import { openAIResponsesApi } from '@earendil-works/pi-ai/api/openai-responses.lazy'
import { normalizeContext } from '@earendil-works/pi-ai/utils/transcript'

import type {
  XianZunPiApi,
  XianZunPiAssistantSnapshot,
  XianZunPiMessage,
  XianZunPiStreamOptions,
  XianZunPiStreamResult,
  XianZunPiToolCall,
  XianZunPiUsage,
} from './types'

const API_FACTORIES: Record<XianZunPiApi, () => ProviderStreams> = {
  'openai-completions': openAICompletionsApi,
  'openai-responses': openAIResponsesApi,
  'anthropic-messages': anthropicMessagesApi,
}

const ZERO_COST = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  total: 0,
}

const emptyUsage = () => ({
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: ZERO_COST,
})

export const mapPiAiUsage = (usage: AssistantMessage['usage']): XianZunPiUsage => {
  const cacheMissTokens = usage.input + usage.cacheWrite
  return {
    promptTokens: cacheMissTokens + usage.cacheRead,
    completionTokens: usage.output,
    totalTokens: usage.totalTokens,
    cacheHitTokens: usage.cacheRead,
    cacheMissTokens,
  }
}

const textContent = (text: string) => text ? [{ type: 'text' as const, text }] : []

const piAiBaseUrl = (api: XianZunPiApi, baseUrl: string): string => {
  let value = baseUrl.trim().replace(/[?#].*$/, '').replace(/\/+$/, '')
  if (api === 'openai-completions') {
    value = value.replace(/\/chat\/completions$/i, '')
  } else if (api === 'openai-responses') {
    value = value.replace(/\/responses$/i, '')
  } else {
    value = value.replace(/\/messages$/i, '').replace(/\/v1$/i, '')
  }
  return value
}

const parseToolArguments = (value: string): JsonObject => {
  try {
    const parsed = JSON.parse(value) as unknown
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as JsonObject
      : {}
  } catch {
    return {}
  }
}

const snapshotAssistantMessage = (
  message: AssistantMessage,
  providerIdentity?: string,
): XianZunPiAssistantSnapshot => ({
  role: 'assistant',
  content: message.content.map((block) => {
    if (block.type === 'text') {
      return {
        type: 'text',
        text: block.text,
        ...(block.textSignature ? { textSignature: block.textSignature } : {}),
      }
    }
    if (block.type === 'thinking') {
      return {
        type: 'thinking',
        thinking: block.thinking,
        ...(block.thinkingSignature ? { thinkingSignature: block.thinkingSignature } : {}),
        ...(block.redacted !== undefined ? { redacted: block.redacted } : {}),
      }
    }
    return {
      type: 'toolCall',
      id: block.id,
      name: block.name,
      arguments: block.arguments,
      ...(block.thoughtSignature ? { thoughtSignature: block.thoughtSignature } : {}),
      ...(block.namespace ? { namespace: block.namespace } : {}),
      ...(block.argumentsError ? { argumentsError: block.argumentsError } : {}),
    }
  }),
  api: message.api as XianZunPiApi,
  provider: message.provider,
  ...(providerIdentity ? { providerIdentity } : {}),
  model: message.model,
  ...(message.responseModel ? { responseModel: message.responseModel } : {}),
  ...(message.responseId ? { responseId: message.responseId } : {}),
  ...(message.providerThinkingLevel
    ? { providerThinkingLevel: message.providerThinkingLevel }
    : {}),
  ...(message.thinkingLevel ? { thinkingLevel: message.thinkingLevel } : {}),
  usage: message.usage,
  stopReason: message.stopReason,
  ...(message.errorMessage ? { errorMessage: message.errorMessage } : {}),
  ...(message.rawStopReason ? { rawStopReason: message.rawStopReason } : {}),
  ...(message.endTurn !== undefined ? { endTurn: message.endTurn } : {}),
  timestamp: message.timestamp,
  ...(message.durationMs !== undefined ? { durationMs: message.durationMs } : {}),
})

const toAssistantMessage = (
  message: XianZunPiMessage,
  model: PiAiModel<Api>,
  provider: string,
): PiAiMessage => {
  if (message.assistantMessage) {
    return {
      ...message.assistantMessage,
      content: message.assistantMessage.content as AssistantMessage['content'],
      thinkingLevel: message.assistantMessage.thinkingLevel as AssistantMessage['thinkingLevel'],
    }
  }
  const content: (TextContent | PiAiToolCall)[] = textContent(message.content)
  for (const toolCall of message.toolCalls ?? []) {
    content.push({
      type: 'toolCall',
      id: toolCall.id,
      name: toolCall.name,
      arguments: parseToolArguments(toolCall.arguments),
    })
  }
  return {
    role: 'assistant',
    content,
    api: model.api,
    provider,
    model: model.id,
    usage: emptyUsage(),
    stopReason: message.toolCalls?.length ? 'toolUse' : 'stop',
    timestamp: 0,
  }
}

export const toPiAiContext = (
  messages: XianZunPiMessage[],
  tools: XianZunPiStreamOptions['tools'],
  model: PiAiModel<Api>,
  provider: string,
): PiAiContext => {
  const system = messages
    .filter(message => message.role === 'system')
    .map(message => message.content)
    .filter(Boolean)
    .join('\n\n')
  const history: PiAiMessage[] = []
  for (const message of messages) {
    const hasStructuredContent = message.role === 'assistant'
      ? Boolean(message.toolCalls?.length || message.assistantMessage)
      : message.role === 'tool' && Boolean(message.toolCallId && message.toolName)
    if (!message.content.trim() && !hasStructuredContent) continue
    if (message.role === 'system') continue
    if (message.role === 'assistant') {
      history.push(toAssistantMessage(message, model, provider))
      continue
    }
    if (message.role === 'tool' && message.toolCallId && message.toolName) {
      history.push({
        role: 'toolResult',
        toolCallId: message.toolCallId,
        toolName: message.toolName,
        content: textContent(message.content),
        isError: message.isError === true,
        timestamp: 0,
      })
      continue
    }
    history.push({
      role: 'user',
      content: message.role === 'tool'
        ? `[Tool result]\n${message.content}`
        : message.content,
      timestamp: 0,
    })
  }
  return {
    ...system ? { systemPrompt: system } : {},
    messages: history,
    ...tools?.length
      ? {
          tools: tools.map(tool => ({
            name: tool.function.name,
            description: tool.function.description,
            parameters: tool.function.parameters as PiAiTool['parameters'],
          })),
        }
      : {},
  }
}

export const toTextFallbackMessages = (
  messages: XianZunPiMessage[],
): XianZunPiMessage[] => messages.map((message) => {
  if (message.role === 'tool') {
    return {
      role: 'user',
      content: [
        `[Tool result for ${message.toolName || message.toolCallId || 'unknown'}]`,
        message.content,
      ].join('\n'),
    }
  }
  if (message.role !== 'assistant') return { role: message.role, content: message.content }

  const parts: string[] = []
  if (message.assistantMessage) {
    for (const block of message.assistantMessage.content) {
      if (block.type === 'text' && block.text) {
        parts.push(block.text)
      } else if (block.type === 'thinking' && block.thinking) {
        parts.push(`[Reasoning]\n${block.thinking}`)
      } else if (block.type === 'toolCall') {
        parts.push([
          `[Tool call ${block.id}: ${block.name}]`,
          JSON.stringify(block.arguments),
        ].join('\n'))
      }
    }
  } else {
    if (message.content) parts.push(message.content)
    for (const toolCall of message.toolCalls ?? []) {
      parts.push([
        `[Tool call ${toolCall.id}: ${toolCall.name}]`,
        toolCall.arguments,
      ].join('\n'))
    }
  }
  return {
    role: 'assistant',
    content: parts.join('\n\n'),
  }
})

export const buildPiAiModel = (
  options: Pick<XianZunPiStreamOptions, 'api' | 'baseUrl' | 'model'>,
  provider: string,
): PiAiModel<Api> => ({
  id: options.model.id,
  name: options.model.name,
  api: options.api,
  provider,
  baseUrl: piAiBaseUrl(options.api, options.baseUrl),
  input: options.model.input,
  contextWindow: options.model.contextWindow,
  maxTokens: options.model.maxTokens,
  reasoning: options.model.reasoning,
  thinkingLevelMap: options.model.thinkingLevelMap,
  compat: options.model.compat,
  cost: ZERO_COST,
})

const reasoningLevel = (value: string | undefined): SimpleStreamOptions['reasoning'] => {
  if (value === 'minimal' || value === 'low' || value === 'medium'
    || value === 'high' || value === 'xhigh' || value === 'max') {
    return value
  }
  return undefined
}

const toolNameAt = (event: AssistantMessageEvent): string | undefined => {
  if (event.type !== 'toolcall_start') return undefined
  const block = event.partial.content[event.contentIndex]
  return block?.type === 'toolCall' ? block.name : undefined
}

const toolIdAt = (event: AssistantMessageEvent): string | undefined => {
  if (event.type !== 'toolcall_start') return undefined
  const block = event.partial.content[event.contentIndex]
  return block?.type === 'toolCall' ? block.id : undefined
}

const abortError = (signal: AbortSignal): Error => {
  const reason = signal.reason
  const error = new Error(
    reason instanceof Error
      ? reason.message
      : typeof reason === 'string' && reason.trim()
        ? reason
        : 'Request aborted',
  )
  error.name = 'AbortError'
  return error
}

export const streamXianZunPiAi = async (
  options: XianZunPiStreamOptions,
): Promise<XianZunPiStreamResult> => {
  const factory = API_FACTORIES[options.api]
  if (!factory) throw new Error(`Unsupported XianZun API protocol: ${options.api}`)

  const provider = 'xianzun'
  const model = buildPiAiModel(options, provider)
  const context = normalizeContext(
    toPiAiContext(options.messages, options.tools, model, provider),
  )
  const stream = factory().streamSimple(model, context, {
    apiKey: options.apiKey,
    headers: options.headers,
    signal: options.signal,
    sessionId: options.sessionId,
    reasoning: reasoningLevel(options.reasoningEffort),
    temperature: options.temperature,
    maxTokens: options.maxTokens,
    maxRetries: 0,
    fetch: options.fetchImpl,
  })

  let content = ''
  let reasoning = ''
  let usage: XianZunPiUsage | null = null
  const streamedToolCalls = new Map<number, { id: string; name: string; arguments: string }>()
  let finalMessage: AssistantMessage | null = null

  try {
    for await (const event of stream) {
      if (event.type === 'text_delta') {
        content += event.delta
        options.onChunk({ content: event.delta })
      } else if (event.type === 'thinking_delta') {
        reasoning += event.delta
        options.onChunk({ reasoning: event.delta })
      } else if (event.type === 'toolcall_start') {
        const id = toolIdAt(event) ?? `tool-${event.contentIndex}`
        const name = toolNameAt(event) ?? ''
        streamedToolCalls.set(event.contentIndex, { id, name, arguments: '' })
        options.onChunk({ toolCall: { id, index: event.contentIndex, name } })
      } else if (event.type === 'toolcall_delta') {
        const current = streamedToolCalls.get(event.contentIndex)
        if (current) {
          current.arguments += event.delta
          options.onChunk({
            toolCall: {
              id: current.id,
              index: event.contentIndex,
              name: current.name || undefined,
              argumentsDelta: event.delta,
            },
          })
        }
      } else if (event.type === 'toolcall_end') {
        const current = streamedToolCalls.get(event.contentIndex)
        if (current) {
          current.id = event.toolCall.id
          current.name = event.toolCall.name
          current.arguments = JSON.stringify(event.toolCall.arguments)
        }
      } else if (event.type === 'done') {
        finalMessage = event.message
        usage = mapPiAiUsage(event.message.usage)
      } else if (event.type === 'error') {
        if (options.signal.aborted) throw abortError(options.signal)
        throw new Error(event.error.errorMessage || 'pi-ai provider stream failed')
      }
    }
  } catch (error) {
    if (options.signal.aborted) throw abortError(options.signal)
    throw error
  }

  if (options.signal.aborted) throw abortError(options.signal)
  if (!finalMessage) throw new Error('pi-ai provider stream ended without a terminal event')

  content = finalMessage.content
    .flatMap(block => block.type === 'text' ? [block.text] : [])
    .join('')
  reasoning = finalMessage.content
    .flatMap(block => block.type === 'thinking' ? [block.thinking] : [])
    .join('')

  const finalToolCalls: XianZunPiToolCall[] = []
  finalMessage.content.forEach((block, contentIndex) => {
    if (block.type !== 'toolCall') return
    const streamed = streamedToolCalls.get(contentIndex)
    const toolCall = {
      id: block.id,
      name: block.name,
      arguments: JSON.stringify(block.arguments),
      index: contentIndex,
    }
    finalToolCalls.push(toolCall)
    if (streamed && streamed.arguments !== toolCall.arguments) {
      options.onChunk({
        toolCall: {
          id: block.id,
          index: contentIndex,
          name: block.name,
          argumentsDelta: toolCall.arguments,
        },
      })
    }
  })

  return {
    content,
    reasoning,
    toolCalls: finalToolCalls,
    usage,
    assistantMessage: snapshotAssistantMessage(finalMessage, options.providerIdentity),
  }
}
