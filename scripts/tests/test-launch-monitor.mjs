import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { reactive } from 'vue';

function monitor(failListen = false, ready = Promise.resolve()) {
  const handlers = new Map(), calls = [], stopped = [];
  const mocks = {
    vue: { reactive },
    '@tauri-apps/api/event': { listen: async (name, handler) => {
      await ready;
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
  return { ...h, calls, stopped, shortcut: () => handlers.get('open-launch-console')?.(), emit: (kind, text = '', id = 'a') => h.handleConsole({
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
  assert.equal(h.stopped.length, 3);
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

test('无控制台路径启动完成后保留阶段提示，由用户空闲时关闭', async () => {
  const h = monitor(); await h.begin(); h.finish();
  assert.equal(h.state.visible, true);
  assert.equal(h.state.launching, false);
  assert.equal(h.state.sessions.length, 0);
  assert.equal(h.hasActiveLoad(), false);
  assert.equal(h.dismiss(), true);
  assert.equal(h.state.visible, false);
});

test('启动未结束或任意来源运行时不可关闭，不以所选来源状态判断', async () => {
  const h = monitor(); await h.begin();
  assert.equal(h.hasActiveLoad(), true);
  assert.equal(h.dismiss(), false);
  h.emit('started'); h.finish();
  await h.begin(); h.emit('started', '', 'b'); h.emit('exit', '', 'b'); h.finish();
  assert.equal(h.state.selectedId, 'b');
  assert.equal(h.dismiss(), false); // 上一代 a 仍在运行。
  h.emit('log_started', 'a', 'reshade');
  h.emit('exit');
  assert.equal(h.dismiss(), false); // Run.exe 退出不代表 ReShade 跟随结束。
  h.emit('exit', '', 'reshade');
  assert.equal(h.hasActiveLoad(), false);
  assert.equal(h.state.visible, true); // 空闲不会自行隐藏。
  assert.equal(h.dismiss(), true);
  assert.equal(h.state.visible, false);
});

test('启动失败仍等待流程释放，来源错误后空闲可关闭且记录保留', async () => {
  const h = monitor(); await h.begin();
  h.handleEvent({ event: 'launch_error', message: 'failed' });
  assert.equal(h.dismiss(), false);
  h.emit('started'); h.emit('error', 'read failed'); h.finish();
  assert.equal(h.hasActiveLoad(), false);
  assert.equal(h.dismiss(), true);
  assert.equal(h.state.sessions[0].error, 'read failed');
  assert.equal(h.state.failure.event, 'launch_error');
});

test('关闭后迟到输出不重新打开，下一次启动重新展开面板并保留历史', async () => {
  const h = monitor(); await h.begin();
  h.emit('started'); h.emit('stdout', 'retained'); h.emit('exit'); h.finish();
  h.state.collapsed = false;
  assert.equal(h.dismiss(), true);
  assert.equal(h.state.collapsed, true);
  h.emit('stdout', ' late');
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'a' });
  h.emit('log_started', 'a', 'late-log');
  assert.equal(h.state.visible, false);
  assert.equal(h.state.sessions[0].output, 'retained late');
  await h.begin();
  assert.equal(h.state.visible, true);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.sessions[0].output, 'retained late');
});

test('监听失败不阻断游戏启动，并清理已经成功注册的监听器', async () => {
  const h = monitor(true);
  await h.begin();
  assert.equal(h.state.launching, true);
  assert.equal(h.stopped.length, 1);
  h.finish();
  assert.equal(h.state.launching, false);
});

test('日志来源不重新展开已收起的浮窗、不抢所选会话，并拒绝输入', async () => {
  const h = monitor(); await h.begin();
  assert.equal(h.state.collapsed, false);
  h.state.collapsed = true;
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


test('监听尚未就绪时立即展开，并在开始新代次前清空旧错误', async () => {
  let release;
  const h = monitor(false, new Promise(resolve => { release = resolve; }));
  h.state.failure = { event: 'launch_error' };
  const pending = h.begin();
  assert.equal(h.state.visible, true);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.generation, 1);
  assert.equal(h.state.failure, null);
  release(); await pending;
});

test('启动队列返回或进程出现不代表注入完成，成功事件后收起且日志继续', async () => {
  const h = monitor(); await h.begin(); h.emit('started');
  h.handleEvent({ event: 'launch_started', consoleSessionId: 'a' });
  h.handleEvent({ event: 'target_created', consoleSessionId: 'a' });
  h.finish(true, true);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.launching, true);
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'a' });
  assert.equal(h.state.collapsed, true);
  assert.equal(h.state.launching, false);
  assert.equal(h.state.sessions[0].running, true);
  h.open();
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'a' });
  assert.equal(h.state.collapsed, false); // 重复完成事件不收起手动打开的面板。
});

test('完成事件早于队列返回时仍等待队列，晚于队列且尚无事件时仍等待注入', async () => {
  const h = monitor(); await h.begin(); h.emit('started');
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'a' });
  assert.equal(h.state.collapsed, false);
  h.finish(true, true); assert.equal(h.state.collapsed, true);
  await h.begin(); h.emit('started', '', 'b');
  h.finish(true, true); assert.equal(h.state.launching, true);
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'b' });
  assert.equal(h.state.collapsed, true);
});

test('失败不收起且后续进度不覆盖错误，取消与无控制台成功分别处理', async () => {
  const h = monitor(); await h.begin();
  h.handleEvent({ event: 'launch_error', message: 'injection failed' });
  h.handleEvent({ event: 'launch_error', message: 'late channel error' });
  h.handleEvent({ event: 'target_created' }); h.finish(true, true);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.failure.message, 'injection failed');
  assert.equal(h.state.launching, false);
  await h.begin(); h.finish(false);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.currentEvent.event, 'launch_cancelled');
  await h.begin(); h.finish(true);
  assert.equal(h.state.collapsed, true);
  assert.equal(h.state.currentEvent.event, 'launch_complete');
});

test('未收到完成事件的进程退出或通道错误不永远等待，也不伪造成功', async () => {
  for (const late of [false, true]) {
    const h = monitor(); await h.begin(); h.emit('started');
    if (late) h.finish(true, true);
    h.emit('exit');
    if (!late) h.finish(true, true);
    assert.equal(h.state.launching, false);
    assert.equal(h.state.collapsed, false);
    assert.equal(h.state.failure.event, 'launch_status_unavailable');
  }
});

test('全局快捷键在没有历史或关闭后展开控制台，不创建程序', async () => {
  const h = monitor(); await h.connect(); h.shortcut();
  assert.equal(h.state.visible, true); assert.equal(h.state.collapsed, false);
  assert.equal(h.state.sessions.length, 0); assert.equal(h.calls.length, 0);
  assert.equal(h.dismiss(), true);
  h.shortcut(); assert.equal(h.state.visible, true);
  assert.equal(h.state.launching, false);
});

test('无法监听时队列仍可提交，但不把无法确认的注入当成功收起', async () => {
  const h = monitor(true); await h.begin(); h.finish(true, true);
  assert.equal(h.state.launching, false);
  assert.equal(h.state.collapsed, false);
  assert.equal(h.state.currentEvent.event, 'launch_status_unavailable');
});

test('注入已确认完成后控制台断线仅保留来源错误，不反向宣称启动失败', async () => {
  const h = monitor(); await h.begin(); h.emit('started');
  h.handleEvent({ event: 'launch_complete', consoleSessionId: 'a' });
  h.finish(true, true);
  h.emit('error', 'socket disconnected');
  assert.equal(h.state.failure, null);
  assert.equal(h.state.currentEvent.event, 'launch_complete');
  assert.equal(h.state.collapsed, true);
  assert.equal(h.state.sessions[0].error, 'socket disconnected');
});
