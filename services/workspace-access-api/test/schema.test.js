import test from 'node:test';
import assert from 'node:assert/strict';
import Ajv from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
import fs from 'node:fs';
import path from 'node:path';
import { attachServerEntryId, normalizeMetadata } from '../src/core.js';

const schema = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../schema/v1/metadata.schema.json'), 'utf8'));
const ajv = new Ajv({ strict: true, strictRequired: false });
addFormats(ajv);
const validate = ajv.compile(schema);

test('server-normalized metadata shape validates against schema v1', () => {
  const normalized = normalizeMetadata({
    schemaVersion: 1,
    gamePreset: 'SRMI',
    workspaceName: 'Schema Test',
    capturedAt: null,
    attribution: { mode: 'anonymous' },
    generator: { name: 'SSMT', version: '1.0.0', validatorVersion: 1 },
    lods: [{ name: 'LOD0', drawIB: [{ hash: '0f8a6711', alias: '' }], skipIB: [], vsCheck: [] }],
    fullData: { available: true, sha256: 'a'.repeat(64), size: 123, uncompressedSize: 456, fileCount: 1 },
  }, '2026-08-14T00:00:00.000Z');
  assert.equal(normalized.error, undefined);
  const metadata = attachServerEntryId(normalized.value, '0191f000-0000-7000-8000-000000000000');
  metadata.fullData.objectKey = `packages/sha256/aa/${'a'.repeat(64)}.ssmtws`;
  assert.equal(validate(metadata), true, ajv.errorsText(validate.errors));
  metadata.lods[0].componentAliases = { '0f8a6711-3-0': '面部.亮 / Face 😀' };
  assert.equal(validate(metadata), true, ajv.errorsText(validate.errors));
  metadata.lods[0].componentAliases['0f8a6711-3-0'] = 'bad\nname';
  assert.equal(validate(metadata), false);
});
