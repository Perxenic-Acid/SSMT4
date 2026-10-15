import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function load(relative, mocks) {
  const source = readFileSync(new URL(relative, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(compiled, { exports, console, require(name) {
    assert.ok(name in mocks, `Unexpected import: ${name}`);
    return mocks[name];
  } });
  return exports;
}

function harness(config, hostConfig = null) {
  let ini = '[Loader]\nloader = XXMI Launcher.exe\nlaunch = C:/old.exe\nlaunch_args = old\nmodule = d3d11.dll\n';
  const calls = [];
  const messages = [];
  const t = key => key;
  const mocks = {
    '@tauri-apps/api/path': { join: async (...parts) => parts.join('/') },
    '@tauri-apps/plugin-fs': {
      exists: async () => true,
      readTextFile: async () => ini,
      writeTextFile: async (_path, value) => { ini = value; },
    },
    '../i18n': { i18n: { global: { t } } },
    '../helper/PathHelper': { PathHelper: { GetCurrentGame3DmigotoFolderPath: async () => 'C:/Migoto' } },
    '../store/ResourceManager': { ResourceManager: {
      loadGameConfig: async () => config,
      resolveMigotoDllSource: async () => ({ targetFileName: 'd3d11.dll' }),
    } },
    '../store/GlobalConfig': {},
    '../store/GameConfig': {},
    '../utils/debugLog': { debugLog() {}, debugWarn() {} },
    '../plugin/playerTweaksCamera': {},
    '../plugin/playerTweaksGameplay': {},
    'element-plus': {
      ElMessage: Object.fromEntries(['info', 'success', 'error', 'warning'].map(kind => [kind, message => messages.push({ kind, message })])),
      ElMessageBox: { alert: async (...args) => { throw new Error(`Unexpected alert: ${args}`); } },
    },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'prepare_plugin_host_config') return hostConfig;
      if (command === 'prepare_hoyoshade_launch') return null;
      if (command === 'launch_programs') return;
      assert.fail(`Unexpected IPC: ${command}`);
    } },
  };
  const iniMocks = { ...mocks };
  const iniManager = load('../../src/store/D3dxIniManager.ts', iniMocks);
  const migoto = load('../../src/store/MigotoManager.ts', {
    ...mocks,
    './D3dxIniManager': iniManager,
    './ResourceManager': mocks['../store/ResourceManager'],
  }).MigotoManager;
  migoto.check3DmigotoIntegrity = async () => true;
  const launch = load('../../src/common/LaunchGame.ts', { ...mocks, '../store/MigotoManager': { MigotoManager: migoto } }).LaunchGame;
  launch.ensureSSMTRuntimeFiles = async () => {};
  launch.ensureSSMTPluginHostFile = async () => {};
  const preflight = launch.prepareLaunch.bind(launch);
  launch.prepareLaunch = async () => ({ migotoDir: 'C:/Migoto', config, targetExe: config.targetExePath, graphicsDecision: 'continue_this_launch' });
  return { launch, migoto, preflight, calls, messages, ini: () => ini };
}

const base = { gamePreset: 'EFMI', targetExePath: 'C:/Game/Endfield.exe', launchArgs: '-force-d3d11', launchTargetProgram: true };

for (const useShell of [false, true]) {
  for (const launchTargetProgram of [false, true]) {
    test(`EFMI queue: shell=${useShell}, automatic=${launchTargetProgram}`, async () => {
      const h = harness({ ...base, useShell, launchTargetProgram, launcherExePath: 'C:/Official/Games.exe' });
      await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
      const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
      const injector = programs.find(program => program.path.endsWith('/Run.exe'));
      assert.match(injector.args, /--inject-mode direct --wait-for-target/);
      assert.equal(programs.some(program => program.path === base.targetExePath), false);
      assert.equal(programs.some(program => program.path === 'C:/Official/Games.exe'), useShell && launchTargetProgram);
      assert.equal(programs.some(program => program.waitOnly), false);
      assert.doesNotMatch(h.ini(), /^\s*launch\s*=/m);
      assert.doesNotMatch(h.ini(), /^\s*launch_args\s*=/m);
      assert.doesNotMatch(h.ini(), /^\s*loader\s*=/m);
      assert.match(h.ini(), /target = C:\/Game\/Endfield.exe/);
    });
  }
}

test('enabled PluginHost config is passed to the external injector', async () => {
  const h = harness({ ...base, launchTargetProgram: false, extraDlls: ['C:/Extra/a.dll', 'C:/Extra/b.dll'] }, 'C:/Plugins/host.json');
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const injector = h.calls.find(call => call.command === 'launch_programs').args.programs[0];
  assert.match(injector.args, /--plugin-host-config "C:\/Plugins\/host.json"/);
  assert.match(injector.args, /--inject-mode direct/);
  assert.match(h.ini(), /inject_dlls = C:\/Extra\/a.dll\|C:\/Extra\/b.dll/);
});

test('disabled plugins add no host argument and no plugin-owned files', async () => {
  const h = harness({ ...base });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs.length, 1);
  assert.doesNotMatch(programs[0].args, /plugin-host/);
});

test('other games retain suspended launch mode and their arguments', async () => {
  const h = harness({ ...base, gamePreset: 'GIMI' });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs[0].args, undefined);
  assert.match(h.ini(), /launch = C:\/Game\/Endfield.exe/);
  assert.match(h.ini(), /launch_args = -force-d3d11/);
  assert.equal(programs[1].waitOnly, true);
});

test('manual mode removes stale launch directives for other presets too', async () => {
  const h = harness({ ...base, gamePreset: 'GIMI', launchTargetProgram: false });
  await h.migoto.patchD3dxForLaunch('GIMI');
  assert.doesNotMatch(h.ini(), /^\s*launch(?:_args)?\s*=/m);
});

test('pure launch still skips runtime configuration and injection', async () => {
  const h = harness({ ...base });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-pure' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs.length, 1);
  assert.equal(programs[0].path, base.targetExePath);
  assert.equal(h.calls.some(call => call.command === 'prepare_plugin_host_config'), false);
  assert.match(h.ini(), /launch = C:\/old.exe/);
});

test('post-launch programs get the full external-launch timeout', async () => {
  const h = harness({ ...base, postLaunchPrograms: [{ exePath: 'C:/After.exe', args: '' }] });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs[1].waitTimeoutSecs, 120);
  assert.equal(programs[2].path, 'C:/After.exe');
});

test('EFMI manual wait requires an explicit executable path', async () => {
  const h = harness({ ...base, launchTargetProgram: false, targetExePath: '' });
  assert.equal(await h.preflight('EFMI', {}, false), null);
  assert.equal(h.messages[0].kind, 'warning');
  assert.equal(h.calls.length, 0);
});
