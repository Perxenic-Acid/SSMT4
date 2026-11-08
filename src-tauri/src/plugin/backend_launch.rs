//! 后台贡献的启动计划。共享数据保持不透明，宿主只检查权限、路径和资源所有权。
use super::{
    backend,
    registry::{InstalledPlugin, PluginRegistry},
    PluginPermission,
};
use crate::{commands::launch_logs::LaunchLogSource, config::path_manager::PathManager};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchDescription {
    #[serde(default)]
    pub exports: BTreeMap<String, Value>,
    pub launch: Option<LaunchClaim>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchClaim {
    pub priority: i32,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchDll {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeLaunchPlan {
    #[serde(default)]
    pub dlls: Vec<LaunchDll>,
    #[serde(default)]
    pub environment: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub required_launch_args: Vec<String>,
    #[serde(default)]
    pub excluded_launch_args: Vec<String>,
    #[serde(default)]
    pub log_sources: Vec<LaunchLogSource>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendLaunchPreparation {
    pub plans: Vec<OwnedLaunchPlan>,
    pub diagnostics: Vec<LaunchDiagnostic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedLaunchPlan {
    pub plugin_id: String,
    pub plugin_name: String,
    pub plan: RuntimeLaunchPlan,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchDiagnostic {
    pub plugin_id: String,
    pub plugin_name: String,
    pub error: String,
}

fn declares(plugin: &InstalledPlugin, event: &str) -> bool {
    plugin
        .manifest
        .contributions
        .backend
        .as_ref()
        .is_some_and(|backend| backend.events.iter().any(|item| item == event))
}

fn validate_description(description: &LaunchDescription) -> Result<(), String> {
    if description.exports.len() > 16 {
        return Err("too many backend exports".into());
    }
    for channel in description.exports.keys() {
        super::validate_plugin_id(channel).map_err(|error| error.to_string())?;
    }
    if let Some(claim) = &description.launch {
        if claim.resources.is_empty() || claim.resources.len() > 32 {
            return Err("launch requires between 1 and 32 resource claims".into());
        }
        let mut seen = BTreeSet::new();
        for resource in &claim.resources {
            super::validate_plugin_id(resource).map_err(|error| error.to_string())?;
            if !seen.insert(resource.to_ascii_lowercase()) {
                return Err("duplicate launch resource claim".into());
            }
        }
    }
    Ok(())
}

/// 同优先级的最高占用者存在歧义时拒绝该资源；准备失败后可重算低优先级候选。
fn select_candidates(
    claims: &[(String, LaunchClaim)],
    excluded: &BTreeSet<String>,
) -> (Vec<String>, BTreeSet<String>) {
    let mut candidates: Vec<_> = claims
        .iter()
        .filter(|(id, _)| !excluded.contains(id))
        .collect();
    candidates.sort_by(|left, right| {
        right
            .1
            .priority
            .cmp(&left.1.priority)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut selected = Vec::new();
    let mut conflicts = BTreeSet::new();
    let mut occupied = BTreeSet::new();
    let mut start = 0;
    while start < candidates.len() {
        let priority = candidates[start].1.priority;
        let end = start
            + candidates[start..]
                .iter()
                .take_while(|(_, claim)| claim.priority == priority)
                .count();
        // 已输给较高优先级的整项候选不再参与其他资源的同级冲突。
        let eligible: Vec<_> = candidates[start..end]
            .iter()
            .filter(|(_, claim)| {
                !claim
                    .resources
                    .iter()
                    .any(|resource| occupied.contains(&resource.to_ascii_lowercase()))
            })
            .collect();
        let mut owners = BTreeMap::<String, Vec<String>>::new();
        for (id, claim) in &eligible {
            for resource in &claim.resources {
                owners
                    .entry(resource.to_ascii_lowercase())
                    .or_default()
                    .push(id.clone());
            }
        }
        for owners in owners.values().filter(|owners| owners.len() > 1) {
            conflicts.extend(owners.iter().cloned());
        }
        for (id, claim) in eligible {
            occupied.extend(
                claim
                    .resources
                    .iter()
                    .map(|resource| resource.to_ascii_lowercase()),
            );
            if !conflicts.contains(id) {
                selected.push(id.clone());
            }
        }
        start = end;
    }
    (selected, conflicts)
}

fn owned_roots(plugin: &InstalledPlugin) -> Vec<PathBuf> {
    std::iter::once(plugin.package_root.clone())
        .chain(std::iter::once(
            PathManager::ssmt_global_config_folder()
                .join("PluginAssets")
                .join(plugin.id()),
        ))
        .chain(std::iter::once(
            PathManager::ssmt_global_config_folder()
                .join("PluginData")
                .join(plugin.id())
                .join("backend"),
        ))
        .chain(
            plugin
                .external_dependencies
                .values()
                .filter(|dependency| {
                    dependency.status == super::registry::ExternalDependencyStatus::Ready
                })
                .filter_map(|dependency| dependency.path.clone()),
        )
        .filter_map(|path| path.canonicalize().ok())
        .collect()
}

fn checked_owned_path(path: &Path, roots: &[PathBuf], must_exist: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("launch path must be absolute".into());
    }
    let canonical = if must_exist || path.exists() {
        path.canonicalize().map_err(|error| error.to_string())?
    } else {
        // 日志尚未创建时只允许已存在目录下的单个普通文件名。
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("invalid launch path")?;
        super::validate_package_relative_path(name).map_err(|error| error.to_string())?;
        path.parent()
            .ok_or("invalid launch path")?
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join(name)
    };
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("launch path is outside plugin-owned roots".into());
    }
    if canonical.exists() && !canonical.is_file() {
        return Err("launch path is not a file".into());
    }
    Ok(canonical)
}

fn validate_plan(plugin: &InstalledPlugin, plan: &RuntimeLaunchPlan) -> Result<(), String> {
    if !plugin
        .manifest
        .permissions
        .contains(&PluginPermission::GameLaunch)
    {
        return Err("game.launch permission is required".into());
    }
    if plan.dlls.len() > 32
        || plan.environment.len() > 64
        || plan.log_sources.len() > 16
        || plan.required_launch_args.len() > 32
        || plan.excluded_launch_args.len() > 32
    {
        return Err("launch plan exceeds limits".into());
    }
    if !plan.dlls.is_empty()
        && (!plugin
            .manifest
            .permissions
            .contains(&PluginPermission::NativeInject)
            || !plugin
                .manifest
                .permissions
                .contains(&PluginPermission::FilesystemRead)
            || !super::commands::has_native_injection_risk_ack(plugin))
    {
        return Err("native injection permission and risk acknowledgement are required".into());
    }
    let roots = owned_roots(plugin);
    for dll in &plan.dlls {
        let path = checked_owned_path(&dll.path, &roots, true)?;
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
            || dll.sha256.len() != 64
            || !dll.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid launch DLL declaration".into());
        }
        super::marketplace::verify_package_sha256(&path, &dll.sha256)
            .map_err(|error| error.to_string())?;
    }
    for log in &plan.log_sources {
        if log.name.is_empty() || log.name.len() > 128 {
            return Err("invalid launch log name".into());
        }
        checked_owned_path(&log.path, &roots, false)?;
    }
    for (name, value) in &plan.environment {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || name.as_bytes()[0].is_ascii_digit()
            || value
                .as_ref()
                .is_some_and(|value| value.len() > 32767 || value.contains(['\0', '\r', '\n', '"']))
        {
            return Err("invalid launch environment declaration".into());
        }
    }
    for argument in plan
        .required_launch_args
        .iter()
        .chain(&plan.excluded_launch_args)
    {
        if !argument.starts_with('-')
            || argument.len() < 2
            || argument.len() > 128
            || !argument
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("invalid launch argument flag".into());
        }
    }
    Ok(())
}

fn check_plan_conflicts(
    existing: &[OwnedLaunchPlan],
    next: &RuntimeLaunchPlan,
) -> Result<(), String> {
    let mut environment = BTreeMap::new();
    for (key, value) in &next.environment {
        if let Some(previous) = environment.insert(key.to_ascii_lowercase(), value) {
            if previous != value {
                return Err("launch environment aliases have different values".into());
            }
        }
    }
    if next.required_launch_args.iter().any(|flag| {
        next.excluded_launch_args
            .iter()
            .any(|other| flag.eq_ignore_ascii_case(other))
    }) {
        return Err("launch plan both requires and excludes the same argument".into());
    }
    let dll_key = |path: &Path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    };
    let mut names = BTreeSet::new();
    for dll in &next.dlls {
        if !names.insert(dll_key(&dll.path))
            || existing.iter().any(|entry| {
                entry
                    .plan
                    .dlls
                    .iter()
                    .any(|other| dll_key(&other.path) == dll_key(&dll.path))
            })
        {
            return Err("multiple launch plans own the same DLL filename".into());
        }
    }
    for existing in existing {
        for (key, value) in &next.environment {
            if existing
                .plan
                .environment
                .iter()
                .any(|(other_key, other_value)| {
                    key.eq_ignore_ascii_case(other_key) && value != other_value
                })
            {
                return Err(format!("launch environment ownership conflict: {key}"));
            }
        }
        if next.required_launch_args.iter().any(|flag| {
            existing
                .plan
                .excluded_launch_args
                .iter()
                .any(|other| flag.eq_ignore_ascii_case(other))
        }) || next.excluded_launch_args.iter().any(|flag| {
            existing
                .plan
                .required_launch_args
                .iter()
                .any(|other| flag.eq_ignore_ascii_case(other))
        }) {
            return Err("launch argument ownership conflict".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn prepare_plugin_backend_launch(
    game_name: String,
    game_executable: String,
    allow_preload: bool,
    existing_dlls: Option<Vec<PathBuf>>,
) -> Result<BackendLaunchPreparation, String> {
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(BackendLaunchPreparation::default());
    };
    prepare_for_registry(
        &registry.for_game(&game_name),
        &game_name,
        &game_executable,
        allow_preload,
        existing_dlls.as_deref().unwrap_or_default(),
    )
    .await
}

async fn prepare_for_registry(
    registry: &PluginRegistry,
    game: &str,
    executable: &str,
    allow_preload: bool,
    existing_dlls: &[PathBuf],
) -> Result<BackendLaunchPreparation, String> {
    let plugins: Vec<_> = registry
        .selected_plugins()
        .filter(|plugin| plugin.enabled)
        .collect();
    prepare_plugins(
        &plugins,
        game,
        executable,
        allow_preload,
        existing_dlls,
        |plugin, game, method, payload| {
            Box::pin(backend::request(plugin, game, method, payload, true))
        },
    )
    .await
}

type BackendFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send + 'a>>;

async fn prepare_plugins<F>(
    plugins: &[&InstalledPlugin],
    game: &str,
    executable: &str,
    allow_preload: bool,
    existing_dlls: &[PathBuf],
    request: F,
) -> Result<BackendLaunchPreparation, String>
where
    F: for<'a> Fn(&'a InstalledPlugin, &'a str, &'a str, Value) -> BackendFuture<'a>,
{
    let mut result = BackendLaunchPreparation::default();
    let baseline = OwnedLaunchPlan {
        plugin_id: "host.launch-configuration".into(),
        plugin_name: "Launch configuration".into(),
        plan: RuntimeLaunchPlan {
            dlls: existing_dlls
                .iter()
                .map(|path| LaunchDll {
                    path: path.clone(),
                    sha256: String::new(),
                })
                .collect(),
            ..Default::default()
        },
    };
    let mut descriptions = BTreeMap::new();
    let context = json!({"gameExecutable":executable, "allowPreload":allow_preload,"existingDlls":existing_dlls});
    let diagnostic = |plugin: &InstalledPlugin, error: String| LaunchDiagnostic {
        plugin_id: plugin.id().into(),
        plugin_name: plugin.manifest.name.clone(),
        error,
    };
    for plugin in plugins
        .iter()
        .copied()
        .filter(|plugin| plugin.enabled && declares(plugin, "launch.describe"))
    {
        if !backend::launch_allowed(game, plugin) {
            continue;
        }
        if plugin
            .manifest
            .permissions
            .contains(&PluginPermission::NativeInject)
            && !super::commands::has_native_injection_risk_ack(plugin)
        {
            result.diagnostics.push(diagnostic(
                plugin,
                "native injection risk acknowledgement is required".into(),
            ));
            continue;
        }
        let description = async {
            let value = request(plugin, game, "launch.describe", context.clone()).await?;
            let description: LaunchDescription =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            validate_description(&description)?;
            if description.launch.is_some()
                && (!declares(plugin, "launch.prepare") || !declares(plugin, "launch.cancel"))
            {
                return Err("launch backend must declare launch.prepare and launch.cancel".into());
            }
            Ok(description)
        }
        .await;
        match description {
            Ok(description) => {
                descriptions.insert(plugin.id().to_owned(), (plugin, description));
            }
            Err(error) => result.diagnostics.push(diagnostic(plugin, error)),
        }
    }
    let claims: Vec<_> = descriptions
        .iter()
        .filter_map(|(id, (_, description))| {
            description.launch.clone().map(|claim| (id.clone(), claim))
        })
        .collect();
    let mut excluded = BTreeSet::new();
    let mut completed = BTreeSet::new();
    loop {
        let (selected, conflicts) = select_candidates(&claims, &excluded);
        for id in &conflicts {
            if !result
                .diagnostics
                .iter()
                .any(|entry| &entry.plugin_id == id)
            {
                result.diagnostics.push(diagnostic(
                    descriptions[id].0,
                    "equal-priority launch resource conflict".into(),
                ));
            }
        }
        let peers: Vec<_> = descriptions.iter().filter(|(id, _)| !excluded.contains(*id)).map(|(id, (plugin, description))| json!({
            "pluginId":id,"pluginVersion":plugin.version(),"exports":description.exports,"selected":selected.contains(id),
            "permissions":plugin.manifest.permissions,
            "nativeInjectionAcknowledged":super::commands::has_native_injection_risk_ack(plugin)
        })).collect();
        let mut retry = false;
        for id in selected {
            if completed.contains(&id) {
                continue;
            }
            let plugin = descriptions[&id].0;
            let prepare = async {
                if !allow_preload {
                    return Err("current launch mode cannot apply a preload plan".into());
                }
                let payload = json!({"launchContext":context,"peers":peers});
                let value = request(plugin, game, "launch.prepare", payload.clone()).await?;
                let plan: RuntimeLaunchPlan =
                    serde_json::from_value(value).map_err(|error| error.to_string())?;
                validate_plan(plugin, &plan)?;
                check_plan_conflicts(std::slice::from_ref(&baseline), &plan)?;
                check_plan_conflicts(&result.plans, &plan)?;
                if declares(plugin, "launch.commit") {
                    request(plugin, game, "launch.commit", payload).await?;
                }
                Ok(plan)
            }
            .await;
            match prepare {
                Ok(plan) => {
                    completed.insert(id.clone());
                    result.plans.push(OwnedLaunchPlan {
                        plugin_id: id,
                        plugin_name: plugin.manifest.name.clone(),
                        plan,
                    });
                }
                Err(error) => {
                    excluded.insert(id);
                    let mut rollback_failed = false;
                    result.diagnostics.push(diagnostic(plugin, error));
                    if let Err(error) = request(
                        plugin,
                        game,
                        "launch.cancel",
                        json!({"launchContext":context}),
                    )
                    .await
                    {
                        rollback_failed = true;
                        result.diagnostics.push(diagnostic(plugin, error));
                    }
                    // 已准备的其他插件可能消费过失败插件的共享数据，全部回滚后重新准备。
                    for prepared in result.plans.drain(..) {
                        let owner = descriptions[&prepared.plugin_id].0;
                        if let Err(error) = request(
                            owner,
                            game,
                            "launch.cancel",
                            json!({"launchContext":context}),
                        )
                        .await
                        {
                            rollback_failed = true;
                            excluded.insert(prepared.plugin_id);
                            result.diagnostics.push(diagnostic(owner, error));
                        }
                    }
                    completed.clear();
                    if rollback_failed {
                        return Err(
                            "plugin launch rollback failed; no fallback plan was applied".into(),
                        );
                    }
                    retry = true;
                    break;
                }
            }
        }
        if !retry {
            break;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claim(priority: i32, resources: &[&str]) -> LaunchClaim {
        LaunchClaim {
            priority,
            resources: resources.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn arbitrary_plugin_sets_resolve_independent_resources_and_failed_owner_fallback() {
        assert!(select_candidates(&[], &BTreeSet::new()).0.is_empty());
        let claims = vec![
            ("a".into(), claim(100, &["shared.host"])),
            ("b".into(), claim(10, &["shared.host"])),
            ("c".into(), claim(0, &["other.resource"])),
        ];
        assert_eq!(select_candidates(&claims[..1], &BTreeSet::new()).0, ["a"]);
        assert_eq!(select_candidates(&claims, &BTreeSet::new()).0, ["a", "c"]);
        assert_eq!(
            select_candidates(&claims, &BTreeSet::from(["a".into()])).0,
            ["b", "c"]
        );
    }

    #[test]
    fn equal_priority_conflict_blocks_lower_claimants_but_not_unrelated_plugins() {
        let claims = vec![
            ("a".into(), claim(10, &["shared.host"])),
            ("b".into(), claim(10, &["SHARED.HOST"])),
            ("c".into(), claim(0, &["shared.host"])),
            ("d".into(), claim(0, &["other.resource"])),
        ];
        let (selected, conflicts) = select_candidates(&claims, &BTreeSet::new());
        assert_eq!(selected, ["d"]);
        assert_eq!(conflicts, BTreeSet::from(["a".into(), "b".into()]));
    }

    #[test]
    fn candidate_losing_one_resource_does_not_block_an_unrelated_winner() {
        let claims = vec![
            ("a".into(), claim(100, &["first.resource"])),
            (
                "b".into(),
                claim(10, &["first.resource", "second.resource"]),
            ),
            ("c".into(), claim(10, &["second.resource"])),
        ];
        let (selected, conflicts) = select_candidates(&claims, &BTreeSet::new());
        assert_eq!(selected, ["a", "c"]);
        assert!(conflicts.is_empty());
    }

    #[tokio::test]
    async fn failed_peer_is_removed_and_all_prepared_consumers_are_rebuilt_before_fallback() {
        let plugin = |id: &str| {
            InstalledPlugin {
            manifest: super::super::PluginManifest::from_json_str(&json!({
                "schemaVersion":2,"id":id,"name":id,"version":"1.0.0","author":"Example",
                "compatibility":{"ssmt":">=4.2.2","pluginApi":">=1.1.0 <2.0.0","platforms":["windows-x64"]},
                "contributions":{"backend":{"executable":"backend.exe","sha256":"a".repeat(64),
                    "events":["launch.describe","launch.prepare","launch.cancel"]}},
                "permissions":["process.spawn","game.launch"]
            }).to_string()).unwrap(),
            package_root: "fixture".into(), enabled: true, external_dependencies: BTreeMap::new(), official: false
        }
        };
        let plugins = [
            plugin("example.a"),
            plugin("example.b"),
            plugin("example.c"),
        ];
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let result = prepare_plugins(&plugins.iter().collect::<Vec<_>>(), "game", "game.exe", true, &[],
            move |plugin, _, method, payload| {
                let peers: Vec<_> = payload["peers"].as_array().into_iter().flatten().map(|peer|peer["pluginId"].as_str().unwrap().to_owned()).collect();
                recorded.lock().unwrap().push((plugin.id().to_owned(),method.to_owned(),peers));
                let response = match method {
                    "launch.describe" => {
                        let (priority,resource) = match plugin.id() {"example.a"=>(100,"first.resource"),"example.b"=>(90,"second.resource"),_=>(10,"second.resource")};
                        Ok(json!({"exports":{"example.shared":plugin.id()},"launch":{"priority":priority,"resources":[resource]}}))
                    }
                    "launch.prepare" if plugin.id()=="example.b" => Err("failed fixture".into()),
                    "launch.prepare" => Ok(json!({})),
                    _ => Ok(Value::Null)
                };
                Box::pin(std::future::ready(response))
            }).await.unwrap();
        assert_eq!(
            result
                .plans
                .iter()
                .map(|entry| entry.plugin_id.as_str())
                .collect::<Vec<_>>(),
            ["example.a", "example.c"]
        );
        let calls = calls.lock().unwrap();
        let prepared: Vec<_> = calls
            .iter()
            .filter(|(id, method, _)| id == "example.a" && method == "launch.prepare")
            .collect();
        assert_eq!(prepared.len(), 2);
        assert!(prepared[0].2.contains(&"example.b".into()));
        assert!(!prepared[1].2.contains(&"example.b".into()));
        assert!(calls
            .iter()
            .any(|(id, method, _)| id == "example.a" && method == "launch.cancel"));
        assert_eq!(result.diagnostics.len(), 1);
    }

    #[test]
    fn launch_paths_stay_inside_owned_roots_even_before_logs_exist() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-launch-paths-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let owned = root.join("owned");
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(root.join("outside.dll"), b"outside").unwrap();
        std::fs::write(owned.join("inside.dll"), b"inside").unwrap();
        let roots = vec![owned.canonicalize().unwrap()];
        assert!(checked_owned_path(&owned.join("inside.dll"), &roots, true).is_ok());
        assert!(checked_owned_path(&owned.join("future.log"), &roots, false).is_ok());
        assert!(checked_owned_path(&owned.join("../outside.dll"), &roots, true).is_err());
        assert!(checked_owned_path(&owned.join("../future.log"), &roots, false).is_err());
        assert!(checked_owned_path(Path::new("relative.dll"), &roots, true).is_err());
        assert!(checked_owned_path(&owned, &roots, true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undeclared_aliases_cannot_bypass_environment_and_dll_conflicts() {
        let existing = vec![OwnedLaunchPlan {
            plugin_id: "a".into(),
            plugin_name: "a".into(),
            plan: RuntimeLaunchPlan {
                dlls: vec![LaunchDll {
                    path: "C:/a/Effect.dll".into(),
                    sha256: "a".repeat(64),
                }],
                environment: BTreeMap::from([("VALUE".into(), Some("a".into()))]),
                ..Default::default()
            },
        }];
        assert!(check_plan_conflicts(
            &existing,
            &RuntimeLaunchPlan {
                environment: BTreeMap::from([("value".into(), Some("b".into()))]),
                ..Default::default()
            }
        )
        .is_err());
        assert!(check_plan_conflicts(
            &existing,
            &RuntimeLaunchPlan {
                dlls: vec![LaunchDll {
                    path: "C:/b/effect.DLL".into(),
                    sha256: "b".repeat(64)
                }],
                ..Default::default()
            }
        )
        .is_err());
        assert!(check_plan_conflicts(&existing, &RuntimeLaunchPlan::default()).is_ok());
    }

    #[tokio::test]
    async fn rollback_failure_does_not_apply_a_fallback_to_unreleased_resources() {
        let plugin = |id: &str| {
            InstalledPlugin {
            manifest: super::super::PluginManifest::from_json_str(&json!({
                "schemaVersion":2,"id":id,"name":id,"version":"1.0.0","author":"Example",
                "compatibility":{"ssmt":">=4.2.2","pluginApi":">=1.1.0 <2.0.0","platforms":["windows-x64"]},
                "contributions":{"backend":{"executable":"backend.exe","sha256":"a".repeat(64),
                    "events":["launch.describe","launch.prepare","launch.cancel"]}},
                "permissions":["process.spawn","game.launch"]
            }).to_string()).unwrap(),
            package_root:"fixture".into(),enabled:true,external_dependencies:BTreeMap::new(),official:false,
        }
        };
        let plugins = [plugin("example.primary"), plugin("example.fallback")];
        let result=prepare_plugins(&plugins.iter().collect::<Vec<_>>(),"game","game.exe",true,&[],|plugin,_,method,_| {
            let response=match method {
                "launch.describe"=>Ok(json!({"exports":{},"launch":{"priority":if plugin.id()=="example.primary" {100}else{10},"resources":["example.shared"]}})),
                "launch.prepare"=> {assert_eq!(plugin.id(),"example.primary");Err("prepare failed".into())},
                "launch.cancel"=>Err("rollback failed".into()),
                _=>unreachable!(),
            };
            Box::pin(std::future::ready(response))
        }).await;
        assert!(result.unwrap_err().contains("rollback failed"));
    }

    #[tokio::test]
    async fn empty_registry_needs_no_game_files_or_backend_data() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-empty-launch-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = PluginRegistry::new(&root).unwrap();
        let result = prepare_for_registry(&registry, "missing-game", "missing.exe", true, &[])
            .await
            .unwrap();
        assert!(result.plans.is_empty() && result.diagnostics.is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }
}
