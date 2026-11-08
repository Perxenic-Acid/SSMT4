<script setup lang="ts">
import {
  computed,
  onActivated,
  onBeforeUnmount,
  onDeactivated,
  onMounted,
  reactive,
  ref,
  watch,
} from 'vue'
import BackendConfigForm from './BackendConfigForm.vue'
import { openPath } from '@tauri-apps/plugin-opener'
import { Refresh, WarningFilled } from '@element-plus/icons-vue'
import { onBeforeRouteLeave } from 'vue-router'
import {
  loadPluginPageCatalog,
  getPluginPageSessionStatus,
  acquirePluginPageLease,
  acquireStaticPluginPageLease,
  releasePluginPageLease,
  releaseStaticPluginPageLease,
  touchPluginPageLease,
  requestPluginCapability,
  requestPluginCapabilityForLease,
  stopPluginPageService,
  PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION,
  PLUGIN_PAGE_BRIDGE_REQUEST_TYPE,
  PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE,
  type DeclarativePageField,
  type PluginCapabilityResponse,
  type PluginPageCatalogEntry,
  type PluginPageServiceInfo,
  type StaticPluginPageLease,
} from '../pageRuntime'

const props = defineProps<{
  pluginId: string
  gameName: string
  enabled: boolean
}>()

const pages = ref<PluginPageCatalogEntry[]>([])
const selectedPageId = ref('')
const loadingCatalog = ref(false)
const catalogError = ref('')
const pageError = ref('')
const serviceLoading = ref(false)
const serviceInfo = ref<PluginPageServiceInfo | null>(null)
const serviceFrame = ref<HTMLIFrameElement | null>(null)
const staticLease = ref<StaticPluginPageLease | null>(null)
const staticFrame = ref<HTMLIFrameElement | null>(null)
const frameLoaded = ref(false)
const frameKey = ref(0)
const declarativeValues = reactive<Record<string, unknown>>({})
const savingKey = ref('')
let disposed = false
let healthTimer: number | undefined
let leaseHeartbeatTimer: number | undefined
let serviceGeneration = 0
let pageActive = true
let startPending = false

const selectedPage = computed(() =>
  pages.value.find(page => page.pageId === selectedPageId.value) || null,
)
const staticUrl = computed(() => {
  const page = selectedPage.value
  if (page?.kind !== 'static' || !staticLease.value?.url) return ''
  const fragment = new URLSearchParams({
    pageLeaseId: staticLease.value.leaseId,
    pluginId: props.pluginId,
    game: props.gameName,
  })
  return `${staticLease.value.url}#${fragment}`
})
const serviceUrl = computed(() =>
  selectedPage.value?.kind === 'local-service' && serviceInfo.value?.running
    ? serviceInfo.value.url
    : '',
)
const activePageUrl = computed(() =>
  selectedPage.value?.kind === 'static' ? staticUrl.value : serviceUrl.value,
)

interface PluginCapabilityBridgeRequest {
  type?: unknown
  protocolVersion?: unknown
  requestId?: unknown
  capability?: unknown
  payload?: unknown
}

async function respondToPluginCapability(event: MessageEvent<PluginCapabilityBridgeRequest>, requestId: string,
  capability: string, response: Omit<PluginCapabilityResponse, 'requestId' | 'capability'>) {
  const origin = new URL(activePageUrl.value).origin
  event.source?.postMessage({
    type: PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE,
    protocolVersion: PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION,
    requestId,
    capability,
    ...response,
  }, { targetOrigin: origin })
}

async function handlePluginCapabilityMessage(event: MessageEvent<PluginCapabilityBridgeRequest>) {
  const frame = selectedPage.value?.kind === 'static' ? staticFrame.value : serviceFrame.value
  if (!frame?.contentWindow || event.source !== frame.contentWindow || !activePageUrl.value) return
  if (event.origin !== new URL(activePageUrl.value).origin) return

  const request = event.data
  if (!request
    || request.type !== PLUGIN_PAGE_BRIDGE_REQUEST_TYPE
    || request.protocolVersion !== PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION
    || typeof request.requestId !== 'string'
    || request.requestId.length === 0
    || request.requestId.length > 128
    || typeof request.capability !== 'string'
    || request.capability.length === 0
    || request.capability.length > 128
    || typeof request.payload !== 'object'
    || request.payload === null
    || Array.isArray(request.payload)) return

  const leaseId = selectedPage.value?.kind === 'static'
    ? staticLease.value?.leaseId
    : serviceInfo.value?.leaseId
  if (!leaseId) {
    await respondToPluginCapability(event, request.requestId, request.capability, {
      ok: false,
      errorCode: 'lease_invalid',
      errorMessage: 'Plugin page lease is unavailable.',
    })
    return
  }

  try {
    const response = await requestPluginCapabilityForLease(
      leaseId,
      request.capability,
      request.payload as Record<string, unknown>,
      request.requestId,
    )
    await respondToPluginCapability(event, request.requestId, request.capability, response)
  } catch (reason) {
    await respondToPluginCapability(event, request.requestId, request.capability, {
      ok: false,
      errorCode: 'bridge_error',
      errorMessage: String(reason),
    })
  }
}

async function refreshCatalog() {
  if (!props.pluginId || !props.gameName) return
  loadingCatalog.value = true
  catalogError.value = ''
  try {
    const allPages = await loadPluginPageCatalog(props.gameName)
    if (disposed) return
    pages.value = allPages.filter(page => page.pluginId === props.pluginId)
    const nextPageId = pages.value.some(page => page.pageId === selectedPageId.value)
      ? selectedPageId.value
      : pages.value[0]?.pageId || ''
    if (nextPageId !== selectedPageId.value) {
      selectedPageId.value = nextPageId
    } else if (nextPageId) {
      await mountSelectedPage()
    } else {
      await releaseServiceLease()
    }
  } catch (reason) {
    catalogError.value = String(reason)
    pages.value = []
  } finally {
    loadingCatalog.value = false
  }
}

function settingKey(pageId: string, fieldKey: string) {
  return `page:${pageId}:${fieldKey}`
}

async function loadDeclarativeValues(page: PluginPageCatalogEntry) {
  for (const key of Object.keys(declarativeValues)) delete declarativeValues[key]
  await Promise.all((page.fields || []).map(async field => {
    try {
      const response = await requestPluginCapability(
        props.pluginId,
        props.gameName,
        'plugin.settings.get',
        { key: settingKey(page.pageId, field.key) },
      ) as { value?: unknown } | null
      const stored = response?.value
      declarativeValues[field.key] = stored ?? field.defaultValue ?? defaultValueFor(field)
    } catch {
      declarativeValues[field.key] = field.defaultValue ?? defaultValueFor(field)
    }
  }))
}

function defaultValueFor(field: DeclarativePageField): unknown {
  if (field.kind === 'boolean') return false
  if (field.kind === 'integer') return field.min ?? 0
  if (field.kind === 'select') return field.options?.[0]?.value ?? ''
  return ''
}

async function updateField(field: DeclarativePageField, value: unknown) {
  const page = selectedPage.value
  if (!page || !props.enabled) return
  declarativeValues[field.key] = value
  savingKey.value = field.key
  try {
    await requestPluginCapability(
      props.pluginId,
      props.gameName,
      'plugin.settings.set',
      {
        key: settingKey(page.pageId, field.key),
        value,
      },
    )
  } catch (reason) {
    pageError.value = String(reason)
  } finally {
    savingKey.value = ''
  }
}

async function releaseStaticPageLease() {
  serviceGeneration += 1
  const leaseId = staticLease.value?.leaseId
  staticLease.value = null
  if (leaseId) await releaseStaticPluginPageLease(leaseId).catch(() => false)
}

async function releaseServiceLease() {
  serviceGeneration += 1
  clearHealthPoll()
  const leaseId = serviceInfo.value?.leaseId
  serviceInfo.value = null
  if (leaseId) await releasePluginPageLease(leaseId).catch(() => false)
}

async function startStaticPage() {
  if (disposed) return
  const page = selectedPage.value
  if (!page || page.kind !== 'static') return
  if (staticLease.value?.pageId === page.pageId) return
  const generation = ++serviceGeneration
  pageError.value = ''
  frameLoaded.value = false
  try {
    const lease = await acquireStaticPluginPageLease(props.pluginId, page.pageId, props.gameName)
    if (disposed || generation !== serviceGeneration) {
      await releaseStaticPluginPageLease(lease.leaseId).catch(() => false)
      return
    }
    staticLease.value = lease
    frameKey.value += 1
  } catch (reason) {
    staticLease.value = null
    pageError.value = String(reason)
  }
}

async function stopServiceNow() {
  serviceGeneration += 1
  clearHealthPoll()
  if (!serviceInfo.value?.sessionId) return
  const sessionId = serviceInfo.value.sessionId
  serviceInfo.value = null
  await stopPluginPageService(sessionId).catch(() => false)
}

async function startSelectedService() {
  if (disposed) return
  const page = selectedPage.value
  if (!page || page.kind !== 'local-service') return
  if (serviceInfo.value?.leaseId && serviceInfo.value.pageId === page.pageId) return
  if (serviceLoading.value) {
    if (pageActive) startPending = true
    return
  }
  if (!props.enabled) {
    pageError.value = '插件未启用，不能启动本地服务页面。'
    return
  }
  serviceLoading.value = true
  const generation = ++serviceGeneration
  pageError.value = ''
  frameLoaded.value = false
  try {
    if (serviceInfo.value?.sessionId && serviceInfo.value.pageId !== page.pageId) {
      await stopServiceNow()
    }
    const started = await acquirePluginPageLease(props.pluginId, page.pageId, props.gameName)
    if (disposed || generation !== serviceGeneration) {
      await releasePluginPageLease(started.leaseId).catch(() => false)
      return
    }
    serviceInfo.value = started
    startHealthPoll()
    frameKey.value += 1
  } catch (reason) {
    pageError.value = String(reason)
    serviceInfo.value = null
  } finally {
    serviceLoading.value = false
    if (startPending) {
      startPending = false
      if (pageActive && !disposed) void startSelectedService()
    }
  }
}

function clearHealthPoll() {
  if (healthTimer !== undefined) {
    window.clearInterval(healthTimer)
    healthTimer = undefined
  }
}

function startHealthPoll() {
  clearHealthPoll()
  const sessionId = serviceInfo.value?.sessionId
  if (!sessionId) return
  healthTimer = window.setInterval(async () => {
    if (disposed || !serviceInfo.value?.sessionId) return
    const status = await getPluginPageSessionStatus(sessionId).catch(() => null)
    if (status) {
      serviceInfo.value = status
      return
    }
    serviceInfo.value = null
    pageError.value = '本地编辑器已停止，请重新连接。'
    clearHealthPoll()
  }, 5000)
}

function startLeaseHeartbeat() {
  if (leaseHeartbeatTimer !== undefined) window.clearInterval(leaseHeartbeatTimer)
  leaseHeartbeatTimer = window.setInterval(() => {
    const leaseId = selectedPage.value?.kind === 'static'
      ? staticLease.value?.leaseId
      : serviceInfo.value?.leaseId
    if (leaseId && !disposed) void touchPluginPageLease(leaseId).catch(() => undefined)
  }, 15_000)
}

async function mountSelectedPage() {
  if (disposed) return
  await releaseServiceLease()
  await releaseStaticPageLease()
  pageError.value = ''
  frameLoaded.value = false
  const page = selectedPage.value
  if (!page) return
  if (page.kind === 'declarative') {
    if (!page.backendConfig) await loadDeclarativeValues(page)
    return
  }
  if (page.kind === 'local-service') {
    await startSelectedService()
  } else if (page.kind === 'static') {
    await startStaticPage()
  }
}

async function restartLocalService() {
  await stopServiceNow()
  await startSelectedService()
}

async function openServicePath(path: string | undefined) {
  if (!path) return
  try {
    await openPath(path)
  } catch (reason) {
    pageError.value = String(reason)
  }
}

watch([() => props.pluginId, () => props.gameName], refreshCatalog)
watch(selectedPageId, () => {
  if (!disposed) void mountSelectedPage()
})
watch(() => props.enabled, enabled => {
  if (disposed) return
  if (!enabled) {
    void stopServiceNow()
    void releaseStaticPageLease()
  }
})

onMounted(() => {
  pageActive = true
  window.addEventListener('message', handlePluginCapabilityMessage)
  startLeaseHeartbeat()
  void refreshCatalog()
})
onActivated(async () => {
  pageActive = true
  if (disposed) return
  if (selectedPage.value?.kind === 'static') {
    if (!staticLease.value?.leaseId) await startStaticPage()
    return
  }
  if (!serviceInfo.value?.sessionId) {
    if (selectedPage.value?.kind === 'local-service') await startSelectedService()
    return
  }
  const status = await getPluginPageSessionStatus(serviceInfo.value.sessionId).catch(() => null)
  if (status) {
    serviceInfo.value = status
    startHealthPoll()
  } else {
    serviceInfo.value = null
    await startSelectedService()
  }
})
onDeactivated(() => {
  pageActive = false
  startPending = false
  void releaseServiceLease()
  void releaseStaticPageLease()
})
onBeforeRouteLeave(() => {
  pageActive = false
  startPending = false
  void releaseServiceLease()
  void releaseStaticPageLease()
})
onBeforeUnmount(() => {
  disposed = true
  pageActive = false
  startPending = false
  if (leaseHeartbeatTimer !== undefined) window.clearInterval(leaseHeartbeatTimer)
  window.removeEventListener('message', handlePluginCapabilityMessage)
  clearHealthPoll()
  void releaseServiceLease()
  void releaseStaticPageLease()
})
</script>

<template>
  <section class="plugin-page-host">
    <div v-if="loadingCatalog" class="plugin-page-loading">正在读取插件页面…</div>
    <div v-else-if="catalogError" class="plugin-page-error">
      <WarningFilled />
      <span>{{ catalogError }}</span>
    </div>
    <template v-else-if="pages.length">
      <nav v-if="pages.length > 1" class="plugin-page-tabs" aria-label="插件页面">
        <button
          v-for="page in pages"
          :key="page.pageId"
          type="button"
          :class="{ active: page.pageId === selectedPageId }"
          @click="selectedPageId = page.pageId"
        >
          {{ page.title }}
        </button>
      </nav>

      <div v-if="pageError" class="plugin-page-error">
        <WarningFilled />
        <span>{{ pageError }}</span>
      </div>

      <BackendConfigForm v-if="selectedPage?.kind === 'declarative' && selectedPage.backendConfig" :key="`${pluginId}:${gameName}:${selectedPage.pageId}`" :plugin-id="pluginId" :page-id="selectedPage.pageId" :game-name="gameName" :enabled="enabled" />
      <section v-else-if="selectedPage?.kind === 'declarative'" class="declarative-page">
        <header>
          <strong>{{ selectedPage.title }}</strong>
          <p v-if="selectedPage.description">{{ selectedPage.description }}</p>
        </header>
        <label v-for="field in selectedPage.fields || []" :key="field.key" class="declarative-field">
          <span>
            <strong>{{ field.label }}</strong>
            <small v-if="field.description">{{ field.description }}</small>
          </span>
          <input
            v-if="field.kind === 'boolean'"
            type="checkbox"
            :checked="Boolean(declarativeValues[field.key])"
            :disabled="!enabled || savingKey === field.key"
            @change="updateField(field, ($event.target as HTMLInputElement).checked)"
          >
          <input
            v-else-if="field.kind === 'integer'"
            type="number"
            :value="declarativeValues[field.key] as number"
            :min="field.min ?? undefined"
            :max="field.max ?? undefined"
            :required="field.required"
            :disabled="!enabled || savingKey === field.key"
            @change="updateField(field, Number(($event.target as HTMLInputElement).value))"
          >
          <select
            v-else-if="field.kind === 'select'"
            :value="String(declarativeValues[field.key] ?? '')"
            :disabled="!enabled || savingKey === field.key"
            @change="updateField(field, ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="option in field.options || []" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
          <input
            v-else
            type="text"
            :value="String(declarativeValues[field.key] ?? '')"
            :required="field.required"
            :disabled="!enabled || savingKey === field.key"
            @change="updateField(field, ($event.target as HTMLInputElement).value)"
          >
        </label>
      </section>

      <section v-else-if="selectedPage?.kind === 'static'" class="plugin-page-frame-wrap">
        <iframe
          v-if="staticUrl"
          ref="staticFrame"
          :key="staticUrl"
          :src="staticUrl"
          class="plugin-page-frame"
          :class="{ ready: frameLoaded }"
          sandbox="allow-scripts allow-same-origin allow-forms"
          title="插件页面"
          @load="frameLoaded = true"
        />
        <div v-else class="plugin-page-error">
          <WarningFilled />
          <span>静态页面路径无效。</span>
        </div>
      </section>

      <section v-else-if="selectedPage?.kind === 'local-service'" class="local-service-page">
        <header class="local-service-toolbar">
          <div>
            <strong>{{ selectedPage.title }}</strong>
            <span :class="{ online: serviceInfo?.running }">
              {{ serviceLoading ? '正在启动…' : serviceInfo?.running ? '本地服务已连接' : '本地服务未连接' }}
            </span>
          </div>
          <div>
            <button type="button" :disabled="!enabled || serviceLoading" @click="restartLocalService">
              <Refresh /> 重新连接
            </button>
            <button type="button" :disabled="!serviceInfo" @click="openServicePath(serviceInfo?.logPath)">
              日志
            </button>
            <button type="button" :disabled="!serviceInfo" @click="openServicePath(serviceInfo?.dataRoot)">
              数据目录
            </button>
          </div>
        </header>
        <div v-if="serviceLoading || !frameLoaded" class="local-service-loading">
          {{ serviceInfo ? '正在加载插件页面…' : '正在分配本地会话…' }}
        </div>
        <iframe
          v-if="serviceUrl"
          ref="serviceFrame"
          :key="frameKey"
          :src="serviceUrl"
          class="plugin-page-frame"
          :class="{ ready: frameLoaded }"
          sandbox="allow-scripts allow-same-origin allow-forms"
          title="插件本地服务页面"
          @load="frameLoaded = true"
        />
      </section>
    </template>
    <div v-else class="muted-copy">当前插件没有提供页面。</div>
  </section>
</template>

<style scoped>
.plugin-page-host { display:flex; flex-direction:column; gap:12px; min-width:0; }
.plugin-page-tabs { display:flex; flex-wrap:wrap; gap:6px; }
.plugin-page-tabs button { min-height:30px; padding:5px 12px; border:1px solid #314248; border-radius:5px; background:#121b1f; color:#b9c8cc; cursor:pointer; }
.plugin-page-tabs button.active { border-color:#70c4ad; background:#17302a; color:#e9fff8; }
.plugin-page-loading,.muted-copy { padding:14px; color:#82969d; font-size:12px; }
.plugin-page-error { display:flex; align-items:flex-start; gap:8px; padding:11px 13px; border:1px solid #704346; border-radius:6px; background:#241517; color:#efb6b2; font-size:12px; line-height:1.55; }
.plugin-page-error svg { flex:0 0 16px; width:16px; height:16px; margin-top:1px; }
.declarative-page { display:flex; flex-direction:column; gap:12px; padding:16px; border:1px solid #26363d; border-radius:8px; background:#0c1418; }
.declarative-page header { display:flex; flex-direction:column; gap:4px; }
.declarative-page header strong { color:#e7f1f3; font-size:15px; }
.declarative-page header p { margin:0; color:#8fa1a8; font-size:11px; line-height:1.6; }
.declarative-field { display:grid; grid-template-columns:minmax(0,1fr) minmax(180px,300px); align-items:center; gap:18px; padding:10px 0; border-top:1px solid #1d2b31; }
.declarative-field > span { display:flex; flex-direction:column; gap:3px; }
.declarative-field strong { color:#d7e4e7; font-size:12px; }
.declarative-field small { color:#789096; font-size:10px; line-height:1.5; }
.declarative-field input:not([type=checkbox]),.declarative-field select { min-height:32px; border:1px solid #34464d; border-radius:5px; background:#10191e; color:#e4edef; padding:5px 9px; }
.plugin-page-frame-wrap,.local-service-page {
  /* Parent .app-ui-scale uses a transform. Render the iframe at the scaled
     pixel density, then undo that transform locally so text is not bitmap-stretched. */
  --plugin-page-height:72vh;
  --plugin-page-min-height:520px;
  --plugin-frame-scale:var(--app-ui-scale, 1);
  --plugin-frame-inverse-scale:var(--app-ui-scale-inverse, 1);
  position:relative;
  height:var(--plugin-page-height);
  min-height:var(--plugin-page-min-height);
  overflow:hidden;
  border:1px solid #26363d;
  border-radius:8px;
  background:#080d10;
}
.local-service-page {
  height:calc(var(--plugin-page-height) + 52px);
  min-height:calc(var(--plugin-page-min-height) + 52px);
}
.plugin-page-frame {
  position:absolute;
  top:0;
  left:0;
  display:block;
  width:calc(100% * var(--plugin-frame-scale));
  height:calc(100% * var(--plugin-frame-scale));
  min-height:0;
  border:0;
  background:#080d10;
  zoom:var(--plugin-frame-scale);
  transform:scale(var(--plugin-frame-inverse-scale));
  transform-origin:top left;
}
.local-service-page .plugin-page-frame {
  top:52px;
  height:calc((100% - 52px) * var(--plugin-frame-scale));
}
.local-service-toolbar { position:relative; z-index:2; display:flex; align-items:center; justify-content:space-between; gap:14px; padding:10px 12px; border-bottom:1px solid #26363d; background:#0d161a; }
.local-service-toolbar > div { display:flex; align-items:center; gap:9px; }
.local-service-toolbar strong { color:#e6f0f2; font-size:12px; }
.local-service-toolbar span { color:#82969d; font-size:10px; }
.local-service-toolbar span.online { color:#75dcb8; }
.local-service-toolbar button { display:inline-flex; align-items:center; gap:5px; min-height:30px; padding:5px 10px; border:1px solid #34464d; border-radius:5px; background:#121d21; color:#c8d5d8; cursor:pointer; }
.local-service-toolbar button:disabled { opacity:.45; cursor:not-allowed; }
.local-service-toolbar button svg { width:14px; height:14px; }
.local-service-loading { position:absolute; inset:52px 0 0; z-index:1; display:grid; place-items:center; background:#080d10; color:#93a7ad; font-size:12px; }
.local-service-page .plugin-page-frame { opacity:0; transition:opacity .2s ease; }
.local-service-page .plugin-page-frame.ready { opacity:1; }
@media (max-width: 760px) { .declarative-field { grid-template-columns:1fr; gap:8px; } .local-service-toolbar { align-items:flex-start; flex-direction:column; } }
</style>
