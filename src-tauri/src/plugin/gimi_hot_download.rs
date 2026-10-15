use super::*;
use crate::plugin::resource_cache::download_verified_with_progress;
use std::io::Write;
use tauri::ipc::Channel;

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    revision: String,
    assets: Vec<Asset>,
    nr_archive: Archive,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    path: String,
    source: String,
    sha256: String,
    size: u64,
    profile: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    url: String,
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
    serde_json::from_str(include_str!("gimi_hot_assets.json")).map_err(|error| error.to_string())
}

// 临时目录只能是本次创建的自有目录；失败时不清理既有安装或下载缓存。
struct PendingInstallation(PathBuf, bool);
impl Drop for PendingInstallation {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        if let (Ok(root), Some(parent)) = (self.0.canonicalize(), self.0.parent()) {
            if let Ok(parent) = parent.canonicalize() {
                if root.parent() == Some(parent.as_path()) {
                    let _ = fs::remove_dir_all(root);
                }
            }
        }
    }
}

fn require_enabled(game_name: &str, executable: &Path) -> Result<(), String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?
        .for_game(game_name);
    if !enabled(&registry) || !applies(executable) {
        return Err("请先为原神启用 DLSS5 插件".into());
    }
    ensure_stopped(executable)
}

// 只读取固定包中所选 Profile 的这一项，绝不执行包内脚本或按其路径解压。
fn extract_nr(archive: &Path, root: &Path, profile: &str) -> Result<(), String> {
    let (addon, files) = profile_files(profile)?;
    let (relative, hash) = files.last().ok_or("NR 清单为空")?;
    let directory = addon.rsplit('/').next().ok_or("NR 目录无效")?;
    let expected_name = format!(
        "DLSS5_GI_Ready/payload/ReShade/reshade-shaders/Addons/{directory}/nvngx_dlssnr.dll"
    );
    let output = root.join(relative);
    fs::create_dir_all(output.parent().ok_or("NR 路径无效")?).map_err(|error| error.to_string())?;
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut reader =
        sevenz_rust::SevenZReader::new(file, size, sevenz_rust::Password::from("yuanshenqidong"))
            .map_err(|error| error.to_string())?;
    let mut found = false;
    reader
        .for_each_entries(|entry, stream| {
            if entry.name().replace('\\', "/") != expected_name {
                // 7z 固实压缩流必须读完跳过的条目，否则后续条目会从错误偏移解码。
                std::io::copy(stream, &mut std::io::sink())?;
                return Ok(true);
            }
            if found || entry.is_directory() || entry.size() > 256 * 1024 * 1024 {
                return Err(sevenz_rust::Error::other("NR 归档项无效或重复"));
            }
            let mut target = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            let bytes = std::io::copy(&mut stream.take(256 * 1024 * 1024 + 1), &mut target)?;
            if bytes != entry.size() || bytes > 256 * 1024 * 1024 {
                return Err(sevenz_rust::Error::other("NR 大小无效"));
            }
            target.flush()?;
            found = true;
            Ok(true)
        })
        .map_err(|error| error.to_string())?;
    if !found {
        return Err("下载包中未找到配套 NR Runtime".into());
    }
    verify(&output, hash)
}

async fn download_at(
    client: &reqwest::Client,
    cache: &Path,
    root: PathBuf,
    profile: String,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<HotRuntimeInstallation, String> {
    profile_files(&profile)?;
    let manifest = manifest()?;
    let assets = manifest
        .assets
        .iter()
        .filter(|asset| {
            asset
                .profile
                .as_deref()
                .is_none_or(|value| value == profile)
        })
        .collect::<Vec<_>>();
    let total_bytes = assets.iter().map(|asset| asset.size).sum::<u64>() + manifest.nr_archive.size;
    let mut completed = 0;
    for asset in assets {
        let url = format!(
            "https://raw.githubusercontent.com/CXP-2024/GIMI_reshade_dlss_integration/{}/{}",
            manifest.revision, asset.source,
        );
        let mut notify = |downloaded_bytes| {
            progress(DownloadProgress {
                phase: "downloading",
                file: asset.path.clone(),
                downloaded_bytes: completed + downloaded_bytes,
                total_bytes,
            })
        };
        notify(0);
        let cached =
            download_verified_with_progress(client, &url, &asset.sha256, asset.size, cache, notify)
                .await
                .map_err(|error| format!("下载 {} 失败：{error}", asset.path))?;
        let target = root.join(&asset.path);
        fs::create_dir_all(target.parent().ok_or("组件目录无效")?)
            .map_err(|error| error.to_string())?;
        fs::copy(cached, target).map_err(|error| error.to_string())?;
        completed += asset.size;
    }
    let archive = &manifest.nr_archive;
    let cached = download_verified_with_progress(
        client,
        &archive.url,
        &archive.sha256,
        archive.size,
        cache,
        |downloaded_bytes| {
            progress(DownloadProgress {
                phase: "downloading",
                file: "NR Runtime".into(),
                downloaded_bytes: completed + downloaded_bytes,
                total_bytes,
            })
        },
    )
    .await
    .map_err(|error| format!("下载 NR Runtime 失败：{error}"))?;
    progress(DownloadProgress {
        phase: "extracting",
        file: "NR Runtime".into(),
        downloaded_bytes: total_bytes,
        total_bytes,
    });
    // 固实压缩包的解码和最终校验放在阻塞线程，不占用异步调度线程。
    let installation = HotRuntimeInstallation { root, profile };
    let candidate = installation.clone();
    tokio::task::spawn_blocking(move || {
        extract_nr(&cached, &candidate.root, &candidate.profile)?;
        validate(&candidate).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())??;
    progress(DownloadProgress {
        phase: "ready",
        file: String::new(),
        downloaded_bytes: total_bytes,
        total_bytes,
    });
    Ok(installation)
}

#[tauri::command]
pub async fn download_gimi_hot_runtime(
    game_name: String,
    game_executable: String,
    profile: String,
    on_progress: Channel<DownloadProgress>,
) -> Result<HotRuntimeInstallation, String> {
    let _lock = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "DLSS5 组件正在下载，请等待当前任务完成")?;
    profile_files(&profile)?;
    require_enabled(&game_name, Path::new(&game_executable))?;
    let client = reqwest::Client::builder()
        .user_agent("SSMT4-DLSS5-Setup")
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|error| error.to_string())?;
    let cache = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssetCache");
    let root = new_installation_root();
    fs::create_dir_all(root.parent().ok_or("组件目录无效")?).map_err(|error| error.to_string())?;
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let mut pending = PendingInstallation(root.clone(), false);
    let mut last_update = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let installation = download_at(&client, &cache, root, profile, |event| {
        // 用户切换页面不应让已授权下载变成失败；重返页面可读取最终状态。
        if event.phase != "downloading" || last_update.elapsed().as_millis() >= 150 {
            let _ = on_progress.send(event);
            last_update = std::time::Instant::now();
        }
    })
    .await?;
    // 网络操作期间游戏可能已启动，或用户已关闭插件；此时保留旧配置。
    require_enabled(&game_name, Path::new(&game_executable))?;
    save_installation(&game_name, &installation)?;
    pending.1 = true;
    Ok(installation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_manifest_matches_all_supported_profiles() {
        let manifest = manifest().unwrap();
        assert_eq!(manifest.revision.len(), 40);
        for profile in ["rtx50", "rtx30"] {
            let (_, files) = profile_files(profile).unwrap();
            for (path, hash) in files.iter().take(files.len() - 1) {
                let asset = manifest
                    .assets
                    .iter()
                    .find(|asset| &asset.path == path)
                    .unwrap();
                assert_eq!(&asset.sha256, hash);
                assert!(asset
                    .profile
                    .as_deref()
                    .is_none_or(|value| value == profile));
            }
        }
        for asset in manifest.assets {
            for path in [&asset.path, &asset.source] {
                assert!(
                    !path.contains("..")
                        && !path.contains('\\')
                        && !path.contains(':')
                        && !path.starts_with('/')
                );
                assert!(!path.starts_with("GIMI/") && !path.ends_with(".exe"));
            }
        }
    }

    #[tokio::test]
    #[ignore = "联网下载约 302 MiB，设置 SSMT_HOT_DOWNLOAD_TEST_ROOT 指定隔离缓存"]
    async fn downloads_and_prepares_both_real_profiles() {
        let root =
            PathBuf::from(std::env::var_os("SSMT_HOT_DOWNLOAD_TEST_ROOT").expect("缺少测试目录"));
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(std::time::Duration::from_secs(900))
            .build()
            .unwrap();
        let run = root.join(format!(
            "verified-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&run).unwrap();
        let _pending = PendingInstallation(run.clone(), false);
        for profile in ["rtx50", "rtx30"] {
            let installation = download_at(
                &client,
                &root.join("cache"),
                run.join(profile),
                profile.into(),
                |_| {},
            )
            .await
            .unwrap();
            let registry = super::super::tests::registry_with_bridge(
                &run.join(profile),
                &super::super::tests::real_bridge(),
            );
            let plan = prepare_at(&registry, &installation).unwrap();
            assert_eq!(plan.dlls.len(), 3);
            assert!(!installation.root.join("GIMI").exists());
            assert!(!installation.root.join("dxgi.dll").exists());
            if profile == "rtx50" {
                // 下一轮须自动修复损坏缓存；已安装的第一套副本不受影响。
                fs::write(
                    root.join("cache").join(format!("{}.asset", COMMON[0].1)),
                    b"damaged",
                )
                .unwrap();
            }
        }
        validate(&HotRuntimeInstallation {
            root: run.join("rtx50"),
            profile: "rtx50".into(),
        })
        .unwrap();
    }
}
