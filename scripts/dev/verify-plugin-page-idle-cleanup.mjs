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
  await send('Runtime.enable')
  await invoke('stop_plugin_page_services_for_plugin', { pluginId: 'ssmt.efmi-glow-studio' })
  await evaluate(`location.reload()`)
  await sleep(1200)
  const pushed = await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    if (!router) return false
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.efmi-glow-studio' } })
    return true
  })()`)
  if (!pushed) throw new Error('Vue router is unavailable')
  await waitFor(`document.querySelector('.plugin-page-frame.ready')`, 'ready plugin iframe', 45000)
  const iframeSrc = await evaluate(`document.querySelector('.plugin-page-frame')?.src || ''`)
  const fragment = new URLSearchParams(new URL(iframeSrc).hash.replace(/^#/, ''))
  const leaseId = fragment.get('leaseId') || ''
  const serviceKey = fragment.get('serviceKey') || ''
  if (!leaseId || !serviceKey) throw new Error('Plugin iframe is missing leaseId or serviceKey')
  const session = await invoke('plugin_page_session_status', { sessionId: leaseId })
  if (!session) throw new Error('Plugin page session status was not found')
  if (!session.processId) throw new Error('UI-created session did not report its process id')

  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router.push('/')
  })()`)
  await waitFor(`location.pathname === '/'`, 'home route')
  await sleep(2_500)
  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router.push({ path: '/plugins', query: { plugin: 'ssmt.efmi-glow-studio' } })
  })()`)
  await waitFor(`location.pathname === '/plugins'`, 'reactivated plugin route')
  const resumed = await invoke('plugin_page_service_status', { serviceKey })
  if (!resumed || resumed.processId !== session.processId || resumed.leaseCount < 1) {
    throw new Error('Plugin page session was not reused during the idle grace period')
  }
  await evaluate(`(() => {
    const router = document.querySelector('#app')?.__vue_app__?.config?.globalProperties?.$router
    void router.push('/')
  })()`)
  await waitFor(`location.pathname === '/'`, 'home route after reactivation')
  await sleep(31_500)
  const statusAfterIdle = await invoke('plugin_page_service_status', { serviceKey })
  let processAlive = false
  try {
    process.kill(session.processId, 0)
    processAlive = true
  } catch {
    processAlive = false
  }
  if (statusAfterIdle || processAlive) {
    throw new Error(JSON.stringify({
      message: 'Idle grace cleanup did not remove the plugin page session and process',
      statusAfterIdle,
      processAlive,
      processId: session.processId,
      serviceKey,
      location: await evaluate('location.pathname'),
    }))
  }
  console.log(JSON.stringify({ idleCleanup: true, serviceKey, processId: session.processId }))
} finally {
  socket.close()
}
