#[path = "dlss5_hot_catalog.rs"]
pub mod catalog;

use super::{
    dlss5::{Dlss5State, DLSS5_PLUGIN_ID},
    gimi_hot_runtime::{
        self, canonical_path, checked_path, ensure_stopped, verify, HotRuntimePlan,
    },
    managed_reshade,
    managed_stack::Dlss5RouteScan,
    registry::{game_scope_preset, PluginRegistry},
    resource_cache::download_verified_with_progress,
    settings::PluginSettingsStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};
use tauri::ipc::Channel;

pub(super) static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installation {
    root: PathBuf,
    profile: String,
    route: String,
    revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline: Option<Manifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    revision: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Asset {
    url: String,
    sha256: String,
    size: u64,
    route: String,
    profile: String,
    zip_offset: usize,
    zip_length: usize,
    files: Vec<AssetFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetFile {
    entry: String,
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    phase: &'static str,
    file: String,
    downloaded_bytes: u64,
    total_bytes: u64,
}

fn manifest() -> Result<Manifest, String> {
    serde_json::from_str(include_str!("dlss5_hot_assets.json")).map_err(|error| error.to_string())
}

fn manifest_for(registry: &PluginRegistry) -> Result<Manifest, String> {
    match super::dlss5_policy::Policy::from_registry(registry)? {
        Some(policy) => serde_json::from_str(&policy.assets(false)?).map_err(|e| e.to_string()),
        None => manifest(),
    }
}

fn route_for_registry(registry: &PluginRegistry, preset: &str, executable: &Path) -> Result<(String, String), String> {
    if let Some(policy) = super::dlss5_policy::Policy::from_registry(registry)? {
        let route = policy.select(preset, executable)?;
        return Ok((route.route.clone(), route.api.clone()));
    }
    route_for(preset, executable).map(|(route, api)| (route.into(), api.into()))
}

pub(super) fn applies(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            [
                "StarRail.exe",
                "Client-Win64-Shipping.exe",
                "ZenlessZoneZero.exe",
                "YuanShen.exe",
                "GenshinImpact.exe",
            ]
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        })
}

fn route_for(preset: &str, executable: &Path) -> Result<(&'static str, &'static str), String> {
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match (
        preset.trim().to_ascii_uppercase().as_str(),
        name.to_ascii_lowercase().as_str(),
    ) {
        ("SRMI", "starrail.exe") => Ok(("feeder", "d3d11")),
        ("GIMI", "yuanshen.exe" | "genshinimpact.exe") => Ok(("feeder", "d3d11")),
        ("WWMI", "client-win64-shipping.exe") | ("ZZMI", "zenlesszonezero.exe") => {
            Ok(("native-bridge", "d3d11"))
        }
        ("ZZMIDX12", "zenlesszonezero.exe") => Ok(("native", "d3d12")),
        _ => Err("当前游戏预设与目标程序没有匹配的 DLSS5 热注入路线".into()),
    }
}

fn enabled(registry: &PluginRegistry) -> bool {
    registry
        .find(DLSS5_PLUGIN_ID)
        .is_some_and(|plugin| plugin.enabled)
}

pub(super) fn route_scan(
    registry: &PluginRegistry,
    preset: &str,
    executable: &Path,
    api: &str,
) -> Result<Dlss5RouteScan, String> {
    if !enabled(registry) {
        return Err("请先为当前游戏启用 DLSS5 插件".into());
    }
    let (route, expected_api) = route_for_registry(registry, preset, executable)?;
    if api != "auto" && api != expected_api {
        return Err("图形 API 与游戏预设不一致，请选择对应 DX11 / DX12 预设".into());
    }
    Ok(Dlss5RouteScan {
        routes: if gimi_hot_runtime::applies(executable) {
            vec!["gimi-hot".into(), "ssmt-feeder".into()]
        } else {
            vec![format!("ssmt-{route}")]
        },
        api: expected_api.clone(),
        api_label: if expected_api == "d3d11" {
            "DirectX 11"
        } else {
            "DirectX 12"
        }
        .into(),
        installed_route: None,
        managed_host: true,
        owner: Some("SSMT".into()),
        anti_cheat_warning: false,
    })
}

fn selected<'a>(
    manifest: &'a Manifest,
    route: &str,
    profile: &str,
) -> Result<Vec<&'a Asset>, String> {
    if !["feeder", "native-bridge", "native"].contains(&route)
        || !["rtx50", "rtx30"].contains(&profile)
    {
        return Err("DLSS5 路线或显卡系列无效".into());
    }
    let assets = manifest
        .assets
        .iter()
        .filter(|asset| {
            (asset.route.is_empty() || asset.route == route)
                && (asset.profile.is_empty() || asset.profile == profile)
        })
        .collect::<Vec<_>>();
    let mut names = HashSet::new();
    if manifest.assets.len() > 64 || manifest.revision.is_empty() || assets.is_empty() { return Err("DLSS5 组件清单为空或过大".into()); }
    for asset in &assets {
        let url = reqwest::Url::parse(&asset.url).map_err(|e| e.to_string())?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
            || asset.size == 0 || asset.size > 1024 * 1024 * 1024 || asset.files.is_empty() || asset.files.len() > 256
            || asset.sha256.len() != 64 || !asset.sha256.bytes().all(|c| c.is_ascii_hexdigit()) { return Err("DLSS5 下载声明无效".into()); }
        for file in &asset.files {
            if !safe_relative(&file.path)
                || file.size == 0
                || file.size > 256 * 1024 * 1024
                || file.sha256.len() != 64 || !file.sha256.bytes().all(|c| c.is_ascii_hexdigit())
                || !names.insert(file.path.to_ascii_lowercase())
            {
                return Err("DLSS5 组件清单含无效或重复目标".into());
            }
        }
    }
    Ok(assets)
}

fn safe_relative(name: &str) -> bool {
    super::validate_package_relative_path(name).is_ok()
        && !name.is_empty()
        && !name.contains(['\\', ':'])
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn installation_parent() -> PathBuf {
    crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssets")
        .join(DLSS5_PLUGIN_ID)
        .join("hot")
}

fn setting_key(game_name: &str) -> String {
    format!(
        "hotRuntime.{:x}",
        Sha256::digest(game_name.trim().to_lowercase().as_bytes())
    )
}

pub(super) fn active_route_key(game_name: &str) -> String {
    format!("hotRoute.{:x}", Sha256::digest(game_name.trim().to_lowercase().as_bytes()))
}

// 老配置没有路线字段时继续使用 FSR2 Bridge；仅显式配置成功才切换。
pub(super) fn gimi_route(game_name: &str) -> Result<String, String> {
    let value = PluginSettingsStore::from_default_location()
        .map_err(|error| error.to_string())?
        .get(DLSS5_PLUGIN_ID, &active_route_key(game_name))
        .map_err(|error| error.to_string())?;
    decode_gimi_route(value)
}

fn decode_gimi_route(value: Option<serde_json::Value>) -> Result<String, String> {
    match value.as_ref().and_then(serde_json::Value::as_str) {
        None if value.is_none() => Ok("gimi-hot".into()),
        Some(route @ ("gimi-hot" | "ssmt-feeder")) => Ok(route.into()),
        _ => Err("原神 DLSS5 路线设置无效，请重新配置组件".into()),
    }
}

fn load(game_name: &str) -> Result<Option<Installation>, String> {
    PluginSettingsStore::from_default_location()
        .map_err(|error| error.to_string())?
        .get(DLSS5_PLUGIN_ID, &setting_key(game_name))
        .map_err(|error| error.to_string())?
        .map(|value| {
            if value.is_null() { Ok(None) } else { serde_json::from_value(value).map(Some).map_err(|error| format!("DLSS5 组件设置无效：{error}")) }
        })
        .transpose().map(Option::flatten)
}

pub(super) fn log_path(game_name: &str) -> Result<Option<PathBuf>, String> {
    let Some(installation) = load(game_name)? else {
        return Ok(None);
    };
    require_owned(&installation)?;
    Ok(Some(installation.root.join("ReShade.log")))
}

fn require_owned(installation: &Installation) -> Result<(), String> {
    let root = canonical_path(&installation.root).map_err(|error| error.to_string())?;
    let parent = canonical_path(&installation_parent()).map_err(|error| error.to_string())?;
    if root.parent() != Some(parent.as_path()) {
        return Err("DLSS5 组件不在 SSMT 自有安装目录中".into());
    }
    Ok(())
}

fn validate(installation: &Installation, manifest: &Manifest) -> Result<PathBuf, String> {
    let manifest = installation.baseline.as_ref().unwrap_or(manifest);
    if installation.revision != manifest.revision {
        return Err("DLSS5 组件版本已改变，请重新配置".into());
    }
    let root = canonical_path(&installation.root).map_err(|error| error.to_string())?;
    for asset in selected(manifest, &installation.route, &installation.profile)? {
        for file in &asset.files {
            let path = checked_path(&root, &file.path)?;
            if fs::metadata(&path)
                .map_err(|error| error.to_string())?
                .len()
                != file.size
            {
                return Err(format!("DLSS5 组件大小不匹配：{}", file.path));
            }
            verify(&path, &file.sha256)?;
        }
    }
    Ok(root)
}

fn write_selected(reader: &mut impl Read, file: &AssetFile, root: &Path) -> Result<(), String> {
    let destination = root.join(&file.path);
    fs::create_dir_all(destination.parent().ok_or("组件目录无效")?)
        .map_err(|error| error.to_string())?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| error.to_string())?;
    let size = std::io::copy(&mut reader.take(file.size + 1), &mut output)
        .map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    if size != file.size {
        return Err(format!("组件解压大小不匹配：{}", file.path));
    }
    verify(&destination, &file.sha256)
}

// 仅解出清单列出的文件；ReShade 安装程序内 ZIP 的偏移与整个安装程序哈希绑定，不运行 EXE。
fn extract(asset: &Asset, cached: &Path, root: &Path) -> Result<(), String> {
    if asset.files.len() == 1 && asset.files[0].entry.is_empty() {
        return write_selected(
            &mut fs::File::open(cached).map_err(|error| error.to_string())?,
            &asset.files[0],
            root,
        );
    }
    let bytes = fs::read(cached).map_err(|error| error.to_string())?;
    let end = if asset.zip_length == 0 {
        bytes.len()
    } else {
        asset
            .zip_offset
            .checked_add(asset.zip_length)
            .ok_or("归档偏移无效")?
    };
    let slice = bytes.get(asset.zip_offset..end).ok_or("归档范围无效")?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(slice)).map_err(|error| error.to_string())?;
    for file in &asset.files {
        let mut found = None;
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|error| error.to_string())?;
            if entry.name().replace('\\', "/") == file.entry {
                if found.replace(index).is_some() || entry.is_dir() || entry.size() != file.size {
                    return Err("归档含重复或无效组件项".into());
                }
            }
        }
        let mut entry = archive
            .by_index(found.ok_or_else(|| format!("归档缺少 {}", file.entry))?)
            .map_err(|error| error.to_string())?;
        write_selected(&mut entry, file, root)?;
    }
    Ok(())
}

fn require_enabled(game: &str, executable: &Path) -> Result<(String, String), String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(game);
    if !enabled(&registry) {
        return Err("请先为当前游戏启用 DLSS5 插件".into());
    }
    let route = route_for_registry(&registry, &game_scope_preset(game), executable)?;
    ensure_stopped(executable)?;
    Ok(route)
}

#[tauri::command]
pub async fn dlss5_hot_runtime_status(game_name: String) -> Result<Option<Installation>, String> {
    super::commands::read_graphics_status(move || dlss5_hot_runtime_status_blocking(game_name)).await
}

fn dlss5_hot_runtime_status_blocking(game_name: String) -> Result<Option<Installation>, String> {
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(None);
    };
    if !enabled(&registry.for_game(&game_name)) {
        return Ok(None);
    }
    let Some(installation) = load(&game_name)? else {
        return Ok(None);
    };
    require_owned(&installation)?;
    validate(&installation, &manifest_for(&registry)?)?;
    Ok(Some(installation))
}

struct PendingInstallation(PathBuf, bool);
impl Drop for PendingInstallation {
    fn drop(&mut self) {
        if !self.1 {
            if let (Ok(root), Ok(parent)) =
                (self.0.canonicalize(), installation_parent().canonicalize())
            {
                if root.parent() == Some(parent.as_path()) {
                    let _ = fs::remove_dir_all(root);
                }
            }
        }
    }
}

#[tauri::command]
pub async fn download_dlss5_hot_runtime(
    game_name: String,
    game_executable: String,
    profile: String,
    on_progress: Channel<DownloadProgress>,
) -> Result<Installation, String> {
    let _lock = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "DLSS5 组件正在配置，请稍候")?;
    let (route, _) = require_enabled(&game_name, Path::new(&game_executable))?;
    let registry = PluginRegistry::from_default_location().map_err(|e| e.to_string())?.for_game(&game_name);
    let manifest = manifest_for(&registry)?;
    let assets = selected(&manifest, &route, &profile)?;
    let total_bytes = assets.iter().map(|asset| asset.size).sum();
    let parent = installation_parent();
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    let root = parent.join(format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let mut pending = PendingInstallation(root.clone(), false);
    let client = reqwest::Client::builder()
        .user_agent("SSMT4-DLSS5-Setup")
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|error| error.to_string())?;
    let cache = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssetCache");
    let mut completed = 0;
    for asset in assets {
        let mut last_update = std::time::Instant::now();
        let cached = download_verified_with_progress(
            &client,
            &asset.url,
            &asset.sha256,
            asset.size,
            &cache,
            |bytes| {
                if bytes == asset.size || last_update.elapsed().as_millis() >= 150 {
                    let _ = on_progress.send(DownloadProgress {
                        phase: "downloading",
                        file: asset.url.clone(),
                        downloaded_bytes: completed + bytes,
                        total_bytes,
                    });
                    last_update = std::time::Instant::now();
                }
            },
        )
        .await?;
        let _ = on_progress.send(DownloadProgress {
            phase: "extracting",
            file: asset.url.clone(),
            downloaded_bytes: completed + asset.size,
            total_bytes,
        });
        extract(asset, &cached, &root)?;
        completed += asset.size;
    }
    let installation = Installation {
        root,
        profile,
        route: route.clone(),
        revision: manifest.revision.clone(),
        baseline: Some(manifest.clone()),
    };
    validate(&installation, &manifest)?;
    let (current_route, _) = require_enabled(&game_name, Path::new(&game_executable))?;
    if current_route != route {
        return Err("下载期间游戏预设已改变，请重新配置".into());
    }
    catalog::record_installation(&game_name, &installation)?;
    pending.1 = true;
    Ok(installation)
}

fn prepare_at(
    registry: &PluginRegistry,
    installation: &Installation,
) -> Result<HotRuntimePlan, String> {
    let root = validate(installation, &manifest_for(registry)?)?;
    let mut required = vec!["renodx-dlss5.addon64".into()];
    match installation.route.as_str() {
        "feeder" => required.push("dlss5-feed.addon64".into()),
        "native-bridge" => required.push("dlss5-bridge.addon64".into()),
        _ => (),
    }
    let dlss_ini = if installation.route == "feeder" {
        format!(
            "[GENERAL]\nEffectSearchPaths={}\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=3\n",
            root.join("Shaders").display()
        )
    } else {
        String::new()
    };
    let dlss_preset = if installation.route == "feeder" {
        "Techniques=Lumenite_Kernel@lumenite_Kernel.fx,DLSS5_Feed@DLSS5_Feed.fx\nTechniqueSorting=Lumenite_Kernel@lumenite_Kernel.fx,DLSS5_Feed@DLSS5_Feed.fx\n[DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=3\nDEBUG_VIEW=0\nMV_SCALE=1.0\nMV_SIGN=1.0,1.0\n"
    } else {
        ""
    };
    let mut composed =
        super::hot_reshade::compose(registry, &root, &required, &dlss_ini, dlss_preset)?;
    for (section, key, value) in [
        ("ADDON", "AddonPath", root.to_string_lossy().into_owned()),
        ("ADDON", "LoadFromDllMain", required.join(",")),
        (
            "GENERAL",
            "PresetPath",
            root.join("ReShadePreset.ini")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "GENERAL",
            "IntermediateCachePath",
            root.join("cache").to_string_lossy().into_owned(),
        ),
        ("RenoDX.DLSS5", "EnableHooks", "2".into()),
        ("RenoDX.DLSS5", "NeuralUplift", "1".into()),
        ("RenoDX.DLSS5", "NREnableUpscaling", "0".into()),
    ] {
        managed_reshade::set_entry(&mut composed.game_ini, section, key, &value);
    }
    super::hot_reshade::save_config(&root, composed, dlss_preset, &required)?;
    // Feeder 的 SDK 也从 Add-on 同目录寻找 snippet；提前加载绝对路径避免搜索到游戏中的旧副本。
    let mut dlls = Vec::new();
    if installation.route == "feeder" {
        dlls.push(root.join("nvngx_dlss.dll"));
    }
    dlls.push(root.join("ReShade64.dll"));
    Ok(HotRuntimePlan {
        dlls,
        log_sources: vec![crate::commands::launch_logs::LaunchLogSource { name: "ReShade".into(), path: root.join("ReShade.log") }],
        environment: [
            (
                "RESHADE_BASE_PATH_OVERRIDE".into(),
                Some(root.to_string_lossy().into_owned()),
            ),
            ("RESHADE_DISABLE_GRAPHICS_HOOK".into(), None),
            ("RESHADE_DISABLE_INPUT_HOOK".into(), None),
            ("GIMI_HOSTED_RESHADE_DLL".into(), None),
            ("GIMI_HOSTED_RESHADE_CONFIG".into(), None),
        ]
        .into_iter()
        .collect(),
        runtime_directory: root,
        required_launch_args: Vec::new(),
        excluded_launch_args: Vec::new(),
    })
}

pub(super) fn ensure_no_external_host(executable: &Path) -> Result<(), String> {
    let game = executable.parent().ok_or("游戏目录无效")?;
    for name in ["dxgi.dll", "d3d11.dll"] {
        if game.join(name).exists() {
            return Err(format!("游戏目录存在图形代理 {name}，无法确认单一 ReShade Host。请确认来源并通过原管理器恢复 RHI / ReShade 安装后再使用 SSMT 热注入；SSMT 不会覆盖原文件"));
        }
    }
    // ReShade 先读取游戏目录的 INSTALL.BasePath，再读取环境变量；旧配置也会劫走自有 Host。
    let ini = game.join("ReShade.ini");
    if ini.is_file() {
        let text = gimi_hot_runtime::read_text(&ini)?;
        let mut install = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') && line.ends_with(']') {
                install = line[1..line.len() - 1]
                    .trim()
                    .eq_ignore_ascii_case("INSTALL");
            } else if install {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("BasePath") && !value.trim().is_empty() {
                        return Err("游戏目录的 ReShade.ini 指向另一套 BasePath，请通过原管理器恢复该配置后再使用 SSMT 热注入；SSMT 不会修改原配置".into());
                    }
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn configure_dlss5_launch_settings(
    game_name: String,
    game_executable: String,
) -> Result<(), String> {
    let executable = Path::new(&game_executable);
    if !gimi_hot_runtime::applies(executable) {
        return Ok(());
    }
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(());
    };
    if !enabled(&registry.for_game(&game_name)) {
        return Ok(());
    }
    route_for_registry(&registry.for_game(&game_name), &game_scope_preset(&game_name), executable)?;
    ensure_stopped(executable)?;
    crate::commands::gimi_settings::configure_dlss5(
        &game_executable,
        gimi_route(&game_name)? == "gimi-hot",
    )
}

#[tauri::command]
pub fn prepare_dlss5_hot_runtime(
    game_name: String,
    game_executable: String,
) -> Result<Option<HotRuntimePlan>, String> {
    let executable = Path::new(&game_executable);
    if !applies(executable) {
        return Ok(None);
    }
    let Some(registry) = super::commands::installed_plugin_registry()? else {
        return Ok(None);
    };
    let registry = registry.for_game(&game_name);
    if !enabled(&registry) {
        return Ok(None);
    }
    let lock = INSTALL_LOCK.try_lock().map_err(|_| "DLSS5 热组件正在配置或清理，本次跳过该贡献")?;
    let (route, _) = route_for_registry(&registry, &game_scope_preset(&game_name), executable)?;
    if gimi_hot_runtime::applies(executable) && gimi_route(&game_name)? == "gimi-hot" {
        return gimi_hot_runtime::prepare_locked(&registry, &game_name, executable, &lock).map(Some);
    }
    ensure_stopped(executable)?;
    if !matches!(
        super::dlss5::inspect_game_executable(executable),
        Dlss5State::NotManaged
    ) {
        return Err("请先在 DLSS5 设置中恢复旧的 Swapper 安装，再配置 SSMT 热注入组件".into());
    }
    ensure_no_external_host(executable)?;
    let installation =
        load(&game_name)?.ok_or("DLSS5 组件尚未配置，请到插件市场的功能设置中自动下载并配置")?;
    require_owned(&installation)?;
    if installation.route != route {
        return Err("DLSS5 组件与当前游戏预设不符，请重新配置".into());
    }
    let mut plan = prepare_at(&registry, &installation)?;
    if let Some(policy) = super::dlss5_policy::Policy::from_registry(&registry)? {
        let route = policy.select(&game_scope_preset(&game_name), executable)?;
        plan.required_launch_args = route.required_launch_args.clone();
        plan.excluded_launch_args = route.excluded_launch_args.clone();
    } else if game_scope_preset(&game_name).eq_ignore_ascii_case("WWMI") {
        // 旧包的冻结适配；新包从声明读取本次参数约束。
        plan.required_launch_args.push("-dx11".into());
        plan.excluded_launch_args = ["-dx11", "-d3d11", "-dx12", "-d3d12", "-vulkan"]
            .into_iter().map(String::from).collect();
    }
    Ok(Some(plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genshin_keeps_legacy_route_until_explicit_setup_and_exposes_both_implementations() {
        assert_eq!(decode_gimi_route(None).unwrap(), "gimi-hot");
        assert_eq!(decode_gimi_route(Some("ssmt-feeder".into())).unwrap(), "ssmt-feeder");
        assert_eq!(decode_gimi_route(Some("gimi-hot".into())).unwrap(), "gimi-hot");
        assert!(decode_gimi_route(Some("native".into())).is_err());
        assert!(decode_gimi_route(Some(serde_json::Value::Null)).is_err());
    }

    #[test]
    fn foreign_proxy_and_base_path_cannot_redirect_or_duplicate_the_managed_host() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-foreign-host-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let exe = root.join("StarRail.exe");
        assert!(ensure_no_external_host(&exe).is_ok());
        for name in ["dxgi.dll", "d3d11.dll"] {
            fs::write(root.join(name), b"foreign owner").unwrap();
            assert!(ensure_no_external_host(&exe).unwrap_err().contains(name));
            assert_eq!(fs::read(root.join(name)).unwrap(), b"foreign owner");
            fs::remove_file(root.join(name)).unwrap();
        }
        let foreign = "\u{feff}[install]\nBasePath=D:/RHI/Owned\n";
        fs::write(root.join("ReShade.ini"), foreign).unwrap();
        assert!(ensure_no_external_host(&exe)
            .unwrap_err()
            .contains("BasePath"));
        assert_eq!(
            fs::read_to_string(root.join("ReShade.ini")).unwrap(),
            foreign
        );
        fs::write(
            root.join("ReShade.ini"),
            "[GENERAL]\nPresetPath=MyPreset.ini\n",
        )
        .unwrap();
        assert!(ensure_no_external_host(&exe).is_ok());
        let canonical = root.canonicalize().unwrap();
        assert_eq!(
            canonical.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }

    #[test]
    fn routes_require_both_actual_preset_and_target() {
        for (preset, exe, expected) in [
            ("GIMI", "YuanShen.exe", ("feeder", "d3d11")),
            ("gimi", "GenshinImpact.exe", ("feeder", "d3d11")),
            ("SRMI", "StarRail.exe", ("feeder", "d3d11")),
            (
                "wwmi",
                "Client-Win64-Shipping.exe",
                ("native-bridge", "d3d11"),
            ),
            ("ZZMI", "ZenlessZoneZero.exe", ("native-bridge", "d3d11")),
            ("ZZMIDX12", "ZenlessZoneZero.exe", ("native", "d3d12")),
        ] {
            assert_eq!(route_for(preset, Path::new(exe)).unwrap(), expected);
        }
        assert!(route_for("GIMI", Path::new("StarRail.exe")).is_err());
        assert!(route_for("SRMI", Path::new("ZenlessZoneZero.exe")).is_err());
    }

    #[test]
    fn every_route_and_profile_has_a_complete_isolated_contract() {
        let manifest = manifest().unwrap();
        for route in ["feeder", "native-bridge", "native"] {
            for profile in ["rtx50", "rtx30"] {
                let files = selected(&manifest, route, profile)
                    .unwrap()
                    .iter()
                    .flat_map(|a| a.files.iter().map(|f| f.path.as_str()))
                    .collect::<Vec<_>>();
                for required in ["ReShade64.dll", "renodx-dlss5.addon64", "nvngx_dlssnr.dll"] {
                    assert!(files.contains(&required));
                }
                assert_eq!(files.contains(&"dlss5-feed.addon64"), route == "feeder");
                assert_eq!(files.contains(&"Shaders/DLSS5_Feed.fx"), route == "feeder");
                assert_eq!(files.contains(&"nvngx_dlss.dll"), route == "feeder");
                assert_eq!(
                    files.contains(&"dlss5-bridge.addon64"),
                    route == "native-bridge"
                );
                assert!(!files
                    .iter()
                    .any(|name| name.contains("Dx11FsrBridge") || name.contains("OptiScaler")));
            }
        }
        for invalid in ["../outside", "/outside", "C:/outside", "Shaders\\outside"] {
            assert!(!safe_relative(invalid));
        }
    }

    #[test]
    #[ignore = "设置 SSMT_DLSS5_COMPONENT_PROBE_DIR 后验证已下载的真实固定组件；不启动游戏"]
    fn extracts_real_pinned_components_and_prepares_each_route() {
        let cache = PathBuf::from(
            std::env::var_os("SSMT_DLSS5_COMPONENT_PROBE_DIR").expect("component probe directory"),
        );
        let inputs = [
            "reshade.exe",
            "consumer.zip",
            "bridge.addon64",
            "feeder.zip",
            "sr.zip",
            "nr50.zip",
            "nr30.zip",
            "lumenite.zip",
            "ReShade.fxh",
            "ReShadeUI.fxh",
        ];
        let manifest = manifest().unwrap();
        let temp = std::env::temp_dir().join(format!(
            "ssmt-dlss5-contracts-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&temp).unwrap();
        let registry = PluginRegistry::new(temp.join("plugins")).unwrap();
        for route in ["feeder", "native-bridge", "native"] {
            for profile in ["rtx50", "rtx30"] {
                let root = temp.join(format!("{route}-{profile}"));
                fs::create_dir(&root).unwrap();
                for asset in selected(&manifest, route, profile).unwrap() {
                    let index = manifest
                        .assets
                        .iter()
                        .position(|a| std::ptr::eq(a, asset))
                        .unwrap();
                    let input = cache.join(inputs[index]);
                    verify(&input, &asset.sha256).unwrap();
                    extract(asset, &input, &root).unwrap();
                }
                let installation = Installation {
                    root,
                    route: route.into(),
                    profile: profile.into(),
                    revision: manifest.revision.clone(),
                    baseline: None,
                };
                let plan = prepare_at(&registry, &installation).unwrap();
                assert_eq!(plan.dlls.len(), if route == "feeder" { 2 } else { 1 });
                assert_eq!(plan.environment["RESHADE_DISABLE_GRAPHICS_HOOK"], None);
                let ini = fs::read_to_string(plan.runtime_directory.join("ReShade.ini")).unwrap();
                assert!(ini.contains("NeuralUplift=1"));
                if route == "feeder" {
                    assert!(ini.contains("DLSS5_MV_PROVIDER=3"));
                    let preset =
                        fs::read_to_string(plan.runtime_directory.join("ReShadePreset.ini"))
                            .unwrap();
                    assert!(
                        preset.find("Lumenite_Kernel").unwrap()
                            < preset.find("DLSS5_Feed@").unwrap()
                    );
                }
            }
        }
        let canonical = temp.canonicalize().unwrap();
        assert_eq!(
            canonical.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(canonical).unwrap();
    }
}
