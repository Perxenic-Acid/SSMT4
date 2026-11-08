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

async function switchToGimi() {
  if (!await evaluate(`document.querySelector('.game-switcher-button')`)) {
    await evaluate(`(() => {
      const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
      void router?.push('/plugins')
    })()`)
    await waitFor(`document.querySelector('.game-switcher-button')`, 'game switcher button')
  }
  const current = await evaluate(`document.querySelector('.game-switcher-button')?.innerText || ''`)
  if (current.includes('(GIMI)')) return
  await evaluate(`document.querySelector('.game-switcher-button')?.click()`)
  await waitFor(`document.querySelector('.game-switcher-card')`, 'game switcher')
  const selected = await evaluate(`(() => {
    const button = [...document.querySelectorAll('.game-switcher-item')]
      .find(candidate => candidate.textContent?.includes('(GIMI)'))
    if (!button) return false
    button.click()
    return true
  })()`)
  if (!selected) throw new Error('GIMI game profile was not found')
  await sleep(1500)
}

async function switchToSrmi() {
  if (!await evaluate(`document.querySelector('.game-switcher-button')`)) {
    await evaluate(`(() => {
      const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
      void router?.push('/plugins')
    })()`)
    await waitFor(`document.querySelector('.game-switcher-button')`, 'game switcher button')
  }
  const current = await evaluate(`document.querySelector('.game-switcher-button')?.innerText || ''`)
  if (current.includes('(SRMI)')) return
  await evaluate(`document.querySelector('.game-switcher-button')?.click()`)
  await waitFor(`document.querySelector('.game-switcher-card')`, 'game switcher')
  const selected = await evaluate(`(() => {
    const button = [...document.querySelectorAll('.game-switcher-item')]
      .find(candidate => candidate.textContent?.includes('(SRMI)'))
    if (!button) return false
    button.click()
    return true
  })()`)
  if (!selected) throw new Error('SRMI game profile was not found')
  await sleep(1500)
}

async function switchToEfmi() {
  if (!await evaluate(`document.querySelector('.game-switcher-button')`)) {
    await evaluate(`(() => {
      const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
      void router?.push('/plugins')
    })()`)
    await waitFor(`document.querySelector('.game-switcher-button')`, 'game switcher button')
  }
  const current = await evaluate(`document.querySelector('.game-switcher-button')?.innerText || ''`)
  if (current.includes('(EFMI)')) return
  await evaluate(`document.querySelector('.game-switcher-button')?.click()`)
  await waitFor(`document.querySelector('.game-switcher-card')`, 'game switcher')
  const selected = await evaluate(`(() => {
    const button = [...document.querySelectorAll('.game-switcher-item')]
      .find(candidate => candidate.textContent?.includes('(EFMI)'))
    if (!button) return false
    button.click()
    return true
  })()`)
  if (!selected) throw new Error('EFMI game profile was not found')
  await sleep(1500)
}

try {
  await send('Page.enable')
  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = packages.find(item =>
    item.inspection?.manifest?.id === 'ssmt.player-tweaks'
    && item.inspection?.manifest?.version === '0.2.0'
  )
  if (!local) throw new Error('Player Tweaks 0.2.0 package was not discovered')

  const installed = await invoke('plugin_registry_snapshot')
  if (!installed.some(item => item.manifest?.id === 'ssmt.player-tweaks'
    && item.manifest?.version === '0.2.0')) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_setting', {
    pluginId: 'ssmt.player-tweaks',
    key: 'native_injection_risk_ack_v1',
    value: { accepted: true, pluginVersion: '0.2.0' },
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'GIMI',
    id: 'ssmt.player-tweaks',
    enabled: true,
    gameExecutable: null,
  })
  await switchToGimi()

  const pages = await invoke('plugin_page_catalog', { gameName: 'GIMI' })
  const page = pages.find(item =>
    item.pluginId === 'ssmt.player-tweaks'
    && item.pageId === 'settings'
    && item.kind === 'static'
  )
  if (!page) throw new Error('Player Tweaks did not expose a generic static page')

  const context = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.player-tweaks',
    gameName: 'GIMI',
    capability: 'game.context.read',
    payload: {},
    requestId: 'player-tweaks-context',
  })
  if (!context.ok || String(context.value?.gamePreset).toUpperCase() !== 'GIMI') {
    throw new Error(`Player Tweaks game context mismatch: ${JSON.stringify(context)}`)
  }

  const lease = await invoke('acquire_static_plugin_page_lease', {
    pluginId: 'ssmt.player-tweaks',
    pageId: 'settings',
    gameName: 'GIMI',
  })
  const migration = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.migration.read',
    payload: { migrationId: 'legacy-game-config' },
    requestId: 'player-tweaks-legacy',
  })
  if (!migration.ok) throw new Error(`Player Tweaks legacy migration failed: ${migration.errorCode}`)
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
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.player-tweaks' } })
  })()`)
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'marketplace refresh action')
  await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (button instanceof HTMLElement) button.click()
  })()`)
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'Player Tweaks static iframe')
  await sleep(1200)
  const gimiRuntimeConfig = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.player-tweaks',
    gameName: 'GIMI',
    capability: 'plugin.data.runtime.read',
    payload: { dataId: 'PlayerTweaks', path: 'SSMT-Player-Tweaks.ini' },
    requestId: 'player-tweaks-gimi-config',
  })
  if (!gimiRuntimeConfig.ok
    || !gimiRuntimeConfig.value?.content?.includes('[Camera]')
    || !gimiRuntimeConfig.value.content.includes('[Gameplay]')) {
    throw new Error(`GIMI runtime config was not generated: ${JSON.stringify(gimiRuntimeConfig)}`)
  }

  const runtimeEntries = await invoke('plugin_page_catalog', { gameName: 'GIMI' })
  const runtimePage = runtimeEntries.find(item =>
    item.pluginId === 'ssmt.player-tweaks' && item.pageId === 'settings')
  if (!runtimePage) throw new Error('Player Tweaks page disappeared during UI load')

  const injected = await invoke('plugin_launch_injected_dlls', { gameName: 'GIMI' })
  const playerDll = injected.dlls?.find(item => item.pluginId === 'ssmt.player-tweaks')
  if (!playerDll || injected.warnings?.length) {
    throw new Error(`Player Tweaks injection preflight failed: ${JSON.stringify(injected)}`)
  }

  await send('Page.bringToFront')
  await evaluate(`document.querySelector('.plugin-page-frame-wrap')?.scrollIntoView({ block: 'start' })`)
  await sleep(500)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-player-tweaks-0.2.0.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  await invoke('set_plugin_enabled_for_game', {
    gameName: 'SRMI',
    id: 'ssmt.player-tweaks',
    enabled: true,
    gameExecutable: null,
  })
  await switchToSrmi()
  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router?.push('/')
  })()`)
  await sleep(300)
  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router?.push({ path: '/plugins', query: { plugin: 'ssmt.player-tweaks' } })
  })()`)
  await waitFor(`document.querySelector('.marketplace-toolbar .toolbar-action')`, 'SRMI marketplace')
  await evaluate(`(() => {
    const button = document.querySelector('.marketplace-toolbar .toolbar-action')
    if (button instanceof HTMLElement) button.click()
  })()`)
  await waitFor(`document.querySelector('.plugin-page-frame.ready')?.src?.includes('game=SRMI')`, 'SRMI static iframe')
  await sleep(1200)
  const srmiSettings = await invoke('plugin_capability_request', {
    pluginId: 'ssmt.player-tweaks',
    gameName: 'SRMI',
    capability: 'plugin.data.read',
    payload: {
      dataId: 'PlayerTweaks',
      path: `games/${[...new TextEncoder().encode('SRMI')].map(value => value.toString(16).padStart(2, '0')).join('')}/settings.json`,
    },
    requestId: 'player-tweaks-srmi-settings',
  })
  if (!srmiSettings.ok || !srmiSettings.value?.content?.includes('fpsUnlock')) {
    throw new Error(`SRMI settings were not initialized: ${JSON.stringify(srmiSettings)}`)
  }
  await switchToEfmi()

  console.log(JSON.stringify({
    package: 'ssmt.player-tweaks@0.2.0',
    gameContext: context.value,
    page,
    legacyMigration: migration.value,
    injectedDll: playerDll,
    gimiRuntimeConfig: true,
    srmiSettingsInitialized: true,
    screenshotPath,
  }, null, 2))
} finally {
  await invoke('stop_all_plugin_page_services').catch(() => 0)
  socket.close()
}
