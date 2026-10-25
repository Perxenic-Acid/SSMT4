import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as vue from 'vue';

const code = ts.transpileModule(readFileSync(new URL('../../src/features/gameMaintenance/useInstallSpace.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
function fixture(invoke, automatic = true) {
  const request = vue.ref({ gameName: 'Genshin', serverId: 'cn', operation: 'install', directory: 'D:/game', audioLanguages: ['zh-cn'] });
  const cleanup = [], timers = new Map(), calls = [];
  let timerId = 0;
  const exports = {};
  vm.runInNewContext(code, { exports, setTimeout(fn) { timers.set(++timerId, fn); return timerId; }, clearTimeout(id) { timers.delete(id); },
    require(name) { return { vue: { ...vue, onBeforeUnmount: fn => cleanup.push(fn) }, '@tauri-apps/api/core': { invoke: async (...args) => {
      calls.push(args); return invoke(...args);
    } } }[name]; },
  });
  const scope = vue.effectScope();
  const state = scope.run(() => exports.useInstallSpace(() => request.value, automatic));
  return { state, request, calls, async fire() {
    const pending = [...timers.values()]; timers.clear(); pending.forEach(fn => fn());
    await Promise.resolve(); await Promise.resolve(); await vue.nextTick();
  }, dispose() { cleanup.forEach(fn => fn()); scope.stop(); } };
}
const enough = { targetVersion: '7.1.0', downloadBytes: 80, gameBytes: 100, requiredBytes: 110, availableBytes: 120, sufficient: true };

test('手动模式选择目录和语音不查询，确定操作后才估算', async () => {
  const f = fixture(() => structuredClone(enough), false);
  f.request.value.directory = 'E:/game';
  f.request.value.audioLanguages = ['ja-jp'];
  await f.fire();
  assert.equal(f.calls.length, 0);
  assert.equal(f.state.loading.value, false);
  await f.state.refresh();
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0][1].request.directory, 'E:/game');
  f.dispose();
});

test('安装空间异步读取，查询本身不发起安装或下载', async () => {
  const f = fixture(() => structuredClone(enough));
  assert.equal(f.state.loading.value, true);
  assert.equal(f.calls.length, 0);
  await f.fire();
  assert.equal(f.state.estimate.value.requiredBytes, 110);
  assert.equal(f.state.loading.value, false);
  assert.equal(f.calls[0][0], 'hoyoplay_estimate');
  assert.equal(f.calls[0][1].request.directory, 'D:/game');
  assert.equal(f.calls.length, 1);
  f.dispose();
});
test('目录、游戏、语音改变立即废弃估算，旧响应不能覆盖新结果', async () => {
  let completeOld;
  const f = fixture((_, args) => args.request.directory === 'D:/game'
    ? new Promise(resolve => { completeOld = resolve; }) : { ...enough, availableBytes: 10, sufficient: false });
  await f.fire();
  f.request.value = { ...f.request.value, gameName: 'Zenless', directory: 'E:/game', audioLanguages: ['ja-jp'] };
  assert.equal(f.state.estimate.value, null);
  await f.fire();
  assert.equal(f.state.estimate.value.sufficient, false);
  completeOld(enough);
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.state.estimate.value.availableBytes, 10);
  assert.equal(f.calls[1][1].request.audioLanguages[0], 'ja-jp');
  f.dispose();
});
test('磁盘查询或官方接口失败不伪造为零空间，允许重试', async () => {
  let failed = true;
  const f = fixture(() => { if (failed) throw Error('无法读取磁盘'); return enough; });
  await f.fire();
  assert.equal(f.state.estimate.value, null);
  assert.match(f.state.error.value, /无法读取磁盘/);
  assert.equal(f.state.loading.value, false);
  failed = false;
  assert.equal((await f.state.refresh()).sufficient, true);
  assert.equal(f.state.error.value, '');
  f.dispose();
});
test('未选择完整安装参数和离开页面后不会应用估算', async () => {
  let complete;
  const f = fixture(() => new Promise(resolve => { complete = resolve; }));
  f.request.value = null;
  await f.fire();
  assert.equal(f.calls.length, 0);
  assert.equal(f.state.loading.value, false);
  f.request.value = { gameName: 'Genshin', directory: 'D:/game' };
  await f.fire();
  f.dispose();
  complete(enough);
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.state.estimate.value, null);
});
