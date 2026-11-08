import { mkdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

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

const fixtureRoot = 'D:/github项目/SSMT4-Dev/build/e2e/hoyo-shade'
const gameRoot = 'D:/github项目/SSMT4-Dev/build/e2e/game'
const gameExecutable = join(gameRoot, 'StarRail.exe')

try {
  await rm(fixtureRoot, { recursive: true, force: true })
  await rm(gameRoot, { recursive: true, force: true })
  for (const directory of [
    join(fixtureRoot, 'LauncherResource'),
    join(fixtureRoot, 'InjectResource'),
    join(fixtureRoot, 'Presets'),
    join(fixtureRoot, 'reshade-shaders', 'Addons'),
    gameRoot,
  ]) {
    await mkdir(directory, { recursive: true })
  }
  for (const relative of [
    'inject.exe',
    'ReShade64.dll',
    'LauncherResource/INIBuild.exe',
    'reshade-shaders/Addons/ShaderToggler.addon64',
  ]) {
    await writeFile(join(fixtureRoot, relative), 'SSMT graphics profile fixture')
  }
  await writeFile(join(fixtureRoot, 'ReShade.ini'), '[GENERAL]\nEffectSearchPaths=Shaders\n[OVERLAY]\nTutorialProgress=4\n')
  await writeFile(join(fixtureRoot, 'Presets/Mod OFF.ini'), 'Techniques=Shade@Shade.fx\n')
  await writeFile(gameExecutable, 'fixture')

  await send('Runtime.enable')
  const packages = await invoke('local_plugin_packages')
  const local = packages.find(item =>
    item.inspection?.manifest?.id === 'ssmt.hoyoshade.bridge'
    && item.inspection?.manifest?.version === '0.2.0'
  )
  if (!local) throw new Error('HoYoShade Bridge 0.2.0 package was not discovered')

  const installed = await invoke('plugin_registry_snapshot')
  if (!installed.some(item => item.manifest?.id === 'ssmt.hoyoshade.bridge'
    && item.manifest?.version === '0.2.0')) {
    await invoke('install_local_plugin', {
      id: local.inspection.manifest.id,
      reviewedSha256: local.inspection.sha256,
      disclaimerAccepted: true,
    })
  }
  await invoke('set_plugin_external_dependency_path', {
    pluginId: 'ssmt.hoyoshade.bridge',
    dependencyId: 'hoyoshade',
    path: fixtureRoot,
  })
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'SRMI',
    id: 'ssmt.hoyoshade.bridge',
    enabled: true,
    gameExecutable,
  })

  const snapshot = await invoke('plugin_registry_snapshot_for_game', { gameName: 'SRMI' })
  const plugin = snapshot.find(item =>
    item.manifest?.id === 'ssmt.hoyoshade.bridge'
    && item.manifest?.version === '0.2.0'
  )
  if (!plugin?.manifest?.contributions?.graphicsProfiles?.some(item =>
    item.strategy === 'reshade-hot' && item.resources.includes('graphics.reshade.host'))) {
    throw new Error(`HoYoShade graphics profile was not parsed: ${JSON.stringify(plugin)}`)
  }

  const preparation = await invoke('prepare_plugin_graphics_plan', {
    gameName: 'SRMI',
    gameExecutable,
    allowHotRuntime: true,
    excludedProviders: [],
    graphicsDecision: null,
  })
  if (preparation.plugin !== 'HoYoShade Bridge v0.2.0'
    || !preparation.plan?.dlls?.some(path => path.replace(/\\/g, '/').endsWith('/ReShade64.dll'))) {
    throw new Error(`Generic ReShade profile did not prepare: ${JSON.stringify(preparation)}`)
  }
  await invoke('configure_plugin_launch_contract', {
    gameName: 'SRMI',
    gameExecutable,
    candidateId: preparation.candidateId,
  })

  console.log(JSON.stringify({
    package: 'ssmt.hoyoshade.bridge@0.2.0',
    profile: plugin.manifest.contributions.graphicsProfiles[0],
    candidateId: preparation.candidateId,
    dlls: preparation.plan.dlls,
    diagnostics: preparation.diagnostics,
  }, null, 2))
} finally {
  await invoke('set_plugin_enabled_for_game', {
    gameName: 'SRMI',
    id: 'ssmt.hoyoshade.bridge',
    enabled: false,
    gameExecutable: null,
  }).catch(() => false)
  socket.close()
}
