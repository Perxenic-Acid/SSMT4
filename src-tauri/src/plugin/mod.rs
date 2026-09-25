use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

pub mod external_manager;
pub mod hoyoshade;
pub mod launch_barrier;
pub mod launch_coordinator;
pub mod process_forwarder;
pub mod registry;

pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_PLATFORM: &str = "windows-x64";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub compatibility: PluginCompatibility,
    pub contributions: PluginContributions,
    #[serde(default)]
    pub external_dependencies: Vec<ExternalDependency>,
    #[serde(default)]
    pub permissions: Vec<PluginPermission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginCompatibility {
    pub ssmt: String,
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginContributions {
    #[serde(default)]
    pub runtime_plugins: Vec<RuntimePluginContribution>,
    #[serde(default)]
    pub launcher_adapters: Vec<LauncherAdapterContribution>,
    #[serde(default)]
    pub external_managers: Vec<ExternalManagerContribution>,
    #[serde(default)]
    pub ui_pages: Vec<UiPageContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalManagerContribution {
    pub id: String,
    pub executable: String,
    #[serde(default)]
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimePluginContribution {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LauncherAdapterContribution {
    #[serde(rename = "type")]
    pub kind: LauncherAdapterType,
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub ready: Option<ReadyCondition>,
    #[serde(default)]
    pub failure_policy: FailurePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadyCondition {
    #[serde(rename = "type")]
    pub kind: ReadyConditionType,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default = "default_ready_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadyConditionType {
    StderrContains,
    StdoutContains,
    ProcessExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailurePolicy {
    #[default]
    Abort,
    Warn,
    Ignore,
}

fn default_ready_timeout_ms() -> u64 {
    15_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LauncherAdapterType {
    ProcessForwarder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPageContribution {
    pub id: String,
    pub route: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalDependency {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ExternalDependencyType,
    #[serde(default)]
    pub required_files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalDependencyType {
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginPermission {
    #[serde(rename = "process.spawn")]
    ProcessSpawn,
    #[serde(rename = "process.observe")]
    ProcessObserve,
    #[serde(rename = "game.launch")]
    GameLaunch,
    #[serde(rename = "filesystem.read")]
    FilesystemRead,
    #[serde(rename = "filesystem.write")]
    FilesystemWrite,
    #[serde(rename = "native.inject")]
    NativeInject,
    #[serde(rename = "network")]
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginManifestError {
    Json(String),
    UnsupportedSchemaVersion(u32),
    InvalidPluginId(String),
    DuplicatePluginId(String),
    InvalidVersion(String),
    InvalidCompatibility(String),
    UnsupportedPlatform(String),
    EmptyField(&'static str),
    DuplicateValue { field: &'static str, value: String },
    InvalidPath { field: &'static str, path: String },
    InvalidContribution(String),
    InvalidExternalDependency(String),
    InvalidPermission(String),
}

impl fmt::Display for PluginManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid plugin manifest JSON: {error}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    formatter,
                    "unsupported plugin manifest schema version: {version}"
                )
            }
            Self::InvalidPluginId(id) => write!(formatter, "invalid plugin id: {id}"),
            Self::DuplicatePluginId(id) => write!(formatter, "duplicate plugin id: {id}"),
            Self::InvalidVersion(version) => write!(formatter, "invalid plugin version: {version}"),
            Self::InvalidCompatibility(version) => {
                write!(formatter, "invalid SSMT compatibility range: {version}")
            }
            Self::UnsupportedPlatform(platform) => {
                write!(formatter, "unsupported platform: {platform}")
            }
            Self::EmptyField(field) => write!(formatter, "{field} must not be empty"),
            Self::DuplicateValue { field, value } => {
                write!(formatter, "duplicate {field}: {value}")
            }
            Self::InvalidPath { field, path } => write!(formatter, "invalid {field} path: {path}"),
            Self::InvalidContribution(detail) => {
                write!(formatter, "invalid plugin contribution: {detail}")
            }
            Self::InvalidExternalDependency(detail) => {
                write!(formatter, "invalid external dependency: {detail}")
            }
            Self::InvalidPermission(permission) => {
                write!(formatter, "invalid permission: {permission}")
            }
        }
    }
}

impl std::error::Error for PluginManifestError {}

impl PluginManifest {
    pub fn from_json_str(raw: &str) -> Result<Self, PluginManifestError> {
        let manifest = serde_json::from_str::<Self>(raw)
            .map_err(|error| PluginManifestError::Json(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), PluginManifestError> {
        if self.schema_version != PLUGIN_MANIFEST_SCHEMA_VERSION {
            return Err(PluginManifestError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }

        validate_plugin_id(&self.id)?;
        validate_non_empty("name", &self.name)?;
        validate_non_empty("author", &self.author)?;
        Version::parse(&self.version)
            .map_err(|_| PluginManifestError::InvalidVersion(self.version.clone()))?;

        if self.compatibility.platforms.is_empty() {
            return Err(PluginManifestError::InvalidCompatibility(
                "at least one platform is required".to_string(),
            ));
        }
        VersionReq::parse(&normalize_version_requirement(&self.compatibility.ssmt)).map_err(
            |_| PluginManifestError::InvalidCompatibility(self.compatibility.ssmt.clone()),
        )?;

        let mut platforms = HashSet::new();
        for platform in &self.compatibility.platforms {
            if platform != SUPPORTED_PLATFORM {
                return Err(PluginManifestError::UnsupportedPlatform(platform.clone()));
            }
            if !platforms.insert(platform) {
                return Err(PluginManifestError::DuplicateValue {
                    field: "platform",
                    value: platform.clone(),
                });
            }
        }

        validate_contributions(&self.contributions, &self.external_dependencies)?;
        validate_external_dependencies(&self.external_dependencies)?;
        validate_permissions(&self.permissions)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginContribution {
    RuntimePlugin(RuntimePluginContribution),
    LauncherAdapter(LauncherAdapterContribution),
    UiPage(UiPageContribution),
}

pub fn validate_unique_plugin_ids<'a, I>(manifests: I) -> Result<(), PluginManifestError>
where
    I: IntoIterator<Item = &'a PluginManifest>,
{
    let mut ids = HashSet::new();
    for manifest in manifests {
        if !ids.insert(manifest.id.as_str()) {
            return Err(PluginManifestError::DuplicatePluginId(manifest.id.clone()));
        }
    }
    Ok(())
}

fn validate_contributions(
    contributions: &PluginContributions,
    dependencies: &[ExternalDependency],
) -> Result<(), PluginManifestError> {
    let dependency_ids: HashSet<&str> = dependencies
        .iter()
        .map(|dependency| dependency.id.as_str())
        .collect();

    let mut runtime_paths = HashSet::new();
    for runtime_plugin in &contributions.runtime_plugins {
        validate_package_path("runtime plugin", &runtime_plugin.path)?;
        if !runtime_paths.insert(runtime_plugin.path.replace('\\', "/")) {
            return Err(PluginManifestError::DuplicateValue {
                field: "runtime plugin path",
                value: runtime_plugin.path.clone(),
            });
        }
    }

    let mut routes = HashSet::new();
    for page in &contributions.ui_pages {
        validate_plugin_id(&page.id)?;
        validate_non_empty("UI page route", &page.route)?;
        validate_package_path("UI page", &page.path)?;
        if !routes.insert(page.route.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "UI page route",
                value: page.route.clone(),
            });
        }
    }

    for adapter in &contributions.launcher_adapters {
        validate_launcher_path("launcher executable", &adapter.executable, &dependency_ids)?;
        if let Some(working_directory) = &adapter.working_directory {
            validate_launcher_path(
                "launcher working directory",
                working_directory,
                &dependency_ids,
            )?;
        }
        for argument in &adapter.arguments {
            validate_template(argument, &dependency_ids)?;
        }
        if let Some(ready) = &adapter.ready {
            if ready.timeout_ms == 0 {
                return Err(PluginManifestError::InvalidContribution(
                    "ready timeoutMs must be greater than zero".to_string(),
                ));
            }
            if !matches!(ready.kind, ReadyConditionType::ProcessExited)
                && ready.value.as_deref().map_or(true, str::is_empty)
            {
                return Err(PluginManifestError::InvalidContribution(
                    "ready condition requires a non-empty value".to_string(),
                ));
            }
        }
    }

    let mut manager_ids = HashSet::new();
    for manager in &contributions.external_managers {
        validate_plugin_id(&manager.id)?;
        if !manager_ids.insert(manager.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "external manager id",
                value: manager.id.clone(),
            });
        }
        validate_launcher_path(
            "external manager executable",
            &manager.executable,
            &dependency_ids,
        )?;
        if let Some(working_directory) = &manager.working_directory {
            validate_launcher_path(
                "external manager working directory",
                working_directory,
                &dependency_ids,
            )?;
        }
    }
    Ok(())
}

fn validate_external_dependencies(
    dependencies: &[ExternalDependency],
) -> Result<(), PluginManifestError> {
    let mut ids = HashSet::new();
    for dependency in dependencies {
        validate_plugin_id(&dependency.id).map_err(|_| {
            PluginManifestError::InvalidExternalDependency(format!("invalid id: {}", dependency.id))
        })?;
        if !ids.insert(dependency.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "external dependency id",
                value: dependency.id.clone(),
            });
        }
        for required_file in &dependency.required_files {
            validate_package_path("external dependency required file", required_file).map_err(
                |_| {
                    PluginManifestError::InvalidExternalDependency(format!(
                        "invalid required file path: {required_file}"
                    ))
                },
            )?;
        }
    }
    Ok(())
}

fn validate_permissions(permissions: &[PluginPermission]) -> Result<(), PluginManifestError> {
    let mut seen = HashSet::new();
    for permission in permissions {
        if !seen.insert(permission) {
            return Err(PluginManifestError::InvalidPermission(format!(
                "duplicate permission: {permission:?}"
            )));
        }
    }
    Ok(())
}

fn validate_launcher_path(
    field: &'static str,
    value: &str,
    dependency_ids: &HashSet<&str>,
) -> Result<(), PluginManifestError> {
    validate_non_empty(field, value)?;
    validate_template(value, dependency_ids)?;

    if !value.contains("${") {
        validate_package_path(field, value)?;
    } else if let Some(package_path) = strip_known_template_prefix(value) {
        if !package_path.is_empty() {
            validate_package_path(field, package_path)?;
        }
    }
    Ok(())
}

fn validate_template(
    value: &str,
    dependency_ids: &HashSet<&str>,
) -> Result<(), PluginManifestError> {
    let mut remainder = value;
    while let Some(start) = remainder.find("${") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err(PluginManifestError::InvalidContribution(format!(
                "unterminated variable in {value}"
            )));
        };
        let variable = &after_start[..end];
        let valid = matches!(
            variable,
            "game.exe" | "game.directory" | "game.processName" | "plugin.root"
        ) || variable
            .strip_prefix("external.")
            .is_some_and(|id| dependency_ids.contains(id));
        if !valid {
            return Err(PluginManifestError::InvalidContribution(format!(
                "unsupported variable: ${{{variable}}}"
            )));
        }
        remainder = &after_start[end + 1..];
    }
    Ok(())
}

fn strip_known_template_prefix(value: &str) -> Option<&str> {
    if let Some(end) = value.find('}') {
        if value.starts_with("${plugin.root}") || value.starts_with("${external.") {
            return Some(value[end + 1..].trim_start_matches(['/', '\\']));
        }
    }
    None
}

fn validate_plugin_id(id: &str) -> Result<(), PluginManifestError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || id.starts_with('.')
        || id.ends_with('.')
    {
        return Err(PluginManifestError::InvalidPluginId(id.to_string()));
    }
    Ok(())
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), PluginManifestError> {
    if value.trim().is_empty() {
        return Err(PluginManifestError::EmptyField(field));
    }
    Ok(())
}

pub fn validate_package_relative_path(path: &str) -> Result<(), PluginManifestError> {
    validate_package_path("package", path)
}

fn validate_package_path(field: &'static str, path: &str) -> Result<(), PluginManifestError> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(1) == Some(&b':')
    {
        return Err(PluginManifestError::InvalidPath {
            field,
            path: path.to_string(),
        });
    }

    for segment in path.replace('\\', "/").split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
            return Err(PluginManifestError::InvalidPath {
                field,
                path: path.to_string(),
            });
        }
    }
    Ok(())
}

fn normalize_version_requirement(requirement: &str) -> String {
    requirement
        .replace('X', "0")
        .replace('x', "0")
        .replace('*', "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> PluginManifest {
        PluginManifest::from_json_str(
            r#"
            {
              "schemaVersion": 1,
              "id": "ssmt.hoyoshade.bridge",
              "name": "HoYoShade Bridge",
              "version": "0.1.0",
              "author": "SSMT",
              "compatibility": {"ssmt": ">=4.x", "platforms": ["windows-x64"]},
              "contributions": {
                "runtimePlugins": [{"path": "native/example.dll"}],
                "launcherAdapters": [{
                  "type": "process-forwarder",
                  "executable": "${external.hoyoshade}/inject.exe",
                  "arguments": ["${game.processName}"],
                  "workingDirectory": "${external.hoyoshade}"
                }],
                "uiPages": [{"id": "hoyoshade", "route": "/plugins/hoyoshade", "path": "ui/index.html"}]
              },
              "externalDependencies": [{
                "id": "hoyoshade",
                "type": "directory",
                "requiredFiles": ["inject.exe", "ReShade64.dll"]
              }],
              "permissions": ["process.spawn", "process.observe"]
            }
            "#,
        )
        .expect("valid manifest")
    }

    #[test]
    fn parses_valid_manifest_and_contributions() {
        let manifest = valid_manifest();
        assert_eq!(
            manifest.contributions.runtime_plugins[0].path,
            "native/example.dll"
        );
        assert_eq!(
            manifest.contributions.launcher_adapters[0].kind,
            LauncherAdapterType::ProcessForwarder
        );
        assert_eq!(
            manifest.external_dependencies[0].kind,
            ExternalDependencyType::Directory
        );
    }

    #[test]
    fn rejects_invalid_schema_version() {
        let error = PluginManifest::from_json_str(
            &serde_json::to_string(&serde_json::json!({
                "schemaVersion": 2,
                "id": "ssmt.test",
                "name": "Test",
                "version": "1.0.0",
                "author": "SSMT",
                "compatibility": {"ssmt": ">=4.0.0", "platforms": ["windows-x64"]},
                "contributions": {}
            }))
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(error, PluginManifestError::UnsupportedSchemaVersion(2));
    }

    #[test]
    fn rejects_duplicate_plugin_ids() {
        let first = valid_manifest();
        let second = first.clone();
        assert_eq!(
            validate_unique_plugin_ids([&first, &second]),
            Err(PluginManifestError::DuplicatePluginId(
                "ssmt.hoyoshade.bridge".to_string()
            ))
        );
    }

    #[test]
    fn rejects_path_traversal() {
        let mut manifest = valid_manifest();
        manifest.contributions.runtime_plugins[0].path = "native/../outside.dll".to_string();
        assert!(matches!(
            manifest.validate(),
            Err(PluginManifestError::InvalidPath { .. })
        ));
        assert!(validate_package_relative_path("..\\outside.dll").is_err());
        assert!(validate_package_relative_path("C:\\outside.dll").is_err());
    }

    #[test]
    fn rejects_unsupported_platform_and_invalid_version() {
        let mut manifest = valid_manifest();
        manifest.compatibility.platforms = vec!["linux-x64".to_string()];
        assert_eq!(
            manifest.validate(),
            Err(PluginManifestError::UnsupportedPlatform(
                "linux-x64".to_string()
            ))
        );

        let mut manifest = valid_manifest();
        manifest.version = "not-a-version".to_string();
        assert_eq!(
            manifest.validate(),
            Err(PluginManifestError::InvalidVersion(
                "not-a-version".to_string()
            ))
        );
    }

    #[test]
    fn rejects_invalid_contribution_and_unknown_dependency() {
        let mut manifest = valid_manifest();
        manifest.contributions.launcher_adapters[0].executable =
            "${external.missing}/inject.exe".to_string();
        assert!(matches!(
            manifest.validate(),
            Err(PluginManifestError::InvalidContribution(_))
        ));

        let mut manifest = valid_manifest();
        manifest.external_dependencies[0].required_files = vec!["../inject.exe".to_string()];
        assert!(matches!(
            manifest.validate(),
            Err(PluginManifestError::InvalidExternalDependency(_))
        ));
    }

    #[test]
    fn serializes_manifest_using_schema_names() {
        let json = serde_json::to_value(valid_manifest()).unwrap();
        assert!(json.get("schemaVersion").is_some());
        assert!(json["contributions"].get("runtimePlugins").is_some());
        assert!(json["externalDependencies"][0]
            .get("requiredFiles")
            .is_some());
    }
}
