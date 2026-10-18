use super::{
    dlss5_hot_runtime::{catalog, INSTALL_LOCK},
    dlss5::DLSS5_PLUGIN_ID,
    managed_reshade,
    registry::PluginRegistry,
    settings::PluginSettingsStore,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[path = "gimi_hot_bridge.rs"]
mod bridge;
#[path = "gimi_hot_download.rs"]
pub mod download;
#[path = "gimi_hot_optiscaler.rs"]
mod optiscaler;

// 这组组件属于 CXP GIMI 集成链；禁止用其 GIMI DLL 覆盖 SSMT Runtime。
// 固定二进制组来自 cc46971 的 package-manifest.sha256，NR Runtime 按显卡 Profile 配对。
const COMMON: &[(&str, &str)] = &[
    (
        "components/ReShade/ReShade64.dll",
        "084fa6b41ac9fc0cf66055e95c91c69a5bef153612dda15e18eacb21c9f52c54",
    ),
    (
        "components/Bridge/Dx11FsrBridge.dll",
        "1ab7fbd90b69d8f57851fdfc039aa3890aee46aea2dcb4db72c8049282140310",
    ),
    (
        "components/OptiScaler/OptiScaler.dll",
        "2d86c3e6728f018f259aabc052f78e53a039a1b0d6bbbc3aeb7571bb67d61f44",
    ),
    (
        "components/OptiScaler/nvngx_dlss.dll",
        "c85f971ce023c9f3492fc7455f0b01a24ba18ea39636407a846902c4360b0b7e",
    ),
    (
        "components/OptiScaler/D3D12_Optiscaler/D3D12Core.dll",
        "07d286c306f8117321422affd9e6388c12d0fb4be1c7fc689d9e899324feeb24",
    ),
];
const ADDON_HASH: &str = "522d979cbff335710f362b9fc2f330988673d7f8c7a1a2d93da9980ec8dda695";

pub fn applies(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("YuanShen.exe")
                || name.eq_ignore_ascii_case("GenshinImpact.exe")
        })
}

fn enabled(registry: &PluginRegistry) -> bool {
    registry
        .find(DLSS5_PLUGIN_ID)
        .is_some_and(|plugin| plugin.enabled)
}

pub(super) fn setting_key(game_name: &str) -> String {
    format!(
        "gimiHotRuntime.{:x}",
        Sha256::digest(game_name.trim().to_lowercase().as_bytes())
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HotRuntimeInstallation {
    pub root: PathBuf,
    pub profile: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotRuntimePlan {
    pub dlls: Vec<PathBuf>,
    pub environment: BTreeMap<String, Option<String>>,
    pub runtime_directory: PathBuf,
    pub required_launch_args: Vec<String>,
    pub excluded_launch_args: Vec<String>,
    pub log_sources: Vec<crate::commands::launch_logs::LaunchLogSource>,
}

fn profile_files(profile: &str) -> Result<(String, Vec<(String, String)>), String> {
    let (directory, chain, runtime) = match profile {
        "rtx50" => (
            "pre-nr",
            "db26e486592b252072ba5734fc2b27412863b8526826225640c837d4b4d11b60",
            "e16bcf15e16e13f527491cdf7845b2fe6521a738d8f7c9c721866a8496e1fc8e",
        ),
        "rtx30" => (
            "pre-nr-rtx30",
            "46041a5ff91ae2fd907e310d132aabc3c4a1ecd48dace511b8672909d5d9c2fb",
            "6eb209e764f39872625debd6abaf45e2bb6322f6f270f781f70c059ae30b3927",
        ),
        _ => return Err("请选择 RTX 50 或 RTX 30/40 组件 Profile".into()),
    };
    let directory = format!("components/DLSS5/Addons/{directory}");
    let mut files = COMMON
        .iter()
        .map(|(path, hash)| (path.to_string(), hash.to_string()))
        .collect::<Vec<_>>();
    for (name, hash) in [
        ("nr-before-sr.zh-CN.addon64", ADDON_HASH),
        ("nrchain_nvngx.dll", chain),
        ("nvngx_dlssnr.dll", runtime),
    ] {
        files.push((format!("{directory}/{name}"), hash.into()));
    }
    Ok((directory, files))
}

pub(super) fn canonical_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    let canonical = path.canonicalize()?;
    // canonicalize 在 Windows 返回 \\?\ 路径；第三方 INI/路径拼接可能含正斜杠，
    // 不满足 verbatim 路径规则。身份校验后以普通绝对路径传给组件。
    #[cfg(windows)]
    if let Some(text) = canonical.to_str() {
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            return Ok(PathBuf::from(format!("\\\\{unc}")));
        }
        if let Some(disk) = text.strip_prefix("\\\\?\\") {
            return Ok(PathBuf::from(disk));
        }
    }
    Ok(canonical)
}

pub(super) fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let canonical = canonical_path(&root.join(relative))
        .map_err(|error| format!("热注入组件缺失 {relative}：{error}"))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(format!("热注入组件越出目录或不是文件：{relative}"));
    }
    Ok(canonical)
}

pub(super) fn verify(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length == 0 || length > 256 * 1024 * 1024 {
        return Err(format!("组件大小无效：{}", path.display()));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != expected {
        return Err(format!("组件版本或显卡 Profile 不匹配：{}", path.display()));
    }
    Ok(())
}

pub(super) fn compatibility_revision(profile: &str) -> Result<String, String> {
    // 历史记录须对应本体支持的固定 Bridge/输入契约，不能仅凭自存哈希接受未知组件组。
    let files = profile_files(profile)?.1;
    let baseline = serde_json::to_vec(&files).map_err(|e| e.to_string())?;
    Ok(format!("fsr2-{:x}", Sha256::digest(baseline)))
}

pub(super) fn require_owned(installation: &HotRuntimeInstallation) -> Result<(), String> {
    let root = canonical_path(&installation.root).map_err(|e| e.to_string())?;
    let parent = canonical_path(&installation_parent()).map_err(|e| e.to_string())?;
    if root.parent() != Some(parent.as_path()) {
        return Err("FSR2 Bridge 组件不在 SSMT 自有安装目录中".into());
    }
    Ok(())
}

pub(super) fn validate(installation: &HotRuntimeInstallation) -> Result<(PathBuf, String), String> {
    let root = canonical_path(&installation.root).map_err(|error| error.to_string())?;
    let (addon, files) = profile_files(&installation.profile)?;
    for (relative, hash) in files {
        verify(&checked_path(&root, &relative)?, &hash)?;
    }
    for path in [
        "components/Bridge/Dx11FsrBridge.ini".to_string(),
        "components/OptiScaler/OptiScaler.ini".into(),
        format!("{addon}/nr_before_sr.ini"),
    ] {
        checked_path(&root, &path)?;
    }
    Ok((root, addon))
}

pub(super) fn read_text(path: &Path) -> Result<String, String> {
    if fs::metadata(path).map_err(|error| error.to_string())?.len() > 4 * 1024 * 1024 {
        return Err(format!("配置文件过大：{}", path.display()));
    }
    fs::read_to_string(path)
        .map(|text| text.trim_start_matches('\u{feff}').to_string())
        .map_err(|error| error.to_string())
}

pub(super) fn rebase_search_paths(text: &mut String, base: &Path) {
    for key in ["EffectSearchPaths", "TextureSearchPaths"] {
        let value = text.lines().find_map(|line| {
            line.trim()
                .split_once('=')
                .filter(|(name, _)| name.trim().eq_ignore_ascii_case(key))
                .map(|(_, value)| value.to_string())
        });
        if let Some(value) = value {
            let paths = value
                .split(',')
                .map(|part| {
                    let path = Path::new(part.trim());
                    if path.is_absolute() {
                        path.to_string_lossy().into_owned()
                    } else {
                        base.join(path).to_string_lossy().into_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            managed_reshade::set_entry(text, "GENERAL", key, &paths);
        }
    }
}

fn prepare_at(
    registry: &PluginRegistry,
    installation: &HotRuntimeInstallation,
) -> Result<HotRuntimePlan, String> {
    let (root, addon) = validate(installation)?;
    let bridge = bridge::stage(registry, &root)?;
    let opti = optiscaler::stage(registry, &root, &installation.profile)?;
    let reshade = root.join("state/reshade-runtime");
    fs::create_dir_all(&reshade).map_err(|error| error.to_string())?;
    let mut composed = super::hot_reshade::compose(
        registry, &root.join(&addon),
        &["nr-before-sr.zh-CN.addon64".into()], "", "",
    )?;
    for (section, key, value) in [
        (
            "ADDON",
            "AddonPath",
            root.join(&addon).to_string_lossy().into_owned(),
        ),
        (
            "ADDON",
            "LoadFromDllMain",
            "nr-before-sr.zh-CN.addon64".into(),
        ),
        (
            "GENERAL",
            "PresetPath",
            reshade
                .join("ReShadePreset.ini")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "GENERAL",
            "IntermediateCachePath",
            reshade.join("cache").to_string_lossy().into_owned(),
        ),
    ] {
        managed_reshade::set_entry(&mut composed.game_ini, section, key, &value);
    }
    let ini_path = opti.join("OptiScaler.ini");
    let mut ini = if ini_path.try_exists().map_err(|error| error.to_string())? {
        let relative = ini_path.strip_prefix(&root).map_err(|error| error.to_string())?;
        read_text(&checked_path(&root, &relative.to_string_lossy())?)?
    } else {
        read_text(&checked_path(&root, "components/OptiScaler/OptiScaler.ini")?)?
    };
    configure_optiscaler(&mut ini, &opti);
    composed.extra_files.insert(opti.join("OptiScaler.ini"), ini.into_bytes());
    super::hot_reshade::save_config(&reshade, composed, "", &["nr-before-sr.zh-CN.addon64".into()])?;
    let dlls = vec![
        root.join("components/ReShade/ReShade64.dll"),
        bridge,
        opti.join("OptiScaler.dll"),
    ];
    let environment = [
        (
            "GIMI_HOSTED_RESHADE_DLL",
            Some(
                root.join("components/ReShade/ReShade64.dll")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ),
        (
            "GIMI_HOSTED_RESHADE_CONFIG",
            Some(reshade.join("ReShade.ini").to_string_lossy().into_owned()),
        ),
        (
            "RESHADE_BASE_PATH_OVERRIDE",
            Some(reshade.to_string_lossy().into_owned()),
        ),
        ("RESHADE_DISABLE_GRAPHICS_HOOK", Some("1".into())),
        ("RESHADE_DISABLE_INPUT_HOOK", None),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect();
    Ok(HotRuntimePlan {
        dlls,
        environment,
        log_sources: vec![crate::commands::launch_logs::LaunchLogSource { name: "ReShade".into(), path: reshade.join("ReShade.log") }],
        runtime_directory: root,
        required_launch_args: Vec::new(),
        excluded_launch_args: Vec::new(),
    })
}

fn configure_optiscaler(ini: &mut String, opti: &Path) {
    for (key, value) in [
        ("OptiDllPath", opti.to_path_buf()),
        ("NvngxDlssPath", opti.join("nvngx_dlss.dll")),
        ("NvngxFeaturePath", opti.to_path_buf()),
    ] {
        managed_reshade::set_entry(ini, "Libraries", key, &value.to_string_lossy());
    }
    // 上游模板的 auto 默认选择 FSR2 DX12；原神 Bridge 的契约必须显式固定为 DX11。
    // 仅指定 dlss_12 输出后端并不能选择输入 API，会在首次 Dispatch 读取空参数表。
    for (section, key, value) in [
        ("Upscalers", "Dx11Upscaler", "dlss_12"),
        ("Inputs", "EnableFsr2Inputs", "true"),
        ("Inputs", "UseFsr2Inputs", "true"),
        ("Inputs", "UseFsr2Dx11Inputs", "true"),
        ("Inputs", "UseFsr2VulkanInputs", "false"),
        ("DLSS", "Enabled", "true"),
        ("DLSS", "UseGenericAppIdWithDlss", "true"),
        ("Hooks", "SkipD3D11DeviceVTableHooks", "true"),
        ("Plugins", "LoadReShade", "false"),
    ] {
        managed_reshade::set_entry(ini, section, key, value);
    }
    // NR 由同一条前置 Add-on 链负责，避免 OptiScaler 再运行一次后置 NR。
    managed_reshade::set_entry(ini, "DlssNr", "Enabled", "false");
}

#[tauri::command]
pub fn import_gimi_hot_runtime(
    game_name: String,
    game_executable: String,
    source_directory: String,
    profile: String,
) -> Result<HotRuntimeInstallation, String> {
    let _lock = INSTALL_LOCK.try_lock().map_err(|_| "热组件正在配置或清理，请稍候")?;
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    if !enabled(&registry) || !applies(Path::new(&game_executable)) {
        return Err("请先为原神启用 DLSS5 插件".into());
    }
    ensure_stopped(Path::new(&game_executable))?;
    let source = HotRuntimeInstallation {
        root: PathBuf::from(source_directory),
        profile,
    };
    let root = new_installation_root();
    fs::create_dir_all(installation_parent()).map_err(|e| e.to_string())?;
    fs::create_dir(&root).map_err(|e| e.to_string())?;
    let mut pending = download::PendingInstallation::new(root.clone(), installation_parent());
    let installation = import_at(&source, root)?;
    // 复制期间启用状态和游戏进程可能变化，切换指针前再次检查。
    download::require_enabled(&game_name, Path::new(&game_executable))?;
    catalog::record_gimi_installation(&game_name, &installation)?;
    pending.committed = true;
    Ok(installation)
}

pub(super) fn installation_parent() -> PathBuf {
    crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssets")
        .join(DLSS5_PLUGIN_ID)
        .join("gimi-hot")
}

fn new_installation_root() -> PathBuf {
    installation_parent().join(format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn import_at(
    source: &HotRuntimeInstallation,
    root: PathBuf,
) -> Result<HotRuntimeInstallation, String> {
    let (source_root, addon) = validate(source)?;
    let (_, binaries) = profile_files(&source.profile)?;
    for relative in binaries.into_iter().map(|(relative, _)| relative).chain([
        "components/Bridge/Dx11FsrBridge.ini".into(),
        "components/OptiScaler/OptiScaler.ini".into(),
        format!("{addon}/nr_before_sr.ini"),
    ]) {
        let source_file = checked_path(&source_root, &relative)?;
        let destination = root.join(&relative);
        fs::create_dir_all(destination.parent().ok_or("组件路径无效")?)
            .map_err(|error| error.to_string())?;
        fs::copy(source_file, destination).map_err(|error| error.to_string())?;
    }
    let installation = HotRuntimeInstallation {
        root,
        profile: source.profile.clone(),
    };
    validate(&installation)?;
    Ok(installation)
}

pub(super) fn ensure_stopped(executable: &Path) -> Result<(), String> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::Always),
    );
    let canonical = executable
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if system.processes().values().any(|process| {
        process
            .exe()
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            == Some(&canonical)
    }) {
        return Err("请关闭游戏后再准备热注入组件".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn gimi_hot_runtime_status(
    game_name: String,
) -> Result<Option<HotRuntimeInstallation>, String> {
    super::commands::read_graphics_status(move || gimi_hot_runtime_status_blocking(game_name)).await
}

fn gimi_hot_runtime_status_blocking(
    game_name: String,
) -> Result<Option<HotRuntimeInstallation>, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(&game_name);
    if !enabled(&registry) {
        return Ok(None);
    }
    PluginSettingsStore::from_default_location()
        .map_err(|error| error.to_string())?
        .get(DLSS5_PLUGIN_ID, &setting_key(&game_name))
        .map_err(|error| error.to_string())?
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
        .transpose()
}

#[tauri::command]
pub fn prepare_gimi_hot_runtime(
    game_name: String,
    game_executable: String,
) -> Result<Option<HotRuntimePlan>, String> {
    if !applies(Path::new(&game_executable)) {
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
    prepare_locked(&registry, &game_name, Path::new(&game_executable), &lock).map(Some)
}

pub(super) fn prepare_locked(
    registry: &PluginRegistry,
    game_name: &str,
    game_executable: &Path,
    _lock: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<HotRuntimePlan, String> {
    // 统一入口已持有同一把锁；保持路线选择到配置准备连续，避免重入锁或并发切换实现。
    ensure_stopped(game_executable)?;
    if !matches!(
        super::dlss5::inspect_game_executable(game_executable),
        super::dlss5::Dlss5State::NotManaged
    ) {
        return Err(
            "请先在 DLSS5 设置中恢复旧的 Swapper 安装，再使用原神热注入组件；不会临时放回代理 DLL"
                .into(),
        );
    }
    let value = PluginSettingsStore::from_default_location()
        .map_err(|error| error.to_string())?
        .get(DLSS5_PLUGIN_ID, &setting_key(game_name))
        .map_err(|error| error.to_string())?
        .filter(|value| !value.is_null())
        .ok_or(
            "原神 DLSS5 组件尚未配置，请到插件市场的 DLSS5 功能设置中选择显卡，自动下载并配置",
        )?;
    let installation = serde_json::from_value(value)
        .map_err(|error| format!("原神热注入组件配置无效：{error}"))?;
    require_owned(&installation)?;
    prepare_at(registry, &installation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};

    #[test]
    fn compatibility_revision_is_profile_specific_and_rejects_unknown_profiles() {
        let rtx50 = compatibility_revision("rtx50").unwrap();
        assert_eq!(rtx50, compatibility_revision("rtx50").unwrap());
        assert_ne!(rtx50, compatibility_revision("rtx30").unwrap());
        assert!(compatibility_revision("unknown").is_err());
    }

    #[test]
    fn optiscaler_templates_select_dx11_input_and_keep_user_options() {
        for input in ["auto", "false", "true"] {
            let mut ini = format!(
                "[Inputs]\nEnableFsr2Inputs={input}\nUseFsr2Inputs={input}\nUseFsr2Dx11Inputs={input}\nUseFsr2VulkanInputs=true\n\
                 [DLSS]\nEnabled=false\nUseGenericAppIdWithDlss=auto\n\
                 [Plugins]\nLoadReShade=true\n[Hooks]\nSkipD3D11DeviceVTableHooks=auto\n\
                 [Menu]\nShortcutKey=0x24\n[FrameGen]\nEnabled=false\n\
                 [DlssNr]\nEnabled=true\n[Upscalers]\nDx11Upscaler=auto\n"
            );
            configure_optiscaler(&mut ini, Path::new("C:/SSMT/组件/OptiScaler"));
            for line in [
                "EnableFsr2Inputs=true",
                "UseFsr2Inputs=true",
                "UseFsr2Dx11Inputs=true",
                "UseFsr2VulkanInputs=false",
                "UseGenericAppIdWithDlss=true",
                "SkipD3D11DeviceVTableHooks=true",
                "LoadReShade=false",
                "Dx11Upscaler=dlss_12",
                "ShortcutKey=0x24",
            ] {
                assert!(
                    ini.lines().any(|actual| actual == line),
                    "缺少 {line}: {ini}"
                );
            }
            assert!(ini.contains("[DLSS]\nEnabled=true"));
            assert!(ini.contains("[DlssNr]\nEnabled=false"));
            assert!(ini.contains("[FrameGen]\nEnabled=false"));
            let once = ini.clone();
            configure_optiscaler(&mut ini, Path::new("C:/SSMT/组件/OptiScaler"));
            assert_eq!(ini, once);
        }
    }

    pub(super) fn registry_with_bridge(root: &Path, dll: &[u8]) -> PluginRegistry {
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let package = registry.plugins_root().join(DLSS5_PLUGIN_ID).join("0.2.1");
        fs::create_dir_all(package.join("bridge")).unwrap();
        fs::write(
            package.join("ssmt-plugin.json"),
            serde_json::json!({
                "schemaVersion": 1, "id": DLSS5_PLUGIN_ID, "name": "Bridge 回归",
                "version": "0.2.1", "author": "SSMT",
                "compatibility": {"ssmt": ">=4.0.0", "platforms": ["windows-x64"]},
                "contributions": {}, "permissions": []
            })
            .to_string(),
        )
        .unwrap();
        fs::write(package.join("bridge/Dx11FsrBridge.dll"), dll).unwrap();
        fs::write(
            package.join("bridge/Dx11FsrBridge.dll.sha256"),
            format!("{:x}", Sha256::digest(dll)),
        )
        .unwrap();
        registry.refresh().unwrap();
        registry.set_enabled(DLSS5_PLUGIN_ID, true).unwrap();
        registry
            .set_game_enabled("GIMI", DLSS5_PLUGIN_ID, true)
            .unwrap();
        registry
    }

    pub(super) fn real_bridge() -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../native/dist/Release/SSMT-Gimi-Bridge.dll"),
        )
        .expect("请先构建 Native Release 的 SSMT-Gimi-Bridge")
    }

    #[test]
    fn adapted_bridge_is_optional_verified_and_keeps_upstream_files() {
        let temp = std::env::temp_dir().join(format!(
            "ssmt-bridge-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(temp.join("hot/components/Bridge")).unwrap();
        let root = canonical_path(&temp.join("hot")).unwrap();
        fs::write(
            root.join("components/Bridge/Dx11FsrBridge.ini"),
            "[Dx11FsrBridge]\nEnabled=1\n",
        )
        .unwrap();
        fs::write(
            root.join("components/Bridge/Dx11FsrBridge.dll"),
            b"upstream",
        )
        .unwrap();
        let mut registry = PluginRegistry::new(temp.join("plugins")).unwrap();
        assert!(bridge::stage(&registry, &root).is_err());
        assert!(!root.join("state").exists());
        registry = registry_with_bridge(&temp, b"adapted fixture");
        let staged = bridge::stage(&registry, &root).unwrap();
        assert_eq!(fs::read(&staged).unwrap(), b"adapted fixture");
        assert_eq!(
            fs::read(root.join("components/Bridge/Dx11FsrBridge.dll")).unwrap(),
            b"upstream"
        );
        assert!(staged.starts_with(root.join("state/bridge")));
        fs::write(&staged, b"damaged cached copy").unwrap();
        assert_eq!(bridge::stage(&registry, &root).unwrap(), staged);
        assert_eq!(fs::read(&staged).unwrap(), b"adapted fixture");
        let package = registry.find(DLSS5_PLUGIN_ID).unwrap().package_root.clone();
        fs::write(
            package.join("bridge/Dx11FsrBridge.dll"),
            b"damaged installed component",
        )
        .unwrap();
        assert!(bridge::stage(&registry, &root).is_err());
        registry.set_enabled(DLSS5_PLUGIN_ID, false).unwrap();
        assert!(bridge::stage(&registry, &root).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn profiles_keep_chain_and_nr_runtime_paired() {
        let (a, x) = profile_files("rtx50").unwrap();
        let (b, y) = profile_files("rtx30").unwrap();
        assert_ne!(a, b);
        assert_ne!(x.last().unwrap().1, y.last().unwrap().1);
        assert!(profile_files("auto").is_err());
        assert!(applies(Path::new("C:/Game/YuanShen.exe")));
        assert!(applies(Path::new("C:/Game/GenshinImpact.exe")));
        assert!(!applies(Path::new("C:/Game/Endfield.exe")));
    }
    #[test]
    fn search_paths_follow_the_declared_effect_owner() {
        let mut ini = "[GENERAL]\nEffectSearchPaths=.\\Shaders\\**,C:\\Shared\\Shaders\nTextureSearchPaths=Textures\n".to_string();
        rebase_search_paths(&mut ini, Path::new("C:/Shade"));
        assert!(ini.contains("C:/Shade\\.\\Shaders\\**"));
        assert!(ini.contains("C:\\Shared\\Shaders"));
        assert!(ini.contains("C:/Shade\\Textures"));
    }
    #[test]
    fn content_hash_and_file_boundary_are_checked() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-hot-runtime-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("component.dll"), b"fixture").unwrap();
        let canonical = canonical_path(&root).unwrap();
        let path = checked_path(&canonical, "component.dll").unwrap();
        verify(&path, &format!("{:x}", Sha256::digest(b"fixture"))).unwrap();
        assert!(verify(&path, &"0".repeat(64)).is_err());
        assert!(checked_path(&canonical, "../").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "需要 SSMT_HOT_TEST_SOURCE 指定完整组件包；可选 SSMT_HOT_TEST_SHADE_ROOT、SSMT_HOT_TEST_OPTISCALER 和 SSMT_HOT_FSR2_FIXTURE"]
    fn prepares_real_components_without_game_directory_writes() {
        let source = HotRuntimeInstallation {
            root: std::env::var_os("SSMT_HOT_TEST_SOURCE")
                .expect("缺少组件包目录")
                .into(),
            profile: std::env::var("SSMT_HOT_TEST_PROFILE").unwrap_or("rtx50".into()),
        };
        let before = fs::read(source.root.join("components/OptiScaler/OptiScaler.ini")).unwrap();
        let temp = std::env::temp_dir().join(format!(
            "ssmt-real-hot-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let installation = import_at(&source, temp.join("owned 组件")).unwrap();
        let mut registry = registry_with_bridge(&temp, &real_bridge());
        let localized = std::env::var_os("SSMT_HOT_TEST_OPTISCALER");
        if let Some(dll) = &localized {
            let package = registry.find(DLSS5_PLUGIN_ID).unwrap().package_root.join("optiscaler");
            fs::create_dir(&package).unwrap();
            let bytes = fs::read(dll).unwrap();
            fs::write(package.join("OptiScaler.dll"), &bytes).unwrap();
            fs::write(package.join("OptiScaler.dll.sha256"), format!("{:x}", Sha256::digest(&bytes))).unwrap();
        }
        let plan = prepare_at(&registry, &installation).unwrap();
        let opti_config = plan.dlls[2].parent().unwrap().join("OptiScaler.ini");
        let opti_ini = read_text(&opti_config).unwrap();
        if localized.is_some() {
            assert!(plan.dlls[2].starts_with(installation.root.join("state/optiscaler")));
            let original = installation.root.join("components/OptiScaler");
            assert_eq!(fs::read(original.join("OptiScaler.ini")).unwrap(), before);
            verify(&original.join("OptiScaler.dll"), COMMON[2].1).unwrap();
            let dll_directory = plan.dlls[2].parent().unwrap();
            verify(&dll_directory.join("nvngx_dlss.dll"), COMMON[3].1).unwrap();
            verify(&dll_directory.join("D3D12_Optiscaler/D3D12Core.dll"), COMMON[4].1).unwrap();
            let mut saved = opti_ini.clone();
            managed_reshade::set_entry(&mut saved, "Menu", "Language", "en");
            fs::write(&opti_config, &saved).unwrap();
            assert_eq!(prepare_at(&registry, &installation).unwrap().dlls, plan.dlls);
            assert_eq!(read_text(&opti_config).unwrap(), saved);
        }
        for line in [
            "UseFsr2Dx11Inputs=true",
            "UseFsr2VulkanInputs=false",
            "SkipD3D11DeviceVTableHooks=true",
            "LoadReShade=false",
        ] {
            assert!(opti_ini.lines().any(|actual| actual == line));
        }
        assert_eq!(plan.dlls.len(), 3);
        assert!(plan.dlls[0].ends_with("ReShade64.dll"));
        assert!(plan.dlls[1].ends_with("Dx11FsrBridge.dll"));
        assert!(plan.dlls[2].ends_with("OptiScaler.dll"));
        let config = installation.root.join("state/reshade-runtime/ReShade.ini");
        assert_eq!(
            canonical_path(Path::new(
                plan.environment["GIMI_HOSTED_RESHADE_CONFIG"]
                    .as_ref()
                    .unwrap()
            ))
            .unwrap(),
            canonical_path(&config).unwrap()
        );
        assert!(read_text(&config)
            .unwrap()
            .contains("LoadFromDllMain=nr-before-sr.zh-CN.addon64"));
        if let Some(fixture) = std::env::var_os("SSMT_HOT_FSR2_FIXTURE") {
            // 只在显式的实组件回归中执行独立测试进程，不启动游戏。
            assert!(localized.is_none(), "旧崩溃 RVA 仅对应原始组件，不适用于汉化重编译版本");
            let mut diagnostics = read_text(&opti_config).unwrap();
            managed_reshade::set_entry(&mut diagnostics, "Log", "LogToFile", "true");
            managed_reshade::set_entry(&mut diagnostics, "Log", "LogLevel", "2");
            let run = || {
                let mut command = std::process::Command::new(&fixture);
                command.args(&plan.dlls);
                for (name, value) in &plan.environment {
                    if let Some(value) = value {
                        command.env(name, value);
                    } else {
                        command.env_remove(name);
                    }
                }
                command
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = command.spawn().unwrap();
                let start = std::time::Instant::now();
                loop {
                    if child.try_wait().unwrap().is_some() {
                        break;
                    }
                    if start.elapsed() > std::time::Duration::from_secs(45) {
                        child.kill().unwrap();
                        let _ = child.wait();
                        panic!(
                            "FSR2 独立回归超时，资源保留于 {}",
                            installation.root.display()
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let output = child.wait_with_output().unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                eprintln!(
                    "status={}\n{stdout}\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
                output
            };
            // 反例只改输入 API，必须命中实际游戏 dump 中同一个空参数表故障。
            let mut legacy = diagnostics.clone();
            managed_reshade::set_entry(&mut legacy, "Inputs", "UseFsr2Dx11Inputs", "auto");
            fs::write(&opti_config, legacy).unwrap();
            let baseline = run();
            assert!(
                baseline.status.code() == Some(10)
                    && String::from_utf8_lossy(&baseline.stdout)
                        .contains("module=OptiScaler.dll rva=1c3e57"),
                "旧配置未复现预期崩溃，资源保留于 {}",
                installation.root.display()
            );
            fs::write(&opti_config, diagnostics).unwrap();
            let output = run();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success()
                    && stdout.contains("FSR2 DX11 component dispatch completed"),
                "FSR2 独立回归失败，资源保留于 {}",
                installation.root.display()
            );
        }
        if let Some(shade) = std::env::var_os("SSMT_HOT_TEST_SHADE_ROOT") {
            let shade_ini_path = Path::new(&shade).join("ReShade.ini");
            let shade_before = fs::read(&shade_ini_path).unwrap();
            let package = registry
                .plugins_root()
                .join(HOYOSHADE_PLUGIN_ID)
                .join("0.1.0");
            fs::create_dir_all(&package).unwrap();
            fs::write(package.join("ssmt-plugin.json"), include_str!("../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.0/ssmt-plugin.json")).unwrap();
            registry.refresh().unwrap();
            registry
                .set_external_dependency_path(
                    HOYOSHADE_PLUGIN_ID,
                    HOYOSHADE_DEPENDENCY_ID,
                    Path::new(&shade),
                )
                .unwrap();
            registry
                .set_game_enabled("GIMI", HOYOSHADE_PLUGIN_ID, true)
                .unwrap();
            let combined = prepare_at(&registry.for_game("GIMI"), &installation).unwrap();
            assert_eq!(combined.dlls, plan.dlls);
            let text = read_text(&config).unwrap();
            let shade_ini = read_text(&shade_ini_path).unwrap();
            if let Some(disabled) = shade_ini
                .lines()
                .find(|line| line.starts_with("DisabledAddons="))
            {
                assert!(text.lines().any(|line| line == disabled));
            }
            assert!(text.contains(
                &canonical_path(Path::new(&shade))
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            ));
            let (addon, _) = profile_files(&installation.profile).unwrap();
            // 只检查组合器实际登记的贡献，不能向目录塞入未知 Add-on 再期待删除。
            let ownership: std::collections::BTreeMap<String, String> = serde_json::from_str(
                &read_text(&config.parent().unwrap().join("ReShade.ssmt-addons.json")).unwrap(),
            ).unwrap();
            for name in ownership.keys() {
                assert!(installation.root.join(&addon).join(name).is_file());
            }
            registry
                .set_game_enabled("GIMI", HOYOSHADE_PLUGIN_ID, false)
                .unwrap();
            prepare_at(&registry.for_game("GIMI"), &installation).unwrap();
            for name in ownership.keys() {
                assert!(!installation.root.join(&addon).join(name).exists());
            }
            // 界面字体、截图目录等用户状态允许保留；停用后不能继续贡献效果搜索路径。
            let saved = read_text(&config).unwrap();
            let shade_root = canonical_path(Path::new(&shade)).unwrap().to_string_lossy().to_string();
            for line in saved.lines().filter(|line| line.starts_with("EffectSearchPaths=") || line.starts_with("TextureSearchPaths=")) {
                assert!(!line.contains(&shade_root));
            }
            assert_eq!(fs::read(shade_ini_path).unwrap(), shade_before);
        }
        assert_eq!(
            fs::read(source.root.join("components/OptiScaler/OptiScaler.ini")).unwrap(),
            before
        );
        fs::remove_dir_all(temp).unwrap();
    }
}
