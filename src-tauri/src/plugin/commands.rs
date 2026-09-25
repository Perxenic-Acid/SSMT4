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
pub fn install_plugin_package(
    archive_path: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_ssmtpkg(&mut registry, &PathBuf::from(archive_path))
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[cfg(test)]
mod tests {
    use super::capabilities_for_permissions;
    use crate::plugin::PluginPermission;

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
}
