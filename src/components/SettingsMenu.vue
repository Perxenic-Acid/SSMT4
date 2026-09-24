<script setup lang="ts">
import { onUnmounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { ElMessage } from 'element-plus';
import { diagnoseDllLocks } from '../utils/FileLocks';
import { PathHelper } from '../helper/PathHelper';
import { MigotoManager } from '../store/MigotoManager';
import { AppStateManager } from '../store/AppStateManager';
import type { D3d11Mode } from '../store/GameConfig';

defineProps<{
  showSettings: boolean;
  isCurrentPresetNtemi: boolean;
}>();

const emit = defineEmits<{
  (e: 'update:showSettings', value: boolean): void;
  (e: 'switchD3d11Mode', mode: D3d11Mode): void;
  (e: 'open3dmigotoFolder'): void;
  (e: 'openD3dxIni'): void;
  (e: 'checkD3D11DllUpdate'): void;
  (e: 'check3DMigotoPackageUpdate'): void;
}>();

const { t } = useI18n();
const appSettings = AppStateManager.appSettings;

// --- Dropdown positioning fix: use position:fixed to avoid overflow clipping ---
const settingsBtnRef = ref<HTMLElement | null>(null);
const isDropdownVisible = ref(false);
const dropdownStyles = ref<Record<string, string>>({});

const updateDropdownPosition = () => {
  const el = settingsBtnRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  // Position above the trigger button, aligned to the right
  dropdownStyles.value = {
    position: 'fixed',
    bottom: `${window.innerHeight - rect.top + 8}px`,
    right: `${window.innerWidth - rect.right}px`,
  };
};

let hideTimer: ReturnType<typeof setTimeout> | null = null;

const clearHideTimer = () => {
  if (hideTimer !== null) {
    clearTimeout(hideTimer);
    hideTimer = null;
  }
};

const showDropdown = () => {
  clearHideTimer();
  updateDropdownPosition();
  isDropdownVisible.value = true;
};

const hideDropdown = () => {
  // Delayed hide so user can move from button to menu (teleported to body)
  clearHideTimer();
  hideTimer = setTimeout(() => {
    isDropdownVisible.value = false;
    hideTimer = null;
  }, 200);
};

const onDropdownEnter = () => {
  clearHideTimer();
  isDropdownVisible.value = true;
};

const onDropdownLeave = () => {
  hideDropdown();
};

const diagnosingLocks = ref(false);
const checkDllLocks = async () => {
  if (diagnosingLocks.value) return;
  diagnosingLocks.value = true;
  isDropdownVisible.value = false;
  try {
    const directory = await PathHelper.GetCurrentGame3DmigotoFolderPath();
    if (!directory) throw new Error(t('fileLocks.configure'));
    await diagnoseDllLocks(directory);
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') ElMessage.error(String(error));
  } finally {
    diagnosingLocks.value = false;
  }
};

const toggleSymlink = async (enable: boolean) => {
  const gameName = appSettings.CurrentGameName?.trim();
  if (!gameName || gameName === 'Default') return;

  try {
    await MigotoManager.toggleSymlinkOption(gameName, enable);
    ElMessage.success(enable ? t('home.messages.symlinkEnabled') : t('home.messages.symlinkDisabled'));
  } catch (e) {
    console.error('Failed to toggle symlink:', e);
    ElMessage.error(t('home.messages.operationFailed', { error: String(e) }));
  }
};

// switchD3d11Mode is handled by the parent Home.vue via emit('switchD3d11Mode', mode)

onUnmounted(() => { clearHideTimer() })

</script>

<template>
  <div
    class="settings-menu-wrapper"
    @mouseenter="showDropdown"
    @mouseleave="hideDropdown"
  >
    <!-- Trigger button -->
    <div ref="settingsBtnRef" class="settings-btn">
      <div class="menu-lines">
        <div class="line"></div>
        <div class="line"></div>
        <div class="line"></div>
      </div>
    </div>

    <!-- Dropdown (teleported to body to avoid overflow clipping) -->
    <Teleport to="body">
    <div
      v-show="isDropdownVisible"
      class="settings-dropdown"
      :style="dropdownStyles"
      @mouseenter="onDropdownEnter"
      @mouseleave="onDropdownLeave"
    >
      <div class="settings-dropdown-inner">
        <!-- Game Settings -->
        <div class="menu-item" @click="emit('update:showSettings', true)">
          {{ t('home.actions.gameSettings') }}
        </div>

        <!-- Open 3Dmigoto Folder -->
        <div class="menu-item" @click="emit('open3dmigotoFolder')">
          {{ t('home.actions.open3dmigotoFolder') }}
        </div>

        <div class="menu-item" :aria-disabled="diagnosingLocks" @click="checkDllLocks">
          {{ diagnosingLocks ? t('fileLocks.checking') : t('fileLocks.menu') }}
        </div>

        <!-- Open d3dx.ini -->
        <div class="menu-item" @click="emit('openD3dxIni')">
          {{ t('home.actions.openD3dxIni') }}
        </div>

        <!-- Symlink submenu -->
        <div class="submenu-wrapper symlink-submenu">
          <div class="menu-item divided symlink-trigger-item">
            <span>{{ t('home.actions.symlink') }}</span>
            <span class="submenu-arrow">›</span>
          </div>
          <div class="submenu-card">
            <div class="submenu-item" @click="toggleSymlink(true)">
              <span class="submenu-item-label">{{ t('home.actions.enableSymlink') }}</span>
            </div>
            <div class="submenu-divider"></div>
            <div class="submenu-item" @click="toggleSymlink(false)">
              <span class="submenu-item-label">{{ t('home.actions.disableSymlink') }}</span>
            </div>
          </div>
        </div>

        <!-- D3D11 Modes submenu -->
        <div class="submenu-wrapper d3d11-submenu">
          <div class="menu-item divided d3d11-trigger-item">
            <span>{{ t('home.actions.d3d11Version') }}</span>
            <span class="submenu-arrow">›</span>
          </div>
          <div class="submenu-card">
            <div v-if="!isCurrentPresetNtemi" class="submenu-item" @click="emit('switchD3d11Mode', 'dev')">
              <span class="submenu-item-label">{{ t('home.actions.switchToDevD3d11') }}</span>
            </div>
            <div v-if="!isCurrentPresetNtemi" class="submenu-divider"></div>
            <div v-if="!isCurrentPresetNtemi" class="submenu-item" @click="emit('switchD3d11Mode', 'play')">
              <span class="submenu-item-label">{{ t('home.actions.switchToPlayD3d11') }}</span>
            </div>
            <div class="submenu-divider"></div>
            <div class="submenu-item" @click="emit('switchD3d11Mode', 'ssice-a')">
              <span class="submenu-item-label">{{ t('home.actions.switchToSsiceAD3d11') }}</span>
            </div>
          </div>
        </div>

        <!-- Updates -->
        <div class="menu-item divided" @click="emit('checkD3D11DllUpdate')">
          {{ t('home.actions.checkD3d11DllUpdate') }}
        </div>
        <div class="menu-item divided" @click="emit('check3DMigotoPackageUpdate')">
          {{ t('home.actions.check3dmigotoPackageUpdate') }}
        </div>
      </div>
    </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* ===== Wrapper: hover zone for both trigger + dropdown ===== */
.settings-menu-wrapper {
  position: relative;
  display: flex;
  align-items: stretch;
}

/* ===== Trigger Button ===== */
.settings-btn {
  width: 56px;
  height: 56px;
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  outline: none;
}

.settings-btn:hover {
  background: rgba(255, 255, 255, 0.14);
}

.menu-lines {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  height: 14px;
  width: 18px;
}

.line {
  height: 2px;
  background: rgba(255, 255, 255, 0.75);
  width: 100%;
  border-radius: 2px;
}

/* ===== Dropdown (fixed position via Teleport to body) ===== */
.settings-dropdown {
  position: fixed;
  z-index: 99999;
  animation: settings-dropdown-in 0.12s ease;
}

@keyframes settings-dropdown-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}

/* ===== Menu Items ===== */
.menu-item {
  padding: 7px 12px;
  border-radius: 4px;
  font-size: 13px;
  color: #cdd6f4;
  cursor: pointer;
  white-space: nowrap;
}

.menu-item:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
}

.menu-item.divided {
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  margin-top: 4px;
  padding-top: 9px;
}

/* ===== Submenu ===== */
.submenu-wrapper {
  position: relative;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  margin-top: 4px;
  padding-top: 4px;
}

.submenu-wrapper .menu-item {
  margin-top: 0;
  border-top: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.submenu-arrow {
  font-size: 14px;
  color: rgba(255, 255, 255, 0.4);
}

/* ===== Submenu Card (left side) ===== */
.submenu-card {
  position: absolute;
  right: calc(100% + 8px);
  top: -4px;
  padding: 4px;
  min-width: 210px;
  opacity: 0;
  pointer-events: none;
  transform: translateX(4px);
  transition: opacity 0.1s, transform 0.1s;
}

/* hover bridge */
.submenu-wrapper::after {
  content: '';
  position: absolute;
  right: 100%;
  top: -4px;
  bottom: -4px;
  width: 14px;
  pointer-events: none;
}

.submenu-wrapper:hover::after {
  pointer-events: auto;
}

.submenu-wrapper:hover .submenu-card,
.submenu-card:hover {
  opacity: 1;
  pointer-events: auto;
  transform: translateX(0);
}

.submenu-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 7px 12px;
  border-radius: 4px;
  cursor: pointer;
  gap: 12px;
  white-space: nowrap;
  font-size: 13px;
  color: #cdd6f4;
}

.submenu-item:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
}

.submenu-item.disabled {
  opacity: 0.4;
  cursor: not-allowed;
  pointer-events: none;
}

.submenu-divider {
  height: 1px;
  background: rgba(255, 255, 255, 0.08);
  margin: 3px 0;
}

/* ===== Status Badge ===== */
.status-badge {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.45);
  flex-shrink: 0;
}

.status-badge.on {
  color: #fff;
}
</style>
