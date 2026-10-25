import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { ref, reactive } from 'vue';

function load(name, dependencies = {}) {
  const source = readFileSync(new URL(`../../src/views/ModsManagement/ModsManagement.${name}.ts`, import.meta.url), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, require: id => dependencies[id], queueMicrotask, setTimeout, clearTimeout });
  return exports;
}
const { reconcileModSnapshot } = load('snapshot', { './ModsManagement.paths': load('paths') });
const mod = (relativePath, enabled = true) => ({ id: relativePath, relativePath, enabled });

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
