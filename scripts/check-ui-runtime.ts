import { mkdir } from 'node:fs/promises'
import { join } from 'node:path'

type CdpMessage = {
  id?: number
  method?: string
  params?: Record<string, unknown>
  result?: Record<string, any>
  error?: { message?: string }
}

type RouteCase = {
  name: string
  path: string
  navId?: string
  selector: string
}

type Mode = 'modern' | 'classic'

const routeCases: RouteCase[] = [
  { name: 'home', path: '/', navId: 'home', selector: '.home-container' },
  { name: 'work', path: '/work', navId: 'work', selector: '.work-page-container' },
  { name: 'mark-texture', path: '/mark-texture-full', navId: 'mark-texture-full', selector: '.mark-texture-page' },
  { name: 'texture-mod-maker', path: '/texture-mod-maker', navId: 'texture-mod-maker', selector: '.tm-page' },
  { name: 'mods', path: '/mods', navId: 'mods', selector: '.mod-manager' },
  { name: 'gamebanana', path: '/gamebanana', navId: 'gamebanana', selector: '.gamebanana-page' },
  { name: 'nexusmods', path: '/nexusmods', navId: 'nexusmods', selector: '.nexusmods-page' },
  { name: 'xianzun', path: '/xianzun', navId: 'xianzun', selector: '.xz-page' },
  { name: 'ui-builder', path: '/ui-builder', navId: 'ui-builder', selector: '.ui-builder-app' },
  { name: 'plugins', path: '/plugins', navId: 'plugins', selector: '.plugin-marketplace' },
  { name: 'settings', path: '/settings', selector: '.settings-page' },
]

const targets = await (await fetch('http://127.0.0.1:9333/json')).json() as Array<{
  title?: string
  url?: string
  webSocketDebuggerUrl?: string
}>
const target = targets.find(item => item.url?.includes('localhost:1422') && item.webSocketDebuggerUrl)
if (!target?.webSocketDebuggerUrl) throw new Error('SSMT4 WebView target was not found on CDP port 9333')

const socket = new WebSocket(target.webSocketDebuggerUrl)
await new Promise<void>((resolve, reject) => {
  socket.addEventListener('open', () => resolve(), { once: true })
  socket.addEventListener('error', () => reject(new Error('Could not connect to the SSMT4 CDP target')), { once: true })
})

let sequence = 0
const pending = new Map<number, {
  resolve: (value: CdpMessage) => void
  reject: (reason: Error) => void
}>()
const runtimeExceptions: string[] = []
const consoleErrors: string[] = []
const seenRuntimeExceptions = new Set<string>()
const seenConsoleErrors = new Set<string>()
let ignoreRuntimeErrors = false

const rememberRuntimeException = (description: string) => {
  if (ignoreRuntimeErrors) return
  const normalized = description.trim()
  const key = normalized.split('\n').slice(0, 2).join('\n')
  if (!key || seenRuntimeExceptions.has(key)) return
  seenRuntimeExceptions.add(key)
  runtimeExceptions.push(normalized)
}

const rememberConsoleError = (description: string) => {
  if (ignoreRuntimeErrors) return
  const normalized = description.trim()
  if (!normalized || seenConsoleErrors.has(normalized)) return
  seenConsoleErrors.add(normalized)
  consoleErrors.push(normalized)
}

socket.addEventListener('message', event => {
  const message = JSON.parse(String(event.data)) as CdpMessage
  if (message.id) {
    const request = pending.get(message.id)
    if (!request) return
    pending.delete(message.id)
    if (message.error) request.reject(new Error(message.error.message || 'CDP request failed'))
    else request.resolve(message)
    return
  }
  if (message.method === 'Runtime.exceptionThrown') {
    const detail = message.params?.exceptionDetails as any
    rememberRuntimeException(detail?.exception?.description || detail?.text || 'Unknown runtime exception')
  }
  if (message.method === 'Runtime.consoleAPICalled') {
    const type = message.params?.type
    if (type === 'error') {
      const args = message.params?.args as Array<{ value?: unknown; description?: unknown }> | undefined
      rememberConsoleError(args?.map(arg => arg.value ?? arg.description ?? '').join(' ') || 'console.error')
    }
  }
})

const send = (method: string, params: Record<string, unknown> = {}): Promise<CdpMessage> => {
  const id = ++sequence
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject })
    socket.send(JSON.stringify({ id, method, params }))
  })
}

const evaluate = async <T = any>(expression: string, awaitPromise = false): Promise<T> => {
  const response = await send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise,
  })
  if (response.result?.exceptionDetails) {
    throw new Error(response.result.exceptionDetails.exception?.description || response.result.exceptionDetails.text)
  }
  return response.result?.result?.value as T
}

const sleep = (milliseconds: number) =>
  new Promise<void>(resolve => setTimeout(resolve, milliseconds))

const waitFor = async (expression: string, label: string, timeoutMs = 8000): Promise<void> => {
  const deadline = Date.now() + timeoutMs
  let lastError = ''
  while (Date.now() < deadline) {
    try {
      if (await evaluate<boolean>(`Boolean(${expression})`)) return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
    }
    await sleep(100)
  }
  throw new Error(`Timed out waiting for ${label}${lastError ? `: ${lastError}` : ''}`)
}

const clearCapturedErrors = () => {
  runtimeExceptions.length = 0
  consoleErrors.length = 0
  seenRuntimeExceptions.clear()
  seenConsoleErrors.clear()
}

const hardReload = async (): Promise<void> => {
  ignoreRuntimeErrors = true
  clearCapturedErrors()
  await send('Page.reload', { ignoreCache: true })
  await sleep(750)
  await waitFor(`document.readyState === 'complete'`, 'hard page reload', 20000)
  await waitFor(
    `Boolean(document.querySelector('#app')?.__vue_app__?._instance?.setupState?.appSettings)`,
    'Vue app readiness after hard reload',
    20000,
  )
  await sleep(750)
  ignoreRuntimeErrors = false
  clearCapturedErrors()
}

const setMode = async (mode: Mode): Promise<void> => {
  const current = await evaluate<string>(`document.documentElement.dataset.uiMode || ''`)
  if (current === mode) return
  const changed = await evaluate<boolean>(`(() => {
    const settings = document.querySelector('#app')?.__vue_app__?._instance?.setupState?.appSettings
    if (!settings) return false
    settings.uiMode = ${JSON.stringify(mode)}
    return true
  })()`)
  if (!changed) throw new Error('Could not update the reactive UI mode')
  await waitFor(`document.documentElement.dataset.uiMode === '${mode}'`, `${mode} UI mode`)
  await sleep(150)
}

const setScale = async (percent: number): Promise<void> => {
  const changed = await evaluate<boolean>(`(() => {
    const settings = document.querySelector('#app')?.__vue_app__?._instance?.setupState?.appSettings
    if (!settings) return false
    settings.uiScale = ${percent / 100}
    return true
  })()`)
  if (!changed) throw new Error('Could not update the reactive UI scale')
  await waitFor(
    `Math.abs(Number(getComputedStyle(document.querySelector('.app-ui-scale')).getPropertyValue('--app-ui-scale')) - ${percent / 100}) < 0.005`,
    `${percent}% UI scale`,
  )
  await sleep(150)
}

const locationLike = (path: string): Promise<boolean> =>
  evaluate<boolean>(`location.pathname === ${JSON.stringify(path)}`)

const navigateTo = async (name: string): Promise<void> => {
  const route = routeCases.find(item => item.name === name)
  if (!route) throw new Error(`Unknown route case: ${name}`)
  if (await locationLike(route.path)) {
    await waitFor(`document.querySelector(${JSON.stringify(route.selector)})`, `${route.path} root`)
    return
  }
  const pushed = await evaluate<boolean>(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push(${JSON.stringify(route.path)})
    return true
  })()`)
  if (!pushed) throw new Error(`Vue router is unavailable while opening ${route.path}`)
  await waitFor(`location.pathname === ${JSON.stringify(route.path)}`, route.path)
  await waitFor(`document.querySelector(${JSON.stringify(route.selector)})`, `${route.path} root`)
  await sleep(route.name === 'ui-builder' ? 350 : 180)
}

const collectPageMetrics = async (route: RouteCase, mode: Mode, scale: number) => {
  await navigateTo(route.name)
  return evaluate<any>(`(() => {
    const root = document.querySelector(${JSON.stringify(route.selector)})
    const content = document.querySelector('.content-scroll-wrapper')
    const rect = root?.getBoundingClientRect()
    const contentRect = content?.getBoundingClientRect()
    return {
      route: ${JSON.stringify(route.path)},
      mode: ${JSON.stringify(mode)},
      scale: ${scale},
      path: location.pathname,
      rootFound: Boolean(root),
      rootWidth: rect?.width || 0,
      rootHeight: rect?.height || 0,
      contentWidth: contentRect?.width || 0,
      contentHeight: contentRect?.height || 0,
      bodyPaddingLeft: getComputedStyle(document.querySelector('.app-shell-body')).paddingLeft,
      navPosition: getComputedStyle(document.querySelector('.caption-tabs')).position,
      documentOverflowX: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    }
  })()`)
}

const capture = async (outputPath: string): Promise<void> => {
  const response = await send('Page.captureScreenshot', {
    format: 'png',
    fromSurface: true,
    captureBeyondViewport: false,
  })
  const data = response.result?.data
  if (!data) throw new Error(`Screenshot failed: ${outputPath}`)
  await Bun.write(outputPath, Buffer.from(data, 'base64'))
}

const setViewport = async (width: number, height: number): Promise<void> => {
  await send('Emulation.setDeviceMetricsOverride', {
    width,
    height,
    deviceScaleFactor: 1,
    mobile: false,
  })
  await sleep(120)
}

const clearViewport = async (): Promise<void> => {
  await send('Emulation.clearDeviceMetricsOverride')
  await sleep(120)
}

await send('Page.enable')
await send('Runtime.enable')
await hardReload()

const initialSettings = await evaluate<{ uiMode: Mode; uiScale: number }>(`(() => {
  const settings = document.querySelector('#app')?.__vue_app__?._instance?.setupState?.appSettings
  return { uiMode: settings?.uiMode || 'modern', uiScale: settings?.uiScale || 1 }
})()`)
const initialMode = initialSettings.uiMode
const initialScale = Math.round(initialSettings.uiScale * 100)

const stamp = new Date().toISOString().replace(/[:.]/g, '-')
const outputDir = `D:/chatgpt工作区/许愿/ssmt-ui-regression-${stamp}`
const screenshotDir = join(outputDir, 'screenshots')
await mkdir(screenshotDir, { recursive: true })

const results: any[] = []
const failures: string[] = []
let fatalError: unknown = null

const recordFailure = (message: string) => {
  failures.push(message)
  console.error(`FAIL: ${message}`)
}

try {
  for (const mode of ['modern', 'classic'] as const) {
    console.log(`MODE ${mode}`)
    await setMode(mode)
    await setScale(100)
    for (const route of routeCases) {
      console.log(`PAGE ${mode}/${route.name}`)
      try {
        const metrics = await collectPageMetrics(route, mode, 100)
        results.push(metrics)
        if (!metrics.rootFound || metrics.rootWidth < 20 || metrics.rootHeight < 20) {
          recordFailure(`${mode}/${route.name}: page root is not visible`)
        }
        if (metrics.documentOverflowX > 2) {
          recordFailure(`${mode}/${route.name}: document overflows horizontally by ${metrics.documentOverflowX}px`)
        }
        // The content is inside the scaled shell, so computed padding remains
        // the unscaled design value while its visual width follows UI scale.
        const expectedPadding = mode === 'modern' ? 74 : 0
        if (Math.abs(Number.parseFloat(metrics.bodyPaddingLeft) - expectedPadding) > 0.5) {
          recordFailure(`${mode}/${route.name}: unexpected content left padding ${metrics.bodyPaddingLeft}`)
        }
        await capture(join(screenshotDir, `${mode}-${route.name}.png`))
      } catch (error) {
        recordFailure(`${mode}/${route.name}: ${error instanceof Error ? error.message : String(error)}`)
      }
    }

    for (const scale of [70, 100, 120]) {
      console.log(`SCALE ${mode}/${scale}%`)
      await setScale(scale)
      for (const routeName of ['home', 'mods', 'plugins']) {
        const route = routeCases.find(item => item.name === routeName)!
        try {
          const metrics = await collectPageMetrics(route, mode, scale)
          results.push(metrics)
          const expectedPadding = mode === 'modern' ? 74 : 0
          if (Number.parseFloat(metrics.bodyPaddingLeft) !== expectedPadding) {
            recordFailure(`${mode}/${routeName}/${scale}%: padding ${metrics.bodyPaddingLeft}, expected ${expectedPadding}px`)
          }
          const navWidth = mode === 'modern' ? 74 * scale / 100 : 0
          const overlayLeft = Number.parseFloat(await evaluate<string>(
            `getComputedStyle(document.documentElement).getPropertyValue('--ssmt-content-overlay-left')`,
          ))
          if (Math.abs(overlayLeft - navWidth) > 0.5) {
            recordFailure(`${mode}/${routeName}/${scale}%: overlay left ${overlayLeft}, expected ${navWidth}`)
          }
          if (metrics.documentOverflowX > 2) {
            recordFailure(`${mode}/${routeName}/${scale}%: horizontal overflow ${metrics.documentOverflowX}px`)
          }
          if (scale === 120) {
            await capture(join(screenshotDir, `${mode}-${routeName}-${scale}.png`))
          }
        } catch (error) {
          recordFailure(`${mode}/${routeName}/${scale}%: ${error instanceof Error ? error.message : String(error)}`)
        }
      }
    }
  }

  await setMode('classic')
  await setScale(100)
  await navigateTo('settings')
  const selectedContrast = await evaluate<any>(`(() => {
    const selected = document.querySelector('.ui-mode-control .ui-mode-option.active')
    const indicator = selected?.querySelector('.ui-mode-indicator')
    return {
      text: selected?.innerText || '',
      color: selected ? getComputedStyle(selected).color : '',
      indicator: indicator ? getComputedStyle(indicator).backgroundColor : '',
    }
  })()`)
  results.push({ assertion: 'classic-segment-contrast', ...selectedContrast })
  if (!selectedContrast.text.includes('正式版') || selectedContrast.color === 'rgb(255, 255, 255)') {
    recordFailure(`Classic layout selector contrast is invalid: ${JSON.stringify(selectedContrast)}`)
  }

  await navigateTo('plugins')
  const modernStructure = await evaluate<any>(`({
    modernHeader: document.querySelectorAll('.marketplace-heading').length,
    compatibility: document.querySelectorAll('.compatibility-panel').length,
    legacySearch: document.querySelectorAll('.legacy-sidebar-search').length,
  })`)
  results.push({ assertion: 'classic-plugin-marketplace', ...modernStructure })
  if (modernStructure.modernHeader !== 0 || modernStructure.compatibility !== 0 || modernStructure.legacySearch !== 1) {
    recordFailure(`Classic plugin marketplace structure is incorrect: ${JSON.stringify(modernStructure)}`)
  }

  await navigateTo('home')
  const dragMetrics = await evaluate<any>(`(() => {
    const track = document.querySelector('.caption-tabs-track').getBoundingClientRect()
    const settings = document.querySelector('.side-nav-footer').getBoundingClientRect()
    const middle = track.right + (settings.left - track.right) / 2
    const hit = document.elementFromPoint(middle, 16)
    return {
      gap: settings.left - track.right,
      hitClass: hit?.className || '',
    }
  })()`)
  results.push({ assertion: 'classic-window-drag-area', ...dragMetrics })
  if (dragMetrics.gap < 200 || !String(dragMetrics.hitClass).includes('caption-drag-region')) {
    recordFailure(`Classic drag area did not span the free caption space: ${JSON.stringify(dragMetrics)}`)
  }

  await navigateTo('plugins')
  const shortcutCheck = await evaluate<any>(`(() => {
    const ctrlK = new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true, cancelable: true })
    const work = document.querySelector('.caption-tab[data-nav-id="work"]')
    work?.focus()
    work?.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }))
    return {
      ctrlKPrevented: !document.dispatchEvent(ctrlK),
      shiftF10Menu: Boolean(document.querySelector('.shell-flyout')),
    }
  })()`)
  results.push({ assertion: 'removed-shortcuts', ...shortcutCheck })
  if (shortcutCheck.ctrlKPrevented || shortcutCheck.shiftF10Menu) {
    recordFailure(`Removed shortcuts are still active: ${JSON.stringify(shortcutCheck)}`)
  }

  const searchSelector = '.marketplace-search input, .legacy-sidebar-search input'
  const originalSearch = await evaluate<string>(`document.querySelector(${JSON.stringify(searchSelector)})?.value || ''`)
  await evaluate(`(() => {
    const input = document.querySelector(${JSON.stringify(searchSelector)})
    if (!input) return false
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    setter?.call(input, '__keepalive_probe__')
    input.dispatchEvent(new Event('input', { bubbles: true }))
    return true
  })()`)
  await sleep(120)
  await navigateTo('home')
  await navigateTo('plugins')
  const restoredSearch = await evaluate<string>(`document.querySelector(${JSON.stringify(searchSelector)})?.value || ''`)
  results.push({ assertion: 'keep-alive-state', expected: '__keepalive_probe__', actual: restoredSearch })
  if (restoredSearch !== '__keepalive_probe__') {
    recordFailure(`KeepAlive search state was not preserved: ${restoredSearch}`)
  }
  await evaluate(`(() => {
    const input = document.querySelector(${JSON.stringify(searchSelector)})
    if (!input) return false
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    setter?.call(input, ${JSON.stringify(originalSearch)})
    input.dispatchEvent(new Event('input', { bubbles: true }))
    return true
  })()`)

  await setMode('modern')
  await setScale(100)
  await navigateTo('plugins')
  const modernPlugin = await evaluate<any>(`({
    modernHeader: document.querySelectorAll('.marketplace-heading').length,
    compatibility: document.querySelectorAll('.compatibility-panel').length,
    legacySearch: document.querySelectorAll('.legacy-sidebar-search').length,
  })`)
  results.push({ assertion: 'modern-plugin-marketplace', ...modernPlugin })
  if (modernPlugin.modernHeader !== 1 || modernPlugin.compatibility !== 1 || modernPlugin.legacySearch !== 0) {
    recordFailure(`Modern plugin marketplace structure is incorrect: ${JSON.stringify(modernPlugin)}`)
  }

  await navigateTo('work')
  const homeNavPoint = await evaluate<{ x: number; y: number } | null>(`(() => {
    const button = document.querySelector('.caption-tab[data-nav-id="home"]')
    if (!button) return null
    const rect = button.getBoundingClientRect()
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
  })()`)
  if (!homeNavPoint) {
    recordFailure('Modern home navigation button was not found')
  } else {
    await send('Input.dispatchMouseEvent', {
      type: 'mouseMoved',
      x: homeNavPoint.x,
      y: homeNavPoint.y,
      button: 'none',
      buttons: 0,
    })
    await send('Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x: homeNavPoint.x,
      y: homeNavPoint.y,
      button: 'left',
      buttons: 1,
      clickCount: 1,
    })
    await send('Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x: homeNavPoint.x,
      y: homeNavPoint.y,
      button: 'left',
      buttons: 0,
      clickCount: 1,
    })
    await waitFor(`location.pathname === '/'`, 'home navigation after pointer click')
    await send('Input.dispatchMouseEvent', {
      type: 'mouseMoved',
      x: 600,
      y: 400,
      button: 'none',
      buttons: 0,
    })
    await sleep(300)
    const pointerFocusState = await evaluate<any>(`(() => {
      const nav = document.querySelector('.caption-tabs')
      const active = document.activeElement
      const activeTab = document.querySelector('.caption-tab.active')
      return {
        activeNavId: active?.closest?.('.caption-tab')?.dataset?.navId || '',
        focusVisible: Boolean(active?.matches?.(':focus-visible')),
        backdropOpacity: nav ? Number(getComputedStyle(nav, '::before').opacity) : -1,
        navTabIndexes: Array.from(document.querySelectorAll('.caption-tab'), item => item.tabIndex),
        activeTabColor: activeTab ? getComputedStyle(activeTab).color : '',
      }
    })()`)
    results.push({ assertion: 'modern-nav-pointer-focus', ...pointerFocusState })
    if (pointerFocusState.focusVisible
      || pointerFocusState.backdropOpacity > 0.05
      || pointerFocusState.navTabIndexes.some((index: number) => index !== -1)
      || pointerFocusState.activeTabColor !== 'rgb(23, 25, 8)') {
      recordFailure(`Modern nav remained visible after pointer navigation: ${JSON.stringify(pointerFocusState)}`)
    }
  }

  for (const mode of ['modern', 'classic'] as const) {
    await setMode(mode)
    for (const [width, height] of [[1006, 600], [1400, 900], [1800, 1000]] as const) {
      await setViewport(width, height)
      for (const routeName of ['home', 'plugins', 'mods']) {
        const route = routeCases.find(item => item.name === routeName)!
        try {
          const metrics = await collectPageMetrics(route, mode, 100)
          results.push({ assertion: 'viewport', width, height, ...metrics })
          if (metrics.documentOverflowX > 2) {
            recordFailure(`${mode}/${routeName}@${width}x${height}: horizontal overflow ${metrics.documentOverflowX}px`)
          }
          if (metrics.rootWidth < 20 || metrics.rootHeight < 20) {
            recordFailure(`${mode}/${routeName}@${width}x${height}: content is not visible`)
          }
        } catch (error) {
          recordFailure(`${mode}/${routeName}@${width}x${height}: ${error instanceof Error ? error.message : String(error)}`)
        }
      }
      await clearViewport()
    }
  }

  if (runtimeExceptions.length) {
    recordFailure(`Runtime exceptions: ${runtimeExceptions.join(' | ')}`)
  }
  const meaningfulConsoleErrors = consoleErrors.filter(message =>
    /TypeError|ReferenceError|SyntaxError|Unhandled|Cannot read|undefined is not/.test(message),
  )
  if (meaningfulConsoleErrors.length) {
    recordFailure(`Console errors: ${meaningfulConsoleErrors.join(' | ')}`)
  }
} catch (error) {
  fatalError = error
  recordFailure(`Fatal runtime test error: ${error instanceof Error ? error.stack || error.message : String(error)}`)
} finally {
  try {
    await setMode(initialMode)
    await setScale(initialScale)
    await clearViewport()
  } catch (error) {
    recordFailure(`Failed to restore initial UI state: ${error instanceof Error ? error.message : String(error)}`)
  }
  socket.close()
}

const summary = {
  passed: failures.length === 0,
  generatedAt: new Date().toISOString(),
  initialMode,
  initialScale,
  routeCount: routeCases.length,
  resultCount: results.length,
  failureCount: failures.length,
  failures,
  runtimeExceptions,
  consoleErrors,
}

await Bun.write(join(outputDir, 'results.json'), JSON.stringify({ summary, results }, null, 2))
await Bun.write(join(outputDir, 'summary.md'), [
  '# SSMT4 UI regression',
  '',
  `- Result: ${summary.passed ? 'PASS' : 'FAIL'}`,
  `- Routes: ${summary.routeCount}`,
  `- Checks recorded: ${summary.resultCount}`,
  `- Failures: ${summary.failureCount}`,
  `- Initial mode: ${summary.initialMode}`,
  `- Initial scale: ${summary.initialScale}%`,
  '',
  ...(failures.length ? ['## Failures', '', ...failures.map(item => `- ${item}`)] : ['All runtime checks passed.']),
  '',
].join('\n'))

console.log(JSON.stringify(summary, null, 2))
if (failures.length || fatalError) process.exit(1)
