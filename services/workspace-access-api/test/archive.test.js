import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { validateR2Archive } from '../src/archive.js';

function storedManifestBytes(manifest) {
  const entries = [
    ['manifest.json', new TextEncoder().encode(JSON.stringify(manifest))],
    ['portable-workspace.json', new TextEncoder().encode('{}')],
    ...manifest.files.map((file) => [file.path, Uint8Array.from({ length: file.size }, (_, index) => index + 1)]),
  ];
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = new TextEncoder().encode(name);
    const header = new Uint8Array(30);
    const view = new DataView(header.buffer);
    view.setUint32(0, 0x04034b50, true);
    view.setUint16(4, 20, true);
    view.setUint16(8, 0, true);
    view.setUint32(18, data.length, true);
    view.setUint32(22, data.length, true);
    view.setUint16(26, nameBytes.length, true);
    locals.push(new Uint8Array([...header, ...nameBytes, ...data]));
    const central = new Uint8Array(46);
    const centralView = new DataView(central.buffer);
    centralView.setUint32(0, 0x02014b50, true);
    centralView.setUint16(4, 20, true);
    centralView.setUint16(6, 20, true);
    centralView.setUint16(8, 0, true);
    centralView.setUint32(20, data.length, true);
    centralView.setUint32(24, data.length, true);
    centralView.setUint16(28, nameBytes.length, true);
    centralView.setUint32(42, offset, true);
    centrals.push(new Uint8Array([...central, ...nameBytes]));
    offset += header.length + nameBytes.length + data.length;
  }
  const directory = new Uint8Array(centrals.reduce((sum, value) => sum + value.length, 0));
  let cursor = 0;
  for (const central of centrals) { directory.set(central, cursor); cursor += central.length; }
  const eocd = new Uint8Array(22);
  const eocdView = new DataView(eocd.buffer);
  eocdView.setUint32(0, 0x06054b50, true);
  eocdView.setUint16(8, entries.length, true);
  eocdView.setUint16(10, entries.length, true);
  eocdView.setUint32(12, directory.length, true);
  eocdView.setUint32(16, offset, true);
  const archive = new Uint8Array(offset + directory.length + eocd.length);
  cursor = 0;
  for (const local of locals) { archive.set(local, cursor); cursor += local.length; }
  archive.set(directory, cursor);
  archive.set(eocd, cursor + directory.length);
  return archive;
}

function zip64ManifestHeader(bytes) {
  const manifestSize = new DataView(bytes.buffer).getUint32(18, true);
  const firstLength = 30 + 13 + manifestSize;
  const shifted = new Uint8Array(bytes.length + 20);
  shifted.set(bytes.slice(0, 30), 0);
  const header = new DataView(shifted.buffer);
  header.setUint32(18, 0xffffffff, true);
  header.setUint32(22, 0xffffffff, true);
  header.setUint16(28, 20, true);
  const extra = new DataView(shifted.buffer, 30 + 13, 20);
  extra.setUint16(0, 1, true);
  extra.setUint16(2, 16, true);
  extra.setBigUint64(4, BigInt(manifestSize), true);
  extra.setBigUint64(12, BigInt(manifestSize), true);
  shifted.set(bytes.slice(30, 43), 30);
  shifted.set(bytes.slice(43, firstLength), 43 + 20);
  shifted.set(bytes.slice(firstLength), firstLength + 20);
  const directoryOffset = new DataView(shifted.buffer).getUint32(shifted.length - 22 + 16, true);
  const shiftedEocd = new DataView(shifted.buffer, shifted.length - 22);
  shiftedEocd.setUint32(16, directoryOffset + 20, true);
  for (let offset = directoryOffset + 20; offset < shifted.length - 22;) {
    if (new DataView(shifted.buffer).getUint32(offset, true) !== 0x02014b50) break;
    const nameLength = new DataView(shifted.buffer).getUint16(offset + 28, true);
    const extraLength = new DataView(shifted.buffer).getUint16(offset + 30, true);
    const commentLength = new DataView(shifted.buffer).getUint16(offset + 32, true);
    if (offset !== directoryOffset + 20) new DataView(shifted.buffer).setUint32(offset + 42, new DataView(shifted.buffer).getUint32(offset + 42, true) + 20, true);
    offset += 46 + nameLength + extraLength + commentLength;
  }
  return shifted;
}

test('R2 validation inspects the stored first manifest without loading the archive', async () => {
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: 1, totalUncompressedSize: 4, files: [{ path: 'payload/LOD0/mesh.buf', role: 'payload', size: 4, sha256: 'a'.repeat(64) }] };
  const bytes = storedManifestBytes(manifest);
  const digest = createHash('sha256').update(bytes).digest('hex');
  let getCount = 0;
  const env = { WORKSPACE_BUCKET: { async head() { return { size: bytes.length }; }, async get() { getCount += 1; return { async arrayBuffer() { return bytes.buffer; }, body: new ReadableStream({ start(controller) { controller.enqueue(bytes); controller.close(); } }) }; } } };
  const metadata = { fullData: { size: bytes.length, uncompressedSize: 4, fileCount: 1, sha256: digest } };
  assert.deepEqual(await validateR2Archive(env, 'packages/test.ssmtws', metadata), manifest);
  assert.equal(getCount, 3);
  await assert.rejects(() => validateR2Archive(env, 'packages/test.ssmtws', { fullData: { uncompressedSize: 5, fileCount: 1 } }), /ARCHIVE_MANIFEST_MISMATCH/);
});

test('R2 validation reads a Zip64 local manifest header', async () => {
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: 1, totalUncompressedSize: 4, files: [{ path: 'payload/LOD0/mesh.buf', role: 'payload', size: 4, sha256: 'a'.repeat(64) }] };
  const bytes = storedManifestBytes(manifest);
  const zipped = zip64ManifestHeader(bytes);
  const env = { WORKSPACE_BUCKET: { async head() { return { size: zipped.length }; }, async get() { return { async arrayBuffer() { return zipped.buffer; }, body: new ReadableStream({ start(controller) { controller.enqueue(zipped); controller.close(); } }) }; } } };
  await assert.doesNotReject(() => validateR2Archive(env, 'packages/test.ssmtws', { fullData: { uncompressedSize: 4, fileCount: 1 } }));
});

test('expands the manifest range using Zip64 sizes when the first 64 KiB is insufficient', async () => {
  const files = Array.from({ length: 500 }, (_, index) => ({ path: `payload/LOD0/model_${index}/mesh.buf`, role: 'payload', size: 4, sha256: 'a'.repeat(64) }));
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: files.length, totalUncompressedSize: files.length * 4, files };
  for (const zip64 of [false, true]) {
    const bytes = zip64 ? zip64ManifestHeader(storedManifestBytes(manifest)) : storedManifestBytes(manifest);
    const ranges = [];
    const env = { WORKSPACE_BUCKET: {
      async head() { return { size: bytes.length }; },
      async get(key, { range } = {}) {
        const chunk = range ? bytes.slice(range.offset, range.offset + range.length) : bytes;
        if (range) ranges.push(range);
        return { async arrayBuffer() { return chunk.buffer; }, body: new ReadableStream({ start(controller) { controller.enqueue(chunk); controller.close(); } }) };
      },
    } };
    const metadata = { fullData: { size: bytes.length, uncompressedSize: manifest.totalUncompressedSize, fileCount: manifest.fileCount, sha256: createHash('sha256').update(bytes).digest('hex') } };
    assert.deepEqual(await validateR2Archive(env, 'packages/test.ssmtws', metadata), manifest);
    assert.equal(ranges[0].length, 64 * 1024);
    assert.ok(ranges[1].length > 64 * 1024 && ranges[1].length < 128 * 1024);
  }
});

test('rejects oversized Zip64 manifest lengths before an expanded R2 read', async () => {
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: 1, totalUncompressedSize: 4, files: [{ path: 'payload/LOD0/mesh.buf', role: 'payload', size: 4, sha256: 'a'.repeat(64) }] };
  const bytes = zip64ManifestHeader(storedManifestBytes(manifest));
  new DataView(bytes.buffer).setBigUint64(30 + 13 + 12, BigInt(32 * 1024 * 1024), true);
  let reads = 0;
  const env = { WORKSPACE_BUCKET: { async get() { reads += 1; return { async arrayBuffer() { return bytes.buffer; } }; } } };
  await assert.rejects(() => validateR2Archive(env, 'packages/test.ssmtws', { fullData: {} }), /ARCHIVE_MANIFEST_INVALID/u);
  assert.equal(reads, 1);
});

test('R2 validation rejects a central-directory payload size that disagrees with the manifest', async () => {
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: 1, totalUncompressedSize: 4, files: [{ path: 'payload/LOD0/mesh.buf', role: 'payload', size: 4, sha256: 'a'.repeat(64) }] };
  const bytes = storedManifestBytes(manifest);
  let matches = 0;
  for (let index = 0; index <= bytes.length - 4; index += 1) {
    if (new DataView(bytes.buffer).getUint32(index, true) === 0x02014b50 && ++matches === 3) new DataView(bytes.buffer).setUint32(index + 24, 5, true);
  }
  const digest = createHash('sha256').update(bytes).digest('hex');
  const env = { WORKSPACE_BUCKET: { async head() { return { size: bytes.length }; }, async get() { return { async arrayBuffer() { return bytes.buffer; }, body: new ReadableStream({ start(controller) { controller.enqueue(bytes); controller.close(); } }) }; } } };
  await assert.rejects(() => validateR2Archive(env, 'packages/test.ssmtws', { fullData: { size: bytes.length, uncompressedSize: 4, fileCount: 1, sha256: digest } }), /ARCHIVE_CENTRAL_DIRECTORY_MISMATCH/);
});

test('keeps structural validation for large archives without a synchronous full-object hash', async () => {
  const manifest = { archiveVersion: 1, portableMetadataPath: 'portable-workspace.json', fileCount: 1, totalUncompressedSize: 4, files: [{ path: 'payload/LOD0/mesh.buf', role: 'payload', size: 4, sha256: 'a'.repeat(64) }] };
  const bytes = storedManifestBytes(manifest);
  let getCount = 0;
  const env = { WORKSPACE_BUCKET: { async head() { return { size: bytes.length }; }, async get() { getCount += 1; return { async arrayBuffer() { return bytes.buffer; }, body: new ReadableStream({ start(controller) { controller.enqueue(bytes); controller.close(); } }) }; } } };
  await assert.doesNotReject(() => validateR2Archive(env, 'packages/test.ssmtws', { fullData: { size: 64 * 1024 * 1024, uncompressedSize: 4, fileCount: 1, sha256: 'a'.repeat(64) } }));
  assert.equal(getCount, 2);
});
