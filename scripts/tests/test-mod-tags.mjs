import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function setup() {
  const files = new Map();
  const root = 'G/Mods/.mod-tags/';
  files.set(root + 'tags.json', JSON.stringify([{ id: 'one', name: '采集物' }, { id: 'two', name: 'NSFW' }]));
  files.set(root + 'mod-mappings.json', JSON.stringify({ A: ['one'], B: ['one', 'two'] }));
  const failures = new Set();
  const fs = {
    exists: async (path) => files.has(path),
    mkdir: async () => {},
    readTextFile: async (path) => {
      await new Promise((resolve) => setTimeout(resolve, 1));
      if (failures.has(path)) throw new Error('拒绝访问');
      return files.get(path);
    },
    writeTextFile: async (path, data) => {
      if (failures.has(path)) throw new Error('写入失败');
      files.set(path, data);
    },
    rename: async (from, to) => { files.set(to, files.get(from)); files.delete(from); },
    readDir: async () => [],
  };
  const source = readFileSync(new URL('../../src/store/ModTagStore.ts', import.meta.url), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, require(name) {
    if (name === '@tauri-apps/plugin-fs') return fs;
    if (name === 'pinia') return { defineStore: (_, factory) => factory };
    if (name.endsWith('RecycleBin')) return { moveFileToRecycleBin: async () => {} };
    if (name.endsWith('ModManager')) return { ModManager: { getInstallDir: async () => 'G' } };
    throw new Error(name);
  } });
  return { store: exports.useModTagStore(), files, failures, root };
}

test('并发修改不同 Mod、分组及路径迁移不会互相覆盖', async () => {
  const { store } = setup();
  await Promise.all([
    store.setModTags('G', 'A', ['one', 'two']),
    store.setModTags('G', 'C', ['two']),
    store.setGroupTags('G', 'Group', ['one']),
    store.remapModPath('G', 'B', 'DISABLED_B'),
  ]);
  const snapshot = JSON.parse(JSON.stringify(await store.load('G')));
  assert.deepEqual(snapshot.modMappings, { A: ['one', 'two'], C: ['two'], DISABLED_B: ['one', 'two'] });
  assert.deepEqual(snapshot.groupMappings, { Group: ['one'] });
});

test('并发切换同一 Mod 的不同标签不会丢失更新，也不会嵌套死锁', async () => {
  const { store } = setup();
  await Promise.all([store.toggleModTag('G', 'C', 'one'), store.toggleModTag('G', 'C', 'two')]);
  assert.equal((await store.load('G')).modMappings.C.length, 2);
});

test('读取错误、无效 JSON 和错误结构阻止写入，修复后队列仍可使用', async () => {
  for (const broken of ['{', 'null', '[]']) {
    const { store, files, root, failures } = setup();
    const path = root + 'mod-mappings.json';
    const original = files.get(path);
    files.set(path, broken);
    await assert.rejects(store.setModTags('G', 'C', ['two']));
    assert.equal(files.get(path), broken);
    files.set(path, original);
    failures.add(path);
    await assert.rejects(store.setModTags('G', 'C', ['two']));
    assert.equal(files.get(path), original);
    failures.clear();
    await store.setModTags('G', 'C', ['two']);
    assert.equal((await store.load('G')).modMappings.C[0], 'two');
  }
});

test('临时文件写入失败保留原映射', async () => {
  const { store, files, root, failures } = setup();
  const path = root + 'mod-mappings.json';
  const original = files.get(path);
  failures.add(path + '.tmp');
  await assert.rejects(store.setModTags('G', 'C', ['two']));
  assert.equal(files.get(path), original);
});

test('目录刷新只读标签，不完整扫描不会清理映射', async () => {
  const { store, files } = setup();
  const before = JSON.stringify([...files]);
  await store.load('G');
  assert.equal(JSON.stringify([...files]), before);
  assert.equal(store.pruneMappings, undefined);
  const source = readFileSync(new URL('../../src/views/ModsManagement/ModsManagement.tags.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /pruneMappings/);
});
