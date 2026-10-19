import test from 'node:test';
import assert from 'node:assert/strict';
import { attachServerEntryId, createStatus, createUuidV7, normalizeMetadata } from '../src/core.js';

const draft = {
  schemaVersion: 1,
  gamePreset: 'SRMI',
  workspaceName: 'Test',
  attribution: { mode: 'custom', displayName: 'self supplied' },
  lods: [{ name: 'LOD0', drawIB: [{ hash: '0F8A6711', alias: 'Face' }], skipIB: [], vsCheck: [] }],
  fullData: { available: false },
};

test('Component aliases round trip without changing keys, and old metadata stays unchanged', () => {
  const lod = draft.lods[0];
  const aliases = { '0f8a6711-16884-0': ' 面部.高光 ', '0f8a6711-25830-16884': 'Body', '0f8a6711-3-0': '' };
  const normalized = normalizeMetadata({ ...draft, lods: [{ ...lod, componentAliases: aliases }] }, new Date().toISOString());
  assert.deepEqual(normalized.value.lods[0].componentAliases, { '0f8a6711-16884-0': '面部.高光', '0f8a6711-25830-16884': 'Body' });
  assert.equal(normalizeMetadata(draft, new Date().toISOString()).value.lods[0].componentAliases, undefined);
  for (const componentAliases of [null, [], { '../other': 'Face' }, { mesh: 'bad\nname' }, { mesh: 'x'.repeat(129) }]) {
    assert.equal(normalizeMetadata({ ...draft, lods: [{ ...lod, componentAliases }] }, new Date().toISOString()).error, 'INVALID_METADATA');
  }
});

test('normalizes metadata and never trusts client timestamps', () => {
  const result = normalizeMetadata(draft, '2026-08-14T00:00:00.000Z');
  assert.equal(result.value.uploadedAt, '2026-08-14T00:00:00.000Z');
  assert.equal(result.value.lods[0].drawIB[0].hash, '0f8a6711');
  assert.equal(result.value.attribution.mode, 'custom');
});

test('normalizes workspace aliases for public library search', () => {
  const result = normalizeMetadata({ ...draft, workspaceAliases: [' Himeko ', 'himeko', 'Fire Form'] }, '2026-08-14T00:00:00.000Z');
  assert.deepEqual(result.value.workspaceAliases, ['Himeko', 'Fire Form']);
  assert.equal(normalizeMetadata({ ...draft, workspaceAliases: ['bad\u0000alias'] }, new Date().toISOString()).error, 'INVALID_METADATA');
});

test('accepts historic camel-cased IB fields and emits canonical protocol keys', () => {
  const result = normalizeMetadata({
    ...draft,
    lods: [{ name: 'LOD0', drawIb: [{ hash: '0F8A6711', alias: 'Face' }], skipIb: [], vsCheck: [] }],
  }, '2026-08-14T00:00:00.000Z');
  assert.deepEqual(result.value.lods[0].drawIB, [{ hash: '0f8a6711', alias: 'Face' }]);
  assert.deepEqual(result.value.lods[0].skipIB, []);
});

test('rejects unsupported presets and unsafe names', () => {
  assert.equal(normalizeMetadata({ ...draft, gamePreset: 'unknown' }, new Date().toISOString()).error, 'UNSUPPORTED_GAME_PRESET');
  assert.equal(normalizeMetadata({ ...draft, workspaceName: 'bad\u0000name' }, new Date().toISOString()).error, 'INVALID_WORKSPACE_NAME');
});

test('rejects malformed portable timestamps, attribution, and supersedes references', () => {
  const now = '2026-08-14T00:00:00.000Z';
  assert.equal(normalizeMetadata({ ...draft, capturedAt: 'not-a-time' }, now).error, 'INVALID_METADATA');
  assert.equal(normalizeMetadata({ ...draft, attribution: { mode: 'custom' } }, now).error, 'INVALID_METADATA');
  assert.equal(normalizeMetadata({ ...draft, supersedes: 'not-a-uuid' }, now).error, 'INVALID_METADATA');
  const result = normalizeMetadata({ ...draft, capturedAt: '2026-08-14T01:02:03+08:00', supersedes: '0191f000-0000-7000-8000-000000000000' }, now);
  assert.equal(result.value.capturedAt, '2026-08-13T17:02:03.000Z');
  assert.equal(result.value.supersedes, '0191f000-0000-7000-8000-000000000000');
});

test('creates server UUIDv7 and review status', () => {
  const entryId = createUuidV7(1723593600000);
  assert.match(entryId, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
  const metadata = normalizeMetadata({ ...draft, fullData: { available: true, sha256: 'a'.repeat(64), size: 209715201, uncompressedSize: 1, fileCount: 1 } }, new Date().toISOString()).value;
  const status = createStatus(entryId, metadata);
  assert.equal(status.moderation.reviewRequired, true);
  assert.equal(status.metadataDownloadCount, 0);
  assert.equal(status.fullPackageDownloadCount, 0);
});

test('metadata normalization leaves the server-owned entry ID out of client control', () => {
  const result = normalizeMetadata({ ...draft, entryId: 'client-controlled-id' }, new Date().toISOString());
  assert.equal(result.value.entryId, undefined);
  assert.equal(attachServerEntryId(result.value, '0191f000-0000-7000-8000-000000000000').entryId, '0191f000-0000-7000-8000-000000000000');
});
