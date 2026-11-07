import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import ts from 'typescript'
import vm from 'node:vm'

const exports = {}
const source = readFileSync(new URL(
  '../../native/secondary-motion-ui/src/runtimeConfig.ts',
  import.meta.url,
), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
vm.runInNewContext(compiled, { exports })

const {
  normalizeRuntimeConfig,
  setRuntimeEnabledPreference,
  applyRuntimeSettings,
  serializeRuntimeConfig,
} = exports

test('legacy disabled config migrates to default-on with a new revision', () => {
  const normalized = normalizeRuntimeConfig({
    revision: 4,
    enabled: false,
    active_preset: 'Default',
  })
  assert.equal(normalized.migrated, true)
  assert.equal(normalized.config.enabled, true)
  assert.equal(normalized.config.enabled_user_set, false)
  assert.equal(normalized.config.revision, 5)
})

test('explicit user preference survives remount and page navigation', () => {
  const enabled = normalizeRuntimeConfig({
    revision: 6,
    enabled: true,
    enabled_user_set: true,
    active_preset: 'Set1',
  })
  assert.equal(enabled.migrated, false)
  assert.equal(enabled.config.enabled, true)

  const disabled = setRuntimeEnabledPreference(enabled.config, false)
  assert.equal(disabled.enabled, false)
  assert.equal(disabled.enabled_user_set, true)
  assert.equal(disabled.revision, 7)
  const reopened = normalizeRuntimeConfig(JSON.parse(serializeRuntimeConfig(disabled)))
  assert.equal(reopened.config.enabled, false)
  assert.equal(reopened.migrated, false)
})

test('applying preset settings preserves the explicit switch preference', () => {
  const config = normalizeRuntimeConfig({
    revision: 8,
    enabled: false,
    enabled_user_set: true,
    active_preset: 'Default',
  }).config
  const applied = applyRuntimeSettings(config, false, 'User')
  assert.equal(applied.active_preset, 'User')
  assert.equal(applied.enabled, false)
  assert.equal(applied.enabled_user_set, true)
  assert.equal(applied.revision, 9)
})
