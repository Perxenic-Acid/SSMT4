use super::process_forwarder::{
    LaunchTemplateContext, ProcessForwarderError, ProcessForwarderSpec,
};
use super::registry::InstalledPlugin;
use super::{ExternalManagerContribution, PluginManifest};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalManagerIntegration {
    pub package_root: PathBuf,
    pub manifest: PluginManifest,
    pub contribution: ExternalManagerContribution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalManagerState {
    pub executable: PathBuf,
    pub executable_ready: bool,
    pub game_manifest: Option<serde_json::Value>,
    pub route: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalManagerError {
    MissingContribution(String),
    InvalidExecutable(PathBuf),
    InvalidGameDirectory(PathBuf),
    ManifestRead(String),
    ManifestInvalid(String),
    Forwarder(ProcessForwarderError),
}

impl std::fmt::Display for ExternalManagerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContribution(id) => write!(formatter, "external manager not found: {id}"),
            Self::InvalidExecutable(path) => write!(
                formatter,
                "external manager executable is invalid: {}",
                path.display()
            ),
            Self::InvalidGameDirectory(path) => {
                write!(formatter, "game directory is invalid: {}", path.display())
            }
            Self::ManifestRead(error) => write!(
                formatter,
                "failed to read external manager manifest: {error}"
            ),
            Self::ManifestInvalid(error) => {
                write!(formatter, "invalid external manager manifest: {error}")
            }
            Self::Forwarder(error) => write!(formatter, "external manager launch error: {error}"),
        }
    }
}

impl std::error::Error for ExternalManagerError {}

impl From<ProcessForwarderError> for ExternalManagerError {
    fn from(error: ProcessForwarderError) -> Self {
        Self::Forwarder(error)
    }
}

impl ExternalManagerIntegration {
    pub fn from_installed(
        plugin: &InstalledPlugin,
        manager_id: &str,
    ) -> Result<Self, ExternalManagerError> {
        let contribution = plugin
            .manifest
            .contributions
            .external_managers
            .iter()
            .find(|manager| manager.id == manager_id)
            .cloned()
            .ok_or_else(|| ExternalManagerError::MissingContribution(manager_id.to_string()))?;
        Ok(Self {
            package_root: plugin.package_root.clone(),
            manifest: plugin.manifest.clone(),
            contribution,
        })
    }

    pub fn build_process_spec(
        &self,
        plugin: &InstalledPlugin,
        context: &LaunchTemplateContext,
        timeout: Option<Duration>,
    ) -> Result<ProcessForwarderSpec, ExternalManagerError> {
        Ok(ProcessForwarderSpec::resolve(
            plugin,
            &super::LauncherAdapterContribution {
                kind: super::LauncherAdapterType::ProcessForwarder,
                executable: self.contribution.executable.clone(),
                arguments: Vec::new(),
                working_directory: self.contribution.working_directory.clone(),
                ready: None,
                failure_policy: super::FailurePolicy::Abort,
            },
            context,
            timeout,
        )?)
    }

    pub fn inspect_game_directory(
        &self,
        executable: &Path,
        game_directory: &Path,
        manifest_relative_path: &str,
    ) -> Result<ExternalManagerState, ExternalManagerError> {
        if !executable.is_file() {
            return Err(ExternalManagerError::InvalidExecutable(
                executable.to_path_buf(),
            ));
        }
        if !game_directory.is_dir() {
            return Err(ExternalManagerError::InvalidGameDirectory(
                game_directory.to_path_buf(),
            ));
        }
        super::validate_package_relative_path(manifest_relative_path)
            .map_err(|error| ExternalManagerError::ManifestInvalid(error.to_string()))?;
        let manifest_path = game_directory.join(manifest_relative_path);
        if !manifest_path.is_file() {
            return Ok(ExternalManagerState {
                executable: executable.to_path_buf(),
                executable_ready: true,
                game_manifest: None,
                route: None,
                reason: Some("manager manifest is not installed".to_string()),
            });
        }
        let raw = fs::read_to_string(&manifest_path)
            .map_err(|error| ExternalManagerError::ManifestRead(error.to_string()))?;
        let value = serde_json::from_str::<serde_json::Value>(&raw)
            .map_err(|error| ExternalManagerError::ManifestInvalid(error.to_string()))?;
        let route = value
            .get("route")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        Ok(ExternalManagerState {
            executable: executable.to_path_buf(),
            executable_ready: true,
            game_manifest: Some(value),
            route,
            reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::InstalledPlugin;
    use crate::plugin::{
        ExternalManagerContribution, PluginCompatibility, PluginContributions, PluginManifest,
    };
    use std::collections::BTreeMap;

    fn fixture(root: &Path) -> InstalledPlugin {
        InstalledPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                id: "ssmt.dlss5.integration".to_string(),
                name: "DLSS 5 Swapper Integration".to_string(),
                version: "0.1.0".to_string(),
                author: "SSMT".to_string(),
                compatibility: PluginCompatibility {
                    ssmt: ">=4.0.0".to_string(),
                    platforms: vec!["windows-x64".to_string()],
                },
                contributions: PluginContributions {
                    external_managers: vec![ExternalManagerContribution {
                        id: "dlss5-swapper".to_string(),
                        executable: "${plugin.root}/DLSS5-Swapper.exe".to_string(),
                        working_directory: Some("${plugin.root}".to_string()),
                    }],
                    ..Default::default()
                },
                external_dependencies: Vec::new(),
                permissions: Vec::new(),
            },
            package_root: root.join("package"),
            enabled: true,
            external_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn reads_route_from_game_manifest_without_automating_manager_ui() {
        let root = std::env::temp_dir().join(format!("ssmt-manager-test-{}", std::process::id()));
        let game = root.join("game");
        let package = root.join("package");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&package).unwrap();
        let executable = root.join("DLSS5-Swapper.exe");
        fs::write(&executable, b"fixture").unwrap();
        let plugin = fixture(&root);
        let integration =
            ExternalManagerIntegration::from_installed(&plugin, "dlss5-swapper").unwrap();
        let missing = integration
            .inspect_game_directory(&executable, &game, "dlss5/manifest.json")
            .unwrap();
        assert_eq!(missing.route, None);
        fs::create_dir_all(game.join("dlss5")).unwrap();
        fs::write(game.join("dlss5/manifest.json"), r#"{"route":"feeder"}"#).unwrap();
        let ready = integration
            .inspect_game_directory(&executable, &game, "dlss5/manifest.json")
            .unwrap();
        assert_eq!(ready.route.as_deref(), Some("feeder"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_external_manager_executable() {
        let root =
            std::env::temp_dir().join(format!("ssmt-manager-missing-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let plugin = fixture(&root);
        let integration =
            ExternalManagerIntegration::from_installed(&plugin, "dlss5-swapper").unwrap();
        assert!(matches!(
            integration.inspect_game_directory(&root.join("missing.exe"), &root, "manifest.json"),
            Err(ExternalManagerError::InvalidExecutable(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
