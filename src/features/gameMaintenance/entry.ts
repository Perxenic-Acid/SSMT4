import type { GameConfig } from '../../store/GameConfig';

export const supportsGameMaintenance = (preset: string | undefined) =>
  ['GIMI', 'SRMI', 'ZZMI', 'ZZMIDX12'].includes((preset || '').trim().toUpperCase());

export interface DiscoveredClient { targetExePath: string; launcherExePath: string }

// 主按钮只在已配置路径和自动匹配都不可用时进入安装，不依赖版本服务联网成功。
export async function resolveClientEntry(config: GameConfig, services: {
  isFile: (path: string) => Promise<boolean>;
  discover: (preset: string) => Promise<DiscoveredClient | null>;
}): Promise<{ needsInstallation: boolean; discovered?: DiscoveredClient }> {
  if (!supportsGameMaintenance(config.gamePreset)) return { needsInstallation: false };
  const preset = (config.gamePreset || '').trim().toUpperCase();
  const path = (config.targetExePath || '').trim();
  if (path && await services.isFile(path)) return { needsInstallation: false };
  let discovered: DiscoveredClient | null = null;
  try { discovered = await services.discover(preset); }
  catch { /* 无法自动匹配时仍提供安装和手动选择，不能卡住主页。 */ }
  if (discovered?.targetExePath && await services.isFile(discovered.targetExePath)) return { needsInstallation: false, discovered };
  return { needsInstallation: true };
}
