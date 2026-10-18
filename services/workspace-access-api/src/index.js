import { attachServerEntryId, createStatus, createUuidV7, ERROR_CODES, json, apiError, normalizeMetadata } from './core.js';
import { validateR2Archive } from './archive.js';
import { fetchPublicEntry, publishDownloadCounts, publishEntry, removeExpiredEntries } from './github.js';
import { abortMultipart, completeMultipart, createMultipart, deleteObject, headObject, presignObject, presignPart } from './r2.js';

const MAX_BODY_BYTES = 1024 * 1024;
const PART_URL_TTL_SECONDS = 3600;

export default {
  async fetch(request, env, ctx) {
    try {
      const url = new URL(request.url);
      if (request.method === 'POST' && url.pathname === '/v1/submissions') return await initializeSubmission(request, env);
      const partsMatch = url.pathname.match(/^\/v1\/submissions\/([^/]+)\/parts$/u);
      if (request.method === 'POST' && partsMatch) return await issueUploadParts(request, env, partsMatch[1]);
      const completeMatch = url.pathname.match(/^\/v1\/submissions\/([^/]+)\/complete$/u);
      if (request.method === 'POST' && completeMatch) return await completeSubmission(request, env, completeMatch[1]);
      const cancelMatch = url.pathname.match(/^\/v1\/submissions\/([^/]+)$/u);
      if (request.method === 'DELETE' && cancelMatch) return await cancelSubmission(env, cancelMatch[1]);
      const downloadMatch = url.pathname.match(/^\/v1\/entries\/([^/]+)\/download$/u);
      if (request.method === 'GET' && downloadMatch) return await downloadEntry(env, downloadMatch[1]);
      const downloadRecordMatch = url.pathname.match(/^\/v1\/entries\/([^/]+)\/downloads$/u);
      if (request.method === 'POST' && downloadRecordMatch) return await recordEntryDownload(request, env, ctx, downloadRecordMatch[1]);
      return apiError('NOT_FOUND', 404);
    } catch (error) {
      console.error('workspace-api request failed', error instanceof Error ? error.name : 'unknown');
      return apiError(ERROR_CODES.INTERNAL_ERROR, 500);
    }
  },
  async scheduled(event, env, ctx) {
    ctx.waitUntil(Promise.all([cleanupStaleSubmissions(env), cleanupExpiredEntries(env), cleanupOldQuotaRecords(env)]));
  },
};

async function cleanupOldQuotaRecords(env) {
  const cutoff = new Date(Date.now() - 2 * 24 * 60 * 60 * 1000).toISOString().slice(0, 10);
  await env.DB.prepare('DELETE FROM daily_quota WHERE utc_date < ?1').bind(cutoff).run();
  await env.DB.prepare('DELETE FROM entry_download_events WHERE utc_date < ?1').bind(cutoff).run();
}

async function cleanupStaleSubmissions(env) {
  const ttlHours = numberEnv(env.UPLOAD_TTL_HOURS, 24);
  const cutoff = new Date(Date.now() - ttlHours * 60 * 60 * 1000).toISOString();
  const result = await env.DB.prepare("SELECT submission_id, status, object_key, upload_id, ip_key, declared_size, created_at FROM submissions WHERE status IN ('awaiting_upload', 'ready_to_publish') AND updated_at < ?1 LIMIT 100").bind(cutoff).all();
  for (const row of result.results || []) {
    try {
      const claimed = await env.DB.prepare("UPDATE submissions SET status = 'cancelling', updated_at = ?2 WHERE submission_id = ?1 AND status IN ('awaiting_upload', 'ready_to_publish')").bind(row.submission_id, new Date().toISOString()).run();
      if (!claimed.meta?.changes) continue;
      if (row.status === 'awaiting_upload' && row.object_key && row.upload_id) await abortMultipart(env, row.object_key, row.upload_id);
      if (row.status === 'ready_to_publish' && row.object_key) await deleteObjectIfUnpublished(env, row.object_key);
      await releaseQuota(env, row.ip_key, row.created_at.slice(0, 10), Number(row.declared_size || 0));
      await env.DB.prepare("UPDATE submissions SET status = 'cancelled', updated_at = ?2 WHERE submission_id = ?1 AND status = 'cancelling'").bind(row.submission_id, new Date().toISOString()).run();
    } catch (error) {
      await env.DB.prepare("UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1 AND status = 'cancelling'").bind(row.submission_id, row.status, new Date().toISOString()).run();
      console.error('workspace-api stale submission cleanup failed', error instanceof Error ? error.name : 'unknown');
    }
  }
}

async function cleanupExpiredEntries(env) {
  const removed = await removeExpiredEntries(env);
  for (const entry of removed) {
    await env.DB.prepare("UPDATE submissions SET status = 'expired_deleted', updated_at = ?2 WHERE entry_id = ?1 AND status = 'published'").bind(entry.entryId, new Date().toISOString()).run();
    if (entry.objectKey && entry.deleteObject) await deleteObject(env, entry.objectKey);
  }
}

async function initializeSubmission(request, env) {
  const idempotencyKey = request.headers.get('Idempotency-Key')?.trim();
  if (!idempotencyKey || idempotencyKey.length > 128) return apiError(ERROR_CODES.IDEMPOTENCY_KEY_REQUIRED);
  const body = await readJsonLimited(request);
  if (!body) return apiError(ERROR_CODES.INVALID_METADATA, 413);
  const now = new Date().toISOString();
  const normalized = normalizeMetadata(body.metadata, now);
  if (normalized.error) return apiError(normalized.error);
  let metadata = normalized.value;
  const declaredSize = Number.isSafeInteger(body.archive?.size) && body.archive.size >= 0 ? body.archive.size : 0;
  const maxArchiveBytes = numberEnv(env.MAX_ARCHIVE_BYTES, 1073741824);
  if (declaredSize > maxArchiveBytes) return apiError(ERROR_CODES.ARCHIVE_TOO_LARGE);
  if (metadata.fullData.available && declaredSize !== metadata.fullData.size) return apiError(ERROR_CODES.INVALID_METADATA);

  const ipKey = await ipDigest(env.RATE_LIMIT_SECRET, request.headers.get('CF-Connecting-IP') || 'unknown', now.slice(0, 10));
  const existing = await env.DB.prepare('SELECT submission_id, entry_id, status FROM submissions WHERE idempotency_key = ?1').bind(idempotencyKey).first();
  if (existing) return json({ submissionId: existing.submission_id, entryId: existing.entry_id, status: existing.status });
  const quota = await reserveQuota(env, ipKey, now.slice(0, 10), declaredSize);
  if (!quota.ok) return apiError(quota.code, 429);

  const submissionId = createUuidV7();
  const entryId = createUuidV7();
  metadata = attachServerEntryId(metadata, entryId);
  const status = createStatus(entryId, metadata, numberEnv(env.LARGE_REVIEW_THRESHOLD, 209715200));
  const fullData = metadata.fullData;
  let reusedExistingObject = false;
  try {
    await env.DB.prepare('INSERT INTO submissions (submission_id, entry_id, idempotency_key, status, metadata_json, ip_key, declared_size, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)')
      .bind(submissionId, entryId, idempotencyKey, 'publishing', JSON.stringify(metadata), ipKey, declaredSize, now).run();
  } catch {
    await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
    const concurrent = await env.DB.prepare('SELECT submission_id, entry_id, status FROM submissions WHERE idempotency_key = ?1').bind(idempotencyKey).first();
    if (concurrent) return json({ submissionId: concurrent.submission_id, entryId: concurrent.entry_id, status: concurrent.status });
    return apiError(ERROR_CODES.INTERNAL_ERROR, 500);
  }
  if (fullData.available) {
    const objectKey = `packages/sha256/${fullData.sha256.slice(0, 2)}/${fullData.sha256}.ssmtws`;
    fullData.objectKey = objectKey;
    await env.DB.prepare('UPDATE submissions SET object_key = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, objectKey, now).run();
    let existing;
    try {
      existing = await existingObject(env, objectKey);
    } catch {
      await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
      await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'failed', now).run();
      return apiError('R2_HEAD_FAILED', 502);
    }
    if (existing) {
      if (existing !== declaredSize) {
        await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
        await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'failed', now).run();
        return apiError('OBJECT_SIZE_MISMATCH', 422);
      }
      try {
        await validateR2Archive(env, objectKey, metadata);
      } catch (error) {
        await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
        await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'failed', now).run();
        console.error('workspace-api existing archive validation failed', error instanceof Error ? error.name : 'unknown');
        return apiError(error instanceof Error && error.message === 'R2_VALIDATION_UNAVAILABLE' ? 'R2_VALIDATION_UNAVAILABLE' : 'ARCHIVE_INVALID', 422, archiveErrorDetail(error));
      }
      reusedExistingObject = true;
    } else {
      try {
        const uploadId = await createMultipart(env, objectKey);
        await env.DB.prepare('UPDATE submissions SET upload_id = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, uploadId, now).run();
      } catch {
        await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
        await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'failed', now).run();
        return apiError('R2_MULTIPART_INIT_FAILED', 502);
      }
    }
  }
  if (!fullData.available || reusedExistingObject) {
    try {
      await publishEntry(env, metadata, status, entryId);
    } catch (error) {
      await releaseQuota(env, ipKey, now.slice(0, 10), declaredSize);
      await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'failed', now).run();
      console.error('workspace-api publish failed', error instanceof Error ? error.name : 'unknown');
      return apiError(ERROR_CODES.GITHUB_PUBLISH_FAILED, 502);
    }
  }
  const finalStatus = fullData.available && !reusedExistingObject ? 'awaiting_upload' : 'published';
  if (finalStatus === 'published') {
    await finalizePublishedSubmission(env, submissionId, ipKey, now.slice(0, 10), declaredSize, JSON.stringify(metadata), now);
  } else {
    await env.DB.prepare('UPDATE submissions SET status = ?2, metadata_json = ?3, updated_at = ?4 WHERE submission_id = ?1').bind(submissionId, finalStatus, JSON.stringify(metadata), now).run();
  }
  return json({ submissionId, entryId, status: finalStatus, metadataPath: `games/${metadata.gamePreset}/entries/${entryId}/metadata.json` }, 201);
}

async function existingObject(env, objectKey) {
  if (!env.WORKSPACE_BUCKET || typeof env.WORKSPACE_BUCKET.head !== 'function') return null;
  const object = await env.WORKSPACE_BUCKET.head(objectKey);
  return object ? Number(object.size || 0) : null;
}

async function issueUploadParts(request, env, submissionId) {
  const row = await env.DB.prepare('SELECT status, object_key, upload_id FROM submissions WHERE submission_id = ?1').bind(submissionId).first();
  if (!row || row.status !== 'awaiting_upload' || !row.object_key || !row.upload_id) return apiError('SUBMISSION_EXPIRED', 409);
  const body = await readJsonLimited(request);
  if (!body || !Array.isArray(body.partNumbers) || body.partNumbers.length === 0 || body.partNumbers.length > 10000) return apiError('UPLOAD_PART_MISMATCH');
  const partNumbers = [...new Set(body.partNumbers)].filter((part) => Number.isSafeInteger(part) && part >= 1 && part <= 10000);
  if (partNumbers.length !== body.partNumbers.length) return apiError('UPLOAD_PART_MISMATCH');
  const urls = await Promise.all(partNumbers.map(async (partNumber) => ({ partNumber, url: await presignPart(env, row.object_key, row.upload_id, partNumber, PART_URL_TTL_SECONDS) })));
  return json({ submissionId, expiresAt: new Date(Date.now() + PART_URL_TTL_SECONDS * 1000).toISOString(), parts: urls });
}

async function completeSubmission(request, env, submissionId) {
  const row = await env.DB.prepare('SELECT * FROM submissions WHERE submission_id = ?1').bind(submissionId).first();
  if (!row) return apiError('SUBMISSION_EXPIRED', 409);
  if (row.status === 'published') return json({ submissionId, entryId: row.entry_id, status: 'published' });
  if (!row.object_key || !row.upload_id) return apiError('SUBMISSION_EXPIRED', 409);
  if (!['awaiting_upload', 'ready_to_publish'].includes(row.status)) return apiError('SUBMISSION_EXPIRED', 409);
  if (row.status === 'awaiting_upload') {
    const body = await readJsonLimited(request);
    if (!body || !Array.isArray(body.parts) || body.parts.length === 0) return apiError('UPLOAD_PART_MISMATCH');
    const parts = body.parts.map((part) => ({ partNumber: part.partNumber, etag: typeof part.etag === 'string' ? part.etag : '' }));
    if (parts.some((part, index) => !Number.isSafeInteger(part.partNumber) || part.partNumber < 1 || !part.etag || (index > 0 && part.partNumber <= parts[index - 1].partNumber))) return apiError('UPLOAD_PART_MISMATCH');
    await completeMultipart(env, row.object_key, row.upload_id, parts);
    await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1')
      .bind(submissionId, 'ready_to_publish', new Date().toISOString()).run();
  }
  const object = await headObject(env, row.object_key);
  if (object.size !== Number(row.declared_size)) {
    const discarded = await discardCompletedSubmission(env, row);
    return discarded ? apiError('OBJECT_SIZE_MISMATCH', 422) : apiError('R2_CLEANUP_FAILED', 502);
  }
  const metadata = JSON.parse(row.metadata_json);
  metadata.fullData.objectKey = row.object_key;
  try {
    await validateR2Archive(env, row.object_key, metadata);
  } catch (error) {
    console.error('workspace-api archive validation failed', error instanceof Error ? error.message : 'unknown');
    if (error instanceof Error && error.message === 'R2_VALIDATION_UNAVAILABLE') return apiError('R2_VALIDATION_UNAVAILABLE', 422);
    const discarded = await discardCompletedSubmission(env, row);
    return discarded ? apiError('ARCHIVE_INVALID', 422, archiveErrorDetail(error)) : apiError('R2_CLEANUP_FAILED', 502);
  }
  const status = createStatus(row.entry_id, metadata, numberEnv(env.LARGE_REVIEW_THRESHOLD, 209715200));
  await env.DB.prepare('UPDATE submissions SET metadata_json = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, JSON.stringify(metadata), new Date().toISOString()).run();
  try {
    await publishEntry(env, metadata, status, row.entry_id);
  } catch (error) {
    console.error('workspace-api publish failed', error instanceof Error ? error.name : 'unknown');
    return apiError(ERROR_CODES.GITHUB_PUBLISH_FAILED, 502);
  }
  await finalizePublishedSubmission(env, submissionId, row.ip_key, row.created_at.slice(0, 10), row.declared_size, null, new Date().toISOString());
  return json({ submissionId, entryId: row.entry_id, status: 'published', metadataPath: `games/${metadata.gamePreset}/entries/${row.entry_id}/metadata.json` });
}

function archiveErrorDetail(error) {
  // 只向客户端公开稳定的校验错误码，不透传 R2 地址或底层异常正文。
  return error instanceof Error && /^ARCHIVE_[A-Z0-9_]+$/u.test(error.message) ? error.message : undefined;
}

async function finalizePublishedSubmission(env, submissionId, ipKey, utcDate, bytes, metadataJson, now) {
  const submission = metadataJson == null
    ? env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'published', now)
    : env.DB.prepare('UPDATE submissions SET status = ?2, metadata_json = ?3, updated_at = ?4 WHERE submission_id = ?1').bind(submissionId, 'published', metadataJson, now);
  const quota = env.DB.prepare('UPDATE daily_quota SET reserved_bytes = MAX(reserved_bytes - ?3, 0), committed_bytes = committed_bytes + ?3 WHERE utc_date = ?1 AND ip_key = ?2').bind(utcDate, ipKey, bytes);
  if (typeof env.DB.batch === 'function') {
    await env.DB.batch([submission, quota]);
  } else {
    await submission.run();
    await quota.run();
  }
}

async function cancelSubmission(env, submissionId) {
  const row = await env.DB.prepare('SELECT * FROM submissions WHERE submission_id = ?1').bind(submissionId).first();
  if (!row) return apiError('ENTRY_NOT_FOUND', 404);
  if (row.status === 'published' || row.status === 'failed' || row.status === 'cancelled' || row.status === 'expired_deleted') return json({ submissionId, status: row.status });
  if (row.status === 'awaiting_upload' && row.object_key && row.upload_id) await abortMultipart(env, row.object_key, row.upload_id);
  if (row.status === 'ready_to_publish' && row.object_key) {
    try {
      await deleteObjectIfUnpublished(env, row.object_key);
    } catch {
      return apiError('R2_CLEANUP_FAILED', 502);
    }
  }
  if (['awaiting_upload', 'publishing', 'ready_to_publish'].includes(row.status)) {
    await releaseQuota(env, row.ip_key, row.created_at.slice(0, 10), row.declared_size);
    await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(submissionId, 'cancelled', new Date().toISOString()).run();
  }
  return json({ submissionId, status: 'cancelled' });
}

async function downloadEntry(env, entryId) {
  const row = await env.DB.prepare('SELECT status, object_key, metadata_json FROM submissions WHERE entry_id = ?1').bind(entryId).first();
  if (!row || row.status !== 'published' || !row.object_key) return apiError('ENTRY_NOT_FOUND', 404);
  try {
    const metadata = JSON.parse(row.metadata_json);
    const publicEntry = await fetchPublicEntry(env, metadata.gamePreset, entryId);
    if (!publicEntry || publicEntry.metadata?.fullData?.available !== true || publicEntry.metadata?.fullData?.objectKey !== row.object_key) return apiError('ENTRY_NOT_FOUND', 404);
    if (publicEntry.status?.availability === 'expired' && publicEntry.status?.deleteAfter && Date.parse(publicEntry.status.deleteAfter) <= Date.now()) return apiError('ENTRY_NOT_FOUND', 404);
  } catch {
    return apiError('PUBLIC_ENTRY_UNAVAILABLE', 503);
  }
  return json({ entryId, url: await presignObject(env, row.object_key) });
}

async function recordEntryDownload(request, env, ctx, entryId) {
  const row = await env.DB.prepare('SELECT entry_id, status, metadata_json, metadata_download_count, full_package_download_count FROM submissions WHERE entry_id = ?1').bind(entryId).first();
  if (!row || row.status !== 'published') return apiError('ENTRY_NOT_FOUND', 404);
  const body = await readJsonLimited(request);
  const kind = body?.kind === 'metadata' || body?.kind === 'fullPackage' ? body.kind : null;
  if (!kind) return apiError('DOWNLOAD_KIND_INVALID');
  let metadata;
  try {
    metadata = JSON.parse(row.metadata_json);
    const publicEntry = await fetchPublicEntry(env, metadata.gamePreset, entryId);
    if (!publicEntry || publicEntry.status?.availability === 'expired') return apiError('ENTRY_NOT_FOUND', 404);
  } catch {
    return apiError('PUBLIC_ENTRY_UNAVAILABLE', 503);
  }
  const utcDate = new Date().toISOString().slice(0, 10);
  const ipKey = await ipDigest(env.RATE_LIMIT_SECRET, request.headers.get('CF-Connecting-IP') || 'unknown', utcDate);
  const event = await env.DB.prepare('INSERT INTO entry_download_events (entry_id, download_kind, utc_date, ip_key) VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING RETURNING entry_id')
    .bind(entryId, kind === 'metadata' ? 'metadata' : 'full_package', utcDate, ipKey).first();
  let counts;
  if (event) {
    const field = kind === 'metadata' ? 'metadata_download_count' : 'full_package_download_count';
    counts = await env.DB.prepare(`UPDATE submissions SET ${field} = ${field} + 1, updated_at = ?2 WHERE entry_id = ?1 RETURNING metadata_download_count, full_package_download_count`)
      .bind(entryId, new Date().toISOString()).first();
  } else {
    counts = await env.DB.prepare('SELECT metadata_download_count, full_package_download_count FROM submissions WHERE entry_id = ?1').bind(entryId).first();
  }
  const normalizedCounts = {
    metadataDownloadCount: Number(counts?.metadata_download_count || 0),
    fullPackageDownloadCount: Number(counts?.full_package_download_count || 0),
  };
  ctx.waitUntil(publishDownloadCounts(env, metadata.gamePreset, entryId, normalizedCounts).catch((error) => {
    console.error('workspace-api download count publish failed', error instanceof Error ? error.name : 'unknown');
  }));
  return json({ entryId, ...normalizedCounts });
}

async function discardCompletedSubmission(env, row) {
  try {
    if (row.object_key) await deleteObject(env, row.object_key);
    await releaseQuota(env, row.ip_key, row.created_at.slice(0, 10), row.declared_size);
    await env.DB.prepare('UPDATE submissions SET status = ?2, updated_at = ?3 WHERE submission_id = ?1').bind(row.submission_id, 'failed', new Date().toISOString()).run();
    return true;
  } catch (error) {
    console.error('workspace-api completed submission cleanup failed', error instanceof Error ? error.name : 'unknown');
    return false;
  }
}

async function deleteObjectIfUnpublished(env, objectKey) {
  const reference = await env.DB.prepare("SELECT submission_id FROM submissions WHERE object_key = ?1 AND status = 'published' LIMIT 1").bind(objectKey).first();
  if (!reference) await deleteObject(env, objectKey);
}

async function reserveQuota(env, ipKey, utcDate, bytes) {
  const maxSubmissions = numberEnv(env.MAX_SUBMISSIONS_PER_IP_PER_DAY, 50);
  const maxBytes = numberEnv(env.MAX_ARCHIVE_BYTES_PER_IP_PER_DAY, 5368709120);
  if (bytes > maxBytes) return { ok: false, code: ERROR_CODES.DAILY_BYTE_QUOTA_EXCEEDED };
  const result = await env.DB.prepare('INSERT INTO daily_quota (utc_date, ip_key, submission_count, reserved_bytes, committed_bytes) VALUES (?1, ?2, 1, ?3, 0) ON CONFLICT(utc_date, ip_key) DO UPDATE SET submission_count = submission_count + 1, reserved_bytes = reserved_bytes + excluded.reserved_bytes WHERE submission_count < ?4 AND reserved_bytes + committed_bytes + excluded.reserved_bytes <= ?5 RETURNING submission_count')
    .bind(utcDate, ipKey, bytes, maxSubmissions, maxBytes).first();
  if (result) return { ok: true };
  const current = await env.DB.prepare('SELECT submission_count, reserved_bytes, committed_bytes FROM daily_quota WHERE utc_date = ?1 AND ip_key = ?2').bind(utcDate, ipKey).first();
  const count = Number(current?.submission_count || 0);
  const used = Number(current?.reserved_bytes || 0) + Number(current?.committed_bytes || 0);
  return { ok: false, code: count >= maxSubmissions ? ERROR_CODES.RATE_LIMIT_EXCEEDED : used + bytes > maxBytes ? ERROR_CODES.DAILY_BYTE_QUOTA_EXCEEDED : ERROR_CODES.RATE_LIMIT_EXCEEDED };
}

async function releaseQuota(env, ipKey, utcDate, bytes) {
  await env.DB.prepare('UPDATE daily_quota SET reserved_bytes = MAX(reserved_bytes - ?3, 0) WHERE utc_date = ?1 AND ip_key = ?2').bind(utcDate, ipKey, bytes).run();
}

async function readJsonLimited(request) {
  const contentLength = Number(request.headers.get('content-length') || 0);
  if (contentLength > MAX_BODY_BYTES) return null;
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > MAX_BODY_BYTES) return null;
  try { return JSON.parse(text); } catch { return null; }
}

async function ipDigest(secret, ip, date) {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(String(secret || 'missing-secret')), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const digest = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${date}\0${ip}`));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

function numberEnv(value, fallback) {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : fallback;
}
