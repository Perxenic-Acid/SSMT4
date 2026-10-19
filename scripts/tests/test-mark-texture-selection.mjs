import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { setImmediate } from 'node:timers/promises';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';
import { parse } from '@vue/compiler-sfc';

const bindings = ['workspaceSources', 'subMeshOptions', 'subMeshTree', 'selectedSubMesh', 'previewSyncSelectedSubMesh',
  'mutedPreviewSubMeshMap', 'soloPreviewSubMeshMap', 'previewSubMeshTargets', 'selectPreviewSubMesh',
  'collapsedLods', 'collapsedIBs', 'subMeshDrawerList', 'renameComponent', 'subMeshOptionItems', 'loadSubMeshOptions'];
function transpile(source) {
  return ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
}
function fixture(t) {
  const tree = {};
  vm.runInNewContext(transpile(readFileSync(new URL('../../src/views/MarkTexture/ComponentTree.ts', import.meta.url), 'utf8')), { exports: tree });
  const state = { prompt: null, writes: [], files: new Map(), directories: new Map() };
  const appSettings = vue.reactive({ DBMTWorkFolder: '/fixture', CurrentGameName: 'GIMI', CurrentWorkSpace: 'Old' });
  const mocks = {
    'vue': { ...vue, onMounted() {}, onActivated() {}, onDeactivated() {}, onBeforeUnmount() {} },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    '../../store/AppStateManager': { AppStateManager: { appSettings } },
    '@tauri-apps/api/path': { join: async (...parts) => parts.join('/') },
    '@tauri-apps/api/core': { invoke: async () => {} },
    '@tauri-apps/plugin-fs': {
      exists: async path => state.files.has(path),
      readTextFile: async path => { assert.ok(state.files.has(path), path); return state.files.get(path); },
      readDir: async path => state.directories.get(path) ?? [],
    },
    '../../common/DrawIBConfig': { readDrawIBConfigFromWorkspace: async () => [] },
    './MarkTextureFull': {
      getMarkTextureComponentJsonPath: async path => `${path}/ComponentName_DrawCallIndexList.json`,
      getMarkTextureTrianglelistJsonPath: async path => `${path}/TrianglelistDedupedFileName.json`,
      getMarkTextureDrawIBComponentJsonPath: async path => `${path}/DrawIB-Component.json`,
      readPersistedSubMeshDrawCallIndexes: async () => [],
      readDrawCallIndexesMatchedByPersistedSubMeshMarks: async () => [],
      readAppliedSubMeshTextureMarks: async () => [],
    },
    './ComponentTree': tree,
    '../../common/ComponentAliases': {
      isComponentAlias: () => true,
      readComponentAliases: async () => ({}),
      writeComponentAlias: async (folder, name, alias) => { state.writes.push({ folder, name, alias }); return { [name]: alias }; },
    },
    'element-plus': { ElMessageBox: { prompt: () => state.prompt() }, ElMessage: { warning() {}, info() {} } },
  };
  const source = parse(readFileSync(new URL('../../src/views/MarkTexture/MarkTextureFull.vue', import.meta.url), 'utf8')).descriptor.scriptSetup.content;
  const exports = {};
  const scope = vue.effectScope();
  scope.run(() => vm.runInNewContext(transpile(`${source}\nObject.assign(exports, {${bindings.join(',')}});`), {
    exports, require: name => mocks[name] ?? {}, console: { log() {}, warn() {}, error: console.error }, setTimeout: () => 1, clearTimeout() {},
    window: { matchMedia: () => ({ matches: true }) },
  }));
  t.after(() => scope.stop());
  const a = '848c5980-3-0', b = '848c5980-6-3';
  const ids = [a, b].map(subMeshName => JSON.stringify({ tabId: 'lod-0', subMeshName }));
  exports.workspaceSources.value = [{
    tabId: 'lod-0', tabName: 'LOD0', workspacePath: '/fixture/LOD0', componentAliases: {},
    subMeshDrawCallMap: { [a]: [], [b]: [] }, trianglelistDedupedData: {}, subMeshTrianglelistDedupedData: {},
    drawIBComponentMap: { [a]: { drawIB: '848c5980', componentIndex: '0' }, [b]: { drawIB: '848c5980', componentIndex: '1' } },
    drawIBAliasMap: {}, drawIBOrderMap: { '848c5980': 0 }, drawIBKeysByLengthDesc: ['848c5980'],
  }];
  exports.subMeshOptions.value = ids;
  return { ...exports, state, ids };
}

test('同步单选只显示对应 Submesh，自动展开 LOD / IB 并仅滚动左栏', async t => {
  const f = fixture(t);
  const calls = [];
  const selected = f.ids[1];
  f.subMeshDrawerList.value = {
    scrollTop: 0, clientHeight: 200, getBoundingClientRect: () => ({ top: 30, bottom: 230 }),
    querySelectorAll: () => [{ dataset: { submesh: selected }, focus: options => calls.push(options.preventScroll),
      getBoundingClientRect: () => ({ top: 400, bottom: 438, height: 38 }) }],
    scrollTo: options => calls.push(options.top),
  };
  f.collapsedLods.value = ['lod-0', 'other-lod'];
  f.collapsedIBs.value = [JSON.stringify(['lod-0', '848c5980']), 'other-ib'];
  f.previewSyncSelectedSubMesh.value = true;
  f.mutedPreviewSubMeshMap.value = { [selected]: true };
  f.soloPreviewSubMeshMap.value = { [f.ids[0]]: true };
  f.selectPreviewSubMesh(selected);
  await setImmediate();
  assert.equal(f.selectedSubMesh.value, selected);
  assert.deepEqual([...f.previewSubMeshTargets.value.map(target => target.id)], [selected]);
  assert.deepEqual([...f.collapsedLods.value], ['other-lod']);
  assert.deepEqual([...f.collapsedIBs.value], ['other-ib']);
  assert.deepEqual(calls, [true, 289]);
  f.previewSyncSelectedSubMesh.value = false;
  assert.deepEqual([...f.previewSubMeshTargets.value.map(target => target.id)], [f.ids[0]]);
});

test('Component 改名只改变显示文本和别名文件，不改变选择 ID、目录或预览目标', async t => {
  const f = fixture(t);
  f.state.prompt = async () => ({ value: '头发.高光' });
  f.selectedSubMesh.value = f.ids[0];
  await f.renameComponent(f.ids[0]);
  assert.deepEqual(f.state.writes, [{ folder: '/fixture/LOD0', name: '848c5980-3-0', alias: '头发.高光' }]);
  assert.equal(f.subMeshTree.value[0].ibs[0].items[0].componentLabel, '头发.高光');
  assert.equal(f.selectedSubMesh.value, f.ids[0]);
  assert.equal(f.previewSubMeshTargets.value[0].subMeshName, '848c5980-3-0');
  f.state.prompt = async () => { throw 'cancel'; };
  await f.renameComponent(f.ids[1]);
  assert.equal(f.state.writes.length, 1);
});

test('没有 WorkPageTabs 索引的旧 LOD 目录及根目录提取结果均可加载三级树', async t => {
  for (const lod of ['', 'LOD0']) {
    const f = fixture(t);
    const root = '/fixture/WorkSpace/GIMI/Old';
    const lodPath = lod ? `${root}/${lod}` : root;
    f.state.directories.set(root, lod ? [{ isDirectory: true, name: lod }] : []);
    f.state.files.set(`${lodPath}/ComponentName_DrawCallIndexList.json`, '{"848c5980-3-0":[]}');
    f.state.files.set(`${lodPath}/TrianglelistDedupedFileName.json`, '{}');
    f.state.files.set(`${lodPath}/DrawIB-Component.json`, '{"848c5980":{"0":"848c5980-3-0"}}');
    await f.loadSubMeshOptions();
    assert.equal(f.subMeshTree.value.length, 1);
    assert.equal(f.subMeshTree.value[0].ibs[0].items[0].drawIB, '848c5980');
    assert.equal(f.subMeshTree.value[0].ibs[0].items[0].componentLabel, 'Component 0');
    assert.equal(JSON.parse(f.selectedSubMesh.value).subMeshName, '848c5980-3-0');
  }
});
