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
  vm.runInNewContext(compiled, { exports, console, setTimeout, clearTimeout, require(name) {
    assert.ok(name in mocks, `Unexpected import: ${name}`);
    return mocks[name];
  } });
  return exports;
}

function harness(config, hostConfig = null, hotRuntime = null, settingsError = null, shadeHotRuntime = null, pluginInjectedDlls = [], backendError = null) {
  let ini = '[Loader]\nloader = XXMI Launcher.exe\nlaunch = C:/old.exe\nlaunch_args = old\nmodule = d3d11.dll\n';
  const calls = [];
  const messages = [];
  const t = (key, params) => key === 'launchGame.messages.graphicsPluginUnavailableContinue' ? `${key} ${JSON.stringify(params)}` : key;
  const mocks = {
    '@tauri-apps/api/path': { join: async (...parts) => parts.join('/') },
    '@tauri-apps/plugin-fs': {
      exists: async () => true,
      stat: async () => ({ isFile: true }),
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
    'element-plus': {
      ElMessage: Object.fromEntries(['info', 'success', 'error', 'warning'].map(kind => [kind, message => messages.push({ kind, message })])),
      ElMessageBox: { alert: async (...args) => { throw new Error(`Unexpected alert: ${args}`); } },
    },
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'plugin_backend_lifecycle') {
        if (backendError && args.event === 'prepare') throw backendError;
        return [];
      }
      if (command === 'cancel_runtime_deployment') return;
      if (command === 'prepare_plugin_host_config') return hostConfig;
      if (command === 'plugin_launch_injected_dlls') return { dlls: pluginInjectedDlls, warnings: [] };
      if (command === 'prepare_plugin_backend_launch') {
        if (typeof hotRuntime === 'function') return hotRuntime(args);
        return hotRuntime || { plans: [], diagnostics: [] };
      }
      if (command === 'launch_programs') return;
      if (command === 'hoyoplay_check_launch') return;
      if (command === 'find_game_executable') return null;
      assert.fail(`Unexpected IPC: ${command}`);
    } },
  };
  const iniMocks = { ...mocks };
  const iniManager = load('../../src/store/D3dxIniManager.ts', iniMocks);
  const migoto = load('../../src/store/MigotoManager.ts', {
    ...mocks,
    './GlobalConfig': {},
    './D3dxIniManager': iniManager,
    './ResourceManager': mocks['../store/ResourceManager'],
  }).MigotoManager;
  migoto.check3DmigotoIntegrity = async () => true;
  const launchPaths = load('../../src/common/LaunchPath.ts', mocks);
  const versions = load('../../src/common/UpdateCheckUtils.ts', {});
  const launch = load('../../src/common/LaunchGame.ts', { ...mocks, './LaunchPath': launchPaths, './UpdateCheckUtils': versions, '../store/MigotoManager': { MigotoManager: migoto } }).LaunchGame;
  launch.ensureSSMTPluginHostFile = async () => {};
  const preflight = launch.prepareLaunch.bind(launch);
  launch.prepareLaunch = async () => ({ migotoDir: 'C:/Migoto', config, targetExe: config.targetExePath });
  return { launch, migoto, preflight, calls, messages, files: mocks['@tauri-apps/plugin-fs'], ini: () => ini };
}

const base = { gamePreset: 'EFMI', targetExePath: 'C:/Game/Endfield.exe', launchArgs: '-force-d3d11', launchTargetProgram: true };

test('通用插件后台不可用时继续普通启动，纯净直启不执行后台生命周期', async () => {
  const failed = harness({ ...base, launcherExePath: 'C:/Official/Games.exe' }, null, null, null, null, [], new Error('backend unavailable'));
  await failed.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.ok(failed.calls.some(call => call.command === 'launch_programs'));
  assert.ok(failed.messages.some(message => message.kind === 'warning'));
  const pure = harness({ ...base, launcherExePath: 'C:/Official/Games.exe' });
  await pure.launch.launch('EFMI', { gameLaunchMode: 'always-pure' }, () => {});
  assert.equal(pure.calls.some(call => call.command === 'plugin_backend_lifecycle'), false);
});

for (const useShell of [false, true]) {
  for (const launchTargetProgram of [false, true]) {
    test(`EFMI queue: shell=${useShell}, automatic=${launchTargetProgram}`, async () => {
      const h = harness({ ...base, useShell, launchTargetProgram, launcherExePath: 'C:/Official/Games.exe' });
      await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
      const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
      const injector = programs.find(program => program.path.endsWith('/Run.exe'));
      const directTarget = launchTargetProgram && !useShell;
      assert.match(injector.args, /--inject-mode direct --wait-for-target/);
      assert.equal(programs.some(program => program.path === base.targetExePath), false);
      if (directTarget) {
        assert.match(injector.args, /--launch-parent "C:\/Official\/Games.exe"/);
        assert.match(injector.args, /--target-args "-force-d3d11"/);
      }
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

test('plugin-provided injected DLLs merge into SSMT Run.exe launch and force native injection', async () => {
  const h = harness({
    ...base,
    useShell: true,
    launcherExePath: 'C:/Official/Games.exe',
    extraDlls: ['C:/Migoto/d3d11.dll'],
  }, null, null, null, null, [
    { pluginId: 'ssmt.secondary-motion', pluginName: 'Secondary Motion', path: 'C:/Plugins/SBM/plugin/sbm.dll' },
  ]);
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs.some(program => program.path === 'C:/Official/Games.exe'), false);
  const injector = programs.find(program => program.path.endsWith('/Run.exe'));
  assert.match(injector.args, /--launch-parent "C:\/Official\/Games.exe"/);
  assert.match(h.ini(), /inject_dlls = C:\/Migoto\/d3d11.dll\|C:\/Plugins\/SBM\/plugin\/sbm.dll/);
});

test('EFMI direct launch falls back to -force-d3d11 when launch arguments are empty', async () => {
  const h = harness({ ...base, launchArgs: '', launcherExePath: 'C:/Official/Games.exe' });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  const injector = programs.find(program => program.path.endsWith('/Run.exe'));
  assert.match(injector.args, /--target-args "-force-d3d11"/);
});

test('EFMI direct launch requires an official launcher path', async () => {
  const h = harness({ ...base, launcherExePath: '' });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.equal(h.calls.some(call => call.command === 'launch_programs'), false);
  assert.equal(h.messages[0].kind, 'error');
});

test('disabled plugins add no host argument and no plugin-owned files', async () => {
  const h = harness({ ...base, launcherExePath: 'C:/Official/Games.exe' });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs.length, 1);
  assert.doesNotMatch(programs[0].args, /plugin-host/);
  assert.equal(programs[0].logSources, undefined);
});

test('other games retain suspended launch mode and their arguments', async () => {
  const h = harness({ ...base, gamePreset: 'GIMI' });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.equal(programs[0].args, '--machine-readable');
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
  const h = harness({ ...base, launcherExePath: 'C:/Official/Games.exe', postLaunchPrograms: [{ exePath: 'C:/After.exe', args: '' }] });
  await h.launch.launch('EFMI', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  const waitProgram = programs.find(program => program.waitOnly);
  assert.equal(waitProgram.waitTimeoutSecs, 120);
  assert.equal(programs.at(-1).path, 'C:/After.exe');
});

test('EFMI manual wait requires an explicit executable path', async () => {
  const h = harness({ ...base, launchTargetProgram: false, targetExePath: '' });
  assert.equal(await h.preflight('EFMI', {}, false), null);
  assert.equal(h.messages[0].kind, 'warning');
  assert.equal(h.calls.length, 0);
});

test('通用启动预检拒绝存在的目录和失效路径，未知文件名的真实文件仍可启动', async () => {
  for (const stat of [async () => ({ isFile: false }), async () => { throw Error('不存在'); }]) {
    const h = harness({ ...base, gamePreset: 'Custom', targetExePath: 'D:/empty-folder' });
    h.files.stat = stat;
    assert.equal(await h.preflight('Custom', {}, true), null);
    assert.equal(h.messages.at(-1).kind, 'error');
    assert.ok(!h.calls.some(c => c.command === 'launch_programs'));
  }
  const h = harness({ ...base, gamePreset: 'ZZMIDX12', targetExePath: 'D:/custom/unknown-name.exe' });
  const prepared = await h.preflight('Custom', {}, true);
  assert.equal(prepared.targetExe, 'D:/custom/unknown-name.exe');
  assert.ok(!h.calls.some(c => c.command === 'find_game_executable'));
});

test('绝区零两种预设在假路径时仅快速匹配，找不到时不会启动', async () => {
  for (const gamePreset of ['ZZMI', 'ZZMIDX12']) {
    const h = harness({ ...base, gamePreset, targetExePath: 'D:/missing.exe' });
    h.files.stat = async () => ({ isFile: false });
    assert.equal(await h.preflight(gamePreset, {}, true), null);
    const discovery = h.calls.find(c => c.command === 'find_game_executable');
    assert.equal(discovery.args.quickOnly, true);
    assert.ok(!h.calls.some(c => c.command === 'launch_programs'));
  }
});

const hotPlan = {
  dlls: ['C:/SSMT/Owned/ReShade64.dll'], runtimeDirectory: 'C:/SSMT/Owned',
  logSources: [{ name: 'ReShade', path: 'C:/SSMT/Owned/ReShade.log' }],
  environment: { RESHADE_BASE_PATH_OVERRIDE: 'C:/SSMT/Owned', RESHADE_DISABLE_GRAPHICS_HOOK: null },
};

const runtimePlan = (name, extra = {}) => ({ pluginId: `example.${name}`, pluginName: name, plan: {
  dlls: [{ path: `C:/Plugin/${name}.dll`, sha256: '0'.repeat(64) }], environment: {}, requiredLaunchArgs: [], excludedLaunchArgs: [], logSources: [], ...extra,
} });
for (const count of [0, 1, 3]) {
  test(`通用后台合并 ${count} 项启动贡献，保留 PluginHost 与普通额外 DLL`, async () => {
    const plans = Array.from({ length: count }, (_, index) => runtimePlan(`p${index}`, {
      environment: { [`PLUGIN_${index}`]: `C:/Plugin/${index}` }, logSources: [{ name: `p${index}`, path: `C:/Plugin/${index}.log` }],
    }));
    const h = harness({ ...base, gamePreset: 'SRMI', extraDlls: ['C:/User/custom.dll'] }, 'C:/Plugins/host.json', { plans, diagnostics: [] });
    await h.launch.launch('自定义游戏', { gameLaunchMode: 'always-normal' }, () => {});
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    const injector = programs.find(program => program.path.endsWith('/Run.exe'));
    assert.match(injector.args, /--plugin-host-config/);
    assert.equal(injector.args.includes('--preload-runtime'), count > 0);
    assert.match(h.ini(), /custom.dll/);
    for (let index = 0; index < count; index++) {
      assert.ok(h.ini().includes(`p${index}.dll`));
      assert.ok(injector.args.includes(`PLUGIN_${index}=C:/Plugin/${index}`));
    }
    assert.equal(injector.logSources?.length || 0, count);
    assert.equal(programs.filter(program => program.path.endsWith('/Run.exe')).length, 1);
    assert.ok(!h.messages.some(message => message.kind === 'warning'));
  });
}

test('通用启动计划仅修改本次参数，移除排斥项并保留无关参数', async () => {
  const config = { ...base, gamePreset: 'SRMI', launchArgs: '-dx12 -other 1' };
  const h = harness(config, null, { plans: [runtimePlan('args', { requiredLaunchArgs: ['-dx11'], excludedLaunchArgs: ['-dx12'] })], diagnostics: [] });
  await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.match(h.ini(), /launch_args = -other 1 -dx11/);
  assert.equal(config.launchArgs, '-dx12 -other 1');
});

test('后台返回故障诊断与回退计划时，展示故障并执行可用贡献', async () => {
  const h = harness({ ...base, gamePreset: 'SRMI' }, null, {
    plans: [runtimePlan('fallback')], diagnostics: [{ pluginId: 'example.broken', pluginName: 'Broken', error: '校验失败' }],
  });
  await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.match(h.ini(), /fallback.dll/);
  assert.equal(h.messages.filter(message => message.kind === 'warning').length, 1);
});

test('后台计划与用户额外 DLL 冲突时撤回状态并继续普通 Mod', async () => {
  const h = harness({ ...base, gamePreset: 'SRMI', extraDlls: ['C:/User/shared.dll'] }, null, { plans: [runtimePlan('shared')], diagnostics: [] });
  await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.ok(h.calls.some(call => call.command === 'plugin_backend_lifecycle' && call.args.event === 'launch.cancel'));
  assert.ok(h.calls.some(call => call.command === 'launch_programs'));
  assert.match(h.ini(), /C:\/User\/shared.dll/);
  assert.doesNotMatch(h.ini(), /C:\/Plugin\/shared.dll/);
});

test('外部启动器和手动等待保持原启动方式，向后台明确禁止预加载', async () => {
  for (const launchTargetProgram of [true, false]) {
    let allowed;
    const h = harness({ ...base, gamePreset: 'SRMI', useShell: true, launchTargetProgram, launcherExePath: 'C:/Official/Launcher.exe' }, null, args => {
      allowed = args.allowPreload; return { plans: [], diagnostics: [] };
    });
    await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
    assert.equal(allowed, false);
    assert.doesNotMatch(h.ini(), /C:\/Plugin/);
  }
});

test('纯净直启不请求后台启动计划', async () => {
  const h = harness({ ...base, gamePreset: 'SRMI' }, null, () => { throw Error('不应执行插件'); });
  await h.launch.launch('SRMI', { gameLaunchMode: 'always-pure' }, () => {});
  assert.ok(!h.calls.some(call => call.command === 'prepare_plugin_backend_launch'));
});
