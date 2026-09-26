use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedReShadeConfig {
    pub game_ini: String,
    pub preset_ini: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniConflict {
    pub section: String,
    pub key: String,
    pub hoyoshade_value: String,
    pub dlss5_value: String,
}

impl std::fmt::Display for IniConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ReShade configuration conflict [{}] {}: HoYoShade={}, DLSS5={}",
            self.section, self.key, self.hoyoshade_value, self.dlss5_value
        )
    }
}

impl std::error::Error for IniConflict {}

#[derive(Debug, Clone)]
struct Entry {
    section: String,
    key: String,
    value: String,
}

fn entries(text: &str) -> Vec<Entry> {
    let mut section = String::new();
    let mut result = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
        } else if !line.starts_with(';') && !line.starts_with('#') {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    result.push(Entry {
                        section: section.clone(),
                        key: key.to_string(),
                        value: value.trim().to_string(),
                    });
                }
            }
        }
    }
    result
}

fn norm(section: &str, key: &str) -> (String, String) {
    (section.to_ascii_lowercase(), key.to_ascii_lowercase())
}

fn list_union(left: &str, right: &str) -> String {
    let mut result: Vec<String> = Vec::new();
    for part in left.split(',').chain(right.split(',')) {
        let item = part.trim();
        if !item.is_empty() && !result.iter().any(|old| old.eq_ignore_ascii_case(item)) {
            result.push(item.to_string());
        }
    }
    result.join(",")
}

fn merge_value(section: &str, key: &str, left: &str, right: &str) -> Result<String, IniConflict> {
    if left == right {
        return Ok(left.to_string());
    }
    let (_, key_lower) = norm(section, key);
    if [
        "addonpath",
        "effectsearchpaths",
        "texturesearchpaths",
        "preprocessordefinitions",
        "techniques",
        "techniquesorting",
    ]
    .contains(&key_lower.as_str())
    {
        // Preprocessor definitions identify themselves by name. Two different
        // values for one definition are a real conflict, not a list union.
        if key_lower == "preprocessordefinitions" {
            let mut values = BTreeMap::<String, String>::new();
            for part in left.split(',').chain(right.split(',')) {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let name = part
                    .split('=')
                    .next()
                    .unwrap_or(part)
                    .trim()
                    .to_ascii_lowercase();
                if let Some(old) = values.insert(name, part.to_string()) {
                    if !old.eq_ignore_ascii_case(part) {
                        return Err(IniConflict {
                            section: section.to_string(),
                            key: key.to_string(),
                            hoyoshade_value: old,
                            dlss5_value: part.to_string(),
                        });
                    }
                }
            }
        }
        if key_lower == "techniques" || key_lower == "techniquesorting" {
            return Ok(list_union(right, left));
        }
        return Ok(list_union(left, right));
    }
    Err(IniConflict {
        section: section.to_string(),
        key: key.to_string(),
        hoyoshade_value: left.to_string(),
        dlss5_value: right.to_string(),
    })
}

fn set_entry(text: &mut String, section: &str, key: &str, value: &str) {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let wanted_section = section.to_ascii_lowercase();
    let mut current_section = String::new();
    let mut insert_at = if section.is_empty() { Some(0) } else { None };
    for (index, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if current_section == wanted_section {
                insert_at = Some(index);
            }
            current_section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
            if current_section == wanted_section {
                insert_at = Some(index + 1);
            }
        } else if current_section == wanted_section {
            if let Some((found_key, _)) = trimmed.split_once('=') {
                if found_key.trim().eq_ignore_ascii_case(key) {
                    *line = format!("{key}={value}");
                    *text = lines.join(newline) + newline;
                    return;
                }
            }
            insert_at = Some(index + 1);
        }
    }
    if let Some(index) = insert_at {
        lines.insert(index, format!("{key}={value}"));
    } else {
        lines.push(format!("[{section}]"));
        lines.push(format!("{key}={value}"));
    }
    *text = lines.join(newline) + newline;
}

fn get_entry(text: &str, section: &str, key: &str) -> Option<String> {
    entries(text)
        .into_iter()
        .find(|entry| norm(&entry.section, &entry.key) == norm(section, key))
        .map(|entry| entry.value)
}

fn merge_document(hoyo: &str, dlss: &str) -> Result<String, IniConflict> {
    let mut result = hoyo.to_string();
    for entry in entries(dlss) {
        let next = match get_entry(&result, &entry.section, &entry.key) {
            Some(old) => merge_value(&entry.section, &entry.key, &old, &entry.value)?,
            None => entry.value,
        };
        set_entry(&mut result, &entry.section, &entry.key, &next);
    }
    Ok(result)
}

pub fn compose(
    hoyo_ini: &str,
    dlss_ini: &str,
    hoyo_preset: &str,
    dlss_preset: &str,
    addon_names: &[String],
) -> Result<ManagedReShadeConfig, IniConflict> {
    // The managed host uses one game-local preset, allowing the Feeder's
    // required techniques and HoYoShade's presets to share that file.
    let mut hoyo_ini = hoyo_ini.to_string();
    set_entry(
        &mut hoyo_ini,
        "GENERAL",
        "PresetPath",
        ".\\ReShadePreset.ini",
    );
    let mut game_ini = merge_document(&hoyo_ini, dlss_ini)?;
    let addon_paths = get_entry(&game_ini, "ADDON", "AddonPath").unwrap_or_default();
    set_entry(
        &mut game_ini,
        "ADDON",
        "AddonPath",
        &list_union(&addon_paths, ".\\"),
    );
    if let Some(disabled) = get_entry(&game_ini, "ADDON", "DisabledAddons") {
        let retained = disabled
            .split(',')
            .map(str::trim)
            .filter(|item| {
                !addon_names.iter().any(|addon| {
                    item.eq_ignore_ascii_case(addon)
                        || item.eq_ignore_ascii_case(addon.split('.').next().unwrap_or(addon))
                })
            })
            .collect::<Vec<_>>()
            .join(",");
        set_entry(&mut game_ini, "ADDON", "DisabledAddons", &retained);
    }
    let preset_ini = merge_document(hoyo_preset, dlss_preset)?;
    Ok(ManagedReShadeConfig {
        game_ini,
        preset_ini,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_host_paths_and_feeder_techniques() {
        let result = compose(
            "[ADDON]\r\nAddonPath=C:\\HoYo\\Addons\\\r\nDisabledAddons=renodx-dlss5,unrelated\r\n[GENERAL]\r\nEffectSearchPaths=C:\\HoYo\\Shaders\\**\r\nPresetPath=C:\\HoYo\\Presets\\Mod OFF.ini\r\n",
            "[ADDON]\nAddonPath=.\\\n[GENERAL]\nEffectSearchPaths=.\\reshade-shaders\\Shaders\\**\nPresetPath=.\\ReShadePreset.ini\n",
            "Techniques=HoYo@hoyo.fx\n", "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n",
            &["renodx-dlss5.addon64".into()],
        ).unwrap();
        assert!(result.game_ini.contains("AddonPath=C:\\HoYo\\Addons\\,.\\"));
        assert!(result
            .game_ini
            .contains("EffectSearchPaths=C:\\HoYo\\Shaders\\**,.\\reshade-shaders\\Shaders\\**"));
        assert!(result.game_ini.contains("DisabledAddons=unrelated"));
        assert!(result.game_ini.contains("PresetPath=.\\ReShadePreset.ini"));
        assert!(result
            .preset_ini
            .contains("Techniques=DLSS5_Feed@DLSS5_Feed.fx,HoYo@hoyo.fx"));
    }

    #[test]
    fn reports_conflicting_scalar_key() {
        let error = compose(
            "[DEPTH]\nUseAspectRatioHeuristics=1\n",
            "[DEPTH]\nUseAspectRatioHeuristics=0\n",
            "",
            "",
            &[],
        )
        .unwrap_err();
        assert_eq!(error.section, "DEPTH");
        assert_eq!(error.key, "UseAspectRatioHeuristics");
    }

    #[test]
    fn reports_conflicting_preprocessor_definition() {
        let error = compose(
            "[GENERAL]\nPreprocessorDefinitions=DEPTH=1\n",
            "[GENERAL]\nPreprocessorDefinitions=DEPTH=0\n",
            "",
            "",
            &[],
        )
        .unwrap_err();
        assert_eq!(error.key, "PreprocessorDefinitions");
    }
}
