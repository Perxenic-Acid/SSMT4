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
use super::registry::{ExternalDependencyState, PluginRegistry, MANIFEST_FILE_NAME};
use super::settings::PluginSettingsStore;
use super::{PluginCompatibility, PluginContributions, PluginManifest, PluginPermission};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
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
    } else if log.contains("NR-VERDICT state=ENGAGED") {
        ("engaged", "nr_evaluated")
    } else if log.contains("NO DLSS CREATE SEEN") {
        ("inactive", "no_dlss_create")
    } else {
        ("unknown", "no_nr_evidence")
    };
    Ok(Dlss5LastRunStatus { verdict, reason, observed_at_ms })
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
    let dlss5_enabled = registry
        .find(DLSS5_PLUGIN_ID)
        .is_some_and(|plugin| plugin.enabled);
    let state = GameGraphicsState {
        dlss5: if dlss5_enabled {
            inspect_game_executable(game_executable)
        } else {
            Dlss5State::NotManaged
        },
    };
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

fn installed_plugin_registry() -> Result<Option<PluginRegistry>, String> {
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
pub fn inspect_dlss5_game_state(game_executable: String) -> Result<Dlss5GameStateSnapshot, String> {
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
pub fn inspect_dlss5_last_run(game_executable: String) -> Result<Dlss5LastRunStatus, String> {
    let executable = PathBuf::from(game_executable);
    if !executable.is_file() {
        return Err(format!("game executable does not exist: {}", executable.display()));
    }
    let managed = match inspect_game_executable(&executable) {
        Dlss5State::Managed(state) => state,
        _ => return Ok(Dlss5LastRunStatus { verdict: "unknown", reason: "no_log", observed_at_ms: None }),
    };
    let log = executable.parent().ok_or("game executable has no parent directory")?.join("ReShade.log");
    if !log.is_file() {
        return Ok(Dlss5LastRunStatus {
            verdict: "unknown",
            reason: "no_log",
            observed_at_ms: None,
        });
    }
    let log_modified = fs::metadata(&log).and_then(|metadata| metadata.modified()).map_err(|error| error.to_string())?;
    let manifest_modified = fs::metadata(&managed.manifest_path).and_then(|metadata| metadata.modified()).map_err(|error| error.to_string())?;
    if log_modified < manifest_modified {
        return Ok(Dlss5LastRunStatus { verdict: "unknown", reason: "log_precedes_install", observed_at_ms: None });
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
pub fn inspect_dlss5_route_options(
    game_name: String,
    game_executable: String,
    api_override: String,
) -> Result<Dlss5RouteScan, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    managed_stack::inspect_routes(&registry, &PathBuf::from(game_executable), &api_override)
}

#[tauri::command]
pub fn install_managed_dlss5_route(
    game_name: String,
    game_executable: String,
    route: String,
    api_override: String,
    anti_cheat_acknowledged: bool,
) -> Result<Dlss5RouteScan, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    managed_stack::install_route(
        &registry,
        &PathBuf::from(game_executable),
        &route,
        &api_override,
        anti_cheat_acknowledged,
    )
}

#[tauri::command]
pub fn restore_dlss5_install(game_name: String, game_executable: String) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    managed_stack::restore_swapper(&registry, &PathBuf::from(game_executable))
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
}

const PLAYER_TWEAKS_ID: &str = "ssmt.player-tweaks";
const PLAYER_TWEAKS_FILE: &str = "SSMT-Player-Tweaks.dll";

fn player_tweaks_available() -> bool {
    crate::config::path_manager::PathManager::ssmt_resources_folder()
        .join(PLAYER_TWEAKS_FILE)
        .is_file()
}

fn game_preset(game_name: &str) -> Option<String> {
    let config = crate::config::path_manager::PathManager::global_config_games_game_folder(game_name)
        .join("Config.json");
    let raw = fs::read_to_string(config).ok()?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()?
        .get("gamePreset")?
        .as_str()
        .map(str::to_owned)
}

fn bundled_player_tweaks_snapshot(
    registry: &PluginRegistry,
    game_name: &str,
) -> Option<InstalledPluginSnapshot> {
    if !player_tweaks_available() {
        return None;
    }
    let compatible = game_preset(game_name)
        .is_some_and(|preset| preset.eq_ignore_ascii_case("GIMI"));
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
                games: vec!["GIMI".to_string()],
            },
            contributions: PluginContributions::default(),
            external_dependencies: Vec::new(),
            permissions: vec![PluginPermission::NativeInject],
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
    })
}

fn snapshot_for_game(registry: &PluginRegistry, game_name: &str) -> Vec<InstalledPluginSnapshot> {
    let mut plugins = snapshot(&registry.for_game(game_name));
    if let Some(bundled) = bundled_player_tweaks_snapshot(registry, game_name) {
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
            manifest: plugin.manifest.clone(),
            enabled: plugin.enabled,
            lifecycle_status: lifecycle_status(plugin),
            external_dependencies: plugin.external_dependencies.clone(),
            bundled: false,
            official: plugin.official,
            app_scoped: plugin.manifest.is_app_scoped(),
        })
        .collect()
}

fn lifecycle_status(plugin: &super::registry::InstalledPlugin) -> PluginLifecycleStatus {
    if plugin
        .external_dependencies
        .values()
        .any(|dependency| dependency.status != super::registry::ExternalDependencyStatus::Ready)
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
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if id == PLAYER_TWEAKS_ID {
        if !player_tweaks_available() {
            return Err("bundled Player Tweaks DLL is unavailable".to_string());
        }
        if !game_preset(&game_name).is_some_and(|preset| preset.eq_ignore_ascii_case("GIMI")) {
            return Err("Player Tweaks only supports GIMI".to_string());
        }
        registry.set_bundled_game_enabled(&game_name, &id, enabled)
    } else {
        if registry.find(&id).is_some_and(|plugin| plugin.manifest.is_app_scoped()) {
            registry.set_enabled(&id, enabled)
        } else {
            registry.set_game_enabled(&game_name, &id, enabled)
        }
    }
    .map_err(|error| error.to_string())?;
    Ok(snapshot_for_game(&registry, &game_name))
}

#[tauri::command]
pub fn bundled_player_tweaks_enabled_for_game(game_name: String, game_preset: String) -> bool {
    if !game_preset.eq_ignore_ascii_case("GIMI") || !player_tweaks_available() {
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
) -> Result<Option<PluginLaunchProgram>, String> {
    let Some(registry) = installed_plugin_registry()?.map(|registry| registry.for_game(&game_name)) else {
        return Ok(None);
    };
    let game_executable = PathBuf::from(game_executable);
    let inspection = inspect_graphics_for_executable(&registry, &game_executable)?;
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
        capabilities_for_permissions, inspect_graphics_for_executable, installed_plugin_registry_at,
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

    #[test]
    fn dlss5_last_run_requires_runtime_evidence() {
        let dir = std::env::temp_dir().join(format!(
            "ssmt-dlss5-last-run-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("ReShade.log");
        fs::write(&log, "addon loaded\n[DLSS 5 Feed] stopped: Direct3D 11 multithread protection is unavailable on this device and a present-path interposer is loaded\n").unwrap();
        let status = inspect_dlss5_last_run_log(&log).unwrap();
        assert_eq!(status.verdict, "inactive");
        assert_eq!(status.reason, "d3d11_interposer_race");
        fs::write(&log, "addon loaded\nNR-VERDICT state=ENGAGED; successful frames 60\n").unwrap();
        assert_eq!(inspect_dlss5_last_run_log(&log).unwrap().verdict, "engaged");
        fs::write(&log, "addon loaded\n").unwrap();
        assert_eq!(inspect_dlss5_last_run_log(&log).unwrap().verdict, "unknown");
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
