import { invoke } from "@tauri-apps/api/core";
import { exists, mkdir, copyFile } from "@tauri-apps/plugin-fs";
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
import { isExistingLaunchFile } from './LaunchPath';
import { withTimeout } from './UpdateCheckUtils';

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

interface PluginInjectedDll {
    pluginId: string;
    pluginName: string;
    path: string;
}

interface PluginInjectedDllsResult {
    dlls: PluginInjectedDll[];
    warnings: string[];
}

interface BackendRuntimePlan {
    dlls: Array<{ path: string; sha256: string }>;
    environment: Record<string, string | null>;
    requiredLaunchArgs?: string[];
    excludedLaunchArgs?: string[];
    logSources?: Array<{ name: string; path: string }>;
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

    private static dedupeDllPaths(paths: string[]): string[] {
        const seen = new Set<string>();
        return paths
            .map(path => path.trim())
            .filter(path => {
                if (!path) return false;
                const key = path.replace(/\//g, "\\").toLocaleLowerCase();
                if (seen.has(key)) return false;
                seen.add(key);
                return true;
            });
    }

    private static applyRequiredLaunchArgs(raw: string, plan: BackendRuntimePlan): string {
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
            await isExistingLaunchFile(targetExe);
        const useShell = launchTargetProgram && !!migotoCfg.useShell;
        const configuredLauncher = (migotoCfg.launcherExePath || "").trim();
        const isConfiguredLauncherValid = !useShell || (
            await isExistingLaunchFile(configuredLauncher)
        );
        const gamePreset = (migotoCfg.gamePreset || "").trim().toUpperCase();
        if (["GIMI", "SRMI", "ZZMI", "ZZMIDX12"].includes(gamePreset)) {
            await invoke("hoyoplay_check_launch", { gameName });
        }
        const supportsGameDiscovery = [
            "GIMI",
            "SRMI",
            "ZZMI",
            "ZZMIDX12",
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
            const quickOnly = ["GIMI", "SRMI", "ZZMI", "ZZMIDX12"].includes(gamePreset);
            const discovery = invoke<DiscoveredGamePaths | null>(
                "find_game_executable",
                { gamePreset, quickOnly },
            );
            const discovered = quickOnly ? await withTimeout(discovery, 2500) : await discovery;
            if (discovered && await isExistingLaunchFile(discovered.targetExePath)
                && (!useShell || await isExistingLaunchFile(discovered.launcherExePath))) {
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

        if (requiresTargetPath && !(await isExistingLaunchFile(targetExe))) {
            ElMessage.error(t("launchGame.messages.targetProcessFileNotFound"));
            return null;
        }

        const pureDirectLaunch = pureMode && launchTargetProgram;
        if (configChanged) {
            await ResourceManager.saveGameConfig(gameName, migotoCfg);
        }

        // A pure launch must not depend on, prepare, or mutate the 3DMigoto
        // runtime. It is intentionally just the configured game executable.
        if (pureDirectLaunch) {
            return { migotoDir: "", config: migotoCfg, targetExe };
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

        // 静默发现 Runtime 发布并临时部署；锁定时保留用户 DLL，未锁定时不回退旧目标。
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

        return { migotoDir, config: migotoCfg, targetExe };
    }

    static async launch(
        gameName: string,
        appSettings: AppSettings,
        onNeedsUpdate: () => Promise<boolean | void> | boolean | void,
        onNeedsConfigureProcessPath?: () => void,
        ctrlPressed = false,
    ): Promise<'native' | 'direct' | false> {
        let launchSubmitted = false;
        let pluginBackendsStarted = false;
        let pluginBackendBridgeAvailable = true;
        let runtimeDirectory: string | undefined;
        let launchExecutable: string | undefined;
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
            );
            if (!preflight) return false;

            const { migotoDir, config, targetExe } = preflight;
            runtimeDirectory = migotoDir;
            launchExecutable = targetExe;
            const launchTargetProgram = config.launchTargetProgram !== false;
            const pureDirectTarget = pureMode && launchTargetProgram;
            if (!pureDirectTarget) {
                pluginBackendBridgeAvailable = await this.preparePluginBackends(gameName, migotoDir);
                pluginBackendsStarted = pluginBackendBridgeAvailable;
            }
            const launcherExePath = (config.launcherExePath || "").trim();
            const targetProcessName = this.getProcessNameFromPath(targetExe);
            let useShell = launchTargetProgram && (config.useShell || false);
            const useRunInjector = !pureMode || !launchTargetProgram;
            let pluginInjectedDlls: string[] = [];
            if (useRunInjector && !pureDirectTarget) {
                try {
                    const injected = await invoke<PluginInjectedDllsResult | null>(
                        "plugin_launch_injected_dlls",
                        { gameName },
                    );
                    pluginInjectedDlls = Array.isArray(injected?.dlls)
                        ? injected.dlls.map(item => item.path)
                        : [];
                    if (Array.isArray(injected?.warnings) && injected.warnings.length) {
                        ElMessage.warning({
                            message: t("launchGame.messages.pluginInjectedDllFailed", {
                                error: injected.warnings.join(" · "),
                            }),
                            grouping: true,
                            showClose: true,
                            duration: 12000,
                        });
                    }
                } catch (error) {
                    ElMessage.warning({
                        message: t("launchGame.messages.pluginInjectedDllFailed", {
                            error: String(error),
                        }),
                        grouping: true,
                        showClose: true,
                        duration: 12000,
                    });
                }
            }
            if (pluginInjectedDlls.length > 0) {
                // Run.exe only consumes extra DLLs on its native launch path.
                useShell = false;
            }
            const configuredExtraDlls = Array.isArray(config.extraDlls)
                ? config.extraDlls.filter((dll): dll is string => typeof dll === "string").map(dll => dll.trim()).filter(Boolean)
                : [];
            const extraDllsEnabled = Array.isArray(config.extraDllsEnabled) ? config.extraDllsEnabled : [];
            const enabledConfiguredExtraDlls = configuredExtraDlls.filter(
                (_, index) => extraDllsEnabled[index] !== false,
            );
            const legacyExtraDll = typeof config.extraDll === "string" ? config.extraDll.trim() : "";
            const launchExtraDlls = this.dedupeDllPaths([
                ...(configuredExtraDlls.length
                    ? enabledConfiguredExtraDlls
                    : legacyExtraDll ? [legacyExtraDll] : []),
                ...pluginInjectedDlls,
            ]);
            const useEfmiDirectWait = useRunInjector &&
                (config.gamePreset || "").trim().toUpperCase() === "EFMI";
            // 与 XXMI 的 EFMI DIRECT 链路一致：默认由 SSMT 直接启动 Endfield.exe，
            // Run.exe 只等待进程并注入；Shell/手动启动保留为回退路径。
            const launchEfmiTargetDirectly =
                useEfmiDirectWait && launchTargetProgram && !useShell;
            const efmiDirectLaunchArgs =
                (config.launchArgs || "").trim() || "-force-d3d11";

            if (launchTargetProgram && useShell && !launcherExePath) {
                ElMessage.warning(
                    t("launchGame.messages.shellLauncherPathRequired"),
                );
                return false;
            }

            if (launchEfmiTargetDirectly && !launcherExePath) {
                ElMessage.error(t("launchGame.messages.efmiDirectLauncherRequired"));
                return false;
            }

            if (
                launchTargetProgram &&
                (useShell || launchEfmiTargetDirectly) &&
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
                await MigotoManager.patchD3dxForLaunch(gameName, {
                    extraDlls: launchExtraDlls,
                    overlayLanguage: appSettings.runtimeOverlayLanguage ?? 'auto',
                });
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

            let backendPlan: BackendRuntimePlan | null = null;
            let backendDlls: string[] = [];
            let backendLaunchArgs = config.launchArgs || "";
            if (!pureDirectTarget && pluginBackendBridgeAvailable && useRunInjector && targetExe) {
                const comparablePath = (path: string) => path.replace(/\\/g, "/").replace(/^\/\/\?\//, "").toLowerCase();
                const allowPreload = launchTargetProgram && !useEfmiDirectWait &&
                    (!useShell || comparablePath(await join(launcherExePath)) === comparablePath(await join(targetExe)));
                try {
                    const prepared = await invoke<{
                        plans: Array<{ pluginId: string; pluginName: string; plan: BackendRuntimePlan }>;
                        diagnostics: Array<{ pluginId: string; pluginName: string; error: string }>;
                    }>("prepare_plugin_backend_launch", { gameName, gameExecutable: targetExe, allowPreload, existingDlls: launchExtraDlls });
                    for (const diagnostic of prepared.diagnostics) {
                        ElMessage.warning({ message: t("launchGame.messages.pluginBackendPrepareFailed", {
                            error: `${diagnostic.pluginName}: ${diagnostic.error}`,
                        }), grouping: true, showClose: true, duration: 12000 });
                    }
                    if (prepared.plans.length) {
                        const plan: BackendRuntimePlan = { dlls: [], environment: {}, requiredLaunchArgs: [], excludedLaunchArgs: [], logSources: [] };
                        for (const entry of prepared.plans) {
                            plan.dlls.push(...entry.plan.dlls);
                            Object.assign(plan.environment, entry.plan.environment);
                            plan.requiredLaunchArgs!.push(...entry.plan.requiredLaunchArgs || []);
                            plan.excludedLaunchArgs!.push(...entry.plan.excludedLaunchArgs || []);
                            plan.logSources!.push(...entry.plan.logSources || []);
                        }
                        const owned = new Set(plan.dlls.map(dll => this.getProcessNameFromPath(dll.path).toLowerCase()));
                        for (const dll of launchExtraDlls) {
                            if (owned.has(this.getProcessNameFromPath(dll).toLowerCase())) throw new Error(`插件启动计划与额外 DLL 冲突：${dll}`);
                        }
                        backendDlls = this.dedupeDllPaths([...plan.dlls.map(dll => dll.path), ...launchExtraDlls]);
                        backendLaunchArgs = this.applyRequiredLaunchArgs(config.launchArgs || "", plan);
                        backendPlan = plan;
                        useShell = false;
                    }
                } catch (error) {
                    // 已提交的后台可能修改了自己的状态，撤回后继续普通启动。
                    await invoke("plugin_backend_lifecycle", { gameName, event: "launch.cancel", launchContext: { gameExecutable: targetExe } }).catch(cancelError =>
                        console.warn("插件后台启动取消失败，继续普通启动：", cancelError));
                    ElMessage.warning({ message: t("launchGame.messages.pluginBackendPrepareFailed", { error: String(error) }), showClose: true, duration: 12000 });
                }
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
            if (backendPlan) {
                await MigotoManager.patchD3dxForLaunch(gameName, {
                    extraDlls: backendDlls,
                    launchArgs: backendLaunchArgs,
                    useShell: false,
                    overlayLanguage: appSettings.runtimeOverlayLanguage ?? 'auto',
                });
            }

            if (useRunInjector) {
                const injectorArgs: string[] = ['--machine-readable'];
                if (backendPlan) {
                    injectorArgs.push("--preload-runtime");
                    for (const [name, value] of Object.entries(backendPlan.environment)) {
                        injectorArgs.push(value === null ? `--unset-env ${name}` : `--set-env "${name}=${value}"`);
                    }
                }
                if (effectivePluginHostConfig) {
                    injectorArgs.push(`--plugin-host-config "${effectivePluginHostConfig.replace(/"/g, '\\"')}"`);
                }
                if (useEfmiDirectWait) {
                    injectorArgs.push("--inject-mode direct --wait-for-target --process-timeout 120");
                    if (launchEfmiTargetDirectly) {
                        injectorArgs.push(`--launch-parent "${launcherExePath.replace(/"/g, '\\"')}"`);
                        injectorArgs.push(`--target-args "${efmiDirectLaunchArgs.replace(/"/g, '\\"')}"`);
                    }
                }
                // 1. Default flow launches Run.exe first.
                programs.push({
                    path: await join(migotoDir, "Run.exe"),
                    logSources: backendPlan?.logSources,
                    workDir: migotoDir,
                    args: injectorArgs.join(" ") || undefined,
                    // Run.exe 带有 requireAdministrator manifest。SSMT 未提权时，
                    // Start-Process 必须显式使用 RunAs，否则 Windows 会返回
                    // ERROR_ELEVATION_REQUIRED。
                    runAsAdministrator: true,
                    waitForProcessName: useShell ? "Run.exe" : undefined,
                    waitTimeoutSecs: useShell ? 30 : undefined,
                });
                if (useEfmiDirectWait) {
                    ElMessage.info({ message: t("launchGame.messages.efmiOfficialLauncherWait"), duration: 15000 });
                }
            }

            // 2. Pure mode, shell mode, and the EFMI direct launch start the target/launcher.
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
            if (!launchSubmitted && pluginBackendsStarted) {
                await invoke<string[]>("plugin_backend_lifecycle", { gameName, event: "cancel", launchContext: { gameExecutable: launchExecutable } }).catch(error =>
                    console.warn("插件后台取消失败：", error));
            }
            if (!launchSubmitted && runtimeDirectory) {
                await invoke('cancel_runtime_deployment', { directory: runtimeDirectory }).catch(error =>
                    console.warn('撤回未提交的 Runtime 部署失败：', error));
            }

        }
    }

    private static async preparePluginBackends(
        gameName: string,
        runtimeDirectory: string,
    ): Promise<boolean> {
        try {
            const warnings = await invoke<string[]>("plugin_backend_lifecycle", { gameName, event: "prepare", launchContext: { runtimeDirectory } });
            for (const warning of warnings) {
                ElMessage.warning({ message: warning, showClose: true, duration: 12000 });
            }
            return true;
        } catch (error) {
            ElMessage.warning({ message: t("launchGame.messages.pluginBackendPrepareFailed", { error: String(error) }), showClose: true, duration: 12000 });
            return !this.isPluginBackendCommandUnavailable(error);
        }
    }

    private static isPluginBackendCommandUnavailable(error: unknown): boolean {
        const message = String(error);
        return message.includes("Command plugin_backend_lifecycle not found")
            || message.includes("Command prepare_plugin_backend_launch not found");
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
