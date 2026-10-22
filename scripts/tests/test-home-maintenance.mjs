import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';

function moduleAt(path, dependencies = {}) {
  const code = ts.transpileModule(readFileSync(new URL(path, import.meta.url), 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, setTimeout, clearTimeout, require(name) {
    assert.ok(name in dependencies, `未注入依赖 ${name}`); return dependencies[name];
  } });
  return exports;
}
const entry = moduleAt('../../src/features/gameMaintenance/entry.ts');
const versions = moduleAt('../../src/common/UpdateCheckUtils.ts');
function fixture(override) {
  const calls = [];
  const cleanup = [];
  const name = vue.ref('Genshin');
  const config = vue.ref({ gamePreset: 'GIMI', targetExePath: 'D:/game/YuanShen.exe' });
  const status = { currentServer: 'cn', selectedServer: 'cn', installedVersion: '7.1.0', latestVersion: '7.1.0',
    clientFound: true, preDownloadVersion: '7.2.0', directory: 'D:/game', audioLanguages: ['zh-cn'], recoveryPending: false };
  let task = null;
  const invoke = async (command, args) => {
    calls.push([command, args]);
    if (override) { const result = override(command, args); if (result !== undefined) return result; }
    if (command === 'hoyoplay_status') return structuredClone(status);
    if (command === 'hoyoplay_task') return task && structuredClone(task);
    if (command === 'hoyoplay_cancel') return;
    if (command === 'hoyoplay_start') {
      task = { id: 'new-task', gameName: name.value, state: 'running', phase: 'query', operation: args.request.operation,
        bytesDone: 0, bytesTotal: 100, filesDone: 0, filesTotal: 1, message: '准备资源' };
      return structuredClone(task);
    }
    throw Error(command);
  };
  const code = ts.transpileModule(readFileSync(new URL('../../src/features/gameMaintenance/useHomeMaintenance.ts', import.meta.url), 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, {
    exports, require: dependency => ({ vue: { ...vue, onBeforeUnmount: fn => cleanup.push(fn) },
      '@tauri-apps/api/core': { invoke }, '../../common/UpdateCheckUtils': versions, './entry': entry })[dependency],
    setTimeout: () => 1, clearTimeout: () => {}, console,
    window: { addEventListener() {}, removeEventListener() {} },
  });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.useHomeMaintenance(() => name.value, () => config.value, () => {}));
  return { state, status, calls, name, config, dispose: () => { cleanup.forEach(fn => fn()); scope.stop(); } };
}
async function settle() { await vue.nextTick(); await Promise.resolve(); await Promise.resolve(); }

test('预下载直接后台开始，主启动按钮保持可用，不启动游戏或弹设置', async () => {
  const f = fixture();
  await settle();
  assert.equal(f.state.canPreload.value, true);
  assert.equal(await f.state.start('preload'), true);
  await settle();
  assert.equal(f.state.busy.value, false);
  assert.equal(f.state.running.value, true);
  const commands = f.calls.map(c => c[0]);
  assert.ok(commands.includes('hoyoplay_start'));
  assert.ok(commands.every(c => ['hoyoplay_status', 'hoyoplay_task', 'hoyoplay_start'].includes(c)));
  const request = f.calls.find(c => c[0] === 'hoyoplay_start')[1].request;
  assert.equal(request.operation, 'preload');
  assert.equal(request.directory, 'D:/game');
  f.dispose();
});
test('本地落后时提示更新，查询本身不自动更新，更新期间主按钮忙碌', async () => {
  const f = fixture(); f.status.installedVersion = '7.0.0';
  await f.state.refresh();
  assert.equal(f.state.needsUpdate.value, true);
  assert.equal(f.state.canPreload.value, false);
  assert.ok(!f.calls.some(c => c[0] === 'hoyoplay_start'));
  assert.equal(await f.state.start('update'), true);
  assert.equal(f.state.busy.value, true);
  assert.equal(f.calls.find(c => c[0] === 'hoyoplay_start')[1].request.operation, 'update');
  f.dispose();
});
test('没有预下载分支不提供预下载，不把旧版本或新于官方的版本判定为必须更新', async () => {
  const f = fixture();
  f.status.preDownloadVersion = null;
  f.status.installedVersion = '7.3.0';
  await f.state.refresh();
  assert.equal(f.state.canPreload.value, false);
  assert.equal(f.state.needsUpdate.value, false);
  assert.equal(await f.state.start('preload'), false);
  assert.ok(!f.calls.some(c => c[0] === 'hoyoplay_start'));
  f.dispose();
});
test('切换游戏后旧版本响应不能覆盖新游戏，也不会锁住页面', async () => {
  let completeOld;
  const f = fixture((command, args) => {
    if (command === 'hoyoplay_status' && args.gameName === 'Genshin') return new Promise(resolve => { completeOld = resolve; });
  });
  assert.equal(f.state.busy.value, false);
  f.name.value = 'StarRail'; f.config.value = { gamePreset: 'SRMI', targetExePath: 'E:/game/StarRail.exe' };
  await settle();
  assert.equal(f.state.status.value.latestVersion, '7.1.0');
  completeOld({ latestVersion: 'obsolete-result' });
  await settle();
  assert.equal(f.state.status.value.latestVersion, '7.1.0');
  assert.equal(f.state.busy.value, false);
  f.dispose();
});

test('任务启动前的迟到空查询不能抹掉正在进行的预下载', async () => {
  let completeOld;
  const f = fixture(command => {
    if (command === 'hoyoplay_task') return new Promise(resolve => { completeOld = resolve; });
  });
  await settle();
  assert.equal(await f.state.start('preload'), true);
  const startedId = f.state.ownTask.value.id;
  completeOld(null);
  await settle();
  assert.equal(f.state.ownTask.value.id, startedId);
  assert.equal(f.state.running.value, true);
  assert.equal(f.state.busy.value, false);
  f.dispose();
});
