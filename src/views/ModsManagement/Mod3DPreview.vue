<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { exists, readDir, readFile } from '@tauri-apps/plugin-fs';
import { join, resourceDir } from '@tauri-apps/api/path';
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { createGIMIHighFidelityMaterial, GIMIShaderController, GIMITextureSet } from '../MarkTexture/GIMIHighFidelityMaterial';
import type { ModInfo } from './ModsManagement.types';
import {
  analyzeModIniFilesFull,
  extractVariables,
  filterActiveBlocks,
  type DrawIBGroup,
  type ModIniFullAnalysis,
} from './MigotoIni';
import { buildMigotoDrawSnapshots } from './MigotoSemantic';

type TextureRole = 'auto' | 'diffuse' | 'normal' | 'light';
type ShadingMode = 'pbr' | 'emission' | 'npr' | 'gimi';
type TextureOption = {
  id: string;
  filename: string;
  label: string;
  absolutePath: string;
};

const props = defineProps<{ mod?: ModInfo }>();
const host = ref<HTMLDivElement>();
const loading = ref(false);
const meshLoading = ref(false);
const error = ref('');
const analysis = ref<ModIniFullAnalysis>();
const groups = ref<DrawIBGroup[]>([]);
const selectedGroupKey = ref('');
const variableStates = ref<Record<string, string>>({});
const submeshTextureBindings = ref<Record<string, string>>({});
const allTextureOptions = ref<TextureOption[]>([]);
const textureThumbnails = ref<Record<string, string>>({});
const openTexturePicker = ref('');
const zoomOpen = ref(false);
const zoomHost = ref<HTMLDivElement>();
const lightAzimuth = ref(0);
const lightElevation = ref(0);
const shadingMode = ref<ShadingMode>('gimi');
try {
  const storedMode = localStorage.getItem('ssmt4:mod-3d-shading-mode');
  if (storedMode === 'pbr' || storedMode === 'emission' || storedMode === 'npr' || storedMode === 'gimi') shadingMode.value = storedMode;
} catch { /* Use GIMI by default. */ }

let renderer: THREE.WebGLRenderer | undefined;
let zoomRenderer: THREE.WebGLRenderer | undefined;
let scene: THREE.Scene | undefined;
let camera: THREE.PerspectiveCamera | undefined;
let controls: OrbitControls | undefined;
let zoomControls: OrbitControls | undefined;
let keyLight: THREE.DirectionalLight | undefined;
let previewMeshes: THREE.Mesh[] = [];
const meshCache = new Map<string, THREE.Mesh>();
let cameraHasInitialFrame = false;
let resizeObserver: ResizeObserver | undefined;
let analysisToken = 0;
let meshLoadToken = 0;
let preparingControlFlow = false;
const shaderController = new GIMIShaderController();
let bundledDefaults: Promise<{ ramp?: THREE.Texture; metal?: THREE.Texture }> | undefined;

const fallbackTexture = (rgba: readonly [number, number, number, number], color = false) => {
  const texture = new THREE.DataTexture(new Uint8Array(rgba), 1, 1, THREE.RGBAFormat, THREE.UnsignedByteType);
  texture.colorSpace = color ? THREE.SRGBColorSpace : THREE.NoColorSpace;
  texture.minFilter = THREE.NearestFilter;
  texture.magFilter = THREE.NearestFilter;
  texture.needsUpdate = true;
  return texture;
};

const lightIndicatorStyle = computed(() => {
  const azimuth = THREE.MathUtils.degToRad(lightAzimuth.value);
  const elevation = THREE.MathUtils.degToRad(lightElevation.value);
  return { transform: `translate(${Math.sin(azimuth) * Math.cos(elevation) * 30}px, ${-Math.sin(elevation) * 30}px)` };
});

const selectedGroup = computed(() => groups.value.find(group => group.groupKey === selectedGroupKey.value));
const namespacedVariableAliases = (result: ModIniFullAnalysis) => {
  const namespaceBySource = new Map<string, string>();
  for (const section of [...result.textureOverrides, ...result.commandLists]) {
    if (section.namespace) namespaceBySource.set(section.sourceIniPath.toLowerCase(), section.namespace);
  }
  const aliases = new Map<string, string>();
  for (const binding of result.constantBindings) {
    const namespace = namespaceBySource.get(binding.declarationSourceIniPath.toLowerCase());
    if (!namespace) continue;
    aliases.set(`$\\${namespace}\\${binding.name.replace(/^\$/, '')}`, binding.name);
  }
  return aliases;
};
const semanticVariableStates = (result: ModIniFullAnalysis, sourceStates: Record<string, string> = variableStates.value) => {
  const states = { ...sourceStates };
  for (const [alias, declared] of namespacedVariableAliases(result)) {
    if (states[declared] !== undefined) states[alias] = states[declared];
  }
  return states;
};
const variables = computed(() => {
  if (!analysis.value) return [];
  const aliases = namespacedVariableAliases(analysis.value);
  const normalizedAliases = new Set([...aliases.keys()].map(alias => alias.toLowerCase()));
  return extractVariables(analysis.value).filter(variable => !normalizedAliases.has(variable.name.toLowerCase()));
});

const semanticGroups = (result: ModIniFullAnalysis, states: Record<string, string> = variableStates.value): DrawIBGroup[] => {
  const snapshots = buildMigotoDrawSnapshots(result, semanticVariableStates(result, states));
  const grouped = new Map<string, typeof snapshots>();
  for (const snapshot of snapshots) {
    const key = `${snapshot.sourceIniPath}:${snapshot.label}:${snapshot.ib?.resourceId || snapshot.ib?.sectionName}`;
    const list = grouped.get(key) || [];
    list.push(snapshot); grouped.set(key, list);
  }
  return [...grouped.entries()].map(([groupKey, draws]) => {
    const first = draws[0];
    const vbFiles = Object.entries(first.vertexCandidates).flatMap(([slot, resources]) => resources.map(resource => ({
      slot, resourceName: resource.canonicalName || resource.sectionName, filename: resource.filename,
      resourceId: resource.resourceId, stride: resource.stride, format: resource.format, sourceIniPath: resource.sourceIniPath,
    })));
    const textureByRole = new Map<string, DrawIBGroup['textureFiles'][number]>();
    for (const draw of draws) {
      for (const [slot, resource] of Object.entries(draw.bindings)) if (/^ps-t\d+$/i.test(slot) && resource?.filename) {
        textureByRole.set(`${slot}:${resource.resourceId}`, { slot, resourceName: resource.canonicalName || resource.sectionName, filename: resource.filename, resourceId: resource.resourceId, sourceIniPath: resource.sourceIniPath });
      }
      for (const [role, resource] of Object.entries(draw.semanticTextures)) if (resource) {
        textureByRole.set(`${role}:${resource.resourceId}`, { slot: `semantic-${role}`, resourceName: resource.canonicalName || resource.sectionName, filename: resource.filename, resourceId: resource.resourceId, sourceIniPath: resource.sourceIniPath, sourceLabel: role });
      }
      for (const binding of draw.hashTextures) {
        const resource = binding.resource;
        textureByRole.set(`hash:${binding.hash}:${resource.resourceId}`, {
          slot: `hash:${binding.hash || '?'}`,
          resourceName: resource.canonicalName || resource.sectionName,
          filename: resource.filename,
          resourceId: resource.resourceId,
          sourceIniPath: resource.sourceIniPath,
          sourceLabel: binding.overrideSectionName,
        });
      }
    }
    return {
      groupKey, drawHash: first.hash, matchFirstIndex: first.matchFirstIndex?.toString(), sectionNames: [first.label],
      ibFile: first.ib!.filename, ibFormat: first.ib!.format, ibSourceIniPath: first.ib!.sourceIniPath, ibResourceId: first.ib!.resourceId,
      vbFiles, textureFiles: [...textureByRole.values()],
      allBlocks: [{ conditions: [], replaces: [], runs: [], drawCalls: draws.map(draw => ({ type: 'drawindexed', value: `${draw.indexCount},${draw.startIndex},${draw.baseVertex}` })) }],
    };
  });
};

const dirname = (path: string): string => path.replace(/\\/g, '/').split('/').slice(0, -1).join('/');
const joinPath = (...parts: Array<string | undefined>): string => parts
  .filter(Boolean).join('/').replace(/\\/g, '/').replace(/\/+/g, '/');
const resolveResourcePath = (filename: string, sourceIniPath?: string): string => {
  if (!props.mod) return filename;
  return joinPath(props.mod.path, dirname(sourceIniPath || ''), filename);
};

const inferTextureRole = (slot: string, resourceName: string, filename: string, sourceLabel = ''): TextureRole => {
  const text = `${slot} ${resourceName} ${filename} ${sourceLabel}`.toLowerCase();
  if (/normal|bump/.test(text)) return 'normal';
  if (/light.?map|\bilm\b/.test(text)) return 'light';
  if (/diffuse|deffuse|albedo|basecolor|base_color/.test(text)) return 'diffuse';
  if (/semantic-diffuse/.test(slot)) return 'diffuse';
  if (/semantic-normal/.test(slot)) return 'normal';
  if (/semantic-light/.test(slot)) return 'light';
  // ps-tN proves that this texture participates in the draw, but N is not a
  // portable material semantic across GIMI/SRMI/ZZMI. Only names or explicit
  // Resource\<preset>\Diffuse/LightMap/NormalMap assignments classify it.
  return 'auto';
};

const effectiveTextureRole = (group: DrawIBGroup, index: number): TextureRole => {
  const texture = group.textureFiles[index];
  return inferTextureRole(texture.slot, texture.resourceName, texture.filename, texture.sourceLabel);
};

const bindingKey = (group: DrawIBGroup, role: 'diffuse' | 'normal' | 'light') => `${group.groupKey}:${role}`;
const setSubmeshTexture = (group: DrawIBGroup, role: 'diffuse' | 'normal' | 'light', textureId: string) => {
  submeshTextureBindings.value[bindingKey(group, role)] = textureId;
  if (props.mod) localStorage.setItem(`ssmt4:mod-3d-submesh-textures:${props.mod.relativePath}`, JSON.stringify(submeshTextureBindings.value));
  openTexturePicker.value = '';
  void rebuildMesh();
};

const selectedTextureOption = (group: DrawIBGroup, role: 'diffuse' | 'normal' | 'light') => {
  const id = submeshTextureBindings.value[bindingKey(group, role)];
  return allTextureOptions.value.find(texture => texture.id === id);
};

const hidePreviewMeshes = () => {
  for (const mesh of previewMeshes) mesh.visible = false;
  previewMeshes = [];
};
const disposeMesh = () => {
  hidePreviewMeshes();
  for (const mesh of meshCache.values()) {
    scene?.remove(mesh);
    mesh.geometry.dispose();
    const material = Array.isArray(mesh.material) ? mesh.material[0] : mesh.material;
    if (!material) continue;
    if (material instanceof THREE.ShaderMaterial) shaderController.detach(material);
    const ownedTextures = material.userData.ssmtOwnedTextures as THREE.Texture[] | undefined;
    ownedTextures?.forEach(texture => texture.dispose());
    material.dispose();
  }
  meshCache.clear();
};

const meshCacheKey = (group: DrawIBGroup) => JSON.stringify({
  group: group.groupKey,
  ib: group.ibResourceId || group.ibFile,
  draws: activeDrawRanges(group),
  mode: shadingMode.value,
  diffuse: submeshTextureBindings.value[bindingKey(group, 'diffuse')] || '',
  light: submeshTextureBindings.value[bindingKey(group, 'light')] || '',
  normal: submeshTextureBindings.value[bindingKey(group, 'normal')] || '',
});
const retainMesh = (key: string, mesh: THREE.Mesh) => {
  mesh.visible = false;
  meshCache.set(key, mesh);
  scene?.add(mesh);
};

const initializeRenderer = async () => {
  await nextTick();
  if (!host.value || renderer) return;
  renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio || 1, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  host.value.appendChild(renderer.domElement);
  scene = new THREE.Scene();
  scene.add(new THREE.HemisphereLight(0xffffff, 0x273044, 2.2));
  keyLight = new THREE.DirectionalLight(0xffffff, 2.5);
  keyLight.position.set(2, 3, 4);
  scene.add(keyLight);
  camera = new THREE.PerspectiveCamera(38, 1, 0.001, 100000);
  camera.position.set(2, 1.4, 3);
  controls = new OrbitControls(camera, renderer.domElement);
  // Event-driven rendering must not use damping: update() emits `change`, and
  // calling it again from the change handler creates an unbounded recursion.
  controls.enableDamping = false;
  controls.addEventListener('change', render);
  resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(host.value);
  resize();
};

const resize = () => {
  if (!host.value || !renderer || !camera) return;
  const width = Math.max(1, host.value.clientWidth);
  const height = Math.max(1, host.value.clientHeight);
  renderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
  render();
};

const render = () => {
  if (!renderer || !scene || !camera) return;
  shaderController.setFrame(performance.now() * 0.06);
  const renderTarget = (target: THREE.WebGLRenderer) => {
    const size = target.getSize(new THREE.Vector2());
    if (size.x > 0 && size.y > 0) {
      camera!.aspect = size.x / size.y;
      camera!.updateProjectionMatrix();
    }
    target.render(scene!, camera!);
  };
  try {
    renderTarget(renderer);
    if (zoomRenderer) renderTarget(zoomRenderer);
  } catch (cause) {
    // A malformed/unsupported DDS must not prevent geometry inspection. Some
    // DDS variants reach WebGLTextures with no 2D image and fail at `.width`.
    const texturedMaterials = previewMeshes.map(mesh => mesh.material).filter((material): material is THREE.MeshStandardMaterial => (
      material instanceof THREE.MeshStandardMaterial && !!(material.map || material.normalMap)
    ));
    if (texturedMaterials.length) {
      for (const material of texturedMaterials) {
        material.map?.dispose();
        material.normalMap?.dispose();
        material.map = null;
        material.normalMap = null;
        material.needsUpdate = true;
      }
      renderer.render(scene, camera);
      return;
    }
    error.value = cause instanceof Error ? cause.message : String(cause);
  }
};

const resizeZoom = () => {
  if (!zoomHost.value || !zoomRenderer || !camera) return;
  const width = Math.max(1, zoomHost.value.clientWidth);
  const height = Math.max(1, zoomHost.value.clientHeight);
  zoomRenderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
  render();
};

const applyLightOrbit = () => {
  const azimuth = THREE.MathUtils.degToRad(lightAzimuth.value);
  const elevation = THREE.MathUtils.degToRad(lightElevation.value);
  const x = Math.sin(azimuth) * Math.cos(elevation);
  const y = Math.sin(elevation);
  const z = Math.cos(azimuth) * Math.cos(elevation);
  if (!camera) { shaderController.setLightDirection(new THREE.Vector3(x, y, z)); return; }
  camera.updateMatrixWorld();
  const right = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 0).normalize();
  const up = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 1).normalize();
  const front = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 2).normalize();
  const direction = right.multiplyScalar(x).addScaledVector(up, y).addScaledVector(front, z).normalize();
  shaderController.setLightDirection(direction);
  keyLight?.position.copy(direction).multiplyScalar(10);
  render();
};

let lightDragging = false;
const setLightFromPointer = (event: PointerEvent, control: HTMLElement) => {
  const bounds = control.getBoundingClientRect();
  let x = ((event.clientX - bounds.left) / bounds.width) * 2 - 1;
  let y = ((event.clientY - bounds.top) / bounds.height) * 2 - 1;
  const length = Math.hypot(x, y);
  if (length > 1) { x /= length; y /= length; }
  lightAzimuth.value = THREE.MathUtils.radToDeg(Math.atan2(x, Math.sqrt(Math.max(1 - x * x - y * y, 0))));
  lightElevation.value = THREE.MathUtils.radToDeg(Math.asin(-y));
  applyLightOrbit();
};
const onLightPointerDown = (event: PointerEvent) => {
  lightDragging = true;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  setLightFromPointer(event, event.currentTarget as HTMLElement);
};
const onLightPointerMove = (event: PointerEvent) => { if (lightDragging) setLightFromPointer(event, event.currentTarget as HTMLElement); };
const onLightPointerUp = (event: PointerEvent) => {
  lightDragging = false;
  const target = event.currentTarget as HTMLElement;
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
};

const onZoomKeydown = (event: KeyboardEvent) => {
  if (event.key !== 'Escape') return;
  event.preventDefault(); event.stopImmediatePropagation(); closeZoom();
};
const closeZoom = () => {
  zoomOpen.value = false;
  window.removeEventListener('keydown', onZoomKeydown, true);
  window.removeEventListener('resize', resizeZoom);
  zoomControls?.dispose(); zoomControls = undefined;
  zoomRenderer?.dispose(); zoomRenderer?.domElement.remove(); zoomRenderer = undefined;
  void nextTick(resize);
};
const openZoom = () => {
  if (!previewMeshes.length || zoomOpen.value) return;
  zoomOpen.value = true;
  void nextTick(() => {
    if (!zoomHost.value || !camera) return;
    zoomRenderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'high-performance' });
    zoomRenderer.setPixelRatio(Math.min(devicePixelRatio || 1, 2));
    zoomRenderer.outputColorSpace = THREE.SRGBColorSpace;
    zoomHost.value.appendChild(zoomRenderer.domElement);
    zoomControls = new OrbitControls(camera, zoomRenderer.domElement);
    zoomControls.enableDamping = false;
    zoomControls.target.copy(controls?.target || new THREE.Vector3());
    zoomControls.addEventListener('change', render);
    zoomControls.update();
    window.addEventListener('keydown', onZoomKeydown, true);
    window.addEventListener('resize', resizeZoom);
    resizeZoom();
  });
};

const parseIndices = (bytes: Uint8Array, format: string): number[] => {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const use32 = /32/i.test(format) || (!/16/i.test(format) && bytes.byteLength % 4 === 0);
  const size = use32 ? 4 : 2;
  const result: number[] = [];
  for (let offset = 0; offset + size <= bytes.byteLength; offset += size) {
    result.push(use32 ? view.getUint32(offset, true) : view.getUint16(offset, true));
  }
  return result;
};

const float16 = (value: number): number => {
  const sign = (value & 0x8000) ? -1 : 1;
  const exponent = (value >> 10) & 0x1f;
  const fraction = value & 0x3ff;
  if (exponent === 0) return sign * 2 ** -14 * (fraction / 1024);
  if (exponent === 31) return fraction ? Number.NaN : sign * Number.POSITIVE_INFINITY;
  return sign * 2 ** (exponent - 15) * (1 + fraction / 1024);
};

type GameTypeElement = { SemanticName?: string; Format?: string; ExtractSlot?: string; Category?: string; ByteWidth?: string | number };
type GameTypeLayout = { slot: string; stride: number; elements: Array<GameTypeElement & { offset: number }> };
let gimiLayoutsPromise: Promise<GameTypeLayout[]> | undefined;

const formatParts = (format = '') => {
  const normalized = format.trim().toUpperCase().replace(/^DXGI_FORMAT_/, '');
  const type = normalized.includes('SNORM') ? 'snorm' : normalized.includes('UNORM') ? 'unorm'
    : normalized.includes('SINT') ? 'sint' : normalized.includes('UINT') ? 'uint' : 'float';
  return (normalized.match(/[RGBA](\d+)/g) || []).map(part => ({ bits: Number(part.slice(1)), type }));
};
const formatWidth = (element: GameTypeElement) => {
  const declared = Number.parseInt(String(element.ByteWidth ?? ''), 10);
  return declared > 0 ? declared : formatParts(element.Format).reduce((sum, part) => sum + part.bits / 8, 0);
};
const readFormatValues = (view: DataView, offset: number, format = ''): number[] | undefined => {
  const values: number[] = [];
  for (const part of formatParts(format)) {
    if (offset + part.bits / 8 > view.byteLength) return undefined;
    let value: number;
    if (part.bits === 32) {
      if (part.type === 'float') value = view.getFloat32(offset, true);
      else if (part.type === 'uint' || part.type === 'unorm') value = view.getUint32(offset, true);
      else value = view.getInt32(offset, true);
      if (part.type === 'unorm') value /= 0xffffffff;
      if (part.type === 'snorm') value = Math.max(-1, value / 0x7fffffff);
    } else if (part.bits === 16) {
      if (part.type === 'float') value = float16(view.getUint16(offset, true));
      else if (part.type === 'uint' || part.type === 'unorm') value = view.getUint16(offset, true);
      else value = view.getInt16(offset, true);
      if (part.type === 'unorm') value /= 0xffff;
      if (part.type === 'snorm') value = Math.max(-1, value / 0x7fff);
    } else if (part.bits === 8) {
      if (part.type === 'uint' || part.type === 'unorm') value = view.getUint8(offset);
      else value = view.getInt8(offset);
      if (part.type === 'unorm') value /= 0xff;
      if (part.type === 'snorm') value = Math.max(-1, value / 0x7f);
    } else return undefined;
    if (!Number.isFinite(value)) return undefined;
    values.push(value); offset += part.bits / 8;
  }
  return values;
};
const loadGimiLayouts = () => gimiLayoutsPromise ||= (async () => {
  const roots: string[] = [];
  // 数据类型随程序发布，只读安装目录那一份（与 Rust 侧 get_game_type_list 一致）。
  try {
    const resources = await resourceDir();
    roots.push(await join(resources, 'GameType', 'GIMI'), await join(resources, 'resources', 'GameType', 'GIMI'));
  } catch { /* Development paths below remain valid. */ }
  roots.push('src-tauri/resources/GameType/GIMI', 'resources/GameType/GIMI');
  for (const root of roots) {
    if (!await exists(root)) continue;
    const layouts: GameTypeLayout[] = [];
    for (const entry of await readDir(root)) {
      if (entry.isDirectory || !entry.name.toLowerCase().endsWith('.json')) continue;
      try {
        const parsed = JSON.parse(new TextDecoder().decode(await readFile(await join(root, entry.name)))) as { D3D11ElementList?: GameTypeElement[] };
        const groups = new Map<string, GameTypeElement[]>();
        for (const element of parsed.D3D11ElementList || []) {
          const key = `${element.ExtractSlot || ''}\u001f${element.Category || ''}`;
          const list = groups.get(key) || []; list.push(element); groups.set(key, list);
        }
        for (const elements of groups.values()) {
          let offset = 0;
          const mapped = elements.map(element => { const result = { ...element, offset }; offset += formatWidth(element); return result; });
          if (offset > 0) layouts.push({ slot: (elements[0]?.ExtractSlot || '').toLowerCase(), stride: offset, elements: mapped });
        }
      } catch { /* One invalid custom GameType must not disable the preview. */ }
    }
    if (layouts.length) return layouts;
  }
  return [];
})();
const semanticElements = (layout: GameTypeLayout, semantic: string) => layout.elements.filter(element =>
  (element.SemanticName || '').toUpperCase() === semantic || new RegExp(`^${semantic}\\d+$`).test((element.SemanticName || '').toUpperCase()));
const chooseLayout = (layouts: GameTypeLayout[], slot: string, stride: number, bytes: Uint8Array, semantic: string) => {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const candidates = layouts.filter(layout => layout.stride === stride && (!layout.slot || layout.slot === slot.toLowerCase()) && semanticElements(layout, semantic).length);
  const score = (layout: GameTypeLayout) => {
    const element = semanticElements(layout, semantic)[0];
    let valid = 0; let varied = 0; let previous = Number.NaN;
    const samples = Math.min(Math.floor(bytes.byteLength / stride), 256);
    for (let vertex = 0; vertex < samples; vertex += 1) {
      const values = readFormatValues(view, vertex * stride + element.offset, element.Format);
      if (values && values.every(value => Math.abs(value) < 1e7)) {
        valid += 1;
        if (!Number.isFinite(previous) || Math.abs(values[0] - previous) > 1e-7) varied += 1;
        previous = values[0];
      }
    }
    return valid * 10 + varied;
  };
  return candidates.sort((left, right) => score(right) - score(left))[0];
};

type UvLayout = { bytes: 2 | 4; offset: number; uv1Offset?: number };
const decodeUvLayout = (bytes: Uint8Array, stride: number, vertexCount: number): { uv: Float32Array; uv1: Float32Array } => {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const usesMixedUvFormats = /[\\/](?:SRMI|ZZMI)[\\/]/i.test(props.mod?.path || '');
  const candidates: UvLayout[] = usesMixedUvFormats
    ? [
        { bytes: 4, offset: 4, uv1Offset: stride >= 20 ? 12 : undefined },
        { bytes: 2, offset: 4, uv1Offset: stride >= 12 ? 8 : undefined },
        { bytes: 4, offset: 0, uv1Offset: stride >= 16 ? 8 : undefined },
      ]
    : [{ bytes: 4, offset: 4, uv1Offset: stride >= 20 ? 12 : undefined }];
  const decode = (layout: UvLayout, offset: number) => {
    const output = new Float32Array(vertexCount * 2);
    for (let vertex = 0; vertex < vertexCount; vertex += 1) {
      const at = vertex * stride + offset;
      output[vertex * 2] = layout.bytes === 4 ? view.getFloat32(at, true) : float16(view.getUint16(at, true));
      output[vertex * 2 + 1] = layout.bytes === 4 ? view.getFloat32(at + layout.bytes, true) : float16(view.getUint16(at + layout.bytes, true));
    }
    return output;
  };
  const score = (values: Float32Array) => {
    let valid = 0; let nonZero = 0; let minimum = Number.POSITIVE_INFINITY; let maximum = Number.NEGATIVE_INFINITY;
    const sampleCount = Math.min(values.length, 4096);
    for (let index = 0; index < sampleCount; index += 1) {
      const value = values[index];
      if (Number.isFinite(value) && Math.abs(value) <= 16) valid += 1;
      if (Math.abs(value) > 1e-7) nonZero += 1;
      minimum = Math.min(minimum, value); maximum = Math.max(maximum, value);
    }
    return valid / sampleCount + nonZero / sampleCount * 0.25 + (maximum - minimum > 1e-4 ? 0.25 : 0);
  };
  const decoded = candidates.filter(layout => layout.offset + layout.bytes * 2 <= stride).map(layout => ({ layout, uv: decode(layout, layout.offset) }));
  const selected = decoded.sort((left, right) => score(right.uv) - score(left.uv))[0];
  if (!selected) return { uv: new Float32Array(vertexCount * 2), uv1: new Float32Array(vertexCount * 2) };
  return { uv: selected.uv, uv1: selected.layout.uv1Offset !== undefined ? decode(selected.layout, selected.layout.uv1Offset) : selected.uv.slice() };
};

const activeDrawRanges = (group: DrawIBGroup): Array<{ count: number; start: number; base: number }> => {
  const blocks = filterActiveBlocks(group.allBlocks, variableStates.value);
  const seen = new Set<string>();
  return blocks.flatMap(block => block.drawCalls).flatMap(call => {
    if (!/^drawindexed/i.test(call.type)) return [];
    const values = call.value.split(',').map(value => Number.parseInt(value.trim(), 10));
    if (!Number.isFinite(values[0]) || values[0] < 0) return [];
    const range = { count: values[0], start: values[1] || 0, base: values[2] || 0 };
    const key = `${range.count}:${range.start}:${range.base}`;
    if (seen.has(key)) return [];
    seen.add(key);
    return [range];
  });
};

const decodeRgbaDds = (bytes: Uint8Array): { width: number; height: number; pixels: Uint8Array } => {
  if (bytes.byteLength < 128 || String.fromCharCode(...bytes.subarray(0, 4)) !== 'DDS ') throw new Error('Invalid DDS texture');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const height = view.getUint32(12, true);
  const width = view.getUint32(16, true);
  const dataOffset = view.getUint32(84, true) === 0x30315844 ? 148 : 128;
  const byteLength = width * height * 4;
  if (!width || !height || dataOffset + byteLength > bytes.byteLength) throw new Error('Incomplete DDS texture');
  return { width, height, pixels: bytes.slice(dataOffset, dataOffset + byteLength) };
};

const TEXTURE_FILE_PATTERN = /\.(?:dds|png|jpe?g|webp|bmp)$/i;
const collectTexturePaths = async (directory: string, output: string[]) => {
  let entries: Awaited<ReturnType<typeof readDir>> = [];
  try { entries = await readDir(directory); } catch { return; }
  for (const entry of entries) {
    const path = joinPath(directory, entry.name);
    if (entry.isDirectory) await collectTexturePaths(path, output);
    else if (TEXTURE_FILE_PATTERN.test(entry.name)) output.push(path);
  }
};

const discoverTextureOptions = async (result: ModIniFullAnalysis): Promise<TextureOption[]> => {
  if (!props.mod) return [];
  const byPath = new Map<string, TextureOption>();
  for (const resource of result.resources) {
    if (!resource.filename || !TEXTURE_FILE_PATTERN.test(resource.filename)) continue;
    const absolutePath = resolveResourcePath(resource.filename, resource.sourceIniPath);
    byPath.set(absolutePath.toLowerCase(), {
      id: resource.resourceId || `${resource.sourceIniPath}:${resource.sectionName}`,
      filename: resource.filename,
      label: resource.sectionName,
      absolutePath,
    });
  }
  const paths: string[] = [];
  await collectTexturePaths(props.mod.path, paths);
  for (const absolutePath of paths) {
    const key = absolutePath.toLowerCase();
    if (byPath.has(key)) continue;
    const filename = absolutePath.replace(/\\/g, '/').slice(props.mod.path.replace(/\\/g, '/').length).replace(/^\/+/, '');
    byPath.set(key, { id: `file:${key}`, filename, label: filename.split('/').pop() || filename, absolutePath });
  }
  return [...byPath.values()].sort((left, right) => left.filename.localeCompare(right.filename, undefined, { sensitivity: 'base' }));
};

const thumbnailLoading = new Set<string>();
const ensureTextureThumbnail = async (texture: TextureOption) => {
  if (textureThumbnails.value[texture.id] || thumbnailLoading.has(texture.id)) return;
  thumbnailLoading.add(texture.id);
  try {
    if (!texture.absolutePath.toLowerCase().endsWith('.dds')) {
      textureThumbnails.value[texture.id] = convertFileSrc(texture.absolutePath);
      return;
    }
    const preparedPath = await invoke<string>('prepare_dds_webgl_preview', { sourcePath: texture.absolutePath, maxDimension: 128 });
    const decoded = decodeRgbaDds(await readFile(preparedPath));
    const canvas = document.createElement('canvas');
    canvas.width = decoded.width; canvas.height = decoded.height;
    const context = canvas.getContext('2d');
    if (!context) return;
    context.putImageData(new ImageData(new Uint8ClampedArray(decoded.pixels), decoded.width, decoded.height), 0, 0);
    textureThumbnails.value[texture.id] = canvas.toDataURL('image/png');
  } catch {
    textureThumbnails.value[texture.id] = '';
  } finally {
    thumbnailLoading.delete(texture.id);
  }
};

const showTexturePicker = (group: DrawIBGroup, role: 'diffuse' | 'normal' | 'light') => {
  openTexturePicker.value = bindingKey(group, role);
  // Keep conversion bounded; four workers progressively fill the visible grid.
  let next = 0;
  const worker = async () => {
    while (next < allTextureOptions.value.length && openTexturePicker.value === bindingKey(group, role)) {
      const texture = allTextureOptions.value[next++];
      await ensureTextureThumbnail(texture);
    }
  };
  void Promise.all(Array.from({ length: 4 }, worker));
};

const configureLoadedTexture = (texture: THREE.Texture, color: boolean): THREE.Texture | undefined => {
    try {
      const image = texture.image as { width?: number; height?: number } | Array<unknown> | undefined;
      const firstMip = texture.mipmaps[0] as { width?: number; height?: number } | undefined;
      const width = !Array.isArray(image) ? (image?.width || firstMip?.width || 0) : 0;
      const height = !Array.isArray(image) ? (image?.height || firstMip?.height || 0) : 0;
      // Cube maps, invalid headers and unsupported DX10/BC formats are not
      // valid MeshStandardMaterial 2D maps. Keep the model and skip the map.
      if (!(width > 0 && height > 0)) {
        texture.dispose();
        return undefined;
      }
      texture.colorSpace = color ? THREE.SRGBColorSpace : THREE.NoColorSpace;
      texture.flipY = false;
      texture.minFilter = texture.mipmaps.length > 1 || !(texture instanceof THREE.CompressedTexture)
        ? THREE.LinearMipmapLinearFilter
        : THREE.LinearFilter;
      texture.magFilter = THREE.LinearFilter;
      texture.anisotropy = renderer?.capabilities.getMaxAnisotropy() || 1;
      texture.needsUpdate = true;
      return texture;
    } catch {
      texture.dispose();
      return undefined;
    }
};

const loadTexture = async (path: string, color = true): Promise<THREE.Texture | undefined> => {
  if (path.toLowerCase().endsWith('.dds')) {
    try {
      const preparedPath = await invoke<string>('prepare_dds_webgl_preview', { sourcePath: path, maxDimension: 4096 });
      const decoded = decodeRgbaDds(await readFile(preparedPath));
      const texture = new THREE.DataTexture(decoded.pixels, decoded.width, decoded.height, THREE.RGBAFormat, THREE.UnsignedByteType);
      texture.flipY = false;
      texture.generateMipmaps = true;
      return configureLoadedTexture(texture, color);
    } catch {
      return undefined;
    }
  }
  return new Promise(resolve => {
    new THREE.TextureLoader().load(convertFileSrc(path), texture => resolve(configureLoadedTexture(texture, color)), undefined, () => resolve(undefined));
  });
};

const loadBundledDefaults = () => bundledDefaults ||= (async () => {
  const directories: string[] = [];
  try {
    const resources = await resourceDir();
    directories.push(await join(resources, 'DefultTextures'), await join(resources, 'resources', 'DefultTextures'));
  } catch { /* Development paths below remain valid. */ }
  directories.push('src-tauri/resources/DefultTextures', 'resources/DefultTextures');
  for (const directory of directories) {
    const rampPath = await join(directory, 'RampMap.dds');
    const metalPath = await join(directory, 'MetalMap.dds');
    if (!await exists(rampPath) || !await exists(metalPath)) continue;
    return { ramp: await loadTexture(rampPath, false), metal: await loadTexture(metalPath, false) };
  }
  return {};
})();

const createShadedMaterial = async (
  diffuse: THREE.Texture,
  normal: THREE.Texture,
  light: THREE.Texture,
): Promise<THREE.Material> => {
  const owned: THREE.Texture[] = [diffuse, normal, light];
  let material: THREE.Material;
  if (shadingMode.value === 'gimi') {
    const shader = createGIMIHighFidelityMaterial(new THREE.Color(0x808080));
    // The post-process preview sends this linear-output shader through an
    // OutputPass.  Mod preview renders directly, so perform the one missing
    // output conversion here (and only on this material instance).
    shader.fragmentShader = shader.fragmentShader.replace(/}\s*$/, '  #include <colorspace_fragment>\n}');
    shaderController.attach(shader);
    shader.uniforms.uUseVertexColorAo.value = 1;
    GIMITextureSet.apply(shader, 'diffuse', diffuse);
    GIMITextureSet.apply(shader, 'normal', normal);
    GIMITextureSet.apply(shader, 'lightMap', light);
    const defaults = await loadBundledDefaults();
    if (defaults.ramp) {
      const ramp = defaults.ramp.clone(); owned.push(ramp); GIMITextureSet.apply(shader, 'rampMap', ramp);
    }
    if (defaults.metal) {
      const metal = defaults.metal.clone(); owned.push(metal); GIMITextureSet.apply(shader, 'metalMap', metal);
    }
    material = shader;
  } else if (shadingMode.value === 'pbr') {
    material = new THREE.MeshStandardMaterial({ map: diffuse, normalMap: normal, lightMap: light, lightMapIntensity: 1, roughness: 0.56, metalness: 0, side: THREE.DoubleSide });
  } else if (shadingMode.value === 'npr') {
    const gradient = new THREE.DataTexture(new Uint8Array([48, 160, 255]), 3, 1, THREE.RedFormat, THREE.UnsignedByteType);
    gradient.minFilter = THREE.NearestFilter; gradient.magFilter = THREE.NearestFilter; gradient.needsUpdate = true;
    owned.push(gradient);
    material = new THREE.MeshToonMaterial({ map: diffuse, normalMap: normal, gradientMap: gradient, side: THREE.DoubleSide });
  } else {
    material = new THREE.MeshBasicMaterial({ map: diffuse, color: 0xffffff, side: THREE.DoubleSide });
  }
  material.userData.ssmtOwnedTextures = owned;
  return material;
};

const buildGroupMesh = async (group: DrawIBGroup, token: number): Promise<THREE.Mesh | undefined> => {
  if (!props.mod) return undefined;
  try {
    const activeBlocks = filterActiveBlocks(group.allBlocks, variableStates.value);
    const activeResourceNames = new Set(activeBlocks.flatMap(block => block.replaces).map(replace => replace.resourceName.toLowerCase()));
    const activeVbs = activeResourceNames.size
      ? group.vbFiles.filter(vb => activeResourceNames.has(vb.resourceName.toLowerCase()))
      : group.vbFiles;
    const resourceMatchesDrawHash = (resource: { resourceName: string; filename: string }) => !!(
      group.drawHash && `${resource.resourceName} ${resource.filename}`.toLowerCase().includes(group.drawHash.toLowerCase())
    );
    const resourceAffinity = (resource: { resourceName: string; filename: string; sourceIniPath?: string }): number => {
      const normalize = (value: string) => value.toLowerCase()
        .replace(/^(?:textureoverride|resource)/, '')
        .replace(/(?:position|texcoord|blend|normal|index|diffuse|lightmap|materialmap|stockingmap|ib).*$/, '')
        .replace(/[^a-z0-9]/g, '');
      const targets = [group.sectionNames[0] || '', group.ibFile].map(normalize).filter(Boolean);
      const candidate = normalize(`${resource.resourceName} ${resource.filename}`);
      let common = 0;
      for (const target of targets) {
        let length = 0;
        while (length < target.length && length < candidate.length && target[length] === candidate[length]) length += 1;
        common = Math.max(common, length);
      }
      // INI namespaces are the primary ownership boundary. This prevents a
      // small unrelated VB elsewhere in a large mod pack from winning merely
      // because it happens to cover the same index range.
      return (resource.sourceIniPath === group.ibSourceIniPath ? 10000 : 0)
        + (resourceMatchesDrawHash(resource) ? 100000 : 0)
        + common * 100;
    };
    if (!group.ibFile) throw new Error('未找到可用的 IB 资源');
    const ibBytes = await readFile(resolveResourcePath(group.ibFile, group.ibSourceIniPath));
    const allIndices = parseIndices(ibBytes, group.ibFormat);
    const ranges = activeDrawRanges(group);
    const requiredVertex = (ranges.length ? ranges.flatMap(range => allIndices.slice(range.start, range.start + range.count).map(index => index + range.base)) : allIndices)
      .reduce((maximum, index) => Math.max(maximum, index), -1) + 1;
    const positionCandidates = (activeVbs.some(vb => vb.slot.toLowerCase() === 'vb0') ? activeVbs : group.vbFiles).filter(vb => vb.slot.toLowerCase() === 'vb0');
    const loadedCandidates = await Promise.all(positionCandidates.map(async vb => ({ vb, bytes: await readFile(vb.absolutePath || resolveResourcePath(vb.filename, vb.sourceIniPath)) })));
    const compatiblePositions = loadedCandidates.filter(candidate => Math.floor(candidate.bytes.byteLength / Math.max(12, Number.parseInt(candidate.vb.stride || '', 10) || 12)) >= requiredVertex);
    // SSMT generated resources carry the draw hash in the resource identity.
    // This is exporter metadata, not a fuzzy section-name similarity check.
    const selectedPosition = compatiblePositions.sort((left, right) =>
      resourceAffinity(right.vb) - resourceAffinity(left.vb)
      || Math.floor(left.bytes.byteLength / (Number.parseInt(left.vb.stride || '', 10) || 12)) - Math.floor(right.bytes.byteLength / (Number.parseInt(right.vb.stride || '', 10) || 12))
    )[0];
    const positionVb = selectedPosition?.vb;
    const vbBytes = selectedPosition?.bytes;
    if (!positionVb || !vbBytes) throw new Error('没有任何 vb0 能覆盖此 DrawIndexed 的顶点范围');
    if (token !== meshLoadToken) return;
    const stride = Math.max(12, Number.parseInt(positionVb.stride || '', 10) || 12);
    const vertexCount = Math.floor(vbBytes.byteLength / stride);
    const view = new DataView(vbBytes.buffer, vbBytes.byteOffset, vbBytes.byteLength);
    const gameTypeLayouts = /[\\/]GIMI[\\/]/i.test(props.mod.path) ? await loadGimiLayouts() : [];
    const positionLayout = chooseLayout(gameTypeLayouts, positionVb.slot, stride, vbBytes, 'POSITION');
    const positionElement = positionLayout && semanticElements(positionLayout, 'POSITION')[0];
    const positions = new Float32Array(vertexCount * 3);
    for (let vertex = 0; vertex < vertexCount; vertex += 1) {
      const offset = vertex * stride;
      const values = positionElement ? readFormatValues(view, offset + positionElement.offset, positionElement.Format) : undefined;
      positions[vertex * 3] = values?.[0] ?? view.getFloat32(offset, true);
      positions[vertex * 3 + 1] = values?.[1] ?? view.getFloat32(offset + 4, true);
      positions[vertex * 3 + 2] = values?.[2] ?? view.getFloat32(offset + 8, true);
    }
    const indices = ranges.length
      ? ranges.flatMap(range => allIndices.slice(range.start, range.start + range.count).map(index => index + range.base))
      : allIndices;
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    geometry.setIndex(indices.filter(index => index >= 0 && index < vertexCount));
    const normalVb = [...new Set([...activeVbs, ...group.vbFiles])]
      .filter(vb => /normal/i.test(`${vb.resourceName} ${vb.filename}`))
      .sort((left, right) => resourceAffinity(right) - resourceAffinity(left))[0];
    if (normalVb && normalVb !== positionVb) {
      const normalBytes = await readFile(resolveResourcePath(normalVb.filename, normalVb.sourceIniPath));
      const normalStride = Math.max(12, Number.parseInt(normalVb.stride || '', 10) || 12);
      if (Math.floor(normalBytes.byteLength / normalStride) === vertexCount) {
        const normalView = new DataView(normalBytes.buffer, normalBytes.byteOffset, normalBytes.byteLength);
        const normals = new Float32Array(vertexCount * 3);
        for (let vertex = 0; vertex < vertexCount; vertex += 1) {
          const offset = vertex * normalStride;
          normals[vertex * 3] = normalView.getFloat32(offset, true);
          normals[vertex * 3 + 1] = normalView.getFloat32(offset + 4, true);
          normals[vertex * 3 + 2] = normalView.getFloat32(offset + 8, true);
        }
        geometry.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
      }
    }
    // Decode the actual DXGI format declared by SSMT's GameType data. GIMI
    // uses both float32 categories and compact half/SNORM categories.
    const packedNormalElement = positionLayout && semanticElements(positionLayout, 'NORMAL')[0];
    if (!geometry.getAttribute('normal') && packedNormalElement) {
      const normals = new Float32Array(vertexCount * 3);
      for (let vertex = 0; vertex < vertexCount; vertex += 1) {
        const values = readFormatValues(view, vertex * stride + packedNormalElement.offset, packedNormalElement.Format);
        normals[vertex * 3] = values?.[0] ?? 0;
        normals[vertex * 3 + 1] = values?.[1] ?? 0;
        normals[vertex * 3 + 2] = values?.[2] ?? 1;
      }
      geometry.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
    }
    if (!geometry.getAttribute('normal')) geometry.computeVertexNormals();
    const fallbackTangents = new Float32Array(vertexCount * 4);
    const vertexColors = new Float32Array(vertexCount * 4);
    const packedTangentElement = positionLayout && semanticElements(positionLayout, 'TANGENT')[0];
    const packedColorElement = positionLayout && semanticElements(positionLayout, 'COLOR')[0];
    for (let vertex = 0; vertex < vertexCount; vertex += 1) {
      const tangent = packedTangentElement && readFormatValues(view, vertex * stride + packedTangentElement.offset, packedTangentElement.Format);
      fallbackTangents.set(tangent ? [tangent[0] ?? 1, tangent[1] ?? 0, tangent[2] ?? 0, tangent[3] ?? 1] : [1, 0, 0, 1], vertex * 4);
      const color = packedColorElement && readFormatValues(view, vertex * stride + packedColorElement.offset, packedColorElement.Format);
      vertexColors.set(color ? [color[0] ?? 1, color[1] ?? 1, color[2] ?? 1, color[3] ?? 1] : [1, 1, 1, 1], vertex * 4);
    }
    geometry.setAttribute('ssmtRawTangent', new THREE.BufferAttribute(fallbackTangents, 4));
    geometry.setAttribute('ssmtRawColor', new THREE.BufferAttribute(vertexColors, 4));
    const texcoordVb = [...new Set([...activeVbs, ...group.vbFiles])]
      .filter(vb => /texcoord|uv/i.test(`${vb.resourceName} ${vb.filename}`))
      .sort((left, right) => resourceAffinity(right) - resourceAffinity(left))[0];
    if (texcoordVb && texcoordVb !== positionVb) {
      const uvBytes = await readFile(resolveResourcePath(texcoordVb.filename, texcoordVb.sourceIniPath));
      const uvStride = Math.max(8, Number.parseInt(texcoordVb.stride || '', 10) || 8);
      if (Math.floor(uvBytes.byteLength / uvStride) === vertexCount) {
        const texcoordLayout = chooseLayout(gameTypeLayouts, texcoordVb.slot, uvStride, uvBytes, 'TEXCOORD');
        const texcoordElements = texcoordLayout ? semanticElements(texcoordLayout, 'TEXCOORD') : [];
        let uvs: Float32Array; let uv1: Float32Array;
        if (texcoordElements.length) {
          uvs = new Float32Array(vertexCount * 2); uv1 = new Float32Array(vertexCount * 2);
          const uvView = new DataView(uvBytes.buffer, uvBytes.byteOffset, uvBytes.byteLength);
          for (let vertex = 0; vertex < vertexCount; vertex += 1) {
            const uv = readFormatValues(uvView, vertex * uvStride + texcoordElements[0].offset, texcoordElements[0].Format);
            const lightUv = texcoordElements[1] ? readFormatValues(uvView, vertex * uvStride + texcoordElements[1].offset, texcoordElements[1].Format) : uv;
            uvs.set([uv?.[0] ?? 0, uv?.[1] ?? 0], vertex * 2);
            uv1.set([lightUv?.[0] ?? 0, lightUv?.[1] ?? 0], vertex * 2);
          }
        } else ({ uv: uvs, uv1 } = decodeUvLayout(uvBytes, uvStride, vertexCount));
        geometry.setAttribute('uv', new THREE.BufferAttribute(uvs, 2));
        const texcoordColor = texcoordLayout && semanticElements(texcoordLayout, 'COLOR')[0];
        if (texcoordColor) for (let vertex = 0; vertex < vertexCount; vertex += 1) {
          const values = readFormatValues(new DataView(uvBytes.buffer, uvBytes.byteOffset, uvBytes.byteLength), vertex * uvStride + texcoordColor.offset, texcoordColor.Format);
          vertexColors.set([values?.[0] ?? 1, values?.[1] ?? 1, values?.[2] ?? 1, values?.[3] ?? 1], vertex * 4);
        }
        geometry.getAttribute('ssmtRawColor').needsUpdate = true;
        // GIMI's 28-byte Texcoord category additionally contains TEXCOORD1
        // and TEXCOORD2 at byte offsets 12 and 20. TEXCOORD1 is the authored
        // shading/light-map UV and must not be substituted with TEXCOORD0.
        geometry.setAttribute('uv1', new THREE.BufferAttribute(uv1, 2));
      }
    }
    // Compact CPU layouts keep TEXCOORD in the same vb0 category.
    if (!geometry.getAttribute('uv') && positionLayout) {
      const texcoords = semanticElements(positionLayout, 'TEXCOORD');
      if (texcoords.length) {
        const uvs = new Float32Array(vertexCount * 2); const uv1 = new Float32Array(vertexCount * 2);
        for (let vertex = 0; vertex < vertexCount; vertex += 1) {
          const uv = readFormatValues(view, vertex * stride + texcoords[0].offset, texcoords[0].Format);
          const lightUv = texcoords[1] ? readFormatValues(view, vertex * stride + texcoords[1].offset, texcoords[1].Format) : uv;
          uvs.set([uv?.[0] ?? 0, uv?.[1] ?? 0], vertex * 2); uv1.set([lightUv?.[0] ?? 0, lightUv?.[1] ?? 0], vertex * 2);
        }
        geometry.setAttribute('uv', new THREE.BufferAttribute(uvs, 2));
        geometry.setAttribute('uv1', new THREE.BufferAttribute(uv1, 2));
      }
    }
    geometry.computeBoundingBox();
    geometry.computeBoundingSphere();
    const activeTextures = activeResourceNames.size
      ? group.textureFiles.filter(texture => activeResourceNames.has(texture.resourceName.toLowerCase()) || texture.slot.startsWith('hash:'))
      : group.textureFiles;
    const textureAffinity = (texture: DrawIBGroup['textureFiles'][number]): number => {
      const text = `${texture.resourceName} ${texture.filename}`.toLowerCase();
      let score = group.drawHash && text.includes(group.drawHash.toLowerCase()) ? 100 : 0;
      if (group.matchFirstIndex && new RegExp(`(?:^|[-_.])${group.matchFirstIndex}(?:[-_.]|$)`).test(text)) score += 1000;
      const groupStem = (group.sectionNames[0] || '').replace(/^textureoverride/i, '').toLowerCase();
      const textureStem = texture.resourceName.replace(/^resource/i, '').replace(/(?:diffuse|deffuse|normal|lightmap|materialmap|stockingmap).*$/i, '').toLowerCase();
      let common = 0;
      while (common < groupStem.length && common < textureStem.length && groupStem[common] === textureStem[common]) common += 1;
      score += common;
      return score;
    };
    const generatedTextureIdentity = (texture: DrawIBGroup['textureFiles'][number]) =>
      `${texture.resourceName} ${texture.filename}`.match(/(?:^|[\\/_.-])([0-9a-f]{8})-(\d+)-(\d+)-(?:light|normal|diffuse|stocking)/i);
    // A generated texture filename records DrawIB hash, caller index count and
    // match_first_index. Only bind it to the draw it actually describes.
    const eligibleTextures = activeTextures.filter(texture => {
      const identity = generatedTextureIdentity(texture);
      if (!identity) return true;
      return identity[1].toLowerCase() === group.drawHash?.toLowerCase()
        && Number(identity[3]) === Number(group.matchFirstIndex || 0);
    });
    const rankedTextures = [...eligibleTextures].sort((left, right) => textureAffinity(right) - textureAffinity(left));
    const chooseTexture = (role: 'diffuse' | 'normal' | 'light') => {
      const manual = submeshTextureBindings.value[bindingKey(group, role)];
      if (manual) return allTextureOptions.value.find(texture => texture.id === manual);
      const roleCandidates = rankedTextures.filter(texture => {
        const index = group.textureFiles.indexOf(texture);
        return effectiveTextureRole(group, index) === role;
      });
      const directBinding = roleCandidates.find(texture => texture.slot === `semantic-${role}`)
        || roleCandidates.find(texture => /^(?:ps|vs)-t\d+$/i.test(texture.slot));
      if (directBinding) return directBinding;
      const groupStem = (group.sectionNames[0] || '').replace(/^textureoverride/i, '').toLowerCase();
      return roleCandidates.find(texture => {
        const textureStem = texture.resourceName.replace(/^resource/i, '').replace(/(?:diffuse|deffuse|normal|lightmap|materialmap|stockingmap).*$/i, '').toLowerCase();
        return textureStem.length >= 4 && (groupStem.startsWith(textureStem) || textureStem.startsWith(groupStem));
      });
    };
    const diffuseTexture = chooseTexture('diffuse');
    let diffuseMap: THREE.Texture | undefined;
    if (diffuseTexture && geometry.getAttribute('uv')) {
      const path = 'absolutePath' in diffuseTexture ? diffuseTexture.absolutePath : resolveResourcePath(diffuseTexture.filename, diffuseTexture.sourceIniPath);
      // GIMI's shader explicitly calls srgbToLinear(mainTex.rgb). Marking the
      // texture itself as sRGB would make WebGL decode it a second time.
      diffuseMap = await loadTexture(path, shadingMode.value !== 'gimi');
    }
    const normalTexture = chooseTexture('normal');
    let normalMap: THREE.Texture | undefined;
    if (normalTexture && geometry.getAttribute('uv')) {
      const path = 'absolutePath' in normalTexture ? normalTexture.absolutePath : resolveResourcePath(normalTexture.filename, normalTexture.sourceIniPath);
      normalMap = await loadTexture(path, false);
    }
    const lightTexture = chooseTexture('light');
    let lightMap: THREE.Texture | undefined;
    if (lightTexture && geometry.getAttribute('uv1')) {
      const path = 'absolutePath' in lightTexture ? lightTexture.absolutePath : resolveResourcePath(lightTexture.filename, lightTexture.sourceIniPath);
      lightMap = await loadTexture(path, false);
    }
    diffuseMap ||= fallbackTexture([0x80, 0x80, 0x80, 0x00], shadingMode.value !== 'gimi');
    lightMap ||= fallbackTexture([0x00, 0x80, 0x00, 0x00]);
    normalMap ||= fallbackTexture([0x80, 0x80, 0xff, 0xff]);
    const material = await createShadedMaterial(diffuseMap, normalMap, lightMap);
    if (token !== meshLoadToken) {
      geometry.dispose();
      if (material instanceof THREE.ShaderMaterial) shaderController.detach(material);
      (material.userData.ssmtOwnedTextures as THREE.Texture[] | undefined)?.forEach(texture => texture.dispose());
      material.dispose();
      return;
    }
    return new THREE.Mesh(geometry, material);
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    return undefined;
  }
};

const rebuildMesh = async () => {
  const token = ++meshLoadToken;
  if (!props.mod || !scene) return;
  meshLoading.value = true;
  hidePreviewMeshes();
  error.value = '';
  try {
    const targets = selectedGroupKey.value === '__all__'
      ? groups.value
      : groups.value.filter(group => group.groupKey === selectedGroupKey.value);
    const built = await Promise.all(targets.map(async group => {
      const key = meshCacheKey(group);
      const cached = meshCache.get(key);
      if (cached) return cached;
      const mesh = await buildGroupMesh(group, token);
      if (mesh) retainMesh(key, mesh);
      return mesh;
    }));
    if (token !== meshLoadToken) return;
    previewMeshes = built.filter((mesh): mesh is THREE.Mesh => !!mesh);
    if (previewMeshes.length) error.value = '';
    for (const mesh of previewMeshes) mesh.visible = true;
    if (!previewMeshes.length) return;
    const box = new THREE.Box3();
    for (const mesh of previewMeshes) box.expandByObject(mesh);
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    if (!cameraHasInitialFrame && camera && controls && Number.isFinite(sphere.radius)) {
      controls.target.copy(sphere.center);
      camera.position.copy(sphere.center).add(new THREE.Vector3(1.1, 0.75, 1.4).normalize().multiplyScalar(Math.max(sphere.radius * 2.8, 0.01)));
      camera.near = Math.max(sphere.radius / 1000, 0.0001);
      camera.far = Math.max(sphere.radius * 1000, 10);
      camera.updateProjectionMatrix();
      controls.update();
      applyLightOrbit();
      cameraHasInitialFrame = true;
    }
    render();
  } finally {
    if (token === meshLoadToken) meshLoading.value = false;
  }
};

const preloadControlFlowMeshes = async (result: ModIniFullAnalysis, baseStates: Record<string, string>) => {
  const aliases = new Set([...namespacedVariableAliases(result).keys()].map(name => name.toLowerCase()));
  const controls = extractVariables(result).filter(variable => !aliases.has(variable.name.toLowerCase()) && variable.possibleValues.length > 0);
  const combinations = controls.reduce((count, variable) => count * Math.max(1, variable.possibleValues.length), 1);
  let statesToLoad: Record<string, string>[] = [{ ...baseStates }];
  if (combinations <= 32) {
    for (const control of controls) {
      statesToLoad = statesToLoad.flatMap(states => control.possibleValues.map(value => ({ ...states, [control.name]: value })));
    }
  } else {
    // Large multi-toggle mods can describe thousands of visual combinations,
    // while their resource branches normally vary one selector at a time.
    // Load every individual branch without constructing an exponential set.
    for (const control of controls) for (const value of control.possibleValues) {
      statesToLoad.push({ ...baseStates, [control.name]: value });
    }
  }
  const variants = new Map<string, DrawIBGroup>();
  for (const states of statesToLoad) for (const group of semanticGroups(result, states)) {
    const key = JSON.stringify({ group: group.groupKey, ib: group.ibResourceId || group.ibFile, draws: group.allBlocks.flatMap(block => block.drawCalls) });
    if (!variants.has(key)) variants.set(key, group);
  }
  const token = ++meshLoadToken;
  for (const group of variants.values()) {
    if (token !== meshLoadToken) return;
    const key = meshCacheKey(group);
    if (meshCache.has(key)) continue;
    const mesh = await buildGroupMesh(group, token);
    if (mesh) retainMesh(key, mesh);
  }
};

const applyCachedControlFlow = () => {
  if (!scene) return;
  hidePreviewMeshes();
  const targets = selectedGroupKey.value === '__all__'
    ? groups.value
    : groups.value.filter(group => group.groupKey === selectedGroupKey.value);
  for (const group of targets) {
    const mesh = meshCache.get(meshCacheKey(group));
    if (!mesh) continue;
    mesh.visible = true;
    previewMeshes.push(mesh);
  }
  render();
};

const analyze = async () => {
  if (zoomOpen.value) closeZoom();
  const token = ++analysisToken;
  ++meshLoadToken;
  meshLoading.value = false;
  disposeMesh();
  cameraHasInitialFrame = false;
  analysis.value = undefined;
  groups.value = [];
  allTextureOptions.value = [];
  textureThumbnails.value = {};
  openTexturePicker.value = '';
  selectedGroupKey.value = '';
  error.value = '';
  if (!props.mod) return;
  try {
    submeshTextureBindings.value = JSON.parse(localStorage.getItem(`ssmt4:mod-3d-submesh-textures:${props.mod.relativePath}`) || '{}');
  } catch {
    submeshTextureBindings.value = {};
  }
  loading.value = true;
  await initializeRenderer();
  try {
    const result = await analyzeModIniFilesFull(props.mod.path);
    if (token !== analysisToken) return;
    analysis.value = result;
    allTextureOptions.value = await discoverTextureOptions(result);
    if (token !== analysisToken) return;
    const states: Record<string, string> = {};
    for (const variable of extractVariables(result)) states[variable.name] = variable.initialValue || variable.possibleValues[0] || '0';
    preparingControlFlow = true;
    variableStates.value = states;
    const regularGroups = semanticGroups(result);
    groups.value = regularGroups;
    meshLoading.value = true;
    await preloadControlFlowMeshes(result, states);
    if (token !== analysisToken) return;
    preparingControlFlow = false;
    meshLoading.value = false;
    selectedGroupKey.value = groups.value.length ? '__all__' : '';
  } catch (cause) {
    preparingControlFlow = false;
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    if (token === analysisToken) {
      preparingControlFlow = false;
      loading.value = false;
    }
  }
};

watch(() => props.mod?.path, analyze, { immediate: true });
watch(selectedGroupKey, applyCachedControlFlow);
watch(shadingMode, value => {
  try { localStorage.setItem('ssmt4:mod-3d-shading-mode', value); } catch { /* Optional preference. */ }
  void rebuildMesh();
});
watch(variableStates, () => {
  if (preparingControlFlow) return;
  if (analysis.value) groups.value = semanticGroups(analysis.value);
  applyCachedControlFlow();
}, { deep: true });

onBeforeUnmount(() => {
  ++analysisToken;
  ++meshLoadToken;
  resizeObserver?.disconnect();
  controls?.dispose();
  if (zoomOpen.value) closeZoom();
  disposeMesh();
  shaderController.dispose();
  void bundledDefaults?.then(defaults => { defaults.ramp?.dispose(); defaults.metal?.dispose(); });
  renderer?.dispose();
  renderer?.domElement.remove();
});
</script>

<template>
  <section class="mod-3d-preview">
    <div class="mod-3d-preview__head">
      <strong>3D Preview</strong>
      <span v-if="mod">{{ mod.name }}</span>
    </div>
    <div ref="host" class="mod-3d-preview__viewport" @dblclick="openZoom">
      <div v-if="!mod" class="mod-3d-preview__message">选择一个 Mod 以预览</div>
      <div v-else-if="loading || meshLoading" class="mod-3d-preview__message mod-3d-preview__message--loading">
        <span class="mod-3d-preview__spinner" />
        <span>{{ loading ? '正在分析 INI 与资源…' : '加载中…' }}</span>
      </div>
      <div v-else-if="error" class="mod-3d-preview__message">{{ error }}</div>
      <div v-else-if="!groups.length" class="mod-3d-preview__message">未找到可预览的 DrawIB 组</div>
      <div class="mod-light-orb" role="slider" tabindex="0" title="光照方向" @dblclick.stop @pointerdown.stop="onLightPointerDown" @pointermove.stop="onLightPointerMove" @pointerup.stop="onLightPointerUp" @pointercancel.stop="onLightPointerUp">
        <span :style="lightIndicatorStyle" />
      </div>
      <button class="mod-preview-zoom" type="button" :disabled="!groups.length" title="大预览" @click.stop="openZoom">⤢</button>
    </div>
    <div v-if="groups.length" class="mod-3d-preview__controls">
      <label>DrawIB
        <select v-model="selectedGroupKey">
          <option value="__all__">全部 Submesh</option>
          <option v-for="group in groups" :key="group.groupKey" :value="group.groupKey">{{ group.sectionNames.find(name => /head|body|hair|dress|extra/i.test(name)) || group.sectionNames[0] }}</option>
        </select>
      </label>
      <label>着色
        <select v-model="shadingMode">
          <option value="pbr">PBR</option>
          <option value="emission">Emission</option>
          <option value="npr">NPR</option>
          <option value="gimi">GIMI</option>
        </select>
      </label>
      <label v-for="variable in variables" :key="variable.name">{{ variable.name }}
        <select v-model="variableStates[variable.name]">
          <option v-for="value in variable.possibleValues" :key="value" :value="value">{{ value }}</option>
        </select>
      </label>
      <template v-if="selectedGroup">
        <div v-for="role in (['diffuse', 'light', 'normal'] as const)" :key="`${selectedGroup.groupKey}:${role}`" class="texture-binding">
          <span>{{ role === 'diffuse' ? 'DiffuseMap' : role === 'light' ? 'LightMap' : 'NormalMap' }}</span>
          <button class="texture-binding__button" type="button" @click="showTexturePicker(selectedGroup, role)">
            <img v-if="selectedTextureOption(selectedGroup, role) && textureThumbnails[selectedTextureOption(selectedGroup, role)!.id]" :src="textureThumbnails[selectedTextureOption(selectedGroup, role)!.id]" alt="">
            <span>{{ selectedTextureOption(selectedGroup, role)?.filename || '自动识别' }}</span>
          </button>
          <div v-if="openTexturePicker === bindingKey(selectedGroup, role)" class="texture-picker">
            <button class="texture-picker__item texture-picker__item--auto" type="button" @click="setSubmeshTexture(selectedGroup, role, '')">
              <span>自动识别</span>
              <small>只使用明确语义或名称匹配</small>
            </button>
            <button v-for="texture in allTextureOptions" :key="`${role}:${texture.id}`" class="texture-picker__item" type="button" :title="texture.filename" @click="setSubmeshTexture(selectedGroup, role, texture.id)" @mouseenter="ensureTextureThumbnail(texture)">
              <img v-if="textureThumbnails[texture.id]" :src="textureThumbnails[texture.id]" alt="">
              <span v-else class="texture-picker__placeholder">DDS</span>
              <small>{{ texture.filename }}</small>
            </button>
          </div>
        </div>
      </template>
    </div>
    <Teleport to="body">
      <div v-if="zoomOpen" class="mod-preview-zoom-overlay" @click.self="closeZoom" @contextmenu.prevent>
        <div ref="zoomHost" class="mod-preview-zoom-canvas">
          <div class="mod-light-orb mod-light-orb--zoom" role="slider" tabindex="0" title="光照方向" @pointerdown.stop="onLightPointerDown" @pointermove.stop="onLightPointerMove" @pointerup.stop="onLightPointerUp" @pointercancel.stop="onLightPointerUp">
            <span :style="lightIndicatorStyle" />
          </div>
          <button class="mod-preview-zoom-close" type="button" title="关闭" @click="closeZoom">×</button>
        </div>
      </div>
    </Teleport>
  </section>
</template>

<style scoped>
.mod-3d-preview { display:flex; flex-direction:column; gap:8px; min-height:340px; padding:10px; border-top:1px solid rgba(255,255,255,.08); }
.mod-3d-preview__head { display:flex; justify-content:space-between; gap:8px; color:rgba(240,245,255,.9); font-size:12px; }
.mod-3d-preview__head span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:rgba(220,228,244,.55); }
.mod-3d-preview__viewport { position:relative; min-height:250px; flex:1; overflow:hidden; border:1px solid rgba(255,255,255,.1); border-radius:10px; background:radial-gradient(circle at 50% 35%, rgba(91,113,155,.16), rgba(7,10,17,.72)); }
.mod-3d-preview__viewport :deep(canvas) { display:block; width:100%; height:100%; }
.mod-3d-preview__message { position:absolute; inset:0; display:grid; place-items:center; padding:18px; text-align:center; color:rgba(220,228,244,.55); font-size:11px; }
.mod-3d-preview__message--loading { z-index:4; align-content:center; gap:9px; background:rgba(7,10,17,.42); backdrop-filter:blur(2px); }
.mod-3d-preview__spinner { width:20px; height:20px; border:2px solid rgba(190,210,245,.18); border-top-color:rgba(190,220,255,.9); border-radius:50%; animation:mod-preview-spin .75s linear infinite; }
@keyframes mod-preview-spin { to { transform:rotate(360deg); } }
.mod-light-orb { position:absolute; top:10px; right:10px; z-index:3; width:62px; height:62px; border:1px solid rgba(210,225,255,.35); border-radius:50%; background:radial-gradient(circle,rgba(80,99,135,.16),rgba(5,8,14,.65)); box-shadow:inset 0 0 0 7px rgba(255,255,255,.025); touch-action:none; }
.mod-light-orb span { position:absolute; top:50%; left:50%; width:11px; height:11px; margin:-5.5px; border-radius:50%; background:#ffe0aa; box-shadow:0 0 12px #ffc978; }
.mod-preview-zoom { position:absolute; right:9px; bottom:9px; z-index:3; width:28px; height:28px; padding:0; border:1px solid rgba(255,255,255,.15); border-radius:6px; background:rgba(10,15,24,.72); color:#eef4ff; cursor:pointer; }
.mod-preview-zoom:disabled { opacity:.35; cursor:default; }
.mod-3d-preview__controls { display:grid; align-content:start; grid-auto-rows:max-content; min-height:0; gap:6px; max-height:190px; overflow-x:hidden; overflow-y:auto; overscroll-behavior:contain; scrollbar-gutter:stable; }
.mod-3d-preview__controls label { display:grid; grid-template-columns:minmax(80px,1fr) minmax(0,1.35fr); align-items:center; gap:6px; color:rgba(225,232,246,.65); font-size:10px; }
.mod-3d-preview__controls select { min-width:0; height:25px; color:rgba(240,245,255,.85); border:1px solid rgba(255,255,255,.12); border-radius:6px; background:#151a25; font-size:10px; }
.texture-binding { position:relative; display:grid; grid-template-columns:minmax(80px,1fr) minmax(0,1.35fr); align-items:center; gap:6px; color:rgba(225,232,246,.65); font-size:10px; }
.texture-binding__button { display:flex; align-items:center; gap:6px; min-width:0; height:34px; padding:3px 7px; color:rgba(240,245,255,.85); border:1px solid rgba(255,255,255,.12); border-radius:6px; background:#151a25; text-align:left; }
.texture-binding__button img { width:26px; height:26px; flex:none; border-radius:4px; object-fit:cover; }
.texture-binding__button span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.texture-picker { position:relative; z-index:5; grid-column:1/-1; display:grid; grid-template-columns:repeat(auto-fill,minmax(92px,1fr)); gap:6px; max-height:260px; padding:7px; overflow:auto; border:1px solid rgba(255,255,255,.12); border-radius:8px; background:#0d111a; box-shadow:0 12px 30px rgba(0,0,0,.4); }
.texture-picker__item { display:flex; min-width:0; min-height:104px; padding:4px; flex-direction:column; gap:4px; align-items:stretch; color:rgba(235,241,252,.78); border:1px solid rgba(255,255,255,.08); border-radius:6px; background:#151a25; }
.texture-picker__item:hover { border-color:rgba(115,166,255,.65); background:#1b2332; }
.texture-picker__item img,.texture-picker__placeholder { width:100%; height:76px; border-radius:4px; object-fit:contain; background:repeating-conic-gradient(#1a2030 0 25%,#111622 0 50%) 50%/12px 12px; }
.texture-picker__placeholder { display:grid; place-items:center; color:rgba(220,230,248,.3); }
.texture-picker__item small { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:9px; }
.texture-picker__item--auto { justify-content:center; }
.texture-picker__item--auto small { white-space:normal; }
:global(.mod-preview-zoom-overlay) { position:fixed; top:var(--ssmt-content-overlay-top); right:0; bottom:0; left:var(--ssmt-content-overlay-left); z-index:2147483000; background:rgba(3,6,12,.72); backdrop-filter:blur(3px); }
:global(.mod-preview-zoom-canvas) { position:absolute; inset:18px; overflow:hidden; border:1px solid rgba(255,255,255,.12); border-radius:12px; background:radial-gradient(circle at 50% 35%,rgba(91,113,155,.16),rgba(7,10,17,.82)); }
:global(.mod-preview-zoom-canvas canvas) { display:block; width:100%; height:100%; }
:global(.mod-light-orb--zoom) { top:14px; right:14px; width:72px; height:72px; }
:global(.mod-preview-zoom-close) { position:absolute; top:14px; left:14px; z-index:4; width:32px; height:32px; border:1px solid rgba(255,255,255,.16); border-radius:7px; background:rgba(8,12,20,.72); color:#eef4ff; font-size:21px; cursor:pointer; }
</style>
