use super::dlss5::{inspect_game_executable, Dlss5Route, Dlss5State};
use super::graphics_stack::{
    CompatibilityLevel, GameGraphicsState, GraphicsLaunchContributions, GraphicsStackResolution,
    GraphicsStackResolver,
};
use super::hoyoshade::{HoYoShadeBridge, HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};
use super::logging::PluginLogWriter;
use super::managed_stack::{self, Dlss5RouteScan, ManagedStackStatus};
use super::package_installer::install_ssmtpkg;
use super::registry::{ExternalDependencyState, PluginRegistry};
use super::settings::PluginSettingsStore;
use super::PluginManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginLaunchProgram {
    pub path: String,
    pub args: String,
    pub work_dir: String,
    pub run_as_administrator: bool,
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
    if !inspection.hoyoshade_requested
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
    let state = GameGraphicsState {
        dlss5: inspect_game_executable(game_executable),
    };
    let resolution = GraphicsStackResolver::resolve(
        &state,
        GraphicsLaunchContributions {
            hoyoshade: hoyoshade_requested,
        },
    );
    Ok(GraphicsLaunchInspection {
        hoyoshade_requested,
        dlss5: Dlss5GameStateSnapshot::from(&state.dlss5),
        resolution,
    })
}

#[tauri::command]
pub fn inspect_graphics_launch(
    game_executable: String,
) -> Result<GraphicsLaunchInspection, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    inspect_graphics_for_executable(&registry, &PathBuf::from(game_executable))
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
pub fn inspect_managed_graphics_stack(
    game_executable: String,
) -> Result<ManagedStackStatus, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
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
    game_executable: String,
    api_override: String,
) -> Result<Dlss5RouteScan, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    managed_stack::inspect_routes(&registry, &PathBuf::from(game_executable), &api_override)
}

#[tauri::command]
pub fn install_managed_dlss5_route(
    game_executable: String,
    route: String,
    api_override: String,
    anti_cheat_acknowledged: bool,
) -> Result<Dlss5RouteScan, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    managed_stack::install_route(
        &registry,
        &PathBuf::from(game_executable),
        &route,
        &api_override,
        anti_cheat_acknowledged,
    )
}

#[tauri::command]
pub fn restore_dlss5_install(game_executable: String) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    managed_stack::restore_swapper(&registry, &PathBuf::from(game_executable))
}

#[tauri::command]
pub fn open_dlss5_swapper() -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
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
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn plugin_ui_routes() -> Result<Vec<PluginUiRouteSnapshot>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let routes = ui_routes(&registry);
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
pub fn plugin_capabilities(plugin_id: String) -> Result<PluginCapabilitiesSnapshot, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
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
pub fn set_plugin_enabled(
    id: String,
    enabled: bool,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    registry
        .set_enabled(&id, enabled)
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
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
    game_executable: String,
    graphics_decision: Option<GraphicsLaunchDecision>,
) -> Result<Option<PluginLaunchProgram>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let game_executable = PathBuf::from(game_executable);
    let inspection = inspect_graphics_for_executable(&registry, &game_executable)?;
    if !should_launch_hoyoshade(&inspection, graphics_decision)? {
        return Ok(None);
    }
    let Some(plugin) = registry
        .find(HOYOSHADE_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
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
    let wait_for_local_dxgi = if managed {
        match inspect_game_executable(&game_executable) {
            Dlss5State::Managed(state) => {
                managed_stack::prepare(&registry, &game_executable, &state)?;
                matches!(state.route, super::dlss5::Dlss5Route::OptiScaler)
            }
            Dlss5State::Broken { reason, .. } => return Err(reason),
            Dlss5State::NotManaged => {
                return Err("DLSS5 is not installed for this game".to_string())
            }
        }
    } else { false };
    let injector = dependency_path.join("inject.exe");
    Ok(Some(PluginLaunchProgram {
        path: injector.to_string_lossy().into_owned(),
        args: if wait_for_local_dxgi {
            format!("--ssmt-managed-config-after-dxgi {process_name}")
        } else if managed {
            format!("--ssmt-managed-config {process_name}")
        } else {
            process_name.to_string()
        },
        work_dir: dependency_path.to_string_lossy().into_owned(),
        run_as_administrator: true,
    }))
}

#[tauri::command]
pub fn install_plugin_package(
    archive_path: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_ssmtpkg(&mut registry, &PathBuf::from(archive_path))
        .map_err(|error| error.to_string())?;
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
        capabilities_for_permissions, lifecycle_status, should_launch_hoyoshade,
        Dlss5GameStateSnapshot, GraphicsLaunchDecision, GraphicsLaunchInspection,
        PluginLifecycleStatus,
    };
    use crate::plugin::graphics_stack::{
        CompatibilityLevel, GraphicsCompatibilityIssue, GraphicsStackResolution,
    };
    use crate::plugin::registry::{
        ExternalDependencyState, ExternalDependencyStatus, InstalledPlugin,
    };
    use crate::plugin::{
        PluginCompatibility, PluginContributions, PluginManifest, PluginPermission,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

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
