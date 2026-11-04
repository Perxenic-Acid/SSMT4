<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Refresh, WarningFilled } from '@element-plus/icons-vue'
import { PathHelper } from '../../helper/PathHelper'
import { AppStateManager } from '../../store/AppStateManager'

interface EditorInfo {
  editorRoot: string
  dataRoot: string
  running: boolean
  url: string
  apiToken: string
  logPath: string
}

const loading = ref(false)
const error = ref('')
const info = ref<EditorInfo | null>(null)
const frameKey = ref(0)
const frameLoaded = ref(false)
const stage = ref(0)
const gameName = computed(() => AppStateManager.appSettings.CurrentGameName || 'EFMI')
const stageText = computed(() => [
  '检查插件运行环境',
  '启动本地编辑器',
  '建立安全 API 连接',
  '载入插件界面',
][stage.value] || '正在连接')
const progress = computed(() => [12, 38, 68, 88][stage.value] || 88)
const frameUrl = computed(() => {
  if (!info.value?.running || !info.value.url || !info.value.apiToken) return ''
  const token = encodeURIComponent(info.value.apiToken)
  const game = encodeURIComponent(gameName.value)
  return `${info.value.url.replace(/\/$/, '')}/ui/#token=${token}&game=${game}`
})

async function connectEditor(forceRestart = false) {
  if (loading.value) return
  loading.value = true
  error.value = ''
  frameLoaded.value = false
  stage.value = 0
  try {
    const migotoRoot = await PathHelper.GetCurrentGame3DmigotoFolderPath()
    if (!migotoRoot) throw new Error('请先配置 EFMI 的 3Dmigoto 目录')
    stage.value = 1
    let next = await invoke<EditorInfo>('liquid_gravity_probe_detect', {
      gameName: gameName.value,
      migotoRoot,
    })
    if (forceRestart && next.running) {
      await invoke('stop_liquid_gravity_probe_editor')
      next = { ...next, running: false }
    }
    if (!next.running) {
      stage.value = 2
      next = await invoke<EditorInfo>('start_liquid_gravity_probe_editor', {
        gameName: gameName.value,
        migotoRoot,
      })
    }
    stage.value = 3
    info.value = next
    frameKey.value += 1
  } catch (reason) {
    info.value = null
    error.value = String(reason)
  } finally {
    loading.value = false
  }
}

function onFrameLoaded() {
  frameLoaded.value = true
  loading.value = false
}

onMounted(() => void connectEditor())
</script>

<template>
  <main class="lgp-host">
    <iframe
      v-if="frameUrl"
      :key="frameKey"
      :src="frameUrl"
      class="lgp-host-frame"
      :class="{ ready: frameLoaded }"
      sandbox="allow-scripts allow-same-origin allow-forms"
      title="Liquid Gravity Probe"
      @load="onFrameLoaded"
    />

    <transition name="lgp-loader-fade">
      <section v-if="!frameUrl || !frameLoaded" class="lgp-loader" aria-live="polite">
        <div class="lgp-loader-grid" />
        <div class="lgp-loader-content">
          <div class="lgp-loader-brand">
            <span class="lgp-drop-mark"><span /></span>
            <div>
              <p class="lgp-loader-eyebrow">SSMT FIRST-PARTY PLUGIN</p>
              <h1>Liquid Gravity Probe</h1>
            </div>
          </div>

          <div v-if="!error" class="lgp-loader-progress">
            <div class="lgp-loader-stream">
              <i v-for="index in 6" :key="index" :style="{ '--drop-index': index }" />
            </div>
            <div class="lgp-loader-copy">
              <span>{{ stageText }}</span>
              <strong>{{ progress }}%</strong>
            </div>
            <div class="lgp-loader-track"><span :style="{ width: `${progress}%` }" /></div>
            <div class="lgp-loader-stages">
              <span :class="{ active: stage >= 0 }">环境</span>
              <span :class="{ active: stage >= 1 }">编辑器</span>
              <span :class="{ active: stage >= 2 }">API</span>
              <span :class="{ active: stage >= 3 }">界面</span>
            </div>
          </div>

          <div v-else class="lgp-loader-error">
            <div class="lgp-loader-error-title">
              <WarningFilled />
              <strong>插件编辑器连接失败</strong>
            </div>
            <p>{{ error }}</p>
            <button type="button" class="lgp-retry" @click="connectEditor(true)">
              <Refresh />
              <span>重新连接</span>
            </button>
          </div>

          <div class="lgp-loader-tags">
            <span>EFMI</span>
            <span>3Dmigoto</span>
            <span>LOCAL</span>
          </div>
        </div>
      </section>
    </transition>
  </main>
</template>

<style scoped>
.lgp-host { position:relative; display:flex; width:100%; height:100%; min-height:0; overflow:hidden; background:#070b0d; }
.lgp-host-frame { display:block; width:100%; height:100%; min-height:0; border:0; background:#090d10; opacity:0; transition:opacity .24s ease; }
.lgp-host-frame.ready { opacity:1; }
.lgp-loader { position:absolute; inset:0; z-index:10; display:grid; place-items:center; overflow:hidden; background:#070b0d; color:#e9f2f4; }
.lgp-loader-grid { position:absolute; inset:0; background-image:linear-gradient(rgba(111,139,148,.06) 1px,transparent 1px),linear-gradient(90deg,rgba(111,139,148,.06) 1px,transparent 1px); background-size:42px 42px; mask-image:linear-gradient(180deg,transparent,#000 28%,#000 72%,transparent); }
.lgp-loader-content { position:relative; z-index:1; display:flex; flex-direction:column; width:min(430px,calc(100vw - 40px)); gap:26px; }
.lgp-loader-brand { display:flex; align-items:center; gap:15px; }
.lgp-loader-brand h1 { margin:2px 0 0; color:#f2f8f9; font-size:24px; font-weight:650; letter-spacing:0; }
.lgp-loader-eyebrow { margin:0; color:#82969d; font-size:10px; font-weight:700; letter-spacing:.08em; }
.lgp-drop-mark { position:relative; display:grid; flex:0 0 56px; width:56px; height:56px; place-items:center; border:1px solid rgba(216,233,78,.42); border-radius:50% 50% 50% 12px; transform:rotate(-45deg); background:#11191d; box-shadow:inset 0 0 22px rgba(216,233,78,.09); }
.lgp-drop-mark::before { content:""; position:absolute; inset:10px; border-radius:50% 50% 50% 8px; background:#d8e94e; animation:lgp-mark-pulse 1.7s ease-in-out infinite; }
.lgp-drop-mark span { position:absolute; top:11px; left:15px; width:8px; height:8px; border-radius:50%; background:#f5ffb0; opacity:.9; }
.lgp-loader-progress { display:flex; flex-direction:column; gap:12px; }
.lgp-loader-stream { position:relative; width:100%; height:46px; overflow:hidden; border-bottom:1px solid rgba(130,155,163,.23); }
.lgp-loader-stream i { position:absolute; top:-12px; left:calc(12% + var(--drop-index) * 14%); width:5px; height:11px; border-radius:50% 50% 55% 55%; background:#d8e94e; opacity:0; filter:drop-shadow(0 0 5px rgba(216,233,78,.55)); animation:lgp-drop-fall 1.65s linear infinite; animation-delay:calc(var(--drop-index) * -.23s); }
.lgp-loader-copy { display:flex; align-items:center; justify-content:space-between; color:#c4d2d6; font-size:12px; }
.lgp-loader-copy strong { color:#d8e94e; font:600 12px/1 ui-monospace,Consolas,monospace; }
.lgp-loader-track { position:relative; height:4px; overflow:hidden; border-radius:2px; background:#162126; }
.lgp-loader-track span { display:block; height:100%; border-radius:2px; background:#d8e94e; box-shadow:0 0 12px rgba(216,233,78,.35); transition:width .35s ease; }
.lgp-loader-track::after { position:absolute; inset:0; content:""; background:linear-gradient(90deg,transparent,rgba(255,255,255,.35),transparent); transform:translateX(-100%); animation:lgp-scan 1.6s linear infinite; }
.lgp-loader-stages { display:grid; grid-template-columns:repeat(4,1fr); gap:8px; color:#65777e; font-size:10px; text-align:center; }
.lgp-loader-stages span { padding-top:7px; border-top:1px solid #26343a; transition:color .25s ease,border-color .25s ease; }
.lgp-loader-stages span.active { color:#c8d4d8; border-color:#9aaa4d; }
.lgp-loader-error { display:flex; flex-direction:column; gap:14px; padding:18px; border:1px solid rgba(223,105,105,.28); border-radius:8px; background:rgba(37,18,20,.66); }
.lgp-loader-error-title { display:flex; align-items:center; gap:8px; color:#ffb0ac; }
.lgp-loader-error-title svg { width:17px; height:17px; }
.lgp-loader-error p { max-height:140px; margin:0; overflow:auto; color:#d8b8b6; font:11px/1.6 ui-monospace,Consolas,monospace; white-space:pre-wrap; overflow-wrap:anywhere; }
.lgp-retry { display:inline-flex; width:max-content; min-width:116px; align-items:center; justify-content:center; gap:7px; padding:8px 16px; border:1px solid #425158; border-radius:6px; background:#151f24; color:#edf5f6; font:inherit; font-size:12px; white-space:nowrap; cursor:pointer; }
.lgp-retry:hover { border-color:#d8e94e; color:#f5ffb0; }
.lgp-retry svg { width:15px; height:15px; }
.lgp-loader-tags { display:flex; gap:8px; align-self:flex-end; }
.lgp-loader-tags span { padding:4px 7px; border:1px solid #26343a; border-radius:999px; color:#6f838a; font:10px/1 ui-monospace,Consolas,monospace; }
.lgp-loader-fade-leave-active { transition:opacity .24s ease; }
.lgp-loader-fade-leave-to { opacity:0; }
@keyframes lgp-mark-pulse { 0%,100% { transform:scale(.92); opacity:.76; } 50% { transform:scale(1); opacity:1; } }
@keyframes lgp-drop-fall { 0% { transform:translateY(-10px) scaleY(.7); opacity:0; } 18% { opacity:.9; } 78% { opacity:.7; } 100% { transform:translateY(48px) scaleY(1.15); opacity:0; } }
@keyframes lgp-scan { to { transform:translateX(100%); } }
@media (max-width:560px) { .lgp-loader-brand h1 { font-size:20px; } .lgp-drop-mark { flex-basis:48px; width:48px; height:48px; } .lgp-loader-content { gap:20px; } }
</style>
