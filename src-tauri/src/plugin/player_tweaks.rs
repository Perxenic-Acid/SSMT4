//! Transitional launch adapter for the SRMI client-settings backend.
//!
//! GIMI configuration and DLL selection are now package-owned. This module only
//! invokes the existing registry transaction until that operation is promoted to
//! a generic client-settings capability.

use super::registry::{game_scope_preset, PluginRegistry};

pub(super) const ID: &str = "ssmt.player-tweaks";

#[tauri::command]
pub async fn prepare_player_tweaks_launch(
    game_name: String,
    game_preset: String,
) -> Result<Option<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        prepare_player_tweaks_launch_blocking(&game_name, &game_preset)
    })
    .await
    .map_err(|error| format!("后台启动任务失败：{error}"))?
}

fn prepare_player_tweaks_launch_blocking(
    game_name: &str,
    requested_preset: &str,
) -> Result<Option<serde_json::Value>, String> {
    let preset = game_scope_preset(game_name);
    if !preset.eq_ignore_ascii_case(requested_preset.trim()) {
        return Err("Player Tweaks 游戏预设已改变".to_string());
    }
    if !preset.eq_ignore_ascii_case("SRMI") {
        return Ok(None);
    }
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if !registry.game_enabled(game_name, ID) {
        return Ok(None);
    }
    super::star_rail_tweaks::prepare_star_rail_player_tweaks(game_name.to_string())?;
    Ok(None)
}
