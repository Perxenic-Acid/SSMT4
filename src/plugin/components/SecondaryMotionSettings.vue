<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { openPath, openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'

interface RuntimeStatus {
  state?: string
  character?: string
  profile?: boolean
  bones?: boolean
  mode?: string
  gait?: string
  last_write_ms?: number
  error?: string
}

interface SecondaryMotionSnapshot {
  pluginId: string
  pluginName: string
  pluginVersion: string
  gameName: string
  ssmtEnabled: boolean
  runtimeEnabled: boolean
  revision: number
  activePreset: string
  presets: string[]
  packageRoot: string
  dataRoot: string
  dllPath: string
  status: RuntimeStatus
  gameRunning: boolean
  characterId: string
  characterInPreset: boolean
  characterSettings: Record<string, any>
}

interface ParameterField {
  path: string
  group: string
  value: string | number | boolean
}

const props = defineProps<{
  pluginId: string
  gameName: string
}>()

const { locale } = useI18n()
const isZh = computed(() => String(locale.value).toLowerCase().startsWith('zh'))
const label = (zh: string, en: string) => isZh.value ? zh : en

const snapshot = ref<SecondaryMotionSnapshot | null>(null)
const runtimeEnabled = ref(false)
const activePreset = ref('')
const loading = ref(false)
const saving = ref(false)
const characterSaving = ref(false)
const error = ref('')
const characterDraft = ref<Record<string, any> | null>(null)
let requestId = 0

const statusState = computed(() => {
  if (snapshot.value && !snapshot.value.gameRunning) return 'STOPPED'
  return (snapshot.value?.status?.state || 'NOT_STARTED').toUpperCase()
})
const statusLabel = computed(() => {
  const labels: Record<string, [string, string]> = {
    NOT_STARTED: ['尚未启动', 'Not started'],
    STOPPED: ['游戏未运行', 'Game not running'],
    STARTING: ['正在启动', 'Starting'],
    CONFIG_LOADED: ['配置已加载', 'Config loaded'],
    SYMBOLS_RESOLVED: ['符号已解析', 'Symbols resolved'],
    HOOKS_INSTALLED: ['Hook 已安装', 'Hooks installed'],
    READY: ['运行中', 'Ready'],
    DEGRADED: ['已运行（降级）', 'Running degraded'],
    DISABLED_SAFE: ['已安全停用', 'Safely disabled'],
  }
  const current = labels[statusState.value] || [statusState.value, statusState.value]
  return isZh.value ? current[0] : current[1]
})
const statusTone = computed(() => {
  if (statusState.value === 'READY') return 'ready'
  if (statusState.value === 'DEGRADED') return 'warning'
  if (statusState.value === 'DISABLED_SAFE') return 'danger'
  return 'idle'
})
const statusDetails = computed(() => {
  const status = snapshot.value?.status
  if (!status) return []
  return [
    status.character ? `${label(snapshot.value?.gameRunning ? '角色' : '上次角色', snapshot.value?.gameRunning ? 'Character' : 'Last character')}: ${status.character}` : '',
    status.mode ? `${label('模式', 'Mode')}: ${status.mode}` : '',
    status.gait ? `${label('步态', 'Gait')}: ${status.gait}` : '',
    typeof status.bones === 'boolean' ? `${label(snapshot.value?.gameRunning ? '骨骼' : '上次骨骼', snapshot.value?.gameRunning ? 'Bones' : 'Last bones')}: ${status.bones ? 'PASS' : 'FAIL'}` : '',
    typeof status.profile === 'boolean' ? `${label(snapshot.value?.gameRunning ? '角色配置' : '上次角色配置', snapshot.value?.gameRunning ? 'Profile' : 'Last profile')}: ${status.profile ? 'PASS' : 'FAIL'}` : '',
  ].filter(Boolean)
})
const characterGroups = computed(() => {
  const groups = new Map<string, ParameterField[]>()
  if (!characterDraft.value) return []
  const visit = (value: unknown, path: string, group: string) => {
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      for (const [key, child] of Object.entries(value)) {
        const nextPath = path ? `${path}.${key}` : key
        const nextGroup = path ? group : key
        visit(child, nextPath, nextGroup)
      }
      return
    }
    if (value === null || value === undefined || Array.isArray(value)) return
    const target = path.includes('.') ? group : label('基础参数', 'General')
    const fields = groups.get(target) ?? []
    fields.push({ path, group: target, value: value as string | number | boolean })
    groups.set(target, fields)
  }
  visit(characterDraft.value, '', '')
  return [...groups.entries()].map(([group, fields]) => ({
    group,
    label: formatGroupLabel(group),
    fields,
  }))
})
const formatGroupLabel = (group: string) => {
  if (group === 'gait') return label('步态幅度', 'Gait amplitudes')
  if (group.startsWith('gait.')) return `${label('步态', 'Gait')} · ${group.slice(5)}`
  if (group === 'envelope') return label('平滑参数', 'Envelope')
  if (group === 'jump') return label('跳跃 / 惯性', 'Jump / inertia')
  if (group === 'native_amplify') return label('原生增益', 'Native amplification')
  if (group === 'axis') return label('运动轴', 'Motion axis')
  if (group === 'bones') return label('骨骼', 'Bones')
  return group
}
const formatFieldLabel = (path: string) => {
  const key = path.split('.').pop() || path
  const labels: Record<string, [string, string]> = {
    enabled: ['启用', 'Enabled'],
    motion_mode: ['运动模式', 'Motion mode'],
    right: ['右侧骨骼', 'Right bone'],
    left: ['左侧骨骼', 'Left bone'],
    allow_fallback_candidates: ['允许候选骨骼回退', 'Allow bone fallback'],
    name: ['轴向', 'Axis'],
    sign: ['方向符号', 'Direction sign'],
    amplitude_deg: ['幅度（度）', 'Amplitude (deg)'],
    amplitude_down_deg: ['下半周期幅度（度）', 'Down amplitude (deg)'],
    frequency_hz: ['频率（Hz）', 'Frequency (Hz)'],
    phase_offset_deg: ['相位偏移（度）', 'Phase offset (deg)'],
    amplitude_attack_tau_sec: ['幅度平滑时间（秒）', 'Amplitude tau (s)'],
    frequency_tau_sec: ['频率平滑时间（秒）', 'Frequency tau (s)'],
    to_idle_release_tau_sec: ['回待机平滑时间（秒）', 'Idle release tau (s)'],
    mode: ['模式', 'Mode'],
    takeoff_delay_sec: ['起跳延迟（秒）', 'Takeoff delay (s)'],
    rising_target_deg: ['上升期目标（度）', 'Rising target (deg)'],
    apex_falling_target_deg: ['高点/下落目标（度）', 'Apex/falling target (deg)'],
    acceleration_response: ['加速度响应', 'Acceleration response'],
    acceleration_filter_tau_sec: ['加速度滤波（秒）', 'Acceleration filter (s)'],
    natural_frequency_hz: ['弹簧频率（Hz）', 'Spring frequency (Hz)'],
    damping_ratio: ['阻尼比', 'Damping ratio'],
    landing_impulse_gain: ['落地冲击增益', 'Landing impulse gain'],
    max_duration_sec: ['最大持续时间（秒）', 'Max duration (s)'],
    factor: ['原生增益倍率', 'Native amplification factor'],
  }
  const labelValue = labels[key]
  return labelValue ? label(labelValue[0], labelValue[1]) : key
}
const setPathValue = (path: string, value: string | number | boolean) => {
  if (!characterDraft.value) return
  const parts = path.split('.')
  let target: Record<string, any> = characterDraft.value
  for (const part of parts.slice(0, -1)) {
    if (!target[part] || typeof target[part] !== 'object') target[part] = {}
    target = target[part]
  }
  target[parts[parts.length - 1]] = value
}

const load = async () => {
  if (!props.pluginId || !props.gameName) return
  const currentRequest = ++requestId
  loading.value = true
  error.value = ''
  try {
    const value = await invoke<SecondaryMotionSnapshot>('secondary_motion_snapshot', {
      pluginId: props.pluginId,
      gameName: props.gameName,
    })
    if (currentRequest !== requestId) return
    snapshot.value = value
    runtimeEnabled.value = value.runtimeEnabled
    activePreset.value = value.activePreset || value.presets[0] || ''
    characterDraft.value = structuredClone(value.characterSettings ?? {})
  } catch (cause) {
    if (currentRequest === requestId) error.value = String(cause)
  } finally {
    if (currentRequest === requestId) loading.value = false
  }
}

const apply = async () => {
  if (!snapshot.value || saving.value) return
  if (!activePreset.value) {
    ElMessage.warning(label('请选择预设', 'Select a preset'))
    return
  }
  saving.value = true
  try {
    const value = await invoke<SecondaryMotionSnapshot>('update_secondary_motion_settings', {
      pluginId: props.pluginId,
      gameName: props.gameName,
      runtimeEnabled: runtimeEnabled.value,
      activePreset: activePreset.value,
    })
    snapshot.value = value
    runtimeEnabled.value = value.runtimeEnabled
    activePreset.value = value.activePreset
    characterDraft.value = structuredClone(value.characterSettings ?? {})
    ElMessage.success(label(
      `配置已应用，revision ${value.revision}。游戏中会自动热加载。`,
      `Settings applied at revision ${value.revision}. The game hot-reloads them automatically.`,
    ))
  } catch (cause) {
    ElMessage.error(label(`应用失败：${String(cause)}`, `Apply failed: ${String(cause)}`))
  } finally {
    saving.value = false
  }
}

const saveCharacter = async () => {
  if (!snapshot.value || !characterDraft.value || !snapshot.value.characterId || characterSaving.value) return
  characterSaving.value = true
  try {
    const value = await invoke<SecondaryMotionSnapshot>('update_secondary_motion_character', {
      pluginId: props.pluginId,
      gameName: props.gameName,
      characterId: snapshot.value.characterId,
      characterSettings: characterDraft.value,
    })
    snapshot.value = value
    runtimeEnabled.value = value.runtimeEnabled
    activePreset.value = value.activePreset
    characterDraft.value = structuredClone(value.characterSettings ?? {})
    ElMessage.success(label(
      `角色参数已保存，revision ${value.revision}。`,
      `Character settings saved at revision ${value.revision}.`,
    ))
  } catch (cause) {
    ElMessage.error(label(`角色参数保存失败：${String(cause)}`, `Failed to save character settings: ${String(cause)}`))
  } finally {
    characterSaving.value = false
  }
}

const openDataFolder = async () => {
  if (!snapshot.value?.dataRoot) return
  try {
    await openPath(snapshot.value.dataRoot)
  } catch (cause) {
    ElMessage.error(String(cause))
  }
}

const openProject = async () => {
  try {
    await openUrl('https://github.com/Sp1cHless/Arknights-Endfield-Plugin-Secondary-bodyphysics')
  } catch (cause) {
    ElMessage.error(String(cause))
  }
}

watch(() => [props.pluginId, props.gameName], () => void load(), { immediate: true })
onMounted(() => {
  if (!snapshot.value && !loading.value) void load()
})
</script>

<template>
  <section class="secondary-motion-settings">
    <div class="settings-heading">
      <div>
        <h3>{{ label('次级身体物理', 'Secondary body physics') }}</h3>
        <p>
          {{ label(
            '由 SSMT 直接注入插件包内的 sbm.dll；不向游戏根目录复制文件。',
            'SSMT directly injects sbm.dll from the plugin package; no files are copied to the game root.',
          ) }}
        </p>
      </div>
      <span class="status-pill" :class="`is-${statusTone}`">{{ statusLabel }}</span>
    </div>

    <aside class="risk-notice" role="note">
      <div class="risk-title">
        <strong>{{ label('⚠ 原生注入风险与免责声明', '⚠ Native injection risk and disclaimer') }}</strong>
      </div>
      <p>
        {{ label(
          '本插件通过 SSMT 直接向游戏进程加载第三方 sbm.dll，并使用 IL2CPP/MinHook 修改 Unity 骨骼更新。它不是官方功能，也未获得游戏开发商或发行商认可。',
          'This plugin uses SSMT to load third-party sbm.dll directly into the game process and uses IL2CPP/MinHook to modify Unity bone updates. It is unofficial and is not endorsed by the game developer or publisher.',
        ) }}
      </p>
      <ul>
        <li>{{ label('可能触发反作弊、账号风控或其他账号处置，风险由使用者自行承担。', 'It may trigger anti-cheat, account risk controls, or other account action. You accept this risk.') }}</li>
        <li>{{ label('可能造成游戏崩溃、卡顿、骨骼异常、穿模或存档相关风险。', 'It may cause crashes, stutter, broken bones, clipping, or save-related risk.') }}</li>
        <li>{{ label('游戏更新可能使插件失效、行为变化或产生兼容问题。', 'Game updates may break the plugin or change its behavior.') }}</li>
        <li>{{ label('第三方 DLL 具有进程内原生代码权限；只应使用可信来源。', 'Third-party DLLs have native in-process access. Use only trusted sources.') }}</li>
      </ul>
      <details>
        <summary>{{ label('查看完整免责声明', 'Full disclaimer') }}</summary>
        <p>
          {{ label(
            '插件按现状提供，不附带任何明示或暗示担保。作者、SSMT 集成者和分发者不对封号、数据损坏、设备故障、存档丢失或其他直接或间接损失承担责任。继续安装、启用或启动游戏即表示你理解并接受上述风险，并自行确认使用方式符合当地法律、游戏条款和所在服务器规则。',
            'The plugin is provided as-is without any express or implied warranty. Its authors, SSMT integrators, and distributors are not responsible for account bans, data corruption, device failure, save loss, or other direct or indirect damage. Installing, enabling, or launching with the plugin means you understand and accept these risks and will confirm compliance with applicable law, game terms, and server rules.',
          ) }}
        </p>
      </details>
      <div class="maintenance-note">
        {{ label(
          '本 SSMT 适配版基于 Sp1cHless 的原项目修改，并非原项目官方发行版。该版本由非原项目作者尽力维护，不保证持续更新或及时兼容后续游戏版本。请勿到原项目作者处催更，也不要向原项目作者反馈本适配版的问题。',
          'This SSMT adaptation is based on Sp1cHless\'s original project and is not an official release of that project. It is maintained by a third party on a best-effort basis, with no guarantee of continued updates or timely compatibility with future game versions. Please do not ask the original project\'s author to expedite updates or report issues with this adaptation to them.',
        ) }}
        <el-button link type="primary" class="project-link" @click="openProject">
          {{ label('查看原项目 GitHub ↗', 'View original project on GitHub ↗') }}
        </el-button>
      </div>
    </aside>

    <div v-if="!snapshot && loading" class="settings-state">{{ label('正在读取配置…', 'Loading settings…') }}</div>
    <div v-else-if="error" class="settings-state is-error">{{ error }}</div>
    <template v-else-if="snapshot">
      <div v-if="!snapshot.ssmtEnabled" class="settings-notice">
        {{ label(
          '插件目前在 SSMT 中未启用。上方启用并重启游戏后才会注入。',
          'The plugin is disabled in SSMT. Enable it above and restart the game to inject it.',
        ) }}
      </div>

      <div class="settings-grid">
        <label class="setting-row">
          <span>
            <strong>{{ label('骨骼物理总开关', 'Global motion switch') }}</strong>
            <small>{{ label('仅控制运行时的骨骼写入，不影响插件注入。', 'Controls runtime bone writes independently of injection.') }}</small>
          </span>
          <el-switch v-model="runtimeEnabled" />
        </label>

        <label class="setting-row">
          <span>
            <strong>{{ label('当前预设', 'Active preset') }}</strong>
            <small>{{ label('应用后 revision 自动递增，游戏内热加载。', 'Applying increments revision for in-game hot reload.') }}</small>
          </span>
          <el-select v-model="activePreset" style="width: 190px" :disabled="snapshot.presets.length === 0">
            <el-option v-for="preset in snapshot.presets" :key="preset" :label="preset" :value="preset" />
          </el-select>
        </label>
      </div>

      <section class="character-editor">
        <div class="character-editor-head">
          <div>
            <strong>{{ label('当前角色参数', 'Current character parameters') }}</strong>
            <small v-if="snapshot.characterId">
              {{ snapshot.characterId }}
              <template v-if="!snapshot.characterInPreset"> · {{ label('尚未写入当前预设', 'Not yet in the active preset') }}</template>
            </small>
            <small v-else>{{ label('进入游戏并切换角色后加载参数。', 'Enter the game and select a character to load parameters.') }}</small>
          </div>
          <el-button v-if="snapshot.characterId" size="small" :loading="characterSaving" @click="saveCharacter">
            {{ label('保存角色参数', 'Save character') }}
          </el-button>
        </div>

        <div v-if="!snapshot.characterId" class="settings-state">
          {{ label('游戏尚未报告当前角色。', 'No active character has been reported yet.') }}
        </div>
        <div v-else-if="characterGroups.length === 0" class="settings-state">
          {{ label('当前角色没有可编辑参数。', 'No editable parameters are available.') }}
        </div>
        <div v-else class="parameter-groups">
          <details
            v-for="group in characterGroups"
            :key="group.group"
            class="parameter-group"
          >
            <summary>{{ group.label }}</summary>
            <div class="parameter-rows">
              <label v-for="field in group.fields" :key="field.path" class="parameter-row">
                <span class="parameter-name" :title="field.path">{{ formatFieldLabel(field.path) }}<small>{{ field.path }}</small></span>
                <el-switch
                  v-if="typeof field.value === 'boolean'"
                  :model-value="field.value"
                  @change="value => setPathValue(field.path, Boolean(value))"
                />
                <el-input-number
                  v-else-if="typeof field.value === 'number'"
                  :model-value="field.value"
                  :controls="false"
                  :step="0.01"
                  @change="value => setPathValue(field.path, Number(value ?? 0))"
                />
                <el-input
                  v-else
                  :model-value="String(field.value)"
                  @input="value => setPathValue(field.path, String(value))"
                />
              </label>
            </div>
          </details>
        </div>
      </section>

      <div class="settings-actions">
        <el-button type="primary" :loading="saving" :disabled="loading" @click="apply">
          {{ label('应用配置', 'Apply settings') }}
        </el-button>
        <el-button :loading="loading" @click="load">{{ label('刷新状态', 'Refresh') }}</el-button>
        <el-button @click="openDataFolder">{{ label('打开数据目录', 'Open data folder') }}</el-button>
        <el-button @click="openProject">{{ label('原项目 GitHub', 'Original GitHub') }}</el-button>
      </div>

      <div class="runtime-status">
        <div class="runtime-status-title">
          <strong>{{ label('运行时状态', 'Runtime status') }}</strong>
          <span>revision {{ snapshot.revision }}</span>
        </div>
        <p v-if="statusDetails.length" class="runtime-status-details">{{ statusDetails.join(' · ') }}</p>
        <p v-else class="runtime-status-details">{{ label('进入游戏并切换角色后会显示检测结果。', 'Detection appears after entering the game and selecting a character.') }}</p>
        <p v-if="snapshot.status?.error" class="runtime-error">{{ snapshot.status.error }}</p>
        <p v-if="snapshot.status?.last_write_ms" class="runtime-status-details">
          {{ label('最近写入', 'Last write') }}: {{ snapshot.status.last_write_ms }} ms
        </p>
      </div>

      <details class="settings-paths">
        <summary>{{ label('注入路径与安装位置', 'Injection and installation paths') }}</summary>
        <dl>
          <dt>{{ label('注入 DLL', 'Injected DLL') }}</dt><dd>{{ snapshot.dllPath }}</dd>
          <dt>{{ label('数据目录', 'Data directory') }}</dt><dd>{{ snapshot.dataRoot }}</dd>
          <dt>{{ label('插件版本', 'Plugin version') }}</dt><dd>{{ snapshot.pluginName }} v{{ snapshot.pluginVersion }}</dd>
        </dl>
      </details>
    </template>
  </section>
</template>

<style scoped>
.secondary-motion-settings { min-width: 0; }
.settings-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
.settings-heading h3 { margin: 0 0 6px; font-size: 15px; }
.settings-heading p { margin: 0; color: var(--t-page-text-muted); font-size: 12px; line-height: 1.65; }
.status-pill { flex: none; padding: 4px 9px; border: 1px solid currentColor; border-radius: 999px; font-size: 11px; }
.status-pill.is-ready { color: var(--t-success-text); background: var(--t-success-bg); }
.status-pill.is-warning { color: var(--t-warning-text); background: var(--t-warning-bg); }
.status-pill.is-danger { color: var(--t-danger-text); background: var(--t-danger-bg); }
.status-pill.is-idle { color: var(--t-page-text-muted); }
.risk-notice { margin-bottom: 16px; padding: 13px 15px; border: 1px solid rgba(255, 170, 70, .38); border-radius: 8px; background: rgba(120, 65, 10, .20); color: rgba(255, 220, 170, .94); font-size: 11px; line-height: 1.65; }
.risk-title { margin-bottom: 6px; color: #ffd08a; font-size: 13px; }
.risk-notice p { margin: 6px 0; }
.risk-notice ul { margin: 8px 0; padding-left: 18px; }
.risk-notice li + li { margin-top: 3px; }
.risk-notice details { margin-top: 8px; }
.risk-notice summary { cursor: pointer; color: #ffd08a; }
.maintenance-note { margin: 10px 0 0; padding: 9px 12px; border-left: 3px solid #ffd08a; border-radius: 4px; background: rgba(255, 208, 138, .08); color: inherit; font-size: 11px; line-height: 1.6; }
.maintenance-note :deep(.project-link) { height: auto; margin-top: 5px; padding: 0; color: #ffd08a; font-size: 11px; }
.settings-state, .settings-notice { padding: 12px 14px; border-radius: 8px; background: var(--t-surface-subtle); color: var(--t-page-text-muted); font-size: 12px; line-height: 1.6; }
.settings-state.is-error { color: var(--t-danger-text); background: var(--t-danger-bg); }
.settings-notice { margin-bottom: 14px; color: var(--t-warning-text); background: var(--t-warning-bg); border: 1px solid var(--t-warning-border); }
.settings-grid { display: grid; gap: 10px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 20px; padding: 14px; border: 1px solid var(--market-line, rgba(var(--theme-surface-tint-rgb), .12)); border-radius: 8px; background: var(--t-surface-subtle); }
.setting-row span { display: flex; min-width: 0; flex-direction: column; gap: 4px; }
.setting-row strong { font-size: 13px; }
.setting-row small { color: var(--t-page-text-muted); font-size: 11px; line-height: 1.5; }
.settings-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 16px; }
.settings-actions :deep(.el-button + .el-button) { margin-left: 0; }
.character-editor { margin-top: 16px; padding: 14px; border: 1px solid var(--market-line, rgba(var(--theme-surface-tint-rgb), .12)); border-radius: 8px; background: var(--t-surface-subtle); }
.character-editor-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 14px; margin-bottom: 12px; }
.character-editor-head > div { display: flex; min-width: 0; flex-direction: column; gap: 4px; }
.character-editor-head strong { font-size: 13px; }
.character-editor-head small { color: var(--t-page-text-muted); font-size: 11px; word-break: break-all; }
.parameter-groups { display: grid; gap: 7px; }
.parameter-group { border: 1px solid var(--market-line, rgba(var(--theme-surface-tint-rgb), .10)); border-radius: 7px; background: rgba(0, 0, 0, .10); }
.parameter-group summary { padding: 9px 11px; cursor: pointer; font-size: 12px; font-weight: 600; }
.parameter-rows { display: grid; gap: 7px; padding: 0 11px 11px; }
.parameter-row { display: grid; grid-template-columns: minmax(150px, 1fr) minmax(130px, 190px); align-items: center; gap: 12px; }
.parameter-name { overflow-wrap: anywhere; color: var(--t-page-text-muted); font-family: ui-monospace, Consolas, monospace; font-size: 11px; }
.parameter-name small { display: block; margin-top: 2px; color: var(--t-page-text-muted); font-size: 9px; opacity: .62; }
.parameter-row :deep(.el-input-number), .parameter-row :deep(.el-input) { width: 100%; }
.runtime-status, .settings-paths { margin-top: 14px; padding: 14px; border: 1px solid var(--market-line, rgba(var(--theme-surface-tint-rgb), .12)); border-radius: 8px; background: var(--t-surface-subtle); }
.runtime-status-title { display: flex; justify-content: space-between; gap: 10px; font-size: 12px; }
.runtime-status-title span { color: var(--t-page-text-muted); }
.runtime-status-details { margin: 9px 0 0; color: var(--t-page-text-muted); font-size: 11px; line-height: 1.65; }
.runtime-error { margin: 9px 0 0; color: var(--t-danger-text); font: 11px/1.6 ui-monospace, Consolas, monospace; }
.settings-paths summary { cursor: pointer; font-size: 12px; }
.settings-paths dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 7px 16px; margin: 12px 0 0; font-size: 11px; line-height: 1.5; }
.settings-paths dt { color: var(--t-page-text-muted); }
.settings-paths dd { margin: 0; overflow-wrap: anywhere; user-select: text; }
@media (max-width: 680px) {
  .setting-row { align-items: flex-start; flex-direction: column; }
  .settings-heading { flex-direction: column; }
  .character-editor-head { flex-direction: column; }
  .parameter-row { grid-template-columns: minmax(0, 1fr); gap: 5px; }
}
</style>
