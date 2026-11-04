<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Ssmt4Mark from './Ssmt4Mark.vue'
import { STARTUP_VISIBLE_EVENT } from '../common/StartupExperience'

const props = defineProps<{
  enabled: boolean
}>()

const emit = defineEmits<{
  finished: []
}>()

type StartupPhase = 'waiting' | 'active' | 'leaving'

const phase = ref<StartupPhase>('waiting')
const progress = ref(0)
let animationFrame = 0
let finishTimer = 0
let leaveTimer = 0
let previewTimer = 0

const statusText = computed(() => {
  if (progress.value >= 100) return 'Workspace ready'
  if (progress.value >= 82) return 'Preparing render workspace'
  if (progress.value >= 48) return 'Mapping game profiles'
  return 'Loading local configuration'
})

const finish = () => {
  if (phase.value !== 'active') return
  phase.value = 'leaving'
  leaveTimer = window.setTimeout(() => emit('finished'), 620)
}

const start = () => {
  if (phase.value !== 'waiting') return
  if (!props.enabled) {
    emit('finished')
    return
  }
  phase.value = 'active'

  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const startedAt = performance.now()
  const duration = reducedMotion ? 420 : 1850

  const tick = (now: number) => {
    const next = Math.min(100, ((now - startedAt) / duration) * 100)
    progress.value = Math.round(next)
    if (next < 100) {
      animationFrame = window.requestAnimationFrame(tick)
      return
    }
    finishTimer = window.setTimeout(finish, reducedMotion ? 80 : 280)
  }

  animationFrame = window.requestAnimationFrame(tick)
}

onMounted(() => {
  window.addEventListener(STARTUP_VISIBLE_EVENT, start, { once: true })
  const tauriWindow = window as Window & { __TAURI_INTERNALS__?: unknown }
  if (!tauriWindow.__TAURI_INTERNALS__) {
    previewTimer = window.setTimeout(start, 120)
  }
})

onBeforeUnmount(() => {
  window.removeEventListener(STARTUP_VISIBLE_EVENT, start)
  window.cancelAnimationFrame(animationFrame)
  window.clearTimeout(finishTimer)
  window.clearTimeout(leaveTimer)
  window.clearTimeout(previewTimer)
})
</script>

<template>
  <section
    class="startup-experience"
    :class="`is-${phase}`"
    role="status"
    aria-live="polite"
    aria-label="Starting SSMT4"
  >
    <div class="startup-grid" aria-hidden="true"></div>

    <div class="startup-corner startup-corner-top" aria-hidden="true">
      <span>BOOT SEQUENCE</span>
      <strong>04 / READY</strong>
    </div>

    <div class="startup-stage">
      <div class="startup-emblem" aria-hidden="true">
        <span class="startup-orbit startup-orbit-outer"><i></i></span>
        <span class="startup-orbit startup-orbit-inner"></span>
        <span class="startup-mark"><Ssmt4Mark /></span>
        <span class="startup-scan"></span>
      </div>

      <div class="startup-copy">
        <p class="startup-kicker">MOD CREATION ENVIRONMENT</p>
        <h1>SSMT<span>4</span></h1>
        <p class="startup-subtitle">LOCAL WORKSPACE / SYSTEM ONLINE</p>
      </div>

      <div class="startup-progress">
        <div class="startup-progress-meta">
          <span>{{ statusText }}</span>
          <strong>{{ progress.toString().padStart(3, '0') }}%</strong>
        </div>
        <div class="startup-progress-track" aria-hidden="true">
          <i :style="{ transform: `scaleX(${progress / 100})` }"></i>
        </div>
        <div class="startup-progress-stages" aria-hidden="true">
          <span :class="{ active: progress >= 1 }">CORE</span>
          <span :class="{ active: progress >= 48 }">PROFILES</span>
          <span :class="{ active: progress >= 82 }">RENDER</span>
          <span :class="{ active: progress >= 100 }">READY</span>
        </div>
      </div>
    </div>

    <footer class="startup-footer">
      <span><i></i> LOCAL SESSION</span>
      <span>SSMT4 / 4.2.2</span>
    </footer>
  </section>
</template>

<style scoped>
.startup-experience {
  position: fixed;
  inset: 0;
  z-index: 2147483646;
  display: grid;
  place-items: center;
  overflow: hidden;
  background:
    linear-gradient(135deg, rgba(216, 233, 78, .045), transparent 34%),
    linear-gradient(180deg, #0b1113 0%, #06090b 52%, #080d0f 100%);
  color: #f2f6f3;
  opacity: 1;
  transform: scale(1);
  transition: opacity .58s cubic-bezier(.4, 0, .2, 1), transform .58s cubic-bezier(.4, 0, .2, 1);
}

.startup-experience.is-leaving {
  opacity: 0;
  transform: scale(1.018);
  pointer-events: none;
}

.startup-experience::before,
.startup-experience::after {
  content: '';
  position: absolute;
  pointer-events: none;
}

.startup-experience::before {
  inset: 18px;
  border: 1px solid rgba(255, 255, 255, .055);
  clip-path: polygon(0 0, 12% 0, 12% 1px, 88% 1px, 88% 0, 100% 0, 100% 100%, 88% 100%, 88% calc(100% - 1px), 12% calc(100% - 1px), 12% 100%, 0 100%);
}

.startup-experience::after {
  left: 50%;
  bottom: 72px;
  width: min(440px, 58vw);
  height: 1px;
  transform: translateX(-50%);
  background: linear-gradient(90deg, transparent, rgba(216, 233, 78, .34), transparent);
}

.startup-grid {
  position: absolute;
  inset: 0;
  opacity: .36;
  background-image:
    linear-gradient(rgba(132, 154, 160, .055) 1px, transparent 1px),
    linear-gradient(90deg, rgba(132, 154, 160, .055) 1px, transparent 1px);
  background-size: 46px 46px;
  mask-image: linear-gradient(180deg, transparent 0%, #000 24%, #000 76%, transparent 100%);
}

.startup-corner {
  position: absolute;
  top: 38px;
  right: 42px;
  display: grid;
  gap: 4px;
  color: rgba(190, 205, 207, .46);
  font: 10px/1.4 ui-monospace, Consolas, monospace;
  text-align: right;
}

.startup-corner strong {
  color: rgba(216, 233, 78, .78);
  font-weight: 600;
}

.startup-stage {
  position: relative;
  z-index: 1;
  display: grid;
  justify-items: center;
  width: min(680px, calc(100vw - 48px));
  transform: translateY(12px);
  opacity: 0;
}

.is-active .startup-stage,
.is-leaving .startup-stage {
  animation: startup-stage-in .72s cubic-bezier(.2, .8, .2, 1) forwards;
}

.startup-emblem {
  position: relative;
  display: grid;
  width: 144px;
  height: 144px;
  place-items: center;
}

.startup-mark {
  position: relative;
  z-index: 2;
  display: block;
  width: 82px;
  height: 82px;
  filter: drop-shadow(0 0 24px rgba(216, 233, 78, .16));
}

.startup-orbit {
  position: absolute;
  border-radius: 50%;
  pointer-events: none;
}

.startup-orbit-outer {
  inset: 0;
  border: 1px solid rgba(216, 233, 78, .2);
  border-top-color: rgba(216, 233, 78, .86);
  border-right-color: rgba(255, 255, 255, .34);
}

.startup-orbit-outer::before,
.startup-orbit-outer::after {
  content: '';
  position: absolute;
  border-radius: 50%;
}

.startup-orbit-outer::before {
  inset: 8px;
  border: 1px dashed rgba(255, 255, 255, .13);
}

.startup-orbit-outer::after {
  top: 50%;
  left: -3px;
  width: 6px;
  height: 6px;
  margin-top: -3px;
  background: #d8e94e;
  box-shadow: 0 0 12px rgba(216, 233, 78, .72);
}

.startup-orbit-inner {
  inset: 24px;
  border: 1px solid rgba(255, 255, 255, .075);
}

.startup-orbit-inner::before {
  content: '';
  position: absolute;
  top: -2px;
  left: 50%;
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: rgba(255, 255, 255, .72);
}

.is-active .startup-orbit-outer,
.is-leaving .startup-orbit-outer {
  animation: startup-orbit 5.6s linear infinite;
}

.is-active .startup-orbit-inner,
.is-leaving .startup-orbit-inner {
  animation: startup-orbit-reverse 7.2s linear infinite;
}

.startup-scan {
  position: absolute;
  z-index: 3;
  top: 70px;
  left: 25px;
  width: 94px;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(216, 233, 78, .92), transparent);
  box-shadow: 0 0 10px rgba(216, 233, 78, .58);
  opacity: 0;
}

.is-active .startup-scan,
.is-leaving .startup-scan {
  animation: startup-scan 1.6s cubic-bezier(.4, 0, .2, 1) infinite;
}

.startup-copy {
  margin-top: 24px;
  text-align: center;
}

.startup-kicker,
.startup-subtitle {
  margin: 0;
  color: rgba(167, 184, 186, .58);
  font: 10px/1.4 ui-monospace, Consolas, monospace;
}

.startup-kicker {
  color: rgba(216, 233, 78, .72);
}

.startup-copy h1 {
  margin: 4px 0 3px;
  color: #f5f8f4;
  font: 700 58px/.9 'Segoe UI', sans-serif;
  letter-spacing: 0;
}

.startup-copy h1 span {
  color: #d8e94e;
  text-shadow: 0 0 22px rgba(216, 233, 78, .24);
}

.startup-progress {
  width: min(460px, 76vw);
  margin-top: 34px;
}

.startup-progress-meta {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 18px;
  color: rgba(215, 225, 226, .64);
  font: 10px/1.4 ui-monospace, Consolas, monospace;
}

.startup-progress-meta strong {
  color: rgba(237, 246, 202, .92);
  font-size: 11px;
  font-weight: 600;
}

.startup-progress-track {
  position: relative;
  height: 2px;
  margin-top: 9px;
  overflow: hidden;
  background: rgba(255, 255, 255, .09);
}

.startup-progress-track i {
  position: absolute;
  inset: 0;
  transform-origin: left center;
  background: linear-gradient(90deg, rgba(216, 233, 78, .48), #d8e94e);
  box-shadow: 0 0 14px rgba(216, 233, 78, .3);
}

.startup-progress-stages {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  margin-top: 9px;
  color: rgba(151, 168, 171, .32);
  font: 9px/1 ui-monospace, Consolas, monospace;
}

.startup-progress-stages span {
  transition: color .25s ease;
}

.startup-progress-stages span:nth-child(2),
.startup-progress-stages span:nth-child(3) {
  text-align: center;
}

.startup-progress-stages span:last-child {
  text-align: right;
}

.startup-progress-stages span.active {
  color: rgba(218, 234, 156, .82);
}

.startup-footer {
  position: absolute;
  right: 42px;
  bottom: 34px;
  left: 42px;
  display: flex;
  justify-content: space-between;
  color: rgba(153, 170, 172, .4);
  font: 9px/1 ui-monospace, Consolas, monospace;
}

.startup-footer span {
  display: flex;
  align-items: center;
  gap: 8px;
}

.startup-footer i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #d8e94e;
  box-shadow: 0 0 9px rgba(216, 233, 78, .66);
}

@keyframes startup-stage-in {
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes startup-orbit {
  to {
    transform: rotate(360deg);
  }
}

@keyframes startup-orbit-reverse {
  to {
    transform: rotate(-360deg);
  }
}

@keyframes startup-scan {
  0%, 18% {
    transform: translateY(0);
    opacity: 0;
  }
  28% {
    opacity: .9;
  }
  76% {
    opacity: .9;
  }
  88%, 100% {
    transform: translateY(82px);
    opacity: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .startup-experience,
  .startup-stage,
  .startup-progress-stages span {
    transition-duration: .01ms;
  }

  .startup-stage {
    animation: none;
    opacity: 1;
    transform: none;
  }

  .startup-orbit,
  .startup-scan {
    animation: none !important;
  }
}

@media (max-width: 640px) {
  .startup-corner {
    top: 26px;
    right: 26px;
  }

  .startup-footer {
    right: 26px;
    bottom: 24px;
    left: 26px;
  }

  .startup-copy h1 {
    font-size: 48px;
  }
}
</style>
