import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import * as vue from 'vue'

const plain = value => JSON.parse(JSON.stringify(value))
function bridge(handler) {
  const source = readFileSync(new URL('../../native/dlss5-integration-ui/src/bridge.ts', import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const calls = []
  let listener
  const parent = {
    postMessage(request) {
      calls.push(plain(request))
      queueMicrotask(async () => {
        const response = { type: 'ssmt:capability-response', protocolVersion: 1, requestId: request.requestId }
        try {
          const value = await handler(request.capability, plain(request.payload))
          listener({ source: {}, data: { ...response, ok: true, value: 'foreign-response' } })
          listener({ source: parent, data: { ...response, ok: true, value } })
        } catch (error) {
          listener({ source: parent, data: { ...response, ok: false, errorMessage: error.message } })
        }
      })
    },
  }
  const exports = {}
  vm.runInNewContext(compiled, {
    exports, require: name => { assert.equal(name, 'vue'); return vue },
    window: { addEventListener: (name, callback) => { assert.equal(name, 'message'); listener = callback } },
    parent, Date,
    setTimeout: (callback, milliseconds) => milliseconds <= 500 ? setTimeout(callback, 0) : setTimeout(callback, milliseconds),
    clearTimeout,
  })
  return { ...exports, calls }
}

test('DLSS5 页面通过通用后台桥轮询任务并保留进度及失败', async () => {
  let fail = false
  let reads = 0
  const s = bridge((capability, { method, payload }) => {
    assert.equal(capability, 'plugin.backend.request')
    if (method === 'task.start') {
      assert.equal(payload.operation, 'runtime.download')
      assert.equal(payload.runtime, 'gimi')
      assert.equal(payload.onProgress, undefined)
      return { taskId: 'task-a', status: 'queued', cancellable: true }
    }
    assert.equal(method, 'task.read')
    assert.equal(payload.taskId, 'task-a')
    return ++reads % 2 ? { taskId: 'task-a', status: 'running', progress: { downloadedBytes: 50 } }
      : { taskId: 'task-a', status: fail ? 'failed' : 'succeeded', error: 'download failed', result: { installed: true } }
  })
  const progress = new s.Channel()
  const updates = []
  progress.onmessage = update => updates.push(plain(update))
  assert.deepEqual(plain(await s.invoke('download_gimi_hot_runtime', { gameName: 'game', onProgress: progress })), { installed: true })
  assert.deepEqual(updates, [{ downloadedBytes: 50 }])
  assert.equal(s.activeTask.value, null)
  fail = true
  await assert.rejects(s.invoke('download_gimi_hot_runtime', {}), /download failed/)
  assert.equal(s.activeTask.value, null)
})

test('DLSS5 页面重开后恢复活动任务并向同一任务发送取消', async () => {
  let cancelled = false
  const s = bridge((_capability, { method, payload }) => {
    if (method === 'task.active') return [{ taskId: 'restored', status: 'running', cancellable: true }]
    assert.equal(payload.taskId, 'restored')
    if (method === 'task.cancel') { cancelled = true; return null }
    assert.equal(method, 'task.read')
    return { taskId: 'restored', status: 'cancelled', cancellable: false }
  })
  const restoring = s.restoreActiveTask()
  while (!s.activeTask.value) await new Promise(resolve => setImmediate(resolve))
  await assert.rejects(s.invoke('download_gimi_hot_runtime', {}), /已有组件任务/)
  await s.cancelActiveTask()
  await restoring
  assert.equal(cancelled, true)
  assert.equal(s.activeTask.value, null)
})

test('DLSS5 页面解包通用目录选择结果并拒收其他窗口响应', async () => {
  const s = bridge((capability, payload) => {
    if (capability === 'host.directory.select') return { path: 'D:/Offline/GIMI' }
    assert.equal(capability, 'host.url.open')
    assert.deepEqual(payload, { url: 'https://example.com/download' })
    return null
  })
  assert.equal(await s.open({ directory: true }), 'D:/Offline/GIMI')
  await s.openUrl('https://example.com/download')
  assert.ok(s.calls.every(call => call.protocolVersion === 1))
})
