import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';
import { parse } from '@vue/compiler-sfc';

const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const ordering = {};
vm.runInNewContext(transpile(readFileSync(new URL('../../src/common/GameSwitcher.ts', import.meta.url), 'utf8')), { exports: ordering });

function fixture(t) {
  const settings = vue.reactive({ homeGameSidebarPinned: false, CurrentGameName: 'A', sidebarGameOrder: ['B', 'A'], pageVisibility: {} });
  const games = vue.reactive([{ name: 'A', showSidebar: true }, { name: 'C', showSidebar: false }, { name: 'B', showSidebar: true }]);
  const route = vue.reactive({ path: '/' });
  const focused = [];
  const selected = [];
  const errors = [];
  const windowCalls = [];
  const nativeWindow = {
    startDragging: async () => { windowCalls.push('drag'); },
    isMaximized: async () => { windowCalls.push('isMaximized'); return false; },
    unmaximize: async () => { throw new Error('unmaximize is not permitted'); },
  };
  const state = { appSettings: settings, gamesList: games, selectGame: async game => { selected.push(game.name); settings.CurrentGameName = game.name; } };
  const mocks = {
    vue: { ...vue, onMounted() {}, onUnmounted() {} },
    'vue-router': { useRoute: () => route, useRouter: () => ({ push() {} }) },
    'vue-i18n': { useI18n: () => ({ t: key => key }) },
    'element-plus': { ElMessage: { error: message => errors.push(message) } },
    '@tauri-apps/api/window': { getCurrentWindow: () => nativeWindow },
    '../store/AppStateManager': { AppStateManager: state },
    '../store/GamePreset': { getGamePresetDisplayName: name => name },
    '../common/GameSwitcher': ordering,
  };
  const scope = vue.effectScope();
  t.after(() => scope.stop());
  function load(component, bindings) {
    const source = parse(readFileSync(new URL(`../../src/components/${component}.vue`, import.meta.url), 'utf8')).descriptor.scriptSetup.content;
    const exports = {};
    scope.run(() => vm.runInNewContext(transpile(`${source}\nObject.assign(exports, {${bindings.join(',')}});`), {
      exports, require: name => mocks[name] ?? {},
      document: { querySelector: selector => ({ focus: () => focused.push(selector) }) },
      console, setTimeout, clearTimeout,
    }));
    return exports;
  }
  const title = load('TitleBar', ['isHome', 'isGameSwitcherHidden', 'isGameSwitcherOpen', 'toggleGameSwitcher', 'pinGameSidebar', 'favoriteGames', 'startDrag', 'isMaximized']);
  const sidebar = load('HomeGameSidebar', ['unpin', 'selectGame', 'groups']);
  return { settings, route, title, sidebar, focused, selected, errors, state, windowCalls, nativeWindow };
}

test('最大化和普通窗口均直接启动原生拖动，不依赖还原权限或异步前置操作', async t => {
  const f = fixture(t);
  for (const maximized of [true, false]) {
    f.title.isMaximized.value = maximized;
    f.windowCalls.length = 0;
    const task = f.title.startDrag({ button: 0, detail: 1 });
    assert.deepEqual(f.windowCalls, ['drag']);
    await task;
    assert.deepEqual(f.windowCalls, ['drag', 'isMaximized']);
    assert.equal(f.title.isMaximized.value, false);
  }
  f.windowCalls.length = 0;
  await f.title.startDrag({ button: 2, detail: 1 });
  await f.title.startDrag({ button: 0, detail: 2 });
  assert.deepEqual(f.windowCalls, []);
});

test('主页图钉双向切换并转移焦点，隐藏入口不能打开菜单', async t => {
  const f = fixture(t);
  f.title.toggleGameSwitcher();
  assert.equal(f.title.isGameSwitcherOpen.value, true);
  await f.title.pinGameSidebar();
  assert.equal(f.settings.homeGameSidebarPinned, true);
  assert.equal(f.title.isGameSwitcherHidden.value, true);
  assert.equal(f.title.isGameSwitcherOpen.value, false);
  f.title.toggleGameSwitcher();
  assert.equal(f.title.isGameSwitcherOpen.value, false);
  assert.match(f.focused[0], /home-game-sidebar/);
  await f.sidebar.unpin();
  assert.equal(f.title.isGameSwitcherHidden.value, false);
  assert.match(f.focused[1], /game-switcher-root/);
});

test('非主页始终显示标题栏选择且不能固定，返回主页保留偏好', async t => {
  const f = fixture(t);
  f.settings.homeGameSidebarPinned = true;
  for (const path of ['/work', '/settings', '/mark-texture-full']) {
    f.route.path = path;
    await vue.nextTick();
    assert.equal(f.title.isHome.value, false);
    assert.equal(f.title.isGameSwitcherHidden.value, false);
    f.title.toggleGameSwitcher();
    assert.equal(f.title.isGameSwitcherOpen.value, true);
    await f.title.pinGameSidebar();
    assert.equal(f.settings.homeGameSidebarPinned, true);
    f.route.path = '/';
    await vue.nextTick();
    assert.equal(f.title.isGameSwitcherOpen.value, false);
    assert.equal(f.title.isGameSwitcherHidden.value, true);
  }
  f.settings.homeGameSidebarPinned = false;
  f.route.path = '/work';
  await f.title.pinGameSidebar();
  assert.equal(f.settings.homeGameSidebarPinned, false);
});

test('两入口保持相同常用游戏顺序，侧栏仍可选择其它游戏并处理失败', async t => {
  const f = fixture(t);
  assert.deepEqual([...f.title.favoriteGames.value.map(game => game.name)], ['B', 'A']);
  assert.deepEqual([...f.sidebar.groups.value[0].games.map(game => game.name)], ['B', 'A']);
  assert.deepEqual([...f.sidebar.groups.value[1].games.map(game => game.name)], ['C']);
  await f.sidebar.selectGame({ name: 'A' });
  assert.equal(f.selected.length, 0);
  await f.sidebar.selectGame({ name: 'C' });
  assert.equal(f.settings.CurrentGameName, 'C');
  f.state.selectGame = async () => { throw new Error('test failure'); };
  await f.sidebar.selectGame({ name: 'B' });
  assert.equal(f.errors.length, 1);
  assert.equal(f.settings.CurrentGameName, 'C');
});
