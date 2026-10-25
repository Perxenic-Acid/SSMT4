import { onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { InstallSpaceEstimate, InstallSpaceRequest } from './types';

export function useInstallSpace(request: () => InstallSpaceRequest | null, automatic = true) {
  const estimate = ref<InstallSpaceEstimate | null>(null);
  const loading = ref(false);
  const error = ref('');
  let revision = 0;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function refresh() {
    if (timer) clearTimeout(timer);
    const selected = request();
    const id = ++revision;
    if (disposed || !selected) return null;
    loading.value = true; error.value = ''; estimate.value = null;
    try {
      const result = await invoke<InstallSpaceEstimate>('hoyoplay_estimate', { request: selected });
      if (disposed || id !== revision) return null;
      estimate.value = result;
      return result;
    } catch (e) {
      if (!disposed && id === revision) error.value = String(e);
      return null;
    } finally { if (!disposed && id === revision) loading.value = false; }
  }
  watch(() => JSON.stringify(request()), () => {
    // 同步失效，避免切换目录或语音之后短暂使用旧磁盘的空间结果。
    ++revision;
    if (timer) clearTimeout(timer);
    estimate.value = null; error.value = ''; loading.value = automatic && !!request();
    if (automatic && request()) timer = setTimeout(() => void refresh(), 300);
  }, { immediate: true, flush: 'sync' });
  onBeforeUnmount(() => { disposed = true; ++revision; if (timer) clearTimeout(timer); });
  return { estimate, loading, error, refresh };
}
