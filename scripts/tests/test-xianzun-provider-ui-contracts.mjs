import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const viewPath = new URL('../../src/views/XianZun/XianZun.vue', import.meta.url)
const providersPath = new URL('../../src/store/XianZunProviders.ts', import.meta.url)

test('provider chooser is searchable and prevents adding unavailable presets', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(source, /v-model="selectedProviderPreset"[\s\S]*?filterable/)
  assert.match(source, /:disabled="!!selectedProviderPresetInfo\?\.unsupportedReason"/)
  assert.match(source, /preset\.unsupportedReason/)
})

test('the API protocol control exposes all three supported protocols', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(source, /OpenAI Chat Completions/)
  assert.match(source, /OpenAI Responses/)
  assert.match(source, /Anthropic Messages/)
  assert.match(source, /v-model="activeProvider\.api"/)
})

test('advanced provider headers and compatibility are editable', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(source, /请求头 JSON/)
  assert.match(source, /兼容参数 JSON/)
  assert.match(source, /v-model="headersJsonDraft"/)
  assert.match(source, /v-model="compatJsonDraft"/)
  assert.match(source, /applyAdvancedProviderConfig/)
})

test('selected model capabilities are visible in provider settings', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(source, /activeModelProfile/)
  assert.match(source, /上下文/)
  assert.match(source, /最大输出/)
  assert.match(source, /输入类型/)
})

test('provider group labels cover CC Switch cloud, third-party and local groups', async () => {
  const source = await readFile(providersPath, 'utf8')

  assert.match(source, /third_party: '第三方服务'/)
  assert.match(source, /cloud: '云平台'/)
  assert.match(source, /local: '本地 \/ 自托管'/)
})

test('provider variants remain reachable from the preset picker', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(source, /selectedProviderVariantId/)
  assert.match(source, /providerPresetRoutes/)
  assert.match(source, /v-model="selectedProviderVariantId"/)
})

test('narrow Cheese Cat headers stack controls without squeezing brand text', async () => {
  const source = await readFile(viewPath, 'utf8')
  const responsiveStart = source.indexOf('@media (max-width: 1280px)')

  assert.notEqual(responsiveStart, -1, 'missing narrow header breakpoint')
  const responsive = source.slice(responsiveStart, responsiveStart + 2400)
  assert.match(responsive, /\.xz-header\s*{[^}]*grid-template-columns:/)
  assert.match(responsive, /\.xz-header-actions\s*{[^}]*grid-column:\s*1\s*\/\s*-1/)
  assert.match(responsive, /\.xz-header-actions\s*{[^}]*flex-wrap:\s*wrap/)
  assert.match(source, /\.xz-name,[\s\S]*?white-space:\s*nowrap/)
  assert.match(source, /\.xz-approval-warning-text\s*{[^}]*text-overflow:\s*ellipsis/)
})

test('narrow composer hints truncate instead of wrapping into the editor controls', async () => {
  const source = await readFile(viewPath, 'utf8')

  assert.match(
    source,
    /\.xz-hint\s*{[^}]*white-space:\s*nowrap;[^}]*overflow:\s*hidden;[^}]*text-overflow:\s*ellipsis/,
  )
  assert.match(source, /class="xz-hint-sep xz-hint-sep-drop"/)
  assert.match(source, /\.xz-hint-drop,[\s\S]*?\.xz-hint-sep-drop\s*{[^}]*display:\s*none/)
})
