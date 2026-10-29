use super::{
    gimi_hot_runtime::{canonical_path, checked_path, read_text},
    managed_reshade::{self, ManagedReShadeConfig},
    registry::PluginRegistry,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub(super) struct PreparedConfig {
    pub game_ini: String,
    pub preset_ini: String,
    addon_sources: BTreeMap<String, PathBuf>,
    addon_directory: Option<PathBuf>,
    pub extra_files: BTreeMap<PathBuf, Vec<u8>>,
}

impl From<ManagedReShadeConfig> for PreparedConfig {
    fn from(config: ManagedReShadeConfig) -> Self {
        Self {
            game_ini: config.game_ini,
            preset_ini: config.preset_ini,
            addon_sources: BTreeMap::new(),
            addon_directory: None,
            extra_files: BTreeMap::new(),
        }
    }
}

/// 配置与默认值分开保存，避免每次启动重置 ReShade 的教程、按键和效果参数。
pub(super) fn save_config(
    directory: &Path,
    defaults: impl Into<PreparedConfig>,
    required_preset: &str,
    addons: &[String],
) -> Result<(), String> {
    let root = canonical_path(directory).map_err(|error| error.to_string())?;
    let prepared = defaults.into();
    let addon_root = prepared.addon_directory.as_deref().unwrap_or(&root);
    let addon_root = canonical_path(addon_root).map_err(|e| e.to_string())?;
    let mut transaction_root = root.clone();
    for directory in std::iter::once(addon_root.as_path())
        .chain(prepared.extra_files.keys().filter_map(|path| path.parent()))
    {
        while !directory.starts_with(&transaction_root) {
            if !transaction_root.pop() {
                return Err("配置目录不在同一安装中".into());
            }
        }
    }
    let transaction = super::managed_files::DirectoryTransaction::open(&transaction_root)?;
    let key = |directory: &Path, name: &str| -> Result<String, String> {
        directory
            .join(name)
            .strip_prefix(&transaction_root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .map_err(|e| e.to_string())
    };
    let defaults = ManagedReShadeConfig {
        game_ini: prepared.game_ini,
        preset_ini: prepared.preset_ini,
    };
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
    let ownership_name = "ReShade.ssmt-addons.json";
    let previous_addons: BTreeMap<String, String> = read(ownership_name)?
        .map(|text| serde_json::from_str(&text).map_err(|e| format!("Add-on 所有权记录无效：{e}")))
        .transpose()?
        .unwrap_or_default();
    let mut changes = BTreeMap::new();
    let mut ownership = BTreeMap::new();
    // 旧配置尚无所有权记录时不能悄悄加载来源不明的遗留 Add-on。
    for entry in fs::read_dir(&addon_root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.to_ascii_lowercase().ends_with(".addon64")
            && !addons.iter().any(|item| item.eq_ignore_ascii_case(&name))
            && !prepared
                .addon_sources
                .keys()
                .any(|item| item.eq_ignore_ascii_case(&name))
            && !previous_addons
                .keys()
                .any(|item| item.eq_ignore_ascii_case(&name))
        {
            return Err(format!(
                "组件目录存在未登记的 Add-on {name}，请重新配置组件；已保留原文件"
            ));
        }
    }
    for (name, source) in prepared.addon_sources {
        super::validate_package_relative_path(&name).map_err(|e| e.to_string())?;
        if name.contains(['/', '\\']) {
            return Err("Add-on 文件名无效".into());
        }
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        use sha2::{Digest, Sha256};
        let applied = format!("{:x}", Sha256::digest(&bytes));
        let current = super::managed_files::hash(&addon_root.join(&name))?;
        if current.is_some()
            && current.as_ref() != Some(&applied)
            && current.as_ref() != previous_addons.get(&name)
        {
            return Err(format!("Add-on {name} 不属于当前贡献，已保留文件"));
        }
        ownership.insert(name.clone(), applied);
        changes.insert(key(&addon_root, &name)?, Some(bytes));
    }
    for (name, applied) in previous_addons {
        super::validate_package_relative_path(&name).map_err(|e| e.to_string())?;
        if name.contains(['/', '\\']) {
            return Err("Add-on 所有权记录文件名无效".into());
        }
        if ownership.contains_key(&name) {
            continue;
        }
        if addons
            .iter()
            .any(|required| required.eq_ignore_ascii_case(&name))
        {
            return Err(format!("Add-on 所有权与必要组件冲突：{name}"));
        }
        let current = super::managed_files::hash(&addon_root.join(&name))?;
        if current.is_some() && current.as_deref() != Some(&applied) {
            return Err(format!("Add-on {name} 已被修改，已保留文件"));
        }
        changes.insert(key(&addon_root, &name)?, None);
    }
    changes.insert(
        key(&root, ownership_name)?,
        Some(serde_json::to_vec(&ownership).map_err(|e| e.to_string())?),
    );
    for (name, content) in [
        ("ReShade.ini", composed.game_ini),
        ("ReShadePreset.ini", composed.preset_ini),
        ("ReShade.ssmt-defaults.json", baseline),
    ] {
        changes.insert(key(&root, name)?, Some(content.into_bytes()));
    }
    for (path, bytes) in prepared.extra_files {
        let relative = path
            .strip_prefix(&transaction_root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if changes.insert(relative, Some(bytes)).is_some() {
            return Err("配置文件所有权冲突".into());
        }
    }
    transaction.commit(changes)?;
    Ok(())
}

/// 所有热注入路线共用一个 Host；只合并当前启用的插件贡献。
pub(super) fn compose(
    registry: &PluginRegistry,
    addon_directory: &Path,
    required: &[String],
    dlss_ini: &str,
    dlss_preset: &str,
) -> Result<PreparedConfig, String> {
    let mut base = String::new();
    let mut preset = String::new();
    let mut addons = required.to_vec();
    let mut addon_sources = BTreeMap::new();
    for contribution in super::host_graphics::reshade_contributions(registry)? {
        let merged = managed_reshade::compose(
            &base,
            &contribution.config,
            &preset,
            &contribution.preset,
            &[],
        )
        .map_err(|e| format!("合并共享 ReShade 贡献失败（{}）：{e}", contribution.source))?;
        base = merged.game_ini;
        preset = merged.preset_ini;
        for (name, source) in contribution.addons {
            if addons.iter().any(|other| other.eq_ignore_ascii_case(&name)) {
                return Err(format!("Add-on 资源所有权冲突：{name}"));
            }
            addons.push(name.clone());
            addon_sources.insert(name, source);
        }
    }

    let mut prepared: PreparedConfig =
        managed_reshade::compose(&base, dlss_ini, &preset, dlss_preset, required)
            .map_err(|error| format!("合并共享 ReShade 与当前图形路线的配置失败：{error}"))?
            .into();
    prepared.addon_sources = addon_sources;
    prepared.addon_directory = Some(canonical_path(addon_directory).map_err(|e| e.to_string())?);
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};

    #[test]
    fn separate_config_and_addon_directories_commit_together_without_loading_unknown_addons() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-separated-config-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let config = root.join("state");
        let addons = root.join("addons");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir(&addons).unwrap();
        let source = root.join("source.dll");
        fs::write(&source, b"optional addon").unwrap();
        let mut prepared: PreparedConfig = managed_reshade::compose("", "", "", "", &[])
            .unwrap()
            .into();
        prepared.addon_directory = Some(addons.clone());
        prepared
            .addon_sources
            .insert("effect.addon64".into(), source);
        prepared
            .extra_files
            .insert(root.join("backend.ini"), b"configured".to_vec());
        save_config(&config, prepared.clone(), "", &[]).unwrap();
        assert!(addons.join("effect.addon64").is_file());
        assert!(!config.join("effect.addon64").exists());
        assert_eq!(fs::read(root.join("backend.ini")).unwrap(), b"configured");
        fs::write(addons.join("unknown.addon64"), b"unknown").unwrap();
        assert!(save_config(&config, prepared, "", &[])
            .unwrap_err()
            .contains("未登记"));
        assert_eq!(
            fs::read(addons.join("unknown.addon64")).unwrap(),
            b"unknown"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_addon_is_preserved_and_failed_prepare_keeps_all_configs() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-addon-ownership-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.dll");
        fs::write(&source, b"original addon").unwrap();
        let defaults =
            managed_reshade::compose("[OVERLAY]\nTutorialProgress=0\n", "", "", "", &[]).unwrap();
        let mut prepared: PreparedConfig = defaults.clone().into();
        prepared
            .addon_sources
            .insert("player.addon64".into(), source);
        save_config(&root, prepared, "", &[]).unwrap();
        let ini = fs::read(root.join("ReShade.ini")).unwrap();
        fs::write(root.join("player.addon64"), b"player updated addon").unwrap();
        assert!(save_config(&root, defaults.clone(), "", &[])
            .unwrap_err()
            .contains("已被修改"));
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), ini);
        assert_eq!(
            fs::read(root.join("player.addon64")).unwrap(),
            b"player updated addon"
        );
        fs::write(root.join("player.addon64"), b"original addon").unwrap();
        save_config(&root, defaults, "", &[]).unwrap();
        assert!(!root.join("player.addon64").exists());
        fs::remove_dir_all(root).unwrap();
    }

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
