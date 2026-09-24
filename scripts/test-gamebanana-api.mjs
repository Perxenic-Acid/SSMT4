import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../src/views/GameBanana/gameBananaApi.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
const warning = '\nWarning: Undefined array key "images" in /home/publisher/live/gamebanana/classes/Library/Cache/TableRowCacher.php on line 87\n';
const comments = JSON.stringify({ _aRecords: [{ _idRow: 42, _sText: 'Keep Warning: and [brackets] inside comments' }] });
function harness(steps) {
  let calls = 0;
  const delays = [];
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, URLSearchParams, AbortController, DOMException,
    setTimeout(callback, ms) { if (ms !== 20000) { delays.push(ms); queueMicrotask(callback); } return 1; },
    clearTimeout() {},
    require(name) {
      assert.equal(name, '@tauri-apps/plugin-http');
      return { fetch: async () => {
        calls++;
        const step = steps.shift();
        assert.ok(step, 'Unexpected extra request');
        if (step instanceof Error) throw step;
        return step;
      } };
    },
  });
  return { api: exports, delays, get calls() { return calls; } };
}
function response(body, status = 200, retryAfter = null) {
  return { ok: status === 200, status, headers: { get: () => retryAfter }, body: { cancel: async () => {} }, text: async () => body };
}

test('recovers actual PHP warning format without altering comment contents', () => {
  const { api } = harness([]);
  assert.equal(JSON.stringify(api.parseGameBananaJson(warning.repeat(15) + comments)), comments);
  assert.equal(JSON.stringify(api.parseGameBananaJson('\uFEFF' + comments)), comments);
  assert.equal(JSON.stringify(api.parseGameBananaJson('<br />\n<b>Warning</b>: bad image<br />\n' + comments)), comments);
});
test('does not salvage HTML, truncated JSON or warning-only responses', () => {
  const { api } = harness([]);
  for (const invalid of ['<html>Denied</html>\n' + comments, warning, warning + '{"_aRecords": [', warning + comments + '\nWarning: trailing junk']) {
    assert.throws(() => api.parseGameBananaJson(invalid), /invalid JSON/);
  }
});
test('loads warning-prefixed comments on the first attempt', async () => {
  const h = harness([response(warning + comments)]);
  const result = await h.api.gameBananaApiGet('/Mod/123/Posts');
  assert.equal(result._aRecords[0]._idRow, 42);
  assert.equal(h.calls, 1);
});
test('retries transient HTTP, stream and incomplete profile failures before succeeding', async () => {
  const h = harness([response('', 503), new TypeError('stream closed'), response('{}'), response('{"_idRow":123}')]);
  assert.equal((await h.api.gameBananaApiGet('/Mod/123/ProfilePage'))._idRow, 123);
  assert.equal(h.calls, 4);
  assert.deepEqual(h.delays, [500, 1200, 2500]);
});
test('malformed responses stop after four attempts', async () => {
  const h = harness(Array.from({ length: 4 }, () => response(warning)));
  await assert.rejects(h.api.gameBananaApiGet('/Mod/123/Posts'), /invalid JSON/);
  assert.equal(h.calls, 4);
});
test('permanent HTTP and API errors do not retry', async () => {
  for (const r of [response('', 404), response('{"_sErrorMessage":"Private submission"}')]) {
    const h = harness([r]);
    await assert.rejects(h.api.gameBananaApiGet('/Mod/123/ProfilePage'));
    assert.equal(h.calls, 1);
  }
});
test('honors short Retry-After and avoids early retry for long rate limits', async () => {
  const h = harness([response('', 429, '3'), response(comments)]);
  await h.api.gameBananaApiGet('/Mod/123/Posts');
  assert.deepEqual(h.delays, [3000]);
  const long = harness([response('', 429, '60')]);
  await assert.rejects(long.api.gameBananaApiGet('/Mod/123/Posts'), /429/);
  assert.equal(long.calls, 1);
});
test('a superseded selection does not continue retrying or publish its response', async () => {
  let active = true;
  const h = harness([{ ...response(''), text: async () => { active = false; return comments; } }]);
  await assert.rejects(h.api.gameBananaApiGet('/Mod/123/Posts', {}, () => active), { name: 'AbortError' });
  assert.equal(h.calls, 1);
});

test('live GameBanana comments and profile parse through the production client', { skip: !process.argv.includes('--live') }, async () => {
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, URLSearchParams, AbortController, DOMException, setTimeout, clearTimeout,
    require: () => ({ fetch: globalThis.fetch }),
  });
  const [posts, profile] = await Promise.all([
    exports.gameBananaApiGet('/Mod/503336/Posts', { _nPage: '1', _nPerpage: '15' }),
    exports.gameBananaApiGet('/Mod/503336/ProfilePage'),
  ]);
  assert.ok(Array.isArray(posts._aRecords));
  assert.equal(profile._idRow, 503336);
});

test('update lists retry incomplete data and preserve update text', async () => {
  const record = { _idRow: 458111, _sName: 'File replacement', _sText: '<p>Correct file uploaded.</p>' };
  const h = harness([response('{}'), response(warning + JSON.stringify({ _aRecords: [record] }))]);
  const result = await h.api.gameBananaApiGet('/Mod/719997/Updates');
  assert.equal(result._aRecords[0]._sText, record._sText);
  assert.equal(h.calls, 2);
});

test('live supplied examples expose updates and individual file descriptions', { skip: !process.argv.includes('--live') }, async () => {
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, URLSearchParams, AbortController, DOMException, setTimeout, clearTimeout,
    require: () => ({ fetch: globalThis.fetch }),
  });
  const [updates, profile] = await Promise.all([
    exports.gameBananaApiGet('/Mod/719997/Updates', { _nPage: '1', _nPerpage: '10' }),
    exports.gameBananaApiGet('/Mod/720026/ProfilePage'),
  ]);
  assert.ok(updates._aRecords.some(update => update._sName && update._sText));
  assert.ok(profile._aFiles.length > 0);
  assert.ok(profile._aFiles.some(file => typeof file._sDescription === 'string' && file._sDescription.length > 0));
});

test('live sorting aliases match official config and sort the full filtered result set', { skip: !process.argv.includes('--live') }, async () => {
  const sortSource = readFileSync(new URL('../src/views/GameBanana/gameBananaSort.ts', import.meta.url), 'utf8');
  const sortCode = ts.transpileModule(sortSource, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText;
  const sorts = {};
  vm.runInNewContext(sortCode, { exports: sorts });
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, URLSearchParams, AbortController, DOMException, setTimeout, clearTimeout,
    require: () => ({ fetch: globalThis.fetch }),
  });
  const config = await exports.gameBananaApiGet('/Mod/ListFilterConfig');
  assert.deepEqual([...sorts.GAMEBANANA_SORTS].sort(), Array.from(config._aSorts, sort => sort._sAlias).sort());
  const fields = {
    Generic_Newest: ['_tsDateAdded', -1], Generic_Oldest: ['_tsDateAdded', 1],
    Generic_MostLiked: ['_nLikeCount', -1], Generic_MostViewed: ['_nViewCount', -1],
    Generic_MostDownloaded: ['_nDownloadCount', -1], Generic_MostCommented: ['_nPostCount', -1],
  };
  for (const sort of sorts.GAMEBANANA_SORTS) {
    const data = await exports.gameBananaApiGet('/Mod/Index', {
      _sSort: sort, _nPage: '1', _nPerpage: '3', '_aFilters[Generic_Game]': '8552',
    });
    assert.equal(data._aRecords.length, 3, sort);
    assert.ok(data._aRecords.every(row => row._aGame._idRow === 8552), sort);
    if (fields[sort]) {
      const [field, direction] = fields[sort];
      const values = data._aRecords.map(row => Number(row[field]));
      assert.ok(values.every(Number.isFinite), `${sort}: missing ${field}`);
      assert.ok(values.slice(1).every((value, i) => direction * (value - values[i]) >= 0), sort);
    }
  }
});
