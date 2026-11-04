use super::registry::{game_scope_preset, PluginRegistry};
use super::PluginPermission;
use crate::config::path_manager::PathManager;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, System};

pub const SECONDARY_MOTION_PLUGIN_ID: &str = "ssmt.secondary-motion";
const CONFIG_RELATIVE_PATH: &str = "runtime/config.json";
const STATUS_RELATIVE_PATH: &str = "runtime/runtime_status.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecondaryMotionSnapshot {
    plugin_id: String,
    plugin_name: String,
    plugin_version: String,
    game_name: String,
    ssmt_enabled: bool,
    runtime_enabled: bool,
    revision: u64,
    active_preset: String,
    presets: Vec<String>,
    package_root: String,
    data_root: String,
    dll_path: String,
    status: Value,
    game_running: bool,
    character_id: String,
    character_in_preset: bool,
    character_settings: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInjectedDll {
    pub plugin_id: String,
    pub plugin_name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInjectedDllsResult {
    pub dlls: Vec<PluginInjectedDll>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn secondary_motion_snapshot(
    plugin_id: String,
    game_name: String,
) -> Result<SecondaryMotionSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        secondary_motion_snapshot_blocking(&plugin_id, &game_name)
    })
    .await
    .map_err(|error| format!("后台读取次级运动配置失败：{error}"))?
}

#[tauri::command]
pub async fn update_secondary_motion_settings(
    plugin_id: String,
    game_name: String,
    runtime_enabled: bool,
    active_preset: String,
) -> Result<SecondaryMotionSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update_secondary_motion_settings_blocking(
            &plugin_id,
            &game_name,
            runtime_enabled,
            &active_preset,
        )?;
        secondary_motion_snapshot_blocking(&plugin_id, &game_name)
    })
    .await
    .map_err(|error| format!("后台保存次级运动配置失败：{error}"))?
}

#[tauri::command]
pub async fn update_secondary_motion_character(
    plugin_id: String,
    game_name: String,
    character_id: String,
    character_settings: Value,
) -> Result<SecondaryMotionSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update_secondary_motion_character_blocking(
            &plugin_id,
            &game_name,
            &character_id,
            character_settings,
        )?;
        secondary_motion_snapshot_blocking(&plugin_id, &game_name)
    })
    .await
    .map_err(|error| format!("后台保存角色参数失败：{error}"))?
}

#[tauri::command]
pub async fn plugin_launch_injected_dlls(
    game_name: String,
) -> Result<PluginInjectedDllsResult, String> {
    tauri::async_runtime::spawn_blocking(move || plugin_launch_injected_dlls_blocking(&game_name))
        .await
        .map_err(|error| format!("后台读取额外注入 DLL 失败：{error}"))?
}

fn secondary_motion_snapshot_blocking(
    plugin_id: &str,
    game_name: &str,
) -> Result<SecondaryMotionSnapshot, String> {
    let context = installed_context(plugin_id, game_name)?;
    sync_secondary_motion_data(&context.data_root)?;
    let (config, revision, active_preset) = read_runtime_config(&context.data_root)?;
    let presets = list_presets(&context.data_root)?;
    let status = read_json_if_present(&context.data_root.join(STATUS_RELATIVE_PATH))
        .unwrap_or_else(|| serde_json::json!({ "state": "NOT_STARTED" }));
    let game_running = game_process_running(game_name);
    let character_id = status
        .get("character")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let (character_in_preset, character_settings) =
        read_character_settings(&context.data_root, &active_preset, &character_id)?;
    let config_enabled = config
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SecondaryMotionSnapshot {
        plugin_id: context.plugin_id,
        plugin_name: context.plugin_name,
        plugin_version: context.plugin_version,
        game_name: game_name.to_string(),
        ssmt_enabled: context.ssmt_enabled,
        runtime_enabled: config_enabled,
        revision,
        active_preset,
        presets,
        package_root: context.package_root.to_string_lossy().into_owned(),
        data_root: context.data_root.to_string_lossy().into_owned(),
        dll_path: context.dll_path.to_string_lossy().into_owned(),
        status,
        game_running,
        character_id,
        character_in_preset,
        character_settings,
    })
}

fn update_secondary_motion_settings_blocking(
    plugin_id: &str,
    game_name: &str,
    runtime_enabled: bool,
    active_preset: &str,
) -> Result<(), String> {
    let context = installed_context(plugin_id, game_name)?;
    sync_secondary_motion_data(&context.data_root)?;
    let active_preset = active_preset.trim();
    if active_preset.is_empty() {
        return Err("当前预设不能为空".into());
    }
    if active_preset
        .chars()
        .any(|ch| matches!(ch, '.' | '/' | '\\' | ':'))
    {
        return Err("当前预设包含不允许的路径字符".into());
    }
    let preset_path = context
        .data_root
        .join("presets")
        .join(format!("{active_preset}.json"));
    if !preset_path.is_file() {
        return Err(format!("预设不存在：{}", preset_path.display()));
    }

    let config_path = context.data_root.join(CONFIG_RELATIVE_PATH);
    let (mut config, revision, _) = read_runtime_config(&context.data_root)?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "runtime/config.json 根节点必须是对象".to_string())?;
    object.insert("revision".into(), Value::from(revision.saturating_add(1)));
    object.insert("enabled".into(), Value::Bool(runtime_enabled));
    object.insert("active_preset".into(), Value::String(active_preset.into()));
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(object.clone()))
        .map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_atomic(&config_path, &bytes)?;
    mirror_secondary_motion_data(&context.data_root)
}

fn update_secondary_motion_character_blocking(
    plugin_id: &str,
    game_name: &str,
    character_id: &str,
    character_settings: Value,
) -> Result<(), String> {
    let context = installed_context(plugin_id, game_name)?;
    sync_secondary_motion_data(&context.data_root)?;
    let character_id = character_id.trim();
    if !valid_character_id(character_id) {
        return Err("当前角色 ID 无效".into());
    }
    validate_character_settings(&character_settings)?;
    let (_, _, active_preset) = read_runtime_config(&context.data_root)?;
    let preset_path = context
        .data_root
        .join("presets")
        .join(format!("{active_preset}.json"));
    let mut preset = read_json_if_present(&preset_path)
        .ok_or_else(|| format!("当前预设无法读取：{}", preset_path.display()))?;
    let root = preset
        .as_object_mut()
        .ok_or_else(|| "预设根节点必须是对象".to_string())?;
    let characters = root
        .entry("characters")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "预设 characters 必须是对象".to_string())?;
    characters.insert(character_id.to_string(), character_settings);
    let mut preset_bytes = serde_json::to_vec_pretty(&preset).map_err(|error| error.to_string())?;
    preset_bytes.push(b'\n');

    let backup = preset_path.with_extension("json.bak");
    if preset_path.is_file() {
        fs::copy(&preset_path, &backup).map_err(|error| error.to_string())?;
    }
    write_atomic(&preset_path, &preset_bytes)?;
    bump_runtime_revision(&context.data_root)?;
    mirror_secondary_motion_data(&context.data_root)
}

fn plugin_launch_injected_dlls_blocking(
    game_name: &str,
) -> Result<PluginInjectedDllsResult, String> {
    if game_name.trim().is_empty() {
        return Ok(PluginInjectedDllsResult {
            dlls: Vec::new(),
            warnings: Vec::new(),
        });
    }
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let scoped = registry.for_game(game_name);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut warnings = Vec::new();

    for plugin in scoped.selected_plugins().filter(|plugin| plugin.enabled) {
        let injected = &plugin.manifest.contributions.injected_dlls;
        if injected.is_empty() {
            continue;
        }
        if !plugin
            .manifest
            .permissions
            .iter()
            .any(|permission| *permission == PluginPermission::NativeInject)
        {
            warnings.push(format!(
                "插件 {} v{} 声明了额外注入 DLL，但缺少 native.inject 权限",
                plugin.manifest.name, plugin.manifest.version
            ));
            continue;
        }
        if !super::commands::has_native_injection_risk_ack(plugin) {
            warnings.push(format!(
                "插件 {} v{} 尚未接受原生注入风险与免责声明，已跳过注入",
                plugin.manifest.name, plugin.manifest.version
            ));
            continue;
        }
        if plugin.manifest.id == SECONDARY_MOTION_PLUGIN_ID {
            sync_secondary_motion_data(&plugin.package_root.join("SecondaryMotion"))?;
        }
        for contribution in injected {
            if let Err(error) = contribution.verify_file(&plugin.package_root) {
                warnings.push(format!(
                    "插件 {} v{} 的注入 DLL 校验失败：{error}",
                    plugin.manifest.name, plugin.manifest.version
                ));
                continue;
            }
            let path = plugin.package_root.join(&contribution.path);
            let key = path
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase();
            if seen.insert(key) {
                result.push(PluginInjectedDll {
                    plugin_id: plugin.manifest.id.clone(),
                    plugin_name: plugin.manifest.name.clone(),
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
    result.sort_by(|left, right| {
        left.path
            .to_ascii_lowercase()
            .cmp(&right.path.to_ascii_lowercase())
    });
    Ok(PluginInjectedDllsResult {
        dlls: result,
        warnings,
    })
}

struct InstalledContext {
    plugin_id: String,
    plugin_name: String,
    plugin_version: String,
    ssmt_enabled: bool,
    package_root: PathBuf,
    data_root: PathBuf,
    dll_path: PathBuf,
}

fn persistent_data_root() -> PathBuf {
    PathManager::ssmt_global_config_folder()
        .join("PluginData")
        .join(SECONDARY_MOTION_PLUGIN_ID)
        .join("SecondaryMotion")
}

fn game_process_running(game_name: &str) -> bool {
    let config_path = PathManager::global_config_games_folder()
        .join(game_name)
        .join("Config.json");
    let Some(config) = read_json_if_present(&config_path) else {
        return false;
    };
    let Some(target_exe) = config.get("targetExePath").and_then(Value::as_str) else {
        return false;
    };
    let Some(process_name) = Path::new(target_exe)
        .file_name()
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessRefreshKind::new());
    system
        .processes()
        .values()
        .any(|process| process.name().eq_ignore_ascii_case(process_name))
}

fn sync_secondary_motion_data(package_data_root: &Path) -> Result<(), String> {
    let persistent_root = persistent_data_root();
    if !persistent_root.exists() {
        copy_tree(package_data_root, &persistent_root)?;
        return Ok(());
    }
    let package_revision = read_runtime_config(package_data_root)
        .map(|(_, revision, _)| revision)
        .unwrap_or(0);
    let persistent_revision = read_runtime_config(&persistent_root)
        .map(|(_, revision, _)| revision)
        .unwrap_or(0);
    if persistent_revision > package_revision {
        copy_tree(&persistent_root, package_data_root)?;
    } else if package_revision > persistent_revision {
        copy_tree(package_data_root, &persistent_root)?;
    }
    Ok(())
}

fn mirror_secondary_motion_data(package_data_root: &Path) -> Result<(), String> {
    copy_tree(package_data_root, &persistent_data_root())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err(format!(
            "Secondary Motion 数据目录不存在：{}",
            source.display()
        ));
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut pending = vec![source.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let source_path = entry.path();
            let relative = source_path
                .strip_prefix(source)
                .map_err(|error| error.to_string())?;
            let destination_path = destination.join(relative);
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                return Err(format!(
                    "Secondary Motion 数据目录不能包含符号链接：{}",
                    source_path.display()
                ));
            }
            if file_type.is_dir() {
                fs::create_dir_all(&destination_path).map_err(|error| error.to_string())?;
                pending.push(source_path);
            } else if file_type.is_file() {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::copy(&source_path, &destination_path).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

fn installed_context(plugin_id: &str, game_name: &str) -> Result<InstalledContext, String> {
    if plugin_id != SECONDARY_MOTION_PLUGIN_ID {
        return Err(format!("不支持的次级运动插件 ID：{plugin_id}"));
    }
    if game_name.trim().is_empty() {
        return Err("请先选择游戏".into());
    }
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let plugin = registry
        .find(plugin_id)
        .ok_or_else(|| "Secondary Motion 插件尚未安装".to_string())?;
    if !plugin
        .manifest
        .compatibility
        .supports_game(&game_scope_preset(game_name))
    {
        return Err(format!(
            "{} v{} 不支持当前游戏 {}",
            plugin.manifest.name, plugin.manifest.version, game_name
        ));
    }
    let dll_contribution = plugin
        .manifest
        .contributions
        .injected_dlls
        .first()
        .ok_or_else(|| "插件清单没有声明 injectedDlls".to_string())?;
    let dll_path = plugin.package_root.join(&dll_contribution.path);
    if !dll_path.is_file() {
        return Err(format!("注入 DLL 不存在：{}", dll_path.display()));
    }
    let data_root = plugin.package_root.join("SecondaryMotion");
    if !data_root.is_dir() {
        return Err(format!(
            "Secondary Motion 数据目录不存在：{}",
            data_root.display()
        ));
    }
    Ok(InstalledContext {
        plugin_id: plugin.manifest.id.clone(),
        plugin_name: plugin.manifest.name.clone(),
        plugin_version: plugin.manifest.version.clone(),
        ssmt_enabled: registry.game_enabled(game_name, plugin_id),
        package_root: plugin.package_root.clone(),
        data_root,
        dll_path,
    })
}

fn read_runtime_config(data_root: &Path) -> Result<(Value, u64, String), String> {
    let path = data_root.join(CONFIG_RELATIVE_PATH);
    let config = read_json_if_present(&path).unwrap_or_else(|| {
        serde_json::json!({
            "revision": 0,
            "enabled": false,
            "active_preset": "Default"
        })
    });
    if !config.is_object() {
        return Err("runtime/config.json 根节点必须是对象".into());
    }
    let revision = config.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let active_preset = config
        .get("active_preset")
        .and_then(Value::as_str)
        .unwrap_or("Default")
        .to_string();
    Ok((config, revision, active_preset))
}

fn read_character_settings(
    data_root: &Path,
    active_preset: &str,
    character_id: &str,
) -> Result<(bool, Value), String> {
    if character_id.trim().is_empty() {
        return Ok((false, serde_json::json!({})));
    }
    let preset_path = data_root
        .join("presets")
        .join(format!("{active_preset}.json"));
    let preset = read_json_if_present(&preset_path).unwrap_or_else(|| serde_json::json!({}));
    let preset_entry = preset
        .get("characters")
        .and_then(Value::as_object)
        .and_then(|characters| characters.get(character_id));

    let database = read_json_if_present(&data_root.join("data/characters.default.json"))
        .unwrap_or_else(|| serde_json::json!({}));
    let database_entry = database
        .get("characters")
        .and_then(Value::as_object)
        .and_then(|characters| characters.get(character_id));
    let mut defaults = serde_json::json!({
        "enabled": true,
        "motion_mode": "synthetic",
        "bones": {
            "right": "",
            "left": "",
            "allow_fallback_candidates": true
        },
        "axis": { "name": "Y", "sign": 1 },
        "gait": {
            "idle": { "amplitude_deg": 0, "frequency_hz": 1.2 },
            "walk": { "amplitude_deg": 3.6, "frequency_hz": 1.5 },
            "run": { "amplitude_deg": 8.5, "frequency_hz": 1.7, "phase_offset_deg": 180 },
            "sprint": { "amplitude_deg": 12, "frequency_hz": 2, "phase_offset_deg": 180 },
            "zipline": { "amplitude_deg": 8.5, "frequency_hz": 1.7 }
        },
        "envelope": {
            "amplitude_attack_tau_sec": 0.15,
            "frequency_tau_sec": 0.2,
            "to_idle_release_tau_sec": 0.02
        },
        "jump": {
            "enabled": false,
            "mode": "off",
            "amplitude_deg": 23.333,
            "damping_tau_sec": 0.35,
            "frequency_hz": 3.0,
            "max_duration_sec": 1.2,
            "takeoff_delay_sec": 0.08,
            "rising_target_deg": -23.333,
            "apex_falling_target_deg": 23.333,
            "acceleration_response": 0.1667,
            "acceleration_filter_tau_sec": 0.08,
            "natural_frequency_hz": 2.2,
            "damping_ratio": 0.52,
            "landing_impulse_gain": 6.667
        },
        "native_amplify": { "factor": 2.0 }
    });
    if let Some(entry) = database_entry {
        if let Some(bones) = entry.get("bones") {
            defaults["bones"] = bones.clone();
        }
        if let Some(axis) = entry.get("axis") {
            defaults["axis"] = axis.clone();
        }
        if let Some(database_defaults) = entry.get("defaults").and_then(Value::as_object) {
            for key in ["gait", "envelope", "jump", "native_amplify"] {
                if let Some(value) = database_defaults.get(key) {
                    defaults[key] = value.clone();
                }
            }
        }
    }
    if let Some(entry) = preset_entry {
        merge_json(&mut defaults, entry);
        Ok((true, defaults))
    } else {
        Ok((false, defaults))
    }
}

fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(key) {
                    merge_json(existing, value);
                } else {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn valid_character_id(character_id: &str) -> bool {
    character_id.starts_with("chr_")
        && character_id.len() <= 96
        && character_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn validate_character_settings(settings: &Value) -> Result<(), String> {
    let object = settings
        .as_object()
        .ok_or_else(|| "角色参数根节点必须是对象".to_string())?;
    if let Some(enabled) = object.get("enabled") {
        if !enabled.is_boolean() {
            return Err("enabled 必须是布尔值".into());
        }
    }
    if let Some(mode) = object.get("motion_mode") {
        if !matches!(mode.as_str(), Some("off" | "synthetic" | "amplify_native")) {
            return Err("motion_mode 必须是 off、synthetic 或 amplify_native".into());
        }
    }
    if let Some(bones) = object.get("bones").and_then(Value::as_object) {
        let right = bones.get("right").and_then(Value::as_str).unwrap_or("");
        let left = bones.get("left").and_then(Value::as_str).unwrap_or("");
        if right.is_empty() != left.is_empty() {
            return Err("左右骨骼必须同时填写或同时留空".into());
        }
    }
    if let Some(axis) = object.get("axis").and_then(Value::as_object) {
        if let Some(name) = axis.get("name") {
            if !matches!(name.as_str(), Some("X" | "Y" | "Z")) {
                return Err("axis.name 必须是 X、Y 或 Z".into());
            }
        }
        if let Some(sign) = axis.get("sign") {
            if !matches!(sign.as_f64(), Some(1.0 | -1.0)) {
                return Err("axis.sign 必须是 1 或 -1".into());
            }
        }
    }
    for section in ["gait", "envelope", "jump", "native_amplify"] {
        if let Some(value) = object.get(section) {
            validate_numeric_tree(value, section)?;
        }
    }
    Ok(())
}

fn validate_numeric_tree(value: &Value, path: &str) -> Result<(), String> {
    match value {
        Value::Number(number) => {
            if !number.as_f64().is_some_and(f64::is_finite) {
                return Err(format!("{path} 必须是有限数字"));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_numeric_tree(item, &format!("{path}[{index}]"))?;
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                validate_numeric_tree(item, &format!("{path}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn bump_runtime_revision(data_root: &Path) -> Result<(), String> {
    let config_path = data_root.join(CONFIG_RELATIVE_PATH);
    let (mut config, revision, _) = read_runtime_config(data_root)?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| "runtime/config.json 根节点必须是对象".to_string())?;
    object.insert("revision".into(), Value::from(revision.saturating_add(1)));
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(object.clone()))
        .map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    write_atomic(&config_path, &bytes)
}

fn list_presets(data_root: &Path) -> Result<Vec<String>, String> {
    let directory = data_root.join("presets");
    let mut presets = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("读取预设目录失败 {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".json")
            || name.to_ascii_lowercase().ends_with(".template.json")
        {
            continue;
        }
        if let Some(stem) = Path::new(&name)
            .file_stem()
            .and_then(|value| value.to_str())
        {
            presets.push(stem.to_string());
        }
    }
    presets.sort_by_key(|name| name.to_ascii_lowercase());
    Ok(presets)
}

fn read_json_if_present(path: &Path) -> Option<Value> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("配置文件没有父目录：{}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    replace_file(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(format!(
            "替换配置文件失败：Windows error {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_presets_without_templates_or_backups() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-secondary-motion-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("presets")).unwrap();
        fs::write(root.join("presets/Default.json"), b"{}").unwrap();
        fs::write(root.join("presets/User.json"), b"{}").unwrap();
        fs::write(root.join("presets/Default.template.json"), b"{}").unwrap();
        fs::write(root.join("presets/Default.json.bak"), b"{}").unwrap();
        assert_eq!(list_presets(&root).unwrap(), vec!["Default", "User"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_write_replaces_config() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-secondary-motion-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("config.json");
        fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_runtime_config_relative_to_secondary_motion_data_root() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-secondary-motion-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("runtime")).unwrap();
        fs::write(
            root.join(CONFIG_RELATIVE_PATH),
            br#"{"revision":7,"enabled":true,"active_preset":"User"}"#,
        )
        .unwrap();
        let (_, revision, preset) = read_runtime_config(&root).unwrap();
        assert_eq!(revision, 7);
        assert_eq!(preset, "User");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_character_parameter_groups() {
        validate_character_settings(&serde_json::json!({
            "enabled": true,
            "motion_mode": "synthetic",
            "bones": { "right": "R_breast", "left": "L_breast", "allow_fallback_candidates": true },
            "axis": { "name": "Y", "sign": -1 },
            "gait": { "walk": { "amplitude_deg": 12.5, "frequency_hz": 1.8 } },
            "envelope": { "amplitude_attack_tau_sec": 0.2 },
            "jump": { "enabled": true, "mode": "off", "landing_impulse_gain": 6.0 },
            "native_amplify": { "factor": 2.0 }
        }))
        .unwrap();
        assert!(validate_character_settings(&serde_json::json!({
            "motion_mode": "invalid"
        }))
        .is_err());
        assert!(validate_character_settings(&serde_json::json!({
            "bones": { "right": "R", "left": "" }
        }))
        .is_err());
    }

    #[test]
    fn builds_editable_defaults_for_character_missing_from_preset() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-secondary-motion-character-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("presets")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("presets/Default.json"),
            br#"{"schema_version":1,"characters":{}}"#,
        )
        .unwrap();
        fs::write(
            root.join("data/characters.default.json"),
            br#"{"schema_version":1,"characters":{}}"#,
        )
        .unwrap();
        let (present, settings) =
            read_character_settings(&root, "Default", "chr_0028_wulfa").unwrap();
        assert!(!present);
        assert_eq!(settings["motion_mode"], "synthetic");
        assert_eq!(settings["bones"]["right"], "");
        assert!(settings["gait"]["walk"]["amplitude_deg"].is_number());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn merges_database_bones_and_defaults_into_partial_preset_entry() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-secondary-motion-merge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("presets")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("presets/Default.json"),
            br#"{"schema_version":1,"characters":{"chr_test":{"gait":{"walk":{"amplitude_deg":99}}}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("data/characters.default.json"),
            br#"{"schema_version":1,"characters":{"chr_test":{"bones":{"right":"R_bone","left":"L_bone","allow_fallback_candidates":true},"axis":{"name":"Z","sign":1},"defaults":{"gait":{"walk":{"amplitude_deg":3.6,"frequency_hz":1.5}}}}}}"#,
        )
        .unwrap();
        let (present, settings) = read_character_settings(&root, "Default", "chr_test").unwrap();
        assert!(present);
        assert_eq!(settings["bones"]["right"], "R_bone");
        assert_eq!(settings["axis"]["name"], "Z");
        assert_eq!(settings["gait"]["walk"]["amplitude_deg"], 99.0);
        assert_eq!(settings["gait"]["walk"]["frequency_hz"], 1.5);
        fs::remove_dir_all(root).unwrap();
    }
}
