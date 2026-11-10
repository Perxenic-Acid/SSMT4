import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import {
  streamXianZunProvider,
  xianZunProviderIdentity,
} from '../../src/services/xianzun/providerTransport.ts'
import {
  fetchXianZunModels,
  mergeXianZunModels,
} from '../../src/store/XianZunApi.ts'
import { normalizeXianZunProvider } from '../../src/store/XianZunProviders.ts'

const servers = []

const closeServers = async () => {
  await Promise.all(servers.splice(0).map(server => new Promise(resolve => {
    server.close(resolve)
    server.closeAllConnections()
  })))
}

const mockServer = async events => {
  const requests = []
  const server = createServer((request, response) => {
    const chunks = []
    request.on('data', chunk => chunks.push(chunk))
    request.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8')
      requests.push({
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: raw ? JSON.parse(raw) : undefined,
      })
      response.writeHead(200, { 'content-type': 'text/event-stream' })
      response.flushHeaders()
      for (const event of events) {
        response.write(typeof event === 'string'
          ? `data: ${event}\n\n`
          : `event: ${event.event}\ndata: ${event.data}\n\n`)
      }
      response.end()
    })
  })
  servers.push(server)
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  return {
    baseUrl: `http://127.0.0.1:${address.port}/v1`,
    requests,
  }
}

const mockResponseServer = async specs => {
  const requests = []
  let requestIndex = 0
  const server = createServer((request, response) => {
    const chunks = []
    request.on('data', chunk => chunks.push(chunk))
    request.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8')
      requests.push({
        method: request.method,
        url: request.url,
        headers: request.headers,
        body: raw ? JSON.parse(raw) : undefined,
      })
      const spec = specs[Math.min(requestIndex, specs.length - 1)]
      requestIndex += 1
      if (spec.json !== undefined) {
        response.writeHead(spec.status ?? 200, { 'content-type': 'application/json' })
        response.end(JSON.stringify(spec.json))
        return
      }
      response.writeHead(spec.status ?? 200, { 'content-type': 'text/event-stream' })
      response.flushHeaders()
      for (const event of spec.events ?? []) {
        response.write(typeof event === 'string'
          ? `data: ${event}\n\n`
          : `event: ${event.event}\ndata: ${event.data}\n\n`)
      }
      response.end()
    })
  })
  servers.push(server)
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  return {
    baseUrl: `http://127.0.0.1:${address.port}/v1`,
    requests,
  }
}

const makeProvider = (api, baseUrl, anthropicAuth = 'x-api-key') => {
  const provider = normalizeXianZunProvider({
    id: 'provider-1',
    name: 'OpenCode Go',
    protocol: api === 'anthropic-messages' ? 'anthropic' : 'openai',
    api,
    baseUrl,
    apiKey: 'test-key',
    model: 'test-model',
    models: ['test-model'],
    modelProfiles: {
      'test-model': {
        id: 'test-model',
        name: 'Test Model',
        input: ['text'],
        contextWindow: 128000,
        maxTokens: 8192,
        reasoning: true,
        compat: api === 'openai-responses'
          ? { supportsDeveloperRole: false }
          : undefined,
      },
    },
    headers: { 'x-provider-header': 'present' },
    sendsOpencodeSessionHeader: true,
    anthropicAuth,
    createdAt: 1,
    updatedAt: 2,
  })
  assert.ok(provider)
  return provider
}

const baseOptions = provider => ({
  provider,
  model: 'test-model',
  messages: [
    { role: 'system', content: 'You are a test.' },
    { role: 'user', content: 'Hello.' },
  ],
  sessionId: 'conversation-session-1',
  reasoningEffort: 'high',
  signal: new AbortController().signal,
  fetchImpl: globalThis.fetch,
  onChunk: () => undefined,
})

test.afterEach(closeServers)

test('provider transport routes Responses through the configured endpoint and headers', async () => {
  const output = [{
    type: 'message',
    id: 'msg_1',
    role: 'assistant',
    content: [{ type: 'output_text', text: 'responses ok', annotations: [] }],
  }]
  const server = await mockServer([
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
  ])
  const result = await streamXianZunProvider(
    baseOptions(makeProvider('openai-responses', server.baseUrl)),
  )

  assert.equal(server.requests[0].url, '/v1/responses')
  assert.equal(server.requests[0].headers['x-provider-header'], 'present')
  assert.equal(server.requests[0].headers['x-opencode-session'], 'conversation-session-1')
  assert.match(server.requests[0].headers['user-agent'], /SSMT4-CheeseCat/)
  assert.equal(result.content, 'responses ok')
  assert.equal(result.usage.promptTokens, 4)
})

test('provider transport routes Anthropic Messages through the native endpoint', async () => {
  const frame = data => ({ event: data.type, data: JSON.stringify(data) })
  const server = await mockServer([
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
    frame({ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'anthropic ok' } }),
    frame({ type: 'content_block_stop', index: 0 }),
    frame({ type: 'message_delta', delta: { stop_reason: 'end_turn', stop_sequence: null }, usage: { output_tokens: 2 } }),
    frame({ type: 'message_stop' }),
  ])
  const result = await streamXianZunProvider(
    baseOptions(makeProvider('anthropic-messages', server.baseUrl)),
  )

  assert.equal(new URL(server.requests[0].url, 'http://localhost').pathname, '/v1/messages')
  assert.equal(server.requests[0].headers['x-provider-header'], 'present')
  assert.equal(server.requests[0].headers['x-opencode-session'], 'conversation-session-1')
  assert.equal(result.content, 'anthropic ok')
})

test('Anthropic auth mode controls the credential header', async () => {
  const frame = data => ({ event: data.type, data: JSON.stringify(data) })
  const events = [
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
  ]

  const bearerServer = await mockServer(events)
  await streamXianZunProvider({
    ...baseOptions(makeProvider('anthropic-messages', bearerServer.baseUrl, 'bearer')),
  })
  assert.equal(bearerServer.requests[0].headers.authorization, 'Bearer test-key')
  assert.equal(bearerServer.requests[0].headers['x-api-key'], undefined)

  const apiKeyServer = await mockServer(events)
  await streamXianZunProvider({
    ...baseOptions(makeProvider('anthropic-messages', apiKeyServer.baseUrl, 'x-api-key')),
  })
  assert.equal(apiKeyServer.requests[0].headers['x-api-key'], 'test-key')
  assert.equal(apiKeyServer.requests[0].headers.authorization, undefined)
})

test('provider identity changes with the endpoint but not a trailing slash', () => {
  const provider = makeProvider('openai-completions', 'https://gateway-a.example/v1')
  const trailingSlash = { ...provider, baseUrl: 'https://gateway-a.example/v1/' }
  const otherGateway = { ...provider, baseUrl: 'https://gateway-b.example/v1' }

  assert.equal(
    xianZunProviderIdentity(provider),
    xianZunProviderIdentity(trailingSlash),
  )
  assert.notEqual(
    xianZunProviderIdentity(provider),
    xianZunProviderIdentity(otherGateway),
  )
})

test('tool-rejecting Chat Completions gateways fall back to the text tool protocol', async () => {
  const server = await mockResponseServer([
    {
      status: 400,
      json: { error: { message: 'tools are not supported by this endpoint' } },
    },
    {
      events: [
        JSON.stringify({
          choices: [{
            index: 0,
            delta: {
              role: 'assistant',
              content: '```tool_call\n{"command":"lookup","arguments":{"key":1}}\n```',
            },
            finish_reason: 'stop',
          }],
        }),
        '[DONE]',
      ],
    },
  ])
  const provider = makeProvider('openai-completions', server.baseUrl)
  const result = await streamXianZunProvider({
    ...baseOptions(provider),
    messages: [
      { role: 'user', content: 'Run lookup.' },
      {
        role: 'assistant',
        content: '',
        assistantMessage: {
          role: 'assistant',
          content: [{
            type: 'toolCall',
            id: 'previous-call',
            name: 'lookup',
            arguments: { key: 1 },
          }],
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
        content: 'previous result',
        toolCallId: 'previous-call',
        toolName: 'lookup',
        isError: false,
      },
    ],
    tools: [{
      type: 'function',
      function: {
        name: 'lookup',
        description: 'Look up a value.',
        parameters: { type: 'object', properties: { key: { type: 'number' } } },
      },
    }],
  })

  assert.equal(server.requests.length, 2)
  assert.ok(server.requests[0].body.tools)
  assert.equal(server.requests[1].body.tools, undefined)
  assert.equal(server.requests[1].body.messages.some(message => message.role === 'tool'), false)
  assert.equal(
    server.requests[1].body.messages.some(message => Array.isArray(message.tool_calls)),
    false,
  )
  assert.match(JSON.stringify(server.requests[1].body.messages), /previous result/)
  assert.equal(result.toolMode, 'text')
  assert.match(result.content, /tool_call/)
})

test('model discovery forwards custom headers and merges partial results', async () => {
  const server = await mockResponseServer([
    {
      json: {
        data: [
          { id: 'discovered-model' },
        ],
      },
    },
  ])
  const provider = makeProvider('openai-completions', server.baseUrl)
  const models = await fetchXianZunModels({
    protocol: provider.protocol,
    baseUrl: provider.baseUrl,
    apiKey: provider.apiKey,
    auth: provider.anthropicAuth,
    headers: provider.headers,
    fetchImpl: globalThis.fetch,
  })

  assert.equal(server.requests[0].headers['x-provider-header'], 'present')
  assert.equal(server.requests[0].headers.authorization, 'Bearer test-key')
  assert.deepEqual(models.map(model => model.id), ['discovered-model'])
  assert.deepEqual(
    mergeXianZunModels(['preset-model', 'current-model'], ['discovered-model'], 'current-model'),
    ['preset-model', 'current-model', 'discovered-model'],
  )
})

test('Cheese Cat routes chat and approval requests through the provider transport', async () => {
  const source = await readFile(
    new URL('../../src/views/XianZun/XianZun.vue', import.meta.url),
    'utf8',
  )

  assert.match(source, /streamXianZunProvider/)
  assert.match(source, /provider:\s*context\.provider/)
  assert.match(source, /provider,/)
  assert.match(source, /sessionId:\s*sessionId/)
  const turnSource = source.slice(
    source.indexOf('const runAgentTurn = async'),
    source.indexOf('const sendMessage = async'),
  )
  assert.match(
    turnSource,
    /const liveProvider = activeProvider\.value[\s\S]*?snapshotXianZunProvider\(liveProvider\)/,
  )
  assert.match(turnSource, /nativeProtocolMessages/)
  assert.match(turnSource, /assistantMessage/)
  assert.match(source, /xianZunProviderIdentity/)
  assert.match(source, /roundProtocolMessages\.filter\(message => message\.role === 'tool'\)/)
  assert.match(source, /nativeToolCalls/)
  assert.match(source, /toolCallId/)
  assert.match(source, /role:\s*'tool'/)
})

test('test connection performs real inference while model discovery stays separate', async () => {
  const source = await readFile(
    new URL('../../src/views/XianZun/XianZun.vue', import.meta.url),
    'utf8',
  )

  assert.match(source, /const testConnection = async[\s\S]*?streamXianZunProvider/)
  assert.match(source, /真实对话连接成功/)
  assert.match(source, /const refreshModelList = async[\s\S]*?fetchXianZunModels/)
})
