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
const send = (method, params = {}, sessionId = null) => {
  const id = ++sequence
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject })
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
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
let fixtureCleanup = async () => {}
const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
const waitFor = async (expression, label, timeoutMs = 45000) => {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await evaluate(`Boolean(${expression})`)) return
    await sleep(150)
  }
  throw new Error(`Timed out waiting for ${label}`)
}
const evaluatePlugin = async (expression, timeoutMs = 15000) => {
  const deadline = Date.now() + timeoutMs
  let urls = []
  while (Date.now() < deadline) {
    const targets = (await (await fetch('http://127.0.0.1:9333/json')).json())
      .filter(target => target.type === 'iframe' && target.webSocketDebuggerUrl)
    urls = targets.map(item => item.url)
    const target = targets.find(candidate =>
      candidate.url?.includes('pluginId=ssmt.secondary-motion')
      && candidate.url?.includes('pageLeaseId='))
    if (target) {
      const attached = await send('Target.attachToTarget', {
        targetId: target.id,
        flatten: true,
      })
      const sessionId = attached.result?.sessionId
      if (!sessionId) throw new Error('Could not attach to Secondary Motion iframe target')
      await send('Runtime.enable', {}, sessionId)
      const response = await send('Runtime.evaluate', {
        expression,
        returnByValue: true,
        awaitPromise: true,
      }, sessionId)
      await send('Target.detachFromTarget', { sessionId }).catch(() => undefined)
      if (response.result?.exceptionDetails) {
        throw new Error(response.result.exceptionDetails.exception?.description || response.result.exceptionDetails.text)
      }
      return response.result?.result?.value
    }
    await sleep(150)
  }
  throw new Error(`Secondary Motion iframe frame was not found: ${urls.join(' | ')}`)
}
const waitForPlugin = async (expression, label, timeoutMs = 15000) => {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await evaluatePlugin(`Boolean(${expression})`, Math.max(1000, deadline - Date.now()))) return
    await sleep(150)
  }
  throw new Error(`Timed out waiting for ${label} inside Secondary Motion iframe`)
}

try {
  await send('Page.enable')
  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = packages.find(item =>
    item.inspection?.manifest?.id === 'ssmt.secondary-motion'
    && item.inspection?.manifest?.version === '3.3.4'
  )
  if (!local) throw new Error('Secondary Motion 3.3.4 package was not discovered')
  const installed = await invoke('plugin_registry_snapshot')
  if (!installed.some(item => item.manifest?.id === 'ssmt.secondary-motion'
    && item.manifest?.version === '3.3.4')) {
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
      pluginVersion: '3.3.4',
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
  if (!String(lease.url || '').startsWith('http://127.0.0.1:')) {
    throw new Error(`Static page was not isolated behind a loopback server: ${JSON.stringify(lease)}`)
  }
  const leaseTouched = await invoke('touch_plugin_page_lease', { leaseId: lease.leaseId })
  if (leaseTouched !== true) throw new Error('Static page lease heartbeat was not accepted')
  const listed = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.list',
    payload: { dataId: 'SecondaryMotion', path: '' },
    requestId: 'secondary-data-list',
  })
  if (!listed.ok || !listed.value?.entries?.some(entry => entry.path === 'runtime')) {
    throw new Error(`Secondary Motion scoped data was unavailable: ${JSON.stringify(listed)}`)
  }
  const statusRead = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.read',
    payload: { dataId: 'SecondaryMotion', path: 'runtime/runtime_status.json' },
    requestId: 'secondary-status-fixture-read',
  })
  const fixtureStatus = {
    state: 'READY',
    character: 'chr_0016_laevat',
    profile: true,
    bones: true,
    mode: 'synthetic',
    gait: 'walk',
    last_write_ms: 0.42,
  }
  const statusFixtureWrite = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.write',
    payload: {
      dataId: 'SecondaryMotion',
      path: 'runtime/runtime_status.json',
      content: `${JSON.stringify(fixtureStatus, null, 2)}\n`,
    },
    requestId: 'secondary-status-fixture-write',
  })
  if (!statusFixtureWrite.ok) throw new Error('Secondary Motion status fixture could not be written')
  await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.publish',
    payload: { dataId: 'SecondaryMotion' },
    requestId: 'secondary-status-fixture-publish',
  })
  fixtureCleanup = async () => {
    const cleanupLease = await invoke('acquire_static_plugin_page_lease', {
      pluginId: 'ssmt.secondary-motion',
      pageId: 'settings',
      gameName: 'EFMI',
    })
    try {
      if (statusRead.ok) {
        await invoke('plugin_capability_request_for_lease', {
          leaseId: cleanupLease.leaseId,
          capability: 'plugin.data.write',
          payload: {
            dataId: 'SecondaryMotion',
            path: 'runtime/runtime_status.json',
            content: statusRead.value.content,
          },
          requestId: 'secondary-status-fixture-restore',
        })
      } else {
        await invoke('plugin_capability_request_for_lease', {
          leaseId: cleanupLease.leaseId,
          capability: 'plugin.data.remove',
          payload: { dataId: 'SecondaryMotion', path: 'runtime/runtime_status.json' },
          requestId: 'secondary-status-fixture-remove',
        })
      }
      await invoke('plugin_capability_request_for_lease', {
        leaseId: cleanupLease.leaseId,
        capability: 'plugin.data.publish',
        payload: { dataId: 'SecondaryMotion' },
        requestId: 'secondary-status-fixture-cleanup-publish',
      })
    } finally {
      await invoke('release_static_plugin_page_lease', { leaseId: cleanupLease.leaseId })
    }
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
  const legacyConfig = { ...originalConfig, enabled: false }
  delete legacyConfig.enabled_user_set
  const legacyWrite = await invoke('plugin_capability_request_for_lease', {
    leaseId: lease.leaseId,
    capability: 'plugin.data.write',
    payload: {
      dataId: 'SecondaryMotion',
      path: 'runtime/config.json',
      content: `${JSON.stringify(legacyConfig, null, 2)}\n`,
    },
    requestId: 'secondary-legacy-disabled',
  })
  if (!legacyWrite.ok) throw new Error('Secondary Motion legacy config fixture could not be written')
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
  await waitForPlugin(`document.querySelector('.manager-layout .manager-sidebar')`, 'original manager layout')
  const layout = await evaluatePlugin(`(() => {
    const root = document.querySelector('.secondary-motion-settings')
    const sidebar = document.querySelector('.manager-sidebar')
    const editor = document.querySelector('.manager-editor')
    const gaitTable = document.querySelector('.gait-table')
    const toolbar = document.querySelector('.editor-toolbar')
    return {
      rootWidth: root?.getBoundingClientRect().width || 0,
      sidebarWidth: sidebar?.getBoundingClientRect().width || 0,
      editorWidth: editor?.getBoundingClientRect().width || 0,
      hasGaitRows: (gaitTable?.querySelectorAll('tbody tr').length || 0) >= 4,
      hasVisibleActions: Boolean(toolbar?.querySelector('.el-button')),
      riskCollapsed: !document.querySelector('.risk-notice')?.hasAttribute('open'),
    }
  })()`)
  if (layout.sidebarWidth < 190 || layout.editorWidth <= layout.sidebarWidth
    || !layout.hasGaitRows || !layout.hasVisibleActions || !layout.riskCollapsed) {
    throw new Error(`Secondary Motion manager layout regression: ${JSON.stringify(layout)}`)
  }
  const hostFrame = await evaluate(`(() => {
    const frame = document.querySelector('.plugin-page-frame.ready')
    const wrap = frame?.parentElement
    const scaleRoot = document.querySelector('.app-ui-scale')
    return frame && wrap && scaleRoot ? {
      scale: Number.parseFloat(getComputedStyle(scaleRoot).getPropertyValue('--app-ui-scale')) || 1,
      frameWidth: frame.getBoundingClientRect().width,
      frameHeight: frame.getBoundingClientRect().height,
      wrapWidth: wrap.getBoundingClientRect().width,
      wrapHeight: wrap.getBoundingClientRect().height,
      zoom: Number.parseFloat(getComputedStyle(frame).zoom) || 1,
    } : null
  })()`)
  const frameViewport = await evaluatePlugin(
    `({ width: innerWidth, height: innerHeight, devicePixelRatio })`,
  )
  const frameScaleHealthy = hostFrame
    && Math.abs(hostFrame.zoom - hostFrame.scale) < 0.02
    && Math.abs(frameViewport.devicePixelRatio - hostFrame.scale) < 0.02
    && hostFrame.frameWidth <= hostFrame.wrapWidth + 4
    && hostFrame.frameHeight <= hostFrame.wrapHeight + 4
  if (!frameScaleHealthy) {
    throw new Error(`Plugin iframe scale compensation regression: ${JSON.stringify({
      hostFrame,
      frameViewport,
    })}`)
  }
  const staticIsolation = await evaluatePlugin(`(async () => {
    try {
      const response = await fetch('http://asset.localhost/C%3A%5CWindows%5Cwin.ini')
      return { blocked: false, status: response.status }
    } catch (error) {
      return { blocked: true, error: String(error) }
    }
  })()`)
  if (!staticIsolation?.blocked) {
    throw new Error(`Static plugin page escaped its isolated origin: ${JSON.stringify(staticIsolation)}`)
  }

  const migratedLease = await invoke('acquire_static_plugin_page_lease', {
    pluginId: 'ssmt.secondary-motion',
    pageId: 'settings',
    gameName: 'EFMI',
  })
  const migrationDeadline = Date.now() + 10000
  let migratedConfig = null
  while (Date.now() < migrationDeadline) {
    const response = await invoke('plugin_capability_request_for_lease', {
      leaseId: migratedLease.leaseId,
      capability: 'plugin.data.read',
      payload: { dataId: 'SecondaryMotion', path: 'runtime/config.json' },
      requestId: `secondary-default-enabled-${Date.now()}`,
    })
    if (response.ok) {
      const candidate = JSON.parse(response.value.content)
      if (candidate.enabled === true) {
        migratedConfig = candidate
        break
      }
    }
    await sleep(150)
  }
  await invoke('release_static_plugin_page_lease', { leaseId: migratedLease.leaseId })
  if (!migratedConfig || migratedConfig.enabled_user_set === true) {
    throw new Error(`Secondary Motion default-on migration failed: ${JSON.stringify(migratedConfig)}`)
  }
  if (migratedConfig.revision <= Number(originalConfig.revision || 0)) {
    throw new Error(`Secondary Motion default-on migration did not increment revision: ${JSON.stringify(migratedConfig)}`)
  }

  const switchLease = await invoke('acquire_static_plugin_page_lease', {
    pluginId: 'ssmt.secondary-motion',
    pageId: 'settings',
    gameName: 'EFMI',
  })
  const readSwitchConfig = async () => {
    const response = await invoke('plugin_capability_request_for_lease', {
      leaseId: switchLease.leaseId,
      capability: 'plugin.data.read',
      payload: { dataId: 'SecondaryMotion', path: 'runtime/config.json' },
      requestId: `secondary-switch-config-${Date.now()}`,
    })
    if (!response.ok) throw new Error(`Secondary Motion switch config read failed: ${response.errorCode}`)
    return JSON.parse(response.value.content)
  }
  await waitForPlugin(
    `document.querySelector('.global-switch .el-switch')?.classList.contains('is-checked')`,
    'motion switch default-on state',
  )
  const initialSwitch = await evaluatePlugin(`(() => {
    const element = document.querySelector('.global-switch .el-switch')
    return element ? {
      ariaChecked: element.getAttribute('aria-checked'),
      className: element.className,
      text: element.textContent,
    } : null
  })()`)
  if (!String(initialSwitch?.className || '').includes('is-checked')) {
    throw new Error(`Secondary Motion default-on switch is not visually enabled: ${JSON.stringify(initialSwitch)}`)
  }
  await evaluatePlugin(`document.querySelector('.global-switch .el-switch')?.click()`)
  const disabledDeadline = Date.now() + 10000
  let disabledConfig = null
  while (Date.now() < disabledDeadline) {
    const candidate = await readSwitchConfig()
    if (candidate.enabled === false && candidate.enabled_user_set === true) {
      disabledConfig = candidate
      break
    }
    await sleep(150)
  }
  if (!disabledConfig) throw new Error('Secondary Motion switch-off preference was not persisted immediately')

  await evaluate(`document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router?.push('/')`)
  await sleep(500)
  await evaluate(`document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router?.push({ path: '/plugins', query: { plugin: 'ssmt.secondary-motion' } })`)
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'Secondary Motion static iframe after navigation', 45000)
  await waitForPlugin(
    `document.querySelector('.global-switch .el-switch') && !document.querySelector('.global-switch .el-switch').classList.contains('is-checked')`,
    'persisted motion switch state after navigation',
  )
  if (String(await evaluatePlugin(`document.querySelector('.global-switch .el-switch')?.className || ''`)).includes('is-checked')) {
    throw new Error('Secondary Motion switch did not preserve the explicit off preference after remount')
  }
  await evaluatePlugin(`document.querySelector('.global-switch .el-switch')?.click()`)
  const enabledDeadline = Date.now() + 10000
  let enabledConfig = null
  while (Date.now() < enabledDeadline) {
    const candidate = await readSwitchConfig()
    if (candidate.enabled === true && candidate.enabled_user_set === true) {
      enabledConfig = candidate
      break
    }
    await sleep(150)
  }
  if (!enabledConfig) throw new Error('Secondary Motion switch-on preference was not persisted')
  await invoke('release_static_plugin_page_lease', { leaseId: switchLease.leaseId })

  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'ssmt.secondary-motion',
    enabled: false,
    gameExecutable: null,
  })
  const disabledInjection = await invoke('plugin_launch_injected_dlls', { gameName: 'EFMI' })
  if (disabledInjection.dlls?.some(item => item.pluginId === 'ssmt.secondary-motion')) {
    throw new Error('Secondary Motion remained injectable after the plugin was disabled')
  }
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'EFMI',
    id: 'ssmt.secondary-motion',
    enabled: true,
    gameExecutable: null,
  })
  const enabledInjection = await invoke('plugin_launch_injected_dlls', { gameName: 'EFMI' })
  if (!enabledInjection.dlls?.some(item => item.pluginId === 'ssmt.secondary-motion')) {
    throw new Error('Secondary Motion injection did not resume after the plugin was enabled')
  }

  await send('Page.bringToFront')
  await evaluate(`document.querySelector('.plugin-page-frame-wrap')?.scrollIntoView({ block: 'start' })`)
  await sleep(1000)
  const screenshot = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: false,
    captureBeyondViewport: false,
  })
  const screenshotPath = 'D:/chatgpt工作区/许愿/ssmt-secondary-motion-3.3.4.png'
  if (screenshot.result?.data) {
    const { writeFile } = await import('node:fs/promises')
    await writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'))
  }

  console.log(JSON.stringify({
    package: 'ssmt.secondary-motion@3.3.4',
    layout,
    hostFrame,
    frameViewport,
    staticIsolation,
    page,
    staticLease: lease,
    dataEntries: listed.value.entries,
    injectedDlls: injected.dlls,
    runtimePublished: true,
    defaultEnabledMigration: migratedConfig,
    switchPersistence: {
      disabled: disabledConfig,
      enabled: enabledConfig,
    },
    pluginLifecycle: {
      disabledInjection: disabledInjection.dlls || [],
      enabledInjection: enabledInjection.dlls || [],
    },
    screenshotPath,
  }, null, 2))
} finally {
  await fixtureCleanup().catch(() => undefined)
  socket.close()
}
