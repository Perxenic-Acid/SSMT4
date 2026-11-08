import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, copyFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve, dirname } from 'node:path'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'

const backend = process.env.SSMT_HOYOSHADE_BACKEND
test('HoYoShade standalone backend composes peers, preserves user settings and refuses foreign graphics files', { skip: !backend }, () => {
  const root = mkdtempSync(join(tmpdir(), 'ssmt-hoyoshade-backend-'))
  try {
    const packageRoot = join(root, 'package'), resources = join(root, 'resources'), game = join(root, 'game')
    for (const path of [join(packageRoot, 'host'), join(resources, 'Presets'), join(resources, 'reshade-shaders', 'Addons'), game]) mkdirSync(path, { recursive: true })
    const policyPath = join(packageRoot, 'host', 'graphics.json')
    copyFileSync('native/official-plugin-packages/ssmt.hoyoshade.bridge/0.2.0/host/graphics.json', policyPath)
    writeFileSync(join(resources, 'ReShade64.dll'), 'offline reshade DLL fixture')
    writeFileSync(join(resources, 'ReShade.ini'), '[GENERAL]\nEffectSearchPaths=reshade-shaders/Shaders\n[OVERLAY]\nTutorialProgress=0\n')
    writeFileSync(join(resources, 'Presets', 'Mod OFF.ini'), 'Techniques=Shade@shade.fx\n[shade.fx]\nStrength=1\n')
    writeFileSync(join(game, 'StarRail.exe'), 'offline executable fixture')
    const context = {
      pluginId: 'ssmt.hoyoshade.bridge', pluginVersion: '0.2.0', gameName: '测试游戏', gamePreset: 'SRMI',
      gameExecutable: join(game, 'StarRail.exe'), packageRoot, dataRoot: join(root, 'data'), assetRoot: join(root, 'assets'),
      externalDependencies: { hoyoshade: { status: 'ready', path: resources } },
    }
    function call(method, payload = null) {
      const child = spawnSync(resolve(backend), [], {
        input: JSON.stringify({ protocolVersion: 1, context, method, event: method !== 'status.read', payload }),
        encoding: 'utf8', timeout: 15000, windowsHide: true,
      })
      assert.equal(child.error, undefined)
      assert.equal(child.status, 0, child.stderr)
      return JSON.parse(child.stdout)
    }
    assert.equal(call('status.read').value.ready, true)
    const description = call('launch.describe')
    assert.equal(description.ok, true)
    assert.deepEqual(description.value.launch.resources, ['graphics.reshade.host'])
    const own = { pluginId: context.pluginId, exports: description.value.exports, selected: true }
    const peerAddon = join(root, 'peer.addon64')
    writeFileSync(peerAddon, 'peer addon fixture')
    const peer = { pluginId: 'example.other', permissions: ['native.inject'], nativeInjectionAcknowledged: true, exports: { 'ssmt.reshade.v1': {
      source: 'other plugin', config: '[GENERAL]\nTextureSearchPaths=C:/Other/Textures\n',
      preset: 'Techniques=Peer@peer.fx\n', addons: { 'peer.addon64': peerAddon },
    } }, selected: false }
    const prepared = call('launch.prepare', { peers: [own, peer] })
    assert.equal(prepared.ok, true, prepared.error)
    assert.equal(prepared.value.dlls.length, 1)
    assert.equal(call('launch.prepare', { peers: [own], launchContext: { existingDlls: ['C:/Other/dxgi.dll'] } }).ok, false)
    assert.equal(call('launch.prepare', { peers: [own, { ...peer, nativeInjectionAcknowledged: false }] }).ok, false)
    assert.equal(prepared.value.dlls[0].sha256, createHash('sha256').update(readFileSync(join(resources, 'ReShade64.dll'))).digest('hex'))
    const state = prepared.value.environment.RESHADE_BASE_PATH_OVERRIDE
    assert.equal(prepared.value.environment.GIMI_HOSTED_RESHADE_DLL, null)
    assert.match(readFileSync(join(state, 'ReShade.ini'), 'utf8'), /C:\/Other\/Textures/)
    assert.equal(readFileSync(join(state, 'peer.addon64'), 'utf8'), 'peer addon fixture')
    const ini = join(state, 'ReShade.ini')
    writeFileSync(ini, readFileSync(ini, 'utf8').replace('TutorialProgress=0', 'TutorialProgress=4'))
    const preset = join(state, 'ReShadePreset.ini')
    writeFileSync(preset, readFileSync(preset, 'utf8').replace('Strength=1', 'Strength=0.6'))
    assert.equal(call('launch.prepare', { peers: [own] }).ok, true)
    assert.match(readFileSync(ini, 'utf8'), /TutorialProgress=4/)
    assert.match(readFileSync(preset, 'utf8'), /Strength=0.6/)
    assert.doesNotMatch(readFileSync(preset, 'utf8'), /Peer@peer.fx/)
    assert.equal(existsSync(join(state, 'peer.addon64')), false)

    const before = readFileSync(ini, 'utf8')
    writeFileSync(join(state, 'unknown.addon64'), 'user addon')
    assert.equal(call('launch.prepare', { peers: [own] }).ok, false)
    assert.equal(readFileSync(ini, 'utf8'), before)
    assert.equal(readFileSync(join(state, 'unknown.addon64'), 'utf8'), 'user addon')
    rmSync(join(state, 'unknown.addon64'))
    writeFileSync(join(game, 'dxgi.dll'), 'foreign proxy')
    assert.equal(call('launch.prepare', { peers: [own] }).ok, false)
    assert.equal(readFileSync(join(game, 'dxgi.dll'), 'utf8'), 'foreign proxy')
    assert.equal(readFileSync(ini, 'utf8'), before)
    rmSync(join(game, 'dxgi.dll'))
    writeFileSync(join(game, 'ReShade.ini'), '[INSTALL]\nBasePath=C:/AnotherInstall\n')
    assert.equal(call('launch.prepare', { peers: [own] }).ok, false)
    assert.equal(readFileSync(join(game, 'ReShade.ini'), 'utf8'), '[INSTALL]\nBasePath=C:/AnotherInstall\n')

    context.gameExecutable = join(game, 'Unrelated.exe')
    assert.equal(call('launch.describe').value.launch, null)
    const policy = JSON.parse(readFileSync(policyPath, 'utf8'))
    policy.hostDll = '../escape.dll'
    writeFileSync(policyPath, JSON.stringify(policy))
    assert.equal(call('launch.describe').ok, false)
    assert.equal(call('arbitrary.method').ok, false)
    assert.equal(call('launch.cancel').ok, true)
  } finally {
    assert.equal(dirname(root), resolve(tmpdir()))
    assert.match(root, /ssmt-hoyoshade-backend-/)
    rmSync(root, { recursive: true, force: true })
  }
})
