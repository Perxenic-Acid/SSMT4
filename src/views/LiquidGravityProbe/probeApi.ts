export interface ProbeCatalogModel {
  id: string
  name: string
  groups: number
  status: string
  errors: string[]
}

export interface ProbeCatalog {
  root: string
  mods: ProbeCatalogModel[]
}

export interface ProbeMesh {
  offset: number
  length: number
  vertices: number
}

export interface ProbeGroup {
  id: string
  label: string
  section: string
  count: number
  start: number
  base: number
  defaultVisible: boolean
  conditional: boolean
  mesh: number
  offset: number
  length: number
  ini: string
  vertexFile: string
}

export interface ProbeAnchor {
  model?: string
  group: string
  triangle: number
  barycentric: number[]
  point: number[]
  offset: number[]
  direction: number[]
  sourceProfile?: number | null
}

export interface ProbeNozzle {
  slot: number
  anchor: ProbeAnchor | null
  sourceProfile?: number | null
  intervalSeconds?: number | null
  pendingApply?: boolean
}

export interface ProbeEffectField {
  key: string
  label: string
  kind: 'toggle' | 'select' | 'number'
  group: string
  default: number
  min: number
  max: number
  step: number
  hidden?: boolean
  options?: Array<{ label: string; value: string | number }>
}

export interface ProbeEffectSettings {
  values: Record<string, number>
  fields: ProbeEffectField[]
}

export interface ProbeKeyBinding {
  key: string
  ctrl: boolean
  alt: boolean
  shift: boolean
  binding: string
  display: string
}

export interface ProbeCleanupPreview {
  preview: true
  hooks: number
  hook_files: number
  profile_files: number
  generated_files: number
  delete_files: number
  delete_directories: number
  legacy_profile_folder: boolean
}

export interface ProbeGeometry {
  id: string
  name: string
  meshes: ProbeMesh[]
  groups: ProbeGroup[]
  errors: Array<{ group: string; error: string }>
  timeAxis?: Record<string, unknown> | null
  anchor: ProbeAnchor | null
  nozzles: ProbeNozzle[]
  effectColor: number[]
  effectStrength: number
  integration: {
    configured: boolean
    installed: boolean
    hooks: number
    expected: number
    errors?: string[]
  }
  coordinateSystem: string
  positions: Float32Array[]
  indices: Uint32Array[]
}

const request = async <T>(url: string, token: string, init: RequestInit = {}): Promise<T> => {
  const headers = new Headers(init.headers)
  headers.set('X-Editor-Token', token)
  const response = await fetch(url, { ...init, headers })
  const body = await response.json().catch(() => ({}))
  if (!response.ok) throw new Error(body?.error || `HTTP ${response.status}`)
  return body as T
}

export const loadProbeCatalog = (serverUrl: string, token: string) =>
  request<ProbeCatalog>(`${serverUrl.replace(/\/$/, '')}/api/catalog`, token)

export const loadProbeGeometry = async (serverUrl: string, token: string, modelId: string): Promise<ProbeGeometry> => {
  const response = await fetch(
    `${serverUrl.replace(/\/$/, '')}/api/geometry?id=${encodeURIComponent(modelId)}`,
    { headers: { 'X-Editor-Token': token } },
  )
  if (!response.ok) {
    const body = await response.json().catch(() => ({}))
    throw new Error(body?.error || `HTTP ${response.status}`)
  }
  const bytes = await response.arrayBuffer()
  const headerLength = new DataView(bytes).getUint32(0, true)
  const info = JSON.parse(
    new TextDecoder().decode(new Uint8Array(bytes, 4, headerLength)),
  ) as Omit<ProbeGeometry, 'positions' | 'indices'>
  const start = 4 + headerLength
  const positions = info.meshes.map(mesh =>
    new Float32Array(bytes.slice(start + mesh.offset, start + mesh.offset + mesh.length))
  )
  const indices = info.groups.map(group =>
    new Uint32Array(bytes.slice(start + group.offset, start + group.offset + group.length))
  )
  return { ...info, positions, indices }
}

export const saveProbeNozzles = (
  serverUrl: string,
  token: string,
  model: string,
  nozzles: Array<ProbeAnchor & { slot: number; sourceProfile?: number | null; intervalSeconds?: number | null }>,
) => request<{ nozzles: number; path: string }>(
  `${serverUrl.replace(/\/$/, '')}/api/save-nozzles`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model, nozzles }),
  },
)

export const applyProbeNozzles = (
  serverUrl: string,
  token: string,
  model: string,
  nozzles: Array<ProbeAnchor & { slot: number; intervalSeconds?: number | null }>,
) => request<{ nozzle_count: number; profiles: number[] }>(
  `${serverUrl.replace(/\/$/, '')}/api/apply-nozzles`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model, nozzles }),
  },
)

export const saveProbeModelEffect = (
  serverUrl: string,
  token: string,
  model: string,
  color: number[],
  strength: number,
) => request<{ color: number[]; strength: number; path: string }>(
  `${serverUrl.replace(/\/$/, '')}/api/model-effect`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model, color, strength }),
  },
)

export const reinjectProbeModel = (
  serverUrl: string,
  token: string,
  model: string,
) => request<{ nozzle_count: number; profiles: number[] }>(
  `${serverUrl.replace(/\/$/, '')}/api/reinject-model`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model }),
  },
)

export const removeProbeModelEffect = (
  serverUrl: string,
  token: string,
  model: string,
) => request<{ nozzle_count: number }>(
  `${serverUrl.replace(/\/$/, '')}/api/remove-model`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ model }),
  },
)

export const loadProbeEffects = (serverUrl: string, token: string) =>
  request<ProbeEffectSettings>(`${serverUrl.replace(/\/$/, '')}/api/effects`, token)

export const saveProbeEffects = (
  serverUrl: string,
  token: string,
  values: Record<string, number>,
) => request<ProbeEffectSettings>(
  `${serverUrl.replace(/\/$/, '')}/api/effects`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ values }),
  },
)

export const loadProbeKeyBinding = (serverUrl: string, token: string) =>
  request<ProbeKeyBinding>(`${serverUrl.replace(/\/$/, '')}/api/keybind`, token)

export const saveProbeKeyBinding = (
  serverUrl: string,
  token: string,
  binding: ProbeKeyBinding,
) => request<ProbeKeyBinding>(
  `${serverUrl.replace(/\/$/, '')}/api/keybind`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(binding),
  },
)

export const previewProbeCleanup = (
  serverUrl: string,
  token: string,
) => request<ProbeCleanupPreview>(
  `${serverUrl.replace(/\/$/, '')}/api/cleanup`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ preview: true }),
  },
)

export const applyProbeCleanup = (
  serverUrl: string,
  token: string,
) => request<Record<string, unknown>>(
  `${serverUrl.replace(/\/$/, '')}/api/cleanup`,
  token,
  {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ confirm: 'DELETE_ALL_PROBE_DATA' }),
  },
)
