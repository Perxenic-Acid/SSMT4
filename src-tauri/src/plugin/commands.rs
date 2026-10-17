use super::dlss5::{inspect_game_executable, Dlss5Route, Dlss5State, DLSS5_PLUGIN_ID};
use super::graphics_stack::{
    CompatibilityLevel, GameGraphicsState, GraphicsLaunchContributions, GraphicsStackResolution,
    GraphicsStackResolver,
};
use super::hoyoshade::{HoYoShadeBridge, HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};
use super::logging::PluginLogWriter;
use super::managed_stack::{self, Dlss5RouteScan, ManagedStackStatus};
use super::package_installer::{inspect_ssmtpkg, install_verified_ssmtpkg, PackageInspection};
use super::official_release;
use super::marketplace::MarketplaceEntry;
use super::registry::{game_scope_preset, ExternalDependencyState, PluginRegistry, MANIFEST_FILE_NAME};
use super::settings::PluginSettingsStore;
use super::{PluginCompatibility, PluginContributions, PluginManifest, PluginPermission};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginLaunchProgram {
    pub path: String,
    pub args: String,
    pub work_dir: String,
    pub run_as_administrator: bool,
    pub post_launch_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dlss5GameStateSnapshot {
    pub state: &'static str,
    pub route: Option<String>,
    pub uses_reshade: Option<bool>,
    pub uses_proxy: Option<bool>,
    pub manifest_path: Option<String>,
    pub reason: Option<String>,
    pub managed_host: Option<bool>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dlss5LastRunStatus {
    pub verdict: &'static str,
    pub reason: &'static str,
    pub observed_at_ms: Option<u128>,
    pub warning: Option<&'static str>,
}

fn inspect_dlss5_last_run_log(path: &std::path::Path) -> Result<Dlss5LastRunStatus, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let observed_at_ms = metadata.modified().ok().and_then(|time| {
        time.duration_since(UNIX_EPOCH).ok().map(|duration| duration.as_millis())
    });
    // ReShade.log 可非常大；只读取最后 1 MiB，避免设置页扫描整份日志。
    const MAX_LOG_BYTES: u64 = 1024 * 1024;
    file.seek(SeekFrom::Start(metadata.len().saturating_sub(MAX_LOG_BYTES)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let log = String::from_utf8_lossy(&bytes);
    let (verdict, reason) = if log.contains("[DLSS 5 Feed] stopped: Direct3D 11 multithread protection is unavailable") {
        ("inactive", "d3d11_interposer_race")
    } else if log.contains("[DLSS 5 Feed] stopped:") {
        ("inactive", "feeder_stopped")
    } else if log.contains("NO DLSS CREATE SEEN") {
        ("inactive", "no_dlss_create")
    } else if log.contains("NR-VERDICT state=ENGAGED") {
        ("engaged", "nr_evaluated")
    } else if log.contains("inline feature 18 evaluation succeeded") {
        ("engaged", "feature18_evaluated")
    } else {
        ("unknown", "no_nr_evidence")
    };
    let warning = log
        .contains("most likely a shadow map or atlas")
        .then_some("depth_source_mismatch");
    Ok(Dlss5LastRunStatus {
        verdict,
        reason,
        observed_at_ms,
        warning,
    })
}

impl From<&Dlss5State> for Dlss5GameStateSnapshot {
    fn from(value: &Dlss5State) -> Self {
        match value {
            Dlss5State::NotManaged => Self {
                state: "not_managed",
                route: None,
                uses_reshade: None,
                uses_proxy: None,
                manifest_path: None,
                reason: None,
                managed_host: None,
                owner: None,
            },
            Dlss5State::Managed(managed) => Self {
                state: "managed",
                route: Some(match &managed.route {
                    Dlss5Route::Native => "native".to_string(),
                    Dlss5Route::Feeder => "feeder".to_string(),
                    Dlss5Route::RenoDx => "renodx".to_string(),
                    Dlss5Route::OptiScaler => "optiscaler".to_string(),
                    Dlss5Route::Unknown(route) => route.clone(),
                }),
                uses_reshade: Some(managed.uses_reshade),
                uses_proxy: Some(managed.uses_proxy),
                manifest_path: Some(managed.manifest_path.to_string_lossy().into_owned()),
                reason: None,
                managed_host: Some(managed.external_host.is_some()),
                owner: managed
                    .external_host
                    .as_ref()
                    .map(|host| host.owner.clone()),
            },
            Dlss5State::Broken {
                manifest_path,
                reason,
            } => Self {
                state: "broken",
                route: None,
                uses_reshade: None,
                uses_proxy: None,
                manifest_path: Some(manifest_path.to_string_lossy().into_owned()),
                reason: Some(reason.clone()),
                managed_host: None,
                owner: None,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsLaunchInspection {
    pub hoyoshade_requested: bool,
    pub dlss5_requested: bool,
    pub host_requested: bool,
    pub dlss5: Dlss5GameStateSnapshot,
    pub resolution: GraphicsStackResolution,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsLaunchDecision {
    ContinueThisLaunch,
    PrepareManagedStack,
    #[serde(rename = "suppress_hoyoshade_this_launch")]
    SuppressHoYoShadeThisLaunch,
}

fn should_launch_hoyoshade(
    inspection: &GraphicsLaunchInspection,
    decision: Option<GraphicsLaunchDecision>,
) -> Result<bool, String> {
    if !inspection.host_requested
        || decision == Some(GraphicsLaunchDecision::SuppressHoYoShadeThisLaunch)
    {
        return Ok(false);
    }
    match inspection.resolution.level {
        CompatibilityLevel::Compatible => Ok(true),
        CompatibilityLevel::Warning
            if decision == Some(GraphicsLaunchDecision::ContinueThisLaunch) =>
        {
            Ok(true)
        }
        CompatibilityLevel::RequiresManagedStack
            if decision == Some(GraphicsLaunchDecision::PrepareManagedStack) =>
        {
            Ok(true)
        }
        _ => Err(inspection
            .resolution
            .issues
            .first()
            .map(|issue| issue.message.clone())
            .unwrap_or_else(|| "graphics stack preflight rejected the launch".to_string())),
    }
}

fn inspect_graphics_for_executable(
    registry: &PluginRegistry,
    game_executable: &std::path::Path,
) -> Result<GraphicsLaunchInspection, String> {
    inspect_graphics_for_launch(registry, game_executable, false)
}

fn inspect_graphics_for_launch(
    registry: &PluginRegistry,
    game_executable: &std::path::Path,
    skip_dlss5: bool,
) -> Result<GraphicsLaunchInspection, String> {
    if !game_executable.is_file() {
        return Err(format!(
            "game executable does not exist: {}",
            game_executable.display()
        ));
    }
    let process_name = game_executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "game executable has no file name".to_string())?;
    let hoyoshade_requested = registry
        .find(HOYOSHADE_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
        .map(|plugin| {
            HoYoShadeBridge::from_installed(plugin)
                .map(|bridge| bridge.supports_process(process_name))
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .unwrap_or(false);
    // 只有启用了 DLSS5 才读取其游戏文件，普通启动不受外部安装记录影响。
    let dlss5_enabled = !skip_dlss5 && registry
        .find(DLSS5_PLUGIN_ID)
        .is_some_and(|plugin| plugin.enabled);
    let state = GameGraphicsState {
        dlss5: if dlss5_enabled {
            inspect_game_executable(game_executable)
        } else {
            Dlss5State::NotManaged
        },
    };
    if dlss5_enabled && (super::gimi_hot_runtime::applies(game_executable) || super::dlss5_hot_runtime::applies(game_executable)) {
        let clean = matches!(state.dlss5, Dlss5State::NotManaged);
        return Ok(GraphicsLaunchInspection {
            hoyoshade_requested,
            dlss5_requested: true,
            host_requested: true,
            dlss5: Dlss5GameStateSnapshot::from(&state.dlss5),
            resolution: GraphicsStackResolution {
                level: if clean {
                    CompatibilityLevel::Compatible
                } else {
                    CompatibilityLevel::Conflict
                },
                issues: if clean {
                    Vec::new()
                } else {
                    vec![super::graphics_stack::GraphicsCompatibilityIssue {
                        level: CompatibilityLevel::Conflict,
                        components: vec![DLSS5_PLUGIN_ID.into()],
                        code: if super::gimi_hot_runtime::applies(game_executable) { "gimi_legacy_cold_install" } else { "dlss5_legacy_cold_install" }.into(),
                        message: "当前 DLSS5 路线使用热注入。请先在 DLSS5 设置中恢复旧的 Swapper 安装，再配置热注入组件。".into(),
                        possible_actions: vec![super::graphics_stack::GraphicsConflictAction::CancelLaunch],
                    }]
                },
            },
        });
    }
    let host_requested = hoyoshade_requested
        || (dlss5_enabled && matches!(state.dlss5, Dlss5State::Managed(_) | Dlss5State::Broken { .. }));
    let resolution = GraphicsStackResolver::resolve(
        &state,
        GraphicsLaunchContributions {
            hoyoshade: hoyoshade_requested,
            dlss5: dlss5_enabled,
        },
    );
    Ok(GraphicsLaunchInspection {
        hoyoshade_requested,
        dlss5_requested: dlss5_enabled,
        host_requested,
        dlss5: Dlss5GameStateSnapshot::from(&state.dlss5),
        resolution,
    })
}

fn installed_plugin_registry_at(plugins_root: PathBuf) -> Result<Option<PluginRegistry>, String> {
    // The plugin page may have created a state file without installing a package.
    // In that case ordinary launch must not parse plugin state or create files.
    let Ok(ids) = fs::read_dir(&plugins_root) else {
        return Ok(None);
    };
    for id in ids.flatten() {
        let Ok(versions) = fs::read_dir(id.path()) else {
            continue;
        };
        if versions
            .flatten()
            .any(|version| version.path().join(MANIFEST_FILE_NAME).is_file())
        {
            return PluginRegistry::new(plugins_root)
                .map(Some)
                .map_err(|error| error.to_string());
        }
    }
    Ok(None)
}

pub(crate) fn installed_plugin_registry() -> Result<Option<PluginRegistry>, String> {
    installed_plugin_registry_at(crate::config::path_manager::PathManager::ssmt_plugins_folder())
}

#[tauri::command]
pub fn inspect_graphics_launch(
    game_name: String,
    game_executable: String,
) -> Result<GraphicsLaunchInspection, String> {
    let Some(registry) = installed_plugin_registry()? else {
        return Ok(GraphicsLaunchInspection {
            hoyoshade_requested: false,
            dlss5_requested: false,
            host_requested: false,
            dlss5: Dlss5GameStateSnapshot::from(&Dlss5State::NotManaged),
            resolution: GraphicsStackResolver::resolve(
                &GameGraphicsState {
                    dlss5: Dlss5State::NotManaged,
                },
                GraphicsLaunchContributions::default(),
            ),
        });
    };
    inspect_graphics_for_executable(&registry.for_game(&game_name), &PathBuf::from(game_executable))
}

#[tauri::command]
pub async fn inspect_dlss5_game_state(game_executable: String) -> Result<Dlss5GameStateSnapshot, String> {
    read_graphics_status(move || inspect_dlss5_game_state_blocking(game_executable)).await
}

// 状态读取会遍历历史并校验大型 DLL；异步命令也必须将这些文件 IO 移出调度线程。
pub(super) async fn read_graphics_status<T: Send + 'static>(
    read: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(read)
        .await
        .map_err(|error| format!("读取图形组件状态失败：{error}"))?
}

fn inspect_dlss5_game_state_blocking(game_executable: String) -> Result<Dlss5GameStateSnapshot, String> {
    let game_executable = PathBuf::from(game_executable);
    if !game_executable.is_file() {
        return Err(format!(
            "game executable does not exist: {}",
            game_executable.display()
        ));
    }
    Ok(Dlss5GameStateSnapshot::from(&inspect_game_executable(
        &game_executable,
    )))
}

#[tauri::command]
pub async fn inspect_dlss5_last_run(
    game_executable: String,
    game_name: Option<String>,
) -> Result<Dlss5LastRunStatus, String> {
    read_graphics_status(move || inspect_dlss5_last_run_blocking(game_executable, game_name)).await
}

fn inspect_dlss5_last_run_blocking(
    game_executable: String,
    game_name: Option<String>,
) -> Result<Dlss5LastRunStatus, String> {
    let executable = PathBuf::from(game_executable);
    if !executable.is_file() {
        return Err(format!(
            "game executable does not exist: {}",
            executable.display()
        ));
    }
    let log = match inspect_game_executable(&executable) {
        Dlss5State::Managed(state) => {
            let game_log = executable
                .parent()
                .ok_or("game executable has no parent directory")?
                .join("ReShade.log");
            if !game_log.is_file() {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "no_log",
                    observed_at_ms: None,
                    warning: None,
                });
            }
            let log_modified = fs::metadata(&game_log)
                .and_then(|metadata| metadata.modified())
                .map_err(|error| error.to_string())?;
            let manifest_modified = fs::metadata(&state.manifest_path)
                .and_then(|metadata| metadata.modified())
                .map_err(|error| error.to_string())?;
            if log_modified < manifest_modified {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "log_precedes_install",
                    observed_at_ms: None,
                    warning: None,
                });
            }
            game_log
        }
        Dlss5State::NotManaged => {
            let Some(game_name) = game_name else {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "no_log",
                    observed_at_ms: None,
                    warning: None,
                });
            };
            let Some(registry) = installed_plugin_registry()? else {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "no_log",
                    observed_at_ms: None,
                    warning: None,
                });
            };
            let scoped = registry.for_game(&game_name);
            if !scoped
                .find(super::dlss5::DLSS5_PLUGIN_ID)
                .is_some_and(|plugin| plugin.enabled)
            {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "no_log",
                    observed_at_ms: None,
                    warning: None,
                });
            }
            let Some(log) = super::dlss5_hot_runtime::log_path(&game_name)? else {
                return Ok(Dlss5LastRunStatus {
                    verdict: "unknown",
                    reason: "no_log",
                    observed_at_ms: None,
                    warning: None,
                });
            };
            log
        }
        Dlss5State::Broken { .. } => {
            return Ok(Dlss5LastRunStatus {
                verdict: "unknown",
                reason: "no_log",
                observed_at_ms: None,
                warning: None,
            });
        }
    };
    if !log.is_file() {
        return Ok(Dlss5LastRunStatus {
            verdict: "unknown",
            reason: "no_log",
            observed_at_ms: None,
            warning: None,
        });
    }
    inspect_dlss5_last_run_log(&log)
}

#[tauri::command]
pub fn inspect_managed_graphics_stack(
    game_name: String,
    game_executable: String,
) -> Result<ManagedStackStatus, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    let executable = PathBuf::from(game_executable);
    match inspect_game_executable(&executable) {
        Dlss5State::Managed(state) => managed_stack::inspect(&registry, &executable, &state),
        Dlss5State::Broken { reason, .. } => Err(reason),
        Dlss5State::NotManaged => Err("DLSS5 is not installed for this game".to_string()),
    }
}

#[tauri::command]
pub fn restore_managed_graphics_stack(game_executable: String) -> Result<bool, String> {
    managed_stack::restore(&PathBuf::from(game_executable))
}

#[tauri::command]
pub async fn inspect_dlss5_route_options(
    game_name: String,
    game_executable: String,
    api_override: String,
) -> Result<Dlss5RouteScan, String> {
    read_graphics_status(move || {
        inspect_dlss5_route_options_blocking(game_name, game_executable, api_override)
    })
    .await
}

fn inspect_dlss5_route_options_blocking(
    game_name: String,
    game_executable: String,
    api_override: String,
) -> Result<Dlss5RouteScan, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    let executable = PathBuf::from(game_executable);
    if super::dlss5_hot_runtime::applies(&executable) {
        let mut scan = super::dlss5_hot_runtime::route_scan(&registry, &game_scope_preset(&game_name), &executable, &api_override)?;
        if super::gimi_hot_runtime::applies(&executable) {
            scan.installed_route = super::dlss5_hot_runtime::gimi_route(&game_name).ok();
        }
        return Ok(scan);
    }
    managed_stack::inspect_routes(&registry, &executable, &api_override)
}

#[tauri::command]
pub fn install_managed_dlss5_route(
    game_name: String,
    game_executable: String,
    route: String,
    api_override: String,
    anti_cheat_acknowledged: bool,
) -> Result<Dlss5RouteScan, String> {
    let mut registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?;
    let executable = PathBuf::from(game_executable);
    let result = managed_stack::install_route(
        &registry.for_game(&game_name),
        &executable,
        &route,
        &api_override,
        anti_cheat_acknowledged,
    )?;
    if let Err(error) = registry.set_managed_game_executable(&game_name, DLSS5_PLUGIN_ID, Some(&executable)) {
        let rollback = managed_stack::restore_swapper(&registry.for_game(&game_name), &executable);
        return Err(format!("cannot record DLSS5 installation: {error}; rollback: {rollback:?}"));
    }
    Ok(result)
}

#[tauri::command]
pub fn restore_dlss5_install(game_name: String, game_executable: String) -> Result<(), String> {
    let mut registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?;
    managed_stack::restore_swapper(&registry.for_game(&game_name), &PathBuf::from(game_executable))?;
    registry.set_managed_game_executable(&game_name, DLSS5_PLUGIN_ID, None)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_dlss5_swapper(game_name: String) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    managed_stack::open_swapper(&registry)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleStatus {
    Installed,
    Enabled,
    Disabled,
    UpdateAvailable,
    Broken,
    Incompatible,
    ExternalDependencyMissing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginSnapshot {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub lifecycle_status: PluginLifecycleStatus,
    pub external_dependencies: BTreeMap<String, ExternalDependencyState>,
    pub bundled: bool,
    pub official: bool,
    pub app_scoped: bool,
    pub features: Vec<String>,
}

const PLAYER_TWEAKS_ID: &str = "ssmt.player-tweaks";
const PLAYER_TWEAKS_FILE: &str = "SSMT-Player-Tweaks.dll";

fn player_tweaks_available() -> bool {
    crate::config::path_manager::PathManager::ssmt_resources_folder()
        .join(PLAYER_TWEAKS_FILE)
        .is_file()
}

fn player_tweaks_features(preset: &str) -> Vec<String> {
    super::player_tweaks::features(None, preset).unwrap_or_default()
}

fn bundled_player_tweaks_snapshot(
    registry: &PluginRegistry,
    game_name: &str,
) -> Option<InstalledPluginSnapshot> {
    let preset = game_scope_preset(game_name);
    if !preset.eq_ignore_ascii_case("SRMI") && !player_tweaks_available() {
        return None;
    }
    let features = player_tweaks_features(&preset);
    let compatible = !features.is_empty();
    let enabled = compatible && registry.game_enabled(game_name, PLAYER_TWEAKS_ID);
    Some(InstalledPluginSnapshot {
        manifest: PluginManifest {
            schema_version: 1,
            id: PLAYER_TWEAKS_ID.to_string(),
            name: "SSMT Player Tweaks".to_string(),
            version: "0.0.0".to_string(),
            author: "SSMT".to_string(),
            compatibility: PluginCompatibility {
                ssmt: ">=4.x".to_string(),
                platforms: vec!["windows-x64".to_string()],
                games: vec!["GIMI".to_string(), "SRMI".to_string()],
            },
            contributions: PluginContributions {
                host_extensions: vec![super::host_extensions::HostExtensionContribution {
                    id: "settings".into(), provider: "ssmt.player-tweaks".into(), priority: 0,
                    config: None, settings_component: Some("ssmt.player-tweaks.settings".into()),
                }],
                ..PluginContributions::default()
            },
            external_dependencies: Vec::new(),
            permissions: if preset.eq_ignore_ascii_case("SRMI") {
                vec![PluginPermission::FilesystemRead, PluginPermission::FilesystemWrite]
            } else { vec![PluginPermission::NativeInject] },
        },
        enabled,
        lifecycle_status: if compatible {
            if enabled { PluginLifecycleStatus::Enabled } else { PluginLifecycleStatus::Disabled }
        } else {
            PluginLifecycleStatus::Incompatible
        },
        external_dependencies: BTreeMap::new(),
        bundled: true,
        official: true,
        app_scoped: false,
        features,
    })
}

fn snapshot_for_game(registry: &PluginRegistry, game_name: &str) -> Vec<InstalledPluginSnapshot> {
    let preset = game_scope_preset(game_name);
    let mut plugins = snapshot(&registry.for_game_preset(game_name, &preset));
    for plugin in &mut plugins {
        if plugin.manifest.id == PLAYER_TWEAKS_ID {
            match super::player_tweaks::features(registry.find(PLAYER_TWEAKS_ID), &preset) {
                Ok(features) => plugin.features = features,
                Err(_) => { plugin.enabled = false; plugin.lifecycle_status = PluginLifecycleStatus::Broken; }
            }
        }
        if !plugin.app_scoped && !plugin.manifest.compatibility.supports_game(&preset) {
            plugin.enabled = false;
            plugin.lifecycle_status = PluginLifecycleStatus::Incompatible;
        }
    }
    if let Some(bundled) = (registry.find(PLAYER_TWEAKS_ID).is_none()).then(|| bundled_player_tweaks_snapshot(registry, game_name)).flatten() {
        plugins.push(bundled);
    }
    plugins
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginUiRouteSnapshot {
    pub plugin_id: String,
    pub plugin_version: String,
    pub page_id: String,
    pub route: String,
    pub package_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilitiesSnapshot {
    pub plugin_id: String,
    pub capabilities: Vec<String>,
}

fn snapshot(registry: &PluginRegistry) -> Vec<InstalledPluginSnapshot> {
    registry
        .installed()
        .iter()
        .map(|plugin| InstalledPluginSnapshot {
            manifest: {
                let mut manifest = plugin.manifest.clone();
                manifest.contributions.host_extensions = super::host_graphics::settings_extensions(plugin);
                manifest.external_dependencies = plugin.manifest.display_dependencies();
                manifest
            },
            enabled: plugin.enabled,
            lifecycle_status: lifecycle_status(plugin),
            external_dependencies: plugin.external_dependencies.clone(),
            bundled: false,
            official: plugin.official,
            app_scoped: plugin.manifest.is_app_scoped(),
            features: Vec::new(),
        })
        .collect()
}

fn lifecycle_status(plugin: &super::registry::InstalledPlugin) -> PluginLifecycleStatus {
    if plugin
        .external_dependencies
        .iter()
        .any(|(dependency_id, dependency)| {
            dependency.status != super::registry::ExternalDependencyStatus::Ready
                && plugin.manifest.requires_dependency(dependency_id, "launch")
        })
    {
        return PluginLifecycleStatus::ExternalDependencyMissing;
    }
    if plugin.enabled {
        PluginLifecycleStatus::Enabled
    } else {
        PluginLifecycleStatus::Disabled
    }
}

fn ui_routes(registry: &PluginRegistry) -> Vec<PluginUiRouteSnapshot> {
    registry
        .installed()
        .iter()
        .filter(|plugin| plugin.enabled)
        .flat_map(|plugin| {
            plugin
                .manifest
                .contributions
                .ui_pages
                .iter()
                .map(|page| PluginUiRouteSnapshot {
                    plugin_id: plugin.id().to_string(),
                    plugin_version: plugin.version().to_string(),
                    page_id: page.id.clone(),
                    route: page.route.clone(),
                    package_path: page.path.clone(),
                })
        })
        .collect()
}

fn capabilities_for_permissions(permissions: &[super::PluginPermission]) -> Vec<String> {
    let mut capabilities = vec!["plugin.settings".to_string()];
    for permission in permissions {
        let capability = match permission {
            super::PluginPermission::FilesystemRead => Some("filesystem.read"),
            super::PluginPermission::FilesystemWrite => Some("filesystem.write"),
            super::PluginPermission::ProcessSpawn => Some("process.spawn"),
            super::PluginPermission::ProcessObserve => Some("process.observe"),
            super::PluginPermission::GameLaunch => Some("launch.start"),
            super::PluginPermission::Network => Some("network"),
            // Native injection remains a core launch/runtime concern and is
            // deliberately not exposed to UI packages.
            super::PluginPermission::NativeInject => None,
        };
        if let Some(capability) = capability {
            capabilities.push(capability.to_string());
        }
    }
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

#[tauri::command]
pub fn plugin_registry_snapshot() -> Result<Vec<InstalledPluginSnapshot>, String> {
    installed_plugin_registry().map(|registry| registry.as_ref().map(snapshot).unwrap_or_default())
}

#[tauri::command]
pub fn plugin_registry_snapshot_for_game(game_name: String) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    Ok(snapshot_for_game(&registry, &game_name))
}

#[tauri::command]
pub fn plugin_ui_routes(game_name: String) -> Result<Vec<PluginUiRouteSnapshot>, String> {
    let routes = installed_plugin_registry()?
        .map(|registry| ui_routes(&registry.for_game(&game_name)))
        .unwrap_or_default();
    let mut seen = HashSet::new();
    for route in &routes {
        if !seen.insert(route.route.as_str()) {
            return Err(format!(
                "duplicate enabled plugin UI route: {}",
                route.route
            ));
        }
    }
    Ok(routes)
}

#[tauri::command]
pub fn plugin_capabilities(plugin_id: String, game_name: String) -> Result<PluginCapabilitiesSnapshot, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    let plugin = registry
        .find(&plugin_id)
        .ok_or_else(|| format!("plugin not found: {plugin_id}"))?;
    if !plugin.enabled {
        return Err(format!("plugin is disabled: {plugin_id}"));
    }
    Ok(PluginCapabilitiesSnapshot {
        plugin_id,
        capabilities: capabilities_for_permissions(&plugin.manifest.permissions),
    })
}

#[tauri::command]
pub fn set_plugin_enabled_for_game(
    game_name: String,
    id: String,
    enabled: bool,
    game_executable: Option<String>,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let cleanup_dlss5 = id == DLSS5_PLUGIN_ID && !enabled
        && (registry.game_enabled(&game_name, &id)
            || registry.managed_game_executable(&game_name, &id).is_some());
    if cleanup_dlss5 {
        let executable = registry.managed_game_executable(&game_name, &id).map(Path::to_path_buf)
            .or_else(|| game_executable.as_deref().map(str::trim).filter(|path| !path.is_empty()).map(PathBuf::from))
            .ok_or("game executable path is required to disable DLSS5 and restore its managed files")?;
        if !executable.is_file() {
            return Err(format!("game executable does not exist: {}", executable.display()));
        }
        match inspect_game_executable(&executable) {
            Dlss5State::Managed(state) if state.external_host.is_some() => {
                managed_stack::restore_swapper(&registry.for_game(&game_name), &executable)?;
            }
            Dlss5State::Broken { reason, .. } => {
                return Err(format!("cannot safely disable DLSS5 before repairing its installation: {reason}"));
            }
            // 没有 SSMT 宿主标记的安装可能由玩家自行管理，不能擅自移除。
            Dlss5State::Managed(_) | Dlss5State::NotManaged => {}
        }
    }
    if id == PLAYER_TWEAKS_ID && registry.find(&id).is_none() {
        let preset = game_scope_preset(&game_name);
        if enabled && player_tweaks_features(&preset).is_empty() {
            return Err("Player Tweaks 当前支持 GIMI 和 SRMI".to_string());
        }
        if enabled && preset.eq_ignore_ascii_case("GIMI") && !player_tweaks_available() {
            return Err("bundled Player Tweaks DLL is unavailable".to_string());
        }
        if !enabled && preset.eq_ignore_ascii_case("SRMI") {
            super::star_rail_tweaks::release_for_game(&game_name)?;
        }
        registry.set_bundled_game_enabled(&game_name, &id, enabled)
    } else {
        if id == PLAYER_TWEAKS_ID && !enabled && game_scope_preset(&game_name).eq_ignore_ascii_case("SRMI") {
            super::star_rail_tweaks::release_for_game(&game_name)?;
        }
        if registry.find(&id).is_some_and(|plugin| plugin.manifest.is_app_scoped()) {
            registry.set_enabled(&id, enabled)
        } else {
            registry.set_game_enabled(&game_name, &id, enabled)
        }
    }
    .map_err(|error| error.to_string())?;
    if cleanup_dlss5 {
        registry.set_managed_game_executable(&game_name, &id, None)
            .map_err(|error| error.to_string())?;
    }
    Ok(snapshot_for_game(&registry, &game_name))
}

#[tauri::command]
pub fn bundled_player_tweaks_enabled_for_game(game_name: String, game_preset: String) -> bool {
    let preset = game_scope_preset(&game_name);
    if !preset.eq_ignore_ascii_case(&game_preset) || player_tweaks_features(&preset).is_empty()
        || (preset.eq_ignore_ascii_case("GIMI") && !player_tweaks_available()) {
        return false;
    }
    PluginRegistry::from_default_location()
        .map(|registry| registry.game_enabled(&game_name, PLAYER_TWEAKS_ID))
        .unwrap_or(false)
}

#[tauri::command]
pub fn set_plugin_external_dependency_path(
    plugin_id: String,
    dependency_id: String,
    path: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    registry
        .set_external_dependency_path(plugin_id.trim(), dependency_id.trim(), path)
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn prepare_hoyoshade_launch(
    game_name: String,
    game_executable: String,
    graphics_decision: Option<GraphicsLaunchDecision>,
    skip_dlss5: Option<bool>,
) -> Result<Option<PluginLaunchProgram>, String> {
    let Some(registry) = installed_plugin_registry()?.map(|registry| registry.for_game(&game_name)) else {
        return Ok(None);
    };
    let game_executable = PathBuf::from(game_executable);
    let hot_route = super::gimi_hot_runtime::applies(&game_executable) || super::dlss5_hot_runtime::applies(&game_executable);
    let skip_dlss5 = skip_dlss5.unwrap_or(false) && hot_route;
    if hot_route
        && !skip_dlss5
        && registry
            .find(DLSS5_PLUGIN_ID)
            .is_some_and(|plugin| plugin.enabled)
    {
        return Err("当前游戏的 DLSS5 必须由热注入计划启动，不能另起 ReShade 注入器".into());
    }
    let inspection = inspect_graphics_for_launch(&registry, &game_executable, skip_dlss5)?;
    if !should_launch_hoyoshade(&inspection, graphics_decision)? {
        return Ok(None);
    }
    let Some(plugin) = registry
        .find(HOYOSHADE_PLUGIN_ID)
        .filter(|plugin| plugin.enabled || inspection.dlss5_requested)
    else {
        return Ok(None);
    };
    let bridge = HoYoShadeBridge::from_installed(plugin).map_err(|error| error.to_string())?;
    let process_name = game_executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "game executable has no file name".to_string())?;
    if !bridge.supports_process(process_name) {
        return Ok(None);
    }
    let dependency = plugin
        .external_dependencies
        .get(HOYOSHADE_DEPENDENCY_ID)
        .ok_or_else(|| "HoYoShade dependency is missing".to_string())?;
    let dependency_path = dependency
        .path
        .as_ref()
        .ok_or_else(|| "HoYoShade path is not configured".to_string())?;
    bridge
        .validate_external_directory(dependency_path)
        .map_err(|error| error.to_string())?;
    let managed = graphics_decision == Some(GraphicsLaunchDecision::PrepareManagedStack);
    if managed {
        if !registry
            .find(DLSS5_PLUGIN_ID)
            .is_some_and(|plugin| plugin.enabled)
        {
            return Err("DLSS5 integration plugin is not enabled".to_string());
        }
        match inspect_game_executable(&game_executable) {
            Dlss5State::Managed(state) => {
                managed_stack::prepare(&registry, &game_executable, &state, inspection.hoyoshade_requested)?;
                // 提权后的 Unity 进程不一定能枚举 OptiScaler 代理，且崩铁可能
                // 通过 D3D12 路径加载它。等待精确的 `dxgi.dll` 模块会把本来
                // 可用的 HoYoShade 注入变成超时；托管注入器已经会等待目标进程，
                // 不需要这个不稳定的模块门槛。
            }
            Dlss5State::Broken { reason, .. } => return Err(reason),
            Dlss5State::NotManaged => {
                return Err("DLSS5 is not installed for this game".to_string())
            }
        }
    }
    let injector = dependency_path.join("inject.exe");
    Ok(Some(PluginLaunchProgram {
        path: injector.to_string_lossy().into_owned(),
        args: if managed {
            format!("--ssmt-managed-config {process_name}")
        } else {
            process_name.to_string()
        },
        work_dir: dependency_path.to_string_lossy().into_owned(),
        run_as_administrator: true,
        // 注入器先安装进程监视，再由 Run.exe 创建游戏；给外部注入器留出启动时间。
        post_launch_delay_ms: 2_000,
    }))
}

#[tauri::command]
pub fn prepare_plugin_host_config(game_name: String, runtime_directory: String) -> Result<Option<String>, String> {
    let Some(registry) = installed_plugin_registry()?.map(|registry| registry.for_game(&game_name)) else {
        return Ok(None);
    };
    let config = registry.generate_plugin_host_config();

    if config.plugins.is_empty() {
        return Ok(None);
    }

    let runtime_directory = PathBuf::from(runtime_directory);
    if !runtime_directory.is_dir() {
        return Err(format!(
            "3DMigoto runtime directory does not exist: {}",
            runtime_directory.display()
        ));
    }

    let config_path = runtime_directory.join("SSMT-PluginHost.json");
    let content = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("failed to serialize PluginHost config: {error}"))?;
    fs::write(&config_path, content).map_err(|error| {
        format!(
            "failed to write PluginHost config {}: {error}",
            config_path.display()
        )
    })?;

    Ok(Some(config_path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn install_plugin_package(
    app: tauri::AppHandle,
    archive_path: String,
    reviewed_sha256: String,
    disclaimer_accepted: bool,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if !disclaimer_accepted {
        return Err("third-party plugin disclaimer must be accepted".into());
    }
    let inspection = inspect_ssmtpkg(&PathBuf::from(&archive_path)).map_err(|error| error.to_string())?;
    if inspection.manifest.id.starts_with("ssmt.") {
        return Err("ssmt.* is reserved for packages from the Native Release".into());
    }
    if inspection.sha256 != reviewed_sha256.to_ascii_lowercase() {
        return Err("package changed since safety review".into());
    }
    if !inspection.manifest.compatibility.supports(&app.package_info().version.to_string(), super::SUPPORTED_PLATFORM, None) {
        return Err("plugin is incompatible with this SSMT version or platform".into());
    }
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_verified_ssmtpkg(&mut registry, &PathBuf::from(archive_path), &reviewed_sha256)
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn inspect_plugin_package(archive_path: String) -> Result<PackageInspection, String> {
    let inspection = inspect_ssmtpkg(&PathBuf::from(archive_path)).map_err(|error| error.to_string())?;
    if inspection.manifest.id.starts_with("ssmt.") {
        return Err("ssmt.* is reserved for packages from the Native Release".into());
    }
    Ok(inspection)
}

#[tauri::command]
pub async fn local_plugin_packages() -> Result<Vec<super::local_packages::LocalPluginPackage>, String> {
    // 包检查会解压并计算压缩包哈希，不能占用 WebView 的交互线程。
    tauri::async_runtime::spawn_blocking(|| {
        super::local_packages::discover(&crate::config::path_manager::PathManager::ssmt_resources_folder().join("PluginPackages"))
    }).await.map_err(|error| format!("读取随包插件目录失败：{error}"))?
}

#[tauri::command]
pub fn install_local_plugin(
    app: tauri::AppHandle,
    id: String,
    reviewed_sha256: String,
    disclaimer_accepted: bool,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if !disclaimer_accepted { return Err("local plugin disclaimer must be accepted".into()); }
    // 本地构建保留 ID 以复用已有贡献，但安装来源仍未验证；不能借此安装任意路径的 ssmt.* 包。
    let package = super::local_packages::reviewed(
        &crate::config::path_manager::PathManager::ssmt_resources_folder().join("PluginPackages"),
        &id, &reviewed_sha256,
    )?;
    if !package.inspection.manifest.compatibility.supports(&app.package_info().version.to_string(), super::SUPPORTED_PLATFORM, None) {
        return Err("plugin is incompatible with this SSMT version or platform".into());
    }
    let mut registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_verified_ssmtpkg(&mut registry, &package.archive_path, &reviewed_sha256).map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub async fn official_plugin_catalog() -> Result<Vec<MarketplaceEntry>, String> {
    official_release::official_catalog().await
}

#[tauri::command]
pub async fn install_official_plugin(app: tauri::AppHandle, id: String, version: String) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    official_release::install_official(&mut registry, &id, &version, &app.package_info().version.to_string()).await?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn read_plugin_log(plugin_id: String) -> Result<String, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginLogWriter::new(&plugin_id)
        .and_then(|writer| writer.read())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn clear_plugin_log(plugin_id: String) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginLogWriter::new(&plugin_id)
        .and_then(|writer| writer.clear())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_plugin_setting(
    plugin_id: String,
    key: String,
) -> Result<Option<serde_json::Value>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginSettingsStore::from_default_location()
        .and_then(|store| store.get(&plugin_id, &key))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_plugin_setting(
    plugin_id: String,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    let mut store =
        PluginSettingsStore::from_default_location().map_err(|error| error.to_string())?;
    store
        .set(&plugin_id, &key, value)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        capabilities_for_permissions, inspect_graphics_for_executable, inspect_graphics_for_launch, installed_plugin_registry_at,
        lifecycle_status,
        should_launch_hoyoshade, Dlss5GameStateSnapshot, GraphicsLaunchDecision,
        GraphicsLaunchInspection, PluginLifecycleStatus, inspect_dlss5_last_run_log,
    };
    use crate::plugin::graphics_stack::{
        CompatibilityLevel, GraphicsCompatibilityIssue, GraphicsStackResolution,
    };
    use crate::plugin::registry::{
        ExternalDependencyState, ExternalDependencyStatus, InstalledPlugin, PluginRegistry,
    };
    use crate::plugin::{
        PluginCompatibility, PluginContributions, PluginManifest, PluginPermission,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test(flavor = "current_thread")]
    async fn graphics_status_io_does_not_block_the_dispatch_thread() {
        let caller = std::thread::current().id();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let read = tokio::spawn(super::read_graphics_status(move || {
            let _ = started.send(std::thread::current().id());
            released
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            Ok(42)
        }));
        let worker = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        // 读取被闸门挂起时，单线程调度器仍可处理别的任务。
        assert_ne!(worker, caller);
        tokio::task::yield_now().await;
        assert!(!read.is_finished());
        release.send(()).unwrap();
        assert_eq!(read.await.unwrap().unwrap(), 42);
    }

    #[tokio::test]
    async fn graphics_status_io_preserves_read_errors() {
        let result = super::read_graphics_status(|| Err::<(), _>("组件校验失败".into())).await;
        assert_eq!(result.unwrap_err(), "组件校验失败");
    }

    #[test]
    fn dlss5_last_run_recognizes_feature18_and_reports_depth_mismatch() {
        let dir = std::env::temp_dir().join(format!(
            "ssmt-dlss5-last-run-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("ReShade.log");
        fs::write(&log, "addon loaded\n[DLSS 5 Feed] stopped: Direct3D 11 multithread protection is unavailable on this device and a present-path interposer is loaded\n").unwrap();
        let status = inspect_dlss5_last_run_log(&log).unwrap();
        assert_eq!(status.verdict, "inactive");
        assert_eq!(status.reason, "d3d11_interposer_race");
        fs::write(
            &log,
            "addon loaded\ninline feature 18 evaluation succeeded (count=60)\nThe depth source is 5120x3200, most likely a shadow map or atlas\n",
        )
        .unwrap();
        let status = inspect_dlss5_last_run_log(&log).unwrap();
        assert_eq!(status.verdict, "engaged");
        assert_eq!(status.reason, "feature18_evaluated");
        assert_eq!(status.warning, Some("depth_source_mismatch"));
        fs::write(
            &log,
            "addon loaded\nNR-VERDICT state=ENGAGED; successful frames 60\n",
        )
        .unwrap();
        assert_eq!(inspect_dlss5_last_run_log(&log).unwrap().verdict, "engaged");
        fs::write(&log, "addon loaded\n").unwrap();
        let status = inspect_dlss5_last_run_log(&log).unwrap();
        assert_eq!(status.verdict, "unknown");
        assert_eq!(status.warning, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn maps_declared_permissions_to_scoped_capabilities() {
        let capabilities = capabilities_for_permissions(&[
            PluginPermission::FilesystemRead,
            PluginPermission::GameLaunch,
            PluginPermission::NativeInject,
            PluginPermission::FilesystemRead,
        ]);
        assert_eq!(
            capabilities,
            vec!["filesystem.read", "launch.start", "plugin.settings"]
        );
    }

    #[test]
    fn hot_preflight_covers_every_game_and_official_plugin_combination() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-gimi-combinations-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("game");
        fs::create_dir_all(game.join("_DLSS5_Backup")).unwrap();
        let executable = game.join("YuanShen.exe");
        fs::write(&executable, b"fixture").unwrap();
        let official =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../native/official-plugin-packages");
        let mut ids = Vec::new();
        for id in fs::read_dir(official)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().is_dir())
        {
            let mut versions = fs::read_dir(id.path())
                .unwrap()
                .flatten()
                .filter(|entry| entry.path().join("ssmt-plugin.json").is_file())
                .collect::<Vec<_>>();
            versions.sort_by_key(|entry| entry.file_name());
            let package = versions.last().unwrap().path();
            let raw = fs::read_to_string(package.join("ssmt-plugin.json")).unwrap();
            let manifest = PluginManifest::from_json_str(&raw).unwrap();
            let installed = root
                .join("plugins")
                .join(&manifest.id)
                .join(&manifest.version);
            fs::create_dir_all(&installed).unwrap();
            fs::write(installed.join("ssmt-plugin.json"), raw).unwrap();
            ids.push(manifest.id);
        }
        assert!(ids.len() >= 3);
        ids.push(super::PLAYER_TWEAKS_ID.into());
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let mut combinations = vec![Vec::new(), ids.clone()];
        for (index, id) in ids.iter().enumerate() {
            combinations.push(vec![id.clone()]);
            for other in &ids[index + 1..] {
                combinations.push(vec![id.clone(), other.clone()]);
            }
        }
        for (preset, name) in [("GIMI", "YuanShen.exe"), ("SRMI", "StarRail.exe"),
            ("WWMI", "Client-Win64-Shipping.exe"), ("ZZMI", "ZenlessZoneZero.exe"), ("ZZMIDX12", "ZenlessZoneZero.exe")] {
          let executable = game.join(name);
          fs::write(&executable, b"fixture").unwrap();
          for combination in &combinations {
            for id in &ids {
                if id == super::PLAYER_TWEAKS_ID {
                    registry.set_bundled_game_enabled(preset, id,
                        !super::player_tweaks_features(preset).is_empty() && combination.contains(id)).unwrap();
                    continue;
                }
                let compatible = registry.find(id).unwrap().manifest.compatibility.supports_game(preset);
                registry
                    .set_game_enabled(preset, id, compatible && combination.contains(id))
                    .unwrap();
            }
            let scoped = registry.for_game(preset);
            let dlss5 = combination.iter().any(|id| id == "ssmt.dlss5.integration");
            let shade = combination.iter().any(|id| id == super::super::hoyoshade::HOYOSHADE_PLUGIN_ID)
                && scoped.find(super::super::hoyoshade::HOYOSHADE_PLUGIN_ID).is_some_and(|plugin|
                    super::super::hoyoshade::HoYoShadeBridge::from_installed(plugin).unwrap().supports_process(name));
            let clean = inspect_graphics_for_executable(&scoped, &executable).unwrap();
            assert_eq!(clean.resolution.level, CompatibilityLevel::Compatible);
            assert_eq!(clean.host_requested, dlss5 || shade);
            fs::write(game.join("_DLSS5_Backup/manifest.json"), b"invalid").unwrap();
            let broken = inspect_graphics_for_executable(&scoped, &executable).unwrap();
            assert_eq!(
                broken.resolution.level,
                if dlss5 {
                    CompatibilityLevel::Conflict
                } else {
                    CompatibilityLevel::Compatible
                }
            );
            let fallback = inspect_graphics_for_launch(&scoped, &executable, true).unwrap();
            assert!(!fallback.dlss5_requested);
            assert_eq!(fallback.host_requested, shade);
            assert_eq!(fallback.resolution.level, CompatibilityLevel::Compatible);
            fs::remove_file(game.join("_DLSS5_Backup/manifest.json")).unwrap();
          }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn player_tweaks_exposes_only_implemented_features_for_each_game() {
        assert_eq!(super::player_tweaks_features("GIMI"), ["camera", "fps", "fast-team"]);
        assert_eq!(super::player_tweaks_features("srmi"), ["fps-120"]);
        for preset in ["WWMI", "ZZMI", "EFMI", "ZZMIDX12"] { assert!(super::player_tweaks_features(preset).is_empty()); }
    }

    #[test]
    fn no_downloaded_plugins_ignore_even_a_broken_dlss5_install() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ssmt-no-plugin-graphics-{}-{nonce}",
            std::process::id()
        ));
        let game = root.join("game");
        fs::create_dir_all(game.join("_DLSS5_Backup")).unwrap();
        let executable = game.join("YuanShen.exe");
        fs::write(&executable, b"fixture").unwrap();
        fs::write(game.join("_DLSS5_Backup/manifest.json"), b"invalid JSON").unwrap();

        let registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let inspection = inspect_graphics_for_executable(&registry, &executable).unwrap();
        assert!(!inspection.hoyoshade_requested);
        assert_eq!(inspection.dlss5.state, "not_managed");
        assert_eq!(inspection.resolution.level, CompatibilityLevel::Compatible);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_plugin_install_does_not_read_stale_state_or_create_registry() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ssmt-empty-plugin-launch-{}-{nonce}",
            std::process::id()
        ));
        let plugins = root.join("plugins");
        assert!(installed_plugin_registry_at(plugins.clone()).unwrap().is_none());
        assert!(!plugins.exists());

        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("registry-state.json"), b"invalid JSON").unwrap();
        assert!(installed_plugin_registry_at(plugins.clone()).unwrap().is_none());
        assert_eq!(fs::read(plugins.join("registry-state.json")).unwrap(), b"invalid JSON");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn derives_application_lifecycle_without_runtime_states() {
        let manifest = PluginManifest {
            schema_version: 1,
            id: "ssmt.fixture".to_string(),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            author: "SSMT".to_string(),
            compatibility: PluginCompatibility {
                ssmt: ">=4.0.0".to_string(),
                platforms: vec!["windows-x64".to_string()],
                games: Vec::new(),
            },
            contributions: PluginContributions::default(),
            external_dependencies: vec![],
            permissions: vec![PluginPermission::ProcessSpawn],
        };
        let mut plugin = InstalledPlugin {
            manifest,
            package_root: PathBuf::from("fixture"),
            enabled: false,
            official: false,
            external_dependencies: BTreeMap::new(),
        };
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Disabled
        ));
        plugin.enabled = true;
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Enabled
        ));
        plugin.external_dependencies.insert(
            "external".to_string(),
            ExternalDependencyState {
                status: ExternalDependencyStatus::Missing,
                path: None,
                reason: Some("missing file".to_string()),
            },
        );
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::ExternalDependencyMissing
        ));
    }

    #[test]
    fn missing_legacy_swapper_paths_do_not_mark_hot_dlss5_as_missing() {
        let mut plugin = InstalledPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                id: super::super::dlss5::DLSS5_PLUGIN_ID.to_string(),
                name: "DLSS5 Integration".to_string(),
                version: "0.2.1".to_string(),
                author: "SSMT".to_string(),
                compatibility: PluginCompatibility {
                    ssmt: ">=4.0.0".to_string(),
                    platforms: vec!["windows-x64".to_string()],
                    games: Vec::new(),
                },
                contributions: PluginContributions::default(),
                external_dependencies: Vec::new(),
                permissions: Vec::new(),
            },
            package_root: PathBuf::from("fixture"),
            enabled: true,
            official: true,
            external_dependencies: BTreeMap::new(),
        };
        for id in ["dlss5-swapper", "dlss5-adapter", "dlss5-payload"] {
            plugin.external_dependencies.insert(
                id.to_string(),
                ExternalDependencyState {
                    status: ExternalDependencyStatus::Missing,
                    path: None,
                    reason: Some("not configured".to_string()),
                },
            );
        }
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Enabled
        ));
        plugin.manifest.version = "0.2.0".to_string();
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::ExternalDependencyMissing
        ));
        plugin.manifest.version = "0.2.1".to_string();
        plugin.external_dependencies.insert(
            "required-feature".to_string(),
            ExternalDependencyState {
                status: ExternalDependencyStatus::Missing,
                path: None,
                reason: Some("missing".to_string()),
            },
        );
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::ExternalDependencyMissing
        ));
    }

    #[test]
    fn launch_gate_requires_confirmation_and_never_bypasses_managed_stack() {
        let mut inspection = GraphicsLaunchInspection {
            hoyoshade_requested: true,
            dlss5_requested: true,
            host_requested: true,
            dlss5: Dlss5GameStateSnapshot {
                state: "managed",
                route: Some("optiscaler".to_string()),
                uses_reshade: Some(false),
                uses_proxy: Some(true),
                manifest_path: None,
                reason: None,
                managed_host: Some(false),
                owner: None,
            },
            resolution: GraphicsStackResolution {
                level: CompatibilityLevel::Warning,
                issues: vec![GraphicsCompatibilityIssue {
                    level: CompatibilityLevel::Warning,
                    components: Vec::new(),
                    code: "warning".to_string(),
                    message: "confirm first".to_string(),
                    possible_actions: Vec::new(),
                }],
            },
        };
        assert!(should_launch_hoyoshade(&inspection, None).is_err());
        assert_eq!(
            should_launch_hoyoshade(
                &inspection,
                Some(GraphicsLaunchDecision::ContinueThisLaunch)
            ),
            Ok(true)
        );
        assert_eq!(
            should_launch_hoyoshade(
                &inspection,
                Some(GraphicsLaunchDecision::SuppressHoYoShadeThisLaunch)
            ),
            Ok(false)
        );

        inspection.resolution.level = CompatibilityLevel::RequiresManagedStack;
        assert!(should_launch_hoyoshade(
            &inspection,
            Some(GraphicsLaunchDecision::ContinueThisLaunch)
        )
        .is_err());
        inspection.resolution.level = CompatibilityLevel::Conflict;
        assert!(should_launch_hoyoshade(
            &inspection,
            Some(GraphicsLaunchDecision::ContinueThisLaunch)
        )
        .is_err());
    }
}
