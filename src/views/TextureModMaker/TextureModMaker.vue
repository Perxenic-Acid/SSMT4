<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { join } from '@tauri-apps/api/path';
import { exists, readDir, readFile } from '@tauri-apps/plugin-fs';
import { ElMessage, ElMessageBox } from 'element-plus';
import { useI18n } from 'vue-i18n';
import { PathHelper } from '../../helper/PathHelper';
import { AppStateManager } from '../../store/AppStateManager';
import { Close } from '@element-plus/icons-vue';

type TextureEntry = { hash: string; fileName: string; path: string; width: number; height: number; format: string };
type MediaInfo = { width: number; height: number; fps?: number; duration?: number };
type SourceKind = 'image' | 'video' | 'sequence';
type FitMode = 'cover' | 'contain' | 'tile' | 'stretch' | 'center';
type DecodedRgba = { width: number; height: number; pixels: Uint8Array };
type GenerationProgress = { running: boolean; phase: string; processed: number; total: number; message: string; stages: Record<string, { processed: number; total: number; elapsedMs?: number }>; logPath?: string };

const { t } = useI18n();
const STORAGE_KEY = 'ssmt4:texture-mod-maker:v1';
const frameAnalysisPath = ref('');
const textures = ref<TextureEntry[]>([]);
const hashInput = ref('');
const textureThumbnails = ref<Record<string, string>>({});
let thumbnailLoadToken = 0;
const selectedTexture = ref<TextureEntry>();
const scanning = ref(false);
const sourceKind = ref<SourceKind>('image');
const sourcePath = ref('');
const preparedSourcePath = ref('');
const sequenceDirectory = ref('');
const sequenceRegex = ref('^.*?\\d+\\.(dds|png|jpg|jpeg|webp|bmp)$');
const sequenceFiles = ref<string[]>([]);
const sequenceFrame = ref(0);
const mediaInfo = ref<MediaInfo>();
const fitMode = ref<FitMode>('contain');
const flipVertical = ref(false);
const flipHorizontal = ref(false);
const rotation = ref<0 | 90 | 180 | 270>(0);
const fps = ref<number>();
const videoTrimRange = ref<[number, number]>([0, 0]);
const frameStep = ref(1);
const sizePercent = ref(100);
const videoQuality = ref(90);
const cpuThreadLimit = Math.max(1, navigator.hardwareConcurrency || 4);
const cpuThreads = ref(Math.max(1, Math.ceil(cpuThreadLimit / 2)));
const gpuWorkers = ref(2);
const bc7Quality = ref<'quick' | 'standard'>('quick');
const texconvBatchSize = ref(16);
const loopMode = ref<'loop' | 'once'>('loop');
const outputDirectory = ref('');
const defaultOutputDirectory = ref('');
const modName = ref('TextureMod');
const ffmpegReady = ref(false);
const installingFfmpeg = ref(false);
const generating = ref(false);
const cancelling = ref(false);
const generationProgress = ref<GenerationProgress>({ running: false, phase: '', processed: 0, total: 0, message: '', stages: {} });
const generationStageValues = ref<Record<string, number>>({});
const generationStageDetails = ref<Record<string, { processed: number; total: number; elapsedMs?: number }>>({});
let generationProgressTimer: ReturnType<typeof setInterval> | undefined;
const generationStages = computed(() => {
  const phases = sourceKind.value === 'video'
    ? ['extracting', 'transforming', 'encoding', 'writing']
    : ['transforming', 'encoding', 'writing'];
  return phases.map(phase => ({
    phase,
    label: t(`textureModMaker.generationPhases.${phase}`),
    value: generationStageValues.value[phase] || 0,
    active: generationProgress.value.phase === phase,
    processed: generationStageDetails.value[phase]?.processed || 0,
    total: generationStageDetails.value[phase]?.total || 0,
    elapsedMs: generationStageDetails.value[phase]?.elapsedMs || 0,
  }));
});
const applyGenerationProgress = (progress: GenerationProgress) => {
  const previousPhase = generationProgress.value.phase;
  generationProgress.value = progress;
  const values = { ...generationStageValues.value };
  const details = { ...generationStageDetails.value };
  for (const [phase, stage] of Object.entries(progress.stages || {})) {
    details[phase] = stage;
    values[phase] = stage.total > 0 ? Math.min(100, stage.processed / stage.total * 100) : 0;
  }
  if (previousPhase === 'extracting' && progress.phase !== 'extracting') {
    values.extracting = 100;
    if (details.extracting) details.extracting = { ...details.extracting, processed: details.extracting.total };
  }
  if (progress.total > 0 && ['extracting','transforming','encoding','writing'].includes(progress.phase)) {
    values[progress.phase] = Math.max(values[progress.phase] || 0, Math.min(100, progress.processed / progress.total * 100));
    const previous = details[progress.phase];
    details[progress.phase] = { processed: Math.max(previous?.processed || 0, progress.processed), total: progress.total, elapsedMs: previous?.elapsedMs };
  }
  if (progress.phase === 'complete') for (const phase of ['extracting','transforming','encoding','writing']) values[phase] = 100;
  generationStageValues.value = values;
  generationStageDetails.value = details;
};
const generationProgressText = computed(() => {
  const progress = generationProgress.value;
  if (!progress.phase) return '';
  const phase = t(`textureModMaker.generationPhases.${progress.phase}`);
  return progress.total > 0 ? t('textureModMaker.progressFrames', { phase, processed: progress.processed, total: progress.total, remaining: Math.max(0, progress.total - progress.processed) }) : phase;
});
const previewVideo = ref<HTMLVideoElement>();
const videoPaused = ref(false);
const videoMuted = ref(true);
const videoTime = ref(0);
const videoDuration = ref(0);
const targetPreview = ref('');
const targetPreviewOpen = ref(false);
const targetPreviewScale = ref(1);
const targetPreviewOffset = ref({ x: 0, y: 0 });
const targetPreviewDragging = ref(false);
let targetPreviewDragStart = { x: 0, y: 0, offsetX: 0, offsetY: 0 };
const viewerCanvas = ref<HTMLCanvasElement>();
const viewerChannels = ref({ R: true, G: true, B: true, A: true });
const targetDecoded = ref<DecodedRgba>();
const sourceDetailOpen = ref(false);
const sourceDetailLoading = ref(false);
const sourceDetailScale = ref(1);
const sourceDetailOffset = ref({ x: 0, y: 0 });
const sourceDetailDragging = ref(false);
const sourceDetailCanvas = ref<HTMLCanvasElement>();
const sourceDetailChannels = ref({ R: true, G: true, B: true, A: true });
let sourceDetailPixels: DecodedRgba | undefined;
let channelRenderToken = 0;
let sourceDetailDragStart = { x: 0, y: 0, offsetX: 0, offsetY: 0 };
const sequencePreviewPath = ref('');
let sourcePreviewToken = 0;
const sourcePreviewUrl = computed(() => {
  const path = sourceKind.value === 'sequence' ? sequencePreviewPath.value : preparedSourcePath.value;
  return path ? convertFileSrc(path) : '';
});
const isVideo = computed(() => sourceKind.value === 'video');
const targetRatio = computed(() => selectedTexture.value?.width && selectedTexture.value.height
  ? `${selectedTexture.value.width} / ${selectedTexture.value.height}` : '16 / 9');
const targetRatioValue = computed(() => selectedTexture.value?.width && selectedTexture.value.height
  ? selectedTexture.value.width / selectedTexture.value.height : 16 / 9);
const outputSize = computed(() => {
  const target = selectedTexture.value; const source = mediaInfo.value;
  if (!target || !source?.width || !source.height) return undefined;
  const rotated = rotation.value % 180 === 90;
  const scale = Math.max(.1, sizePercent.value / 100);
  const sw = Math.max(1, Math.round((rotated ? source.height : source.width) * scale));
  const sh = Math.max(1, Math.round((rotated ? source.width : source.height) * scale));
  const targetRatio = target.width / target.height; const sourceRatio = sw / sh;
  if (fitMode.value === 'cover') return sourceRatio > targetRatio
    ? { width: Math.max(1, Math.round(sh * targetRatio)), height: sh }
    : { width: sw, height: Math.max(1, Math.round(sw / targetRatio)) };
  return sourceRatio > targetRatio
    ? { width: sw, height: Math.max(1, Math.ceil(sw / targetRatio)) }
    : { width: Math.max(1, Math.ceil(sh * targetRatio)), height: sh };
});
const previewBoxStyle = computed(() => ({
  aspectRatio: targetRatio.value,
  width: `min(100%, calc(420px * ${targetRatioValue.value}))`,
  backgroundImage: fitMode.value === 'tile' && sourcePreviewUrl.value ? `url('${sourcePreviewUrl.value}')` : undefined,
}));
const previewTransform = computed(() => {
  const transforms = [flipHorizontal.value ? 'scaleX(-1)' : '', flipVertical.value ? 'scaleY(-1)' : '', `rotate(${rotation.value}deg)`].filter(Boolean).join(' ');
  return { transform: transforms || undefined };
});
const previewObjectFit = computed(() => ({ cover: 'cover', contain: 'contain', stretch: 'fill', center: 'none', tile: 'none' }[fitMode.value]));
const canGenerate = computed(() => !!selectedTexture.value && !!modName.value.trim() && !!(outputDirectory.value.trim() || defaultOutputDirectory.value)
  && (sourceKind.value === 'sequence' ? (!!sequenceDirectory.value || sequenceFiles.value.length > 0) : !!sourcePath.value)
  && (sourceKind.value !== 'video' || !!fps.value));
const filteredTextures = computed(() => {
  const query = hashInput.value.trim().toLowerCase();
  return query ? textures.value.filter(item => item.hash.includes(query) || item.fileName.toLowerCase().includes(query)) : textures.value;
});
const effectiveOutputDirectory = computed(() => outputDirectory.value.trim() || defaultOutputDirectory.value);
const finalOutputPath = computed(() => effectiveOutputDirectory.value && modName.value.trim()
  ? `${effectiveOutputDirectory.value.replace(/[\\/]+$/, '')}\\${modName.value.trim()}` : '');
const refreshDefaultOutputDirectory = async () => {
  try {
    const migotoPath = await PathHelper.GetCurrentGame3DmigotoFolderPath();
    if (!migotoPath) { defaultOutputDirectory.value = ''; return; }
    const category = sourceKind.value === 'video' ? 'DynamicTextureMod'
      : sourceKind.value === 'sequence' ? 'TextureSequenceMod' : 'StaticTextureMod';
    defaultOutputDirectory.value = await join(migotoPath, 'Mods', 'SSMTGeneratedMod', 'TextureMod', category);
  } catch { defaultOutputDirectory.value = ''; }
};
const expectedFrameCount = computed(() => {
  if (sourceKind.value === 'image') return sourcePath.value ? 1 : 0;
  if (sourceKind.value === 'sequence') return Math.ceil(sequenceFiles.value.length / Math.max(1, frameStep.value));
  if (!mediaInfo.value?.duration || !fps.value) return 0;
  return Math.max(1, Math.ceil((videoTrimRange.value[1] - videoTrimRange.value[0] + 1) / Math.max(1, frameStep.value)));
});
const videoFrameMax = computed(() => Math.max(0, Math.ceil((mediaInfo.value?.duration || 0) * (fps.value || 0)) - 1));
let previousVideoFrameMax = 0;
watch(videoFrameMax, value => {
  const [start, end] = videoTrimRange.value;
  const nextEnd = end === previousVideoFrameMax ? value : Math.min(end, value);
  videoTrimRange.value = [Math.min(start, nextEnd), nextEnd];
  previousVideoFrameMax = value;
});
const setTrimStart = (value: number | undefined) => {
  videoTrimRange.value = [Math.min(Math.max(0, value || 0), videoTrimRange.value[1]), videoTrimRange.value[1]];
};
const setTrimEnd = (value: number | undefined) => {
  videoTrimRange.value = [videoTrimRange.value[0], Math.max(videoTrimRange.value[0], Math.min(videoFrameMax.value, value ?? videoFrameMax.value))];
};
const formatDuration = (seconds: number) => {
  if (seconds < 60) return t('textureModMaker.seconds', { count: Math.max(1, Math.round(seconds)) });
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return t('textureModMaker.minutes', { count: minutes });
  const hours = Math.floor(minutes / 60); const rest = minutes % 60;
  return t('textureModMaker.hoursMinutes', { hours, minutes: rest });
};
const generationEstimate = computed(() => {
  const target = selectedTexture.value; const frames = expectedFrameCount.value;
  if (!target || !frames) return '';
  const megapixels = outputSize.value ? outputSize.value.width * outputSize.value.height / 1_000_000 : target.width * target.height / 1_000_000;
  const format = target.format.toUpperCase();
  const perFrame = format.includes('BC7') ? (bc7Quality.value === 'quick' ? .06 + megapixels * .055 : .16 + megapixels * .09)
    : format.includes('BC6') ? .18 + megapixels * .16
      : format.includes('BC') ? .045 + megapixels * .04
        : .025 + megapixels * .025;
  const center = 2 + frames * perFrame;
  return t('textureModMaker.estimate', {
    frames,
    lower: formatDuration(Math.max(1, center * .65)),
    upper: formatDuration(Math.max(2, center * 1.8)),
  });
});

const persist = () => localStorage.setItem(STORAGE_KEY, JSON.stringify({ frameAnalysisPath: frameAnalysisPath.value, sourceKind: sourceKind.value,
  sequenceRegex: sequenceRegex.value, fitMode: fitMode.value, flipVertical: flipVertical.value, flipHorizontal: flipHorizontal.value,
  rotation: rotation.value, frameStep: frameStep.value, sizePercent: sizePercent.value, videoQuality: videoQuality.value, cpuThreads: cpuThreads.value, gpuWorkers: gpuWorkers.value,
  bc7Quality: bc7Quality.value, texconvBatchSize: texconvBatchSize.value,
  loopMode: loopMode.value, outputDirectory: outputDirectory.value, modName: modName.value }));
watch([frameAnalysisPath, sourceKind, sequenceRegex, fitMode, flipVertical, flipHorizontal, rotation, frameStep, sizePercent, videoQuality, cpuThreads, gpuWorkers, bc7Quality, texconvBatchSize, loopMode, outputDirectory, modName], persist);

const pickFrameAnalysis = async () => {
  const value = await open({ directory: true, multiple: false, title: t('textureModMaker.pickFrameAnalysis') });
  if (typeof value === 'string') { frameAnalysisPath.value = value; await scanTextures(); }
};
const useLatestFrameAnalysis = async () => {
  try {
    const migotoPath = await PathHelper.GetCurrentGame3DmigotoFolderPath();
    if (!migotoPath) return;
    const folders = (await readDir(migotoPath)).filter(entry => entry.isDirectory && entry.name?.startsWith('FrameAnalysis'))
      .map(entry => entry.name as string).sort((a, b) => b.localeCompare(a));
    if (!folders.length) { ElMessage.warning(t('textureModMaker.noFrameAnalysis')); return; }
    frameAnalysisPath.value = await join(migotoPath, folders[0]!);
    await scanTextures();
  } catch (error) { ElMessage.error(String(error)); }
};
const scanTextures = async () => {
  if (!frameAnalysisPath.value.trim()) return;
  scanning.value = true;
  try {
    textures.value = await invoke<TextureEntry[]>('scan_frame_analysis_textures', { folder: frameAnalysisPath.value });
    if (!textures.value.length) ElMessage.warning(t('textureModMaker.noTextures'));
    else ElMessage.success(t('textureModMaker.textureCount', { count: textures.value.length }));
    if (selectedTexture.value && !textures.value.some(v => v.hash === selectedTexture.value?.hash)) selectedTexture.value = undefined;
    void loadTextureThumbnails();
  } catch (error) { ElMessage.error(String(error)); }
  finally { scanning.value = false; }
};
const loadTextureThumbnails = async () => {
  const token = ++thumbnailLoadToken; textureThumbnails.value = {};
  let cursor = 0;
  const workers = Array.from({ length: Math.min(4, textures.value.length) }, async () => {
    while (cursor < textures.value.length && token === thumbnailLoadToken) {
      const item = textures.value[cursor++]; if (!item) continue;
      try {
        const path = await invoke<string>('prepare_texture_mod_preview', { sourcePath: item.path });
        if (token !== thumbnailLoadToken) return;
        textureThumbnails.value = { ...textureThumbnails.value, [item.hash]: convertFileSrc(path) };
      } catch { /* Keep the card selectable even when its thumbnail fails. */ }
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    }
  });
  await Promise.all(workers);
};
const selectTexture = async (item: TextureEntry) => {
  selectedTexture.value = item; hashInput.value = item.hash; targetDecoded.value = undefined;
  targetPreview.value = textureThumbnails.value[item.hash] || '';
  if (!targetPreview.value) {
    try { targetPreview.value = convertFileSrc(await invoke<string>('prepare_texture_mod_preview', { sourcePath: item.path })); } catch { /* Detailed DDS decoding remains available. */ }
  }
};
const showTargetPreview = async () => {
  if (!selectedTexture.value) return;
  targetPreviewScale.value = 1; targetPreviewOffset.value = { x: 0, y: 0 }; viewerChannels.value = { R: true, G: true, B: true, A: true };
  targetPreviewOpen.value = true; await nextTick();
  try {
    const prepared = await invoke<string>('prepare_dds_webgl_preview', { sourcePath: selectedTexture.value.path });
    targetDecoded.value = decodeRgbaDds(await readFile(prepared));
    await renderTargetChannels();
  } catch (error) { ElMessage.error(String(error)); }
};
const pickSource = async () => {
  const filters = sourceKind.value === 'image'
    ? [{ name: 'Images', extensions: ['dds','png','jpg','jpeg','webp','bmp','gif'] }]
    : [{ name: 'Videos', extensions: ['mp4','mkv','mov','avi','webm','wmv','gif'] }];
  const value = await open({ multiple: false, directory: false, filters });
  if (typeof value !== 'string') return;
  sourcePath.value = value;
  preparedSourcePath.value = sourceKind.value === 'image'
    ? await invoke<string>('prepare_texture_mod_preview', { sourcePath: value })
    : value;
  try {
    mediaInfo.value = await invoke<MediaInfo>('texture_mod_media_info', { path: value });
    if (sourceKind.value === 'video' && mediaInfo.value.fps) fps.value = Number(mediaInfo.value.fps.toFixed(3));
    if (sourceKind.value === 'video') {
      previousVideoFrameMax = videoFrameMax.value;
      videoTrimRange.value = [0, videoFrameMax.value];
    }
  } catch (error) { mediaInfo.value = undefined; ElMessage.warning(String(error)); }
};
const pickSequenceDirectory = async () => {
  const value = await open({ directory: true, multiple: false });
  if (typeof value === 'string') { sequenceDirectory.value = value; await loadSequence(); }
};
const loadSequence = async () => {
  if (!sequenceDirectory.value) return;
  try {
    sequenceFiles.value = await invoke<string[]>('list_texture_mod_sequence', { folder: sequenceDirectory.value, pattern: sequenceRegex.value });
    sequenceFrame.value = 0;
    if (sequenceFiles.value[0]) mediaInfo.value = await invoke<MediaInfo>('texture_mod_media_info', { path: sequenceFiles.value[0] });
    if (!sequenceFiles.value.length) ElMessage.warning(t('textureModMaker.noFrames'));
  } catch (error) { sequenceFiles.value = []; ElMessage.error(String(error)); }
};
const pickSequenceFiles = async () => {
  const value = await open({ multiple: true, directory: false, filters: [{ name: 'Images', extensions: ['dds','png','jpg','jpeg','webp','bmp'] }] });
  if (Array.isArray(value)) { sequenceFiles.value = value; sequenceDirectory.value = ''; sequenceFrame.value = 0; if (value[0]) mediaInfo.value = await invoke<MediaInfo>('texture_mod_media_info', { path: value[0] }); }
};
const pickOutput = async () => {
  const value = await open({ directory: true, multiple: false });
  if (typeof value === 'string') outputDirectory.value = value;
};
const installFfmpeg = async () => {
  installingFfmpeg.value = true;
  try { await invoke('install_texture_mod_ffmpeg'); ffmpegReady.value = true; ElMessage.success(t('textureModMaker.ffmpegInstalled')); }
  catch (error) { ElMessage.error(String(error)); }
  finally { installingFfmpeg.value = false; }
};
const generate = async () => {
  if (!canGenerate.value || !selectedTexture.value) return;
  let overwrite = false;
  if (finalOutputPath.value && await exists(finalOutputPath.value)) {
    try {
      await ElMessageBox.confirm(t('textureModMaker.overwriteConfirmMessage', { path: finalOutputPath.value }), t('textureModMaker.overwriteConfirmTitle'), {
        type: 'warning', confirmButtonText: t('textureModMaker.confirmOverwrite'), cancelButtonText: t('textureModMaker.cancelOverwrite'),
      });
      overwrite = true;
    } catch { return; }
  }
  generating.value = true;
  generationStageValues.value = {};
  generationStageDetails.value = {};
  const refreshProgress = async () => {
    try { applyGenerationProgress(await invoke<GenerationProgress>('texture_mod_generation_progress')); } catch { /* Generation result reports errors. */ }
  };
  applyGenerationProgress({ running: true, phase: 'preparing', processed: 0, total: 0, message: '', stages: {} });
  generationProgressTimer = setInterval(() => { void refreshProgress(); }, 150);
  try {
    const path = await invoke<string>('generate_texture_mod', { request: {
      targetPath: selectedTexture.value.path, textureHash: selectedTexture.value.hash, sourceKind: sourceKind.value,
      sourcePath: sourcePath.value || null, sequenceFiles: sequenceFiles.value, sequenceDirectory: sequenceDirectory.value || null,
      sequenceRegex: sequenceRegex.value || null, outputDirectory: effectiveOutputDirectory.value, modName: modName.value,
      fitMode: fitMode.value, flipVertical: flipVertical.value, flipHorizontal: flipHorizontal.value, rotation: rotation.value,
      fps: fps.value || null, frameStep: frameStep.value, sizePercent: sizePercent.value, videoQuality: videoQuality.value, loopMode: loopMode.value,
      trimStartFrame: sourceKind.value === 'video' ? videoTrimRange.value[0] : null,
      trimEndFrame: sourceKind.value === 'video' ? videoTrimRange.value[1] : null,
      cpuThreads: cpuThreads.value,
      gpuWorkers: gpuWorkers.value,
      bc7Quality: bc7Quality.value,
      texconvBatchSize: texconvBatchSize.value,
      logLanguage: AppStateManager.appSettings.modelExtractionLogLanguage,
      overwrite,
    }});
    ElMessage.success(t('textureModMaker.generated'));
    await openPath(path);
  } catch (error) {
    const reason = String(error);
    console.error('Texture mod generation failed:', error);
    ElMessage.error({ message: reason.length > 280 ? `${reason.slice(0, 280)}…` : reason, duration: 8000, showClose: true });
  }
  finally { generating.value = false; if (generationProgressTimer) clearInterval(generationProgressTimer); generationProgressTimer = undefined; await refreshProgress(); }
};
const cancelGeneration = async () => {
  if (!generating.value || cancelling.value) return;
  cancelling.value = true;
  try {
    await ElMessageBox.confirm(t('textureModMaker.cancelConfirmMessage'), t('textureModMaker.cancelConfirmTitle'), {
      type: 'warning', confirmButtonText: t('textureModMaker.confirmCancel'), cancelButtonText: t('textureModMaker.keepGenerating'),
    });
    await invoke('cancel_texture_mod_generation');
  } catch { /* User kept the generation running. */ }
  finally { cancelling.value = false; }
};
const openGenerationLog = async () => {
  if (!generationProgress.value.logPath) return;
  try { await openPath(generationProgress.value.logPath); }
  catch (error) { ElMessage.error(String(error)); }
};
const syncVideoState = () => {
  const video = previewVideo.value; if (!video) return;
  if (sourceKind.value === 'video' && fps.value) {
    const startTime = videoTrimRange.value[0] / fps.value;
    const endTime = (videoTrimRange.value[1] + 1) / fps.value;
    if (video.currentTime < startTime || video.currentTime >= endTime) video.currentTime = startTime;
  }
  videoPaused.value = video.paused; videoMuted.value = video.muted;
  videoTime.value = Number.isFinite(video.currentTime) ? video.currentTime : 0;
  videoDuration.value = Number.isFinite(video.duration) ? video.duration : 0;
};
const togglePreviewVideo = async () => {
  const video = previewVideo.value; if (!video) return;
  if (video.paused) await video.play(); else video.pause();
  syncVideoState();
};
const togglePreviewMute = () => {
  const video = previewVideo.value; if (!video) return;
  video.muted = !video.muted; syncVideoState();
};
const seekPreviewVideo = (event: Event) => {
  const video = previewVideo.value; if (!video) return;
  video.currentTime = Number((event.target as HTMLInputElement).value); syncVideoState();
};
const displayVideoTime = (seconds: number) => {
  if (!Number.isFinite(seconds)) return '0:00';
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
};

const decodeRgbaDds = (bytes: Uint8Array): DecodedRgba => {
  if (bytes.byteLength < 148 || String.fromCharCode(...bytes.subarray(0, 4)) !== 'DDS ') throw new Error('Invalid DDS preview cache');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength); const height = view.getUint32(12, true); const width = view.getUint32(16, true);
  const hasDx10 = view.getUint32(84, true) === 0x30315844; const offset = hasDx10 ? 148 : 128;
  if (hasDx10 && ![28, 29].includes(view.getUint32(128, true))) throw new Error('Unexpected DDS preview format');
  const length = width * height * 4; if (!width || !height || offset + length > bytes.byteLength) throw new Error('DDS preview is incomplete');
  return { width, height, pixels: bytes.slice(offset, offset + length) };
};
const channelMask = (channels: { R: boolean; G: boolean; B: boolean; A: boolean }) => (channels.R ? 1 : 0) | (channels.G ? 2 : 0) | (channels.B ? 4 : 0) | (channels.A ? 8 : 0);
const renderRgbaAsync = async (canvas: HTMLCanvasElement | undefined, decoded: DecodedRgba | undefined, mask: number) => {
  if (!canvas || !decoded) return; const token = ++channelRenderToken;
  const output = new Uint8ClampedArray(decoded.pixels); const alphaOnly = (mask & 15) === 8;
  for (let start = 0; start < output.length; start += 1_048_576) {
    const end = Math.min(output.length, start + 1_048_576);
    for (let index = start - (start % 4); index < end; index += 4) {
      if (alphaOnly) { const alpha = output[index + 3]!; output[index] = alpha; output[index + 1] = alpha; output[index + 2] = alpha; output[index + 3] = 255; }
      else { if (!(mask & 1)) output[index] = 0; if (!(mask & 2)) output[index + 1] = 0; if (!(mask & 4)) output[index + 2] = 0; if (!(mask & 8)) output[index + 3] = 255; }
    }
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); if (token !== channelRenderToken) return;
  }
  canvas.width = decoded.width; canvas.height = decoded.height;
  canvas.getContext('2d')?.putImageData(new ImageData(output, decoded.width, decoded.height), 0, 0);
};
const renderTargetChannels = () => renderRgbaAsync(viewerCanvas.value, targetDecoded.value, channelMask(viewerChannels.value));
const toggleViewerChannel = (channel: 'R' | 'G' | 'B' | 'A') => {
  viewerChannels.value = { ...viewerChannels.value, [channel]: !viewerChannels.value[channel] }; void renderTargetChannels();
};
const targetPreviewTransform = computed(() => ({ transform: `translate(${targetPreviewOffset.value.x}px, ${targetPreviewOffset.value.y}px) scale(${targetPreviewScale.value})` }));
const startTargetPreviewDrag = (event: PointerEvent) => {
  targetPreviewDragging.value = true;
  targetPreviewDragStart = { x: event.clientX, y: event.clientY, offsetX: targetPreviewOffset.value.x, offsetY: targetPreviewOffset.value.y };
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
};
const moveTargetPreviewDrag = (event: PointerEvent) => {
  if (!targetPreviewDragging.value) return;
  targetPreviewOffset.value = { x: targetPreviewDragStart.offsetX + event.clientX - targetPreviewDragStart.x, y: targetPreviewDragStart.offsetY + event.clientY - targetPreviewDragStart.y };
};
const stopTargetPreviewDrag = (event: PointerEvent) => {
  targetPreviewDragging.value = false;
  const element = event.currentTarget as HTMLElement;
  if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId);
};
const resetTargetPreview = () => { targetPreviewScale.value = 1; targetPreviewOffset.value = { x: 0, y: 0 }; };
const resetSourceDetail = () => { sourceDetailScale.value = 1; sourceDetailOffset.value = { x: 0, y: 0 }; };
const openSourceDetail = async () => {
  if (!sourcePreviewUrl.value || isVideo.value) return;
  sourceDetailLoading.value = true;
  try {
    const image = new Image(); image.src = sourcePreviewUrl.value; await image.decode();
    resetSourceDetail(); sourceDetailChannels.value = { R: true, G: true, B: true, A: true }; sourceDetailOpen.value = true; await nextTick();
    const canvas = sourceDetailCanvas.value; if (!canvas) return;
    canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    const context = canvas.getContext('2d', { willReadFrequently: true }); if (!context) return;
    context.drawImage(image, 0, 0); const data = context.getImageData(0, 0, canvas.width, canvas.height);
    sourceDetailPixels = { width: canvas.width, height: canvas.height, pixels: new Uint8Array(data.data) };
    await renderRgbaAsync(canvas, sourceDetailPixels, channelMask(sourceDetailChannels.value));
  } catch { ElMessage.error(t('textureModMaker.previewLoadFailed')); }
  finally { sourceDetailLoading.value = false; }
};
const toggleSourceDetailChannel = (channel: 'R' | 'G' | 'B' | 'A') => {
  sourceDetailChannels.value = { ...sourceDetailChannels.value, [channel]: !sourceDetailChannels.value[channel] };
  void renderRgbaAsync(sourceDetailCanvas.value, sourceDetailPixels, channelMask(sourceDetailChannels.value));
};
const zoomSourceDetail = (event: WheelEvent) => {
  sourceDetailScale.value = Math.min(16, Math.max(0.1, sourceDetailScale.value * (event.deltaY < 0 ? 1.12 : 0.89)));
};
const startSourceDetailDrag = (event: PointerEvent) => {
  sourceDetailDragging.value = true;
  sourceDetailDragStart = { x: event.clientX, y: event.clientY, offsetX: sourceDetailOffset.value.x, offsetY: sourceDetailOffset.value.y };
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
};
const moveSourceDetailDrag = (event: PointerEvent) => {
  if (!sourceDetailDragging.value) return;
  sourceDetailOffset.value = { x: sourceDetailDragStart.offsetX + event.clientX - sourceDetailDragStart.x, y: sourceDetailDragStart.offsetY + event.clientY - sourceDetailDragStart.y };
};
const stopSourceDetailDrag = (event: PointerEvent) => {
  sourceDetailDragging.value = false;
  const element = event.currentTarget as HTMLElement;
  if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId);
};
const sourceDetailTransform = computed(() => ({ transform: `translate(${sourceDetailOffset.value.x}px, ${sourceDetailOffset.value.y}px) scale(${sourceDetailScale.value})` }));
const updateSequencePreview = async () => {
  const token = ++sourcePreviewToken;
  const path = sequenceFiles.value[sequenceFrame.value];
  if (!path) { sequencePreviewPath.value = ''; return; }
  try {
    const prepared = await invoke<string>('prepare_texture_mod_preview', { sourcePath: path });
    if (token === sourcePreviewToken) sequencePreviewPath.value = prepared;
  } catch { if (token === sourcePreviewToken) sequencePreviewPath.value = ''; }
};
watch([sequenceFiles, sequenceFrame], () => { void updateSequencePreview(); }, { deep: true });
watch(videoTrimRange, range => {
  if (previewVideo.value && fps.value) previewVideo.value.currentTime = range[0] / fps.value;
}, { deep: true });
watch(sourceKind, () => { sourcePath.value = ''; preparedSourcePath.value = ''; mediaInfo.value = undefined; fps.value = undefined; void refreshDefaultOutputDirectory(); });

onMounted(async () => {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');
    if (saved.frameAnalysisPath) frameAnalysisPath.value = saved.frameAnalysisPath;
    if (['image','video','sequence'].includes(saved.sourceKind)) sourceKind.value = saved.sourceKind;
    if (saved.sequenceRegex) sequenceRegex.value = saved.sequenceRegex;
    if (['cover','contain','tile','stretch','center'].includes(saved.fitMode)) fitMode.value = saved.fitMode;
    flipVertical.value = !!saved.flipVertical; flipHorizontal.value = !!saved.flipHorizontal;
    if ([0,90,180,270].includes(saved.rotation)) rotation.value = saved.rotation;
    if (saved.frameStep) frameStep.value = saved.frameStep; if (saved.sizePercent) sizePercent.value = saved.sizePercent;
    if (Number.isFinite(saved.videoQuality)) videoQuality.value = Math.min(100, Math.max(1, saved.videoQuality));
    if (Number.isFinite(saved.cpuThreads)) cpuThreads.value = Math.min(cpuThreadLimit, Math.max(1, Math.round(saved.cpuThreads)));
    if (Number.isFinite(saved.gpuWorkers)) gpuWorkers.value = Math.min(2, Math.max(1, Math.round(saved.gpuWorkers)));
    if (['quick','standard'].includes(saved.bc7Quality)) bc7Quality.value = saved.bc7Quality;
    if (Number.isFinite(saved.texconvBatchSize)) texconvBatchSize.value = Math.min(64, Math.max(1, Math.round(saved.texconvBatchSize)));
    if (['loop','once'].includes(saved.loopMode)) loopMode.value = saved.loopMode;
    if (saved.outputDirectory) outputDirectory.value = saved.outputDirectory; if (saved.modName) modName.value = saved.modName;
  } catch { /* Ignore malformed local preferences. */ }
  ffmpegReady.value = await invoke<boolean>('texture_mod_ffmpeg_status');
  await refreshDefaultOutputDirectory();
  if (frameAnalysisPath.value) await scanTextures();
  await nextTick();
});
onUnmounted(() => { if (generationProgressTimer) clearInterval(generationProgressTimer); });
</script>

<template>
  <div class="tm-page">
    <section class="tm-card">
      <div class="tm-section-head"><b>1</b><div><h2>{{ t('textureModMaker.targetTitle') }}</h2><p>{{ t('textureModMaker.targetDesc') }}</p></div><div class="tm-tool-status" :class="{ ready: ffmpegReady }"><span></span><div><strong>FFmpeg</strong><small>{{ ffmpegReady ? t('textureModMaker.ready') : t('textureModMaker.missing') }}</small></div><el-button v-if="!ffmpegReady" size="small" type="primary" :loading="installingFfmpeg" @click="installFfmpeg">{{ t('textureModMaker.install') }}</el-button></div></div>
      <div class="tm-path-row"><el-input v-model="frameAnalysisPath" :placeholder="t('textureModMaker.frameAnalysisPlaceholder')" @keyup.enter="scanTextures"/><el-button @click="pickFrameAnalysis">{{ t('textureModMaker.choose') }}</el-button><el-button @click="useLatestFrameAnalysis">{{ t('textureModMaker.useLatest') }}</el-button><el-button type="primary" :loading="scanning" @click="scanTextures">{{ t('textureModMaker.scan') }}</el-button></div>
      <div class="tm-target-row">
        <el-input v-model="hashInput" :placeholder="t('textureModMaker.textureFilterPlaceholder')" clearable/>
        <div v-if="selectedTexture" class="tm-target-chip"><code>{{ selectedTexture.hash }}</code><span>{{ selectedTexture.width }}×{{ selectedTexture.height }}</span><span>{{ selectedTexture.format }}</span><span>{{ selectedTexture.fileName }}</span><el-button text @click="showTargetPreview">{{ t('textureModMaker.view') }}</el-button></div>
      </div>
      <div v-if="textures.length" class="tm-texture-grid">
        <button v-for="item in filteredTextures" :key="item.hash" type="button" class="tm-texture-card" :class="{ selected: selectedTexture?.hash === item.hash }" @click="selectTexture(item)" @dblclick="selectTexture(item).then(showTargetPreview)">
          <div class="tm-texture-thumb"><img v-if="textureThumbnails[item.hash]" :src="textureThumbnails[item.hash]" alt="" decoding="async"/><span v-else class="tm-thumb-loading"></span></div>
          <div class="tm-texture-info"><code>{{ item.hash }}</code><small>{{ item.width }}×{{ item.height }} · {{ item.format }}</small></div>
        </button>
      </div>
      <div v-if="textures.length && !filteredTextures.length" class="tm-empty-filter">{{ t('textureModMaker.noMatchingTextures') }}</div>
    </section>

    <div class="tm-columns">
      <section class="tm-card">
        <div class="tm-section-head"><b>2</b><div><h2>{{ t('textureModMaker.sourceTitle') }}</h2><p>{{ t('textureModMaker.sourceDesc') }}</p></div></div>
        <el-radio-group v-model="sourceKind" class="tm-multi-switch tm-source-switch"><el-radio-button value="image">{{ t('textureModMaker.image') }}</el-radio-button><el-radio-button value="video">{{ t('textureModMaker.video') }}</el-radio-button><el-radio-button value="sequence">{{ t('textureModMaker.sequence') }}</el-radio-button></el-radio-group>
        <template v-if="sourceKind !== 'sequence'">
          <div class="tm-path-row"><el-input v-model="sourcePath" readonly :placeholder="t('textureModMaker.sourcePlaceholder')"/><el-button @click="pickSource">{{ t('textureModMaker.choose') }}</el-button></div>
          <small v-if="mediaInfo" class="tm-meta">{{ mediaInfo.width }}×{{ mediaInfo.height }}<template v-if="mediaInfo.fps"> · {{ mediaInfo.fps.toFixed(2) }} FPS</template><template v-if="mediaInfo.duration"> · {{ mediaInfo.duration.toFixed(2) }}s</template></small>
        </template>
        <template v-else>
          <div class="tm-path-row"><el-input v-model="sequenceDirectory" readonly :placeholder="t('textureModMaker.sequenceFolder')"/><el-button @click="pickSequenceDirectory">{{ t('textureModMaker.chooseFolder') }}</el-button><el-button @click="pickSequenceFiles">{{ t('textureModMaker.chooseFrames') }}</el-button></div>
          <div v-if="sequenceDirectory" class="tm-path-row"><el-input v-model="sequenceRegex" @keyup.enter="loadSequence"><template #prepend>{{ t('textureModMaker.frameRegex') }}</template></el-input><el-button @click="loadSequence">{{ t('textureModMaker.applyRegex') }}</el-button></div>
          <div v-if="sequenceFiles.length" class="tm-sequence-nav"><span>{{ sequenceFrame + 1 }}/{{ sequenceFiles.length }}</span><el-slider v-model="sequenceFrame" :min="0" :max="sequenceFiles.length - 1" :show-tooltip="false"/></div>
        </template>
      </section>

      <section class="tm-card tm-preview-card">
        <div class="tm-section-head"><b>3</b><div><h2>{{ t('textureModMaker.previewTitle') }}</h2><p>{{ t('textureModMaker.previewDesc') }}</p></div></div>
        <div class="tm-preview" :class="`fit-${fitMode}`" :style="previewBoxStyle">
          <video v-if="isVideo && sourcePreviewUrl" ref="previewVideo" :src="sourcePreviewUrl" autoplay muted loop :style="previewTransform" :class="`object-${previewObjectFit}`" @loadedmetadata="syncVideoState" @timeupdate="syncVideoState" @play="syncVideoState" @pause="syncVideoState"/>
          <div v-if="isVideo && sourcePreviewUrl" class="tm-video-controls" @click.stop>
            <button type="button" @click="togglePreviewVideo">{{ videoPaused ? '▶' : '❚❚' }}</button>
            <span>{{ displayVideoTime(videoTime) }}</span>
            <input type="range" min="0" :max="videoDuration || 0" step="0.01" :value="videoTime" @input="seekPreviewVideo"/>
            <span>{{ displayVideoTime(videoDuration) }}</span>
            <button type="button" @click="togglePreviewMute">{{ videoMuted ? '🔇' : '🔊' }}</button>
          </div>
          <img v-else-if="sourcePreviewUrl && fitMode !== 'tile'" :src="sourcePreviewUrl" alt="" decoding="async" :style="previewTransform" :class="`object-${previewObjectFit}`"/>
          <span v-else-if="!sourcePreviewUrl">{{ t('textureModMaker.previewEmpty') }}</span>
          <el-button v-if="sourcePreviewUrl && !isVideo" class="tm-preview-detail-button" size="small" :loading="sourceDetailLoading" @click.stop="openSourceDetail">{{ t('textureModMaker.viewSource') }}</el-button>
          <em v-if="outputSize">{{ outputSize.width }}×{{ outputSize.height }} · {{ sizePercent }}%</em>
        </div>
      </section>
    </div>

    <section class="tm-card">
      <div class="tm-section-head"><b>4</b><div><h2>{{ t('textureModMaker.optionsTitle') }}</h2><p>{{ t('textureModMaker.optionsDesc') }}</p></div></div>
      <div class="tm-options">
        <label class="tm-option-wide"><span>{{ t('textureModMaker.fit') }}</span><el-radio-group v-model="fitMode" class="tm-multi-switch"><el-radio-button v-for="value in (['cover','contain','tile','stretch','center'] as FitMode[])" :key="value" :value="value">{{ t(`textureModMaker.fitModes.${value}`) }}</el-radio-button></el-radio-group></label>
        <label class="tm-option-wide"><span>{{ t('textureModMaker.rotation') }}</span><el-radio-group v-model="rotation" class="tm-multi-switch"><el-radio-button v-for="value in [0,90,180,270]" :key="value" :value="value">{{ value }}°</el-radio-button></el-radio-group></label>
        <label class="tm-switch"><span>{{ t('textureModMaker.flipHorizontal') }}</span><el-switch v-model="flipHorizontal"/></label>
        <label class="tm-switch"><span>{{ t('textureModMaker.flipVertical') }}</span><el-switch v-model="flipVertical"/></label>
        <label v-if="sourceKind !== 'image'"><span>{{ t('textureModMaker.fps') }}</span><el-input-number v-model="fps" :min="0.01" :max="240" :precision="3"/></label>
        <label v-if="sourceKind !== 'image'"><span>{{ t('textureModMaker.frameStep') }}</span><el-input-number v-model="frameStep" :min="1" :max="1000"/></label>
        <label v-if="sourceKind === 'video'" class="tm-option-wide tm-trim-option"><span>{{ t('textureModMaker.trimRange') }}</span><div class="tm-trim-control"><el-slider v-model="videoTrimRange" range :min="0" :max="videoFrameMax" :disabled="videoFrameMax < 1"/><div class="tm-trim-endpoints"><div><span>{{ t('textureModMaker.trimStart') }}</span><el-input-number :model-value="videoTrimRange[0]" :min="0" :max="videoTrimRange[1]" controls-position="right" @update:model-value="setTrimStart"/></div><div><span>{{ t('textureModMaker.trimEnd') }}</span><el-input-number :model-value="videoTrimRange[1]" :min="videoTrimRange[0]" :max="videoFrameMax" controls-position="right" @update:model-value="setTrimEnd"/></div></div></div></label>
        <label><span>{{ t('textureModMaker.size') }}</span><el-slider v-model="sizePercent" :min="10" :max="100" :step="5" show-input/></label>
        <label v-if="sourceKind === 'video'"><span>{{ t('textureModMaker.videoQuality') }}</span><el-slider v-model="videoQuality" :min="1" :max="100" show-input/></label>
        <label><span>{{ t('textureModMaker.cpuThreads') }}</span><el-input-number v-model="cpuThreads" :min="1" :max="cpuThreadLimit"/><small class="tm-thread-hint">{{ t('textureModMaker.cpuThreadsHint', { max: cpuThreadLimit }) }}</small></label>
        <label><span>{{ t('textureModMaker.gpuWorkers') }}</span><el-input-number v-model="gpuWorkers" :min="1" :max="2"/><small class="tm-thread-hint">{{ t('textureModMaker.gpuWorkersHint') }}</small></label>
        <label v-if="selectedTexture?.format.toUpperCase().startsWith('BC7')"><span>{{ t('textureModMaker.bc7Quality') }}</span><el-radio-group v-model="bc7Quality" class="tm-multi-switch"><el-radio-button value="quick">{{ t('textureModMaker.bc7QualityModes.quick') }}</el-radio-button><el-radio-button value="standard">{{ t('textureModMaker.bc7QualityModes.standard') }}</el-radio-button></el-radio-group><small class="tm-thread-hint">{{ t('textureModMaker.bc7QualityHint') }}</small></label>
        <label><span>{{ t('textureModMaker.texconvBatchSize') }}</span><el-input-number v-model="texconvBatchSize" :min="1" :max="64"/><small class="tm-thread-hint">{{ t('textureModMaker.texconvBatchSizeHint') }}</small></label>
        <label v-if="sourceKind !== 'image'"><span>{{ t('textureModMaker.loop') }}</span><el-radio-group v-model="loopMode" class="tm-multi-switch"><el-radio-button value="loop">{{ t('textureModMaker.loopModes.loop') }}</el-radio-button><el-radio-button value="once">{{ t('textureModMaker.loopModes.once') }}</el-radio-button></el-radio-group></label>
      </div>
    </section>

    <section class="tm-card tm-output">
      <div class="tm-section-head"><b>5</b><div><h2>{{ t('textureModMaker.outputTitle') }}</h2><p>{{ t('textureModMaker.outputDesc') }}</p></div></div>
      <div class="tm-output-grid"><el-input v-model="modName" :placeholder="t('textureModMaker.modName')"/><div class="tm-path-row"><el-input v-model="outputDirectory" clearable :placeholder="t('textureModMaker.outputFolderDefault')"/><el-button @click="pickOutput">{{ t('textureModMaker.choose') }}</el-button></div><div class="tm-generate-actions"><el-button v-if="generating" type="danger" size="large" :loading="cancelling" @click="cancelGeneration">{{ t('textureModMaker.cancelGeneration') }}</el-button><el-button v-else type="primary" size="large" :disabled="!canGenerate" @click="generate">{{ t('textureModMaker.generate') }}</el-button></div></div>
      <small v-if="finalOutputPath" class="tm-output-location">{{ t('textureModMaker.outputLocation') }} {{ finalOutputPath }}</small>
      <small v-if="generationEstimate" class="tm-generation-estimate">{{ generationEstimate }}</small>
      <el-button v-if="generationProgress.logPath" class="tm-log-button" text size="small" @click="openGenerationLog">{{ t('textureModMaker.openGenerationLog') }}</el-button>
      <div v-if="generating || generationProgress.phase === 'error'" class="tm-generation-progress"><div><span>{{ generationProgressText }}</span><small v-if="generationProgress.message" :title="generationProgress.message">{{ generationProgress.message }}</small></div><div class="tm-pipeline-progress" role="progressbar"><span v-for="stage in generationStages" :key="stage.phase" class="tm-pipeline-stage" :class="[`phase-${stage.phase}`, { active: stage.active, complete: stage.value >= 100 }]" :style="{ width: `${stage.value}%` }" :title="`${stage.label} ${Math.round(stage.value)}%`"></span></div><div class="tm-pipeline-details"><span v-for="stage in generationStages" :key="stage.phase" :class="{ active: stage.active, complete: stage.value >= 100 }"><i></i>{{ stage.label }}<b v-if="stage.total">{{ stage.processed }}/{{ stage.total }}</b><b v-else>{{ Math.round(stage.value) }}%</b><b v-if="stage.elapsedMs">{{ (stage.elapsedMs / 1000).toFixed(1) }}s</b></span></div></div>
    </section>

    <div v-if="targetPreviewOpen" class="channel-preview-overlay" role="dialog" aria-modal="true" @click.self="targetPreviewOpen = false"><div class="channel-preview-modal" @click.stop><header class="channel-preview-modal-header"><h3>{{ selectedTexture ? `${selectedTexture.hash} · ${selectedTexture.width}×${selectedTexture.height}` : '' }}</h3><div class="channel-preview-toolbar"><button v-for="channel in (['R','G','B','A'] as const)" :key="channel" type="button" class="channel-preview-toggle" :class="{ 'is-active': viewerChannels[channel] }" @click="toggleViewerChannel(channel)">{{ channel }}</button><span class="channel-preview-zoom">{{ Math.round(targetPreviewScale * 100) }}%</span><button class="channel-preview-close-btn" type="button" @click="targetPreviewOpen = false"><el-icon><Close/></el-icon></button></div></header><div class="channel-preview-stage" :class="{ dragging: targetPreviewDragging }" @wheel.prevent="targetPreviewScale = Math.min(12, Math.max(.2, targetPreviewScale * ($event.deltaY < 0 ? 1.12 : .89)))" @pointerdown="startTargetPreviewDrag" @pointermove="moveTargetPreviewDrag" @pointerup="stopTargetPreviewDrag" @pointercancel="stopTargetPreviewDrag" @dblclick="resetTargetPreview"><canvas ref="viewerCanvas" :style="targetPreviewTransform"/></div></div></div>

    <div v-if="sourceDetailOpen" class="channel-preview-overlay" role="dialog" aria-modal="true" @click.self="sourceDetailOpen = false"><div class="channel-preview-modal tm-source-preview-modal" @click.stop><header class="channel-preview-modal-header"><h3>{{ t('textureModMaker.sourcePreviewTitle') }}</h3><div class="channel-preview-toolbar"><button v-for="channel in (['R','G','B','A'] as const)" :key="channel" type="button" class="channel-preview-toggle" :class="{ 'is-active': sourceDetailChannels[channel] }" @click="toggleSourceDetailChannel(channel)">{{ channel }}</button><span class="channel-preview-zoom">{{ Math.round(sourceDetailScale * 100) }}%</span><button class="channel-preview-close-btn" type="button" @click="sourceDetailOpen = false"><el-icon><Close/></el-icon></button></div></header><div class="channel-preview-stage tm-source-preview-stage" :class="{ dragging: sourceDetailDragging }" @wheel.prevent="zoomSourceDetail" @pointerdown="startSourceDetailDrag" @pointermove="moveSourceDetailDrag" @pointerup="stopSourceDetailDrag" @pointercancel="stopSourceDetailDrag" @dblclick="resetSourceDetail"><canvas ref="sourceDetailCanvas" :style="sourceDetailTransform"/></div></div></div>
  </div>
</template>

<style scoped>
.tm-page{min-height:100%;padding:28px;box-sizing:border-box;color:#f4f7f8}.tm-hero{display:flex;align-items:flex-start;justify-content:space-between;gap:24px;margin-bottom:18px}.tm-hero h1{margin:5px 0 8px;font-size:28px}.tm-hero p,.tm-section-head p{margin:0;color:rgba(255,255,255,.55);line-height:1.5}.tm-kicker{color:#75d6bb;font-size:11px;font-weight:800;letter-spacing:.16em}.tm-tool-status{display:flex;align-items:center;gap:10px;min-width:230px;padding:12px 14px;border:1px solid rgba(255,150,120,.25);border-radius:12px;background:rgba(10,14,20,.68)}.tm-tool-status>span{width:9px;height:9px;border-radius:50%;background:#ff856b;box-shadow:0 0 10px #ff856b}.tm-tool-status.ready{border-color:rgba(117,214,187,.25)}.tm-tool-status.ready>span{background:#75d6bb;box-shadow:0 0 10px #75d6bb}.tm-tool-status div{display:flex;flex:1;flex-direction:column}.tm-tool-status small{color:rgba(255,255,255,.5)}.tm-card{margin-bottom:14px;padding:18px;border:1px solid rgba(255,255,255,.09);border-radius:15px;background:rgba(9,14,21,.7);box-shadow:0 12px 30px rgba(0,0,0,.16);backdrop-filter:blur(14px)}.tm-section-head{display:flex;align-items:center;gap:12px;margin-bottom:15px}.tm-section-head>b{display:grid;place-items:center;width:28px;height:28px;border-radius:9px;background:rgba(117,214,187,.16);color:#75d6bb}.tm-section-head h2{margin:0 0 3px;font-size:15px}.tm-section-head p{font-size:12px}.tm-path-row,.tm-target-row{display:flex;align-items:center;gap:8px}.tm-target-row{margin-top:10px}.tm-target-row>.el-autocomplete{width:230px}.tm-target-chip{display:flex;align-items:center;gap:10px;min-width:0;padding:5px 8px;border-radius:8px;background:rgba(117,214,187,.09);font-size:12px}.tm-target-chip code{color:#8be2ca}.tm-target-chip span:last-of-type{max-width:240px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:rgba(255,255,255,.55)}.tm-columns{display:grid;grid-template-columns:1fr 1fr;gap:14px}.tm-columns>.tm-card{min-width:0}.tm-card :deep(.el-segmented){margin-bottom:12px}.tm-meta{display:block;margin-top:8px;color:rgba(255,255,255,.48)}.tm-sequence-nav{display:flex;align-items:center;gap:12px;margin-top:10px}.tm-sequence-nav .el-slider{flex:1}.tm-preview-card{display:flex;flex-direction:column}.tm-preview{position:relative;display:grid;place-items:center;flex:1;min-height:210px;max-height:420px;overflow:hidden;border:1px solid rgba(255,255,255,.08);border-radius:12px;background-color:#080b10;background-image:linear-gradient(45deg,#161b22 25%,transparent 25%),linear-gradient(-45deg,#161b22 25%,transparent 25%),linear-gradient(45deg,transparent 75%,#161b22 75%),linear-gradient(-45deg,transparent 75%,#161b22 75%);background-position:0 0,0 8px,8px -8px,-8px 0;background-size:16px 16px;background-repeat:repeat}.tm-preview.fit-tile{background-repeat:repeat;background-size:auto}.tm-preview img,.tm-preview video{width:100%;height:100%;transition:transform .2s ease}.tm-preview .object-cover{object-fit:cover}.tm-preview .object-contain{object-fit:contain}.tm-preview .object-fill{object-fit:fill}.tm-preview .object-none{object-fit:none}.tm-preview>span{color:rgba(255,255,255,.35)}.tm-preview>em{position:absolute;right:8px;bottom:8px;padding:4px 7px;border-radius:6px;background:rgba(0,0,0,.62);font-size:10px;font-style:normal}.tm-options{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px 24px}.tm-options label{display:grid;grid-template-columns:150px minmax(0,1fr);align-items:center;gap:12px}.tm-options label>span{font-size:13px;color:rgba(255,255,255,.78)}.tm-options .tm-switch :deep(.el-switch){justify-self:start}.tm-output-grid{display:grid;grid-template-columns:minmax(150px,.45fr) minmax(280px,1fr) auto;gap:10px}.tm-viewer{display:grid;place-items:center;height:min(65vh,650px);overflow:hidden;background:#080b10}.tm-viewer img{max-width:100%;max-height:100%;image-rendering:auto;transition:transform .12s ease}.tm-viewer-dialog :deep(.el-dialog){background:#111720}.tm-viewer-dialog :deep(.el-dialog__title){color:white}@media(max-width:850px){.tm-columns{grid-template-columns:1fr}.tm-options{grid-template-columns:1fr}.tm-output-grid{grid-template-columns:1fr}.tm-hero{flex-direction:column}.tm-tool-status{width:100%;box-sizing:border-box}}@media(max-width:620px){.tm-page{padding:16px}.tm-path-row,.tm-target-row{align-items:stretch;flex-wrap:wrap}.tm-target-chip{width:100%}.tm-options label{grid-template-columns:1fr}}
.tm-viewer-tools{display:flex;align-items:center;gap:6px;margin-bottom:8px}.tm-viewer-tools span{margin-left:auto;color:rgba(255,255,255,.55);font-size:12px}.tm-viewer canvas{max-width:100%;max-height:100%;transition:transform .12s ease}.tm-viewer-source{position:absolute;width:1px;height:1px;opacity:0;pointer-events:none}
.tm-preview{flex:none;align-self:center;height:auto;min-height:0;max-height:420px}.tm-option-wide{grid-column:1/-1}.tm-source-switch{margin-bottom:12px}.tm-multi-switch{display:inline-flex;justify-self:start;width:max-content;max-width:100%;min-height:30px;gap:0!important;overflow:hidden;border-radius:7px}.tm-multi-switch :deep(.el-radio-button){flex:0 0 auto;margin:0!important}.tm-multi-switch :deep(.el-radio-button__inner){display:flex;align-items:center;justify-content:center;box-sizing:border-box;width:auto;min-width:72px;min-height:30px;margin:0!important;padding:0 14px;border:none!important;border-radius:0!important;outline:none!important;background:rgba(255,255,255,.055);color:rgba(var(--theme-text-primary-rgb),.78);font:inherit;font-size:11px;line-height:1;white-space:nowrap;box-shadow:none!important;transition:background .16s ease,color .16s ease}.tm-multi-switch :deep(.el-radio-button__inner:hover){background:rgba(var(--theme-surface-tint-rgb),.16);color:rgba(var(--theme-text-primary-rgb),.96)}.tm-multi-switch :deep(.el-radio-button.is-active .el-radio-button__inner),.tm-multi-switch :deep(.el-radio-button__original-radio:checked + .el-radio-button__inner){background:rgba(var(--theme-surface-tint-rgb),.20);color:rgba(var(--theme-text-primary-rgb),.98);box-shadow:none!important}.tm-preview-detail-button{position:absolute;top:8px;right:8px;z-index:2;background:rgba(9,14,21,.78);backdrop-filter:blur(8px)}.tm-source-detail{cursor:grab;touch-action:none}.tm-source-detail.dragging{cursor:grabbing}.tm-source-detail img{max-width:100%;max-height:100%;user-select:none;pointer-events:none;transform-origin:center;transition:none}.tm-viewer-tools small{color:rgba(255,255,255,.48)}
.tm-texture-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:9px;max-height:390px;margin-top:12px;padding:2px 6px 2px 2px;overflow-y:auto;scrollbar-width:thin}.tm-texture-card{display:grid;grid-template-columns:64px minmax(0,1fr);align-items:center;gap:10px;min-width:0;padding:7px;border:1px solid rgba(255,255,255,.09);border-radius:10px;background:rgba(255,255,255,.035);color:white;text-align:left;cursor:pointer;transition:border-color .16s ease,background .16s ease,transform .16s ease}.tm-texture-card:hover{border-color:rgba(var(--theme-surface-tint-rgb),.34);background:rgba(var(--theme-surface-tint-rgb),.08)}.tm-texture-card.selected{border-color:rgba(117,214,187,.65);background:rgba(117,214,187,.15);box-shadow:inset 0 0 0 1px rgba(117,214,187,.12)}.tm-texture-thumb{display:grid;place-items:center;width:64px;height:64px;overflow:hidden;border-radius:7px;background-color:#171c24;background-image:linear-gradient(45deg,#242b36 25%,transparent 25%),linear-gradient(-45deg,#242b36 25%,transparent 25%),linear-gradient(45deg,transparent 75%,#242b36 75%),linear-gradient(-45deg,transparent 75%,#242b36 75%);background-size:12px 12px;background-position:0 0,0 6px,6px -6px,-6px 0}.tm-texture-thumb img{max-width:100%;max-height:100%;object-fit:contain}.tm-thumb-loading{width:18px;height:18px;border:2px solid rgba(255,255,255,.15);border-top-color:#75d6bb;border-radius:50%;animation:tm-spin .8s linear infinite}.tm-texture-info{display:flex;min-width:0;flex-direction:column;gap:5px}.tm-texture-info code{color:rgba(255,255,255,.9);font-size:13px}.tm-texture-info small{overflow:hidden;color:rgba(255,255,255,.48);font-size:10px;text-overflow:ellipsis;white-space:nowrap}.tm-empty-filter{padding:24px;text-align:center;color:rgba(255,255,255,.42)}@keyframes tm-spin{to{transform:rotate(360deg)}}
.tm-source-detail-square{width:min(68vh,100%);height:auto;aspect-ratio:1/1;margin:0 auto}.tm-source-detail-square canvas{max-width:80%;max-height:80%;transform-origin:center}.tm-viewer:not(.tm-source-detail-square) canvas{max-width:92%;max-height:92%;transform-origin:center}
.tm-page{padding:var(--t-page-padding);color:var(--t-page-text)}
.tm-card{border:var(--t-page-panel-border);border-radius:var(--t-page-panel-radius);background:var(--t-page-panel-bg);box-shadow:var(--t-page-panel-shadow);backdrop-filter:var(--t-page-panel-blur);-webkit-backdrop-filter:var(--t-page-panel-blur)}
.tm-hero p,.tm-section-head p{color:var(--t-page-text-muted)}
@media(max-width:620px){.tm-page{padding:var(--t-page-padding-compact)}}
.tm-section-head>.tm-tool-status{margin-left:auto;min-width:170px;padding:7px 10px}.tm-section-head>.tm-tool-status>span{width:7px;height:7px}.tm-section-head>.tm-tool-status strong{font-size:12px}.tm-section-head>.tm-tool-status small{font-size:10px}
.channel-preview-overlay{position:fixed;inset:0;z-index:3000;display:flex;align-items:center;justify-content:center;padding:16px;background:rgba(5,9,16,.42);backdrop-filter:blur(6px)}.channel-preview-modal{position:relative;width:min(82vw,1100px);height:min(82vh,850px);max-width:calc(100vw - 32px);max-height:calc(100vh - 32px);overflow:hidden;display:flex;flex-direction:column;border:var(--t-card-dark-border);border-radius:14px;background:var(--t-card-dark-bg);box-shadow:var(--t-card-dark-shadow);backdrop-filter:blur(12px) saturate(1.15);user-select:none}.channel-preview-modal-header{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:44px;padding:0 10px 0 14px;border-bottom:1px solid rgba(var(--theme-surface-tint-rgb),.12);background:rgba(var(--theme-surface-tint-rgb),.035)}.channel-preview-modal-header h3{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin:0;color:rgba(var(--theme-text-primary-rgb),.94);font-size:13px;font-weight:650}.channel-preview-toolbar{display:flex;align-items:center;gap:5px;flex:0 0 auto}.channel-preview-toggle,.channel-preview-close-btn{display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;padding:0;border:1px solid rgba(var(--theme-surface-tint-rgb),.16);border-radius:6px;background:rgba(255,255,255,.035);color:rgba(var(--theme-text-primary-rgb),.38);font-size:12px;font-weight:750;cursor:pointer}.channel-preview-toggle.is-active{border-color:rgba(117,214,187,.5);background:rgba(117,214,187,.16);color:rgba(224,255,247,.96)}.channel-preview-close-btn{background:rgba(255,255,255,.045);color:rgba(var(--theme-text-primary-rgb),.8)}.channel-preview-close-btn:hover{background:rgba(239,68,68,.16);border-color:rgba(239,68,68,.4);color:rgba(var(--theme-text-primary-rgb),.98)}.channel-preview-zoom{min-width:42px;color:rgba(var(--theme-text-secondary-rgb),.65);font-size:11px;text-align:center}.channel-preview-stage{position:relative;flex:1 1 auto;min-height:0;overflow:hidden;display:flex;align-items:center;justify-content:center;background-color:#242832;background-image:linear-gradient(45deg,#343946 25%,transparent 25%),linear-gradient(-45deg,#343946 25%,transparent 25%),linear-gradient(45deg,transparent 75%,#343946 75%),linear-gradient(-45deg,transparent 75%,#343946 75%);background-position:0 0,0 8px,8px -8px,-8px 0;background-size:16px 16px;cursor:grab;touch-action:none}.channel-preview-stage.dragging{cursor:grabbing}.channel-preview-stage canvas{display:block;image-rendering:pixelated;max-width:92%;max-height:92%;object-fit:contain;transform-origin:center;will-change:transform}.tm-source-preview-modal{width:min(82vh,850px);height:min(82vh,850px);aspect-ratio:1/1}.tm-source-preview-stage canvas{max-width:80%;max-height:80%}
.channel-preview-overlay {
  top: var(--ssmt-content-overlay-top);
  right: 0;
  bottom: 0;
  left: var(--ssmt-content-overlay-left);
}
.channel-preview-modal {
  max-width: calc(100vw - var(--ssmt-content-overlay-left) - 32px);
  max-height: calc(100vh - var(--ssmt-content-overlay-top) - 32px);
}
.tm-options .tm-multi-switch :deep(> .el-radio-button){display:block!important;grid-template-columns:none!important;gap:0!important;width:auto!important;padding:0!important}
.tm-generation-estimate{display:block;margin-top:9px;color:rgba(var(--theme-text-secondary-rgb),.62);font-size:11px}
.tm-output-location{display:block;overflow:hidden;margin-top:8px;color:rgba(var(--theme-text-secondary-rgb),.42);font-size:10px;text-overflow:ellipsis;white-space:nowrap}
.tm-generate-actions{display:flex}.tm-generate-actions>.el-button{width:100%;margin:0}.tm-video-controls{position:absolute;z-index:3;right:8px;bottom:8px;left:8px;display:flex;align-items:center;gap:7px;height:34px;padding:0 8px;border:1px solid rgba(255,255,255,.12);border-radius:8px;background:rgba(5,9,14,.82);color:rgba(255,255,255,.78);font-size:10px;backdrop-filter:blur(10px)}.tm-video-controls button{display:grid;place-items:center;flex:0 0 auto;width:24px;height:24px;padding:0;border:0;border-radius:5px;background:rgba(255,255,255,.08);color:white;cursor:pointer}.tm-video-controls input{min-width:40px;flex:1;accent-color:#75d6bb}
.tm-trim-control{display:grid;min-width:0;gap:5px}.tm-trim-control>.el-slider{min-width:0;margin:0 10px;width:calc(100% - 20px)}.tm-trim-endpoints{display:flex;align-items:center;justify-content:space-between;gap:16px}.tm-trim-endpoints>div{display:flex;align-items:center;gap:7px}.tm-trim-endpoints span{color:rgba(var(--theme-text-secondary-rgb),.58);font-size:10px}.tm-trim-endpoints .el-input-number{width:116px}@media(max-width:720px){.tm-trim-option{grid-template-columns:1fr!important}.tm-trim-endpoints{align-items:stretch}.tm-trim-endpoints>div{flex:1}.tm-trim-endpoints .el-input-number{width:100%}}
.tm-generation-progress{display:grid;gap:7px;margin-top:10px;padding:9px 11px;border:1px solid rgba(var(--theme-surface-tint-rgb),.12);border-radius:9px;background:rgba(var(--theme-surface-tint-rgb),.035)}.tm-generation-progress>div:first-child{display:flex;align-items:center;justify-content:space-between;gap:12px;color:rgba(var(--theme-text-primary-rgb),.78);font-size:11px}.tm-generation-progress small{max-width:55%;overflow:hidden;color:#ff9b87;text-overflow:ellipsis;white-space:nowrap}.tm-pipeline-progress{position:relative;width:100%;height:8px;overflow:hidden;border-radius:999px;background:rgba(var(--theme-surface-tint-rgb),.08)}.tm-pipeline-stage{position:absolute;inset:0 auto 0 0;display:block;transition:width .22s ease}.tm-pipeline-stage.phase-extracting{z-index:1;background:rgba(117,214,187,.24)}.tm-pipeline-stage.phase-transforming{z-index:2;background:rgba(117,214,187,.44)}.tm-pipeline-stage.phase-encoding{z-index:3;background:rgba(117,214,187,.7)}.tm-pipeline-stage.phase-writing{z-index:4;background:rgba(117,214,187,.94)}.tm-pipeline-stage.active{box-shadow:inset -2px 0 0 rgba(225,255,247,.9)}.tm-pipeline-details{display:flex!important;align-items:center!important;justify-content:flex-start!important;flex-wrap:wrap;gap:7px 15px!important}.tm-pipeline-details>span{display:flex;align-items:center;gap:5px;color:rgba(var(--theme-text-secondary-rgb),.48);font-size:10px}.tm-pipeline-details i{width:6px;height:6px;border-radius:50%;background:rgba(117,214,187,.3)}.tm-pipeline-details span.active{color:rgba(var(--theme-text-primary-rgb),.86)}.tm-pipeline-details span.active i{background:rgba(117,214,187,.7)}.tm-pipeline-details span.complete{color:rgba(var(--theme-text-secondary-rgb),.68)}.tm-pipeline-details span.complete i{background:rgba(117,214,187,.9)}.tm-pipeline-details b{font-weight:600;color:inherit}.tm-thread-hint{grid-column:2;color:rgba(var(--theme-text-secondary-rgb),.52);font-size:10px}

/* Unify legacy tool accents with the current SSMT brand accent. */
.tm-kicker,
.tm-section-head > b,
.tm-target-chip code,
.tm-thumb-loading {
  color: var(--ssmt-brand-accent);
}

.tm-tool-status.ready {
  border-color: rgba(var(--ssmt-brand-accent-rgb), .25);
}

.tm-tool-status.ready > span,
.tm-pipeline-details span.complete i {
  background: var(--ssmt-brand-accent);
  box-shadow: 0 0 10px rgba(var(--ssmt-brand-accent-rgb), .5);
}

.tm-section-head > b,
.tm-target-chip,
.tm-texture-card.selected,
.channel-preview-toggle.is-active {
  background: rgba(var(--ssmt-brand-accent-rgb), .14);
}

.tm-texture-card.selected,
.channel-preview-toggle.is-active {
  border-color: rgba(var(--ssmt-brand-accent-rgb), .62);
  color: var(--ssmt-brand-accent-soft);
}

.tm-texture-card.selected {
  box-shadow: inset 0 0 0 1px rgba(var(--ssmt-brand-accent-rgb), .12);
}

.tm-thumb-loading {
  border-top-color: var(--ssmt-brand-accent);
}

.tm-video-controls input {
  accent-color: var(--ssmt-brand-accent);
}

.tm-pipeline-stage.phase-extracting { background: rgba(var(--ssmt-brand-accent-rgb), .24); }
.tm-pipeline-stage.phase-transforming { background: rgba(var(--ssmt-brand-accent-rgb), .44); }
.tm-pipeline-stage.phase-encoding { background: rgba(var(--ssmt-brand-accent-rgb), .70); }
.tm-pipeline-stage.phase-writing { background: rgba(var(--ssmt-brand-accent-rgb), .94); }
.tm-pipeline-stage.active { box-shadow: inset -2px 0 0 var(--ssmt-brand-accent-soft); }
.tm-pipeline-details i { background: rgba(var(--ssmt-brand-accent-rgb), .30); }
.tm-pipeline-details span.active i { background: rgba(var(--ssmt-brand-accent-rgb), .70); }
</style>
