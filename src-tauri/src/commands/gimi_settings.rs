use serde::Deserialize;
use serde_json::{json, Value};

// 原神会同时保存运行时画质和性能预设；只修改其中一份会在下次启动时被覆盖。
// 与 XXMI GIMI 一致，关闭动态角色分辨率，避免 Mod 使用不同 LOD 的骨骼/顶点缓冲。
#[cfg(test)]
fn disable_dynamic_character_resolution(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    update_graphics_settings(data, true, None)
}

fn update_graphics_settings(
    data: &[u8],
    disable_dynamic: bool,
    fsr2: Option<bool>,
) -> Result<Option<Vec<u8>>, String> {
    let end = data
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(data.len());
    let mut root: Value = serde_json::from_slice(&data[..end])
        .map_err(|error| format!("原神画质设置格式无法读取：{error}"))?;
    let mut changed = false;
    for (field, list, key, value_key, disabled) in [
        ("graphicsData", "customVolatileGrades", "key", "value", 1),
        ("globalPerfData", "saveItems", "entryType", "index", 0),
    ] {
        let raw = root
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("原神画质设置缺少 {field}"))?;
        let mut settings: Value = serde_json::from_str(raw)
            .map_err(|error| format!("原神 {field} 格式无法读取：{error}"))?;
        let entries = settings
            .get_mut(list)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("原神 {field} 缺少 {list}"))?;
        let mut found = false;
        let mut field_changed = false;
        for entry in entries
            .iter_mut()
            .filter(|entry| disable_dynamic && entry.get(key) == Some(&json!(21)))
        {
            found = true;
            let current = entry
                .get(value_key)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("原神动态角色分辨率 {value_key} 格式未知"))?;
            if current != disabled {
                if current != if disabled == 1 { 2 } else { 1 } {
                    return Err("原神动态角色分辨率取值未知，未修改游戏设置".to_string());
                }
                entry[value_key] = json!(disabled);
                field_changed = true;
            }
        }
        if disable_dynamic && !found {
            let mut entry = json!({key: 21, value_key: disabled});
            if field == "globalPerfData" {
                entry["itemVersion"] = json!("OSRELWin5.0.0");
            }
            entries.push(entry);
            field_changed = true;
        }
        if let Some(require_fsr2) = fsr2 {
            // Collapse 的原神设置模型：AA 为 entryType=8，Off/FSR2/SMAA 的 index 为 0/1/2，
            // graphicsData 的 value 比 index 大一。Feeder 自行重建，统一关闭游戏抗锯齿。
            let offset = if field == "graphicsData" { 1 } else { 0 };
            let mut found = false;
            for entry in entries
                .iter_mut()
                .filter(|entry| entry.get(key) == Some(&json!(8)))
            {
                found = true;
                let current = entry
                    .get(value_key)
                    .and_then(Value::as_u64)
                    .ok_or("原神抗锯齿设置格式未知")?;
                if !(offset..=offset + 2).contains(&current) {
                    return Err("原神抗锯齿取值未知，未修改游戏设置".into());
                }
                let desired = offset + u64::from(require_fsr2);
                if desired != current {
                    entry[value_key] = json!(desired);
                    field_changed = true;
                }
            }
            if !found {
                let mut entry = json!({key: 8, value_key: offset + u64::from(require_fsr2)});
                if field == "globalPerfData" {
                    entry["itemVersion"] = json!("OSRELWin5.0.0");
                }
                entries.push(entry);
                field_changed = true;
            }
        }
        if field_changed {
            root[field] =
                Value::String(serde_json::to_string(&settings).map_err(|error| error.to_string())?);
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }

    // 保持官方注册表记录的 ASCII JSON 编码，包括非 ASCII 字符的 UTF-16 转义。
    let text = serde_json::to_string(&root).map_err(|error| error.to_string())?;
    let mut encoded = String::new();
    for ch in text.chars() {
        if ch.is_ascii() {
            encoded.push(ch);
        } else {
            use std::fmt::Write;
            for unit in ch.encode_utf16(&mut [0; 2]) {
                write!(&mut encoded, "\\u{unit:04x}").map_err(|error| error.to_string())?;
            }
        }
    }
    let mut bytes = encoded.into_bytes();
    bytes.push(0);
    Ok(Some(bytes))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureGimiLaunchSettingsOptions {
    pub target_exe_path: String,
    pub configure_game: bool,
}

#[cfg(windows)]
fn configure(target: &str, disable_dynamic: bool, fsr2: Option<bool>) -> Result<(), String> {
    use crate::config::path_manager::PathManager;
    use std::{fs, io::Write, path::Path, ptr};
    use windows_sys::Win32::System::Registry::*;

    let executable = Path::new(target);
    if !executable.is_file() {
        return Err("原神游戏程序不存在".to_string());
    }
    let game_key = match executable
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "yuanshen.exe" => "原神",
        "genshinimpact.exe" => "Genshin Impact",
        _ => return Err("GIMI 设置只适用于 YuanShen.exe / GenshinImpact.exe".to_string()),
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let key_name = wide(&format!("SOFTWARE\\miHoYo\\{game_key}"));
    let value_name = wide("GENERAL_DATA_h2389025596");
    let mut raw_key = ptr::null_mut();
    let error = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut raw_key,
        )
    };
    if error != 0 {
        return Err(format!(
            "无法打开原神设置，请先通过官方启动器运行一次游戏（{error}）"
        ));
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let key = Key(raw_key);
    let read = || -> Result<Vec<u8>, String> {
        let (mut kind, mut size) = (0, 0);
        let error = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                &mut size,
            )
        };
        if error != 0 || kind != REG_BINARY || size == 0 || size > 4 * 1024 * 1024 {
            return Err(format!("原神画质设置不是有效的 REG_BINARY（{error}）"));
        }
        let mut bytes = vec![0; size as usize];
        let error = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                ptr::null(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut size,
            )
        };
        if error != 0 || kind != REG_BINARY {
            return Err(format!("无法读取原神画质设置（{error}）"));
        }
        bytes.truncate(size as usize);
        Ok(bytes)
    };
    let original = read()?;
    let Some(updated) = update_graphics_settings(&original, disable_dynamic, fsr2)? else {
        return Ok(());
    };
    let folder = PathManager::ssmt_global_config_folder()
        .join("GameSettingsBackups")
        .join("GIMI")
        .join(game_key);
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let backup = folder.join(format!(
        "GENERAL_DATA-{}.bin",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .and_then(|mut file| file.write_all(&original))
        .map_err(|error| format!("无法备份原神设置：{error}"))?;
    if read()? != original {
        return Err("原神设置已被其他程序修改，请关闭游戏后重试".to_string());
    }
    let error = unsafe {
        RegSetValueExW(
            key.0,
            value_name.as_ptr(),
            0,
            REG_BINARY,
            updated.as_ptr(),
            updated.len() as u32,
        )
    };
    if error != 0 {
        return Err(format!("无法写入原神画质设置（{error}）"));
    }
    println!(
        "[GameLauncher] 原神画质设置已更新：关闭动态角色分辨率={disable_dynamic}，FSR2={fsr2:?}；原始设置备份：{}", backup.display()
    );
    Ok(())
}

#[tauri::command]
pub async fn configure_gimi_launch_settings(
    options: ConfigureGimiLaunchSettingsOptions,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || configure_gimi_launch_settings_blocking(options))
        .await
        .map_err(|error| format!("后台启动任务失败：{error}"))?
}

fn configure_gimi_launch_settings_blocking(
    options: ConfigureGimiLaunchSettingsOptions,
) -> Result<(), String> {
    if !options.configure_game {
        return Ok(());
    }
    #[cfg(windows)]
    {
        configure(&options.target_exe_path, true, None)
    }
    #[cfg(not(windows))]
    {
        Err("原神画质设置仅适用于 Windows".to_string())
    }
}

pub(crate) fn configure_dlss5(target: &str, fsr2: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        configure(target, false, Some(fsr2))
    }
    #[cfg(not(windows))]
    {
        let _ = (target, fsr2);
        Err("原神画质设置仅适用于 Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(graphics: Value, perf: Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&json!({
            "graphicsData": json!({"customVolatileGrades": graphics, "untouched": "画质😀"}).to_string(),
            "globalPerfData": json!({"saveItems": perf}).to_string(),
            "other": [1, 2, 3],
        })).unwrap();
        bytes.push(0);
        bytes
    }
    #[test]
    fn dlss_routes_control_both_aa_records_without_touching_dynamic_resolution() {
        let source = fixture(
            json!([{"key":8,"value":3},{"key":21,"value":2},{"key":2,"value":8}]),
            json!([{"entryType":8,"index":1,"itemVersion":"CNRELWin7.1.0"},{"entryType":21,"index":1}]),
        );
        for fsr2 in [true, false] {
            let updated = update_graphics_settings(&source, false, Some(fsr2))
                .unwrap()
                .unwrap();
            let root: Value = serde_json::from_slice(&updated[..updated.len() - 1]).unwrap();
            let graphics: Value =
                serde_json::from_str(root["graphicsData"].as_str().unwrap()).unwrap();
            let perf: Value =
                serde_json::from_str(root["globalPerfData"].as_str().unwrap()).unwrap();
            assert_eq!(
                graphics["customVolatileGrades"][0]["value"],
                1 + u64::from(fsr2)
            );
            assert_eq!(perf["saveItems"][0]["index"], u64::from(fsr2));
            assert_eq!(perf["saveItems"][0]["itemVersion"], "CNRELWin7.1.0");
            assert_eq!(graphics["customVolatileGrades"][1]["value"], 2);
            assert_eq!(graphics["customVolatileGrades"][2]["value"], 8);
            assert_eq!(perf["saveItems"][1]["index"], 1);
            assert_eq!(
                update_graphics_settings(&updated, false, Some(fsr2)).unwrap(),
                None
            );
        }
        assert_eq!(
            update_graphics_settings(&source, false, None).unwrap(),
            None
        );
    }

    #[test]
    fn aa_rejects_future_values_and_repairs_missing_record() {
        for (value, index) in [(4, 1), (2, 3)] {
            let source = fixture(
                json!([{"key":8,"value":value}]),
                json!([{"entryType":8,"index":index}]),
            );
            assert!(update_graphics_settings(&source, false, Some(true)).is_err());
        }
        let updated = update_graphics_settings(&fixture(json!([]), json!([])), true, Some(true))
            .unwrap()
            .unwrap();
        assert_eq!(
            update_graphics_settings(&updated, true, Some(true)).unwrap(),
            None
        );
    }
    #[test]
    fn disables_both_records_and_preserves_other_settings() {
        let source = fixture(
            json!([{"key":21,"value":2},{"key":3,"value":4}]),
            json!([{"entryType":21,"index":1,"itemVersion":"CNRELWin7.1.0"}]),
        );
        let updated = disable_dynamic_character_resolution(&source)
            .unwrap()
            .unwrap();
        assert!(updated.is_ascii());
        assert_eq!(updated.last(), Some(&0));
        let root: Value = serde_json::from_slice(&updated[..updated.len() - 1]).unwrap();
        let graphics: Value = serde_json::from_str(root["graphicsData"].as_str().unwrap()).unwrap();
        let perf: Value = serde_json::from_str(root["globalPerfData"].as_str().unwrap()).unwrap();
        assert_eq!(
            graphics["customVolatileGrades"],
            json!([{"key":21,"value":1},{"key":3,"value":4}])
        );
        assert_eq!(graphics["untouched"], "画质😀");
        assert_eq!(
            perf["saveItems"][0],
            json!({"entryType":21,"index":0,"itemVersion":"CNRELWin7.1.0"})
        );
        assert_eq!(root["other"], json!([1, 2, 3]));
        assert_eq!(
            disable_dynamic_character_resolution(&updated).unwrap(),
            None
        );
    }
    #[test]
    fn adds_missing_entries_and_handles_unterminated_json() {
        let mut source = fixture(json!([]), json!([]));
        source.pop();
        let updated = disable_dynamic_character_resolution(&source)
            .unwrap()
            .unwrap();
        assert_eq!(
            disable_dynamic_character_resolution(&updated).unwrap(),
            None
        );
    }
    #[test]
    fn repairs_inconsistent_records() {
        for (value, index) in [(1, 1), (2, 0)] {
            let source = fixture(
                json!([{"key":21,"value":value}]),
                json!([{"entryType":21,"index":index}]),
            );
            assert!(disable_dynamic_character_resolution(&source)
                .unwrap()
                .is_some());
        }
    }
    #[test]
    fn rejects_unknown_format_without_partial_output() {
        for source in [
            b"invalid".to_vec(),
            b"{}".to_vec(),
            fixture(json!([{"key":21,"value":3}]), json!([])),
            fixture(json!([]), json!({})),
        ] {
            assert!(disable_dynamic_character_resolution(&source).is_err());
        }
    }
}
