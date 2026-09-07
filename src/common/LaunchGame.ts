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
    waitForProcessName?: string;
    waitTimeoutSecs?: number;
    waitOnly?: boolean;
}

interface DiscoveredGamePaths {
    targetExePath: string;
    launcherExePath: string;
}

type LaunchProgramPhase = "preLaunchPrograms" | "postLaunchPrograms";

export class LaunchGame {
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

    private static notifyD3d11CopyFailure(error: unknown, label: string): void {
        const message = t("launchGame.messages.d3d11CopySkippedContinue", {
            mode: label,
            error: String(error),
        });

        ElMessage({
            message,
            type: "error",
            duration: 8000,
            showClose: true,
            offset: 48,
        });
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
    ): Promise<{
        migotoDir: string;
        config: GameConfig;
        targetExe: string;
    } | null> {
        const config = await ResourceManager.loadGameConfig(gameName);
        const migotoCfg = config ?? ({} as GameConfig);
        const launchTargetProgram = migotoCfg.launchTargetProgram !== false;
        let targetExe = (migotoCfg.targetExePath || "").trim();
        const isConfiguredTargetValid =
            targetExe.length > 0 && (await exists(targetExe));
        const configuredLauncher = (migotoCfg.launcherExePath || "").trim();
        const isConfiguredLauncherValid =
            configuredLauncher.length > 0 && (await exists(configuredLauncher));
        const gamePreset = (migotoCfg.gamePreset || "").trim().toUpperCase();
        const supportsGameDiscovery = [
            "GIMI",
            "SRMI",
            "ZZMI",
            "NTEMI",
            "WWMI",
        ].includes(gamePreset);

        let configChanged = false;
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
            configChanged = true;
            await ResourceManager.saveGameConfig(gameName, migotoCfg);
            configChanged = false;

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

        if (configChanged) {
            await ResourceManager.saveGameConfig(gameName, migotoCfg);
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

        return { migotoDir, config: migotoCfg, targetExe };
    }

    static async launch(
        gameName: string,
        appSettings: AppSettings,
        onNeedsUpdate: () => Promise<boolean | void> | boolean | void,
        onNeedsConfigureProcessPath?: () => void,
        onNeedsDllUpdate?: () => Promise<boolean | void>,
        ctrlPressed = false,
    ): Promise<void> {
        try {
            const pureMode =
                appSettings.gameLaunchMode === "always-pure" ||
                (appSettings.gameLaunchMode === "ctrl-pure" && ctrlPressed);
            const libsReady = await this.ensureXXMILibsReady(
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
            );
            if (!preflight) return;

            const { migotoDir, config, targetExe } = preflight;
            await this.ensureSSMTRuntimeFiles(migotoDir, config.gamePreset);
            const launchTargetProgram = config.launchTargetProgram !== false;
            const launcherExePath = (config.launcherExePath || "").trim();
            const targetProcessName = this.getProcessNameFromPath(targetExe);
            const useShell = launchTargetProgram && (config.useShell || false);

            if (launchTargetProgram && useShell && !launcherExePath) {
                ElMessage.warning(
                    t("launchGame.messages.shellLauncherPathRequired"),
                );
                return;
            }

            if (
                launchTargetProgram &&
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
            const safe = await MigotoManager.check3DmigotoIntegrity(gameName);
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

            await MigotoManager.patchD3dxForLaunch(gameName);

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

            if (!pureMode || !launchTargetProgram) {
                // 1. Default flow launches Run.exe first.
                programs.push({
                    path: await join(migotoDir, "Run.exe"),
                    workDir: migotoDir,
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
                ((!pureMode && !useShell && !launcherExePath) ||
                    postLaunchPrograms.length > 0);

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
        }
    }

    private static async ensureSSMTRuntimeFiles(
        migotoDir: string,
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

        // SSMT-Player-Tweaks.dll is only meaningful for GIMI (Genshin).
        // For other presets do not require/copy it; if a stale copy exists in
        // this game directory, remove it so Run.exe does not inject it.
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

        if (!isGimi) {
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
                        ElMessageBox.alert(
                            msg,
                            t("launchGame.messages.copyFailedTitle"),
                            {
                                type: "error",
                                confirmButtonText: t(
                                    "launchGame.common.confirm",
                                ),
                            },
                        );
                        return;
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
                        ElMessageBox.alert(
                            msg,
                            t("launchGame.messages.copyFailedTitle"),
                            {
                                type: "error",
                                confirmButtonText: t(
                                    "launchGame.common.confirm",
                                ),
                            },
                        );
                        return;
                    }
                }
            }

            await MigotoManager.applyD3d11ModeToMigotoDir(
                gameName,
                migotoPath,
                config,
                {
                    continueOnCopyFailure: true,
                    onCopyFailure: (error, context) => {
                        debugWarn(
                            "GameLauncher",
                            "Failed to refresh migoto DLL before launch. Continuing launch flow.",
                            {
                                mode: context.mode,
                                sourcePath: context.sourcePath,
                                destPath: context.destPath,
                                dllLabel: context.dllLabel,
                                error,
                            },
                        );
                        this.notifyD3d11CopyFailure(error, context.dllLabel);
                    },
                },
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
