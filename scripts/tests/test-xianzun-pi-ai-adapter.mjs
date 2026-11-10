import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import test from 'node:test'

import {
  mapPiAiUsage,
  streamXianZunPiAi,
  toTextFallbackMessages,
} from '../../src/services/xianzun/piAiAdapter.ts'

const servers = []

const closeServers = async () => {
  await Promise.all(servers.splice(0).map(server => new Promise(resolve => {
    server.close(resolve)
    server.closeAllConnections()
  })))
}

const mockServer = async ({ events, holdOpen = false, delayMs = 0 }) => {
  const requests = []
  const requestReceived = Promise.withResolvers()
  const responseClosed = Promise.withResolvers()
  const server = createServer((request, response) => {
    const chunks = []
    response.on('close', () => responseClosed.resolve())
    request.on('data', chunk => chunks.push(chunk))
    request.on('end', () => {
      const body = Buffer.concat(chunks).toString('utf8')
      requests.push({
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: body ? JSON.parse(body) : undefined,
      })
      requestReceived.resolve()
      response.writeHead(200, { 'content-type': 'text/event-stream' })
      response.flushHeaders()
      let index = 0
      const writeNext = () => {
        const event = events[index++]
        if (event === undefined) {
          if (!holdOpen) response.end()
          return
        }
        response.write(typeof event === 'string'
          ? `data: ${event}\n\n`
          : `event: ${event.event}\ndata: ${event.data}\n\n`)
        setTimeout(writeNext, delayMs)
      }
      writeNext()
    })
  })
  servers.push(server)
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  return {
    baseUrl: `http://127.0.0.1:${address.port}/v1`,
    requests,
    requestReceived: requestReceived.promise,
    responseClosed: responseClosed.promise,
  }
}

const model = api => ({
  id: 'test-model',
  name: 'Test Model',
  api,
  input: ['text'],
  contextWindow: 8192,
  maxTokens: 2048,
  reasoning: true,
  compat: api === 'openai-completions'
    ? {
        supportsDeveloperRole: false,
        maxTokensField: 'max_tokens',
        thinkingFormat: 'deepseek',
        requiresReasoningContentOnAssistantMessages: true,
      }
    : undefined,
})

const commonOptions = (api, baseUrl) => ({
  api,
  baseUrl,
  apiKey: 'test-key',
  model: model(api),
  messages: [
    { role: 'system', content: 'You are a test.' },
    { role: 'user', content: 'Say hello.' },
  ],
  tools: [{
    type: 'function',
    function: {
      name: 'lookup',
      description: 'Look up a value.',
      parameters: {
        type: 'object',
        properties: { key: { type: 'number' } },
        required: ['key'],
      },
    },
  }],
  providerIdentity: 'provider-a',
  sessionId: 'conversation-1',
  maxTokens: 16,
  headers: { 'x-test-header': 'present' },
  signal: new AbortController().signal,
  onChunk: () => undefined,
})

test.afterEach(closeServers)

test('OpenAI Chat Completions streams text, reasoning and tool arguments', async () => {
  const server = await mockServer({
    events: [
      JSON.stringify({
        choices: [{
          index: 0,
          delta: {
            role: 'assistant',
            content: 'Hello',
            reasoning_content: 'thinking',
            tool_calls: [{
              index: 0,
              id: 'call_1',
              type: 'function',
              function: { name: 'lookup', arguments: '{"key":' },
            }],
          },
          finish_reason: null,
        }],
      }),
      JSON.stringify({
        choices: [{
          index: 0,
          delta: {
            tool_calls: [{
              index: 0,
              function: { arguments: '1}' },
            }],
          },
          finish_reason: null,
        }],
      }),
      JSON.stringify({
        choices: [{ index: 0, delta: {}, finish_reason: 'tool_calls' }],
        usage: { prompt_tokens: 5, completion_tokens: 3, total_tokens: 8 },
      }),
      '[DONE]',
    ],
  })
  const chunks = []
  const result = await streamXianZunPiAi({
    ...commonOptions('openai-completions', server.baseUrl),
    onChunk: chunk => chunks.push(chunk),
  })

  assert.equal(server.requests[0].url, '/v1/chat/completions')
  assert.equal(server.requests[0].headers['x-test-header'], 'present')
  assert.equal(server.requests[0].body.max_tokens, 16)
  assert.equal(result.content, 'Hello')
  assert.equal(result.reasoning, 'thinking')
  assert.equal(result.toolCalls.length, 1)
  assert.equal(result.toolCalls[0].name, 'lookup')
  assert.deepEqual(JSON.parse(result.toolCalls[0].arguments), { key: 1 })
  assert.equal(result.usage.promptTokens, 5)
  assert.equal(result.assistantMessage.providerIdentity, 'provider-a')
  assert.ok(chunks.some(chunk => chunk.content === 'Hello'))
  assert.ok(chunks.some(chunk => chunk.reasoning === 'thinking'))
  assert.ok(chunks.some(chunk => chunk.toolCall?.name === 'lookup'))
})

test('full resource URLs are normalized without duplicating protocol paths', async () => {
  const chatServer = await mockServer({
    events: [
      JSON.stringify({
        choices: [{ index: 0, delta: { content: 'ok' }, finish_reason: 'stop' }],
      }),
      '[DONE]',
    ],
  })
  await streamXianZunPiAi({
    ...commonOptions('openai-completions', `${chatServer.baseUrl}/chat/completions`),
    tools: undefined,
  })
  assert.equal(chatServer.requests[0].url, '/v1/chat/completions')

  const responsesServer = await mockServer({
    events: [
      JSON.stringify({ type: 'response.created', response: { id: 'resp_1' } }),
      JSON.stringify({
        type: 'response.completed',
        response: {
          id: 'resp_1',
          status: 'completed',
          output: [{
            type: 'message',
            id: 'msg_1',
            role: 'assistant',
            content: [{ type: 'output_text', text: 'ok', annotations: [] }],
          }],
          usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
        },
      }),
    ],
  })
  await streamXianZunPiAi({
    ...commonOptions('openai-responses', `${responsesServer.baseUrl}/responses`),
    tools: undefined,
  })
  assert.equal(responsesServer.requests[0].url, '/v1/responses')

  const frame = data => ({ event: data.type, data: JSON.stringify(data) })
  const anthropicServer = await mockServer({
    events: [
      frame({
        type: 'message_start',
        message: {
          id: 'msg_1',
          type: 'message',
          role: 'assistant',
          model: 'test-model',
          content: [],
          stop_reason: null,
          stop_sequence: null,
          usage: { input_tokens: 1, output_tokens: 1 },
        },
      }),
      frame({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }),
      frame({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'ok' } }),
      frame({ type: 'content_block_stop', index: 0 }),
      frame({ type: 'message_delta', delta: { stop_reason: 'end_turn', stop_sequence: null }, usage: { output_tokens: 1 } }),
      frame({ type: 'message_stop' }),
    ],
  })
  await streamXianZunPiAi({
    ...commonOptions('anthropic-messages', `${anthropicServer.baseUrl}/messages`),
    tools: undefined,
  })
  assert.equal(
    new URL(anthropicServer.requests[0].url, 'http://localhost').pathname,
    '/v1/messages',
  )
})

test('structured tool calls and results are sent back with matching IDs', async () => {
  const server = await mockServer({
    events: [
      JSON.stringify({
        choices: [{ index: 0, delta: { content: 'done' }, finish_reason: 'stop' }],
      }),
      '[DONE]',
    ],
  })
  await streamXianZunPiAi({
    ...commonOptions('openai-completions', server.baseUrl),
    tools: undefined,
    messages: [
      { role: 'user', content: 'Run lookup.' },
      {
        role: 'assistant',
        content: '',
        toolCalls: [{
          id: 'call_lookup',
          name: 'lookup',
          arguments: '{"key":1}',
        }],
      },
      {
        role: 'tool',
        content: 'lookup result',
        toolCallId: 'call_lookup',
        toolName: 'lookup',
        isError: false,
      },
    ],
  })

  const messages = server.requests[0].body.messages
  const assistant = messages.find(message => message.role === 'assistant' && message.tool_calls)
  const result = messages.find(message => message.role === 'tool')
  assert.ok(assistant)
  assert.equal(assistant.tool_calls[0].id, 'call_lookup')
  assert.deepEqual(assistant.tool_calls[0].function, {
    name: 'lookup',
    arguments: '{"key":1}',
  })
  assert.ok(result)
  assert.equal(result.tool_call_id, 'call_lookup')
  assert.equal(result.content, 'lookup result')
})

test('native assistant replay preserves thinking signatures and tool call blocks', async () => {
  const server = await mockServer({
    events: [
      JSON.stringify({
        choices: [{ index: 0, delta: { content: 'continued' }, finish_reason: 'stop' }],
      }),
      '[DONE]',
    ],
  })
  await streamXianZunPiAi({
    ...commonOptions('openai-completions', server.baseUrl),
    tools: undefined,
    messages: [
      { role: 'user', content: 'Run lookup.' },
      {
        role: 'assistant',
        content: '',
        assistantMessage: {
          role: 'assistant',
          content: [
            { type: 'thinking', thinking: 'private reasoning', thinkingSignature: 'reasoning_content' },
            {
              type: 'toolCall',
              id: 'call_lookup',
              name: 'lookup',
              arguments: { key: 1 },
            },
          ],
          api: 'openai-completions',
          provider: 'xianzun',
          model: 'test-model',
          usage: {
            input: 5,
            output: 2,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 7,
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
          },
          stopReason: 'toolUse',
          timestamp: 0,
        },
      },
      {
        role: 'tool',
        content: 'lookup result',
        toolCallId: 'call_lookup',
        toolName: 'lookup',
        isError: false,
      },
    ],
  })

  const assistant = server.requests[0].body.messages.find(message => message.role === 'assistant')
  assert.ok(assistant)
  assert.equal(assistant.reasoning_content, 'private reasoning')
  assert.equal(assistant.tool_calls[0].id, 'call_lookup')
})

test('text fallback converts native assistant and tool history into plain text', () => {
  const messages = toTextFallbackMessages([
    { role: 'user', content: 'Run lookup.' },
    {
      role: 'assistant',
      content: 'Checking.',
      toolCalls: [{ id: 'call_lookup', name: 'lookup', arguments: '{"key":1}' }],
      assistantMessage: {
        role: 'assistant',
        content: [
          { type: 'text', text: 'Checking.' },
          { type: 'toolCall', id: 'call_lookup', name: 'lookup', arguments: { key: 1 } },
        ],
        api: 'openai-completions',
        provider: 'xianzun',
        model: 'test-model',
        usage: {
          input: 5,
          output: 2,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 7,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
        stopReason: 'toolUse',
        timestamp: 0,
      },
    },
    {
      role: 'tool',
      content: 'lookup result',
      toolCallId: 'call_lookup',
      toolName: 'lookup',
      isError: false,
    },
  ])

  assert.equal(messages[1].assistantMessage, undefined)
  assert.equal(messages[1].toolCalls, undefined)
  assert.match(messages[1].content, /lookup/)
  assert.equal(messages[2].role, 'user')
  assert.match(messages[2].content, /lookup result/)
})

test('pi-ai usage maps to the UI total prompt and cache semantics', () => {
  const usage = mapPiAiUsage({
    input: 70,
    output: 10,
    cacheRead: 30,
    cacheWrite: 5,
    totalTokens: 115,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  })

  assert.equal(usage.promptTokens, 105)
  assert.equal(usage.completionTokens, 10)
  assert.equal(usage.totalTokens, 115)
  assert.equal(usage.cacheHitTokens, 30)
  assert.equal(usage.cacheMissTokens, 75)
})

test('OpenAI Responses uses the responses endpoint and parses text', async () => {
  const output = [{
    type: 'message',
    id: 'msg_1',
    role: 'assistant',
    content: [{ type: 'output_text', text: 'Response hello', annotations: [] }],
  }]
  const server = await mockServer({
    events: [
      JSON.stringify({ type: 'response.created', response: { id: 'resp_1' } }),
      JSON.stringify({ type: 'response.output_item.added', output_index: 0, item: output[0] }),
      JSON.stringify({ type: 'response.output_item.done', output_index: 0, item: output[0] }),
      JSON.stringify({
        type: 'response.completed',
        response: {
          id: 'resp_1',
          status: 'completed',
          output,
          usage: { input_tokens: 4, output_tokens: 2, total_tokens: 6 },
        },
      }),
    ],
  })
  const result = await streamXianZunPiAi(
    commonOptions('openai-responses', server.baseUrl),
  )

  assert.equal(server.requests[0].url, '/v1/responses')
  assert.equal(server.requests[0].headers['x-test-header'], 'present')
  assert.equal(result.content, 'Response hello')
  assert.equal(result.usage.promptTokens, 4)
})

test('Anthropic Messages uses the native endpoint and parses text', async () => {
  const frame = data => ({ event: data.type, data: JSON.stringify(data) })
  const server = await mockServer({
    events: [
      frame({
        type: 'message_start',
        message: {
          id: 'msg_1',
          type: 'message',
          role: 'assistant',
          model: 'test-model',
          content: [],
          stop_reason: null,
          stop_sequence: null,
          usage: { input_tokens: 3, output_tokens: 1 },
        },
      }),
      frame({ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }),
      frame({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Anthropic hello' } }),
      frame({ type: 'content_block_stop', index: 0 }),
      frame({
        type: 'message_delta',
        delta: { stop_reason: 'end_turn', stop_sequence: null },
        usage: { output_tokens: 2 },
      }),
      frame({ type: 'message_stop' }),
    ],
  })
  const result = await streamXianZunPiAi(
    commonOptions('anthropic-messages', server.baseUrl),
  )

  assert.equal(new URL(server.requests[0].url, 'http://localhost').pathname, '/v1/messages')
  assert.equal(server.requests[0].headers['x-test-header'], 'present')
  assert.equal(result.content, 'Anthropic hello')
  assert.equal(result.usage.promptTokens, 3)
})

test('AbortSignal terminates an in-flight provider stream', async () => {
  const server = await mockServer({
    delayMs: 5000,
    holdOpen: true,
    events: [
      JSON.stringify({
        choices: [{ index: 0, delta: { role: 'assistant', content: 'partial' }, finish_reason: null }],
      }),
    ],
  })
  const controller = new AbortController()
  const pending = streamXianZunPiAi({
    ...commonOptions('openai-completions', server.baseUrl),
    signal: controller.signal,
    onChunk: () => controller.abort(),
  })

  await assert.rejects(pending, error => error?.name === 'AbortError')
  await Promise.race([
    server.responseClosed,
    new Promise((_, reject) => setTimeout(() => reject(new Error('provider stream stayed open')), 2000)),
  ])
})
