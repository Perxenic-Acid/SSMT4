import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { setImmediate } from 'node:timers/promises'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import * as vue from 'vue'
import { parse } from '@vue/compiler-sfc'

const plain = value => JSON.parse(JSON.stringify(value))

function page(t, handler, initialStorage = {}) {
  const source = parse(readFileSync(
    new URL('../../native/dlss5-integration-ui/src/Dlss5Page.vue', import.meta.url),
    'utf8',
  )).descriptor.scriptSetup.content
  const compiled = ts.transpileModule(`${source}
Object.assign(exports, {
  activeTask, error, gameExecutable, history, installedRuntime, lastRun,
  manageHistory, refresh, routes, runtimeStatus, selectedProfile,
  selectedRoute, startTask, installSelected
})`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const mounted = []
  const unmounted = []
  const storage = new Map(Object.entries(initialStorage))
  const windowMock = {
    setTimeout(callback) {
      queueMicrotask(callback)
      return 1
    },
    clearTimeout() {},
    addEventListener() {},
    removeEventListener() {},
    parent: { postMessage() {} },
  }
  const scope = vue.effectScope()
  const exports = {}
  scope.run(() => vm.runInNewContext(compiled, {
    exports,
    console,
    Date,
    Math,
    Promise,
    Set,
    URLSearchParams,
    window: windowMock,
    localStorage: {
      getItem: key => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: key => storage.delete(key),
    },
    require(name) {
      if (name === 'vue') {
        return {
          ...vue,
          onMounted: callback => mounted.push(callback),
          onBeforeUnmount: callback => unmounted.push(callback),
        }
      }
      if (name === './bridge') return { requestCapability: handler }
      if (name === 'element-plus') {
        return {
          ElMessage: { success() {}, warning() {}, error() {} },
          ElMessageBox: { confirm: async () => undefined },
        }
      }
      throw new Error(`Unexpected import: ${name}`)
    },
  }))
  exports.dispose = () => {
    scope.stop()
    unmounted.splice(0).forEach(callback => callback())
  }
  t.after(exports.dispose)
  return { ...exports, mounted, storage }
}

function baseHandler(overrides = {}) {
  const calls = []
  const handler = async (capability, payload = {}) => {
    calls.push({ capability, payload })
    if (capability in overrides) return overrides[capability](payload, calls)
    if (capability === 'game.context.read') {
      return { gameName: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe' }
    }
    if (capability === 'graphics.routes.read') {
      return {
        routes: ['gimi-hot', 'ssmt-feeder'],
        installedRoute: 'gimi-hot',
        antiCheatWarning: false,
      }
    }
    if (capability === 'graphics.status.read') {
      return {
        gameState: { state: 'not_managed', route: null, managedHost: null, owner: null, reason: null },
        lastRun: { verdict: 'unknown', reason: 'no_log', observedAtMs: null, warning: null },
      }
    }
    if (capability === 'graphics.runtime.status') {
      return { gimi: null, dlss5: null }
    }
    if (capability === 'graphics.runtime.history') return []
    throw new Error(`Unexpected capability: ${capability}`)
  }
  return { handler, calls }
}

test('DLSS5 plugin page reads state only through the PageLease capability bridge', async t => {
  const pagePath = new URL('../../native/dlss5-integration-ui/src/Dlss5Page.vue', import.meta.url)
  if (!existsSync(pagePath)) {
    t.skip('DLSS5 UI is outside the minimal Native checkout')
    return
  }
  const { handler, calls } = baseHandler()
  const s = page(t, handler)
  await s.refresh()
  assert.equal(s.gameExecutable.value, 'C:/Game/YuanShen.exe')
  assert.deepEqual(Array.from(s.routes.value), ['gimi-hot', 'ssmt-feeder'])
  assert.equal(s.selectedRoute.value, 'gimi-hot')
  assert.ok(calls.every(call => !('pluginId' in call.payload) && !('gameName' in call.payload)))
  assert.deepEqual(calls.map(call => call.capability), [
    'game.context.read',
    'graphics.routes.read',
    'graphics.status.read',
    'graphics.runtime.status',
    'graphics.runtime.history',
  ])
})

test('DLSS5 download uses a host task, reports progress and resumes after remount', async t => {
  const pagePath = new URL('../../native/dlss5-integration-ui/src/Dlss5Page.vue', import.meta.url)
  if (!existsSync(pagePath)) {
    t.skip('DLSS5 UI is outside the minimal Native checkout')
    return
  }
  let taskReads = 0
  const { handler, calls } = baseHandler({
    'graphics.task.start': () => ({
      taskId: 'graphics-1',
      operation: 'runtime.download',
      status: 'queued',
      phase: 'queued',
      message: 'queued',
      downloadedBytes: 0,
      totalBytes: 200,
      cancellable: true,
    }),
    'graphics.task.status': () => ++taskReads < 3
      ? {
          taskId: 'graphics-1',
          operation: 'runtime.download',
          status: 'running',
          phase: 'downloading',
          message: 'downloading',
          downloadedBytes: taskReads * 50,
          totalBytes: 200,
          cancellable: true,
        }
      : {
          taskId: 'graphics-1',
          operation: 'runtime.download',
          status: 'succeeded',
          phase: 'completed',
          message: 'completed',
          downloadedBytes: 200,
          totalBytes: 200,
          cancellable: false,
        },
  })
  const s = page(t, handler)
  await s.refresh()
  s.selectedProfile.value = 'rtx30'
  await s.installSelected(false)
  assert.deepEqual(
    plain(calls.find(call => call.capability === 'graphics.task.start')?.payload),
    { operation: 'runtime.download', runtime: 'gimi', route: 'gimi-hot', profile: 'rtx30' },
  )
  await setImmediate()
  await setImmediate()
  await setImmediate()
  assert.equal(s.storage.get('ssmt.dlss5.task.gimi'), undefined)
  assert.equal(s.activeTask.value, null)

  const resumed = page(t, handler, { 'ssmt.dlss5.task.gimi': 'graphics-restored' })
  await resumed.refresh()
  await setImmediate()
  assert.ok(calls.some(call =>
    call.capability === 'graphics.task.status' && call.payload.taskId === 'graphics-restored'))
})

test('DLSS5 offline import uses the host directory picker before creating an import task', async t => {
  const pagePath = new URL('../../native/dlss5-integration-ui/src/Dlss5Page.vue', import.meta.url)
  if (!existsSync(pagePath)) {
    t.skip('DLSS5 UI is outside the minimal Native checkout')
    return
  }
  const { handler, calls } = baseHandler({
    'graphics.directory.select': () => ({ path: 'D:/Offline/GIMI' }),
    'graphics.task.start': payload => ({
      taskId: 'graphics-import',
      operation: payload.operation,
      status: 'succeeded',
      phase: 'completed',
      message: 'completed',
      downloadedBytes: 0,
      totalBytes: 0,
      cancellable: false,
    }),
  })
  const s = page(t, handler)
  await s.refresh()
  await s.installSelected(true)
  assert.deepEqual(plain(calls.find(call => call.capability === 'graphics.directory.select')?.payload), {
    purpose: 'runtime.import',
  })
  assert.deepEqual(plain(calls.find(call => call.capability === 'graphics.task.start')?.payload), {
    operation: 'runtime.import',
    sourceDirectory: 'D:/Offline/GIMI',
    profile: 'rtx50',
  })
  assert.ok(!calls.some(call => call.payload.operation === 'runtime.download'))
})
