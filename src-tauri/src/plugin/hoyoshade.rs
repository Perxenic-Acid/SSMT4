use super::gimi_hot_runtime::{canonical_path, checked_path, read_text, HotRuntimePlan};
use super::managed_reshade;
use super::process_forwarder::{
    LaunchTemplateContext, ProcessForwarderError, ProcessForwarderSpec,
};
use super::registry::{ExternalDependencyStatus, InstalledPlugin, PluginRegistry};
use super::{LauncherAdapterType, PluginManifest};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const HOYOSHADE_PLUGIN_ID: &str = "ssmt.hoyoshade.bridge";
pub const HOYOSHADE_DEPENDENCY_ID: &str = "hoyoshade";
const HOYOSHADE_GAME_PROCESSES: &[&str] = &[
    "YuanShen.exe",
    "GenshinImpact.exe",
    "Genshin.exe",
    "BH3.exe",
    "StarRail.exe",
    "ZenlessZoneZero.exe",
    "ZenlessZoneZeroBeta.exe",
    "ZZZ.exe",
];

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

    pub fn supports_process(&self, process_name: &str) -> bool {
        HOYOSHADE_GAME_PROCESSES
            .iter()
            .any(|name| name.eq_ignore_ascii_case(process_name))
    }
}

fn prepare_hot_at(
    registry: &PluginRegistry,
    executable: &Path,
    directory: &Path,
) -> Result<Option<HotRuntimePlan>, String> {
    let Some(plugin) = registry
        .find(HOYOSHADE_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
    else {
        return Ok(None);
    };
    let bridge = HoYoShadeBridge::from_installed(plugin).map_err(|error| error.to_string())?;
    if !bridge.supports_process(
        executable
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    ) {
        return Ok(None);
    }
    let dependency = plugin
        .external_dependencies
        .get(HOYOSHADE_DEPENDENCY_ID)
        .filter(|dependency| dependency.status == ExternalDependencyStatus::Ready)
        .and_then(|dependency| dependency.path.as_ref())
        .ok_or("Shade Bridge 的资源目录尚未就绪")?;
    let dependency = canonical_path(dependency).map_err(|error| error.to_string())?;
    bridge
        .validate_external_directory(&dependency)
        .map_err(|error| error.to_string())?;
    let dll = checked_path(&dependency, "ReShade64.dll")?;
    super::gimi_hot_runtime::ensure_stopped(executable)?;
    super::dlss5_hot_runtime::ensure_no_external_host(executable)?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let root = canonical_path(directory).map_err(|error| error.to_string())?;
    let mut composed = super::hot_reshade::compose(registry, &root, &[], "", "")?;
    for (key, value) in [
        ("PresetPath", root.join("ReShadePreset.ini")),
        ("IntermediateCachePath", root.join("cache")),
    ] {
        managed_reshade::set_entry(
            &mut composed.game_ini,
            "GENERAL",
            key,
            &value.to_string_lossy(),
        );
    }
    managed_reshade::set_entry(
        &mut composed.game_ini,
        "ADDON",
        "AddonPath",
        &root.to_string_lossy(),
    );
    if !root.join("ReShade.ini").exists() {
        // 旧独立注入器将状态写在游戏目录。迁移界面、教程和按键，不继承它
        // 重写的 Add-on / Host 路径，也不修改任何游戏目录文件。
        let legacy = executable
            .parent()
            .ok_or("游戏目录无效")?
            .join("ReShade.ini");
        if legacy.is_file() {
            let mut ui = String::new();
            let mut keep = false;
            for line in read_text(&legacy)?.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    keep = ["OVERLAY", "INPUT", "STYLE", "SCREENSHOT"]
                        .iter()
                        .any(|section| trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section));
                }
                if keep {
                    ui.push_str(line);
                    ui.push('\n');
                }
            }
            let current = managed_reshade::ManagedReShadeConfig {
                game_ini: ui,
                preset_ini: composed.preset_ini.clone(),
            };
            composed = managed_reshade::retain_user_state(&composed, None, &current, "", &[]);
        }
    }
    super::hot_reshade::save_config(&root, composed, "", &[])?;
    Ok(Some(HotRuntimePlan {
        dlls: vec![dll],
        environment: [
            (
                "RESHADE_BASE_PATH_OVERRIDE".into(),
                Some(root.to_string_lossy().into_owned()),
            ),
            ("RESHADE_DISABLE_GRAPHICS_HOOK".into(), None),
            ("RESHADE_DISABLE_INPUT_HOOK".into(), None),
            ("GIMI_HOSTED_RESHADE_DLL".into(), None),
            ("GIMI_HOSTED_RESHADE_CONFIG".into(), None),
        ]
        .into_iter()
        .collect(),
        runtime_directory: root,
    }))
}

#[tauri::command]
pub fn prepare_hoyoshade_hot_runtime(
    game_name: String,
    game_executable: String,
) -> Result<Option<HotRuntimePlan>, String> {
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(None);
    };
    let directory = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssets")
        .join(HOYOSHADE_PLUGIN_ID)
        .join("hot")
        .join(format!(
            "{:x}",
            Sha256::digest(game_name.trim().to_lowercase().as_bytes())
        ));
    prepare_hot_at(
        &registry.for_game(&game_name),
        Path::new(&game_executable),
        &directory,
    )
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
                            kind: super::super::ReadyConditionType::ProcessExited,
                            value: None,
                            timeout_ms: 15_000,
                        }),
                        failure_policy: FailurePolicy::Abort,
                    }],
                    ..Default::default()
                },
                external_dependencies: vec![ExternalDependency {
                    id: HOYOSHADE_DEPENDENCY_ID.to_string(),
                    kind: ExternalDependencyType::Directory,
                    required_files: vec![
                        "inject.exe".to_string(),
                        "ReShade64.dll".to_string(),
                        "ReShade.ini".to_string(),
                    ],
                }],
                permissions: vec![
                    PluginPermission::ProcessSpawn,
                    PluginPermission::ProcessObserve,
                ],
            },
            package_root: root.join("package"),
            enabled: true,
            official: false,
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
        fs::write(root.join("ReShade.ini"), b"fixture").unwrap();
        bridge.validate_external_directory(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_targets_known_hoyoverse_game_processes() {
        let root =
            std::env::temp_dir().join(format!("ssmt-hoyoshade-process-{}", std::process::id()));
        let bridge = HoYoShadeBridge::from_installed(&fixture(&root)).unwrap();
        assert!(bridge.supports_process("YuanShen.exe"));
        assert!(bridge.supports_process("StarRail.exe"));
        assert!(!bridge.supports_process("notepad.exe"));
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

    #[test]
    fn standalone_shade_uses_one_ordered_host_and_preserves_state_without_game_writes() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-shade-hot-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        let executable = game.join("StarRail.exe");
        fs::write(&executable, b"fixture").unwrap();
        let managed = root.join("managed");
        assert!(prepare_hot_at(&registry, &executable, &managed)
            .unwrap()
            .is_none());
        assert!(!managed.exists());
        let package = registry
            .plugins_root()
            .join(HOYOSHADE_PLUGIN_ID)
            .join("0.1.0");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("ssmt-plugin.json"), include_str!("../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.0/ssmt-plugin.json")).unwrap();
        let source = root.join("shade");
        for directory in [
            "LauncherResource",
            "InjectResource",
            "Presets",
            "reshade-shaders/Addons",
        ] {
            fs::create_dir_all(source.join(directory)).unwrap();
        }
        for file in [
            "inject.exe",
            "LauncherResource/INIBuild.exe",
            "ReShade64.dll",
            "reshade-shaders/Addons/ShaderToggler.addon64",
        ] {
            fs::write(source.join(file), b"fixture").unwrap();
        }
        let source_ini = "[GENERAL]\nEffectSearchPaths=Shaders\n[ADDON]\nDisabledAddons=ShaderToggler\n[OVERLAY]\nTutorialProgress=0\n";
        fs::write(source.join("ReShade.ini"), source_ini).unwrap();
        fs::write(
            source.join("Presets/Mod OFF.ini"),
            "Techniques=Shade@Shade.fx\n",
        )
        .unwrap();
        let game_ini = "[ADDON]\nAddonPath=C:/old\nDisabledAddons=\n[OVERLAY]\nTutorialProgress=4\n[INPUT]\nKeyOverlay=36,0,0,0\n";
        fs::write(game.join("ReShade.ini"), game_ini).unwrap();
        registry.refresh().unwrap();
        registry
            .set_external_dependency_path(HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID, &source)
            .unwrap();
        assert!(
            prepare_hot_at(&registry.for_game("SRMI"), &executable, &managed)
                .unwrap()
                .is_none()
        );
        assert!(!managed.exists());
        registry
            .set_game_enabled("SRMI", HOYOSHADE_PLUGIN_ID, true)
            .unwrap();
        let plan = prepare_hot_at(&registry.for_game("SRMI"), &executable, &managed)
            .unwrap()
            .unwrap();
        assert_eq!(
            plan.dlls,
            vec![canonical_path(&source.join("ReShade64.dll")).unwrap()]
        );
        assert_eq!(plan.environment["RESHADE_DISABLE_GRAPHICS_HOOK"], None);
        assert_eq!(plan.environment["GIMI_HOSTED_RESHADE_DLL"], None);
        assert_eq!(plan.runtime_directory, canonical_path(&managed).unwrap());
        let config = read_text(&managed.join("ReShade.ini")).unwrap();
        assert!(config.contains("TutorialProgress=4"));
        assert!(config.contains("DisabledAddons=ShaderToggler"));
        assert!(!config.contains("C:/old"));
        assert!(managed.join("ShaderToggler.addon64").is_file());
        let mut updated = config;
        managed_reshade::set_entry(&mut updated, "INPUT", "KeyOverlay", "49,0,0,0");
        fs::write(managed.join("ReShade.ini"), updated).unwrap();
        prepare_hot_at(&registry.for_game("SRMI"), &executable, &managed)
            .unwrap()
            .unwrap();
        assert!(read_text(&managed.join("ReShade.ini"))
            .unwrap()
            .contains("KeyOverlay=49,0,0,0"));
        assert_eq!(read_text(&source.join("ReShade.ini")).unwrap(), source_ini);
        assert_eq!(read_text(&game.join("ReShade.ini")).unwrap(), game_ini);
        fs::write(
            game.join("ReShade.ini"),
            "[INSTALL]\nBasePath=C:/another-host\n",
        )
        .unwrap();
        assert!(prepare_hot_at(
            &registry.for_game("SRMI"),
            &executable,
            &root.join("blocked")
        )
        .unwrap_err()
        .contains("BasePath"));
        assert!(!root.join("blocked").exists());
        registry
            .set_game_enabled("SRMI", HOYOSHADE_PLUGIN_ID, false)
            .unwrap();
        fs::remove_file(source.join("ReShade64.dll")).unwrap();
        assert!(prepare_hot_at(
            &registry.for_game("SRMI"),
            &executable,
            &root.join("disabled")
        )
        .unwrap()
        .is_none());
        assert!(!root.join("disabled").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "需要 SSMT_SHADE_TEST_SOURCE 与 SSMT_SHADE_TEST_GAME；仅检查资源准备，不加载 DLL 或启动游戏"]
    fn real_shade_assets_prepare_without_executing_or_modifying_sources() {
        let source = PathBuf::from(std::env::var("SSMT_SHADE_TEST_SOURCE").unwrap());
        let executable = PathBuf::from(std::env::var("SSMT_SHADE_TEST_GAME").unwrap());
        let source_ini = fs::read(source.join("ReShade.ini")).unwrap();
        let game_ini_path = executable.parent().unwrap().join("ReShade.ini");
        let game_ini = fs::read(&game_ini_path).unwrap();
        let root = std::env::temp_dir().join(format!(
            "ssmt-shade-real-assets-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let package = registry
            .plugins_root()
            .join(HOYOSHADE_PLUGIN_ID)
            .join("0.1.0");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("ssmt-plugin.json"), include_str!("../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.0/ssmt-plugin.json")).unwrap();
        registry.refresh().unwrap();
        registry
            .set_external_dependency_path(HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID, &source)
            .unwrap();
        registry
            .set_game_enabled("SRMI", HOYOSHADE_PLUGIN_ID, true)
            .unwrap();
        let managed = root.join("managed");
        for _ in 0..2 {
            let plan = prepare_hot_at(&registry.for_game("SRMI"), &executable, &managed)
                .unwrap()
                .unwrap();
            assert_eq!(
                plan.dlls,
                vec![canonical_path(&source.join("ReShade64.dll")).unwrap()]
            );
            let config = read_text(&managed.join("ReShade.ini")).unwrap();
            // 源资源的禁用项与游戏已完成的教程状态应进入自有副本，不能被模板重置。
            let defaults = read_text(&source.join("ReShade.ini")).unwrap();
            for line in defaults
                .lines()
                .filter(|line| line.starts_with("DisabledAddons="))
            {
                assert!(config.contains(line));
            }
            let previous = read_text(&game_ini_path).unwrap();
            for line in previous
                .lines()
                .filter(|line| line.starts_with("TutorialProgress="))
            {
                assert!(config.contains(line));
            }
            assert!(managed.join("ReShadePreset.ini").is_file());
            assert!(managed.join("ReShade.ssmt-defaults.json").is_file());
            assert_eq!(fs::read(source.join("ReShade.ini")).unwrap(), source_ini);
            assert_eq!(fs::read(&game_ini_path).unwrap(), game_ini);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
