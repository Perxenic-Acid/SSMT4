import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { reactive } from 'vue';

function monitor(failListen = false) {
  const handlers = new Map(), calls = [], stopped = [];
  const mocks = {
    vue: { reactive },
    '@tauri-apps/api/event': { listen: async (name, handler) => {
      if (failListen) throw new Error('listen unavailable');
      handlers.set(name, event => handler({ payload: event }));
      return () => stopped.push(name);
    } },
    '@tauri-apps/api/core': { invoke: async (...args) => calls.push(args) },
    '../plugin/launchEventBus': { subscribeLaunchEvents: async handler => {
      handlers.set('launch-event', handler); return () => stopped.push('launch-event');
    } },
  };
  const source = readFileSync(new URL('../../src/store/LaunchMonitor.ts', import.meta.url), 'utf8');
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, console: { warn() {} }, require: name => {
    assert.ok(name in mocks, `Unexpected import ${name}`); return mocks[name];
  } });
  const h = exports.createLaunchMonitor();
  return { ...h, calls, stopped, emit: (kind, text = '', id = 'a') => h.handleConsole({
    sessionId: id, program: `${id}.exe`, kind, text, pid: 123, code: kind === 'exit' ? 7 : undefined,
  }) };
}

test('监听在启动之前就绪，输入发给所选会话，完成和隐藏不终止程序', async () => {
  const h = monitor();
  await h.begin();
  h.emit('started');
  h.emit('stdout', 'prompt>');
  h.emit('log_path', 'C:/Logs/launch.log');
  h.finish();
  assert.equal(h.state.visible, true);
  h.state.visible = false;
  h.emit('stdout', 'still running');
  assert.equal(h.state.visible, false);
  await h.send('hello');
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0][0], 'write_launch_console');
  assert.equal(h.calls[0][1].sessionId, 'a');
  assert.equal(h.calls[0][1].text, 'hello');
  assert.equal(h.state.sessions[0].logPath, 'C:/Logs/launch.log');
  h.emit('exit');
  await assert.rejects(() => h.send('late'));
  assert.equal(h.state.sessions[0].code, 7);
  h.dispose();
  assert.equal(h.stopped.length, 2);
});

test('上一次启动的进程继续输出，但其启动事件不会覆盖新启动状态', async () => {
  const h = monitor();
  await h.begin(); h.emit('started'); h.finish();
  await h.begin(); h.emit('started', '', 'b');
  h.handleEvent({ event: 'inject_complete', consoleSessionId: 'b' });
  h.handleEvent({ event: 'launch_error', consoleSessionId: 'a' });
  assert.equal(h.state.currentEvent.event, 'inject_complete');
  h.emit('stdout', 'old output', 'a');
  assert.equal(h.state.sessions[0].output, 'old output');
  assert.equal(h.state.selectedId, 'b');
});

test('超量输出和历史记录有界，未知会话输出被忽略，文本不作为 HTML', async () => {
  const h = monitor(); await h.begin();
  h.emit('stdout', 'ignored'); assert.equal(h.state.sessions.length, 0);
  h.emit('started'); h.emit('stdout', 'x'.repeat(200000));
  assert.equal(h.state.sessions[0].output.length, 128 * 1024);
  assert.equal(h.state.sessions[0].truncated, true);
  h.emit('stdout', '\x1b[31m<script>plain text</script>\x1b[0m');
  assert.ok(h.state.sessions[0].output.endsWith('<script>plain text</script>'));
  for (let i = 0; i < 30; i++) { h.emit('started', '', `${i}`); h.emit('exit', '', `${i}`); }
  assert.equal(h.state.sessions.length, 24);
  assert.ok(h.state.sessions.some(item => item.id === 'a' && item.running));
});

test('无控制台路径启动完成后关闭短暂提示', async () => {
  const h = monitor(); await h.begin(); h.finish();
  assert.equal(h.state.visible, false);
  assert.equal(h.state.launching, false);
  assert.equal(h.state.sessions.length, 0);
});

test('监听失败不阻断游戏启动，并清理已经成功注册的监听器', async () => {
  const h = monitor(true);
  await h.begin();
  assert.equal(h.state.launching, true);
  assert.equal(h.stopped.length, 1);
  h.finish();
  assert.equal(h.state.launching, false);
});

test('默认只显示阶段，日志来源不展开浮窗、不抢所选会话，并拒绝输入', async () => {
  const h = monitor(); await h.begin();
  assert.equal(h.state.collapsed, true);
  h.emit('started');
  h.state.collapsed = false;
  h.state.visible = false;
  h.emit('log_started', '', 'reshade');
  assert.equal(h.state.visible, false);
  assert.equal(h.state.selectedId, 'a');
  h.state.selectedId = 'reshade';
  assert.equal(h.state.sessions[1].readOnly, true);
  await assert.rejects(() => h.send('input'), /不接受输入/);
  h.emit('exit', '', 'a');
  h.emit('stdout', 'game still rendering', 'reshade');
  assert.equal(h.state.sessions[1].running, true);
  assert.equal(h.state.sessions[1].output, 'game still rendering');
  h.emit('error', '具体读取错误', 'reshade');
  h.emit('exit', '', 'reshade');
  assert.equal(h.state.sessions[1].error, '具体读取错误');
  h.finish();
  await h.begin();
  h.emit('started', '', 'new-run');
  h.emit('log_started', 'a', 'old-shade');
  assert.equal(h.state.sessions.find(item => item.id === 'old-shade').generation, 1);
  assert.equal(h.state.selectedId, 'new-run');
  h.emit('log_started', 'missing-parent', 'ignored-shade');
  assert.equal(h.state.sessions.some(item => item.id === 'ignored-shade'), false);
});
