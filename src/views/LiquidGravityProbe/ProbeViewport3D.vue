<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as THREE from 'three'
import { OrbitControls } from 'three/addons/controls/OrbitControls.js'
import {
  loadProbeGeometry,
  type ProbeAnchor,
  type ProbeGeometry,
  type ProbeGroup,
  type ProbeNozzle,
} from './probeApi'

const props = defineProps<{
  serverUrl: string
  apiToken: string
  modelId: string
  nozzles: ProbeNozzle[]
  activeSlot: number
  selectedGroup?: string | null
  hiddenGroups?: string[]
  flowPreview?: boolean
}>()

const emit = defineEmits<{
  picked: [anchor: ProbeAnchor]
  loaded: [geometry: ProbeGeometry]
  groupSelected: [groupId: string]
}>()

const host = ref<HTMLDivElement>()
const loading = ref(false)
const error = ref('')
const loaded = ref(false)

let renderer: THREE.WebGLRenderer | undefined
let scene: THREE.Scene | undefined
let camera: THREE.PerspectiveCamera | undefined
let controls: OrbitControls | undefined
let gridHelper: THREE.GridHelper | undefined
let resizeObserver: ResizeObserver | undefined
let animationFrame = 0
let loadToken = 0
let pointerStart: { x: number; y: number } | null = null
let geometry: ProbeGeometry | null = null
let flowPoints: Float32Array | null = null
let flowTimes: number[] = []
let flowLifetime = 0.48
let flowBeadGeometry: THREE.BufferGeometry | null = null
let flowBeadPositions: Float32Array | null = null
let flowClock = 0
let lastFrameTime = 0

const meshes: Array<{
  group: ProbeGroup
  object: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>
  positions: Float32Array
  indices: Uint32Array
}> = []
const markerRoot = new THREE.Group()
const raycaster = new THREE.Raycaster()
const pointer = new THREE.Vector2()
const toScene = (value: ArrayLike<number>): THREE.Vector3 =>
  new THREE.Vector3(value[0], value[2], -value[1])
const fromScene = (value: THREE.Vector3): number[] => [value.x, -value.z, value.y]
const normalizedDirection = (value: ArrayLike<number>): THREE.Vector3 => {
  const direction = toScene(value)
  return direction.lengthSq() > 1e-10 ? direction.normalize() : new THREE.Vector3(0, 0, 1)
}

const add = (left: number[], right: number[]) => left.map((value, index) => value + right[index])
const subtract = (left: number[], right: number[]) => left.map((value, index) => value - right[index])
const scale = (value: number[], amount: number) => value.map(item => item * amount)
const dot = (left: number[], right: number[]) =>
  left.reduce((sum, value, index) => sum + value * right[index], 0)
const cross = (left: number[], right: number[]) => [
  left[1] * right[2] - left[2] * right[1],
  left[2] * right[0] - left[0] * right[2],
  left[0] * right[1] - left[1] * right[0],
]
const unit = (value: number[]) => {
  const length = Math.hypot(...value)
  return length > 1e-10 ? scale(value, 1 / length) : [0, 0, -1]
}
const clamp = (value: number, minimum: number, maximum: number) =>
  Math.max(minimum, Math.min(maximum, value))

function defaultFlowTrajectory(point: number[], offset: number[], direction: number[]) {
  const smooth = (start: number, end: number, value: number) => {
    const t = clamp((value - start) / (end - start), 0, 1)
    return t * t * (3 - 2 * t)
  }
  const setting = 1.69
  const lifetime = 0.48
  const dt = 1 / 240
  const opening = clamp(0.65 + 0.35 * Math.sqrt(setting), 0.8, 1.75)
  const discharge = 0.78 + 0.22 * clamp((opening - 0.7) / 0.7, 0, 1)
  const speed = 0.3807886553 * Math.sqrt(setting) * discharge / (0.78 + 0.22 * (0.3 / 0.7))
  const dir = unit(direction)
  const up = scale(dir, -1)
  const reference = Math.abs(dot(up, [0, -1, 0])) > 0.95 ? [1, 0, 0] : [0, -1, 0]
  const right = unit(cross(up, reference))
  const forward = unit(cross(right, up))
  let position = add(add(point, offset), add(scale(dir, 0.004), scale(forward, -0.001)))
  let velocity = scale(dir, speed)
  let travel = 0
  const points = [position.slice()]
  const times = [0]
  for (let time = 0; time < lifetime - 1e-9;) {
    const step = Math.min(dt, lifetime - time)
    const gravity = -9.81 * (1 - 0.45 * smooth(0.40, 1.20, speed) * (1 - smooth(0.12, 0.30, travel)))
    const next = add(position, add(scale(velocity, step), [0, 0, 0.5 * gravity * step * step]))
    velocity = add(velocity, [0, 0, gravity * step])
    travel += Math.hypot(...subtract(next, position))
    position = next
    time += step
    points.push(position.slice())
    times.push(time)
  }
  return { points, times, lifetime }
}

function initRenderer() {
  if (!host.value || renderer) return
  renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'low-power' })
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2))
  renderer.outputColorSpace = THREE.SRGBColorSpace
  renderer.domElement.className = 'probe-viewport-canvas'
  host.value.appendChild(renderer.domElement)

  scene = new THREE.Scene()
  scene.background = new THREE.Color(0x0b1116)
  scene.fog = new THREE.Fog(0x0b1116, 6, 24)
  scene.add(new THREE.HemisphereLight(0xe7f4ff, 0x203040, 2.2))
  const key = new THREE.DirectionalLight(0xffffff, 2.7)
  key.position.set(-3, 5, 4)
  scene.add(key)
  const rim = new THREE.DirectionalLight(0x73c9b2, 1.1)
  rim.position.set(4, 2, -5)
  scene.add(rim)
  scene.add(markerRoot)

  camera = new THREE.PerspectiveCamera(42, 1, 0.001, 1000)
  controls = new OrbitControls(camera, renderer.domElement)
  controls.enableDamping = true
  controls.dampingFactor = 0.08
  controls.screenSpacePanning = true
  controls.mouseButtons.LEFT = THREE.MOUSE.ROTATE
  controls.mouseButtons.RIGHT = THREE.MOUSE.PAN

  resizeObserver = new ResizeObserver(() => resize())
  resizeObserver.observe(host.value)
  renderer.domElement.addEventListener('pointerdown', onPointerDown)
  renderer.domElement.addEventListener('pointerup', onPointerUp)
  renderer.domElement.addEventListener('contextmenu', preventContextMenu)
  resize()
  animate()
}

function resize() {
  if (!host.value || !renderer || !camera) return
  const width = Math.max(1, host.value.clientWidth)
  const height = Math.max(1, host.value.clientHeight)
  renderer.setSize(width, height, false)
  camera.aspect = width / height
  camera.updateProjectionMatrix()
}

function animate() {
  animationFrame = requestAnimationFrame(animate)
  const now = performance.now()
  const delta = lastFrameTime ? Math.min((now - lastFrameTime) / 1000, 0.05) : 0
  lastFrameTime = now
  updateFlowPreview(delta)
  controls?.update()
  if (renderer && scene && camera) renderer.render(scene, camera)
}

function clearMeshes() {
  for (const item of meshes) {
    scene?.remove(item.object)
    item.object.geometry.dispose()
    item.object.material.dispose()
  }
  disposeGrid()
  meshes.splice(0)
  disposeMarkers()
}

function disposeMarkers() {
  for (const child of markerRoot.children) {
    child.traverse(object => {
      const mesh = object as THREE.Mesh
      mesh.geometry?.dispose()
      const materials = Array.isArray(mesh.material) ? mesh.material : mesh.material ? [mesh.material] : []
      for (const material of materials) material.dispose()
    })
  }
  markerRoot.clear()
  flowPoints = null
  flowTimes = []
  flowBeadGeometry = null
  flowBeadPositions = null
}

function disposeGrid() {
  if (!gridHelper) return
  scene?.remove(gridHelper)
  gridHelper.geometry.dispose()
  const materials = Array.isArray(gridHelper.material) ? gridHelper.material : [gridHelper.material]
  for (const material of materials) material.dispose()
  gridHelper = undefined
}

function buildMaterials() {
  for (const [index, item] of meshes.entries()) {
    const visible = !(props.hiddenGroups || []).includes(item.group.id)
    item.object.visible = visible
    const selected = props.selectedGroup === item.group.id
    const hue = (index * 0.61803398875) % 1
    const color = new THREE.Color().setHSL(hue, selected ? 0.32 : 0.10, selected ? 0.62 : 0.54)
    item.object.material.color.copy(color)
    item.object.material.emissive.set(selected ? 0x1b4b3f : 0x000000)
    item.object.material.emissiveIntensity = selected ? 0.45 : 0
  }
}

function buildMarkers() {
  disposeMarkers()
  const bounds = new THREE.Box3()
  for (const item of meshes) if (item.object.visible) bounds.expandByObject(item.object)
  const size = bounds.isEmpty() ? 1 : Math.max(bounds.getSize(new THREE.Vector3()).length(), 0.1)
  const radius = Math.max(size * 0.0018, 0.0003)
  for (const nozzle of props.nozzles) {
    if (!nozzle.anchor) continue
    const anchor = nozzle.anchor
    const origin = toScene([
      anchor.point[0] + (anchor.offset?.[0] || 0),
      anchor.point[1] + (anchor.offset?.[1] || 0),
      anchor.point[2] + (anchor.offset?.[2] || 0),
    ])
    const active = nozzle.slot === props.activeSlot
    const material = new THREE.MeshBasicMaterial({ color: active ? 0xffd45c : 0x58c8ff, depthTest: false })
    const sphere = new THREE.Mesh(new THREE.SphereGeometry(active ? radius * 1.15 : radius, 18, 12), material)
    sphere.position.copy(origin)
    sphere.renderOrder = 20
    markerRoot.add(sphere)
    if (active) {
      const direction = normalizedDirection(anchor.direction || [0, 0, -1])
      const arrow = new THREE.ArrowHelper(direction, origin, size * 0.075, 0xffd45c, size * 0.024, size * 0.014)
      arrow.renderOrder = 21
      markerRoot.add(arrow)
      const axes = new THREE.AxesHelper(size * 0.045)
      axes.position.copy(origin)
      axes.renderOrder = 21
      markerRoot.add(axes)
    }
  }
  buildFlowPreview(size)
}

function buildFlowPreview(size: number) {
  const nozzle = props.nozzles.find(item => item.slot === props.activeSlot)
  const anchor = nozzle?.anchor
  if (!props.flowPreview || !anchor) return
  const trajectory = defaultFlowTrajectory(
    anchor.point,
    anchor.offset || [0, 0, 0],
    anchor.direction || [0, 0, -1],
  )
  flowLifetime = trajectory.lifetime
  flowTimes = trajectory.times
  const linePositions = new Float32Array(trajectory.points.length * 3)
  trajectory.points.forEach((point, index) => {
    const mapped = toScene(point)
    linePositions[index * 3] = mapped.x
    linePositions[index * 3 + 1] = mapped.y
    linePositions[index * 3 + 2] = mapped.z
  })
  flowPoints = linePositions
  const lineGeometry = new THREE.BufferGeometry()
  lineGeometry.setAttribute('position', new THREE.BufferAttribute(linePositions, 3))
  const line = new THREE.Line(
    lineGeometry,
    new THREE.LineBasicMaterial({ color: 0x4de8ff, transparent: true, opacity: 0.9, depthWrite: false }),
  )
  line.renderOrder = 18
  markerRoot.add(line)

  flowBeadPositions = new Float32Array(24 * 3)
  flowBeadGeometry = new THREE.BufferGeometry()
  flowBeadGeometry.setAttribute('position', new THREE.BufferAttribute(flowBeadPositions, 3))
  const beads = new THREE.Points(
    flowBeadGeometry,
    new THREE.PointsMaterial({
      color: 0xffef69,
      size: Math.max(size * 0.008, 0.0015),
      sizeAttenuation: true,
      transparent: true,
      opacity: 0.95,
      depthWrite: false,
    }),
  )
  beads.renderOrder = 19
  markerRoot.add(beads)
}

function updateFlowPreview(delta: number) {
  if (!props.flowPreview || !flowBeadGeometry || !flowBeadPositions || !flowPoints || flowTimes.length < 2) return
  flowClock = (flowClock + delta * 0.10) % flowLifetime
  const count = flowBeadPositions.length / 3
  for (let index = 0; index < count; index++) {
    const age = (flowClock + index * flowLifetime / count) % flowLifetime
    const sample = Math.min(Math.floor(age / (1 / 240)), flowTimes.length - 2)
    const span = flowTimes[sample + 1] - flowTimes[sample]
    const fraction = span > 0 ? clamp((age - flowTimes[sample]) / span, 0, 1) : 0
    const offset = sample * 3
    flowBeadPositions[index * 3] = flowPoints[offset] + (flowPoints[offset + 3] - flowPoints[offset]) * fraction
    flowBeadPositions[index * 3 + 1] = flowPoints[offset + 1] + (flowPoints[offset + 4] - flowPoints[offset + 1]) * fraction
    flowBeadPositions[index * 3 + 2] = flowPoints[offset + 2] + (flowPoints[offset + 5] - flowPoints[offset + 2]) * fraction
  }
  flowBeadGeometry.getAttribute('position').needsUpdate = true
}

function fit() {
  if (!camera || !controls) return
  const box = new THREE.Box3()
  for (const item of meshes) if (item.object.visible) box.expandByObject(item.object)
  if (box.isEmpty()) return
  const sphere = box.getBoundingSphere(new THREE.Sphere())
  const distance = Math.max(sphere.radius * 2.25, 0.2)
  controls.target.copy(sphere.center)
  camera.position.copy(sphere.center).add(new THREE.Vector3(0, sphere.radius * 0.08, distance))
  camera.near = Math.max(distance / 2000, 0.0001)
  camera.far = Math.max(distance * 100, 100)
  camera.updateProjectionMatrix()
  controls.update()
  const gridSize = Math.max(sphere.radius * 8, 1)
  const grid = new THREE.GridHelper(gridSize, 20, 0x29404a, 0x1a2830)
  grid.position.set(sphere.center.x, box.min.y, sphere.center.z)
  disposeGrid()
  gridHelper = grid
  scene?.add(grid)
}

async function loadModel() {
  if (!props.serverUrl || !props.apiToken || !props.modelId) return
  initRenderer()
  const token = ++loadToken
  loading.value = true
  loaded.value = false
  error.value = ''
  clearMeshes()
  try {
    const next = await loadProbeGeometry(props.serverUrl, props.apiToken, props.modelId)
    if (token !== loadToken) return
    geometry = next
    const scenePositions = next.positions.map(source => {
      const mapped = new Float32Array(source.length)
      for (let offset = 0; offset + 2 < source.length; offset += 3) {
        mapped[offset] = source[offset]
        mapped[offset + 1] = source[offset + 2]
        mapped[offset + 2] = -source[offset + 1]
      }
      return mapped
    })
    for (const [index, group] of next.groups.entries()) {
      const positions = scenePositions[group.mesh]
      const indices = next.indices[index]
      const buffer = new THREE.BufferGeometry()
      buffer.setAttribute('position', new THREE.BufferAttribute(positions, 3))
      buffer.setIndex(new THREE.BufferAttribute(indices, 1))
      buffer.computeVertexNormals()
      const material = new THREE.MeshStandardMaterial({
        color: 0xaeb9c3,
        roughness: 0.72,
        metalness: 0.02,
        side: THREE.DoubleSide,
        transparent: false,
      })
      const object = new THREE.Mesh(buffer, material)
      object.userData.probeGroup = group
      scene?.add(object)
      meshes.push({ group, object, positions, indices })
    }
    buildMaterials()
    buildMarkers()
    fit()
    loaded.value = meshes.length > 0
    emit('loaded', next)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (token === loadToken) loading.value = false
  }
}

function pointerPosition(event: PointerEvent) {
  if (!renderer) return
  const rect = renderer.domElement.getBoundingClientRect()
  pointer.set(
    ((event.clientX - rect.left) / rect.width) * 2 - 1,
    -((event.clientY - rect.top) / rect.height) * 2 + 1,
  )
}

function onPointerDown(event: PointerEvent) {
  if (event.button === 0) pointerStart = { x: event.clientX, y: event.clientY }
}

function onPointerUp(event: PointerEvent) {
  if (!pointerStart || event.button !== 0) return
  const moved = Math.hypot(event.clientX - pointerStart.x, event.clientY - pointerStart.y)
  pointerStart = null
  if (moved > 4 || !camera) return
  pointerPosition(event)
  raycaster.setFromCamera(pointer, camera)
  const visibleMeshes = meshes
    .filter(item => item.object.visible)
    .map(item => item.object)
  const intersections = raycaster.intersectObjects(visibleMeshes, false)
  const hit = intersections.find(item => item.faceIndex !== undefined)
  if (!hit || hit.faceIndex === undefined) return
  const item = meshes.find(entry => entry.object === hit.object)
  if (!item) return
  const face = hit.faceIndex
  if (face === null) return
  const indexOffset = face * 3
  const ids = [
    item.indices[indexOffset],
    item.indices[indexOffset + 1],
    item.indices[indexOffset + 2],
  ]
  const a = new THREE.Vector3().fromBufferAttribute(item.object.geometry.getAttribute('position'), ids[0])
  const b = new THREE.Vector3().fromBufferAttribute(item.object.geometry.getAttribute('position'), ids[1])
  const c = new THREE.Vector3().fromBufferAttribute(item.object.geometry.getAttribute('position'), ids[2])
  const point = hit.point.clone()
  const v0 = b.clone().sub(a)
  const v1 = c.clone().sub(a)
  const v2 = point.clone().sub(a)
  const d00 = v0.dot(v0)
  const d01 = v0.dot(v1)
  const d11 = v1.dot(v1)
  const d20 = v2.dot(v0)
  const d21 = v2.dot(v1)
  const denominator = d00 * d11 - d01 * d01
  if (!denominator) return
  const v = (d11 * d20 - d01 * d21) / denominator
  const w = (d00 * d21 - d01 * d20) / denominator
  const u = 1 - v - w
  emit('picked', {
    model: geometry?.id,
    group: item.group.id,
    triangle: face,
    barycentric: [u, v, w],
    point: fromScene(point),
    offset: [0, 0, 0],
    direction: [0, 0, -1],
  })
  emit('groupSelected', item.group.id)
}

function preventContextMenu(event: Event) {
  event.preventDefault()
}

watch(() => [props.serverUrl, props.apiToken, props.modelId], () => void loadModel())
watch(() => [props.nozzles, props.activeSlot], () => buildMarkers(), { deep: true })
watch(() => [props.selectedGroup, props.hiddenGroups, props.flowPreview], () => {
  buildMaterials()
  buildMarkers()
}, { deep: true })

onMounted(async () => {
  await nextTick()
  initRenderer()
  if (props.modelId) await loadModel()
})

onBeforeUnmount(() => {
  ++loadToken
  cancelAnimationFrame(animationFrame)
  resizeObserver?.disconnect()
  if (renderer) {
    renderer.domElement.removeEventListener('pointerdown', onPointerDown)
    renderer.domElement.removeEventListener('pointerup', onPointerUp)
    renderer.domElement.removeEventListener('contextmenu', preventContextMenu)
  }
  controls?.dispose()
  clearMeshes()
  renderer?.dispose()
  renderer?.domElement.remove()
})

defineExpose({ fit })
</script>

<template>
  <div ref="host" class="probe-viewport">
    <div v-if="loading" class="probe-viewport-message">
      <span class="probe-viewport-spinner" />
      <span>正在加载 Three.js 模型…</span>
    </div>
    <div v-else-if="error" class="probe-viewport-message is-error">{{ error }}</div>
    <div v-else-if="!loaded" class="probe-viewport-message">选择一个可编辑模型</div>
    <div v-if="loaded" class="probe-viewport-hint">左键旋转 · 右键平移 · 滚轮缩放 · 单击选择表面 · H 隐藏/显示选中部件</div>
    <div v-if="loaded" class="probe-viewport-actions">
      <button type="button" title="重新对焦模型" @click="fit">对焦</button>
    </div>
  </div>
</template>

<style scoped>
.probe-viewport { position:relative; width:100%; height:100%; min-height:0; overflow:hidden; border:1px solid rgba(255,255,255,.09); border-radius:8px; background:#0b1116; }
.probe-viewport :deep(.probe-viewport-canvas) { display:block; width:100%; height:100%; }
.probe-viewport-message { position:absolute; inset:0; z-index:4; display:grid; place-content:center; justify-items:center; gap:10px; padding:24px; background:rgba(8,13,18,.9); color:#bccbd1; font-size:12px; text-align:center; }
.probe-viewport-message.is-error { color:#f1a09a; }
.probe-viewport-spinner { width:22px; height:22px; border:2px solid rgba(255,255,255,.14); border-top-color:rgb(var(--ssmt-brand-accent-rgb,216,233,78)); border-radius:50%; animation:probe-spin .8s linear infinite; }
.probe-viewport-hint { position:absolute; left:12px; right:58px; bottom:10px; z-index:3; overflow:hidden; color:rgba(235,243,246,.76); font-size:11px; text-overflow:ellipsis; white-space:nowrap; pointer-events:none; }
.probe-viewport-actions { position:absolute; top:10px; right:10px; z-index:3; }
.probe-viewport-actions button { height:30px; padding:0 10px; border:1px solid rgba(255,255,255,.16); border-radius:6px; background:rgba(10,15,20,.82); color:#e7f0f2; font-size:12px; cursor:pointer; }
@keyframes probe-spin { to { transform:rotate(360deg); } }
</style>
