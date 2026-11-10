import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { computed, ref, watch, reactive, nextTick } from 'vue';
const transpile = source => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

test('工作空间搜索仅筛选当前 props，切换游戏清空关键词且保留排序', async () => {
  const source = readFileSync('src/views/WorkPage/SidePanel.vue', 'utf8');
  const props = reactive({ gameKey: 'A', workspaceOptions: ['Alpha', 'Beta', 'alpha 2'], workspaceModifiedTimes: { Alpha: 1, Beta: 3, 'alpha 2': 2 } });
  const context = { props, computed, ref, watch, exports: {} };
  vm.runInNewContext(transpile(source.slice(source.indexOf('type WorkspaceSortMode'), source.indexOf('const emit =')) + '\nexports.state = { workspaceSearch, sortedWorkspaceOptions };'), context);
  const { workspaceSearch, sortedWorkspaceOptions } = context.exports.state;
  workspaceSearch.value = ' ALPHA ';
  assert.deepEqual(Array.from(sortedWorkspaceOptions.value), ['alpha 2', 'Alpha']);
  assert.equal(props.workspaceOptions.length, 3);
  props.gameKey = 'B';
  props.workspaceOptions = ['Beta B'];
  await nextTick();
  assert.equal(workspaceSearch.value, '');
  assert.deepEqual(Array.from(sortedWorkspaceOptions.value), ['Beta B']);
  workspaceSearch.value = 'missing';
  assert.equal(sortedWorkspaceOptions.value.length, 0);
  workspaceSearch.value = '';
  assert.deepEqual(Array.from(sortedWorkspaceOptions.value), ['Beta B']);
});

test('旧游戏扫描及 A→B→A 迟到结果不得覆盖当前列表', async () => {
  const source = readFileSync('src/views/WorkPage/WorkPage.vue', 'utf8');
  const code = transpile(source.slice(source.indexOf('const refreshWorkspaces = async'), source.indexOf('const createWorkspaceDirectory')) + '\nexports.refresh = refreshWorkspaces;');
  for (const nextGame of ['B', 'A']) {
    let complete;
    const requested = [];
    const context = { exports: {}, console, game: 'A', workspaceContextRevision: 1,
      getCurrentWorkspaceMemoryGameKey: () => context.game,
      isScanningWorkspaces: ref(false), workspaceModifiedTimes: ref({}), workspaceOptions: ref(['current']),
      getWorkspaceBaseDir: async game => { requested.push(game); return '/'+game; },
      mkdir: async () => {}, readDir: async () => new Promise(resolve => { complete = resolve; }),
      stat: async () => ({mtime: new Date()}), join: async (...parts) => parts.join('/'), debugLog() {},
    };
    vm.runInNewContext(code, context);
    const running = context.exports.refresh();
    await new Promise(resolve => setImmediate(resolve));
    context.game = nextGame;
    context.workspaceContextRevision += 2;
    complete([{ name: 'Old A', isDirectory: true }]);
    await running;
    assert.deepEqual(requested, ['A']);
    assert.deepEqual(Array.from(context.workspaceOptions.value), ['current']);
  }
});
