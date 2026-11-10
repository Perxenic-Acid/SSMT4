import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import * as vue from 'vue'
const code = ts.transpileModule(readFileSync('src/views/ModsManagement/ModsManagement.install.ts', 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
function fixture(archives, answers = [], selection = [], conflicts = []) {
  const exports = {}, calls = [], prompts = [], messages = [], picks = [], installed = [], failures = [], reconciled = []
  let refreshed = 0
  const options = { selectedGame: vue.ref('game'), selectedGroup: vue.ref('Default'), pendingDropGroup: vue.ref('Dropped'), loading: vue.ref(false), t: (key, params) => JSON.stringify({key,...params}), reconcileInstalledMods: async (...args) => { refreshed++; reconciled.push(args) }, suppressFsRefresh() {} }
  vm.runInNewContext(code, { exports, setTimeout: () => 1, clearTimeout() {}, console,
    require: name => ({
      vue,
      '@tauri-apps/api/core': { invoke: async (command, args) => {
        calls.push({ command, ...args })
        const source = args.path ?? args.archivePath
        const archive = archives[source]
        if (!archive) throw 'unsupported'
        if (archive.corrupt) throw 'archive corrupted'
        if (archive.password && (command === 'install_mod_archive' || archive.headers) && args.password !== archive.password) throw args.password ? 'ARCHIVE_PASSWORD_INVALID' : 'ARCHIVE_PASSWORD_REQUIRED'
        if (command === 'install_mod_archive') {
          if (archive.existing && args.targetName === source.replace(/\.(zip|7z|rar)$/i, '') && args.conflictAction === 'ask') throw 'MOD_IMPORT_CONFLICT'
          installed.push(source)
        }
        return { file_count: 1, has_ini: true, format: 'zip', root_dirs: [] }
      } },
      '@tauri-apps/plugin-dialog': { open: async options => { picks.push(options); return selection } },
      'element-plus': { ElMessage: Object.fromEntries(['success','error','warning'].map(kind => [kind, value => messages.push({ kind, ...value })])), ElMessageBox: Object.assign(async options => { const choice = conflicts.shift(); if (!choice) throw 'cancel'; options.message().children[2].children.find(option => option.children[0].props.value === choice).children[0].props.onChange() }, { alert: async (message) => { failures.push(message) }, prompt: async (message, title, options) => {
        prompts.push({ message, title, options })
        if (options.inputType === 'password') assert.equal(options.inputValidator(''), JSON.stringify({ key: 'modsManagement.messages.archivePasswordEmpty' }))
        const answer = answers.shift()
        if (answer === undefined || answer === null) throw 'cancel'
        return { value: answer }
      } }) },
      '../../store/ModManager': { ModManager: { getInstallDir: async () => 'D:/game' } },
      '../../store/ModTagStore': { ModTagStore: { setModTags: async () => {} } },
      '../../utils/debugLog': { debugError() {} },
    })[name],
  })
  return { api: exports.useModsManagementInstall(options), calls, prompts, installed, picks, messages, failures, reconciled, options, get refreshed() { return refreshed } }
}
test('批量导入顺延密码，普通文件不清除密码，不同密码失败后重新询问', async () => {
  const f = fixture({ 'a.zip': { password: ' a ', headers: true }, 'plain.zip': {}, 'b.7z': { password: ' a ', headers: true }, 'c.rar': { password: 'b' }, 'd.zip': { password: 'b' } }, [' a ', 'b'])
  await f.api.batchInstallFromPaths(['a.zip','plain.zip','b.7z','c.rar','d.zip'])
  assert.deepEqual(f.installed, ['a.zip','plain.zip','b.7z','c.rar','d.zip'])
  assert.equal(f.prompts.length, 2)
  assert.match(f.prompts[1].message, /archivePasswordIncorrect/)
  assert.ok(f.calls.filter(call => call.command === 'install_mod_archive').every(call => call.targetGroup === 'Dropped'))
  assert.equal(f.refreshed, 1)
  assert.equal(f.api.isInstalling.value, false)
  assert.equal(f.options.loading.value, false)
  // 新批次不继承上一批次密码。
  await f.api.batchInstallFromPaths(['a.zip'])
  assert.equal(f.prompts.length, 3)
})
test('错误输入重复提示；取消只跳过当前文件，损坏文件不弹密码框', async () => {
  const f = fixture({ 'a.zip': { password: 'right' }, 'other.rar': { password: 'different' }, 'broken.zip': { corrupt: true }, 'plain.zip': {} }, ['wrong', 'right', null])
  await f.api.batchInstallFromPaths(['a.zip','other.rar','broken.zip','plain.zip'])
  assert.deepEqual(f.installed, ['a.zip','plain.zip'])
  assert.equal(f.prompts.length, 3)
  const summary = JSON.parse(f.messages.at(-1).message)
  assert.equal(summary.success, 2)
  assert.equal(summary.fail, 1)
  assert.equal(summary.skipped, 1)
})
test('单文件拖拽在预览时解锁，安装沿用密码且不弹重复浮窗', async () => {
  const f = fixture({ 'a.7z': { password: 'secret', headers: true } }, ['secret'])
  await f.api.handleFileDrop('a.7z')
  assert.equal(f.api.showInstallDialog.value, true)
  await f.api.confirmInstall()
  assert.deepEqual(f.installed, ['a.7z'])
  assert.equal(f.prompts.length, 1)
})
test('文件与目录选择器均允许多选，并走同一批量导入流程', async () => {
  for (const directory of [false,true]) {
    const f = fixture({ 'a': {}, 'b': {} }, [], ['a','b'])
    await f.api[directory ? 'pickInstallFolder' : 'pickInstallArchive']()
    assert.equal(f.picks[0].multiple, true)
    assert.equal(Boolean(f.picks[0].directory), directory)
    assert.deepEqual(f.installed, ['a','b'])
  }
})

test('取消不同密码的文件后，后续文件仍可复用之前已成功的密码', async () => {
  const f = fixture({ 'a.zip': { password: 'a' }, 'b.zip': { password: 'b' }, 'c.zip': { password: 'a' } }, ['a','wrong',null])
  await f.api.batchInstallFromPaths(['a.zip','b.zip','c.zip'])
  assert.deepEqual(f.installed, ['a.zip','c.zip'])
  assert.equal(f.prompts.length, 3)
})
test('单文件安装时取消密码浮窗会关闭本次导入并释放忙碌状态', async () => {
  const f = fixture({ 'a.zip': { password: 'a' } }, [null])
  await f.api.handleFileDrop('a.zip')
  await f.api.confirmInstall()
  assert.equal(f.api.showInstallDialog.value, false)
  assert.equal(f.api.isInstalling.value, false)
  assert.deepEqual(f.installed, [])
})

test('五个文件两个失败时继续导入，展示失败文件与原因，仅刷新成功目标', async () => {
  const f = fixture({ 'a.7z': {}, 'b.7z': { corrupt: true }, 'c.7z': {}, 'd.7z': { corrupt: true }, 'e.7z': {} })
  const loadingChanges = []
  const stop = vue.watch(f.options.loading, value => loadingChanges.push(value), { flush: 'sync' })
  await f.api.batchInstallFromPaths(['a.7z','b.7z','c.7z','d.7z','e.7z'])
  stop()
  assert.deepEqual(f.installed, ['a.7z','c.7z','e.7z'])
  assert.match(f.failures[0], /b\.7z: archive corrupted/)
  assert.match(f.failures[0], /d\.7z: archive corrupted/)
  assert.equal(JSON.stringify(f.reconciled), JSON.stringify([['game','Dropped',['a','c','e']]]))
  assert.deepEqual(loadingChanges, [])
  assert.equal(f.messages.at(-1).kind, 'warning')
})

test('同名目标支持覆盖、直接写入、跳过与重命名，重命名后的路径用于增量刷新', async () => {
  for (const choice of ['replace','merge','skip','rename']) {
    const f = fixture({ 'same.7z': { existing: true }, 'next.7z': {} }, choice === 'rename' ? ['renamed'] : [], [], [choice])
    await f.api.batchInstallFromPaths(['same.7z','next.7z'])
    const installs = f.calls.filter(call => call.command === 'install_mod_archive')
    if (choice === 'skip') assert.deepEqual(f.installed, ['next.7z'])
    else {
      assert.deepEqual(f.installed, ['same.7z','next.7z'])
      assert.equal(installs[1].conflictAction, choice === 'rename' ? 'ask' : choice)
      assert.equal(installs[1].targetName, choice === 'rename' ? 'renamed' : 'same')
    }
    assert.equal(f.failures.length, 0)
    const expected = choice === 'skip' ? ['next'] : [choice === 'rename' ? 'renamed' : 'same', 'next']
    assert.equal(JSON.stringify(f.reconciled[0][2]), JSON.stringify(expected))
  }
})
