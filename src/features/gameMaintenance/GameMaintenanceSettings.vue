<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { dirname } from '@tauri-apps/api/path';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import type { GameMaintenanceStatus as Status, GameMaintenanceTask as Task } from './types';
import referenceNotices from './NOTICE.md?raw';
import { useInstallSpace } from './useInstallSpace';

const props = defineProps<{ gameName: string; gamePreset: string; targetExePath: string }>();
const emit = defineEmits<{ (e: 'completed'): void }>();
const { locale } = useI18n();
const text = (zh: string, en: string) => locale.value.startsWith('zh') ? zh : en;
const status = ref<Status | null>(null);
const task = ref<Task | null>(null);
const loading = ref(false);
const starting = ref(false);
const error = ref('');
const queryError = ref('');
const directory = ref('');
const server = ref('cn');
const languages = ref(['zh-cn']);
const active = computed(() => task.value?.state === 'running');
const ownTask = computed(() => task.value?.gameName === props.gameName ? task.value : null);
const switching = computed(() => !!status.value?.currentServer && status.value.currentServer !== server.value);
const canPreload = computed(() => status.value?.selectedServer === server.value && !!status.value?.preDownloadVersion
  && !!status.value?.clientFound && !switching.value);
const percent = computed(() => ownTask.value?.bytesTotal ? Math.min(100, Math.floor(ownTask.value.bytesDone / ownTask.value.bytesTotal * 100)) : 0);
const voiceNames = [['zh-cn', '中文', 'Chinese'], ['en-us', '英语', 'English'], ['ja-jp', '日语', 'Japanese'], ['ko-kr', '韩语', 'Korean']];
const installSpace = useInstallSpace(() => status.value && !status.value.clientFound && !active.value && directory.value && languages.value.length
  ? { gameName: props.gameName, serverId: server.value, operation: 'install', directory: directory.value, audioLanguages: [...languages.value] }
  : null, false);
const { estimate: spaceEstimate, loading: spaceLoading, error: spaceError } = installSpace;
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
let revision = 0;
let disposed = false;
let polling = false;
let completedId = '';
let taskRevision = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let refreshTimer: ReturnType<typeof setTimeout> | undefined;

function scheduleRefresh() {
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => { if (!disposed && !starting.value) void refresh(server.value); }, 200);
}

async function refresh(selected?: string, localOnly = false) {
  const id = ++revision;
  const gameName = props.gameName;
  const previousLanguages = JSON.stringify(languages.value);
  loading.value = !localOnly; queryError.value = '';
  try {
    const result = await invoke<Status>('hoyoplay_status', { gameName, serverId: selected ?? null, localOnly, homeOnly: true });
    if (disposed || id !== revision) return;
    status.value = result;
    if (!selected) {
      server.value = result.selectedServer;
      if (!directory.value && result.directory) directory.value = result.directory;
      if (JSON.stringify(languages.value) === previousLanguages) languages.value = result.audioLanguages;
    }
  } catch (e) { if (!disposed && id === revision) queryError.value = String(e); }
  finally {
    if (!disposed && id === revision) {
      loading.value = false;
      if (localOnly) scheduleRefresh();
    }
  }
}
function selectServer(value: string) {
  ++revision;
  loading.value = false;
  server.value = value;
  error.value = '';
  if (status.value) status.value = { ...status.value, selectedServer: value, latestVersion: '', preDownloadVersion: null };
  void refresh(value, true);
}
async function poll() {
  if (disposed || polling) return;
  polling = true;
  const id = taskRevision;
  try {
    const result = await invoke<Task | null>('hoyoplay_task');
    if (disposed || id !== taskRevision) return;
    task.value = result;
    if (result && result.id !== completedId && result.state !== 'running' && result.gameName === props.gameName) {
      completedId = result.id;
      emit('completed');
      if (result?.state === 'completed') void refresh(undefined, true);
    }
  } catch { /* 进度查询失败不会终止后台下载，下一次轮询重新连接。 */ }
  finally {
    polling = false;
    if (!disposed) timer = setTimeout(() => void poll(), 1000);
  }
}
async function chooseDirectory() {
  const selected = await open({ directory: true, multiple: false, title: text('选择空的游戏安装目录', 'Choose an empty installation folder') });
  if (typeof selected === 'string') directory.value = selected;
}
function toggleLanguage(value: string) {
  if (active.value || starting.value) return;
  languages.value = languages.value.includes(value) ? languages.value.filter(v => v !== value) : [...languages.value, value];
}
async function start(operation: string) {
  if (starting.value || active.value) return;
  starting.value = true;
  if (refreshTimer) clearTimeout(refreshTimer);
  ++revision;
  loading.value = false;
  const gameName = props.gameName;
  const selectedServer = server.value;
  const selectedDirectory = directory.value;
  const selectedLanguages = [...languages.value];
  error.value = '';
  const labels: Record<string, string> = {
    install: text('安装', 'Install'), update: text('更新 / 修复', 'Update / repair'), preload: text('预下载', 'Pre-download'),
    switch: text('切换原神客户端', 'Convert Genshin client'), recover: text('恢复原客户端', 'Restore original client'),
  };
  try {
    // 用户确定操作后才联网；结果只用于本次执行，不回写用户选择。
    if (operation !== 'recover') {
      const checked = await invoke<Status>('hoyoplay_status', { gameName, serverId: selectedServer });
      if (disposed || props.gameName !== gameName || server.value !== selectedServer || directory.value !== selectedDirectory
        || JSON.stringify(languages.value) !== JSON.stringify(selectedLanguages)) return;
      status.value = checked;
      if (checked.recoveryPending) throw text('请先恢复上次中断的客户端。', 'Restore the interrupted client first.');
      if (operation === 'install' || operation === 'update') {
        operation = checked.clientFound ? (checked.currentServer && checked.currentServer !== selectedServer ? 'switch' : 'update') : 'install';
      }
      if (operation === 'switch' && props.gamePreset !== 'GIMI') throw text('当前游戏暂不支持换服。', 'Client conversion is not supported for this game.');
      if (operation === 'preload' && (!checked.clientFound || !checked.preDownloadVersion || (checked.currentServer && checked.currentServer !== selectedServer))) {
        throw text('当前客户端或所选渠道没有可用的预下载。', 'Pre-download is unavailable for this client or channel.');
      }
    }
    let spaceInfo = '';
    if (operation === 'install') {
      const space = await installSpace.refresh();
      if (!space) return;
      if (!space.sufficient) throw text('磁盘空间不足，请更换安装目录或释放空间。', 'Insufficient disk space. Choose another folder or free some space.');
      spaceInfo = `\n${text('预计需要可用空间', 'Estimated free space required')}: ${gib(space.requiredBytes)} · ${text('可用', 'Available')}: ${gib(space.availableBytes)}`;
    }
    if (disposed || props.gameName !== gameName || directory.value !== selectedDirectory || server.value !== selectedServer
      || JSON.stringify(languages.value) !== JSON.stringify(selectedLanguages)) return;
    const info = operation === 'preload'
      ? text('仅下载到缓存，不替换当前客户端。', 'Download to cache without replacing the current client.')
      : operation === 'recover'
        ? text('恢复上次中断提交前的客户端文件。请先关闭游戏。', 'Restore client files from before the interrupted commit. Close the game first.')
        : text('请先关闭游戏。文件校验完成后才会提交；下载期间可暂停，下次继续。', 'Close the game first. Files are committed after verification; downloads can be paused and resumed.');
    const confirmChannel = !!status.value?.clientFound && !status.value?.currentServer;
    const channelWarning = confirmChannel ? text('\n当前客户端未声明渠道，请确认你选择的渠道与它一致。', '\nThe existing client does not declare its channel. Confirm that your selection matches it.') : '';
    await ElMessageBox.confirm(`${info}${channelWarning}\n\n${selectedDirectory}\n${selectedServer} · ${status.value?.latestVersion ?? ''}${spaceInfo}`, labels[operation], {
      confirmButtonText: labels[operation], cancelButtonText: text('取消', 'Cancel'), type: 'info',
    });
    if (disposed || props.gameName !== gameName || directory.value !== selectedDirectory || server.value !== selectedServer
      || JSON.stringify(languages.value) !== JSON.stringify(selectedLanguages)) return;
    const started = await invoke<Task>('hoyoplay_start', { request: {
      gameName, serverId: selectedServer, operation, directory: selectedDirectory, audioLanguages: selectedLanguages, confirmUnrecognizedChannel: confirmChannel,
    } });
    ++taskRevision;
    task.value = started;
  } catch (e) { if (e !== 'cancel' && e !== 'close') error.value = String(e); }
  finally { starting.value = false; }
}
async function pause() {
  try { await invoke('hoyoplay_cancel'); }
  catch (e) { error.value = String(e); }
}
watch(() => props.gameName, async () => {
  ++revision; status.value = null; error.value = ''; directory.value = ''; server.value = 'cn';
  const gameName = props.gameName;
  if (props.targetExePath) {
    const folder = await dirname(props.targetExePath);
    if (disposed || gameName !== props.gameName) return;
    directory.value = folder;
  }
  void refresh(undefined, true);
}, { immediate: true });
void poll();
onBeforeUnmount(() => { disposed = true; ++revision; if (timer) clearTimeout(timer); if (refreshTimer) clearTimeout(refreshTimer); });
</script>

<template>
  <section class="maintenance">
    <div class="section-heading">
      <p>{{ text('管理渠道、语音与客户端更新', 'Manage your channel, voices and client updates') }}</p>
      <el-button plain :loading="loading" :disabled="active || starting" @click="refresh(server)">{{ text('刷新版本', 'Refresh') }}</el-button>
    </div>
    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon />
    <div class="version-overview">
      <div class="overview-heading"><span>{{ text('版本概览', 'Version overview') }}</span>
        <span class="sync-status" :class="{ pending: loading || !status?.latestVersion, failed: queryError }" :title="queryError">
          <i></i>{{ loading ? text('后台同步中', 'Syncing in background') : queryError ? text('同步失败，可重试', 'Sync failed · Retry') : status?.latestVersion ? text('已获取版本信息', 'Version information available') : text('准备同步', 'Preparing to sync') }}
        </span>
      </div>
      <div class="versions" :aria-busy="loading">
        <div><span>{{ text('本地客户端', 'Installed client') }}</span><strong>{{ status?.installedVersion || (status?.clientFound ? '—' : text('未安装', 'Not installed')) }}</strong></div>
        <div><span>{{ text('最新正式版', 'Latest release') }}</span><strong :class="{ skeleton: !status?.latestVersion && !queryError }">{{ status?.latestVersion || '—' }}</strong></div>
        <div><span>{{ text('预下载', 'Pre-download') }}</span><strong :class="{ skeleton: !status?.latestVersion && !queryError }">{{ status?.latestVersion ? (status.preDownloadVersion ?? text('暂无', 'Unavailable')) : '—' }}</strong></div>
      </div>
    </div>
    <div class="configuration-card">
    <div class="card-heading"><h4>{{ text('安装选项', 'Client options') }}</h4><span>{{ text('选择即时生效，执行前校验', 'Choose freely · Verified before execution') }}</span></div>
    <div class="channel-row">
      <span class="field-label">{{ text('客户端渠道', 'Client channel') }}</span>
      <div class="pills">
        <button v-for="entry in (status?.servers ?? [{ id: 'cn', name: '国服' }, { id: 'bilibili', name: 'B服' }, { id: 'global', name: '国际服' }])" :key="entry.id"
          class="pill" :class="{ selected: server === entry.id }" :aria-pressed="server === entry.id" :disabled="active || starting" @click="selectServer(entry.id)">
          {{ text(entry.name, ({ cn: 'China', bilibili: 'Bilibili', global: 'Global' } as Record<string, string>)[entry.id] ?? entry.name) }}
        </button>
      </div>
    </div>
    <div class="folder-row">
      <label class="field-label">{{ text('游戏目录', 'Game folder') }}</label>
      <code :title="directory">{{ directory || text('请选择空的安装目录', 'Choose an empty installation folder') }}</code>
      <el-button v-if="!status?.clientFound" :disabled="active || starting" @click="chooseDirectory">{{ text('选择目录', 'Browse') }}</el-button>
    </div>
    <div class="channel-row">
      <span class="field-label">{{ text('语音资源', 'Voice packages') }}</span>
      <div class="pills"><button v-for="voice in voiceNames" :key="voice[0]" class="pill" :class="{ selected: languages.includes(voice[0]) }"
        :disabled="active || starting" :aria-pressed="languages.includes(voice[0])" @click="toggleLanguage(voice[0])">{{ text(voice[1], voice[2]) }}</button></div>
    </div>
    <p class="hint">{{ text('已安装的语音会一并更新；取消选择不会卸载语音。预下载缓存位于游戏目录，需要额外磁盘空间。', 'Installed voices are also updated; deselecting does not uninstall them. Pre-download cache uses extra space in the game folder.') }}</p>
    </div>
    <div v-if="status && !status.clientFound && directory && languages.length && !active" class="space-card" aria-live="polite" :aria-busy="spaceLoading">
      <div class="progress-heading"><strong>{{ text('安装空间预估', 'Installation space estimate') }}</strong>
        <el-button text :loading="spaceLoading" :disabled="starting || loading" @click="installSpace.refresh()">{{ text('重新估算', 'Recalculate') }}</el-button></div>
      <p v-if="spaceLoading">{{ text('正在读取官方资源大小与磁盘可用空间…', 'Checking official resource sizes and available disk space…') }}</p>
      <p v-else-if="spaceError" class="space-warning">{{ spaceError }}</p>
      <p v-else-if="!spaceEstimate">{{ text('选择完成后点击安装，将检查资源大小与可用空间。', 'Choose your options, then install to check resource sizes and free space.') }}</p>
      <template v-else-if="spaceEstimate">
        <div class="space-values">
          <div><span>{{ text('游戏与所选语音', 'Game & selected voices') }}</span><strong>{{ gib(spaceEstimate.gameBytes) }}</strong></div>
          <div><span>{{ text('预计下载', 'Estimated download') }}</span><strong>{{ gib(spaceEstimate.downloadBytes) }}</strong></div>
          <div><span>{{ text('预计需要可用空间', 'Estimated free space required') }}</span><strong>{{ gib(spaceEstimate.requiredBytes) }}</strong></div>
          <div><span>{{ text('磁盘可用', 'Disk available') }}</span><strong :class="{ 'space-warning': !spaceEstimate.sufficient }">{{ gib(spaceEstimate.availableBytes) }}</strong></div>
        </div>
        <p v-if="!spaceEstimate.sufficient" class="space-warning">{{ text('空间不足，请更换目录或释放磁盘空间。', 'Insufficient space. Choose another folder or free disk space.') }}</p>
        <p class="hint">{{ text('分块下载、校验并解压，不保留完整压缩包。空间估算包含临时缓存、渠道组件和安全余量；续装会复用已有数据，实际需求可能更少。', 'Chunks are downloaded, verified and extracted as they arrive. The estimate includes temporary cache, channel components and a safety margin; resuming can reuse existing data.') }}</p>
      </template>
    </div>
    <div v-if="switching" class="notice">{{ gamePreset === 'GIMI' ? text('换服会转换客户端文件与渠道 SDK，不迁移账号或存档。请先把原客户端更新到目标正式版本。', 'Conversion changes client files and channel SDK, not accounts or saves. Update the original client to the target release first.') : text('当前游戏首批支持安装更新，暂不支持已有客户端换服。', 'Installation and updates are available; conversion of an existing client is not supported yet.') }}</div>
    <div class="actions">
      <span class="action-caption">{{ starting ? text('正在检查执行条件…', 'Checking requirements…') : text('准备好后开始', 'Start when you’re ready') }}</span>
      <el-button v-if="switching && gamePreset === 'GIMI'" type="primary" :loading="starting" :disabled="active || starting || !languages.length || !directory || status?.recoveryPending" @click="start('switch')">{{ text('切换客户端', 'Convert client') }}</el-button>
      <el-button v-else type="primary" :loading="starting" :disabled="active || starting || !directory || !languages.length || switching || status?.recoveryPending" @click="start(status?.clientFound ? 'update' : 'install')">
        {{ status?.clientFound ? text('更新 / 修复', 'Update / repair') : text('安装游戏', 'Install game') }}
      </el-button>
      <el-button v-if="canPreload" :disabled="active || starting || !languages.length || status?.recoveryPending" @click="start('preload')">{{ text('预下载', 'Pre-download') }}</el-button>
      <el-button v-if="status?.recoveryPending || ownTask?.state === 'failed' || error" :disabled="active || starting || !directory" @click="start('recover')">{{ text('恢复原客户端', 'Restore client') }}</el-button>
    </div>
    <div v-if="ownTask" class="progress-card" role="status" aria-live="polite">
      <div class="progress-heading"><strong>{{ ownTask.message }}</strong><el-button v-if="active" :disabled="ownTask.phase === 'commit'" @click="pause">{{ text('暂停', 'Pause') }}</el-button></div>
      <el-progress v-if="active && ownTask.bytesTotal" :percentage="percent" :stroke-width="6" />
      <small v-if="ownTask.filesTotal">{{ ownTask.filesDone }} / {{ ownTask.filesTotal }} {{ text('文件', 'files') }} · {{ (ownTask.bytesDone / 1073741824).toFixed(2) }} / {{ (ownTask.bytesTotal / 1073741824).toFixed(2) }} GiB</small>
      <p v-if="ownTask.state === 'paused'" class="hint">{{ text('再次执行相同操作即可使用已校验缓存继续。', 'Repeat the same operation to resume from verified cache.') }}</p>
    </div>
    <p v-else-if="active" class="hint">{{ text('另一游戏的维护任务正在运行。', 'A maintenance task for another game is running.') }}</p>
    <details class="reference-notices"><summary>{{ text('参考项目与许可', 'References & licenses') }}</summary><pre>{{ referenceNotices }}</pre></details>
  </section>
</template>

<style scoped>
.maintenance { display: flex; flex-direction: column; gap: 16px; min-width: 0; color: rgba(255, 255, 255, .8); font: inherit; }
.section-heading, .channel-row, .folder-row, .progress-heading { display: flex; align-items: center; gap: 16px; }
.section-heading, .progress-heading { justify-content: space-between; }
.version-overview, .configuration-card { border: 1px solid rgba(255, 255, 255, .04); border-radius: 12px; background: var(--t-surface-subtle); overflow: hidden; }
.overview-heading { display: flex; justify-content: space-between; gap: 12px; padding: 18px 22px 0; font-size: 12px; font-weight: 500; }
.sync-status { display: inline-flex; align-items: center; gap: 6px; color: var(--el-text-color-secondary); font-size: 11px; font-weight: 400; }
.sync-status i { width: 5px; height: 5px; border-radius: 50%; background: var(--el-color-success); }
.sync-status.pending i { background: var(--el-color-primary); }
.sync-status.failed i { background: var(--el-color-warning); }
.configuration-card { padding: 16px 18px; display: flex; flex-direction: column; gap: 16px; }
.card-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; padding-bottom: 8px; border-bottom: 1px solid rgba(255, 255, 255, .04); }
.card-heading h4 { margin: 0; font-size: 13px; font-weight: 600; }
.card-heading > span { font-size: 11px; color: var(--el-text-color-secondary); }
p { margin: 0; color: var(--text-secondary, #a9adba); font-size: 13px; line-height: 1.6; }
.field-label { font-size: 13px; min-width: 76px; flex-shrink: 0; }
.pills { display: flex; flex-wrap: wrap; gap: 8px; }
.pill { border: 1px solid transparent; border-radius: 8px; background: var(--el-fill-color-light); color: var(--el-text-color-regular); padding: 9px 18px; cursor: pointer; font: inherit; font-size: 13px; transition: background var(--motion-duration-fast) var(--motion-ease-out), border-color var(--motion-duration-fast) var(--motion-ease-out); }
.pill:hover { background: var(--el-fill-color); }
.pill.selected { background: var(--el-color-primary-light-9); color: var(--el-color-primary); border-color: var(--el-color-primary-light-5); }
.pill:focus-visible { outline: 2px solid var(--el-color-primary); outline-offset: 2px; }
.pill:disabled { opacity: .5; cursor: default; }
.versions { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); padding: 24px 0; }
.versions > div { padding: 0 22px; display: flex; flex-direction: column; gap: 12px; }
.versions > div + div { border-left: 1px solid var(--el-border-color-lighter); }
.versions span { font-size: 12px; color: var(--text-secondary); }
.versions strong { font-size: 18px; font-weight: 600; line-height: 1.4; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.versions strong.skeleton { width: 88px; max-width: 100%; border-radius: 6px; background: var(--el-fill-color); color: transparent; }
.folder-row code { font-size: 12px; font-family: inherit; min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 11px 13px; background: var(--el-fill-color-light); border-radius: 8px; color: var(--el-text-color-regular); }
.hint { font-size: 12px; }
.reference-notices { font-size: 12px; color: var(--text-secondary); }.reference-notices summary { cursor: pointer; }.reference-notices pre { white-space: pre-wrap; overflow-wrap: anywhere; font-family: inherit; line-height: 1.6; }
.notice, .progress-card { padding: 16px; border: 1px solid var(--el-border-color); border-radius: 12px; font-size: 13px; line-height: 1.6; }
.notice { border-color: var(--el-color-warning); }
.space-card { padding: 16px; border: 1px solid var(--el-border-color); border-radius: 12px; }
.space-values { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; margin-bottom: 12px; }
.space-values > div { display: flex; flex-direction: column; gap: 6px; }
.space-values span { font-size: 12px; color: var(--text-secondary); }
.space-values strong { font-size: 17px; }
.space-warning { color: var(--el-color-warning); }
.actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding-top: 4px; }.actions .el-button { margin: 0; min-height: 38px; border-radius: 8px; }
.action-caption { margin-right: auto; font-size: 12px; color: var(--el-text-color-secondary); }
.maintenance :deep(.el-button) { font-family: inherit; font-size: 12px; border: 0; border-radius: 8px; background: rgba(255, 255, 255, .06); color: rgba(255, 255, 255, .7); }
.maintenance :deep(.el-button:hover) { background: rgba(255, 255, 255, .12); color: rgba(255, 255, 255, .9); }
.maintenance :deep(.el-button--primary) { background: rgba(0, 122, 204, .2); border: 1px solid rgba(0, 122, 204, .35); color: #61afef; }
.maintenance :deep(.el-button--primary:hover) { background: rgba(0, 122, 204, .35); }
.maintenance :deep(.el-button.is-disabled) { opacity: .45; }
.progress-heading { margin-bottom: 12px; }.progress-heading strong { overflow-wrap: anywhere; min-width: 0; }
@media (max-width: 760px) { .section-heading, .folder-row, .card-heading { flex-wrap: wrap; }.configuration-card { padding: 16px; }.channel-row { align-items: flex-start; }.versions > div { padding: 0 14px; }.versions strong { font-size: 19px; }.action-caption { flex-basis: 100%; } }
@media (max-width: 480px) { .versions { grid-template-columns: 1fr; gap: 18px; }.versions > div + div { border-left: 0; }.overview-heading { flex-wrap: wrap; }.channel-row { flex-direction: column; gap: 10px; } }
</style>
