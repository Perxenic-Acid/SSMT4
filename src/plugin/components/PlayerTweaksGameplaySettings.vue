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
  <section class="gameplay-settings">
    <h3>{{ label('运行设置', 'Runtime settings') }}</h3>
    <p class="gameplay-hint">{{ label('按当前游戏单独保存；下次通过 SSMT 启动时生效。', 'Saved per game; takes effect on the next SSMT launch.') }}</p>
    <p class="gameplay-hint">{{ label('帧率数值只设置游戏内部目标；NVIDIA App 等外部限制或覆盖仍可能决定实际 FPS。', 'The FPS value sets the game target; NVIDIA App and other external controls may determine actual FPS.') }}</p>
    <p v-if="!enabled" class="gameplay-hint">{{ label('插件已禁用，这些设置不会注入游戏。', 'The plugin is disabled; these settings will not be injected.') }}</p>
    <p v-if="!gameName" class="gameplay-hint">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="gameplay-grid" :class="{ loading }">
      <label><input v-model="gameplay.fpsUnlock" type="checkbox">{{ label('解锁帧率并关闭 VSync', 'Unlock FPS and disable VSync') }}</label>
      <label class="number-field">{{ label('游戏内部目标帧率（30–240）', 'Game target FPS (30–240)') }}
        <el-input-number v-model="gameplay.targetFps" :min="30" :max="240" :step="1" size="small" :disabled="!gameplay.fpsUnlock" />
      </label>
      <label><input v-model="gameplay.fastTeamPage" type="checkbox">{{ label('快速打开队伍页', 'Fast team page') }}</label>
    </div>
    <el-button size="small" type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
      {{ label('保存设置', 'Save settings') }}
    </el-button>
  </section>
</template>

<style scoped>
.gameplay-settings { border-top: 1px solid var(--market-line); padding-top: 16px; }
.gameplay-settings h3 { margin: 0 0 7px; font-size: 13px; }
.gameplay-hint { margin: 0 0 10px; color: rgba(var(--theme-text-secondary-rgb), .8); font-size: 11px; line-height: 1.5; }
.gameplay-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(215px, 1fr)); gap: 10px 16px; margin: 12px 0 16px; font-size: 12px; }
.gameplay-grid.loading { opacity: .5; pointer-events: none; }
.gameplay-grid label { display: flex; align-items: center; gap: 7px; min-height: 25px; }
.gameplay-grid input[type='checkbox'] { accent-color: var(--t-page-accent); }
.gameplay-grid .number-field { justify-content: space-between; gap: 10px; }
.gameplay-grid :deep(.el-input-number) { width: 110px; flex: none; }
</style>
