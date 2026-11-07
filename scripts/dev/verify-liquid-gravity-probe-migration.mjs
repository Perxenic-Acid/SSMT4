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
const waitFor = async (expression, label, timeoutMs = 45000) => {
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
  const local = packages.find(item =>
    item.inspection?.manifest?.id === 'ssmt.liquid-gravity-probe'
    && item.inspection?.manifest?.version === '0.2.33'
  )
  if (!local) throw new Error('Liquid Gravity Probe 0.2.33 package was not discovered')

  const installed = await invoke('plugin_registry_snapshot')
  const currentInstalled = installed.some(item =>
    item.manifest?.id === 'ssmt.liquid-gravity-probe'
    && item.manifest?.version === '0.2.33'
  )
  if (!currentInstalled) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_external_dependency_path', {
    pluginId: 'ssmt.liquid-gravity-probe',
    dependencyId: 'python-runtime',
    path: 'F:\\Programs\\Python\\Python314',
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'ssmt.liquid-gravity-probe',
    enabled: true,
    gameExecutable: null,
  })
  await invoke('stop_plugin_page_services_for_plugin', { pluginId: 'ssmt.liquid-gravity-probe' })

  const pages = await invoke('plugin_page_catalog', { gameName: 'EFMI' })
  const page = pages.find(item =>
    item.pluginId === 'ssmt.liquid-gravity-probe'
    && item.pageId === 'editor'
    && item.kind === 'local-service'
  )
  if (!page) throw new Error('Liquid Gravity Probe did not expose a generic local-service page')

  const lease = await invoke('acquire_plugin_page_lease', {
    pluginId: 'ssmt.liquid-gravity-probe',
    pageId: 'editor',
    gameName: 'EFMI',
  })
  const leaseUrl = new URL(lease.url)
  const token = new URLSearchParams(leaseUrl.hash.slice(1)).get('token') || ''
  if (!token) throw new Error('Liquid Gravity Probe service URL has no instance token')
  const headers = { 'X-Editor-Token': token }
  const healthResponse = await fetch(new URL('/api/health', leaseUrl), { headers })
  if (!healthResponse.ok) throw new Error(`Liquid Gravity Probe health failed: HTTP ${healthResponse.status}`)
  const health = await healthResponse.json()
  if (health.service !== 'liquid-gravity-probe-editor' || health.protocolVersion !== 1) {
    throw new Error('Liquid Gravity Probe health response did not match protocol 1')
  }
  const uiResponse = await fetch(new URL('/ui/index.html', leaseUrl))
  if (!uiResponse.ok) throw new Error(`Liquid Gravity Probe UI failed: HTTP ${uiResponse.status}`)
  const uiHtml = await uiResponse.text()
  if (!uiHtml.includes('./assets/')) throw new Error('Liquid Gravity Probe UI assets were not built')

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
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.liquid-gravity-probe' } })
    return true
  })()`)
  if (!pushed) throw new Error('Vue router is unavailable')
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'marketplace refresh action')
  await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (button instanceof HTMLElement) button.click()
  })()`)
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'generic Liquid Gravity Probe iframe')

  await send('Page.bringToFront')
  await evaluate(`document.querySelector('.local-service-page')?.scrollIntoView({ block: 'start' })`)
  await sleep(500)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-liquid-gravity-probe-0.2.33.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  const stopped = await invoke('stop_plugin_page_service', { sessionId: lease.leaseId })
  const legacyNavigation = await evaluate(`(async () => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return null
    await router.push('/plugins/liquid-gravity-probe')
    await new Promise(resolve => setTimeout(resolve, 300))
    return {
      path: location.pathname,
      plugin: new URLSearchParams(location.search).get('plugin'),
      dedicatedNavPresent: Boolean(document.querySelector('[data-nav-id="liquid-gravity-probe"]')),
    }
  })()`)
  if (legacyNavigation?.path !== '/plugins'
    || legacyNavigation?.plugin !== 'ssmt.liquid-gravity-probe'
    || legacyNavigation?.dedicatedNavPresent) {
    throw new Error(`Legacy Liquid Gravity navigation was not redirected: ${JSON.stringify(legacyNavigation)}`)
  }
  console.log(JSON.stringify({
    package: 'ssmt.liquid-gravity-probe@0.2.33',
    schemaVersion: 2,
    page,
    session: {
      serviceKey: lease.serviceKey,
      processId: lease.processId,
      dataRoot: lease.dataRoot,
      logPath: lease.logPath,
    },
    screenshotPath,
    legacyNavigation,
    stopped,
  }, null, 2))
} finally {
  socket.close()
}
