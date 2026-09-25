<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { subscribeLaunchEvents, type LaunchEvent } from '../plugin/launchEventBus'

const props = defineProps<{ visible: boolean }>()
const { t } = useI18n()
const detailsOpen = ref(false)
const currentEvent = ref<LaunchEvent | null>(null)
const events = ref<LaunchEvent[]>([])
let unlisten: UnlistenFn | null = null

const progressByEvent: Record<string, number> = {
  launch_started: 8,
  target_created: 28,
  inject_begin: 42,
  inject_complete: 58,
  plugin_host_begin: 68,
  plugin_host_ready: 78,
  plugin_host_disabled: 78,
  before_resume: 88,
  target_resumed: 94,
  runtime_detected: 98,
  launch_complete: 100,
}

const progress = computed(() => currentEvent.value ? progressByEvent[currentEvent.value.event] ?? 12 : 0)
const stageLabel = computed(() => currentEvent.value?.event
  ? t(`launchUi.events.${currentEvent.value.event}`, currentEvent.value.event)
  : t('launchUi.events.launch_started'))
const error = computed(() => currentEvent.value?.event === 'launch_error' ? currentEvent.value : null)

const reset = () => {
  currentEvent.value = null
  events.value = []
  detailsOpen.value = false
}

const handleEvent = (event: LaunchEvent) => {
  currentEvent.value = event
  events.value = [...events.value.slice(-39), event]
}

watch(() => props.visible, (visible) => {
  if (visible) reset()
})

onMounted(async () => {
  unlisten = await subscribeLaunchEvents(handleEvent)
})

onUnmounted(() => {
  unlisten?.()
})
</script>

<template>
  <Transition name="launch-ui-fade">
    <section v-if="visible" class="launch-ui-overlay" role="status" aria-live="polite">
      <div class="launch-ui-panel">
        <div class="launch-ui-kicker">SSMT LAUNCH</div>
        <h2>{{ t('launchUi.title') }}</h2>
        <p class="launch-ui-stage">{{ error ? t('launchUi.failed') : stageLabel }}</p>
        <div class="launch-ui-progress" aria-hidden="true"><span :style="{ width: `${progress}%` }"></span></div>
        <div class="launch-ui-progress-meta"><span>{{ progress }}%</span><span v-if="currentEvent?.pid">PID {{ currentEvent.pid }}</span></div>
        <div v-if="error" class="launch-ui-error">
          <strong>{{ error.code || t('launchUi.failed') }}</strong>
          <span>{{ error.message || t('launchUi.unknownError') }}</span>
        </div>
        <button type="button" class="launch-ui-details" @click="detailsOpen = !detailsOpen">
          {{ detailsOpen ? t('launchUi.hideDetails') : t('launchUi.showDetails') }}
        </button>
        <pre v-if="detailsOpen" class="launch-ui-log">{{ events.map(item => JSON.stringify(item)).join('\n') || t('launchUi.waiting') }}</pre>
      </div>
    </section>
  </Transition>
</template>

<style scoped>
.launch-ui-overlay { position: absolute; inset: 0; z-index: 12; display: grid; place-items: center; padding: 24px; background: rgba(3, 6, 12, .48); backdrop-filter: blur(8px); }
.launch-ui-panel { width: min(440px, calc(100vw - 48px)); padding: 28px; border: 1px solid rgba(255,255,255,.16); border-radius: 14px; background: rgba(10, 16, 23, .88); box-shadow: 0 24px 70px rgba(0,0,0,.35); }
.launch-ui-kicker { color: #75d6bb; font-size: 10px; font-weight: 800; letter-spacing: .16em; }
.launch-ui-panel h2 { margin: 8px 0 10px; font-size: 23px; }
.launch-ui-stage { min-height: 20px; margin: 0 0 18px; color: rgba(235,242,240,.68); font-size: 13px; }
.launch-ui-progress { height: 6px; overflow: hidden; border-radius: 5px; background: rgba(255,255,255,.1); }
.launch-ui-progress span { display: block; height: 100%; border-radius: inherit; background: #75d6bb; transition: width .28s ease; }
.launch-ui-progress-meta { display: flex; justify-content: space-between; margin-top: 7px; color: rgba(235,242,240,.45); font-size: 11px; }
.launch-ui-error { display: grid; gap: 5px; margin-top: 16px; padding: 10px 12px; border-left: 3px solid #ff8f82; background: rgba(255,120,105,.1); color: #ffb4a9; font-size: 12px; line-height: 1.45; }
.launch-ui-error strong { color: #ff9b8e; }
.launch-ui-details { margin-top: 18px; padding: 0; border: 0; background: transparent; color: rgba(235,242,240,.62); font: inherit; font-size: 12px; cursor: pointer; }
.launch-ui-details:hover { color: #fff; }
.launch-ui-log { max-height: 170px; overflow: auto; margin: 12px 0 0; padding: 10px; border-radius: 7px; background: rgba(0,0,0,.3); color: rgba(235,242,240,.58); font: 10px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; white-space: pre-wrap; overflow-wrap: anywhere; }
.launch-ui-fade-enter-active, .launch-ui-fade-leave-active { transition: opacity .18s ease; }
.launch-ui-fade-enter-from, .launch-ui-fade-leave-to { opacity: 0; }
</style>
