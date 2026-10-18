const MAX_HEADER_RANGE = 64 * 1024;
const MAX_MANIFEST_SIZE = 16 * 1024 * 1024;
const MAX_DIRECTORY_RANGE = 16 * 1024 * 1024;
const MAX_UNCOMPRESSED_SIZE = 2 * 1024 * 1024 * 1024;
const DEFAULT_MAX_SERVER_SHA256_VALIDATION_BYTES = 32 * 1024 * 1024;

export async function validateR2Archive(env, objectKey, metadata) {
  if (!env.WORKSPACE_BUCKET || typeof env.WORKSPACE_BUCKET.get !== 'function') throw new Error('R2_VALIDATION_UNAVAILABLE');
  const object = await env.WORKSPACE_BUCKET.get(objectKey, { range: { offset: 0, length: MAX_HEADER_RANGE } });
  if (!object) throw new Error('R2_OBJECT_NOT_FOUND');
  let bytes = new Uint8Array(await object.arrayBuffer());
  let manifest;
  try {
    manifest = parseStoredManifest(bytes);
  } catch (error) {
    if (error?.message !== 'ARCHIVE_MANIFEST_RANGE_REQUIRED') throw error;
    const requiredLength = manifestRequiredLength(bytes);
    if (requiredLength > MAX_MANIFEST_SIZE) throw new Error('ARCHIVE_MANIFEST_INVALID');
    const expanded = await env.WORKSPACE_BUCKET.get(objectKey, { range: { offset: 0, length: requiredLength } });
    if (!expanded) throw new Error('R2_OBJECT_NOT_FOUND');
    bytes = new Uint8Array(await expanded.arrayBuffer());
    manifest = parseStoredManifest(bytes);
  }
  if (manifest.archiveVersion !== 1 || manifest.portableMetadataPath !== 'portable-workspace.json') throw new Error('ARCHIVE_MANIFEST_INVALID');
  if (!Array.isArray(manifest.files) || manifest.fileCount !== manifest.files.length || manifest.fileCount > 100000 || manifest.totalUncompressedSize > MAX_UNCOMPRESSED_SIZE) throw new Error('ARCHIVE_MANIFEST_INVALID');
  if (metadata.fullData.uncompressedSize !== manifest.totalUncompressedSize || metadata.fullData.fileCount !== manifest.fileCount) throw new Error('ARCHIVE_MANIFEST_MISMATCH');
  const paths = new Set();
  for (const file of manifest.files) {
    if (!file || typeof file.path !== 'string' || !file.path.startsWith('payload/') || paths.has(file.path.toUpperCase()) || !safeArchivePath(file.path) || typeof file.role !== 'string' || file.role.length > 64 || !Number.isSafeInteger(file.size) || file.size < 0 || !/^[0-9a-f]{64}$/u.test(file.sha256)) throw new Error('ARCHIVE_MANIFEST_INVALID');
    paths.add(file.path.toUpperCase());
  }
  await validateCentralDirectory(env, objectKey, manifest);
  // A complete JS SHA-256 pass keeps a Worker request open for too long on
  // ordinary large workspaces.  Their integrity is still checked by the
  // desktop client before upload and after download; the Worker always checks
  // the R2 object size and ZIP structure.  Keep an independent full pass for
  // smaller archives where it is safely within the synchronous request budget.
  const maxServerSha256Bytes = numberEnv(
    env.SERVER_SHA256_VALIDATION_MAX_BYTES,
    DEFAULT_MAX_SERVER_SHA256_VALIDATION_BYTES,
  );
  if (metadata.fullData.sha256 && metadata.fullData.size <= maxServerSha256Bytes) {
    const actualSha256 = await sha256R2Object(env, objectKey);
    if (actualSha256 !== metadata.fullData.sha256.toLowerCase()) throw new Error('ARCHIVE_SHA256_MISMATCH');
  }
  return manifest;
}

function numberEnv(value, fallback) {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : fallback;
}

async function validateCentralDirectory(env, objectKey, manifest) {
  if (!env.WORKSPACE_BUCKET || typeof env.WORKSPACE_BUCKET.head !== 'function') throw new Error('R2_VALIDATION_UNAVAILABLE');
  const head = await env.WORKSPACE_BUCKET.head(objectKey);
  const objectSize = Number(head?.size);
  if (!Number.isSafeInteger(objectSize) || objectSize < 22) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  const tailLength = Math.min(objectSize, MAX_DIRECTORY_RANGE);
  const tailObject = await env.WORKSPACE_BUCKET.get(objectKey, { range: { offset: objectSize - tailLength, length: tailLength } });
  if (!tailObject) throw new Error('R2_OBJECT_NOT_FOUND');
  const tail = new Uint8Array(await tailObject.arrayBuffer());
  const eocdRelative = findSignature(tail, 0x06054b50, tail.length - 22);
  if (eocdRelative < 0) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  let entryCount = readU16(tail, eocdRelative + 10);
  let directorySize = readU32(tail, eocdRelative + 12);
  let directoryOffset = readU32(tail, eocdRelative + 16);
  if (entryCount === 0xffff || directorySize === 0xffffffff || directoryOffset === 0xffffffff) {
    const locator = eocdRelative - 20;
    if (locator < 0 || readU32(tail, locator) !== 0x07064b50) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    const zip64Offset = readU64(tail, locator + 8);
    if (zip64Offset === null || zip64Offset < objectSize - tailLength || zip64Offset + 56 > objectSize) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    const record = await readRange(env, objectKey, zip64Offset, 56);
    if (readU32(record, 0) !== 0x06064b50) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    entryCount = readU64(record, 32);
    directorySize = readU64(record, 40);
    directoryOffset = readU64(record, 48);
    if (entryCount === null || directorySize === null || directoryOffset === null) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  }
  if (!Number.isSafeInteger(entryCount) || entryCount < manifest.fileCount + 2 || entryCount > 100000 || !Number.isSafeInteger(directorySize) || !Number.isSafeInteger(directoryOffset) || directoryOffset + directorySize > objectSize) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  const directory = directoryOffset >= objectSize - tailLength && directoryOffset + directorySize <= objectSize - tailLength + tail.length
    ? tail.slice(directoryOffset - (objectSize - tailLength), directoryOffset - (objectSize - tailLength) + directorySize)
    : await readRange(env, objectKey, directoryOffset, directorySize);
  const entries = parseCentralEntries(directory, entryCount);
  const expected = new Map([['manifest.json', null], ['portable-workspace.json', null], ...manifest.files.map((file) => [file.path, file.size])]);
  if (entries.length !== entryCount || entries.length !== expected.size) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_MISMATCH');
  for (const entry of entries) {
    if (!expected.has(entry.path) || entry.uncompressedSize !== (expected.get(entry.path) ?? entry.uncompressedSize)) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_MISMATCH');
    if (entry.path.startsWith('payload/') && ((entry.compressedSize === 0 && entry.uncompressedSize > 0) || (entry.compressedSize > 0 && entry.uncompressedSize / entry.compressedSize > 1000))) throw new Error('ARCHIVE_COMPRESSION_RATIO_INVALID');
    expected.delete(entry.path);
  }
  if (expected.size !== 0) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_MISMATCH');
}

async function readRange(env, objectKey, offset, length) {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || length < 0 || length > MAX_DIRECTORY_RANGE) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  const object = await env.WORKSPACE_BUCKET.get(objectKey, { range: { offset, length } });
  if (!object) throw new Error('R2_OBJECT_NOT_FOUND');
  return new Uint8Array(await object.arrayBuffer());
}

function parseCentralEntries(bytes, expectedCount) {
  const entries = [];
  const seenPaths = new Set();
  let offset = 0;
  while (offset < bytes.length && entries.length < expectedCount) {
    if (offset + 46 > bytes.length || readU32(bytes, offset) !== 0x02014b50) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    const flags = readU16(bytes, offset + 8);
    const compressedSize = readU32(bytes, offset + 20);
    const uncompressedSize = readU32(bytes, offset + 24);
    const nameLength = readU16(bytes, offset + 28);
    const extraLength = readU16(bytes, offset + 30);
    const commentLength = readU16(bytes, offset + 32);
    const end = offset + 46 + nameLength + extraLength + commentLength;
    if (end > bytes.length || (flags & 0x1) !== 0) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    const name = new TextDecoder().decode(bytes.slice(offset + 46, offset + 46 + nameLength));
    if (!/^(?:manifest\.json|portable-workspace\.json|payload\/[A-Za-z0-9._/-]+)$/u.test(name) || (name.startsWith('payload/') && !safeArchivePath(name))) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    if (seenPaths.has(name.toUpperCase())) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    seenPaths.add(name.toUpperCase());
    const extra = bytes.slice(offset + 46 + nameLength, offset + 46 + nameLength + extraLength);
    const values = readZip64EntryValues(extra, compressedSize, uncompressedSize, readU32(bytes, offset + 42));
    if (!values) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
    entries.push({ path: name, compressedSize: values.compressedSize, uncompressedSize: values.uncompressedSize });
    offset = end;
  }
  if (offset !== bytes.length) throw new Error('ARCHIVE_CENTRAL_DIRECTORY_INVALID');
  return entries;
}

function readZip64EntryValues(extra, compressedSize, uncompressedSize, offset) {
  if (compressedSize !== 0xffffffff && uncompressedSize !== 0xffffffff && offset !== 0xffffffff) return { compressedSize, uncompressedSize };
  let cursor = 0;
  while (cursor + 4 <= extra.length) {
    const id = readU16(extra, cursor);
    const size = readU16(extra, cursor + 2);
    cursor += 4;
    if (cursor + size > extra.length) return null;
    if (id === 0x0001) {
      const data = extra.slice(cursor, cursor + size);
      let position = 0;
      const read = () => { const value = readU64(data, position); position += 8; return value; };
      const expanded = uncompressedSize === 0xffffffff ? read() : uncompressedSize;
      const compressed = compressedSize === 0xffffffff ? read() : compressedSize;
      if (compressedSize === 0xffffffff && compressed === null) return null;
      if (offset === 0xffffffff && read() === null) return null;
      return expanded === null || compressed === null ? null : { compressedSize: compressed, uncompressedSize: expanded };
    }
    cursor += size;
  }
  return null;
}

function findSignature(bytes, signature, start) {
  for (let offset = start; offset >= 0; offset -= 1) if (readU32(bytes, offset) === signature) return offset;
  return -1;
}

async function sha256R2Object(env, objectKey) {
  const object = await env.WORKSPACE_BUCKET.get(objectKey);
  if (!object?.body || typeof object.body.getReader !== 'function') throw new Error('R2_VALIDATION_UNAVAILABLE');
  const digest = new Sha256Stream();
  const reader = object.body.getReader();
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) break;
    digest.update(chunk.value);
  }
  return digest.hex();
}

function parseStoredManifest(bytes) {
  if (bytes.length < 30 || readU32(bytes, 0) !== 0x04034b50) throw new Error('ARCHIVE_HEADER_INVALID');
  const flags = readU16(bytes, 6);
  const method = readU16(bytes, 8);
  const compressedSizeField = readU32(bytes, 18);
  const uncompressedSizeField = readU32(bytes, 22);
  const nameLength = readU16(bytes, 26);
  const extraLength = readU16(bytes, 28);
  if ((flags & 0x1) !== 0 || method !== 0 || nameLength !== 13) throw new Error('ARCHIVE_MANIFEST_INVALID');
  const nameStart = 30;
  const dataStart = nameStart + nameLength + extraLength;
  const name = new TextDecoder().decode(bytes.slice(nameStart, dataStart - extraLength));
  if (name !== 'manifest.json') throw new Error('ARCHIVE_MANIFEST_INVALID');
  const extra = bytes.slice(nameStart + nameLength, dataStart);
  const compressedSize = readZip64LocalSize(extra, compressedSizeField, uncompressedSizeField);
  if (compressedSize === null) throw new Error('ARCHIVE_MANIFEST_INVALID');
  if (compressedSize > bytes.length - dataStart) throw new Error('ARCHIVE_MANIFEST_RANGE_REQUIRED');
  try { return JSON.parse(new TextDecoder().decode(bytes.slice(dataStart, dataStart + compressedSize))); } catch { throw new Error('ARCHIVE_MANIFEST_INVALID'); }
}

function readZip64LocalSize(extra, compressedSize, uncompressedSize) {
  if (compressedSize !== 0xffffffff && uncompressedSize !== 0xffffffff) return compressedSize;
  let cursor = 0;
  while (cursor + 4 <= extra.length) {
    const id = readU16(extra, cursor);
    const size = readU16(extra, cursor + 2);
    cursor += 4;
    if (cursor + size > extra.length) return null;
    if (id === 0x0001) {
      const data = extra.slice(cursor, cursor + size);
      let position = 0;
      const read = () => {
        const value = readU64(data, position);
        position += 8;
        return value;
      };
      if (uncompressedSize === 0xffffffff && read() === null) return null;
      const expanded = compressedSize === 0xffffffff ? read() : compressedSize;
      return expanded;
    }
    cursor += size;
  }
  return null;
}

function manifestRequiredLength(bytes) {
  if (bytes.length < 30 || readU32(bytes, 0) !== 0x04034b50) throw new Error('ARCHIVE_MANIFEST_INVALID');
  const nameLength = readU16(bytes, 26);
  const dataStart = 30 + nameLength + readU16(bytes, 28);
  if (dataStart > bytes.length) throw new Error('ARCHIVE_MANIFEST_INVALID');
  // SSMT 的 ZIP64 头会将 32 位长度写成 0xffffffff；扩展 Range 时也必须解析 extra。
  const size = readZip64LocalSize(bytes.slice(30 + nameLength, dataStart), readU32(bytes, 18), readU32(bytes, 22));
  if (size === null || !Number.isSafeInteger(dataStart + size)) throw new Error('ARCHIVE_MANIFEST_INVALID');
  return dataStart + size;
}

function readU16(bytes, offset) { return bytes[offset] | (bytes[offset + 1] << 8); }
function readU32(bytes, offset) { return (bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0; }
function readU64(bytes, offset) {
  if (offset + 8 > bytes.length) return null;
  const value = Number(readU32(bytes, offset)) + Number(readU32(bytes, offset + 4)) * 0x100000000;
  return Number.isSafeInteger(value) ? value : null;
}

function safeArchivePath(value) {
  if (!value.startsWith('payload/')) return false;
  const segments = value.slice('payload/'.length).split('/');
  if (segments.length < 2) return false;
  return segments.every((segment, index) => {
    const stem = segment.split('.')[0].toUpperCase();
    const extension = segment.includes('.') ? segment.slice(segment.lastIndexOf('.') + 1).toLowerCase() : '';
    return segment.length > 0 && segment !== '.' && segment !== '..' && !segment.includes(':') && !segment.endsWith(' ') && !segment.endsWith('.') && !/^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/u.test(stem) && (index < segments.length - 1 || /^(?:json|buf|ib|dds|txt|jpeg|jpg|png)$/u.test(extension));
  });
}

class Sha256Stream {
  constructor() {
    this.state = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    this.buffer = new Uint8Array(64);
    this.bufferLength = 0;
    this.bytes = 0;
  }
  update(input) {
    let offset = 0;
    this.bytes += input.length;
    while (offset < input.length) {
      const length = Math.min(64 - this.bufferLength, input.length - offset);
      this.buffer.set(input.subarray(offset, offset + length), this.bufferLength);
      this.bufferLength += length;
      offset += length;
      if (this.bufferLength === 64) { this.compress(this.buffer); this.bufferLength = 0; }
    }
  }
  compress(block) {
    const words = new Uint32Array(64);
    for (let i = 0; i < 16; i++) words[i] = (block[i * 4] << 24) | (block[i * 4 + 1] << 16) | (block[i * 4 + 2] << 8) | block[i * 4 + 3];
    for (let i = 16; i < 64; i++) { const x = words[i - 15]; const y = words[i - 2]; words[i] = (smallSigma1(y) + words[i - 7] + smallSigma0(x) + words[i - 16]) >>> 0; }
    let [a, b, c, d, e, f, g, h] = this.state;
    for (let i = 0; i < 64; i++) { const t1 = (h + bigSigma1(e) + choose(e, f, g) + SHA256_K[i] + words[i]) >>> 0; const t2 = (bigSigma0(a) + majority(a, b, c)) >>> 0; h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0; }
    this.state = this.state.map((value, index) => (value + [a, b, c, d, e, f, g, h][index]) >>> 0);
  }
  hex() {
    const bitLength = this.bytes * 8;
    const padding = new Uint8Array(((this.bufferLength + 9 + 63) & ~63) - this.bufferLength);
    padding[0] = 0x80;
    const view = new DataView(padding.buffer);
    view.setUint32(padding.length - 4, bitLength >>> 0, false);
    view.setUint32(padding.length - 8, Math.floor(bitLength / 0x100000000), false);
    this.update(padding);
    return this.state.map((word) => word.toString(16).padStart(8, '0')).join('');
  }
}

const SHA256_K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
const rotate = (value, bits) => (value >>> bits) | (value << (32 - bits));
const smallSigma0 = (value) => rotate(value, 7) ^ rotate(value, 18) ^ (value >>> 3);
const smallSigma1 = (value) => rotate(value, 17) ^ rotate(value, 19) ^ (value >>> 10);
const bigSigma0 = (value) => rotate(value, 2) ^ rotate(value, 13) ^ rotate(value, 22);
const bigSigma1 = (value) => rotate(value, 6) ^ rotate(value, 11) ^ rotate(value, 25);
const choose = (e, f, g) => (e & f) ^ (~e & g);
const majority = (a, b, c) => (a & b) ^ (a & c) ^ (b & c);
