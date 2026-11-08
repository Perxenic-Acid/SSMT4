use super::registry::PluginRegistry;
pub(super) use ssmt_plugin_graphics::reshade_state::{save_config, PreparedConfig};
use std::path::Path;

/// 所有热注入路线共用一个 Host；只合并当前启用的插件贡献。
pub(super) fn compose(
    registry: &PluginRegistry,
    addon_directory: &Path,
    required: &[String],
    dlss_ini: &str,
    dlss_preset: &str,
) -> Result<PreparedConfig, String> {
    ssmt_plugin_graphics::reshade_state::compose(
        super::host_graphics::reshade_contributions(registry)?,
        addon_directory,
        required,
        dlss_ini,
        dlss_preset,
    )
}

#[cfg(test)]
mod tests {
    use super::super::gimi_hot_runtime::read_text;
    use super::super::managed_reshade;
    use super::*;
    use crate::plugin::hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};
    use std::fs;
    #[test]
    fn optional_shade_contributions_follow_enabled_state_and_keep_one_host() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-hot-shade-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let addon = root.join("owned");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("dlss5-feed.addon64"), b"feed").unwrap();
        let required = vec!["dlss5-feed.addon64".into()];
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let no_plugins = compose(&registry, &addon, &required, "", "").unwrap();
        assert!(!no_plugins.game_ini.contains("ShadeEffect"));
        let package = registry
            .plugins_root()
            .join(HOYOSHADE_PLUGIN_ID)
            .join("0.1.0");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("ssmt-plugin.json"), include_str!("../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.0/ssmt-plugin.json")).unwrap();
        let newer = package.parent().unwrap().join("0.1.1");
        fs::create_dir_all(&newer).unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(package.join("ssmt-plugin.json")).unwrap()).unwrap();
        manifest["version"] = "0.1.1".into();
        fs::write(
            newer.join("ssmt-plugin.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let shade = root.join("shade");
        fs::create_dir_all(shade.join("reshade-shaders/Addons")).unwrap();
        fs::create_dir_all(shade.join("Presets")).unwrap();
        fs::create_dir_all(shade.join("InjectResource")).unwrap();
        fs::create_dir_all(shade.join("LauncherResource")).unwrap();
        for required in [
            "inject.exe",
            "ReShade64.dll",
            "LauncherResource/INIBuild.exe",
        ] {
            fs::write(shade.join(required), b"fixture").unwrap();
        }
        fs::write(
            shade.join("ReShade.ini"),
            "[GENERAL]\nPresetPath=C:\\HoYo\\Presets\\Mod OFF.ini\nEffectSearchPaths=Shaders\n[ADDON]\nDisabledAddons=ShadeEffect\n",
        )
        .unwrap();
        fs::write(
            shade.join("Presets/Mod OFF.ini"),
            "Techniques=ShadeEffect@Shade.fx\n",
        )
        .unwrap();
        fs::write(
            shade.join("reshade-shaders/Addons/ShadeEffect.addon64"),
            b"shade",
        )
        .unwrap();
        registry.refresh().unwrap();
        registry
            .set_external_dependency_path(HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID, &shade)
            .unwrap();
        for preset in ["GIMI", "SRMI", "WWMI", "ZZMI", "ZZMIDX12"] {
            registry
                .set_game_enabled(preset, HOYOSHADE_PLUGIN_ID, true)
                .unwrap();
            let combined = compose(
                &registry.for_game(preset),
                &addon,
                &required,
                "",
                "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n",
            )
            .unwrap();
            assert!(combined.game_ini.contains("DisabledAddons=ShadeEffect"));
            assert!(combined
                .preset_ini
                .contains("DLSS5_Feed@DLSS5_Feed.fx,ShadeEffect@Shade.fx"));
            assert!(!addon.join("ShadeEffect.addon64").exists());
            assert!(combined
                .game_ini
                .contains("PresetPath=.\\ReShadePreset.ini"));
            save_config(
                &addon,
                combined,
                "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n",
                &required,
            )
            .unwrap();
            let mut saved = read_text(&addon.join("ReShade.ini")).unwrap();
            managed_reshade::set_entry(&mut saved, "OVERLAY", "TutorialProgress", "4");
            fs::write(addon.join("ReShade.ini"), saved).unwrap();
            registry
                .set_game_enabled(preset, HOYOSHADE_PLUGIN_ID, false)
                .unwrap();
            let disabled = compose(&registry.for_game(preset), &addon, &required, "", "").unwrap();
            assert!(!disabled.game_ini.contains("ShadeEffect"));
            save_config(&addon, disabled, "", &required).unwrap();
            let saved = read_text(&addon.join("ReShade.ini")).unwrap();
            let saved_preset = read_text(&addon.join("ReShadePreset.ini")).unwrap();
            assert!(saved.contains("TutorialProgress=4"));
            assert!(!saved_preset.contains("ShadeEffect"));
            assert!(!addon.join("ShadeEffect.addon64").exists());
            assert!(addon.join("dlss5-feed.addon64").is_file());
        }
        registry
            .set_game_enabled("GIMI", HOYOSHADE_PLUGIN_ID, true)
            .unwrap();
        fs::write(
            shade.join("reshade-shaders/Addons/dlss5-feed.addon64"),
            b"conflict",
        )
        .unwrap();
        assert!(
            compose(&registry.for_game("GIMI"), &addon, &required, "", "")
                .unwrap_err()
                .contains("所有权冲突")
        );
        fs::write(
            shade.join("ReShade.ini"),
            "[DEPTH]\nUseAspectRatioHeuristics=1\n",
        )
        .unwrap();
        let conflict = compose(
            &registry.for_game("GIMI"),
            &addon,
            &[],
            "[DEPTH]\nUseAspectRatioHeuristics=0\n",
            "",
        )
        .unwrap_err();
        assert!(conflict.contains("共享 ReShade"));
        assert!(conflict.contains("[DEPTH] UseAspectRatioHeuristics"));
        let canonical = root.canonicalize().unwrap();
        assert_eq!(
            canonical.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }
}
