//! Player Tweaks 的功能与资源声明。游戏 Hook 与设置后端均为本体已注册实现。
use super::{gimi_hot_runtime::{canonical_path, checked_path, read_text, verify}, registry::{game_scope_preset, InstalledPlugin}};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(super) const ID: &str = "ssmt.player-tweaks";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy { schema_version: u32, games: Vec<Game> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Game { preset: String, backend: String, features: Vec<String>, #[serde(default)] dll: Option<String> }

fn policy(plugin: &InstalledPlugin) -> Result<Policy, String> {
    let config = plugin.manifest.contributions.host_extensions.iter()
        .find(|item| item.provider == "ssmt.player-tweaks").and_then(|item| item.config.as_deref())
        .ok_or("Player Tweaks 缺少功能声明")?;
    let root = canonical_path(&plugin.package_root).map_err(|e| e.to_string())?;
    let text = read_text(&checked_path(&root, config)?)?;
    let policy: Policy = serde_json::from_str(&text).map_err(|e| format!("Player Tweaks 声明无效：{e}"))?;
    let mut presets = std::collections::HashSet::new();
    if policy.schema_version != 1 || policy.games.is_empty() { return Err("不支持的 Player Tweaks 声明".into()); }
    for game in &policy.games {
        // 包声明不能把已验证的游戏专有实现绑定到另一款游戏。
        let allowed = match (game.preset.as_str(), game.backend.as_str()) {
            ("GIMI", "gimi-hook") if game.dll.is_some() => ["camera", "fps", "fast-team"].as_slice(),
            ("SRMI", "star-rail-settings") if game.dll.is_none() => ["fps-120"].as_slice(),
            _ => return Err("Player Tweaks 游戏与后端不匹配".into()),
        };
        if !presets.insert(&game.preset) || game.features.is_empty() || game.features.iter().any(|item| !allowed.contains(&item.as_str())) {
            return Err("Player Tweaks 功能声明无效".into());
        }
        if let Some(path) = &game.dll { super::validate_package_relative_path(path).map_err(|e| e.to_string())?; }
    }
    Ok(policy)
}

pub(super) fn features(plugin: &InstalledPlugin, preset: &str) -> Result<Vec<String>, String> {
    Ok(policy(plugin)?.games.into_iter().find(|game| game.preset.eq_ignore_ascii_case(preset.trim()))
        .map(|game| game.features).unwrap_or_default())
}

#[derive(Serialize)]
pub struct LaunchAssets { pub dll: PathBuf }

#[tauri::command]
pub fn prepare_player_tweaks_launch(game_name: String, game_preset: String) -> Result<Option<LaunchAssets>, String> {
    let preset = game_scope_preset(&game_name);
    if !preset.eq_ignore_ascii_case(&game_preset) { return Err("Player Tweaks 游戏预设已改变".into()); }
    let Some(registry) = super::commands::installed_plugin_registry()? else { return Ok(None); };
    let scoped = registry.for_game(&game_name);
    let Some(installed) = scoped.find(ID).filter(|plugin| plugin.enabled) else { return Ok(None); };
    let Some(game) = policy(installed)?.games.into_iter().find(|game| game.preset.eq_ignore_ascii_case(&preset)) else { return Ok(None); };
    if game.backend == "star-rail-settings" {
        super::star_rail_tweaks::prepare_star_rail_player_tweaks(game_name)?;
        return Ok(None);
    }
    let root = canonical_path(&installed.package_root).map_err(|e| e.to_string())?;
    let relative = game.dll.ok_or("Player Tweaks DLL 未声明")?;
    let dll = checked_path(&root, &relative)?;
    let hash = read_text(&checked_path(&root, &format!("{relative}.sha256"))?)?;
    verify(&dll, hash.trim())?;
    if !dll.is_file() { return Err("Player Tweaks DLL 不可用".into()); }
    Ok(Some(LaunchAssets { dll }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_package_owns_game_features_and_broken_policy_does_not_fall_back() {
        let root = std::env::temp_dir().join(format!("ssmt-player-policy-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(root.join("host")).unwrap();
        let mut plugin = InstalledPlugin {
            manifest: super::super::PluginManifest::from_json_str(include_str!("../../../native/official-plugin-packages/ssmt.player-tweaks/0.1.0/ssmt-plugin.json")).unwrap(),
            package_root: root.clone(), enabled: true, external_dependencies: Default::default(), official: false,
        };
        assert!(features(&plugin, "GIMI").is_err(), "不能回退到随应用内置的功能声明");
        std::fs::write(root.join("host/features.json"), include_str!("../../../native/official-plugin-packages/ssmt.player-tweaks/0.1.0/host/features.json")).unwrap();
        assert_eq!(features(&plugin, "GIMI").unwrap(), ["camera", "fps", "fast-team"]);
        assert_eq!(features(&plugin, "srmi").unwrap(), ["fps-120"]);
        for preset in ["WWMI", "ZZMI", "EFMI", "ZZMIDX12"] { assert!(features(&plugin, preset).unwrap().is_empty()); }
        plugin.manifest.contributions.host_extensions.clear();
        assert!(features(&plugin, "GIMI").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
