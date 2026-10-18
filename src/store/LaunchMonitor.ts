import { reactive } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { subscribeLaunchEvents, type LaunchEvent } from '../plugin/launchEventBus';

export interface ConsoleEvent {
  sessionId: string; program: string; kind: string; text: string; pid?: number; code?: number;
}
export interface ConsoleSession {
  id: string; program: string; pid?: number; running: boolean; output: string; code?: number;
  error: string; generation: number; truncated: boolean; logPath?: string; readOnly: boolean;
}

const OUTPUT_LIMIT = 128 * 1024;
export function createLaunchMonitor() {
  const state = reactive({
    launching: false, visible: false, collapsed: true, selectedId: '', generation: 0,
    currentEvent: null as LaunchEvent | null, events: [] as LaunchEvent[], sessions: [] as ConsoleSession[],
  });
  let connection: Promise<void> | undefined;
  let unlisten: Array<() => void> = [];
  const handleEvent = (event: LaunchEvent) => {
    const source = event.consoleSessionId;
    if (source && !state.sessions.some(session => session.id === source && session.generation === state.generation)) return;
    state.currentEvent = event;
    state.events.push(event);
    if (state.events.length > 80) state.events.splice(0, state.events.length - 80);
  };
  const handleConsole = (event: ConsoleEvent) => {
    let session = state.sessions.find(item => item.id === event.sessionId);
    if ((event.kind === 'started' || event.kind === 'log_started') && !session) {
      const generation = event.kind === 'log_started' && event.text ? state.sessions.find(item => item.id === event.text)?.generation : state.generation;
      if (generation === undefined) return;
      // 运行中的会话不能因开始下一次启动被丢弃；仅清理最早的已退出记录。
      while (state.sessions.length >= 24) {
        const index = state.sessions.findIndex(item => !item.running);
        if (index < 0) break;
        state.sessions.splice(index, 1);
      }
      session = reactive({ id: event.sessionId, program: event.program, pid: event.pid,
        running: true, output: '', error: '', generation, truncated: false,
        readOnly: event.kind === 'log_started' });
      state.sessions.push(session);
      // 迟到的日志不能抢走所选会话，也不能展开已隐藏的窗口。
      if (!state.selectedId || (generation === state.generation && state.launching && !state.sessions.some(item => item.id === state.selectedId && item.generation === state.generation))) state.selectedId = session.id;
    }
    if (!session) return;
    if (event.kind === 'stdout' || event.kind === 'stderr') {
      // 按纯文本渲染，剔除 ANSI 控制序列，不把工具输出作为 HTML。
      const text = event.text.replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, '')
        .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').replace(/\r\n/g, '\n').replace(/\r/g, '\n')
        .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '');
      session.output += text;
      if (session.output.length > OUTPUT_LIMIT) {
        session.output = session.output.slice(-OUTPUT_LIMIT);
        session.truncated = true;
      }
    } else if (event.kind === 'log_path') {
      session.logPath = event.text;
    } else if (event.kind === 'truncated') {
      session.truncated = true;
    } else if (event.kind === 'exit' || event.kind === 'error') {
      session.running = false;
      session.code = event.code;
      if (event.kind === 'error') session.error = event.text;
    }
  };
  const connect = () => connection ??= Promise.allSettled([
    subscribeLaunchEvents(handleEvent),
    listen<ConsoleEvent>('launch-console-event', ({ payload }) => handleConsole(payload)),
  ]).then(results => {
    const listeners = results.flatMap(result => result.status === 'fulfilled' ? [result.value] : []);
    const failure = results.find(result => result.status === 'rejected');
    if (failure?.status === 'rejected') {
      listeners.forEach(stop => stop());
      throw failure.reason;
    }
    unlisten = listeners;
  }).catch(error => { connection = undefined; throw error; });
  const begin = async () => {
    state.launching = true;
    try { await connect(); }
    catch (error) { console.warn('[LaunchMonitor] 无法监听启动输出，继续启动', error); }
    state.generation++;
    state.currentEvent = null;
    state.events = [];
    state.visible = true;
    state.collapsed = true;
  };
  const finish = () => {
    state.launching = false;
    if (!state.sessions.some(session => session.generation === state.generation)) state.visible = false;
  };
  return { state, connect, begin, finish, handleEvent, handleConsole,
    async send(text: string) {
      const session = state.sessions.find(item => item.id === state.selectedId);
      if (!session?.running) throw new Error('控制台程序已退出');
      if (session.readOnly) throw new Error('日志来源不接受输入');
      await invoke('write_launch_console', { sessionId: session.id, text });
    },
    dispose() { unlisten.forEach(stop => stop()); unlisten = []; connection = undefined; },
  };
}

export const LaunchMonitor = createLaunchMonitor();
