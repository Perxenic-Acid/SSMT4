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

function gameplaySettings(t, registryFps) {
  const module = { exports: {} };
  const normalized = ts.transpileModule(readFileSync(new URL('../../src/plugin/playerTweaksGameplay.ts', import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  vm.runInNewContext(normalized, { exports: module.exports });
  const saved = [];
  const props = vue.reactive({ gameName: registryFps ? 'Custom Star Rail' : 'Custom Genshin', enabled: true, registryFps });
  const s = component(t, '../../src/plugin/components/PlayerTweaksGameplaySettings.vue', ['gameplay', 'save'], {
    'vue-i18n': i18n,
    'element-plus': { ElMessage: { success() {}, error(error) { throw new Error(error); } } },
    '../playerTweaksGameplay': module.exports,
    '../../store/ResourceManager': { ResourceManager: {
      loadGameConfig: async () => ({ gamePreset: registryFps ? 'SRMI' : 'GIMI', unrelated: 'preserve' }),
      saveGameConfig: async (game, config) => saved.push({ game, config }),
    } },
  }, props);
  return { s, saved };
}

test('崩铁 Player Tweaks 只保存 120 FPS，原神继续支持原有帧率和队伍页选项', async t => {
  for (const registry of [true, false]) {
    const { s, saved } = gameplaySettings(t, registry);
    await setImmediate();
    s.gameplay.targetFps = 165;
    s.gameplay.fastTeamPage = true;
    s.gameplay.fpsUnlock = false;
    await s.save();
    assert.equal(saved.length, 1);
    assert.equal(saved[0].config.unrelated, 'preserve');
    assert.equal(saved[0].config.playerTweaksGameplay.fpsUnlock, false);
    assert.equal(saved[0].config.playerTweaksGameplay.targetFps, registry ? 120 : 165);
    assert.equal(saved[0].config.playerTweaksGameplay.fastTeamPage, !registry);
  }
});
const dlssPlugin = enabled => ({
  manifest: { id: 'ssmt.dlss5.integration', name: 'DLSS5', author: 'SSMT', version: '0.1.0', externalDependencies: [], permissions: [] },
  enabled, externalDependencies: {}, lifecycleStatus: 'installed', official: true,
});

function marketplace(t, options = {}) {
  return component(t, '../../src/views/PluginMarketplace/PluginMarketplace.vue', [
    'installed', 'installedGameName', 'selectedGameName', 'selectedId', 'detailTab', 'openPluginSettings', 'graphicsTargetExe', 'graphicsConfigLoading',
    'catalog', 'catalogError', 'refreshAll', 'refreshing', 'loading', 'openPluginLog', 'clearSelectedPluginLog', 'logPlugin', 'pluginLog', 'togglePlugin', 'lifecycleFor',
  ], {
    '@tauri-apps/api/core': { invoke: options.invoke ?? (() => { throw new Error('Unexpected IPC'); }) },
    '@tauri-apps/plugin-dialog': {}, 'element-plus': { ElMessage: { success() {}, error: message => { throw new Error(message); } } },
    '@element-plus/icons-vue': {}, 'vue-i18n': i18n,
    'vue-router': { useRouter: () => ({ push() { throw new Error('DLSS5 settings must stay in marketplace'); } }) },
    '../../plugin/logs': options.logs ?? {}, '../../store/GamePreset': { getGamePresetPriority: () => 0 },
    '../../store/AppStateManager': { AppStateManager: { gamesList: [], appSettings: { CurrentGameName: 'GIMI' } } },
    '../../store/ResourceManager': { ResourceManager: { loadGameConfig: options.loadGameConfig ?? (() => { throw new Error('Unexpected config read'); }) } },
  });
}

test('marketplace exposes DLSS5 settings only for the selected enabled game and discards stale target reads', async t => {
  const first = deferred(), second = deferred();
  const reads = [];
  const s = marketplace(t, { loadGameConfig: game => { reads.push(game); return game === 'GIMI' ? first.promise : second.promise; } });
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

test('不兼容插件不能启用，旧启用状态仍可关闭，更新提示不覆盖不兼容提示', async t => {
  const calls = [];
  const s = marketplace(t, { invoke: async (command, args) => { calls.push({ command, args }); return []; } });
  s.installedGameName.value = 'GIMI';
  const poser = { ...dlssPlugin(false), lifecycleStatus: 'incompatible', manifest: { ...dlssPlugin(false).manifest, id: 'dev.ssmt.poser.genshin-probe' } };
  await s.togglePlugin(poser);
  assert.equal(calls.length, 0);
  assert.equal(s.lifecycleFor({ version: '9.0.0' }, poser), 'incompatible');
  await s.togglePlugin({ ...poser, enabled: true });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args.enabled, false);
  s.selectedGameName.value = 'SRMI';
  await s.togglePlugin({ ...poser, enabled: true });
  assert.equal(calls.length, 1, '切换游戏后的旧快照不能修改新游戏的插件选择');
});

test('marketplace refresh stays busy for every source and preserves the catalog on failure', async t => {
  const registry = deferred(), remote = deferred(), local = deferred(), calls = [];
  const previous = { ...dlssPlugin(false).manifest, supportedGames: [], official: true };
  let retry = false;
  const s = marketplace(t, { invoke: command => {
    calls.push(command);
    if (command === 'plugin_registry_snapshot_for_game') return retry ? [] : registry.promise;
    if (command === 'official_plugin_catalog') return retry ? [previous] : remote.promise;
    if (command === 'local_plugin_packages') return retry ? [] : local.promise;
    throw new Error(`Unexpected command ${command}`);
  } });
  s.catalog.value = [previous];
  const running = s.refreshAll();
  assert.equal(s.refreshing.value, true);
  await s.refreshAll();
  assert.equal(calls.length, 3, 'repeated refresh must not start another set of requests');
  registry.resolve([]); local.resolve([]);
  await setImmediate();
  assert.equal(s.loading.value, false);
  assert.equal(s.refreshing.value, true, 'the catalog is still being read');
  remote.reject(new Error('offline'));
  await running;
  assert.equal(s.refreshing.value, false);
  assert.equal(s.catalog.value[0].id, previous.id);
  assert.match(s.catalogError.value, /offline/);
  retry = true;
  await s.refreshAll();
  assert.equal(s.catalogError.value, '');
  assert.equal(s.refreshing.value, false);
});

test('switching the selected plugin keeps the pending log and clear action bound to their original plugin', async t => {
  const read = deferred(), cleared = [];
  const s = marketplace(t, { logs: {
    readPluginLog: id => { assert.equal(id, 'ssmt.dlss5.integration'); return read.promise; },
    clearPluginLog: async id => { cleared.push(id); },
  } });
  const bridge = { ...dlssPlugin(false), manifest: { ...dlssPlugin(false).manifest, id: 'ssmt.hoyoshade.bridge', name: 'Shade Bridge' } };
  s.installed.value = [dlssPlugin(false), bridge];
  await vue.nextTick();
  const reading = s.openPluginLog();
  s.detailTab.value = 'permissions';
  s.selectedId.value = bridge.manifest.id;
  await vue.nextTick();
  assert.equal(s.detailTab.value, 'features');
  read.resolve('original plugin log');
  await reading;
  assert.equal(s.logPlugin.value.id, 'ssmt.dlss5.integration');
  assert.equal(s.logPlugin.value.name, 'DLSS5');
  assert.equal(s.pluginLog.value, 'original plugin log');
  await s.clearSelectedPluginLog();
  assert.deepEqual(cleared, ['ssmt.dlss5.integration']);
  assert.equal(s.pluginLog.value, '');
});

function graphics(t, options = {}) {
  const calls = [], messages = [], job = deferred();
  const props = vue.reactive({ gameName: options.gameName ?? 'GIMI', targetExePath: options.targetExePath ?? 'C:/Game/YuanShen.exe' });
  const state = { root: 'C:/Assets/previous', profile: 'rtx50' };
  let pickerCount = 0;
  const s = component(t, '../../src/plugin/components/Dlss5GraphicsSettings.vue', [
    'refreshGraphicsState', 'installSelectedDlss5Route', 'hotRuntime', 'hotProfile', 'downloadProgress', 'graphicsActionBusy', 'graphicsStatusError', 'graphicsStateLoading', 'graphicsSelectedRoute',
  ], {
    '@tauri-apps/api/core': { Channel: class {}, invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'inspect_dlss5_game_state') return options.inspectState ? options.inspectState() : { state: 'not_managed' };
      if (command === 'gimi_hot_runtime_status' || command === 'dlss5_hot_runtime_status') {
        if (options.statusError) throw new Error(options.statusError);
        return options.status ? options.status(command) : state;
      }
      if (command === 'inspect_dlss5_route_options') return { routes: options.routes ?? [options.route ?? 'gimi-hot'], installedRoute: options.installedRoute };
      if (command === 'download_gimi_hot_runtime' || command === 'download_dlss5_hot_runtime') return job.promise;
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

test('崩铁、鸣潮及绝区零按各自路线下载，不调用 GIMI 或外部 Swapper', async t => {
  for (const [gameName, executable, route] of [
    ['SRMI', 'StarRail', 'ssmt-feeder'], ['WWMI', 'Client-Win64-Shipping', 'ssmt-native-bridge'],
    ['ZZMI', 'ZenlessZoneZero', 'ssmt-native-bridge'], ['ZZMIDX12', 'ZenlessZoneZero', 'ssmt-native'],
  ]) {
    const { s, calls, job } = graphics(t, { gameName, targetExePath: `C:/Game/${executable}.exe`, route });
    await s.refreshGraphicsState();
    assert.ok(calls.some(call => call.command === 'dlss5_hot_runtime_status'));
    assert.ok(!calls.some(call => call.command.includes('download')));
    const pending = s.installSelectedDlss5Route();
    assert.equal(calls.find(call => call.command === 'download_dlss5_hot_runtime').args.gameName, gameName);
    job.resolve({}); await pending;
    assert.ok(!calls.some(call => /gimi|swapper|install_managed/.test(call.command)));
  }
});

test('原神记住所选实现，Feeder 与 FSR2 Bridge 分别下载和展示组件', async t => {
  const { s, calls, job } = graphics(t, {
    gameName: '我的原神', routes: ['gimi-hot', 'ssmt-feeder'], installedRoute: 'ssmt-feeder',
    status: command => ({ root: command === 'gimi_hot_runtime_status' ? 'C:/FSR2' : 'C:/Feeder', profile: 'rtx30', route: 'feeder' }),
  });
  await s.refreshGraphicsState();
  assert.equal(s.graphicsSelectedRoute.value, 'ssmt-feeder');
  assert.equal(s.hotRuntime.value.root, 'C:/Feeder');
  const pending = s.installSelectedDlss5Route();
  assert.ok(calls.some(call => call.command === 'download_dlss5_hot_runtime' && call.args.gameName === '我的原神'));
  assert.ok(!calls.some(call => call.command === 'download_gimi_hot_runtime'));
  job.resolve({}); await pending;
  s.graphicsSelectedRoute.value = 'gimi-hot'; await vue.nextTick();
  assert.equal(s.hotRuntime.value.root, 'C:/FSR2');
  await s.installSelectedDlss5Route();
  assert.ok(calls.some(call => call.command === 'download_gimi_hot_runtime'));
});

test('损坏热注入组件仍保留自动下载修复入口', async t => {
  const { s, calls, job } = graphics(t, { gameName: 'SRMI', targetExePath: 'C:/Game/StarRail.exe', route: 'ssmt-feeder', statusError: '组件哈希不匹配' });
  await s.refreshGraphicsState();
  assert.match(s.graphicsStatusError.value, /哈希/);
  const pending = s.installSelectedDlss5Route();
  assert.ok(calls.some(call => call.command === 'download_dlss5_hot_runtime'));
  job.resolve({}); await pending;
});

test('clearing the executable while reading graphics state ends loading and discards the old response', async t => {
  const state = deferred();
  const { s, props } = graphics(t, { inspectState: () => state.promise });
  const reading = s.refreshGraphicsState();
  assert.equal(s.graphicsStateLoading.value, true);
  props.targetExePath = '';
  await vue.nextTick();
  await s.refreshGraphicsState();
  assert.equal(s.graphicsStateLoading.value, false);
  state.resolve({ state: 'managed' });
  await reading;
  assert.equal(s.hotRuntime.value, null);
  assert.equal(s.graphicsStateLoading.value, false);
});

test('switching games cannot install using the previous game route before the new scan', async t => {
  const { s, props, calls } = graphics(t);
  await s.refreshGraphicsState();
  props.gameName = 'Another';
  props.targetExePath = 'D:/Another/GenshinImpact.exe';
  await vue.nextTick();
  assert.equal(s.hotRuntime.value, null);
  await s.installSelectedDlss5Route();
  assert.ok(!calls.some(call => call.command.includes('download') || call.command.includes('install_')));
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
