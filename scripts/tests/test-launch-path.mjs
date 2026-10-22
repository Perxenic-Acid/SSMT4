import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../../src/common/LaunchPath.ts', import.meta.url), 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
function load(stat) {
  const exports = {};
  vm.runInNewContext(source, { exports, setTimeout, clearTimeout, require: () => ({ stat }) });
  return exports.isExistingLaunchFile;
}

test('空白、目录和已删除文件无效，真实文件不限制游戏名称', async () => {
  const root = mkdtempSync(join(tmpdir(), 'SSMT-launch-path-'));
  try {
    const directory = join(root, 'ZenlessZoneZero.exe');
    const file = join(root, 'custom-name.bin');
    mkdirSync(directory); writeFileSync(file, 'fixture');
    let reads = 0;
    const isFile = load(async path => { reads++; return { isFile: statSync(path).isFile() }; });
    assert.equal(await isFile('   '), false);
    assert.equal(reads, 0);
    assert.equal(await isFile(directory), false);
    assert.equal(await isFile(join(root, 'missing.exe')), false);
    assert.equal(await isFile(file), true);
    rmSync(file);
    assert.equal(await isFile(file), false);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('网络或磁盘检查挂起时有等待上限，迟到结果不能重新改变判定', async () => {
  let complete;
  const isFile = load(() => new Promise(resolve => { complete = resolve; }));
  assert.equal(await isFile('Z:/unavailable/game.exe', 5), false);
  complete({ isFile: true });
  await Promise.resolve();
});
