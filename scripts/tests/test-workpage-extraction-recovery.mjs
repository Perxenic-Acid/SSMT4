import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import {
  createWorkspaceTabsAfterClear,
  getNextWorkspaceTabIndex,
} from '../../src/views/WorkPage/WorkspaceTabs.ts'

const extractorPath = name =>
  new URL(`../../src-tauri/src/extract_new/${name}.rs`, import.meta.url)

test('clearing a workspace always resets the default tab to LOD0', () => {
  const tabs = createWorkspaceTabsAfterClear('LOD', () => 'reset-tab')

  assert.deepEqual(tabs, [{ id: 'reset-tab', name: 'LOD0' }])
  assert.equal(
    getNextWorkspaceTabIndex([
      { id: 'a', name: 'LOD0' },
      { id: 'b', name: 'LOD4' },
    ], 'LOD'),
    5,
  )
})

test('full extraction wraps each DrawIB failure instead of propagating it', async () => {
  for (const name of [
    'gimi',
    'himi',
    'zzmi',
    'zzmi_dx12',
    'gf2',
    'ailimit',
    'naraka',
    'narakam',
    'snowbreak',
    'ntemi',
    'yysls',
    'apmi2',
    'srmi',
  ]) {
    const source = await readFile(extractorPath(name), 'utf8')
    assert.match(
      source,
      /evaluate_drawib_extraction\(/,
      `${name} must isolate per-DrawIB failures`,
    )
    assert.match(
      source,
      /evaluate_drawib_extraction\([\s\S]*?!self\.specify_drawib_extract[\s\S]*?\?;/,
      `${name} must still propagate targeted-extraction errors`,
    )
  }
})

test('texture sync skips one malformed IB file instead of aborting the workspace', async () => {
  const source = await readFile(
    new URL('../../src-tauri/src/helper/workspace_texture_sync.rs', import.meta.url),
    'utf8',
  )

  assert.match(source, /continue_after_item_error: bool/)
  assert.match(source, /handle_drawib_extraction_error\(/)
})

test('targeted extraction propagates texture sync errors', async () => {
  for (const name of [
    'ailimit',
    'apmi2',
    'gf2',
    'gimi',
    'identityv',
    'naraka',
    'narakam',
    'snowbreak',
    'wwmi',
    'yysls',
    'zzmi',
    'zzmi_dx12',
  ]) {
    const source = await readFile(extractorPath(name), 'utf8')
    assert.match(
      source,
      /sync_[a-z_]+\([\s\S]{0,220}!self(?:\.base)?\.specify_drawib_extract/,
      `${name} must keep targeted-extraction texture sync failures fatal`,
    )
  }
})

test('CAMI keeps its existing per-file texture sync tolerance', async () => {
  const source = await readFile(
    new URL('../../src-tauri/src/helper/workspace_texture_sync.rs', import.meta.url),
    'utf8',
  )

  assert.match(
    source,
    /sync_cami_workspace_deduped_textures_and_json[\s\S]*?true,\s*true,?\s*\)/,
  )
})
