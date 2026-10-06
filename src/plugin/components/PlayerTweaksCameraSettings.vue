<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { ResourceManager } from '../../store/ResourceManager'
import { defaultPlayerTweaksCamera, normalizePlayerTweaksCamera } from '../playerTweaksCamera'

const props = defineProps<{ gameName: string; enabled: boolean }>()
const { locale } = useI18n()
const zh = () => String(locale.value).startsWith('zh')
const label = (chinese: string, english: string) => zh() ? chinese : english
const camera = reactive(defaultPlayerTweaksCamera())
const loading = ref(false)
const saving = ref(false)
let requestId = 0

watch(() => props.gameName, async gameName => {
  const current = ++requestId
  Object.assign(camera, defaultPlayerTweaksCamera())
  if (!gameName) return
  loading.value = true
  try {
    const config = await ResourceManager.loadGameConfig(gameName)
    if (current === requestId) Object.assign(camera, normalizePlayerTweaksCamera(config.playerTweaksCamera))
  } catch (error) {
    if (current === requestId) ElMessage.error(String(error))
  } finally {
    if (current === requestId) loading.value = false
  }
}, { immediate: true })

const save = async () => {
  const gameName = props.gameName
  if (!gameName || loading.value || saving.value) return
  saving.value = true
  try {
    const config = await ResourceManager.loadGameConfig(gameName)
    config.playerTweaksCamera = normalizePlayerTweaksCamera({ ...camera })
    await ResourceManager.saveGameConfig(gameName, config)
    ElMessage.success(label('镜头设置已保存，下次启动游戏生效', 'Camera settings saved for the next launch'))
  } catch (error) {
    ElMessage.error(String(error))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="camera-settings">
    <h3>{{ label('镜头设置', 'Camera settings') }}</h3>
    <p class="camera-hint">{{ label('按当前游戏单独保存；下次通过 SSMT 启动时生效。', 'Saved per game; takes effect on the next SSMT launch.') }}</p>
    <p v-if="!enabled" class="camera-hint">{{ label('当前插件已禁用，设置不会注入游戏。', 'The plugin is disabled; these settings will not be injected.') }}</p>
    <p v-if="!gameName" class="camera-hint">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="camera-grid" :class="{ loading }">
      <label><input v-model="camera.customFov" type="checkbox">{{ label('自定义视野角', 'Custom FOV') }}</label>
      <label class="number-field">{{ label('视野角（30–120）', 'FOV (30–120)') }}
        <el-input-number v-model="camera.fov" :min="30" :max="120" :step="1" size="small" :disabled="!camera.customFov" />
      </label>
      <label><input v-model="camera.preserveAimingFov" type="checkbox" :disabled="!camera.customFov">{{ label('瞄准时保留游戏视野角', 'Preserve aiming FOV') }}</label>
      <label><input v-model="camera.preserveCutsceneFov" type="checkbox" :disabled="!camera.customFov">{{ label('过场时保留游戏视野角', 'Preserve cutscene FOV') }}</label>
      <label><input v-model="camera.restoreInUi" type="checkbox" :disabled="!camera.customFov">{{ label('打开界面时恢复游戏视野角', 'Restore game FOV in UI') }}</label>
      <label class="number-field">{{ label('过渡速度（0 为立即切换）', 'Transition speed (0 = instant)') }}
        <el-input-number v-model="camera.transitionSpeed" :min="0" :max="30" :step="1" size="small" :disabled="!camera.customFov" />
      </label>
      <label><input v-model="camera.disableInputSmoothing" type="checkbox">{{ label('关闭镜头输入平滑', 'Disable camera input smoothing') }}</label>
      <label><input v-model="camera.disableTransitionBlend" type="checkbox">{{ label('关闭镜头过渡混合', 'Disable camera transition blend') }}</label>
      <label><input v-model="camera.disableCharacterFade" type="checkbox">{{ label('关闭角色靠近镜头时淡出', 'Disable character fade') }}</label>
      <label><input v-model="camera.disableEventCameraMovement" type="checkbox">{{ label('关闭事件镜头移动', 'Disable event camera movement') }}</label>
    </div>
    <p v-if="gameName && camera.customFov && camera.preserveAimingFov" class="camera-hint">
      {{ label('已知限制：瞄准时仍可能比原生视野略宽。', 'Known limitation: aiming can remain wider than the original game view.') }}
    </p>
    <el-button size="small" type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
      {{ label('保存设置', 'Save settings') }}
    </el-button>
  </section>
</template>

<style scoped>
.camera-settings { border-top: 1px solid var(--market-line); padding-top: 16px; }
.camera-settings h3 { margin: 0 0 7px; font-size: 13px; }
.camera-hint { margin: 0 0 10px; color: rgba(var(--theme-text-secondary-rgb), .8); font-size: 11px; line-height: 1.5; }
.camera-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(215px, 1fr)); gap: 10px 16px; margin: 12px 0 16px; font-size: 12px; }
.camera-grid.loading { opacity: .5; pointer-events: none; }
.camera-grid label { display: flex; align-items: center; gap: 7px; min-height: 25px; }
.camera-grid input[type='checkbox'] { accent-color: var(--t-page-accent); }
.camera-grid .number-field { justify-content: space-between; gap: 10px; }
.camera-grid :deep(.el-input-number) { width: 110px; flex: none; }
</style>
