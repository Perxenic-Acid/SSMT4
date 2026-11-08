const pluginIds = ['example.codex.launch-alpha', 'example.codex.launch-beta']
const targets = await (await fetch('http://127.0.0.1:9333/json')).json()
const target = targets.find(item => item.url?.includes('localhost:1422') && item.webSocketDebuggerUrl)
if (!target?.webSocketDebuggerUrl) throw new Error('SSMT4 WebView target was not found')

const socket = new WebSocket(target.webSocketDebuggerUrl)
await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true })
  socket.addEventListener('error', () => reject(new Error('Could not connect to SSMT4')), { once: true })
})

let sequence = 0
const pending = new Map()
socket.addEventListener('message', event => {
  const message = JSON.parse(String(event.data))
  if (!message.id) return
  const request = pending.get(message.id)
  if (!request) return
  pending.delete(message.id)
  if (message.error) request.reject(new Error(message.error.message || 'CDP request failed'))
  else request.resolve(message)
})
const send = (method, params = {}) => {
  const id = ++sequence
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject })
    socket.send(JSON.stringify({ id, method, params }))
  })
}
const evaluate = async expression => {
  const response = await send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  })
  if (response.result?.exceptionDetails) {
    throw new Error(response.result.exceptionDetails.exception?.description || response.result.exceptionDetails.text)
  }
  return response.result?.result?.value
}
const invoke = (command, args = {}) => evaluate(
  `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})`,
)
const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
const waitFor = async (expression, label, timeoutMs = 30000) => {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await evaluate(`Boolean(${expression})`)) return
    await sleep(150)
  }
  throw new Error(`Timed out waiting for ${label}`)
}
const uninstall = async pluginId => {
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: pluginId,
    enabled: false,
    gameExecutable: null,
  }).catch(() => false)
  await invoke('uninstall_plugin', { pluginId, gameName: 'EFMI' }).catch(() => false)
}

try {
  await send('Page.enable')
  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = pluginIds.map(pluginId => {
    const item = packages.find(candidate => candidate.inspection?.manifest?.id === pluginId)
    if (!item) throw new Error(`Launch plan fixture was not discovered: ${pluginId}`)
    return item
  })

  for (const pluginId of pluginIds) await uninstall(pluginId)
  for (const item of local) {
    await invoke('install_local_plugin', {
      id: item.inspection.manifest.id,
      reviewedSha256: item.inspection.sha256,
      disclaimerAccepted: true,
    })
    await invoke('set_plugin_enabled_for_game', {
      gameName: 'EFMI',
      id: item.inspection.manifest.id,
      enabled: true,
      gameExecutable: null,
    })
  }

  const conflictPlan = await invoke('plugin_launch_plan_preview', { gameName: 'EFMI' })
  const conflict = conflictPlan.conflicts?.find(item => item.resourceKey === 'plugin:proxy:fixture')
  if (!conflict || conflictPlan.fileActions?.length !== 0) {
    throw new Error(`Equal-priority launch conflict was not reported: ${JSON.stringify(conflictPlan)}`)
  }
  const candidates = new Set(conflict.candidates.map(item => item.pluginId))
  if (!candidates.has(pluginIds[0]) || !candidates.has(pluginIds[1])) {
    throw new Error(`Launch conflict candidates are incomplete: ${JSON.stringify(conflict.candidates)}`)
  }

  const pushed = await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push({ path: '/plugins', query: { plugin: '${pluginIds[0]}' } })
    return true
  })()`)
  if (!pushed) throw new Error('Vue router is unavailable')
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'marketplace refresh action')
  await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (button instanceof HTMLElement) button.click()
  })()`)
  await waitFor(`document.querySelector('.launch-plan-conflict')`, 'launch resource conflict prompt')
  await send('Page.bringToFront')
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-plugin-launch-conflict.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: pluginIds[1],
    enabled: false,
    gameExecutable: null,
  })
  const selectedPlan = await invoke('plugin_launch_plan_preview', { gameName: 'EFMI' })
  if (selectedPlan.conflicts?.length || selectedPlan.fileActions?.length !== 1
    || selectedPlan.fileActions[0].pluginId !== pluginIds[0]) {
    throw new Error(`Single-owner launch plan was not deterministic: ${JSON.stringify(selectedPlan)}`)
  }
  const selectedTarget = selectedPlan.fileActions[0].target
  let targetExists = true
  try {
    const { access } = await import('node:fs/promises')
    await access(selectedTarget)
  } catch {
    targetExists = false
  }
  if (targetExists) throw new Error(`Launch plan preview wrote its target: ${selectedTarget}`)

  console.log(JSON.stringify({
    packages: local.map(item => ({
      id: item.inspection.manifest.id,
      version: item.inspection.manifest.version,
      sha256: item.inspection.sha256,
    })),
    conflict: {
      resourceKey: conflict.resourceKey,
      candidates: conflict.candidates,
    },
    selected: selectedPlan.fileActions[0],
    screenshotPath,
    targetExists,
    gameDirectoryWrites: 0,
  }, null, 2))
} finally {
  for (const pluginId of pluginIds) await uninstall(pluginId)
  socket.close()
}
