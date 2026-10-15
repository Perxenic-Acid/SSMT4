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
  <section class="tweak-settings">
    <h3>{{ label('镜头设置', 'Camera settings') }}</h3>
    <p class="tweak-hint">{{ label('按当前游戏单独保存；下次通过 SSMT 启动时生效。', 'Saved per game; takes effect on the next SSMT launch.') }}</p>
    <p v-if="!enabled" class="tweak-hint tweak-hint-warning">{{ label('当前插件已禁用，设置不会注入游戏。', 'The plugin is disabled; these settings will not be injected.') }}</p>
    <p v-if="!gameName" class="tweak-hint tweak-hint-warning">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="tweak-grid" :class="{ loading }">
      <el-checkbox v-model="camera.customFov">{{ label('自定义视野角', 'Custom FOV') }}</el-checkbox>
      <div class="tweak-number">
        <span>{{ label('视野角（30–120）', 'FOV (30–120)') }}</span>
        <el-input-number v-model="camera.fov" :min="30" :max="120" :step="1" size="small" :disabled="!camera.customFov" />
      </div>
      <el-checkbox v-model="camera.preserveAimingFov" :disabled="!camera.customFov">{{ label('瞄准时保留游戏视野角', 'Preserve aiming FOV') }}</el-checkbox>
      <el-checkbox v-model="camera.preserveCutsceneFov" :disabled="!camera.customFov">{{ label('过场时保留游戏视野角', 'Preserve cutscene FOV') }}</el-checkbox>
      <el-checkbox v-model="camera.restoreInUi" :disabled="!camera.customFov">{{ label('打开界面时恢复游戏视野角', 'Restore game FOV in UI') }}</el-checkbox>
      <div class="tweak-number">
        <span>{{ label('过渡速度（0 为立即切换）', 'Transition speed (0 = instant)') }}</span>
        <el-input-number v-model="camera.transitionSpeed" :min="0" :max="30" :step="1" size="small" :disabled="!camera.customFov" />
      </div>
      <el-checkbox v-model="camera.disableInputSmoothing">{{ label('关闭镜头输入平滑', 'Disable camera input smoothing') }}</el-checkbox>
      <el-checkbox v-model="camera.disableTransitionBlend">{{ label('关闭镜头过渡混合', 'Disable camera transition blend') }}</el-checkbox>
      <el-checkbox v-model="camera.disableCharacterFade">{{ label('关闭角色靠近镜头时淡出', 'Disable character fade') }}</el-checkbox>
      <el-checkbox v-model="camera.disableEventCameraMovement">{{ label('关闭事件镜头移动', 'Disable event camera movement') }}</el-checkbox>
      <el-checkbox v-model="camera.cameraZoom">{{ label('解锁原生缩放上下限', 'Unlock native zoom limits') }}</el-checkbox>
    </div>
    <p v-if="gameName && camera.customFov && camera.preserveAimingFov" class="tweak-hint">
      {{ label('已知限制：瞄准时仍可能比原生视野略宽。', 'Known limitation: aiming can remain wider than the original game view.') }}
    </p>
    <el-button size="small" type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
      {{ label('保存设置', 'Save settings') }}
    </el-button>
  </section>
</template>

<style scoped>
.tweak-settings { padding-top: 0; }
.tweak-settings h3 { margin: 0 0 7px; font-size: 13px; font-weight: 650; }
.tweak-hint { margin: 0 0 10px; color: rgba(var(--theme-text-secondary-rgb), .8); font-size: 11px; line-height: 1.5; }
.tweak-hint-warning { color: var(--t-warning-text); }
.tweak-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(235px, 1fr)); gap: 10px 16px; margin: 14px 0 16px; font-size: 12px; }
.tweak-grid.loading { opacity: .5; pointer-events: none; }
.tweak-grid .el-checkbox { margin-right: 0; }
.tweak-number { display: flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 25px; color: rgba(var(--theme-text-secondary-rgb), .9); }
.tweak-number :deep(.el-input-number) { width: 110px; flex: none; }
</style>

