use super::package_installer::install_ssmtpkg;
use super::registry::{ExternalDependencyState, PluginRegistry};
use super::PluginManifest;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginSnapshot {
    pub manifest: PluginManifest,
    pub enabled: bool,
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

fn snapshot(registry: &PluginRegistry) -> Vec<InstalledPluginSnapshot> {
    registry
        .installed()
        .iter()
        .map(|plugin| InstalledPluginSnapshot {
            manifest: plugin.manifest.clone(),
            enabled: plugin.enabled,
            external_dependencies: plugin.external_dependencies.clone(),
        })
        .collect()
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
pub fn install_plugin_package(
    archive_path: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_ssmtpkg(&mut registry, &PathBuf::from(archive_path))
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}
