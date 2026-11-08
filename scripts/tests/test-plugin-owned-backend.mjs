import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

const backend = process.env.SSMT_PLAYER_TWEAKS_BACKEND
test('Player Tweaks packaged backend preserves settings, prepares INI and reports errors through JSON', { skip: !backend }, () => {
  const root = mkdtempSync(join(tmpdir(), 'ssmt-plugin-owned-'))
  try {
    const executable = join(root, 'Game.exe')
    writeFileSync(executable, 'offline executable path fixture')
    mkdirSync(join(root, 'runtime'), { recursive: true })
    const context = {
      gameName: '配置甲', gamePreset: 'GIMI', gameExecutable: executable,
      packageRoot: root, dataRoot: join(root, 'data'),
      migrations: { 'legacy-game-config': { playerTweaksCamera: { customFov: true, fov: 75 }, playerTweaksGameplay: { targetFps: 144 } } },
    }
    function call(method, payload = null) {
      const child = spawnSync(resolve(backend), [], {
        input: JSON.stringify({ protocolVersion: 1, context, method, payload }),
        encoding: 'utf8', timeout: 10000, windowsHide: true,
      })
      assert.equal(child.error, undefined)
      assert.equal(child.status, 0, child.stderr)
      return JSON.parse(child.stdout)
    }
    const initial = call('settings.read')
    assert.equal(initial.ok, true)
    assert.equal(initial.value.camera.fov, 75)
    assert.equal(initial.value.gameplay.targetFps, 144)
    initial.value.gameplay.targetFps = 165
    assert.equal(call('settings.save', initial.value).ok, true)
    context.migrations = {}
    assert.equal(call('settings.read').value.gameplay.targetFps, 165)
    assert.equal(call('prepare').ok, true)
    const ini = readFileSync(join(root, 'runtime/SSMT-Player-Tweaks.ini'), 'utf8')
    assert.match(ini, /CustomFov=1\nFov=75\n/)
    assert.match(ini, /TargetFps=165\n/)
    context.gameName = '配置乙'
    assert.equal(call('settings.read').value.gameplay.targetFps, 120)
    assert.equal(call('not-a-method').ok, false)
    context.gamePreset = 'SRMI'
    const disabled = call('settings.read').value
    disabled.gameplay.fpsUnlock = false
    assert.equal(call('settings.save', disabled).ok, true)
    // 空恢复记录下走实际后台协议，不接触真实注册表。
    assert.equal(call('prepare').ok, true)
    assert.equal(call('cancel').ok, true)
    assert.equal(call('disabled').ok, true)
    context.gameName = '配置甲'
    context.gamePreset = 'GIMI'
    assert.equal(call('settings.read').value.gameplay.targetFps, 165)
    assert.equal(existsSync(join(root, 'data/ownership.lock')), true)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
