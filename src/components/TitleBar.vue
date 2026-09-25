<script setup lang="ts">
import { computed, nextTick, reactive, ref, onMounted, onUnmounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { ElMessage } from 'element-plus';
import { getAlwaysOnTop, setAlwaysOnTop } from '../store/WindowPinStore';
import { AppStateManager, type GameInfo } from '../store/AppStateManager';
import { getGamePresetDisplayName } from '../store/GamePreset';
import { calculateContextMenuPosition } from '../utils/ContextMenuPosition';

const appWindow = getCurrentWindow();
const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const isMaximized = ref(false);
const isPinned = ref(false);
const captionBarRef = ref<HTMLElement | null>(null);

const appSettings = AppStateManager.appSettings;
const gamesList = AppStateManager.gamesList;

/* ═══════════════════════ Game quick-switcher ═══════════════════════ */

const currentGame = computed(() => gamesList.find(g => g.name === appSettings.CurrentGameName));

/** Favorite games (formerly the sidebar strip), honoring the user's manual order. */
const favoriteGames = computed(() => {
    const orderIndex = new Map(appSettings.sidebarGameOrder.map((name, index) => [name, index]));
    return gamesList
        .filter(g => g.showSidebar)
        .sort((left, right) => {
            const leftIndex = orderIndex.get(left.name) ?? Number.MAX_SAFE_INTEGER;
            const rightIndex = orderIndex.get(right.name) ?? Number.MAX_SAFE_INTEGER;
            return leftIndex - rightIndex;
        });
});

const otherGames = computed(() => gamesList.filter(g => !g.showSidebar));

const isGameSwitcherOpen = ref(false);

const toggleGameSwitcher = () => {
    isGameSwitcherOpen.value = !isGameSwitcherOpen.value;
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

const displayItems = computed<NavItem[]>(() => {
    const map = new Map(allNavItems.map(i => [i.id, i]));
    const result: NavItem[] = [];

    navOrder.value.forEach(id => {
        const item = map.get(id);
        if (item) {
            result.push({ ...item, label: t(item.labelKey) });
        }
    });

    allNavItems.forEach(i => {
        if (!navOrder.value.includes(i.id)) {
            result.push({ ...i, label: t(i.labelKey) });
        }
    });

    return result.filter(item => {
        const key = visibilityKeyByNavId[item.id];
        return !key || appSettings.pageVisibility[key];
    });
});

const isNavItemActive = (item: NavItem) => {
    if (item.path === '/') return route.path === '/';
    return route.path === item.path || route.path.startsWith(`${item.path}/`);
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

const closeNavContextMenu = () => {
    navContextMenu.visible = false;
    navContextMenu.item = null;
};

const openNavContextMenu = (event: MouseEvent, item: NavItem) => {
    // Only pages with a visibility key can be hidden — home always stays.
    if (!visibilityKeyByNavId[item.id]) return;

    navContextMenu.item = item;
    navContextMenu.x = event.clientX;
    navContextMenu.y = event.clientY;
    navContextMenu.visible = true;

    nextTick(() => {
        const menuEl = navContextMenuRef.value;
        if (!menuEl) return;
        const rect = menuEl.getBoundingClientRect();
        const pos = calculateContextMenuPosition({
            clientX: event.clientX,
            clientY: event.clientY,
            menuWidth: rect.width,
            menuHeight: rect.height,
        });

        // The caption bar host sits at the maximum z-index, so a menu that
        // overlaps the bar would be covered by it — push the menu fully
        // below the bar's bottom edge instead.
        const captionBottom = captionBarRef.value?.getBoundingClientRect().bottom ?? 0;
        const minY = captionBottom + 4;

        navContextMenu.x = pos.x;
        navContextMenu.y = Math.max(pos.y, minY);
    });
};

const hideNavItem = async (item: NavItem) => {
    const key = visibilityKeyByNavId[item.id];
    if (!key) return;
    appSettings.pageVisibility[key] = false;
    closeNavContextMenu();
    await AppStateManager.saveSettingsNow();
};

/** Tab strip: vertical wheel scrolls horizontally when tabs overflow. */
const tabsTrackRef = ref<HTMLElement | null>(null);
const handleTabsWheel = (event: WheelEvent) => {
    const el = tabsTrackRef.value;
    if (!el) return;
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    event.preventDefault();
    el.scrollLeft += event.deltaY;
};

/* ═══════════════════════ Window controls ═══════════════════════ */

const checkMaximized = async () => {
    isMaximized.value = await appWindow.isMaximized();
};

let unlistenResize: (() => void) | null = null;

onMounted(async () => {
    checkMaximized();
    loadOrder();
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

onUnmounted(() => {
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
const startDrag = (event: MouseEvent) => {
    // Skip the drag on the second click of a double-click so the dblclick
    // handler (maximize toggle) can fire instead of restarting the drag.
    if (event.detail > 1) return;
    appWindow.startDragging();
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
    <div ref="captionBarRef" class="caption-bar">
        <!-- Drag region (window move; double-click toggles maximize) -->
        <div class="caption-drag-region" data-tauri-drag-region @mousedown="startDrag" @dblclick="toggleMaximize"></div>

        <!-- Game quick-switcher at the far left: current game icon, opens the game picker card -->
        <div class="game-switcher-root">
            <el-tooltip :content="t('titlebar.selectGame')" placement="bottom" :show-after="250" :disabled="isGameSwitcherOpen">
                <button
                    type="button"
                    class="caption-button game-switcher-button"
                    :class="{ active: isGameSwitcherOpen }"
                    :aria-label="t('titlebar.selectGame')"
                    :aria-expanded="isGameSwitcherOpen"
                    @click="toggleGameSwitcher"
                >
                    <img
                        v-if="currentGame?.iconPath"
                        class="game-switcher-icon"
                        :src="currentGame.iconPath"
                        :alt="getGamePresetDisplayName(currentGame.name, t)"
                        draggable="false"
                        @error="(e) => ((e.target as HTMLImageElement).style.opacity = '0')"
                    />
                    <svg v-else xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                        <rect x="3" y="3" width="7" height="7" rx="1.5"></rect>
                        <rect x="14" y="3" width="7" height="7" rx="1.5"></rect>
                        <rect x="14" y="14" width="7" height="7" rx="1.5"></rect>
                        <rect x="3" y="14" width="7" height="7" rx="1.5"></rect>
                    </svg>
                </button>
            </el-tooltip>

            <transition name="game-switcher-pop">
                <div v-if="isGameSwitcherOpen" class="game-switcher-card" role="menu" :aria-label="t('titlebar.selectGame')">
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
        <nav class="caption-tabs" aria-label="SSMT4">
            <div ref="tabsTrackRef" class="caption-tabs-track" @wheel="handleTabsWheel">
                <el-tooltip
                    v-for="item in displayItems"
                    :key="item.id"
                    :content="item.label"
                    placement="bottom"
                    :show-after="400"
                >
                    <button
                        type="button"
                        class="caption-tab"
                        :class="{ active: isNavItemActive(item), 'drag-hover': navHoverId === item.id, dragging: navDraggingId === item.id }"
                        :data-nav-id="item.id"
                        :aria-label="item.label"
                        @click="navTo(item.path)"
                        @mousedown.prevent="onNavMouseDown($event, item)"
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
                            <svg v-else-if="item.id === 'plugins'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M9 3v3M15 3v3M7 6h10a2 2 0 0 1 2 2v3h-3a2 2 0 1 0 0 4h3v1a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-3h3a2 2 0 1 0 0-4H5V8a2 2 0 0 1 2-2Z"></path></svg>
                        </span>
                        <span class="caption-tab-label">{{ item.label }}</span>
                    </button>
                </el-tooltip>
            </div>
        </nav>

        <!-- Reserved drag/spacing region: keeps a guaranteed gap between the
             tabs and the window controls, and is always available for
             dragging the window even when tabs fill the bar.
             Double-click toggles maximize, like a native caption bar. -->
        <div class="caption-spacer" data-tauri-drag-region @mousedown="startDrag" @dblclick="toggleMaximize"></div>

        <!-- Window controls (WinUI3 caption buttons) -->
        <div class="caption-controls">
            <el-tooltip :content="isPinned ? t('titlebar.unpinWindow') : t('titlebar.pinWindow')" placement="bottom" :show-after="250">
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
            </el-tooltip>

            <el-tooltip :content="t('titlebar.settings')" placement="bottom" :show-after="250">
                <button
                    type="button"
                    class="caption-button settings-button"
                    :class="{ active: route.path === '/settings' }"
                    :aria-label="t('titlebar.settings')"
                    @click="toggleSettingsPage"
                >
                    <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1.82 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>
                </button>
            </el-tooltip>

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
            >
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
    height: 32px;
    flex: 0 0 auto;
    display: flex;
    align-items: stretch;
    justify-content: flex-start;
    user-select: none;
    z-index: 30;
    /* Frosted scrim: blurs the game background behind the bar so tabs and
       caption glyphs stay readable no matter how bright the backdrop is. */
    background: var(--shell-caption-scrim);
    backdrop-filter: var(--shell-caption-blur);
    -webkit-backdrop-filter: var(--shell-caption-blur);
}

.caption-drag-region {
    position: absolute;
    inset: 0;
    cursor: default;
}

/* ═══════════ Game quick-switcher (far left of the caption bar) ═══════════ */
.game-switcher-root {
    position: relative;
    display: flex;
    align-items: stretch;
    height: 100%;
    z-index: 1;
}

.game-switcher-button.active {
    background: var(--shell-caption-btn-hover);
}

.game-switcher-icon {
    width: 18px;
    height: 18px;
    border-radius: 5px;
    object-fit: cover;
    display: block;
    filter: drop-shadow(var(--shell-caption-glyph-shadow));
}

.game-switcher-card {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 40;
    width: 248px;
    max-height: min(420px, calc(100vh - 64px));
    overflow-y: auto;
    overflow-x: hidden;
    padding: 6px;
    box-sizing: border-box;
    border: 1px solid var(--shell-flyout-border);
    border-radius: 10px;
    background: var(--shell-flyout-bg);
    backdrop-filter: blur(20px) saturate(1.3);
    -webkit-backdrop-filter: blur(20px) saturate(1.3);
    box-shadow: var(--shell-flyout-shadow);
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.22) transparent;
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
    transition: opacity 140ms cubic-bezier(0.16, 1, 0.3, 1), transform 140ms cubic-bezier(0.16, 1, 0.3, 1);
}

.game-switcher-pop-enter-from,
.game-switcher-pop-leave-to {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
}

/* ═══════════ Navigation tabs ═══════════ */
.caption-tabs {
    position: relative;
    flex: 0 1 auto;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: stretch;
    z-index: 1;
}

.caption-tabs-track {
    display: flex;
    align-items: stretch;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    /* Scrollbars always hidden — wheel scrolling stays */
    scrollbar-width: none;
}

.caption-tabs-track::-webkit-scrollbar {
    display: none;
}

.caption-tab {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    flex: 0 0 auto;
    max-width: 148px;
    height: 100%;
    padding: 0 12px;
    border: 0;
    background: transparent;
    color: var(--shell-caption-fg);
    font: inherit;
    font-size: 12px;
    cursor: default;
    white-space: nowrap;
    box-sizing: border-box;
    opacity: 0.78;
    transition: background-color 83ms ease, opacity 83ms ease;
}

.caption-tab:hover {
    background: var(--shell-caption-btn-hover);
    opacity: 1;
}

.caption-tab:active {
    background: var(--shell-caption-btn-pressed);
}

.caption-tab.active {
    background: var(--shell-caption-btn-hover);
    opacity: 1;
    font-weight: 600;
}

/* Active tab indicator — a short accent bar at the bottom edge */
.caption-tab.active::after {
    content: '';
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 0;
    height: 2px;
    border-radius: 2px 2px 0 0;
    background: var(--shell-accent);
}

.caption-tab-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    flex: 0 0 auto;
}

.caption-tab-icon svg {
    width: 14px;
    height: 14px;
    display: block;
    shape-rendering: geometricPrecision;
    filter: drop-shadow(var(--shell-caption-glyph-shadow));
}

.caption-tab-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    text-shadow: var(--shell-caption-glyph-shadow);
}

/* Drag-to-reorder affordances */
.caption-tab.drag-hover {
    background: var(--shell-caption-btn-hover);
    outline: 1px dashed var(--shell-accent-soft);
    outline-offset: -1px;
    opacity: 1;
}

.caption-tab.dragging {
    opacity: 0.55;
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

/* ═══════════ Window controls ═══════════ */
/* Reserved gap between tabs and window controls — absorbs all free space in
   the caption bar so there is always a drag handle, and never shrinks below
   its minimum width even when the tabs overflow. */
.caption-spacer {
    position: relative;
    flex: 1 1 auto;
    min-width: 72px;
    height: 100%;
    z-index: 1;
}

.caption-controls {
    position: relative;
    display: flex;
    align-items: stretch;
    height: 100%;
    z-index: 1;
}

/* WinUI3 caption button: 46x32, subtle hover fill, instant press feedback */
.caption-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 100%;
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

/* Settings button (right of pin) */
.settings-button svg {
    fill: none;
    stroke: currentColor;
    transition: transform 250ms cubic-bezier(0.16, 1, 0.3, 1);
}

.settings-button:hover svg {
    transform: rotate(40deg);
}

.settings-button.active {
    color: var(--shell-caption-fg);
    background: var(--shell-caption-btn-hover);
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
</style>
