import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, mkdirSync, cpSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { setTimeout } from 'node:timers/promises'

const backend = process.env.SSMT_DLSS5_BACKEND
test('DLSS5 独立 JSON 后台按游戏隔离任务、拒绝越权请求并回收失效任务', { skip: !backend }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'ssmt-dlss5-backend-'))
  try {
    const packageRoot = join(root, 'package'), game = join(root, 'game')
    cpSync('native/official-plugin-packages/ssmt.dlss5.integration/0.3.0', packageRoot, { recursive: true })
    mkdirSync(game)
    const executable = join(game, 'YuanShen.exe')
    writeFileSync(executable, 'offline executable fixture')
    const context = {
      pluginId: 'ssmt.dlss5.integration', pluginVersion: '0.3.0', gameName: '离线原神', gamePreset: 'GIMI',
      gameExecutable: executable, packageRoot, dataRoot: join(root, 'global', 'PluginData', 'ssmt.dlss5.integration', 'backend'),
      assetRoot: join(root, 'global', 'PluginAssets', 'ssmt.dlss5.integration'), externalDependencies: {}, legacySettings: {},
    }
    function call(method, payload = {}, scope = context) {
      const child = spawnSync(resolve(backend), [], { input: JSON.stringify({ protocolVersion: 1, context: scope, method, payload }), encoding: 'utf8', timeout: 15000, windowsHide: true })
      assert.equal(child.error, undefined)
      assert.equal(child.status, 0, child.stderr)
      return JSON.parse(child.stdout)
    }
    assert.equal(call('launch.describe').ok, true)
    const described = call('launch.describe', { gameExecutable: executable }, { ...context, gameExecutable: join(game, 'Other.exe') })
    assert.equal(described.ok, true, described.error)
    assert.ok(described.value.launch)
    const conflict = call('launch.prepare', { launchContext: { existingDlls: ['C:/Other/d3d11.dll'] } })
    assert.equal(conflict.ok, false)
    assert.match(conflict.error, /额外图形代理/)
    const status = call('inspect_dlss5_game_state')
    assert.equal(status.ok, true, status.error)
    assert.equal(status.value.state, 'not_managed')
    assert.equal(call('inspect_dlss5_route_options').ok, true)
    assert.equal(call('launch.cancel').ok, true)
    assert.equal(call('task.active').value.length, 0)
    assert.equal(call('task.start', { operation: 'unknown' }).ok, false)
    assert.equal(call('task.read', { taskId: '../escape' }).ok, false)
    assert.equal(call('inspect_dlss5_game_state', { gameName: '另一游戏' }).ok, false)
    assert.equal(call('unknown').ok, false)

    const started = call('task.start', { operation: 'runtime.import', profile: 'rtx50', sourceDirectory: join(root, 'missing') })
    assert.equal(started.ok, true, started.error)
    const taskId = started.value.taskId
    assert.equal(call('task.read', { taskId }, { ...context, gameName: '其他原神' }).ok, false)
    let task
    const deadline = Date.now() + 15000
    do {
      await setTimeout(150)
      task = call('task.read', { taskId }).value
    } while (['queued', 'running'].includes(task.status) && Date.now() < deadline)
    assert.equal(task.status, 'failed')
    assert.equal(typeof task.error, 'string')
    assert.equal(readFileSync(executable, 'utf8'), 'offline executable fixture')
    assert.equal(existsSync(join(game, 'dxgi.dll')), false)

    const orphanId = 'dead-beef', orphanRoot = join(context.dataRoot, 'tasks', orphanId)
    mkdirSync(orphanRoot)
    writeFileSync(join(orphanRoot, 'state.json'), JSON.stringify({ taskId: orphanId, gameName: context.gameName, status: 'running', workerPid: 0, createdAtMs: 0 }))
    assert.equal(call('task.read', { taskId: orphanId }).value.status, 'failed')
    assert.equal(call('task.active').value.length, 0)
    assert.equal(call('disabled').ok, true)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
