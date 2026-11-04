<script setup lang="ts">
defineProps<{
  open: boolean
  busy?: boolean
}>()

defineEmits<{
  accept: []
  decline: []
}>()
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="lgp-consent-overlay">
      <section class="lgp-consent-dialog" role="dialog" aria-modal="true" aria-labelledby="lgp-consent-title">
        <header class="lgp-consent-header">
          <span class="lgp-consent-kicker">LIQUID GRAVITY PROBE</span>
          <h2 id="lgp-consent-title">使用与免责声明</h2>
          <p>请阅读以下内容后选择是否启用插件。选择“不同意”会停止启用并退出当前入口。</p>
        </header>

        <div class="lgp-consent-body">
          <article>
            <strong>非官方插件</strong>
            <p>本插件是非官方社区工具，与游戏开发商、发行商、平台运营方及任何第三方 Mod 作者均无隶属、授权、合作或背书关系。</p>
          </article>
          <article>
            <strong>会修改 Mod 配置</strong>
            <p>应用喷口、方向、间隔、粒子颜色、粒子池或其他设置时，插件会修改对应 Mod 的 INI、配置和运行时资源。请先自行备份原始 Mod；本插件不保留持久化备份。</p>
          </article>
          <article>
            <strong>使用风险</strong>
            <p>错误配置、游戏更新、第三方 Mod 冲突、权限问题或硬件性能不足，可能导致效果异常、卡顿、闪退、配置损坏或存档相关问题。请先在单人、离线环境测试。</p>
          </article>
          <article>
            <strong>账号与平台规则</strong>
            <p>在在线游戏、竞技环境或受平台规则约束的场景中使用 Mod、第三方插件或成人内容前，请确认相关规则。账号处罚、封禁、内容下架或平台纠纷由使用者承担。</p>
          </article>
          <article>
            <strong>第三方内容</strong>
            <p>插件不保证任何第三方模型、贴图、音频、成人内容或 Mod 的合法性、安全性、版权状态和兼容性。使用者应自行确认来源、授权范围和当地法律要求。</p>
          </article>
          <article>
            <strong>性能与兼容性</strong>
            <p>粒子效果、流体模拟、喷口数量和自定义间隔会增加 GPU/CPU 负载。不同硬件、驱动、游戏版本和 Mod 结构可能产生不同结果。</p>
          </article>
          <article>
            <strong>随缘维护</strong>
            <p>本项目由作者和贡献者在时间允许时维护，不承诺固定更新频率、问题响应时间、兼容性修复、功能请求处理或长期支持。</p>
          </article>
          <article>
            <strong>责任限制</strong>
            <p>本插件按“现状”提供，不作持续兼容、无错误或不中断运行的担保。在适用法律允许的范围内，作者与贡献者不对数据丢失、设备损坏、账号处罚、内容损失或间接损失承担责任。</p>
          </article>
        </div>

        <footer class="lgp-consent-actions">
          <button type="button" class="lgp-consent-button secondary" :disabled="busy" @click="$emit('decline')">
            不同意并停用
          </button>
          <button type="button" class="lgp-consent-button primary" :disabled="busy" @click="$emit('accept')">
            {{ busy ? '正在处理…' : '同意并启用' }}
          </button>
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.lgp-consent-overlay {
  position: fixed;
  inset: 0;
  z-index: 2147483646;
  display: grid;
  place-items: center;
  padding: 24px;
  box-sizing: border-box;
  background: rgba(2, 5, 7, .78);
  backdrop-filter: blur(12px);
}

.lgp-consent-dialog {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) auto;
  width: min(760px, calc(100vw - 48px));
  max-height: min(720px, calc(100vh - 48px));
  overflow: hidden;
  border: 1px solid #34464d;
  border-radius: 10px;
  background: #0b1215;
  color: #edf4f6;
  box-shadow: 0 28px 80px rgba(0, 0, 0, .54);
}

.lgp-consent-header {
  padding: 22px 24px 18px;
  border-bottom: 1px solid #24343a;
  background: linear-gradient(135deg, rgba(216, 233, 78, .07), transparent 42%);
}

.lgp-consent-kicker {
  color: #a9bb55;
  font: 10px/1.4 ui-monospace, Consolas, monospace;
}

.lgp-consent-header h2 {
  margin: 5px 0 8px;
  font-size: 22px;
  letter-spacing: 0;
}

.lgp-consent-header p {
  margin: 0;
  color: #93a6ac;
  font-size: 12px;
  line-height: 1.65;
}

.lgp-consent-body {
  display: grid;
  gap: 12px;
  min-height: 0;
  padding: 18px 24px;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: #53674a transparent;
}

.lgp-consent-body article {
  padding-left: 12px;
  border-left: 2px solid #677832;
}

.lgp-consent-body strong {
  display: block;
  margin-bottom: 3px;
  color: #e9f0cb;
  font-size: 12px;
}

.lgp-consent-body p {
  margin: 0;
  color: #aab8bc;
  font-size: 11px;
  line-height: 1.7;
}

.lgp-consent-actions {
  display: flex;
  justify-content: flex-end;
  gap: 9px;
  padding: 14px 24px 18px;
  border-top: 1px solid #24343a;
  background: #091013;
}

.lgp-consent-button {
  min-width: 132px;
  min-height: 36px;
  padding: 8px 16px;
  border: 1px solid #34464d;
  border-radius: 6px;
  background: #111b1f;
  color: #dbe5e7;
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.lgp-consent-button.primary {
  border-color: #d8e94e;
  background: #d8e94e;
  color: #11180b;
  font-weight: 700;
}

.lgp-consent-button.secondary:hover {
  border-color: #80605d;
  background: #1b1516;
  color: #efaaa4;
}

.lgp-consent-button:disabled {
  opacity: .6;
  cursor: wait;
}

@media (max-width: 560px) {
  .lgp-consent-overlay {
    padding: 12px;
  }

  .lgp-consent-dialog {
    width: calc(100vw - 24px);
    max-height: calc(100vh - 24px);
  }

  .lgp-consent-header,
  .lgp-consent-body {
    padding-right: 16px;
    padding-left: 16px;
  }

  .lgp-consent-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    padding-right: 16px;
    padding-left: 16px;
  }

  .lgp-consent-button {
    min-width: 0;
  }
}
</style>
