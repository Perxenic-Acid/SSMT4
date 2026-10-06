import { defineStore } from 'pinia'
import { GlobalConfig } from "./GlobalConfig";
import { SSMTFileUtils } from "../utils/SSMTFileUtils";
import { exists, mkdir, readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';
import { moveDirectoryToRecycleBin } from "../utils/RecycleBin";
import { isValidGamePreset, resolveGamePresetByGameName } from "./GamePreset";

export type HuntingMode = '0' | '1' | '2'

export interface LaunchProgramConfig {
    exePath?: string;
    args?: string;
}

const normalizeHuntingMode = (value: unknown): HuntingMode => {
    if (value === '0' || value === '1' || value === '2') {
        return value
    }
    return '2'
}

const normalizeLaunchProgramItem = (value: unknown): LaunchProgramConfig | null => {
    if (typeof value === 'string') {
        const exePath = value.trim()
        if (!exePath) {
            return null
        }
        return {
            exePath,
            args: '',
        }
    }

    if (!value || typeof value !== 'object') {
        return null
    }

    const candidate = value as LaunchProgramConfig
    return {
        exePath: (candidate.exePath || '').trim(),
        args: candidate.args || '',
    }
}

const normalizeLaunchProgramList = (value: unknown): LaunchProgramConfig[] => {
    if (!Array.isArray(value)) {
        return []
    }

    return value
        .map((item) => normalizeLaunchProgramItem(item))
        .filter((item): item is LaunchProgramConfig => item !== null)
}

const normalizeExtraDll = (value: unknown): string => {
    if (typeof value !== 'string') {
        return ''
    }

    return value.trim()
}

const normalizeExtraDlls = (value: unknown, legacyExtraDll?: unknown): string[] => {
    const normalized = Array.isArray(value)
        ? value
            .filter((item): item is string => typeof item === 'string')
            .map(item => item.trim())
            .filter(Boolean)
        : []

    if (normalized.length > 0) {
        return normalized
    }

    const legacy = normalizeExtraDll(legacyExtraDll)
    return legacy ? [legacy] : []
}

export type GameConfig = {
    gamePreset?: string;
    logicName?: string;
    packageVersion?: string;
    packageReleaseDescription?: string;
    backgroundType?: string;
    pureMode?: boolean;
    check3DmigotoPackageUpdateBeforeLaunch?: boolean;
    includePrereleaseUpdates?: boolean;
    installDir?: string;
    targetExePath?: string;
    launcherExePath?: string;
    launchTargetProgram?: boolean;
    launchArgs?: string;
    showErrorPopup?: boolean;
    autoSetAnalyseOptions?: boolean;
    huntingMode?: HuntingMode;
    useShell?: boolean;
    useUpx?: boolean;
    /**
     * Whether the user has explicitly chosen a DLL packing option.
     * When false/undefined, GIMI defaults to UPX and preset changes may auto-select UPX.
     */
    useUpxManuallySet?: boolean;
    delay?: number;
    autoExitSeconds?: number;
    extraDll?: string;
    extraDlls?: string[];
    useSpecificIbDump?: boolean;
    specificIbDumpRestoreAnalyseOptions?: string;
    configureGame?: boolean;
    applyPerfTweaks?: boolean;
    unlockFps?: boolean;
    forceMaxLodBias?: boolean;
    disableWoundedFx?: boolean;
    preLaunchPrograms?: LaunchProgramConfig[];
    postLaunchPrograms?: LaunchProgramConfig[];
    backgroundUpdateMode?: 'manual' | 'auto';
    lastBackgroundUrl?: string;
    backgroundEntries?: Array<{
        id: string;
        type: 'Image' | 'Video';
        imagePath?: string;
        videoPath?: string;
        themePath?: string;
        imageUrl?: string;
        videoUrl?: string;
        themeUrl?: string;
    }>;
    selectedBackgroundId?: string;

    [key: string]: unknown;
};

const DEFAULT_UPX_GAME_PRESETS: ReadonlySet<string> = new Set(['GIMI'])

export const getDefaultUseUpxForGamePreset = (gamePreset: unknown): boolean => {
    const normalizedPreset = typeof gamePreset === 'string' ? gamePreset.trim().toUpperCase() : ''
    return DEFAULT_UPX_GAME_PRESETS.has(normalizedPreset)
}

/**
 * Resolves the actual DLL packing setting for a config.
 *
 * GIMI defaults to UPX unless the user has explicitly picked another option.
 * For other presets we keep the stored value (falling back to no packing).
 */
export const getEffectiveUseUpx = (
    config: Pick<GameConfig, 'useUpx' | 'useUpxManuallySet' | 'gamePreset'> | null | undefined,
): boolean => {
    if (!config) return false

    if (config.useUpxManuallySet === true) {
        return config.useUpx === true
    }

    if (getDefaultUseUpxForGamePreset(config.gamePreset)) {
        return true
    }

    return config.useUpx === true
}

export const useGameConfigStore = defineStore('gameConfig', () => {
    async function getConfigPath(gameName: string): Promise<string> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        return SSMTFileUtils.JoinPath(gamesRoot, gameName, 'Config.json');
    }

    function buildInitialConfig(gameName: string, config?: GameConfig): GameConfig {
        const defaultPreset = resolveGamePresetByGameName(gameName);
        return normalizeConfig({
            ...defaultGameConfig(),
            ...(defaultPreset ? { gamePreset: defaultPreset } : null),
            ...(config || {}),
        });
    }

    function normalizeConfig(config: GameConfig): GameConfig {
        // 旧版本的 DLL 来源与更新开关不再参与启动，也不写回新配置。
        const { d3d11Mode: _legacyMode, allowDllUpdates: _legacyUpdates,
            checkDllUpdateBeforeLaunch: _legacyCheck, ...currentConfig } = config
        const extraDlls = normalizeExtraDlls(config.extraDlls, config.extraDll)
        const extraDll = extraDlls[0] || ''
        const useUpxManuallySet = config.useUpxManuallySet === true
        const useUpx = getEffectiveUseUpx(config)

        return {
            ...currentConfig,
            huntingMode: normalizeHuntingMode(config.huntingMode),
            launchTargetProgram: config.launchTargetProgram !== false,
            useUpx,
            useUpxManuallySet,
            extraDll,
            extraDlls,
            useShell: extraDlls.length > 0 ? false : !!config.useShell,
            preLaunchPrograms: normalizeLaunchProgramList(config.preLaunchPrograms),
            postLaunchPrograms: normalizeLaunchProgramList(config.postLaunchPrograms),
        }
    }

    function applyMatchedGamePreset(gameName: string, config: GameConfig): GameConfig {
        const matchedPreset = resolveGamePresetByGameName(gameName);
        if (!matchedPreset) {
            return config;
        }

        if (isValidGamePreset(config.gamePreset)) {
            return config;
        }

        return {
            ...config,
            gamePreset: matchedPreset,
        };
    }

    function defaultGameConfig(): GameConfig {
        return {
            gamePreset: '',
            packageVersion: '',
            packageReleaseDescription: '',
            backgroundType: 'Image',
            pureMode: false,
            check3DmigotoPackageUpdateBeforeLaunch: true,
            includePrereleaseUpdates: true,
            installDir: '',
            targetExePath: '',
            launcherExePath: '',
            launchTargetProgram: true,
            launchArgs: '',
            showErrorPopup: true,
            autoSetAnalyseOptions: true,
            huntingMode: '2',
            useShell: false,
            useUpx: false,
            useUpxManuallySet: false,
            delay: 100,
            autoExitSeconds: 5,
            extraDll: '',
            extraDlls: [],
            configureGame: true,
            applyPerfTweaks: false,
            unlockFps: false,
            forceMaxLodBias: false,
            disableWoundedFx: false,
            preLaunchPrograms: [],
            postLaunchPrograms: [],
            backgroundUpdateMode: 'manual',
        };
    }

    async function createNewConfig(gameName: string, config?: GameConfig): Promise<void> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);

        if (!(await exists(dirPath))) {
            await mkdir(dirPath, { recursive: true });
        }

        const newConfig = buildInitialConfig(gameName, config);
        await saveGameConfig(gameName, newConfig);
    }

    async function ensureConfigExists(gameName: string): Promise<GameConfig> {
        const configPath = await getConfigPath(gameName);
        if (await exists(configPath)) {
            const loadedConfig = await loadGameConfig(gameName);
            const normalizedConfig = applyMatchedGamePreset(gameName, loadedConfig);
            if (normalizedConfig.gamePreset !== loadedConfig.gamePreset) {
                await saveGameConfig(gameName, normalizedConfig);
            }
            return normalizedConfig;
        }

        const newConfig = buildInitialConfig(gameName);
        await createNewConfig(gameName, newConfig);
        return newConfig;
    }

    async function deleteGameConfigFolder(gameName: string): Promise<void> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);
        if (await exists(dirPath)) {
            await moveDirectoryToRecycleBin(dirPath);
        }
    }

    async function loadGameConfig(gameName: string): Promise<GameConfig> {
        const configPath = await getConfigPath(gameName);
        try {
            const raw = await readTextFile(configPath);
            let parsed = JSON.parse(raw);

            const def = defaultGameConfig();
            return normalizeConfig(applyMatchedGamePreset(gameName, {
                ...def,
                ...parsed, // Use values directly without backwards compatibility for legacy nested structures
            } as GameConfig));
        } catch {
            return buildInitialConfig(gameName);
        }
    }

    async function saveGameConfig(gameName: string, config: GameConfig): Promise<void> {
        const gamesRoot = await GlobalConfig.GlobalGamesFolder();
        const dirPath = await SSMTFileUtils.JoinPath(gamesRoot, gameName);
        const configPath = await SSMTFileUtils.JoinPath(dirPath, 'Config.json');
        const normalizedConfig = normalizeConfig(applyMatchedGamePreset(gameName, config));

        await mkdir(dirPath, { recursive: true });
        await writeTextFile(configPath, JSON.stringify(normalizedConfig, null, 2));
    }

    return {
        defaultGameConfig,
        createNewConfig,
        ensureConfigExists,
        deleteGameConfigFolder,
        loadGameConfig,
        saveGameConfig,
    }
})

// Backward-compatible wrapper so existing imports of GameConfigManager work unchanged.
export const GameConfigManager = {
    get defaultGameConfig() { return useGameConfigStore().defaultGameConfig },
    get createNewConfig() { return useGameConfigStore().createNewConfig },
    get ensureConfigExists() { return useGameConfigStore().ensureConfigExists },
    get deleteGameConfigFolder() { return useGameConfigStore().deleteGameConfigFolder },
    get loadGameConfig() { return useGameConfigStore().loadGameConfig },
    get saveGameConfig() { return useGameConfigStore().saveGameConfig },
}
