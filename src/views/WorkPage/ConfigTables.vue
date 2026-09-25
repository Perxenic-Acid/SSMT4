<script setup lang="ts">
import { Delete, Rank } from '@element-plus/icons-vue';
import { onBeforeUnmount, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { SkipRow, VSCheckRow } from './WorkPage.types';

const { t } = useI18n();

const skipRows = defineModel<SkipRow[]>('skipRows', { required: true });
const vsRows = defineModel<VSCheckRow[]>('vsRows', { required: true });

const emit = defineEmits<{
  removeSkipRow: [index: number];
  moveSkipRow: [index: number, direction: 'up' | 'down'];
  generateIBSkip: [];
  removeVSCheckRow: [index: number];
  moveVSCheckRow: [index: number, direction: 'up' | 'down'];
  updateVSCheck: [];
  generateVSCheck: [];
}>();

const draggingSkipRow = ref<number | null>(null);
const draggingVSRow = ref<number | null>(null);
const recentlyMovedKind = ref<'skip' | 'vs' | null>(null);
const recentlyMovedRow = ref<number | null>(null);
const skipTableHost = ref<HTMLElement | null>(null);
const vsTableHost = ref<HTMLElement | null>(null);
let dragKind: 'skip' | 'vs' | null = null;
let dragPointerId: number | null = null;
let recentMoveTimer: ReturnType<typeof setTimeout> | undefined;
const rowClass = (kind: 'skip' | 'vs') => ({ rowIndex }: { rowIndex: number }) => {
  const classes: string[] = [];
  const dragging = getDragState(kind).value;
  if (dragging === rowIndex) classes.push('work-row-dragging');
  if (recentlyMovedKind.value === kind && recentlyMovedRow.value === rowIndex) classes.push('work-row-recently-moved');
  return classes.join(' ');
};
const getDragState = (kind: 'skip' | 'vs') => kind === 'skip' ? draggingSkipRow : draggingVSRow;
const rowIndexAtPoint = (host: HTMLElement, clientX: number, clientY: number): number => {
  const row = document.elementFromPoint(clientX, clientY)?.closest('tr');
  if (!row || !host.contains(row)) return -1;
  const body = row.parentElement;
  return body ? Array.from(body.children).indexOf(row) : -1;
};
const stopRowDrag = () => {
  window.removeEventListener('pointermove', moveRowDrag);
  window.removeEventListener('pointerup', stopRowDrag);
  window.removeEventListener('pointercancel', stopRowDrag);
  if (recentMoveTimer) clearTimeout(recentMoveTimer);
  recentlyMovedKind.value = null;
  recentlyMovedRow.value = null;
  draggingSkipRow.value = null;
  draggingVSRow.value = null;
  dragKind = null;
  dragPointerId = null;
};
const moveRowDrag = (event: PointerEvent) => {
  if (!dragKind || dragPointerId !== event.pointerId) return;
  const state = getDragState(dragKind);
  const host = dragKind === 'skip' ? skipTableHost.value : vsTableHost.value;
  if (!host || state.value === null) return;
  event.preventDefault();
  const rowCount = dragKind === 'skip' ? skipRows.value.length : vsRows.value.length;
  const target = Math.min(rowIndexAtPoint(host, event.clientX, event.clientY), rowCount - 2);
  const from = state.value;
  if (target < 0 || from === target || from >= rowCount - 1) return;
  const direction = from < target ? 'down' : 'up';
  if (dragKind === 'skip') emit('moveSkipRow', from, direction);
  else emit('moveVSCheckRow', from, direction);
  recentlyMovedKind.value = dragKind;
  recentlyMovedRow.value = from;
  if (recentMoveTimer) clearTimeout(recentMoveTimer);
  recentMoveTimer = setTimeout(() => {
    recentlyMovedKind.value = null;
    recentlyMovedRow.value = null;
  }, 220);
  state.value = target;
};
const startRowDrag = (kind: 'skip' | 'vs', index: number, event: PointerEvent) => {
  const rowCount = kind === 'skip' ? skipRows.value.length : vsRows.value.length;
  if (event.button !== 0 || index >= rowCount - 1) return;
  event.preventDefault();
  event.stopPropagation();
  dragKind = kind;
  dragPointerId = event.pointerId;
  getDragState(kind).value = index;
  window.addEventListener('pointermove', moveRowDrag, { passive: false });
  window.addEventListener('pointerup', stopRowDrag);
  window.addEventListener('pointercancel', stopRowDrag);
};
onBeforeUnmount(stopRowDrag);
</script>

<template>
  <section class="inner-panel">
    <div ref="skipTableHost" class="table-row">
      <el-table :data="skipRows" border size="small" class="model-table glass-table" :row-class-name="rowClass('skip')">
        <el-table-column width="40" align="center" :resizable="false" class-name="model-table-drag-column">
          <template #default="{ $index }">
            <button type="button" class="row-drag-handle" :class="{ 'is-dragging': draggingSkipRow === $index }" :aria-label="t('workPage.ui.dragRow')" @pointerdown="startRowDrag('skip', $index, $event)"><el-icon><Rank /></el-icon></button>
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.skipIB')" min-width="110" :resizable="false">
          <template #default="{ $index }">
            <el-input
              v-model="skipRows[$index].skipIB"
              :placeholder="t('workPage.placeholders.enterSkipIB')"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.aliasName')" min-width="120" :resizable="false">
          <template #default="{ $index }">
            <el-input
              v-model="skipRows[$index].aliasName"
              :placeholder="t('workPage.placeholders.enterAlias')"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.indexCount')" min-width="90" :resizable="false">
          <template #default="{ $index }">
            <el-input
              v-model="skipRows[$index].indexCount"
              :placeholder="t('workPage.placeholders.enterIndexCount')"
            />
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.firstIndex')" min-width="90" :resizable="false">
          <template #default="{ $index }">
            <el-input
              v-model="skipRows[$index].firstIndex"
              :placeholder="t('workPage.placeholders.enterFirstIndex')"
            />
          </template>
        </el-table-column>
        <el-table-column width="56" align="center" :resizable="false" class-name="model-table-actions-column" label-class-name="model-table-actions-column">
          <template #default="{ $index }">
            <el-tooltip :content="t('workPage.common.delete')" placement="left">
              <button
                type="button"
                class="config-row-delete-btn"
                @click.stop="emit('removeSkipRow', $index)"
              >
                <el-icon><Delete /></el-icon>
              </button>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <div class="controls-row">
      <el-button type="primary" plain @click="emit('generateIBSkip')">
        {{ t('workPage.actions.generateIBSkip') }}
      </el-button>
    </div>
  </section>

  <section class="inner-panel">
    <div ref="vsTableHost" class="table-row">
      <el-table :data="vsRows" border size="small" class="model-table glass-table" :row-class-name="rowClass('vs')">
        <el-table-column width="40" align="center" :resizable="false" class-name="model-table-drag-column">
          <template #default="{ $index }">
            <button type="button" class="row-drag-handle" :class="{ 'is-dragging': draggingVSRow === $index }" :aria-label="t('workPage.ui.dragRow')" @pointerdown="startRowDrag('vs', $index, $event)"><el-icon><Rank /></el-icon></button>
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.enabled')" width="80" min-width="72" align="center" :resizable="false">
          <template #default="{ $index }">
            <el-checkbox v-model="vsRows[$index].enabled" />
          </template>
        </el-table-column>
        <el-table-column :label="t('workPage.columns.vsHash')" min-width="180" :resizable="false">
          <template #default="{ $index }">
            <el-input
              v-model="vsRows[$index].hash"
              :placeholder="t('workPage.placeholders.enterVSHash')"
            />
          </template>
        </el-table-column>
        <el-table-column width="56" align="center" :resizable="false" class-name="model-table-actions-column" label-class-name="model-table-actions-column">
          <template #default="{ $index }">
            <el-tooltip :content="t('workPage.common.delete')" placement="left">
              <button
                type="button"
                class="config-row-delete-btn"
                @click.stop="emit('removeVSCheckRow', $index)"
              >
                <el-icon><Delete /></el-icon>
              </button>
            </el-tooltip>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <div class="controls-row">
      <el-button type="primary" plain @click="emit('updateVSCheck')">
        {{ t('workPage.actions.updateVSCheckList') }}
      </el-button>
      <el-button type="primary" @click="emit('generateVSCheck')">
        {{ t('workPage.actions.generateVSCheck') }}
      </el-button>
    </div>
  </section>
</template>

<style scoped>
.controls-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 16px;
}

.table-row {
  margin-top: 16px;
}

.config-row-delete-btn {
  width: 26px;
  height: 26px;
  border: 1px solid rgba(255, 90, 90, 0.18);
  border-radius: 6px;
  background: rgba(255, 70, 70, 0.055);
  color: rgba(255, 145, 145, 0.78);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.18s ease;
}

.config-row-delete-btn:hover {
  border-color: rgba(255, 105, 105, 0.42);
  background: rgba(255, 75, 75, 0.14);
  color: rgba(255, 215, 215, 0.96);
  box-shadow: 0 0 14px rgba(255, 70, 70, 0.16);
}

.config-row-delete-btn:active {
  transform: scale(0.94);
}

.row-drag-handle {
  width: 22px;
  height: 24px;
  display: inline-grid;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: rgba(var(--theme-text-secondary-rgb), 0.58);
  cursor: grab;
  touch-action: none;
  user-select: none;
}

.row-drag-handle:hover,
.row-drag-handle.is-dragging {
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  color: var(--theme-accent);
}

.row-drag-handle.is-dragging {
  cursor: grabbing;
  opacity: 0.72;
}

.model-table :deep(.work-row-dragging > td) {
  opacity: 0.68;
  transition: opacity 140ms ease, background-color 140ms ease;
}

.model-table :deep(.work-row-recently-moved > td) {
  animation: work-row-shift 220ms ease-out;
}

@keyframes work-row-shift {
  0% { transform: translateY(-4px); background-color: rgba(var(--theme-accent-rgb), 0.15); }
  100% { transform: translateY(0); background-color: transparent; }
}

.inner-panel {
  background: rgba(var(--theme-surface-tint-rgb), 0.022);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.10);
  border-radius: 8px;
  padding: 14px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.06);
  transition: all 0.25s ease;
  position: relative;
  /* 不加 backdrop-filter：inner-panel 高度随表格行数增长，滚动时模糊层会引发合成器闪烁/文字消失 */
}

.inner-panel::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(var(--theme-surface-tint-rgb), 0.16), transparent);
  pointer-events: none;
  border-radius: 8px 8px 0 0;
}
</style>
