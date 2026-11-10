import assert from 'node:assert/strict'
import test from 'node:test'

import { buildXianZunRequestHeaders } from '../../src/store/XianZunRequest.ts'
import {
  createXianZunProvider,
  XIANZUN_PROVIDER_PRESETS,
} from '../../src/store/XianZunProviders.ts'

test('OpenCode requests include a stable conversation session header', () => {
  const headers = buildXianZunRequestHeaders({
    apiKey: 'test-key',
    baseUrl: 'https://opencode.ai/zen/go/v1',
    sessionId: 'xz-conversation-1',
  })

  assert.equal(headers.Authorization, 'Bearer test-key')
  assert.equal(headers['x-opencode-session'], 'xz-conversation-1')
  assert.match(headers['User-Agent'], /^SSMT4-CheeseCat\//)
})

test('non-OpenCode providers do not receive OpenCode session headers', () => {
  const headers = buildXianZunRequestHeaders({
    apiKey: 'test-key',
    baseUrl: 'https://api.deepseek.com',
    sessionId: 'xz-conversation-1',
  })

  assert.equal(headers['x-opencode-session'], undefined)
  assert.equal(headers['User-Agent'], undefined)
})

test('OpenCode Go is available as an OpenAI-compatible provider preset', () => {
  const preset = XIANZUN_PROVIDER_PRESETS.find((item) => item.id === 'opencode-go')

  assert.ok(preset)
  assert.equal(preset.name, 'OpenCode Go')
  assert.equal(preset.protocol, 'openai')
  assert.equal(preset.baseUrl, 'https://opencode.ai/zen/go/v1')
  assert.equal(preset.defaultModel, 'deepseek-v4.1-flash')

  const provider = createXianZunProvider(preset)
  assert.equal(provider.presetId, 'opencode-go')
  assert.equal(provider.model, 'deepseek-v4.1-flash')
})
