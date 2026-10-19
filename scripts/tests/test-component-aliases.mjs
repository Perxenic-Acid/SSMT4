import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function load(relative, mocks = {}) {
  const exports = {};
  const code = ts.transpileModule(readFileSync(new URL(relative, import.meta.url), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  vm.runInNewContext(code, { exports, require(name) { assert.ok(name in mocks, name); return mocks[name]; } });
  return exports;
}

function fixture() {
  const files = new Map();
  const api = load('../../src/common/ComponentAliases.ts', {
    '@tauri-apps/api/path': { join: async (...parts) => path.posix.join(...parts) },
    '@tauri-apps/plugin-fs': {
      exists: async file => files.has(file), readTextFile: async file => files.get(file),
      writeTextFile: async (file, content) => files.set(file, content),
    },
  });
  return { files, ...api };
}

test('旧目录无别名可读，重命名及清空保持 Component 与其他 LOD 独立', async () => {
  const f = fixture();
  assert.equal(Object.keys(await f.readComponentAliases('/LOD0')).length, 0);
  await f.writeComponentAlias('/LOD0', '848c5980-16884-0', ' 面部.高光 ');
  await f.writeComponentAlias('/LOD0', '848c5980-25830-16884', '衣服');
  await f.writeComponentAlias('/LOD1', '848c5980-16884-0', '低模');
  assert.equal((await f.readComponentAliases('/LOD0'))['848c5980-16884-0'], '面部.高光');
  await f.writeComponentAlias('/LOD0', '848c5980-16884-0', '');
  assert.equal((await f.readComponentAliases('/LOD0'))['848c5980-16884-0'], undefined);
  assert.equal((await f.readComponentAliases('/LOD0'))['848c5980-25830-16884'], '衣服');
  assert.equal((await f.readComponentAliases('/LOD1'))['848c5980-16884-0'], '低模');
  assert.deepEqual([...f.files.keys()].sort(), ['/LOD0/ComponentAliases.json', '/LOD1/ComponentAliases.json']);
});

test('损坏元数据不被改名覆盖；别名接受 Unicode，拒绝超长文本、控制字符和非法标识', async () => {
  const f = fixture();
  f.files.set('/LOD0/ComponentAliases.json', 'broken');
  await assert.rejects(f.writeComponentAlias('/LOD0', '848c5980-3-0', 'Body'));
  assert.equal(f.files.get('/LOD0/ComponentAliases.json'), 'broken');
  for (const bad of ['a'.repeat(129), 'bad\nname', 'bad\u0000name']) assert.equal(f.isComponentAlias(bad), false);
  assert.equal(f.isComponentAlias('😀'.repeat(128)), true);
  await assert.rejects(f.writeComponentAlias('/LOD1', '../other', 'Body'));
});

test('三级树保持来源顺序，LOD 下的同一 IB 和重复别名均不串组', () => {
  const { buildComponentTree } = load('../../src/views/MarkTexture/ComponentTree.ts');
  const items = [['LOD0', 'a', 'one'], ['LOD0', 'a', 'two'], ['LOD0', 'b', 'three'], ['LOD1', 'a', 'four']]
    .map(([lod, ib, id]) => ({ value: id, tabId: lod, tabName: lod, drawIB: ib, ibLabel: ib, componentLabel: '相同别名' }));
  const tree = buildComponentTree(items);
  assert.equal(tree.length, 2);
  assert.equal(tree[0].ibs.length, 2);
  assert.equal(tree[0].ibs[0].items[1], items[1]);
  assert.equal(tree[1].ibs[0].items[0], items[3]);
  assert.notEqual(tree[0].ibs[0].key, tree[1].ibs[0].key);
});
