import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setImmediate } from 'node:timers/promises';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';
import { parse } from '@vue/compiler-sfc';

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function component(t, relative, bindings, mocks, props = {}) {
  const source = parse(readFileSync(new URL(relative, import.meta.url), 'utf8')).descriptor.scriptSetup.content;
  const compiled = ts.transpileModule(`${source}\nObject.assign(exports, { ${bindings.join(',')} });`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const scope = vue.effectScope();
  t.after(() => scope.stop());
  const exports = {};
  const unmount = [];
  scope.run(() => vm.runInNewContext(compiled, {
    exports, console, defineProps: () => props,
    setTimeout: () => 1, clearTimeout() {},
    require(name) {
      if (name === 'vue') return { ...vue, onMounted() {}, onUnmounted: fn => unmount.push(fn) };
      assert.ok(name in mocks, `Unexpected import: ${name}`);
      return mocks[name];
    },
  }));
  t.after(() => unmount.forEach(fn => fn()));
  return exports;
}

const i18n = { useI18n: () => ({ t: key => key, locale: vue.ref('zh-CN') }) };
const dlssPlugin = enabled => ({
  manifest: { id: 'ssmt.dlss5.integration', name: 'DLSS5', author: 'SSMT', version: '0.1.0', externalDependencies: [], permissions: [] },
  enabled, externalDependencies: {}, lifecycleStatus: 'installed', official: true,
});

test('marketplace exposes DLSS5 settings only for the selected enabled game and discards stale target reads', async t => {
  const first = deferred(), second = deferred();
  const reads = [];
  const s = component(t, '../../src/views/PluginMarketplace/PluginMarketplace.vue', [
    'installed', 'installedGameName', 'selectedGameName', 'selectedId', 'detailTab', 'openPluginSettings', 'graphicsTargetExe', 'graphicsConfigLoading',
  ], {
    '@tauri-apps/api/core': { invoke() { throw new Error('Unexpected IPC'); } },
    '@tauri-apps/plugin-dialog': {}, 'element-plus': { ElMessage: {} },
    '@element-plus/icons-vue': {}, 'vue-i18n': i18n,
    'vue-router': { useRouter: () => ({ push() { throw new Error('DLSS5 settings must stay in marketplace'); } }) },
    '../../plugin/logs': {}, '../../store/GamePreset': { getGamePresetPriority: () => 0 },
    '../../store/AppStateManager': { AppStateManager: { gamesList: [], appSettings: { CurrentGameName: 'GIMI' } } },
    '../../store/ResourceManager': { ResourceManager: { loadGameConfig: game => { reads.push(game); return game === 'GIMI' ? first.promise : second.promise; } } },
  });
  s.installed.value = [dlssPlugin(false)]; s.installedGameName.value = 'GIMI';
  await vue.nextTick();
  assert.deepEqual(reads, []);
  s.installed.value = [dlssPlugin(true)];
  await vue.nextTick();
  assert.deepEqual(reads, ['GIMI']);
  s.detailTab.value = 'dependencies'; s.openPluginSettings();
  assert.equal(s.detailTab.value, 'features');
  s.selectedGameName.value = 'Other Genshin';
  await vue.nextTick();
  assert.equal(s.graphicsTargetExe.value, '');
  assert.deepEqual(reads, ['GIMI'], 'old enabled snapshot cannot authorize reads for a new game');
  s.installed.value = [dlssPlugin(false)]; s.installedGameName.value = 'Other Genshin';
  await vue.nextTick();
  assert.deepEqual(reads, ['GIMI']);
  s.installed.value = [dlssPlugin(true)];
  await vue.nextTick();
  second.resolve({ targetExePath: 'D:/Other/GenshinImpact.exe' });
  await setImmediate();
  first.resolve({ targetExePath: 'C:/Old/YuanShen.exe' });
  await setImmediate();
  assert.equal(s.graphicsTargetExe.value, 'D:/Other/GenshinImpact.exe');
  assert.equal(s.graphicsConfigLoading.value, false);
});

function graphics(t, options = {}) {
  const calls = [], messages = [], job = deferred();
  const props = vue.reactive({ gameName: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe' });
  const state = { root: 'C:/Assets/previous', profile: 'rtx50' };
  let pickerCount = 0;
  const s = component(t, '../../src/plugin/components/Dlss5GraphicsSettings.vue', [
    'refreshGraphicsState', 'installSelectedDlss5Route', 'hotRuntime', 'hotProfile', 'downloadProgress', 'graphicsActionBusy', 'graphicsStatusError',
  ], {
    '@tauri-apps/api/core': { Channel: class {}, invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'inspect_dlss5_game_state') return { state: 'not_managed' };
      if (command === 'gimi_hot_runtime_status') return state;
      if (command === 'inspect_dlss5_route_options') return { routes: ['gimi-hot'] };
      if (command === 'download_gimi_hot_runtime') return job.promise;
      if (command === 'import_gimi_hot_runtime') return {};
      throw new Error(`Unexpected command ${command}`);
    } },
    '@tauri-apps/plugin-dialog': { open: async () => { pickerCount++; return options.directory ?? 'D:/Offline'; } },
    '@tauri-apps/plugin-opener': { openUrl() {} },
    'element-plus': { ElMessage: { error: message => messages.push(message), success: message => messages.push(message) }, ElMessageBox: {} },
    'vue-i18n': i18n,
  }, props);
  return { s, props, calls, messages, job, pickerCount: () => pickerCount };
}

test('automatic setup runs only on click, reports progress, and captures the selected profile', async t => {
  const { s, calls, job, pickerCount } = graphics(t);
  await s.refreshGraphicsState();
  assert.ok(!calls.some(call => call.command.includes('download')));
  s.hotProfile.value = 'rtx30';
  const running = s.installSelectedDlss5Route();
  assert.equal(s.graphicsActionBusy.value, true);
  const call = calls.find(call => call.command === 'download_gimi_hot_runtime');
  assert.equal(call.args.profile, 'rtx30');
  assert.equal(call.args.gameName, 'GIMI');
  call.args.onProgress.onmessage({ phase: 'downloading', downloadedBytes: 100, totalBytes: 200 });
  assert.equal(s.downloadProgress.value.downloadedBytes, 100);
  job.resolve({ root: 'C:/New', profile: 'rtx30' });
  await running;
  assert.equal(pickerCount(), 0);
  assert.equal(s.graphicsActionBusy.value, false);
  assert.ok(!calls.some(call => call.command === 'install_managed_dlss5_route'));
});

test('offline import remains available without starting any download', async t => {
  const { s, calls, pickerCount } = graphics(t);
  await s.refreshGraphicsState();
  await s.installSelectedDlss5Route(true);
  assert.equal(pickerCount(), 1);
  assert.equal(calls.find(call => call.command === 'import_gimi_hot_runtime').args.sourceDirectory, 'D:/Offline');
  assert.ok(!calls.some(call => call.command.includes('download')));
});

test('failed download retains the previous installation and permits retry', async t => {
  const { s, job, pickerCount } = graphics(t);
  await s.refreshGraphicsState();
  const before = s.hotRuntime.value;
  const running = s.installSelectedDlss5Route();
  job.reject(new Error('network unavailable'));
  await running;
  assert.equal(s.hotRuntime.value, before);
  assert.match(s.graphicsStatusError.value, /network unavailable/);
  assert.equal(s.graphicsActionBusy.value, false);
  await s.installSelectedDlss5Route(true);
  assert.equal(pickerCount(), 1);
});

test('switching games while downloading ignores old progress and old errors', async t => {
  const { s, job, props, calls } = graphics(t);
  await s.refreshGraphicsState();
  const running = s.installSelectedDlss5Route();
  const call = calls.find(call => call.command === 'download_gimi_hot_runtime');
  props.gameName = 'Another'; props.targetExePath = 'D:/Another/GenshinImpact.exe';
  call.args.onProgress.onmessage({ phase: 'downloading', downloadedBytes: 100, totalBytes: 200 });
  assert.equal(s.downloadProgress.value, null);
  job.reject(new Error('old game failed'));
  await running;
  assert.equal(s.graphicsStatusError.value, '');
  assert.equal(call.args.gameName, 'GIMI');
});
