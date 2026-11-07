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
  const response = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
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
  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = packages.find(item => item.inspection?.manifest?.id === 'example.codex.page-sdk')
  if (!local) throw new Error('Unknown SDK fixture package was not discovered')

  const installed = await invoke('plugin_registry_snapshot')
  const currentInstalled = installed.some(item =>
    item.manifest?.id === local.inspection.manifest.id
    && item.manifest?.version === local.inspection.manifest.version
  )
  if (!currentInstalled) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_external_dependency_path', {
    pluginId: 'example.codex.page-sdk',
    dependencyId: 'python-runtime',
    path: 'F:\\Programs\\Python\\Python314',
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'example.codex.page-sdk',
    enabled: true,
    gameExecutable: null,
  })

  const catalog = await invoke('plugin_capability_catalog')
  if (catalog.apiVersion !== 1
    || catalog.pluginApiVersion !== '1.0.0'
    || catalog.pageBridgeProtocolVersion !== 1
    || catalog.manifestSchemaVersion !== 2
    || !catalog.capabilities?.some(capability => capability.id === 'plugin.settings.get')
    || !catalog.errorCodes?.includes('capability_denied')) {
    throw new Error(`Plugin API 1 catalog contract mismatch: ${JSON.stringify(catalog)}`)
  }
  const unknown = await invoke('plugin_capability_request', {
    pluginId: 'example.unknown.sdk',
    gameName: 'EFMI',
    capability: 'plugin.settings.get',
    payload: { key: 'x' },
    requestId: 'sdk-denied',
  })
  if (unknown.ok || unknown.errorCode !== 'plugin_not_installed') {
    throw new Error('Unknown plugin identity was not rejected')
  }
  const setValue = await invoke('plugin_capability_request', {
    pluginId: 'example.codex.page-sdk',
    gameName: 'EFMI',
    capability: 'plugin.settings.set',
    payload: { key: 'mode', value: 'fast' },
    requestId: 'sdk-set',
  })
  if (!setValue.ok) throw new Error(`SDK settings write failed: ${setValue.errorCode}`)
  const getValue = await invoke('plugin_capability_request', {
    pluginId: 'example.codex.page-sdk',
    gameName: 'EFMI',
    capability: 'plugin.settings.get',
    payload: { key: 'mode' },
    requestId: 'sdk-get',
  })
  if (!getValue.ok || getValue.value?.value !== 'fast') throw new Error('SDK settings round trip failed')

  const pages = await invoke('plugin_page_catalog', { gameName: 'EFMI' })
  const fixturePages = pages.filter(item => item.pluginId === 'example.codex.page-sdk')
  if (!fixturePages.some(item => item.kind === 'declarative')
    || !fixturePages.some(item => item.kind === 'local-service')) {
    throw new Error('SDK fixture pages were not registered generically')
  }

  const lease = await invoke('acquire_plugin_page_lease', {
    pluginId: 'example.codex.page-sdk',
    pageId: 'editor',
    gameName: 'EFMI',
  })
  const leaseUrl = new URL(lease.url)
  const token = new URLSearchParams(leaseUrl.hash.slice(1)).get('token') || ''
  if (!token) throw new Error('SDK fixture service URL is missing its instance token')
  const serviceHeaders = { 'X-Editor-Token': token }
  const health = await fetch(new URL('/api/health', leaseUrl), { headers: serviceHeaders })
  if (!health.ok) throw new Error(`SDK fixture service health failed: HTTP ${health.status}`)

  const leaseCapability = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.settings.get',
    payload: { key: 'mode' },
    requestId: 'sdk-lease-get',
  })
  if (!leaseCapability.ok || leaseCapability.value?.value !== 'fast') {
    throw new Error('Lease-bound capability request did not resolve the plugin identity')
  }
  const invalidLeaseCapability = await invoke('plugin_capability_request_for_lease', {
    leaseId: 'missing-lease',
    capability: 'plugin.settings.get',
    payload: { key: 'mode' },
    requestId: 'sdk-invalid-lease',
  })
  if (invalidLeaseCapability.ok || invalidLeaseCapability.errorCode !== 'lease_invalid') {
    throw new Error('Invalid ServiceLease identity was not rejected')
  }

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
    void router.push({ path: '/plugins', query: { plugin: 'example.codex.page-sdk' } })
    return true
  })()`)
  if (!pushed) throw new Error('Vue router is unavailable')
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'marketplace refresh action')
  const refreshed = await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (!(button instanceof HTMLElement)) return false
    button.click()
    return true
  })()`)
  if (!refreshed) throw new Error('Plugin marketplace refresh action was unavailable')
  await waitFor(`document.querySelector('.plugin-page-tabs')`, 'SDK page tabs')
  const selectedServiceTab = await evaluate(`(() => {
    const button = [...document.querySelectorAll('.plugin-page-tabs button')]
      .find(candidate => candidate.textContent?.includes('SDK 服务'))
    if (!button) return false
    button.click()
    return true
  })()`)
  if (!selectedServiceTab) throw new Error('SDK local-service page tab was not found')
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'SDK plugin iframe', 45000)

  let bridgeStatus = null
  const bridgeDeadline = Date.now() + 10000
  while (Date.now() < bridgeDeadline) {
    const response = await fetch(new URL('/api/bridge-status', leaseUrl), { headers: serviceHeaders })
    if (!response.ok) throw new Error(`SDK bridge status failed: HTTP ${response.status}`)
    const candidate = await response.json()
    if (candidate.reason !== 'bridge not called') {
      bridgeStatus = candidate
      break
    }
    await sleep(200)
  }
  if (!bridgeStatus?.ok) {
    throw new Error(`Plugin iframe capability bridge failed: ${JSON.stringify(bridgeStatus)}`)
  }

  await send('Page.bringToFront')
  await evaluate(`document.querySelector('.local-service-page')?.scrollIntoView({ block: 'start' })`)
  await sleep(500)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-plugin-sdk-ui.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'example.codex.page-sdk',
    enabled: false,
    gameExecutable: null,
  })
  const disabled = await invoke('plugin_capability_request', {
    pluginId: 'example.codex.page-sdk',
    gameName: 'EFMI',
    capability: 'plugin.settings.get',
    payload: { key: 'mode' },
    requestId: 'sdk-disabled',
  })
  if (disabled.ok || disabled.errorCode !== 'plugin_disabled') {
    throw new Error('Disabled plugin capability request was not rejected')
  }

  await invoke('stop_plugin_page_services_for_plugin', { pluginId: 'example.codex.page-sdk' })
  await invoke('uninstall_plugin', {
    pluginId: 'example.codex.page-sdk',
    gameName: 'EFMI',
  })
  const afterUninstall = await invoke('plugin_registry_snapshot')
  if (afterUninstall.some(item => item.manifest?.id === 'example.codex.page-sdk')) {
    throw new Error('SDK fixture remains installed after uninstall')
  }

  console.log(JSON.stringify({
    plugin: `${local.inspection.manifest.id}@${local.inspection.manifest.version}`,
    schemaVersion: local.inspection.manifest.schemaVersion,
    pluginApi: local.inspection.manifest.compatibility.pluginApi,
    capabilityApiVersion: catalog.apiVersion,
    pages: fixturePages.map(item => ({ id: item.pageId, kind: item.kind })),
    iframeBridge: bridgeStatus,
    screenshotPath,
    disabledRequestRejected: true,
    invalidLeaseRejected: true,
    serviceProcessId: lease.processId,
    uninstalled: true,
  }, null, 2))
} finally {
  socket.close()
}
