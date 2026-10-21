import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { prepareDevResources, shouldPrepareDevResources } from './dev/prepare-resources.mjs';

const repoRoot = process.cwd();
const privateKeyPath = join(repoRoot, 'updater-signing.key');
const latestJsonScriptPath = join(repoRoot, 'generate-latest-json.ps1');
const binDir = join(repoRoot, 'node_modules', '.bin');
const tauriCmdCandidates =
  process.platform === 'win32'
    ? ['tauri.cmd', 'tauri.exe', 'tauri.bunx']
    : ['tauri'];
const tauriCmdPath = tauriCmdCandidates.map((name) => join(binDir, name)).find(existsSync);

if (!tauriCmdPath) {
  console.error(`Tauri CLI not found. Checked: ${tauriCmdCandidates.join(', ')}`);
  process.exit(1);
}

const args = process.argv.slice(2);
const isBuildLike = args.some((arg) => arg === 'build' || arg === 'signer');
const shouldGenerateLatestJson = args.some((arg) => arg === 'build');
const childEnv = { ...process.env };

if (shouldPrepareDevResources(args)) {
  try {
    prepareDevResources(repoRoot);
  } catch (error) {
    console.error(`[dev-resources] ${error.message}`);
    process.exit(1);
  }
}

const hasProxyEnvironment = Object.keys(childEnv).some((name) =>
  ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY'].includes(name.toUpperCase()),
);

const getWindowsSystemProxy = () => {
  if (process.platform !== 'win32') return null;

  const registryKey = 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings';
  const proxyEnabled = spawnSync('reg.exe', ['query', registryKey, '/v', 'ProxyEnable'], {
    encoding: 'utf8',
    windowsHide: true,
  });
  if (proxyEnabled.status !== 0 || !/REG_DWORD\s+0x1\b/i.test(proxyEnabled.stdout)) return null;

  const proxyServer = spawnSync('reg.exe', ['query', registryKey, '/v', 'ProxyServer'], {
    encoding: 'utf8',
    windowsHide: true,
  });
  if (proxyServer.status !== 0) return null;

  const match = proxyServer.stdout.match(/ProxyServer\s+REG_SZ\s+(.+)$/im);
  if (!match) return null;

  const configuredProxy = match[1].trim();
  const protocolEntries = Object.fromEntries(
    configuredProxy
      .split(';')
      .map((entry) => entry.split('=').map((part) => part.trim()))
      .filter(([protocol, address]) => protocol && address),
  );
  const address = protocolEntries.https || protocolEntries.http || configuredProxy;
  return /^[a-z][a-z\d+.-]*:\/\//i.test(address) ? address : `http://${address}`;
};

if (!hasProxyEnvironment) {
  const systemProxy = getWindowsSystemProxy();
  if (systemProxy) {
    childEnv.HTTP_PROXY = systemProxy;
    childEnv.HTTPS_PROXY = systemProxy;
    console.log(`Using Windows system proxy for Tauri downloads: ${systemProxy}`);
  }
}

if (isBuildLike) {
  // 更新签名私钥：优先从环境变量获取，避免私钥进入仓库。
  // 解析顺序：TAURI_SIGNING_PRIVATE_KEY（密钥内容）> TAURI_SIGNING_PRIVATE_KEY_PATH > 项目根目录 updater-signing.key（仅向后兼容，不推荐）
  let privateKeyContent = childEnv.TAURI_SIGNING_PRIVATE_KEY;
  if (!privateKeyContent) {
    const keyPath = childEnv.TAURI_SIGNING_PRIVATE_KEY_PATH || privateKeyPath;
    if (existsSync(keyPath)) {
      privateKeyContent = readFileSync(keyPath, 'utf8');
    }
  }
  if (!privateKeyContent) {
    console.error(
      '[signing] 未找到更新签名私钥。请先运行 "bunx tauri signer generate -w <项目外路径>/updater-signing.key" 生成新密钥对，\n' +
      '      并将私钥保存在项目目录之外，通过环境变量 TAURI_SIGNING_PRIVATE_KEY_PATH（或 TAURI_SIGNING_PRIVATE_KEY）提供给本脚本。',
    );
    process.exit(1);
  }
  childEnv.TAURI_SIGNING_PRIVATE_KEY = privateKeyContent;
  if (!childEnv.TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
    console.error('[signing] 缺少环境变量 TAURI_SIGNING_PRIVATE_KEY_PASSWORD（私钥密码），无法为更新包签名。');
    process.exit(1);
  }
}

const runLatestJsonGenerator = () => {
  if (!existsSync(latestJsonScriptPath)) {
    console.error(`latest.json generator not found at ${latestJsonScriptPath}`);
    process.exit(1);
  }

  const powershellCmd = process.platform === 'win32' ? 'powershell.exe' : 'pwsh';
  const psArgs = ['-NoProfile'];
  if (process.platform === 'win32') {
    psArgs.push('-ExecutionPolicy', 'Bypass');
  }
  psArgs.push('-File', latestJsonScriptPath);

  console.log('\nRunning generate-latest-json.ps1...');
  const ps = spawn(powershellCmd, psArgs, {
    cwd: repoRoot,
    stdio: 'inherit',
    env: childEnv,
    shell: false,
  });

  ps.on('exit', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 0);
  });

  ps.on('error', (error) => {
    console.error(error);
    process.exit(1);
  });
};

const child = spawn(process.env.ComSpec || 'cmd.exe', ['/d', '/s', '/c', tauriCmdPath, ...args], {
  stdio: 'inherit',
  env: childEnv,
  shell: false,
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  if ((code ?? 0) !== 0) {
    process.exit(code ?? 1);
    return;
  }

  if (shouldGenerateLatestJson) {
    runLatestJsonGenerator();
    return;
  }

  process.exit(code ?? 0);
});

child.on('error', (error) => {
  console.error(error);
  process.exit(1);
});
