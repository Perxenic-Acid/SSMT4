<script setup lang="ts">
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { useI18n } from 'vue-i18n';

defineProps<{
  /** '' = 未选择；'default' = 使用安装目录内默认位置；'custom' = 自定义位置 */
  mode: '' | 'default' | 'custom';
  /** 安装目录下的默认缓存路径（由父组件解析一次） */
  defaultPath: string;
  /** 用户通过浏览对话框选择的自定义路径 */
  customPath: string;
}>();

const emit = defineEmits<{
  (e: 'update:mode', value: '' | 'default' | 'custom'): void;
  (e: 'update:customPath', value: string): void;
}>();

const { t } = useI18n();

const chooseCustomPath = async () => {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    title: t('firstRun.fields.cacheDir'),
  });
  if (typeof selected === 'string' && selected.trim()) {
    emit('update:customPath', selected.trim());
    emit('update:mode', 'custom');
  }
};
</script>

<template>
  <div class="cache-folder-picker">
    <div class="cache-picker-grid">
      <button type="button" class="cache-picker-option"
        :class="{ 'is-selected': mode === 'default' }" @click="emit('update:mode', 'default')">
        <strong>{{ t('firstRun.cacheDefaultLabel') }}</strong>
        <span class="cache-picker-path">{{ defaultPath || '—' }}</span>
        <span class="cache-picker-hint">{{ t('firstRun.cacheDefaultHint') }}</span>
        <i v-if="mode === 'default'" class="cache-picker-check"></i>
      </button>
      <button type="button" class="cache-picker-option"
        :class="{ 'is-selected': mode === 'custom' }" @click="emit('update:mode', 'custom')">
        <strong>{{ t('firstRun.cacheCustomLabel') }}</strong>
        <span class="cache-picker-path">{{ customPath || t('firstRun.cacheCustomEmpty') }}</span>
        <span class="cache-picker-hint">{{ t('firstRun.cacheCustomHint') }}</span>
        <i v-if="mode === 'custom'" class="cache-picker-check"></i>
      </button>
    </div>
    <div v-if="mode === 'custom'" class="cache-picker-path-row">
      <el-input :model-value="customPath" readonly :placeholder="t('firstRun.cacheCustomEmpty')" />
      <el-button @click="chooseCustomPath">{{ t('firstRun.choose') }}</el-button>
    </div>
  </div>
</template>

<style scoped>
.cache-picker-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

.cache-picker-option {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
  padding: 14px;
  text-align: left;
  border: 1px solid rgba(255, 255, 255, .1);
  border-radius: 12px;
  background: rgba(255, 255, 255, .045);
  color: inherit;
  cursor: pointer;
  transition: border-color .18s ease, background .18s ease;
  font: inherit;
}

.cache-picker-option:hover {
  border-color: rgba(var(--ssmt-brand-accent-rgb), .45);
}

.cache-picker-option.is-selected {
  border-color: rgba(var(--ssmt-brand-accent-rgb), .85);
  background: rgba(var(--ssmt-brand-accent-rgb), .09);
}

.cache-picker-option strong {
  font-size: 14px;
  color: #fff;
}

.cache-picker-path {
  font-size: 12px;
  line-height: 1.5;
  word-break: break-all;
  color: rgba(var(--ssmt-brand-accent-rgb), .85);
  background: rgba(0, 0, 0, .28);
  border-radius: 8px;
  padding: 8px 10px;
}

.cache-picker-hint {
  font-size: 12px;
  line-height: 1.5;
  color: rgba(255, 255, 255, .52);
}

.cache-picker-check {
  position: absolute;
  top: 10px;
  right: 12px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--ssmt-brand-accent);
  box-shadow: 0 0 8px rgba(var(--ssmt-brand-accent-rgb), .9);
}

.cache-picker-path-row {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}

.cache-picker-path-row .el-input {
  flex: 1;
}
</style>
