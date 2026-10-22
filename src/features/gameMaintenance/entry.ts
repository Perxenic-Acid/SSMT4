import type { GameConfig } from '../../store/GameConfig';

export const supportsGameMaintenance = (preset: string | undefined) =>
  ['GIMI', 'SRMI', 'ZZMI', 'ZZMIDX12'].includes((preset || '').trim().toUpperCase());

export interface DiscoveredClient { targetExePath: string; launcherExePath: string }

// 主按钮只在已配置路径和自动匹配都不可用时进入安装，不依赖版本服务联网成功。
export async function resolveClientEntry(config: GameConfig, services: {
  isFile: (path: string) => Promise<boolean>;
  discover: (preset: string) => Promise<DiscoveredClient | null>;
}, timeoutMs = 3500): Promise<{ needsInstallation: boolean; discovered?: DiscoveredClient }> {
  if (!supportsGameMaintenance(config.gamePreset)) return { needsInstallation: false };
  const checkFile = async (path: string) => {
    try { return !!path && await services.isFile(path); } catch { return false; }
  };
  const resolve = async () => {
    const preset = (config.gamePreset || '').trim().toUpperCase();
    const path = (config.targetExePath || '').trim();
    if (await checkFile(path)) return { needsInstallation: false };
    let discovered: DiscoveredClient | null = null;
    try { discovered = await services.discover(preset); }
    catch { /* 无法自动匹配时仍提供安装和手动选择，不能卡住主页。 */ }
    if (discovered?.targetExePath && await checkFile(discovered.targetExePath)) return { needsInstallation: false, discovered };
    return { needsInstallation: true };
  };
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    // 慢盘、网络路径或旧后端的全盘扫描不能无限扣住主按钮。迟到结果不写配置。
    return await Promise.race([
      resolve(),
      new Promise<{ needsInstallation: boolean }>(done => { timer = setTimeout(() => done({ needsInstallation: true }), timeoutMs); }),
    ]);
  } finally { if (timer) clearTimeout(timer); }
}
