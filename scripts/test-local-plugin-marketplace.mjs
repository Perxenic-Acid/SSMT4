import assert from 'node:assert/strict'
import fs from 'node:fs'
import vm from 'node:vm'
import ts from 'typescript'

// 验证离线发现到安装审阅的真实页面逻辑，不连接用户注册表或启动游戏。
const vue = fs.readFileSync(new URL('../src/views/PluginMarketplace/PluginMarketplace.vue', import.meta.url), 'utf8')
const script = vue.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('marketplace.ts', script, ts.ScriptTarget.Latest, true)
const imports = parsed.statements.filter(ts.isImportDeclaration)
let source = script
for (const item of imports.reverse()) source = source.slice(0, item.getFullStart()) + source.slice(item.getEnd())
source += '\nglobalThis.api={refreshAll,installLocal,installFromFile,installReviewed,allEntries,reviewOpen,reviewAccepted,reviewArchive,reviewLocalId,sourceLabelFor};'

const calls = []
const local = {
  archivePath: 'C:\\Repo\\build\\plugin-packages\\ssmt.poser.genshin-probe-0.1.0.ssmtpkg',
  inspection: { manifest: { id: 'ssmt.poser.genshin-probe', name: 'Poser', author: 'SSMT', version: '0.1.0', permissions: ['native.inject'], externalDependencies: [], compatibility: { games: ['GIMI'] } }, sha256: 'a'.repeat(64), fileCount: 3, unpackedSize: 123 },
}
let dialogFile = 'C:\\Downloads\\third-party.ssmtpkg'
const context = vm.createContext({
  console: { error() {} },
  ref: value => ({ value }),
  computed: get => ({ get value() { return get() } }),
  watch() {}, onMounted() {}, defineAsyncComponent() {},
  useRoute: () => ({ query: {} }), useI18n: () => ({ t: key => key }),
  AppStateManager: { gamesList: [], appSettings: { CurrentGameName: 'Default' } },
  ElMessage: { success() {}, error() {} },
  openDialog: async () => dialogFile,
  invoke: async (command, args) => {
    calls.push({ command, args })
    if (command === 'official_plugin_catalog') throw new Error('offline')
    if (command === 'local_plugin_packages') return [local]
    if (command === 'inspect_plugin_package') return { ...local.inspection, manifest: { ...local.inspection.manifest, id: 'third.party' } }
    return []
  },
})
vm.runInContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context)
const page = context.api
await page.refreshAll()
assert.equal(page.allEntries.value.length, 1)
const entry = page.allEntries.value[0]
assert.equal(entry.id, local.inspection.manifest.id)
assert.equal(page.sourceLabelFor(entry), 'pluginMarketplace.localBuild')
assert.equal(calls.filter(call => call.command.startsWith('install')).length, 0)
await page.installLocal(entry)
assert.equal(page.reviewOpen.value, true)
assert.equal(page.reviewLocalId.value, entry.id)
assert.equal(page.reviewArchive.value, local.archivePath)
await page.installReviewed()
assert.equal(calls.filter(call => call.command.startsWith('install')).length, 0)
page.reviewAccepted.value = true
await page.installReviewed()
const installed = calls.find(call => call.command === 'install_local_plugin')
assert.equal(installed.args.id, entry.id)
assert.equal(installed.args.reviewedSha256, local.inspection.sha256)
assert.equal(installed.args.disclaimerAccepted, true)
assert.equal(installed.args.archivePath, undefined)
await page.installFromFile()
assert.equal(page.reviewLocalId.value, '')
page.reviewAccepted.value = true
await page.installReviewed()
assert.equal(calls.find(call => call.command === 'install_plugin_package').args.archivePath, dialogFile)
console.log('本地市场：离线发现、来源标识、审阅门槛、本地与第三方安装分流通过。')
