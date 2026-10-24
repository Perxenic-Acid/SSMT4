<script setup lang="ts">
import { useI18n } from 'vue-i18n';
defineProps<{ pinned?: boolean }>();
defineEmits<{ click: [] }>();
const { t } = useI18n();
</script>

<template>
    <button type="button" class="game-switcher-pin" :class="{ pinned }"
        :aria-label="t(pinned ? 'titlebar.unpinGamesFromSidebar' : 'titlebar.pinGamesToSidebar')"
        :title="t(pinned ? 'titlebar.unpinGamesFromSidebar' : 'titlebar.pinGamesToSidebar')"
        :aria-pressed="!!pinned" @click.stop="$emit('click')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M16 9V4l1-1H7l1 1v5l-3 4v2h14v-2l-3-4ZM12 15v7" />
        </svg>
    </button>
</template>

<style scoped>
.game-switcher-pin {
    display: inline-flex; align-items: center; justify-content: center;
    width: 30px; height: 30px; flex: 0 0 30px; border: 0; border-radius: 8px;
    background: transparent; color: inherit; cursor: pointer;
}
.game-switcher-pin:hover, .game-switcher-pin:focus-visible { background: var(--shell-caption-btn-hover, rgba(255,255,255,.12)); }
.game-switcher-pin svg { transform: rotate(35deg); }
.game-switcher-pin.pinned svg { transform: none; }
</style>
