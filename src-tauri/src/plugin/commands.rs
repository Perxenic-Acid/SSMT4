use super::package_installer::install_ssmtpkg;
use super::registry::{ExternalDependencyState, PluginRegistry};
use super::PluginManifest;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginSnapshot {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub external_dependencies: BTreeMap<String, ExternalDependencyState>,
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

#[tauri::command]
pub fn plugin_registry_snapshot() -> Result<Vec<InstalledPluginSnapshot>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
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
