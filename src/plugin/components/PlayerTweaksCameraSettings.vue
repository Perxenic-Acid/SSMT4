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
  <section class="tweak-settings" :aria-busy="loading || saving">
    <header class="tweak-section-header">
      <div>
        <h3>{{ label('镜头设置', 'Camera settings') }}</h3>
        <p class="tweak-hint">{{ label('按当前游戏保存，下次通过 SSMT 启动时生效。', 'Saved for this game; applies on the next SSMT launch.') }}</p>
      </div>
      <el-button type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
        {{ label('保存镜头设置', 'Save camera settings') }}
      </el-button>
    </header>
    <p v-if="!enabled" class="tweak-hint tweak-hint-warning">{{ label('当前插件已禁用，设置不会注入游戏。', 'The plugin is disabled; these settings will not be injected.') }}</p>
    <p v-if="!gameName" class="tweak-hint tweak-hint-warning">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="tweak-groups" :inert="loading || saving">
      <section class="tweak-card">
        <h4>{{ label('视野与缩放', 'Field of view & zoom') }}</h4>
        <div class="tweak-row">
          <div class="tweak-row-copy">
            <span>{{ label('自定义视野角', 'Custom FOV') }}</span>
            <small>{{ label('调整常规游玩时的镜头视野。', 'Adjust the field of view during normal gameplay.') }}</small>
          </div>
          <el-switch v-model="camera.customFov" :aria-label="label('自定义视野角', 'Custom FOV')" />
        </div>
        <div class="tweak-numbers" :class="{ 'is-muted': !camera.customFov }">
          <label class="tweak-number">
            <span>{{ label('视野角', 'Field of view') }} <small>30–120°</small></span>
            <el-input-number v-model="camera.fov" :min="30" :max="120" :step="1" size="small" :disabled="!camera.customFov" :aria-label="label('视野角', 'Field of view')" />
          </label>
          <label class="tweak-number">
            <span>{{ label('过渡速度', 'Transition speed') }} <small>{{ label('0 为立即切换', '0 = instant') }}</small></span>
            <el-input-number v-model="camera.transitionSpeed" :min="0" :max="30" :step="1" size="small" :disabled="!camera.customFov" :aria-label="label('过渡速度', 'Transition speed')" />
          </label>
        </div>
        <div class="tweak-row">
          <div class="tweak-row-copy">
            <span>{{ label('解锁原生缩放上下限', 'Unlock native zoom limits') }}</span>
            <small>{{ label('扩展镜头拉近与拉远的范围。', 'Extend the camera zoom range.') }}</small>
          </div>
          <el-switch v-model="camera.cameraZoom" :aria-label="label('解锁原生缩放上下限', 'Unlock native zoom limits')" />
        </div>
      </section>
      <section class="tweak-card">
        <h4>{{ label('场景视野', 'Contextual FOV') }}</h4>
        <p class="tweak-card-hint">{{ label('启用自定义视野后，可保留这些场景的游戏视野。', 'With custom FOV enabled, preserve game FOV in these contexts.') }}</p>
        <div class="tweak-row" :class="{ 'is-muted': !camera.customFov }">
          <div class="tweak-row-copy"><span>{{ label('瞄准时', 'While aiming') }}</span></div>
          <el-switch v-model="camera.preserveAimingFov" :disabled="!camera.customFov" :aria-label="label('瞄准时保留游戏视野角', 'Preserve aiming FOV')" />
        </div>
        <div class="tweak-row" :class="{ 'is-muted': !camera.customFov }">
          <div class="tweak-row-copy"><span>{{ label('过场动画中', 'During cutscenes') }}</span></div>
          <el-switch v-model="camera.preserveCutsceneFov" :disabled="!camera.customFov" :aria-label="label('过场时保留游戏视野角', 'Preserve cutscene FOV')" />
        </div>
        <div class="tweak-row" :class="{ 'is-muted': !camera.customFov }">
          <div class="tweak-row-copy"><span>{{ label('打开游戏界面时', 'In game menus') }}</span></div>
          <el-switch v-model="camera.restoreInUi" :disabled="!camera.customFov" :aria-label="label('打开界面时恢复游戏视野角', 'Restore game FOV in UI')" />
        </div>
        <p v-if="camera.customFov && camera.preserveAimingFov" class="tweak-card-hint tweak-footnote">
          {{ label('已知限制：瞄准时仍可能比原生视野略宽。', 'Known limitation: aiming can remain wider than the original game view.') }}
        </p>
      </section>
      <section class="tweak-card tweak-card-wide">
        <h4>{{ label('镜头行为', 'Camera behavior') }}</h4>
        <div class="tweak-options">
          <div class="tweak-row">
            <div class="tweak-row-copy"><span>{{ label('关闭镜头输入平滑', 'Disable input smoothing') }}</span><small>{{ label('直接响应镜头操作。', 'Respond directly to camera input.') }}</small></div>
            <el-switch v-model="camera.disableInputSmoothing" :aria-label="label('关闭镜头输入平滑', 'Disable camera input smoothing')" />
          </div>
          <div class="tweak-row">
            <div class="tweak-row-copy"><span>{{ label('关闭镜头过渡混合', 'Disable transition blending') }}</span><small>{{ label('停用游戏原有的镜头混合。', 'Disable the game’s camera blending.') }}</small></div>
            <el-switch v-model="camera.disableTransitionBlend" :aria-label="label('关闭镜头过渡混合', 'Disable camera transition blend')" />
          </div>
          <div class="tweak-row">
            <div class="tweak-row-copy"><span>{{ label('关闭角色近镜头淡出', 'Disable character fade') }}</span><small>{{ label('镜头靠近时仍显示角色。', 'Keep the character visible near the camera.') }}</small></div>
            <el-switch v-model="camera.disableCharacterFade" :aria-label="label('关闭角色靠近镜头时淡出', 'Disable character fade')" />
          </div>
          <div class="tweak-row">
            <div class="tweak-row-copy"><span>{{ label('关闭事件镜头移动', 'Disable event camera movement') }}</span><small>{{ label('停用事件触发的自动镜头位移。', 'Disable automatic camera movement triggered by events.') }}</small></div>
            <el-switch v-model="camera.disableEventCameraMovement" :aria-label="label('关闭事件镜头移动', 'Disable event camera movement')" />
          </div>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped src="./playerTweaksSettings.css"></style>

