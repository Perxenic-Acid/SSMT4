<script setup lang="ts">
import { computed, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { ElMessage } from 'element-plus';
import { AppStateManager, type GameInfo } from '../store/AppStateManager';
import { getGamePresetDisplayName } from '../store/GamePreset';
import { orderedFavoriteGames } from '../common/GameSwitcher';
import GameSwitcherPin from './GameSwitcherPin.vue';

const { t } = useI18n();
const appSettings = AppStateManager.appSettings;
const isModernUi = computed(() => appSettings.uiMode === 'modern');
const groups = computed(() => [
    { label: t('titlebar.favoriteGames'), games: orderedFavoriteGames(AppStateManager.gamesList, appSettings.sidebarGameOrder) },
    { label: t('titlebar.otherGames'), games: AppStateManager.gamesList.filter(game => !game.showSidebar) },
].filter(group => group.games.length));

const unpin = async () => {
    appSettings.homeGameSidebarPinned = false;
    await nextTick();
    document.querySelector<HTMLButtonElement>('.game-switcher-root .game-switcher-button')?.focus({ preventScroll: true });
};

const selectGame = async (game: GameInfo) => {
    if (game.name === appSettings.CurrentGameName) return;
    try { await AppStateManager.selectGame(game); }
    catch (error) { ElMessage.error(t('appState.messages.loadSettingsFailed', { error: String(error) })); }
};
</script>

<template>
    <aside class="home-game-sidebar" :aria-label="t('titlebar.selectGame')">
        <div class="home-game-sidebar-header">
            <GameSwitcherPin v-if="!isModernUi" pinned @click="unpin" />
            <button v-else type="button" class="home-game-sidebar-collapse"
                :aria-label="t('titlebar.unpinGamesFromSidebar')"
                :title="t('titlebar.unpinGamesFromSidebar')"
                @click="unpin">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                    stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M16 9V4l1-1H7l1 1v5l-3 4v2h14v-2l-3-4ZM12 15v7" />
                </svg>
                <span>{{ t('titlebar.collapseGameBar') }}</span>
            </button>
        </div>
        <div class="home-game-sidebar-list">
            <div v-for="group in groups" :key="group.label" class="home-game-sidebar-group" role="group" :aria-label="group.label">
                <el-tooltip v-for="game in group.games" :key="game.name" :content="getGamePresetDisplayName(game.name, t)" placement="right" :show-after="200">
                    <button type="button" class="home-game-sidebar-item"
                        :class="{ active: appSettings.CurrentGameName === game.name }"
                        :aria-label="getGamePresetDisplayName(game.name, t)"
                        :aria-pressed="appSettings.CurrentGameName === game.name" @click="selectGame(game)">
                        <img v-if="game.iconPath" :src="game.iconPath" alt="" draggable="false"
                            @error="(event) => ((event.target as HTMLImageElement).style.opacity = '0')" />
                        <span v-else>{{ getGamePresetDisplayName(game.name, t).slice(0, 2) }}</span>
                    </button>
                </el-tooltip>
            </div>
        </div>
    </aside>
</template>

<style scoped>
.home-game-sidebar {
    display: flex;
    flex: 0 1 auto;
    flex-direction: column;
    align-items: center;
    width: 72px;
    min-height: 0;
    max-height: calc(100% - 32px);
    margin: 16px 8px 16px 16px;
    padding: 8px 4px;
    gap: 8px;
    box-sizing: border-box;
    color: var(--shell-caption-fg, #fff);
}
.home-game-sidebar-header { display: flex; justify-content: center; flex-shrink: 0; width: 60px; padding-bottom: 6px; color: var(--shell-text-secondary); }
.home-game-sidebar-header:hover, .home-game-sidebar-header:focus-within { color: var(--shell-caption-fg); }
.home-game-sidebar-collapse {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: 58px;
    height: 26px;
    padding: 0 6px;
    border: 1px solid rgba(255, 255, 255, .09);
    border-radius: 8px;
    background: rgba(255, 255, 255, .045);
    color: inherit;
    font: inherit;
    font-size: 9px;
    cursor: pointer;
}
.home-game-sidebar-collapse:hover,
.home-game-sidebar-collapse:focus-visible {
    border-color: rgba(var(--ssmt-brand-accent-rgb), .32);
    background: rgba(var(--ssmt-brand-accent-rgb), .08);
    color: var(--ssmt-brand-accent-soft);
}
.home-game-sidebar-collapse svg { flex: none; transform: rotate(35deg); }
.home-game-sidebar-list {
    flex: 0 1 auto; min-height: 0; overflow-y: auto; overflow-x: hidden; width: 100%;
    scrollbar-width: thin; scrollbar-color: rgba(160,170,190,.25) transparent;
}
.home-game-sidebar-group { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 2px 0; }
.home-game-sidebar-group + .home-game-sidebar-group { margin-top: 10px; }
.home-game-sidebar-group + .home-game-sidebar-group::before { content: ''; width: 20px; height: 1px; margin-bottom: 6px; background: var(--shell-text-disabled); }
.home-game-sidebar-item {
    position: relative;
    display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 52px; height: 52px; padding: 6px; border: 0; border-radius: 13px;
    background: transparent; color: inherit; cursor: pointer;
    transition: background var(--motion-duration-fast) var(--motion-ease-out);
}
.home-game-sidebar-item img { width: 40px; height: 40px; border-radius: 10px; object-fit: cover; filter: drop-shadow(0 2px 4px rgba(0,0,0,.18)); }
.home-game-sidebar-item:hover { background: var(--shell-caption-btn-hover, rgba(255,255,255,.1)); }
.home-game-sidebar-item:focus-visible { outline: 2px solid var(--theme-accent, #90bcff); outline-offset: -2px; }
.home-game-sidebar-item.active { background: var(--shell-fill-selected); }
.home-game-sidebar-item.active::before {
    content: ''; position: absolute; left: 0; top: 16px; width: 3px; height: 20px;
    border-radius: 3px; background: var(--theme-accent, #90bcff);
}
</style>
