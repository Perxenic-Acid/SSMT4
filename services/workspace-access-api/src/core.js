export const SUPPORTED_GAME_PRESETS = new Set([
  'GIMI', 'HIMI', 'SRMI', 'ZZMI', 'ZZMIDX12', 'WWMI', 'EFMI', 'NTEMI',
  'GF2', 'IdentityV', 'AILIMIT', 'DOAV', 'SnowBreak', 'YYSLS', 'APMI',
  'Naraka', 'NarakaM',
]);

export const ERROR_CODES = Object.freeze({
  INVALID_METADATA: 'INVALID_METADATA',
  UNSUPPORTED_GAME_PRESET: 'UNSUPPORTED_GAME_PRESET',
  INVALID_WORKSPACE_NAME: 'INVALID_WORKSPACE_NAME',
  RATE_LIMIT_EXCEEDED: 'RATE_LIMIT_EXCEEDED',
  DAILY_BYTE_QUOTA_EXCEEDED: 'DAILY_BYTE_QUOTA_EXCEEDED',
  ARCHIVE_TOO_LARGE: 'ARCHIVE_TOO_LARGE',
  IDEMPOTENCY_KEY_REQUIRED: 'IDEMPOTENCY_KEY_REQUIRED',
  GITHUB_PUBLISH_FAILED: 'GITHUB_PUBLISH_FAILED',
  INTERNAL_ERROR: 'INTERNAL_ERROR',
});

export function apiError(code, status = 400, detail = undefined) {
  return new Response(JSON.stringify({ error: { code, ...(detail ? { detail } : {}) } }), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
  });
}

export function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
  });
}

export function normalizeMetadata(input, now) {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return { error: ERROR_CODES.INVALID_METADATA };
  const schemaVersion = input.schemaVersion;
  const gamePreset = typeof input.gamePreset === 'string' ? input.gamePreset.trim() : '';
  const workspaceName = typeof input.workspaceName === 'string' ? input.workspaceName.trim() : '';
  if (schemaVersion !== 1) return { error: ERROR_CODES.INVALID_METADATA };
  if (!SUPPORTED_GAME_PRESETS.has(gamePreset)) return { error: ERROR_CODES.UNSUPPORTED_GAME_PRESET };
  if (!workspaceName || [...workspaceName].length > 128 || [...workspaceName].some((char) => /[\u0000-\u001f\u007f]/u.test(char))) {
    return { error: ERROR_CODES.INVALID_WORKSPACE_NAME };
  }
  if (!Array.isArray(input.lods) || input.lods.length === 0 || input.lods.length > 128) return { error: ERROR_CODES.INVALID_METADATA };
  const capturedAt = normalizeTimestamp(input.capturedAt);
  if (input.capturedAt != null && !capturedAt) return { error: ERROR_CODES.INVALID_METADATA };
  const attribution = normalizeAttribution(input.attribution);
  if (!attribution) return { error: ERROR_CODES.INVALID_METADATA };
  const supersedes = normalizeSupersedes(input.supersedes);
  if (input.supersedes != null && !supersedes) return { error: ERROR_CODES.INVALID_METADATA };
  const workspaceAliases = normalizeStringList(input.workspaceAliases, 128, 128);
  if (!workspaceAliases) return { error: ERROR_CODES.INVALID_METADATA };

  const lods = [];
  const seenLods = new Set();
  for (const lod of input.lods) {
    if (!lod || typeof lod !== 'object' || typeof lod.name !== 'string') return { error: ERROR_CODES.INVALID_METADATA };
    const name = lod.name.trim();
    if (!name || name.length > 128 || seenLods.has(name.normalize('NFKC').toLocaleLowerCase())) return { error: ERROR_CODES.INVALID_METADATA };
    seenLods.add(name.normalize('NFKC').toLocaleLowerCase());
    // Rust's historic `rename_all = camelCase` serialization emitted drawIb/skipIb.
    // Accept those drafts, but always publish the protocol's canonical ...IB spelling.
    const drawIB = normalizeHashAliases(lod.drawIB ?? lod.drawIb, 8, 16);
    const skipIB = normalizeSkipIB(lod.skipIB ?? lod.skipIb);
    const vsCheck = normalizeVS(lod.vsCheck);
    const componentAliases = normalizeComponentAliases(lod.componentAliases);
    if (!drawIB || !skipIB || !vsCheck || !componentAliases || drawIB.length === 0) return { error: ERROR_CODES.INVALID_METADATA };
    lods.push({ name, drawIB, skipIB, vsCheck, ...(Object.keys(componentAliases).length ? { componentAliases } : {}) });
  }
  return {
    value: {
      schemaVersion: 1,
      gamePreset,
      workspaceName,
      description: normalizeDescription(input.description),
      uploadedAt: now,
      capturedAt,
      gameBuild: typeof input.gameBuild === 'string' ? input.gameBuild.trim().slice(0, 256) || null : null,
      attribution,
      supersedes,
      workspaceAliases,
      generator: normalizeGenerator(input.generator),
      lods,
      fullData: normalizeFullData(input.fullData),
    },
  };
}

function normalizeComponentAliases(value) {
  if (value === undefined) return {};
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const entries = Object.entries(value);
  if (entries.length > 4096) return null;
  const result = [];
  for (const [key, alias] of entries) {
    // 键是 LOD 内的提取目录标识，别名只作为文本保存，不能变成路径或 Component 编号。
    if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/u.test(key) || typeof alias !== 'string' || [...alias].length > 128 || /[\u0000-\u001f\u007f]/u.test(alias)) return null;
    if (alias.trim()) result.push([key, alias.trim()]);
  }
  return Object.fromEntries(result);
}

function normalizeStringList(value, maxItems, maxLength) {
  if (value == null) return [];
  if (!Array.isArray(value) || value.length > maxItems) return null;
  const seen = new Set();
  const result = [];
  for (const item of value) {
    const normalized = typeof item === 'string' ? item.trim() : '';
    if (!normalized || normalized.length > maxLength || [...normalized].some((char) => /[\u0000-\u001f\u007f]/u.test(char))) return null;
    const key = normalized.normalize('NFKC').toLocaleLowerCase();
    if (!seen.has(key)) { seen.add(key); result.push(normalized); }
  }
  return result;
}

function normalizeHashAliases(value, min, max) {
  if (!Array.isArray(value) || value.length > 4096) return null;
  const result = [];
  for (const item of value) {
    if (!item || typeof item.hash !== 'string' || !isHash(item.hash, min, max)) return null;
    result.push({ hash: item.hash.trim().toLowerCase(), alias: typeof item.alias === 'string' ? item.alias.trim().slice(0, 256) : '' });
  }
  return result;
}

function normalizeSkipIB(value) {
  if (!Array.isArray(value) || value.length > 4096) return null;
  return value.map((item) => {
    if (!item || typeof item.hash !== 'string' || !isHash(item.hash, 8, 16)) return null;
    if (!Number.isSafeInteger(item.indexCount) || item.indexCount < 0 || !Number.isSafeInteger(item.firstIndex) || item.firstIndex < 0) return null;
    return { hash: item.hash.trim().toLowerCase(), alias: typeof item.alias === 'string' ? item.alias.trim().slice(0, 256) : '', indexCount: item.indexCount, firstIndex: item.firstIndex };
  }).every(Boolean) ? value.map((item) => ({ hash: item.hash.trim().toLowerCase(), alias: typeof item.alias === 'string' ? item.alias.trim().slice(0, 256) : '', indexCount: item.indexCount, firstIndex: item.firstIndex })) : null;
}

function normalizeVS(value) {
  if (!Array.isArray(value) || value.length > 4096) return null;
  return value.map((item) => {
    if (!item || typeof item.hash !== 'string' || !isHash(item.hash, 8, 64)) return null;
    return { enabled: item.enabled !== false, hash: item.hash.trim().toLowerCase() };
  }).every(Boolean) ? value.map((item) => ({ enabled: item.enabled !== false, hash: item.hash.trim().toLowerCase() })) : null;
}

function normalizeAttribution(value) {
  if (!value || typeof value !== 'object' || (value.mode !== 'anonymous' && value.mode !== 'custom')) return null;
  if (value.mode === 'anonymous') return { mode: 'anonymous' };
  const displayName = typeof value.displayName === 'string' ? value.displayName.trim().slice(0, 128) : '';
  return displayName ? { mode: 'custom', displayName } : null;
}

function normalizeTimestamp(value) {
  if (value == null) return null;
  if (typeof value !== 'string' || value.length > 64) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toISOString() : null;
}

function normalizeSupersedes(value) {
  if (value == null) return null;
  const normalized = typeof value === 'string' ? value.trim().toLowerCase() : '';
  return /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(normalized) ? normalized : null;
}

function normalizeGenerator(value) {
  if (!value || typeof value !== 'object') return { name: 'SSMT', version: 'unknown', validatorVersion: 1 };
  return { name: typeof value.name === 'string' ? value.name.slice(0, 64) : 'SSMT', version: typeof value.version === 'string' ? value.version.slice(0, 64) : 'unknown', validatorVersion: 1 };
}

function normalizeFullData(value) {
  if (!value || value.available !== true) return { available: false };
  if (typeof value.sha256 !== 'string' || !/^[0-9a-f]{64}$/iu.test(value.sha256) || !Number.isSafeInteger(value.size) || value.size < 0) return { available: false };
  return { available: true, archiveVersion: 1, objectKey: null, sha256: value.sha256.toLowerCase(), size: value.size, uncompressedSize: Number.isSafeInteger(value.uncompressedSize) && value.uncompressedSize >= 0 ? value.uncompressedSize : 0, fileCount: Number.isSafeInteger(value.fileCount) && value.fileCount >= 0 ? value.fileCount : 0 };
}

function normalizeDescription(value) {
  return typeof value === 'string' ? value.trim().slice(0, 2000) || null : null;
}

function isHash(value, min, max) {
  return new RegExp(`^[0-9a-f]{${min},${max}}$`, 'iu').test(value.trim());
}

export function createStatus(entryId, metadata, reviewThreshold = 209715200) {
  return {
    schemaVersion: 1,
    entryId,
    availability: 'active',
    moderation: { reviewRequired: metadata.fullData.available && metadata.fullData.size > reviewThreshold, reviewState: metadata.fullData.available && metadata.fullData.size > reviewThreshold ? 'pending' : 'not_required', reviewedAt: null },
    warning: { markedAt: null, reason: null },
    metadataDownloadCount: 0,
    fullPackageDownloadCount: 0,
    deleteAfter: null,
    replacementEntryId: null,
  };
}

export function createUuidV7(now = Date.now()) {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  const timestamp = BigInt(now);
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = Number(timestamp >> BigInt((5 - index) * 8) & 0xffn);
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function attachServerEntryId(metadata, entryId) {
  return { ...metadata, entryId };
}
