<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { ArrowDown, ArrowUp, Close, CopyDocument, Document, Monitor, Promotion, RefreshRight } from '@element-plus/icons-vue';
import { LaunchMonitor } from '../store/LaunchMonitor';
import { openPath } from '@tauri-apps/plugin-opener';

const { t, te } = useI18n();
const state = LaunchMonitor.state;
const tab = ref<'output' | 'events'>('output');
const input = ref('');
const sending = ref(false);
const inputError = ref('');
const log = ref<HTMLElement>();
const followOutput = ref(true);
const selected = computed(() => state.sessions.find(item => item.id === state.selectedId));
const expanded = computed(() => state.visible && !state.collapsed);
const hasActiveLoad = computed(() => LaunchMonitor.hasActiveLoad());
const runningSources = computed(() => state.sessions.filter(session => session.running).length);
const activityLabel = computed(() => runningSources.value
  ? t('launchUi.console.activeSources', { count: runningSources.value })
  : t(state.launching ? 'launchUi.console.starting' : 'launchUi.console.idle'));
const eventLabel = (name: string) => te(`launchUi.events.${name}`) ? t(`launchUi.events.${name}`) : name;
const error = computed(() => state.failure);
const stageLabel = computed(() => {
  if (error.value) return error.value.event === 'launch_error' ? t('launchUi.failed') : eventLabel(error.value.event);
  if (state.launching && state.currentEvent?.event === 'launch_complete') return eventLabel('finishing_launch');
  return state.currentEvent?.event ? eventLabel(state.currentEvent.event)
    : t(state.launching ? 'launchUi.events.launch_started' : 'launchUi.console.history');
});
const busy = computed(() => state.launching && !error.value);
const scrollToEnd = async () => { await nextTick(); if (log.value) log.value.scrollTop = log.value.scrollHeight; };
watch(() => selected.value?.output, () => { if (expanded.value && followOutput.value) void scrollToEnd(); });
watch(() => state.selectedId, () => { inputError.value = ''; followOutput.value = true; void scrollToEnd(); });
watch([expanded, tab], () => { if (followOutput.value) void scrollToEnd(); });
const onScroll = () => { if (log.value) followOutput.value = log.value.scrollHeight - log.value.scrollTop - log.value.clientHeight < 36; };
const send = async () => {
  if (sending.value || !selected.value?.running || selected.value.readOnly) return;
  sending.value = true; inputError.value = '';
  try { await LaunchMonitor.send(input.value); input.value = ''; }
  catch (error) { inputError.value = String(error); }
  finally { sending.value = false; }
};
const copy = async () => {
  try { await navigator.clipboard.writeText(tab.value === 'output' ? selected.value?.output || '' : state.events.map(item => JSON.stringify(item)).join('\n')); }
  catch (error) { inputError.value = String(error); }
};
const openLog = async () => {
  if (!selected.value?.logPath) return;
  try { await openPath(selected.value.logPath); }
  catch (error) { inputError.value = String(error); }
};
</script>

<template>
  <aside v-if="state.visible" class="launch-monitor" :class="{ expanded }">
    <section class="monitor-panel" :aria-label="t('launchUi.console.title')">
      <header class="monitor-header">
        <button class="monitor-summary" type="button" :aria-expanded="expanded" :title="expanded ? t('launchUi.hideDetails') : t('launchUi.showDetails')"
          @click="state.collapsed = !state.collapsed">
          <span class="monitor-emblem" aria-hidden="true"><Monitor class="icon" /></span>
          <span class="monitor-heading">
            <span v-if="expanded" class="monitor-caption">{{ t('launchUi.console.title') }}</span>
            <span class="monitor-stage" role="status"><span class="monitor-status" :class="{ busy, failed: error }" aria-hidden="true" /><span class="stage-text">{{ stageLabel }}</span></span>
          </span>
          <ArrowDown v-if="expanded" class="icon summary-chevron" /><ArrowUp v-else class="icon summary-chevron" />
        </button>
        <span v-if="expanded" class="activity-badge" :class="{ active: hasActiveLoad }">{{ activityLabel }}</span>
        <span class="close-action" :title="t(hasActiveLoad ? 'launchUi.console.closeBlocked' : 'launchUi.console.close')">
          <button class="icon-button close-button" type="button" :disabled="hasActiveLoad" :aria-label="t('launchUi.console.close')" @click="LaunchMonitor.dismiss()"><Close class="icon" /></button>
        </span>
      </header>
      <template v-if="expanded">
        <nav class="monitor-tabs" :aria-label="t('launchUi.console.title')">
          <button type="button" :class="{ active: tab === 'output' }" :aria-pressed="tab === 'output'" @click="tab = 'output'">{{ t('launchUi.console.output') }}<span>{{ state.sessions.length }}</span></button>
          <button type="button" :class="{ active: tab === 'events' }" :aria-pressed="tab === 'events'" @click="tab = 'events'">{{ t('launchUi.console.events') }}</button>
          <button class="icon-button copy-button" type="button" :title="t('launchUi.console.copy')" :aria-label="t('launchUi.console.copy')" @click="copy"><CopyDocument class="icon" /></button>
        </nav>
        <div v-if="error" class="monitor-error"><template v-if="error.code">{{ error.code }} · </template>{{ error.message || eventLabel(error.event) }}</div>
        <template v-if="tab === 'output'">
          <div v-if="state.sessions.length" class="monitor-toolbar">
            <select v-model="state.selectedId" :aria-label="t('launchUi.console.program')">
              <option v-for="session in state.sessions" :key="session.id" :value="session.id">{{ session.program }}{{ session.pid ? ` · PID ${session.pid}` : '' }} · {{ session.running ? t(session.readOnly ? 'launchUi.console.following' : 'launchUi.console.running') : t('launchUi.console.stopped') }}</option>
            </select>
            <span class="source-kind" :class="{ live: selected?.running }">{{ t(selected?.readOnly ? 'launchUi.console.readOnly' : 'launchUi.console.interactive') }}</span>
            <button v-if="selected?.logPath" class="icon-button" type="button" :title="t('launchUi.console.log')" :aria-label="t('launchUi.console.log')" @click="openLog"><Document class="icon" /></button>
          </div>
          <div v-if="selected" ref="log" class="monitor-log glass-scrollbar" tabindex="0" :aria-label="t('launchUi.console.output')" @scroll="onScroll"><span v-if="selected.truncated" class="monitor-muted">{{ t('launchUi.console.truncated') }}&#10;</span><span>{{ selected.output || t('launchUi.waiting') }}</span></div>
          <div v-else class="monitor-empty">{{ t(state.launching ? 'launchUi.waiting' : 'launchUi.console.emptyHistory') }}</div>
          <div v-if="selected?.error || inputError" class="monitor-error">{{ inputError || selected?.error }}</div>
          <form v-if="selected && !selected.readOnly && selected.running" class="monitor-input" @submit.prevent="send">
            <span aria-hidden="true">&gt;</span>
            <input v-model="input" :disabled="sending" :placeholder="t('launchUi.console.input')" :aria-label="t('launchUi.console.input')" autocomplete="off" spellcheck="false" maxlength="4000" />
            <button class="icon-button" type="submit" :disabled="sending" :title="t('launchUi.console.send')" :aria-label="t('launchUi.console.send')"><Promotion class="icon" /></button>
          </form>
          <footer>
            <span>{{ selected?.readOnly && !selected.running ? t('launchUi.console.stopped') : selected?.readOnly ? t('launchUi.console.logHint') : selected && !selected.running ? t('launchUi.console.exited', { code: selected.code ?? '?' }) : t('launchUi.console.hideHint') }}</span>
            <button v-if="selected && !followOutput" type="button" @click="followOutput = true; scrollToEnd()"><RefreshRight class="icon" />{{ t('launchUi.console.following') }}</button>
            <button v-else-if="selected" type="button" @click="selected.output = ''; selected.truncated = false">{{ t('launchUi.console.clear') }}</button>
          </footer>
        </template>
        <ol v-else class="monitor-events glass-scrollbar">
          <li v-for="(event, index) in state.events" :key="index" :class="{ failed: event.event === 'launch_error' }">
            <span class="event-number">{{ String(index + 1).padStart(2, '0') }}</span>
            <div><strong>{{ eventLabel(event.event) }}</strong><p v-if="event.module || event.message || event.code">{{ event.message || event.module || event.code }}</p><small v-if="event.pid">PID {{ event.pid }}</small></div>
          </li>
          <li v-if="!state.events.length" class="monitor-muted">{{ t('launchUi.waiting') }}</li>
        </ol>
      </template>
    </section>
  </aside>
</template>

<style scoped>
.launch-monitor { position: absolute; right: 20px; bottom: 20px; z-index: 100; width: 340px; max-width: calc(100% - 40px); color: var(--theme-text, #eee); pointer-events: none; font-size: 12px; }
.launch-monitor.expanded { top: 60px; width: 660px; display: flex; align-items: flex-end; }
.monitor-panel { width: 100%; min-height: 0; max-height: 100%; display: flex; flex-direction: column; overflow: hidden; pointer-events: auto; border: var(--t-material-border); border-radius: 18px; background: var(--t-surface-solid, #171822); box-shadow: var(--t-material-shadow); }
.launch-monitor:not(.expanded) .monitor-panel { border-radius: 999px; opacity: .58; transition: opacity .18s ease; }
.launch-monitor:not(.expanded):hover .monitor-panel, .launch-monitor:not(.expanded):focus-within .monitor-panel { opacity: 1; }
.monitor-header { display: flex; align-items: center; gap: 8px; padding: 6px 10px 6px 6px; flex-shrink: 0; }
.expanded .monitor-header { padding: 12px 14px 12px 8px; }
button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; border: 0; border-radius: 8px; padding: 7px 10px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
button:hover:not(:disabled) { background: rgba(128,128,128,.13); }
button:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid var(--theme-accent); outline-offset: -2px; }
button:disabled { opacity: .3; cursor: not-allowed; }
.icon { width: 15px; height: 15px; flex-shrink: 0; }
.icon-button { padding: 7px; }
.monitor-summary { min-width: 0; flex: 1; justify-content: flex-start; gap: 10px; padding: 4px 8px; border-radius: 999px; }
.expanded .monitor-summary { border-radius: 10px; }
.monitor-emblem { display: flex; align-items: center; justify-content: center; width: 30px; height: 30px; flex-shrink: 0; border: 1px solid rgba(128,128,128,.16); border-radius: 10px; background: var(--t-surface-soft, rgba(128,128,128,.06)); }
.expanded .monitor-emblem { width: 36px; height: 36px; border-radius: 12px; }
.monitor-heading { display: flex; flex-direction: column; gap: 5px; flex: 1; min-width: 0; text-align: left; }
.monitor-caption { font-size: 13px; font-weight: 600; letter-spacing: .02em; }
.monitor-stage { display: flex; align-items: center; gap: 8px; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.stage-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.expanded .monitor-stage { font-size: 11px; opacity: .7; }
.summary-chevron { opacity: .5; }
.activity-badge { padding: 5px 9px; border-radius: 999px; background: var(--t-surface-soft, rgba(128,128,128,.08)); font-size: 10px; white-space: nowrap; opacity: .65; }
.activity-badge.active { opacity: 1; color: var(--theme-accent); }
.close-action { display: inline-flex; flex-shrink: 0; padding-left: 6px; border-left: 1px solid rgba(128,128,128,.15); }
.close-button:hover:not(:disabled) { color: #f5b29a; background: rgba(230,140,80,.12); }
.monitor-status { width: 7px; height: 7px; border-radius: 50%; background: var(--theme-accent, #44bfb3); flex-shrink: 0; }
.monitor-status.busy { width: 10px; height: 10px; box-sizing: border-box; border: 2px solid rgba(128,128,128,.25); border-top-color: var(--theme-accent); background: transparent; animation: spin 1s linear infinite; }
.monitor-status.failed { background: #e99865; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .monitor-status.busy { animation: none; } .launch-monitor:not(.expanded) .monitor-panel { transition: none; } }
@media (hover: none) { .launch-monitor:not(.expanded) .monitor-panel { opacity: 1; } }
.monitor-tabs { display: flex; align-items: center; gap: 6px; padding: 0 14px 10px; border-bottom: 1px solid rgba(128,128,128,.15); flex-shrink: 0; }
.monitor-tabs button.active { color: var(--theme-accent); background: rgba(var(--theme-surface-tint-rgb), .14); }
.monitor-tabs span { font-size: 10px; opacity: .6; }
.copy-button { margin-left: auto; }
.monitor-toolbar { display: flex; align-items: center; gap: 10px; padding: 12px 14px; flex-shrink: 0; }
select { min-width: 0; flex: 1; padding: 7px 8px; background: var(--t-surface-solid, #171822); color: inherit; border: 1px solid rgba(128,128,128,.25); border-radius: 8px; font: inherit; }
.source-kind { padding: 5px 8px; border: 1px solid rgba(128,128,128,.15); border-radius: 6px; font-size: 10px; opacity: .6; white-space: nowrap; }
.source-kind.live { opacity: .9; }
.monitor-log { height: 300px; min-height: 65px; flex: 1 1 300px; overflow: auto; margin: 0 14px; padding: 12px 14px; border: 1px solid rgba(128,128,128,.12); border-radius: 10px; background: rgba(0,0,0,.23); font: 12px/1.75 Consolas, ui-monospace, monospace; white-space: pre-wrap; overflow-wrap: anywhere; tab-size: 4; }
.monitor-muted { opacity: .5; }
.monitor-empty { padding: 40px 16px; text-align: center; opacity: .6; }
.monitor-input { display: flex; align-items: center; gap: 10px; margin: 8px 14px 0; border: 1px solid rgba(128,128,128,.2); border-radius: 9px; padding: 2px 6px 2px 10px; flex-shrink: 0; }
.monitor-input > span { color: var(--theme-accent); }
.monitor-input input { min-width: 0; flex: 1; padding: 7px 0; background: transparent; border: 0; color: inherit; font: 12px Consolas, monospace; }
footer { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 8px 14px; font-size: 10px; flex-shrink: 0; }
footer > span { min-width: 0; opacity: .6; }
footer > button { flex-shrink: 0; }
.monitor-events { margin: 0; padding: 4px 18px 14px; list-style: none; min-height: 0; height: 350px; overflow: auto; }
.monitor-events li { display: flex; gap: 12px; padding: 10px 0; border-bottom: 1px solid rgba(128,128,128,.12); }
.event-number { padding-top: 2px; font: 10px Consolas, monospace; opacity: .4; }
.monitor-events strong { font-size: 12px; font-weight: 500; }
.monitor-events p { margin: 5px 0 0; opacity: .65; font: 11px/1.6 Consolas, monospace; overflow-wrap: anywhere; }
.monitor-events small { display: block; margin-top: 4px; opacity: .45; }
.monitor-error, .monitor-events li.failed { color: #e99865; }
.monitor-error { flex-shrink: 0; padding: 8px 16px; background: rgba(230,140,80,.08); overflow-wrap: anywhere; }
@media (max-width: 480px) {
  .launch-monitor { right: 12px; bottom: 12px; max-width: calc(100% - 24px); }
  .activity-badge { display: none; }
  .monitor-toolbar { gap: 6px; }
  .source-kind { padding: 5px; }
}
</style>
