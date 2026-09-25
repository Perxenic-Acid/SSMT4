use super::process_forwarder::{
    LaunchTemplateContext, ProcessForwarderError, ProcessForwarderSpec,
};
use super::registry::{ExternalDependencyStatus, InstalledPlugin};
use super::{LauncherAdapterType, PluginManifest};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const HOYOSHADE_PLUGIN_ID: &str = "ssmt.hoyoshade.bridge";
pub const HOYOSHADE_DEPENDENCY_ID: &str = "hoyoshade";
pub const HOYOSHADE_READY_MARKER: &str = "HOYOSHADE_READY:9999";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoYoShadeBridge {
    pub package_root: PathBuf,
    pub manifest: PluginManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoYoShadeBridgeError {
    InvalidManifest(String),
    WrongPluginId(String),
    MissingAdapter,
    MissingDependency,
    DependencyNotReady(String),
    MissingRequiredFile(PathBuf),
    Forwarder(ProcessForwarderError),
}

impl std::fmt::Display for HoYoShadeBridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(error) => {
                write!(formatter, "invalid HoYoShade manifest: {error}")
            }
            Self::WrongPluginId(id) => write!(formatter, "unexpected HoYoShade plugin id: {id}"),
            Self::MissingAdapter => write!(formatter, "HoYoShade Bridge has no launcher adapter"),
            Self::MissingDependency => {
                write!(formatter, "HoYoShade Bridge has no hoyoshade dependency")
            }
            Self::DependencyNotReady(reason) => {
                write!(formatter, "HoYoShade dependency is not ready: {reason}")
            }
            Self::MissingRequiredFile(path) => {
                write!(formatter, "HoYoShade file is missing: {}", path.display())
            }
            Self::Forwarder(error) => write!(formatter, "HoYoShade launcher error: {error}"),
        }
    }
}

impl std::error::Error for HoYoShadeBridgeError {}

impl From<ProcessForwarderError> for HoYoShadeBridgeError {
    fn from(error: ProcessForwarderError) -> Self {
        Self::Forwarder(error)
    }
}

impl HoYoShadeBridge {
    pub fn from_installed(plugin: &InstalledPlugin) -> Result<Self, HoYoShadeBridgeError> {
        if plugin.manifest.id != HOYOSHADE_PLUGIN_ID {
            return Err(HoYoShadeBridgeError::WrongPluginId(
                plugin.manifest.id.clone(),
            ));
        }
        if plugin.manifest.contributions.launcher_adapters.len() != 1 {
            return Err(HoYoShadeBridgeError::MissingAdapter);
        }
        if !plugin
            .manifest
            .external_dependencies
            .iter()
            .any(|dependency| dependency.id == HOYOSHADE_DEPENDENCY_ID)
        {
            return Err(HoYoShadeBridgeError::MissingDependency);
        }
        Ok(Self {
            package_root: plugin.package_root.clone(),
            manifest: plugin.manifest.clone(),
        })
    }

    pub fn validate_external_directory(&self, path: &Path) -> Result<(), HoYoShadeBridgeError> {
        if !path.is_dir() {
            return Err(HoYoShadeBridgeError::DependencyNotReady(
                "configured path is not a directory".to_string(),
            ));
        }
        let dependency = self
            .manifest
            .external_dependencies
            .iter()
            .find(|dependency| dependency.id == HOYOSHADE_DEPENDENCY_ID)
            .ok_or(HoYoShadeBridgeError::MissingDependency)?;
        for required_file in &dependency.required_files {
            let required = path.join(required_file);
            if !required.exists() {
                return Err(HoYoShadeBridgeError::MissingRequiredFile(required));
            }
        }
        Ok(())
    }

    pub fn build_forwarder(
        &self,
        plugin: &InstalledPlugin,
        context: &LaunchTemplateContext,
        timeout: Duration,
    ) -> Result<ProcessForwarderSpec, HoYoShadeBridgeError> {
        let state = plugin
            .external_dependencies
            .get(HOYOSHADE_DEPENDENCY_ID)
            .ok_or(HoYoShadeBridgeError::MissingDependency)?;
        if state.status != ExternalDependencyStatus::Ready {
            return Err(HoYoShadeBridgeError::DependencyNotReady(
                state
                    .reason
                    .clone()
                    .unwrap_or_else(|| format!("status: {:?}", state.status)),
            ));
        }
        let adapter = self
            .manifest
            .contributions
            .launcher_adapters
            .first()
            .ok_or(HoYoShadeBridgeError::MissingAdapter)?;
        if adapter.kind != LauncherAdapterType::ProcessForwarder {
            return Err(HoYoShadeBridgeError::MissingAdapter);
        }
        Ok(ProcessForwarderSpec::resolve(
            plugin,
            adapter,
            context,
            Some(timeout),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::ExternalDependencyState;
    use crate::plugin::{
        ExternalDependency, ExternalDependencyType, FailurePolicy, LauncherAdapterContribution,
        PluginCompatibility, PluginContributions, PluginPermission,
    };
    use std::collections::BTreeMap;
    use std::fs;

    fn fixture(root: &Path) -> InstalledPlugin {
        InstalledPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                id: HOYOSHADE_PLUGIN_ID.to_string(),
                name: "HoYoShade Bridge".to_string(),
                version: "0.1.0".to_string(),
                author: "SSMT".to_string(),
                compatibility: PluginCompatibility {
                    ssmt: ">=4.0.0".to_string(),
                    platforms: vec!["windows-x64".to_string()],
                    games: Vec::new(),
                },
                contributions: PluginContributions {
                    launcher_adapters: vec![LauncherAdapterContribution {
                        kind: LauncherAdapterType::ProcessForwarder,
                        executable: "${external.hoyoshade}/inject.exe".to_string(),
                        arguments: vec!["${game.processName}".to_string()],
                        working_directory: Some("${external.hoyoshade}".to_string()),
                        ready: Some(super::super::ReadyCondition {
                            kind: super::super::ReadyConditionType::StderrContains,
                            value: Some(HOYOSHADE_READY_MARKER.to_string()),
                            timeout_ms: 15_000,
                        }),
                        failure_policy: FailurePolicy::Abort,
                    }],
                    ..Default::default()
                },
                external_dependencies: vec![ExternalDependency {
                    id: HOYOSHADE_DEPENDENCY_ID.to_string(),
                    kind: ExternalDependencyType::Directory,
                    required_files: vec!["inject.exe".to_string(), "ReShade64.dll".to_string()],
                }],
                permissions: vec![
                    PluginPermission::ProcessSpawn,
                    PluginPermission::ProcessObserve,
                ],
            },
            package_root: root.join("package"),
            enabled: true,
            external_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn validates_real_hoyoshade_dependency_files() {
        let root = std::env::temp_dir().join(format!("ssmt-hoyoshade-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("inject.exe"), b"fixture").unwrap();
        let bridge = HoYoShadeBridge::from_installed(&fixture(&root)).unwrap();
        assert!(matches!(
            bridge.validate_external_directory(&root),
            Err(HoYoShadeBridgeError::MissingRequiredFile(_))
        ));
        fs::write(root.join("ReShade64.dll"), b"fixture").unwrap();
        bridge.validate_external_directory(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unready_bridge_dependency() {
        let root =
            std::env::temp_dir().join(format!("ssmt-hoyoshade-unready-{}", std::process::id()));
        let mut plugin = fixture(&root);
        plugin.external_dependencies.insert(
            HOYOSHADE_DEPENDENCY_ID.to_string(),
            ExternalDependencyState {
                status: ExternalDependencyStatus::Missing,
                path: None,
                reason: Some("not configured".to_string()),
            },
        );
        let bridge = HoYoShadeBridge::from_installed(&plugin).unwrap();
        let context = LaunchTemplateContext {
            game_executable: root.join("YuanShen.exe"),
            game_directory: root.clone(),
            game_process_name: "YuanShen.exe".to_string(),
            external_dependencies: BTreeMap::new(),
        };
        assert!(matches!(
            bridge.build_forwarder(&plugin, &context, Duration::from_secs(1)),
            Err(HoYoShadeBridgeError::DependencyNotReady(_))
        ));
    }
}
