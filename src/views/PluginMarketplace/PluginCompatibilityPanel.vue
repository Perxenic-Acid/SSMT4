<script setup lang="ts">
import { CircleCheck, CircleClose, InfoFilled, WarningFilled } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import type { PluginCompatibilityItem, PluginCompatibilityState } from './pluginMarketplaceModel'

defineProps<{
  status: 'ready' | 'limited' | 'blocked'
  title: string
  summary: string
  items: PluginCompatibilityItem[]
  checkedAt?: string
  sourceNote: string
}>()

const { t } = useI18n()

const stateLabel = (state: PluginCompatibilityState) =>
  t(`pluginMarketplace.compatibility.state.${state}`)
</script>

<template>
  <section class="compatibility-panel" :class="`is-${status}`" aria-live="polite"
    :aria-label="title">
    <header class="compatibility-head">
      <span class="compatibility-status-icon" aria-hidden="true">
        <CircleClose v-if="status === 'blocked'" />
        <WarningFilled v-else-if="status === 'limited'" />
        <CircleCheck v-else />
      </span>
      <div class="compatibility-heading">
        <div class="compatibility-title-row">
          <h3>{{ title }}</h3>
          <span class="compatibility-status">
            {{ t(`pluginMarketplace.compatibility.status.${status}`) }}
          </span>
        </div>
        <p>{{ summary }}</p>
      </div>
      <time v-if="checkedAt" class="compatibility-time">
        {{ t('pluginMarketplace.compatibility.checkedAt', { time: checkedAt }) }}
      </time>
    </header>

    <div class="compatibility-grid">
      <article v-for="item in items" :key="item.id" class="compatibility-item"
        :class="`is-${item.state}`">
        <span class="compatibility-item-icon" aria-hidden="true">
          <CircleCheck v-if="item.state === 'supported'" />
          <WarningFilled v-else-if="item.state === 'limited'" />
          <CircleClose v-else-if="item.state === 'blocked'" />
          <InfoFilled v-else />
        </span>
        <span class="compatibility-item-copy">
          <span class="compatibility-item-label">{{ item.label }}</span>
          <strong>{{ item.value }}</strong>
          <small v-if="item.hint">{{ item.hint }}</small>
        </span>
        <span class="compatibility-item-state">{{ stateLabel(item.state) }}</span>
      </article>
    </div>

    <footer class="compatibility-foot">
      <InfoFilled aria-hidden="true" />
      <span>{{ sourceNote }}</span>
    </footer>
  </section>
</template>

<style scoped>
.compatibility-panel {
  overflow: hidden;
  margin-top: 15px;
  border: 1px solid var(--market-line);
  border-radius: 12px;
  background: rgba(var(--theme-surface-tint-rgb), .025);
}

.compatibility-panel.is-ready {
  border-color: color-mix(in srgb, var(--t-success-border) 72%, var(--market-line));
}

.compatibility-panel.is-limited {
  border-color: color-mix(in srgb, var(--t-warning-border) 76%, var(--market-line));
}

.compatibility-panel.is-blocked {
  border-color: color-mix(in srgb, var(--t-danger-border) 84%, var(--market-line));
}

.compatibility-head {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 13px 14px;
  border-bottom: 1px solid var(--market-line);
}

.compatibility-panel.is-ready .compatibility-head {
  background: var(--t-success-bg);
  color: var(--t-success-text);
}

.compatibility-panel.is-limited .compatibility-head {
  background: var(--t-warning-bg);
  color: var(--t-warning-text);
}

.compatibility-panel.is-blocked .compatibility-head {
  background: var(--t-danger-bg);
  color: var(--t-danger-text);
}

.compatibility-status-icon,
.compatibility-item-icon {
  display: grid;
  place-items: center;
  flex: none;
}

.compatibility-status-icon {
  width: 30px;
  height: 30px;
  border-radius: 9px;
  background: rgba(255, 255, 255, .07);
}

.compatibility-status-icon > svg {
  width: 17px;
  height: 17px;
}

.compatibility-heading {
  min-width: 0;
  flex: 1;
}

.compatibility-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.compatibility-title-row h3 {
  margin: 0;
  color: var(--t-page-text);
  font-size: 13px;
  font-weight: 650;
}

.compatibility-heading p {
  margin: 4px 0 0;
  color: var(--market-muted);
  font-size: 11px;
  line-height: 1.55;
}

.compatibility-status {
  padding: 2px 7px;
  border: 1px solid currentColor;
  border-radius: 999px;
  font-size: 9px;
  font-weight: 650;
  opacity: .82;
}

.compatibility-time {
  flex: none;
  color: var(--t-page-text-muted);
  font-size: 9px;
  white-space: nowrap;
}

.compatibility-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
  padding: 10px;
}

.compatibility-item {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  min-width: 0;
  padding: 10px;
  border: 1px solid var(--market-line);
  border-radius: 9px;
  background: var(--t-surface-subtle);
}

.compatibility-item.is-limited {
  border-color: color-mix(in srgb, var(--t-warning-border) 62%, var(--market-line));
}

.compatibility-item.is-blocked {
  border-color: color-mix(in srgb, var(--t-danger-border) 66%, var(--market-line));
}

.compatibility-item-icon {
  width: 24px;
  height: 24px;
  border-radius: 7px;
  background: rgba(var(--theme-surface-tint-rgb), .07);
  color: var(--market-muted);
}

.compatibility-item-icon > svg {
  width: 14px;
  height: 14px;
}

.compatibility-item.is-supported .compatibility-item-icon {
  background: var(--t-success-bg);
  color: var(--t-success-text);
}

.compatibility-item.is-limited .compatibility-item-icon {
  background: var(--t-warning-bg);
  color: var(--t-warning-text);
}

.compatibility-item.is-blocked .compatibility-item-icon {
  background: var(--t-danger-bg);
  color: var(--t-danger-text);
}

.compatibility-item-copy {
  display: grid;
  min-width: 0;
  flex: 1;
  gap: 2px;
}

.compatibility-item-label {
  color: var(--t-page-text-muted);
  font-size: 9px;
  font-weight: 650;
  letter-spacing: .04em;
}

.compatibility-item-copy strong {
  overflow: hidden;
  color: var(--t-page-text);
  font-size: 11px;
  font-weight: 600;
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.compatibility-item-copy small {
  display: -webkit-box;
  overflow: hidden;
  color: var(--market-muted);
  font-size: 9px;
  line-height: 1.4;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.compatibility-item-state {
  flex: none;
  color: var(--market-muted);
  font-size: 9px;
  white-space: nowrap;
}

.compatibility-item.is-supported .compatibility-item-state {
  color: var(--t-success-text);
}

.compatibility-item.is-limited .compatibility-item-state {
  color: var(--t-warning-text);
}

.compatibility-item.is-blocked .compatibility-item-state {
  color: var(--t-danger-text);
}

.compatibility-foot {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  padding: 9px 14px 11px;
  border-top: 1px solid var(--market-line);
  color: var(--t-page-text-muted);
  font-size: 9px;
  line-height: 1.5;
}

.compatibility-foot > svg {
  flex: none;
  width: 13px;
  height: 13px;
  margin-top: 1px;
}

@container plugin-marketplace (max-width: 1080px) {
  .compatibility-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .compatibility-time {
    display: none;
  }
}
</style>
