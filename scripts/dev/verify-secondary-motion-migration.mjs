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
    item.inspection?.manifest?.id === 'ssmt.secondary-motion'
    && item.inspection?.manifest?.version === '3.3.1'
  )
  if (!local) throw new Error('Secondary Motion 3.3.1 package was not discovered')
  const installed = await invoke('plugin_registry_snapshot')
  if (!installed.some(item => item.manifest?.id === 'ssmt.secondary-motion'
    && item.manifest?.version === '3.3.1')) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_setting', {
    pluginId: 'ssmt.secondary-motion',
    key: 'native_injection_risk_ack_v1',
    value: {
      accepted: true,
      pluginVersion: '3.3.1',
    },
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'ssmt.secondary-motion',
    enabled: true,
    gameExecutable: null,
  })

  const pages = await invoke('plugin_page_catalog', { gameName: 'EFMI' })
  const page = pages.find(item =>
    item.pluginId === 'ssmt.secondary-motion'
    && item.pageId === 'settings'
    && item.kind === 'static'
  )
  if (!page) throw new Error('Secondary Motion did not expose a generic static settings page')

  const lease = await invoke('acquire_static_plugin_page_lease', {
    pluginId: 'ssmt.secondary-motion',
    pageId: 'settings',
    gameName: 'EFMI',
  })
  const listed = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.list',
    payload: { dataId: 'SecondaryMotion', path: '' },
    requestId: 'secondary-data-list',
  })
  if (!listed.ok || !listed.value?.entries?.some(entry => entry.path === 'runtime')) {
    throw new Error(`Secondary Motion scoped data was unavailable: ${JSON.stringify(listed)}`)
  }
  const configRead = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.read',
    payload: { dataId: 'SecondaryMotion', path: 'runtime/config.json' },
    requestId: 'secondary-config-read',
  })
  if (!configRead.ok) throw new Error(`Secondary Motion config read failed: ${configRead.errorCode}`)
  const configWrite = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.write',
    payload: {
      dataId: 'SecondaryMotion',
      path: 'runtime/config.json',
      content: configRead.value.content,
    },
    requestId: 'secondary-config-write',
  })
  if (!configWrite.ok) throw new Error(`Secondary Motion config write failed: ${configWrite.errorCode}`)
  const publish = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.publish',
    payload: { dataId: 'SecondaryMotion' },
    requestId: 'secondary-publish',
  })
  if (!publish.ok || publish.value?.published !== true) {
    throw new Error('Secondary Motion runtime data mirror was not published')
  }
  const originalConfig = JSON.parse(configRead.value.content)
  const markedConfig = { ...originalConfig, e2eMirrorMarker: `probe-${Date.now()}` }
  await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.write',
    payload: {
      dataId: 'SecondaryMotion',
      path: 'runtime/config.json',
      content: `${JSON.stringify(markedConfig, null, 2)}\n`,
    },
    requestId: 'secondary-mirror-write',
  })
  const injected = await invoke('plugin_launch_injected_dlls', { gameName: 'EFMI' })
  if (injected.warnings?.length || !injected.dlls?.some(item => item.pluginId === 'ssmt.secondary-motion')) {
    throw new Error(`Secondary Motion injection preflight failed: ${JSON.stringify(injected)}`)
  }
  const runtimeConfig = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.runtime.read',
    payload: { dataId: 'SecondaryMotion', path: 'runtime/config.json' },
    requestId: 'secondary-mirror-read',
  })
  if (!runtimeConfig.ok || JSON.parse(runtimeConfig.value.content).e2eMirrorMarker !== markedConfig.e2eMirrorMarker) {
    throw new Error('Generic launch preflight did not mirror Secondary Motion data')
  }
  await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.write',
    payload: {
      dataId: 'SecondaryMotion',
      path: 'runtime/config.json',
      content: configRead.value.content,
    },
    requestId: 'secondary-mirror-restore',
  })
  await invoke('plugin_launch_injected_dlls', { gameName: 'EFMI' })
  await invoke('release_static_plugin_page_lease', { leaseId: lease.leaseId })

  const resetRoute = await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push('/')
    return true
  })()`)
  if (!resetRoute) throw new Error('Vue router is unavailable')
  await sleep(300)
  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.secondary-motion' } })
  })()`)
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'marketplace refresh action')
  await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (button instanceof HTMLElement) button.click()
  })()`)
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'Secondary Motion static iframe')

  await send('Page.bringToFront')
  await evaluate(`document.querySelector('.plugin-page-frame-wrap')?.scrollIntoView({ block: 'start' })`)
  await sleep(1000)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-secondary-motion-3.3.1.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  console.log(JSON.stringify({
    package: 'ssmt.secondary-motion@3.3.1',
    page,
    staticLease: lease,
    dataEntries: listed.value.entries,
    injectedDlls: injected.dlls,
    runtimePublished: true,
    screenshotPath,
  }, null, 2))
} finally {
  socket.close()
}
