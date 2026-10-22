import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../../src/features/gameMaintenance/entry.ts', import.meta.url), 'utf8');
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
const exports = {};
vm.runInNewContext(code, { exports });
const { resolveClientEntry, supportsGameMaintenance } = exports;

test('首批仅三个米家游戏，两个绝区零预设共享入口', () => {
  for (const preset of ['GIMI', 'SRMI', 'ZZMI', 'ZZMIDX12']) assert.equal(supportsGameMaintenance(preset), true);
  for (const preset of ['HIMI', 'WWMI', 'EFMI', undefined]) assert.equal(supportsGameMaintenance(preset), false);
});
test('非适配游戏不探测文件或触发自动匹配', async () => {
  const unexpected = () => { throw Error('不应调用'); };
  assert.equal((await resolveClientEntry({ gamePreset: 'WWMI' }, { isFile: unexpected, discover: unexpected })).needsInstallation, false);
});
test('可用的手动进程路径优先，不触发匹配和安装', async () => {
  const result = await resolveClientEntry({ gamePreset: 'GIMI', targetExePath: 'D:/game/YuanShen.exe' }, {
    isFile: async path => path === 'D:/game/YuanShen.exe', discover: () => { throw Error('不应匹配'); },
  });
  assert.equal(result.needsInstallation, false); assert.equal(result.discovered, undefined);
});
test('空路径或已失效路径优先复用自动匹配到的现有客户端', async () => {
  for (const targetExePath of ['', 'D:/removed/StarRail.exe']) {
    let queried = '';
    const result = await resolveClientEntry({ gamePreset: 'srmi', targetExePath }, {
      isFile: async path => path === 'E:/game/StarRail.exe',
      discover: async preset => { queried = preset; return { targetExePath: 'E:/game/StarRail.exe', launcherExePath: 'E:/game/StarRail.exe' }; },
    });
    assert.equal(queried, 'SRMI'); assert.equal(result.needsInstallation, false);
    assert.equal(result.discovered.targetExePath, 'E:/game/StarRail.exe');
  }
});
test('未匹配、匹配结果已失效或匹配报错时进入安装入口', async () => {
  for (const discover of [async () => null, async () => ({ targetExePath: 'missing.exe' }), async () => { throw Error('扫描失败'); }]) {
    assert.equal((await resolveClientEntry({ gamePreset: 'ZZMIDX12' }, { isFile: async () => false, discover })).needsInstallation, true);
  }
});
