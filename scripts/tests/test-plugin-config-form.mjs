import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import vm from 'node:vm'
import ts from 'typescript'
import * as vue from 'vue'
import { parse } from '@vue/compiler-sfc'
const plain = value => JSON.parse(JSON.stringify(value))
const exports = {}
vm.runInNewContext(ts.transpileModule(readFileSync('src/plugin/configForm.ts', 'utf8'), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, { exports })
const { decodeConfigForm, readConfigField, writeConfigField } = exports
const fixture = () => ({ encoding: 'ssmt-config/1', title: '设置', groups: [{ id: 'options', title: '选项', fields: [{ path: ['options', 'enabled'], kind: 'boolean', label: '启用' }, { path: ['options', 'amount'], kind: 'number', label: '数值', min: 0, max: 10, step: 0.1 }] }] })
test('配置解码保留小数约束，修改可见字段时保留隐藏配置', () => {
  const form = decodeConfigForm(fixture()), field = form.groups[0].fields[1]
  const values = { options: { enabled: true, amount: 2.5, hidden: 42 }, hidden: { other: true } }
  assert.equal(readConfigField(values, field), 2.5)
  writeConfigField(values, field, 3.1)
  assert.equal(readConfigField(values, field), 3.1)
  assert.equal(values.options.hidden, 42)
  assert.deepEqual(values.hidden, { other: true })
  assert.equal(field.step, 0.1)
})
test('配置解码拒绝未知版本、脚本、原型路径、重复绑定与无效范围', () => {
  for (const change of [
    form => { form.encoding = 'ssmt-config/2' },
    form => { form.script = 'alert(1)' },
    form => { form.groups[0].fields[0].path = ['__proto__', 'polluted'] },
    form => { form.groups[0].fields[0].path = ['options', 'constructor'] },
    form => { form.groups[0].fields[1].path = ['options', 'enabled'] },
    form => { form.groups[0].fields[1].path = ['options'] },
    form => { form.groups[0].fields[1].step = 0 },
    form => { form.groups[0].fields[1].max = -1 },
    form => { form.groups[0].fields[1].min = Infinity },
    form => { form.groups[0].fields[0].kind = 'html' },
  ]) { const form = fixture(); change(form); assert.throws(() => decodeConfigForm(form)) }
  assert.equal({}.polluted, undefined)
  const fields = decodeConfigForm(fixture()).groups[0].fields
  assert.throws(() => readConfigField({ options: { enabled: 'false' } }, fields[0]))
  assert.throws(() => readConfigField({ options: { amount: 12 } }, fields[1]))
})
function component(handler) {
  const source = parse(readFileSync('src/plugin/components/BackendConfigForm.vue', 'utf8')).descriptor.scriptSetup.content
  const state = {}, props = { pluginId: 'example.settings', pageId: 'settings', gameName: 'game', enabled: true }
  let mounted
  const code = ts.transpileModule(source + '\nObject.assign(exports,{schema,values,error,saved,load,save,update})', { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  vm.runInNewContext(code, {
    exports: state, defineProps: () => props,
    require: name => name === 'vue' ? { ...vue, onMounted: fn => { mounted = fn }, onBeforeUnmount: () => {} }
      : name === '../configForm' ? exports : { requestPluginConfigForm: handler },
  })
  return { state, props, mount: () => mounted() }
}
test('宿主表单按声明动作读写后台，失败可重试且停用后不保存', async () => {
  const calls = []
  let fail = true
  const s = component(async (plugin, page, game, action, payload) => {
    assert.equal(plugin, 'example.settings'); assert.equal(page, 'settings'); assert.equal(game, 'game')
    calls.push(action)
    if (action === 'schema') return fixture()
    if (action === 'read') return { options: { enabled: true, amount: 2.5 }, hidden: 42 }
    if (fail) throw Error('save failed')
    assert.equal(payload.hidden, 42)
    return plain(payload)
  })
  await s.mount()
  assert.deepEqual(calls, ['schema', 'read'])
  s.state.update(s.state.schema.value.groups[0].fields[1], 3.5)
  await s.state.save()
  assert.match(s.state.error.value, /save failed/)
  assert.equal(s.state.saved.value, false)
  fail = false
  await s.state.save()
  assert.equal(s.state.saved.value, true)
  s.props.enabled = false
  const count = calls.length
  await s.state.save()
  assert.equal(calls.length, count)
})
test('宿主拒绝无效描述，不继续读取或渲染插件内容', async () => {
  const calls = []
  const s = component(async (_plugin, _page, _game, action) => { calls.push(action); return { encoding: 'unknown' } })
  await s.mount()
  assert.deepEqual(calls, ['schema'])
  assert.equal(s.state.schema.value, null)
  assert.match(s.state.error.value, /不支持/)
})
