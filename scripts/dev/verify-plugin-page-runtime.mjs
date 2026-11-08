const targets = await (await fetch('http://127.0.0.1:9333/json')).json()
const target = targets.find(item => item.url?.includes('localhost:1422') && item.webSocketDebuggerUrl)
if (!target?.webSocketDebuggerUrl) {
  throw new Error('SSMT4 WebView target was not found on CDP port 9333')
}

const socket = new WebSocket(target.webSocketDebuggerUrl)
await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true })
  socket.addEventListener('error', () => reject(new Error('Could not connect to the SSMT4 CDP target')), { once: true })
})

let sequence = 0
const pending = new Map()
const observedEvents = []
socket.addEventListener('message', event => {
  const message = JSON.parse(String(event.data))
  if (message.method === 'Network.loadingFailed'
    || message.method === 'Network.responseReceived'
    || message.method === 'Runtime.consoleAPICalled'
    || message.method === 'Log.entryAdded') {
    observedEvents.push(message)
  }
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

try {
  await send('Page.enable')
  await send('Network.enable')
  await send('Log.enable')
  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = packages.find(item => item.inspection?.manifest?.id === 'ssmt.efmi-glow-studio')
  if (!local) throw new Error('Glow Studio 0.2.5 local package was not discovered')

  const installed = await invoke('plugin_registry_snapshot')
  const alreadyInstalled = installed.some(item =>
    item.manifest?.id === local.inspection.manifest.id
    && item.manifest?.version === local.inspection.manifest.version
  )
  if (!alreadyInstalled) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_external_dependency_path', {
    pluginId: 'ssmt.efmi-glow-studio',
    dependencyId: 'python-runtime',
    path: 'F:\\Programs\\Python\\Python314',
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'ssmt.efmi-glow-studio',
    enabled: true,
    gameExecutable: null,
  })
  await invoke('stop_plugin_page_services_for_plugin', { pluginId: 'ssmt.efmi-glow-studio' })
  await send('Page.reload', { ignoreCache: true })
  await sleep(1200)

  const capabilityCatalog = await invoke('plugin_capability_catalog')
  if (capabilityCatalog.apiVersion !== 1
    || !capabilityCatalog.capabilities.some(item => item.id === 'plugin.settings.get')) {
    throw new Error('Versioned capability catalog is incomplete')
  }
  const capabilityKey = `e2e:${Date.now()}`
  const settingsWrite = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'plugin.settings.set',
    payload: { key: capabilityKey, value: { enabled: true } },
    requestId: 'e2e-settings-set',
  })
  if (!settingsWrite.ok) throw new Error(`Capability settings write failed: ${settingsWrite.errorCode}`)
  const settingsRead = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'plugin.settings.get',
    payload: { key: capabilityKey },
    requestId: 'e2e-settings-get',
  })
  if (!settingsRead.ok || settingsRead.value?.value?.enabled !== true) {
    throw new Error('Capability settings round trip failed')
  }
  const storageRead = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'plugin.storage.set',
    payload: { key: capabilityKey, value: 'scoped' },
    requestId: 'e2e-storage-set',
  })
  if (!storageRead.ok) throw new Error('Authorized plugin storage write was denied')
  await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'plugin.storage.remove',
    payload: { key: capabilityKey },
    requestId: 'e2e-storage-remove',
  })
  await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'plugin.settings.remove',
    payload: { key: capabilityKey },
    requestId: 'e2e-settings-remove',
  })
  const gameContext = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.efmi-glow-studio',
    gameName: 'EFMI',
    capability: 'game.context.read',
    payload: {},
    requestId: 'e2e-game-context',
  })
  if (!gameContext.ok || !gameContext.value?.installDir) {
    throw new Error('Game context capability did not return installDir')
  }
  const denied = await invoke('plugin_capability_request', {
    pluginId: 'example.unknown',
    gameName: 'EFMI',
    capability: 'plugin.settings.get',
    payload: { key: 'x' },
    requestId: 'e2e-denied',
  })
  if (denied.ok || denied.errorCode !== 'plugin_not_installed') {
    throw new Error('Capability broker did not reject an unknown plugin identity')
  }

  const pages = await invoke('plugin_page_catalog', { gameName: 'EFMI' })
  const page = pages.find(item =>
    item.pluginId === 'ssmt.efmi-glow-studio'
    && item.pageId === 'editor'
    && item.kind === 'local-service'
  )
  if (!page) throw new Error('Generic local-service plugin page was not registered')

  const resetRoute = await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push('/')
    return true
  })()`)
  if (!resetRoute) throw new Error('Vue router is unavailable')
  await sleep(300)
  const pushed = await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.efmi-glow-studio' } })
    return true
  })()`)
  if (!pushed) throw new Error('Vue router is unavailable')
  await waitFor(`document.querySelector('.plugin-page-host')`, 'plugin page host')
  await waitFor(`document.querySelector('.local-service-toolbar')`, 'local service toolbar')
  await waitFor(
    `document.querySelector('.plugin-page-frame.ready')?.src`,
    'ready local service iframe',
    45000,
  )

  const iframeSrc = await evaluate(`document.querySelector('.plugin-page-frame')?.src || ''`)
  const parsedUrl = new URL(iframeSrc)
  const fragment = new URLSearchParams(parsedUrl.hash.replace(/^#/, ''))
  const uiLeaseId = fragment.get('leaseId') || ''
  const serviceKey = fragment.get('serviceKey') || ''
  const token = fragment.get('token') || ''
  if (!uiLeaseId || !serviceKey || !token) {
    throw new Error('UI iframe URL is missing leaseId, serviceKey or token')
  }

  const healthResponse = await fetch(`${parsedUrl.origin}/api/health`, {
    headers: { 'X-Editor-Token': token },
  })
  if (!healthResponse.ok) throw new Error(`Plugin page health check failed: HTTP ${healthResponse.status}`)
  const health = await healthResponse.json()
  if (health.service !== 'efmi-glow-studio-editor') {
    throw new Error('Plugin page health response did not identify the Glow editor')
  }

  const htmlResponse = await fetch(`${parsedUrl.origin}${parsedUrl.pathname}`)
  if (!htmlResponse.ok) throw new Error(`Plugin page HTML failed: HTTP ${htmlResponse.status}`)
  const html = await htmlResponse.text()
  if (!html.includes('/ui/assets/') && !html.includes('./assets/')) {
    throw new Error('Plugin page HTML does not reference the built editor assets')
  }

  const secondLease = await invoke('acquire_plugin_page_lease', {
    pluginId: 'ssmt.efmi-glow-studio',
    pageId: 'editor',
    gameName: 'EFMI',
  })
  if (secondLease.serviceKey !== serviceKey || secondLease.processId !== health.pid) {
    throw new Error('Generic service supervisor did not reuse the UI-created service instance')
  }
  if (secondLease.leaseId === uiLeaseId || secondLease.leaseCount < 2) {
    throw new Error('Service leases were not tracked independently')
  }
  if (!secondLease.running || !secondLease.url.includes('#token=')) {
    throw new Error('Plugin page service did not return a protected local URL')
  }
  await invoke('release_plugin_page_lease', { leaseId: secondLease.leaseId })

  await evaluate(`document.querySelector('.local-service-page')?.scrollIntoView({ block: 'start' })`)
  await sleep(4000)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: true,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-plugin-page-integrated.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }
  const stopped = await invoke('stop_plugin_page_service', { sessionId: uiLeaseId })
  if (!stopped) throw new Error('Plugin page service could not be stopped by session id')
  await sleep(300)
  let processAlive = false
  try {
    if (secondLease.processId) process.kill(secondLease.processId, 0)
    processAlive = Boolean(secondLease.processId)
  } catch {
    processAlive = false
  }
  if (processAlive) throw new Error(`Plugin service process ${secondLease.processId} is still running`)

  console.log(JSON.stringify({
    package: `${local.inspection.manifest.id}@${local.inspection.manifest.version}`,
    page,
    capabilityCatalog: {
      apiVersion: capabilityCatalog.apiVersion,
      pluginApiVersion: capabilityCatalog.pluginApiVersion,
      capabilityCount: capabilityCatalog.capabilities.length,
    },
    session: {
      running: secondLease.running,
      serviceKey: secondLease.serviceKey,
      processId: secondLease.processId,
      pagePath: secondLease.pagePath,
      dataRoot: secondLease.dataRoot,
      logPath: secondLease.logPath,
    },
    screenshotPath,
    iframeSrc,
    networkFailures: observedEvents
      .filter(event => event.method === 'Network.loadingFailed' || event.method === 'Log.entryAdded')
      .slice(-10),
    stopped,
  }, null, 2))
} finally {
  socket.close()
}
