<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { openPath } from '@tauri-apps/plugin-opener'
import { ElMessage } from 'element-plus'
import { ArrowDown, Close, Document, FolderOpened, Refresh, VideoPlay } from '@element-plus/icons-vue'
import { useRouter } from 'vue-router'
import { PathHelper } from '../../helper/PathHelper'
import { AppStateManager } from '../../store/AppStateManager'
import { LIQUID_GRAVITY_PROBE_PLUGIN_ID } from '../../plugin/liquidGravityProbe'
import {
  acceptLiquidGravityProbeConsent,
  declineLiquidGravityProbeConsent,
  hasLiquidGravityProbeConsent,
  setLiquidGravityProbeEnabled,
} from '../../plugin/liquidGravityProbeConsent'
import LiquidGravityProbeDisclaimer from '../../components/LiquidGravityProbeDisclaimer.vue'
import ProbeViewport3D from './ProbeViewport3D.vue'
import {
  applyProbeCleanup,
  applyProbeNozzles,
  loadProbeEffects,
  loadProbeKeyBinding,
  loadProbeCatalog,
  previewProbeCleanup,
  reinjectProbeModel,
  removeProbeModelEffect,
  saveProbeEffects,
  saveProbeKeyBinding,
  saveProbeModelEffect,
  saveProbeNozzles,
  type ProbeAnchor,
  type ProbeCatalog,
  type ProbeCatalogModel,
  type ProbeEffectField,
  type ProbeEffectSettings,
  type ProbeGeometry,
  type ProbeKeyBinding,
  type ProbeNozzle,
} from './probeApi'

const props = withDefaults(defineProps<{
  embeddedUrl?: string
  embeddedToken?: string
  embeddedGame?: string
}>(), {
  embeddedUrl: '',
  embeddedToken: '',
  embeddedGame: 'EFMI',
})

interface EditorInfo {
  editorRoot: string
  dataRoot: string
  running: boolean
  url: string
  apiToken: string
  logPath: string
}

const pluginId = LIQUID_GRAVITY_PROBE_PLUGIN_ID
const router = useRouter()
const loading = ref(false)
const info = ref<EditorInfo | null>(null)
const error = ref('')
const pluginReady = ref(false)
const pluginChecked = ref(false)
const disclaimerOpen = ref(false)
const disclaimerBusy = ref(false)
const panelTab = ref<'nozzle' | 'effects' | 'key'>('nozzle')
const catalog = ref<ProbeCatalog | null>(null)
const selectedModelId = ref('')
const previewRevision = ref(0)
const modelSearch = ref('')
const geometryInfo = ref<ProbeGeometry | null>(null)
const modelEffectColor = ref<number[] | null>(null)
const modelEffectStrength = ref(1)
const savedModelEffectColor = ref<number[] | null>(null)
const savedModelEffectStrength = ref(1)
const nozzles = ref<ProbeNozzle[]>([{ slot: 0, anchor: null, sourceProfile: null, intervalSeconds: null }])
const activeSlot = ref(0)
const selectedGroup = ref<string | null>(null)
const hiddenGroups = ref<string[]>([])
const flowPreviewEnabled = ref(true)
const groupDetailsOpen = ref(true)
const nativeError = ref('')
const nativeLoading = ref(false)
const effectData = ref<ProbeEffectSettings | null>(null)
const effectValues = ref<Record<string, number>>({})
const keyBinding = ref<ProbeKeyBinding | null>(null)
const recordingKey = ref(false)
const openSelectKey = ref<string | null>(null)
let pollTimer: number | undefined
let workspaceStarted = false

const embeddedMode = computed(() => Boolean(props.embeddedUrl && props.embeddedToken))
const currentGame = computed(() => embeddedMode.value ? props.embeddedGame : AppStateManager.appSettings.CurrentGameName)
const isEndfield = computed(() => currentGame.value.toUpperCase() === 'EFMI')
const statusTitle = computed(() => {
  if (embeddedMode.value) return '插件内置编辑器运行中'
  if (!pluginReady.value) return pluginChecked.value ? '插件未启用' : '正在检查插件'
  if (error.value) return '未就绪'
  return info.value?.running ? '插件编辑器运行中' : '编辑器未启动'
})
const filteredModels = computed(() => {
  const query = modelSearch.value.trim().toLowerCase()
  const models = catalog.value?.mods || []
  return query ? models.filter(model => model.name.toLowerCase().includes(query)) : models
})
const selectedModel = computed(() =>
  catalog.value?.mods.find(model => model.id === selectedModelId.value)
)
const activeNozzle = computed(() =>
  nozzles.value.find(nozzle => nozzle.slot === activeSlot.value) || nozzles.value[0]
)
const activeAnchor = computed(() => activeNozzle.value?.anchor || null)
const integrationStatus = computed(() => geometryInfo.value?.integration || null)
const effectGroups = computed(() => {
  const groups = new Map<string, ProbeEffectField[]>()
  for (const field of effectData.value?.fields || []) {
    if (field.hidden) continue
    groups.set(field.group, [...(groups.get(field.group) || []), field])
  }
  return [...groups.entries()].map(([name, fields]) => ({ name, fields }))
})
const visibleEffectGroups = computed(() =>
  effectGroups.value.filter(group => group.name === '性能')
)
const particlePalette = [
  { label: '琥珀（原版）', color: '#c7820e' },
  { label: '冰蓝', color: '#65d6ff' },
  { label: '青绿', color: '#8ef0c7' },
  { label: '粉红', color: '#ff6e9a' },
  { label: '紫罗兰', color: '#b88cff' },
  { label: '亮白', color: '#ffffff' },
]
const particleColorValue = computed(() =>
  `#${(modelEffectColor.value || [
    Number(effectValues.value.particle_red ?? 0),
    Number(effectValues.value.particle_green ?? 0),
    Number(effectValues.value.particle_blue ?? 0),
  ])
    .map(value => Math.max(0, Math.min(255, Math.round(Number(value) * 255))))
    .map(value => value.toString(16).padStart(2, '0')).join('')}`
)
const currentParticleColorLabel = computed(() =>
  particlePalette.find(item => item.color === particleColorValue.value)?.label || '自定义颜色'
)

function selectOptionLabel(field: ProbeEffectField) {
  const value = Number(effectValues.value[field.key] ?? field.default)
  return field.options?.find(option => Number(option.value) === value)?.label || String(value)
}

function chooseEffectOption(field: ProbeEffectField, value: string | number) {
  effectValues.value[field.key] = Number(value)
  openSelectKey.value = null
}

function setParticleColor(color: string) {
  const match = /^#?([0-9a-f]{6})$/i.exec(color)
  if (!match) return
  const value = Number.parseInt(match[1], 16)
  const rgb = [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255]
  modelEffectColor.value = rgb
  effectValues.value.particle_red = rgb[0]
  effectValues.value.particle_green = rgb[1]
  effectValues.value.particle_blue = rgb[2]
}

async function migotoRoot() {
  const path = await PathHelper.GetCurrentGame3DmigotoFolderPath()
  if (!path) throw new Error('请先在 SSMT 中选择 EFMI 游戏配置')
  return path
}

async function refreshPluginState() {
  if (embeddedMode.value) {
    pluginReady.value = true
    pluginChecked.value = true
    return true
  }
  if (!isEndfield.value) {
    pluginReady.value = false
    pluginChecked.value = true
    return false
  }
  try {
    const plugins = await invoke<Array<{ manifest: { id: string }; enabled: boolean }>>(
      'plugin_registry_snapshot_for_game',
      { gameName: currentGame.value },
    )
    pluginReady.value = plugins.some(plugin => plugin.manifest.id === pluginId && plugin.enabled)
  } catch {
    pluginReady.value = false
  } finally {
    pluginChecked.value = true
  }
  return pluginReady.value
}

async function refreshStatus() {
  if (embeddedMode.value) {
    info.value = {
      editorRoot: '',
      dataRoot: '',
      running: true,
      url: props.embeddedUrl,
      apiToken: props.embeddedToken,
      logPath: '',
    }
    pluginReady.value = true
    pluginChecked.value = true
    return
  }
  loading.value = true
  error.value = ''
  try {
    if (!await refreshPluginState()) {
      info.value = null
      return
    }
    const next = await invoke<EditorInfo>('liquid_gravity_probe_detect', {
      gameName: currentGame.value,
      migotoRoot: await migotoRoot(),
    })
    info.value = next
  } catch (reason) {
    info.value = null
    error.value = String(reason)
  } finally {
    loading.value = false
  }
}

async function loadWorkspace() {
  if (!info.value?.running) return
  nativeLoading.value = true
  nativeError.value = ''
  try {
    const token = info.value.apiToken
    const [nextCatalog, nextEffects, nextKeyBinding] = await Promise.all([
      loadProbeCatalog(info.value.url, token),
      loadProbeEffects(info.value.url, token),
      loadProbeKeyBinding(info.value.url, token),
    ])
    catalog.value = nextCatalog
    effectData.value = nextEffects
    effectValues.value = { ...nextEffects.values }
    keyBinding.value = nextKeyBinding
    if (!catalog.value.mods.some(model => model.id === selectedModelId.value)) {
      selectedModelId.value = catalog.value.mods[0]?.id || ''
    }
    if (!selectedModelId.value) geometryInfo.value = null
  } catch (reason) {
    nativeError.value = String(reason)
  } finally {
    nativeLoading.value = false
  }
}

function selectNativeModel(model: ProbeCatalogModel) {
  selectedModelId.value = model.id
  nativeError.value = ''
}

function onGeometryLoaded(next: ProbeGeometry) {
  geometryInfo.value = next
  modelEffectColor.value = next.effectColor?.length === 3 ? [...next.effectColor] : null
  modelEffectStrength.value = Number(next.effectStrength ?? 1)
  savedModelEffectColor.value = modelEffectColor.value ? [...modelEffectColor.value] : null
  savedModelEffectStrength.value = modelEffectStrength.value
  if (modelEffectColor.value) {
    effectValues.value.particle_red = modelEffectColor.value[0]
    effectValues.value.particle_green = modelEffectColor.value[1]
    effectValues.value.particle_blue = modelEffectColor.value[2]
  }
  hiddenGroups.value = next.groups.filter(group => !group.defaultVisible).map(group => group.id)
  const loadedNozzles = (next.nozzles || []).map((item, index) => ({
    slot: item.slot ?? index,
    anchor: item.anchor ? { ...item.anchor, model: next.id } : null,
    sourceProfile: item.sourceProfile ?? item.anchor?.sourceProfile ?? null,
    intervalSeconds: typeof item.intervalSeconds === 'number' ? item.intervalSeconds : null,
    pendingApply: false,
  }))
  nozzles.value = loadedNozzles.length
    ? loadedNozzles
    : [{
        slot: 0,
        anchor: next.anchor ? { ...next.anchor, model: next.id } : null,
        sourceProfile: next.anchor?.sourceProfile ?? null,
        intervalSeconds: null,
        pendingApply: false,
      }]
  activeSlot.value = nozzles.value[0]?.slot ?? 0
  selectedGroup.value = nozzles.value[0]?.anchor?.group || null
}

function selectNativeNozzle(slot: number) {
  activeSlot.value = slot
  selectedGroup.value = activeNozzle.value?.anchor?.group || null
}

function addNativeNozzle() {
  if (nozzles.value.length >= 4) {
    ElMessage.warning('最多支持 4 个喷口')
    return
  }
  const slot = nozzles.value.length
  nozzles.value.push({ slot, anchor: null, sourceProfile: null, intervalSeconds: null, pendingApply: true })
  selectNativeNozzle(slot)
}

async function removeNativeNozzle() {
  if (nativeLoading.value) return
  const removed = activeNozzle.value
  if (!removed) return
  const remaining = nozzles.value
    .filter(nozzle => nozzle.slot !== activeSlot.value)
    .map((nozzle, slot) => ({ ...nozzle, slot }))
  const wasApplied = typeof removed.sourceProfile === 'number'
  if (wasApplied) {
    if (!remaining.length && !window.confirm('确认移除当前 Mod 的全部粒子效果？应用写入的钩子会被删除，Mod 将恢复默认。')) {
      return
    }
    if (remaining.some(nozzle => !nozzle.anchor)) {
      ElMessage.warning('其余喷口尚未完成选点，请先完成选点再删除已应用的喷口')
      return
    }
    if (remaining.some(nozzle => nozzle.pendingApply)) {
      ElMessage.warning('其余喷口还有未应用修改，请先应用或重新加载后再删除')
      return
    }
    nativeLoading.value = true
    try {
      await applyProbeNozzles(
        info.value!.url,
        info.value!.apiToken,
        geometryInfo.value!.id,
        remaining.map((nozzle, slot) => ({
          ...nozzle.anchor!,
          slot,
          intervalSeconds: nozzle.intervalSeconds ?? null,
        })),
      )
      ElMessage.success(remaining.length ? '喷口已删除并同步到 Mod' : '粒子效果已移除，Mod 已恢复默认；请按 F10')
    } catch (reason) {
      ElMessage.error(String(reason))
      return
    } finally {
      nativeLoading.value = false
    }
  }
  nozzles.value = remaining.length
    ? remaining
    : [{ slot: 0, anchor: null, sourceProfile: null, intervalSeconds: null }]
  selectNativeNozzle(0)
  previewRevision.value += 1
}

function onNativePicked(anchor: ProbeAnchor) {
  const nozzle = activeNozzle.value
  if (!nozzle) return
  const previous = nozzle.anchor
  nozzle.anchor = {
    ...anchor,
    offset: previous?.offset ? [...previous.offset] : [...(anchor.offset || [0, 0, 0])],
    direction: previous?.direction
      ? [...previous.direction]
      : [...(anchor.direction || [0, 0, -1])],
  }
  nozzle.sourceProfile = null
  nozzle.pendingApply = true
  selectedGroup.value = anchor.group
}

function updateOffset(axis: number, value: number) {
  const anchor = activeAnchor.value
  if (!anchor || axis < 0 || axis > 2) return
  const offset = [...(anchor.offset || [0, 0, 0])]
  offset[axis] = value
  anchor.offset = offset
  if (activeNozzle.value) activeNozzle.value.pendingApply = true
}

function updateDirection(axis: number, value: number) {
  const anchor = activeAnchor.value
  if (!anchor || axis < 0 || axis > 2) return
  const direction = [...(anchor.direction || [0, 0, -1])]
  direction[axis] = value
  if (Math.hypot(...direction) > 0.0001) anchor.direction = direction
  if (activeNozzle.value) activeNozzle.value.pendingApply = true
}

function updateNozzleInterval(event: Event) {
  const nozzle = activeNozzle.value
  if (!nozzle) return
  const input = event.target as HTMLInputElement
  const raw = input.value.trim()
  if (!raw) {
    nozzle.intervalSeconds = null
    nozzle.pendingApply = true
    return
  }
  const value = Number(raw)
  if (!Number.isFinite(value) || value < 0.5 || value > 300) {
    ElMessage.warning('喷射间隔必须在 0.5–300 秒之间；留空表示默认连续喷射')
    input.value = nozzle.intervalSeconds?.toString() || ''
    return
  }
  nozzle.intervalSeconds = Math.round(value * 1000) / 1000
  nozzle.pendingApply = true
}

function toggleGroup(groupId: string, visible: boolean) {
  hiddenGroups.value = visible
    ? hiddenGroups.value.filter(id => id !== groupId)
    : [...new Set([...hiddenGroups.value, groupId])]
}

function selectDrawGroup(groupId: string) {
  selectedGroup.value = groupId
  groupDetailsOpen.value = true
}

function handleVisibilityShortcut(event: KeyboardEvent) {
  if (event.defaultPrevented || event.ctrlKey || event.altKey || event.metaKey) return
  if (event.key.toLowerCase() !== 'h' || !selectedGroup.value) return
  const target = event.target
  if (target instanceof HTMLElement && target.matches('input, textarea, select, [contenteditable="true"]')) return
  event.preventDefault()
  const hidden = hiddenGroups.value.includes(selectedGroup.value)
  toggleGroup(selectedGroup.value, hidden)
}

async function rescanNativeWorkspace() {
  if (!info.value?.running || !info.value.apiToken) return
  nativeLoading.value = true
  nativeError.value = ''
  try {
    const response = await fetch(`${info.value.url.replace(/\/$/, '')}/api/rescan`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Editor-Token': info.value.apiToken },
      body: '{}',
    })
    const result = await response.json()
    if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`)
    catalog.value = result
    if (!catalog.value?.mods.some(model => model.id === selectedModelId.value)) {
      selectedModelId.value = catalog.value?.mods[0]?.id || ''
    }
    previewRevision.value += 1
  } catch (reason) {
    nativeError.value = String(reason)
  } finally {
    nativeLoading.value = false
  }
}

function completeNozzles() {
  if (!geometryInfo.value || !info.value?.apiToken) throw new Error('编辑器数据尚未加载')
  if (nozzles.value.some(nozzle => !nozzle.anchor)) throw new Error('请先为每个喷口选择表面')
  return nozzles.value.map((nozzle, slot) => ({
    ...nozzle.anchor!,
    slot,
    sourceProfile: nozzle.sourceProfile,
    intervalSeconds: nozzle.intervalSeconds ?? null,
  }))
}

async function saveNativeNozzles() {
  try {
    const result = await saveProbeNozzles(
      info.value!.url,
        info.value!.apiToken,
      geometryInfo.value!.id,
      completeNozzles(),
    )
    ElMessage.success(`已保存 ${result.nozzles} 个喷口草稿`)
  } catch (reason) {
    ElMessage.error(String(reason))
  }
}

async function applyNativeNozzles() {
  if (nativeLoading.value) return
  nativeLoading.value = true
  try {
    if (!window.confirm('确认将当前喷口应用到这个 Mod？应用后请回到游戏按 F10。')) return
    const result = await applyProbeNozzles(
      info.value!.url,
        info.value!.apiToken,
      geometryInfo.value!.id,
      completeNozzles().map(({ sourceProfile: _sourceProfile, ...anchor }) => anchor),
    )
    result.profiles.forEach((profile, index) => {
      if (nozzles.value[index]) {
        nozzles.value[index].sourceProfile = profile
        nozzles.value[index].pendingApply = false
      }
    })
    if (geometryInfo.value) {
      geometryInfo.value.integration = {
        configured: true,
        installed: true,
        hooks: result.profiles.length,
        expected: result.profiles.length,
      }
    }
    ElMessage.success(`已应用 ${result.nozzle_count} 个喷口`)
  } catch (reason) {
    ElMessage.error(String(reason))
  } finally {
    nativeLoading.value = false
  }
}

async function reinjectNativeModel() {
  if (nativeLoading.value || !info.value?.apiToken || !geometryInfo.value) return
  if (!integrationStatus.value?.configured) {
    ElMessage.warning('当前 Mod 还没有保存过喷口')
    return
  }
  nativeLoading.value = true
  try {
    await reinjectProbeModel(info.value.url, info.value.apiToken, geometryInfo.value.id)
    ElMessage.success('当前 Mod 的粒子钩子已重新注入；请按 F10')
    previewRevision.value += 1
  } catch (reason) {
    ElMessage.error(String(reason))
  } finally {
    nativeLoading.value = false
  }
}

async function removeNativeModelEffect() {
  if (nativeLoading.value || !info.value?.apiToken || !geometryInfo.value) return
  if (!window.confirm('确认移除当前 Mod 的全部粒子效果并恢复默认？')) return
  nativeLoading.value = true
  try {
    await removeProbeModelEffect(info.value.url, info.value.apiToken, geometryInfo.value.id)
    nozzles.value = [{ slot: 0, anchor: null, sourceProfile: null, intervalSeconds: null, pendingApply: false }]
    activeSlot.value = 0
    ElMessage.success('当前 Mod 的粒子效果已移除；请按 F10')
    previewRevision.value += 1
  } catch (reason) {
    ElMessage.error(String(reason))
  } finally {
    nativeLoading.value = false
  }
}

function keyTokenFromEvent(event: KeyboardEvent): string | null {
  const code = event.code || ''
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code)) return code
  if (/^Key[A-Z]$/.test(code)) return code.slice(3)
  if (/^Digit[0-9]$/.test(code)) return code.slice(5)
  if (/^Numpad[0-9]$/.test(code)) return `VK_NUMPAD${code.slice(6)}`
  const map: Record<string, string> = {
    ArrowLeft: 'VK_LEFT',
    ArrowRight: 'VK_RIGHT',
    ArrowUp: 'VK_UP',
    ArrowDown: 'VK_DOWN',
    Space: 'VK_SPACE',
    Enter: 'VK_RETURN',
    Tab: 'VK_TAB',
    Escape: 'VK_ESCAPE',
    Insert: 'VK_INSERT',
    Delete: 'VK_DELETE',
    Home: 'VK_HOME',
    End: 'VK_END',
    PageUp: 'VK_PRIOR',
    PageDown: 'VK_NEXT',
    NumpadAdd: 'VK_ADD',
    NumpadSubtract: 'VK_SUBTRACT',
    NumpadMultiply: 'VK_MULTIPLY',
    NumpadDivide: 'VK_DIVIDE',
    NumpadDecimal: 'VK_DECIMAL',
  }
  return map[code] || null
}

function keyDisplay(binding: ProbeKeyBinding) {
  const modifiers = [
    binding.ctrl ? 'Ctrl' : '',
    binding.alt ? 'Alt' : '',
    binding.shift ? 'Shift' : '',
  ].filter(Boolean)
  const label = binding.key.startsWith('VK_')
    ? binding.key.slice(3).replaceAll('_', ' ')
    : binding.key
  return [...modifiers, label].join(' + ')
}

let keyRecordHandler: ((event: KeyboardEvent) => void) | null = null
function stopKeyRecording() {
  if (keyRecordHandler) window.removeEventListener('keydown', keyRecordHandler, true)
  keyRecordHandler = null
  recordingKey.value = false
}

function startKeyRecording() {
  stopKeyRecording()
  recordingKey.value = true
  keyRecordHandler = event => {
    if (event.repeat) return
    const token = keyTokenFromEvent(event)
    if (!token || !keyBinding.value) return
    event.preventDefault()
    event.stopPropagation()
    keyBinding.value = {
      ...keyBinding.value,
      key: token,
      ctrl: event.ctrlKey,
      alt: event.altKey,
      shift: event.shiftKey,
      display: keyDisplay({ ...keyBinding.value, key: token, ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey }),
    }
    stopKeyRecording()
  }
  window.addEventListener('keydown', keyRecordHandler, true)
}

function updateKeyModifier(modifier: 'ctrl' | 'alt' | 'shift', checked: boolean) {
  if (!keyBinding.value) return
  keyBinding.value[modifier] = checked
  keyBinding.value.display = keyDisplay(keyBinding.value)
}

async function persistEffects(message: string) {
  try {
    effectData.value = await saveProbeEffects(
      info.value!.url,
        info.value!.apiToken,
      { ...effectValues.value },
    )
    effectValues.value = { ...effectData.value.values }
    ElMessage.success(message)
  } catch (reason) {
    ElMessage.error(String(reason))
  }
}

async function saveNativeEffects() {
  await persistEffects('效果参数已保存；修改粒子池后请完全重启游戏，其余参数可按 F10')
}

async function saveParticleColor(color: string) {
  if (!info.value?.apiToken || !geometryInfo.value) {
    ElMessage.error('请先加载一个 Mod')
    return
  }
  setParticleColor(color)
  try {
    const result = await saveProbeModelEffect(
      info.value.url,
      info.value.apiToken,
      geometryInfo.value.id,
      modelEffectColor.value || [0.78, 0.51, 0.055],
      modelEffectStrength.value,
    )
    modelEffectColor.value = [...result.color]
    savedModelEffectColor.value = [...result.color]
    ElMessage.success('当前 Mod 的粒子颜色已保存；改色按 F10 生效')
  } catch (reason) {
    if (savedModelEffectColor.value) {
      const previous = [...savedModelEffectColor.value]
      modelEffectColor.value = previous
      effectValues.value.particle_red = previous[0]
      effectValues.value.particle_green = previous[1]
      effectValues.value.particle_blue = previous[2]
    }
    ElMessage.error(String(reason))
  }
}

async function saveModelStrength() {
  if (!info.value?.apiToken || !geometryInfo.value) return
  const previous = savedModelEffectStrength.value
  try {
    const result = await saveProbeModelEffect(
      info.value.url,
      info.value.apiToken,
      geometryInfo.value.id,
      modelEffectColor.value || [0.78, 0.51, 0.055],
      modelEffectStrength.value,
    )
    modelEffectStrength.value = result.strength
    savedModelEffectStrength.value = result.strength
    ElMessage.success('当前 Mod 的喷射强度已保存；按 F10 生效')
  } catch (reason) {
    modelEffectStrength.value = previous
    ElMessage.error(String(reason))
  }
}

async function saveNativeKeyBinding() {
  if (!keyBinding.value) return
  try {
    keyBinding.value = await saveProbeKeyBinding(
      info.value!.url,
        info.value!.apiToken,
      keyBinding.value,
    )
    ElMessage.success('粒子效果开关键已保存，回到游戏按 F10 生效')
  } catch (reason) {
    ElMessage.error(String(reason))
  }
}

async function cleanupNativeProbeData() {
  if (!info.value?.apiToken) return
  try {
    const preview = await previewProbeCleanup(info.value!.url, info.value.apiToken)
    const total = preview.hooks + preview.profile_files + preview.generated_files
      + preview.delete_files + preview.delete_directories
    if (!total && !preview.legacy_profile_folder) {
      ElMessage.info('没有检测到需要清理的数据')
      return
    }
    const message = `将删除 ${preview.hooks} 个钩子、${preview.profile_files} 个角色配置、${preview.generated_files} 个生成文件。此操作不可恢复。`
    if (!window.confirm(message)) return
    await applyProbeCleanup(info.value!.url, info.value.apiToken)
    ElMessage.success('Liquid Gravity Probe 生成的数据已清理')
    await loadWorkspace()
  } catch (reason) {
    ElMessage.error(String(reason))
  }
}

async function startEditor() {
  loading.value = true
  error.value = ''
  try {
    if (!await refreshPluginState()) {
      ElMessage.warning('请先在插件市场安装并启用 Liquid Gravity Probe')
      return
    }
    info.value = await invoke<EditorInfo>('start_liquid_gravity_probe_editor', {
      gameName: currentGame.value,
      migotoRoot: await migotoRoot(),
    })
    ElMessage.success('Liquid Gravity Probe 编辑器已启动')
    await loadWorkspace()
  } catch (reason) {
    error.value = String(reason)
    ElMessage.error(error.value)
  } finally {
    loading.value = false
  }
}

async function stopEditor() {
  try {
    const stopped = await invoke<boolean>('stop_liquid_gravity_probe_editor')
    if (!stopped) ElMessage.info('当前编辑器不是由 SSMT 启动，未执行强制停止')
    await refreshStatus()
  } catch (reason) {
    ElMessage.error(String(reason))
  }
}

async function openEditorFolder() {
  if (!info.value) return
  await openPath(info.value.editorRoot).catch(reason => ElMessage.error(String(reason)))
}

async function openEditorLog() {
  if (!info.value?.logPath) return
  await openPath(info.value.logPath).catch(reason => ElMessage.error(String(reason)))
}

async function openPluginMarketplace() {
  await router.push({ name: 'PluginMarketplace', query: { plugin: pluginId } })
}

async function initializeWorkspace() {
  if (workspaceStarted) return
  workspaceStarted = true
  await refreshStatus()
  if (pluginReady.value && isEndfield.value && info.value && !info.value.running) {
    await startEditor()
  }
  window.addEventListener('keydown', handleVisibilityShortcut)
  pollTimer = window.setInterval(() => {
    if (!loading.value) void refreshStatus()
  }, 5000)
}

async function acceptDisclaimer() {
  if (disclaimerBusy.value) return
  disclaimerBusy.value = true
  try {
    await acceptLiquidGravityProbeConsent()
    if (isEndfield.value && currentGame.value) {
      await setLiquidGravityProbeEnabled(currentGame.value, true)
    }
    disclaimerOpen.value = false
    await initializeWorkspace()
  } catch (reason) {
    ElMessage.error(String(reason))
  } finally {
    disclaimerBusy.value = false
  }
}

async function declineDisclaimer() {
  if (disclaimerBusy.value) return
  disclaimerBusy.value = true
  try {
    await declineLiquidGravityProbeConsent()
    if (currentGame.value) {
      await setLiquidGravityProbeEnabled(currentGame.value, false)
    }
    disclaimerOpen.value = false
    await refreshStatus()
    ElMessage.info('已停用 Liquid Gravity Probe')
  } catch (reason) {
    ElMessage.error(String(reason))
  } finally {
    disclaimerBusy.value = false
  }
}

watch(
  () => [info.value?.running, info.value?.url],
  ([running]) => {
    if (running) void loadWorkspace()
  },
)
onMounted(async () => {
  if (embeddedMode.value) {
    await refreshStatus()
    await loadWorkspace()
    window.addEventListener('keydown', handleVisibilityShortcut)
    return
  }
  if (isEndfield.value && !await hasLiquidGravityProbeConsent()) {
    pluginChecked.value = true
    disclaimerOpen.value = true
    return
  }
  await initializeWorkspace()
})

onBeforeUnmount(() => {
  stopKeyRecording()
  window.removeEventListener('keydown', handleVisibilityShortcut)
  if (pollTimer !== undefined) window.clearInterval(pollTimer)
})
</script>

<template>
  <main class="lgp-page">
    <LiquidGravityProbeDisclaimer
      :open="disclaimerOpen"
      :busy="disclaimerBusy"
      @accept="acceptDisclaimer"
      @decline="declineDisclaimer"
    />

    <header class="lgp-header">
      <div class="lgp-brand-block">
        <span class="lgp-brand-mark"><span /></span>
        <div>
          <p class="lgp-eyebrow">EFMI / 3DMIGOTO PLUGIN</p>
          <h1>Liquid Gravity Probe</h1>
          <p class="lgp-subtitle">粒子喷口、独立颜色、重新注入与性能参数管理器</p>
        </div>
      </div>
      <div v-if="!embeddedMode" class="lgp-actions">
        <button type="button" class="lgp-button" :disabled="loading" @click="refreshStatus">
          <Refresh /> 重新检测
        </button>
        <button type="button" class="lgp-button primary" :disabled="loading" @click="startEditor">
          <VideoPlay /> {{ info?.running ? '编辑器已连接' : '启动编辑器' }}
        </button>
      </div>
    </header>

    <section class="lgp-status">
      <div class="lgp-status-copy">
        <span class="lgp-dot" :class="{ online: info?.running, error: !!error }" />
        <div>
          <strong>{{ statusTitle }}</strong>
          <p v-if="error">{{ error }}</p>
          <div v-else-if="info && !embeddedMode" class="lgp-status-paths">
            <span class="lgp-status-label">编辑器</span>
            <code :title="info.editorRoot">{{ info.editorRoot }}</code>
            <span class="lgp-status-label">数据目录</span>
            <code :title="info.dataRoot">{{ info.dataRoot }}</code>
          </div>
          <p v-else-if="embeddedMode">插件页面已连接</p>
          <p v-else-if="!pluginReady">安装并启用插件后才会加载编辑器。</p>
          <p v-else>正在读取插件编辑器…</p>
        </div>
      </div>
      <div class="lgp-chip">{{ isEndfield ? 'EFMI' : currentGame || '未选择游戏' }}</div>
      <div v-if="!embeddedMode" class="lgp-status-actions">
        <button type="button" class="lgp-icon-button" title="打开编辑器目录" :disabled="!info" @click="openEditorFolder"><FolderOpened /></button>
        <button type="button" class="lgp-icon-button" title="打开编辑器日志" :disabled="!info?.logPath" @click="openEditorLog"><Document /></button>
        <button type="button" class="lgp-icon-button" title="停止 SSMT 启动的编辑器" :disabled="!info?.running" @click="stopEditor"><Close /></button>
      </div>
    </section>

    <section v-if="pluginReady && info?.running" class="lgp-native-workspace">
      <aside class="lgp-native-sidebar">
        <div class="lgp-panel-heading">
          <div>
            <strong>Mod 模型</strong>
            <small>{{ catalog?.mods.length || 0 }} 个可编辑模型</small>
          </div>
          <button type="button" class="lgp-icon-button" title="重新扫描文件" @click="rescanNativeWorkspace"><Refresh /></button>
        </div>
        <input v-model="modelSearch" class="lgp-search" type="search" placeholder="搜索 Mod">
        <div class="lgp-model-list">
          <button
            v-for="model in filteredModels"
            :key="model.id"
            type="button"
            class="lgp-model-item"
            :class="{ active: model.id === selectedModelId }"
            @click="selectNativeModel(model)"
          >
            <strong>{{ model.name }}</strong>
            <span>{{ model.groups }} 个绘制项</span>
          </button>
          <p v-if="!nativeLoading && !filteredModels.length" class="lgp-panel-empty">没有可编辑模型</p>
        </div>
      </aside>

      <section class="lgp-native-preview">
        <div class="lgp-preview-toolbar">
          <div>
            <strong>{{ selectedModel?.name || '选择模型' }}</strong>
            <span v-if="geometryInfo">{{ geometryInfo.meshes.length }} 个网格 · {{ geometryInfo.groups.length }} 个绘制项</span>
          </div>
          <span v-if="nativeLoading" class="lgp-inline-loading">加载中…</span>
        </div>
        <ProbeViewport3D
          v-if="selectedModelId"
          :key="`${selectedModelId}:${previewRevision}`"
          :server-url="info.url"
          :api-token="info.apiToken"
          :model-id="selectedModelId"
          :nozzles="nozzles"
          :active-slot="activeSlot"
          :selected-group="selectedGroup"
          :hidden-groups="hiddenGroups"
          :flow-preview="flowPreviewEnabled"
          @picked="onNativePicked"
          @loaded="onGeometryLoaded"
          @group-selected="selectedGroup = $event"
        />
        <div v-else class="lgp-native-placeholder">
          <strong>{{ nativeError || '请选择一个 Mod 模型' }}</strong>
          <p v-if="nativeError">{{ nativeError }}</p>
        </div>
      </section>

      <aside class="lgp-native-sidebar lgp-nozzle-sidebar">
        <div class="lgp-detail-tabs">
          <button type="button" :class="{ active: panelTab === 'nozzle' }" @click="panelTab = 'nozzle'">喷口</button>
          <button type="button" :class="{ active: panelTab === 'effects' }" @click="panelTab = 'effects'">效果</button>
          <button type="button" :class="{ active: panelTab === 'key' }" @click="panelTab = 'key'">按键</button>
        </div>

        <div v-if="panelTab === 'nozzle'" class="lgp-panel-scroll">
          <div class="lgp-panel-heading">
            <div>
              <strong>喷口</strong>
              <small>{{ nozzles.length }}/4</small>
            </div>
            <div class="lgp-panel-actions">
              <button type="button" class="lgp-button compact" :disabled="nozzles.length >= 4" @click="addNativeNozzle">添加</button>
              <button type="button" class="lgp-button compact" :disabled="nativeLoading" @click="removeNativeNozzle">删除</button>
            </div>
          </div>

          <div class="lgp-model-integration">
            <span :class="{ online: integrationStatus?.installed, warning: integrationStatus?.configured && !integrationStatus?.installed }">
              {{ integrationStatus?.configured
                ? (integrationStatus?.installed ? '粒子钩子已连接' : `需要重新注入（${integrationStatus?.hooks}/${integrationStatus?.expected}）`)
                : '当前 Mod 尚未注入' }}
            </span>
            <button type="button" class="lgp-button compact" :disabled="nativeLoading || !integrationStatus?.configured" @click="reinjectNativeModel">重新注入</button>
            <button type="button" class="lgp-button compact danger" :disabled="nativeLoading || !integrationStatus?.configured" @click="removeNativeModelEffect">恢复默认</button>
          </div>

          <div class="lgp-nozzle-list">
            <button
              v-for="nozzle in nozzles"
              :key="nozzle.slot"
              type="button"
              class="lgp-nozzle-item"
              :class="{ active: nozzle.slot === activeSlot }"
              @click="selectNativeNozzle(nozzle.slot)"
            >
              <strong>{{ nozzle.slot + 1 }}</strong>
              <span>{{ nozzle.anchor ? '已绑定表面' : '点击模型选择表面' }}</span>
              <small>{{ nozzle.intervalSeconds != null ? `${nozzle.intervalSeconds}s/次` : nozzle.sourceProfile != null ? `#${nozzle.sourceProfile}` : nozzle.anchor ? '草稿' : '新建' }}</small>
            </button>
          </div>

          <div class="lgp-control-group">
            <div class="lgp-control-heading">喷射间隔</div>
            <label class="lgp-interval-row">
              <span>每隔</span>
              <input
                type="number"
                min="0.5"
                max="300"
                step="0.1"
                placeholder="默认连续"
                :value="activeNozzle?.intervalSeconds ?? ''"
                @change="updateNozzleInterval"
              >
              <span>秒喷一次</span>
            </label>
            <p class="lgp-control-hint">留空时保持当前连续喷射；填写后只影响当前喷口。</p>
          </div>

          <template v-if="activeAnchor">
            <div class="lgp-control-group">
              <label class="lgp-toggle-row">
                <span>流体粒子预览</span>
                <input v-model="flowPreviewEnabled" type="checkbox">
              </label>
            </div>
            <div class="lgp-control-group">
              <div class="lgp-control-heading">沿表面偏移 mm</div>
              <label v-for="(axis, index) in ['X', 'Y', 'Z']" :key="axis" class="lgp-slider-row">
                <span>{{ axis }}</span>
                <input
                  type="range"
                  min="-250"
                  max="250"
                  step="0.5"
                  :value="(activeAnchor.offset?.[index] || 0) * 1000"
                  @input="updateOffset(index, Number(($event.target as HTMLInputElement).value) / 1000)"
                >
                <input
                  type="number"
                  min="-250"
                  max="250"
                  step="0.5"
                  :value="((activeAnchor.offset?.[index] || 0) * 1000).toFixed(1)"
                  @change="updateOffset(index, Number(($event.target as HTMLInputElement).value) / 1000)"
                >
              </label>
            </div>

            <div class="lgp-control-group">
              <div class="lgp-control-heading">喷射方向</div>
              <div class="lgp-vector-row">
                <label v-for="(axis, index) in ['X', 'Y', 'Z']" :key="axis">
                  <span>{{ axis }}</span>
                  <input
                    type="number"
                    step="0.1"
                    :value="activeAnchor.direction?.[index] ?? (index === 2 ? -1 : 0)"
                    @change="updateDirection(index, Number(($event.target as HTMLInputElement).value))"
                  >
                </label>
              </div>
            </div>

            <details
              class="lgp-group-visibility"
              :open="groupDetailsOpen"
              @toggle="groupDetailsOpen = ($event.target as HTMLDetailsElement).open"
            >
              <summary>绘制部件显示</summary>
              <label
                v-for="group in geometryInfo?.groups || []"
                :key="group.id"
                :data-probe-group="group.id"
                :class="{ selected: selectedGroup === group.id }"
                @click="selectDrawGroup(group.id)"
              >
                <input
                  type="checkbox"
                  :checked="!hiddenGroups.includes(group.id)"
                  @change="toggleGroup(group.id, ($event.target as HTMLInputElement).checked)"
                >
                <span>{{ group.label }}</span>
              </label>
            </details>
          </template>
          <p v-else class="lgp-panel-empty">在当前喷口启用选取后，单击模型表面。</p>

          <div class="lgp-sidebar-actions">
            <button type="button" class="lgp-button" :disabled="!activeAnchor" @click="saveNativeNozzles">保存草稿</button>
            <button type="button" class="lgp-button primary" :disabled="nozzles.some(nozzle => !nozzle.anchor)" @click="applyNativeNozzles">应用到游戏</button>
          </div>
        </div>

        <div v-else-if="panelTab === 'effects'" class="lgp-panel-scroll">
          <div class="lgp-control-group">
            <div class="lgp-control-heading">当前 Mod 喷射强度</div>
            <label class="lgp-slider-row">
              <span>强度</span>
              <input
                v-model.number="modelEffectStrength"
                type="range"
                min="0.2"
                max="2"
                step="0.05"
                @change="saveModelStrength"
              >
              <input
                v-model.number="modelEffectStrength"
                type="number"
                min="0.2"
                max="2"
                step="0.05"
                @change="saveModelStrength"
              >
            </label>
          </div>
          <section class="lgp-color-panel">
            <div class="lgp-control-heading">粒子调色盘</div>
            <div class="lgp-color-row">
              <input
                class="lgp-color-input"
                type="color"
                :value="particleColorValue"
                title="自定义粒子颜色"
                @input="setParticleColor(($event.target as HTMLInputElement).value)"
                @change="saveParticleColor(($event.target as HTMLInputElement).value)"
              >
              <div class="lgp-color-swatches">
                <button
                  v-for="swatch in particlePalette"
                  :key="swatch.color"
                  type="button"
                  class="lgp-color-swatch"
                  :class="{ active: particleColorValue.toLowerCase() === swatch.color }"
                  :style="{ background: swatch.color }"
                  :title="swatch.label"
                  :aria-label="swatch.label"
                  @click="saveParticleColor(swatch.color)"
                />
              </div>
            </div>
            <div class="lgp-color-current">当前：{{ currentParticleColorLabel }}</div>
          </section>
          <section v-for="group in visibleEffectGroups" :key="group.name" class="lgp-effect-group">
            <h4>{{ group.name }}</h4>
            <label
              v-for="field in group.fields"
              :key="field.key"
              class="lgp-effect-field"
              :class="{ 'has-select': field.kind === 'select' }"
            >
              <span>{{ field.label }}</span>
              <input
                v-if="field.kind === 'toggle'"
                type="checkbox"
                :checked="Number(effectValues[field.key]) > 0.5"
                @change="effectValues[field.key] = ($event.target as HTMLInputElement).checked ? 1 : 0"
              >
              <div v-else-if="field.kind === 'select'" class="lgp-select-control">
                <button
                  type="button"
                  class="lgp-select-trigger"
                  :aria-expanded="openSelectKey === field.key"
                  @click="openSelectKey = openSelectKey === field.key ? null : field.key"
                >
                  <span>{{ selectOptionLabel(field) }}</span>
                  <ArrowDown />
                </button>
                <div v-if="openSelectKey === field.key" class="lgp-select-menu">
                  <button
                    v-for="option in field.options || []"
                    :key="option.value"
                    type="button"
                    :class="{ active: Number(effectValues[field.key]) === Number(option.value) }"
                    @click="chooseEffectOption(field, option.value)"
                  >
                    {{ option.label }}
                  </button>
                </div>
              </div>
              <input
                v-else
                v-model.number="effectValues[field.key]"
                type="number"
                :min="field.min"
                :max="field.max"
                :step="field.step"
              >
            </label>
          </section>
          <div class="lgp-sidebar-actions">
            <button type="button" class="lgp-button primary" :disabled="!effectData" @click="saveNativeEffects">保存效果参数</button>
            <button type="button" class="lgp-button danger" @click="cleanupNativeProbeData">清理生成数据</button>
          </div>
        </div>

        <div v-else class="lgp-panel-scroll">
          <div class="lgp-keybind-panel">
            <label>
              <span>粒子效果开关键</span>
              <input :value="recordingKey ? '请按下按键…' : keyBinding?.display || ''" readonly>
            </label>
            <button type="button" class="lgp-button" @click="startKeyRecording">{{ recordingKey ? '等待按键…' : '录制按键' }}</button>
            <div class="lgp-key-modifiers">
              <label><input type="checkbox" :checked="keyBinding?.ctrl" @change="updateKeyModifier('ctrl', ($event.target as HTMLInputElement).checked)"> Ctrl</label>
              <label><input type="checkbox" :checked="keyBinding?.alt" @change="updateKeyModifier('alt', ($event.target as HTMLInputElement).checked)"> Alt</label>
              <label><input type="checkbox" :checked="keyBinding?.shift" @change="updateKeyModifier('shift', ($event.target as HTMLInputElement).checked)"> Shift</label>
            </div>
            <button type="button" class="lgp-button primary" :disabled="!keyBinding" @click="saveNativeKeyBinding">保存按键</button>
          </div>
        </div>
      </aside>
    </section>

    <section v-else-if="pluginReady" class="lgp-empty">
      <h2>等待本地编辑器</h2>
      <p>编辑器源码由插件提供，运行需要 Python 3.12 或更高版本；请在插件市场的“外部依赖”中选择包含 <code>python.exe</code> 的安装目录。</p>
      <p>喷口、效果和按键配置保存在 <code>Mods/SSMT_LiquidGravityProbe</code>；<code>Mods</code> 同时用于扫描用户要编辑的模型。</p>
      <p v-if="!isEndfield">当前选择的游戏是 {{ currentGame || '未选择' }}，请切换到 EFMI。</p>
      <button type="button" class="lgp-button primary lgp-start-button" :disabled="loading" @click="startEditor">
        <VideoPlay /> {{ loading ? '正在启动…' : '启动编辑器' }}
      </button>
    </section>

    <section v-else class="lgp-empty">
      <h2>{{ isEndfield ? 'Liquid Gravity Probe 插件未启用' : '需要 EFMI 游戏配置' }}</h2>
      <p v-if="isEndfield">先安装 <code>{{ pluginId }}</code>，并在插件市场为 EFMI 启用它。启用后此页面才会启动本地 Python 编辑器。</p>
      <p v-else>请切换到 EFMI，并为 EFMI 启用 Liquid Gravity Probe。</p>
      <button type="button" class="lgp-button primary" @click="openPluginMarketplace">打开插件市场</button>
    </section>
  </main>
</template>

<style scoped>
.lgp-page { --lgp-text:#edf4f6; --lgp-muted:#bccbd1; --lgp-subtle:#94a6ae; --lgp-accent:rgb(var(--ssmt-brand-accent-rgb,216,233,78)); --lgp-accent-ink:#11180b; display:flex; flex-direction:column; height:100%; min-height:0; box-sizing:border-box; overflow:hidden; padding:18px 20px; color:var(--lgp-text); font-size:12px; line-height:1.45; background:linear-gradient(180deg, rgba(8,13,17,.97), rgba(8,13,17,.93)); }
.lgp-header { display:flex; flex:0 0 auto; justify-content:space-between; align-items:flex-end; gap:20px; margin-bottom:12px; }
.lgp-eyebrow { margin: 0 0 5px; color: var(--lgp-muted); font-size: 11px; letter-spacing: 0; }
.lgp-header h1 { margin: 0; font-size: 24px; letter-spacing: 0; }
.lgp-subtitle { margin: 7px 0 0; color: var(--lgp-muted); font-size: 12px; }
.lgp-actions, .lgp-status-actions, .lgp-frame-toolbar { display: flex; align-items: center; gap: 7px; }
.lgp-button, .lgp-icon-button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; border: 1px solid var(--t-border); border-radius: 7px; background: var(--t-surface-soft); color: var(--lgp-text); padding: 8px 11px; font: inherit; font-size: 12px; cursor: pointer; }
.lgp-button svg, .lgp-icon-button svg { width: 15px; height: 15px; }
.lgp-button:hover, .lgp-icon-button:hover { border-color: var(--lgp-accent); background: var(--t-surface-hover); }
.lgp-button.primary { background:var(--lgp-accent); border-color:var(--lgp-accent); color:var(--lgp-accent-ink); font-weight:700; }
.lgp-button.primary:hover { border-color:#edf8b0; background:#e4f486; }
.lgp-button.compact { padding: 5px 8px; font-size: 11px; }
.lgp-button:disabled, .lgp-icon-button:disabled { opacity: .72; cursor: not-allowed; }
.lgp-status { display:flex; flex:0 0 auto; align-items:center; gap:10px; padding:9px 11px; margin-bottom:10px; border:1px solid var(--t-border); border-radius:8px; background:var(--t-surface-soft); }
.lgp-status-copy { display: flex; align-items: center; gap: 9px; min-width: 0; flex: 1; }
.lgp-status-copy strong { display: block; font-size: 13px; }
.lgp-status-copy p { margin: 3px 0 0; color: var(--lgp-muted); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.lgp-status-paths { display: grid; grid-template-columns: 56px minmax(0, 1fr); align-items: center; gap: 3px 8px; min-width: 0; margin-top: 4px; color: var(--lgp-muted); font-size: 11px; }
.lgp-status-label { text-align: right; white-space: nowrap; }
.lgp-status-paths code { min-width: 0; overflow: hidden; color: var(--lgp-muted); font: inherit; text-overflow: ellipsis; white-space: nowrap; }
.lgp-dot { width: 8px; height: 8px; flex: 0 0 auto; border-radius: 50%; background: #8d979c; }
.lgp-dot.online { background: #5fd1ad; box-shadow: 0 0 9px #5fd1ad99; }
.lgp-dot.error { background: #ef8c84; }
.lgp-chip { padding: 4px 7px; border: 1px solid var(--t-border); border-radius: 999px; color: var(--lgp-muted); font-size: 11px; }
.lgp-icon-button { width: 30px; height: 30px; padding: 0; }
.lgp-native-workspace { display:grid; flex:1; grid-template-columns:220px minmax(0,1fr) 330px; height:auto; min-height:0; overflow:hidden; border:1px solid var(--t-border); border-radius:8px; background:#0b1116; }
.lgp-native-sidebar { display:flex; flex-direction:column; min-width:0; min-height:0; padding:12px; box-sizing:border-box; background:rgba(10,16,20,.86); }
.lgp-native-sidebar:first-child { border-right:1px solid var(--t-border); }
.lgp-nozzle-sidebar { border-left:1px solid var(--t-border); overflow:hidden; }
.lgp-detail-tabs { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:3px; padding:3px; margin-bottom:10px; border:1px solid var(--t-border); border-radius:7px; background:rgba(255,255,255,.025); }
.lgp-detail-tabs button { height:32px; border:0; border-radius:5px; background:transparent; color:var(--lgp-muted); font:inherit; font-size:12px; cursor:pointer; }
.lgp-detail-tabs button.active { background:var(--t-surface-raised); color:var(--lgp-text); font-weight:650; }
.lgp-panel-scroll { display:flex; flex:1; flex-direction:column; min-height:0; overflow-y:auto; padding-right:2px; }
.lgp-panel-heading { display:flex; align-items:center; justify-content:space-between; gap:8px; min-height:34px; }
.lgp-panel-heading strong { display:block; color:var(--lgp-text); font-size:13px; }
.lgp-panel-heading small { display:block; margin-top:2px; color:var(--lgp-muted); font-size:11px; }
.lgp-panel-actions { display:flex; gap:5px; }
.lgp-model-integration { display:grid; grid-template-columns:minmax(0,1fr) auto auto; align-items:center; gap:6px; margin:5px 0 8px; padding:6px 7px; border:1px solid var(--lgp-border); border-radius:6px; background:var(--lgp-surface-soft); }
.lgp-model-integration span { min-width:0; overflow:hidden; color:var(--lgp-muted); font-size:11px; text-overflow:ellipsis; white-space:nowrap; }
.lgp-model-integration span.online { color:#80d99b; }
.lgp-model-integration span.warning { color:#e8b85d; }
.lgp-search { width:100%; height:32px; box-sizing:border-box; padding:0 9px; margin:9px 0; border:1px solid var(--t-border); border-radius:6px; outline:none; background:rgba(255,255,255,.04); color:var(--lgp-text); font:inherit; font-size:12px; }
.lgp-search:focus { border-color:var(--lgp-accent); }
.lgp-model-list, .lgp-nozzle-list { display:grid; align-content:start; gap:6px; min-height:0; overflow-y:auto; }
.lgp-model-list { flex:1; }
.lgp-model-item, .lgp-nozzle-item { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:3px 8px; width:100%; padding:9px; border:1px solid transparent; border-radius:7px; background:rgba(255,255,255,.035); color:var(--lgp-text); text-align:left; cursor:pointer; }
.lgp-model-item strong, .lgp-nozzle-item span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.lgp-model-item strong { font-size:12px; }
.lgp-model-item span, .lgp-nozzle-item small { color:var(--lgp-muted); font-size:11px; }
.lgp-model-item.active, .lgp-nozzle-item.active { border-color:rgba(95,209,173,.42); background:rgba(95,209,173,.10); }
.lgp-nozzle-item { grid-template-columns:22px minmax(0,1fr) auto; align-items:center; }
.lgp-nozzle-item strong { display:grid; width:22px; height:22px; place-items:center; border-radius:50%; background:rgba(95,209,173,.13); color:#8ae1c4; font-size:10px; }
.lgp-nozzle-item span { font-size:12px; }
.lgp-native-preview { position:relative; display:flex; flex-direction:column; min-width:0; min-height:0; padding:10px; box-sizing:border-box; }
.lgp-preview-toolbar { display:flex; align-items:center; justify-content:space-between; gap:10px; min-height:38px; padding:0 2px 8px; }
.lgp-preview-toolbar strong { display:block; color:var(--lgp-text); font-size:13px; }
.lgp-preview-toolbar span { display:block; margin-top:2px; color:var(--lgp-muted); font-size:11px; }
.lgp-inline-loading { color:#79d6ba !important; }
.lgp-native-preview :deep(.probe-viewport) { flex:1; min-height:0; }
.lgp-native-placeholder { display:grid; flex:1; place-content:center; justify-items:center; gap:8px; border:1px dashed var(--t-border); border-radius:8px; color:var(--lgp-muted); text-align:center; }
.lgp-native-placeholder strong { color:var(--lgp-text); font-size:14px; }
.lgp-native-placeholder p { margin:0; font-size:11px; }
.lgp-control-group { display:grid; gap:7px; padding:11px 0; border-top:1px solid rgba(255,255,255,.07); }
.lgp-control-heading { color:var(--lgp-muted); font-size:11px; font-weight:650; letter-spacing:0; text-transform:none; }
.lgp-toggle-row { display:flex; align-items:center; justify-content:space-between; gap:8px; color:var(--lgp-text); font-size:12px; }
.lgp-toggle-row input { accent-color:var(--lgp-accent); }
.lgp-slider-row { display:grid; grid-template-columns:20px minmax(0,1fr) 64px; align-items:center; gap:7px; color:var(--lgp-muted); font-size:11px; }
.lgp-slider-row input[type='range'] { min-width:0; accent-color:var(--lgp-accent); }
.lgp-slider-row input[type='number'], .lgp-vector-row input, .lgp-interval-row input { width:100%; height:28px; box-sizing:border-box; padding:0 6px; border:1px solid var(--t-border); border-radius:5px; background:rgba(255,255,255,.04); color:var(--lgp-text); font:inherit; font-size:11px; }
.lgp-interval-row { display:grid; grid-template-columns:auto minmax(0,1fr) auto; align-items:center; gap:7px; color:var(--lgp-muted); font-size:11px; }
.lgp-interval-row input::placeholder { color:var(--lgp-subtle); }
.lgp-control-hint { margin:0; color:var(--lgp-muted); font-size:10px; line-height:1.5; }
.lgp-vector-row { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:6px; }
.lgp-vector-row label { display:grid; gap:4px; color:var(--lgp-muted); font-size:11px; }
.lgp-group-visibility { border-top:1px solid rgba(255,255,255,.07); padding-top:10px; color:var(--lgp-muted); font-size:12px; }
.lgp-group-visibility summary { margin-bottom:8px; color:var(--lgp-text); font-size:12px; font-weight:650; cursor:pointer; }
.lgp-group-visibility label { display:grid; grid-template-columns:16px minmax(0,1fr); align-items:center; gap:7px; min-height:28px; padding:4px 6px; border-left:2px solid transparent; border-radius:4px; cursor:pointer; }
.lgp-group-visibility label:hover { background:rgba(255,255,255,.035); }
.lgp-group-visibility label.selected { border-left-color:#5fd1ad; background:rgba(95,209,173,.13); color:var(--lgp-text); }
.lgp-group-visibility label.selected span { font-weight:600; }
.lgp-group-visibility span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.lgp-panel-empty { margin:14px 0; color:var(--lgp-muted); font-size:11px; line-height:1.6; }
.lgp-sidebar-actions { position:sticky; bottom:0; z-index:2; display:grid; gap:7px; margin-top:auto; padding:12px 0 2px; background:linear-gradient(180deg, rgba(10,16,20,0), rgba(10,16,20,.98) 30%); }
.lgp-color-panel { display:grid; gap:9px; padding:2px 0 12px; border-bottom:1px solid rgba(255,255,255,.07); }
.lgp-color-row { display:flex; align-items:center; gap:10px; }
.lgp-color-input { width:48px; height:32px; flex:0 0 auto; padding:2px; border:1px solid var(--t-border); border-radius:6px; background:#172126; cursor:pointer; }
.lgp-color-swatches { display:flex; flex-wrap:wrap; gap:7px; }
.lgp-color-swatch { width:24px; height:24px; padding:0; border:2px solid transparent; border-radius:50%; box-shadow:0 0 0 1px rgba(255,255,255,.28); cursor:pointer; }
.lgp-color-swatch:hover { transform:scale(1.08); }
.lgp-color-swatch.active { border-color:#f7ffd0; box-shadow:0 0 0 2px var(--lgp-accent); }
.lgp-color-current { color:var(--lgp-muted); font-size:11px; }
.lgp-effect-group { display:grid; gap:6px; padding:9px 0; border-top:1px solid rgba(255,255,255,.07); }
.lgp-effect-group:first-of-type { border-top:0; }
.lgp-effect-group h4 { margin:0 0 3px; color:var(--lgp-text); font-size:12px; font-weight:650; }
.lgp-effect-field { display:grid; grid-template-columns:minmax(0,1fr) 88px; align-items:center; gap:9px; min-height:28px; color:var(--lgp-muted); font-size:11px; }
.lgp-effect-field > span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.lgp-effect-field input[type='number'], .lgp-effect-field select, .lgp-keybind-panel input:not([type='checkbox']) { width:100%; height:28px; box-sizing:border-box; padding:0 6px; border:1px solid var(--t-border); border-radius:5px; background:rgba(255,255,255,.04); color:var(--lgp-text); font:inherit; font-size:11px; }
.lgp-effect-field select { color-scheme:dark; background:#172126; color:var(--lgp-text); }
.lgp-effect-field select option { background:#172126; color:var(--lgp-text); }
.lgp-effect-field input[type='checkbox'], .lgp-key-modifiers input { accent-color:var(--lgp-accent); }
.lgp-keybind-panel { display:grid; gap:10px; padding:8px 0; }
.lgp-keybind-panel > label { display:grid; gap:6px; color:var(--lgp-muted); font-size:11px; }
.lgp-key-modifiers { display:flex; flex-wrap:wrap; gap:10px; color:var(--lgp-muted); font-size:11px; }
.lgp-key-modifiers label { display:flex; align-items:center; gap:4px; }
.lgp-button.danger { border-color:rgba(239,140,132,.28); color:#efaaa4; }
.lgp-button.danger:hover { border-color:rgba(239,140,132,.55); background:rgba(239,140,132,.08); }
.lgp-frame-shell { position: relative; height: calc(100vh - 180px); min-height: 500px; display: flex; flex-direction: column; border: 1px solid var(--t-border); border-radius: 8px; overflow: hidden; background: #101619; }
.lgp-frame-toolbar { justify-content: space-between; min-height: 38px; padding: 0 9px 0 12px; border-bottom: 1px solid var(--t-border); color: var(--t-page-text-muted); font: 10px ui-monospace, Consolas, monospace; }
.lgp-frame { flex: 1; width: 100%; border: 0; background: #101619; }
.lgp-frame-overlay { position: absolute; inset: 39px 0 0; display: grid; place-content: center; justify-items: center; gap: 10px; padding: 28px; background: rgba(9, 14, 18, .96); color: var(--t-page-text); text-align: center; }
.lgp-frame-overlay strong { font-size: 13px; }
.lgp-frame-overlay p { margin: 0; color: var(--t-page-text-muted); font-size: 11px; }
.lgp-spinner { width: 22px; height: 22px; border: 2px solid rgba(255, 255, 255, .16); border-top-color: var(--lgp-accent); border-radius: 50%; animation: lgp-spin .8s linear infinite; }
.lgp-spinner.error { border-top-color: #ef8c84; animation: none; }
@keyframes lgp-spin { to { transform: rotate(360deg); } }
.lgp-empty { max-width: 720px; padding: 34px; border: 1px dashed var(--t-border); border-radius: 8px; color: var(--t-page-text-muted); }
.lgp-empty h2 { margin: 0 0 10px; color: var(--t-page-text); font-size: 16px; }
.lgp-empty p { font-size: 12px; line-height: 1.75; }
.lgp-empty code { color: var(--t-page-text); }
.lgp-start-button { width:max-content; min-width:150px; margin-top:8px; padding:10px 18px; font-size:13px; }
@media (max-width: 1050px) { .lgp-native-workspace { grid-template-columns:180px minmax(0,1fr) 300px; } }
@media (max-width: 820px) { .lgp-native-workspace { grid-template-columns:1fr; height:auto; min-height:0; } .lgp-native-sidebar:first-child { border-right:0; border-bottom:1px solid var(--t-border); } .lgp-native-preview { min-height:55vh; } .lgp-nozzle-sidebar { border-left:0; border-top:1px solid var(--t-border); } }
@media (max-height: 740px) and (min-width: 821px) { .lgp-page { padding:10px 16px; } .lgp-header { align-items:center; margin-bottom:7px; } .lgp-eyebrow { display:none; } .lgp-header h1 { font-size:20px; } .lgp-subtitle { margin-top:3px; font-size:11px; } .lgp-status { padding:6px 9px; margin-bottom:7px; } .lgp-status-paths { grid-template-columns:50px minmax(0,1fr); gap:1px 7px; } .lgp-native-sidebar { padding:8px; } .lgp-preview-toolbar { min-height:31px; padding-bottom:5px; } }
@media (max-width: 760px) { .lgp-page { padding:14px; overflow:auto; } .lgp-header { align-items:flex-start; flex-direction:column; } }

/* Unified first-party manager surface */
.lgp-page {
  --lgp-border:#26353b;
  --lgp-surface:#0d1519;
  --lgp-surface-raised:#111b20;
  --lgp-surface-soft:#0a1114;
  gap:0;
  padding:14px 16px 16px;
  background:
    linear-gradient(rgba(104,134,143,.035) 1px,transparent 1px),
    linear-gradient(90deg,rgba(104,134,143,.035) 1px,transparent 1px),
    #070b0d;
  background-size:38px 38px;
}
.lgp-header { align-items:center; min-height:58px; margin-bottom:10px; padding:0 2px; }
.lgp-brand-block { display:flex; align-items:center; gap:13px; min-width:0; }
.lgp-brand-mark { position:relative; display:grid; flex:0 0 42px; width:42px; height:42px; place-items:center; border:1px solid rgba(216,233,78,.38); border-radius:50% 50% 50% 9px; transform:rotate(-45deg); background:#10181c; }
.lgp-brand-mark::before { content:""; width:18px; height:18px; border-radius:50% 50% 50% 6px; background:var(--lgp-accent); box-shadow:0 0 14px rgba(216,233,78,.18); }
.lgp-brand-mark span { position:absolute; top:9px; left:12px; width:6px; height:6px; border-radius:50%; background:#f6ffc3; opacity:.9; }
.lgp-eyebrow { margin:0 0 3px; color:#93a74a; font-size:9px; font-weight:700; letter-spacing:.08em; }
.lgp-header h1 { margin:0; color:#f1f7f8; font-size:20px; font-weight:650; line-height:1.15; }
.lgp-subtitle { margin:4px 0 0; color:#84969c; font-size:11px; }
.lgp-actions { gap:6px; }
.lgp-button, .lgp-icon-button { border-color:var(--lgp-border); border-radius:6px; background:#111a1f; box-shadow:none; }
.lgp-button:hover, .lgp-icon-button:hover { border-color:#6e7e3d; background:#172126; }
.lgp-button.primary { border-color:var(--lgp-accent); background:var(--lgp-accent); color:#11180b; }
.lgp-button.primary:hover { background:#e5f56f; border-color:#e5f56f; }
.lgp-icon-button { width:29px; height:29px; }
.lgp-status { min-height:46px; gap:9px; padding:7px 10px; margin-bottom:9px; border-color:var(--lgp-border); border-radius:6px; background:#0d1519; }
.lgp-status-copy strong { color:#dce8ea; font-size:12px; }
.lgp-status-copy p { margin-top:2px; color:#7f9298; font-size:10px; }
.lgp-status-paths { margin-top:2px; }
.lgp-chip { border-color:#33454b; background:#111b1f; color:#9fb0b5; font:10px ui-monospace,Consolas,monospace; }
.lgp-dot.online { background:#6ed7a4; box-shadow:0 0 10px rgba(110,215,164,.45); }
.lgp-native-workspace { grid-template-columns:220px minmax(0,1fr) 340px; border-color:var(--lgp-border); border-radius:8px; background:#091013; box-shadow:0 18px 44px rgba(0,0,0,.22); }
.lgp-native-sidebar { padding:13px; background:#0d1519; }
.lgp-native-sidebar:first-child { border-right-color:var(--lgp-border); }
.lgp-nozzle-sidebar { border-left-color:var(--lgp-border); background:#0c1418; }
.lgp-detail-tabs { gap:3px; padding:3px; margin-bottom:12px; border-color:#26373d; border-radius:6px; background:#091013; }
.lgp-detail-tabs button { height:31px; border-radius:4px; color:#83959b; font-size:11px; }
.lgp-detail-tabs button:hover { color:#d5e1e4; }
.lgp-detail-tabs button.active { background:var(--lgp-accent); color:#11180b; font-weight:700; }
.lgp-panel-heading { min-height:30px; padding-bottom:6px; border-bottom:1px solid #1c292e; }
.lgp-panel-heading strong { color:#e4edef; font-size:12px; }
.lgp-panel-heading small { color:#71848a; font-size:10px; }
.lgp-panel-actions .lgp-button { min-width:42px; padding:4px 7px; font-size:10px; }
.lgp-search { height:31px; margin:10px 0; border-color:#283a40; border-radius:5px; background:#091114; color:#dce7e9; font-size:11px; }
.lgp-search::placeholder { color:#60747a; }
.lgp-model-list, .lgp-nozzle-list { gap:5px; }
.lgp-model-item, .lgp-nozzle-item { border-color:#1e2c31; border-radius:6px; background:#0a1215; }
.lgp-model-item:hover, .lgp-nozzle-item:hover { border-color:#405158; background:#101a1e; }
.lgp-model-item.active, .lgp-nozzle-item.active { border-color:#81923f; background:#151d18; box-shadow:inset 3px 0 0 var(--lgp-accent); }
.lgp-model-item strong { color:#dce7e9; font-size:11px; }
.lgp-model-item span { color:#71848a; font-size:10px; }
.lgp-nozzle-item strong { background:#1b282c; color:#d8e94e; }
.lgp-nozzle-item span { color:#d7e2e5; font-size:11px; }
.lgp-native-preview { padding:11px; background:#080d10; }
.lgp-preview-toolbar { min-height:34px; padding:0 3px 7px; }
.lgp-preview-toolbar strong { color:#e1ebed; font-size:12px; }
.lgp-preview-toolbar span { color:#71848a; font-size:10px; }
.lgp-native-preview :deep(.probe-viewport) { border-color:#203037; border-radius:6px; background:#070c0f; }
.lgp-panel-scroll { padding:10px 3px 0 0; scrollbar-color:#34474e transparent; scrollbar-width:thin; }
.lgp-model-integration { margin:8px 0 10px; padding:7px 8px; border-color:#26373d; border-radius:6px; background:#091114; }
.lgp-control-group { gap:8px; padding:12px 0; border-top-color:#1e2c31; }
.lgp-control-heading { color:#899ca2; font-size:10px; font-weight:700; letter-spacing:.04em; }
.lgp-slider-row input[type='number'], .lgp-vector-row input, .lgp-interval-row input, .lgp-effect-field input[type='number'], .lgp-effect-field select, .lgp-keybind-panel input:not([type='checkbox']) { border-color:#2a3b41; border-radius:5px; background:#091114; color:#e1ebed; }
.lgp-control-hint { color:#71848a; }
.lgp-group-visibility { border-top-color:#1e2c31; padding-top:11px; }
.lgp-group-visibility summary { color:#dce7e9; font-size:11px; }
.lgp-group-visibility label { min-height:27px; padding:4px 7px; border-radius:4px; }
.lgp-group-visibility label:hover { background:#111a1e; }
.lgp-group-visibility label.selected { border-left-color:var(--lgp-accent); background:#151d18; }
.lgp-color-panel { gap:10px; padding:4px 0 13px; border-bottom-color:#1e2c31; }
.lgp-color-input { border-color:#2b3d43; border-radius:6px; background:#0a1215; }
.lgp-color-swatch { width:23px; height:23px; border-radius:50%; }
.lgp-effect-group { border-top-color:#1e2c31; }
.lgp-effect-group h4 { color:#dce7e9; font-size:11px; }
.lgp-effect-field { color:#83969c; font-size:10px; }
.lgp-effect-field.has-select { grid-template-columns:minmax(0,1fr) minmax(150px,1.1fr); align-items:start; }
.lgp-select-control { position:relative; display:grid; min-width:0; gap:4px; }
.lgp-select-trigger { display:flex; width:100%; min-width:0; min-height:30px; align-items:center; justify-content:space-between; gap:8px; padding:0 8px; border:1px solid #2a3b41; border-radius:5px; background:#091114; color:#e1ebed; font:inherit; font-size:10px; text-align:left; cursor:pointer; }
.lgp-select-trigger span { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.lgp-select-trigger svg { width:13px; height:13px; flex:0 0 auto; color:#87999f; transition:transform .18s ease; }
.lgp-select-trigger[aria-expanded='true'] { border-color:var(--lgp-accent); }
.lgp-select-trigger[aria-expanded='true'] svg { transform:rotate(180deg); }
.lgp-select-menu { display:grid; gap:3px; padding:4px; border:1px solid #314249; border-radius:6px; background:#0b1418; box-shadow:0 8px 18px rgba(0,0,0,.26); }
.lgp-select-menu button { min-height:30px; padding:6px 8px; border:0; border-radius:4px; background:transparent; color:#9babb0; font:inherit; font-size:10px; line-height:1.35; text-align:left; white-space:normal; cursor:pointer; }
.lgp-select-menu button:hover { background:#121d21; color:#e4edef; }
.lgp-select-menu button.active { background:#1a2318; color:#e9f58b; box-shadow:inset 2px 0 0 var(--lgp-accent); }
.lgp-keybind-panel > label, .lgp-key-modifiers { color:#83969c; font-size:10px; }
.lgp-sidebar-actions { gap:7px; padding:11px 0 1px; border-top:1px solid #1e2c31; background:linear-gradient(180deg,rgba(12,20,24,0),#0c1418 25%); }
.lgp-sidebar-actions .lgp-button { min-height:34px; }
.lgp-sidebar-actions .lgp-button.primary { min-height:38px; font-size:12px; }
.lgp-empty { margin:18px auto; border-color:#2b3b41; background:#0c1418; }
@media (max-width:1050px){.lgp-native-workspace{grid-template-columns:190px minmax(0,1fr) 310px}}
@media (max-width:820px){.lgp-page{overflow:auto}.lgp-native-workspace{grid-template-columns:1fr;height:auto}.lgp-native-preview{min-height:58vh}.lgp-header{align-items:flex-start}.lgp-brand-block{align-items:flex-start}.lgp-brand-mark{flex-basis:36px;width:36px;height:36px}.lgp-header h1{font-size:18px}}
@media (max-width:560px){.lgp-page{padding:10px}.lgp-subtitle{display:none}.lgp-status-paths{display:none}.lgp-detail-tabs button{font-size:10px}}
</style>
