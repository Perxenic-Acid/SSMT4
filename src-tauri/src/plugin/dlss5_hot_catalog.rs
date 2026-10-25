//! 热组件历史与清理只操作当前实例登记的自有目录，不触碰游戏目录。
use super::*;

const OWNER_FILE: &str = ".ssmt-installation-owner";

fn history_key(game: &str) -> String {
    format!(
        "hotHistory.{:x}",
        Sha256::digest(game.trim().to_lowercase().as_bytes())
    )
}
fn owner(game: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(game.trim().to_lowercase().as_bytes())
    )
}

fn entries(store: &PluginSettingsStore, game: &str) -> Result<Vec<Installation>, String> {
    let value = store
        .get(DLSS5_PLUGIN_ID, &history_key(game))
        .map_err(|e| e.to_string())?;
    let mut entries: Vec<Installation> = value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("热组件历史损坏：{e}"))?
        .unwrap_or_default();
    // 保留旧设置键，首次管理时将两种实现的当前安装一起纳入历史。
    for route in ["gimi-hot", "native"] {
        if let Some(current) = current(store, game, route)? {
            if !entries.iter().any(|item| item.root == current.root) {
                entries.push(current);
            }
        }
    }
    Ok(entries)
}

fn gimi_entry(
    installation: gimi_hot_runtime::HotRuntimeInstallation,
) -> Result<Installation, String> {
    Ok(Installation {
        revision: gimi_hot_runtime::compatibility_revision(&installation.profile)?,
        root: installation.root,
        profile: installation.profile,
        route: "gimi-hot".into(),
        baseline: None,
    })
}

fn gimi_installation(item: &Installation) -> gimi_hot_runtime::HotRuntimeInstallation {
    gimi_hot_runtime::HotRuntimeInstallation {
        root: item.root.clone(),
        profile: item.profile.clone(),
    }
}

fn current_key(game: &str, route: &str) -> String {
    if route == "gimi-hot" {
        gimi_hot_runtime::setting_key(game)
    } else {
        setting_key(game)
    }
}

fn current(
    store: &PluginSettingsStore,
    game: &str,
    route: &str,
) -> Result<Option<Installation>, String> {
    store
        .get(DLSS5_PLUGIN_ID, &current_key(game, route))
        .map_err(|e| e.to_string())?
        .filter(|v| !v.is_null())
        .map(|value| {
            if route == "gimi-hot" {
                gimi_entry(serde_json::from_value(value).map_err(|e| e.to_string())?)
            } else {
                serde_json::from_value(value).map_err(|e| e.to_string())
            }
        })
        .transpose()
}

fn launch_route(item: &Installation) -> String {
    if item.route == "gimi-hot" {
        item.route.clone()
    } else {
        format!("ssmt-{}", item.route)
    }
}

fn is_active(store: &PluginSettingsStore, game: &str, item: &Installation) -> Result<bool, String> {
    let selected = current(store, game, &item.route)?;
    let route = store
        .get(DLSS5_PLUGIN_ID, &active_route_key(game))
        .map_err(|e| e.to_string())?;
    // 未保存路线的原神旧配置继续选择 FSR2 Bridge。
    let route_matches = route
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .map_or(item.route == "gimi-hot", |route| {
            route == launch_route(item)
        });
    Ok(route_matches && selected.is_some_and(|selected| selected.root == item.root))
}

fn save(
    store: &mut PluginSettingsStore,
    game: &str,
    route: &str,
    current: Option<&Installation>,
    history: &[Installation],
    activate: bool,
) -> Result<(), String> {
    let value = match current {
        Some(item) if route == "gimi-hot" => serde_json::to_value(gimi_installation(item)),
        _ => serde_json::to_value(current),
    }
    .map_err(|e| e.to_string())?;
    let mut values = vec![
        (current_key(game, route), value),
        (
            history_key(game),
            serde_json::to_value(history).map_err(|e| e.to_string())?,
        ),
    ];
    if activate {
        if let Some(current) = current {
            values.push((
                active_route_key(game),
                serde_json::Value::from(launch_route(current)),
            ));
        }
    }
    store
        .set_many(DLSS5_PLUGIN_ID, values)
        .map_err(|e| e.to_string())
}

pub(super) fn record_installation(game: &str, installation: &Installation) -> Result<(), String> {
    let mut store = PluginSettingsStore::from_default_location().map_err(|e| e.to_string())?;
    let mut history = entries(&store, game)?;
    // 旧安装没有快照时，仅补入可确认的冻结基线，不能猜测它的组件版本。
    let legacy = manifest()?;
    for item in &mut history {
        if item.route != "gimi-hot" && item.baseline.is_none() && item.revision == legacy.revision {
            item.baseline = Some(legacy.clone());
        }
    }
    if !history.iter().any(|item| item.root == installation.root) {
        history.push(installation.clone());
    }
    if history.len() > 32 {
        return Err("热组件历史已达 32 套，请先清理不再使用的组件".into());
    }
    require_entry_owned(installation)?;
    let mut marker = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(installation.root.join(OWNER_FILE))
        .map_err(|e| e.to_string())?;
    marker
        .write_all(owner(game).as_bytes())
        .map_err(|e| e.to_string())?;
    save(
        &mut store,
        game,
        &installation.route,
        Some(installation),
        &history,
        true,
    )
}

pub(crate) fn record_gimi_installation(
    game: &str,
    installation: &gimi_hot_runtime::HotRuntimeInstallation,
) -> Result<(), String> {
    record_installation(game, &gimi_entry(installation.clone())?)
}

fn require_entry_owned(item: &Installation) -> Result<(), String> {
    if item.route == "gimi-hot" {
        gimi_hot_runtime::require_owned(&gimi_installation(item))
    } else {
        require_owned(item)
    }
}

fn validate_entry(item: &Installation, registry: &PluginRegistry) -> Result<(), String> {
    require_entry_owned(item)?;
    if item.route == "gimi-hot" {
        if item.revision != gimi_hot_runtime::compatibility_revision(&item.profile)? {
            return Err("该 FSR2 Bridge 组件基线不受当前版本支持，请重新配置".into());
        }
        gimi_hot_runtime::validate(&gimi_installation(item)).map(|_| ())
    } else {
        validate(item, &manifest_for(registry)?).map(|_| ())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    root: PathBuf,
    profile: String,
    route: String,
    revision: String,
    current: bool,
    usable: bool,
    reason: Option<String>,
    removable: bool,
    cleanup_reason: Option<String>,
}

#[tauri::command]
pub async fn dlss5_hot_runtime_history(game_name: String) -> Result<Vec<HistoryEntry>, String> {
    super::super::commands::read_graphics_status(move || dlss5_hot_runtime_history_blocking(game_name))
        .await
}

fn dlss5_hot_runtime_history_blocking(game_name: String) -> Result<Vec<HistoryEntry>, String> {
    let Some(registry) = super::super::commands::installed_plugin_registry()? else {
        return Ok(Vec::new());
    };
    if !enabled(&registry.for_game(&game_name)) {
        return Ok(Vec::new());
    }
    let store = PluginSettingsStore::from_default_location().map_err(|e| e.to_string())?;
    entries(&store, &game_name)?
        .into_iter()
        .map(|item| {
            let result = validate_entry(&item, &registry);
            let cleanup = checked_cleanup_root(&item.root, &entry_parent(&item), &game_name);
            Ok(HistoryEntry {
                current: is_active(&store, &game_name, &item)?,
                usable: result.is_ok(),
                reason: result.err(),
                root: item.root,
                profile: item.profile,
                route: item.route,
                revision: item.revision,
                removable: cleanup.is_ok(),
                cleanup_reason: cleanup.err(),
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

#[tauri::command]
pub fn select_dlss5_hot_runtime(
    game_name: String,
    game_executable: String,
    root: PathBuf,
) -> Result<(), String> {
    let _lock = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "热组件正在配置，请稍候")?;
    let (route, _) = require_enabled(&game_name, Path::new(&game_executable))?;
    let mut store = PluginSettingsStore::from_default_location().map_err(|e| e.to_string())?;
    let history = entries(&store, &game_name)?;
    let selected = history
        .iter()
        .find(|item| item.root == root)
        .ok_or("该组件不在当前实例历史中")?;
    if selected.route == "gimi-hot" && !gimi_hot_runtime::applies(Path::new(&game_executable))
        || selected.route != "gimi-hot" && !supports_route(&route, &game_scope_preset(&game_name), Path::new(&game_executable), &selected.route)
    {
        return Err("组件路线与当前游戏预设不匹配".into());
    }
    let registry = PluginRegistry::from_default_location().map_err(|e| e.to_string())?;
    validate_entry(selected, &registry)?;
    let (confirmed_route, _) = require_enabled(&game_name, Path::new(&game_executable))?;
    if confirmed_route != route {
        return Err("校验期间游戏预设已改变，请刷新后重新选择组件".into());
    }
    save(
        &mut store,
        &game_name,
        &selected.route,
        Some(selected),
        &history,
        true,
    )
}

fn entry_parent(item: &Installation) -> PathBuf {
    if item.route == "gimi-hot" {
        gimi_hot_runtime::installation_parent()
    } else {
        installation_parent()
    }
}

fn checked_cleanup_root(root: &Path, parent: &Path, game: &str) -> Result<PathBuf, String> {
    let parent = canonical_path(parent).map_err(|e| e.to_string())?;
    let lexical_parent =
        canonical_path(root.parent().ok_or("组件路径无效")?).map_err(|e| e.to_string())?;
    if lexical_parent != parent {
        return Err("清理目标越出热组件目录".into());
    }
    if !root.exists() {
        return Ok(root.to_path_buf());
    }
    let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if !metadata.is_dir() || linked {
        return Err("清理目标不能是链接或普通文件".into());
    }
    let canonical = canonical_path(root).map_err(|e| e.to_string())?;
    if canonical.parent() != Some(parent.as_path()) {
        return Err("清理目标越出热组件目录".into());
    }
    // 中断可能发生在删除最后的标记后、移除空目录前，仅允许补完空目录。
    if !root.join(OWNER_FILE).exists()
        && fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .next()
            .is_none()
    {
        return Ok(canonical);
    }
    if fs::read_to_string(root.join(OWNER_FILE))
        .map_err(|_| "该旧目录缺少实例所有权记录，已保留文件")?
        != owner(game)
    {
        return Err("组件目录不属于当前实例，已保留文件".into());
    }
    Ok(canonical)
}

fn cleanup_files(root: &Path) -> Result<(), String> {
    fn collect(
        root: &Path,
        files: &mut Vec<PathBuf>,
        directories: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let linked = metadata.file_type().is_symlink();
            if linked {
                return Err("组件目录包含链接，已停止清理并保留登记".into());
            }
            if metadata.is_dir() {
                collect(&path, files, directories)?;
                directories.push(path);
            } else if metadata.is_file() {
                files.push(path);
            } else {
                return Err("组件目录包含未知文件类型".into());
            }
        }
        Ok(())
    }
    if !root.exists() {
        return Ok(());
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    collect(root, &mut files, &mut directories)?;
    files.retain(|path| path != &root.join(OWNER_FILE));
    for file in files {
        fs::remove_file(file).map_err(|e| format!("清理未完成，已保留登记以便重试：{e}"))?;
    }
    for directory in directories {
        fs::remove_dir(directory).map_err(|e| e.to_string())?;
    }
    if root.join(OWNER_FILE).exists() {
        fs::remove_file(root.join(OWNER_FILE)).map_err(|e| e.to_string())?;
    }
    fs::remove_dir(root).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_dlss5_hot_runtime(
    game_name: String,
    game_executable: String,
    root: PathBuf,
) -> Result<(), String> {
    let _lock = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "热组件正在配置，请稍候")?;
    require_enabled(&game_name, Path::new(&game_executable))?;
    let mut store = PluginSettingsStore::from_default_location().map_err(|e| e.to_string())?;
    let mut history = entries(&store, &game_name)?;
    let selected = history
        .iter()
        .find(|item| item.root == root)
        .ok_or("该组件不在当前实例历史中")?;
    let route = selected.route.clone();
    let target = checked_cleanup_root(&selected.root, &entry_parent(selected), &game_name)?;
    let current = current(&store, &game_name, &route)?.filter(|item| item.root != root);
    // 先撤销选择，清理失败或进程中断时不再启动残缺组件；历史保留供重试。
    save(
        &mut store,
        &game_name,
        &route,
        current.as_ref(),
        &history,
        false,
    )?;
    cleanup_files(&target)?;
    history.retain(|item| item.root != root);
    save(
        &mut store,
        &game_name,
        &route,
        current.as_ref(),
        &history,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_gimi_and_feeder_history_switch_together_without_cleanup_changing_routes() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-gimi-history-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let mut store = PluginSettingsStore::new(root.join("settings.json")).unwrap();
        let legacy = gimi_hot_runtime::HotRuntimeInstallation {
            root: root.join("old-gimi"),
            profile: "rtx50".into(),
        };
        store
            .set(
                DLSS5_PLUGIN_ID,
                &gimi_hot_runtime::setting_key("原神"),
                serde_json::to_value(&legacy).unwrap(),
            )
            .unwrap();
        let gimi = entries(&store, "原神").unwrap().pop().unwrap();
        assert!(is_active(&store, "原神", &gimi).unwrap());
        let feeder = Installation {
            root: root.join("feeder"),
            route: "feeder".into(),
            profile: "rtx30".into(),
            revision: "v1".into(),
            baseline: None,
        };
        let history = [gimi.clone(), feeder.clone()];
        save(
            &mut store,
            "原神",
            &feeder.route,
            Some(&feeder),
            &history,
            true,
        )
        .unwrap();
        assert!(!is_active(&store, "原神", &gimi).unwrap());
        assert!(is_active(&store, "原神", &feeder).unwrap());
        let reopened = PluginSettingsStore::new(root.join("settings.json")).unwrap();
        assert_eq!(entries(&reopened, "原神").unwrap().len(), 2);
        assert!(entries(&reopened, "另一个实例").unwrap().is_empty());

        save(&mut store, "原神", &gimi.route, Some(&gimi), &history, true).unwrap();
        let raw = store
            .get(DLSS5_PLUGIN_ID, &gimi_hot_runtime::setting_key("原神"))
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_value::<gimi_hot_runtime::HotRuntimeInstallation>(raw)
                .unwrap()
                .root,
            legacy.root
        );
        assert!(is_active(&store, "原神", &gimi).unwrap());
        assert!(!is_active(&store, "原神", &feeder).unwrap());

        // 清理非活动 Feeder 的旧副本，不能顺带把路线切回 Feeder。
        save(
            &mut store,
            "原神",
            &feeder.route,
            Some(&feeder),
            &history,
            false,
        )
        .unwrap();
        assert!(is_active(&store, "原神", &gimi).unwrap());
        save(&mut store, "原神", &gimi.route, None, &history, false).unwrap();
        assert!(current(&store, "原神", "gimi-hot").unwrap().is_none());
        assert!(current(&store, "原神", "feeder").unwrap().is_some());
        assert!(!is_active(&store, "原神", &feeder).unwrap());
        assert_eq!(
            entries(&store, "原神").unwrap().len(),
            2,
            "撤销选择后仍保留历史供清理重试"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unmarked_legacy_directories_are_preserved_and_nested_marker_files_can_be_cleaned() {
        let parent = std::env::temp_dir().join(format!(
            "ssmt-gimi-cleanup-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let root = parent.join("legacy");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("user.ini"), b"preserve").unwrap();
        assert!(checked_cleanup_root(&root, &parent, "原神").is_err());
        assert_eq!(fs::read(root.join("user.ini")).unwrap(), b"preserve");
        fs::write(root.join(OWNER_FILE), owner("原神")).unwrap();
        fs::write(root.join("nested").join(OWNER_FILE), b"nested file").unwrap();
        assert!(checked_cleanup_root(&root, &parent, "另一个实例").is_err());
        cleanup_files(&checked_cleanup_root(&root, &parent, "原神").unwrap()).unwrap();
        assert!(!root.exists());
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn history_switch_is_atomic_and_cleanup_preserves_other_instances() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-hot-history-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let first = Installation {
            root: root.join("first"),
            route: "native-bridge".into(),
            profile: "rtx50".into(),
            revision: "v1".into(),
            baseline: Some(manifest().unwrap()),
        };
        let second = Installation {
            root: root.join("second"),
            profile: "rtx30".into(),
            ..first.clone()
        };
        let mut store = PluginSettingsStore::new(root.join("settings.json")).unwrap();
        save(
            &mut store,
            "WWMI",
            &first.route,
            Some(&first),
            &[first.clone(), second.clone()],
            true,
        )
        .unwrap();
        save(
            &mut store,
            "WWMI",
            &second.route,
            Some(&second),
            &[first.clone(), second.clone()],
            true,
        )
        .unwrap();
        assert_eq!(entries(&store, "WWMI").unwrap().len(), 2);
        assert_eq!(
            store
                .get(DLSS5_PLUGIN_ID, &setting_key("WWMI"))
                .unwrap()
                .unwrap()["profile"],
            "rtx30"
        );
        save(
            &mut store,
            "WWMI",
            &second.route,
            None,
            &[first.clone(), second.clone()],
            false,
        )
        .unwrap();
        let reopened = PluginSettingsStore::new(root.join("settings.json")).unwrap();
        assert!(reopened
            .get(DLSS5_PLUGIN_ID, &setting_key("WWMI"))
            .unwrap()
            .unwrap()
            .is_null());
        assert_eq!(entries(&reopened, "WWMI").unwrap().len(), 2);
        fs::create_dir(&first.root).unwrap();
        fs::write(first.root.join(OWNER_FILE), owner("WWMI")).unwrap();
        fs::write(first.root.join("test.dll"), b"owned").unwrap();
        assert!(checked_cleanup_root(&first.root, &root, "Another Game").is_err());
        assert!(checked_cleanup_root(&first.root, &root.join("missing"), "WWMI").is_err());
        let cleanup = checked_cleanup_root(&first.root, &root, "WWMI").unwrap();
        cleanup_files(&cleanup).unwrap();
        assert!(!first.root.exists());
        fs::create_dir(&first.root).unwrap();
        let cleanup = checked_cleanup_root(&first.root, &root, "WWMI").unwrap();
        cleanup_files(&cleanup).unwrap();
        assert!(!first.root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_baseline_is_retained_but_modified_components_are_rejected() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-hot-baseline-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("fixture.dll"), b"old").unwrap();
        let baseline = Manifest {
            revision: "previous".into(),
            assets: vec![Asset {
                url: "https://example.invalid/pinned".into(),
                sha256: format!("{:x}", Sha256::digest(b"old")),
                size: 3,
                route: String::new(),
                profile: String::new(),
                zip_offset: 0,
                zip_length: 0,
                files: vec![AssetFile {
                    entry: String::new(),
                    path: "fixture.dll".into(),
                    sha256: format!("{:x}", Sha256::digest(b"old")),
                    size: 3,
                }],
            }],
        };
        let installation = Installation {
            root: root.clone(),
            route: "native-bridge".into(),
            profile: "rtx50".into(),
            revision: "previous".into(),
            baseline: Some(baseline),
        };
        assert!(validate(&installation, &manifest().unwrap()).is_ok());
        fs::write(root.join("fixture.dll"), b"new").unwrap();
        assert!(validate(&installation, &manifest().unwrap()).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn failed_cleanup_keeps_owner_record_for_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!(
            "ssmt-hot-locked-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join(OWNER_FILE), owner("WWMI")).unwrap();
        fs::write(root.join("locked.dll"), b"locked").unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(root.join("locked.dll"))
            .unwrap();
        assert!(cleanup_files(&root).is_err());
        assert!(root.join(OWNER_FILE).is_file());
        drop(locked);
        cleanup_files(&root).unwrap();
        assert!(!root.exists());
    }
}
