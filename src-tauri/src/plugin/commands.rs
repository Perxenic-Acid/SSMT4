use super::logging::PluginLogWriter;
use super::marketplace::MarketplaceEntry;
use super::official_release;
use super::package_installer::{inspect_ssmtpkg, install_verified_ssmtpkg, PackageInspection};
use super::registry::{
    game_scope_preset, ExternalDependencyState, PluginRegistry, MANIFEST_FILE_NAME,
};
use super::settings::PluginSettingsStore;
use super::PluginManifest;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, System};
use tauri::Manager;

fn installed_plugin_registry_at(plugins_root: PathBuf) -> Result<Option<PluginRegistry>, String> {
    // The plugin page may have created a state file without installing a package.
    // In that case ordinary launch must not parse plugin state or create files.
    let Ok(ids) = fs::read_dir(&plugins_root) else {
        return Ok(None);
    };
    for id in ids.flatten() {
        let Ok(versions) = fs::read_dir(id.path()) else {
            continue;
        };
        if versions
            .flatten()
            .any(|version| version.path().join(MANIFEST_FILE_NAME).is_file())
        {
            return PluginRegistry::new(plugins_root)
                .map(Some)
                .map_err(|error| error.to_string());
        }
    }
    Ok(None)
}

pub(crate) fn installed_plugin_registry() -> Result<Option<PluginRegistry>, String> {
    installed_plugin_registry_at(crate::config::path_manager::PathManager::ssmt_plugins_folder())
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleStatus {
    Installed,
    Enabled,
    Disabled,
    UpdateAvailable,
    Broken,
    Incompatible,
    ExternalDependencyMissing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginSnapshot {
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub enabled_games: Vec<String>,
    pub lifecycle_status: PluginLifecycleStatus,
    pub external_dependencies: BTreeMap<String, ExternalDependencyState>,
    pub official: bool,
    pub app_scoped: bool,
    pub features: Vec<String>,
}

pub(crate) const NATIVE_INJECTION_RISK_ACK_KEY: &str = "native_injection_risk_ack_v1";

pub(crate) fn has_native_injection_risk_ack(plugin: &super::registry::InstalledPlugin) -> bool {
    if !plugin
        .manifest
        .permissions
        .contains(&super::PluginPermission::NativeInject)
    {
        return true;
    }
    let Ok(store) = PluginSettingsStore::from_default_location() else {
        return false;
    };
    let Ok(Some(value)) = store.get(&plugin.manifest.id, NATIVE_INJECTION_RISK_ACK_KEY) else {
        return false;
    };
    value.get("accepted").and_then(serde_json::Value::as_bool) == Some(true)
        && value
            .get("pluginVersion")
            .and_then(serde_json::Value::as_str)
            == Some(plugin.manifest.version.as_str())
}

fn snapshot_for_game(registry: &PluginRegistry, game_name: &str) -> Vec<InstalledPluginSnapshot> {
    let preset = game_scope_preset(game_name);
    let scoped = registry.for_game_preset(game_name, &preset);
    let mut plugins = snapshot(&scoped);
    for plugin in &mut plugins {
        if !plugin.manifest.supports_current_plugin_api()
            || (!plugin.app_scoped && !plugin.manifest.compatibility.supports_game(&preset))
        {
            plugin.enabled = false;
            plugin.lifecycle_status = PluginLifecycleStatus::Incompatible;
        }
        let installed_plugin = scoped
            .find(&plugin.manifest.id)
            .is_some_and(|installed| has_native_injection_risk_ack(installed));
        if !installed_plugin
            && plugin
                .manifest
                .permissions
                .contains(&super::PluginPermission::NativeInject)
        {
            plugin.enabled = false;
            plugin.lifecycle_status = PluginLifecycleStatus::Disabled;
        }
    }
    plugins
}

fn snapshot(registry: &PluginRegistry) -> Vec<InstalledPluginSnapshot> {
    registry
        .installed()
        .iter()
        .map(|plugin| InstalledPluginSnapshot {
            manifest: {
                let mut manifest = plugin.manifest.clone();
                manifest.external_dependencies = plugin.manifest.display_dependencies();
                manifest
            },
            enabled: plugin.enabled,
            enabled_games: registry.enabled_games_for_plugin(plugin.id()),
            lifecycle_status: lifecycle_status(plugin),
            external_dependencies: plugin.external_dependencies.clone(),
            official: plugin.official,
            app_scoped: plugin.manifest.is_app_scoped(),
            features: Vec::new(),
        })
        .collect()
}

fn lifecycle_status(plugin: &super::registry::InstalledPlugin) -> PluginLifecycleStatus {
    if !plugin.manifest.supports_current_plugin_api() {
        return PluginLifecycleStatus::Incompatible;
    }
    let injected = &plugin.manifest.contributions.injected_dlls;
    if !injected.is_empty()
        && (!plugin
            .manifest
            .permissions
            .iter()
            .any(|permission| *permission == super::PluginPermission::NativeInject)
            || injected
                .iter()
                .any(|contribution| contribution.verify_file(&plugin.package_root).is_err()))
    {
        return PluginLifecycleStatus::Broken;
    }
    if plugin
        .external_dependencies
        .iter()
        .any(|(dependency_id, dependency)| {
            dependency.status != super::registry::ExternalDependencyStatus::Ready
                && plugin.manifest.requires_dependency(dependency_id, "launch")
        })
    {
        return PluginLifecycleStatus::ExternalDependencyMissing;
    }
    if plugin.enabled {
        PluginLifecycleStatus::Enabled
    } else {
        PluginLifecycleStatus::Disabled
    }
}

#[tauri::command]
pub fn plugin_registry_snapshot() -> Result<Vec<InstalledPluginSnapshot>, String> {
    installed_plugin_registry().map(|registry| registry.as_ref().map(snapshot).unwrap_or_default())
}

#[tauri::command]
pub fn plugin_registry_snapshot_for_game(
    game_name: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    Ok(snapshot_for_game(&registry, &game_name))
}

#[tauri::command]
pub fn set_plugin_enabled_for_game(
    app: tauri::AppHandle,
    game_name: String,
    id: String,
    enabled: bool,
    _game_executable: Option<String>,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if !enabled {
        app.state::<super::page_runtime::PluginPageRuntime>()
            .stop_plugin(&id)?;
    }
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if enabled {
        if let Some(plugin) = registry.find(&id) {
            if !plugin.manifest.supports_current_plugin_api() {
                return Err(format!("plugin {id} requires an incompatible Plugin API"));
            }
            if plugin
                .manifest
                .permissions
                .contains(&super::PluginPermission::NativeInject)
                && !has_native_injection_risk_ack(plugin)
            {
                return Err("启用原生注入插件前必须阅读并同意风险与免责声明".into());
            }
        }
    }
    if !enabled && registry.game_enabled(&game_name, &id) {
        if let Some(plugin) = registry.find(&id) {
            if plugin
                .manifest
                .contributions
                .backend
                .as_ref()
                .is_some_and(|backend| backend.events.iter().any(|event| event == "disabled"))
            {
                tauri::async_runtime::block_on(super::backend::request(
                    plugin,
                    &game_name,
                    "disabled",
                    serde_json::Value::Null,
                    true,
                ))?;
            }
        }
    }
    if registry
        .find(&id)
        .is_some_and(|plugin| plugin.manifest.is_app_scoped())
    {
        registry.set_enabled(&id, enabled)
    } else {
        registry.set_game_enabled(&game_name, &id, enabled)
    }
    .map_err(|error| error.to_string())?;
    Ok(snapshot_for_game(&registry, &game_name))
}

#[tauri::command]
pub fn set_plugin_external_dependency_path(
    plugin_id: String,
    dependency_id: String,
    path: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    registry
        .set_external_dependency_path(plugin_id.trim(), dependency_id.trim(), path)
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub async fn prepare_plugin_host_config(
    game_name: String,
    runtime_directory: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        prepare_plugin_host_config_blocking(game_name, runtime_directory)
    })
    .await
    .map_err(|error| format!("后台启动任务失败：{error}"))?
}

fn prepare_plugin_host_config_blocking(
    game_name: String,
    runtime_directory: String,
) -> Result<Option<String>, String> {
    let Some(registry) = installed_plugin_registry()?.map(|registry| registry.for_game(&game_name))
    else {
        return Ok(None);
    };
    let mut config = registry.generate_plugin_host_config();
    let allowed: HashSet<_> = registry
        .selected_plugins()
        .filter(|plugin| {
            plugin.enabled
                && has_native_injection_risk_ack(plugin)
                && super::backend::launch_allowed(&game_name, plugin)
        })
        .flat_map(|plugin| {
            plugin
                .manifest
                .contributions
                .runtime_plugins
                .iter()
                .map(move |entry| {
                    format!(
                        "{}/{}/{}",
                        plugin.id(),
                        plugin.version(),
                        entry.path.replace('\\', "/")
                    )
                })
        })
        .collect();
    config
        .plugins
        .retain(|path| allowed.contains(&path.replace('\\', "/")));

    if config.plugins.is_empty() {
        return Ok(None);
    }

    let runtime_directory = PathBuf::from(runtime_directory);
    if !runtime_directory.is_dir() {
        return Err(format!(
            "3DMigoto runtime directory does not exist: {}",
            runtime_directory.display()
        ));
    }

    let config_path = runtime_directory.join("SSMT-PluginHost.json");
    let content = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("failed to serialize PluginHost config: {error}"))?;
    fs::write(&config_path, content).map_err(|error| {
        format!(
            "failed to write PluginHost config {}: {error}",
            config_path.display()
        )
    })?;

    Ok(Some(config_path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn install_plugin_package(
    app: tauri::AppHandle,
    archive_path: String,
    reviewed_sha256: String,
    disclaimer_accepted: bool,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if !disclaimer_accepted {
        return Err("third-party plugin disclaimer must be accepted".into());
    }
    let inspection =
        inspect_ssmtpkg(&PathBuf::from(&archive_path)).map_err(|error| error.to_string())?;
    if inspection.manifest.id.starts_with("ssmt.") {
        return Err("ssmt.* is reserved for packages from the Native Release".into());
    }
    if inspection.sha256 != reviewed_sha256.to_ascii_lowercase() {
        return Err("package changed since safety review".into());
    }
    app.state::<super::page_runtime::PluginPageRuntime>()
        .stop_plugin(&inspection.manifest.id)?;
    if !inspection.manifest.compatibility.supports(
        &app.package_info().version.to_string(),
        super::SUPPORTED_PLATFORM,
        None,
    ) {
        return Err("plugin is incompatible with this SSMT version or platform".into());
    }
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_verified_ssmtpkg(
        &mut registry,
        &PathBuf::from(archive_path),
        &reviewed_sha256,
    )
    .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn inspect_plugin_package(archive_path: String) -> Result<PackageInspection, String> {
    let inspection =
        inspect_ssmtpkg(&PathBuf::from(archive_path)).map_err(|error| error.to_string())?;
    if inspection.manifest.id.starts_with("ssmt.") {
        return Err("ssmt.* is reserved for packages from the Native Release".into());
    }
    Ok(inspection)
}

#[tauri::command]
pub async fn local_plugin_packages(
) -> Result<Vec<super::local_packages::LocalPluginPackage>, String> {
    let Some(directory) = super::local_packages::development_directory(cfg!(debug_assertions))
    else {
        return Ok(Vec::new());
    };
    // 包检查会解压并计算压缩包哈希，不能占用 WebView 的交互线程。
    tauri::async_runtime::spawn_blocking(move || super::local_packages::discover(&directory))
        .await
        .map_err(|error| format!("读取开发插件目录失败：{error}"))?
}

#[tauri::command]
pub fn install_local_plugin(
    app: tauri::AppHandle,
    id: String,
    reviewed_sha256: String,
    disclaimer_accepted: bool,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if !disclaimer_accepted {
        return Err("local plugin disclaimer must be accepted".into());
    }
    app.state::<super::page_runtime::PluginPageRuntime>()
        .stop_plugin(&id)?;
    let directory = super::local_packages::development_directory(cfg!(debug_assertions))
        .ok_or("本地构建包仅限开发模式；请从官方目录或第三方包入口安装插件")?;
    // 本地构建保留 ID 以复用已有贡献，但安装来源仍未验证；不能借此安装任意路径的 ssmt.* 包。
    let package = super::local_packages::reviewed(&directory, &id, &reviewed_sha256)?;
    if !package.inspection.manifest.compatibility.supports(
        &app.package_info().version.to_string(),
        super::SUPPORTED_PLATFORM,
        None,
    ) {
        return Err("plugin is incompatible with this SSMT version or platform".into());
    }
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    install_verified_ssmtpkg(&mut registry, &package.archive_path, &reviewed_sha256)
        .map_err(|error| error.to_string())?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub async fn official_plugin_catalog() -> Result<Vec<MarketplaceEntry>, String> {
    official_release::official_catalog().await
}

#[tauri::command]
pub async fn install_official_plugin(
    app: tauri::AppHandle,
    id: String,
    version: String,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    app.state::<super::page_runtime::PluginPageRuntime>()
        .stop_plugin(&id)?;
    let mut registry =
        PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    official_release::install_official(
        &mut registry,
        &id,
        &version,
        &app.package_info().version.to_string(),
    )
    .await?;
    Ok(snapshot(&registry))
}

#[tauri::command]
pub fn uninstall_plugin(
    app: tauri::AppHandle,
    plugin_id: String,
    game_name: Option<String>,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    if any_configured_game_process_running() {
        return Err("请先退出所有游戏，再卸载插件，避免残留已加载的运行时".into());
    }
    app.state::<super::page_runtime::PluginPageRuntime>()
        .stop_plugin(&plugin_id)?;
    disable_plugin_everywhere_for_uninstall(&app, &plugin_id, game_name.as_deref())?;
    let plugins_root = crate::config::path_manager::PathManager::ssmt_plugins_folder();
    let global_root = crate::config::path_manager::PathManager::ssmt_global_config_folder();
    uninstall_plugin_from_roots(
        &plugin_id,
        game_name.as_deref(),
        &plugins_root,
        &global_root,
    )
}

fn disable_plugin_everywhere_for_uninstall(
    app: &tauri::AppHandle,
    plugin_id: &str,
    current_game_name: Option<&str>,
) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let Some(plugin) = registry.find(plugin_id) else {
        return Ok(());
    };
    if plugin.manifest.is_app_scoped() {
        if registry.plugin_enabled_anywhere(plugin_id) {
            set_plugin_enabled_for_game(
                app.clone(),
                current_game_name
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Default")
                    .to_string(),
                plugin_id.to_string(),
                false,
                None,
            )?;
        }
        return Ok(());
    }

    let mut enabled_games = registry.enabled_games_for_plugin(plugin_id);
    enabled_games.sort_by_key(|name| name.to_ascii_lowercase());
    enabled_games.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    for game_name in enabled_games {
        set_plugin_enabled_for_game(app.clone(), game_name, plugin_id.to_string(), false, None)?;
    }
    let refreshed = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if refreshed.plugin_enabled_anywhere(plugin_id) {
        let mut registry = refreshed;
        registry
            .set_enabled(plugin_id, false)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn any_configured_game_process_running() -> bool {
    let games_root = crate::config::path_manager::PathManager::global_config_games_folder();
    let Ok(entries) = fs::read_dir(games_root) else {
        return false;
    };
    let mut names = HashSet::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let config_path = entry.path().join("Config.json");
        let Ok(raw) = fs::read_to_string(config_path) else {
            continue;
        };
        let Ok(config) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(path) = config
            .get("targetExePath")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        names.insert(name.to_ascii_lowercase());
    }
    if names.is_empty() {
        return false;
    }
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessRefreshKind::new());
    system
        .processes()
        .values()
        .any(|process| names.contains(&process.name().to_ascii_lowercase()))
}

fn remove_owned_path(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取卸载目标失败 {}：{error}", path.display())),
    };
    if directory && metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| format!("删除卸载目标失败 {}：{error}", path.display()))
}

fn cached_package_paths_for_plugin(
    cache_root: &Path,
    plugin_id: &str,
) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(cache_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取插件缓存失败：{error}")),
    };
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("asset"))
        {
            continue;
        }
        let owned = inspect_ssmtpkg(&path)
            .is_ok_and(|inspection| inspection.manifest.id == plugin_id)
            || cached_manifest_plugin_id(&path).as_deref() == Some(plugin_id);
        if owned {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn cached_manifest_plugin_id(path: &Path) -> Option<String> {
    const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let entry = archive.by_name(MANIFEST_FILE_NAME).ok()?;
    if entry.size() > MAX_MANIFEST_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

fn uninstall_plugin_from_roots(
    plugin_id: &str,
    game_name: Option<&str>,
    plugins_root: &Path,
    global_root: &Path,
) -> Result<Vec<InstalledPluginSnapshot>, String> {
    let plugin_id = plugin_id.trim();
    if plugin_id.is_empty()
        || plugin_id == "."
        || plugin_id == ".."
        || plugin_id
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
    {
        return Err("插件 ID 不合法".into());
    }
    let mut registry = PluginRegistry::new(plugins_root).map_err(|error| error.to_string())?;
    let package_installed = registry.find(plugin_id).is_some();
    if package_installed && registry.plugin_enabled_anywhere(plugin_id) {
        return Err("请先禁用该插件的所有游戏，再执行卸载".into());
    }

    let package_dir = plugins_root.join(plugin_id);
    if package_dir.parent() != Some(plugins_root) {
        return Err("插件目录不在受控插件根目录内".into());
    }
    // Delete the package first: if files are locked, no settings or state is lost.
    remove_owned_path(&package_dir, true)?;

    let data_dir = global_root.join("PluginData").join(plugin_id);
    remove_owned_path(&data_dir, true)?;
    let mirror_state_dir = global_root.join("PluginMirrorState").join(plugin_id);
    remove_owned_path(&mirror_state_dir, true)?;
    let log_file = global_root
        .join("PluginLogs")
        .join(format!("{plugin_id}.log"));
    remove_owned_path(&log_file, false)?;
    let log_tmp = global_root
        .join("PluginLogs")
        .join(format!("{plugin_id}.log.tmp"));
    remove_owned_path(&log_tmp, false)?;
    for cached in cached_package_paths_for_plugin(&global_root.join("PluginAssetCache"), plugin_id)?
    {
        remove_owned_path(&cached, false)?;
    }

    registry
        .remove_plugin_state(plugin_id)
        .map_err(|error| error.to_string())?;
    let mut settings = PluginSettingsStore::new(global_root.join("plugin-settings.json"))
        .map_err(|error| error.to_string())?;
    settings
        .remove_namespace(plugin_id)
        .map_err(|error| error.to_string())?;

    let snapshots =
        if let Some(game_name) = game_name.map(str::trim).filter(|name| !name.is_empty()) {
            snapshot_for_game(&registry, game_name)
        } else {
            snapshot(&registry)
        };
    Ok(snapshots)
}

#[tauri::command]
pub fn read_plugin_log(plugin_id: String) -> Result<String, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginLogWriter::new(&plugin_id)
        .and_then(|writer| writer.read())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn clear_plugin_log(plugin_id: String) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginLogWriter::new(&plugin_id)
        .and_then(|writer| writer.clear())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_plugin_setting(
    plugin_id: String,
    key: String,
) -> Result<Option<serde_json::Value>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    PluginSettingsStore::from_default_location()
        .and_then(|store| store.get(&plugin_id, &key))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_plugin_setting(
    plugin_id: String,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    if registry.find(&plugin_id).is_none() {
        return Err(format!("plugin not found: {plugin_id}"));
    }
    let mut store =
        PluginSettingsStore::from_default_location().map_err(|error| error.to_string())?;
    store
        .set(&plugin_id, &key, value)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{
        registry::{ExternalDependencyStatus, InstalledPlugin},
        PluginCompatibility, PluginContributions, PluginPermission,
    };
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn uninstall_test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-plugin-uninstall-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_uninstall_fixture(root: &Path, id: &str) -> PathBuf {
        let package = root.join("source").join(id);
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("ssmt-plugin.json"), format!(r#"{{
          "schemaVersion":1,"id":"{id}","name":"Uninstall fixture","version":"1.0.0","author":"Test",
          "compatibility":{{"ssmt":">=4.0.0","platforms":["windows-x64"]}},
          "contributions":{{}},"externalDependencies":[],"permissions":[]
        }}"#)).unwrap();
        package
    }

    fn write_cached_package(path: &Path, id: &str) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("ssmt-plugin.json", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(
            format!(
                r#"{{
          "schemaVersion":1,"id":"{id}","name":"Cached fixture",
          "description":"full inspection intentionally rejects this cache",
          "version":"1.0.0","author":"Test",
          "compatibility":{{"ssmt":">=4.0.0","platforms":["windows-x64"]}},
          "contributions":{{}},"externalDependencies":[],"permissions":[]
        }}"#
            )
            .as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn uninstall_removes_package_data_logs_cache_settings_and_state() {
        let root = uninstall_test_root("complete");
        let plugins_root = root.join("Plugins");
        let global_root = root.join("Global");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        for id in ["example.remove", "example.keep"] {
            let package = write_uninstall_fixture(&root, id);
            registry.install_directory_package(&package).unwrap();
            registry.set_game_enabled("GIMI", id, true).unwrap();
        }
        let enabled_error =
            uninstall_plugin_from_roots("example.remove", None, &plugins_root, &global_root)
                .unwrap_err();
        assert!(enabled_error.contains("请先禁用"));
        registry
            .set_game_enabled("GIMI", "example.remove", false)
            .unwrap();

        let data = global_root.join("PluginData/example.remove/presets");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("Default.json"), b"preset").unwrap();
        let logs = global_root.join("PluginLogs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("example.remove.log"), b"log").unwrap();
        fs::write(logs.join("example.remove.log.tmp"), b"tmp").unwrap();
        let mirror_state = global_root.join("PluginMirrorState/example.remove");
        fs::create_dir_all(&mirror_state).unwrap();
        fs::write(mirror_state.join("presets.json"), b"{}").unwrap();
        let cache = global_root.join("PluginAssetCache");
        fs::create_dir_all(&cache).unwrap();
        let cached = cache.join("fixture.asset");
        write_cached_package(&cached, "example.remove");
        let mut settings = crate::plugin::settings::PluginSettingsStore::new(
            global_root.join("plugin-settings.json"),
        )
        .unwrap();
        settings
            .set(
                "example.remove",
                "native_injection_risk_ack_v1",
                serde_json::json!({"accepted":true}),
            )
            .unwrap();

        let snapshots =
            uninstall_plugin_from_roots("example.remove", None, &plugins_root, &global_root)
                .unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].manifest.id, "example.keep");
        assert!(!plugins_root.join("example.remove").exists());
        assert!(!global_root.join("PluginData/example.remove").exists());
        assert!(!logs.join("example.remove.log").exists());
        assert!(!logs.join("example.remove.log.tmp").exists());
        assert!(!global_root
            .join("PluginMirrorState/example.remove")
            .exists());
        assert!(!cached.exists());
        let restarted = PluginRegistry::new(&plugins_root).unwrap();
        assert!(restarted.find("example.remove").is_none());
        assert!(restarted.find("example.keep").is_some());
        let settings = crate::plugin::settings::PluginSettingsStore::new(
            global_root.join("plugin-settings.json"),
        )
        .unwrap();
        assert_eq!(
            settings
                .get("example.remove", "native_injection_risk_ack_v1")
                .unwrap(),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uninstall_can_retry_after_the_package_was_already_removed() {
        let root = uninstall_test_root("retry");
        let plugins_root = root.join("Plugins");
        let global_root = root.join("Global");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        let package = write_uninstall_fixture(&root, "example.retry");
        registry.install_directory_package(&package).unwrap();
        registry
            .set_game_enabled("GIMI", "example.retry", true)
            .unwrap();
        let data = global_root.join("PluginData/example.retry/presets");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("Default.json"), b"preset").unwrap();
        let mirror_state = global_root.join("PluginMirrorState/example.retry");
        fs::create_dir_all(&mirror_state).unwrap();
        fs::write(mirror_state.join("SecondaryMotion.json"), b"{}").unwrap();

        fs::remove_dir_all(plugins_root.join("example.retry")).unwrap();
        uninstall_plugin_from_roots("example.retry", None, &plugins_root, &global_root).unwrap();

        assert!(!global_root.join("PluginData/example.retry").exists());
        assert!(!global_root.join("PluginMirrorState/example.retry").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_exposes_every_game_scope_enabled_for_uninstall_confirmation() {
        let root = uninstall_test_root("enabled-games");
        let plugins_root = root.join("Plugins");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        let package = write_uninstall_fixture(&root, "example.cross-game");
        registry.install_directory_package(&package).unwrap();
        registry
            .set_game_enabled("GIMI", "example.cross-game", true)
            .unwrap();
        registry
            .set_game_enabled("SRMI", "example.cross-game", true)
            .unwrap();

        let snapshot = super::snapshot(&registry);
        let plugin = snapshot
            .iter()
            .find(|plugin| plugin.manifest.id == "example.cross-game")
            .unwrap();
        assert_eq!(plugin.enabled_games, vec!["gimi", "srmi"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_installation_has_no_synthetic_optional_plugin_entry() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-empty-market-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let registry = PluginRegistry::new(&root).unwrap();
        for preset in ["GIMI", "SRMI", "WWMI", "ZZMI", "EFMI"] {
            assert!(super::snapshot_for_game(&registry, preset).is_empty());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_plugin_install_does_not_read_stale_state_or_create_registry() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ssmt-empty-plugin-launch-{}-{nonce}",
            std::process::id()
        ));
        let plugins = root.join("plugins");
        assert!(installed_plugin_registry_at(plugins.clone())
            .unwrap()
            .is_none());
        assert!(!plugins.exists());

        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("registry-state.json"), b"invalid JSON").unwrap();
        assert!(installed_plugin_registry_at(plugins.clone())
            .unwrap()
            .is_none());
        assert_eq!(
            fs::read(plugins.join("registry-state.json")).unwrap(),
            b"invalid JSON"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn derives_application_lifecycle_without_runtime_states() {
        let manifest = PluginManifest {
            schema_version: 1,
            id: "ssmt.fixture".to_string(),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            author: "SSMT".to_string(),
            plugin_api_version: 1,
            compatibility: PluginCompatibility {
                ssmt: ">=4.0.0".to_string(),
                platforms: vec!["windows-x64".to_string()],
                plugin_api: None,
                games: Vec::new(),
            },
            contributions: PluginContributions::default(),
            external_dependencies: vec![],
            permissions: vec![PluginPermission::ProcessSpawn],
        };
        let mut plugin = InstalledPlugin {
            manifest,
            package_root: PathBuf::from("fixture"),
            enabled: false,
            official: false,
            external_dependencies: BTreeMap::new(),
        };
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Disabled
        ));
        plugin.enabled = true;
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Enabled
        ));
        plugin.external_dependencies.insert(
            "external".to_string(),
            ExternalDependencyState {
                status: ExternalDependencyStatus::Missing,
                path: None,
                reason: Some("missing file".to_string()),
            },
        );
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::ExternalDependencyMissing
        ));
        plugin.external_dependencies.clear();
        plugin.manifest.schema_version = 2;
        plugin.manifest.compatibility.plugin_api = Some(">=2.0.0 <3.0.0".into());
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Incompatible
        ));
    }

    #[test]
    fn missing_direct_injected_dll_marks_plugin_broken() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-injected-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("plugin")).unwrap();
        let manifest = PluginManifest {
            schema_version: 1,
            id: "example.motion".to_string(),
            name: "Secondary Motion".to_string(),
            version: "3.2.0".to_string(),
            author: "SSMT".to_string(),
            plugin_api_version: 1,
            compatibility: PluginCompatibility {
                ssmt: ">=4.2.0".to_string(),
                platforms: vec!["windows-x64".to_string()],
                plugin_api: None,
                games: vec!["EFMI".to_string()],
            },
            contributions: PluginContributions {
                injected_dlls: vec![crate::plugin::InjectedDllContribution {
                    path: "plugin/sbm.dll".to_string(),
                    executable_names: Vec::new(),
                    sha256: None,
                }],
                ..PluginContributions::default()
            },
            external_dependencies: Vec::new(),
            permissions: vec![PluginPermission::NativeInject],
        };
        let mut plugin = InstalledPlugin {
            manifest,
            package_root: root.clone(),
            enabled: true,
            official: false,
            external_dependencies: BTreeMap::new(),
        };
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Broken
        ));
        fs::write(root.join("plugin/sbm.dll"), b"fixture").unwrap();
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Enabled
        ));
        plugin.manifest.permissions.clear();
        assert!(matches!(
            lifecycle_status(&plugin),
            PluginLifecycleStatus::Broken
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
