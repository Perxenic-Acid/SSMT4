//! 宿主信任侧的插件能力代理。
//!
//! 当前调用方是 SSMT 自身页面/桥，不是 iframe 内的插件代码。后续增加插件消息桥时，
//! 必须把调用方绑定到 Lease/ServiceInstance 身份，不能让插件自行提交 `pluginId`。

use super::data_store::ScopedDataStore;
use super::graphics_tasks::{GraphicsTaskBroker, GraphicsTaskError, GraphicsTaskOwner};
use super::page_runtime::{PageLeaseIdentity, PluginPageRuntime};
use super::registry::PluginRegistry;
use super::settings::PluginSettingsStore;
use super::{PluginMigrationSource, PluginPermission};
use crate::config::path_manager::PathManager;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

pub const CAPABILITY_API_VERSION: u32 = 1;

pub const CAPABILITY_ERROR_CODES: &[&str] = &[
    "backend_failed",
    "bridge_error",
    "capability_denied",
    "capability_not_supported",
    "directory_picker_failed",
    "external_manager_failed",
    "external_manager_invalid",
    "external_manager_missing",
    "external_manager_not_declared",
    "game_context_invalid",
    "game_context_unavailable",
    "graphics_history_error",
    "graphics_operation_failed",
    "graphics_profile_not_declared",
    "graphics_routes_error",
    "graphics_runtime_error",
    "graphics_status_error",
    "graphics_task_conflict",
    "graphics_task_error",
    "graphics_task_failed",
    "graphics_task_limit",
    "invalid_payload",
    "lease_invalid",
    "lease_lookup_failed",
    "lease_required",
    "migration_not_declared",
    "plugin_api_incompatible",
    "plugin_data_error",
    "plugin_disabled",
    "plugin_not_installed",
    "plugin_registry_unavailable",
    "storage_error",
    "storage_unavailable",
    "task_cancelled",
    "task_not_cancellable",
    "task_not_found",
    "task_not_running",
    "task_store_unavailable",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub id: String,
    pub description: String,
    pub required_permissions: Vec<String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCatalog {
    pub api_version: u32,
    pub plugin_api_version: String,
    pub manifest_schema_version: u32,
    pub minimum_manifest_schema_version: u32,
    pub page_bridge_protocol_version: u32,
    pub page_kinds: Vec<String>,
    pub bridge_request_type: String,
    pub bridge_response_type: String,
    pub error_codes: Vec<String>,
    pub capabilities: Vec<CapabilityDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityResponse {
    pub request_id: Option<String>,
    pub capability: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[tauri::command]
pub fn plugin_capability_catalog() -> CapabilityCatalog {
    CapabilityCatalog {
        api_version: CAPABILITY_API_VERSION,
        plugin_api_version: super::PLUGIN_API_VERSION.to_string(),
        manifest_schema_version: super::PLUGIN_MANIFEST_SCHEMA_VERSION,
        minimum_manifest_schema_version: super::MIN_PLUGIN_MANIFEST_SCHEMA_VERSION,
        page_bridge_protocol_version: super::page_runtime::PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION,
        page_kinds: ["declarative", "static", "local-service"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        bridge_request_type: super::page_runtime::PLUGIN_PAGE_BRIDGE_REQUEST_TYPE.to_string(),
        bridge_response_type: super::page_runtime::PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE.to_string(),
        error_codes: CAPABILITY_ERROR_CODES
            .iter()
            .map(|code| (*code).to_string())
            .collect(),
        capabilities: descriptors(),
    }
}

#[tauri::command]
pub async fn plugin_capability_request(
    app: AppHandle,
    tasks: State<'_, GraphicsTaskBroker>,
    plugin_id: String,
    game_name: String,
    capability: String,
    payload: Value,
    request_id: Option<String>,
) -> Result<CapabilityResponse, String> {
    let task_broker = tasks.inner().clone();
    match execute_capability(
        &plugin_id,
        &game_name,
        &capability,
        payload,
        None,
        &app,
        &task_broker,
    )
    .await
    {
        Ok(value) => Ok(success(request_id, capability, value)),
        Err((code, message)) => Ok(failure(request_id, capability, code, message)),
    }
}

/// Plugin-originated requests resolve their identity from an active ServiceLease.
/// The untrusted page never supplies `pluginId` or `gameName` directly.
#[tauri::command]
pub async fn plugin_capability_request_for_lease(
    app: AppHandle,
    tasks: State<'_, GraphicsTaskBroker>,
    lease_id: String,
    capability: String,
    payload: Value,
    request_id: Option<String>,
    runtime: State<'_, PluginPageRuntime>,
) -> Result<CapabilityResponse, String> {
    let identity = match runtime.lease_identity(&lease_id) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            return Ok(failure(
                request_id,
                capability,
                "lease_invalid",
                "plugin page lease is missing, expired or stopped".to_string(),
            ));
        }
        Err(message) => {
            return Ok(failure(
                request_id,
                capability,
                "lease_lookup_failed",
                message,
            ));
        }
    };
    let task_broker = tasks.inner().clone();
    match execute_capability(
        &identity.plugin_id,
        &identity.game_name,
        &capability,
        payload,
        Some(&identity),
        &app,
        &task_broker,
    )
    .await
    {
        Ok(value) => Ok(success(request_id, capability, value)),
        Err((code, message)) => Ok(failure(request_id, capability, code, message)),
    }
}

async fn execute_capability(
    plugin_id: &str,
    game_name: &str,
    capability: &str,
    payload: Value,
    lease_identity: Option<&PageLeaseIdentity>,
    app: &AppHandle,
    tasks: &GraphicsTaskBroker,
) -> Result<Value, (&'static str, String)> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| ("plugin_registry_unavailable", error.to_string()))?;
    let scoped = registry.for_game(game_name);
    let plugin = scoped.find(plugin_id).ok_or_else(|| {
        (
            "plugin_not_installed",
            format!("plugin is not installed: {plugin_id}"),
        )
    })?;
    if !plugin.enabled {
        return Err((
            "plugin_disabled",
            format!("plugin is disabled for {game_name}: {plugin_id}"),
        ));
    }
    if !plugin.manifest.supports_current_plugin_api() {
        return Err((
            "plugin_api_incompatible",
            "plugin API is not compatible with this host".to_string(),
        ));
    }

    match capability {
        "plugin.backend.request" => {
            require_permission(plugin, PluginPermission::ProcessSpawn)?;
            let method = payload_string(&payload, "method")?;
            super::backend::request(
                plugin,
                game_name,
                method,
                payload.get("payload").cloned().unwrap_or(Value::Null),
                false,
            )
            .await
            .map_err(|error| ("backend_failed", error))
        }
        "plugin.settings.get" => {
            let key = payload_key(&payload)?;
            let store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            let value = store
                .get(plugin_id, key)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({ "value": value }))
        }
        "plugin.settings.set" => {
            let key = payload_key(&payload)?;
            let value = payload
                .get("value")
                .cloned()
                .ok_or_else(|| ("invalid_payload", "missing value".to_string()))?;
            let mut store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            store
                .set(plugin_id, key, value)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({}))
        }
        "plugin.settings.remove" => {
            let key = payload_key(&payload)?;
            let mut store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            let removed = store
                .remove(plugin_id, key)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({ "removed": removed }))
        }
        "plugin.storage.get" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let key = storage_key(&payload)?;
            let store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            let value = store
                .get(plugin_id, &key)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({ "value": value }))
        }
        "plugin.storage.set" => {
            require_permission(plugin, PluginPermission::FilesystemWrite)?;
            let key = storage_key(&payload)?;
            let value = payload
                .get("value")
                .cloned()
                .ok_or_else(|| ("invalid_payload", "missing value".to_string()))?;
            let mut store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            store
                .set(plugin_id, &key, value)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({}))
        }
        "plugin.storage.remove" => {
            require_permission(plugin, PluginPermission::FilesystemWrite)?;
            let key = storage_key(&payload)?;
            let mut store = PluginSettingsStore::from_default_location()
                .map_err(|error| ("storage_unavailable", error.to_string()))?;
            let removed = store
                .remove(plugin_id, &key)
                .map_err(|error| ("storage_error", error.to_string()))?;
            Ok(json!({ "removed": removed }))
        }
        "game.context.read" => {
            let config_path =
                PathManager::global_config_games_game_folder(game_name).join("Config.json");
            let raw = std::fs::read_to_string(&config_path).map_err(|error| {
                (
                    "game_context_unavailable",
                    format!("cannot read {}: {error}", config_path.display()),
                )
            })?;
            let config = serde_json::from_str::<Value>(&raw)
                .map_err(|error| ("game_context_invalid", error.to_string()))?;
            Ok(json!({
                "gameName": game_name,
                "gamePreset": config.get("gamePreset").cloned().unwrap_or(Value::Null),
                "installDir": config.get("installDir").cloned().unwrap_or(Value::Null),
                "targetExePath": config.get("targetExePath").cloned().unwrap_or(Value::Null),
            }))
        }
        "plugin.data.list" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let data_id = payload_string(&payload, "dataId")?;
            let path = payload.get("path").and_then(Value::as_str).unwrap_or("");
            let entries = ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .list(path)
                .map_err(data_error)?;
            Ok(json!({ "entries": entries }))
        }
        "plugin.data.read" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let data_id = payload_string(&payload, "dataId")?;
            let path = payload_string(&payload, "path")?;
            let content = ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .read(path)
                .map_err(data_error)?;
            Ok(json!({ "content": content }))
        }
        "plugin.data.runtime.read" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let data_id = payload_string(&payload, "dataId")?;
            let path = payload_string(&payload, "path")?;
            let content = ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .read_runtime(path)
                .map_err(data_error)?;
            Ok(json!({ "content": content }))
        }
        "plugin.data.write" => {
            require_permission(plugin, PluginPermission::FilesystemWrite)?;
            let data_id = payload_string(&payload, "dataId")?;
            let path = payload_string(&payload, "path")?;
            let content = payload_string(&payload, "content")?;
            ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .write_atomic(path, content)
                .map_err(data_error)?;
            Ok(json!({}))
        }
        "plugin.data.remove" => {
            require_permission(plugin, PluginPermission::FilesystemWrite)?;
            let data_id = payload_string(&payload, "dataId")?;
            let path = payload_string(&payload, "path")?;
            let removed = ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .remove(path)
                .map_err(data_error)?;
            Ok(json!({ "removed": removed }))
        }
        "plugin.data.publish" => {
            require_permission(plugin, PluginPermission::FilesystemWrite)?;
            let data_id = payload_string(&payload, "dataId")?;
            ScopedDataStore::for_plugin(plugin_id, game_name, data_id)
                .map_err(data_error)?
                .publish()
                .map_err(data_error)?;
            Ok(json!({ "published": true }))
        }
        "game.process.read" => {
            require_permission(plugin, PluginPermission::ProcessObserve)?;
            let config_path =
                PathManager::global_config_games_game_folder(game_name).join("Config.json");
            let config: Value =
                serde_json::from_slice(&std::fs::read(&config_path).map_err(|error| {
                    (
                        "game_context_unavailable",
                        format!("cannot read {}: {error}", config_path.display()),
                    )
                })?)
                .map_err(|error| ("game_context_invalid", error.to_string()))?;
            let target = config
                .get("targetExePath")
                .and_then(Value::as_str)
                .unwrap_or("");
            let process_name = std::path::Path::new(target)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let mut system = sysinfo::System::new();
            system.refresh_processes_specifics(
                sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::Always),
            );
            let running = !process_name.is_empty()
                && system.processes().values().any(|process| {
                    process.name().eq_ignore_ascii_case(process_name)
                        || process
                            .exe()
                            .is_some_and(|path| path.to_string_lossy().eq_ignore_ascii_case(target))
                });
            Ok(json!({ "running": running, "processName": process_name }))
        }
        "plugin.migration.read" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let migration_id = payload_string(&payload, "migrationId")?;
            let migration = plugin
                .manifest
                .contributions
                .migrations
                .iter()
                .find(|migration| migration.id == migration_id)
                .ok_or_else(|| {
                    (
                        "migration_not_declared",
                        format!("plugin migration is not declared: {migration_id}"),
                    )
                })?;
            match migration.source {
                PluginMigrationSource::GameConfig => {
                    let config_path =
                        PathManager::global_config_games_game_folder(game_name).join("Config.json");
                    let config: Value =
                        serde_json::from_slice(&std::fs::read(&config_path).map_err(|error| {
                            (
                                "game_context_unavailable",
                                format!("cannot read {}: {error}", config_path.display()),
                            )
                        })?)
                        .map_err(|error| ("game_context_invalid", error.to_string()))?;
                    let mut values = serde_json::Map::new();
                    for key in &migration.keys {
                        if let Some(value) = config.get(key) {
                            values.insert(key.clone(), value.clone());
                        }
                    }
                    Ok(json!({ "values": values }))
                }
            }
        }
        "graphics.status.read" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let executable = configured_game_executable(game_name)?;
            let game_state = super::commands::inspect_dlss5_game_state(executable.clone())
                .await
                .map_err(|error| ("graphics_status_error", error))?;
            let last_run =
                super::commands::inspect_dlss5_last_run(executable, Some(game_name.to_string()))
                    .await
                    .map_err(|error| ("graphics_status_error", error))?;
            Ok(json!({ "gameState": game_state, "lastRun": last_run }))
        }
        "graphics.routes.read" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let api_override = payload
                .get("apiOverride")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("auto");
            let scan = super::commands::inspect_dlss5_route_options(
                game_name.to_string(),
                configured_game_executable(game_name)?,
                api_override.to_string(),
            )
            .await
            .map_err(|error| ("graphics_routes_error", error))?;
            serde_json::to_value(scan).map_err(|error| {
                (
                    "graphics_routes_error",
                    format!("cannot serialize graphics routes: {error}"),
                )
            })
        }
        "graphics.runtime.status" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let game = game_name.to_string();
            let (gimi, dlss5) = tokio::try_join!(
                super::gimi_hot_runtime::gimi_hot_runtime_status(game.clone()),
                super::dlss5_hot_runtime::dlss5_hot_runtime_status(game),
            )
            .map_err(|error| ("graphics_runtime_error", error))?;
            Ok(json!({ "gimi": gimi, "dlss5": dlss5 }))
        }
        "graphics.runtime.history" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            let history =
                super::dlss5_hot_runtime::catalog::dlss5_hot_runtime_history(game_name.to_string())
                    .await
                    .map_err(|error| ("graphics_history_error", error))?;
            serde_json::to_value(history).map_err(|error| {
                (
                    "graphics_history_error",
                    format!("cannot serialize graphics history: {error}"),
                )
            })
        }
        "graphics.task.start" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            let identity = require_lease_identity(lease_identity)?;
            require_task_permissions(plugin, payload_string(&payload, "operation")?)?;
            start_task(tasks, identity, payload)
        }
        "graphics.runtime.download"
        | "graphics.runtime.import"
        | "graphics.runtime.select"
        | "graphics.runtime.remove"
        | "graphics.route.install"
        | "graphics.route.restore" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            let identity = require_lease_identity(lease_identity)?;
            let operation = capability.trim_start_matches("graphics.");
            require_task_permissions(plugin, operation)?;
            let mut request = payload;
            request["operation"] = Value::String(operation.to_string());
            start_task(tasks, identity, request)
        }
        "graphics.task.status" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            let identity = require_lease_identity(lease_identity)?;
            let owner = GraphicsTaskOwner::from(identity);
            let task = tasks
                .status(&owner, payload_string(&payload, "taskId")?)
                .map_err(task_error)?;
            serde_json::to_value(task).map_err(|error| {
                (
                    "graphics_task_error",
                    format!("cannot serialize graphics task: {error}"),
                )
            })
        }
        "graphics.task.cancel" => {
            require_graphics_profile(plugin, "dlss5-hot")?;
            let identity = require_lease_identity(lease_identity)?;
            let owner = GraphicsTaskOwner::from(identity);
            let task = tasks
                .cancel(&owner, payload_string(&payload, "taskId")?)
                .map_err(task_error)?;
            serde_json::to_value(task).map_err(|error| {
                (
                    "graphics_task_error",
                    format!("cannot serialize graphics task: {error}"),
                )
            })
        }
        "graphics.directory.select" => {
            require_permission(plugin, PluginPermission::FilesystemRead)?;
            if payload_string(&payload, "purpose")? != "runtime.import" {
                return Err((
                    "capability_denied",
                    "directory picker purpose is not allowed".to_string(),
                ));
            }
            let app = app.clone();
            let selected = tauri::async_runtime::spawn_blocking(move || {
                app.dialog().file().blocking_pick_folder()
            })
            .await
            .map_err(|error| {
                (
                    "directory_picker_failed",
                    format!("directory picker task failed: {error}"),
                )
            })?;
            let path = selected
                .map(|path| {
                    path.into_path()
                        .map(|path| path.to_string_lossy().into_owned())
                        .map_err(|error| {
                            (
                                "directory_picker_failed",
                                format!("cannot read selected directory: {error}"),
                            )
                        })
                })
                .transpose()?;
            Ok(json!({ "path": path }))
        }
        "graphics.external.open" => {
            require_permission(plugin, PluginPermission::ProcessSpawn)?;
            open_external_manager(plugin, payload_string(&payload, "externalManagerId")?)
        }
        _ => Err((
            "capability_not_supported",
            format!("unsupported capability: {capability}"),
        )),
    }
}

fn payload_key(payload: &Value) -> Result<&str, (&'static str, String)> {
    payload
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| ("invalid_payload", "missing or empty key".to_string()))
}

fn data_error(error: String) -> (&'static str, String) {
    ("plugin_data_error", error)
}

fn payload_string<'a>(payload: &'a Value, key: &str) -> Result<&'a str, (&'static str, String)> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ("invalid_payload", format!("missing or empty {key}")))
}

fn storage_key(payload: &Value) -> Result<String, (&'static str, String)> {
    Ok(format!("storage:{}", payload_key(payload)?))
}

fn require_permission(
    plugin: &super::registry::InstalledPlugin,
    permission: PluginPermission,
) -> Result<(), (&'static str, String)> {
    if plugin
        .manifest
        .permissions
        .iter()
        .any(|candidate| *candidate == permission)
    {
        Ok(())
    } else {
        Err((
            "capability_denied",
            format!("plugin is missing permission: {permission:?}"),
        ))
    }
}

fn require_graphics_profile(
    plugin: &super::registry::InstalledPlugin,
    strategy: &str,
) -> Result<(), (&'static str, String)> {
    let declared = plugin
        .manifest
        .contributions
        .graphics_profiles
        .iter()
        .any(|profile| profile.strategy.as_str() == strategy)
        || plugin
            .manifest
            .contributions
            .host_extensions
            .iter()
            .any(|extension| extension.provider == strategy);
    if declared {
        Ok(())
    } else {
        Err((
            "graphics_profile_not_declared",
            format!("plugin does not declare a {strategy} graphics profile"),
        ))
    }
}

fn require_lease_identity(
    identity: Option<&PageLeaseIdentity>,
) -> Result<&PageLeaseIdentity, (&'static str, String)> {
    identity.ok_or((
        "lease_required",
        "graphics task capabilities require an active plugin page lease".to_string(),
    ))
}

fn require_task_permissions(
    plugin: &super::registry::InstalledPlugin,
    operation: &str,
) -> Result<(), (&'static str, String)> {
    let permissions: &[PluginPermission] = match operation {
        "runtime.download" => &[
            PluginPermission::Network,
            PluginPermission::FilesystemRead,
            PluginPermission::FilesystemWrite,
        ],
        "runtime.import" | "runtime.select" | "runtime.remove" => &[
            PluginPermission::FilesystemRead,
            PluginPermission::FilesystemWrite,
        ],
        "route.install" | "route.restore" => &[
            PluginPermission::FilesystemRead,
            PluginPermission::FilesystemWrite,
            PluginPermission::NativeInject,
            PluginPermission::ProcessSpawn,
        ],
        _ => {
            return Err((
                "invalid_payload",
                format!("unknown graphics task operation: {operation}"),
            ))
        }
    };
    for permission in permissions {
        require_permission(plugin, permission.clone())?;
    }
    Ok(())
}

fn start_task(
    tasks: &GraphicsTaskBroker,
    identity: &PageLeaseIdentity,
    payload: Value,
) -> Result<Value, (&'static str, String)> {
    let task = tasks
        .start(GraphicsTaskOwner::from(identity), payload)
        .map_err(task_error)?;
    serde_json::to_value(task).map_err(|error| {
        (
            "graphics_task_error",
            format!("cannot serialize graphics task: {error}"),
        )
    })
}

fn task_error(error: GraphicsTaskError) -> (&'static str, String) {
    (error.code, error.message)
}

fn configured_game_executable(game_name: &str) -> Result<String, (&'static str, String)> {
    let path = PathManager::global_config_games_game_folder(game_name).join("Config.json");
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        (
            "game_context_unavailable",
            format!("cannot read {}: {error}", path.display()),
        )
    })?;
    let config = serde_json::from_str::<Value>(&raw)
        .map_err(|error| ("game_context_invalid", error.to_string()))?;
    config
        .get("targetExePath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            (
                "game_context_unavailable",
                "game configuration is missing targetExePath".to_string(),
            )
        })
}

fn open_external_manager(
    plugin: &super::registry::InstalledPlugin,
    manager_id: &str,
) -> Result<Value, (&'static str, String)> {
    let manager = plugin
        .manifest
        .contributions
        .external_managers
        .iter()
        .find(|manager| manager.id == manager_id)
        .ok_or_else(|| {
            (
                "external_manager_not_declared",
                format!("external manager is not declared: {manager_id}"),
            )
        })?;
    let executable = PathBuf::from(resolve_plugin_template(plugin, &manager.executable)?);
    if !executable.is_file() {
        return Err((
            "external_manager_missing",
            format!(
                "external manager executable does not exist: {}",
                executable.display()
            ),
        ));
    }
    let mut command = Command::new(&executable);
    if let Some(working_directory) = manager.working_directory.as_deref() {
        let working_directory = PathBuf::from(resolve_plugin_template(plugin, working_directory)?);
        if !working_directory.is_dir() {
            return Err((
                "external_manager_missing",
                format!(
                    "external manager working directory does not exist: {}",
                    working_directory.display()
                ),
            ));
        }
        command.current_dir(working_directory);
    }
    command.spawn().map_err(|error| {
        (
            "external_manager_failed",
            format!("cannot open external manager: {error}"),
        )
    })?;
    Ok(json!({}))
}

fn resolve_plugin_template(
    plugin: &super::registry::InstalledPlugin,
    template: &str,
) -> Result<String, (&'static str, String)> {
    let mut resolved = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find("${") {
        resolved.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err((
                "external_manager_invalid",
                format!("unterminated template variable: {template}"),
            ));
        };
        let variable = &after_start[..end];
        let replacement = if variable == "plugin.root" {
            plugin.package_root.to_string_lossy().into_owned()
        } else if let Some(dependency_id) = variable.strip_prefix("external.") {
            plugin
                .external_dependencies
                .get(dependency_id)
                .and_then(|state| state.path.as_deref())
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    (
                        "external_manager_missing",
                        format!("external dependency is not ready: {dependency_id}"),
                    )
                })?
                .to_string_lossy()
                .into_owned()
        } else {
            return Err((
                "external_manager_invalid",
                format!("unsupported template variable: ${{{variable}}}"),
            ));
        };
        resolved.push_str(&replacement);
        remaining = &after_start[end + 1..];
    }
    resolved.push_str(remaining);
    Ok(resolved)
}

fn success(request_id: Option<String>, capability: String, value: Value) -> CapabilityResponse {
    CapabilityResponse {
        request_id,
        capability,
        ok: true,
        value: Some(value),
        error_code: None,
        error_message: None,
    }
}

fn failure(
    request_id: Option<String>,
    capability: String,
    code: &'static str,
    message: String,
) -> CapabilityResponse {
    debug_assert!(
        CAPABILITY_ERROR_CODES.contains(&code),
        "capability error code is not part of Plugin API 1: {code}"
    );
    CapabilityResponse {
        request_id,
        capability,
        ok: false,
        value: None,
        error_code: Some(code.to_string()),
        error_message: Some(message),
    }
}

fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        descriptor(
            "plugin.backend.request",
            "Call a method declared by the plugin-owned backend",
            &["process.spawn"],
            json!({"type":"object","required":["method"],"properties":{"method":{"type":"string"},"payload":{}}}),
            json!({}),
        ),
        descriptor(
            "plugin.settings.get",
            "Read a key from this plugin's settings namespace.",
            &[],
            json!({"type":"object","required":["key"],"properties":{"key":{"type":"string"}}}),
            json!({"type":"object","properties":{"value":{}}}),
        ),
        descriptor(
            "plugin.settings.set",
            "Write a key in this plugin's settings namespace.",
            &[],
            json!({"type":"object","required":["key","value"],"properties":{"key":{"type":"string"},"value":{}}}),
            json!({"type":"object"}),
        ),
        descriptor(
            "plugin.settings.remove",
            "Remove a key from this plugin's settings namespace.",
            &[],
            json!({"type":"object","required":["key"],"properties":{"key":{"type":"string"}}}),
            json!({"type":"object","properties":{"removed":{"type":"boolean"}}}),
        ),
        descriptor(
            "plugin.storage.get",
            "Read a plugin-scoped storage value.",
            &["filesystem.read"],
            json!({"type":"object","required":["key"],"properties":{"key":{"type":"string"}}}),
            json!({"type":"object","properties":{"value":{}}}),
        ),
        descriptor(
            "plugin.storage.set",
            "Write a plugin-scoped storage value.",
            &["filesystem.write"],
            json!({"type":"object","required":["key","value"],"properties":{"key":{"type":"string"},"value":{}}}),
            json!({"type":"object"}),
        ),
        descriptor(
            "plugin.storage.remove",
            "Remove a plugin-scoped storage value.",
            &["filesystem.write"],
            json!({"type":"object","required":["key"],"properties":{"key":{"type":"string"}}}),
            json!({"type":"object","properties":{"removed":{"type":"boolean"}}}),
        ),
        descriptor(
            "game.context.read",
            "Read sanitized context for the current game.",
            &[],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","properties":{"gameName":{"type":"string"},"gamePreset":{},"installDir":{},"targetExePath":{}}}),
        ),
        descriptor(
            "plugin.data.list",
            "List files in a declared plugin-scoped data directory.",
            &["filesystem.read"],
            json!({"type":"object","required":["dataId"],"properties":{"dataId":{"type":"string"},"path":{"type":"string"}}}),
            json!({"type":"object","properties":{"entries":{"type":"array"}}}),
        ),
        descriptor(
            "plugin.data.read",
            "Read a UTF-8 file from a declared plugin-scoped data directory.",
            &["filesystem.read"],
            json!({"type":"object","required":["dataId","path"],"properties":{"dataId":{"type":"string"},"path":{"type":"string"}}}),
            json!({"type":"object","properties":{"content":{"type":"string"}}}),
        ),
        descriptor(
            "plugin.data.runtime.read",
            "Read a UTF-8 file from the current package runtime mirror.",
            &["filesystem.read"],
            json!({"type":"object","required":["dataId","path"],"properties":{"dataId":{"type":"string"},"path":{"type":"string"}}}),
            json!({"type":"object","properties":{"content":{"type":"string"}}}),
        ),
        descriptor(
            "plugin.data.write",
            "Atomically write a UTF-8 file in a declared plugin-scoped data directory.",
            &["filesystem.write"],
            json!({"type":"object","required":["dataId","path","content"],"properties":{"dataId":{"type":"string"},"path":{"type":"string"},"content":{"type":"string"}}}),
            json!({"type":"object"}),
        ),
        descriptor(
            "plugin.data.remove",
            "Remove a file from a declared plugin-scoped data directory.",
            &["filesystem.write"],
            json!({"type":"object","required":["dataId","path"],"properties":{"dataId":{"type":"string"},"path":{"type":"string"}}}),
            json!({"type":"object","properties":{"removed":{"type":"boolean"}}}),
        ),
        descriptor(
            "plugin.data.publish",
            "Publish a declared persistent data directory to its package runtime mirror.",
            &["filesystem.write"],
            json!({"type":"object","required":["dataId"],"properties":{"dataId":{"type":"string"}}}),
            json!({"type":"object","properties":{"published":{"type":"boolean"}}}),
        ),
        descriptor(
            "game.process.read",
            "Read whether the configured game executable is currently running.",
            &["process.observe"],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","properties":{"running":{"type":"boolean"},"processName":{"type":"string"}}}),
        ),
        descriptor(
            "plugin.migration.read",
            "Read only the legacy game-config keys explicitly declared by the plugin.",
            &["filesystem.read"],
            json!({"type":"object","required":["migrationId"],"properties":{"migrationId":{"type":"string"}}}),
            json!({"type":"object","properties":{"values":{"type":"object"}}}),
        ),
        descriptor(
            "graphics.status.read",
            "Read the managed graphics state and the last verified runtime verdict.",
            &["filesystem.read"],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","properties":{"gameState":{"type":"object"},"lastRun":{"type":"object"}}}),
        ),
        descriptor(
            "graphics.routes.read",
            "Read graphics routes supported by the current game and executable.",
            &["filesystem.read"],
            json!({"type":"object","properties":{"apiOverride":{"type":"string"}}}),
            json!({"type":"object"}),
        ),
        descriptor(
            "graphics.runtime.status",
            "Read verified graphics runtime installations for the current game.",
            &["filesystem.read"],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","properties":{"gimi":{},"dlss5":{}}}),
        ),
        descriptor(
            "graphics.runtime.history",
            "Read graphics runtime installations owned by the current game.",
            &["filesystem.read"],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"array"}),
        ),
        descriptor(
            "graphics.task.start",
            "Start a host-owned graphics operation and return a task handle.",
            &["filesystem.read", "filesystem.write"],
            json!({"type":"object","required":["operation"],"properties":{"operation":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.task.status",
            "Read progress and the final result of a graphics operation.",
            &[],
            json!({"type":"object","required":["taskId"],"properties":{"taskId":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"},"phase":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.task.cancel",
            "Request cancellation of a cancellable graphics operation.",
            &[],
            json!({"type":"object","required":["taskId"],"properties":{"taskId":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.runtime.download",
            "Download and verify a graphics runtime through the task broker.",
            &["network", "filesystem.read", "filesystem.write"],
            json!({"type":"object","required":["runtime","profile"],"properties":{"runtime":{"type":"string"},"route":{"type":"string"},"profile":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.runtime.import",
            "Import a user-selected graphics runtime through the task broker.",
            &["filesystem.read", "filesystem.write"],
            json!({"type":"object","required":["sourceDirectory","profile"],"properties":{"sourceDirectory":{"type":"string"},"profile":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.runtime.select",
            "Select a verified graphics runtime from the current game's history.",
            &["filesystem.read", "filesystem.write"],
            json!({"type":"object","required":["root"],"properties":{"root":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.runtime.remove",
            "Remove an owned graphics runtime from the current game's history.",
            &["filesystem.read", "filesystem.write"],
            json!({"type":"object","required":["root"],"properties":{"root":{"type":"string"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.route.install",
            "Install a declared managed graphics route through the task broker.",
            &[
                "filesystem.read",
                "filesystem.write",
                "native.inject",
                "process.spawn",
            ],
            json!({"type":"object","required":["route"],"properties":{"route":{"type":"string"},"apiOverride":{"type":"string"},"antiCheatAcknowledged":{"type":"boolean"}}}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.route.restore",
            "Restore an existing managed graphics route through the task broker.",
            &[
                "filesystem.read",
                "filesystem.write",
                "native.inject",
                "process.spawn",
            ],
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","properties":{"taskId":{"type":"string"},"status":{"type":"string"}}}),
        ),
        descriptor(
            "graphics.directory.select",
            "Open the host-controlled directory picker for an allowed graphics operation.",
            &["filesystem.read"],
            json!({"type":"object","required":["purpose"],"properties":{"purpose":{"type":"string"}}}),
            json!({"type":"object","properties":{"path":{"type":["string","null"]}}}),
        ),
        descriptor(
            "graphics.external.open",
            "Open an external manager explicitly declared by the plugin.",
            &["process.spawn"],
            json!({"type":"object","required":["externalManagerId"],"properties":{"externalManagerId":{"type":"string"}}}),
            json!({"type":"object"}),
        ),
    ]
}

fn descriptor(
    id: &str,
    description: &str,
    permissions: &[&str],
    input_schema: Value,
    output_schema: Value,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: id.to_string(),
        description: description.to_string(),
        required_permissions: permissions.iter().map(|value| value.to_string()).collect(),
        input_schema,
        output_schema,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::InstalledPlugin;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_catalog_is_versioned_and_discriminating() {
        let catalog = plugin_capability_catalog();
        assert_eq!(catalog.api_version, 1);
        assert_eq!(catalog.plugin_api_version, super::super::PLUGIN_API_VERSION);
        assert_eq!(catalog.page_bridge_protocol_version, 1);
        assert_eq!(catalog.manifest_schema_version, 2);
        assert_eq!(catalog.minimum_manifest_schema_version, 1);
        assert!(catalog
            .capabilities
            .iter()
            .any(|capability| capability.id == "plugin.settings.get"));
        assert!(catalog
            .capabilities
            .iter()
            .any(|capability| capability.id == "game.context.read"));
        assert!(catalog
            .capabilities
            .iter()
            .any(|capability| capability.id == "graphics.task.start"));
        assert!(catalog
            .capabilities
            .iter()
            .any(|capability| capability.id == "graphics.runtime.history"));
    }

    #[test]
    fn plugin_api_v1_contract_snapshot_is_exact() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/plugin-api-v1-contract.json");
        let contract: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let catalog = plugin_capability_catalog();
        assert_eq!(
            contract["pluginApiVersion"],
            super::super::PLUGIN_API_VERSION
        );
        assert_eq!(contract["capabilityApiVersion"], CAPABILITY_API_VERSION);
        assert_eq!(
            contract["manifestSchemaVersion"],
            super::super::PLUGIN_MANIFEST_SCHEMA_VERSION
        );
        assert_eq!(
            contract["minimumManifestSchemaVersion"],
            super::super::MIN_PLUGIN_MANIFEST_SCHEMA_VERSION
        );
        assert_eq!(
            contract["pageBridgeProtocolVersion"],
            super::super::page_runtime::PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION
        );
        assert_eq!(
            contract["bridgeMessageTypes"]["request"],
            super::super::page_runtime::PLUGIN_PAGE_BRIDGE_REQUEST_TYPE
        );
        assert_eq!(
            contract["bridgeMessageTypes"]["response"],
            super::super::page_runtime::PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE
        );
        assert_eq!(
            contract["pageKinds"],
            serde_json::to_value(&catalog.page_kinds).unwrap()
        );
        assert_eq!(
            contract["errorCodes"],
            serde_json::to_value(&catalog.error_codes).unwrap()
        );

        let mut expected = contract["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|capability| {
                (
                    capability["id"].as_str().unwrap().to_string(),
                    capability["permissions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|permission| permission.as_str().unwrap().to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut actual = catalog
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.id.clone(),
                    capability.required_permissions.clone(),
                )
            })
            .collect::<Vec<_>>();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);

        let rules = contract["compatibilityRules"].as_object().unwrap();
        assert_eq!(rules["unknownManifestFields"], "reject");
        assert_eq!(rules["capabilityRemovalWithinMajor"], "forbidden");
        assert_eq!(rules["capabilityRenameWithinMajor"], "forbidden");
        assert_eq!(rules["capabilityMeaningChangeWithinMajor"], "forbidden");
        assert_eq!(rules["additiveCapabilitiesAllowed"], true);
        assert_eq!(rules["bridgeBreakingChangeRequiresProtocolBump"], true);
        assert_eq!(rules["pageRemovalRequiresMajor"], true);
        assert_eq!(rules["pluginUpdatesMayChangeBundledUi"], true);
        assert_eq!(rules["hostUpdatesMustNotRequirePluginRebuild"], true);
        assert_eq!(rules["futurePluginApi"], "install-visible-incompatible");
    }

    #[test]
    fn rejects_missing_payload_keys_with_stable_error_code() {
        let error = payload_key(&json!({})).unwrap_err();
        assert_eq!(error.0, "invalid_payload");
        assert_eq!(
            storage_key(&json!({"key":"draft"})).unwrap(),
            "storage:draft"
        );
    }

    #[test]
    fn plugin_api_requirement_is_checked_separately_from_schema() {
        let manifest = super::super::PluginManifest::from_json_str(
            r#"{
              "schemaVersion": 2,
              "id": "example.sdk",
              "name": "SDK Fixture",
              "version": "1.0.0",
              "author": "Example",
              "compatibility": {
                "ssmt": ">=4.2.2",
                "pluginApi": ">=1.0.0 <2.0.0",
                "platforms": ["windows-x64"]
              },
              "contributions": {},
              "permissions": []
            }"#,
        )
        .unwrap();
        let requirement = manifest.compatibility.plugin_api.as_deref().unwrap();
        let parsed = super::super::parse_plugin_api_requirement(requirement).unwrap();
        assert!(
            parsed.matches(&semver::Version::parse(super::super::PLUGIN_API_VERSION).unwrap()),
            "parsed requirement {parsed} from {requirement}"
        );
        assert!(manifest
            .compatibility
            .supports_plugin_api(super::super::PLUGIN_API_VERSION));
        assert!(manifest.supports_current_plugin_api());
    }

    #[test]
    fn permission_gate_denies_storage_without_filesystem_permission() {
        let manifest = super::super::PluginManifest::from_json_str(
            r#"{
              "schemaVersion": 2,
              "id": "example.sdk",
              "name": "SDK Fixture",
              "version": "1.0.0",
              "author": "Example",
              "compatibility": {
                "ssmt": ">=4.2.2",
                "pluginApi": ">=1.0.0 <2.0.0",
                "platforms": ["windows-x64"]
              },
              "contributions": {},
              "permissions": []
            }"#,
        )
        .unwrap();
        let plugin = InstalledPlugin {
            manifest,
            package_root: PathBuf::from("fixture"),
            enabled: true,
            external_dependencies: BTreeMap::new(),
            official: false,
        };
        let error = require_permission(&plugin, PluginPermission::FilesystemWrite).unwrap_err();
        assert_eq!(error.0, "capability_denied");
    }

    #[test]
    fn graphics_task_permissions_are_checked_by_operation() {
        let manifest = super::super::PluginManifest::from_json_str(
            r#"{
              "schemaVersion": 2,
              "id": "example.graphics",
              "name": "Graphics Fixture",
              "version": "1.0.0",
              "author": "Example",
              "compatibility": {
                "ssmt": ">=4.2.2",
                "pluginApi": ">=1.0.0 <2.0.0",
                "platforms": ["windows-x64"],
                "games": ["GIMI"]
              },
              "contributions": {
                "graphicsProfiles": [{
                  "id": "graphics",
                  "strategy": "dlss5-hot",
                  "resources": ["graphics.dlss5.route"]
                }]
              },
              "permissions": ["filesystem.read", "filesystem.write", "native.inject"]
            }"#,
        )
        .unwrap();
        let plugin = InstalledPlugin {
            manifest,
            package_root: PathBuf::from("fixture"),
            enabled: true,
            external_dependencies: BTreeMap::new(),
            official: false,
        };
        require_graphics_profile(&plugin, "dlss5-hot").unwrap();
        require_task_permissions(&plugin, "runtime.import").unwrap();
        assert_eq!(
            require_task_permissions(&plugin, "runtime.download")
                .unwrap_err()
                .0,
            "capability_denied"
        );
        assert_eq!(
            require_task_permissions(&plugin, "route.restore")
                .unwrap_err()
                .0,
            "capability_denied"
        );
        assert_eq!(
            require_graphics_profile(&plugin, "reshade-hot")
                .unwrap_err()
                .0,
            "graphics_profile_not_declared"
        );
    }
}
