<script setup lang="ts">
import { computed } from 'vue'
import { Lock, Unlock } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { orderedPermissions, permissionDefinitionFor, type PluginPermissionRisk } from './pluginMarketplaceModel'

const props = defineProps<{ permissions: string[] }>()
const { t } = useI18n()

const definitions = computed(() => orderedPermissions(props.permissions).map(permission => ({
  permission,
  definition: permissionDefinitionFor(permission),
})))

const riskLabel = (risk: PluginPermissionRisk) =>
  t(`pluginMarketplace.impact.permissionRisk.${risk}`)
</script>

<template>
  <div v-if="definitions.length" class="permission-cards">
    <article v-for="{ permission, definition } in definitions" :key="permission"
      class="permission-card" :class="`risk-${definition.risk}`">
      <span class="permission-card-icon" aria-hidden="true">
        <Lock v-if="definition.risk !== 'low'" />
        <Unlock v-else />
      </span>
      <span class="permission-card-copy">
        <span class="permission-card-title">
          <strong>{{ t(definition.labelKey) }}</strong>
          <code>{{ permission }}</code>
        </span>
        <small v-if="definition.descriptionKey">{{ t(definition.descriptionKey) }}</small>
      </span>
      <span class="permission-card-risk">{{ riskLabel(definition.risk) }}</span>
    </article>
  </div>
  <span v-else class="muted-copy">{{ t('pluginMarketplace.none') }}</span>
</template>

<style scoped>
.permission-cards {
  display: grid;
  gap: 8px;
}

.permission-card {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 11px;
  border: 1px solid var(--market-line);
  border-radius: 9px;
  background: var(--t-surface-subtle);
}

.permission-card.risk-medium {
  border-color: color-mix(in srgb, var(--t-warning-border) 56%, var(--market-line));
}

.permission-card.risk-high {
  border-color: color-mix(in srgb, var(--t-danger-border) 64%, var(--market-line));
  background: color-mix(in srgb, var(--t-danger-bg) 30%, var(--t-surface-subtle));
}

.permission-card-icon {
  display: grid;
  place-items: center;
  flex: none;
  width: 26px;
  height: 26px;
  border-radius: 7px;
  background: rgba(var(--theme-surface-tint-rgb), .07);
  color: var(--market-muted);
}

.permission-card.risk-medium .permission-card-icon {
  background: var(--t-warning-bg);
  color: var(--t-warning-text);
}

.permission-card.risk-high .permission-card-icon {
  background: var(--t-danger-bg);
  color: var(--t-danger-text);
}

.permission-card-icon > svg {
  width: 14px;
  height: 14px;
}

.permission-card-copy {
  display: grid;
  min-width: 0;
  flex: 1;
  gap: 3px;
}

.permission-card-title {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}

.permission-card-title strong {
  color: var(--t-page-text);
  font-size: 11px;
  font-weight: 600;
}

.permission-card-title code {
  padding: 1px 5px;
  border-radius: 4px;
  background: rgba(var(--theme-surface-tint-rgb), .06);
  color: var(--t-page-text-muted);
  font: 9px/1.55 ui-monospace, Consolas, monospace;
}

.permission-card-copy small {
  color: var(--market-muted);
  font-size: 10px;
  line-height: 1.5;
}

.permission-card-risk {
  flex: none;
  padding-top: 2px;
  color: var(--market-muted);
  font-size: 9px;
  white-space: nowrap;
}

.permission-card.risk-medium .permission-card-risk {
  color: var(--t-warning-text);
}

.permission-card.risk-high .permission-card-risk {
  color: var(--t-danger-text);
}
</style>
