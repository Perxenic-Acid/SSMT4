import { invoke } from "@tauri-apps/api/core";
import { exists, mkdir, copyFile, remove, writeTextFile } from "@tauri-apps/plugin-fs";
import { join } from "@tauri-apps/api/path";
import { ElMessage, ElMessageBox } from "element-plus";
import { ResourceManager } from "../store/ResourceManager";
import { MigotoManager, type RuntimeSyncResult } from "../store/MigotoManager";
import { GlobalConfig } from "../store/GlobalConfig";
import { PathHelper } from "../helper/PathHelper";
import { i18n } from "../i18n";
import { getEffectiveUseUpx, type GameConfig, type LaunchProgramConfig } from "../store/GameConfig";
import type { AppSettings } from "../store/AppSettings";
import { debugLog, debugWarn } from "../utils/debugLog";
import { serializePlayerTweaksCamera } from '../plugin/playerTweaksCamera';
import { serializePlayerTweaksGameplay } from '../plugin/playerTweaksGameplay';

const t = i18n.global.t;

export interface ProgramToLaunch {
    path: string;
    args?: string;
    workDir?: string;
    postLaunchDelayMs?: number;
    waitForProcessName?: string;
    waitTimeoutSecs?: number;
    waitOnly?: boolean;
    runAsAdministrator?: boolean;
    logSources?: Array<{ name: string; path: string }>;
}

interface DiscoveredGamePaths {
    targetExePath: string;
    launcherExePath: string;
}

interface PluginLaunchProgram {
    path: string;
    args: string;
    workDir: string;
    runAsAdministrator: boolean;
    postLaunchDelayMs: number;
}

interface HotRuntimePlan {
    dlls: string[];
    environment: Record<string, string | null>;
    runtimeDirectory: string;
    requiredLaunchArgs?: string[];
    excludedLaunchArgs?: string[];
    logSources?: Array<{ name: string; path: string }>;
}

type GraphicsLaunchDecision =
    | "continue_this_launch"
    | "prepare_managed_stack"
    | "suppress_hoyoshade_this_launch";

type GraphicsCompatibilityLevel =
    | "compatible"
    | "warning"
    | "requires_managed_stack"
    | "conflict";

interface GraphicsLaunchInspection {
    hoyoshadeRequested: boolean;
    dlss5Requested: boolean;
    hostRequested: boolean;
    resolution: {
        level: GraphicsCompatibilityLevel;
        issues: Array<{ code: string; message: string }>;
    };
}

interface ManagedStackStatus {
    route: string;
    availableRoutes: string[];
    swapperOwner: string;
}

type LaunchProgramPhase = "preLaunchPrograms" | "postLaunchPrograms";

export class LaunchGame {
    private static async preflightGraphicsLaunch(
        gameName: string,
        targetExe: string,
    ): Promise<GraphicsLaunchDecision | null> {
        const inspection = await invoke<GraphicsLaunchInspection>(
            "inspect_graphics_launch",
            { gameName, gameExecutable: targetExe },
        );
        const { level, issues } = inspection.resolution;
        if (!inspection.hostRequested || level === "compatible") {
            return "continue_this_launch";
        }

        const message = issues[0]?.message || t("launchGame.messages.graphicsConflict");
        if (level === "requires_managed_stack") {
            let status: ManagedStackStatus;
            try {
                status = await invoke<ManagedStackStatus>("inspect_managed_graphics_stack", {
                    gameName,
                    gameExecutable: targetExe,
                });
            } catch (error) {
                if (inspection.dlss5Requested) {
                    ElMessage.error(`${message}\n${String(error)}`);
                    return null;
                }
                try {
                    await ElMessageBox.confirm(
                        `${message}\n\n${String(error)}`,
                        t("launchGame.messages.graphicsStackTitle"),
                        {
                            confirmButtonText: t("launchGame.messages.graphicsSuppressHoYoShade"),
                            cancelButtonText: t("launchGame.common.cancel"),
                            type: "warning",
                        },
                    );
                    return "suppress_hoyoshade_this_launch";
                } catch {
                    return null;
                }
            }
            if (inspection.dlss5Requested) {
                try {
                    await ElMessageBox.confirm(
                        t("launchGame.messages.graphicsManagedReady", {
                            route: status.route,
                            routes: status.availableRoutes.join(", "),
                            owner: status.swapperOwner,
                        }),
                        t("launchGame.messages.graphicsStackTitle"),
                        {
                            confirmButtonText: t("launchGame.messages.graphicsPrepareManaged"),
                            cancelButtonText: t("launchGame.common.cancel"),
                            type: "warning",
                        },
                    );
                    return "prepare_managed_stack";
                } catch {
                    return null;
                }
            }
            try {
                await ElMessageBox.confirm(
                    t("launchGame.messages.graphicsManagedReady", {
                        route: status.route,
                        routes: status.availableRoutes.join(", "),
                        owner: status.swapperOwner,
                    }),
                    t("launchGame.messages.graphicsStackTitle"),
                    {
                        confirmButtonText: t("launchGame.messages.graphicsPrepareManaged"),
                        cancelButtonText: t("launchGame.messages.graphicsSuppressHoYoShade"),
                        distinguishCancelAndClose: true,
                        showClose: true,
                        type: "warning",
                    },
                );
                return "prepare_managed_stack";
            } catch (action) {
                return action === "cancel" ? "suppress_hoyoshade_this_launch" : null;
            }
        }
        if (level === "warning") {
            try {
                await ElMessageBox.confirm(message, t("launchGame.messages.graphicsStackTitle"), {
                    confirmButtonText: t("launchGame.messages.graphicsContinue"),
                    cancelButtonText: t("launchGame.messages.graphicsSuppressHoYoShade"),
                    distinguishCancelAndClose: true,
                    showClose: true,
                    type: "warning",
                });
                return "continue_this_launch";
            } catch (action) {
                return action === "cancel" ? "suppress_hoyoshade_this_launch" : null;
            }
        }

        try {
            await ElMessageBox.confirm(message, t("launchGame.messages.graphicsStackTitle"), {
                confirmButtonText: t("launchGame.messages.graphicsSuppressHoYoShade"),
                cancelButtonText: t("launchGame.common.cancel"),
                type: "warning",
            });
            return "suppress_hoyoshade_this_launch";
        } catch {
            return null;
        }
    }

    private static formatProgramSummary(program: ProgramToLaunch): string {
        return [
            `path=${program.path || "<wait-only>"}`,
            `name=${this.getProcessNameFromPath(program.path) || "<none>"}`,
            `workDir=${program.workDir || "<auto>"}`,
            `args=${program.args || "<none>"}`,
            `waitForProcessName=${program.waitForProcessName || "<none>"}`,
            `waitTimeoutSecs=${program.waitTimeoutSecs ?? "<none>"}`,
            `waitOnly=${program.waitOnly ? "true" : "false"}`,
        ].join(", ");
    }

    private static formatProgramsForLog(programs: ProgramToLaunch[]): string {
        return programs
            .map(
                (program, index) =>
                    `#${index + 1}: ${this.formatProgramSummary(program)}`,
            )
            .join("\n");
    }

    private static getProcessNameFromPath(filePath: string): string {
        const trimmed = (filePath || "").trim();
        if (!trimmed) {
            return "";
        }

        const lastSlash = Math.max(
            trimmed.lastIndexOf("/"),
            trimmed.lastIndexOf("\\"),
        );
        return lastSlash >= 0 ? trimmed.substring(lastSlash + 1) : trimmed;
    }

    private static applyRequiredLaunchArgs(raw: string, plan: HotRuntimePlan): string {
        const required = plan.requiredLaunchArgs || [];
        const excluded = plan.excludedLaunchArgs || [];
        if (!required.length && !excluded.length) return raw;
        if (![...required, ...excluded].every(flag => /^-[a-z0-9_-]+$/i.test(flag))) {
            throw new Error("插件启动参数声明无效");
        }
        const rejected = new Set([...required, ...excluded].map(flag => flag.toLowerCase()));
        const tokens = raw.match(/"[^"\r\n]*"|[^\s"]+(?:"[^"\r\n]*"[^\s"]*)*/g) || [];
        return [...tokens.filter(token => !rejected.has(token.replace(/^"|"$/g, '').toLowerCase())), ...required].join(' ');
    }

    private static async configureWWMILaunchSettings(
        targetExe: string,
        config: GameConfig,
    ): Promise<void> {
        await invoke("configure_wwmi_launch_settings", {
            options: {
                targetExePath: targetExe,
                configureGame: config.configureGame !== false,
                applyPerfTweaks: !!config.applyPerfTweaks,
                unlockFps: !!config.unlockFps,
                forceMaxLodBias: !!config.forceMaxLodBias,
                disableWoundedFx: !!config.disableWoundedFx,
            },
        });
    }

    private static async configureZZMILaunchSettings(
        targetExe: string,
        config: GameConfig,
    ): Promise<void> {
        await invoke("configure_zzmi_launch_settings", {
            options: {
                targetExePath: targetExe,
                configureGame: config.configureGame !== false,
            },
        });
    }

    private static getLaunchProgramGroupLabel(
        phase: LaunchProgramPhase,
    ): string {
        return t(`gameSettingsModal.fields.${phase}`);
    }

    private static isD3d11BusyError(errorText: string): boolean {
        const normalized = errorText.toLowerCase();
        if (!normalized.includes("d3d11.dll")) {
            return false;
        }

        return [
            "os error 32",
            "being used by another process",
            "process cannot access the file",
            "access is denied",
            "permission denied",
            "另一个程序正在使用",
            "正由另一进程使用",
            "无法访问该文件",
            "拒绝访问",
            "权限不足",
        ].some((keyword) => normalized.includes(keyword.toLowerCase()));
    }

    private static formatLaunchError(errorText: string): string {
        if (!this.isD3d11BusyError(errorText)) {
            return errorText;
        }

        return `${errorText}\n\n${t("launchGame.messages.d3d11BusyHint")}`;
    }

    private static async buildConfiguredPrograms(
        programsConfig: LaunchProgramConfig[] | undefined,
        phase: LaunchProgramPhase,
    ): Promise<ProgramToLaunch[] | null> {
        const programs: ProgramToLaunch[] = [];

        for (const [index, item] of (programsConfig || []).entries()) {
            const exePath = (item.exePath || "").trim();
            const args = item.args || "";
            const hasAnyValue = exePath.length > 0 || args.trim().length > 0;

            if (!hasAnyValue) {
                continue;
            }

            if (!exePath) {
                ElMessage.error(
                    t("launchGame.messages.customProgramPathMissing", {
                        group: this.getLaunchProgramGroupLabel(phase),
                        index: index + 1,
                    }),
                );
                return null;
            }

            if (!(await exists(exePath))) {
                ElMessage.error(
                    t("launchGame.messages.customProgramFileNotFound", {
                        group: this.getLaunchProgramGroupLabel(phase),
                        index: index + 1,
                    }),
                );
                return null;
            }

            programs.push({
                path: exePath,
                args,
            });
        }

        return programs;
    }

    static async resolveMigotoDirForLaunch(
        _gameName: string,
        _cfg: GameConfig,
        _appSettings: AppSettings,
    ): Promise<string> {
        const resolved = await PathHelper.GetCurrentGame3DmigotoFolderPath();
        if (resolved && resolved.trim()) return resolved;
        throw new Error(t("launchGame.messages.configureMigotoPathFirst"));
    }

    static async prepareLaunch(
        gameName: string,
        appSettings: AppSettings,
        pureMode: boolean,
        onNeedsConfigureProcessPath?: () => void,
        onNeedsPackageUpdate?: () => Promise<boolean | void> | boolean | void,
        onGraphicsPreflight?: (targetExe: string) => Promise<GraphicsLaunchDecision | null>,
    ): Promise<{
        migotoDir: string;
        config: GameConfig;
        targetExe: string;
        graphicsDecision: GraphicsLaunchDecision;
    } | null> {
        const config = await ResourceManager.loadGameConfig(gameName);
        const migotoCfg = config ?? ({} as GameConfig);
        const launchTargetProgram = migotoCfg.launchTargetProgram !== false;
        let targetExe = (migotoCfg.targetExePath || "").trim();
        const isConfiguredTargetValid =
            targetExe.length > 0 && (await exists(targetExe));
        const useShell = launchTargetProgram && !!migotoCfg.useShell;
        const configuredLauncher = (migotoCfg.launcherExePath || "").trim();
        const isConfiguredLauncherValid = !useShell || (
            configuredLauncher.length > 0 && (await exists(configuredLauncher))
        );
        const gamePreset = (migotoCfg.gamePreset || "").trim().toUpperCase();
        const supportsGameDiscovery = [
            "GIMI",
            "SRMI",
            "ZZMI",
            "NTEMI",
            "WWMI",
        ].includes(gamePreset);

        let configChanged = false;

        if (
            launchTargetProgram &&
            (!isConfiguredTargetValid || !isConfiguredLauncherValid) &&
            supportsGameDiscovery
        ) {
            targetExe = "";
            debugLog(
                "GameLauncher",
                `${gamePreset} target is missing or invalid; starting silent discovery.`,
            );
            const discovered = await invoke<DiscoveredGamePaths | null>(
                "find_game_executable",
                { gamePreset },
            );
            if (discovered) {
                targetExe = discovered.targetExePath;
                migotoCfg.targetExePath = discovered.targetExePath;
                migotoCfg.launcherExePath = discovered.launcherExePath;
                configChanged = true;
                ElMessage.info(
                    t("launchGame.messages.gameTargetMatched", {
                        path: discovered.targetExePath,
                    }),
                );
                ElMessage.info(
                    t("launchGame.messages.gameLauncherMatched", {
                        path: discovered.launcherExePath,
                    }),
                );
                debugLog(
                    "GameLauncher",
                    `Discovered and saved ${gamePreset} paths.`,
                    discovered,
                );
            }
        }

        const requiresTargetPath = launchTargetProgram || gamePreset === "EFMI";
        if (requiresTargetPath && !targetExe) {
            ElMessage.warning(
                t("launchGame.messages.targetProcessPathNotConfigured"),
            );
            onNeedsConfigureProcessPath?.();
            return null;
        }

        if (requiresTargetPath && !(await exists(targetExe))) {
            ElMessage.error(t("launchGame.messages.targetProcessFileNotFound"));
            return null;
        }

        const pureDirectLaunch = pureMode && launchTargetProgram;
        const graphicsDecision = !pureDirectLaunch && launchTargetProgram && onGraphicsPreflight
            ? await onGraphicsPreflight(targetExe)
            : "continue_this_launch";
        if (!graphicsDecision) return null;

        if (configChanged) {
            await ResourceManager.saveGameConfig(gameName, migotoCfg);
        }

        // A pure launch must not depend on, prepare, or mutate the 3DMigoto
        // runtime. It is intentionally just the configured game executable.
        if (pureDirectLaunch) {
            return { migotoDir: "", config: migotoCfg, targetExe, graphicsDecision };
        }

        let configuredMigotoDir = (migotoCfg.installDir || "").trim();
        const configuredD3dxIni = configuredMigotoDir
            ? await join(configuredMigotoDir, "d3dx.ini")
            : "";
        const isMigotoDirValid =
            configuredMigotoDir.length > 0 &&
            (await exists(configuredMigotoDir)) &&
            (await exists(configuredD3dxIni));
        if (!isMigotoDirValid) {
            const cacheRoot = await GlobalConfig.SSMT4CustomCacheFolder();
            configuredMigotoDir = await join(cacheRoot, "3Dmigoto", gameName);
            migotoCfg.installDir = configuredMigotoDir;
            await ResourceManager.saveGameConfig(gameName, migotoCfg);

            ElMessage.info(
                t("launchGame.messages.migotoDirectoryRestoring", {
                    path: configuredMigotoDir,
                }),
            );
            const updateHandled = await onNeedsPackageUpdate?.();
            const restoredD3dxIni = await join(configuredMigotoDir, "d3dx.ini");
            if (updateHandled === false || !(await exists(restoredD3dxIni))) {
                ElMessage.warning(
                    t("launchGame.messages.migotoDirectoryRestoreFailed"),
                );
                return null;
            }
        }

        if (launchTargetProgram && (migotoCfg.gamePreset || "").trim() === "GIMI") {
            await invoke("configure_gimi_launch_settings", {
                options: { targetExePath: targetExe, configureGame: migotoCfg.configureGame !== false },
            });
        }

        if (
            launchTargetProgram &&
            (migotoCfg.gamePreset || "").trim() === "WWMI"
        ) {
            await this.configureWWMILaunchSettings(targetExe, migotoCfg);
        }

        if (
            launchTargetProgram &&
            (migotoCfg.gamePreset || "").trim() === "ZZMI"
        ) {
            await this.configureZZMILaunchSettings(targetExe, migotoCfg);
        }

        let migotoDir = "";
        try {
            migotoDir = await this.resolveMigotoDirForLaunch(
                gameName,
                migotoCfg,
                appSettings,
            );
        } catch (err: unknown) {
            ElMessage.error(err instanceof Error ? err.message : String(err));
            return null;
        }

        // 静默发现 SSMT Runtime 发布并部署；锁定时保留当前 DLL，失败时使用已有版本。
        // UPX: respect per-game config, but default GIMI to UPX until the user chooses otherwise.
        const useUpx = getEffectiveUseUpx(migotoCfg);
        const environmentReady = await this.prepareGameEnvironment(
            gameName,
            migotoDir,
            migotoCfg,
            useUpx,
        );
        if (!environmentReady) return null;

        if (!pureMode || !launchTargetProgram) {
            const runExePath = await join(migotoDir, "Run.exe");
            if (!(await exists(runExePath))) {
                ElMessage.error(t("launchGame.messages.runExeMissing"));
                return null;
            }
        }

        return { migotoDir, config: migotoCfg, targetExe, graphicsDecision };
    }

    static async launch(
        gameName: string,
        appSettings: AppSettings,
        onNeedsUpdate: () => Promise<boolean | void> | boolean | void,
        onNeedsConfigureProcessPath?: () => void,
        ctrlPressed = false,
    ): Promise<'native' | 'direct' | false> {
        let managedStagedTarget: string | null = null;
        let launchSubmitted = false;
        try {
            const pureMode =
                appSettings.gameLaunchMode === "always-pure" ||
                (appSettings.gameLaunchMode === "ctrl-pure" && ctrlPressed);
            const preflight = await this.prepareLaunch(
                gameName,
                appSettings,
                pureMode,
                onNeedsConfigureProcessPath,
                onNeedsUpdate,
                (targetExe) => this.preflightGraphicsLaunch(gameName, targetExe),
            );
            if (!preflight) return false;

            const { migotoDir, config, targetExe, graphicsDecision } = preflight;
            const launchTargetProgram = config.launchTargetProgram !== false;
            const pureDirectTarget = pureMode && launchTargetProgram;
            if (!pureDirectTarget) {
                await this.ensureSSMTRuntimeFiles(migotoDir, gameName, config.gamePreset);
            }
            const launcherExePath = (config.launcherExePath || "").trim();
            const targetProcessName = this.getProcessNameFromPath(targetExe);
            let useShell = launchTargetProgram && (config.useShell || false);
            const useRunInjector = !pureMode || !launchTargetProgram;
            const useEfmiDirectWait = useRunInjector &&
                (config.gamePreset || "").trim().toUpperCase() === "EFMI";

            if (launchTargetProgram && useShell && !launcherExePath) {
                ElMessage.warning(
                    t("launchGame.messages.shellLauncherPathRequired"),
                );
                return false;
            }

            if (
                launchTargetProgram &&
                useShell &&
                launcherExePath &&
                !(await exists(launcherExePath))
            ) {
                ElMessage.error(
                    t("launchGame.messages.launcherProgramFileNotFound", {
                        path: launcherExePath,
                    }),
                );
                return false;
            }

            // Check 3Dmigoto Integrity
            const safe = pureDirectTarget || await MigotoManager.check3DmigotoIntegrity(gameName);
            if (!safe) {
                try {
                    await ElMessageBox.confirm(
                        t(
                            "launchGame.messages.missingCoreComponentConfirmContent",
                        ),
                        t("launchGame.messages.missingCoreComponentTitle"),
                        {
                            confirmButtonText: t(
                                "launchGame.common.checkUpdate",
                            ),
                            cancelButtonText: t("launchGame.common.cancel"),
                            type: "warning",
                        },
                    );
                    onNeedsUpdate();
                } catch {
                    // Cancelled
                }
                return false;
            }

            if (!pureDirectTarget) {
                await MigotoManager.patchD3dxForLaunch(gameName, { overlayLanguage: appSettings.runtimeOverlayLanguage ?? 'auto' });
            }

            const preLaunchPrograms = launchTargetProgram
                ? await this.buildConfiguredPrograms(
                      config.preLaunchPrograms,
                      "preLaunchPrograms",
                  )
                : [];
            if (!preLaunchPrograms) return false;

            const postLaunchPrograms = launchTargetProgram
                ? await this.buildConfiguredPrograms(
                      config.postLaunchPrograms,
                      "postLaunchPrograms",
                  )
                : [];
            if (!postLaunchPrograms) return false;

            // Construct programs list
            const programs: ProgramToLaunch[] = [...preLaunchPrograms];

            let hotRuntime: HotRuntimePlan | null = null;
            let graphicsHotHandled = false;
            let skipDlss5 = false;
            let hotRuntimeDlls: string[] = [];
            let hotLaunchArgs = config.launchArgs || "";
            const contributionWarnings = new Map<string, Set<string>>();
            const warnContribution = (plugin: string, error: unknown) => {
                const reason = String(error).trim();
                const plugins = contributionWarnings.get(reason) ?? new Set<string>();
                plugins.add(plugin);
                contributionWarnings.set(reason, plugins);
            };
            const collectHotRuntimeDlls = (plan: HotRuntimePlan): string[] => {
                const configuredExtras = Array.isArray(config.extraDlls)
                    ? config.extraDlls.filter((dll): dll is string => typeof dll === "string").map(dll => dll.trim()).filter(Boolean)
                    : [];
                const legacyExtra = typeof config.extraDll === "string" ? config.extraDll.trim() : "";
                const extras = configuredExtras.length ? configuredExtras : legacyExtra ? [legacyExtra] : [];
                const owned = plan.dlls.map(path => this.getProcessNameFromPath(path).toLowerCase());
                for (const dll of extras) {
                    if ([...owned, "d3d11.dll", "dxgi.dll"].includes(this.getProcessNameFromPath(dll).toLowerCase())) {
                        throw new Error(`热注入组件与额外 DLL 重复或所有权冲突：${dll}`);
                    }
                }
                return [...plan.dlls, ...extras];
            };
            if (!pureDirectTarget && useRunInjector && targetExe) {
                const comparablePath = (path: string) => path.replace(/\\/g, "/").replace(/^\/\/\?\//, "").toLowerCase();
                const canCreateTarget = launchTargetProgram && !useEfmiDirectWait &&
                    (!useShell || comparablePath(await join(launcherExePath)) === comparablePath(await join(targetExe)));
                const excludedProviders: string[] = [];
                try {
                    while (true) {
                        const prepared = await invoke<{
                            candidateId: string | null; plugin: string | null; plan: HotRuntimePlan | null;
                            handled: boolean; diagnostics: Array<{ candidateId: string; plugin: string; error: string }>;
                        }>("prepare_plugin_graphics_plan", { gameName, gameExecutable: targetExe, allowHotRuntime: canCreateTarget, excludedProviders, graphicsDecision });
                        graphicsHotHandled ||= prepared.handled;
                        for (const diagnostic of prepared.diagnostics) {
                            skipDlss5 = true;
                            warnContribution(diagnostic.plugin, diagnostic.error);
                            if (diagnostic.candidateId && !excludedProviders.includes(diagnostic.candidateId)) {
                                excludedProviders.push(diagnostic.candidateId);
                            }
                        }
                        if (!prepared.plan) break;
                        const candidateId = prepared.candidateId;
                        if (!candidateId || excludedProviders.includes(candidateId)) throw new Error("插件返回了重复或无效的启动候选");
                        try {
                            const dlls = collectHotRuntimeDlls(prepared.plan);
                            const args = this.applyRequiredLaunchArgs(config.launchArgs || "", prepared.plan);
                            await invoke("configure_plugin_launch_contract", { gameName, gameExecutable: targetExe, candidateId });
                            hotRuntime = prepared.plan;
                            hotRuntimeDlls = dlls;
                            hotLaunchArgs = args;
                            useShell = false;
                            break;
                        } catch (error) {
                            excludedProviders.push(candidateId);
                            skipDlss5 = true;
                            warnContribution(prepared.plugin || "Plugin", error);
                        }
                    }
                } catch (error) {
                    graphicsHotHandled = true;
                    skipDlss5 = true;
                    warnContribution("Plugin", error);
                }
            }
            let hoyoshade: PluginLaunchProgram | null = null;
            if (!hotRuntime && !graphicsHotHandled && !pureDirectTarget && launchTargetProgram && targetExe) {
                try {
                    hoyoshade = await invoke<PluginLaunchProgram | null>("prepare_hoyoshade_launch", {
                      gameName,
                      gameExecutable: targetExe,
                      graphicsDecision,
                      skipDlss5,
                    });
                } catch (error) {
                    if (!skipDlss5) throw error;
                    console.warn("Shade Bridge 准备失败，继续普通启动：", error);
                    warnContribution("Shade Bridge", error);
                }
            }
            for (const [error, plugins] of contributionWarnings) {
                ElMessage.warning({
                    message: t("launchGame.messages.graphicsPluginUnavailableContinue", { plugin: [...plugins].join("、"), error }),
                    grouping: true,
                    showClose: true,
                    duration: 12000,
                });
            }
            // Runtime plugins are injected by Run.exe, including configured-target mode.
            const effectivePluginHostConfig = useRunInjector
                ? await invoke<string | null>("prepare_plugin_host_config", {
                      gameName,
                      runtimeDirectory: migotoDir,
                  })
                : null;
            if (effectivePluginHostConfig) {
                await this.ensureSSMTPluginHostFile(migotoDir);
            }
            if (hoyoshade && graphicsDecision === "prepare_managed_stack") {
                managedStagedTarget = targetExe;
            }

            if (hoyoshade) {
                // HoYoShade's injector waits for the target process itself, so it
                // must be started before Run.exe/the game launcher.
                programs.push({
                    path: hoyoshade.path,
                    args: hoyoshade.args,
                    workDir: hoyoshade.workDir,
                    runAsAdministrator: hoyoshade.runAsAdministrator,
                    postLaunchDelayMs: hoyoshade.postLaunchDelayMs,
                });
            }

            if (hotRuntime) {
                await MigotoManager.patchD3dxForLaunch(gameName, {
                    extraDlls: hotRuntimeDlls,
                    launchArgs: hotLaunchArgs,
                    useShell: false,
                    overlayLanguage: appSettings.runtimeOverlayLanguage ?? 'auto',
                });
            }

            if (useRunInjector) {
                const injectorArgs: string[] = ['--machine-readable'];
                if (hotRuntime) {
                    injectorArgs.push("--preload-runtime");
                    for (const [name, value] of Object.entries(hotRuntime.environment)) {
                        injectorArgs.push(value === null ? `--unset-env ${name}` : `--set-env "${name}=${value}"`);
                    }
                }
                if (effectivePluginHostConfig) {
                    injectorArgs.push(`--plugin-host-config "${effectivePluginHostConfig.replace(/"/g, '\\"')}"`);
                }
                if (useEfmiDirectWait) {
                    injectorArgs.push("--inject-mode direct --wait-for-target --process-timeout 120");
                }
                // 1. Default flow launches Run.exe first.
                programs.push({
                    path: await join(migotoDir, "Run.exe"),
                    logSources: hotRuntime?.logSources,
                    workDir: migotoDir,
                    args: injectorArgs.join(" ") || undefined,
                    // Run.exe 带有 requireAdministrator manifest。SSMT 未提权时，
                    // Start-Process 必须显式使用 RunAs，否则 Windows 会返回
                    // ERROR_ELEVATION_REQUIRED，HoYoShade 也会一直等待不存在的进程。
                    runAsAdministrator: true,
                    waitForProcessName: useShell ? "Run.exe" : undefined,
                    waitTimeoutSecs: useShell ? 30 : undefined,
                });
                if (useEfmiDirectWait) {
                    ElMessage.info({ message: t("launchGame.messages.efmiOfficialLauncherWait"), duration: 15000 });
                }
            }

            // 2. Pure mode launches the target directly; shell mode keeps existing behavior.
            if (launchTargetProgram && (pureMode || useShell)) {
                const exePath = useShell ? launcherExePath : targetExe;
                if (!exePath) {
                    debugWarn(
                        "GameLauncher",
                        "Skipping main launch step because executable path is empty.",
                        {
                            useShell,
                            pureMode,
                            launcherExePath,
                            targetExe,
                        },
                    );
                    return false;
                }
                programs.push({
                    path: exePath,
                    args: config.launchArgs || "",
                });
            }

            const shouldWaitForTargetProcess =
                launchTargetProgram &&
                !!targetProcessName &&
                ((!pureMode && !useShell && !useEfmiDirectWait) || postLaunchPrograms.length > 0);

            if (shouldWaitForTargetProcess) {
                programs.push({
                    path: "",
                    waitOnly: true,
                    waitForProcessName: targetProcessName,
                    waitTimeoutSecs: useEfmiDirectWait ? 120 : 30,
                });
            }

            programs.push(...postLaunchPrograms);

            debugLog(
                "GameLauncher",
                "Launch program queue:\n" + this.formatProgramsForLog(programs),
            );

            // Execute programs sequentially via Rust tool method
            await invoke("launch_programs", { programs });
            launchSubmitted = true;
            ElMessage.success(t("launchGame.messages.launchFlowExecuted"));
            return useRunInjector ? "native" : "direct";
        } catch (e: unknown) {
            const errorText = String(e);
            console.error("Start Game Error:", e);
            ElMessageBox.alert(
                t("launchGame.messages.launchFailedDetailed", {
                    error: this.formatLaunchError(errorText),
                }),
                t("launchGame.common.errorTitle"),
                {
                    type: "error",
                    confirmButtonText: t("launchGame.common.confirm"),
                },
            );
            throw e;
        } finally {
            if (managedStagedTarget && !launchSubmitted) {
                try {
                    await invoke("restore_managed_graphics_stack", {
                        gameExecutable: managedStagedTarget,
                    });
                } catch (error) {
                    ElMessage.error(t("launchGame.messages.graphicsRestoreFailed", { error: String(error) }));
                }
            }
        }
    }

    private static async ensureSSMTRuntimeFiles(
        migotoDir: string,
        gameName: string,
        gamePreset?: string,
    ): Promise<void> {
        const dllTargetPath = await join(migotoDir, "SSMT-Player-Tweaks.dll");
        const configTargetPath = await join(migotoDir, "SSMT-Player-Tweaks.ini");
        const removeStaleDll = async () => {
            if (await exists(dllTargetPath)) await remove(dllTargetPath);
        };
        let assets: { dll: string } | null = null;
        try {
            assets = await invoke<{ dll: string } | null>("prepare_player_tweaks_launch", {
                gameName, gamePreset: gamePreset || "",
            });
        } catch (error) {
            ElMessage.warning({ message: `Player Tweaks 准备失败，本次继续普通启动：${String(error)}`, showClose: true, duration: 12000 });
        }
        if (!assets) {
            await removeStaleDll();
            return;
        }
        const dllSourcePath = assets.dll;
        try {
            const gameConfig = await ResourceManager.loadGameConfig(gameName);
            await writeTextFile(configTargetPath,
                serializePlayerTweaksCamera(gameConfig?.playerTweaksCamera) +
                serializePlayerTweaksGameplay(gameConfig?.playerTweaksGameplay));
        } catch (error) {
            await removeStaleDll();
            console.warn('Failed to prepare Player Tweaks config:', error);
            ElMessage.warning(`Player Tweaks 配置写入失败，已跳过本次注入：${String(error)}`);
            return;
        }
        try {
            await copyFile(dllSourcePath, dllTargetPath);
        } catch (error) {
            await removeStaleDll();
            console.warn('Failed to deploy Player Tweaks:', error);
            ElMessage.warning(`Player Tweaks 部署失败，已跳过本次注入：${String(error)}`);
        }
    }

    private static async ensureSSMTPluginHostFile(migotoDir: string): Promise<void> {
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();
        const source = await join(resourcesDir, "SSMT-PluginHost.dll");
        if (!(await exists(source))) {
            throw new Error(`Missing optional runtime plugin host: ${source}`);
        }
        await copyFile(source, await join(migotoDir, "SSMT-PluginHost.dll"));
    }

    private static runtimeSyncReady(result: RuntimeSyncResult, file: string): boolean {
        if (result.status === "updated" || result.status === "unchanged") {
            if (result.warning) {
                console.warn(`SSMT Runtime 在线更新失败 (${file})：`, result.warning);
                ElMessage.warning({
                    message: t("launchGame.messages.ssmtRuntimeRemoteFallback", { reason: result.warning }),
                    showClose: true,
                    duration: 10000,
                });
            }
            return true;
        }
        console.warn(`SSMT Runtime 同步失败 (${file})：`, result.error);
        const reason = result.error instanceof Error ? result.error.message : String(result.error);
        ElMessage.warning({
            message: t(result.status === "kept_existing"
                ? "launchGame.messages.ssmtRuntimeUpdateFallback"
                : "launchGame.messages.ssmtRuntimeUnavailable", { file, reason }),
            showClose: true,
            duration: 10000,
        });
        return result.status === "kept_existing";
    }

    private static async packStagedSSMTRuntime(resourceDir: string, stagingPath: string): Promise<void> {
        const candidates = [
            await join(resourceDir, "upx.exe"),
            await join(resourceDir, "resources", "upx.exe"),
            await join("resources", "upx.exe"),
        ];
        let upxPath = "";
        for (const candidate of candidates) {
            if (await exists(candidate)) { upxPath = candidate; break; }
        }
        if (!upxPath) throw new Error("SSMT Runtime 所需的 upx.exe 不存在");

        const execute = (args: string[]) => invoke<{ code: number; stdout: string; stderr: string }>(
            "execute_external_program", { programPath: upxPath, args },
        );
        // 发行资源可能已经打包；先验证，避免把 AlreadyPacked 当成更新失败。
        if ((await execute(["-t", stagingPath])).code === 0) return;
        const output = await execute([stagingPath]);
        if (output.code !== 0) {
            throw new Error(`UPX (${output.code}): ${output.stderr || output.stdout}`);
        }
    }

    private static async prepareGameEnvironment(
        gameName: string,
        migotoPath: string,
        config: GameConfig,
        useUpx: boolean,
    ): Promise<boolean> {
        try {
            if (!(await exists(migotoPath))) {
                await mkdir(migotoPath, { recursive: true });
            }

            const resourceDir = await GlobalConfig.SSMTResourcesFolder();
            const filesToCopy = ["d3dcompiler_47.dll"];

            for (const file of filesToCopy) {
                const destPath = await join(migotoPath, file);
                const sourcePath = await join(resourceDir, file);
                if (!(await exists(destPath))) {
                    const result = await MigotoManager.syncRuntimeFile(sourcePath, destPath);
                    if (!this.runtimeSyncReady(result, file)) return false;
                }
            }

            const runSourcePath = await join(resourceDir, "Run.exe");
            const runDestPath = await join(migotoPath, "Run.exe");
            const runResult = await MigotoManager.syncRuntimeFile(runSourcePath, runDestPath);
            if (!this.runtimeSyncReady(runResult, "Run.exe")) return false;

            const dllResult = await MigotoManager.applySSMTRuntimeToMigotoDir(
                gameName,
                migotoPath,
                config,
                useUpx ? path => this.packStagedSSMTRuntime(resourceDir, path) : undefined,
            );
            const dllSource = await ResourceManager.resolveMigotoDllSource(config);
            return this.runtimeSyncReady(dllResult, dllSource.targetFileName);
        } catch (error) {
            console.error("Failed to prepare game environment: ", error);
            throw error;
        }
    }
}
