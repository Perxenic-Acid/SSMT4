//! 插件自有后台通过受限的 JSON 标准输入输出协议工作；宿主不解释业务方法。
use super::registry::{InstalledPlugin, PluginRegistry};
use super::PluginPermission;
use crate::config::path_manager::PathManager;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
static PREPARED: std::sync::Mutex<std::collections::BTreeSet<(String, String, String)>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

pub fn launch_allowed(game: &str, plugin: &InstalledPlugin) -> bool {
    if !plugin
        .manifest
        .contributions
        .backend
        .as_ref()
        .is_some_and(|backend| backend.events.iter().any(|event| event == "prepare"))
    {
        return true;
    }
    PREPARED
        .lock()
        .map(|prepared| {
            prepared.contains(&(
                game.to_owned(),
                plugin.id().to_owned(),
                plugin.version().to_owned(),
            ))
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackendContribution {
    pub executable: String,
    pub sha256: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
}

impl BackendContribution {
    pub fn validate(&self) -> Result<(), super::PluginManifestError> {
        super::validate_package_relative_path(&self.executable)?;
        if !self.executable.to_ascii_lowercase().ends_with(".exe")
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.methods.len() > 64
            || self.events.len() > 8
        {
            return Err(super::PluginManifestError::InvalidContribution(
                "invalid backend declaration".into(),
            ));
        }
        for method in &self.methods {
            super::validate_plugin_id(method)?;
        }
        for event in &self.events {
            if !matches!(
                event.as_str(),
                "prepare"
                    | "cancel"
                    | "disabled"
                    | "recover"
                    | "launch.describe"
                    | "launch.prepare"
                    | "launch.commit"
                    | "launch.cancel"
            ) {
                return Err(super::PluginManifestError::InvalidContribution(
                    "unsupported backend lifecycle event".into(),
                ));
            }
        }
        Ok(())
    }
}

pub async fn request(
    plugin: &InstalledPlugin,
    game: &str,
    method: &str,
    payload: Value,
    event: bool,
) -> Result<Value, String> {
    if (game.trim().is_empty() && !plugin.manifest.is_app_scoped())
        || matches!(game, "." | "..")
        || game.contains(['/', '\\', ':', '\0'])
    {
        return Err("invalid game scope".into());
    }
    if event && matches!(method, "prepare" | "cancel" | "disabled" | "recover") {
        PREPARED
            .lock()
            .map_err(|_| "backend preparation state unavailable")?
            .remove(&(
                game.to_owned(),
                plugin.id().to_owned(),
                plugin.version().to_owned(),
            ));
    }
    let backend = plugin
        .manifest
        .contributions
        .backend
        .as_ref()
        .ok_or("plugin has no backend")?;
    let allowed = if event {
        &backend.events
    } else {
        &backend.methods
    };
    if !allowed.iter().any(|item| item == method)
        || !plugin
            .manifest
            .permissions
            .contains(&PluginPermission::ProcessSpawn)
        || !plugin.manifest.supports_current_plugin_api()
    {
        return Err("backend method or permission is not declared".into());
    }
    let root = plugin
        .package_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let executable = root
        .join(&backend.executable)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !executable.starts_with(&root) {
        return Err("backend executable escapes package".into());
    }
    super::marketplace::verify_package_sha256(&executable, &backend.sha256)
        .map_err(|error| error.to_string())?;
    let data = PathManager::ssmt_global_config_folder()
        .join("PluginData")
        .join(plugin.id())
        .join("backend");
    std::fs::create_dir_all(&data).map_err(|error| error.to_string())?;
    let config = (!game.trim().is_empty())
        .then(|| PathManager::global_config_games_game_folder(game).join("Config.json"));
    let config: Value = config
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(json!({}));
    let settings = super::settings::PluginSettingsStore::from_default_location()
        .map_err(|error| error.to_string())?;
    let mut migrations = serde_json::Map::new();
    for migration in &plugin.manifest.contributions.migrations {
        let mut values = serde_json::Map::new();
        for key in &migration.keys {
            if let Some(value) = config.get(key) {
                values.insert(key.clone(), value.clone());
            }
        }
        migrations.insert(migration.id.clone(), Value::Object(values));
    }
    let input = json!({
        "protocolVersion": 1, "method": method, "event": event, "payload": payload,
        "context": { "pluginId": plugin.id(), "pluginVersion": plugin.manifest.version,
            "gameName": game, "gamePreset": super::registry::game_scope_preset(game),
            "gameExecutable": config.get("targetExePath"), "packageRoot": root,
            "dataRoot": data,
            "assetRoot": PathManager::ssmt_global_config_folder().join("PluginAssets").join(plugin.id()),
            "hostProcessId": std::process::id(),
            "externalDependencies": plugin.external_dependencies,
            "legacySettings": settings.namespace(plugin.id()), "migrations": migrations }
    });
    let bytes = serde_json::to_vec(&input).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err("backend request is too large".into());
    }
    let mut command = tokio::process::Command::new(executable);
    command
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut stdin = child.stdin.take().ok_or("backend stdin unavailable")?;
    let mut stdout = child.stdout.take().ok_or("backend stdout unavailable")?;
    let exchange = async {
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        stdin.shutdown().await.map_err(|error| error.to_string())?;
        drop(stdin);
        let mut output = Vec::new();
        (&mut stdout)
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|error| error.to_string())?;
        if output.len() as u64 > OUTPUT_LIMIT {
            return Err("backend response is too large".into());
        }
        let status = child.wait().await.map_err(|error| error.to_string())?;
        let result: Value = serde_json::from_slice(&output)
            .map_err(|error| format!("invalid backend response: {error}"))?;
        if !status.success() || result.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("backend failed")
                .to_owned());
        }
        Ok(result.get("value").cloned().unwrap_or(Value::Null))
    };
    tokio::time::timeout(std::time::Duration::from_secs(120), exchange)
        .await
        .map_err(|_| "backend timed out".to_string())?
}

#[tauri::command]
pub async fn plugin_backend_lifecycle(
    game_name: String,
    event: String,
    launch_context: Option<Value>,
) -> Result<Vec<String>, String> {
    if !matches!(
        event.as_str(),
        "prepare" | "cancel" | "recover" | "launch.cancel"
    ) {
        return Err("invalid lifecycle event".into());
    }
    let Some(registry) =
        super::commands::installed_plugin_registry()?.map(|registry| registry.for_game(&game_name))
    else {
        return Ok(Vec::new());
    };
    lifecycle_for_registry(&registry, &game_name, &event, launch_context).await
}

async fn lifecycle_for_registry(
    registry: &PluginRegistry,
    game_name: &str,
    event: &str,
    launch_context: Option<Value>,
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    for plugin in registry.selected_plugins() {
        if !plugin.enabled {
            continue;
        }
        if matches!(event, "cancel" | "recover" | "launch.cancel")
            && plugin
                .manifest
                .contributions
                .backend
                .as_ref()
                .is_some_and(|backend| {
                    backend
                        .events
                        .iter()
                        .any(|candidate| candidate == "launch.cancel")
                })
        {
            if let Err(error) = request(
                plugin,
                game_name,
                "launch.cancel",
                json!({"launchContext":launch_context}),
                true,
            )
            .await
            {
                warnings.push(format!("{}: {error}", plugin.manifest.name));
            }
        }
        if event == "launch.cancel" {
            continue;
        }
        if !plugin
            .manifest
            .contributions
            .backend
            .as_ref()
            .is_some_and(|backend| backend.events.iter().any(|candidate| candidate == event))
        {
            continue;
        }
        let key = (
            game_name.to_owned(),
            plugin.id().to_owned(),
            plugin.version().to_owned(),
        );
        PREPARED
            .lock()
            .map_err(|_| "backend preparation state unavailable")?
            .remove(&key);
        match request(
            plugin,
            &game_name,
            &event,
            launch_context.clone().unwrap_or(Value::Null),
            true,
        )
        .await
        {
            Ok(_) if event == "prepare" => {
                PREPARED
                    .lock()
                    .map_err(|_| "backend preparation state unavailable")?
                    .insert(key);
            }
            Ok(_) => {}
            Err(error) => {
                warnings.push(format!("{}: {error}", plugin.manifest.name));
                if event == "prepare"
                    && plugin
                        .manifest
                        .contributions
                        .backend
                        .as_ref()
                        .is_some_and(|backend| backend.events.iter().any(|event| event == "cancel"))
                {
                    if let Err(error) =
                        request(plugin, &game_name, "cancel", Value::Null, true).await
                    {
                        warnings.push(format!("{}: {error}", plugin.manifest.name));
                    }
                }
            }
        }
    }
    Ok(warnings)
}

pub fn verify_backend(root: &Path, backend: &BackendContribution) -> Result<(), String> {
    super::marketplace::verify_package_sha256(&root.join(&backend.executable), &backend.sha256)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn no_plugins_do_not_create_backend_data_or_require_executables() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-empty-backends-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = PluginRegistry::new(&root).unwrap();
        assert!(
            lifecycle_for_registry(&registry, "不存在的游戏配置", "prepare", None)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn backend_declarations_reject_escape_hash_and_unknown_lifecycle_events() {
        let backend = BackendContribution {
            executable: "backend/worker.exe".into(),
            sha256: "a".repeat(64),
            methods: vec!["settings.read".into()],
            events: vec!["prepare".into(), "disabled".into()],
        };
        assert!(backend.validate().is_ok());
        let mut invalid = backend.clone();
        invalid.executable = "../worker.exe".into();
        assert!(invalid.validate().is_err());
        let mut invalid = backend.clone();
        invalid.sha256 = "z".repeat(64);
        assert!(invalid.validate().is_err());
        let mut invalid = backend.clone();
        invalid.events.push("execute-anything".into());
        assert!(invalid.validate().is_err());
        let mut invalid = backend;
        invalid.methods.push("../method".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn injected_dll_conditions_follow_package_declarations() {
        let dll = super::super::InjectedDllContribution {
            path: "runtime/test.dll".into(),
            sha256: None,
            executable_names: vec!["Target.exe".into()],
        };
        assert!(dll.applies_to_executable(Some(Path::new("C:/Games/target.EXE"))));
        assert!(!dll.applies_to_executable(Some(Path::new("C:/Games/Other.exe"))));
        assert!(!dll.applies_to_executable(None));
    }
}
