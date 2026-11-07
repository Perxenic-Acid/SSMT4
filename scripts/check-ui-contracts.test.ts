import { expect, test } from 'bun:test'
import { normalizeSsmtUiMode } from '../src/store/AppSettings'
import {
  permissionDefinitionFor,
  versionRequirementState,
} from '../src/views/PluginMarketplace/pluginMarketplaceModel'

const projectRoot = new URL('..', import.meta.url)
const rustPluginSource = await Bun.file(new URL('src-tauri/src/plugin/mod.rs', projectRoot)).text()
const rustPermissionBlock = rustPluginSource.match(
  /pub enum PluginPermission\s*\{([\s\S]*?)\n\}/,
)?.[1] ?? ''
const rustPermissions = [...rustPermissionBlock.matchAll(/#\[serde\(rename = "([^"]+)"\)\]/g)]
  .map(match => match[1])
  .sort()

const frontendPermissions = [
  'filesystem.read',
  'filesystem.write',
  'game.launch',
  'native.inject',
  'network',
  'process.observe',
  'process.spawn',
].sort()

test('UI mode normalization keeps supported modes and falls back safely', () => {
  expect(normalizeSsmtUiMode('modern')).toBe('modern')
  expect(normalizeSsmtUiMode('classic')).toBe('classic')
  expect(normalizeSsmtUiMode('unknown')).toBe('modern')
  expect(normalizeSsmtUiMode(undefined)).toBe('modern')
})

test('plugin version requirements use the current SSMT version', () => {
  expect(versionRequirementState('4.2.2', '>=4.x', null)).toBe('supported')
  expect(versionRequirementState('4.2.2', '>=4.3.0', null)).toBe('blocked')
  expect(versionRequirementState('4.2.2', null, '4.2.2')).toBe('supported')
  expect(versionRequirementState('4.2.2', null, '4.2.1')).toBe('blocked')
  expect(versionRequirementState('', '>=4.0.0', null)).toBe('unknown')
})

test('frontend and Rust plugin permission identifiers stay aligned', () => {
  expect(rustPermissions).toEqual(frontendPermissions)
  for (const permission of frontendPermissions) {
    expect(permissionDefinitionFor(permission).id).toBe(permission)
  }
})
