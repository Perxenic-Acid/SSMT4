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

function harness(config, hostConfig = null, hotRuntime = null, settingsError = null, shadeHotRuntime = null) {
  let ini = '[Loader]\nloader = XXMI Launcher.exe\nlaunch = C:/old.exe\nlaunch_args = old\nmodule = d3d11.dll\n';
  const calls = [];
  const messages = [];
  const t = (key, params) => key === 'launchGame.messages.graphicsPluginUnavailableContinue' ? `${key} ${JSON.stringify(params)}` : key;
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
      if (command === 'prepare_plugin_graphics_plan') {
        const diagnostics = [];
        let handled = false;
        for (const [id, plugin, candidate] of [['dlss', 'DLSS5', hotRuntime], ['shade', 'Shade', shadeHotRuntime]]) {
          if (!candidate) continue;
          if (args.excludedProviders.includes(id)) { handled = true; continue; }
          if (!args.allowHotRuntime) { diagnostics.push({ candidateId: id, plugin, error: '当前启动方式无法继承热注入计划' }); continue; }
          handled = true;
          try {
            const plan = typeof candidate === 'function' ? candidate() : candidate;
            if (plan) return { candidateId: id, plugin, plan, handled, diagnostics };
          } catch (error) { diagnostics.push({ candidateId: id, plugin, error: String(error) }); }
        }
        return { candidateId: null, plugin: null, plan: null, handled, diagnostics };
      }
      if (command === 'configure_plugin_launch_contract') {
        if (settingsError && args.candidateId === 'dlss') throw settingsError;
        return;
      }
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

const hotPlan = {
  dlls: ['C:/SSMT/Owned/ReShade64.dll'], runtimeDirectory: 'C:/SSMT/Owned',
  environment: { RESHADE_BASE_PATH_OVERRIDE: 'C:/SSMT/Owned', RESHADE_DISABLE_GRAPHICS_HOOK: null },
};

test('Shade 单独启用或 DLSS5 准备失败时仍与 Runtime 有序预加载，不调用独立注入器和 AA 配置', async () => {
  for (const useShell of [false, true]) {
    for (const hostConfig of [null, 'C:/Plugins/host.json']) {
      for (const dlssPlan of [null, () => { throw new Error('NR 组件缺失'); }]) {
        const target = 'C:/Game/StarRail.exe';
        const h = harness({ ...base, gamePreset: 'SRMI', targetExePath: target, useShell, launcherExePath: target }, hostConfig, dlssPlan, null, hotPlan);
        await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
        const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
        const injector = programs.find(program => program.path.endsWith('/Run.exe'));
        assert.match(injector.args, /--preload-runtime/);
        assert.match(injector.args, /--set-env "RESHADE_BASE_PATH_OVERRIDE=/);
        assert.equal(programs.some(program => program.path === target), false);
        assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
        assert.equal(h.calls.some(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss'), false);
        assert.equal(injector.args.includes('--plugin-host-config'), !!hostConfig);
        assert.match(h.ini(), /inject_dlls = C:\/SSMT\/Owned\/ReShade64.dll/);
        assert.doesNotMatch(h.ini(), /dll_initialization_delay = 150/);
      }
    }
  }
});

test('Shade 热注入失败仅横幅提示并保留普通 Mod，纯净启动不准备 Shade', async () => {
  const failed = harness({ ...base, gamePreset: 'SRMI', targetExePath: 'C:/Game/StarRail.exe' }, null, null, null, () => { throw new Error('Shade 资源损坏'); });
  await failed.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.ok(failed.calls.some(call => call.command === 'launch_programs'));
  assert.equal(failed.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
  assert.equal(failed.messages.filter(message => message.kind === 'warning').length, 1);
  const pure = harness({ ...base }, null, null, null, hotPlan);
  await pure.launch.launch('SRMI', { gameLaunchMode: 'always-pure' }, () => {});
  assert.equal(pure.calls.some(call => call.command === 'prepare_plugin_graphics_plan'), false);
});

test('DLSS5 画质配置失败时 Shade 仍走有序预加载，不回退到独立注入器', async () => {
  const dlssPlan = { ...hotPlan, dlls: [...hotPlan.dlls, 'C:/SSMT/Owned/SSMT-Gimi-Bridge.dll'] };
  const h = harness({ ...base, gamePreset: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe' }, null, dlssPlan, new Error('备份画质失败'), hotPlan);
  await h.launch.launch('原神', { gameLaunchMode: 'always-normal' }, () => {});
  const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
  assert.match(programs[0].args, /--preload-runtime/);
  assert.match(h.ini(), /inject_dlls = C:\/SSMT\/Owned\/ReShade64.dll/);
  assert.doesNotMatch(h.ini(), /SSMT-Gimi-Bridge.dll/);
  assert.equal(h.calls.filter(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss').length, 1);
  assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
  assert.equal(h.messages.filter(message => message.kind === 'warning').length, 1);
});

test('共享故障按原因合并，列出所有受影响插件；不同原因仍分别显示', async () => {
  for (const sameReason of [true, false]) {
    const reason = 'ReShade 配置冲突：[DEPTH] UseAspectRatioHeuristics，已有值=1；新增值=0';
    const h = harness({ ...base, gamePreset: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe' }, null,
      () => { throw new Error(reason); }, null,
      () => { throw new Error(sameReason ? reason : 'HoYoShade 缺失 ReShade64.dll'); });
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    const warnings = h.messages.filter(message => message.kind === 'warning');
    assert.equal(warnings.length, sameReason ? 1 : 2);
    assert.ok(warnings[0].message.message.includes(reason));
    assert.ok(warnings[0].message.message.includes('DLSS5'));
    assert.ok(warnings[sameReason ? 0 : 1].message.message.includes('Shade'));
    assert.equal(warnings[0].message.grouping, true);
    assert.ok(h.calls.some(call => call.command === 'launch_programs'));
    assert.doesNotMatch(h.ini(), /C:\/SSMT\/Owned/);
  }
});

test('回退重试不再执行已报错候选，也不重复发出横幅', async () => {
  let attempts = 0;
  const h = harness({ ...base, gamePreset: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe', extraDlls: ['C:/Player/ReShade64.dll'] }, null,
    () => { attempts++; throw new Error('NR 组件校验失败：hash mismatch'); }, null, hotPlan);
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.equal(attempts, 1);
  assert.equal(h.messages.filter(message => message.kind === 'warning').length, 2);
  assert.ok(h.calls.some(call => call.command === 'launch_programs'));
  assert.doesNotMatch(h.ini(), /C:\/SSMT\/Owned/);
});

test('各游戏热注入计划合并进单次挂起启动，保留 PluginHost，不再启动第二套 Shade 注入器', async () => {
  for (const [preset, name] of [['SRMI', 'StarRail'], ['WWMI', 'Client-Win64-Shipping'], ['ZZMI', 'ZenlessZoneZero'], ['ZZMIDX12', 'ZenlessZoneZero']]) {
    const target = `C:/Game/${name}.exe`;
    const h = harness({ ...base, gamePreset: preset, targetExePath: target, useShell: true, launcherExePath: target }, 'C:/Plugins/host.json', hotPlan);
    await h.launch.launch(preset, { gameLaunchMode: 'always-normal' }, () => {});
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    assert.equal(programs.filter(program => program.path.endsWith('/Run.exe')).length, 1);
    assert.ok(!programs.some(program => program.path === target));
    assert.match(programs[0].args, /--preload-runtime/);
    assert.match(programs[0].args, /--set-env "RESHADE_BASE_PATH_OVERRIDE=C:\/SSMT\/Owned"/);
    assert.match(programs[0].args, /--unset-env RESHADE_DISABLE_GRAPHICS_HOOK/);
    assert.match(programs[0].args, /--plugin-host-config/);
    assert.match(h.ini(), /inject_dlls = C:\/SSMT\/Owned\/ReShade64.dll/);
    assert.ok(!h.calls.some(call => call.command === 'prepare_hoyoshade_launch'));
    assert.ok(!h.messages.some(message => message.kind === 'warning'));
  }
});

test('手动等待或外部启动器无法继承热注入环境时只跳过 DLSS5，保留原启动方式', async () => {
  for (const launchTargetProgram of [false, true]) {
    const launcher = 'C:/Official/Launcher.exe';
    const h = harness({ ...base, gamePreset: 'SRMI', targetExePath: 'C:/Game/StarRail.exe', useShell: true, launchTargetProgram, launcherExePath: launcher }, null, hotPlan);
    await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    assert.doesNotMatch(programs[0].args || '', /--preload-runtime|--set-env/);
    assert.equal(h.calls.some(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss'), false);
    assert.equal(programs.some(program => program.path === launcher), launchTargetProgram);
    assert.ok(h.messages.some(message => message.kind === 'warning'));
  }
});

test('热注入组件准备失败仍提交普通启动队列并传递插件跳过状态', async () => {
  const h = harness({ ...base, gamePreset: 'SRMI', targetExePath: 'C:/Game/StarRail.exe' }, null, () => { throw new Error('组件损坏'); });
  await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.ok(h.calls.some(call => call.command === 'launch_programs'));
  assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
  assert.ok(h.messages.some(message => message.kind === 'warning'));
  assert.equal(h.calls.some(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss'), false);
});

test('只有实际采用热注入计划才调整画质契约，写入失败仅跳过 DLSS5', async () => {
  for (const settingsError of [null, new Error('无法备份画质设置')]) {
    const h = harness({ ...base, gamePreset: 'GIMI', targetExePath: 'C:/Game/YuanShen.exe' }, null, hotPlan, settingsError);
    await h.launch.launch('原神', { gameLaunchMode: 'always-normal' }, () => {});
    const settingsIndex = h.calls.findIndex(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss');
    const launchIndex = h.calls.findIndex(call => call.command === 'launch_programs');
    assert.ok(settingsIndex >= 0 && launchIndex > settingsIndex);
    const programs = h.calls[launchIndex].args.programs;
    assert.equal(/--preload-runtime/.test(programs[0].args || ''), !settingsError);
    assert.equal(h.messages.some(message => message.kind === 'warning'), !!settingsError);
  }
  for (const gameLaunchMode of ['always-normal', 'always-pure']) {
    const h = harness({ ...base, gamePreset: 'GIMI' });
    await h.launch.launch('原神', { gameLaunchMode }, () => {});
    assert.equal(h.calls.some(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss'), false);
  }
});


test('可选热注入 DLL 冲突仅提示并继续普通 Mod，不调整画质契约', async () => {
  for (const shadeOnly of [false, true]) {
    const plan = { dlls: ['C:/Owned/ReShade64.dll'], environment: {}, runtimeDirectory: 'C:/Owned' };
    const config = { ...base, gamePreset: 'SRMI', targetExePath: 'C:/Game/StarRail.exe', extraDlls: ['C:/Player/ReShade64.dll'] };
    const h = harness(config, null, shadeOnly ? null : plan, null, shadeOnly ? plan : null);
    await h.launch.launch('SRMI', { gameLaunchMode: 'always-normal' }, () => {});
    assert.ok(h.calls.some(call => call.command === 'launch_programs'));
    assert.equal(h.calls.some(call => call.command === 'configure_plugin_launch_contract' && call.args.candidateId === 'dlss'), false);
    if (shadeOnly) assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
    assert.ok(h.messages.some(message => message.kind === 'warning'));
    assert.doesNotMatch(h.ini(), /C:\/Owned/);
    assert.match(h.ini(), /C:\/Player\/ReShade64.dll/);
  }
});

test('鸣潮热路线仅本次约束 DX11，保留其他参数和保存配置', async () => {
  const original = '-dx12 -tracking_code="keep spaces" -custom=1';
  const config = { ...base, gamePreset: 'WWMI', targetExePath: 'C:/Game/Client-Win64-Shipping.exe', launchArgs: original };
  const plan = { dlls: ['C:/Owned/ReShade64.dll'], environment: {}, runtimeDirectory: 'C:/Owned', requiredLaunchArgs: ['-dx11'], excludedLaunchArgs: ['-dx12', '-d3d12', '-vulkan'] };
  const h = harness(config, null, plan);
  await h.launch.launch('WWMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.match(h.ini(), /launch_args = -tracking_code="keep spaces" -custom=1 -dx11/);
  assert.equal(config.launchArgs, original);
  const disabled = harness(config);
  await disabled.launch.launch('WWMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.match(disabled.ini(), /launch_args = -dx12/);
});
