<script setup lang="ts">
import { computed, ref } from 'vue';
import { ArrowDown, Edit, Plus, FolderAdd, FolderOpened, BrushFilled, Delete, UploadFilled, Download, Connection } from '@element-plus/icons-vue';
import { useI18n } from 'vue-i18n';
import type { DrawerCollapsedState } from './WorkPage.types';

const { t } = useI18n();

const workspaceDraftName = defineModel<string>('workspaceDraftName', { required: true });
const drawerCollapsed = defineModel<DrawerCollapsedState>('drawerCollapsed', { required: true });
const useSpecificIbDump = defineModel<boolean>('useSpecificIbDump', { default: false });

const props = defineProps<{
  workspaceName: string;
  workspaceOptions: string[];
  workspaceModifiedTimes: Record<string, number>;
  workspaceProvenance: { attribution: string; uploadedAt: string; aliases: string[] } | null;
  isSpecificIbDumpToggling: boolean;
  workspaceUploadProgress: number;
  workspaceUploadActive: boolean;
}>();

type WorkspaceSortMode = 'date' | 'name';
const workspaceSortMode = ref<WorkspaceSortMode>('date');
const sortedWorkspaceOptions = computed(() => [...props.workspaceOptions].sort((a, b) => {
  if (workspaceSortMode.value === 'date') {
    const dateDifference = (props.workspaceModifiedTimes[b] ?? 0) - (props.workspaceModifiedTimes[a] ?? 0);
    if (dateDifference !== 0) return dateDifference;
  }
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
}));

const emit = defineEmits<{
  createWorkspace: [];
  createFromConfig: [];
  openWorkspace: [];
  openGeneratedMod: [];
  clearWorkspace: [];
  deleteWorkspace: [name: string];
  selectWorkspace: [name: string];
  folderMenu: [cmd: string];
  textureMenu: [cmd: unknown];
  openWorkspaceUpload: [];
  openWorkspaceDownload: [];
  openWorkspaceMerge: [];
  specificIbDumpToggle: [value: string | number | boolean];
}>();

const toggleDrawer = (key: keyof DrawerCollapsedState) => {
  drawerCollapsed.value = {
    ...drawerCollapsed.value,
    [key]: !drawerCollapsed.value[key],
  };
};
</script>

<template>
  <aside class="side-column glass-scrollbar">
    <div class="workspace-controls">
      <section class="side-drawer" :class="{ 'is-collapsed': drawerCollapsed.workspace }">
        <button
          type="button"
          class="side-drawer-head"
          @click="toggleDrawer('workspace')"
        >
          <el-icon class="side-drawer-arrow"><ArrowDown /></el-icon>
          <span>工作空间</span>
        </button>
        <div v-show="!drawerCollapsed.workspace" class="side-drawer-body">
          <div class="workspace-card">
          <el-input
            v-model="workspaceDraftName"
            class="workspace-inline-input"
            :placeholder="t('workPage.ui.selectOrEnterWorkspace')"
            clearable
            :prefix-icon="Edit"
            @keyup.enter="emit('createWorkspace')"
          />

          <div class="workspace-action-strip workspace-action-strip--management">
            <el-tooltip :content="t('workPage.actions.createWorkspace')" placement="top">
              <button
                type="button"
                class="workspace-inline-action workspace-inline-action--primary"
                :class="{ 'is-disabled': !workspaceDraftName }"
                @click="workspaceDraftName && emit('createWorkspace')"
              >
                <el-icon><Plus /></el-icon>
              </button>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.createWorkspaceFromCurrentDrawIB')" placement="top">
              <button
                type="button"
                class="workspace-inline-action workspace-inline-action--secondary"
                :class="{ 'is-disabled': !workspaceDraftName }"
                @click="workspaceDraftName && emit('createFromConfig')"
              >
                <el-icon><FolderAdd /></el-icon>
              </button>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.openWorkspaceFolder')" placement="top">
              <div class="workspace-icon-btn" role="button" tabindex="0" @click="emit('openWorkspace')">
                <el-icon><FolderOpened /></el-icon>
              </div>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.openGeneratedModFolder')" placement="top">
              <div
                class="workspace-icon-btn workspace-icon-btn--generated"
                :class="{ 'is-disabled': !workspaceName }"
                role="button"
                tabindex="0"
                @click="workspaceName && emit('openGeneratedMod')"
              >
                <el-icon><FolderAdd /></el-icon>
              </div>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.clearWorkspace')" placement="top">
              <div
                class="workspace-icon-btn workspace-icon-btn--warning"
                :class="{ 'is-disabled': !workspaceName }"
                role="button"
                tabindex="0"
                @click="workspaceName && emit('clearWorkspace')"
              >
                <el-icon class="workspace-clean-icon"><BrushFilled /></el-icon>
              </div>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.deleteWorkspace')" placement="top">
              <button
                type="button"
                class="workspace-inline-action workspace-inline-action--danger"
                :class="{ 'is-disabled': !workspaceDraftName }"
                @click="workspaceDraftName && emit('deleteWorkspace', workspaceDraftName)"
              >
                <el-icon><Delete /></el-icon>
              </button>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.openWorkspaceUpload')" placement="bottom">
              <button
                type="button"
                class="workspace-inline-action workspace-inline-action--library"
                :class="{ 'is-disabled': !workspaceName, 'is-uploading': workspaceUploadActive }"
                :disabled="workspaceUploadActive"
                @click="workspaceName && emit('openWorkspaceUpload')"
              >
                <span
                  v-if="workspaceUploadActive"
                  class="workspace-upload-progress-fill"
                  :style="{ height: `${Math.max(4, workspaceUploadProgress)}%` }"
                />
                <el-icon class="workspace-inline-action-icon"><UploadFilled /></el-icon>
              </button>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.openWorkspaceDownload')" placement="bottom">
              <button type="button" class="workspace-inline-action workspace-inline-action--library" @click="emit('openWorkspaceDownload')">
                <el-icon><Download /></el-icon>
              </button>
            </el-tooltip>
            <el-tooltip :content="t('workPage.actions.mergeWorkspaces')" placement="bottom">
              <button type="button" class="workspace-inline-action workspace-inline-action--secondary" @click="emit('openWorkspaceMerge')">
                <el-icon><Connection /></el-icon>
              </button>
            </el-tooltip>
          </div>
          <p v-if="workspaceProvenance" class="workspace-provenance">
            {{ t('workPage.ui.workspaceLibrarySource', { attribution: workspaceProvenance.attribution, uploadedAt: workspaceProvenance.uploadedAt }) }}
          </p>
          </div>
        </div>
      </section>

      <section class="side-drawer" :class="{ 'is-collapsed': drawerCollapsed.workspaceSelector }">
        <button
          type="button"
          class="side-drawer-head"
          @click="toggleDrawer('workspaceSelector')"
        >
          <el-icon class="side-drawer-arrow"><ArrowDown /></el-icon>
          <span>选择工作空间</span>
        </button>
        <div v-show="!drawerCollapsed.workspaceSelector" class="side-drawer-body">
          <div class="workspace-sort-control" role="group" :aria-label="t('workPage.ui.sortWorkspaces')">
            <button
              type="button"
              class="workspace-sort-option"
              :class="{ 'is-active': workspaceSortMode === 'date' }"
              @click="workspaceSortMode = 'date'"
            >
              {{ t('workPage.ui.sortByNewest') }}
            </button>
            <button
              type="button"
              class="workspace-sort-option"
              :class="{ 'is-active': workspaceSortMode === 'name' }"
              @click="workspaceSortMode = 'name'"
            >
              {{ t('workPage.ui.sortByName') }}
            </button>
          </div>
          <div class="workspace-list glass-scrollbar">
            <button
              v-for="name in sortedWorkspaceOptions"
              :key="name"
              type="button"
              class="workspace-list-item"
              :class="{ 'is-active': name === workspaceName }"
              @click="emit('selectWorkspace', name)"
            >
              <span class="workspace-list-name">{{ name }}</span>
            </button>
          </div>
        </div>
      </section>

      <section class="side-drawer specific-ib-dump-drawer">
        <div class="side-drawer-body">
          <div class="specific-ib-dump-toggle">
            <div class="specific-ib-dump-toggle__header">
              <span>{{ t('workPage.actions.specificIbDump') }}</span>
              <el-switch
                v-model="useSpecificIbDump"
                :loading="isSpecificIbDumpToggling"
                style="--el-switch-on-color: var(--theme-success);"
                @change="(val: string | number | boolean) => emit('specificIbDumpToggle', val)"
              />
            </div>
            <p class="specific-ib-dump-toggle__hint">{{ t('workPage.ui.specificIbDumpHint') }}</p>
          </div>
        </div>
      </section>

      <section class="side-drawer" :class="{ 'is-collapsed': drawerCollapsed.commonFolders }">
        <button
          type="button"
          class="side-drawer-head"
          @click="toggleDrawer('commonFolders')"
        >
          <el-icon class="side-drawer-arrow"><ArrowDown /></el-icon>
          <span>常用位置</span>
        </button>
        <div v-show="!drawerCollapsed.commonFolders" class="side-drawer-body">
          <div class="side-menu-list">
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'migoto')">
              <span>{{ t('workPage.menu.open3DmigotoFolder') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'mods')">
              <span>{{ t('workPage.menu.openModsFolder') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'latestFA')">
              <span>{{ t('workPage.menu.openLatestFrameAnalysisFolder') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'latestFALog')">
              <span>{{ t('workPage.menu.openLatestFrameAnalysisLog') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'latestFADeduped')">
              <span>{{ t('workPage.menu.openDedupedFolderInLatestFrameAnalysis') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('folderMenu', 'ssmt4GlobalConfigs')">
              <span>{{ t('workPage.menu.openSSMT4GlobalConfigsFolder') }}</span>
            </button>
          </div>
        </div>
      </section>

      <section class="side-drawer" :class="{ 'is-collapsed': drawerCollapsed.textureExtract }">
        <button
          type="button"
          class="side-drawer-head"
          @click="toggleDrawer('textureExtract')"
        >
          <el-icon class="side-drawer-arrow"><ArrowDown /></el-icon>
          <span>贴图提取</span>
        </button>
        <div v-show="!drawerCollapsed.textureExtract" class="side-drawer-body">
          <div class="side-menu-list">
            <button type="button" class="side-menu-trigger" @click="emit('textureMenu', 'allTextures')">
              <span>{{ t('workPage.menu.extractAllTextureTypes') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('textureMenu', 'dedupedTextures')">
              <span>{{ t('workPage.menu.extractDedupedTextures') }}</span>
            </button>
            <button type="button" class="side-menu-trigger" @click="emit('textureMenu', 'trianglelistTextures')">
              <span>{{ t('workPage.menu.extractTrianglelistTextures') }}</span>
            </button>
          </div>
        </div>
      </section>

    </div>
  </aside>
</template>

<style scoped>
.side-column {
  min-width: 0;
  min-height: 0;
  height: 100%;
  max-height: 100%;
  display: flex;
  gap: 14px;
  align-self: stretch;
  position: sticky;
  top: 0;
  overflow-y: auto;
  overflow-x: visible;
  padding-right: 4px;
}

.workspace-controls {
  display: flex;
  flex-direction: column;
  gap: 10px;
  width: 100%;
  min-height: max-content;
  overflow: visible;
  backdrop-filter: blur(2px);
  -webkit-backdrop-filter: blur(2px);
}

.side-drawer {
  flex: 0 0 auto;
  border-radius: 10px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  background: rgba(255, 255, 255, 0.025);
  overflow: hidden;
}

.side-drawer-head {
  width: 100%;
  min-height: 40px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  border: 0;
  background: transparent;
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  font-size: 13px;
  font-weight: 700;
  text-align: left;
  cursor: pointer;
}

.side-drawer-head:hover {
  background: rgba(255, 255, 255, 0.04);
}

.side-drawer-arrow {
  flex: 0 0 auto;
  transition: transform 0.18s ease;
  color: rgba(var(--theme-surface-tint-rgb), 0.78);
}

.side-drawer.is-collapsed .side-drawer-arrow {
  transform: rotate(-90deg);
}

.side-drawer-body {
  padding: 10px 16px 16px;
}

.side-drawer-body > * + * {
  margin-top: 12px;
}

.side-menu-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.workspace-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  overflow: visible;
}

.workspace-icon-btn {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  box-sizing: border-box;
  padding: 0;
  margin-left: 0;
  border-radius: 8px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.18);
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
  color: rgba(var(--theme-surface-tint-rgb), 0.78);
  cursor: pointer;
  user-select: none;
  transition: all 0.2s ease;
}

.workspace-clean-icon {
  transform: rotate(135deg);
}

.workspace-icon-btn:hover {
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.34);
  color: var(--theme-accent);
  transform: translateY(-2px);
  box-shadow: 0 6px 18px rgba(var(--theme-surface-tint-rgb), 0.12);
}

.workspace-action-strip {
  display: grid;
  grid-template-columns: repeat(4, 32px);
  gap: 6px;
  justify-content: start;
  padding: 3px 0 0;
  overflow: visible;
}

.workspace-action-strip--management {
  grid-template-columns: repeat(6, 32px);
  padding-top: 0;
}

.workspace-inline-input {
  width: 100%;
  display: block;
  margin-bottom: 8px;
}

.workspace-inline-input :deep(.el-input__wrapper) {
  min-height: 34px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.045);
  box-shadow: 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.10) inset;
}

.workspace-inline-input :deep(.el-input__wrapper:hover),
.workspace-inline-input :deep(.el-input__wrapper.is-focus) {
  box-shadow: 0 0 0 1px rgba(var(--theme-surface-tint-rgb), 0.32) inset;
}

.workspace-inline-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.workspace-icon-btn--generated {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.22);
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: rgba(var(--theme-surface-tint-rgb), 0.86);
}

.workspace-icon-btn--generated:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.40);
  background: rgba(var(--theme-surface-tint-rgb), 0.14);
  color: var(--theme-accent);
  box-shadow: 0 6px 18px rgba(var(--theme-surface-tint-rgb), 0.14);
}

.workspace-icon-btn--warning {
  border-color: rgba(230, 162, 60, 0.24);
  background: rgba(230, 162, 60, 0.10);
  color: rgba(255, 205, 110, 0.94);
}

.workspace-icon-btn--warning:hover {
  border-color: rgba(230, 162, 60, 0.42);
  background: rgba(230, 162, 60, 0.18);
  color: rgba(255, 224, 145, 1);
  box-shadow: 0 6px 18px rgba(230, 162, 60, 0.16);
}

.workspace-icon-btn--danger {
  border-color: rgba(245, 108, 108, 0.42);
  background: rgba(245, 108, 108, 0.18);
  color: rgba(255, 178, 178, 1);
}

.workspace-icon-btn--danger:hover {
  border-color: rgba(245, 108, 108, 0.68);
  background: rgba(245, 108, 108, 0.28);
  color: rgba(255, 220, 220, 1);
  box-shadow: 0 6px 18px rgba(245, 108, 108, 0.24);
}

.workspace-icon-btn.is-disabled,
.workspace-icon-btn.is-disabled:hover {
  opacity: 0.35;
  cursor: not-allowed;
  transform: none;
}

.workspace-inline-action {
  position: relative;
  overflow: hidden;
  width: 34px;
  height: 34px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border-radius: 8px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.18);
  background: rgba(var(--theme-surface-tint-rgb), 0.07);
  color: rgba(var(--theme-surface-tint-rgb), 0.82);
  cursor: pointer;
  transition: all 0.2s ease;
}

.workspace-inline-action-icon {
  position: relative;
  z-index: 1;
}

.workspace-upload-progress-fill {
  position: absolute;
  z-index: 0;
  inset: auto 0 0;
  background: linear-gradient(180deg, rgba(92, 205, 255, 0.82), rgba(71, 133, 255, 0.72));
  transition: height 0.24s ease-out;
  pointer-events: none;
}

.workspace-inline-action.is-uploading {
  cursor: progress;
  color: rgba(245, 252, 255, 1);
  border-color: rgba(104, 204, 255, 0.58);
}

.workspace-inline-action:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.34);
  background: rgba(var(--theme-surface-tint-rgb), 0.12);
  color: var(--theme-accent);
  transform: translateY(-1px);
}

.workspace-inline-action--primary {
  border-color: rgba(64, 158, 255, 0.32);
  background: rgba(64, 158, 255, 0.10);
  color: rgba(160, 211, 255, 0.96);
}

.workspace-inline-action--primary:hover {
  border-color: rgba(64, 158, 255, 0.48);
  background: rgba(64, 158, 255, 0.18);
  color: rgba(215, 238, 255, 1);
}

.workspace-inline-action--secondary {
  border-color: rgba(255, 255, 255, 0.10);
  background: rgba(255, 255, 255, 0.04);
  color: rgba(var(--theme-text-secondary-rgb), 0.74);
}

.workspace-inline-action--secondary:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.24);
  background: rgba(var(--theme-surface-tint-rgb), 0.08);
  color: rgba(var(--theme-text-secondary-rgb), 0.94);
}

.workspace-inline-action--library {
  border-color: rgba(177, 121, 255, 0.56);
  background: linear-gradient(135deg, rgba(137, 83, 224, 0.52), rgba(96, 57, 173, 0.46));
  color: rgba(244, 233, 255, 0.98);
}

.workspace-inline-action--library:hover {
  border-color: rgba(209, 172, 255, 0.84);
  background: linear-gradient(135deg, rgba(156, 101, 244, 0.70), rgba(111, 68, 199, 0.62));
  color: #fff;
  box-shadow: 0 6px 18px rgba(124, 72, 210, 0.28);
}

.workspace-inline-action--danger {
  border-color: rgba(245, 108, 108, 0.34);
  background: rgba(245, 108, 108, 0.12);
  color: rgba(255, 188, 188, 0.98);
}

.workspace-inline-action--danger:hover {
  border-color: rgba(245, 108, 108, 0.54);
  background: rgba(245, 108, 108, 0.22);
  color: rgba(255, 226, 226, 1);
}

.workspace-inline-action.is-disabled,
.workspace-inline-action.is-disabled:hover {
  opacity: 0.36;
  cursor: not-allowed;
  transform: none;
}

.workspace-list {
  display: flex;
  flex-direction: column;
  gap: 5px;
  max-height: 220px;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 4px 2px;
  border-radius: 8px;
  border: 0;
  background: transparent;
}

.workspace-list-item {
  box-sizing: border-box;
  width: 100%;
  min-height: 34px;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.045);
  color: rgba(232, 236, 245, 0.82);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  transition: all 0.18s ease;
}

.workspace-list-name {
  min-width: 0;
  flex: 1 1 auto;
  overflow-wrap: anywhere;
  word-break: break-word;
  white-space: normal;
  line-height: 1.35;
}

.workspace-list-item:hover {
  background: rgba(255, 255, 255, 0.075);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.40);
  color: rgba(var(--theme-text-primary-rgb), 0.94);
}

.workspace-list-item.is-active {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.58);
  background: rgba(var(--theme-surface-tint-rgb), 0.14);
  color: #b8e5ff;
}

.side-menu-trigger {
  width: 100%;
  min-height: 34px;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 0;
  padding: 8px 12px;
  border-radius: 4px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.14);
  background: rgba(var(--theme-surface-tint-rgb), 0.045);
  color: rgba(var(--theme-text-secondary-rgb), 0.76);
  font-size: 12px;
  font-weight: 700;
  line-height: 1.35;
  box-sizing: border-box;
  cursor: pointer;
  user-select: none;
  transition: all 0.2s ease;
  white-space: normal;
  text-align: left;
}

.side-menu-trigger span {
  min-width: 0;
  overflow: visible;
  text-overflow: clip;
  white-space: normal;
  word-break: break-word;
  overflow-wrap: anywhere;
}

.side-menu-trigger:hover {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.45);
  background: rgba(var(--theme-surface-tint-rgb), 0.10);
  color: #a9dfff;
}

.specific-ib-dump-toggle {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border-radius: 8px;
  background: rgba(var(--theme-surface-tint-rgb), 0.035);
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.06);
  position: relative;
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
}

.specific-ib-dump-toggle::before {
  display: none;
}

.workspace-provenance {
  margin: 8px 0 0;
  color: var(--work-crystal-text, rgba(255, 255, 255, 0.7));
  font-size: 12px;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.workspace-sort-control {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  margin-bottom: 8px;
  padding: 3px;
  box-sizing: border-box;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.12);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.025);
}

.workspace-sort-option {
  flex: 1 1 0;
  min-width: 0;
  min-height: 27px;
  padding: 4px 6px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: rgba(var(--theme-text-secondary-rgb), 0.64);
  font: inherit;
  font-size: 12px;
  line-height: 1.2;
  cursor: pointer;
  transition: color 0.18s ease, background 0.18s ease, border-color 0.18s ease;
}

.workspace-sort-option:hover {
  color: rgba(var(--theme-text-primary-rgb), 0.92);
  background: rgba(var(--theme-surface-tint-rgb), 0.06);
}

.workspace-sort-option.is-active {
  border-color: rgba(var(--theme-surface-tint-rgb), 0.22);
  background: rgba(var(--theme-surface-tint-rgb), 0.10);
  color: var(--theme-accent);
  box-shadow: 0 3px 10px rgba(0, 0, 0, 0.10);
}

.specific-ib-dump-toggle__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 0.94rem;
  color: rgba(var(--theme-text-secondary-rgb), 0.86);
}

.specific-ib-dump-toggle__hint {
  margin: 0;
  font-size: 0.8rem;
  line-height: 1.4;
  color: rgba(255, 255, 255, 0.58);
}
</style>
