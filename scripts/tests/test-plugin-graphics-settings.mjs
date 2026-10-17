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
  const exports = {};
  const unmount = [];
  scope.run(() => vm.runInNewContext(compiled, {
    exports, console, URL, defineProps: () => props,
    setTimeout: () => 1, clearTimeout() {},
    require(name) {
      if (name === 'vue') return { ...vue, onMounted() {}, onUnmounted: fn => unmount.push(fn) };
      assert.ok(name in mocks, `Unexpected import: ${name}`);
      return mocks[name];
    },
  }));
  exports.dispose = () => {
    scope.stop();
    unmount.splice(0).forEach(fn => fn());
  };
  t.after(exports.dispose);
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
  manifest: { id: 'ssmt.dlss5.integration', name: 'DLSS5', author: 'SSMT', version: '0.1.0', externalDependencies: [], permissions: [], contributions: { hostExtensions: [{ id: 'graphics', settingsComponent: 'ssmt.dlss5.settings' }] } },
  enabled, externalDependencies: {}, lifecycleStatus: 'installed', official: true,
});

function marketplace(t, options = {}) {
  const appSettings = vue.reactive({ CurrentGameName: 'GIMI' });
  const settings = component(t, '../../src/views/PluginMarketplace/PluginMarketplace.vue', [
    'installed', 'installedGameName', 'selectedGameName', 'selectedId', 'detailTab', 'openPluginSettings', 'graphicsTargetExe', 'graphicsConfigLoading',
    'catalog', 'catalogError', 'allEntries', 'refreshAll', 'refreshing', 'loading', 'openPluginLog', 'clearSelectedPluginLog', 'logPlugin', 'pluginLog', 'togglePlugin', 'lifecycleFor',
    'selectedDependencies', 'dependencyActionsEnabled', 'openDependencyDownload', 'openDependencySettings', 'optionalDependency',
    'openDependencyDirectory',
  ], {
    '@tauri-apps/api/core': { invoke: options.invoke ?? (() => { throw new Error('Unexpected IPC'); }) },
    '@tauri-apps/plugin-opener': { openUrl: options.openUrl ?? (() => { throw new Error('Unexpected external download'); }), openPath: options.openPath },
    '@tauri-apps/plugin-dialog': {}, 'element-plus': { ElMessage: { success() {}, error: message => { throw new Error(message); } } },
    '@element-plus/icons-vue': {}, 'vue-i18n': i18n,
    'vue-router': { useRoute: () => ({ query: options.query ?? {} }) },
    '../../plugin/logs': options.logs ?? {}, '../../store/GamePreset': { getGamePresetPriority: () => 0 },
    '../../store/AppStateManager': { AppStateManager: { appSettings } },
    '../../store/ResourceManager': { ResourceManager: { loadGameConfig: options.loadGameConfig ?? (() => { throw new Error('Unexpected config read'); }) } },
  });
  return { ...settings, appSettings };
}

test('首次进入市场没有虚拟内置插件，目录失败仍可展示开发包且不扫描插件资源', async t => {
  const registry = deferred(), official = deferred(), local = deferred();
  const calls = [];
  const s = marketplace(t, { invoke: command => {
    calls.push(command);
    if (command === 'plugin_registry_snapshot_for_game') return registry.promise;
    if (command === 'official_plugin_catalog') return official.promise;
    if (command === 'local_plugin_packages') return local.promise;
    throw new Error(`Unexpected startup IPC: ${command}`);
  } });
  const refresh = s.refreshAll();
  registry.resolve([]);
  await setImmediate();
  assert.deepEqual(Array.from(s.allEntries.value, entry => entry.id), []);
  assert.equal(s.refreshing.value, true);
  official.reject(new Error('官方目录离线'));
  const manifest = { ...dlssPlugin(false).manifest, contributions: {}, compatibility: { games: ['GIMI'] } };
  local.resolve([{ archivePath: 'C:/Repo/build/plugin-packages/example.ssmtpkg', inspection: { manifest } }]);
  await refresh;
  assert.deepEqual(Array.from(s.allEntries.value, entry => entry.id), ['ssmt.dlss5.integration']);
  assert.match(s.catalogError.value, /官方目录离线/);
  assert.equal(s.refreshing.value, false);
  assert.equal(s.selectedId.value, 'ssmt.dlss5.integration');
  assert.equal(calls.length, 3, '发现目录不会配置、下载或扫描未安装插件的专有资源');
});

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
  s.appSettings.CurrentGameName = 'Other Genshin';
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

test('刷新相同插件快照与目录时保留 DLSS 设置，避免清空目标并重新挂载', async t => {
  let reads = 0;
  const s = marketplace(t, { loadGameConfig: async () => {
    reads += 1;
    return { targetExePath: 'C:/Game/YuanShen.exe' };
  } });
  s.installed.value = [dlssPlugin(true)];
  s.installedGameName.value = 'GIMI';
  await setImmediate();
  assert.equal(reads, 1);
  const targets = [];
  const stop = vue.watch(s.graphicsTargetExe, target => targets.push(target));
  t.after(stop);
  for (let index = 0; index < 3; index += 1) {
    s.installed.value = [dlssPlugin(true)];
    s.catalog.value = [{ ...dlssPlugin(true).manifest, supportedGames: ['GIMI'], official: true }];
    await setImmediate();
  }
  assert.equal(reads, 1, '相同游戏、插件版本及启用状态无需重新加载配置');
  assert.deepEqual(targets, [], '保持当前组件挂载所需的目标路径');
  assert.equal(s.graphicsTargetExe.value, 'C:/Game/YuanShen.exe');
  s.installed.value = [{ ...dlssPlugin(true), manifest: { ...dlssPlugin(true).manifest, version: '0.2.0' } }];
  await setImmediate();
  assert.equal(reads, 2, '实际更新已安装版本仍会重新检查设置');
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
  s.appSettings.CurrentGameName = 'SRMI';
  await s.togglePlugin({ ...poser, enabled: true });
  assert.equal(calls.length, 1, '切换游戏后的旧快照不能修改新游戏的插件选择');
});

test('依赖获取只响应已启用插件的显式操作，使用已安装声明并拒绝危险链接和过期游戏快照', async t => {
  const opened = [];
  const s = marketplace(t, { openUrl: async url => { opened.push(url); } });
  const dependency = { id: 'tool', requiredFiles: ['Tool.exe'], downloadUrl: 'https://example.com/installed/releases' };
  const plugin = { ...dlssPlugin(false), manifest: { ...dlssPlugin(false).manifest, id: 'example.tools', contributions: {}, externalDependencies: [dependency] } };
  s.catalog.value = [{ ...plugin.manifest, description: '', supportedGames: [], externalDependencies: [{ ...dependency, downloadUrl: 'https://example.com/new/releases' }] }];
  s.installed.value = [plugin]; s.installedGameName.value = 'GIMI';
  await vue.nextTick();
  assert.equal(opened.length, 0, '显示状态不会触发下载');
  await s.openDependencyDownload('tool');
  assert.equal(opened.length, 0, '停用时不提供资源获取');
  s.installed.value = [{ ...plugin, enabled: true }];
  await vue.nextTick();
  await s.openDependencyDownload('tool');
  assert.deepEqual(opened, ['https://example.com/installed/releases']);
  assert.equal(s.optionalDependency({ requiredFor: ['external-manager'] }), true);
  assert.equal(s.optionalDependency({ requiredFor: ['launch'] }), false);
  assert.equal(s.optionalDependency({}), false);
  for (const url of ['javascript:alert(1)', 'file:///C:/Tool.exe', 'http://example.com', 'https://user:pass@example.com', 'https://example.com/\nfile']) {
    s.installed.value = [{ ...plugin, enabled: true, manifest: { ...plugin.manifest, externalDependencies: [{ ...dependency, downloadUrl: url }] } }];
    await s.openDependencyDownload('tool');
  }
  assert.equal(opened.length, 1);
  s.installed.value = [{ ...plugin, enabled: true, lifecycleStatus: 'incompatible' }];
  await s.openDependencyDownload('tool');
  assert.equal(opened.length, 1, '不兼容的旧启用状态不能获取组件');
  s.appSettings.CurrentGameName = 'Other game';
  await s.openDependencyDownload('tool');
  assert.equal(opened.length, 1, '旧启用快照不能为新游戏授权下载');
  s.installed.value = [];
  await s.openDependencyDownload('tool');
  assert.equal(opened.length, 1, '仅有目录条目不能授权下载');
});

test('依赖卡片复用 DLSS5 配置入口，停用及切换游戏时不打开配置', async t => {
  const s = marketplace(t, { loadGameConfig: async () => ({}) });
  s.installed.value = [dlssPlugin(false)]; s.installedGameName.value = 'GIMI';
  await vue.nextTick();
  s.detailTab.value = 'dependencies'; s.openDependencySettings();
  assert.equal(s.detailTab.value, 'dependencies');
  s.installed.value = [dlssPlugin(true)];
  await vue.nextTick();
  s.openDependencySettings();
  assert.equal(s.detailTab.value, 'features');
  s.detailTab.value = 'dependencies'; s.appSettings.CurrentGameName = 'Other game';
  s.openDependencySettings();
  assert.equal(s.detailTab.value, 'dependencies');
});

test('市场跟随全局游戏，HoYoShade 的旧入口定位依赖卡片且只打开已验证目录', async t => {
  const opened = [];
  const s = marketplace(t, { query: { plugin: 'ssmt.hoyoshade.bridge', tab: 'dependencies' }, openPath: async path => opened.push(path) });
  const plugin = { ...dlssPlugin(true), manifest: { ...dlssPlugin(true).manifest, id: 'ssmt.hoyoshade.bridge', contributions: { hostExtensions: [{ settingsComponent: 'ssmt.reshade.settings' }] } }, externalDependencies: { hoyoshade: { path: 'C:/Tools/HoYoShade', status: 'ready' } } };
  s.installed.value = [plugin]; s.installedGameName.value = 'GIMI';
  await vue.nextTick();
  assert.equal(s.selectedId.value, 'ssmt.hoyoshade.bridge');
  assert.equal(s.detailTab.value, 'dependencies');
  s.detailTab.value = 'features'; s.openPluginSettings();
  assert.equal(s.detailTab.value, 'dependencies');
  await s.openDependencyDirectory('hoyoshade');
  assert.deepEqual(opened, ['C:/Tools/HoYoShade']);
  s.installed.value = [{ ...plugin, externalDependencies: { hoyoshade: { path: 'C:/Tools/Unexpected.exe', status: 'invalid' } } }];
  await s.openDependencyDirectory('hoyoshade');
  assert.equal(opened.length, 1);
  s.appSettings.CurrentGameName = 'Default';
  assert.equal(s.selectedGameName.value, '', '不自动回落到首个游戏');
  assert.equal(s.dependencyActionsEnabled.value, false);
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
    'refreshGraphicsState', 'installSelectedDlss5Route', 'hotRuntime', 'hotProfile', 'downloadProgress', 'graphicsActionBusy', 'graphicsStatusError', 'graphicsStateLoading', 'graphicsRoutesLoading', 'graphicsSelectedRoute', 'hotHistory', 'historySelection', 'manageHotHistory', 'dlss5LastRun',
  ], {
    '@tauri-apps/api/core': { Channel: class {}, invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'inspect_dlss5_game_state') return options.inspectState ? options.inspectState() : { state: 'not_managed' };
      if (command === 'gimi_hot_runtime_status' || command === 'dlss5_hot_runtime_status') {
        if (options.statusError) throw new Error(options.statusError);
        return options.status ? options.status(command) : state;
      }
      if (command === 'inspect_dlss5_route_options') return options.scan ? options.scan(args) : { routes: options.routes ?? [options.route ?? 'gimi-hot'], installedRoute: options.installedRoute };
      if (command === 'inspect_dlss5_last_run') return options.lastRun ? options.lastRun(args) : { verdict: 'unknown', reason: 'no_log' };
      if (command === 'download_gimi_hot_runtime' || command === 'download_dlss5_hot_runtime') return job.promise;
      if (command === 'import_gimi_hot_runtime') return {};
      if (command === 'dlss5_hot_runtime_history') return typeof options.history === 'function' ? options.history(args) : options.history ?? [];
      if (command === 'select_dlss5_hot_runtime' || command === 'remove_dlss5_hot_runtime') return options.manage ? options.manage(command, args) : null;
      throw new Error(`Unexpected command ${command}`);
    } },
    '@tauri-apps/plugin-dialog': { open: async () => { pickerCount++; return options.directory ?? 'D:/Offline'; } },
    '@tauri-apps/plugin-opener': { openUrl() {} },
    'element-plus': { ElMessage: { error: message => messages.push(message), success: message => messages.push(message) }, ElMessageBox: { confirm: options.confirm ?? (async () => {}) } },
    'vue-i18n': i18n,
  }, props);
  return { s, props, calls, messages, job, pickerCount: () => pickerCount };
}

test('组件校验与历史挂起时，路线先就绪，配置操作可用且旧读取不能覆盖操作结果', async t => {
  const status = deferred(), history = deferred();
  const { s, calls, job } = graphics(t, { status: () => status.promise, history: () => history.promise });
  const reading = s.refreshGraphicsState();
  await setImmediate();
  assert.ok(calls.some(call => call.command === 'inspect_dlss5_route_options'));
  assert.ok(calls.some(call => call.command === 'dlss5_hot_runtime_history'));
  assert.equal(s.graphicsSelectedRoute.value, 'gimi-hot');
  assert.equal(s.graphicsRoutesLoading.value, false);
  assert.equal(s.graphicsStateLoading.value, true);
  s.hotProfile.value = 'rtx30';
  const installing = s.installSelectedDlss5Route();
  assert.equal(s.graphicsActionBusy.value, true);
  assert.equal(calls.find(call => call.command === 'download_gimi_hot_runtime').args.profile, 'rtx30');
  status.resolve({ root: 'C:/Stale', profile: 'rtx50' });
  history.resolve([{ root: 'C:/Stale', current: true }]);
  await reading;
  assert.equal(s.hotRuntime.value, null);
  assert.equal(s.hotHistory.value.length, 0);
  assert.equal(s.hotProfile.value, 'rtx30');
  job.reject(new Error('保持原安装'));
  await installing;
});

test('单项状态读取失败不阻断其他状态、路线、历史和修复入口', async t => {
  const { s, calls } = graphics(t, {
    inspectState: async () => { throw new Error('读取旧安装失败'); },
    status: async command => {
      if (command === 'gimi_hot_runtime_status') throw new Error('FSR2 记录损坏');
      return { root: 'C:/Feeder', profile: 'rtx30', route: 'feeder' };
    },
    routes: ['gimi-hot', 'ssmt-feeder'], installedRoute: 'ssmt-feeder',
    history: [{ root: 'C:/Feeder', current: true }],
  });
  await s.refreshGraphicsState();
  assert.equal(s.graphicsSelectedRoute.value, 'ssmt-feeder');
  assert.equal(s.hotRuntime.value.root, 'C:/Feeder');
  assert.equal(s.hotHistory.value.length, 1);
  assert.ok(calls.some(call => call.command === 'inspect_dlss5_last_run'));
  assert.equal(s.graphicsStateLoading.value, false);
  assert.equal(s.graphicsRoutesLoading.value, false);
  assert.match(s.graphicsStatusError.value, /记录损坏/);
});

test('慢速组件状态不会覆盖等待期间选择的显卡，历史挂起不阻断其他结果', async t => {
  const status = deferred(), history = deferred();
  const { s } = graphics(t, { status: () => status.promise, history: () => history.promise });
  const reading = s.refreshGraphicsState();
  await setImmediate();
  s.hotProfile.value = 'rtx30';
  status.resolve({ root: 'C:/Configured', profile: 'rtx50' });
  await setImmediate();
  assert.equal(s.hotRuntime.value.root, 'C:/Configured');
  assert.equal(s.hotProfile.value, 'rtx30');
  assert.equal(s.dlss5LastRun.value.verdict, 'unknown');
  history.resolve([]);
  await reading;
  assert.equal(s.graphicsStateLoading.value, false);
});

test('切换实例后旧状态、历史和错误全部丢弃，请求始终使用原实例快照', async t => {
  const oldStatus = deferred(), oldHistory = deferred();
  const { s, props, calls } = graphics(t, {
    status: () => oldStatus.promise, history: () => oldHistory.promise,
  });
  const reading = s.refreshGraphicsState();
  props.gameName = 'New Instance';
  props.targetExePath = 'D:/Game/StarRail.exe';
  await vue.nextTick();
  oldStatus.reject(new Error('旧实例错误'));
  oldHistory.resolve([{ root: 'C:/Stale', current: true }]);
  await reading;
  assert.equal(s.hotRuntime.value, null);
  assert.equal(s.hotHistory.value.length, 0);
  assert.equal(s.graphicsStatusError.value, '');
  assert.equal(s.graphicsSelectedRoute.value, '');
  assert.ok(calls.filter(call => 'gameName' in call.args).every(call => call.args.gameName === 'GIMI'));
});

test('离开设置后迟到的组件读取不再更新状态或继续读取日志', async t => {
  const status = deferred(), history = deferred();
  const { s, calls } = graphics(t, { status: () => status.promise, history: () => history.promise });
  const reading = s.refreshGraphicsState();
  s.dispose();
  status.resolve({ root: 'C:/Old', profile: 'rtx30' });
  history.resolve([{ root: 'C:/Old', current: true }]);
  await reading;
  assert.equal(s.hotRuntime.value, null);
  assert.equal(s.hotHistory.value.length, 0);
  assert.equal(s.graphicsStateLoading.value, false);
  assert.ok(!calls.some(call => call.command === 'inspect_dlss5_last_run'));
});

test('同一实例重读后，上一轮迟到的校验结果不能覆盖新组件状态', async t => {
  const status = deferred(), history = deferred();
  let statusReads = 0, historyReads = 0;
  const { s } = graphics(t, {
    status: () => ++statusReads <= 2 ? status.promise : { root: 'C:/Current', profile: 'rtx50' },
    history: () => ++historyReads === 1 ? history.promise : [{ root: 'C:/Current', current: true }],
  });
  const first = s.refreshGraphicsState();
  await s.refreshGraphicsState();
  status.resolve({ root: 'C:/Stale', profile: 'rtx30' });
  history.resolve([{ root: 'C:/Stale', current: true }]);
  await first;
  assert.equal(s.hotRuntime.value.root, 'C:/Current');
  assert.equal(s.hotHistory.value[0].root, 'C:/Current');
  assert.equal(s.hotProfile.value, 'rtx50');
  assert.equal(s.graphicsStateLoading.value, false);
});

test('历史组件切换绑定当前实例，清理需明确确认，取消和切换游戏不会提交清理', async t => {
  const history = [{ root: 'C:/Owned/previous', profile: 'rtx30', revision: 'v1', route: 'native-bridge', usable: true, current: false }];
  const confirmation = deferred();
  const { s, props, calls } = graphics(t, { gameName: 'WWMI', targetExePath: 'C:/Game/Client-Win64-Shipping.exe', history, confirm: () => confirmation.promise });
  await s.refreshGraphicsState();
  await s.manageHotHistory();
  const selected = calls.find(call => call.command === 'select_dlss5_hot_runtime');
  assert.equal(selected.args.root, history[0].root);
  assert.equal(selected.args.gameName, 'WWMI');
  const pending = s.manageHotHistory(true);
  assert.ok(!calls.some(call => call.command === 'remove_dlss5_hot_runtime'));
  props.gameName = 'Another Instance'; await vue.nextTick();
  confirmation.resolve(); await pending;
  assert.ok(!calls.some(call => call.command === 'remove_dlss5_hot_runtime'));
  const cancelled = graphics(t, { history, confirm: async () => { throw 'cancel'; } });
  await cancelled.s.refreshGraphicsState(); await cancelled.s.manageHotHistory(true);
  assert.ok(!cancelled.calls.some(call => call.command === 'remove_dlss5_hot_runtime'));
});

test('FSR2 Bridge 与 Feeder 从同一历史入口切换，缺少所有权的旧目录不提交清理', async t => {
  let active = 'ssmt-feeder';
  const history = [
    { root: 'C:/Owned/FSR2', route: 'gimi-hot', profile: 'rtx50', revision: 'fsr2-v1', usable: true, removable: false, cleanupReason: '缺少实例所有权记录' },
    { root: 'C:/Owned/Feeder', route: 'feeder', profile: 'rtx30', revision: 'v1', usable: true, removable: true },
  ];
  const { s, calls } = graphics(t, {
    routes: ['gimi-hot', 'ssmt-feeder'], history,
    status: command => ({ root: command === 'gimi_hot_runtime_status' ? history[0].root : history[1].root, profile: 'rtx50', route: 'feeder' }),
    manage: (command, args) => {
      if (command === 'select_dlss5_hot_runtime') active = args.root === history[0].root ? 'gimi-hot' : 'ssmt-feeder';
    },
  });
  await s.refreshGraphicsState();
  s.graphicsSelectedRoute.value = 'ssmt-feeder';
  s.historySelection.value = history[0].root;
  await s.manageHotHistory();
  assert.equal(active, 'gimi-hot');
  assert.equal(s.graphicsSelectedRoute.value, 'gimi-hot');
  assert.equal(s.hotRuntime.value.root, history[0].root);
  s.historySelection.value = history[0].root;
  await s.manageHotHistory(true);
  assert.ok(!calls.some(call => call.command === 'remove_dlss5_hot_runtime'));
  s.historySelection.value = history[1].root;
  await s.manageHotHistory();
  assert.equal(active, 'ssmt-feeder');
  assert.equal(s.graphicsSelectedRoute.value, 'ssmt-feeder');
  assert.equal(s.hotRuntime.value.root, history[1].root);
});

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
