import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { GameConfig } from '../../store/GameConfig';
import { isRemoteVersionNewer } from '../../common/UpdateCheckUtils';
import { supportsGameMaintenance } from './entry';
import type { GameMaintenanceStatus, GameMaintenanceTask } from './types';

export function useHomeMaintenance(gameName: () => string, config: () => GameConfig, onCompleted: () => void) {
  const status = ref<GameMaintenanceStatus | null>(null);
  const task = ref<GameMaintenanceTask | null>(null);
  const error = ref('');
  const starting = ref(false);
  const ownTask = computed(() => task.value?.gameName === gameName() ? task.value : null);
  const running = computed(() => ownTask.value?.state === 'running');
  const busy = computed(() => starting.value || (running.value && ownTask.value?.operation !== 'preload'));
  const needsUpdate = computed(() => !!status.value?.clientFound && !!status.value.currentServer
    && !!status.value.installedVersion && isRemoteVersionNewer(status.value.installedVersion, status.value.latestVersion));
  const canPreload = computed(() => !!status.value?.clientFound && !!status.value.currentServer
    && !!status.value.preDownloadVersion && !status.value.recoveryPending && !needsUpdate.value);
  const progress = computed(() => ownTask.value?.bytesTotal
    ? Math.min(100, Math.floor(ownTask.value.bytesDone / ownTask.value.bytesTotal * 100)) : 0);
  let revision = 0;
  let disposed = false;
  let polling = false;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let completedId = '';
  let taskRevision = 0;

  async function poll() {
    if (disposed || polling) return;
    if (pollTimer) clearTimeout(pollTimer);
    polling = true;
    const id = taskRevision;
    try {
      const result = await invoke<GameMaintenanceTask | null>('hoyoplay_task');
      if (disposed || id !== taskRevision) return;
      task.value = result;
      if (result && result.id !== completedId && result.gameName === gameName() && result.state !== 'running') {
        completedId = result.id;
        if (result.state === 'completed') { onCompleted(); void refresh(); }
      }
    } catch { /* IPC 暂时断开时保留状态；后台任务不依赖页面生命周期。 */ }
    finally {
      polling = false;
      if (!disposed && task.value?.state === 'running') pollTimer = setTimeout(() => void poll(), 1000);
    }
  }
  async function refresh() {
    const id = ++revision;
    const name = gameName();
    if (refreshTimer) clearTimeout(refreshTimer);
    if (!name || !supportsGameMaintenance(config().gamePreset)) { status.value = null; return; }
    try {
      const result = await invoke<GameMaintenanceStatus>('hoyoplay_status', { gameName: name, serverId: null, homeOnly: true });
      if (disposed || id !== revision || name !== gameName()) return;
      status.value = result;
    } catch (e) {
      // 检查失败不阻断普通启动，也不把未知版本伪装成必须更新。
      if (!disposed && id === revision) console.warn('Game version check failed:', e);
    } finally {
      if (!disposed && id === revision) refreshTimer = setTimeout(() => void refresh(), 180000);
    }
  }
  async function start(operation: 'preload' | 'update') {
    if (starting.value) return false;
    if (task.value?.state === 'running') {
      error.value = '已有游戏维护任务，请等待完成或暂停后再开始';
      return false;
    }
    const current = status.value;
    if (!current?.directory || !current.currentServer || current.recoveryPending) return false;
    if (operation === 'preload' && !canPreload.value) return false;
    starting.value = true; error.value = '';
    try {
      const started = await invoke<GameMaintenanceTask>('hoyoplay_start', { request: {
        gameName: gameName(), serverId: current.currentServer, operation, directory: current.directory,
        audioLanguages: current.audioLanguages,
      } });
      // 启动前发出的进度查询可能稍后返回空任务，不能覆盖刚建立的任务并停止轮询。
      ++taskRevision;
      task.value = started;
      if (pollTimer) clearTimeout(pollTimer);
      void poll();
      return true;
    } catch (e) { error.value = String(e); return false; }
    finally { starting.value = false; }
  }
  async function pause() {
    try { await invoke('hoyoplay_cancel'); }
    catch (e) { error.value = String(e); }
  }
  watch(() => [gameName(), config().gamePreset, config().targetExePath], () => {
    status.value = null; error.value = '';
    void refresh();
    if (pollTimer) clearTimeout(pollTimer);
    void poll();
  }, { immediate: true });
  const focused = () => { void refresh(); void poll(); };
  window.addEventListener('focus', focused);
  onBeforeUnmount(() => {
    disposed = true; ++revision;
    if (pollTimer) clearTimeout(pollTimer);
    if (refreshTimer) clearTimeout(refreshTimer);
    window.removeEventListener('focus', focused);
  });
  return { status, ownTask, running, busy, needsUpdate, canPreload, progress, error, start, pause, refresh, poll };
}
