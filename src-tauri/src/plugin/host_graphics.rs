//! 统一的本体图形贡献入口。前端只执行候选计划，不编排特定插件。
use super::{
    gimi_hot_runtime::HotRuntimePlan,
    host_extensions::HostExtensionContribution,
    registry::{InstalledPlugin, PluginRegistry},
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub candidate_id: String,
    pub plugin: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsPreparation {
    pub candidate_id: Option<String>,
    pub plugin: Option<String>,
    pub plan: Option<HotRuntimePlan>,
    pub handled: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// 缺省声明只服务于已存在的旧包；新包应显式声明 provider 与配置。
pub(super) fn extensions(plugin: &InstalledPlugin) -> Vec<HostExtensionContribution> {
    if !plugin.manifest.contributions.graphics_profiles.is_empty() {
        return plugin
            .manifest
            .contributions
            .graphics_profiles
            .iter()
            .map(|profile| HostExtensionContribution {
                id: profile.id.clone(),
                provider: profile.strategy.as_str().to_string(),
                priority: profile.priority,
                config: profile.config.clone(),
                settings_component: profile.settings_component.clone(),
            })
            .collect();
    }
    if !plugin.manifest.contributions.host_extensions.is_empty() {
        return plugin.manifest.contributions.host_extensions.clone();
    }
    let (provider, priority, settings) = match plugin.id() {
        super::dlss5::DLSS5_PLUGIN_ID => ("ssmt.graphics.dlss5", 100, "ssmt.dlss5.settings"),
        super::hoyoshade::HOYOSHADE_PLUGIN_ID => {
            ("ssmt.graphics.reshade", 10, "ssmt.reshade.settings")
        }
        _ => return Vec::new(),
    };
    vec![HostExtensionContribution {
        id: "graphics".into(),
        provider: provider.into(),
        priority,
        config: None,
        settings_component: Some(settings.into()),
    }]
}

pub(super) fn settings_extensions(plugin: &InstalledPlugin) -> Vec<HostExtensionContribution> {
    extensions(plugin)
        .into_iter()
        .map(|mut extension| {
            let supported = matches!(
                (
                    extension.provider.as_str(),
                    extension.settings_component.as_deref()
                ),
                ("dlss5-hot", Some("ssmt.dlss5.settings"))
                    | ("ssmt.graphics.dlss5", Some("ssmt.dlss5.settings"))
                    | ("reshade-hot", Some("ssmt.reshade.settings"))
                    | ("ssmt.graphics.reshade", Some("ssmt.reshade.settings"))
            );
            if !supported {
                extension.settings_component = None;
            }
            extension
        })
        .collect()
}

fn prepare(
    extension: &HostExtensionContribution,
    plugin: &InstalledPlugin,
    game: &str,
    executable: &str,
) -> Result<Option<HotRuntimePlan>, String> {
    match extension.provider.as_str() {
        "dlss5-hot" => {
            super::dlss5_hot_runtime::prepare_dlss5_hot_runtime(game.into(), executable.into())
        }
        "ssmt.graphics.dlss5" if plugin.id() == super::dlss5::DLSS5_PLUGIN_ID => {
            super::dlss5_hot_runtime::prepare_dlss5_hot_runtime(game.into(), executable.into())
        }
        "reshade-hot" => {
            super::hoyoshade::prepare_hoyoshade_hot_runtime(game.into(), executable.into())
        }
        "ssmt.graphics.reshade" if plugin.id() == super::hoyoshade::HOYOSHADE_PLUGIN_ID => {
            super::hoyoshade::prepare_hoyoshade_hot_runtime(game.into(), executable.into())
        }
        _ => Err(format!("尚未注册本体能力 {}", extension.provider)),
    }
}

pub(super) fn reshade_contributions(
    registry: &PluginRegistry,
) -> Result<Vec<super::shade_policy::Contribution>, String> {
    let mut result = Vec::new();
    for plugin in registry.selected_plugins().filter(|plugin| plugin.enabled) {
        for extension in extensions(plugin) {
            if !matches!(
                extension.provider.as_str(),
                "reshade-hot" | "ssmt.graphics.reshade"
            ) {
                continue;
            }
            result.push(super::shade_policy::Policy::load(plugin)?.contribution(plugin)?);
        }
    }
    Ok(result)
}

pub(super) fn collect(
    registry: &PluginRegistry,
    allowed: bool,
    excluded: &[String],
    mut prepare: impl FnMut(
        &HostExtensionContribution,
        &InstalledPlugin,
    ) -> Result<Option<HotRuntimePlan>, String>,
) -> GraphicsPreparation {
    let mut candidates: Vec<_> = registry
        .selected_plugins()
        .filter(|p| p.enabled)
        .flat_map(|plugin| {
            extensions(plugin)
                .into_iter()
                .filter(|extension| {
                    extension.provider.starts_with("ssmt.graphics.")
                        || matches!(extension.provider.as_str(), "reshade-hot" | "dlss5-hot")
                })
                .map(move |extension| (plugin, extension))
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.1.priority
            .cmp(&a.1.priority)
            .then_with(|| a.0.id().cmp(b.0.id()))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    let mut result = GraphicsPreparation::default();
    let mut resources = BTreeMap::<String, Vec<(String, i32)>>::new();
    for (plugin, extension) in &candidates {
        let candidate_id = format!("{}:{}", plugin.id(), extension.id);
        for resource in resources_for(plugin, extension) {
            resources
                .entry(resource)
                .or_default()
                .push((candidate_id.clone(), extension.priority));
        }
    }
    let mut conflicting = HashSet::new();
    for (resource, claims) in resources {
        let max = claims.iter().map(|(_, priority)| *priority).max();
        let top: Vec<_> = claims
            .iter()
            .filter(|(_, priority)| Some(*priority) == max)
            .collect();
        if top.len() > 1 {
            for (candidate, _) in top {
                conflicting.insert(candidate.clone());
            }
            result.handled = true;
            result.diagnostics.push(Diagnostic {
                candidate_id: format!("resource:{resource}"),
                plugin: "graphicsProfiles".to_string(),
                error: "多个图形 Profile 以相同优先级占用同一资源".to_string(),
            });
        }
    }
    for (plugin, extension) in candidates {
        let id = format!("{}:{}", plugin.id(), extension.id);
        if conflicting.contains(&id) {
            result.handled = true;
            continue;
        }
        if excluded.contains(&id) {
            result.handled = true;
            continue;
        }
        if !allowed {
            result.diagnostics.push(Diagnostic {
                candidate_id: id,
                plugin: format!("{} v{}", plugin.manifest.name, plugin.version()),
                error: "当前启动方式无法继承热注入计划，请由 SSMT 直接创建目标游戏进程".into(),
            });
            continue;
        }
        result.handled = true;
        match prepare(&extension, plugin) {
            Ok(Some(plan)) => {
                result.candidate_id = Some(id);
                result.plugin = Some(format!("{} v{}", plugin.manifest.name, plugin.version()));
                result.plan = Some(plan);
                break;
            }
            Ok(None) => (),
            Err(error) => result.diagnostics.push(Diagnostic {
                candidate_id: id,
                plugin: format!("{} v{}", plugin.manifest.name, plugin.version()),
                error,
            }),
        }
    }
    result
}

fn resources_for(plugin: &InstalledPlugin, extension: &HostExtensionContribution) -> Vec<String> {
    if let Some(profile) = plugin
        .manifest
        .contributions
        .graphics_profiles
        .iter()
        .find(|profile| profile.id == extension.id)
    {
        return profile.resources.clone();
    }
    match extension.provider.as_str() {
        "reshade-hot" | "ssmt.graphics.reshade" => vec!["graphics.reshade.host".to_string()],
        "dlss5-hot" | "ssmt.graphics.dlss5" => vec![
            "graphics.dlss5.route".to_string(),
            "graphics.reshade.host".to_string(),
        ],
        _ => Vec::new(),
    }
}

#[tauri::command]
pub async fn prepare_plugin_graphics_plan(
    game_name: String,
    game_executable: String,
    allow_hot_runtime: bool,
    excluded_providers: Vec<String>,
    graphics_decision: Option<String>,
) -> Result<GraphicsPreparation, String> {
    tauri::async_runtime::spawn_blocking(move || {
        prepare_plugin_graphics_plan_blocking(
            game_name,
            game_executable,
            allow_hot_runtime,
            excluded_providers,
            graphics_decision,
        )
    })
    .await
    .map_err(|error| format!("后台启动任务失败：{error}"))?
}

fn prepare_plugin_graphics_plan_blocking(
    game_name: String,
    game_executable: String,
    allow_hot_runtime: bool,
    excluded_providers: Vec<String>,
    graphics_decision: Option<String>,
) -> Result<GraphicsPreparation, String> {
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(GraphicsPreparation::default());
    };
    Ok(collect(
        &registry.for_game(&game_name),
        allow_hot_runtime,
        &excluded_providers,
        |extension, plugin| {
            if graphics_decision.as_deref() == Some("suppress_hoyoshade_this_launch")
                && matches!(
                    extension.provider.as_str(),
                    "reshade-hot" | "ssmt.graphics.reshade"
                )
            {
                return Ok(None);
            }
            prepare(extension, plugin, &game_name, &game_executable)
        },
    ))
}

#[tauri::command]
pub async fn configure_plugin_launch_contract(
    game_name: String,
    game_executable: String,
    candidate_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        configure_plugin_launch_contract_blocking(game_name, game_executable, candidate_id)
    })
    .await
    .map_err(|error| format!("后台启动任务失败：{error}"))?
}

fn configure_plugin_launch_contract_blocking(
    game_name: String,
    game_executable: String,
    candidate_id: String,
) -> Result<(), String> {
    let registry = super::commands::installed_plugin_registry()?
        .ok_or("插件已移除")?
        .for_game(&game_name);
    for plugin in registry.selected_plugins().filter(|p| p.enabled) {
        for extension in extensions(plugin) {
            if format!("{}:{}", plugin.id(), extension.id) != candidate_id {
                continue;
            }
            return match extension.provider.as_str() {
                "dlss5-hot" => super::dlss5_hot_runtime::configure_dlss5_launch_settings(
                    game_name,
                    game_executable,
                ),
                "ssmt.graphics.dlss5" if plugin.id() == super::dlss5::DLSS5_PLUGIN_ID => {
                    super::dlss5_hot_runtime::configure_dlss5_launch_settings(
                        game_name,
                        game_executable,
                    )
                }
                "reshade-hot" => Ok(()),
                "ssmt.graphics.reshade" if plugin.id() == super::hoyoshade::HOYOSHADE_PLUGIN_ID => {
                    Ok(())
                }
                _ => Err("插件输入契约尚未注册".into()),
            };
        }
    }
    Err("计划所属插件已停用或声明已改变".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn schema_two_graphics_profiles_are_strategy_based() {
        for (path, strategy, settings) in [
            (
                "../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.2.0/ssmt-plugin.json",
                "reshade-hot",
                "ssmt.reshade.settings",
            ),
            (
                "../native/official-plugin-packages/ssmt.dlss5.integration/0.3.0/ssmt-plugin.json",
                "dlss5-hot",
                "ssmt.dlss5.settings",
            ),
        ] {
            let raw =
                fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
                    .unwrap();
            let manifest = super::super::PluginManifest::from_json_str(&raw).unwrap();
            let plugin = InstalledPlugin {
                manifest,
                package_root: std::path::PathBuf::from("fixture"),
                enabled: true,
                external_dependencies: Default::default(),
                official: false,
            };
            let extension = &extensions(&plugin)[0];
            assert_eq!(extension.provider, strategy);
            if strategy == "dlss5-hot" {
                assert!(plugin
                    .manifest
                    .contributions
                    .pages
                    .iter()
                    .any(|page| { page.id() == "settings" && page.kind() == "static" }));
            }
            assert_eq!(
                settings_extensions(&plugin)[0]
                    .settings_component
                    .as_deref(),
                Some(settings)
            );
        }
    }

    #[test]
    fn equal_priority_graphics_resources_are_reported_as_conflicts() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-graphics-conflict-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        for id in ["example.graphics.a", "example.graphics.b"] {
            let package = registry.plugins_root().join(id).join("1.0.0");
            fs::create_dir_all(package.join("host")).unwrap();
            fs::write(package.join("host/graphics.json"), "{}").unwrap();
            fs::write(
                package.join("ssmt-plugin.json"),
                serde_json::json!({
                    "schemaVersion": 2,
                    "id": id,
                    "name": id,
                    "version": "1.0.0",
                    "author": "Example",
                    "compatibility": {
                        "ssmt": ">=4.2.2",
                        "pluginApi": ">=1.0.0 <2.0.0",
                        "platforms": ["windows-x64"]
                    },
                    "contributions": {
                        "graphicsProfiles": [{
                            "id": "graphics",
                            "strategy": "reshade-hot",
                            "priority": 10,
                            "config": "host/graphics.json",
                            "resources": ["graphics.reshade.host"]
                        }]
                    },
                    "permissions": [
                        "native.inject",
                        "filesystem.read",
                        "filesystem.write"
                    ]
                })
                .to_string(),
            )
            .unwrap();
        }
        registry.refresh().unwrap();
        for id in ["example.graphics.a", "example.graphics.b"] {
            registry.set_game_enabled("GIMI", id, true).unwrap();
        }
        let scoped = registry.for_game("GIMI");
        let result = collect(&scoped, true, &[], |_, _| {
            panic!("conflicting graphics profiles must not execute")
        });
        assert!(result.plan.is_none());
        assert!(result.handled);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.candidate_id.contains("graphics.reshade.host")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "需先生成本地测试插件包，并设置 SSMT_HOST_PACKAGE_TEST_ROOT；不加载 DLL"]
    fn actual_packages_install_and_expose_declared_routes_settings_and_assets() {
        let packages = std::path::PathBuf::from(
            std::env::var_os("SSMT_HOST_PACKAGE_TEST_ROOT").expect("缺少测试包目录"),
        );
        let root = std::env::temp_dir().join(format!(
            "ssmt-host-package-check-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        for (id, version) in [
            (super::super::dlss5::DLSS5_PLUGIN_ID, "0.2.2"),
            (super::super::hoyoshade::HOYOSHADE_PLUGIN_ID, "0.1.1"),
        ] {
            let archive = packages.join(format!("{id}-{version}.ssmtpkg"));
            let inspection = super::super::package_installer::inspect_ssmtpkg(&archive).unwrap();
            assert_eq!(inspection.manifest.id, id);
            super::super::package_installer::install_ssmtpkg(&mut registry, &archive).unwrap();
            let plugin = registry.find(id).unwrap();
            assert!(settings_extensions(plugin)[0].settings_component.is_some());
        }
        let policy = super::super::dlss5_policy::Policy::from_registry(&registry)
            .unwrap()
            .unwrap();
        assert_eq!(
            policy
                .select("WWMI", std::path::Path::new("Client-Win64-Shipping.exe"))
                .unwrap()
                .required_launch_args,
            ["-dx11"]
        );
        assert!(policy.assets(false).unwrap().contains("sha256"));
        let shade = registry
            .find(super::super::hoyoshade::HOYOSHADE_PLUGIN_ID)
            .unwrap();
        assert!(super::super::shade_policy::Policy::load(shade)
            .unwrap()
            .processes
            .iter()
            .any(|name| name == "StarRail.exe"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_single_multiple_and_all_candidates_follow_declarations_and_fallback() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-host-plans-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        assert!(!collect(&registry, true, &[], |_, _| panic!("无插件不能准备资源")).handled);
        for ((id, priority), version) in [("example.a", 10), ("example.b", 50), ("example.c", 20)]
            .into_iter()
            .flat_map(|plugin| {
                ["1.9.0", "1.10.0"]
                    .into_iter()
                    .map(move |version| (plugin, version))
            })
        {
            let package = registry.plugins_root().join(id).join(version);
            fs::create_dir_all(&package).unwrap();
            fs::write(package.join("ssmt-plugin.json"), serde_json::json!({
                "schemaVersion":1,"id":id,"name":id,"version":version,"author":"Example",
                "compatibility":{"ssmt":">=4.0.0","platforms":["windows-x64"]},
                "contributions":{"hostExtensions":[{"id":"host","provider":"ssmt.graphics.test","priority":priority}]}
            }).to_string()).unwrap();
        }
        registry.refresh().unwrap();
        let plugin = registry.find("example.a").unwrap();
        let mut foreign = plugin.clone();
        foreign.manifest.contributions.host_extensions[0].settings_component =
            Some("ssmt.dlss5.settings".into());
        assert!(settings_extensions(&foreign)[0]
            .settings_component
            .is_none());
        for mask in 0..8 {
            for (index, id) in ["example.a", "example.b", "example.c"].iter().enumerate() {
                registry
                    .set_game_enabled("WWMI", id, mask & (1 << index) != 0)
                    .unwrap();
            }
            let scoped = registry.for_game("WWMI");
            let mut visited = Vec::new();
            let result = collect(&scoped, true, &[], |_, plugin| {
                assert_eq!(plugin.version(), "1.10.0");
                visited.push(plugin.id().to_string());
                Err("模拟准备失败".into())
            });
            assert_eq!(visited.len(), (mask as u32).count_ones() as usize);
            assert_eq!(result.diagnostics.len(), visited.len());
            for diagnostic in &result.diagnostics {
                assert!(diagnostic.plugin.ends_with(" v1.10.0"));
                assert!(diagnostic.candidate_id.ends_with(":host"));
            }
            assert!(
                !collect(&scoped, false, &[], |_, _| panic!("外部启动不准备资源"))
                    .plan
                    .is_some()
            );
        }
        let scoped = registry.for_game("WWMI");
        let result = collect(&scoped, true, &["example.b:host".into()], |_, plugin| {
            Ok(Some(HotRuntimePlan {
                dlls: vec![plugin.package_root.join("ReShade64.dll")],
                environment: Default::default(),
                runtime_directory: plugin.package_root.clone(),
                required_launch_args: Vec::new(),
                excluded_launch_args: Vec::new(),
                log_sources: Vec::new(),
            }))
        });
        assert_eq!(result.candidate_id.as_deref(), Some("example.c:host"));
        fs::remove_dir_all(root).unwrap();
    }
}
