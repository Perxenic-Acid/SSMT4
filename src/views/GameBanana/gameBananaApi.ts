import { fetch } from '@tauri-apps/plugin-http';

const API_BASE = 'https://gamebanana.com/apiv11';
const RETRY_DELAYS = [500, 1200, 2500];

class ApiFailure extends Error {
  constructor(message: string, readonly retryable = true, readonly retryAfter = 0) {
    super(message);
  }
}

/** Recover PHP diagnostics before a complete JSON value, never arbitrary HTML or broken JSON. */
export function parseGameBananaJson(body: string): unknown {
  const text = body.replace(/^\uFEFF/, '').trim();
  try { return JSON.parse(text); } catch { /* Inspect only a known diagnostic prefix. */ }
  if (/^(?:<br\s*\/?>\s*)?(?:<b>)?(?:PHP\s+)?(?:Warning|Notice|Deprecated|Strict Standards)(?:<\/b>)?:/i.test(text)) {
    const boundary = /(?:\r?\n|<br\s*\/?>)\s*(?=[{[])/i.exec(text);
    if (boundary) {
      try { return JSON.parse(text.slice(boundary.index + boundary[0].length)); } catch { /* Retry the response. */ }
    }
  }
  throw new ApiFailure('GameBanana returned an invalid JSON response');
}

function validatePayload(path: string, data: unknown): void {
  if (!data || typeof data !== 'object') throw new ApiFailure('GameBanana returned an empty response');
  const record = data as Record<string, unknown>;
  const error = record._sErrorMessage || record.error;
  if (typeof error === 'string' && error) throw new ApiFailure(error, false);
  if (/\/(Posts|Updates)$/.test(path) && !Array.isArray(record._aRecords)) {
    throw new ApiFailure('GameBanana returned an incomplete record list');
  }
  if (/\/ProfilePage$/.test(path) && !(typeof record._idRow === 'number' && record._idRow > 0)) {
    throw new ApiFailure('GameBanana returned an incomplete profile');
  }
}

export async function gameBananaApiGet<T>(
  path: string,
  params: Record<string, string> = {},
  isCurrent: () => boolean = () => true,
): Promise<T> {
  const checkCurrent = () => {
    if (!isCurrent()) throw new DOMException('Request superseded', 'AbortError');
  };
  const url = `${API_BASE}${path}?${new URLSearchParams(params)}`;
  for (let attempt = 0; ; attempt++) {
    checkCurrent();
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 20000);
    let failure: unknown;
    try {
      const response = await fetch(url, { method: 'GET', signal: controller.signal });
      if (!response.ok) {
        const retryAfter = response.headers.get('Retry-After');
        const seconds = retryAfter ? Number(retryAfter) : NaN;
        const delay = Number.isFinite(seconds) ? seconds * 1000
          : retryAfter ? Date.parse(retryAfter) - Date.now() : 0;
        await response.body?.cancel().catch(() => {});
        throw new ApiFailure(`GameBanana HTTP ${response.status}`,
          [408, 425, 429].includes(response.status) || response.status >= 500,
          Math.max(0, Number.isFinite(delay) ? delay : 0));
      }
      // Keep JSON parsing outside the native response stream controller.
      const data = parseGameBananaJson(await response.text());
      checkCurrent();
      validatePayload(path, data);
      return data as T;
    } catch (error) {
      checkCurrent();
      failure = error;
    } finally {
      clearTimeout(timeout);
    }
    if (attempt >= RETRY_DELAYS.length || (failure instanceof ApiFailure && !failure.retryable)) throw failure;
    const delay = Math.max(RETRY_DELAYS[attempt], failure instanceof ApiFailure ? failure.retryAfter : 0);
    // A long rate limit should surface rather than block the page or retry before the server permits it.
    if (delay > 30000) throw failure;
    await new Promise(resolve => setTimeout(resolve, delay));
  }
}
