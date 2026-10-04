import { invoke } from "@tauri-apps/api/core";
import { exists, mkdir, copyFile, remove } from "@tauri-apps/plugin-fs";
import { join } from "@tauri-apps/api/path";
import { ElMessage, ElMessageBox } from "element-plus";
import { ResourceManager } from "../store/ResourceManager";
import { MigotoManager } from "../store/MigotoManager";
import { GlobalConfig } from "../store/GlobalConfig";
import { PathHelper } from "../helper/PathHelper";
import { i18n } from "../i18n";
import { getEffectiveUseUpx, type GameConfig, type LaunchProgramConfig } from "../store/GameConfig";
import type { AppSettings } from "../store/AppSettings";
import { debugLog, debugWarn } from "../utils/debugLog";

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

    private static async ensureXXMILibsReady(
        gameName: string,
        onNeedsDllUpdate?: () => Promise<boolean | void>,
    ): Promise<boolean> {
        const missingFiles =
            await ResourceManager.getMissingXXMILibsFiles(gameName);
        if (missingFiles.length === 0) {
            return true;
        }

        try {
            await ElMessageBox.confirm(
                t("launchGame.messages.missingDllPackageConfirmContent", {
                    files: missingFiles.join(", "),
                }),
                t("launchGame.messages.missingDllPackageTitle"),
                {
                    confirmButtonText: t("launchGame.common.checkUpdate"),
                    cancelButtonText: t("launchGame.common.cancel"),
                    type: "warning",
                },
            );
        } catch {
            return false;
        }

        const updateHandled = await onNeedsDllUpdate?.();
        if (updateHandled === false) {
            return false;
        }

        const missingAfterUpdate =
            await ResourceManager.getMissingXXMILibsFiles(gameName);
        if (missingAfterUpdate.length > 0) {
            ElMessage.error(
                t("launchGame.messages.missingDllPackageStillMissing", {
                    files: missingAfterUpdate.join(", "),
                }),
            );
            return false;
        }

        return true;
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

        if (launchTargetProgram && !targetExe) {
            ElMessage.warning(
                t("launchGame.messages.targetProcessPathNotConfigured"),
            );
            onNeedsConfigureProcessPath?.();
            return null;
        }

        if (launchTargetProgram && !(await exists(targetExe))) {
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

        // Initialize Migoto environment and apply the selected d3d11 mode before launch.
        // UPX: respect per-game config, but default GIMI to UPX until the user chooses otherwise.
        const useUpx = getEffectiveUseUpx(migotoCfg);
        await this.prepareGameEnvironment(
            gameName,
            migotoDir,
            migotoCfg,
            useUpx,
        );

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
        onNeedsDllUpdate?: () => Promise<boolean | void>,
        ctrlPressed = false,
    ): Promise<void> {
        let managedStagedTarget: string | null = null;
        let launchSubmitted = false;
        try {
            const pureMode =
                appSettings.gameLaunchMode === "always-pure" ||
                (appSettings.gameLaunchMode === "ctrl-pure" && ctrlPressed);
            const pureDirectLaunch =
                pureMode &&
                (await ResourceManager.loadGameConfig(gameName))?.launchTargetProgram !== false;
            const libsReady = pureDirectLaunch
                ? true
                : await this.ensureXXMILibsReady(
                      gameName,
                      onNeedsDllUpdate,
                  );
            if (!libsReady) return;

            const preflight = await this.prepareLaunch(
                gameName,
                appSettings,
                pureMode,
                onNeedsConfigureProcessPath,
                onNeedsUpdate,
                (targetExe) => this.preflightGraphicsLaunch(gameName, targetExe),
            );
            if (!preflight) return;

            const { migotoDir, config, targetExe, graphicsDecision } = preflight;
            const launchTargetProgram = config.launchTargetProgram !== false;
            const pureDirectTarget = pureMode && launchTargetProgram;
            if (!pureDirectTarget) {
                await this.ensureSSMTRuntimeFiles(migotoDir, gameName, config.gamePreset);
            }
            const launcherExePath = (config.launcherExePath || "").trim();
            const targetProcessName = this.getProcessNameFromPath(targetExe);
            const useShell = launchTargetProgram && (config.useShell || false);
            const useRunInjector = !pureMode || !launchTargetProgram;

            if (launchTargetProgram && useShell && !launcherExePath) {
                ElMessage.warning(
                    t("launchGame.messages.shellLauncherPathRequired"),
                );
                return;
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
                return;
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
                return;
            }

            if (!pureDirectTarget) {
                await MigotoManager.patchD3dxForLaunch(gameName);
            }

            const preLaunchPrograms = launchTargetProgram
                ? await this.buildConfiguredPrograms(
                      config.preLaunchPrograms,
                      "preLaunchPrograms",
                  )
                : [];
            if (!preLaunchPrograms) return;

            const postLaunchPrograms = launchTargetProgram
                ? await this.buildConfiguredPrograms(
                      config.postLaunchPrograms,
                      "postLaunchPrograms",
                  )
                : [];
            if (!postLaunchPrograms) return;

            // Construct programs list
            const programs: ProgramToLaunch[] = [...preLaunchPrograms];

            const hoyoshade = !pureDirectTarget && launchTargetProgram && targetExe
                ? await invoke<PluginLaunchProgram | null>("prepare_hoyoshade_launch", {
                      gameName,
                      gameExecutable: targetExe,
                      graphicsDecision,
                  })
                : null;
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

            // 仅在本次启动确实包含 HoYoShade 时覆盖用户的原始延迟配置。
            if (hoyoshade) {
                await MigotoManager.patchD3dxForLaunch(gameName, {
                    dllInitializationDelay: 150,
                });
            }

            if (useRunInjector) {
                // 1. Default flow launches Run.exe first.
                programs.push({
                    path: await join(migotoDir, "Run.exe"),
                    workDir: migotoDir,
                    args: effectivePluginHostConfig
                        ? `--plugin-host-config "${effectivePluginHostConfig.replace(/"/g, '\\"')}"`
                        : undefined,
                    // Run.exe 带有 requireAdministrator manifest。SSMT 未提权时，
                    // Start-Process 必须显式使用 RunAs，否则 Windows 会返回
                    // ERROR_ELEVATION_REQUIRED，HoYoShade 也会一直等待不存在的进程。
                    runAsAdministrator: true,
                    waitForProcessName: useShell ? "Run.exe" : undefined,
                    waitTimeoutSecs: useShell ? 30 : undefined,
                });
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
                    return;
                }
                programs.push({
                    path: exePath,
                    args: config.launchArgs || "",
                });
            }

            const shouldWaitForTargetProcess =
                launchTargetProgram &&
                !!targetProcessName &&
                ((!pureMode && !useShell) || postLaunchPrograms.length > 0);

            if (shouldWaitForTargetProcess) {
                programs.push({
                    path: "",
                    waitOnly: true,
                    waitForProcessName: targetProcessName,
                    waitTimeoutSecs: 30,
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
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();

        // Run.exe is the base injector/launcher used by the default flow.
        const runSourcePath = await join(resourcesDir, "Run.exe");
        const runTargetPath = await join(migotoDir, "Run.exe");

        if (!(await exists(runSourcePath))) {
            throw new Error(`Missing SSMT runtime file: ${runSourcePath}`);
        }

        await copyFile(runSourcePath, runTargetPath);

        // Run.exe 会注入同目录中的 Player Tweaks。只在本游戏明确启用时部署，
        // 否则移除旧副本，避免插件市场的禁用状态与实际注入不一致。
        const isGimi = (gamePreset || "").trim().toUpperCase() === "GIMI";
        const dllSourcePath = await join(
            resourcesDir,
            "SSMT-Player-Tweaks.dll",
        );
        const dllTargetPath = await join(migotoDir, "SSMT-Player-Tweaks.dll");

        const removeStaleDll = async () => {
            try {
                if (await exists(dllTargetPath)) {
                    await remove(dllTargetPath);
                }
            } catch (e) {
                console.warn(
                    "Failed to remove stale SSMT-Player-Tweaks.dll:",
                    e,
                );
            }
        };

        const playerTweaksEnabled = isGimi && await invoke<boolean>(
            "bundled_player_tweaks_enabled_for_game",
            { gameName, gamePreset: gamePreset || "" },
        ).catch((error) => {
            console.warn("Failed to read bundled plugin selection:", error);
            return false;
        });
        if (!playerTweaksEnabled) {
            await removeStaleDll();
            return;
        }

        if (!(await exists(dllSourcePath))) {
            await removeStaleDll();
            ElMessage.warning(
                t("launchGame.messages.missingPlayerTweaksSkipped"),
            );
            return;
        }

        await copyFile(dllSourcePath, dllTargetPath);
    }

    private static async ensureSSMTPluginHostFile(migotoDir: string): Promise<void> {
        const resourcesDir = await GlobalConfig.SSMTResourcesFolder();
        const source = await join(resourcesDir, "SSMT-PluginHost.dll");
        if (!(await exists(source))) {
            throw new Error(`Missing optional runtime plugin host: ${source}`);
        }
        await copyFile(source, await join(migotoDir, "SSMT-PluginHost.dll"));
    }

    private static async prepareGameEnvironment(
        gameName: string,
        migotoPath: string,
        config: GameConfig,
        useUpx: boolean,
    ): Promise<void> {
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
                    try {
                        console.log(`Copying ${file} to ${destPath}`);
                        await copyFile(sourcePath, destPath);
                    } catch (e) {
                        const msg = t(
                            "launchGame.messages.copyFailedEnsureClosed",
                            { file, error: String(e) },
                        );
                        console.error(msg);
                        throw new Error(msg);
                    }
                }
            }

            const runSourcePath = await join(resourceDir, "Run.exe");
            const runDestPath = await join(migotoPath, "Run.exe");
            if (await exists(runSourcePath)) {
                const sourceMd5 = await invoke<string>("file_md5", {
                    path: runSourcePath,
                });
                let shouldCopyRun = !(await exists(runDestPath));

                if (!shouldCopyRun) {
                    const destMd5 = await invoke<string>("file_md5", {
                        path: runDestPath,
                    }).catch(() => "");
                    shouldCopyRun = sourceMd5 !== destMd5;
                }

                if (shouldCopyRun) {
                    try {
                        if (await exists(runDestPath)) {
                            await remove(runDestPath);
                        }
                        console.log(
                            `Copying Run.exe to ${runDestPath} because md5 differs`,
                        );
                        await copyFile(runSourcePath, runDestPath);
                    } catch (e) {
                        const msg = t(
                            "launchGame.messages.copyFailedEnsureClosed",
                            { file: "Run.exe", error: String(e) },
                        );
                        console.error(msg);
                        throw new Error(msg);
                    }
                }
            }

            await MigotoManager.applyD3d11ModeToMigotoDir(
                gameName,
                migotoPath,
                config,
            );

            if (useUpx) {
                debugLog(
                    "GameLauncher",
                    "UPX packing enabled. Packing d3d11.dll...",
                );
                const d3d11Path = await join(migotoPath, "d3d11.dll");

                if (await exists(d3d11Path)) {
                    let upxToUse = await join(resourceDir, "upx.exe");

                    if (!(await exists(upxToUse))) {
                        const nestedUpx = await join(
                            resourceDir,
                            "resources",
                            "upx.exe",
                        );
                        if (await exists(nestedUpx)) {
                            upxToUse = nestedUpx;
                        } else {
                            const devUpx = await join("resources", "upx.exe");
                            if (await exists(devUpx)) {
                                upxToUse = devUpx;
                            }
                        }
                    }

                    if (await exists(upxToUse)) {
                        debugLog(
                            "GameLauncher",
                            `Running UPX: ${upxToUse} ${d3d11Path}`,
                        );
                        try {
                            const output = await invoke<{
                                code: number;
                                stdout: string;
                                stderr: string;
                            }>("execute_external_program", {
                                programPath: upxToUse,
                                args: [d3d11Path],
                            });

                            if (output.code !== 0) {
                                debugLog(
                                    "GameLauncher",
                                    `UPX failed with exit code: ${output.code}`,
                                );
                                debugLog(
                                    "GameLauncher",
                                    `UPX stdout: ${output.stdout}`,
                                );
                                debugLog(
                                    "GameLauncher",
                                    `UPX stderr: ${output.stderr}`,
                                );
                            } else {
                                debugLog(
                                    "GameLauncher",
                                    "UPX packing successful.",
                                );
                            }
                        } catch (e) {
                            console.error(
                                `Failed to execute UPX process: ${e}`,
                            );
                        }
                    } else {
                        debugLog(
                            "GameLauncher",
                            "Warning: upx.exe not found. Skipping packing.",
                        );
                    }
                }
            }
        } catch (error) {
            console.error("Failed to prepare game environment: ", error);
            throw error;
        }
    }
}
