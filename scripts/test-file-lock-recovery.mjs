import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the production async flow while controlling IPC and dialog responses.
const source = readFileSync(new URL('../src/utils/FileLocks.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
const from = "C:/Mods/O'Brien/skin";
const to = "C:/Mods/O'Brien/DISABLED_skin";
const root = 'C:/Mods';
const terminal = { pid: 42, started: '123', executable: 'C:\\PowerShell\\pwsh.exe', name: 'pwsh.exe', canTerminate: true };
const report = (owners = [terminal], paths = [from]) => ({ paths, owners, warnings: [] });
const locked = { error: 'FILE_LOCKED: sharing violation' };

function harness(steps, dialogs) {
  const messages = [];
  const dialog = async (kind, body, title, options) => {
    const expected = dialogs.shift();
    assert.ok(expected, `Unexpected ${kind} dialog: ${title}`);
    assert.equal(kind, expected.kind);
    if (expected.title) assert.equal(title, expected.title);
    messages.push({ body, title, options });
    if (expected.action) throw expected.action;
  };
  const mocks = {
    '../i18n': { i18n: { global: { t: (key, params) => key + (params ? JSON.stringify(params) : '') } } },
    vue: { h: (...args) => args },
    'element-plus': { ElMessageBox: { confirm: (...args) => dialog('confirm', ...args), alert: (...args) => dialog('alert', ...args) } },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      const step = steps.shift();
      assert.ok(step, `Unexpected IPC: ${command}`);
      assert.equal(command, step.command);
      step.check?.(args);
      if (step.error) throw step.error;
      return step.value;
    } },
  };
  const exports = {};
  vm.runInNewContext(compiled, { exports, require: name => { assert.ok(name in mocks); return mocks[name]; } });
  return { api: exports, messages, done() { assert.equal(steps.length, 0); assert.equal(dialogs.length, 0); } };
}
const rename = extra => ({ command: 'rename_mod_with_retry', ...extra });
const query = value => ({ command: 'query_file_locks', value });

test('moving a terminal out retries the original rename without killing it', async () => {
  const h = harness([rename(locked), query(report()), rename({ check: args => {
    assert.equal(args.source, from); assert.equal(args.destination, to); assert.equal(args.watchRoot, root);
  } })], [{ kind: 'confirm', title: 'fileLocks.terminalTitle' }]);
  await h.api.renameModWithLockRecovery(from, to, root);
  assert.ok(JSON.stringify(h.messages).includes(to));
  h.done();
});

test('closing the terminal prompt cancels without termination or a second rename', async () => {
  const h = harness([rename(locked), query(report())], [{ kind: 'confirm', action: 'close' }]);
  await assert.rejects(h.api.renameModWithLockRecovery(from, to, root), /fileLocks.cancelled/);
  h.done();
});

test('choosing force still requires a fresh owner list and explicit confirmation', async () => {
  const fresh = { ...terminal, pid: 99, started: '456' };
  const h = harness([
    rename(locked), query(report()), rename(locked), query(report([fresh])),
    { command: 'terminate_file_lock_owners', check: args => {
      assert.equal(args.approved.length, 1); assert.equal(args.approved[0].pid, 99);
    } }, rename(),
  ], [
    { kind: 'confirm', title: 'fileLocks.terminalTitle', action: 'cancel' },
    { kind: 'confirm', title: 'fileLocks.confirmTitle' },
    { kind: 'alert', title: 'fileLocks.afterRename', action: 'close' },
  ]);
  await h.api.renameModWithLockRecovery(from, to, root);
  h.done();
});

test('cancelling force confirmation never ends a process', async () => {
  const h = harness([rename(locked), query(report()), rename(locked), query(report())], [
    { kind: 'confirm', action: 'cancel' }, { kind: 'confirm', action: 'cancel' },
  ]);
  await assert.rejects(h.api.renameModWithLockRecovery(from, to, root), /fileLocks.cancelled/);
  h.done();
});

test('terminal release during the prompt avoids even the force confirmation', async () => {
  const h = harness([rename(locked), query(report()), rename()], [{ kind: 'confirm', action: 'cancel' }]);
  await h.api.renameModWithLockRecovery(from, to, root);
  h.done();
});

test('DLL lookup sends full game paths and does not terminate when no owners exist', async () => {
  const directory = 'C:/Games/GIMI/3Dmigoto';
  const h = harness(['d3dcompiler_47.dll', 'd3d11.dll'].map(name => ({
    command: 'query_file_locks', value: report([], [`${directory}/${name}`]),
    check: args => assert.equal(args.paths[0], `${directory}/${name}`),
  })), [{ kind: 'alert', title: 'fileLocks.diagnosis' }]);
  await h.api.diagnoseDllLocks(directory);
  h.done();
});
