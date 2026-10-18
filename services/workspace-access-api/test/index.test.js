import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import worker from '../src/index.js';

const run = promisify(execFile);
const entryId = '0191f000-0000-7000-8000-000000000000';

test('async route failures return a structured service error without exception contents', async () => {
  const env = { DB: { prepare() { return { bind() { return { async first() { throw new Error('private request URL'); } }; } }; } } };
  const response = await worker.fetch(new Request('https://example.test/v1/submissions/broken/complete', { method: 'POST' }), env, {});
  assert.equal(response.status, 500);
  assert.deepEqual(await response.json(), { error: { code: 'INTERNAL_ERROR' } });
});

function submissionEnv(row) {
  const writes = [];
  return {
    env: {
      DB: {
        prepare(sql) {
          return {
            bind(...values) {
              return {
                async first() { return /^SELECT \* FROM submissions/u.test(sql) ? row : null; },
                async run() { writes.push({ sql, values }); return { meta: { changes: 1 } }; },
              };
            },
          };
        },
      },
    },
    writes,
  };
}

test('published completion retries return their terminal result before legacy upload-field checks', async () => {
  const { env, writes } = submissionEnv({ submission_id: 'submission-1', entry_id: entryId, status: 'published', object_key: null, upload_id: null });
  const response = await worker.fetch(new Request('https://example.test/v1/submissions/submission-1/complete', { method: 'POST' }), env, {});
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { submissionId: 'submission-1', entryId, status: 'published' });
  assert.equal(writes.length, 0);
});

test('cancelling terminal submissions is idempotent and preserves their state', async () => {
  for (const status of ['published', 'failed', 'cancelled', 'expired_deleted']) {
    const { env, writes } = submissionEnv({ submission_id: `submission-${status}`, status });
    const response = await worker.fetch(new Request(`https://example.test/v1/submissions/submission-${status}`, { method: 'DELETE' }), env, {});
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { submissionId: `submission-${status}`, status });
    assert.equal(writes.length, 0);
  }
});

test('cancellation releases reserved bytes without refunding the daily initialization count', async () => {
  const { env, writes } = submissionEnv({
    submission_id: 'submission-cancelled',
    status: 'publishing',
    ip_key: 'ip-key',
    declared_size: 1024,
    created_at: '2026-08-14T00:00:00Z',
  });
  const response = await worker.fetch(new Request('https://example.test/v1/submissions/submission-cancelled', { method: 'DELETE' }), env, {});
  assert.equal(response.status, 200);
  const release = writes.find((item) => item.sql?.startsWith('UPDATE daily_quota SET'));
  assert.ok(release);
  assert.match(release.sql, /reserved_bytes/u);
  assert.doesNotMatch(release.sql, /submission_count/u);
});

test('part URL responses include a bounded expiry for resumable upload state', async () => {
  const row = {
    status: 'awaiting_upload',
    object_key: 'packages/sha256/aa/test.ssmtws',
    upload_id: 'upload-1',
  };
  const env = {
    R2_ENDPOINT: 'https://r2.example.test',
    R2_BUCKET_NAME: 'bucket',
    R2_ACCESS_KEY_ID: 'key',
    R2_SECRET_ACCESS_KEY: 'secret',
    DB: {
      prepare() {
        return {
          bind() {
            return { async first() { return row; } };
          },
        };
      },
    },
  };
  const before = Date.now();
  const response = await worker.fetch(new Request('https://example.test/v1/submissions/submission-1/parts', {
    method: 'POST',
    body: JSON.stringify({ partNumbers: [1, 2] }),
  }), env, {});
  const body = await response.json();
  assert.equal(response.status, 200);
  assert.equal(body.parts.length, 2);
  assert.match(body.parts[0].url, /X-Amz-Expires=3600/u);
  assert.ok(Date.parse(body.expiresAt) >= before + 59 * 60 * 1000);
  assert.ok(Date.parse(body.expiresAt) <= before + 61 * 60 * 1000);
});

test('the fifty-first daily initialization is rejected even after reservable byte capacity remains', async () => {
  const submissions = new Map();
  let submissionCount = 0;
  const env = {
    RATE_LIMIT_SECRET: 'test-secret',
    R2_ENDPOINT: 'https://r2.example.test',
    R2_BUCKET_NAME: 'bucket',
    R2_ACCESS_KEY_ID: 'key',
    R2_SECRET_ACCESS_KEY: 'secret',
    WORKSPACE_BUCKET: { async head() { return null; } },
    DB: {
      prepare(sql) {
        return {
          bind(...values) {
            return {
              async first() {
                if (sql.startsWith('SELECT submission_id, entry_id, status FROM submissions WHERE idempotency_key')) return submissions.get(values[0]) || null;
                if (sql.startsWith('INSERT INTO daily_quota')) {
                  if (submissionCount >= 50) return null;
                  submissionCount += 1;
                  return { submission_count: submissionCount };
                }
                if (sql.startsWith('SELECT submission_count, reserved_bytes, committed_bytes')) return { submission_count: submissionCount, reserved_bytes: 0, committed_bytes: 0 };
                return null;
              },
              async run() {
                if (sql.startsWith('INSERT INTO submissions')) {
                  submissions.set(values[2], { submission_id: values[0], entry_id: values[1], status: values[3] });
                }
                return { meta: { changes: 1 } };
              },
            };
          },
        };
      },
    },
  };
  const metadata = {
    schemaVersion: 1,
    gamePreset: 'SRMI',
    workspaceName: 'Rate limit test',
    attribution: { mode: 'anonymous' },
    lods: [{ name: 'LOD0', drawIB: [{ hash: '0f8a6711', alias: '' }], skipIB: [], vsCheck: [] }],
    fullData: { available: true, sha256: 'a'.repeat(64), size: 1, uncompressedSize: 1, fileCount: 1 },
  };
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response('<UploadId>upload-1</UploadId>', { status: 200 });
  try {
    for (let index = 0; index < 50; index += 1) {
      const response = await worker.fetch(new Request('https://example.test/v1/submissions', {
        method: 'POST',
        headers: { 'Idempotency-Key': `rate-limit-${index}`, 'CF-Connecting-IP': '198.51.100.1' },
        body: JSON.stringify({ metadata, archive: { size: 1 } }),
      }), env, {});
      assert.equal(response.status, 201);
    }
    const response = await worker.fetch(new Request('https://example.test/v1/submissions', {
      method: 'POST',
      headers: { 'Idempotency-Key': 'rate-limit-50', 'CF-Connecting-IP': '198.51.100.1' },
      body: JSON.stringify({ metadata, archive: { size: 1 } }),
    }), env, {});
    assert.equal(response.status, 429);
    assert.equal((await response.json()).error.code, 'RATE_LIMIT_EXCEEDED');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('invalid completed archives are deleted and their quota reservation is released', async () => {
  const writes = [];
  const row = {
    submission_id: 'submission-invalid', entry_id: entryId, status: 'awaiting_upload', object_key: 'packages/sha256/aa/test.ssmtws', upload_id: 'upload-1',
    declared_size: 10, ip_key: 'ip-key', created_at: '2026-08-14T00:00:00Z', metadata_json: JSON.stringify({ fullData: {} }),
  };
  const env = {
    R2_ENDPOINT: 'https://r2.example.test', R2_BUCKET_NAME: 'bucket', R2_ACCESS_KEY_ID: 'key', R2_SECRET_ACCESS_KEY: 'secret',
    WORKSPACE_BUCKET: { async head() { return { size: 10 }; }, async get() { return null; }, async delete(key) { writes.push({ key, delete: true }); } },
    DB: { prepare(sql) { return { bind(...values) { return { async first() { return /^SELECT \* FROM submissions/u.test(sql) ? row : null; }, async run() { writes.push({ sql, values }); return { meta: { changes: 1 } }; } }; } }; } },
  };
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (_url, init = {}) => new Response(null, { status: 200, headers: init.method === 'HEAD' ? { 'content-length': '10' } : {} });
  try {
    const response = await worker.fetch(new Request('https://example.test/v1/submissions/submission-invalid/complete', { method: 'POST', body: JSON.stringify({ parts: [{ partNumber: 1, etag: 'etag' }] }) }), env, {});
    assert.equal(response.status, 422);
    assert.equal((await response.json()).error.code, 'ARCHIVE_INVALID');
  } finally {
    globalThis.fetch = originalFetch;
  }
  assert.ok(writes.some((item) => item.delete === true && item.key === row.object_key));
  assert.ok(writes.some((item) => item.sql?.includes('UPDATE daily_quota SET reserved_bytes')));
  assert.ok(writes.some((item) => item.sql?.includes("status = ?2") && item.values?.includes('failed')));
});

test('build-index derives searchable entries from metadata and status', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'ssmt-index-'));
  const dir = path.join(root, 'games', 'SRMI', 'entries', entryId);
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(path.join(dir, 'metadata.json'), JSON.stringify({
    schemaVersion: 1, entryId, gamePreset: 'SRMI', workspaceName: 'Test', uploadedAt: '2026-08-14T00:00:00Z', capturedAt: null,
    attribution: { mode: 'anonymous' },
    generator: { name: 'SSMT', version: '1.0.0', validatorVersion: 1 },
    lods: [{ name: 'LOD0', drawIB: [{ hash: '0f8a6711', alias: 'Face' }], skipIB: [], vsCheck: [] }],
    fullData: { available: false },
  }));
  await fs.writeFile(path.join(dir, 'status.json'), JSON.stringify({ schemaVersion: 1, entryId, availability: 'active', moderation: { reviewRequired: false, reviewState: 'not_required', reviewedAt: null }, warning: { markedAt: null, reason: null }, deleteAfter: null, replacementEntryId: null }));
  await fs.cp(path.resolve(import.meta.dirname, '..', 'schema'), path.join(root, 'schema'), { recursive: true });
  await run(process.execPath, ['scripts/build-index.mjs', root], { cwd: path.resolve(import.meta.dirname, '..') });
  await run(process.execPath, ['scripts/validate-entries.mjs', root], { cwd: path.resolve(import.meta.dirname, '..') });
  const index = JSON.parse(await fs.readFile(path.join(root, 'index', 'v1', 'SRMI.json'), 'utf8'));
  assert.equal(index.entries.length, 1);
  assert.deepEqual(index.entries[0].drawIB, ['0f8a6711']);
  assert.equal(index.entries[0].attributionVerified, false);
  await fs.rm(root, { recursive: true, force: true });
});
