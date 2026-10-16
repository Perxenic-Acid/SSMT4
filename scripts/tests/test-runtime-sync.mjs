import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import * as fs from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

function load(relative, mocks) {
  const source = readFileSync(new URL(relative, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(compiled, { exports, console: { ...console, warn() {} }, require(name) {
    assert.ok(name in mocks, `Unexpected import: ${name}`);
    return mocks[name];
  } });
  return exports;
}

async function harness(t, options = {}) {
  const root = await fs.mkdtemp(path.join(tmpdir(), 'ssmt-runtime-test-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const resourceDir = path.join(root, 'resources');
  const migotoDir = path.join(root, 'migoto');
  await fs.mkdir(path.join(resourceDir, 'DX12'), { recursive: true });
  await fs.mkdir(migotoDir);
  const source = path.join(resourceDir, 'd3d11.dll');
  const target = path.join(migotoDir, 'd3d11.dll');
  await fs.writeFile(source, 'adapted SSMT Runtime');
  if (!options.noExisting) await fs.writeFile(target, 'previous working Runtime');
  for (const name of ['Run.exe', 'd3dcompiler_47.dll']) {
    await fs.writeFile(path.join(resourceDir, name), `new ${name}`);
    await fs.writeFile(path.join(migotoDir, name), `old ${name}`);
  }
  const gameExe = path.join(root, 'Game.exe');
  await fs.writeFile(gameExe, 'game fixture');
  await fs.writeFile(path.join(migotoDir, 'd3dx.ini'), '[Loader]\nmodule = d3d11.dll\n');
  const config = { gamePreset: options.gamePreset || 'GIMI', targetExePath: gameExe, installDir: migotoDir, useUpx: !!options.useUpx, configureGame: options.configureGame, ...options.config };
  if (config.useShell && !config.launcherExePath) config.launcherExePath = gameExe;
  if (options.hostConfig) await fs.writeFile(path.join(resourceDir, 'SSMT-PluginHost.dll'), 'host fixture');
  const calls = [];
  const messages = [];
  const dialogs = [];
  const writes = [];
  const fsMocks = {
    exists: async file => fs.access(file).then(() => true, () => false),
    mkdir: fs.mkdir,
    readTextFile: file => fs.readFile(file, 'utf8'),
    writeTextFile: (file, text) => fs.writeFile(file, text),
    copyFile: async (from, to) => {
      writes.push({ from, to });
      if (options.fail === 'copy' && from === source) throw new Error('copy denied');
      await fs.copyFile(from, to);
      if (options.fail === 'integrity' && from === source) await fs.writeFile(to, 'truncated');
    },
    rename: async (from, to) => {
      if (to === target && ['replace', 'cleanup'].includes(options.fail)) throw new Error('file in use');
      if (to === path.join(migotoDir, 'Run.exe') && options.fail === 'run-replace') throw new Error('injector in use');
      await fs.rename(from, to);
    },
    remove: async file => {
      assert.notEqual(file, target, 'The existing DLL must never be removed');
      if (options.fail === 'cleanup') throw new Error('cleanup denied');
      await fs.rm(file);
    },
  };
  const globalConfig = { SSMTResourcesFolder: async () => resourceDir };
  const mocks = {
    '@tauri-apps/api/path': { join: async (...parts) => path.join(...parts) },
    '@tauri-apps/plugin-fs': fsMocks,
    '@tauri-apps/api/core': { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'file_md5') {
        if (options.fail === 'hash' && args.path === source) throw new Error('hash denied');
        return createHash('md5').update(await fs.readFile(args.path)).digest('hex');
      }
      if (command === 'inspect_graphics_launch') return { hostRequested: false, resolution: { level: 'compatible', issues: [] } };
      if (command === 'bundled_player_tweaks_enabled_for_game') return false;
      if (command === 'configure_gimi_launch_settings') return null;
      if (command === 'prepare_dlss5_hot_runtime') {
        if (options.hotError) throw new Error(options.hotError);
        return options.hotRuntime ?? null;
      }
      if (command === 'prepare_plugin_host_config') return options.hostConfig ?? null;
      if (command === 'prepare_hoyoshade_launch') {
        if (options.shadeError) throw new Error(options.shadeError);
        return options.shade ?? null;
      }
      if (command === 'launch_programs') return null;
      if (command === 'execute_external_program') return options.upxResult?.(args) ?? { code: 1, stdout: '', stderr: 'UPX failed' };
      assert.fail(`Unexpected IPC: ${command}`);
    } },
    '../i18n': { i18n: { global: { t: (key, args) => `${key}${args ? ` ${JSON.stringify(args)}` : ''}` } } },
    '../helper/PathHelper': { PathHelper: { GetCurrentGame3DmigotoFolderPath: async () => migotoDir } },
    '../store/GlobalConfig': { GlobalConfig: globalConfig },
    '../store/GameConfig': { getEffectiveUseUpx: value => value.useUpx },
    '../utils/debugLog': { debugLog() {}, debugWarn() {} },
    '../plugin/playerTweaksCamera': {},
    '../plugin/playerTweaksGameplay': {},
    'element-plus': {
      ElMessage: Object.fromEntries(['info', 'success', 'error', 'warning'].map(kind => [kind, value => messages.push({ kind, value })])),
      ElMessageBox: Object.fromEntries(['confirm', 'alert'].map(kind => [kind, async (...args) => { dialogs.push({ kind, args }); }])),
    },
  };
  const resourceManager = load('../../src/store/ResourceManager.ts', {
    ...mocks,
    pinia: { defineStore: (_id, setup) => { const store = setup(); return () => store; } },
    './GlobalConfig': { GlobalConfig: globalConfig },
    '../utils/SSMTFileUtils': {},
    '../utils/SSMTJsonUtils': {},
    './GameConfig': {},
    './GamePreset': {},
    '@tauri-apps/plugin-http': { fetch: async url => {
      calls.push({ command: 'fetch', url });
      return { ok: true, json: async () => [
        { tag_name: 'v2', body: 'upstream notes', assets: [{ name: 'runtime.zip', browser_download_url: 'https://example.com/runtime.zip' }] },
        { tag_name: 'draft', draft: true },
        { tag_name: 'preview', prerelease: true },
      ] };
    } },
  }).ResourceManager;
  const resource = {
    loadGameConfig: async () => config,
    resolveMigotoDllSource: resourceManager.resolveMigotoDllSource,
  };
  mocks['../store/ResourceManager'] = { ResourceManager: resource };
  const ini = load('../../src/store/D3dxIniManager.ts', mocks);
  const migoto = load('../../src/store/MigotoManager.ts', {
    ...mocks, './ResourceManager': { ResourceManager: resource }, './D3dxIniManager': ini,
  }).MigotoManager;
  const launch = load('../../src/common/LaunchGame.ts', { ...mocks, '../store/MigotoManager': { MigotoManager: migoto } }).LaunchGame;
  return { root, source, target, resourceDir, migotoDir, config, calls, messages, dialogs, writes, migoto, launch, resourceManager };
}

test('XXMI upstream changelog remains read-only and exposes no install URL', async t => {
  const h = await harness(t);
  const releases = await h.resourceManager.getXXMILibsReleaseList('test-token');
  assert.equal(h.calls[0].url, 'https://api.github.com/repos/SpectrumQT/XXMI-Libs-Package/releases?per_page=100');
  assert.equal(releases.length, 1);
  assert.equal(releases[0].version, 'v2');
  assert.equal(releases[0].download_url, '');
});

test('a verified staged copy replaces the existing DLL', async t => {
  const h = await harness(t);
  assert.equal((await h.migoto.applySSMTRuntimeToMigotoDir('GIMI', h.migotoDir, h.config)).status, 'updated');
  assert.equal(await fs.readFile(h.target, 'utf8'), 'adapted SSMT Runtime');
  assert.equal((await fs.readdir(h.migotoDir)).some(name => name.includes('.ssmt-staging-')), false);
});

test('identical bundled files need no replacement', async t => {
  const h = await harness(t, { fail: 'replace' });
  await fs.copyFile(h.source, h.target);
  assert.equal((await h.migoto.syncRuntimeFile(h.source, h.target)).status, 'unchanged');
  assert.equal(h.writes.length, 0);
});

for (const fail of ['copy', 'integrity', 'hash', 'replace', 'cleanup', 'missing-source', 'prepare']) {
  test(`${fail} preserves the exact existing DLL`, async t => {
    const h = await harness(t, { fail });
    if (fail === 'missing-source') await fs.rm(h.source);
    const prepare = fail === 'prepare' ? async staging => {
      await fs.writeFile(staging, 'broken UPX output');
      throw new Error('UPX failed');
    } : undefined;
    const result = await h.migoto.syncRuntimeFile(h.source, h.target, prepare);
    assert.equal(result.status, 'kept_existing');
    assert.ok(result.error);
    assert.equal(await fs.readFile(h.target, 'utf8'), 'previous working Runtime');
    if (fail !== 'cleanup') assert.equal((await fs.readdir(h.migotoDir)).some(name => name.includes('.ssmt-staging-')), false);
  });
}

test('DX12 retains its bundled source and never calls DX11 packing', async t => {
  const h = await harness(t);
  await fs.writeFile(path.join(h.resourceDir, 'DX12', 'd3d12.dll'), 'DX12 Runtime');
  const result = await h.migoto.applySSMTRuntimeToMigotoDir('ZZMIDX12', h.migotoDir,
    { gamePreset: 'ZZMIDX12' }, async () => assert.fail('DX12 must not use UPX'));
  assert.equal(result.status, 'updated');
  assert.equal(await fs.readFile(path.join(h.migotoDir, 'd3d12.dll'), 'utf8'), 'DX12 Runtime');
});

for (const fail of ['replace', 'run-replace', 'missing-source', 'upx', 'missing-upx']) {
  test(`normal launch continues with a banner after ${fail}`, async t => {
    const h = await harness(t, { fail, useUpx: fail === 'upx' || fail === 'missing-upx' });
    if (fail === 'missing-source') {
      await fs.rm(h.source);
      await fs.rm(path.join(h.resourceDir, 'Run.exe'));
    }
    if (fail === 'upx') await fs.writeFile(path.join(h.resourceDir, 'upx.exe'), 'UPX fixture');
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => assert.fail('No update dialog'));
    assert.equal(await fs.readFile(h.target, 'utf8'), fail === 'run-replace' ? 'adapted SSMT Runtime' : 'previous working Runtime');
    if (fail === 'run-replace' || fail === 'missing-source') {
      assert.equal(await fs.readFile(path.join(h.migotoDir, 'Run.exe'), 'utf8'), 'old Run.exe');
    }
    assert.equal(h.dialogs.length, 0);
    assert.ok(h.messages.some(({ kind, value }) => kind === 'warning' && value.message.includes('ssmtRuntimeUpdateFallback')));
    const programs = h.calls.find(call => call.command === 'launch_programs')?.args.programs;
    assert.ok(programs?.some(program => program.path === path.join(h.migotoDir, 'Run.exe')));
    assert.equal(h.writes.some(write => write.to.endsWith('SSMT-PluginHost.dll')), false);
  });
}

test('first install without a usable DLL warns without a modal or starting the injector', async t => {
  const h = await harness(t, { noExisting: true, fail: 'copy' });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => assert.fail('No update dialog'));
  assert.equal(h.dialogs.length, 0);
  assert.ok(h.messages.some(({ value }) => value.message?.includes('ssmtRuntimeUnavailable')));
  assert.equal(h.calls.some(call => call.command === 'launch_programs'), false);
});

test('pure launch does not read or deploy runtime or plugin resources', async t => {
  const h = await harness(t, { fail: 'copy' });
  await fs.rm(h.resourceDir, { recursive: true });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-pure' }, () => assert.fail('No update dialog'));
  assert.equal(h.writes.length, 0);
  assert.equal(h.dialogs.length, 0);
  assert.equal(h.messages.some(({ kind }) => kind === 'warning'), false);
  assert.deepEqual(h.calls.map(call => call.command), ['launch_programs']);
});

test('GIMI configuration follows the saved switch before injection', async t => {
  const h = await harness(t, { configureGame: false });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  const configure = h.calls.findIndex(call => call.command === 'configure_gimi_launch_settings');
  assert.ok(configure >= 0);
  assert.equal(h.calls[configure].args.options.configureGame, false);
  assert.ok(configure < h.calls.findIndex(call => call.command === 'launch_programs'));
});

const hotPlan = {
  dlls: ['C:/Managed/ReShade64.dll', 'C:/Managed/Dx11FsrBridge.dll', 'C:/Managed/OptiScaler.dll'],
  environment: { GIMI_HOSTED_RESHADE_DLL: 'C:/Managed/ReShade64.dll', RESHADE_DISABLE_GRAPHICS_HOOK: '1', RESHADE_DISABLE_INPUT_HOOK: null },
  runtimeDirectory: 'C:/Managed',
};

for (const hostConfig of [null, 'C:/Plugins/all-enabled.json']) {
  test(`hot DLSS5 uses one host and keeps ordered extras; PluginHost=${!!hostConfig}`, async t => {
    const h = await harness(t, { hotRuntime: hotPlan, hostConfig, config: { extraDlls: ['C:/Extra/Other.dll'] } });
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    const injector = programs.find(program => program.path.endsWith('Run.exe'));
    assert.equal(programs.length, 2); // Run.exe 和目标等待项。
    assert.match(injector.args, /--preload-runtime/);
    assert.match(injector.args, /--set-env "GIMI_HOSTED_RESHADE_DLL=C:\/Managed\/ReShade64.dll"/);
    assert.match(injector.args, /--unset-env RESHADE_DISABLE_INPUT_HOOK/);
    assert.equal(injector.args.includes('--plugin-host-config'), !!hostConfig);
    assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
    const ini = await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8');
    assert.match(ini, /inject_dlls = C:\/Managed\/ReShade64.dll\|C:\/Managed\/Dx11FsrBridge.dll\|C:\/Managed\/OptiScaler.dll\|C:\/Extra\/Other.dll/);
  });
}

for (const hostConfig of [null, 'C:/Plugins/all-enabled.json']) {
  test(`Shell 直接启动同一游戏时使用单次挂起热注入，保留设置与参数；PluginHost=${!!hostConfig}`, async t => {
    const h = await harness(t, { hotRuntime: hotPlan, hostConfig, config: { useShell: true, launchArgs: '-force-d3d11 -test="a b"' } });
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    assert.equal(h.dialogs.length, 0);
    const programs = h.calls.find(call => call.command === 'launch_programs')?.args.programs;
    assert.ok(programs);
    assert.equal(programs.length, 2);
    assert.ok(programs[0].path.endsWith('Run.exe'));
    assert.match(programs[0].args, /--preload-runtime/);
    assert.equal(programs[0].args.includes('--plugin-host-config'), !!hostConfig);
    assert.equal(programs[0].waitForProcessName, undefined);
    assert.equal(programs[1].waitOnly, true);
    assert.equal(programs.some(program => program.path === h.config.targetExePath), false, '游戏只能由 Run.exe 创建一次');
    const ini = await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8');
    assert.ok(ini.includes(`launch = ${h.config.targetExePath}`));
    assert.match(ini, /launch_args = -force-d3d11 -test="a b"/);
    assert.match(ini, /inject_dlls = C:\/Managed\/ReShade64.dll\|C:\/Managed\/Dx11FsrBridge.dll\|C:\/Managed\/OptiScaler.dll/);
    assert.equal(h.config.useShell, true, '仅改变本次启动方式，不写回用户设置');
    assert.equal(h.calls.some(call => call.command === 'prepare_hoyoshade_launch'), false);
  });
}

test('同一目标路径中的分隔符和点目录不导致 Shell 热注入误判', async t => {
  const h = await harness(t, { hotRuntime: hotPlan, config: { useShell: true } });
  h.config.launcherExePath = `${h.root}/./Game.exe`;
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.equal(h.dialogs.length, 0);
  assert.ok(h.calls.find(call => call.command === 'launch_programs')?.args.programs[0].args.includes('--preload-runtime'));
});

for (const differentLauncher of [false, true]) {
  test(`没有热注入计划时保留原有 Shell 启动队列；不同启动程序=${differentLauncher}`, async t => {
    const h = await harness(t, { config: { useShell: true, launchArgs: '-force-d3d11' } });
    if (differentLauncher) {
      h.config.launcherExePath = path.join(h.root, 'Launcher.exe');
      await fs.writeFile(h.config.launcherExePath, 'launcher fixture');
    }
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    assert.equal(h.dialogs.length, 0);
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    assert.equal(programs.length, 2);
    assert.equal(programs[0].waitForProcessName, 'Run.exe');
    assert.doesNotMatch(programs[0].args || '', /preload-runtime|set-env/);
    assert.equal(programs[1].path, h.config.launcherExePath);
    assert.equal(programs[1].args, '-force-d3d11');
    assert.doesNotMatch(await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8'), /^\s*launch(?:_args)?\s*=/m);
  });
}

for (const config of [{ extraDlls: ['C:/Other/ReShade64.dll'] }, { launchTargetProgram: false }, { useShell: true }]) {
  test(`热注入冲突阻止重复 Host，不支持的启动方式保留普通启动：${JSON.stringify(config)}`, async t => {
    const h = await harness(t, { hotRuntime: hotPlan, config });
    if (config.useShell) {
      const otherGame = path.join(h.root, 'OtherGame');
      await fs.mkdir(otherGame);
      h.config.launcherExePath = path.join(otherGame, 'Game.exe');
      await fs.writeFile(h.config.launcherExePath, 'launcher fixture');
    }
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    assert.ok(h.calls.some(call => call.command === 'prepare_dlss5_hot_runtime'), '必须验证模式检查，不能因无效路径提前返回');
    if (config.extraDlls) {
      assert.equal(h.calls.some(call => call.command === 'launch_programs'), false);
      assert.ok(h.dialogs.some(dialog => dialog.kind === 'alert'));
    } else {
      const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
      assert.doesNotMatch(programs[0].args || '', /--preload-runtime/);
      assert.equal(h.dialogs.length, 0);
      const warning = h.messages.find(message => message.kind === 'warning');
      assert.ok(warning);
      assert.match(JSON.stringify(warning), config.useShell ? /gimiHotExternalLauncherUnsupported/ : /gimiHotManualLaunchUnsupported/);
    }
  });
}

for (const useShell of [false, true]) {
  test(`disabling hot DLSS5 removes its prior INI entries and per-launch environment; Shell=${useShell}`, async t => {
    const options = { hotRuntime: hotPlan, config: { useShell } };
    const h = await harness(t, options);
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    options.hotRuntime = null;
    h.calls.length = 0;
    await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
    const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
    assert.doesNotMatch(programs[0].args || '', /preload-runtime|set-env/);
    const ini = await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8');
    assert.doesNotMatch(ini, /inject_dlls\s*=/);
    if (useShell) {
      assert.doesNotMatch(ini, /^\s*launch(?:_args)?\s*=/m);
      assert.equal(programs[1].path, h.config.launcherExePath);
    }
  });
}

for (const shade of [null, { path: 'C:/Shade/inject.exe', args: 'YuanShen.exe', workDir: 'C:/Shade', runAsAdministrator: true, postLaunchDelayMs: 150 }]) {
  for (const hostConfig of [null, 'C:/Plugins/player-tweaks.json']) {
    test(`unavailable DLSS5 warns without blocking other contributions: Shade=${!!shade}, Host=${!!hostConfig}`, async t => {
      const h = await harness(t, { hotError: 'component package is missing or damaged', shade, hostConfig });
      await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => assert.fail('No update dialog'));
      const programs = h.calls.find(call => call.command === 'launch_programs').args.programs;
      const injector = programs.find(program => program.path.endsWith('Run.exe'));
      assert.ok(injector);
      assert.doesNotMatch(injector.args || '', /preload-runtime|set-env/);
      assert.equal(injector.args?.includes('--plugin-host-config') || false, !!hostConfig);
      assert.equal(programs.some(program => program.path === 'C:/Shade/inject.exe'), !!shade);
      assert.equal(h.calls.find(call => call.command === 'prepare_hoyoshade_launch').args.skipDlss5, true);
      assert.equal(h.messages.some(message => message.kind === 'warning'), true);
      assert.equal(h.dialogs.length, 0);
      assert.doesNotMatch(await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8'), /ReShade64.dll|Dx11FsrBridge.dll|OptiScaler.dll/);
    });
  }
}

test('unavailable DLSS5 and Shade resources still allow ordinary Mod launch', async t => {
  const h = await harness(t, { hotError: 'missing NR runtime', shadeError: 'missing Shade resources' });
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => assert.fail('No update dialog'));
  assert.ok(h.calls.some(call => call.command === 'launch_programs'));
  assert.equal(h.dialogs.length, 0);
  assert.equal(h.messages.filter(message => message.kind === 'warning').length, 2);
});

test('DLSS5 preparation failure removes a prior hot plan without changing saved options', async t => {
  const options = { hotRuntime: hotPlan };
  const h = await harness(t, options);
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  options.hotError = 'NR Runtime was moved';
  h.calls.length = 0;
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => {});
  assert.ok(h.calls.some(call => call.command === 'launch_programs'));
  assert.equal(h.dialogs.length, 0);
  assert.doesNotMatch(await fs.readFile(path.join(h.migotoDir, 'd3dx.ini'), 'utf8'), /inject_dlls\s*=/);
  assert.equal(h.config.extraDlls, undefined);
});

test('already packed runtime passes UPX validation without repacking', async t => {
  const h = await harness(t, { useUpx: true, upxResult: () => ({ code: 0, stdout: 'OK', stderr: '' }) });
  await fs.writeFile(path.join(h.resourceDir, 'upx.exe'), 'UPX fixture');
  await h.launch.launch('GIMI', { gameLaunchMode: 'always-normal' }, () => assert.fail('No update dialog'));
  const upxCalls = h.calls.filter(call => call.command === 'execute_external_program');
  assert.equal(upxCalls.length, 1);
  assert.equal(upxCalls[0].args.args[0], '-t');
  assert.notEqual(upxCalls[0].args.args[1], h.target);
  assert.equal(h.messages.some(({ kind }) => kind === 'warning'), false);
  assert.equal(await fs.readFile(h.target, 'utf8'), 'adapted SSMT Runtime');
});
