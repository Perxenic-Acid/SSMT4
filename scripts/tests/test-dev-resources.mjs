import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { prepareDevResources, shouldPrepareDevResources } from '../dev/prepare-resources.mjs';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'ssmt-dev-resources-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (relative, bytes) => {
    const path = join(root, relative);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, bytes);
  };
  return { root, write, resource: name => join(root, 'src-tauri', 'resources', name) };
}

test('only real dev invocations prepare resources', () => {
  for (const args of [['dev'], ['--verbose', 'dev'], ['dev', '--', '--help']]) {
    assert.equal(shouldPrepareDevResources(args), true);
  }
  for (const args of [[], ['build'], ['signer', 'generate'], ['dev', '--help'], ['dev', '-h'], ['build', '--', 'dev']]) {
    assert.equal(shouldPrepareDevResources(args), false);
  }
});

test('missing resources are staged from the normal local build outputs', t => {
  const h = fixture(t);
  h.write('runtime/x64/Release/d3d11.dll', Buffer.from([0, 1, 255]));
  h.write('native/dist/Release/Run.exe', 'release injector');
  h.write('native/dist/Debug/Run.exe', 'debug injector');
  h.write('src-tauri/resources/SSMT-PluginHost.dll', 'existing host');
  const logs = [];
  prepareDevResources(h.root, message => logs.push(message));
  assert.deepEqual(readFileSync(h.resource('d3d11.dll')), Buffer.from([0, 1, 255]));
  assert.equal(readFileSync(h.resource('Run.exe'), 'utf8'), 'release injector');
  assert.equal(readFileSync(h.resource('SSMT-PluginHost.dll'), 'utf8'), 'existing host');
  assert.equal(logs.length, 2);
  prepareDevResources(h.root, message => logs.push(message));
  assert.equal(logs.length, 2, 'subsequent launches must not copy again');
});

test('existing resources are preserved without needing any build trees or plugins', t => {
  const h = fixture(t);
  for (const name of ['d3d11.dll', 'Run.exe', 'SSMT-PluginHost.dll']) h.write(`src-tauri/resources/${name}`, `custom ${name}`);
  h.write('runtime/x64/Release/d3d11.dll', 'newer runtime');
  const before = statSync(h.resource('d3d11.dll')).mtimeMs;
  prepareDevResources(h.root, () => assert.fail('existing resources must not be copied'));
  assert.equal(readFileSync(h.resource('d3d11.dll'), 'utf8'), 'custom d3d11.dll');
  assert.equal(statSync(h.resource('d3d11.dll')).mtimeMs, before);
  assert.equal(statSync(join(h.root, 'src-tauri/resources/PluginPackages'), { throwIfNoEntry: false }), undefined);
});

test('debug-only Native output can supply missing core tools', t => {
  const h = fixture(t);
  h.write('src-tauri/resources/d3d11.dll', 'custom runtime');
  h.write('native/dist/Debug/Run.exe', 'debug injector');
  h.write('native/dist/Debug/SSMT-PluginHost.dll', 'debug host');
  h.write('native/dist/Debug/SSMT-Player-Tweaks.dll', 'optional plugin');
  prepareDevResources(h.root, () => {});
  assert.equal(readFileSync(h.resource('Run.exe'), 'utf8'), 'debug injector');
  assert.equal(readFileSync(h.resource('SSMT-PluginHost.dll'), 'utf8'), 'debug host');
  assert.equal(statSync(h.resource('SSMT-Player-Tweaks.dll'), { throwIfNoEntry: false }), undefined);
});

test('a fresh checkout reports all missing components before staging anything', t => {
  const h = fixture(t);
  assert.throws(() => prepareDevResources(h.root), error => {
    for (const name of ['d3d11.dll', 'Run.exe', 'SSMT-PluginHost.dll', 'stage-ssmt-runtime.ps1 -Build', 'build_native.ps1']) {
      assert.ok(error.message.includes(name), name);
    }
    return true;
  });
  h.write('runtime/x64/Release/d3d11.dll', 'available runtime');
  assert.throws(() => prepareDevResources(h.root), /Run\.exe/);
  assert.equal(statSync(h.resource('d3d11.dll'), { throwIfNoEntry: false }), undefined);
});

test('a directory occupying a resource path is rejected without overwriting it', t => {
  const h = fixture(t);
  mkdirSync(h.resource('d3d11.dll'), { recursive: true });
  assert.throws(() => prepareDevResources(h.root), /开发资源路径不是文件/);
  assert.ok(statSync(h.resource('d3d11.dll')).isDirectory());
});

test('the real Tauri wrapper reports missing resources before launching the CLI', t => {
  const h = fixture(t);
  const cliName = process.platform === 'win32' ? 'tauri.cmd' : 'tauri';
  h.write(`node_modules/.bin/${cliName}`, 'This placeholder must never run');
  const wrapper = fileURLToPath(new URL('../tauri-with-signing.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [wrapper, 'dev'], { cwd: h.root, encoding: 'utf8', timeout: 10000 });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /\[dev-resources\]/);
  assert.match(result.stderr, /stage-ssmt-runtime\.ps1 -Build/);
  assert.match(result.stderr, /Run\.exe/);
});
