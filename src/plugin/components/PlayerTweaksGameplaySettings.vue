<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { ResourceManager } from '../../store/ResourceManager'
import { defaultPlayerTweaksGameplay, normalizePlayerTweaksGameplay } from '../playerTweaksGameplay'

const props = defineProps<{ gameName: string; enabled: boolean }>()
const { locale } = useI18n()
const label = (chinese: string, english: string) => String(locale.value).startsWith('zh') ? chinese : english
const gameplay = reactive(defaultPlayerTweaksGameplay())
const loading = ref(false)
const saving = ref(false)
let requestId = 0

watch(() => props.gameName, async gameName => {
  const current = ++requestId
  Object.assign(gameplay, defaultPlayerTweaksGameplay())
  if (!gameName) return
  loading.value = true
  try {
    const config = await ResourceManager.loadGameConfig(gameName)
    if (current === requestId) Object.assign(gameplay, normalizePlayerTweaksGameplay(config.playerTweaksGameplay))
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
    config.playerTweaksGameplay = normalizePlayerTweaksGameplay({ ...gameplay })
    await ResourceManager.saveGameConfig(gameName, config)
    ElMessage.success(label('运行设置已保存，下次启动游戏生效', 'Runtime settings saved for the next launch'))
  } catch (error) {
    ElMessage.error(String(error))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="tweak-settings">
    <h3>{{ label('运行设置', 'Runtime settings') }}</h3>
    <p class="tweak-hint">{{ label('按当前游戏单独保存；下次通过 SSMT 启动时生效。', 'Saved per game; takes effect on the next SSMT launch.') }}</p>
    <p class="tweak-hint">{{ label('帧率数值只设置游戏内部目标；NVIDIA App 等外部限制或覆盖仍可能决定实际 FPS。', 'The FPS value sets the game target; NVIDIA App and other external controls may determine actual FPS.') }}</p>
    <p v-if="!enabled" class="tweak-hint tweak-hint-warning">{{ label('插件已禁用，这些设置不会注入游戏。', 'The plugin is disabled; these settings will not be injected.') }}</p>
    <p v-if="!gameName" class="tweak-hint tweak-hint-warning">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="tweak-grid" :class="{ loading }">
      <el-checkbox v-model="gameplay.fpsUnlock">{{ label('解锁帧率并关闭 VSync', 'Unlock FPS and disable VSync') }}</el-checkbox>
      <div class="tweak-number">
        <span>{{ label('游戏内部目标帧率（30–240）', 'Game target FPS (30–240)') }}</span>
        <el-input-number v-model="gameplay.targetFps" :min="30" :max="240" :step="1" size="small" :disabled="!gameplay.fpsUnlock" />
      </div>
      <el-checkbox v-model="gameplay.fastTeamPage">{{ label('快速打开队伍页', 'Fast team page') }}</el-checkbox>
    </div>
    <el-button size="small" type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
      {{ label('保存设置', 'Save settings') }}
    </el-button>
  </section>
</template>

<style scoped>
.tweak-settings { border-top: 1px solid var(--market-line); padding-top: 18px; margin-top: 20px; }
.tweak-settings h3 { margin: 0 0 7px; font-size: 13px; font-weight: 650; }
.tweak-hint { margin: 0 0 10px; color: rgba(var(--theme-text-secondary-rgb), .8); font-size: 11px; line-height: 1.5; }
.tweak-hint-warning { color: var(--t-warning-text); }
.tweak-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(235px, 1fr)); gap: 10px 16px; margin: 14px 0 16px; font-size: 12px; }
.tweak-grid.loading { opacity: .5; pointer-events: none; }
.tweak-grid .el-checkbox { margin-right: 0; }
.tweak-number { display: flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 25px; color: rgba(var(--theme-text-secondary-rgb), .9); }
.tweak-number :deep(.el-input-number) { width: 110px; flex: none; }
</style>

