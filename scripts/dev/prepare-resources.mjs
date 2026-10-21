import { constants, copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

export function shouldPrepareDevResources(args) {
  const separator = args.indexOf('--');
  const cliArgs = separator < 0 ? args : args.slice(0, separator);
  return cliArgs.includes('dev') && !cliArgs.some(arg => arg === '--help' || arg === '-h');
}

// 开发启动只补齐缺失的本地资源，不覆盖手工指定的 DLL，也不构建、下载或安装插件。
export function prepareDevResources(repoRoot, log = console.log) {
  const resourcesDir = join(repoRoot, 'src-tauri', 'resources');
  const nativeBuild = '.\\build_native.ps1 -Configuration Release';
  const artifacts = [
    {
      name: 'd3d11.dll',
      sources: [join(repoRoot, 'runtime', 'x64', 'Release', 'd3d11.dll')],
      command: '.\\scripts\\stage-ssmt-runtime.ps1 -Build',
    },
    ...['Run.exe', 'SSMT-PluginHost.dll'].map(name => ({
      name,
      sources: ['Release', 'Debug'].map(profile => join(repoRoot, 'native', 'dist', profile, name)),
      command: nativeBuild,
    })),
  ];
  const pending = [];
  const missing = [];
  for (const artifact of artifacts) {
    const target = join(resourcesDir, artifact.name);
    const targetStat = statSync(target, { throwIfNoEntry: false });
    if (targetStat?.isFile()) continue;
    if (targetStat) throw new Error(`开发资源路径不是文件：${target}`);
    const source = artifact.sources.find(path => statSync(path, { throwIfNoEntry: false })?.isFile());
    if (source) pending.push({ source, target });
    else missing.push(`缺少 ${artifact.name}，且没有本地构建产物。\n  请运行：${artifact.command}\n  检查位置：${artifact.sources.join('、')}`);
  }
  // 在启动 Vite / Cargo 前一次性报告缺项；缺项检查通过后才开始复制。
  if (missing.length) throw new Error(missing.join('\n'));
  if (pending.length) mkdirSync(resourcesDir, { recursive: true });
  for (const { source, target } of pending) {
    try {
      copyFileSync(source, target, constants.COPYFILE_EXCL);
    } catch (error) {
      // 检查与复制之间其他构建可能刚好完成暂存，同样保留其文件。
      if (error.code === 'EEXIST' && statSync(target, { throwIfNoEntry: false })?.isFile()) continue;
      throw new Error(`开发资源暂存失败：${source} -> ${target}：${error.message}`, { cause: error });
    }
    log(`[dev-resources] 已暂存 ${source} -> ${target}`);
  }
}
