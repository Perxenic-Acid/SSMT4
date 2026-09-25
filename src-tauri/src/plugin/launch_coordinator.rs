use super::registry::{
    ExternalDependencyStatus, InstalledPlugin, PluginHostConfig, PluginRegistry,
    PluginRegistryError,
};
use super::PluginPermission;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchRequest {
    pub run_executable: PathBuf,
    #[serde(default)]
    pub game_executable: Option<PathBuf>,
    #[serde(default)]
    pub game_working_directory: Option<PathBuf>,
    #[serde(default)]
    pub game_arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlugin {
    pub id: String,
    pub version: String,
    pub package_root: PathBuf,
    pub runtime_plugins: Vec<PathBuf>,
    pub launcher_adapter_count: usize,
}

impl ResolvedPlugin {
    fn from_installed(plugin: &InstalledPlugin) -> Self {
        Self {
            id: plugin.manifest.id.clone(),
            version: plugin.manifest.version.clone(),
            package_root: plugin.package_root.clone(),
            runtime_plugins: plugin
                .manifest
                .contributions
                .runtime_plugins
                .iter()
                .map(|runtime| plugin.package_root.join(&runtime.path))
                .collect(),
            launcher_adapter_count: plugin.manifest.contributions.launcher_adapters.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub request: LaunchRequest,
    pub enabled_plugins: Vec<ResolvedPlugin>,
    pub plugin_host_config: Option<PluginHostConfig>,
    pub launcher_adapter_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPhase {
    Created,
    ResolvingPlugins,
    Preflight,
    AdaptersPrepared,
    RunStarted,
    Observing,
    CoordinatingAdapters,
    Running,
    TargetExited,
    CleaningUp,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSession {
    pub plan: LaunchPlan,
    pub phase: LaunchPhase,
    pub failure: Option<String>,
}

impl LaunchSession {
    pub fn transition(&mut self, next: LaunchPhase) -> Result<(), LaunchCoordinatorError> {
        if !is_valid_transition(self.phase, next) {
            return Err(LaunchCoordinatorError::InvalidTransition {
                from: self.phase,
                to: next,
            });
        }
        self.phase = next;
        Ok(())
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.failure = Some(error.into());
        self.phase = LaunchPhase::Failed;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCoordinatorError {
    Registry(PluginRegistryErrorMessage),
    InvalidRequest(String),
    MissingExecutable {
        kind: &'static str,
        path: PathBuf,
    },
    MissingExternalDependency {
        plugin_id: String,
        dependency_id: String,
        status: ExternalDependencyStatus,
        reason: Option<String>,
    },
    MissingPermission {
        plugin_id: String,
        permission: PluginPermission,
    },
    InvalidTransition {
        from: LaunchPhase,
        to: LaunchPhase,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRegistryErrorMessage(pub String);

impl fmt::Display for LaunchCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => write!(formatter, "plugin registry error: {}", error.0),
            Self::InvalidRequest(error) => write!(formatter, "invalid launch request: {error}"),
            Self::MissingExecutable { kind, path } => {
                write!(
                    formatter,
                    "{kind} does not exist or is not a file: {}",
                    path.display()
                )
            }
            Self::MissingExternalDependency {
                plugin_id,
                dependency_id,
                status,
                reason,
            } => write!(
                formatter,
                "plugin {plugin_id} dependency {dependency_id} is {status:?}: {}",
                reason.as_deref().unwrap_or("no reason provided")
            ),
            Self::MissingPermission {
                plugin_id,
                permission,
            } => write!(
                formatter,
                "plugin {plugin_id} is missing permission {permission:?}"
            ),
            Self::InvalidTransition { from, to } => {
                write!(
                    formatter,
                    "invalid launch phase transition: {from:?} -> {to:?}"
                )
            }
        }
    }
}

impl std::error::Error for LaunchCoordinatorError {}

impl From<PluginRegistryError> for LaunchCoordinatorError {
    fn from(error: PluginRegistryError) -> Self {
        Self::Registry(PluginRegistryErrorMessage(error.to_string()))
    }
}

pub struct LaunchCoordinator<'a> {
    registry: &'a PluginRegistry,
}

impl<'a> LaunchCoordinator<'a> {
    pub fn new(registry: &'a PluginRegistry) -> Self {
        Self { registry }
    }

    pub fn prepare(&self, request: LaunchRequest) -> Result<LaunchSession, LaunchCoordinatorError> {
        validate_request(&request)?;

        let mut session = LaunchSession {
            plan: LaunchPlan {
                request,
                enabled_plugins: Vec::new(),
                plugin_host_config: None,
                launcher_adapter_count: 0,
            },
            phase: LaunchPhase::Created,
            failure: None,
        };
        session.transition(LaunchPhase::ResolvingPlugins)?;

        let enabled_plugins = self
            .registry
            .installed()
            .iter()
            .filter(|plugin| plugin.enabled)
            .map(|plugin| {
                validate_plugin_preflight(plugin)?;
                Ok(ResolvedPlugin::from_installed(plugin))
            })
            .collect::<Result<Vec<_>, LaunchCoordinatorError>>()?;
        session.plan.launcher_adapter_count = enabled_plugins
            .iter()
            .map(|plugin| plugin.launcher_adapter_count)
            .sum();
        session.plan.enabled_plugins = enabled_plugins;

        session.transition(LaunchPhase::Preflight)?;
        session.plan.plugin_host_config = enabled_plugins_host_config(self.registry);
        session.transition(LaunchPhase::AdaptersPrepared)?;
        Ok(session)
    }
}

fn validate_request(request: &LaunchRequest) -> Result<(), LaunchCoordinatorError> {
    if !request.run_executable.is_file() {
        return Err(LaunchCoordinatorError::MissingExecutable {
            kind: "Run.exe",
            path: request.run_executable.clone(),
        });
    }
    if let Some(game_executable) = &request.game_executable {
        if !game_executable.is_file() {
            return Err(LaunchCoordinatorError::MissingExecutable {
                kind: "game executable",
                path: game_executable.clone(),
            });
        }
    }
    if let Some(working_directory) = &request.game_working_directory {
        if !working_directory.is_dir() {
            return Err(LaunchCoordinatorError::InvalidRequest(format!(
                "game working directory does not exist or is not a directory: {}",
                working_directory.display()
            )));
        }
    }
    Ok(())
}

fn validate_plugin_preflight(plugin: &InstalledPlugin) -> Result<(), LaunchCoordinatorError> {
    for (dependency_id, dependency_state) in &plugin.external_dependencies {
        if dependency_state.status != ExternalDependencyStatus::Ready {
            return Err(LaunchCoordinatorError::MissingExternalDependency {
                plugin_id: plugin.manifest.id.clone(),
                dependency_id: dependency_id.clone(),
                status: dependency_state.status,
                reason: dependency_state.reason.clone(),
            });
        }
    }

    if !plugin.manifest.contributions.runtime_plugins.is_empty()
        && !has_permission(plugin, PluginPermission::NativeInject)
    {
        return Err(LaunchCoordinatorError::MissingPermission {
            plugin_id: plugin.manifest.id.clone(),
            permission: PluginPermission::NativeInject,
        });
    }

    if !plugin.manifest.contributions.launcher_adapters.is_empty() {
        for required in [
            PluginPermission::ProcessSpawn,
            PluginPermission::ProcessObserve,
        ] {
            if !has_permission(plugin, required.clone()) {
                return Err(LaunchCoordinatorError::MissingPermission {
                    plugin_id: plugin.manifest.id.clone(),
                    permission: required,
                });
            }
        }
    }
    Ok(())
}

fn has_permission(plugin: &InstalledPlugin, required: PluginPermission) -> bool {
    plugin
        .manifest
        .permissions
        .iter()
        .any(|permission| *permission == required)
}

fn enabled_plugins_host_config(registry: &PluginRegistry) -> Option<PluginHostConfig> {
    let config = registry.generate_plugin_host_config();
    (!config.plugins.is_empty()).then_some(config)
}

fn is_valid_transition(from: LaunchPhase, to: LaunchPhase) -> bool {
    matches!(
        (from, to),
        (LaunchPhase::Created, LaunchPhase::ResolvingPlugins)
            | (LaunchPhase::ResolvingPlugins, LaunchPhase::Preflight)
            | (LaunchPhase::Preflight, LaunchPhase::AdaptersPrepared)
            | (LaunchPhase::AdaptersPrepared, LaunchPhase::RunStarted)
            | (LaunchPhase::RunStarted, LaunchPhase::Observing)
            | (LaunchPhase::Observing, LaunchPhase::CoordinatingAdapters)
            | (LaunchPhase::CoordinatingAdapters, LaunchPhase::Running)
            | (LaunchPhase::Running, LaunchPhase::TargetExited)
            | (LaunchPhase::TargetExited, LaunchPhase::CleaningUp)
            | (LaunchPhase::CleaningUp, LaunchPhase::Completed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::PluginRegistry;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-launch-coordinator-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn request(root: &PathBuf) -> LaunchRequest {
        let run = root.join("Run.exe");
        let game = root.join("Game.exe");
        fs::create_dir_all(root).unwrap();
        fs::write(&run, b"run").unwrap();
        fs::write(&game, b"game").unwrap();
        LaunchRequest {
            run_executable: run,
            game_executable: Some(game),
            game_working_directory: Some(root.clone()),
            game_arguments: vec!["--test".to_string()],
        }
    }

    fn write_package(root: &PathBuf, id: &str, permissions: &[&str]) {
        let package = root.join("Plugins").join(id).join("1.0.0");
        fs::create_dir_all(package.join("native")).unwrap();
        fs::write(package.join("native/plugin.dll"), b"plugin").unwrap();
        let permissions = permissions
            .iter()
            .map(|permission| format!(r#""{permission}""#))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            package.join("ssmt-plugin.json"),
            format!(
                r#"{{
                  "schemaVersion": 1,
                  "id": "{id}",
                  "name": "Test",
                  "version": "1.0.0",
                  "author": "Test",
                  "compatibility": {{"ssmt": ">=4.0.0", "platforms": ["windows-x64"]}},
                  "contributions": {{"runtimePlugins": [{{"path": "native/plugin.dll"}}]}},
                  "permissions": [{permissions}]
                }}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn prepares_enabled_plugins_and_runs_phase_machine() {
        let root = temp_root("valid");
        write_package(&root, "ssmt.runtime", &["native.inject"]);
        let mut registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        registry.set_enabled("ssmt.runtime", true).unwrap();
        let coordinator = LaunchCoordinator::new(&registry);
        let mut session = coordinator.prepare(request(&root)).unwrap();
        assert_eq!(session.phase, LaunchPhase::AdaptersPrepared);
        assert!(session.plan.plugin_host_config.is_some());
        for phase in [
            LaunchPhase::RunStarted,
            LaunchPhase::Observing,
            LaunchPhase::CoordinatingAdapters,
            LaunchPhase::Running,
            LaunchPhase::TargetExited,
            LaunchPhase::CleaningUp,
            LaunchPhase::Completed,
        ] {
            session.transition(phase).unwrap();
        }
        assert_eq!(session.phase, LaunchPhase::Completed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_enabled_runtime_plugin_without_native_permission() {
        let root = temp_root("permission");
        write_package(&root, "ssmt.runtime", &[]);
        let mut registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        registry.set_enabled("ssmt.runtime", true).unwrap();
        let error = LaunchCoordinator::new(&registry)
            .prepare(request(&root))
            .unwrap_err();
        assert!(matches!(
            error,
            LaunchCoordinatorError::MissingPermission { .. }
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_phase_transition() {
        let root = temp_root("transition");
        let registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        let mut session = LaunchCoordinator::new(&registry)
            .prepare(request(&root))
            .unwrap();
        assert!(matches!(
            session.transition(LaunchPhase::Running),
            Err(LaunchCoordinatorError::InvalidTransition { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
