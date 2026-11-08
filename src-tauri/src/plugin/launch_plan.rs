//! Generic launch-hook resolution and resource arbitration.
//!
//! Plugins declare intent. The host validates package files, target paths and resource
//! ownership, then produces a deterministic plan or an explicit conflict.

use super::file_transaction::{self, FileMutation, FileTransactionReport};
use super::registry::{InstalledPlugin, PluginRegistry};
use super::{LaunchHookAction, LaunchHookContribution};
use crate::config::path_manager::PathManager;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_RESOLVED_ACTIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchHookPlugin {
    pub plugin_id: String,
    pub plugin_version: String,
    pub package_root: PathBuf,
    pub hooks: Vec<LaunchHookContribution>,
}

impl LaunchHookPlugin {
    pub fn from_installed(plugin: &InstalledPlugin) -> Self {
        Self {
            plugin_id: plugin.manifest.id.clone(),
            plugin_version: plugin.manifest.version.clone(),
            package_root: plugin.package_root.clone(),
            hooks: plugin.manifest.contributions.launch_hooks.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLaunchFileAction {
    pub action_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub hook_id: String,
    pub priority: i32,
    pub source: PathBuf,
    pub target: PathBuf,
    pub target_relative: String,
    pub resource_keys: Vec<String>,
    pub expected_source_sha256: String,
    pub expected_target_sha256: Option<String>,
    #[serde(skip)]
    pub remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLaunchArgumentAppend {
    pub action_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub hook_id: String,
    pub priority: i32,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchActionCandidate {
    pub plugin_id: String,
    pub plugin_version: String,
    pub hook_id: String,
    pub priority: i32,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResourceConflict {
    pub resource_key: String,
    pub reason: String,
    pub candidates: Vec<LaunchActionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedLaunchAction {
    pub plugin_id: String,
    pub plugin_version: String,
    pub hook_id: String,
    pub priority: i32,
    pub target: Option<String>,
    pub resource_keys: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedHookPlan {
    pub file_actions: Vec<ResolvedLaunchFileAction>,
    pub argument_appends: Vec<ResolvedLaunchArgumentAppend>,
    pub skipped_actions: Vec<SkippedLaunchAction>,
    pub conflicts: Vec<LaunchResourceConflict>,
}

impl ResolvedHookPlan {
    pub fn is_executable(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub fn file_mutations(&self) -> Vec<FileMutation> {
        self.file_actions
            .iter()
            .map(|action| FileMutation {
                target: action.target_relative.clone(),
                source: (!action.remove).then(|| action.source.clone()),
                expected_before_sha256: action.expected_target_sha256.clone(),
            })
            .collect()
    }

    pub fn apply_file_actions(&self, game_root: &Path) -> Result<FileTransactionReport, String> {
        if !self.is_executable() {
            return Err("launch plan contains unresolved resource conflicts".to_string());
        }
        let mutations = self.file_mutations();
        if mutations.is_empty() {
            return Err("launch plan contains no file actions".to_string());
        }
        file_transaction::apply(game_root, &mutations)
    }

    pub fn restore_file_actions(game_root: &Path, transaction_id: &str) -> Result<bool, String> {
        file_transaction::restore_applied(game_root, transaction_id)
    }
}

#[derive(Debug, Clone)]
enum PendingAction {
    File(ResolvedLaunchFileAction),
    Argument(ResolvedLaunchArgumentAppend),
}

impl PendingAction {
    fn candidate(&self) -> LaunchActionCandidate {
        match self {
            Self::File(action) => LaunchActionCandidate {
                plugin_id: action.plugin_id.clone(),
                plugin_version: action.plugin_version.clone(),
                hook_id: action.hook_id.clone(),
                priority: action.priority,
                target: Some(action.target_relative.clone()),
            },
            Self::Argument(action) => LaunchActionCandidate {
                plugin_id: action.plugin_id.clone(),
                plugin_version: action.plugin_version.clone(),
                hook_id: action.hook_id.clone(),
                priority: action.priority,
                target: None,
            },
        }
    }

    fn priority(&self) -> i32 {
        match self {
            Self::File(action) => action.priority,
            Self::Argument(action) => action.priority,
        }
    }
}

pub fn resolve_hook_plan(
    game_root: &Path,
    plugins: &[LaunchHookPlugin],
) -> Result<ResolvedHookPlan, String> {
    let game_root = game_root
        .canonicalize()
        .map_err(|error| format!("game directory is unavailable: {error}"))?;
    let metadata = fs::symlink_metadata(&game_root).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || reparse(&metadata) {
        return Err(format!(
            "game root is not a regular directory: {}",
            game_root.display()
        ));
    }

    let mut actions = Vec::new();
    for plugin in plugins {
        for hook in &plugin.hooks {
            for (action_index, action) in hook.actions.iter().enumerate() {
                let action_id = format!("{}:{}:{action_index}", plugin.plugin_id, hook.id);
                actions.push(resolve_action(&game_root, plugin, hook, action_id, action)?);
                if actions.len() > MAX_RESOLVED_ACTIONS {
                    return Err(format!(
                        "launch plan exceeds {MAX_RESOLVED_ACTIONS} actions"
                    ));
                }
            }
        }
    }

    let applied_targets = file_transaction::applied_targets(&game_root)?;
    let active_blocked: HashMap<usize, String> = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| match action {
            PendingAction::File(file) => applied_targets
                .get(&file.target_relative.replace('\\', "/").to_ascii_lowercase())
                .cloned()
                .map(|transaction| (index, transaction)),
            PendingAction::Argument(_) => None,
        })
        .collect();

    let mut resources: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, action) in actions.iter().enumerate() {
        if active_blocked.contains_key(&index) {
            continue;
        }
        if let PendingAction::File(file) = action {
            for resource in &file.resource_keys {
                resources.entry(resource.clone()).or_default().push(index);
            }
        }
    }

    let mut winners = HashMap::<String, usize>::new();
    let mut blocked = active_blocked.keys().copied().collect::<HashSet<_>>();
    let mut conflicts = active_blocked
        .iter()
        .map(|(index, transaction_id)| {
            let action = &actions[*index];
            LaunchResourceConflict {
                resource_key: match action {
                    PendingAction::File(file) => format!(
                        "file:{}",
                        file.target_relative.replace('\\', "/").to_ascii_lowercase()
                    ),
                    PendingAction::Argument(_) => "argument".to_string(),
                },
                reason: format!("resource is owned by applied launch deployment {transaction_id}"),
                candidates: vec![action.candidate()],
            }
        })
        .collect::<Vec<_>>();
    for (resource, candidates) in &resources {
        if candidates.len() < 2 {
            continue;
        }
        let max_priority = candidates
            .iter()
            .map(|index| actions[*index].priority())
            .max()
            .unwrap_or_default();
        let top: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|index| actions[*index].priority() == max_priority)
            .collect();
        if top.len() == 1 {
            winners.insert(resource.clone(), top[0]);
        } else {
            blocked.extend(top.iter().copied());
            conflicts.push(LaunchResourceConflict {
                resource_key: resource.clone(),
                reason: "multiple plugins claim this resource at the same priority".to_string(),
                candidates: top
                    .iter()
                    .map(|index| actions[*index].candidate())
                    .collect(),
            });
        }
    }
    conflicts.sort_by(|left, right| left.resource_key.cmp(&right.resource_key));

    let mut plan = ResolvedHookPlan {
        conflicts,
        ..Default::default()
    };
    for (index, action) in actions.iter().enumerate() {
        if let PendingAction::File(file) = action {
            if blocked.contains(&index) {
                plan.skipped_actions.push(SkippedLaunchAction {
                    plugin_id: file.plugin_id.clone(),
                    plugin_version: file.plugin_version.clone(),
                    hook_id: file.hook_id.clone(),
                    priority: file.priority,
                    target: Some(file.target_relative.clone()),
                    resource_keys: file.resource_keys.clone(),
                    reason: active_blocked
                        .get(&index)
                        .map(|transaction| {
                            format!("owned by applied launch deployment {transaction}")
                        })
                        .unwrap_or_else(|| "resource conflict".to_string()),
                });
                continue;
            }
            let lost = file.resource_keys.iter().any(|resource| {
                resources
                    .get(resource)
                    .is_some_and(|group| group.len() > 1 && winners.get(resource) != Some(&index))
            });
            if lost {
                plan.skipped_actions.push(SkippedLaunchAction {
                    plugin_id: file.plugin_id.clone(),
                    plugin_version: file.plugin_version.clone(),
                    hook_id: file.hook_id.clone(),
                    priority: file.priority,
                    target: Some(file.target_relative.clone()),
                    resource_keys: file.resource_keys.clone(),
                    reason: "a higher-priority plugin owns the resource".to_string(),
                });
            } else if plan.conflicts.is_empty() {
                plan.file_actions.push(file.clone());
            }
        } else if let PendingAction::Argument(argument) = action {
            plan.argument_appends.push(argument.clone());
        }
    }

    if !plan.conflicts.is_empty() {
        plan.file_actions.clear();
    }
    plan.file_actions.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.cmp(&right.plugin_id))
            .then_with(|| left.hook_id.cmp(&right.hook_id))
            .then_with(|| left.target_relative.cmp(&right.target_relative))
    });
    plan.argument_appends.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.cmp(&right.plugin_id))
            .then_with(|| left.hook_id.cmp(&right.hook_id))
            .then_with(|| left.action_id.cmp(&right.action_id))
    });
    plan.skipped_actions.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then_with(|| left.hook_id.cmp(&right.hook_id))
            .then_with(|| left.target.cmp(&right.target))
    });
    Ok(plan)
}

fn resolve_action(
    game_root: &Path,
    plugin: &LaunchHookPlugin,
    hook: &LaunchHookContribution,
    action_id: String,
    action: &LaunchHookAction,
) -> Result<PendingAction, String> {
    match action {
        LaunchHookAction::FileCopy {
            source,
            target,
            sha256,
            resource,
            expected_target_sha256,
        } => {
            let package_root = plugin
                .package_root
                .canonicalize()
                .map_err(|error| format!("plugin package is unavailable: {error}"))?;
            let source = package_root.join(source);
            let source = source
                .canonicalize()
                .map_err(|error| format!("launch source is missing: {error}"))?;
            if !source.starts_with(&package_root) {
                return Err("launch source escapes its plugin package".to_string());
            }
            checked_parents(&package_root, &source, "launch source")?;
            require_regular_file(&source)?;
            let actual = hash_file(&source)?;
            if actual != sha256.to_ascii_lowercase() {
                return Err(format!(
                    "launch source checksum mismatch: {}",
                    source.display()
                ));
            }
            let target = preview_target(game_root, target)?;
            let actual_target = hash_optional(&target)?;
            let expected_target = expected_target_sha256
                .as_ref()
                .map(|value| value.to_ascii_lowercase());
            if expected_target.is_some() && actual_target != expected_target {
                return Err(format!(
                    "launch target does not match its declared baseline: {}",
                    target.display()
                ));
            }
            Ok(PendingAction::File(ResolvedLaunchFileAction {
                action_id,
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                hook_id: hook.id.clone(),
                priority: hook.priority,
                source,
                target_relative: target_relative(game_root, target.clone())?,
                target,
                resource_keys: resource_keys(action, resource)?,
                expected_source_sha256: actual,
                expected_target_sha256: expected_target,
                remove: false,
            }))
        }
        LaunchHookAction::FileRemove {
            target,
            resource,
            expected_target_sha256,
        } => {
            let target = preview_target(game_root, target)?;
            let expected_target = expected_target_sha256
                .as_ref()
                .map(|value| value.to_ascii_lowercase());
            if expected_target.is_some() && hash_optional(&target)? != expected_target {
                return Err(format!(
                    "launch removal target does not match its declared baseline: {}",
                    target.display()
                ));
            }
            Ok(PendingAction::File(ResolvedLaunchFileAction {
                action_id,
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                hook_id: hook.id.clone(),
                priority: hook.priority,
                source: PathBuf::new(),
                target: target.clone(),
                target_relative: target_relative(game_root, target)?,
                resource_keys: resource_keys(action, resource)?,
                expected_source_sha256: String::new(),
                expected_target_sha256: expected_target,
                remove: true,
            }))
        }
        LaunchHookAction::ArgumentAppend { values } => {
            Ok(PendingAction::Argument(ResolvedLaunchArgumentAppend {
                action_id,
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                hook_id: hook.id.clone(),
                priority: hook.priority,
                values: values.clone(),
            }))
        }
    }
}

fn target_name(action: &LaunchHookAction) -> Result<&str, String> {
    match action {
        LaunchHookAction::FileCopy { target, .. } | LaunchHookAction::FileRemove { target, .. } => {
            Ok(target)
        }
        LaunchHookAction::ArgumentAppend { .. } => Err("action has no target".to_string()),
    }
}

fn target_relative(game_root: &Path, target: PathBuf) -> Result<String, String> {
    target
        .strip_prefix(game_root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .map_err(|_| "launch target escapes the game directory".to_string())
}

fn resource_keys(
    action: &LaunchHookAction,
    explicit: &Option<String>,
) -> Result<Vec<String>, String> {
    let target = target_name(action)?;
    let mut keys = vec![format!(
        "file:{}",
        target.replace('\\', "/").to_ascii_lowercase()
    )];
    if let Some(resource) = explicit {
        keys.push(format!("plugin:{}", resource.to_ascii_lowercase()));
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

fn preview_target(game_root: &Path, relative: &str) -> Result<PathBuf, String> {
    super::validate_package_relative_path(relative).map_err(|error| error.to_string())?;
    let target = game_root.join(relative);
    checked_parents(game_root, &target, "launch target")?;
    let parent = target
        .parent()
        .ok_or_else(|| "launch target has no parent".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("launch target parent is missing: {error}"))?;
    if !parent.starts_with(game_root) {
        return Err("launch target escapes the game directory".to_string());
    }
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if !meta.is_file() || reparse(&meta) {
            return Err(format!("unsafe launch target: {}", target.display()));
        }
    }
    Ok(target)
}

fn checked_parents(root: &Path, target: &Path, label: &str) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{label} has no parent"))?;
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| format!("{label} escapes its root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let meta = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(format!(
                "{label} parent is not a regular directory: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn hash_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !reparse(&meta) => hash_file(path).map(Some),
        Ok(_) => Err(format!("unsafe launch target: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn require_regular_file(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|error| format!("missing launch file {}: {error}", path.display()))?;
    if !meta.is_file() || reparse(&meta) {
        return Err(format!("unsafe launch file: {}", path.display()));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(windows)]
fn reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

pub fn preview_for_game(game_name: &str) -> Result<ResolvedHookPlan, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let scoped = registry.for_game(game_name);
    let plugins: Vec<_> = scoped
        .selected_plugins()
        .filter(|plugin| plugin.enabled && !plugin.manifest.contributions.launch_hooks.is_empty())
        .map(LaunchHookPlugin::from_installed)
        .collect();
    let game_root = configured_game_root(game_name)?;
    resolve_hook_plan(&game_root, &plugins)
}

#[tauri::command]
pub fn plugin_launch_plan_preview(game_name: String) -> Result<ResolvedHookPlan, String> {
    preview_for_game(&game_name)
}

fn configured_game_root(game_name: &str) -> Result<PathBuf, String> {
    let config_path = PathManager::global_config_games_game_folder(game_name).join("Config.json");
    let config: Value = serde_json::from_slice(
        &fs::read(&config_path)
            .map_err(|error| format!("cannot read {}: {error}", config_path.display()))?,
    )
    .map_err(|error| format!("invalid {}: {error}", config_path.display()))?;
    if let Some(target) = config.get("targetExePath").and_then(Value::as_str) {
        let target = PathBuf::from(target);
        if let Some(parent) = target.parent().filter(|parent| parent.is_dir()) {
            return Ok(parent.to_path_buf());
        }
    }
    if let Some(install_dir) = config.get("installDir").and_then(Value::as_str) {
        let install_dir = PathBuf::from(install_dir);
        if install_dir.is_dir() {
            return Ok(install_dir);
        }
    }
    Err(format!("game directory is not configured for {game_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plugin::LaunchHookAction, plugin::LaunchHookContribution};

    fn fixture(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ssmt-launch-plan-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("game");
        let package = root.join("package");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("payload.dll"), b"payload").unwrap();
        (game, package)
    }

    fn hook(priority: i32, sha256: String) -> LaunchHookContribution {
        LaunchHookContribution {
            id: "install".into(),
            priority,
            actions: vec![LaunchHookAction::FileCopy {
                source: "payload.dll".into(),
                target: "dxgi.dll".into(),
                sha256,
                resource: Some("proxy:dxgi".into()),
                expected_target_sha256: None,
            }],
        }
    }

    #[test]
    fn equal_priority_resource_claims_are_a_deterministic_conflict() {
        let (game, first_package) = fixture("equal");
        let second_package = first_package.with_file_name("package-two");
        fs::create_dir(&second_package).unwrap();
        fs::write(second_package.join("payload.dll"), b"payload").unwrap();
        let sha256 = hash_file(&first_package.join("payload.dll")).unwrap();
        let plugins = vec![
            LaunchHookPlugin {
                plugin_id: "example.alpha".into(),
                plugin_version: "1.0.0".into(),
                package_root: first_package,
                hooks: vec![hook(10, sha256.clone())],
            },
            LaunchHookPlugin {
                plugin_id: "example.beta".into(),
                plugin_version: "1.0.0".into(),
                package_root: second_package,
                hooks: vec![hook(10, sha256)],
            },
        ];
        let plan = resolve_hook_plan(&game, &plugins).unwrap();
        assert!(!plan.is_executable());
        assert!(plan.file_actions.is_empty());
        assert!(plan
            .conflicts
            .iter()
            .any(|conflict| conflict.resource_key == "plugin:proxy:dxgi"));
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }

    #[test]
    fn higher_priority_wins_and_lower_action_is_reported() {
        let (game, first_package) = fixture("priority");
        let second_package = first_package.with_file_name("package-two");
        fs::create_dir(&second_package).unwrap();
        fs::write(second_package.join("payload.dll"), b"payload").unwrap();
        let sha256 = hash_file(&first_package.join("payload.dll")).unwrap();
        let plugins = vec![
            LaunchHookPlugin {
                plugin_id: "example.alpha".into(),
                plugin_version: "1.0.0".into(),
                package_root: first_package,
                hooks: vec![hook(20, sha256.clone())],
            },
            LaunchHookPlugin {
                plugin_id: "example.beta".into(),
                plugin_version: "1.0.0".into(),
                package_root: second_package,
                hooks: vec![hook(10, sha256)],
            },
        ];
        let plan = resolve_hook_plan(&game, &plugins).unwrap();
        assert!(plan.is_executable());
        assert_eq!(plan.file_actions.len(), 1);
        assert_eq!(plan.file_actions[0].plugin_id, "example.alpha");
        assert_eq!(plan.skipped_actions.len(), 1);
        assert_eq!(plan.skipped_actions[0].plugin_id, "example.beta");
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_source_checksum_mismatch() {
        let (game, package) = fixture("hash");
        let plugin = LaunchHookPlugin {
            plugin_id: "example.alpha".into(),
            plugin_version: "1.0.0".into(),
            package_root: package,
            hooks: vec![hook(10, "0".repeat(64))],
        };
        assert!(resolve_hook_plan(&game, &[plugin])
            .unwrap_err()
            .contains("checksum mismatch"));
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }

    #[test]
    fn argument_hooks_are_ordered_deterministically() {
        let (game, package) = fixture("args");
        let plugins = vec![
            LaunchHookPlugin {
                plugin_id: "example.beta".into(),
                plugin_version: "1.0.0".into(),
                package_root: package.clone(),
                hooks: vec![LaunchHookContribution {
                    id: "args".into(),
                    priority: 10,
                    actions: vec![LaunchHookAction::ArgumentAppend {
                        values: vec!["--beta".into()],
                    }],
                }],
            },
            LaunchHookPlugin {
                plugin_id: "example.alpha".into(),
                plugin_version: "1.0.0".into(),
                package_root: package,
                hooks: vec![LaunchHookContribution {
                    id: "args".into(),
                    priority: 10,
                    actions: vec![LaunchHookAction::ArgumentAppend {
                        values: vec!["--alpha".into()],
                    }],
                }],
            },
        ];
        let plan = resolve_hook_plan(&game, &plugins).unwrap();
        assert_eq!(
            plan.argument_appends
                .iter()
                .map(|append| append.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["example.alpha", "example.beta"]
        );
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }

    #[test]
    fn resolved_plan_applies_and_restores_files() {
        let (game, package) = fixture("apply");
        let sha256 = hash_file(&package.join("payload.dll")).unwrap();
        let plugin = LaunchHookPlugin {
            plugin_id: "example.alpha".into(),
            plugin_version: "1.0.0".into(),
            package_root: package,
            hooks: vec![hook(10, sha256)],
        };
        let plan = resolve_hook_plan(&game, &[plugin]).unwrap();
        let report = plan.apply_file_actions(&game).unwrap();
        assert_eq!(fs::read(game.join("dxgi.dll")).unwrap(), b"payload");
        assert!(ResolvedHookPlan::restore_file_actions(&game, &report.transaction_id).unwrap());
        assert!(!game.join("dxgi.dll").exists());
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }

    #[test]
    fn applied_deployment_blocks_a_second_plan_until_restored() {
        let (game, package) = fixture("active-owner");
        let sha256 = hash_file(&package.join("payload.dll")).unwrap();
        let plugin = LaunchHookPlugin {
            plugin_id: "example.alpha".into(),
            plugin_version: "1.0.0".into(),
            package_root: package.clone(),
            hooks: vec![hook(10, sha256.clone())],
        };
        let first = resolve_hook_plan(&game, &[plugin]).unwrap();
        let report = first.apply_file_actions(&game).unwrap();
        let second = resolve_hook_plan(
            &game,
            &[LaunchHookPlugin {
                plugin_id: "example.beta".into(),
                plugin_version: "1.0.0".into(),
                package_root: package,
                hooks: vec![hook(10, sha256)],
            }],
        )
        .unwrap();
        assert!(!second.is_executable());
        assert!(second.conflicts.iter().any(|conflict| {
            conflict.reason.contains(&report.transaction_id)
                && conflict.resource_key == "file:dxgi.dll"
        }));
        assert!(ResolvedHookPlan::restore_file_actions(&game, &report.transaction_id).unwrap());
        fs::remove_dir_all(game.parent().unwrap()).unwrap();
    }
}
