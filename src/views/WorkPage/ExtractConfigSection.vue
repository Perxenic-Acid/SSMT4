<script setup lang="ts">
import { FolderOpened, Download, Delete, Document, Rank } from '@element-plus/icons-vue';
import { useI18n } from 'vue-i18n';
import { onBeforeUnmount, ref } from 'vue';
import type { ModelRow } from './WorkPage.types';
import type { FullExtractDataTypeFilter } from './WorkPage.Extract';

const { t } = useI18n();

const selectedFrameAnalysis = defineModel<string>('selectedFrameAnalysis', { default: '' });
const frameAnalysisFolderPath = defineModel<string>('frameAnalysisFolderPath', { default: '' });
const extractPanelTab = defineModel<string>('extractPanelTab', { default: 'drawib' });
const modelRows = defineModel<ModelRow[]>('modelRows', { required: true });
const fullExtractDataTypeFilter = defineModel<FullExtractDataTypeFilter>('fullExtractDataTypeFilter', { default: 'all' });

defineProps<{
  frameAnalysisOptions: string[];
  isRefreshing: boolean;
  isFrameAnalysisPathInvalid: boolean;
  isExtracting: boolean;
  hasLatestExtractionLog: boolean;
  fullExtractDataTypeFilterOptions: Array<{ value: FullExtractDataTypeFilter; labelKey: string }>;
}>();

const emit = defineEmits<{
  refresh: [];
  selectLatest: [];
  analyzeMissingGameTypes: [];
  pickFolder: [];
  openFolder: [];
  dropFolder: [event: DragEvent];
  selectFrameAnalysisOption: [item: string];
  moveModelRow: [index: number, direction: 'up' | 'down'];
  removeModelRow: [index: number];
  extractModels: [];
  fullExtract: [];
  openLatestExtractionLog: [];
}>();

const draggingModelRow = ref<number | null>(null);
const recentlyMovedModelRow = ref<number | null>(null);
const modelTableHost = ref<HTMLElement | null>(null);
let modelDragPointerId: number | null = null;
let recentMoveTimer: ReturnType<typeof setTimeout> | undefined;

const modelRowClass = ({ rowIndex }: { rowIndex: number }) => {
  const classes: string[] = [];
  if (draggingModelRow.value === rowIndex) classes.push('work-row-dragging');
  if (recentlyMovedModelRow.value === rowIndex) classes.push('work-row-recently-moved');
  return classes.join(' ');
};

const rowIndexAtPoint = (host: HTMLElement, clientX: number, clientY: number): number => {
  const row = document.elementFromPoint(clientX, clientY)?.closest('tr');
  if (!row || !host.contains(row)) return -1;
  const body = row.parentElement;
  return body ? Array.from(body.children).indexOf(row) : -1;
};

const stopModelRowDrag = () => {
  window.removeEventListener('pointermove', moveModelRowDrag);
  window.removeEventListener('pointerup', stopModelRowDrag);
  window.removeEventListener('pointercancel', stopModelRowDrag);
  if (recentMoveTimer) clearTimeout(recentMoveTimer);
  recentlyMovedModelRow.value = null;
  draggingModelRow.value = null;
  modelDragPointerId = null;
};

const moveModelRowDrag = (event: PointerEvent) => {
  if (modelDragPointerId !== event.pointerId || draggingModelRow.value === null) return;
  event.preventDefault();
  const from = draggingModelRow.value;
  const to = rowIndexAtPoint(modelTableHost.value!, event.clientX, event.clientY);
  const target = Math.min(to, modelRows.value.length - 2);
  if (target < 0 || from === target || from >= modelRows.value.length - 1) return;
  const direction = from < target ? 'down' : 'up';
  emit('moveModelRow', from, direction);
  recentlyMovedModelRow.value = from;
  if (recentMoveTimer) clearTimeout(recentMoveTimer);
  recentMoveTimer = setTimeout(() => { recentlyMovedModelRow.value = null; }, 220);
  draggingModelRow.value = target;
};

const startModelRowDrag = (index: number, event: PointerEvent) => {
  if (event.button !== 0 || index >= modelRows.value.length - 1) return;
  event.preventDefault();
  event.stopPropagation();
  modelDragPointerId = event.pointerId;
  draggingModelRow.value = index;
  window.addEventListener('pointermove', moveModelRowDrag, { passive: false });
  window.addEventListener('pointerup', stopModelRowDrag);
  window.addEventListener('pointercancel', stopModelRowDrag);
};

onBeforeUnmount(stopModelRowDrag);

</script>

<template>
  <section class="inner-panel extract-panel">
    <div class="controls-row">
      <el-select
        v-model="selectedFrameAnalysis"
        class="fa-select"
        :placeholder="t('workPage.ui.selectFrameAnalysisFolder')"
        filterable
        clearable
      >
        <el-option
          v-for="item in frameAnalysisOptions"
          :key="item"
          :label="item"
          :value="item"
          @click="emit('selectFrameAnalysisOption', item)"
        />
        <template #empty>
          <span class="empty-placeholder">{{ t('workPage.ui.clickRefreshToSync') }}</span>
        </template>
      </el-select>

      <el-button :loading="isRefreshing" plain @click="emit('refresh')">
        <el-icon><RefreshRight /></el-icon>
        {{ t('workPage.actions.refresh') }}
      </el-button>

      <el-button @click="emit('selectLatest')">
        {{ t('workPage.actions.useLatestFrameAnalysisFolder') }}
      </el-button>

      <el-button @click="emit('analyzeMissingGameTypes')">
        {{ t('workPage.actions.analyzeMissingGameTypes') }}
      </el-button>
    </div>

    <div class="controls-row fa-path-row">
      <div class="fa-path-drop-zone" @dragover.prevent @drop.prevent="emit('dropFolder', $event)">
        <el-input
          v-model="frameAnalysisFolderPath"
          :class="{ 'frame-path-invalid': isFrameAnalysisPathInvalid }"
          :placeholder="t('workPage.ui.fullFrameAnalysisPathPlaceholder')"
          clearable
        />
      </div>
      <el-button plain @click="emit('pickFolder')">
        <el-icon><FolderOpened /></el-icon>
        {{ t('workPage.actions.selectFolder') }}
      </el-button>
      <el-button plain @click="emit('openFolder')">
        <el-icon><FolderOpened /></el-icon>
        {{ t('workPage.actions.open') }}
      </el-button>
    </div>
  </section>

  <section class="inner-panel extract-tabs-panel">
    <el-tabs v-model="extractPanelTab" class="extract-tabs">
      <el-tab-pane :label="t('workPage.tabs.extractByDrawIB')" name="drawib">
        <div ref="modelTableHost" class="table-row">
          <el-table :data="modelRows" border size="small" class="model-table glass-table" :row-class-name="modelRowClass">
            <el-table-column width="40" align="center" :resizable="false" class-name="model-table-drag-column">
              <template #default="{ $index }">
                <button
                  type="button"
                  class="row-drag-handle"
                  :class="{ 'is-dragging': draggingModelRow === $index }"
                  :aria-label="t('workPage.ui.dragRow')"
                  @pointerdown="startModelRowDrag($index, $event)"
                ><el-icon><Rank /></el-icon></button>
              </template>
            </el-table-column>
            <el-table-column :label="t('workPage.columns.drawIB')" min-width="130" :resizable="false">
              <template #default="{ $index }">
                <el-input
                  v-model="modelRows[$index].drawIB"
                  :placeholder="t('workPage.placeholders.enterDrawIB')"
                />
              </template>
            </el-table-column>
            <el-table-column :label="t('workPage.columns.aliasName')" min-width="140" :resizable="false">
              <template #default="{ $index }">
                <el-input
                  v-model="modelRows[$index].aliasName"
                  :placeholder="t('workPage.placeholders.enterAlias')"
                />
              </template>
            </el-table-column>
            <el-table-column width="56" align="center" :resizable="false" class-name="model-table-actions-column" label-class-name="model-table-actions-column">
              <template #default="{ $index }">
                <el-tooltip :content="t('workPage.common.delete')" placement="left">
                  <button
                    type="button"
                    class="config-row-delete-btn"
                    @click.stop="emit('removeModelRow', $index)"
                  >
                    <el-icon><Delete /></el-icon>
                  </button>
                </el-tooltip>
              </template>
            </el-table-column>
          </el-table>
        </div>

        <div class="controls-row">
          <el-button type="primary" :loading="isExtracting" @click="emit('extractModels')">
            <el-icon><Download /></el-icon>
            <span>{{ t('workPage.actions.extractModels') }}</span>
          </el-button>
          <el-button v-if="hasLatestExtractionLog" plain @click="emit('openLatestExtractionLog')">
            <el-icon><Document /></el-icon>
            <span>{{ t('workPage.actions.openLatestExtractionLog') }}</span>
          </el-button>
        </div>
      </el-tab-pane>

      <el-tab-pane :label="t('workPage.tabs.fullExtract')" name="full">
        <div class="full-extract-options">
          <span class="full-extract-filter-label">{{ t('workPage.actions.fullExtractDataTypeFilter') }}</span>
          <el-select v-model="fullExtractDataTypeFilter" style="width: 280px;">
            <el-option
              v-for="option in fullExtractDataTypeFilterOptions"
              :key="option.value"
              :label="t(option.labelKey)"
              :value="option.value"
            />
          </el-select>
        </div>
        <div class="controls-row full-extract-row">
          <el-button type="primary" :loading="isExtracting" @click="emit('fullExtract')">
            <el-icon><Download /></el-icon>
            <span>{{ t('workPage.actions.fullExtract') }}</span>
          </el-button>
        </div>
      </el-tab-pane>
    </el-tabs>
  </section>
</template>

<style scoped>
.fa-path-drop-zone {
  flex: 1;
  min-width: 0;
}

.fa-select {
  min-width: 260px;
  max-width: 360px;
}

.fa-path-row :deep(.el-input) {
  flex: 1;
}

:deep(.frame-path-invalid .el-input__wrapper) {
  box-shadow: 0 0 0 1px rgba(245, 108, 108, 0.95) inset;
}

.controls-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 16px;
}

.table-row {
  margin-top: 16px;
}

.extract-option-row {
  display: flex;
  align-items: center;
  margin-top: 12px;
}

.full-extract-options {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 8px;
}

.full-extract-row {
  margin-top: 8px;
}

.full-extract-filter-label {
  color: rgba(var(--theme-text-secondary-rgb), 0.76);
  font-size: 0.92rem;
}

.empty-placeholder {
  color: #8b93a7;
  font-size: 0.9rem;
}

.extract-tabs :deep(.el-tabs__item) {
  --el-color-primary: var(--theme-accent);
  color: rgba(var(--theme-text-secondary-rgb), 0.62);
}

.extract-tabs :deep(.el-tabs__item.is-active),
.extract-tabs :deep(.el-tabs__item:hover) {
  color: var(--theme-accent);
}

.extract-tabs :deep(.el-tabs__active-bar) {
  background-color: var(--theme-accent) !important;
}

.extract-tabs :deep(.el-tabs__nav-wrap::after) {
  background-color: rgba(var(--theme-surface-tint-rgb), 0.12);
}

.model-table :deep(.model-table-actions-column) {
  cursor: default !important;
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
