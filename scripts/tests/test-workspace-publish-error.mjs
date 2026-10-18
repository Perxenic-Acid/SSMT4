import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../../src/views/WorkPage/WorkspacePublishError.ts', import.meta.url), 'utf8');
const exports = {};
vm.runInNewContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText, { exports });
const messages = JSON.parse(readFileSync(new URL('../../src/i18n/locales/zhs.json', import.meta.url))).workPage.publishErrors;
const t = (key, values = {}) => {
  let result = key.replace('workPage.publishErrors.', '').split('.').reduce((value, part) => value[part], messages);
  for (const [name, value] of Object.entries(values)) result = result.replace(`{${name}}`, value);
  return result;
};

test('shows final validation stage, HTTP reason and diagnostic log', () => {
  const text = exports.workspacePublishErrorDetails({ stage: 'finalize', message: 'ARCHIVE_INVALID: HTTP 422; ARCHIVE_MANIFEST_INVALID', logPath: 'C:/cache/Logs/upload.log' }, t);
  assert.match(text, /服务端校验与发布/u);
  assert.match(text, /HTTP 422; ARCHIVE_MANIFEST_INVALID/u);
  assert.match(text, /C:\/cache\/Logs\/upload.log/u);
});

test('keeps missing-file paths and unknown error details', () => {
  assert.match(exports.workspacePublishErrorDetails('MISSING_REFERENCED_FILE: LOD0/头发.buf', t), /LOD0\/头发.buf/u);
  assert.equal(exports.workspacePublishErrorDetails(new Error('future failure'), t), 'future failure');
});

test('localizes daily quotas and network failures instead of hiding error codes', () => {
  assert.match(exports.workspacePublishErrorDetails('DAILY_BYTE_QUOTA_EXCEEDED: HTTP 429', t), /今天的上传容量/u);
  assert.match(exports.workspacePublishErrorDetails('WORKSPACE_SERVICE_UNAVAILABLE: timeout', t), /timeout/u);
});
