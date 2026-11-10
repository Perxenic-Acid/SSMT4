import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import {
  createXianZunProvider,
  XIANZUN_PROVIDER_PRESETS,
} from '../../src/store/XianZunProviders.ts'

const catalogPath = new URL('../../src/data/xianzun/cc-switch-presets.json', import.meta.url)

const loadCatalog = async () => JSON.parse(await readFile(catalogPath, 'utf8'))

test('CC Switch catalog imports the complete deduplicated provider set', async () => {
  const catalog = await loadCatalog()

  assert.deepEqual(catalog.stats, {
    providers: 159,
    piProviders: 117,
    piModels: 421,
    extraProviders: 42,
  })
  assert.equal(catalog.presets.length, 159)
  assert.equal(new Set(catalog.presets.map(preset => preset.id)).size, 159)
})

test('CC Switch catalog contains no credentials or secret-shaped fields', async () => {
  const catalog = await loadCatalog()
  const forbidden = /^(apiKey|api_key|token|accessToken|refreshToken|secret)$/i

  const walk = value => {
    if (!value || typeof value !== 'object') return
    for (const [key, child] of Object.entries(value)) {
      assert.equal(forbidden.test(key), false, `unexpected secret field: ${key}`)
      walk(child)
    }
  }
  walk(catalog)
})

test('all supported presets have an endpoint and at least one usable model', async () => {
  const catalog = await loadCatalog()

  for (const preset of catalog.presets) {
    if (preset.unsupportedReason) continue
    const routes = preset.routes?.length ? preset.routes : [preset]
    for (const route of routes) {
      if (route.unsupportedReason) continue
      assert.ok(route.baseUrl, `${preset.name} has no base URL`)
      assert.ok(route.models.length > 0, `${preset.name} has no models`)
      assert.ok(
        ['openai-completions', 'openai-responses', 'anthropic-messages'].includes(route.api),
        `${preset.name} has unsupported API ${route.api}`,
      )
      for (const model of route.models) {
        assert.ok(route.modelProfiles[model], `${preset.name} lacks profile for ${model}`)
      }
    }
  }
})

test('special authentication providers are present but explicitly unavailable', async () => {
  const catalog = await loadCatalog()
  const byName = new Map(catalog.presets.map(preset => [preset.name, preset]))

  for (const name of [
    'AWS Bedrock',
    'AWS Bedrock (AKSK)',
    'AWS Bedrock (API Key)',
    'GitHub Copilot',
    'xAI (Grok) OAuth',
  ]) {
    assert.ok(byName.has(name), `${name} missing`)
    assert.match(byName.get(name).unsupportedReason, /鉴权|OAuth|AWS|Bedrock/i)
  }
})

test('OpenCode Go retains its endpoint, models and session-header capability', async () => {
  const catalog = await loadCatalog()
  const preset = catalog.presets.find(item => item.name === 'OpenCode Go')

  assert.ok(preset)
  assert.equal(preset.unsupportedReason, undefined)
  assert.match(preset.baseUrl, /opencode\.ai/)
  assert.ok(preset.models.includes('deepseek-v4.1-flash'))
  assert.equal(
    preset.sendsOpencodeSessionHeader,
    true,
  )
  assert.ok(preset.routes.some(route =>
    route.api === 'openai-completions'
    && /\/zen\/go\/v1$/.test(route.baseUrl)
  ))
  assert.ok(preset.routes.some(route =>
    route.api === 'anthropic-messages'
    && /\/inference\/go\/anthropic$/.test(route.baseUrl)
  ))
})

test('CC Switch catalog preserves undeclared reasoning capability as unknown', async () => {
  const catalog = await loadCatalog()
  const preset = catalog.presets.find(item => item.name === '88API')
  assert.ok(preset)
  const hermesRoute = preset.routes.find(
    route => route.id === 'cc-switch-88api-hermes-openai-completions-bc58b5fc',
  )
  assert.ok(hermesRoute)
  assert.equal(
    hermesRoute.modelProfiles['claude-opus-5-5'].reasoningKnown,
    false,
  )
})

test('Claude presets infer Anthropic protocol and known roots are normalized', async () => {
  const catalog = await loadCatalog()
  const byName = new Map(catalog.presets.map(preset => [preset.name, preset]))

  const astron = byName.get('Astron Coding Plan')
  assert.ok(astron)
  assert.equal(astron.api, 'anthropic-messages')
  assert.equal(astron.protocol, 'anthropic')

  const openai = byName.get('OpenAI API')
  assert.ok(openai)
  assert.equal(openai.api, 'openai-responses')
  assert.equal(openai.baseUrl, 'https://api.openai.com/v1')

  const foundry = byName.get('Microsoft Foundry')
  assert.ok(foundry)
  assert.match(foundry.unsupportedReason, /RESOURCE_NAME|资源|配置变量/i)
})

test('native Gemini routes are imported as visibly unsupported variants', async () => {
  const catalog = await loadCatalog()
  const openRouter = catalog.presets.find(preset => preset.name === 'OpenRouter')

  assert.ok(openRouter)
  const geminiRoute = openRouter.routes.find(route => route.source === 'gemini')
  assert.ok(geminiRoute)
  assert.equal(geminiRoute.api, 'gemini-native')
  assert.match(geminiRoute.unsupportedReason, /Gemini|协议|原生/i)
})

test('provider factory follows the generated primary route for multi-route presets', () => {
  for (const name of [
    'Compshare',
    'Compshare Coding Plan',
    'XycAi',
    '千问AI平台 Coding Plan',
  ]) {
    const preset = XIANZUN_PROVIDER_PRESETS.find(item => item.name === name)
    assert.ok(preset, `${name} missing`)
    assert.ok(preset.primaryRouteId, `${name} missing primary route id`)
    const primary = preset.variants.find(route => route.id === preset.primaryRouteId)
    assert.ok(primary, `${name} primary route not found`)
    assert.equal(primary.unsupportedReason, undefined)

    const provider = createXianZunProvider(preset)
    assert.equal(provider.baseUrl, primary.baseUrl)
    assert.equal(provider.api, primary.api)
  }

  const unavailable = XIANZUN_PROVIDER_PRESETS.find(item => item.name === 'PatewayAI')
  assert.ok(unavailable)
  assert.match(unavailable.unsupportedReason, /模型目录|暂不支持|模型/)
})

test('CC Switch attribution includes the upstream MIT notice', async () => {
  const notice = await readFile(
    new URL('../../docs/third-party/cc-switch.md', import.meta.url),
    'utf8',
  )

  assert.match(notice, /Copyright \(c\) 2025 Jason Young/)
  assert.match(notice, /Permission is hereby granted/)
  assert.match(notice, /THE SOFTWARE IS PROVIDED "AS IS"/)
})

test('Cheese Cat exposes every imported preset plus the custom source', async () => {
  assert.equal(XIANZUN_PROVIDER_PRESETS.length, 160)
  assert.ok(XIANZUN_PROVIDER_PRESETS.some(preset => preset.name === 'OpenCode Go'))
  assert.ok(XIANZUN_PROVIDER_PRESETS.some(preset => preset.name === 'SiliconFlow'))
  assert.ok(XIANZUN_PROVIDER_PRESETS.some(preset => preset.name === 'GitHub Copilot' && preset.unsupportedReason))
  assert.equal(
    XIANZUN_PROVIDER_PRESETS.at(-1).id,
    'custom',
  )
})
