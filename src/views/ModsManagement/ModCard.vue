<script setup lang="ts">
import { Picture, ArrowLeft, ArrowRight } from '@element-plus/icons-vue';
import type { ModInfo } from './ModsManagement.types';
import type { ModTagDefinition } from '../../store/ModTagStore';
import type { ModKeyInfo } from '../../store/ModManager';
import { useI18n } from 'vue-i18n';
import { AppStateManager } from '../../store/AppStateManager';
import { ref, watch } from 'vue';

const { t } = useI18n();
const appSettings = AppStateManager.appSettings;

const props = defineProps<{
  mod: ModInfo;
  isDragOver: boolean;
  isPulsing: boolean;
  dynamicStyle: Record<string, string>;
  previewUrl: string | null;
  previewIndex: number;
  hasMultipleImages: boolean;
  tags: ModTagDefinition[];
  keyItems: ModKeyInfo[];
  groupIconUrl: string | null;
  groupDisplayName: string;
  isRootGroup: boolean;
  presetName: string | null;
  blurNsfwPreview: boolean;
}>();

const displayedPreviewUrl = ref(props.previewUrl);
// 启停会重命名目录；替换图片就绪前保留已显示的内容，首次加载仍由 el-image 懒加载。
watch(() => props.previewUrl, (url, _previous, onCleanup) => {
  if (!url || !displayedPreviewUrl.value) { displayedPreviewUrl.value = url; return; }
  let active = true;
  const image = new Image();
  image.onload = () => { if (active) displayedPreviewUrl.value = url; };
  image.onerror = () => { if (active) displayedPreviewUrl.value = null; };
  image.src = url;
  onCleanup(() => { active = false; image.onload = null; image.onerror = null; });
});

const emit = defineEmits<{
  contextmenu: [event: MouseEvent];
  'card-mousedown': [event: MouseEvent];
  mousemove: [event: MouseEvent];
  mouseleave: [event: MouseEvent];
  dblclick: [];
  'open-tag-dialog': [];
  'show-key-floater': [event: MouseEvent];
  'hide-key-floater': [];
  'edit-keys': [];
  'prev-preview': [];
  'next-preview': [];
  'set-preview-index': [index: number];
  toggle: [];
  'open-presets': [event: MouseEvent];
}>();

const onModCardMouseMove = (e: MouseEvent) => {
  const card = e.currentTarget as HTMLElement | null;
  if (!card) return;
  const rect = card.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return;
  const px = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  const py = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
  card.style.setProperty('--mx', `${(px * 100).toFixed(2)}%`);
  card.style.setProperty('--my', `${(py * 100).toFixed(2)}%`);
};

const onModCardMouseLeave = (e: MouseEvent) => {
  const card = e.currentTarget as HTMLElement | null;
  if (!card) return;
  card.style.setProperty('--mx', '50%');
  card.style.setProperty('--my', '18%');
};
</script>

<template>
  <div
    class="mod-card"
    :class="{
      'is-disabled': !mod.enabled,
      'is-directory-link': mod.isDirectoryLink,
      'reorder-hover': isDragOver,
      'state-pulse': isPulsing
    }"
    :style="dynamicStyle"
    @contextmenu.prevent.stop="emit('contextmenu', $event)"
    :draggable="false"
    @mousedown="emit('card-mousedown', $event)"
    @mousemove="onModCardMouseMove"
    @mouseleave="onModCardMouseLeave"
    @dblclick.stop="emit('dblclick')"
    :data-mod-id="mod.id"
  >
    <div class="mod-card-main">
      <!-- Preview Image -->
      <div class="card-preview" :class="{ 'has-multiple-images': hasMultipleImages }">
        <div class="mod-card-actions">
          <button
            type="button"
            class="mod-tag-badge"
            :aria-label="tags.length ? t('modsManagement.actions.editModTags') : t('modsManagement.actions.addModTags')"
            @click.stop="emit('open-tag-dialog')"
            @mousedown.stop
            @dblclick.stop
          >
            <span class="mod-tag-badge-text">Tag</span>
          </button>
          <button
            v-if="keyItems.length > 0"
            type="button"
            class="mod-key-badge"
            :aria-label="t('modsManagement.ui.modKeyListTitle')"
            :title="t('modsManagement.actions.editModKeys')"
            @mouseenter="emit('show-key-floater', $event)"
            @mouseleave="emit('hide-key-floater')"
            @click.stop="emit('edit-keys')"
            @mousedown.stop
            @dblclick.stop
          >
            <span class="mod-key-badge-text">K</span>
          </button>
          <button
            type="button"
            class="mod-preset-badge"
            :class="{ active: !!presetName }"
            :aria-label="presetName ? `Preset: ${presetName}` : 'Presets'"
            @click.stop="emit('open-presets', $event)"
            @mousedown.stop
            @dblclick.stop
          >
            <span class="mod-preset-badge-text">{{ presetName ? presetName.slice(0, 6) : 'P' }}</span>
          </button>
        </div>
        <div
          class="preview-nav prev"
          v-if="hasMultipleImages"
          @click.stop="emit('prev-preview')"
        >
          <el-icon><ArrowLeft /></el-icon>
        </div>

        <div class="mod-crystal-wrapper" :class="{ active: mod.enabled }">
          <div v-if="displayedPreviewUrl" class="image-wrapper" :class="{ 'is-nsfw-blurred': blurNsfwPreview, 'can-hover-reveal': appSettings.revealBlurredImagesOnHover }">
            <div class="slide-item">
              <el-image
                :src="displayedPreviewUrl"
                fit="cover"
                loading="lazy"
                class="zoom-image"
              >
                <template #error>
                  <div class="image-placeholder"><el-icon><Picture /></el-icon></div>
                </template>
              </el-image>
            </div>
            <div v-if="blurNsfwPreview" class="nsfw-preview-shield"></div>
          </div>
          <div v-else class="image-placeholder">
            <span class="char-avatar">{{ mod.group === 'Root' ? mod.name.charAt(0) : mod.group.charAt(0) }}</span>
          </div>
        </div>

        <div
          class="preview-nav next"
          v-if="hasMultipleImages"
          @click.stop="emit('next-preview')"
        >
          <el-icon><ArrowRight /></el-icon>
        </div>

        <div class="preview-indicators" v-if="hasMultipleImages">
          <span
            v-for="(_img, index) in (mod.previewImages || [])"
            :key="index"
            class="indicator-dot"
            :class="{ active: index === previewIndex }"
            @click.stop="emit('set-preview-index', index)"
          ></span>
        </div>
      </div>

      <div class="card-info">
        <div class="header-row">
          <div class="text-content">
            <div class="mod-name" :title="mod.name">{{ mod.name }}</div>
            <div class="mod-group">
              <template v-if="!isRootGroup">
                <img v-if="groupIconUrl" :src="groupIconUrl" class="mini-group-icon" />
                <span>{{ groupDisplayName }}</span>
              </template>
              <span v-else>{{ t('modsManagement.ui.uncategorized') }}</span>
            </div>
            <div
              v-if="tags.length"
              class="mod-tag-line"
              :title="tags.map((tag) => tag.name).join(' / ')"
            >
              {{ tags.map((tag) => tag.name).join(' / ') }}
            </div>
          </div>
          <el-switch
            :model-value="mod.enabled"
            @change="emit('toggle')"
            inline-prompt
            :active-text="t('modsManagement.common.on')"
            :inactive-text="t('modsManagement.common.off')"
            style="--el-switch-on-color: var(--el-color-success); --el-switch-off-color: var(--el-color-danger);"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mod-card {
  --mx: 50%;
  --my: 18%;
  --phase-a: 0s;
  --phase-b: 0s;
  --breath-duration: 4.2s;
  --sheen-duration: 5.2s;
  --rotate-duration: 8.5s;
  border-radius: 12px;
  overflow: hidden;
  transition: background 0.3s ease, border-color 0.3s ease, box-shadow 0.3s ease, opacity 0.2s ease;
  display: flex;
  flex-direction: column;
  width: 240px;
  position: relative;
  user-select: none;
  cursor: grab;
  z-index: 1;
  background: rgba(255,255,255,0.10);
  border: 1px solid rgba(255,255,255,0.22);
  box-shadow:
    0 0 20px rgba(var(--theme-surface-tint-rgb), 0.16),
    0 12px 32px rgba(0,0,0,0.15);
  animation: none;
}

.mod-card-main {
  display: flex;
  flex-direction: column;
  width: 100%;
  overflow: hidden;
  border-radius: 12px;
	clip-path: inset(0 round 11px);
}

.mod-card:not(.is-disabled) {
  /* 呼吸基态（原 keyframes 0%/100% 的值，动画不再覆盖 border/box-shadow，hover 阴影恢复生效） */
  border-color: rgba(var(--theme-surface-tint-rgb), 0.24);
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.08) inset,
    0 0 24px rgba(var(--theme-surface-tint-rgb), 0.18),
    0 12px 26px rgba(0, 0, 0, 0.38);
}

/* 呼吸脉冲层：只动 opacity，合成器混合完成，每张卡片每帧零重绘。
   原实现直接动画 box-shadow/border-color，所有可见卡片每帧都在主线程重绘。 */
.mod-card:not(.is-disabled)::before {
  content: "";
  position: absolute;
  /* 不能用 inset:-1px：卡片 overflow:hidden 会把伪元素裁到 padding-box，边框环会被裁没 */
  inset: 0;
  border-radius: 12px;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.48);
  box-shadow:
    0 0 18px rgba(var(--theme-surface-tint-rgb), 0.20) inset,
    0 0 0 1px rgba(255, 255, 255, 0.04) inset;
  opacity: 0;
  pointer-events: none;
  animation: modEnabledBreath var(--breath-duration) ease-in-out infinite;
  animation-delay: var(--phase-a);
}

.mod-card.state-pulse::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 12px;
  pointer-events: none;
  border: 1px solid rgba(var(--theme-surface-tint-rgb), 0.70);
  box-shadow: 0 0 0 rgba(var(--theme-surface-tint-rgb), 0.45);
  animation: modStatePulse 0.54s ease-out 1;
  z-index: 7;
}

.mod-card[draggable="true"] { cursor: move; }
.mod-card[draggable="false"] { cursor: grab; }

.mod-card.reorder-hover {
  outline: 2px dashed rgba(var(--theme-surface-tint-rgb), 0.60);
  border-color: rgba(var(--theme-surface-tint-rgb), 0.40);
}

.mod-card:active {
  cursor: grabbing;
}

.mod-card:hover {
  transform: none;
  box-shadow:
    0 0 40px rgba(var(--theme-surface-tint-rgb), 0.28),
    0 0 60px rgba(142, 230, 255, 0.12),
    0 18px 48px rgba(0,0,0,0.20),
    0 0 0 1px rgba(255,255,255,0.12) inset;
  border-color: rgba(255,255,255,0.28);
  z-index: 2;
}

.mod-card.is-disabled {
  opacity: 1;
  filter: none;
  border-color: rgba(255,255,255,0.12);
  box-shadow:
    0 0 0 1px rgba(255,255,255,0.08) inset,
    0 4px 18px rgba(0,0,0,0.15);
}

.mod-card.is-disabled:hover {
  transform: none;
  border-color: rgba(255,255,255,0.24);
  box-shadow:
    0 12px 32px rgba(0,0,0,0.18),
    0 0 0 1px rgba(255,255,255,0.10) inset;
}

.card-preview {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  flex-shrink: 0;
  background: rgba(255,255,255,0.02);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
}

.mod-crystal-wrapper {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: radial-gradient(circle at var(--mx) var(--my),
    rgba(255, 255, 255, 0.25) 0%,
    rgba(255, 255, 255, 0.08) 40%,
    transparent 100%);
}

.mod-crystal-wrapper::after {
  content: "";
  position: absolute;
  top: 0;
  left: -150%;
  width: 200%;
  height: 100%;
  background: linear-gradient(115deg,
    transparent 40%,
    rgba(255, 255, 255, 0.08) 45%,
    rgba(255, 255, 255, 0.35) 50%,
    rgba(255, 255, 255, 0.08) 55%,
    transparent 60%);
  transform: skewX(-20deg);
  pointer-events: none;
  z-index: 5;
  opacity: 0.5;
}

.preview-nav {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  width: 24px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  z-index: 10;
  background: rgba(0,0,0,0.3);
  color: rgba(255,255,255,0.7);
  opacity: 0;
  transition: opacity 0.3s, background 0.3s;
  border-radius: 4px;
  margin: 0 4px;
}

.mod-card:hover .preview-nav {
  opacity: 1;
}

.preview-nav.prev { left: 0; }
.preview-nav.next { right: 0; }

.preview-nav:hover {
  background: rgba(0,0,0,0.6);
  color: #fff;
}

.mod-card-actions {
  position: absolute;
  top: 10px;
  right: 10px;
  z-index: 22;
  display: flex;
  align-items: center;
  gap: 8px;
}

.mod-key-badge,
.mod-tag-badge {
  width: 28px;
  height: 28px;
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 999px;
  background: rgba(255,255,255,0.06);
  /* 性能：逐卡片元素不加 backdrop-filter（每个 mod 卡片都会创建一个模糊合成层，滚动时引发闪烁/文字消失），半透明背景已足够 */
  box-shadow: 0 4px 12px rgba(0,0,0,0.12), 0 0 0 1px rgba(255,255,255,0.04) inset;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.2s ease;
}

.mod-key-badge {
  color: rgba(var(--theme-surface-tint-rgb),0.75);
}

.mod-tag-badge {
  width: auto;
  min-width: 44px;
  padding: 0 10px;
  border-radius: 999px;
  color: rgba(255,255,255,0.60);
}

.mod-key-badge:hover {
  transform: scale(1.08);
  background: rgba(var(--theme-surface-tint-rgb),0.10);
  border-color: rgba(var(--theme-surface-tint-rgb),0.30);
  color: var(--theme-accent);
  box-shadow: 0 4px 14px rgba(0,0,0,0.18), 0 0 12px rgba(var(--theme-surface-tint-rgb),0.06);
}

.mod-tag-badge:hover {
  transform: translateY(-1px) scale(1.05);
  background: rgba(255,255,255,0.10);
  border-color: rgba(255,255,255,0.28);
  color: rgba(255,255,255,0.90);
  box-shadow: 0 6px 20px rgba(0,0,0,0.20), 0 0 0 1px rgba(255,255,255,0.06) inset;
}

.mod-tag-badge:active {
  transform: scale(0.95);
}

.mod-key-badge-text {
  font-size: 12px;
  font-weight: 800;
  letter-spacing: 0.5px;
  line-height: 1;
}

.mod-tag-badge-text {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.4px;
  line-height: 1;
}

.mod-tag-badge:hover .mod-tag-badge-text {
  color: var(--theme-accent);
}

.mod-preset-badge {
  width: 28px;
  height: 28px;
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 999px;
  background: rgba(255,255,255,0.06);
  /* 性能：逐卡片元素不加 backdrop-filter（每个 mod 卡片都会创建一个模糊合成层，滚动时引发闪烁/文字消失），半透明背景已足够 */
  box-shadow: 0 4px 12px rgba(0,0,0,0.12), 0 0 0 1px rgba(255,255,255,0.04) inset;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.2s ease;
  color: rgba(255,255,255,0.50);
}
.mod-preset-badge.active {
  border-color: rgba(180, 160, 255, 0.35);
  background: rgba(160, 140, 255, 0.10);
  color: rgba(200, 180, 255, 0.80);
  box-shadow: 0 4px 12px rgba(0,0,0,0.12), 0 0 10px rgba(160, 140, 255, 0.10), 0 0 0 1px rgba(255,255,255,0.04) inset;
}
.mod-preset-badge:hover {
  transform: translateY(-1px) scale(1.05);
  color: rgba(255,255,255,0.90);
  border-color: rgba(255,255,255,0.28);
  box-shadow: 0 6px 20px rgba(0,0,0,0.20), 0 0 0 1px rgba(255,255,255,0.06) inset;
}
.mod-preset-badge-text {
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 0.3px;
  line-height: 1;
}

.preview-indicators {
  position: absolute;
  bottom: 8px;
  left: 0;
  right: 0;
  display: flex;
  justify-content: center;
  gap: 6px;
  z-index: 15;
  pointer-events: none;
}

.indicator-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.4);
  transition: background 0.3s, transform 0.3s;
  pointer-events: auto;
  cursor: pointer;
  box-shadow: 0 1px 2px rgba(0,0,0,0.3);
}

.indicator-dot:hover {
  background: rgba(255, 255, 255, 0.7);
  transform: scale(1.1);
}

.indicator-dot.active {
  background: #fff;
  transform: scale(1.2);
  box-shadow: 0 0 4px rgba(255,255,255,0.4);
}

.slide-item {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
}

.preview-slide-enter-active,
.preview-slide-leave-active {
  transition: transform 0.4s cubic-bezier(0.25, 1, 0.5, 1), opacity 0.4s ease;
}

.preview-slide-enter-from {
  transform: translateX(100%);
  opacity: 0.8;
}

.preview-slide-leave-to {
  transform: translateX(-100%);
  opacity: 0.8;
}

.image-wrapper {
  width: 100%;
  height: 100%;
  position: relative;
  overflow: hidden;
}

.image-wrapper::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(to bottom, rgba(var(--theme-surface-tint-rgb), 0.08), rgba(0, 0, 0, 0) 34%, rgba(5, 8, 14, 0.24) 100%);
  z-index: 1;
  pointer-events: none;
}

.image-wrapper.is-nsfw-blurred .zoom-image {
  filter: blur(18px) saturate(0.82);
  transform: scale(1.12);
}

.mod-card.is-selected {
  border-color: color-mix(in srgb, var(--theme-accent) 72%, white 12%);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--theme-accent) 30%, transparent), var(--card-shadow);
}

.mod-card.is-selected::before {
  opacity: 1;
}

.image-wrapper.is-nsfw-blurred.can-hover-reveal:hover .zoom-image {
  filter: blur(0) saturate(1);
  transform: scale(1.04);
}

.image-wrapper.is-nsfw-blurred.can-hover-reveal:hover .nsfw-preview-shield {
  opacity: 0;
}

.image-wrapper.is-nsfw-blurred::after {
  background:
    linear-gradient(to bottom, rgba(8, 10, 15, 0.32), rgba(8, 10, 15, 0.52)),
    linear-gradient(to bottom, rgba(var(--theme-surface-tint-rgb), 0.08), rgba(0, 0, 0, 0) 34%, rgba(5, 8, 14, 0.28) 100%);
}

.nsfw-preview-shield {
  position: absolute;
  inset: 0;
  z-index: 3;
  display: grid;
  place-items: center;
  color: rgba(255,255,255,0.88);
  font-size: 12px;
  font-weight: 800;
  letter-spacing: 0.18em;
  text-shadow: 0 1px 8px rgba(0,0,0,0.8);
  pointer-events: none;
  transition: opacity 220ms ease;
}

.zoom-image {
  width: 100%;
  height: 100%;
  display: block;
  transition: filter 220ms ease, transform 220ms ease;
  position: relative;
  z-index: 0;
}

.mod-card:hover .zoom-image {
  transform: scale(1.05);
}

.mod-card.is-disabled .mod-name {
  color: rgba(255,255,255,0.88);
}

.mod-card.is-disabled .mod-group {
  color: rgba(255,255,255,0.56);
}

.mod-card.is-disabled .card-info {
  background: linear-gradient(to top, rgba(7, 10, 16, 0.78), rgba(7, 10, 16, 0.36));
}

@keyframes modEnabledBreath {
  0%,
  100% {
    opacity: 0;
  }
  50% {
    opacity: 1;
  }
}

@keyframes modStatePulse {
  0% {
    opacity: 0.85;
    transform: scale(1);
    box-shadow: 0 0 0 rgba(var(--theme-surface-tint-rgb), 0.42);
  }
  100% {
    opacity: 0;
    transform: scale(1.03);
    box-shadow: 0 0 26px rgba(var(--theme-surface-tint-rgb), 0);
  }
}

:deep(.mod-card img) {
  -webkit-user-drag: none;
  pointer-events: none;
}

.image-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background:
    linear-gradient(135deg, rgba(var(--theme-surface-tint-rgb), 0.08), rgba(var(--theme-surface-tint-rgb), 0.025)),
    rgba(255, 255, 255, 0.03);
  color: rgba(var(--theme-surface-tint-rgb), 0.28);
  font-size: 48px;
  font-weight: 800;
}

.char-avatar {
  text-transform: uppercase;
}

.card-info {
	box-sizing: border-box;
	width: 100%;
	min-width: 0;
  padding: 12px 14px 14px;
  background: rgba(255,255,255,0.03);
  /* 性能：逐卡片元素不加 backdrop-filter（每个 mod 卡片都会创建一个模糊合成层，滚动时引发闪烁/文字消失），半透明背景已足够 */
  border-top: 1px solid rgba(255,255,255,0.06);
  display: flex;
  flex-direction: column;
  justify-content: center;
  min-height: 96px;
  position: relative;
  z-index: 2;
	overflow: hidden;
}

.header-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
	min-width: 0;
	width: 100%;
}

.header-row :deep(.el-switch) {
	flex: 0 0 auto;
}

.text-content {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.mod-name {
  font-weight: 600;
  color: rgba(255,255,255,0.88);
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 14px;
  letter-spacing: 0.3px;
  line-height: 1.35;
  word-break: break-word;
}

.mod-group {
  font-size: 11px;
  color: rgba(255,255,255,0.45);
  display: flex;
  align-items: center;
  gap: 4px;
}

.mini-group-icon {
  width: 14px;
  height: 14px;
  border-radius: 2px;
  object-fit: cover;
}

.mod-tag-line {
  min-height: 16px;
  font-size: 11px;
  line-height: 1.45;
  color: rgba(255,255,255,0.55);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mod-tag-line::before {
  content: '\26AC ';
  color: rgba(var(--theme-surface-tint-rgb),0.50);
  font-size: 9px;
  margin-right: 2px;
}

:deep(.el-switch__core) {
  background-color: rgba(255,255,255,0.1);
  border-color: transparent;
}
</style>
