<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { LaunchMonitor } from '../store/LaunchMonitor';
import { openPath } from '@tauri-apps/plugin-opener';

const { t } = useI18n();
const state = LaunchMonitor.state;
const detailsOpen = ref(false);
const input = ref('');
const sending = ref(false);
const inputError = ref('');
const log = ref<HTMLElement>();
const followOutput = ref(true);
const selected = computed(() => state.sessions.find(item => item.id === state.selectedId));
const activeCount = computed(() => state.sessions.filter(item => item.running).length);
const stageLabel = computed(() => state.currentEvent?.event === 'launch_error' ? t('launchUi.failed') : state.currentEvent?.event
  ? t(`launchUi.events.${state.currentEvent.event}`, state.currentEvent.event)
  : state.launching ? t('launchUi.events.launch_started') : t('launchUi.console.title'));
const error = computed(() => state.currentEvent?.event === 'launch_error' ? state.currentEvent : null);
const scrollToEnd = async () => {
  await nextTick();
  if (log.value) log.value.scrollTop = log.value.scrollHeight;
};
watch(() => selected.value?.output, () => { if (followOutput.value) void scrollToEnd(); });
watch(() => state.selectedId, () => { inputError.value = ''; followOutput.value = true; void scrollToEnd(); });
watch(() => state.collapsed, collapsed => { if (!collapsed && followOutput.value) void scrollToEnd(); });
const onScroll = () => {
  if (log.value) followOutput.value = log.value.scrollHeight - log.value.scrollTop - log.value.clientHeight < 36;
};
const send = async () => {
  if (sending.value || !selected.value?.running) return;
  sending.value = true;
  inputError.value = '';
  try { await LaunchMonitor.send(input.value); input.value = ''; }
  catch (error) { inputError.value = String(error); }
  finally { sending.value = false; }
};
const copy = async () => {
  try { await navigator.clipboard.writeText(selected.value?.output || ''); }
  catch (error) { inputError.value = String(error); }
};
const openLog = async () => {
  if (!selected.value?.logPath) return;
  try { await openPath(selected.value.logPath); }
  catch (error) { inputError.value = String(error); }
};
</script>

<template>
  <aside v-if="state.visible || state.sessions.length" class="launch-monitor" :class="{ compact: !state.visible || state.collapsed }">
    <button v-if="!state.visible || state.collapsed" type="button" class="monitor-reopen" @click="state.visible = true; state.collapsed = false">
      <span aria-hidden="true">&gt;_</span> {{ t('launchUi.console.title') }}
      <span v-if="activeCount" class="monitor-count">{{ activeCount }}</span>
    </button>
    <section v-else class="monitor-panel" :aria-label="t('launchUi.console.title')">
      <header class="monitor-header">
        <div><strong>{{ t('launchUi.console.title') }}</strong><span class="monitor-stage" role="status">{{ stageLabel }}</span></div>
        <div class="monitor-actions">
          <button type="button" :title="t('launchUi.console.minimize')" :aria-label="t('launchUi.console.minimize')" @click="state.collapsed = true">−</button>
          <button type="button" :title="t('launchUi.console.hide')" :aria-label="t('launchUi.console.hide')" @click="state.visible = false">×</button>
        </div>
      </header>
      <div v-if="error" class="monitor-error">{{ error.code }} · {{ error.message || t('launchUi.unknownError') }}</div>
      <div v-if="state.sessions.length" class="monitor-toolbar">
        <select v-model="state.selectedId" :aria-label="t('launchUi.console.program')">
          <option v-for="session in state.sessions" :key="session.id" :value="session.id">
            {{ session.program }} · {{ session.pid }} · {{ session.running ? t('launchUi.console.running') : t('launchUi.console.exited', { code: session.code ?? '?' }) }}
          </option>
        </select>
        <button type="button" @click="copy">{{ t('launchUi.console.copy') }}</button>
        <button v-if="selected?.logPath" type="button" @click="openLog">{{ t('launchUi.console.log') }}</button>
        <button type="button" @click="selected && (selected.output = '', selected.truncated = false)">{{ t('launchUi.console.clear') }}</button>
      </div>
      <div v-if="selected" ref="log" class="monitor-log glass-scrollbar" tabindex="0" @scroll="onScroll">
        <span v-if="selected.truncated" class="monitor-muted">{{ t('launchUi.console.truncated') }}&#10;</span>
        <span>{{ selected.output || t('launchUi.waiting') }}</span>
      </div>
      <p v-else class="monitor-waiting">{{ t('launchUi.waiting') }}</p>
      <p v-if="selected?.error || inputError" class="monitor-error">{{ inputError || selected?.error }}</p>
      <form v-if="selected" class="monitor-input" @submit.prevent="send">
        <span aria-hidden="true">&gt;</span>
        <input v-model="input" :disabled="!selected.running || sending" :placeholder="t('launchUi.console.input')" :aria-label="t('launchUi.console.input')" autocomplete="off" spellcheck="false" maxlength="4000" />
        <button type="submit" :disabled="!selected.running || sending">{{ t('launchUi.console.send') }}</button>
      </form>
      <footer>
        <button type="button" @click="detailsOpen = !detailsOpen">{{ detailsOpen ? t('launchUi.hideDetails') : t('launchUi.showDetails') }}</button>
        <span>{{ t('launchUi.console.hideHint') }}</span>
      </footer>
      <pre v-if="detailsOpen" class="monitor-events glass-scrollbar">{{ state.events.map(item => JSON.stringify(item)).join('\n') || t('launchUi.waiting') }}</pre>
    </section>
  </aside>
</template>

<style scoped>
.launch-monitor { position: absolute; top: 52px; right: 20px; bottom: 20px; z-index: 100; display: flex; align-items: flex-end; width: min(680px, calc(100% - 40px)); pointer-events: none; color: var(--theme-text, #eee); }
.launch-monitor.compact { top: auto; width: auto; }
.monitor-panel { display: flex; flex-direction: column; width: 100%; max-height: 100%; overflow: hidden; pointer-events: auto; border: var(--t-material-border); border-radius: 14px; background: var(--t-surface-solid, #111827); box-shadow: var(--t-material-shadow); }
.monitor-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 16px; }
.monitor-header > div:first-child { min-width: 0; display: grid; gap: 4px; }
.monitor-header strong { font-size: 14px; }
.monitor-stage { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; opacity: .65; }
.monitor-actions { display: flex; }
button { cursor: pointer; border: 1px solid rgba(255,255,255,.13); border-radius: 7px; padding: 5px 10px; background: rgba(255,255,255,.05); color: inherit; font: inherit; font-size: 12px; }
button:hover { background: rgba(255,255,255,.12); }
button:disabled { cursor: default; opacity: .4; }
.monitor-actions button { background: transparent; border: 0; font-size: 20px; }
.monitor-toolbar { display: flex; gap: 8px; padding: 0 14px 10px; }
.monitor-toolbar select { min-width: 0; flex: 1; padding: 5px 8px; background: var(--t-surface-solid, #111827); color: inherit; border: 1px solid rgba(255,255,255,.15); border-radius: 7px; }
.monitor-log { height: 260px; min-height: 80px; overflow: auto; padding: 12px 16px; background: rgba(0,0,0,.3); font: 12px/1.6 Consolas, ui-monospace, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.monitor-muted { opacity: .5; }
.monitor-waiting { padding: 0 16px; opacity: .6; font-size: 12px; }
.monitor-input { display: flex; align-items: center; gap: 8px; padding: 10px 14px; }
.monitor-input > span { color: var(--theme-accent); }
.monitor-input input { min-width: 0; flex: 1; padding: 6px; background: transparent; border: 0; color: inherit; outline: none; font: 12px Consolas, monospace; }
.monitor-input:focus-within { box-shadow: inset 0 -1px var(--theme-accent); }
footer { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 0 14px 12px; font-size: 10px; }
footer > span { opacity: .5; }
footer button { border: 0; padding-left: 0; background: transparent; }
.monitor-events { flex-shrink: 1; max-height: 120px; min-height: 0; overflow: auto; margin: 0; padding: 12px 16px; border-top: 1px solid rgba(255,255,255,.1); font: 10px/1.5 Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.monitor-error { flex-shrink: 0; margin: 0; padding: 8px 16px; color: #ffb4a9; background: rgba(255,120,105,.1); font-size: 12px; overflow-wrap: anywhere; }
.monitor-reopen { display: flex; align-items: center; gap: 10px; padding: 10px 14px; pointer-events: auto; background: var(--t-surface-solid, #111827); box-shadow: var(--t-material-shadow); border-radius: 24px; }
.monitor-count { padding: 1px 6px; border-radius: 10px; background: rgba(var(--theme-surface-tint-rgb), .2); color: var(--theme-accent); }
</style>
