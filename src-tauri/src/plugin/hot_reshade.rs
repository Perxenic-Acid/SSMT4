use super::{
    gimi_hot_runtime::{canonical_path, checked_path, read_text, rebase_search_paths},
    hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID},
    managed_reshade::{self, ManagedReShadeConfig},
    registry::{ExternalDependencyStatus, PluginRegistry},
};
use std::{fs, io::Write, path::Path};

/// 配置与默认值分开保存，避免每次启动重置 ReShade 的教程、按键和效果参数。
pub(super) fn save_config(
    directory: &Path,
    defaults: ManagedReShadeConfig,
    required_preset: &str,
    addons: &[String],
) -> Result<(), String> {
    let root = canonical_path(directory).map_err(|error| error.to_string())?;
    let read = |name: &str| -> Result<Option<String>, String> {
        if root.join(name).exists() {
            read_text(&checked_path(&root, name)?).map(Some)
        } else {
            Ok(None)
        }
    };
    let previous = read("ReShade.ssmt-defaults.json")?
        .and_then(|text| serde_json::from_str::<ManagedReShadeConfig>(&text).ok());
    let current = ManagedReShadeConfig {
        game_ini: read("ReShade.ini")?.unwrap_or_else(|| defaults.game_ini.clone()),
        preset_ini: read("ReShadePreset.ini")?.unwrap_or_else(|| defaults.preset_ini.clone()),
    };
    let composed = managed_reshade::retain_user_state(
        &defaults,
        previous.as_ref(),
        &current,
        required_preset,
        addons,
    );
    let baseline = serde_json::to_string(&defaults).map_err(|error| error.to_string())?;
    for (name, content) in [
        ("ReShade.ini", composed.game_ini),
        ("ReShadePreset.ini", composed.preset_ini),
        ("ReShade.ssmt-defaults.json", baseline),
    ] {
        // 临时文件与最终文件均位于本次管理目录中，既有链接不能跳出目录。
        if root.join(name).exists() {
            checked_path(&root, name)?;
        }
        let temporary = root.join(format!(
            "{name}.{}-{}.tmp",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let write = (|| -> Result<(), std::io::Error> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(content.as_bytes())?;
            drop(file);
            fs::rename(&temporary, root.join(name))
        })();
        if let Err(error) = write {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
    }
    Ok(())
}

/// 所有热注入路线共用一个 Host；只合并当前启用的插件贡献。
pub(super) fn compose(
    registry: &PluginRegistry,
    addon_directory: &Path,
    required: &[String],
    dlss_ini: &str,
    dlss_preset: &str,
) -> Result<ManagedReShadeConfig, String> {
    let mut base = String::new();
    let mut preset = String::new();
    let mut addons = required.to_vec();
    if let Some(plugin) = registry
        .find(HOYOSHADE_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
    {
        let dependency = plugin
            .external_dependencies
            .get(HOYOSHADE_DEPENDENCY_ID)
            .filter(|dependency| dependency.status == ExternalDependencyStatus::Ready)
            .and_then(|dependency| dependency.path.as_ref())
            .ok_or("Shade Bridge 的资源目录尚未就绪")?;
        let dependency = canonical_path(dependency).map_err(|error| error.to_string())?;
        base = read_text(&checked_path(&dependency, "ReShade.ini")?)?;
        rebase_search_paths(&mut base, &dependency);
        if dependency.join("Presets/Mod OFF.ini").is_file() {
            preset = read_text(&checked_path(&dependency, "Presets/Mod OFF.ini")?)?;
        }
        let sources = dependency.join("reshade-shaders/Addons");
        if sources.is_dir() {
            for entry in fs::read_dir(sources).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if !path.is_file() || !name.to_ascii_lowercase().ends_with(".addon64") {
                    continue;
                }
                if addons.iter().any(|other| other.eq_ignore_ascii_case(&name)) {
                    return Err(format!("Add-on 资源所有权冲突：{name}"));
                }
                let canonical = canonical_path(&path).map_err(|error| error.to_string())?;
                if !canonical.starts_with(&dependency) {
                    return Err("Shade Add-on 越出声明的资源目录".into());
                }
                fs::copy(canonical, addon_directory.join(&name))
                    .map_err(|error| error.to_string())?;
                addons.push(name);
            }
        }
    }
    // 关闭插件后移除其上次贡献；不把配置中的非必要禁用项强行打开。
    for entry in fs::read_dir(addon_directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.to_ascii_lowercase().ends_with(".addon64")
            && !addons
                .iter()
                .any(|active| active.eq_ignore_ascii_case(&name))
        {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }
    managed_reshade::compose(&base, dlss_ini, &preset, dlss_preset, required)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_launch_preserves_tutorial_controls_and_uniforms_but_removes_disabled_contributions()
    {
        let root = std::env::temp_dir().join(format!(
            "ssmt-reshade-state-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let required = vec!["feed.addon64".into()];
        let feeder = "Techniques=MV@mv.fx,Feed@feed.fx\nTechniqueSorting=MV@mv.fx,Feed@feed.fx\n[feed.fx]\nPreprocessorDefinitions=PROVIDER=3\nStrength=1\n";
        let mut defaults = managed_reshade::compose(
            "[GENERAL]\nEffectSearchPaths=C:/Shade/FX\n[OVERLAY]\nTutorialProgress=0\n[ADDON]\nDisabledAddons=unrelated\n",
            "", "Techniques=Shade@shade.fx\n[shade.fx]\nBrightness=1\n", feeder, &required,
        ).unwrap();
        save_config(&root, defaults.clone(), feeder, &required).unwrap();
        let mut current = read_text(&root.join("ReShade.ini")).unwrap();
        for (section, key, value) in [
            ("OVERLAY", "TutorialProgress", "4"),
            ("INPUT", "KeyOverlay", "36,0,0,0"),
            ("GENERAL", "EffectSearchPaths", "C:/Shade/FX,C:/User/FX"),
            ("ADDON", "DisabledAddons", "feed,unrelated,user-addon"),
            ("ADDON", "AddonPath", "C:/wrong-host"),
        ] {
            managed_reshade::set_entry(&mut current, section, key, value);
        }
        fs::write(root.join("ReShade.ini"), current).unwrap();
        let mut preset = read_text(&root.join("ReShadePreset.ini")).unwrap();
        managed_reshade::set_entry(&mut preset, "", "Techniques", "User@user.fx,Feed@feed.fx");
        managed_reshade::set_entry(&mut preset, "feed.fx", "Strength", "0.75");
        managed_reshade::set_entry(
            &mut preset,
            "feed.fx",
            "PreprocessorDefinitions",
            "PROVIDER=0",
        );
        managed_reshade::set_entry(&mut preset, "user.fx", "Color", "0.5");
        fs::write(root.join("ReShadePreset.ini"), preset).unwrap();
        for _ in 0..2 {
            save_config(&root, defaults.clone(), feeder, &required).unwrap();
            let ini = read_text(&root.join("ReShade.ini")).unwrap();
            assert!(ini.contains("TutorialProgress=4"));
            assert!(ini.contains("KeyOverlay=36,0,0,0"));
            assert!(ini.contains("DisabledAddons=unrelated,user-addon"));
            assert!(!ini.contains("C:/wrong-host"));
            let preset = read_text(&root.join("ReShadePreset.ini")).unwrap();
            assert!(preset.contains("Techniques=MV@mv.fx,Feed@feed.fx,User@user.fx"));
            assert!(!preset
                .lines()
                .find(|line| line.starts_with("Techniques="))
                .unwrap()
                .contains("Shade@"));
            assert!(preset.contains("Strength=0.75"));
            assert!(preset.contains("PROVIDER=3"));
        }
        defaults = managed_reshade::compose("", "", "", feeder, &required).unwrap();
        save_config(&root, defaults, feeder, &required).unwrap();
        let ini = read_text(&root.join("ReShade.ini")).unwrap();
        let preset = read_text(&root.join("ReShadePreset.ini")).unwrap();
        assert!(ini.contains("TutorialProgress=4"));
        assert!(!ini.contains("C:/Shade/FX"));
        assert!(ini.contains("C:/User/FX"));
        assert!(!preset.contains("shade.fx"));
        assert!(preset.contains("Color=0.5"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn existing_gimi_configuration_migrates_without_resetting_tutorial() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-reshade-migrate-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("ReShade.ini"),
            "[OVERLAY]\nTutorialProgress=4\n[INPUT]\nKeyOverlay=35,0,0,0\n",
        )
        .unwrap();
        let defaults = managed_reshade::compose("", "", "", "", &[]).unwrap();
        for _ in 0..2 {
            save_config(&root, defaults.clone(), "", &[]).unwrap();
            let ini = read_text(&root.join("ReShade.ini")).unwrap();
            assert!(ini.contains("TutorialProgress=4"));
            assert!(ini.contains("KeyOverlay=35,0,0,0"));
        }
        fs::remove_dir_all(root).unwrap();
    }

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
            "[GENERAL]\nEffectSearchPaths=Shaders\n[ADDON]\nDisabledAddons=ShadeEffect\n",
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
            assert!(addon.join("ShadeEffect.addon64").is_file());
            save_config(&addon, combined, "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n", &required).unwrap();
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
        let canonical = root.canonicalize().unwrap();
        assert_eq!(
            canonical.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }
}
