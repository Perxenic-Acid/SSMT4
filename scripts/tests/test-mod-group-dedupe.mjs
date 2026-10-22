import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function loadGroupPaths() {
  const source = readFileSync(new URL('../../src/views/ModsManagement/ModsManagement.paths.ts', import.meta.url), 'utf8');
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, require() {} });
  return exports;
}

const group = (id, path, enabled, modCount = 0) => ({
  id,
  name: id.split('/').pop(),
  path,
  enabled,
  modCount,
});

test('同 ID 的启用与禁用分组只保留启用版本，并合并计数和图标', () => {
  const { dedupeGroupsById } = loadGroupPaths();
  const iconPath = 'C:/icon.png';
  const groups = dedupeGroupsById([
    { ...group('Characters', 'DISABLED_Characters', false, 2), iconPath },
    group('Characters', 'Characters', true, 5),
  ]);

  assert.equal(groups.length, 1);
  assert.equal(groups[0].path, 'Characters');
  assert.equal(groups[0].enabled, true);
  assert.equal(groups[0].modCount, 5);
  assert.equal(groups[0].iconPath, iconPath);
});

test('禁用版本晚到不会覆盖启用版本，重复扫描保持原始顺序', () => {
  const { dedupeGroupsById } = loadGroupPaths();
  const groups = dedupeGroupsById([
    group('Characters', 'Characters', true, 1),
    group('Vehicles', 'Vehicles', false, 4),
    group('Characters', 'DISABLED_Characters', false, 9),
    group('Characters', 'Characters', true, 3),
  ]);

  assert.deepEqual([...groups.map((item) => item.id)], ['Characters', 'Vehicles']);
  assert.equal(groups[0].path, 'Characters');
  assert.equal(groups[0].enabled, true);
  assert.equal(groups[0].modCount, 3);
});

test('启用空分组不会继承同名禁用分组的 Mod 数量', () => {
  const { dedupeGroupsById } = loadGroupPaths();
  const groups = dedupeGroupsById([
    group('SSMTGeneratedMod', 'DISABLED_SSMTGeneratedMod', false, 11),
    group('SSMTGeneratedMod', 'SSMTGeneratedMod', true, 0),
  ]);

  assert.equal(groups.length, 1);
  assert.equal(groups[0].enabled, true);
  assert.equal(groups[0].path, 'SSMTGeneratedMod');
  assert.equal(groups[0].modCount, 0);
});
