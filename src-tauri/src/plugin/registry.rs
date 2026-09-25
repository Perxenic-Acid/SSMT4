use super::{ExternalDependency, ExternalDependencyType, PluginManifest, PluginManifestError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST_FILE_NAME: &str = "ssmt-plugin.json";
const STATE_FILE_NAME: &str = "registry-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalDependencyStatus {
    Missing,
    Invalid,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDependencyState {
    pub status: ExternalDependencyStatus,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl ExternalDependencyState {
    fn missing(reason: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            status: ExternalDependencyStatus::Missing,
            path,
            reason: Some(reason.into()),
        }
    }

    fn invalid(reason: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            status: ExternalDependencyStatus::Invalid,
            path,
            reason: Some(reason.into()),
        }
    }

    fn ready(path: PathBuf) -> Self {
        Self {
            status: ExternalDependencyStatus::Ready,
            path: Some(path),
            reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub package_root: PathBuf,
    pub enabled: bool,
    pub external_dependencies: BTreeMap<String, ExternalDependencyState>,
}

impl InstalledPlugin {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn version(&self) -> &str {
        &self.manifest.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginHostConfig {
    pub plugin_directory: PathBuf,
    pub plugins: Vec<String>,
}

#[derive(Debug)]
pub enum PluginRegistryError {
    Io(io::Error),
    Manifest(PluginManifestError),
    State(String),
    InvalidPackage(String),
    PackageAlreadyInstalled {
        id: String,
        version: String,
    },
    PluginNotFound(String),
    DependencyNotFound {
        plugin_id: String,
        dependency_id: String,
    },
}

impl std::fmt::Display for PluginRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "plugin registry I/O error: {error}"),
            Self::Manifest(error) => write!(formatter, "plugin manifest error: {error}"),
            Self::State(error) => write!(formatter, "plugin registry state error: {error}"),
            Self::InvalidPackage(error) => write!(formatter, "invalid plugin package: {error}"),
            Self::PackageAlreadyInstalled { id, version } => {
                write!(
                    formatter,
                    "plugin package already installed: {id} {version}"
                )
            }
            Self::PluginNotFound(id) => write!(formatter, "plugin not found: {id}"),
            Self::DependencyNotFound {
                plugin_id,
                dependency_id,
            } => write!(
                formatter,
                "external dependency not found: {plugin_id}/{dependency_id}"
            ),
        }
    }
}

impl std::error::Error for PluginRegistryError {}

impl From<io::Error> for PluginRegistryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<PluginManifestError> for PluginRegistryError {
    fn from(error: PluginManifestError) -> Self {
        Self::Manifest(error)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryState {
    #[serde(default)]
    plugins: BTreeMap<String, PluginSettings>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginSettings {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    external_paths: BTreeMap<String, PathBuf>,
}

pub struct PluginRegistry {
    plugins_root: PathBuf,
    state_path: PathBuf,
    state: RegistryState,
    installed: Vec<InstalledPlugin>,
}

impl PluginRegistry {
    pub fn from_default_location() -> Result<Self, PluginRegistryError> {
        Self::new(crate::config::path_manager::PathManager::ssmt_plugins_folder())
    }

    pub fn new(plugins_root: impl Into<PathBuf>) -> Result<Self, PluginRegistryError> {
        let plugins_root = plugins_root.into();
        fs::create_dir_all(&plugins_root)?;
        let state_path = plugins_root.join(STATE_FILE_NAME);
        let state = read_state(&state_path)?;
        let mut registry = Self {
            plugins_root,
            state_path,
            state,
            installed: Vec::new(),
        };
        registry.refresh()?;
        Ok(registry)
    }

    pub fn plugins_root(&self) -> &Path {
        &self.plugins_root
    }

    pub fn refresh(&mut self) -> Result<(), PluginRegistryError> {
        self.installed.clear();
        let mut packages = Vec::new();
        for id_entry in fs::read_dir(&self.plugins_root)? {
            let id_entry = id_entry?;
            let id_path = id_entry.path();
            if !id_entry.file_type()?.is_dir() {
                continue;
            }
            for version_entry in fs::read_dir(&id_path)? {
                let version_entry = version_entry?;
                let package_root = version_entry.path();
                if !version_entry.file_type()?.is_dir() {
                    continue;
                }
                packages.push((id_path.clone(), package_root));
            }
        }
        packages.sort_by(|left, right| left.1.cmp(&right.1));

        for (id_path, package_root) in packages {
            let manifest_path = package_root.join(MANIFEST_FILE_NAME);
            if !manifest_path.is_file() {
                return Err(PluginRegistryError::InvalidPackage(format!(
                    "missing manifest: {}",
                    manifest_path.display()
                )));
            }
            let raw = fs::read_to_string(&manifest_path)?;
            let manifest = PluginManifest::from_json_str(&raw)?;
            let directory_id = id_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    PluginRegistryError::InvalidPackage("invalid plugin directory name".to_string())
                })?;
            let directory_version = package_root
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    PluginRegistryError::InvalidPackage(
                        "invalid plugin version directory name".to_string(),
                    )
                })?;
            if directory_id != manifest.id || directory_version != manifest.version {
                return Err(PluginRegistryError::InvalidPackage(format!(
                    "directory does not match manifest: {}",
                    package_root.display()
                )));
            }

            let key = package_key(&manifest.id, &manifest.version);
            let settings = self.state.plugins.get(&key).cloned().unwrap_or_default();
            let external_dependencies = manifest
                .external_dependencies
                .iter()
                .map(|dependency| {
                    let configured_path = settings.external_paths.get(&dependency.id).cloned();
                    (
                        dependency.id.clone(),
                        evaluate_dependency(dependency, configured_path),
                    )
                })
                .collect();
            self.installed.push(InstalledPlugin {
                manifest,
                package_root,
                enabled: settings.enabled,
                external_dependencies,
            });
        }
        Ok(())
    }

    pub fn installed(&self) -> &[InstalledPlugin] {
        &self.installed
    }

    pub fn find(&self, id: &str) -> Option<&InstalledPlugin> {
        self.installed
            .iter()
            .filter(|plugin| plugin.id() == id)
            .max_by(|left, right| {
                semver::Version::parse(left.version())
                    .expect("validated plugin version")
                    .cmp(
                        &semver::Version::parse(right.version()).expect("validated plugin version"),
                    )
            })
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), PluginRegistryError> {
        let plugin = self
            .find(id)
            .ok_or_else(|| PluginRegistryError::PluginNotFound(id.to_string()))?;
        let key = package_key(plugin.id(), plugin.version());
        self.state.plugins.entry(key).or_default().enabled = enabled;
        write_state(&self.state_path, &self.state)?;
        self.refresh()
    }

    pub fn set_external_dependency_path(
        &mut self,
        id: &str,
        dependency_id: &str,
        path: impl Into<PathBuf>,
    ) -> Result<(), PluginRegistryError> {
        let plugin = self
            .find(id)
            .ok_or_else(|| PluginRegistryError::PluginNotFound(id.to_string()))?;
        if !plugin
            .manifest
            .external_dependencies
            .iter()
            .any(|dependency| dependency.id == dependency_id)
        {
            return Err(PluginRegistryError::DependencyNotFound {
                plugin_id: id.to_string(),
                dependency_id: dependency_id.to_string(),
            });
        }
        let key = package_key(plugin.id(), plugin.version());
        self.state
            .plugins
            .entry(key)
            .or_default()
            .external_paths
            .insert(dependency_id.to_string(), path.into());
        write_state(&self.state_path, &self.state)?;
        self.refresh()
    }

    pub fn generate_plugin_host_config(&self) -> PluginHostConfig {
        let mut plugins = self
            .installed
            .iter()
            .filter(|plugin| plugin.enabled)
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .contributions
                    .runtime_plugins
                    .iter()
                    .map(|runtime| {
                        plugin
                            .package_root
                            .join(&runtime.path)
                            .strip_prefix(&self.plugins_root)
                            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_else(|_| {
                                plugin
                                    .package_root
                                    .join(&runtime.path)
                                    .to_string_lossy()
                                    .replace('\\', "/")
                            })
                    })
            })
            .collect::<Vec<_>>();
        plugins.sort();
        PluginHostConfig {
            plugin_directory: self.plugins_root.clone(),
            plugins,
        }
    }

    pub fn install_directory_package(
        &mut self,
        source: &Path,
    ) -> Result<InstalledPlugin, PluginRegistryError> {
        let manifest_path = source.join(MANIFEST_FILE_NAME);
        let raw = fs::read_to_string(&manifest_path).map_err(|error| {
            PluginRegistryError::InvalidPackage(format!(
                "cannot read {}: {error}",
                manifest_path.display()
            ))
        })?;
        let manifest = PluginManifest::from_json_str(&raw)?;
        let destination = self.plugins_root.join(&manifest.id).join(&manifest.version);
        if destination.exists() {
            return Err(PluginRegistryError::PackageAlreadyInstalled {
                id: manifest.id,
                version: manifest.version,
            });
        }
        let parent = destination.parent().ok_or_else(|| {
            PluginRegistryError::InvalidPackage("package destination has no parent".to_string())
        })?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".{}.install-{}", manifest.version, unique_suffix()));
        if let Err(error) = copy_package_tree(source, &temporary) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error.into());
        }
        self.refresh()?;
        self.installed
            .iter()
            .find(|plugin| plugin.id() == manifest.id && plugin.version() == manifest.version)
            .cloned()
            .ok_or_else(|| {
                PluginRegistryError::InvalidPackage(
                    "installed package not found after refresh".to_string(),
                )
            })
    }
}

fn package_key(id: &str, version: &str) -> String {
    format!("{id}@{version}")
}

fn read_state(path: &Path) -> Result<RegistryState, PluginRegistryError> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| PluginRegistryError::State(error.to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RegistryState::default()),
        Err(error) => Err(error.into()),
    }
}

fn write_state(path: &Path, state: &RegistryState) -> Result<(), PluginRegistryError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| PluginRegistryError::State(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn evaluate_dependency(
    dependency: &ExternalDependency,
    configured_path: Option<PathBuf>,
) -> ExternalDependencyState {
    let Some(path) = configured_path else {
        return ExternalDependencyState::missing("external path is not configured", None);
    };
    if !path.exists() {
        return ExternalDependencyState::missing("configured directory does not exist", Some(path));
    }
    if !path.is_dir() || !matches!(dependency.kind, ExternalDependencyType::Directory) {
        return ExternalDependencyState::invalid("configured path is not a directory", Some(path));
    }
    for required_file in &dependency.required_files {
        let required_path = path.join(required_file);
        if !required_path.exists() {
            return ExternalDependencyState::invalid(
                format!("missing required file: {required_file}"),
                Some(path),
            );
        }
    }
    ExternalDependencyState::ready(path)
}

fn copy_package_tree(source: &Path, destination: &Path) -> Result<(), PluginRegistryError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginRegistryError::InvalidPackage(format!(
            "package source is not a normal directory: {}",
            source.display()
        )));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(PluginRegistryError::InvalidPackage(format!(
                "package contains symlink: {}",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            copy_package_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            return Err(PluginRegistryError::InvalidPackage(format!(
                "package contains unsupported file: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ssmt-plugin-registry-{label}-{}", unique_suffix()))
    }

    fn fixture_package(root: &Path, id: &str, version: &str) -> PathBuf {
        let package = root.join("source");
        fs::create_dir_all(package.join("native")).unwrap();
        fs::write(package.join("native/example.dll"), b"fixture").unwrap();
        fs::write(
            package.join(MANIFEST_FILE_NAME),
            format!(
                r#"{{
                  "schemaVersion": 1,
                  "id": "{id}",
                  "name": "Fixture",
                  "version": "{version}",
                  "author": "Test",
                  "compatibility": {{"ssmt": ">=4.0.0", "platforms": ["windows-x64"]}},
                  "contributions": {{"runtimePlugins": [{{"path": "native/example.dll"}}]}},
                  "externalDependencies": [],
                  "permissions": []
                }}"#
            ),
        )
        .unwrap();
        package
    }

    #[test]
    fn install_discover_enable_disable_and_restore_state() {
        let root = temp_root("state");
        let source = fixture_package(&root, "ssmt.fixture", "1.0.0");
        let plugins_root = root.join("Plugins");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        registry.install_directory_package(&source).unwrap();
        assert!(!registry.find("ssmt.fixture").unwrap().enabled);
        registry.set_enabled("ssmt.fixture", true).unwrap();
        assert!(registry.find("ssmt.fixture").unwrap().enabled);
        drop(registry);

        let mut restarted = PluginRegistry::new(&plugins_root).unwrap();
        assert!(restarted.find("ssmt.fixture").unwrap().enabled);
        restarted.set_enabled("ssmt.fixture", false).unwrap();
        assert!(!restarted.find("ssmt.fixture").unwrap().enabled);
        drop(restarted);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_external_path_and_generates_runtime_plugin_list() {
        let root = temp_root("external");
        let source = fixture_package(&root, "ssmt.external", "1.0.0");
        let raw = fs::read_to_string(source.join(MANIFEST_FILE_NAME)).unwrap().replace(
            r#""externalDependencies": []"#,
            r#""externalDependencies": [{"id":"tool","type":"directory","requiredFiles":["tool.exe"]}]"#,
        );
        fs::write(source.join(MANIFEST_FILE_NAME), raw).unwrap();
        let plugins_root = root.join("Plugins");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        registry.install_directory_package(&source).unwrap();
        registry.set_enabled("ssmt.external", true).unwrap();
        assert_eq!(
            registry
                .find("ssmt.external")
                .unwrap()
                .external_dependencies["tool"]
                .status,
            ExternalDependencyStatus::Missing
        );

        let external = root.join("external");
        fs::create_dir_all(&external).unwrap();
        registry
            .set_external_dependency_path("ssmt.external", "tool", &external)
            .unwrap();
        assert_eq!(
            registry
                .find("ssmt.external")
                .unwrap()
                .external_dependencies["tool"]
                .status,
            ExternalDependencyStatus::Invalid
        );
        fs::write(external.join("tool.exe"), b"fixture").unwrap();
        registry.refresh().unwrap();
        assert_eq!(
            registry
                .find("ssmt.external")
                .unwrap()
                .external_dependencies["tool"]
                .status,
            ExternalDependencyStatus::Ready
        );

        let config = registry.generate_plugin_host_config();
        assert_eq!(config.plugin_directory, plugins_root);
        assert_eq!(
            config.plugins,
            vec!["ssmt.external/1.0.0/native/example.dll"]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
