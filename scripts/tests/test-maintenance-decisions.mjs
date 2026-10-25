import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';

const source = readFileSync(new URL('../../src/features/gameMaintenance/GameMaintenanceSettings.vue', import.meta.url), 'utf8')
  .match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
const compile = source => ts.transpileModule(source, { compilerOptions: {
  target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS,
} }).outputText;
const spaceCode = compile(readFileSync(new URL('../../src/features/gameMaintenance/useInstallSpace.ts', import.meta.url), 'utf8'));

function fixture(override = () => {}) {
  const calls = [], cleanup = [], confirmations = [], timers = new Map();
  let timerId = 0;
  const props = vue.reactive({ gameName: 'Genshin', gamePreset: 'GIMI', targetExePath: 'D:/game/YuanShen.exe' });
  const local = { clientFound: true, currentServer: 'cn', selectedServer: 'cn', directory: 'D:/game',
    audioLanguages: ['zh-cn'], latestVersion: '', preDownloadVersion: null, recoveryPending: false };
  const invoke = async (command, args) => {
    calls.push([command, args]);
    const custom = override(command, args);
    if (custom !== undefined) return custom;
    if (command === 'hoyoplay_task') return null;
    if (command === 'hoyoplay_status') return { ...local, selectedServer: args.serverId ?? 'cn',
      latestVersion: args.localOnly ? '' : '7.1.0' };
    if (command === 'hoyoplay_start') return { id: 'task', state: 'running', gameName: props.gameName };
    throw Error(`意外调用 ${command}`);
  };
  const modules = { vue: { ...vue, onBeforeUnmount: fn => cleanup.push(fn) },
    '@tauri-apps/api/core': { invoke }, '@tauri-apps/api/path': { dirname: async () => 'D:/game' },
    '@tauri-apps/plugin-dialog': { open: async () => 'E:/game' },
    'element-plus': { ElMessageBox: { confirm: async (...args) => confirmations.push(args) } },
    'vue-i18n': { useI18n: () => ({ locale: vue.ref('zhs') }) }, './NOTICE.md?raw': { default: '' } };
  const evaluate = code => {
    const exports = {};
    vm.runInNewContext(code, { exports, require: name => {
      assert.ok(name in modules, name); return modules[name];
    }, defineProps: () => props, defineEmits: () => () => {},
    setTimeout: (fn, delay) => { timers.set(++timerId, { fn, delay }); return timerId; }, clearTimeout: id => timers.delete(id) });
    return exports;
  };
  modules['./useInstallSpace'] = evaluate(spaceCode);
  const scope = vue.effectScope();
  const state = scope.run(() => evaluate(compile(source + '\nexport {server, languages, directory, status, canPreload, start, selectServer, toggleLanguage, chooseDirectory, refresh};')));
  return { calls, confirmations, state, props, async sync() {
    for (const [id, { fn, delay }] of [...timers]) if (delay === 200) { timers.delete(id); fn(); }
    await settle();
  }, dispose() { cleanup.forEach(fn => fn()); scope.stop(); } };
}
async function settle() { for (let i = 0; i < 5; i++) { await vue.nextTick(); await Promise.resolve(); } }

test('预下载入口只在所选渠道已确认有预下载时出现', async () => {
  const f = fixture(); await settle();
  assert.equal(f.state.canPreload.value, false);
  f.state.status.value = { ...f.state.status.value, preDownloadVersion: '7.2.0' };
  assert.equal(f.state.canPreload.value, true);
  f.state.selectServer('global');
  assert.equal(f.state.canPreload.value, false);
  await settle();
  f.state.status.value = { ...f.state.status.value, preDownloadVersion: '7.2.0' };
  assert.equal(f.state.canPreload.value, false);
  f.dispose();
});

test('打开仅查询本地，连续选择渠道语音和目录不联网', async () => {
  const f = fixture(); await settle();
  assert.equal(f.calls.filter(([c]) => c === 'hoyoplay_status').length, 1);
  assert.equal(f.calls.find(([c]) => c === 'hoyoplay_status')[1].localOnly, true);
  f.state.selectServer('global'); f.state.toggleLanguage('ja-jp'); await f.state.chooseDirectory();
  await settle();
  assert.equal(f.state.server.value, 'global');
  assert.equal(f.state.directory.value, 'E:/game');
  assert.ok(f.calls.filter(([c]) => c === 'hoyoplay_status').every(([, a]) => a.localOnly));
  await f.sync();
  const remote = f.calls.filter(([c, a]) => c === 'hoyoplay_status' && !a.localOnly);
  assert.equal(remote.length, 1);
  assert.equal(remote[0][1].serverId, 'global');
  assert.equal(remote[0][1].homeOnly, true);
  assert.ok(f.state.languages.value.includes('ja-jp'));
  f.dispose();
});

test('点击执行才检查，确认后携带最终选择发起切服', async () => {
  const f = fixture(); await settle(); f.state.selectServer('global');
  await f.state.start('switch');
  assert.equal(f.calls.filter(([c, a]) => c === 'hoyoplay_status' && !a.localOnly).length, 1);
  assert.equal(f.confirmations.length, 1);
  const request = f.calls.find(([c]) => c === 'hoyoplay_start')[1].request;
  assert.equal(request.serverId, 'global'); assert.equal(request.operation, 'switch');
  f.dispose();
});

test('执行检查失败不确认、不启动；离开游戏后丢弃检查结果', async () => {
  const f = fixture((c, a) => c === 'hoyoplay_status' && !a.localOnly ? Promise.reject(Error('离线')) : undefined);
  await settle(); await f.state.start('update');
  assert.equal(f.confirmations.length, 0);
  assert.ok(!f.calls.some(([c]) => c === 'hoyoplay_start')); f.dispose();
  let finish;
  const g = fixture((c, a) => c === 'hoyoplay_status' && !a.localOnly ? new Promise(r => { finish = r; }) : undefined);
  await settle(); const starting = g.state.start('update'); g.props.gameName = 'Other';
  finish({ clientFound: true }); await starting;
  assert.equal(g.confirmations.length, 0); g.dispose();
});

test('安装决定后才估算空间，空间不足不启动', async () => {
  const f = fixture((c, a) => {
    if (c === 'hoyoplay_status') return { clientFound: false, selectedServer: a.serverId ?? 'cn',
      audioLanguages: ['zh-cn'], latestVersion: a.localOnly ? '' : '7.1.0', currentServer: null };
    if (c === 'hoyoplay_estimate') return { sufficient: false };
  });
  await settle();
  assert.ok(!f.calls.some(([c]) => c === 'hoyoplay_estimate'));
  await f.state.start('install');
  assert.equal(f.calls.filter(([c]) => c === 'hoyoplay_estimate').length, 1);
  assert.equal(f.confirmations.length, 0);
  assert.ok(!f.calls.some(([c]) => c === 'hoyoplay_start'));
  f.dispose();
});

test('本地初始化迟到不覆盖用户已选语音，主动检查保留选择', async () => {
  let finish;
  const f = fixture((c, a) => c === 'hoyoplay_status' && a.localOnly ? new Promise(r => { finish = r; }) : undefined);
  await settle(); f.state.toggleLanguage('ja-jp');
  finish({ currentServer: 'cn', selectedServer: 'cn', clientFound: true, audioLanguages: ['en-us'] });
  await settle();
  assert.ok(f.state.languages.value.includes('ja-jp'));
  f.state.selectServer('global'); await f.state.refresh('global');
  assert.equal(f.state.server.value, 'global');
  assert.ok(f.state.languages.value.includes('ja-jp'));
  f.dispose();
});
