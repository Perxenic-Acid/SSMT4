import { defineStore } from 'pinia';
import { GlobalConfig } from "./GlobalConfig";
import { SSMTFileUtils } from "../utils/SSMTFileUtils";

import { exists, writeFile, remove, mkdir, readDir, writeTextFile, copyFile } from '@tauri-apps/plugin-fs';
import { join, resourceDir } from '@tauri-apps/api/path';
import { fetch } from '@tauri-apps/plugin-http';
import { invoke } from '@tauri-apps/api/core';
import { SSMTJsonUtils } from '../utils/SSMTJsonUtils';
import { PathHelper } from '../helper/PathHelper';
import { i18n } from '../i18n';
import { debugLog } from '../utils/debugLog';

import { GameConfig, GameConfigManager, normalizeD3d11Mode, type D3d11Mode } from "./GameConfig";
import { getGithubRepoByGamePreset, isMihoyoGamePreset } from './GamePreset';

const t = i18n.global.t;

export type UpdateInfo = {
    version: string;
    description: string;
    download_url: string;
    is_latest?: boolean;
    is_prerelease?: boolean;
};

type GithubReleaseQueryOptions = {
    includePrerelease?: boolean;
    assetMatcher?: (assetName: string) => boolean;
};

type GithubReleaseAsset = {
    name?: string;
    browser_download_url?: string;
};

type GithubRelease = {
    tag_name?: string;
    body?: string;
    draft?: boolean;
    prerelease?: boolean;
    published_at?: string;
    assets?: GithubReleaseAsset[];
};

type FixedBackgroundSource = {
    imageUrl?: string;
    videoUrl?: string;
};

type D3d11ReleaseFileRule = {
    sourceFileName: string;
    targetFileName: string;
    optional?: boolean;
};

type D3d11ReleaseSource = {
    repo: string;
    cacheFileName: string;
    assetMatcher: (assetName: string) => boolean;
    filesToInstall: D3d11ReleaseFileRule[];
};

const hypPresetToGameId: Record<string, string> = {
    GIMI: '1Z8W5NHUQb',
    HIMI: 'osvnlOc0S8',
    SRMI: '64kMb5iAWu',
    ZZMI: 'x6znKlJ0xK',
    ZZMIDX12: 'x6znKlJ0xK',
};

const fixedBackgroundSources: Record<string, FixedBackgroundSource> = {
    NTEMI: {
        imageUrl: 'https://yh.wanmei.com/images/cover260408/section-head-bg.jpg',
        videoUrl: 'https://yhvmg.wmupd.com/webops/yh/yh_bgvideo_20260418.mp4',
    },
    WWMI: {
        imageUrl: 'https://mc.kurogames.com/website-preface/video/bg/bg-poster.webp',
    },
};

const PLAY_D3D11_FILE_NAME = 'd3d11.play.dll';
const DEV_D3D11_FILE_NAME = 'd3d11.dev.dll';
const IDENTITY_V_DEV_D3D11_FILE_NAME = 'd3d11.identityv.dev.dll';
const CAPPED_DEV_D3D11_FILE_NAME = IDENTITY_V_DEV_D3D11_FILE_NAME;
const SSICE_A_D3D11_FILE_NAME = 'd3d11.ssice-a.dll';
const D3DCOMPILER_FILE_NAME = 'd3dcompiler_47.dll';
const DX12_PRESET = 'ZZMIDX12';
const DX12_D3D12_FILE_NAME = 'd3d12.dll';
const CAPPED_DEV_D3D11_PRESETS = ['IDENTITYV', 'NARAKA', 'NARAKAM'] as const;
const CAPPED_DEV_D3D11_PRESET_SET: ReadonlySet<string> = new Set(CAPPED_DEV_D3D11_PRESETS);
const CAPPED_DEV_D3D11_MAX_VERSION = [0, 9, 2] as const;

const isCappedDevD3d11Preset = (gamePreset?: string | null): boolean => (
    CAPPED_DEV_D3D11_PRESET_SET.has((gamePreset || '').trim().toUpperCase())
);

const getD3d11CacheFileName = (mode: D3d11Mode, gamePreset?: string | null): string => (
    mode === 'dev' && isCappedDevD3d11Preset(gamePreset)
        ? CAPPED_DEV_D3D11_FILE_NAME
        : D3D11_RELEASE_SOURCES[mode].cacheFileName
);

const isCappedDevD3d11VersionSupported = (version: string): boolean => {
    const match = version.trim().match(/^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/i);
    if (!match) return false;

    const candidate = match.slice(1).map(part => Number.parseInt(part, 10));
    for (let index = 0; index < CAPPED_DEV_D3D11_MAX_VERSION.length; index += 1) {
        if (candidate[index] < CAPPED_DEV_D3D11_MAX_VERSION[index]) return true;
        if (candidate[index] > CAPPED_DEV_D3D11_MAX_VERSION[index]) return false;
    }
    return true;
};

const getCappedDevD3d11MaxVersion = (): string => (
    `v${CAPPED_DEV_D3D11_MAX_VERSION.join('.')}`
);

const isCappedDevD3d11Mode = (mode: D3d11Mode, gamePreset?: string | null): boolean => (
    mode === 'dev' && isCappedDevD3d11Preset(gamePreset)
);

const isD3d11VersionAllowedForMode = (version: string, mode: D3d11Mode, gamePreset?: string | null): boolean => (
    !isCappedDevD3d11Mode(mode, gamePreset) || isCappedDevD3d11VersionSupported(version)
);

const constrainD3d11ReleasesForGame = (
    releases: UpdateInfo[],
    mode: D3d11Mode,
    gamePreset?: string,
): UpdateInfo[] => {
    if (mode !== 'dev' || !isCappedDevD3d11Preset(gamePreset)) {
        return releases;
    }

    return releases
        .filter(release => isCappedDevD3d11VersionSupported(release.version))
        .map((release, index) => ({ ...release, is_latest: index === 0 }));
};



const D3D11_RELEASE_SOURCES: Record<D3d11Mode, D3d11ReleaseSource> = {
    dev: {
        repo: 'SpectrumQT/XXMI-Libs-Package',
        cacheFileName: DEV_D3D11_FILE_NAME,
        assetMatcher: (assetName) => {
            const normalizedName = assetName.toLowerCase();
            return normalizedName.endsWith('.zip')
                && normalizedName.includes('xxmi')
                && normalizedName.includes('package');
        },
        filesToInstall: [
            { sourceFileName: 'd3d11.dll', targetFileName: DEV_D3D11_FILE_NAME },
            { sourceFileName: 'd3dcompiler_47.dll', targetFileName: D3DCOMPILER_FILE_NAME, optional: true },
            { sourceFileName: '3dmloader.dll', targetFileName: '3dmloader.dll', optional: true },
        ],
    },
    play: {
        repo: 'StarBobis/Doodle',
        cacheFileName: PLAY_D3D11_FILE_NAME,
        assetMatcher: (assetName) => {
            const normalizedName = assetName.toLowerCase();
            return normalizedName.endsWith('.zip')
                && normalizedName.includes('3dmigotodll');
        },
        filesToInstall: [
            { sourceFileName: 'd3d11.dll', targetFileName: PLAY_D3D11_FILE_NAME },
            { sourceFileName: 'd3dcompiler_47.dll', targetFileName: D3DCOMPILER_FILE_NAME, optional: true },
        ],
    },
    'ssice-a': {
        repo: 'ssice-a/XXMI-Libs-Package',
        cacheFileName: SSICE_A_D3D11_FILE_NAME,
        assetMatcher: (assetName) => {
            const normalizedName = assetName.toLowerCase();
            return normalizedName.endsWith('.zip')
                && (
                    (normalizedName.includes('xxmi') && normalizedName.includes('package'))
                    || normalizedName.includes('ntmi-package')
                );
        },
        filesToInstall: [
            { sourceFileName: 'd3d11.dll', targetFileName: SSICE_A_D3D11_FILE_NAME },
            { sourceFileName: 'd3dcompiler_47.dll', targetFileName: D3DCOMPILER_FILE_NAME, optional: true },
            { sourceFileName: '3dmloader.dll', targetFileName: '3dmloader.dll', optional: true },
        ],
    },
};

export type BGType = 'Image' | 'Video';

const BACKGROUND_IMAGE_CANDIDATES = ['Background.png', 'Background.webp', 'Background.jpg', 'Background.jpeg', 'Background.gif', 'Background.svg', 'Background.bmp', 'Background.ico', 'Background.avif'];
const BACKGROUND_VIDEO_CANDIDATES = ['Background.mp4', 'Background.webm', 'Background.mkv', 'Background.ogg', 'Background.mov'];

export type GameInfo = {
    name: string;
    icon_path: string;
    bg_path: string;
    bg_video_path?: string | null;
    bg_type: BGType;
    show_sidebar: boolean;
};


export const useResourceManagerStore = defineStore('resourceManager', () => {
    // ============================================================
    // Reactive state
    // ============================================================

    // ============================================================
    // Private helpers (not returned from store)
    // ============================================================

    function getD3d11ReleaseSource(mode: D3d11Mode): D3d11ReleaseSource {
        return D3D11_RELEASE_SOURCES[mode];
    }

    async function findExistingFileIgnoreCase(dirPath: string, candidates: readonly string[]): Promise<string> {
        let children: Array<{ name?: string; isDirectory?: boolean }> = [];
        try {
            children = await readDir(dirPath);
        } catch {
            return '';
        }

        const nameMap = new Map<string, string>();
        children.forEach(child => {
            if (child?.name) {
                nameMap.set(child.name.toLowerCase(), child.name);
            }
        });

        for (const candidate of candidates) {
            const matchedName = nameMap.get(candidate.toLowerCase());
            if (matchedName) {
                return SSMTFileUtils.JoinPath(dirPath, matchedName);
            }
        }

        return '';
    }

    function getFixedBackgroundUrl(gamePreset: string, bgType: BGType): string {
        const source = fixedBackgroundSources[gamePreset];
        if (!source) {
            return '';
        }

        const candidate = bgType === 'Video' ? source.videoUrl : source.imageUrl;
        return typeof candidate === 'string' ? candidate.trim() : '';
    }

    async function getHypBackgroundUrl(gamePreset: string, bgType: BGType): Promise<string> {
        const gameId = hypPresetToGameId[gamePreset];
        if (!gameId) {
            return '';
        }

        const apiUrl = `https://hyp-api.mihoyo.com/hyp/hyp-connect/api/getAllGameBasicInfo?launcher_id=jGHBHlcOq1&language=zh-cn&game_id=${gameId}`;
        const resp = await fetch(apiUrl, { method: 'GET' });
        if (!resp.ok) {
            throw new Error(t('resourceManager.messages.requestFailed', { status: resp.status }));
        }

        const json = await resp.json();
        const backgrounds = json?.data?.game_info_list?.[0]?.backgrounds;
        if (!Array.isArray(backgrounds) || backgrounds.length === 0) {
            throw new Error(t('resourceManager.messages.backgroundsMissingFromServer'));
        }

        for (const item of backgrounds) {
            if (!item || typeof item !== 'object') continue;

            const candidate = bgType === 'Video'
                ? item?.video?.url
                : item?.background?.url;

            if (typeof candidate === 'string' && candidate.trim() !== '') {
                return candidate.trim();
            }
        }

        return '';
    }

    async function resolveBackgroundDownloadUrl(gamePreset: string, bgType: BGType): Promise<string> {
        const fixedUrl = getFixedBackgroundUrl(gamePreset, bgType);
        if (fixedUrl) {
            return fixedUrl;
        }

        return getHypBackgroundUrl(gamePreset, bgType);
    }

    function supportsAutoUpdateBackground(gamePreset: string): boolean {
        return Object.prototype.hasOwnProperty.call(fixedBackgroundSources, gamePreset)
            || Object.prototype.hasOwnProperty.call(hypPresetToGameId, gamePreset);
    }

    async function resolveGithubToken(githubToken?: string): Promise<string> {
        // 未配置 token 时返回空字符串，请求不带 Authorization 头。
        return (githubToken || '').trim();
    }

    async function fetchGithubReleases(
        repo: string,
        githubToken?: string,
    ): Promise<GithubRelease[]> {
        const url = `https://api.github.com/repos/${repo}/releases?per_page=100`;
        const headers: Record<string, string> = {
            "User-Agent": "ssmt4-app",
            "Accept": "application/vnd.github.v3+json",
        };

        const effectiveToken = await resolveGithubToken(githubToken);

        if (effectiveToken) {
            headers["Authorization"] = `Bearer ${effectiveToken}`;
        }

        const resp = await fetch(url, { headers });
        if (!resp.ok) {
            const body = await resp.text().catch(() => "");
            throw new Error(t('resourceManager.messages.githubApiError', { status: resp.status, body }));
        }

        const json = await resp.json();
        return Array.isArray(json) ? json as GithubRelease[] : [];
    }

    function resolveGithubReleaseAsset(
        assets: GithubReleaseAsset[],
        assetMatcher?: (assetName: string) => boolean,
    ): GithubReleaseAsset | null {
        let asset = assets.find((candidate) => {
            const name = candidate?.name;
            if (typeof name !== 'string') return false;
            if (assetMatcher) return assetMatcher(name);
            return name.endsWith('.zip');
        });

        if (!asset && !assetMatcher) {
            asset = assets[0];
        }

        return asset ?? null;
    }

    function mapGithubReleaseToUpdateInfo(
        release: GithubRelease,
        isLatest: boolean,
        assetMatcher?: (assetName: string) => boolean,
    ): UpdateInfo | null {
        const tag_name = release.tag_name ?? null;
        if (!tag_name) {
            return null;
        }

        const assets = Array.isArray(release.assets) ? release.assets as GithubReleaseAsset[] : [];
        if (assets.length === 0) {
            return null;
        }

        const asset = resolveGithubReleaseAsset(assets, assetMatcher);
        if (!asset) {
            return null;
        }

        const download_url = asset.browser_download_url;
        if (!download_url) {
            return null;
        }

        return {
            version: tag_name,
            description: release.body ?? t('resourceManager.messages.noDescription'),
            download_url,
            is_latest: isLatest,
            is_prerelease: !!release.prerelease,
        };
    }

    async function getGithubReleaseList(
        repo: string,
        githubToken?: string,
        options: GithubReleaseQueryOptions = {},
    ): Promise<UpdateInfo[]> {
        const releases = await fetchGithubReleases(repo, githubToken);
        const includePrerelease = options.includePrerelease ?? false;
        const assetMatcher = options.assetMatcher;

        return releases
            .filter((candidate) => {
                if (!candidate || candidate.draft) {
                    return false;
                }

                if (!includePrerelease && candidate.prerelease) {
                    return false;
                }

                const assets = Array.isArray(candidate.assets) ? candidate.assets : [];
                if (assets.length === 0) {
                    return false;
                }

                return resolveGithubReleaseAsset(assets, assetMatcher) !== null;
            })
            .map((release, index) => mapGithubReleaseToUpdateInfo(release, index === 0, assetMatcher))
            .filter((release): release is UpdateInfo => release !== null);
    }

    async function getLatestGithubRelease(
        repo: string,
        githubToken?: string,
        includePrerelease = false,
        assetMatcher?: (assetName: string) => boolean,
    ): Promise<UpdateInfo> {
        const releases = await getGithubReleaseList(repo, githubToken, {
            includePrerelease,
            assetMatcher,
        });

        const release = releases[0];
        if (!release) {
            throw new Error(t('resourceManager.messages.releaseNotFoundForCriteria'));
        }

        return release;
    }

    async function findFileRecursive(dirPath: string, fileName: string): Promise<string | null> {
        const entries = await readDir(dirPath);

        for (const entry of entries) {
            if (!entry?.name) continue;

            const entryPath = await join(dirPath, entry.name);
            if (entry.isDirectory) {
                const foundPath = await findFileRecursive(entryPath, fileName);
                if (foundPath) {
                    return foundPath;
                }
                continue;
            }

            if (entry.name.toLowerCase() === fileName.toLowerCase()) {
                return entryPath;
            }
        }

        return null;
    }

    async function copyFileOverwrite(sourcePath: string, targetPath: string): Promise<void> {
        if (await exists(targetPath)) {
            await remove(targetPath);
        }
        await copyFile(sourcePath, targetPath);
    }

    function buildSidebarMap(iconConfig: { GameIconSettingList?: Array<{ GameName: string; Show: boolean }>; list?: Array<{ game_name: string; show: boolean }> }): Map<string, boolean> {
        const sidebarMap = new Map<string, boolean>();

        if (iconConfig && Array.isArray(iconConfig.GameIconSettingList)) {
            for (const it of iconConfig.GameIconSettingList) {
                if (it && typeof it.GameName === 'string') {
                    sidebarMap.set(it.GameName, !!it.Show);
                }
            }
        } else if (iconConfig && Array.isArray(iconConfig.list)) {
            for (const it of iconConfig.list) {
                if (it && typeof it.game_name === 'string') {
                    sidebarMap.set(it.game_name, !!it.show);
                }
            }
        }

        return sidebarMap;
    }

    async function ensureGlobalGameIconConfigExists(): Promise<string> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        await SSMTFileUtils.CreateFolderIfNotExists(gamesRoot);

        const targetConfigPath = await GlobalConfig.GlobalGameIconConfigFilePath();
        if (await exists(targetConfigPath)) {
            return targetConfigPath;
        }

        const sourceGamesFolder = await GlobalConfig.SSMTResourcesGamesFolder();
        const sourceConfigPath = await join(sourceGamesFolder, 'GameIconConfig.json');

        if (await exists(sourceConfigPath)) {
            await SSMTFileUtils.CopyFileIfMissing(sourceConfigPath, targetConfigPath);
        } else {
            await writeTextFile(targetConfigPath, JSON.stringify({ GameIconSettingList: [] }, null, 2));
        }

        return targetConfigPath;
    }

    async function loadGameIconSidebarMap(): Promise<Map<string, boolean>> {
        const configPath = await ensureGlobalGameIconConfigExists();
        const iconConfig = await SSMTJsonUtils.readJson(configPath) as {
            GameIconSettingList?: Array<{ GameName: string; Show: boolean }>;
            list?: Array<{ game_name: string; show: boolean }>;
        } | null;
        return buildSidebarMap(iconConfig ?? {});
    }

    // ============================================================
    // Public methods (returned from store)
    // ============================================================

    async function ensureGameConfigExists(gameName: string): Promise<GameConfig> {
        return GameConfigManager.ensureConfigExists(gameName);
    }

    async function createNewConfig(gameName: string, config?: GameConfig): Promise<void> {
        return GameConfigManager.createNewConfig(gameName, config);
    }

    async function deleteGameConfigFolder(gameName: string): Promise<void> {
        return GameConfigManager.deleteGameConfigFolder(gameName);
    }

    async function setGameIcon(gameName: string, sourcePath: string): Promise<void> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);
        if (!(await exists(dirPath))) {
            throw new Error(t('resourceManager.messages.gameDirectoryNotFound', { path: dirPath }));
        }

        const targetPath = await SSMTFileUtils.JoinPath(dirPath, 'Icon.png');
        try {
            if (await exists(targetPath)) {
                await remove(targetPath);
            }
        } catch {}

        await copyFile(sourcePath, targetPath);
    }

    async function setGameBackground(gameName: string, sourcePath: string, bgType: 'Image' | 'Video'): Promise<void> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);
        if (!(await exists(dirPath))) {
            throw new Error(t('resourceManager.messages.gameDirectoryNotFound', { path: dirPath }));
        }

        const ext = sourcePath.split('.').pop()?.toLowerCase() || '';
        const targetName = `Background.${ext}`;
        const targetPath = await SSMTFileUtils.JoinPath(dirPath, targetName);

        const candidates = bgType === 'Image'
            ? BACKGROUND_IMAGE_CANDIDATES
            : BACKGROUND_VIDEO_CANDIDATES;

        for (const c of candidates) {
            const p = await SSMTFileUtils.JoinPath(dirPath, c);
            if (await exists(p)) {
                await remove(p);
            }
        }

        await copyFile(sourcePath, targetPath);

        const conf = await loadGameConfig(gameName);
        conf.backgroundType = bgType;
        await saveGameConfig(gameName, conf);
    }

    async function updateGameBackground(gameName: string, gamePreset: string, bgType: 'Image' | 'Video', lastUrl?: string): Promise<{ path: string; url: string; changed: boolean }> {
        if (!supportsAutoUpdateBackground(gamePreset)) {
            throw new Error(t('resourceManager.messages.unsupportedPresetForAutoUpdate'));
        }

        const targetUrl = await resolveBackgroundDownloadUrl(gamePreset, bgType);
        if (!targetUrl) {
            throw new Error(
                bgType === 'Video'
                    ? t('resourceManager.messages.noVideoBackgroundAvailable')
                    : t('resourceManager.messages.noImageBackgroundAvailable')
            );
        }

        // URL change detection: if the same URL was already downloaded, skip the download
        if (lastUrl && lastUrl === targetUrl) {
            return { path: '', url: targetUrl, changed: false };
        }

        const downloadResp = await fetch(targetUrl, { method: 'GET' });
        if (!downloadResp.ok) {
            throw new Error(t('resourceManager.messages.downloadFailedWithStatus', { status: downloadResp.status }));
        }
        const bytes = new Uint8Array(await downloadResp.arrayBuffer());

        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);
        if (!(await exists(dirPath))) {
            await mkdir(dirPath, { recursive: true });
        }

        const ext = extensionFromUrl(targetUrl, bgType === 'Video' ? 'mp4' : 'png');
        const filename = `Background.${ext}`;
        const targetPath = await SSMTFileUtils.JoinPath(dirPath, filename);

        const cleanup = [...BACKGROUND_IMAGE_CANDIDATES, ...BACKGROUND_VIDEO_CANDIDATES];

        for (const candidate of cleanup) {
            const candidatePath = await SSMTFileUtils.JoinPath(dirPath, candidate);
            if (await exists(candidatePath)) {
                try {
                    await remove(candidatePath);
                } catch (err) {
                    console.warn('Failed to remove old background file', candidatePath, err);
                }
            }
        }

        await writeFile(targetPath, bytes);

        const conf = await loadGameConfig(gameName);
        conf.backgroundType = bgType;
        conf.lastBackgroundUrl = targetUrl;
        await saveGameConfig(gameName, conf);

        return { path: targetPath, url: targetUrl, changed: true };
    }

    async function loadGameConfig(gameName: string): Promise<GameConfig> {
        return GameConfigManager.loadGameConfig(gameName);
    }

    async function saveGameConfig(gameName: string, config: GameConfig): Promise<void> {
        return GameConfigManager.saveGameConfig(gameName, config);
    }

    async function findGameBackgroundPath(gameName: string, bgType: BGType = 'Image'): Promise<string> {
        const normalizedGameName = (gameName || '').trim();
        if (!normalizedGameName) {
            return '';
        }

        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, normalizedGameName);
        if (!(await exists(dirPath))) {
            return '';
        }

        const preferredCandidates = bgType === 'Video' ? BACKGROUND_VIDEO_CANDIDATES : BACKGROUND_IMAGE_CANDIDATES;
        const fallbackCandidates = bgType === 'Video' ? BACKGROUND_IMAGE_CANDIDATES : BACKGROUND_VIDEO_CANDIDATES;
        return (await findExistingFileIgnoreCase(dirPath, preferredCandidates))
            || (await findExistingFileIgnoreCase(dirPath, fallbackCandidates));
    }

    function getEffectiveD3d11Mode(config?: Pick<GameConfig, 'd3d11Mode' | 'gamePreset'> | null): D3d11Mode {
        return normalizeD3d11Mode(config?.d3d11Mode, config?.gamePreset);
    }

    async function getGameD3d11Mode(gameName: string): Promise<D3d11Mode> {
        const config = await loadGameConfig(gameName);
        return getEffectiveD3d11Mode(config);
    }

    async function resolveD3d11SourcePathByMode(mode: D3d11Mode, gamePreset?: string | null): Promise<string> {
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();
        return join(resourcesDir, getD3d11CacheFileName(mode, gamePreset));
    }

    function isDx12GamePreset(gamePreset?: string | null): boolean {
        return (gamePreset || '').trim().toUpperCase() === DX12_PRESET;
    }

    async function resolveBootDllSource(gamePreset?: string | null): Promise<{ sourcePath: string; targetFileName: string; label: string }> {
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();

        if (isDx12GamePreset(gamePreset)) {
            return {
                sourcePath: await join(resourcesDir, 'DX12', DX12_D3D12_FILE_NAME),
                targetFileName: DX12_D3D12_FILE_NAME,
                label: DX12_D3D12_FILE_NAME,
            };
        }

        return {
            sourcePath: await join(resourcesDir, 'd3d11.dll'),
            targetFileName: 'd3d11.dll',
            label: 'd3d11.dll',
        };
    }

    async function resolveMigotoDllSource(config?: Pick<GameConfig, 'd3d11Mode' | 'gamePreset'> | null): Promise<{ sourcePath: string; targetFileName: string; label: string; mode: D3d11Mode }> {
        if (isDx12GamePreset(config?.gamePreset)) {
            const bootDll = await resolveBootDllSource(config?.gamePreset);
            return {
                ...bootDll,
                mode: getEffectiveD3d11Mode(config),
            };
        }

        const mode = getEffectiveD3d11Mode(config);
        return {
            sourcePath: await resolveD3d11SourcePathByMode(mode, config?.gamePreset),
            targetFileName: 'd3d11.dll',
            label: 'd3d11.dll',
            mode,
        };
    }

    async function CopyGamesToGlobalConfig(includeMihoyoGames = false): Promise<string> {
        //确保全局配置文件夹存在
        const ssmt_global_configs_folder = await GlobalConfig.SSMT4GlobalConfigsFolder()
        await SSMTFileUtils.CreateFolderIfNotExists(ssmt_global_configs_folder)
        //确保全局配置文件夹下面的Games文件夹存在
        const target_games_folder = await join(ssmt_global_configs_folder, 'Games')
        await SSMTFileUtils.CreateFolderIfNotExists(target_games_folder)

        //SSMT源文件夹
        const ssmt_games_folder = await GlobalConfig.SSMTResourcesGamesFolder()
        if (!(await exists(ssmt_games_folder))) {
            return target_games_folder
        }

        const source_entries = await readDir(ssmt_games_folder)
        for (const entry of source_entries) {
            if (!entry?.isDirectory || !entry.name) continue
            if (!includeMihoyoGames && isMihoyoGamePreset(entry.name)) continue

            const source_game_folder = await join(ssmt_games_folder, entry.name)
            const target_game_folder = await join(target_games_folder, entry.name)

            if (!(await exists(target_game_folder))) {
                await SSMTFileUtils.CopyDirRecursive(source_game_folder, target_game_folder)
            }
        }

        await ensureGlobalGameIconConfigExists()
        return target_games_folder
    }


    async function Copy3DmigotoDllFiles(targetDir: string, gamePreset?: string): Promise<void> {
        const resDir = await resourceDir()
        const candidates = [resDir, await join(resDir, 'resources')]
        const bootDll = isDx12GamePreset(gamePreset)
            ? { source: await join('DX12', DX12_D3D12_FILE_NAME), target: DX12_D3D12_FILE_NAME }
            : { source: 'd3d11.dll', target: 'd3d11.dll' }
        const BOOT_FILES = [bootDll, { source: 'd3dcompiler_47.dll', target: 'd3dcompiler_47.dll' }, { source: 'Run.exe', target: 'Run.exe' }]

        for (const file of BOOT_FILES) {
            let src: string | null = null
            for (const base of candidates) {
                const path = await join(base, file.source)
                if (await exists(path)) {
                    src = path
                    break
                }
            }
            if (!src) continue
            const dst = await join(targetDir, file.target)
            await SSMTFileUtils.CopyFileIfMissing(src, dst)
        }
    }

    async function getMissingXXMILibsFiles(gameName?: string): Promise<string[]> {
        const config = gameName ? await loadGameConfig(gameName) : null;
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();
        const requiredFiles = isDx12GamePreset(config?.gamePreset)
            ? [await join('DX12', DX12_D3D12_FILE_NAME)]
            : [getD3d11CacheFileName(getEffectiveD3d11Mode(config), config?.gamePreset)];
        const missingFiles: string[] = [];

        for (const fileName of requiredFiles) {
            const filePath = await join(resourcesDir, fileName);
            if (!(await exists(filePath))) {
                missingFiles.push(fileName);
            }
        }

        return missingFiles;
    }

    async function get3DMigotoLatestRelease(
        gamePreset: string,
        githubToken?: string,
        includePrerelease = false,
    ): Promise<UpdateInfo | null> {
        const repo = getGithubRepoByGamePreset(gamePreset);

        if (!repo) return null;

        return getLatestGithubRelease(repo, githubToken, includePrerelease);
    }

    async function get3DMigotoReleaseList(
        gamePreset: string,
        githubToken?: string,
        includePrerelease = false,
    ): Promise<UpdateInfo[] | null> {
        const repo = getGithubRepoByGamePreset(gamePreset);

        if (!repo) return null;

        return getGithubReleaseList(repo, githubToken, { includePrerelease });
    }

    async function getXXMILibsLatestRelease(
        githubToken?: string,
        includePrerelease = false,
    ): Promise<UpdateInfo> {
        return getD3d11LatestRelease('dev', githubToken, includePrerelease);
    }

    async function getAppLatestRelease(
        githubToken?: string,
        includePrerelease = false,
    ): Promise<UpdateInfo> {
        return getLatestGithubRelease(
            'Perxenic-Acid/SSMT4',
            githubToken,
            includePrerelease,
        );
    }

    async function getXXMILibsReleaseList(
        githubToken?: string,
        includePrerelease = false,
    ): Promise<UpdateInfo[]> {
        return getD3d11ReleaseList('dev', githubToken, includePrerelease);
    }

    async function getD3d11LatestRelease(
        mode: D3d11Mode,
        githubToken?: string,
        includePrerelease = false,
        gamePreset?: string,
    ): Promise<UpdateInfo> {
        const releases = await getD3d11ReleaseList(mode, githubToken, includePrerelease, gamePreset);
        const release = releases[0];
        if (!release) {
            throw new Error(t('resourceManager.messages.releaseNotFoundForCriteria'));
        }
        return release;
    }

    async function getD3d11ReleaseList(
        mode: D3d11Mode,
        githubToken?: string,
        includePrerelease = false,
        gamePreset?: string,
    ): Promise<UpdateInfo[]> {
        const source = getD3d11ReleaseSource(mode);
        const releases = await getGithubReleaseList(source.repo, githubToken, {
            includePrerelease,
            assetMatcher: source.assetMatcher,
        });
        return constrainD3d11ReleasesForGame(releases, mode, gamePreset);
    }

    /**
     * 前端实现的 3Dmigoto 包更新：下载 zip 到目标目录并使用 PowerShell Expand-Archive 解压。
     * installDir 优先；若为空，则回退到 SSMT4CustomCacheFolder()/3Dmigoto/{gameName}。
     */
    async function install3DMigotoUpdate(
        gameName: string,
        downloadUrl: string,
        cacheDir?: string,
        installDir?: string,
    ): Promise<void> {
        debugLog('3DMigotoUpdate', 'game=', gameName, 'downloadUrl=', downloadUrl, 'cacheDir=', cacheDir, 'installDirArg=', installDir);

        const conf = await loadGameConfig(gameName);
        const confInstallDir = (installDir || conf.installDir || '').trim();

        let target_dir: string | undefined = confInstallDir;

        if (!target_dir) {
            target_dir = await PathHelper.GetCurrentGame3DmigotoFolderPath();
        }

        if (!target_dir) {
            throw new Error(t('resourceManager.messages.cannotUpdateBecauseInstallDirMissing'));
        }

        debugLog('3DMigotoUpdate', 'resolved targetDir=', target_dir);

        if (!(await exists(target_dir))) {
            await mkdir(target_dir, { recursive: true });
            debugLog('3DMigotoUpdate', 'created targetDir');
        }

        const resp = await fetch(downloadUrl, { method: 'GET' });
        if (!resp.ok) {
            const body = await resp.text().catch(() => '');
            throw new Error(t('resourceManager.messages.downloadFailedWithStatusAndBody', {
                status: resp.status,
                body,
            }));
        }
        const bytes = new Uint8Array(await resp.arrayBuffer());
        debugLog('3DMigotoUpdate', 'downloaded bytes=', bytes.length);

        const target_zip_path = await SSMTFileUtils.JoinPath(target_dir, '_3dmigoto_package.zip');
        await writeFile(target_zip_path, bytes);
        debugLog('3DMigotoUpdate', 'wrote zip to', target_zip_path);

        debugLog('3DMigotoUpdate', 'invoking rust extract_zip_archive', { zip_path: target_zip_path, dest_dir: target_dir });
        await invoke('extract_zip_archive', { zipPath: target_zip_path, destDir: target_dir });

        // Cleanup zip
        try {
            await remove(target_zip_path);
            debugLog('3DMigotoUpdate', 'cleaned zip');
        } catch (e) {
            console.warn('cleanup zip failed', e);
        }
    }

    async function installXXMILibsUpdate(downloadUrl: string): Promise<void> {
        return installD3d11Update('dev', downloadUrl);
    }

    async function installD3d11Update(mode: D3d11Mode, downloadUrl: string, gamePreset?: string, version?: string): Promise<void> {
        if (version && isCappedDevD3d11Mode(mode, gamePreset) && !isCappedDevD3d11VersionSupported(version)) {
            throw new Error(t('resourceManager.messages.d3d11VersionCapExceeded', {
                version,
                maxVersion: getCappedDevD3d11MaxVersion(),
            }));
        }
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();
        await SSMTFileUtils.CreateFolderIfNotExists(resourcesDir);

        const tempRoot = await join(resourcesDir, '_xxmi_libs_update_tmp');
        const extractDir = await join(tempRoot, 'extracted');
        const zipPath = await join(tempRoot, 'xxmi-libs-package.zip');
        const source = getD3d11ReleaseSource(mode);

        if (await exists(tempRoot)) {
            await remove(tempRoot, { recursive: true });
        }

        await mkdir(extractDir, { recursive: true });

        try {
            const resp = await fetch(downloadUrl, { method: 'GET' });
            if (!resp.ok) {
                const body = await resp.text().catch(() => '');
                throw new Error(t('resourceManager.messages.downloadFailedWithStatusAndBody', {
                    status: resp.status,
                    body,
                }));
            }

            const bytes = new Uint8Array(await resp.arrayBuffer());
            await writeFile(zipPath, bytes);
            await invoke('extract_zip_archive', { zipPath, destDir: extractDir });

            for (const fileRule of source.filesToInstall) {
                const extractedPath = await findFileRecursive(extractDir, fileRule.sourceFileName);
                if (!extractedPath) {
                    if (fileRule.optional) {
                        continue;
                    }

                    throw new Error(t('resourceManager.messages.requiredUpdateFileMissing', { fileName: fileRule.sourceFileName }));
                }

                const targetFileName = fileRule.sourceFileName.toLowerCase() === 'd3d11.dll'
                    ? getD3d11CacheFileName(mode, gamePreset)
                    : fileRule.targetFileName;
                const targetPath = await join(resourcesDir, targetFileName);
                await copyFileOverwrite(extractedPath, targetPath);
            }
        } finally {
            if (await exists(tempRoot)) {
                try {
                    await remove(tempRoot, { recursive: true });
                } catch (error) {
                    console.warn('Failed to cleanup XXMI libs temp directory', error);
                }
            }
        }
    }

    async function scanGames(): Promise<GameInfo[]> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const normalizedRoot = gamesRoot.replace(/\\/g, '/');
        const results: GameInfo[] = [];

        const sidebarMap = await loadGameIconSidebarMap();

        let entries: Array<{ name?: string; isDirectory?: boolean }> = [];
        try {
            entries = await readDir(gamesRoot);
        } catch (e) {
            return results;
        }

        for (const entry of entries) {
            if (!entry?.isDirectory) continue;
            const name = entry.name || '';
            if (!name) continue;

            const basePath = `${normalizedRoot}/${name}`;

            let iconPath = '';
            let bgPath = '';
            let bgVideoPath: string | null = null;

            const imgCandidates = BACKGROUND_IMAGE_CANDIDATES;
            const videoCandidates = BACKGROUND_VIDEO_CANDIDATES;

            let children = [] as Array<{ name?: string; isDirectory?: boolean }>;
            try {
                children = await readDir(basePath);
            } catch (e) {
                children = [];
            }

            const nameMap = new Map<string, string>();
            children.forEach(c => {
                const n = c.name;
                if (n) nameMap.set(n.toLowerCase(), n);
            });
            debugLog('ScanGames', `Checking ${name} (path: ${basePath}). Found files:`, Array.from(nameMap.values()));

            if (nameMap.has('icon.png')) {
                iconPath = `${basePath}/${nameMap.get('icon.png')}`;
            }

            let bgType: BGType = 'Image';
            const configPath = `${basePath}/Config.json`;
            const cfg = await SSMTJsonUtils.readJson(configPath) as Record<string, unknown> | null;
            if (cfg && cfg.backgroundType) {
                const v = String(cfg.backgroundType);
                bgType = v.toLowerCase() === 'video' ? 'Video' : 'Image';
            } else if (cfg && cfg.background_type) {
                const v = String(cfg.background_type);
                bgType = v.toLowerCase() === 'video' ? 'Video' : 'Image';
            }

            if (bgType === 'Video') {
                for (const c of videoCandidates) {
                    if (nameMap.has(c.toLowerCase())) {
                        bgVideoPath = `${basePath}/${nameMap.get(c.toLowerCase())}`;
                        break;
                    }
                }
                if (!bgVideoPath) {
                    for (const c of imgCandidates) {
                        if (nameMap.has(c.toLowerCase())) {
                            bgPath = `${basePath}/${nameMap.get(c.toLowerCase())}`;
                            bgType = 'Image';
                            break;
                        }
                    }
                }
            } else {
                for (const c of imgCandidates) {
                    if (nameMap.has(c.toLowerCase())) {
                        bgPath = `${basePath}/${nameMap.get(c.toLowerCase())}`;
                        break;
                    }
                }
                if (!bgPath) {
                    for (const c of videoCandidates) {
                        if (nameMap.has(c.toLowerCase())) {
                            bgVideoPath = `${basePath}/${nameMap.get(c.toLowerCase())}`;
                            bgType = 'Video';
                            break;
                        }
                    }
                }
            }

            debugLog('ScanGames', `${name} -> icon: ${iconPath}, bg: ${bgPath}, video: ${bgVideoPath}, type: ${bgType}`);

            const show_sidebar = sidebarMap.get(name) ?? false;

            results.push({
                name,
                icon_path: iconPath,
                bg_path: bgPath,
                bg_video_path: bgVideoPath,
                bg_type: bgType,
                show_sidebar,
            });
        }

        return results;
    }

    async function setGameVisibility(gameName: string, visible: boolean): Promise<void> {
        const configPath = await ensureGlobalGameIconConfigExists();

        const config = await SSMTJsonUtils.readJsonOrDefault<{ GameIconSettingList?: Array<{ GameName: string; Show: boolean }>; list?: Array<{ game_name: string; show: boolean }> }>(configPath, { GameIconSettingList: [] });
        if (!config.GameIconSettingList) {
            if (config.list) {
                config.GameIconSettingList = config.list.map((it) => ({ GameName: it.game_name, Show: it.show }));
            } else {
                config.GameIconSettingList = [];
            }
        }

        const list = config.GameIconSettingList as Array<{ GameName: string; Show: boolean }>;
        const existing = list.find(x => x.GameName === gameName);

        if (existing) {
            existing.Show = visible;
        } else {
            list.push({ GameName: gameName, Show: visible });
        }

        await writeTextFile(configPath, JSON.stringify(config, null, 2));
    }

    function extensionFromUrl(url: string, fallback: string): string {
        try {
            const clean = url.split('?')[0].split('#')[0];
            const segments = clean.split('.');
            const ext = segments.pop();
            if (ext && ext.length <= 5) {
                return ext.toLowerCase();
            }
        } catch (e) {
            console.warn('Failed to parse extension from url', url, e);
        }
        return fallback;
    }

    // ============================================================
    // Return all public methods
    // ============================================================
    return {
        // Game config CRUD
        ensureGameConfigExists,
        createNewConfig,
        deleteGameConfigFolder,
        loadGameConfig,
        saveGameConfig,
        // Game icon / background
        setGameIcon,
        setGameBackground,
        updateGameBackground,
        findGameBackgroundPath,
        // D3D11 mode
        getEffectiveD3d11Mode,
        getGameD3d11Mode,
        resolveD3d11SourcePathByMode,
        resolveBootDllSource,
        resolveMigotoDllSource,
        // Capped dev d3d11.dll version cap helpers
        isCappedDevD3d11Mode,
        getCappedDevD3d11MaxVersion,
        isD3d11VersionAllowedForMode,
        // Global config
        CopyGamesToGlobalConfig,
        Copy3DmigotoDllFiles,
        getMissingXXMILibsFiles,
        // Release fetchers
        get3DMigotoLatestRelease,
        get3DMigotoReleaseList,
        getXXMILibsLatestRelease,
        getAppLatestRelease,
        getXXMILibsReleaseList,
        getD3d11LatestRelease,
        getD3d11ReleaseList,
        // Install
        install3DMigotoUpdate,
        installXXMILibsUpdate,
        installD3d11Update,
        // Scan / visibility
        scanGames,
        setGameVisibility,
        // Utility
        extensionFromUrl,
    };
});

/**
 * Backward-compatible wrapper that delegates every public method and
 * property to the Pinia store instance, so existing callers using
 * `ResourceManager.methodName()` continue to work unchanged.
 */
export const ResourceManager = new Proxy({} as Record<string, unknown>, {
    get(_target, prop: string) {
        const store = useResourceManagerStore();
        const value = (store as unknown as Record<string, unknown>)[prop];
        if (typeof value === 'function') {
            return (...args: unknown[]) => value.apply(store, args);
        }
        return value;
    },
}) as {
    ensureGameConfigExists: (gameName: string) => Promise<GameConfig>;
    createNewConfig: (gameName: string, config?: GameConfig) => Promise<void>;
    deleteGameConfigFolder: (gameName: string) => Promise<void>;
    loadGameConfig: (gameName: string) => Promise<GameConfig>;
    saveGameConfig: (gameName: string, config: GameConfig) => Promise<void>;
    setGameIcon: (gameName: string, sourcePath: string) => Promise<void>;
    setGameBackground: (gameName: string, sourcePath: string, bgType: 'Image' | 'Video') => Promise<void>;
    updateGameBackground: (gameName: string, gamePreset: string, bgType: 'Image' | 'Video', lastUrl?: string) => Promise<{ path: string; url: string; changed: boolean }>;
    findGameBackgroundPath: (gameName: string, bgType?: BGType) => Promise<string>;
    getEffectiveD3d11Mode: (config?: Pick<GameConfig, 'd3d11Mode' | 'gamePreset'> | null) => D3d11Mode;
    getGameD3d11Mode: (gameName: string) => Promise<D3d11Mode>;
    resolveD3d11SourcePathByMode: (mode: D3d11Mode, gamePreset?: string | null) => Promise<string>;
    isCappedDevD3d11Mode: (mode: D3d11Mode, gamePreset?: string | null) => boolean;
    getCappedDevD3d11MaxVersion: () => string;
    isD3d11VersionAllowedForMode: (version: string, mode: D3d11Mode, gamePreset?: string | null) => boolean;
    CopyGamesToGlobalConfig: (includeMihoyoGames?: boolean) => Promise<string>;
    resolveBootDllSource: (gamePreset?: string | null) => Promise<{ sourcePath: string; targetFileName: string; label: string }>;
    resolveMigotoDllSource: (config?: Pick<GameConfig, 'd3d11Mode' | 'gamePreset'> | null) => Promise<{ sourcePath: string; targetFileName: string; label: string; mode: D3d11Mode }>;
    Copy3DmigotoDllFiles: (targetDir: string, gamePreset?: string) => Promise<void>;
    getMissingXXMILibsFiles: (gameName?: string) => Promise<string[]>;
    get3DMigotoLatestRelease: (gamePreset: string, githubToken?: string, includePrerelease?: boolean) => Promise<UpdateInfo | null>;
    get3DMigotoReleaseList: (gamePreset: string, githubToken?: string, includePrerelease?: boolean) => Promise<UpdateInfo[] | null>;
    getXXMILibsLatestRelease: (githubToken?: string, includePrerelease?: boolean) => Promise<UpdateInfo>;
    getAppLatestRelease: (githubToken?: string, includePrerelease?: boolean) => Promise<UpdateInfo>;
    getXXMILibsReleaseList: (githubToken?: string, includePrerelease?: boolean) => Promise<UpdateInfo[]>;
    getD3d11LatestRelease: (mode: D3d11Mode, githubToken?: string, includePrerelease?: boolean, gamePreset?: string) => Promise<UpdateInfo>;
    getD3d11ReleaseList: (mode: D3d11Mode, githubToken?: string, includePrerelease?: boolean, gamePreset?: string) => Promise<UpdateInfo[]>;
    install3DMigotoUpdate: (gameName: string, downloadUrl: string, cacheDir?: string, installDir?: string) => Promise<void>;
    installXXMILibsUpdate: (downloadUrl: string) => Promise<void>;
    installD3d11Update: (mode: D3d11Mode, downloadUrl: string, gamePreset?: string, version?: string) => Promise<void>;
    scanGames: () => Promise<GameInfo[]>;
    setGameVisibility: (gameName: string, visible: boolean) => Promise<void>;
    extensionFromUrl: (url: string, fallback: string) => string;
};
