import { createApp } from "vue";
import App from "./App.vue";
import router, { prefetchRouteComponents } from "./router";
import { ArrowLeft, ArrowRight, Delete, Download, Edit, Folder, FolderAdd, FolderOpened, Picture, Plus, Refresh, RefreshRight, VideoPlay } from '@element-plus/icons-vue';
import 'element-plus/dist/index.css';

import 'element-plus/theme-chalk/dark/css-vars.css';
import './styles/index.css';
import { AppStateManager } from "./store/AppStateManager";
import { pinia } from "./pinia";
import { getCurrentWindow } from '@tauri-apps/api/window';
import { bindI18nLocaleToSettings, i18n } from "./i18n";
import { triggerSilentStartupAppUpdateCheck } from "./common/AppSelfUpdate";
import { announceStartupVisible } from "./common/StartupExperience";

const app = createApp(App);

const tryGetCurrentWindow = () => {
  try {
    return getCurrentWindow();
  } catch (err) {
    console.error('Failed to access Tauri current window during bootstrap:', err);
    return null;
  }
};

window.addEventListener('error', (event) => {
  console.error('Global runtime error:', event.error || event.message);
});

window.addEventListener('unhandledrejection', (event) => {
  console.error('Unhandled promise rejection:', event.reason);
});

app.use(pinia);
app.use(i18n);

bindI18nLocaleToSettings(AppStateManager.appSettings);

app.component('ArrowLeft', ArrowLeft)
app.component('ArrowRight', ArrowRight)
app.component('Delete', Delete)
app.component('Download', Download)
app.component('Edit', Edit)
app.component('Folder', Folder)
app.component('FolderAdd', FolderAdd)
app.component('FolderOpened', FolderOpened)
app.component('Picture', Picture)
app.component('Plus', Plus)
app.component('Refresh', Refresh)
app.component('RefreshRight', RefreshRight)
app.component('VideoPlay', VideoPlay)

app.use(router);
app.mount("#app");

let startupAnnounced = false;
const revealStartupExperience = () => {
  if (startupAnnounced) return;
  startupAnnounced = true;
  announceStartupVisible();
};

AppStateManager.initAppState().then(() => {
  // Warm sub-page chunks in the background so first-time page switches are fast
  prefetchRouteComponents();

  const appWindow = tryGetCurrentWindow();
  if (!appWindow) {
    revealStartupExperience();
    return;
  }

  const finalizeWindowBootstrap = async () => {
    try {
      await AppStateManager.prepareWindowForDisplay();
      await appWindow.show();
      revealStartupExperience();
      await appWindow.setFocus();

      triggerSilentStartupAppUpdateCheck();
    } catch (err) {
      revealStartupExperience();
      console.warn('Failed to show/focus window:', err);
    }
  }

  void finalizeWindowBootstrap()
}).catch((err: unknown) => {
  console.error('initAppState failed:', err);
  // Ensure window shows even if init fails.
  const appWindow = tryGetCurrentWindow();
  if (!appWindow) {
    revealStartupExperience();
    return;
  }

  void AppStateManager.prepareWindowForDisplay()
    .catch((prepareErr: unknown) => {
      console.warn('Failed to prepare window after init error:', prepareErr);
    })
    .finally(() => {
      appWindow.show().catch((showErr: unknown) => {
        console.warn('Failed to show window after init error:', showErr);
      }).finally(() => {
        revealStartupExperience();
        triggerSilentStartupAppUpdateCheck();
      });
    });
});
