<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { ResourceManager } from '../../store/ResourceManager'
import { defaultPlayerTweaksGameplay, normalizePlayerTweaksGameplay } from '../playerTweaksGameplay'

const props = defineProps<{ gameName: string; enabled: boolean; registryFps?: boolean }>()
const { locale } = useI18n()
const label = (chinese: string, english: string) => String(locale.value).startsWith('zh') ? chinese : english
const gameplay = reactive(defaultPlayerTweaksGameplay())
const loading = ref(false)
const saving = ref(false)
let requestId = 0

watch(() => [props.gameName, props.registryFps] as const, async ([gameName]) => {
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
    const updated = normalizePlayerTweaksGameplay({ ...gameplay })
    if (props.registryFps) { updated.targetFps = 120; updated.fastTeamPage = false }
    const config = await ResourceManager.loadGameConfig(gameName)
    config.playerTweaksGameplay = updated
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
  <section class="tweak-settings" :aria-busy="loading || saving">
    <header class="tweak-section-header">
      <div>
        <h3>{{ label('性能与便捷', 'Performance & convenience') }}</h3>
        <p class="tweak-hint">{{ label('按当前游戏保存，下次通过 SSMT 启动时生效。', 'Saved for this game; applies on the next SSMT launch.') }}</p>
      </div>
      <el-button type="primary" :loading="saving" :disabled="!gameName || loading" @click="save">
        {{ label('保存运行设置', 'Save runtime settings') }}
      </el-button>
    </header>
    <p v-if="!enabled" class="tweak-hint tweak-hint-warning">{{ label('插件已禁用，这些设置不会应用到游戏。', 'The plugin is disabled; these settings will not be applied.') }}</p>
    <p v-if="!gameName" class="tweak-hint tweak-hint-warning">{{ label('请先在上方选择游戏。', 'Select a game above first.') }}</p>
    <div v-else class="tweak-groups" :inert="loading || saving">
      <section class="tweak-card" :class="{ 'tweak-card-wide': registryFps }">
        <h4>{{ label('帧率', 'Frame rate') }}</h4>
        <div class="tweak-row">
          <div class="tweak-row-copy">
            <span>{{ registryFps ? label('启用 120 FPS', 'Enable 120 FPS') : label('解锁帧率', 'Unlock frame rate') }}</span>
            <small>{{ registryFps ? label('通过客户端画质设置启用。', 'Apply through client graphics settings.') : label('同时关闭游戏内 VSync。', 'Also disable in-game VSync.') }}</small>
          </div>
          <el-switch v-model="gameplay.fpsUnlock" :aria-label="registryFps ? label('启用 120 FPS', 'Enable 120 FPS') : label('解锁帧率并关闭 VSync', 'Unlock FPS and disable VSync')" />
        </div>
        <div v-if="!registryFps" class="tweak-row" :class="{ 'is-muted': !gameplay.fpsUnlock }">
          <div class="tweak-row-copy"><span>{{ label('游戏内部目标帧率', 'Game target FPS') }}</span><small>30–240 FPS</small></div>
          <el-input-number v-model="gameplay.targetFps" :min="30" :max="240" :step="1" size="small" :disabled="!gameplay.fpsUnlock" :aria-label="label('游戏内部目标帧率', 'Game target FPS')" />
        </div>
        <p class="tweak-card-hint tweak-footnote">{{ registryFps
          ? label('按游戏信息区分国服与国际服。关闭插件或以关闭此选项的设置启动时恢复原 FPS，保留其他画质设置。', 'CN and Global clients are detected from game metadata. Disabling the plugin or launching with this option off restores the original FPS and preserves other graphics settings.')
          : label('NVIDIA App 等外部限制或覆盖仍可能决定实际 FPS。', 'NVIDIA App and other external controls may determine actual FPS.') }}</p>
      </section>
      <section v-if="!registryFps" class="tweak-card">
        <h4>{{ label('便捷操作', 'Convenience') }}</h4>
        <div class="tweak-row">
          <div class="tweak-row-copy"><span>{{ label('快速打开队伍页', 'Fast team page') }}</span><small>{{ label('跳过打开队伍配置时的等待。', 'Skip the wait when opening team setup.') }}</small></div>
          <el-switch v-model="gameplay.fastTeamPage" :aria-label="label('快速打开队伍页', 'Fast team page')" />
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped src="./playerTweaksSettings.css"></style>

