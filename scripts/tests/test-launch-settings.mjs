import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const cache = new Map();
function load(name) {
  if (cache.has(name)) return cache.get(name);
  const source = readFileSync(new URL(`../../src/store/${name}.ts`, import.meta.url), 'utf8');
  const exports = {};
  const code = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS,
  } }).outputText;
  vm.runInNewContext(code, { exports, require: dependency => {
    assert.match(dependency, /^\.\/(AppTheme|XianZunProviders)$/);
    return load(dependency.slice(2));
  } });
  cache.set(name, exports);
  return exports;
}
const { AppSettings } = load('AppSettings');

test('主页游戏侧栏默认关闭，固定状态独立持久化', () => {
  assert.equal(new AppSettings({}).homeGameSidebarPinned, false);
  assert.equal(new AppSettings({ homeGameSidebarPinned: 'true' }).homeGameSidebarPinned, false);
  for (const homeGameSidebarPinned of [true, false]) {
    const settings = new AppSettings({ homeGameSidebarPinned, sidebarGameOrder: ['SRMI', 'GIMI'] });
    const restored = new AppSettings(JSON.parse(JSON.stringify(settings)));
    assert.equal(restored.homeGameSidebarPinned, homeGameSidebarPinned);
    assert.deepEqual([...restored.sidebarGameOrder], ['SRMI', 'GIMI']);
  }
});

test('旧配置迁移为默认控制台快捷键与自动叠加层语言', () => {
  const settings = new AppSettings({});
  assert.equal(settings.launchConsoleShortcut, 'Ctrl+Shift+L');
  assert.equal(settings.runtimeOverlayLanguage, 'auto');
  assert.equal(new AppSettings({ runtimeOverlayLanguage: 'invalid' }).runtimeOverlayLanguage, 'auto');
});

test('快捷键修改或禁用与三种叠加层语言独立持久化，不改变界面语言', () => {
  for (const launchConsoleShortcut of ['', 'Ctrl+Alt+KeyK']) {
    for (const runtimeOverlayLanguage of ['auto', 'zh_s', 'en']) {
      const settings = new AppSettings({ launchConsoleShortcut, runtimeOverlayLanguage, locale: 'ja' });
      const saved = JSON.parse(JSON.stringify(settings));
      const restored = new AppSettings(saved);
      assert.equal(restored.launchConsoleShortcut, launchConsoleShortcut);
      assert.equal(restored.runtimeOverlayLanguage, runtimeOverlayLanguage);
      assert.equal(restored.locale, settings.locale);
    }
  }
});
