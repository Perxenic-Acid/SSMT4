import assert from 'node:assert/strict'
import { readFile, readdir, stat } from 'node:fs/promises'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../', import.meta.url))
const contractPath = join(root, 'docs', 'plugin-api-v1-contract.json')
const contract = JSON.parse(await readFile(contractPath, 'utf8'))
const packageRoot = join(root, 'native', 'official-plugin-packages')

const currentApi = contract.pluginApiVersion
const [major, minor, patch] = currentApi.split('.').map(Number)

async function pathExists(path) {
  try {
    await stat(path)
    return true
  } catch {
    return false
  }
}

function parseVersion(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(value)
  if (!match) throw new Error(`Invalid semantic version: ${value}`)
  return match.slice(1).map(Number)
}

function compareVersions(left, right) {
  const a = parseVersion(left)
  const b = parseVersion(right)
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index]
  }
  return 0
}

function supportsVersion(range, version) {
  const comparators = String(range)
    .replace(/,/g, ' ')
    .trim()
    .split(/\s+/)
    .filter(Boolean)
  if (!comparators.length) throw new Error('Empty pluginApi version range')
  return comparators.every(comparator => {
    const match = /^(>=|<=|>|<|=)?(\d+\.\d+\.\d+)$/.exec(comparator)
    if (!match) throw new Error(`Unsupported pluginApi comparator: ${comparator}`)
    const result = compareVersions(version, match[2])
    switch (match[1] || '=') {
      case '>': return result > 0
      case '>=': return result >= 0
      case '<': return result < 0
      case '<=': return result <= 0
      case '=': return result === 0
      default: return false
    }
  })
}

function normalizePluginApi(manifest, label) {
  if (manifest.schemaVersion >= 2) {
    assert.equal(
      typeof manifest.compatibility?.pluginApi,
      'string',
      `${label}: Schema 2 must declare compatibility.pluginApi`,
    )
    return manifest.compatibility.pluginApi
  }
  const legacy = manifest.pluginApiVersion ?? contract.capabilityApiVersion
  assert.equal(
    legacy,
    contract.capabilityApiVersion,
    `${label}: unsupported legacy pluginApiVersion ${legacy}`,
  )
  return `>=${currentApi} <${major + 1}.0.0`
}

const idDirectories = (await readdir(packageRoot, { withFileTypes: true }))
  .filter(entry => entry.isDirectory())
  .map(entry => entry.name)
  .sort()

const manifests = []
for (const id of idDirectories) {
  const versionDirectories = (await readdir(join(packageRoot, id), { withFileTypes: true }))
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name)
  for (const version of versionDirectories) {
    const manifestPath = join(packageRoot, id, version, 'ssmt-plugin.json')
    try {
      await stat(manifestPath)
    } catch {
      continue
    }
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'))
    const label = `${id}@${version}`
    assert.equal(manifest.id, id, `${label}: directory ID mismatch`)
    assert.equal(manifest.version, version, `${label}: directory version mismatch`)
    assert.ok(
      manifest.schemaVersion >= contract.minimumManifestSchemaVersion
        && manifest.schemaVersion <= contract.manifestSchemaVersion,
      `${label}: unsupported manifest schema ${manifest.schemaVersion}`,
    )
    const pluginApi = normalizePluginApi(manifest, label)
    assert.ok(
      supportsVersion(pluginApi, currentApi),
      `${label}: pluginApi ${pluginApi} does not accept current API ${currentApi}`,
    )
    manifests.push({
      id,
      version,
      root: join(packageRoot, id, version),
      manifest,
      pluginApi,
    })
  }
}

const expectedLatest = new Map([
  ['ssmt.dlss5.integration', { version: '0.3.0', schemaVersion: 2, pageKind: 'static', draft: true, optionalInSubset: true }],
  ['ssmt.hoyoshade.bridge', { version: '0.2.0', schemaVersion: 2, pageKind: 'static', draft: true, optionalInSubset: true }],
  ['ssmt.liquid-gravity-probe', { version: '0.2.33', schemaVersion: 2, pageKind: 'local-service' }],
  ['ssmt.player-tweaks', { version: '0.2.0', schemaVersion: 2, pageKind: 'static', optionalInSubset: true }],
  ['ssmt.poser.genshin-probe', { version: '0.1.1', schemaVersion: 1, optionalInSubset: true }],
  ['ssmt.secondary-motion', { version: '3.3.4', schemaVersion: 2, pageKind: 'static' }],
])

const verifiedLatest = []
const optionalPackagesOutsideSubset = []
for (const [id, expected] of expectedLatest) {
  const candidates = manifests.filter(item => item.id === id)
  if (!candidates.length && expected.optionalInSubset) {
    optionalPackagesOutsideSubset.push(`${id}@${expected.version}`)
    continue
  }
  assert.ok(candidates.length, `${id}: no package versions found`)
  const latest = candidates.sort((a, b) => compareVersions(a.version, b.version)).at(-1)
  if (expected.optionalInSubset && compareVersions(latest.version, expected.version) < 0) {
    optionalPackagesOutsideSubset.push(`${id}@${expected.version} (checkout latest: ${latest.version})`)
    continue
  }
  assert.equal(latest.version, expected.version, `${id}: unexpected latest package version`)
  assert.equal(latest.manifest.schemaVersion, expected.schemaVersion, `${id}: unexpected latest schema`)
  if (expected.pageKind) {
    const pages = latest.manifest.contributions?.pages ?? []
    assert.ok(
      pages.some(page => page.kind === expected.pageKind),
      `${id}: latest package is missing a ${expected.pageKind} page`,
    )
    for (const page of pages) {
      if (!page.path) continue
      assert.ok(
        (await stat(join(latest.root, page.path))).isFile(),
        `${id}@${latest.version}: page file is missing: ${page.path}`,
      )
    }
  }
  if (expected.draft) {
    const status = JSON.parse(await readFile(join(latest.root, 'release-status.json'), 'utf8'))
    assert.equal(status.publish, false, `${id}@${latest.version}: draft package must not be publishable`)
  }
  verifiedLatest.push(`${id}@${latest.version}`)
}

const pageRuntime = await readFile(join(root, 'src', 'plugin', 'pageRuntime.ts'), 'utf8')
const pageHost = await readFile(join(root, 'src', 'plugin', 'components', 'PluginPageHost.vue'), 'utf8')
assert.match(
  pageRuntime,
  new RegExp(`PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION\\s*=\\s*${contract.pageBridgeProtocolVersion}\\b`),
  'Frontend bridge protocol constant differs from the frozen contract',
)
assert.ok(
  pageHost.includes('PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION')
    && pageHost.includes('PLUGIN_PAGE_BRIDGE_REQUEST_TYPE')
    && pageHost.includes('PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE'),
  'PluginPageHost does not use the frozen bridge constants',
)

const bridgeFiles = [
  join(root, 'native', 'dlss5-integration-ui', 'src', 'bridge.ts'),
  join(root, 'native', 'liquid-gravity-probe-ui', 'src', 'consent.ts'),
  join(root, 'native', 'player-tweaks-ui', 'src', 'bridge.ts'),
  join(root, 'native', 'secondary-motion-ui', 'src', 'bridge.ts'),
  join(root, 'examples', 'plugin-sdk-fixture', 'ui', 'index.html'),
]
for (const file of bridgeFiles) {
  if (!await pathExists(file)) continue
  const source = await readFile(file, 'utf8')
  assert.ok(
    source.includes(`protocolVersion: ${contract.pageBridgeProtocolVersion}`),
    `${relative(root, file)}: request protocol version is not frozen`,
  )
  assert.ok(
    source.includes(`protocolVersion !== ${contract.pageBridgeProtocolVersion}`),
    `${relative(root, file)}: response protocol version is not frozen`,
  )
  assert.ok(
    source.includes(contract.bridgeMessageTypes.request)
      && source.includes(contract.bridgeMessageTypes.response),
    `${relative(root, file)}: bridge message names differ from the contract`,
  )
}

const pluginUiRoots = [
  join(root, 'native', 'dlss5-integration-ui', 'src'),
  join(root, 'native', 'liquid-gravity-probe-ui', 'src'),
  join(root, 'native', 'player-tweaks-ui', 'src'),
  join(root, 'native', 'secondary-motion-ui', 'src'),
]

async function collectSourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) files.push(...await collectSourceFiles(path))
    else if (/\.(?:ts|vue|js|css)$/.test(entry.name)) files.push(path)
  }
  return files
}

for (const directory of pluginUiRoots) {
  if (!await pathExists(directory)) continue
  for (const file of await collectSourceFiles(directory)) {
    const source = await readFile(file, 'utf8')
    assert.ok(
      !source.includes('@tauri-apps/'),
      `${relative(root, file)}: plugin UI must not import host Tauri APIs`,
    )
  }
}

assert.equal(
  new Set(contract.capabilities.map(item => item.id)).size,
  contract.capabilities.length,
  'Frozen capability list contains duplicate IDs',
)
assert.equal(
  new Set(contract.errorCodes).size,
  contract.errorCodes.length,
  'Frozen error-code list contains duplicates',
)
assert.equal(
  contract.compatibilityRules.futurePluginApi,
  'install-visible-incompatible',
  'Future Plugin API compatibility policy changed',
)
assert.equal(minor, 1, 'Plugin API 1.1 backend contract has an unexpected minor version')
assert.equal(patch, 0, 'Plugin API 1 contract was frozen at an unexpected patch version')

console.log(JSON.stringify({
  ok: true,
  contractVersion: contract.contractVersion,
  pluginApiVersion: currentApi,
  bridgeProtocolVersion: contract.pageBridgeProtocolVersion,
  packagesChecked: manifests.length,
  latestPackages: verifiedLatest,
  optionalPackagesOutsideSubset,
  capabilities: contract.capabilities.length,
  errorCodes: contract.errorCodes.length,
}, null, 2))
