<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { dirname } from '@tauri-apps/api/path';
import { open } from '@tauri-apps/plugin-dialog';
import { ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import type { GameMaintenanceStatus as Status, GameMaintenanceTask as Task } from './types';
import referenceNotices from './NOTICE.md?raw';

const props = defineProps<{ gameName: string; gamePreset: string; targetExePath: string }>();
const emit = defineEmits<{ (e: 'completed'): void }>();
const { locale } = useI18n();
const text = (zh: string, en: string) => locale.value.startsWith('zh') ? zh : en;
const status = ref<Status | null>(null);
const task = ref<Task | null>(null);
const loading = ref(false);
const starting = ref(false);
const error = ref('');
const directory = ref('');
const server = ref('cn');
const languages = ref(['zh-cn']);
const active = computed(() => task.value?.state === 'running');
const ownTask = computed(() => task.value?.gameName === props.gameName ? task.value : null);
const switching = computed(() => !!status.value?.currentServer && status.value.currentServer !== server.value);
const percent = computed(() => ownTask.value?.bytesTotal ? Math.min(100, Math.floor(ownTask.value.bytesDone / ownTask.value.bytesTotal * 100)) : 0);
const voiceNames = [['zh-cn', '中文', 'Chinese'], ['en-us', '英语', 'English'], ['ja-jp', '日语', 'Japanese'], ['ko-kr', '韩语', 'Korean']];
let revision = 0;
let disposed = false;
let polling = false;
let completedId = '';
let taskRevision = 0;
let timer: ReturnType<typeof setTimeout> | undefined;

async function refresh(selected?: string) {
  const id = ++revision;
  const gameName = props.gameName;
  loading.value = true; error.value = '';
  try {
    const result = await invoke<Status>('hoyoplay_status', { gameName, serverId: selected ?? null });
    if (disposed || id !== revision) return;
    status.value = result; server.value = result.selectedServer;
    if (result.directory) directory.value = result.directory;
    if (!selected) languages.value = result.audioLanguages;
  } catch (e) { if (!disposed && id === revision) error.value = String(e); }
  finally { if (!disposed && id === revision) loading.value = false; }
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
      if (result?.state === 'completed') void refresh(server.value);
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
    const info = operation === 'preload'
      ? text('仅下载到缓存，不替换当前客户端。', 'Download to cache without replacing the current client.')
      : operation === 'recover'
        ? text('恢复上次中断提交前的客户端文件。请先关闭游戏。', 'Restore client files from before the interrupted commit. Close the game first.')
        : text('请先关闭游戏。文件校验完成后才会提交；下载期间可暂停，下次继续。', 'Close the game first. Files are committed after verification; downloads can be paused and resumed.');
    const confirmChannel = !!status.value?.clientFound && !status.value?.currentServer;
    const channelWarning = confirmChannel ? text('\n当前客户端未声明渠道，请确认你选择的渠道与它一致。', '\nThe existing client does not declare its channel. Confirm that your selection matches it.') : '';
    await ElMessageBox.confirm(`${info}${channelWarning}\n\n${directory.value}\n${server.value} · ${status.value?.latestVersion ?? ''}`, labels[operation], {
      confirmButtonText: labels[operation], cancelButtonText: text('取消', 'Cancel'), type: 'info',
    });
    if (disposed || props.gameName !== gameName) return;
    starting.value = true;
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
  void refresh();
}, { immediate: true });
void poll();
onBeforeUnmount(() => { disposed = true; ++revision; if (timer) clearTimeout(timer); });
</script>

<template>
  <section class="maintenance">
    <div class="section-heading">
      <div><h3>{{ text('客户端与资源', 'Client & resources') }}</h3><p>{{ text('使用官方资源，安装、更新和预下载由你发起。', 'Official resources. You control installation, updates and pre-downloads.') }}</p></div>
      <el-button :loading="loading" :disabled="active || starting" @click="refresh(server)">{{ text('检查版本', 'Check versions') }}</el-button>
    </div>
    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon />
    <div class="channel-row">
      <span class="field-label">{{ text('客户端渠道', 'Client channel') }}</span>
      <div class="pills">
        <button v-for="entry in (status?.servers ?? [{ id: 'cn', name: '国服' }, { id: 'bilibili', name: 'B服' }, { id: 'global', name: '国际服' }])" :key="entry.id"
          class="pill" :class="{ selected: server === entry.id }" :disabled="active || starting || loading" @click="refresh(entry.id)">
          {{ text(entry.name, ({ cn: 'China', bilibili: 'Bilibili', global: 'Global' } as Record<string, string>)[entry.id] ?? entry.name) }}
        </button>
      </div>
    </div>
    <div class="versions">
      <div><span>{{ text('已安装', 'Installed') }}</span><strong>{{ status?.installedVersion ?? '—' }}</strong></div>
      <div><span>{{ text('正式版本', 'Current release') }}</span><strong>{{ status?.latestVersion ?? '—' }}</strong></div>
      <div><span>{{ text('预下载版本', 'Pre-download') }}</span><strong>{{ status?.preDownloadVersion ?? text('暂未开放', 'Unavailable') }}</strong></div>
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
    <div v-if="switching" class="notice">{{ gamePreset === 'GIMI' ? text('换服会转换客户端文件与渠道 SDK，不迁移账号或存档。请先把原客户端更新到目标正式版本。', 'Conversion changes client files and channel SDK, not accounts or saves. Update the original client to the target release first.') : text('当前游戏首批支持安装更新，暂不支持已有客户端换服。', 'Installation and updates are available; conversion of an existing client is not supported yet.') }}</div>
    <div class="actions">
      <el-button v-if="switching && gamePreset === 'GIMI'" type="primary" :disabled="active || starting || loading || !languages.length" @click="start('switch')">{{ text('切换客户端', 'Convert client') }}</el-button>
      <el-button v-else type="primary" :disabled="active || starting || loading || !status || !directory || !languages.length || switching || status.recoveryPending" @click="start(status?.clientFound ? 'update' : 'install')">
        {{ status?.clientFound ? text('更新 / 修复', 'Update / repair') : text('安装游戏', 'Install game') }}
      </el-button>
      <el-button :disabled="active || starting || loading || !status?.preDownloadVersion || !status?.clientFound || switching || !languages.length || status?.recoveryPending" @click="start('preload')">{{ text('预下载', 'Pre-download') }}</el-button>
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
.maintenance { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
.section-heading, .channel-row, .folder-row, .progress-heading { display: flex; align-items: center; gap: 16px; }
.section-heading, .progress-heading { justify-content: space-between; }
h3 { margin: 0 0 6px; font-size: 17px; }
p { margin: 0; color: var(--text-secondary, #a9adba); font-size: 13px; line-height: 1.6; }
.field-label { font-size: 13px; min-width: 76px; flex-shrink: 0; }
.pills { display: flex; flex-wrap: wrap; gap: 8px; }
.pill { border: 1px solid var(--el-border-color); border-radius: 20px; background: transparent; color: inherit; padding: 7px 17px; cursor: pointer; transition: background .15s; }
.pill.selected { background: var(--el-color-primary-light-9); color: var(--el-color-primary); border-color: var(--el-color-primary); }
.pill:disabled { opacity: .5; cursor: default; }
.versions { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); border: 1px solid var(--el-border-color); border-radius: 12px; }
.versions > div { padding: 16px; display: flex; flex-direction: column; gap: 9px; }
.versions span { font-size: 12px; color: var(--text-secondary); }
.versions strong { font-size: 18px; overflow-wrap: anywhere; }
.folder-row code { font-size: 12px; min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.hint { font-size: 12px; }
.reference-notices { font-size: 12px; color: var(--text-secondary); }.reference-notices summary { cursor: pointer; }.reference-notices pre { white-space: pre-wrap; overflow-wrap: anywhere; font-family: inherit; line-height: 1.6; }
.notice, .progress-card { padding: 16px; border: 1px solid var(--el-border-color); border-radius: 12px; font-size: 13px; line-height: 1.6; }
.notice { border-color: var(--el-color-warning); }
.actions { display: flex; gap: 10px; flex-wrap: wrap; }.actions .el-button { margin: 0; }
.progress-heading { margin-bottom: 12px; }.progress-heading strong { overflow-wrap: anywhere; min-width: 0; }
@media (max-width: 760px) { .section-heading, .folder-row { flex-wrap: wrap; }.versions { grid-template-columns: 1fr; } }
</style>
