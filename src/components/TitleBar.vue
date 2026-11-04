<script setup lang="ts">
import { computed, nextTick, reactive, ref, onMounted, onUnmounted, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { ElMessage } from 'element-plus';
import { getAlwaysOnTop, setAlwaysOnTop } from '../store/WindowPinStore';
import { AppStateManager, type GameInfo } from '../store/AppStateManager';
import { getGamePresetDisplayName } from '../store/GamePreset';
import { calculateContextMenuPosition } from '../utils/ContextMenuPosition';
import { orderedFavoriteGames } from '../common/GameSwitcher';
import GameSwitcherPin from './GameSwitcherPin.vue';
import Ssmt4Mark from './Ssmt4Mark.vue';
import { PLUGIN_REGISTRY_CHANGED_EVENT } from '../plugin/pluginRegistryEvents';
import {
    LIQUID_GRAVITY_PROBE_NAME,
    LIQUID_GRAVITY_PROBE_NAV_ID,
    LIQUID_GRAVITY_PROBE_PLUGIN_ID,
    LIQUID_GRAVITY_PROBE_ROUTE,
} from '../plugin/liquidGravityProbe';

const appWindow = getCurrentWindow();
const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const isMaximized = ref(false);
const isPinned = ref(false);
const captionBarRef = ref<HTMLElement | null>(null);

const appSettings = AppStateManager.appSettings;
const gamesList = AppStateManager.gamesList;
const isLegacyUi = computed(() => appSettings.uiMode === 'classic');

/* ═══════════════════════ Game quick-switcher ═══════════════════════ */

const currentGame = computed(() => gamesList.find(g => g.name === appSettings.CurrentGameName));
const liquidGravityProbeEnabled = ref(false);
let pluginAvailabilityRequest = 0;

const refreshLiquidGravityProbeAvailability = async () => {
    const requestId = ++pluginAvailabilityRequest;
    if (appSettings.CurrentGameName.toUpperCase() !== 'EFMI') {
        liquidGravityProbeEnabled.value = false;
        return;
    }
    try {
        const plugins = await invoke<Array<{ manifest: { id: string }; enabled: boolean }>>(
            'plugin_registry_snapshot_for_game',
            { gameName: appSettings.CurrentGameName },
        );
        if (requestId === pluginAvailabilityRequest) {
            liquidGravityProbeEnabled.value = plugins.some(plugin =>
                plugin.manifest.id === LIQUID_GRAVITY_PROBE_PLUGIN_ID && plugin.enabled
            );
        }
    } catch {
        if (requestId === pluginAvailabilityRequest) liquidGravityProbeEnabled.value = false;
    }
};

/** Favorite games (formerly the sidebar strip), honoring the user's manual order. */
const favoriteGames = computed(() => orderedFavoriteGames(gamesList, appSettings.sidebarGameOrder));
const isHome = computed(() => route.path === '/');
const isGameSwitcherHidden = computed(() => isHome.value && appSettings.homeGameSidebarPinned);

const otherGames = computed(() => gamesList.filter(g => !g.showSidebar));

const isGameSwitcherOpen = ref(false);

const toggleGameSwitcher = () => {
    if (isGameSwitcherHidden.value) return;
    isGameSwitcherOpen.value = !isGameSwitcherOpen.value;
};

watch(() => route.path, () => { isGameSwitcherOpen.value = false; });

const pinGameSidebar = async () => {
    if (!isHome.value) return;
    isGameSwitcherOpen.value = false;
    appSettings.homeGameSidebarPinned = true;
    await nextTick();
    document.querySelector<HTMLButtonElement>('.home-game-sidebar .home-game-sidebar-collapse')?.focus({ preventScroll: true });
};

const closeGameSwitcher = () => {
    isGameSwitcherOpen.value = false;
};

/** Light-dismiss: presses outside the switcher close it; any outside press
    also closes the tab context menu (its own pointerdown is stopped). */
const handleCaptionPointerDown = (event: PointerEvent) => {
    if (isGameSwitcherOpen.value) {
        const target = event.target as HTMLElement | null;
        if (target && !target.closest('.game-switcher-root')) {
            closeGameSwitcher();
        }
    }
    closeNavContextMenu();
};

const handleSwitchGame = async (game: GameInfo) => {
    closeGameSwitcher();
    if (game.name === appSettings.CurrentGameName) return;

    try {
        await AppStateManager.selectGame(game);
    } catch (error) {
        console.error('Failed to switch game:', error);
        ElMessage.error(t('appState.messages.loadSettingsFailed', { error: String(error) }));
    }
};

/* ═══════════════════════ Navigation tabs (replaces the WinUI3 nav pane) ═══════════════════════ */

interface NavItem { id: string; path: string; labelKey: string; label?: string }

const STORAGE_KEY_NAV_ORDER = 'ssmt4_nav_order';

const visibilityKeyByNavId: Partial<Record<string, keyof typeof appSettings.pageVisibility>> = {
    work: 'work',
    'mark-texture-full': 'markTexture',
    'texture-mod-maker': 'textureModMaker',
    mods: 'mods',
    gamebanana: 'gameBanana',
    nexusmods: 'nexusMods',
    xianzun: 'xianzun',
    'ui-builder': 'uiBuilder',
};

const allNavItems: NavItem[] = [
    { id: 'home', path: '/', labelKey: 'titlebar.nav.home' },
    { id: 'work', path: '/work', labelKey: 'titlebar.nav.work' },
    { id: 'mark-texture-full', path: '/mark-texture-full', labelKey: 'titlebar.nav.markTexture' },
    { id: 'texture-mod-maker', path: '/texture-mod-maker', labelKey: 'titlebar.nav.textureModMaker' },
    { id: 'mods', path: '/mods', labelKey: 'titlebar.nav.mods' },
    { id: 'gamebanana', path: '/gamebanana', labelKey: 'titlebar.nav.gameBanana' },
    { id: 'nexusmods', path: '/nexusmods', labelKey: 'titlebar.nav.nexusMods' },
    { id: 'xianzun', path: '/xianzun', labelKey: 'titlebar.nav.xianzun' },
    { id: 'ui-builder', path: '/ui-builder', labelKey: 'titlebar.nav.uiBuilder' },
    { id: LIQUID_GRAVITY_PROBE_NAV_ID, path: LIQUID_GRAVITY_PROBE_ROUTE, labelKey: '', label: LIQUID_GRAVITY_PROBE_NAME },
    { id: 'plugins', path: '/plugins', labelKey: 'titlebar.nav.plugins' },
];

const navOrder = ref<string[]>([]);

const ensureNavOrderCompleteness = () => {
    const ids = allNavItems.map(i => i.id);
    const cleaned = navOrder.value.filter(id => ids.includes(id));
    ids.forEach(id => {
        if (!cleaned.includes(id)) cleaned.push(id);
    });
    navOrder.value = cleaned;
};

const loadOrder = () => {
    try {
        const stored = localStorage.getItem(STORAGE_KEY_NAV_ORDER);
        if (stored) {
            const parsed = JSON.parse(stored);
            if (Array.isArray(parsed)) {
                navOrder.value = parsed;
                ensureNavOrderCompleteness();
                localStorage.setItem(STORAGE_KEY_NAV_ORDER, JSON.stringify(navOrder.value));
                return;
            }
        }
    } catch (e) { console.warn('Failed to load nav order', e); }

    navOrder.value = allNavItems.map(i => i.id);
};

const focusNavItem = async (id: string) => {
    await nextTick();
    document.querySelector<HTMLButtonElement>(`.caption-tab[data-nav-id="${id}"]`)?.focus({ preventScroll: true });
};

const moveNavItem = async (item: NavItem, direction: -1 | 1) => {
    const visibleIds = displayItems.value.map(entry => entry.id);
    const currentIndex = visibleIds.indexOf(item.id);
    const targetId = visibleIds[currentIndex + direction];
    if (!targetId) return;
    applyNavReorder(item.id, targetId);
    closeNavContextMenu();
    await focusNavItem(item.id);
};

const canMoveNavItem = (item: NavItem, direction: -1 | 1) => {
    const visibleIds = displayItems.value.map(entry => entry.id);
    const index = visibleIds.indexOf(item.id);
    return index >= 0 && index + direction >= 0 && index + direction < visibleIds.length;
};

const displayItems = computed<NavItem[]>(() => {
    const map = new Map(allNavItems.map(i => [i.id, i]));
    const result: NavItem[] = [];

    navOrder.value.forEach(id => {
        const item = map.get(id);
        if (item) {
            result.push({ ...item, label: item.label ?? t(item.labelKey) });
        }
    });

    allNavItems.forEach(i => {
        if (!navOrder.value.includes(i.id)) {
            result.push({ ...i, label: i.label ?? t(i.labelKey) });
        }
    });

    return result.filter(item => {
        if (item.id === LIQUID_GRAVITY_PROBE_NAV_ID && !liquidGravityProbeEnabled.value) return false;
        const key = visibilityKeyByNavId[item.id];
        return !key || appSettings.pageVisibility[key];
    });
});

const isNavItemActive = (item: NavItem) => {
    if (item.path === '/') return route.path === '/';
    return route.path === item.path || route.path.startsWith(`${item.path}/`);
};
const hasNavMenu = (item: NavItem) => Boolean(visibilityKeyByNavId[item.id]);
const isNavMenuExpanded = (item: NavItem) =>
    hasNavMenu(item) && navContextMenu.visible && navContextMenu.item?.id === item.id;
const syncNavMenuAria = async () => {
    await nextTick();
    document.querySelectorAll<HTMLButtonElement>('.caption-tab[data-nav-id]').forEach(button => {
        const item = allNavItems.find(entry => entry.id === button.dataset.navId);
        if (!item || !hasNavMenu(item)) {
            button.removeAttribute('aria-haspopup');
            button.removeAttribute('aria-expanded');
            return;
        }
        button.setAttribute('aria-haspopup', 'menu');
        if (isNavMenuExpanded(item)) button.setAttribute('aria-expanded', 'true');
        else button.removeAttribute('aria-expanded');
    });
};

const navTo = (path: string) => {
    if (suppressNavClick.value) return;
    if (route.path !== path) {
        router.push(path);
    }
};

/* ─────── Tab drag-to-reorder (manual, Tauri-safe) ─────── */

const navHoverId = ref<string | null>(null);
const navDraggingId = ref<string | null>(null);
const suppressNavClick = ref(false);
const navDragState = reactive({
    active: false,
    startX: 0,
    startY: 0,
    hasMoved: false,
    itemId: null as string | null,
});

const resetNavDrag = () => {
    navDragState.active = false;
    navDragState.hasMoved = false;
    navDragState.itemId = null;
    navHoverId.value = null;
    navDraggingId.value = null;
    document.body.style.userSelect = '';
};

const saveOrder = () => {
    localStorage.setItem(STORAGE_KEY_NAV_ORDER, JSON.stringify(navOrder.value));
};

const applyNavReorder = (sourceId: string, targetId: string) => {
    const oldIndex = navOrder.value.indexOf(sourceId);
    const newIndex = navOrder.value.indexOf(targetId);
    if (oldIndex === -1 || newIndex === -1) return;
    const newOrder = [...navOrder.value];
    newOrder.splice(oldIndex, 1);
    newOrder.splice(newIndex, 0, sourceId);
    navOrder.value = newOrder;
    saveOrder();
};

const onNavMouseDown = (e: MouseEvent, item: NavItem) => {
    if (e.button !== 0) return;
    navDragState.active = true;
    navDragState.startX = e.clientX;
    navDragState.startY = e.clientY;
    navDragState.hasMoved = false;
    navDragState.itemId = item.id;
    navDraggingId.value = item.id;
    suppressNavClick.value = false;
    document.addEventListener('mousemove', onNavMouseMove);
    document.addEventListener('mouseup', onNavMouseUp);
};

const onNavMouseMove = (e: MouseEvent) => {
    if (!navDragState.active || !navDragState.itemId) return;
    const dx = e.clientX - navDragState.startX;
    const dy = e.clientY - navDragState.startY;
    if (!navDragState.hasMoved && Math.hypot(dx, dy) > 4) {
        navDragState.hasMoved = true;
        suppressNavClick.value = true;
        document.body.style.userSelect = 'none';
    }

    if (navDragState.hasMoved) {
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const btn = el?.closest?.('.caption-tab[data-nav-id]') as HTMLElement | null;
        const targetId = btn?.dataset.navId || null;
        navHoverId.value = targetId && targetId !== navDragState.itemId ? targetId : null;
    }
};

const onNavMouseUp = (_e: MouseEvent) => {
    document.removeEventListener('mousemove', onNavMouseMove);
    document.removeEventListener('mouseup', onNavMouseUp);

    if (navDragState.active && navDragState.hasMoved && navDragState.itemId && navHoverId.value && navHoverId.value !== navDragState.itemId) {
        applyNavReorder(navDragState.itemId, navHoverId.value);
    }

    resetNavDrag();
    window.setTimeout(() => {
        suppressNavClick.value = false;
    }, 0);
};

/* ─────── Tab context menu (hide page) ─────── */

const navContextMenu = reactive({ visible: false, x: 0, y: 0, item: null as NavItem | null });
const navContextMenuRef = ref<HTMLElement | null>(null);
const navContextMenuTrigger = ref<HTMLElement | null>(null);

const closeNavContextMenu = (restoreFocus = false) => {
    navContextMenu.visible = false;
    navContextMenu.item = null;
    const trigger = navContextMenuTrigger.value;
    navContextMenuTrigger.value = null;
    if (restoreFocus && trigger?.isConnected) {
        void nextTick(() => trigger.focus({ preventScroll: true }));
    }
};

const openNavContextMenu = (event: MouseEvent | KeyboardEvent, item: NavItem) => {
    // Only pages with a visibility key can be hidden — home always stays.
    if (!visibilityKeyByNavId[item.id]) return;

    const trigger = event.currentTarget as HTMLElement | null;
    const rect = trigger?.getBoundingClientRect();
    const clientX = event instanceof MouseEvent ? event.clientX : rect?.right ?? 0;
    const clientY = event instanceof MouseEvent ? event.clientY : rect?.bottom ?? 0;
    navContextMenu.item = item;
    navContextMenuTrigger.value = trigger;
    navContextMenu.x = clientX;
    navContextMenu.y = clientY;
    navContextMenu.visible = true;

    nextTick(() => {
        const menuEl = navContextMenuRef.value;
        if (!menuEl) return;
        const rect = menuEl.getBoundingClientRect();
        const pos = calculateContextMenuPosition({
            clientX,
            clientY,
            menuWidth: rect.width,
            menuHeight: rect.height,
        });

        // The caption bar host sits at the maximum z-index, so a menu that
        // overlaps the bar would be covered by it — push the menu fully
        // below the bar's bottom edge instead.
        const captionBottom = captionBarRef.value
            ?.querySelector<HTMLElement>('.caption-drag-region')
            ?.getBoundingClientRect().bottom ?? 0;
        const minY = captionBottom + 4;
        const railRight = captionBarRef.value
            ?.querySelector<HTMLElement>('.caption-tabs')
            ?.getBoundingClientRect().right ?? 0;

        navContextMenu.x = Math.max(pos.x, railRight + 4);
        navContextMenu.y = Math.max(pos.y, minY);
        menuEl.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
    });
};

const hideNavItem = async (item: NavItem) => {
    const key = visibilityKeyByNavId[item.id];
    if (!key) return;
    const leavingCurrentPage = isNavItemActive(item);
    appSettings.pageVisibility[key] = false;
    closeNavContextMenu();
    await AppStateManager.saveSettingsNow();
    if (leavingCurrentPage) {
        await router.push('/');
    }
    await focusNavItem('home');
};

/* ═══════════════════════ Window controls ═══════════════════════ */

const checkMaximized = async () => {
    isMaximized.value = await appWindow.isMaximized();
};

let unlistenResize: (() => void) | null = null;

onMounted(async () => {
    checkMaximized();
    loadOrder();
    await syncNavMenuAria();
    await refreshLiquidGravityProbeAvailability();
    window.addEventListener(PLUGIN_REGISTRY_CHANGED_EVENT, refreshLiquidGravityProbeAvailability);
    document.addEventListener('pointerdown', handleCaptionPointerDown);
    // Listen to resize event to update maximized state icon
    unlistenResize = await appWindow.onResized(() => {
        checkMaximized();
    });
    // Restore always-on-top state
    try {
        const pinned = await getAlwaysOnTop();
        if (pinned) {
            isPinned.value = true;
            await appWindow.setAlwaysOnTop(true);
        }
    } catch (e) {
        console.error('Failed to restore always-on-top state', e);
    }
});

watch([displayItems, () => navContextMenu.visible, () => navContextMenu.item?.id], () => {
    void syncNavMenuAria();
});
watch(() => appSettings.CurrentGameName, () => {
    void refreshLiquidGravityProbeAvailability();
});

onUnmounted(() => {
    window.removeEventListener(PLUGIN_REGISTRY_CHANGED_EVENT, refreshLiquidGravityProbeAvailability);
    document.removeEventListener('pointerdown', handleCaptionPointerDown);
    document.removeEventListener('mousemove', onNavMouseMove);
    document.removeEventListener('mouseup', onNavMouseUp);
    resetNavDrag();
    if (unlistenResize) {
        unlistenResize();
    }
});

const minimize = () => appWindow.minimize();
const toggleMaximize = async () => {
    await appWindow.toggleMaximize();
    checkMaximized();
};
const close = () => appWindow.close();
const startDrag = async (event: MouseEvent) => {
    // Skip the drag on the second click of a double-click so the dblclick
    // handler (maximize toggle) can fire instead of restarting the drag.
    if (event.button !== 0 || event.detail > 1) return;

    try {
        // 直接交给系统的标题栏拖动，保留 Windows 的拖动还原和贴靠行为。
        // 不要先异步还原：会丢失原始拖动手势，还可能因权限缺失中断。
        await appWindow.startDragging();
        await checkMaximized();
    } catch (error) {
        console.warn('startDragging failed', error);
    }
};

const togglePin = async () => {
    const newVal = !isPinned.value;
    isPinned.value = newVal;
    await appWindow.setAlwaysOnTop(newVal);
    await setAlwaysOnTop(newVal);
};

const toggleSettingsPage = () => {
    if (route.path === '/settings') {
        if (window.history.length > 1) {
            router.back();
        } else {
            router.push('/');
        }
    } else {
        router.push('/settings');
    }
};
</script>

<template>
    <div ref="captionBarRef" class="caption-bar" :class="{ 'is-legacy-ui': isLegacyUi }">
        <!-- Drag region (window move; double-click toggles maximize).
             Keep the manual startDragging path only: data-tauri-drag-region
             would trigger a second drag request. -->
        <div class="caption-drag-region" @mousedown="startDrag" @dblclick="toggleMaximize"></div>
        <button type="button" class="caption-brand" :aria-label="'SSMT4 · ' + t('titlebar.nav.home')"
            @click="navTo('/')">
            <Ssmt4Mark class="caption-brand-mark" />
            <span class="caption-brand-name">SSMT4</span>
        </button>
        <!-- Game quick-switcher at the far left: current game icon, opens the game picker card -->
        <div class="game-switcher-root" :class="{ 'is-home-sidebar': isGameSwitcherHidden }"
            :inert="isGameSwitcherHidden" :aria-hidden="isGameSwitcherHidden || undefined">
            <el-tooltip :content="t('titlebar.selectGame')" placement="right"
                popper-class="caption-game-switcher-tooltip" :show-after="250" :disabled="isGameSwitcherOpen">
                <button
                    type="button"
                    class="caption-button game-switcher-button"
                    :class="{ active: isGameSwitcherOpen }"
                    :aria-label="t('titlebar.selectGame')"
                    :aria-expanded="isGameSwitcherOpen"
                    aria-haspopup="menu"
                    @click="toggleGameSwitcher"
                >
                    <span class="game-switcher-icon-frame" aria-hidden="true">
                        <img
                            v-if="currentGame?.iconPath"
                            class="game-switcher-icon"
                            :src="currentGame.iconPath"
                            :alt="getGamePresetDisplayName(currentGame.name, t)"
                            draggable="false"
                            @error="(e) => ((e.target as HTMLImageElement).style.opacity = '0')"
                        />
                        <svg v-else xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="3" y="3" width="7" height="7" rx="1.5"></rect>
                            <rect x="14" y="3" width="7" height="7" rx="1.5"></rect>
                            <rect x="14" y="14" width="7" height="7" rx="1.5"></rect>
                            <rect x="3" y="14" width="7" height="7" rx="1.5"></rect>
                        </svg>
                    </span>
                    <span class="game-switcher-chevron" aria-hidden="true">
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                            <path d="m3 4.5 3 3 3-3"></path>
                        </svg>
                    </span>
                </button>
            </el-tooltip>

            <transition name="game-switcher-pop">
                <div v-if="isGameSwitcherOpen" class="game-switcher-card" role="menu" :aria-label="t('titlebar.selectGame')">
                    <div class="game-switcher-card-head">
                        <span class="game-switcher-card-kicker">{{ t('titlebar.selectGame') }}</span>
                        <strong>{{ currentGame ? getGamePresetDisplayName(currentGame.name, t) : t('titlebar.selectGame') }}</strong>
                    </div>
                    <div v-if="isHome" class="game-switcher-pin-row">
                        <span>{{ t('titlebar.selectGame') }}</span>
                        <GameSwitcherPin @click="pinGameSidebar" />
                    </div>
                    <div v-if="favoriteGames.length > 0" class="game-switcher-group">
                        <div class="game-switcher-group-title">{{ t('titlebar.favoriteGames') }}</div>
                        <button
                            v-for="game in favoriteGames"
                            :key="game.name"
                            type="button"
                            class="game-switcher-item"
                            :class="{ active: appSettings.CurrentGameName === game.name }"
                            role="menuitem"
                            @click="handleSwitchGame(game)"
                        >
                            <span class="game-switcher-item-icon-wrap">
                                <img
                                    v-if="game.iconPath"
                                    class="game-switcher-item-icon"
                                    :src="game.iconPath"
                                    alt=""
                                    draggable="false"
                                    @error="(e) => ((e.target as HTMLImageElement).style.opacity = '0')"
                                />
                            </span>
                            <span class="game-switcher-item-name">{{ getGamePresetDisplayName(game.name, t) }}</span>
                        </button>
                    </div>

                    <div v-if="otherGames.length > 0" class="game-switcher-group">
                        <div class="game-switcher-group-title">{{ t('titlebar.otherGames') }}</div>
                        <button
                            v-for="game in otherGames"
                            :key="game.name"
                            type="button"
                            class="game-switcher-item"
                            :class="{ active: appSettings.CurrentGameName === game.name }"
                            role="menuitem"
                            @click="handleSwitchGame(game)"
                        >
                            <span class="game-switcher-item-icon-wrap">
                                <img
                                    v-if="game.iconPath"
                                    class="game-switcher-item-icon"
                                    :src="game.iconPath"
                                    alt=""
                                    draggable="false"
                                    @error="(e) => ((e.target as HTMLImageElement).style.opacity = '0')"
                                />
                            </span>
                            <span class="game-switcher-item-name">{{ getGamePresetDisplayName(game.name, t) }}</span>
                        </button>
                    </div>
                </div>
            </transition>
        </div>

        <!-- Navigation tabs (replaces the old WinUI3 sidebar) -->
        <nav v-if="!AppStateManager.isFirstRunOnboardingOpen.value" class="caption-tabs" aria-label="SSMT4">
            <div class="caption-tabs-track">
                <button
                    v-for="item in displayItems"
                    :key="item.id"
                    type="button"
                    class="caption-tab"
                    tabindex="-1"
                    :class="{ active: isNavItemActive(item), 'drag-hover': navHoverId === item.id, dragging: navDraggingId === item.id }"
                    :data-nav-id="item.id"
                    :aria-label="item.label"
                    :aria-current="isNavItemActive(item) ? 'page' : undefined"
                    @click="navTo(item.path)"
                    @mousedown="onNavMouseDown($event, item)"
                    @contextmenu.prevent.stop="openNavContextMenu($event, item)"
                >
                    <span class="caption-tab-icon" aria-hidden="true">
                            <svg v-if="item.id === 'home'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path><polyline points="9 22 9 12 15 12 15 22"></polyline></svg>
                            <svg v-else-if="item.id === 'work'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="4" width="16" height="16" rx="3"></rect><path d="M9 4v16"></path><path d="M4 9h16"></path><path d="M12.5 14h4"></path><path d="M12.5 17h2.5"></path></svg>
                            <svg v-else-if="item.id === 'mark-texture-full'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4h16v16H4z"></path><path d="M9 9h6"></path><path d="M9 13h6"></path><path d="M9 17h4"></path></svg>
                            <svg v-else-if="item.id === 'texture-mod-maker'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"></rect><circle cx="8" cy="9" r="2"></circle><path d="m4 17 5-5 3 3 2-2 6 6"></path><path d="M17 2v4M15 4h4"></path></svg>
                            <svg v-else-if="item.id === 'mods'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"></path><polyline points="3.27 6.96 12 12.01 20.73 6.96"></polyline><line x1="12" y1="22.08" x2="12" y2="12"></line></svg>
                            <svg v-else-if="item.id === 'gamebanana'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4h10l2 4-2 12H7L5 8l2-4z"></path><path d="M9 4l1 4h4l1-4"></path><path d="M9 12h6"></path><path d="M10 16h4"></path></svg>
                            <svg v-else-if="item.id === 'nexusmods'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4h12v16H6z"></path><path d="M9 8h6"></path><path d="M9 12h6"></path><path d="M9 16h4"></path></svg>
                            <svg v-else-if="item.id === 'xianzun'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"></path><path d="M8.5 11h7"></path><path d="M8.5 14.5h4"></path></svg>
                            <svg v-else-if="item.id === 'ui-builder'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M8 8h8v8H8z"></path><path d="M8 12h8M12 8v8"></path></svg>
                            <svg v-else-if="item.id === LIQUID_GRAVITY_PROBE_NAV_ID" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2.5S6.5 9 6.5 14a5.5 5.5 0 0 0 11 0C17.5 9 12 2.5 12 2.5Z"></path><path d="M9.5 15.2a2.8 2.8 0 0 0 2.8 2.2"></path><circle cx="12" cy="18.8" r=".8"></circle></svg>
                            <svg v-else-if="item.id === 'plugins'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M9 3v3M15 3v3M7 6h10a2 2 0 0 1 2 2v3h-3a2 2 0 1 0 0 4h3v1a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-3h3a2 2 0 1 0 0-4H5V8a2 2 0 0 1 2-2Z"></path></svg>
                    </span>
                    <span class="caption-tab-label">{{ item.label }}</span>
                </button>
            </div>
            <div class="side-nav-footer">
                <button
                    type="button"
                    class="caption-tab side-nav-settings"
                    tabindex="-1"
                    :class="{ active: route.path === '/settings' }"
                    :aria-label="t('titlebar.settings')"
                    :aria-current="route.path === '/settings' ? 'page' : undefined"
                    @click="toggleSettingsPage"
                >
                    <span class="caption-tab-icon" aria-hidden="true">
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1.82 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>
                    </span>
                    <span class="caption-tab-label">{{ t('titlebar.settings') }}</span>
                </button>
            </div>
        </nav>

        <!-- Reserved drag/spacing region: keeps a guaranteed gap between the
             tabs and the window controls, and is always available for
             dragging the window even when tabs fill the bar.
             Double-click toggles maximize, like a native caption bar. -->
        <div class="caption-spacer" @mousedown="startDrag" @dblclick="toggleMaximize"></div>

        <!-- Window controls (WinUI3 caption buttons) -->
        <div class="caption-controls">
            <button
                type="button"
                class="caption-button pin-button"
                :class="{ active: isPinned }"
                :aria-label="isPinned ? t('titlebar.unpinWindow') : t('titlebar.pinWindow')"
                @click="togglePin"
            >
                <svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M16 3L21 8 18 11 19 16 14 14 11 17H8L4 21 3 20 7 16V13L10 10 8 5 13 7 16 3Z"/>
                </svg>
            </button>

            <button type="button" class="caption-button" :aria-label="t('titlebar.windowControls.minimize')" @click="minimize">
                <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
                    <path d="M0,5 L10,5 L10,6 L0,6 Z" />
                </svg>
            </button>

            <button type="button" class="caption-button" :aria-label="t('titlebar.windowControls.maximize')" @click="toggleMaximize">
                <svg v-if="!isMaximized" xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
                    <path d="M1,1 L9,1 L9,9 L1,9 L1,1 Z M0,0 L0,10 L10,10 L10,0 L0,0 Z" />
                </svg>
                <svg v-else xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
                    <path d="M2.1,0v2H0v8.1h8.2v-2h2V0H2.1z M7.2,9.2H1V3h6.1V9.2z M9.2,7.1h-1V2H3.1V1h6.1V7.1z" />
                </svg>
            </button>

            <button type="button" class="caption-button close-button" :aria-label="t('titlebar.windowControls.close')" @click="close">
                <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10">
                    <path d="M1,0 L5,4 L9,0 L10,1 L6,5 L10,9 L9,10 L5,6 L1,10 L0,9 L4,5 L0,1 L1,0 Z" />
                </svg>
            </button>
        </div>

        <!-- Tab context menu (hide page) — teleported to body, positioned at cursor -->
        <Teleport to="body">
            <div
                v-if="navContextMenu.visible && navContextMenu.item"
                ref="navContextMenuRef"
                class="shell-flyout"
                :style="{ left: `${navContextMenu.x}px`, top: `${navContextMenu.y}px` }"
                @pointerdown.stop
                @contextmenu.prevent
                @keydown.esc="closeNavContextMenu(true)"
            >
                <button type="button" class="shell-flyout-item" :disabled="!canMoveNavItem(navContextMenu.item, -1)"
                    @click="moveNavItem(navContextMenu.item, -1)">
                    {{ t('titlebar.moveUp') }}
                </button>
                <button type="button" class="shell-flyout-item" :disabled="!canMoveNavItem(navContextMenu.item, 1)"
                    @click="moveNavItem(navContextMenu.item, 1)">
                    {{ t('titlebar.moveDown') }}
                </button>
                <button type="button" class="shell-flyout-item" @click="hideNavItem(navContextMenu.item)">
                    {{ t('titlebar.hidePage') }}
                </button>
            </div>
        </Teleport>
    </div>
</template>

<style scoped>
.caption-bar {
    position: relative;
    width: 100%;
    height: 100%;
    user-select: none;
    z-index: 30;
    pointer-events: none;
}

.caption-bar::before {
    content: '';
    position: absolute;
    top: 0;
    right: 0;
    left: 0;
    height: 32px;
    border-bottom: 1px solid rgba(255, 255, 255, .065);
    background:
        radial-gradient(circle at 84% 0%, rgba(var(--ssmt-brand-accent-rgb), .055), transparent 20%),
        linear-gradient(90deg, rgba(8, 9, 11, .985), rgba(12, 13, 15, .955));
    backdrop-filter: var(--shell-caption-blur);
    -webkit-backdrop-filter: var(--shell-caption-blur);
    pointer-events: none;
}

.caption-drag-region {
    position: absolute;
    top: 0;
    right: 0;
    left: 120px;
    height: 32px;
    cursor: grab;
    pointer-events: auto;
}

.caption-drag-region:active {
    cursor: grabbing;
}

.caption-brand {
    position: absolute;
    top: 0;
    left: 6px;
    z-index: 4;
    display: flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 2px 9px 2px 4px;
    border: 0;
    border-radius: 0 9px 9px 0;
    background: transparent;
    color: #f3f4f0;
    font: inherit;
    cursor: pointer;
    pointer-events: auto;
}

.caption-brand:hover {
    background: rgba(255, 255, 255, .055);
}

.caption-brand:focus-visible {
    outline: 2px solid var(--ssmt-brand-accent);
    outline-offset: -2px;
}

.caption-brand-mark {
    display: block;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    filter: drop-shadow(0 3px 8px rgba(0, 0, 0, .28));
    transition: filter 160ms ease, transform 160ms ease;
}

.caption-brand:hover .caption-brand-mark {
    filter:
        drop-shadow(0 3px 8px rgba(0, 0, 0, .28))
        drop-shadow(0 0 5px rgba(var(--ssmt-brand-accent-rgb), .24));
    transform: scale(1.04);
}

.caption-brand-name {
    font-size: 10px;
    font-weight: 750;
    letter-spacing: .11em;
    line-height: 1;
}

/* ═══════════ Game quick-switcher (far left of the caption bar) ═══════════ */
.game-switcher-root {
    position: absolute;
    top: auto;
    bottom: 65px;
    left: 0;
    display: flex;
    align-items: stretch;
    justify-content: center;
    width: var(--ssmt-side-nav-width);
    height: 48px;
    z-index: 5;
    pointer-events: auto;
    transition: opacity var(--motion-duration-fast) var(--motion-ease-out) var(--motion-delay-handoff),
        transform var(--motion-duration-standard) var(--motion-ease-out) var(--motion-delay-handoff), visibility 0s;
}

:global(.el-popper.caption-game-switcher-tooltip) {
    z-index: 2147483647 !important;
    pointer-events: none;
}

.game-switcher-root > :deep(.el-tooltip__trigger),
.game-switcher-root > .caption-button {
    width: 58px;
}

.game-switcher-button {
    position: relative;
    overflow: visible;
    height: 48px;
    border-radius: 10px;
}

.game-switcher-button:hover {
    background: rgba(255, 255, 255, .055);
}

.game-switcher-button.active {
    background: rgba(var(--ssmt-brand-accent-rgb), .08);
}

.game-switcher-icon-frame {
    display: grid;
    width: 27px;
    height: 27px;
    place-items: center;
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, .12);
    border-radius: 8px;
    background: rgba(255, 255, 255, .065);
    box-shadow: 0 4px 12px rgba(0, 0, 0, .20);
    transition: border-color 120ms ease, background 120ms ease, box-shadow 120ms ease;
}

.game-switcher-icon-frame > svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
}

.game-switcher-button:hover .game-switcher-icon-frame,
.game-switcher-button.active .game-switcher-icon-frame {
    border-color: rgba(var(--ssmt-brand-accent-rgb), .48);
    background: rgba(var(--ssmt-brand-accent-rgb), .11);
    box-shadow: 0 0 0 3px rgba(var(--ssmt-brand-accent-rgb), .055);
}

.game-switcher-chevron {
    position: absolute;
    right: 10px;
    bottom: 2px;
    display: grid;
    width: 11px;
    height: 11px;
    place-items: center;
    border: 1px solid rgba(255, 255, 255, .14);
    border-radius: 50%;
    background: rgba(10, 11, 13, .96);
    color: rgba(255, 255, 255, .72);
    box-shadow: 0 2px 6px rgba(0, 0, 0, .24);
    transition: transform 140ms ease, color 140ms ease;
    transform: rotate(180deg);
}

.game-switcher-chevron svg {
    width: 8px;
    height: 8px;
}

.game-switcher-button.active .game-switcher-chevron {
    color: var(--ssmt-brand-accent);
    transform: rotate(0);
}

.game-switcher-root.is-home-sidebar {
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
    transform: translate(-8px, 12px) scale(.85);
    transition-delay: 0s, 0s, var(--motion-duration-standard);
}

.game-switcher-pin-row {
    display: flex; align-items: center; justify-content: space-between;
    gap: 12px; padding: 0 4px 6px; font-size: 12px;
}

@media (prefers-reduced-motion: reduce) {
    .game-switcher-root { transition: none; }
}

.game-switcher-button.active {
    background: var(--shell-caption-btn-hover);
}

.game-switcher-icon {
    width: 23px;
    height: 23px;
    border-radius: 5px;
    object-fit: cover;
    display: block;
}

.game-switcher-card {
    position: absolute;
    top: auto;
    bottom: calc(100% + 8px);
    left: calc(100% + 9px);
    z-index: 60;
    width: 264px;
    max-height: min(470px, calc(100vh - 56px));
    overflow-y: auto;
    overflow-x: hidden;
    padding: 6px;
    box-sizing: border-box;
    border: 1px solid var(--shell-flyout-border);
    border-radius: 13px;
    background: color-mix(in srgb, var(--shell-flyout-bg) 94%, #08090b);
    backdrop-filter: blur(20px) saturate(1.3);
    -webkit-backdrop-filter: blur(20px) saturate(1.3);
    box-shadow: 0 18px 44px rgba(0, 0, 0, .32);
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.22) transparent;
}

.game-switcher-card-head {
    display: grid;
    gap: 2px;
    padding: 8px 10px 9px;
    margin-bottom: 2px;
    border-bottom: 1px solid var(--shell-divider);
}

.game-switcher-card-kicker {
    color: var(--shell-flyout-text-muted);
    font-size: 9px;
    font-weight: 650;
    letter-spacing: .08em;
    text-transform: uppercase;
}

.game-switcher-card-head strong {
    overflow: hidden;
    color: var(--shell-flyout-text);
    font-size: 13px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.game-switcher-group + .game-switcher-group {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--shell-divider);
}

.game-switcher-group-title {
    padding: 5px 10px 4px;
    color: var(--shell-flyout-text-muted);
    font-size: 11px;
    font-weight: 650;
    letter-spacing: 0.02em;
    white-space: nowrap;
}

.game-switcher-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 36px;
    padding: 4px 10px 4px 6px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--shell-flyout-text);
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
    transition: background 83ms ease;
    box-sizing: border-box;
}

.game-switcher-item:hover {
    background: var(--shell-flyout-hover);
}

.game-switcher-item.active {
    background: var(--shell-fill-selected);
    font-weight: 600;
}

.game-switcher-item-icon-wrap {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    overflow: hidden;
    background: rgba(255, 255, 255, 0.08);
}

.game-switcher-item-icon {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}

.game-switcher-item-name {
    min-width: 0;
    flex: 1 1 auto;
    line-height: 1.3;
    overflow-wrap: anywhere;
}

/* Card open/close animation */
.game-switcher-pop-enter-active,
.game-switcher-pop-leave-active {
    transition: opacity var(--motion-duration-fast) var(--motion-ease-out), transform var(--motion-duration-fast) var(--motion-ease-out);
}

.game-switcher-pop-enter-from,
.game-switcher-pop-leave-to {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
}

/* ═══════════ Navigation tabs ═══════════ */
.caption-tabs {
    --side-nav-accent: var(--ssmt-brand-accent);
    --side-nav-accent-ink: var(--ssmt-brand-accent-ink);
    position: absolute;
    top: 32px;
    bottom: 0;
    left: 0;
    width: var(--ssmt-side-nav-width);
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    gap: 6px;
    padding: 8px 0;
    z-index: 3;
    border-right: 1px solid rgba(255, 255, 255, .065);
    background:
        radial-gradient(circle at 50% 0%, rgba(var(--ssmt-brand-accent-rgb), .055), transparent 24%),
        rgba(8, 9, 11, .96);
    box-shadow: 8px 0 26px rgba(0, 0, 0, .16);
    pointer-events: auto;
}

.caption-bar:not(.is-legacy-ui) .caption-tabs {
    isolation: isolate;
    border-right-color: transparent;
    background: transparent;
    box-shadow: none;
    transition: border-color 160ms ease;
}

.caption-bar:not(.is-legacy-ui) .caption-tabs::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: -1;
    pointer-events: none;
    background:
        radial-gradient(circle at 50% 0%, rgba(var(--ssmt-brand-accent-rgb), .055), transparent 24%),
        linear-gradient(180deg, rgba(8, 10, 13, .72), rgba(8, 10, 13, .56));
    opacity: 0;
    backdrop-filter: blur(16px) saturate(1.15);
    -webkit-backdrop-filter: blur(16px) saturate(1.15);
    transition: opacity 160ms ease;
}

.caption-bar:not(.is-legacy-ui) .caption-tabs:hover {
    border-right-color: rgba(255, 255, 255, .1);
}

.caption-bar:not(.is-legacy-ui) .caption-tabs:hover::before {
    opacity: 1;
}

.caption-tabs-track {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    flex: 1;
    width: 100%;
    min-height: 0;
    padding: 0 0 58px;
    scroll-padding-bottom: 58px;
    overflow-x: hidden;
    overflow-y: auto;
    box-sizing: border-box;
    scrollbar-width: none;
}

.caption-tabs-track::-webkit-scrollbar {
    display: none;
}

.side-nav-footer {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 100%;
    padding-top: 7px;
    border-top: 1px solid rgba(255, 255, 255, .065);
}

.side-nav-settings { cursor: pointer; }

.caption-bar:not(.is-legacy-ui) .side-nav-footer {
    border-top-color: transparent;
    transition: border-color 160ms ease;
}

.caption-bar:not(.is-legacy-ui) .caption-tabs:hover .side-nav-footer {
    border-top-color: rgba(255, 255, 255, .065);
}

.caption-tab {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    flex: 0 0 48px;
    width: 58px;
    height: 48px;
    padding: 4px 2px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: rgba(255, 255, 255, .58);
    font: inherit;
    font-size: 9px;
    line-height: 1.05;
    cursor: pointer;
    white-space: normal;
    box-sizing: border-box;
    opacity: 1;
    transition: background-color 120ms ease, color 120ms ease, transform 120ms ease;
}

.caption-tab:hover {
    background: rgba(255, 255, 255, .065);
    color: rgba(255, 255, 255, .92);
}

.caption-tab:active {
    transform: scale(.96);
}

.caption-tab.active {
    background: var(--side-nav-accent);
    color: var(--side-nav-accent-ink);
    box-shadow: 0 8px 22px rgba(var(--ssmt-brand-accent-rgb), .10);
    font-weight: 650;
}

.caption-tab.active::after {
    display: none;
}

.caption-bar:not(.is-legacy-ui) .caption-tab {
    color: rgba(255, 255, 255, .82);
    text-shadow: 0 1px 3px rgba(0, 0, 0, .82), 0 0 8px rgba(0, 0, 0, .44);
}

.caption-bar:not(.is-legacy-ui) .caption-tab-icon svg {
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, .72));
}

.caption-bar:not(.is-legacy-ui) .caption-tab:not(.active):hover {
    background: rgba(7, 9, 12, .58);
    color: #fff;
}

.caption-bar:not(.is-legacy-ui) .caption-tab.active {
    color: var(--side-nav-accent-ink);
    text-shadow: none;
}

.caption-bar:not(.is-legacy-ui) .caption-tab.active .caption-tab-icon svg {
    filter: none;
}

.caption-tab-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    flex: 0 0 auto;
}

.caption-tab-icon svg {
    width: 18px;
    height: 18px;
    display: block;
    shape-rendering: geometricPrecision;
    filter: none;
}

.caption-tab-label {
    display: -webkit-box;
    min-width: 0;
    max-width: 54px;
    overflow: hidden;
    text-align: center;
    text-overflow: ellipsis;
    text-shadow: none;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
}

/* Drag-to-reorder affordances */
.caption-tab.drag-hover {
    background: rgba(var(--ssmt-brand-accent-rgb), .15);
    color: var(--ssmt-brand-accent-soft);
    outline: 1px dashed rgba(var(--ssmt-brand-accent-rgb), .58);
    outline-offset: -1px;
}

.caption-tab.dragging {
    opacity: 0.55;
}

@media (prefers-reduced-motion: reduce) {
    .caption-bar:not(.is-legacy-ui) .caption-tabs,
    .caption-bar:not(.is-legacy-ui) .caption-tabs::before,
    .caption-bar:not(.is-legacy-ui) .side-nav-footer {
        transition: none;
    }
}

/* ═══════════ Flyouts (tab context menu) ═══════════ */
.shell-flyout {
    position: fixed;
    z-index: 30000;
    min-width: 172px;
    padding: 5px;
    box-sizing: border-box;
    border: 1px solid var(--shell-flyout-border);
    border-radius: 8px;
    background: var(--shell-flyout-bg);
    backdrop-filter: blur(20px) saturate(1.3);
    -webkit-backdrop-filter: blur(20px) saturate(1.3);
    box-shadow: var(--shell-flyout-shadow);
    animation: shell-flyout-in 140ms cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes shell-flyout-in {
    from {
        opacity: 0;
        transform: translateY(-3px) scale(0.98);
    }
    to {
        opacity: 1;
        transform: translateY(0) scale(1);
    }
}

.shell-flyout-item {
    display: flex;
    align-items: center;
    width: 100%;
    height: 32px;
    padding: 0 11px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--shell-flyout-text);
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
    white-space: nowrap;
    transition: background 83ms ease;
}

.shell-flyout-item:hover {
    background: var(--shell-flyout-hover);
}

.shell-flyout-item:disabled {
    opacity: .38;
    cursor: default;
}

/* ═══════════ Window controls ═══════════ */
/* Reserved gap between tabs and window controls — absorbs all free space in
   the caption bar so there is always a drag handle, and never shrinks below
   its minimum width even when the tabs overflow. */
.caption-spacer {
    position: absolute;
    top: 0;
    right: 184px;
    left: var(--ssmt-side-nav-width);
    height: 32px;
    z-index: 2;
    cursor: grab;
    pointer-events: auto;
}

.caption-spacer:active {
    cursor: grabbing;
}

.caption-controls {
    position: absolute;
    top: 0;
    right: 0;
    display: flex;
    align-items: stretch;
    height: 32px;
    z-index: 2;
    pointer-events: auto;
}

/* WinUI3 caption button: 46x32, subtle hover fill, instant press feedback */
.caption-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 32px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--shell-caption-fg);
    cursor: default;
    transition: background-color 83ms ease, color 83ms ease;
}

.caption-button svg {
    display: block;
    fill: currentColor;
    filter: drop-shadow(var(--shell-caption-glyph-shadow));
}

.caption-button:hover {
    background: var(--shell-caption-btn-hover);
}

.caption-button:active {
    background: var(--shell-caption-btn-pressed);
}

/* Pin button */
.pin-button svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    transition: transform 167ms cubic-bezier(0.16, 1, 0.3, 1);
}

.pin-button.active {
    color: var(--shell-caption-fg);
    background: var(--shell-caption-btn-hover);
}

.pin-button.active svg {
    fill: currentColor;
    transform: rotate(-45deg);
}

/* Close button — Windows red */
.close-button:hover {
    background: var(--shell-caption-close-hover);
    color: #ffffff;
}

.close-button:active {
    background: var(--shell-caption-close-pressed);
    color: #ffffff;
}

.close-button:hover svg,
.close-button:active svg {
    filter: none;
}

/* ── Official v4.2.2 vertical-to-horizontal layout override ── */
.caption-bar.is-legacy-ui {
    display: flex;
    align-items: stretch;
    height: 32px;
    pointer-events: auto;
    background: var(--shell-caption-scrim);
    backdrop-filter: var(--shell-caption-blur);
    -webkit-backdrop-filter: var(--shell-caption-blur);
}

.caption-bar.is-legacy-ui::before {
    display: none;
}

.is-legacy-ui .caption-brand {
    display: none;
}

.is-legacy-ui .caption-drag-region {
    inset: 0;
}

.is-legacy-ui .game-switcher-root {
    position: relative;
    top: auto;
    bottom: auto;
    left: auto;
    width: auto;
    height: 32px;
    flex: none;
    z-index: 2;
}

.is-legacy-ui .game-switcher-button {
    width: 46px;
    height: 32px;
    border-radius: 0;
}

.is-legacy-ui .game-switcher-icon-frame {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    border-color: transparent;
    background: transparent;
    box-shadow: none;
}

.is-legacy-ui .game-switcher-chevron {
    display: none;
}

.is-legacy-ui .caption-tabs {
    position: relative;
    inset: auto;
    display: contents;
    flex-direction: row;
    width: auto;
    height: 32px;
    min-width: 0;
    padding: 0;
    gap: 0;
    border: 0;
    background: transparent;
    box-shadow: none;
}

.is-legacy-ui .caption-tabs::before {
    display: none;
}

.is-legacy-ui .caption-tabs-track {
    flex: 0 1 auto;
    flex-direction: row;
    align-items: stretch;
    width: auto;
    height: 32px;
    padding: 0;
    gap: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scroll-padding-bottom: 0;
}

.is-legacy-ui .caption-tab {
    flex: 0 0 auto;
    flex-direction: row;
    width: auto;
    height: 32px;
    padding: 0 11px;
    gap: 7px;
    border-radius: 0;
    font-size: 12px;
    line-height: 1;
    color: var(--shell-caption-fg);
    opacity: .78;
}

.is-legacy-ui .caption-tab:hover {
    background: var(--shell-caption-btn-hover);
    opacity: 1;
}

.is-legacy-ui .caption-tab.active {
    background: var(--shell-caption-btn-hover);
    color: var(--shell-caption-fg);
    box-shadow: none;
    font-weight: 600;
    opacity: 1;
}

.is-legacy-ui .caption-tab-icon,
.is-legacy-ui .caption-tab-icon svg {
    width: 14px;
    height: 14px;
}

.is-legacy-ui .caption-tab-label {
    display: block;
    max-width: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    -webkit-line-clamp: unset;
}

.is-legacy-ui .side-nav-footer {
    flex: none;
    margin-left: auto;
    width: auto;
    padding: 0;
    border: 0;
}

.is-legacy-ui .side-nav-settings {
    flex-direction: row;
    width: auto;
    height: 32px;
    padding: 0 8px;
    gap: 5px;
    border-radius: 0;
}

.is-legacy-ui .caption-spacer {
    position: relative;
    inset: auto;
    flex: 0 0 0;
    min-width: 0;
    height: 32px;
}

.is-legacy-ui .caption-controls {
    position: relative;
    inset: auto;
    flex: none;
    height: 32px;
}
</style>
