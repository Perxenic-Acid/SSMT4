use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;

pub mod backend;
pub mod backend_launch;
pub mod capability_broker;
pub mod commands;
pub mod data_store;
pub mod dlss5;
pub mod dlss5_hot_runtime;
mod dlss5_policy;
pub mod external_manager;
pub mod file_transaction;
pub mod gimi_hot_runtime;
pub mod graphics_stack;
pub mod graphics_tasks;
pub mod host_extensions;
pub mod host_graphics;
mod hot_reshade;
pub mod hoyoshade;
pub mod injected_dlls;
pub mod launch_barrier;
pub mod launch_coordinator;
pub mod launch_event_bus;
pub mod launch_plan;
pub mod local_packages;
pub mod logging;
pub mod managed_cold_files;
pub mod managed_reshade;
pub mod managed_reshade_journal;
pub mod managed_stack;
pub mod marketplace;
pub mod official_release;
pub mod package_installer;
pub mod page_runtime;
pub mod process_forwarder;
pub mod registry;
pub mod resource_cache;
pub mod settings;
mod shade_policy;

pub const MIN_PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 2;
pub const PLUGIN_API_VERSION: &str = "1.1.0";
pub const SUPPORTED_PLUGIN_API_VERSION: u32 = 1;
pub const SUPPORTED_PLATFORM: &str = "windows-x64";

fn default_plugin_api_version() -> u32 {
    SUPPORTED_PLUGIN_API_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    #[serde(default = "default_plugin_api_version")]
    pub plugin_api_version: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_api: Option<String>,
    #[serde(default)]
    pub games: Vec<String>,
}

impl PluginCompatibility {
    pub fn supports_game(&self, game: &str) -> bool {
        self.games.is_empty()
            || self
                .games
                .iter()
                .any(|item| item.eq_ignore_ascii_case(game.trim()))
    }

    pub fn supports(&self, ssmt_version: &str, platform: &str, game: Option<&str>) -> bool {
        let Ok(version) = Version::parse(ssmt_version) else {
            return false;
        };
        let Ok(requirement) = VersionReq::parse(&self.ssmt.replace(['x', 'X', '*'], "0")) else {
            return false;
        };
        if !requirement.matches(&version) || !self.platforms.iter().any(|item| item == platform) {
            return false;
        }
        match game {
            Some(game) => self.supports_game(game),
            _ => true,
        }
    }

    pub fn supports_plugin_api(&self, version: &str) -> bool {
        let Some(requirement) = self.plugin_api.as_deref() else {
            return SUPPORTED_PLUGIN_API_VERSION == 1;
        };
        let Ok(requirement) = parse_plugin_api_requirement(requirement) else {
            return false;
        };
        Version::parse(version).is_ok_and(|version| requirement.matches(&version))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginContributions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<backend::BackendContribution>,
    #[serde(default)]
    pub host_extensions: Vec<host_extensions::HostExtensionContribution>,
    #[serde(default)]
    pub runtime_plugins: Vec<RuntimePluginContribution>,
    /// DLLs loaded directly by Run.exe's extra-DLL path. Unlike
    /// `runtimePlugins`, these do not need to export the SSMT PluginHost ABI.
    #[serde(default)]
    pub injected_dlls: Vec<InjectedDllContribution>,
    #[serde(default)]
    pub launcher_adapters: Vec<LauncherAdapterContribution>,
    #[serde(default)]
    pub external_managers: Vec<ExternalManagerContribution>,
    #[serde(default)]
    pub ui_pages: Vec<UiPageContribution>,
    #[serde(default)]
    pub pages: Vec<PluginPageContribution>,
    #[serde(default)]
    pub launch_hooks: Vec<LaunchHookContribution>,
    #[serde(default)]
    pub data_directories: Vec<PluginDataDirectoryContribution>,
    #[serde(default)]
    pub migrations: Vec<PluginMigrationContribution>,
    #[serde(default)]
    pub graphics_profiles: Vec<GraphicsProfileContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PluginPageContribution {
    Declarative {
        id: String,
        title: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        fields: Vec<DeclarativePageField>,
    },
    Static {
        id: String,
        title: String,
        path: String,
    },
    LocalService {
        id: String,
        title: String,
        path: String,
        service: LocalPageServiceContribution,
    },
}

impl PluginPageContribution {
    pub fn id(&self) -> &str {
        match self {
            Self::Declarative { id, .. }
            | Self::Static { id, .. }
            | Self::LocalService { id, .. } => id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Declarative { title, .. }
            | Self::Static { title, .. }
            | Self::LocalService { title, .. } => title,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Declarative { .. } => "declarative",
            Self::Static { .. } => "static",
            Self::LocalService { .. } => "local-service",
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Declarative { .. } => None,
            Self::Static { path, .. } | Self::LocalService { path, .. } => Some(path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarativePageField {
    pub key: String,
    pub label: String,
    pub kind: DeclarativePageFieldKind,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub options: Vec<DeclarativePageOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeclarativePageFieldKind {
    Boolean,
    Integer,
    String,
    Select,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarativePageOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalPageServiceContribution {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    pub data_directory: String,
    pub health_path: String,
    #[serde(default)]
    pub token_header: Option<String>,
    #[serde(default = "default_page_ready_timeout_ms")]
    pub ready_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchHookContribution {
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub actions: Vec<LaunchHookAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LaunchHookAction {
    FileCopy {
        source: String,
        target: String,
        sha256: String,
        #[serde(default)]
        resource: Option<String>,
        #[serde(default)]
        expected_target_sha256: Option<String>,
    },
    FileRemove {
        target: String,
        #[serde(default)]
        resource: Option<String>,
        #[serde(default)]
        expected_target_sha256: Option<String>,
    },
    ArgumentAppend {
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDataDirectoryContribution {
    pub id: String,
    pub package_path: String,
    #[serde(default = "default_true")]
    pub seed_from_package: bool,
    #[serde(default = "default_true")]
    pub mirror_on_launch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMigrationContribution {
    pub id: String,
    pub source: PluginMigrationSource,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginMigrationSource {
    GameConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphicsProfileContribution {
    pub id: String,
    pub strategy: GraphicsProfileStrategy,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default)]
    pub settings_component: Option<String>,
    #[serde(default)]
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphicsProfileStrategy {
    ReshadeHot,
    Dlss5Hot,
}

impl GraphicsProfileStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReshadeHot => "reshade-hot",
            Self::Dlss5Hot => "dlss5-hot",
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_page_ready_timeout_ms() -> u64 {
    30_000
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
pub struct InjectedDllContribution {
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executable_names: Vec<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl InjectedDllContribution {
    pub fn applies_to_executable(&self, executable: Option<&Path>) -> bool {
        self.executable_names.is_empty()
            || executable
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    self.executable_names
                        .iter()
                        .any(|expected| name.eq_ignore_ascii_case(expected))
                })
    }

    pub fn verify_file(&self, package_root: &Path) -> Result<(), String> {
        let path = package_root.join(&self.path);
        if !path.is_file() {
            return Err(format!("injected DLL does not exist: {}", path.display()));
        }
        if let Some(expected) = &self.sha256 {
            crate::plugin::marketplace::verify_package_sha256(&path, expected)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
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
    #[serde(default)]
    pub required_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_hint: Option<String>,
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
    pub fn is_app_scoped(&self) -> bool {
        self.compatibility.games.is_empty()
            && (!self.contributions.ui_pages.is_empty()
                || !self.contributions.pages.is_empty()
                || self.contributions.backend.is_some())
            && self.contributions.runtime_plugins.is_empty()
            && self.contributions.injected_dlls.is_empty()
            && self.contributions.launcher_adapters.is_empty()
            && self.contributions.external_managers.is_empty()
            && self.contributions.launch_hooks.is_empty()
            && self.contributions.data_directories.is_empty()
            && self.contributions.migrations.is_empty()
            && self.contributions.graphics_profiles.is_empty()
    }

    pub fn supports_current_plugin_api(&self) -> bool {
        if let Some(requirement) = self.compatibility.plugin_api.as_deref() {
            return parse_plugin_api_requirement(requirement)
                .is_ok_and(|_| self.compatibility.supports_plugin_api(PLUGIN_API_VERSION));
        }
        self.plugin_api_version == SUPPORTED_PLUGIN_API_VERSION
    }

    pub fn from_json_str(raw: &str) -> Result<Self, PluginManifestError> {
        let manifest = serde_json::from_str::<Self>(raw)
            .map_err(|error| PluginManifestError::Json(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), PluginManifestError> {
        if !(MIN_PLUGIN_MANIFEST_SCHEMA_VERSION..=PLUGIN_MANIFEST_SCHEMA_VERSION)
            .contains(&self.schema_version)
        {
            return Err(PluginManifestError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.schema_version >= 2 && self.compatibility.plugin_api.as_deref().is_none() {
            return Err(PluginManifestError::InvalidCompatibility(
                "schemaVersion 2 requires compatibility.pluginApi".to_string(),
            ));
        }
        if self.schema_version < 2 && !self.contributions.launch_hooks.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "launchHooks require schemaVersion 2".to_string(),
            ));
        }
        if self.schema_version < 2 && !self.contributions.data_directories.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "dataDirectories require schemaVersion 2".to_string(),
            ));
        }
        if self.schema_version < 2 && !self.contributions.migrations.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "migrations require schemaVersion 2".to_string(),
            ));
        }
        if self.schema_version < 2 && !self.contributions.graphics_profiles.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "graphicsProfiles require schemaVersion 2".to_string(),
            ));
        }
        if let Some(requirement) = self.compatibility.plugin_api.as_deref() {
            parse_plugin_api_requirement(requirement).map_err(|_| {
                PluginManifestError::InvalidCompatibility(format!(
                    "invalid pluginApi: {requirement}"
                ))
            })?;
        }
        // Future Plugin API declarations remain installable and visible so the UI can
        // mark them incompatible. Execution is gated by supports_current_plugin_api().
        if self.plugin_api_version == 0 {
            return Err(PluginManifestError::InvalidContribution(format!(
                "unsupported pluginApiVersion: {}",
                self.plugin_api_version
            )));
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
        if let Some(backend) = &self.contributions.backend {
            backend.validate()?;
            if !self.permissions.contains(&PluginPermission::ProcessSpawn) {
                return Err(PluginManifestError::InvalidContribution(
                    "backend requires process.spawn".into(),
                ));
            }
        }
        validate_launch_hook_permissions(&self.contributions.launch_hooks, &self.permissions)?;
        if !self.contributions.data_directories.is_empty()
            && (!self.permissions.contains(&PluginPermission::FilesystemRead)
                || !self
                    .permissions
                    .contains(&PluginPermission::FilesystemWrite))
        {
            return Err(PluginManifestError::InvalidContribution(
                "data directories require filesystem.read and filesystem.write".to_string(),
            ));
        }
        if !self.contributions.migrations.is_empty()
            && !self.permissions.contains(&PluginPermission::FilesystemRead)
        {
            return Err(PluginManifestError::InvalidContribution(
                "migrations require filesystem.read".to_string(),
            ));
        }
        if !self.contributions.graphics_profiles.is_empty()
            && (!self.permissions.contains(&PluginPermission::NativeInject)
                || !self.permissions.contains(&PluginPermission::FilesystemRead)
                || !self
                    .permissions
                    .contains(&PluginPermission::FilesystemWrite))
        {
            return Err(PluginManifestError::InvalidContribution(
                "graphicsProfiles require native.inject, filesystem.read and filesystem.write"
                    .to_string(),
            ));
        }
        if self
            .contributions
            .pages
            .iter()
            .any(|page| matches!(page, PluginPageContribution::LocalService { .. }))
        {
            if self.compatibility.games.is_empty() {
                return Err(PluginManifestError::InvalidContribution(
                    "local-service pages require at least one compatible game".to_string(),
                ));
            }
            if !self
                .permissions
                .iter()
                .any(|permission| *permission == PluginPermission::ProcessSpawn)
            {
                return Err(PluginManifestError::InvalidContribution(
                    "local-service pages require process.spawn permission".to_string(),
                ));
            }
        }
        if !self.contributions.launch_hooks.is_empty() && self.compatibility.games.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "launch hooks require at least one compatible game".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginContribution {
    RuntimePlugin(RuntimePluginContribution),
    LauncherAdapter(LauncherAdapterContribution),
    UiPage(UiPageContribution),
    Page(PluginPageContribution),
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
    let mut extension_ids = HashSet::new();
    for extension in &contributions.host_extensions {
        extension.validate()?;
        if !extension_ids.insert(&extension.id) {
            return Err(PluginManifestError::DuplicateValue {
                field: "host extension id",
                value: extension.id.clone(),
            });
        }
    }
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

    let mut injected_dll_paths = HashSet::new();
    for injected_dll in &contributions.injected_dlls {
        validate_package_path("injected DLL", &injected_dll.path)?;
        if !injected_dll.path.to_ascii_lowercase().ends_with(".dll") {
            return Err(PluginManifestError::InvalidContribution(format!(
                "injected DLL must use a .dll extension: {}",
                injected_dll.path
            )));
        }
        if let Some(hash) = &injected_dll.sha256 {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(PluginManifestError::InvalidContribution(format!(
                    "invalid injected DLL SHA256: {}",
                    injected_dll.path
                )));
            }
        }
        let normalized = injected_dll.path.replace('\\', "/");
        if !injected_dll_paths.insert(normalized.clone()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "injected DLL path",
                value: injected_dll.path.clone(),
            });
        }
        if runtime_paths.contains(&normalized) {
            return Err(PluginManifestError::InvalidContribution(format!(
                "a DLL cannot be both a PluginHost runtime and a direct injected DLL: {}",
                injected_dll.path
            )));
        }
    }

    let mut routes = HashSet::new();
    for page in &contributions.ui_pages {
        validate_plugin_id(&page.id)?;
        validate_ui_route(&page.route)?;
        validate_package_path("UI page", &page.path)?;
        if !routes.insert(page.route.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "UI page route",
                value: page.route.clone(),
            });
        }
    }

    let mut page_ids = HashSet::new();
    for page in &contributions.pages {
        validate_plugin_id(page.id())?;
        validate_non_empty("page title", page.title())?;
        if !page_ids.insert(page.id()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "plugin page id",
                value: page.id().to_string(),
            });
        }
        match page {
            PluginPageContribution::Declarative { fields, .. } => {
                validate_declarative_fields(fields)?;
            }
            PluginPageContribution::Static { path, .. } => {
                validate_package_path("static page", path)?;
            }
            PluginPageContribution::LocalService { path, service, .. } => {
                validate_package_path("local service page", path)?;
                validate_local_page_service(service, &dependency_ids)?;
            }
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
    let mut data_ids = HashSet::new();
    let mut data_paths = HashSet::new();
    if contributions.data_directories.len() > 32 {
        return Err(PluginManifestError::InvalidContribution(
            "a plugin may declare at most 32 data directories".to_string(),
        ));
    }
    for data in &contributions.data_directories {
        validate_plugin_id(&data.id)?;
        if !data_ids.insert(data.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "data directory id",
                value: data.id.clone(),
            });
        }
        validate_package_path("data directory package path", &data.package_path)?;
        let normalized = data.package_path.replace('\\', "/").to_ascii_lowercase();
        if normalized.starts_with("plugin/") || normalized == "plugin" {
            return Err(PluginManifestError::InvalidContribution(
                "data directory package path must not overlap runtime plugin files".to_string(),
            ));
        }
        if !data_paths.insert(normalized.clone()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "data directory package path",
                value: data.package_path.clone(),
            });
        }
    }
    let mut migration_ids = HashSet::new();
    if contributions.migrations.len() > 16 {
        return Err(PluginManifestError::InvalidContribution(
            "a plugin may declare at most 16 migrations".to_string(),
        ));
    }
    for migration in &contributions.migrations {
        validate_plugin_id(&migration.id)?;
        if !migration_ids.insert(migration.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "migration id",
                value: migration.id.clone(),
            });
        }
        if migration.keys.is_empty() || migration.keys.len() > 32 {
            return Err(PluginManifestError::InvalidContribution(
                "migration must declare between 1 and 32 keys".to_string(),
            ));
        }
        let mut keys = HashSet::new();
        for key in &migration.keys {
            if key.is_empty()
                || key.len() > 128
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
                || !keys.insert(key)
            {
                return Err(PluginManifestError::InvalidContribution(format!(
                    "invalid migration key: {key}"
                )));
            }
        }
    }
    let mut graphics_ids = HashSet::new();
    if contributions.graphics_profiles.len() > 8 {
        return Err(PluginManifestError::InvalidContribution(
            "a plugin may declare at most 8 graphics profiles".to_string(),
        ));
    }
    for profile in &contributions.graphics_profiles {
        validate_plugin_id(&profile.id)?;
        if !graphics_ids.insert(profile.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "graphics profile id",
                value: profile.id.clone(),
            });
        }
        if !(-10_000..=10_000).contains(&profile.priority) {
            return Err(PluginManifestError::InvalidContribution(
                "graphics profile priority must be between -10000 and 10000".to_string(),
            ));
        }
        if let Some(config) = &profile.config {
            validate_package_path("graphics profile config", config)?;
        }
        if let Some(settings) = &profile.settings_component {
            validate_plugin_id(settings)?;
        }
        if profile.resources.is_empty() {
            return Err(PluginManifestError::InvalidContribution(
                "graphics profile must declare at least one resource".to_string(),
            ));
        }
        let mut resources = HashSet::new();
        for resource in &profile.resources {
            validate_resource_key(resource)?;
            if !resources.insert(resource) {
                return Err(PluginManifestError::DuplicateValue {
                    field: "graphics resource",
                    value: resource.clone(),
                });
            }
        }
    }
    validate_launch_hooks(&contributions.launch_hooks)?;
    Ok(())
}

fn validate_launch_hooks(hooks: &[LaunchHookContribution]) -> Result<(), PluginManifestError> {
    if hooks.len() > 128 {
        return Err(PluginManifestError::InvalidContribution(
            "a plugin may declare at most 128 launch hooks".to_string(),
        ));
    }
    let mut ids = HashSet::new();
    for hook in hooks {
        validate_plugin_id(&hook.id)?;
        if !ids.insert(hook.id.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "launch hook id",
                value: hook.id.clone(),
            });
        }
        if !(-10_000..=10_000).contains(&hook.priority) {
            return Err(PluginManifestError::InvalidContribution(format!(
                "launch hook priority must be between -10000 and 10000: {}",
                hook.id
            )));
        }
        if hook.actions.is_empty() || hook.actions.len() > 64 {
            return Err(PluginManifestError::InvalidContribution(format!(
                "launch hook must contain between 1 and 64 actions: {}",
                hook.id
            )));
        }
        let mut targets = HashSet::new();
        for action in &hook.actions {
            match action {
                LaunchHookAction::FileCopy {
                    source,
                    target,
                    sha256,
                    resource,
                    expected_target_sha256,
                } => {
                    validate_package_path("launch file source", source)?;
                    validate_launch_target(target)?;
                    validate_sha256("launch file source SHA256", sha256)?;
                    if let Some(expected) = expected_target_sha256 {
                        validate_sha256("launch target expectation", expected)?;
                    }
                    if let Some(resource) = resource {
                        validate_resource_key(resource)?;
                    }
                    if !targets.insert(target.replace('\\', "/").to_ascii_lowercase()) {
                        return Err(PluginManifestError::InvalidContribution(format!(
                            "launch hook writes the same target more than once: {target}"
                        )));
                    }
                }
                LaunchHookAction::FileRemove {
                    target,
                    resource,
                    expected_target_sha256,
                } => {
                    validate_launch_target(target)?;
                    if let Some(expected) = expected_target_sha256 {
                        validate_sha256("launch target expectation", expected)?;
                    }
                    if let Some(resource) = resource {
                        validate_resource_key(resource)?;
                    }
                    if !targets.insert(target.replace('\\', "/").to_ascii_lowercase()) {
                        return Err(PluginManifestError::InvalidContribution(format!(
                            "launch hook writes the same target more than once: {target}"
                        )));
                    }
                }
                LaunchHookAction::ArgumentAppend { values } => {
                    if values.is_empty() || values.len() > 32 {
                        return Err(PluginManifestError::InvalidContribution(
                            "launch argument action requires between 1 and 32 values".to_string(),
                        ));
                    }
                    if values
                        .iter()
                        .any(|value| value.is_empty() || value.len() > 4096 || value.contains('\0'))
                    {
                        return Err(PluginManifestError::InvalidContribution(
                            "launch argument value is empty, too long or contains NUL".to_string(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_launch_target(target: &str) -> Result<(), PluginManifestError> {
    validate_package_path("launch target", target)?;
    if target
        .replace('\\', "/")
        .split('/')
        .any(|segment| segment.to_ascii_lowercase().starts_with(".ssmt-"))
    {
        return Err(PluginManifestError::InvalidPath {
            field: "launch target",
            path: target.to_string(),
        });
    }
    Ok(())
}

fn validate_resource_key(resource: &str) -> Result<(), PluginManifestError> {
    if resource.is_empty()
        || resource.len() > 128
        || resource.starts_with(':')
        || resource.ends_with(':')
        || !resource
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(PluginManifestError::InvalidContribution(format!(
            "invalid launch resource key: {resource}"
        )));
    }
    Ok(())
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), PluginManifestError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PluginManifestError::InvalidContribution(format!(
            "invalid {field}: {value}"
        )));
    }
    Ok(())
}

fn validate_launch_hook_permissions(
    hooks: &[LaunchHookContribution],
    permissions: &[PluginPermission],
) -> Result<(), PluginManifestError> {
    let has = |permission: PluginPermission| permissions.contains(&permission);
    for hook in hooks {
        for action in &hook.actions {
            match action {
                LaunchHookAction::FileCopy { .. }
                    if !has(PluginPermission::FilesystemRead)
                        || !has(PluginPermission::FilesystemWrite) =>
                {
                    return Err(PluginManifestError::InvalidContribution(format!(
                        "launch hook {} file-copy requires filesystem.read and filesystem.write",
                        hook.id
                    )));
                }
                LaunchHookAction::FileRemove { .. } if !has(PluginPermission::FilesystemWrite) => {
                    return Err(PluginManifestError::InvalidContribution(format!(
                        "launch hook {} file-remove requires filesystem.write",
                        hook.id
                    )));
                }
                LaunchHookAction::ArgumentAppend { .. } if !has(PluginPermission::GameLaunch) => {
                    return Err(PluginManifestError::InvalidContribution(format!(
                        "launch hook {} argument-append requires game.launch",
                        hook.id
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_declarative_fields(fields: &[DeclarativePageField]) -> Result<(), PluginManifestError> {
    let mut keys = HashSet::new();
    for field in fields {
        if field.key.is_empty()
            || field.key.len() > 128
            || !field
                .key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(PluginManifestError::InvalidContribution(format!(
                "invalid declarative field key: {}",
                field.key
            )));
        }
        if !keys.insert(field.key.as_str()) {
            return Err(PluginManifestError::DuplicateValue {
                field: "declarative field key",
                value: field.key.clone(),
            });
        }
        validate_non_empty("declarative field label", &field.label)?;
        if let (Some(min), Some(max)) = (field.min, field.max) {
            if min > max {
                return Err(PluginManifestError::InvalidContribution(format!(
                    "declarative field range is reversed: {}",
                    field.key
                )));
            }
        }
        if field.kind == DeclarativePageFieldKind::Select && field.options.is_empty() {
            return Err(PluginManifestError::InvalidContribution(format!(
                "select field has no options: {}",
                field.key
            )));
        }
        let mut option_values = HashSet::new();
        for option in &field.options {
            if option.value.is_empty()
                || option.label.is_empty()
                || !option_values.insert(option.value.as_str())
            {
                return Err(PluginManifestError::InvalidContribution(format!(
                    "invalid or duplicate select option: {}",
                    field.key
                )));
            }
        }
    }
    Ok(())
}

fn validate_local_page_service(
    service: &LocalPageServiceContribution,
    dependency_ids: &HashSet<&str>,
) -> Result<(), PluginManifestError> {
    validate_page_template(
        "local service executable",
        &service.executable,
        dependency_ids,
    )?;
    for argument in &service.arguments {
        validate_page_template("local service argument", argument, dependency_ids)?;
    }
    if let Some(working_directory) = &service.working_directory {
        validate_page_template(
            "local service working directory",
            working_directory,
            dependency_ids,
        )?;
    }
    validate_package_path("local service data directory", &service.data_directory)?;
    if service.data_directory.contains('/') || service.data_directory.contains('\\') {
        return Err(PluginManifestError::InvalidContribution(
            "local service dataDirectory must be one path segment".to_string(),
        ));
    }
    if !service.health_path.starts_with('/')
        || service.health_path.starts_with("//")
        || service.health_path.contains("://")
        || service.health_path.contains("..")
        || service.health_path.chars().any(char::is_control)
    {
        return Err(PluginManifestError::InvalidContribution(format!(
            "invalid local service healthPath: {}",
            service.health_path
        )));
    }
    if let Some(header) = &service.token_header {
        if header.is_empty()
            || header.len() > 128
            || !header
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(PluginManifestError::InvalidContribution(format!(
                "invalid local service tokenHeader: {header}"
            )));
        }
    }
    if service.ready_timeout_ms == 0 || service.ready_timeout_ms > 120_000 {
        return Err(PluginManifestError::InvalidContribution(
            "local service readyTimeoutMs must be between 1 and 120000".to_string(),
        ));
    }
    Ok(())
}

fn validate_page_template(
    field: &'static str,
    value: &str,
    dependency_ids: &HashSet<&str>,
) -> Result<(), PluginManifestError> {
    validate_non_empty(field, value)?;
    if !value.contains("${") {
        validate_package_path(field, value)?;
        return Ok(());
    }
    let mut remainder = value;
    while let Some(start) = remainder.find("${") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err(PluginManifestError::InvalidContribution(format!(
                "unterminated variable in {field}: {value}"
            )));
        };
        let variable = &after_start[..end];
        let valid = matches!(
            variable,
            "plugin.root"
                | "game.exe"
                | "game.directory"
                | "game.mods"
                | "service.port"
                | "service.token"
                | "service.data"
                | "host.pid"
        ) || variable
            .strip_prefix("external.")
            .is_some_and(|id| dependency_ids.contains(id));
        if !valid {
            return Err(PluginManifestError::InvalidContribution(format!(
                "unsupported page variable: ${{{variable}}}"
            )));
        }
        remainder = &after_start[end + 1..];
    }
    Ok(())
}

fn validate_ui_route(route: &str) -> Result<(), PluginManifestError> {
    let Some(suffix) = route.strip_prefix("/plugins/") else {
        return Err(PluginManifestError::InvalidContribution(format!(
            "UI page route must be under /plugins/: {route}"
        )));
    };
    if suffix.is_empty()
        || suffix.split('/').any(|segment| {
            segment.is_empty()
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(PluginManifestError::InvalidContribution(format!(
            "invalid UI page route: {route}"
        )));
    }
    Ok(())
}

fn validate_external_dependencies(
    dependencies: &[ExternalDependency],
) -> Result<(), PluginManifestError> {
    let mut ids = HashSet::new();
    for dependency in dependencies {
        if let Some(raw) = &dependency.download_url {
            let valid = reqwest::Url::parse(raw).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            if !valid || raw.len() > 4096 || raw.trim() != raw || raw.chars().any(char::is_control)
            {
                return Err(PluginManifestError::InvalidExternalDependency(format!(
                    "download URL must use HTTPS without credentials: {}",
                    dependency.id
                )));
            }
        }
        if dependency
            .download_hint
            .as_ref()
            .is_some_and(|hint| hint.trim().is_empty() || hint.len() > 4096)
        {
            return Err(PluginManifestError::InvalidExternalDependency(format!(
                "invalid download hint: {}",
                dependency.id
            )));
        }
        if let Some(features) = &dependency.required_for {
            for feature in features {
                validate_plugin_id(feature)?;
            }
        }
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
        let stem = segment
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.ends_with(['.', ' '])
            || segment
                .chars()
                .any(|character| character < ' ' || "<>:\"|?*".contains(character))
            || reserved
        {
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

/// Semver's Rust parser uses comma-separated comparators, while plugin manifests commonly
/// use the npm-style space-separated form. Accept both without making the manifest syntax
/// depend on a parser implementation detail.
fn parse_plugin_api_requirement(requirement: &str) -> Result<VersionReq, semver::Error> {
    let compact = requirement.split_whitespace().collect::<Vec<_>>().join(" ");
    VersionReq::parse(&compact).or_else(|_| {
        let mut separated = String::with_capacity(compact.len() + 4);
        let mut characters = compact.chars().peekable();
        while let Some(character) = characters.next() {
            if character == ' '
                && matches!(characters.peek(), Some('<' | '>' | '='))
                && !separated.trim_end().ends_with(',')
            {
                separated.push(',');
            }
            separated.push(character);
        }
        VersionReq::parse(&separated)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_windows_aliases_in_package_paths() {
        for path in [
            "CON.txt",
            "tools/COM1.dll",
            "module. ",
            "a<file>.dll",
            "dir/name:stream",
        ] {
            assert!(validate_package_relative_path(path).is_err(), "{path}");
        }
        assert!(validate_package_relative_path("native/example.dll").is_ok());
    }

    #[test]
    fn app_page_without_game_contributions_uses_app_scope() {
        let mut manifest = valid_manifest();
        assert!(!manifest.is_app_scoped());
        manifest.contributions.runtime_plugins.clear();
        manifest.contributions.launcher_adapters.clear();
        assert!(manifest.is_app_scoped());
        manifest.compatibility.games.push("GIMI".into());
        assert!(!manifest.is_app_scoped());
    }

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
                "requiredFiles": ["inject.exe", "ReShade64.dll", "ReShade.ini"]
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
    fn validates_direct_injected_dll_contributions() {
        let manifest = PluginManifest::from_json_str(
            r#"{
              "schemaVersion": 1,
              "id": "ssmt.secondary-motion",
              "name": "Secondary Motion",
              "version": "3.2.0",
              "author": "SSMT",
              "compatibility": {"ssmt": ">=4.2.0", "platforms": ["windows-x64"], "games": ["EFMI"]},
              "contributions": {"injectedDlls": [{"path": "plugin/sbm.dll"}]},
              "permissions": ["native.inject"]
            }"#,
        )
        .unwrap();
        assert_eq!(
            manifest.contributions.injected_dlls[0].path,
            "plugin/sbm.dll"
        );
        assert!(!manifest.is_app_scoped());

        let mut invalid = manifest.clone();
        invalid.contributions.injected_dlls[0].path = "plugin/sbm.txt".into();
        assert!(invalid.validate().is_err());

        invalid = manifest;
        invalid.contributions.runtime_plugins = vec![RuntimePluginContribution {
            path: "plugin/sbm.dll".into(),
        }];
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn rejects_schema_two_without_plugin_api_range() {
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
        assert!(matches!(
            error,
            PluginManifestError::InvalidCompatibility(_)
        ));
    }

    #[test]
    fn rejects_unsupported_manifest_schema_version() {
        let error = PluginManifest::from_json_str(
            &serde_json::to_string(&serde_json::json!({
                "schemaVersion": 3,
                "id": "ssmt.test",
                "name": "Test",
                "version": "1.0.0",
                "author": "SSMT",
                "compatibility": {
                    "ssmt": ">=4.0.0",
                    "pluginApi": ">=1.0.0 <2.0.0",
                    "platforms": ["windows-x64"]
                },
                "contributions": {}
            }))
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(error, PluginManifestError::UnsupportedSchemaVersion(3));
    }

    #[test]
    fn plugin_api_accepts_comma_and_space_separated_ranges() {
        for requirement in [">=1.0.0, <2.0.0", ">=1.0.0 <2.0.0"] {
            let parsed = parse_plugin_api_requirement(requirement).unwrap();
            assert!(parsed.matches(&Version::parse(PLUGIN_API_VERSION).unwrap()));
            assert!(!parsed.matches(&Version::parse("2.0.0").unwrap()));
        }
        assert!(parse_plugin_api_requirement(">=1.0.0 <oops").is_err());
    }

    #[test]
    fn future_plugin_api_is_valid_but_runtime_incompatible() {
        let mut manifest = valid_manifest();
        manifest.schema_version = 2;
        manifest.compatibility.plugin_api = Some(">=2.0.0 <3.0.0".into());
        manifest.validate().unwrap();
        assert!(!manifest.supports_current_plugin_api());

        manifest.compatibility.plugin_api = Some(">=1.0.0 <oops".into());
        assert!(matches!(
            manifest.validate(),
            Err(PluginManifestError::InvalidCompatibility(_))
        ));
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
    fn rejects_ui_routes_outside_plugin_namespace() {
        let mut manifest = valid_manifest();
        manifest.contributions.ui_pages.push(UiPageContribution {
            id: "settings".to_string(),
            route: "/settings/override".to_string(),
            path: "ui/index.html".to_string(),
        });
        assert!(matches!(
            manifest.validate(),
            Err(PluginManifestError::InvalidContribution(_))
        ));
    }

    #[test]
    fn compatibility_matches_version_platform_and_optional_game() {
        let compatibility = PluginCompatibility {
            ssmt: ">=4.0.0, <5.0.0".to_string(),
            platforms: vec!["windows-x64".to_string()],
            plugin_api: None,
            games: vec!["GIMI".to_string()],
        };
        assert!(compatibility.supports("4.1.89", "windows-x64", Some("GIMI")));
        assert!(!compatibility.supports("5.0.0", "windows-x64", Some("GIMI")));
        assert!(!compatibility.supports("4.1.89", "windows-x64", Some("ZZMI")));
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
    fn validates_versioned_declarative_static_and_local_service_pages() {
        let manifest = PluginManifest::from_json_str(
            r#"{
              "schemaVersion": 1,
              "pluginApiVersion": 1,
              "id": "ssmt.page.fixture",
              "name": "Page Fixture",
              "version": "1.0.0",
              "author": "SSMT",
              "compatibility": {"ssmt": ">=4.2.2", "platforms": ["windows-x64"], "games": ["EFMI"]},
              "contributions": {
                "pages": [
                  {
                    "kind": "declarative",
                    "id": "settings",
                    "title": "Settings",
                    "fields": [{"key": "enabled", "label": "Enabled", "kind": "boolean"}]
                  },
                  {
                    "kind": "static",
                    "id": "help",
                    "title": "Help",
                    "path": "ui/help.html"
                  },
                  {
                    "kind": "local-service",
                    "id": "editor",
                    "title": "Editor",
                    "path": "ui/index.html",
                    "service": {
                      "executable": "${external.python-runtime}/python.exe",
                      "arguments": ["${plugin.root}/server.py", "--port", "${service.port}"],
                      "workingDirectory": "${plugin.root}",
                      "dataDirectory": "SSMT_PageFixture",
                      "healthPath": "/api/health",
                      "tokenHeader": "X-Editor-Token",
                      "readyTimeoutMs": 30000
                    }
                  }
                ]
              },
              "externalDependencies": [{
                "id": "python-runtime",
                "type": "directory",
                "requiredFiles": ["python.exe"],
                "requiredFor": ["local-service"]
              }],
              "permissions": ["process.spawn"]
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.plugin_api_version, 1);
        assert_eq!(manifest.contributions.pages.len(), 3);
        assert_eq!(manifest.contributions.pages[2].kind(), "local-service");

        let mut invalid = manifest.clone();
        invalid.plugin_api_version = 2;
        assert!(invalid.validate().is_ok());
        assert!(!invalid.supports_current_plugin_api());

        invalid = manifest.clone();
        if let PluginPageContribution::Static { path, .. } = &mut invalid.contributions.pages[1] {
            *path = "../outside.html".to_string();
        }
        assert!(invalid.validate().is_err());

        invalid = manifest;
        if let PluginPageContribution::LocalService { service, .. } =
            &mut invalid.contributions.pages[2]
        {
            service.health_path = "https://example.com/health".to_string();
        }
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn validates_launch_hook_paths_hashes_and_permissions() {
        let manifest = |permissions: &str, target: &str, action_type: &str| {
            PluginManifest::from_json_str(&format!(
                r#"{{
                  "schemaVersion": 2,
                  "id": "example.launch-hook",
                  "name": "Launch Hook",
                  "version": "1.0.0",
                  "author": "Example",
                  "compatibility": {{
                    "ssmt": ">=4.2.2",
                    "pluginApi": ">=1.0.0 <2.0.0",
                    "platforms": ["windows-x64"],
                    "games": ["EFMI"]
                  }},
                  "contributions": {{
                    "launchHooks": [{{
                      "id": "install",
                      "priority": 10,
                      "actions": [{{
                        "type": "{action_type}",
                        "source": "payload/proxy.dll",
                        "target": "{target}",
                        "sha256": "{}",
                        "resource": "proxy:dxgi"
                      }}]
                    }}]
                  }},
                  "permissions": [{permissions}]
                }}"#,
                "0".repeat(64)
            ))
        };
        assert!(manifest(
            r#""filesystem.read","filesystem.write""#,
            "dxgi.dll",
            "file-copy"
        )
        .is_ok());
        let mut no_game = manifest(
            r#""filesystem.read","filesystem.write""#,
            "dxgi.dll",
            "file-copy",
        )
        .unwrap();
        no_game.compatibility.games.clear();
        assert!(no_game.validate().is_err());
        let mut schema_one = manifest(
            r#""filesystem.read","filesystem.write""#,
            "dxgi.dll",
            "file-copy",
        )
        .unwrap();
        schema_one.schema_version = 1;
        assert!(schema_one.validate().is_err());
        assert!(manifest(r#""filesystem.read""#, "dxgi.dll", "file-copy").is_err());
        assert!(manifest(
            r#""filesystem.read","filesystem.write""#,
            "../outside.dll",
            "file-copy"
        )
        .is_err());
        assert!(manifest(
            r#""filesystem.read","filesystem.write""#,
            ".ssmt-launch-transactions/owned.dll",
            "file-copy"
        )
        .is_err());
    }

    #[test]
    fn launch_argument_hooks_require_game_launch_permission() {
        let raw = |permission: &str| {
            format!(
                r#"{{
                  "schemaVersion": 2,
                  "id": "example.launch-args",
                  "name": "Launch Args",
                  "version": "1.0.0",
                  "author": "Example",
                  "compatibility": {{
                    "ssmt": ">=4.2.2",
                    "pluginApi": ">=1.0.0 <2.0.0",
                    "platforms": ["windows-x64"],
                    "games": ["EFMI"]
                  }},
                  "contributions": {{
                    "launchHooks": [{{
                      "id": "arguments",
                      "actions": [{{"type": "argument-append", "values": ["--fixture"]}}]
                    }}]
                  }},
                  "permissions": [{permission}]
                }}"#
            )
        };
        assert!(PluginManifest::from_json_str(&raw(r#""game.launch""#)).is_ok());
        assert!(PluginManifest::from_json_str(&raw(r#""filesystem.read""#)).is_err());
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

    #[test]
    fn dependency_download_metadata_is_optional_and_requires_safe_https() {
        let original = valid_manifest();
        assert!(original.external_dependencies[0].download_url.is_none());
        let mut manifest = original;
        manifest.external_dependencies[0].download_url =
            Some("https://example.com/releases?version=1".into());
        manifest.external_dependencies[0].download_hint = Some("下载并解压后选择目录。".into());
        let json = serde_json::to_string(&manifest).unwrap();
        assert_eq!(PluginManifest::from_json_str(&json).unwrap(), manifest);
        for url in [
            "http://example.com",
            "javascript:alert(1)",
            "file:///C:/tool.exe",
            "https://user:pass@example.com",
            "https://user@example.com",
            "https://",
            " https://example.com",
            "https://example.com/\npath",
        ] {
            manifest.external_dependencies[0].download_url = Some(url.into());
            assert!(manifest.validate().is_err(), "accepted unsafe URL: {url}");
        }
    }
}
