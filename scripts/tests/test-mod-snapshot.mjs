import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { ref, reactive } from 'vue';
import TreeStore from 'element-plus/es/components/tree/src/model/tree-store.mjs';

function load(name, dependencies = {}) {
  const source = readFileSync(new URL(`../../src/views/ModsManagement/ModsManagement.${name}.ts`, import.meta.url), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, require: id => dependencies[id], queueMicrotask, setTimeout, clearTimeout });
  return exports;
}
const { reconcileModSnapshot } = load('snapshot', { './ModsManagement.paths': load('paths') });
const mod = (relativePath, enabled = true) => ({ id: relativePath, relativePath, enabled });

test('切换游戏后的流式监听拒绝旧游戏分块及迟到的最终结果', async () => {
  const vue = readFileSync(new URL('../../src/views/ModsManagement/ModsManagement.vue', import.meta.url), 'utf8');
  const source = vue.slice(vue.indexOf('const loadGroupModsStreaming ='), vue.indexOf('const setGroupIcon ='));
  const code = ts.transpileModule(source + '\nexports.scan = loadGroupModsStreaming;', {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const listeners = new Set();
  const requests = [];
  const groups = [];
  const context = {
    exports: {}, Date, console, Promise,
    selectedGame: ref('A'), selectedGroup: ref('Root'), refreshModsToken: 1, groupLoadToken: 1,
    totalModsCount: ref(0), hasVisibleContent: () => false,
    ModManager: { getInstallDir: async game => game, isScanCancelled: () => false },
    listen: async (_event, callback) => { listeners.add(callback); return () => listeners.delete(callback); },
    invoke: async (_command, args) => new Promise(resolve => requests.push({ args, resolve })),
    updateAvailableGroups: incoming => groups.push(...incoming.map(group => group.id)),
    dedupeGroupsById: incoming => incoming, reconcileCurrentSubGroups: () => {},
    appendModsIncrementally: async () => {},
  };
  vm.runInNewContext(code, context);
  const first = context.exports.scan('A', 'Root', 1);
  await new Promise(resolve => setImmediate(resolve));
  context.selectedGame.value = 'B';
  context.refreshModsToken = 2;
  context.groupLoadToken = 2;
  const second = context.exports.scan('B', 'Root', 2);
  await new Promise(resolve => setImmediate(resolve));
  for (const listener of listeners) listener({ payload: {
    phase: 'chunk', scanId: requests[0].args.scanId, groups: [{ id: 'A/Old' }],
  } });
  for (const listener of listeners) listener({ payload: {
    phase: 'chunk', scanId: requests[1].args.scanId, groups: [{ id: 'B/New' }],
  } });
  requests[0].resolve({ mods: [], groups: [{ id: 'A/Late' }] });
  requests[1].resolve({ mods: [], groups: [{ id: 'B/Final' }] });
  await Promise.all([first, second]);
  assert.deepEqual(groups, ['B/New', 'B/Final']);
  assert.equal(listeners.size, 0);
});

test('懒加载箭头无需选中即可展开，重复展开不扫描，失败可重试', async () => {
  const { createGroupTreeLoader } = load('tree');
  const data = reactive([{ id: 'A', leaf: false, children: [] }]);
  let scans = 0;
  let fail = true;
  const loadChildren = createGroupTreeLoader(() => data, async () => {
    scans++;
    if (fail) throw new Error('读取失败');
    return [{ id: 'A/Child', leaf: true, children: [] }];
  });
  const store = new TreeStore({ key: 'id', data, lazy: true, load: loadChildren,
    props: { children: 'children', isLeaf: 'leaf' } });
  store.initialize();
  const parent = store.getNode('A');
  assert.equal(parent.isLeaf, false);
  parent.expand();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(parent.loaded, false);
  assert.equal(parent.loading, false);
  fail = false;
  await new Promise(resolve => parent.expand(resolve));
  const child = store.getNode('A/Child');
  assert.ok(child);
  assert.equal(child.isLeaf, true);
  assert.equal(store.currentNode, null);
  parent.collapse();
  await new Promise(resolve => parent.expand(resolve));
  assert.equal(store.getNode('A/Child'), child);
  assert.equal(scans, 2);
});

test('分类缓存与扫描更新保留 Element Plus 内部节点、展开和选中状态', () => {
  const { reconcileGroupTree } = load('tree');
  const entry = (id, children = [], count = 1) => ({ id, label: id, count, children });
  const data = reactive([entry('A', [entry('A/Child')]), entry('B')]);
  const store = new TreeStore({ key: 'id', data, props: { label: 'label', children: 'children' } });
  store.initialize();
  const parent = store.getNode('A');
  const child = store.getNode('A/Child');
  const children = data[0].children;
  parent.expand();
  for (const selected of ['A/Child', 'B', 'A', 'A/Child']) {
    store.setCurrentNodeKey(selected);
    reconcileGroupTree(data, [entry('A', [entry('A/Child')], 2), entry('B')]);
    store.setData(data);
    parent.updateChildren();
    assert.equal(store.getNode('A'), parent);
    assert.equal(store.getNode('A/Child'), child);
    assert.equal(data[0].children, children);
    assert.equal(parent.expanded, true);
    assert.equal(store.currentNode.key, selected);
    assert.equal(parent.data.count, 2);
  }
  // 实际新增和删除子项时，只改变相应节点。
  reconcileGroupTree(data, [entry('A', [entry('A/Child'), entry('A/New')]), entry('B')]);
  store.setData(data);
  parent.updateChildren();
  assert.equal(store.getNode('A/Child'), child);
  assert.ok(store.getNode('A/New'));
  reconcileGroupTree(data, [entry('A', [entry('A/Child')]), entry('B')]);
  store.setData(data);
  parent.updateChildren();
  assert.equal(store.getNode('A/New'), null);
  assert.equal(store.getNode('A'), parent);
  assert.equal(store.getNode('A/Child'), child);
  assert.equal(parent.expanded, true);
});

test('启停目录前缀变化时复用对象，完整刷新一次替换旧结果', () => {
  const existing = mod('Characters/A');
  const current = [existing, mod('Removed')];
  const next = reconcileModSnapshot(current, [mod('Characters/DISABLED_A', false), mod('Added')]);
  assert.equal(next.length, 2);
  assert.equal(next[0], existing);
  assert.equal(existing.enabled, false);
  assert.equal(next[1].id, 'Added');
  assert.equal(current.length, 2);
});

test('流式分块保留已有卡片并合并重复记录', () => {
  const existing = mod('A');
  const next = reconcileModSnapshot([existing], [mod('B'), mod('B'), mod('DISABLED_A', false)], false);
  assert.deepEqual(Array.from(next, value => value.id), ['DISABLED_A', 'B']);
  assert.equal(next[0], existing);
});

test('确认空结果可以清空列表', () => {
  assert.equal(reconcileModSnapshot([mod('A')], []).length, 0);
});

test('预览失效和扫描失败保留旧图，成功重试后仅替换相关分组', async () => {
  let fail = true;
  const { useModsManagementPreviews } = load('previews', {
    vue: { ref, reactive },
    '@tauri-apps/api/core': { convertFileSrc: value => value },
    '../../store/ModManager': { ModManager: {
      getCachedGroup: () => null,
      scanGroup: async () => {
        if (fail) throw new Error('暂时读取失败');
        return { mods: [{ enabled: true, previewImages: ['new.png'] }] };
      },
    } },
    './ModsManagement.paths': load('paths'),
    './ModsManagement.storage': {
      loadSubgroupPreviewCache: () => ({}),
      persistSubgroupPreviewCache: () => {},
      SUBGROUP_PREVIEW_MAX_IMAGES: 4,
    },
  });
  const previews = ref({ 'Game::A': ['old.png'], 'Game::B': ['other.png'] });
  const api = useModsManagementPreviews({ selectedGame: ref('Game'), mods: ref([]), subgroupPreviewMap: previews });
  api.invalidateSubgroupPreviewByGroupId('Game', 'A');
  assert.equal(previews.value['Game::A'][0], 'old.png');
  await api.preloadSubgroupPreviewImages([{ id: 'A' }]);
  assert.equal(previews.value['Game::A'][0], 'old.png');
  fail = false;
  await api.preloadSubgroupPreviewImages([{ id: 'A' }, { id: 'B' }]);
  assert.equal(previews.value['Game::A'][0], 'new.png');
  assert.equal(previews.value['Game::B'][0], 'other.png');
});

test('合并的结构变更向失效处理传递正确影响范围', async () => {
  const calls = [];
  const { createMutationReconciler } = load('mutations');
  const reconciler = createMutationReconciler({
    getGameName: () => 'Game',
    invalidateClientState: (game, impact) => calls.push([game, impact]),
    refreshCurrentContent: async () => calls.push('content'),
    refreshStructure: async () => calls.push('structure'),
  });
  await Promise.all([reconciler.reconcile('content'), reconciler.reconcile('structure')]);
  assert.deepEqual(calls, [['Game', 'structure'], 'structure']);
  await reconciler.reconcile('content');
  assert.deepEqual(calls.slice(2), [['Game', 'content'], 'content']);
});

test('导入只扫描目标及新增子树，不扫描已有分类，刷新当前视图时保留分析缓存', async () => {
  const source = readFileSync('src/views/ModsManagement/ModsManagement.vue', 'utf8');
  const callback = source.slice(source.indexOf('    reconcileInstalledMods: async'), source.indexOf('    suppressFsRefresh: (durationMs)'));
  const code = ts.transpileModule('exports.options = ({' + callback + '});', { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const scans = [], views = [];
  const context = {
    exports: {}, availableGroups: ref([{ id: 'Target' }]), selectedGame: ref('game'), selectedGroup: ref('All'), allModsCatalogLoadedForGame: ref('game'),
    ModManager: { scanGroup: async (game, group, signal, options) => {
      scans.push(group);
      assert.equal(options.refresh, true);
      return { mods: [], groups: group === 'Target' ? [{ id: 'Target/New' }, { id: 'Target/Existing' }] : [] };
    } },
    updateAvailableGroups() {}, withOrderPersistencePaused: fn => fn(), loadTagState: async () => {},
    loadGroupMods: async (group, options) => views.push({ group, ...options }),
  };
  vm.runInNewContext(code, context);
  await context.exports.options.reconcileInstalledMods('game', 'Target', ['New']);
  assert.deepEqual(scans, ['Target','Target/New']);
  assert.deepEqual(views, [{ group: 'All', showProgress: false, refreshAnalysis: false }]);
  context.selectedGame.value = 'other';
  await context.exports.options.reconcileInstalledMods('game', 'Target', ['New']);
  assert.equal(views.length, 1);
});

test('导入合并仅追加新卡片，不修改已有对象或清除其分析订阅', async () => {
  const source = readFileSync('src/views/ModsManagement/ModsManagement.vue', 'utf8');
  const callback = source.slice(source.indexOf('const appendModsIncrementally ='), source.indexOf('const reconcileCurrentSubGroups ='));
  const code = ts.transpileModule(callback + '\nexports.append = appendModsIncrementally;', { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const old = { id: 'Old', relativePath: 'Old', preview_images: ['old.png'] };
  const state = ref([old]);
  const existing = state.value[0];
  const context = { exports: {}, mods: state, modHydrationDepth: ref(0), groupLoadToken: 1,
    totalModsCount: ref(0), loadedModsCount: ref(0), normalizeModIdentity: value => value,
    clearModAnalysisSubscriptions: () => assert.fail('不能清除已有分析'),
    queueVisibleModAnalysis: () => assert.fail('不能重新分析已有卡片'),
  };
  vm.runInNewContext(code, context);
  await context.exports.append([{ id: 'Old', relativePath: 'Old', preview_images: [] }, { id: 'New', relativePath: 'New' }], 1, { refreshAnalysis: false });
  assert.equal(state.value.length, 2);
  assert.equal(state.value[0], existing);
  assert.deepEqual(existing.preview_images, ['old.png']);
  assert.equal(context.totalModsCount.value, 2);
});
