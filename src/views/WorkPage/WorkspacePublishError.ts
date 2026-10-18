type Translate = (key: string, values?: Record<string, string>) => string;

const REASON_KEYS: Record<string, string> = {
  WORKSPACE_SERVICE_UNAVAILABLE: 'serviceUnavailable',
  WORKSPACE_SERVICE_REQUEST_FAILED: 'serviceRequestFailed',
  WORKSPACE_SERVICE_INVALID_RESPONSE: 'serviceInvalidResponse',
  UPLOAD_PART_FAILED: 'partFailed',
  UPLOAD_ETAG_MISSING: 'etagMissing',
  RATE_LIMIT_EXCEEDED: 'rateLimit',
  DAILY_BYTE_QUOTA_EXCEEDED: 'byteQuota',
  ARCHIVE_TOO_LARGE: 'archiveTooLarge',
  ARCHIVE_INVALID: 'archiveInvalid',
  GITHUB_PUBLISH_FAILED: 'githubPublishFailed',
  UPLOAD_INITIALIZATION_FAILED: 'initializationFailed',
  MISSING_REFERENCED_FILE: 'missingReference',
  UNSUPPORTED_FILE_EXTENSION: 'unsupportedExtension',
  MISSING_DRAWIB: 'missingDrawIB',
  WORKSPACE_CHANGED_DURING_ARCHIVE: 'workspaceChanged',
};

export function workspacePublishErrorDetails(error: unknown, t: Translate): string {
  const value = error !== null && typeof error === 'object'
    ? error as { message?: unknown; stage?: unknown; logPath?: unknown } : {};
  const message = typeof value.message === 'string' ? value.message : String(error);
  const code = message.match(/^[A-Z][A-Z0-9_]+(?=[:;\s]|$)/u)?.[0] || '';
  const reasonKey = REASON_KEYS[code];
  const lines = reasonKey ? [t(`workPage.publishErrors.${reasonKey}`), message] : [message];
  if (typeof value.stage === 'string' && ['archive', 'initialize', 'upload', 'finalize'].includes(value.stage)) {
    lines.unshift(t('workPage.publishErrors.stage', { stage: t(`workPage.publishErrors.stages.${value.stage}`) }));
  }
  if (typeof value.logPath === 'string' && value.logPath) {
    lines.push(t('workPage.publishErrors.logPath', { path: value.logPath }));
  }
  return lines.join('\n\n');
}
