import assert from 'node:assert/strict'
import test from 'node:test'

import {
  createXianZunProvider,
  migrateLegacyXianZunProvider,
  normalizeXianZunProvider,
  resolveXianZunProviderModel,
  snapshotXianZunProvider,
  XIANZUN_PROVIDER_PRESETS,
} from '../../src/store/XianZunProviders.ts'

const baseProvider = {
  id: 'provider-1',
  name: 'Example',
  baseUrl: 'https://example.test/v1',
  apiKey: 'secret',
  model: 'model-1',
  models: ['model-1', 'model-2'],
  createdAt: 1,
  updatedAt: 2,
}

test('legacy OpenAI providers migrate to openai-completions', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'openai',
    anthropicAuth: 'bearer',
  })

  assert.ok(provider)
  assert.equal(provider.api, 'openai-completions')
  assert.equal(provider.protocol, 'openai')
  assert.equal(provider.apiKey, 'secret')
  assert.deepEqual(provider.models, ['model-1', 'model-2'])
})

test('legacy Anthropic providers migrate to anthropic-messages', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'anthropic',
    anthropicAuth: 'x-api-key',
  })

  assert.ok(provider)
  assert.equal(provider.api, 'anthropic-messages')
  assert.equal(provider.protocol, 'anthropic')
  assert.equal(provider.anthropicAuth, 'x-api-key')
})

test('existing OpenCode configuration is preserved during normalization', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    name: 'OpenCode Go',
    baseUrl: 'https://opencode.ai/zen/go/v1',
    model: 'deepseek-v4.1-flash',
    models: ['deepseek-v4.1-flash'],
    protocol: 'openai',
    anthropicAuth: 'bearer',
    presetId: 'opencode-go',
  })

  assert.ok(provider)
  assert.equal(provider.name, 'OpenCode Go')
  assert.equal(provider.baseUrl, 'https://opencode.ai/zen/go/v1')
  assert.equal(provider.model, 'deepseek-v4.1-flash')
  assert.equal(provider.presetId, 'opencode-go')
})

test('new provider profiles retain protocol, headers, compat and model metadata', () => {
  const provider = createXianZunProvider({
    id: 'example-v2',
    name: 'Example V2',
    group: 'aggregator',
    protocol: 'openai',
    api: 'openai-responses',
    baseUrl: 'https://example.test/v1',
    defaultModel: 'model-1',
    models: ['model-1'],
    headers: { 'x-provider': 'value' },
    compat: { supportsDeveloperRole: false },
    modelProfiles: {
      'model-1': {
        id: 'model-1',
        name: 'Model One',
        input: ['text', 'image'],
        contextWindow: 128000,
        maxTokens: 8192,
        reasoning: true,
        compat: { maxTokensField: 'max_tokens' },
      },
    },
  })

  assert.equal(provider.api, 'openai-responses')
  assert.equal(provider.protocol, 'openai')
  assert.deepEqual(provider.headers, { 'x-provider': 'value' })
  assert.equal(provider.modelProfiles['model-1'].contextWindow, 128000)

  const normalized = normalizeXianZunProvider({
    ...provider,
    createdAt: 1,
    updatedAt: 2,
  })
  assert.ok(normalized)
  assert.equal(normalized.api, 'openai-responses')
  assert.equal(normalized.headers['x-provider'], 'value')
  assert.equal(normalized.modelProfiles['model-1'].name, 'Model One')
  assert.deepEqual(JSON.parse(JSON.stringify(normalized)), normalized)
})

test('resolved models merge provider and model compatibility defaults', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'openai',
    api: 'openai-completions',
    compat: {
      supportsDeveloperRole: false,
      maxTokensField: 'max_tokens',
    },
    modelProfiles: {
      'model-1': {
        id: 'model-1',
        name: 'Model One',
        input: ['text'],
        contextWindow: 64000,
        maxTokens: 4096,
        reasoning: true,
        compat: {
          thinkingFormat: 'deepseek',
          maxTokensField: 'max_tokens',
        },
      },
    },
  })
  assert.ok(provider)

  const resolved = resolveXianZunProviderModel(provider, 'model-1')
  assert.equal(resolved.id, 'model-1')
  assert.equal(resolved.name, 'Model One')
  assert.equal(resolved.contextWindow, 64000)
  assert.equal(resolved.compat.supportsDeveloperRole, false)
  assert.equal(resolved.compat.thinkingFormat, 'deepseek')
  assert.equal(resolved.compat.maxTokensField, 'max_tokens')
})

test('unknown models use conservative request defaults', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'openai',
  })
  assert.ok(provider)

  const resolved = resolveXianZunProviderModel(provider, 'unknown-model')
  assert.equal(resolved.contextWindow, 128000)
  assert.equal(resolved.maxTokens, 8192)
  assert.equal(resolved.reasoning, false)
  assert.equal(
    resolveXianZunProviderModel(provider, 'unknown-model', 'high').reasoning,
    true,
  )
})

test('known non-reasoning models still drop an explicit reasoning effort', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'openai',
    modelProfiles: {
      'model-1': {
        id: 'model-1',
        name: 'Model One',
        input: ['text'],
        contextWindow: 64000,
        maxTokens: 4096,
        reasoning: false,
      },
    },
  })
  assert.ok(provider)

  assert.equal(
    resolveXianZunProviderModel(provider, 'model-1', 'high').reasoning,
    false,
  )
})

test('unknown reasoning capability survives provider snapshot normalization', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'openai',
    modelProfiles: {
      'model-1': {
        id: 'model-1',
        name: 'Model One',
        input: ['text'],
        contextWindow: 64000,
        maxTokens: 4096,
        reasoning: false,
        reasoningKnown: false,
      },
    },
  })
  assert.ok(provider)

  const snapshot = snapshotXianZunProvider(provider)
  assert.equal(
    resolveXianZunProviderModel(snapshot, 'model-1', 'high').reasoning,
    true,
  )
})

test('provider snapshots isolate an in-flight turn from later edits', () => {
  const provider = normalizeXianZunProvider({
    ...baseProvider,
    protocol: 'anthropic',
    anthropicAuth: 'bearer',
    headers: { 'x-provider': 'original' },
    modelProfiles: {
      'model-1': {
        id: 'model-1',
        name: 'Model One',
        input: ['text'],
        contextWindow: 64000,
        maxTokens: 4096,
        reasoning: false,
        compat: { maxTokensField: 'max_tokens' },
      },
    },
  })
  assert.ok(provider)

  const snapshot = snapshotXianZunProvider(provider)
  provider.baseUrl = 'https://attacker.invalid/v1'
  provider.headers['x-provider'] = 'changed'
  provider.modelProfiles['model-1'].compat.maxTokensField = 'max_completion_tokens'

  assert.equal(snapshot.baseUrl, 'https://example.test/v1')
  assert.equal(snapshot.headers['x-provider'], 'original')
  assert.equal(snapshot.modelProfiles['model-1'].compat.maxTokensField, 'max_tokens')
})

test('flat legacy Cheese Cat configurations migrate through the shared loader', () => {
  const provider = migrateLegacyXianZunProvider(
    'https://legacy.example/v1',
    'legacy-secret',
    'legacy-model',
  )

  assert.equal(provider.api, 'openai-completions')
  assert.equal(provider.baseUrl, 'https://legacy.example/v1')
  assert.equal(provider.apiKey, 'legacy-secret')
  assert.equal(provider.model, 'legacy-model')
})

test('selecting a provider route creates the matching protocol and auth mode', () => {
  const preset = XIANZUN_PROVIDER_PRESETS.find(item => item.name === 'OpenCode Go')
  assert.ok(preset)
  const anthropicRoute = preset.variants.find(route => route.api === 'anthropic-messages')
  assert.ok(anthropicRoute)

  const provider = createXianZunProvider(preset, anthropicRoute.id)
  assert.equal(provider.api, 'anthropic-messages')
  assert.equal(provider.protocol, 'anthropic')
  assert.equal(provider.baseUrl, anthropicRoute.baseUrl)
  assert.equal(provider.anthropicAuth, anthropicRoute.anthropicAuth)
})
