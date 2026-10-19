use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ssmt-runtime-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn release() -> Release {
    Release {
        tag_name: "rolling".into(),
        draft: false,
        assets: vec![
            Asset {
                id: 1,
                name: ARCHIVE.into(),
                size: 100,
                updated_at: "today".into(),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/rolling/{ARCHIVE}"
                ),
                digest: None,
            },
            Asset {
                id: 2,
                name: CHECKSUMS.into(),
                size: 100,
                updated_at: "today".into(),
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/rolling/{CHECKSUMS}"
                ),
                digest: None,
            },
        ],
    }
}

fn zip_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        writer
            .start_file(*name, zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

// 系统 DLL 只用作 PE/版本资源夹具，不加载，不调用图形 API，也不修改系统文件。
#[cfg(windows)]
fn dll(version: [u16; 4]) -> Vec<u8> {
    let mut bytes = fs::read(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/version.dll"),
    )
    .unwrap();
    let offset = bytes
        .windows(8)
        .position(|data| data == b"\xbd\x04\xef\xfe\x00\x00\x01\x00")
        .unwrap();
    bytes[offset + 8..offset + 12]
        .copy_from_slice(&((version[0] as u32) << 16 | version[1] as u32).to_le_bytes());
    bytes[offset + 12..offset + 16]
        .copy_from_slice(&((version[2] as u32) << 16 | version[3] as u32).to_le_bytes());
    bytes
}

fn receipt(dll: &[u8], asset_id: u64) -> Receipt {
    Receipt {
        asset_id,
        asset_updated_at: format!("update-{asset_id}"),
        archive_sha256: "a".repeat(64),
        dll_sha256: hash(dll),
    }
}

#[test]
fn accepts_only_unique_bounded_assets_from_our_runtime_release() {
    let mut value = release();
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_ok());
    value.assets[0].browser_download_url = format!(
        "https://github.com/SpectrumQT/XXMI-Libs-Package/releases/download/rolling/{ARCHIVE}"
    );
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
    value = release();
    value.assets[0].size = MAX_ARCHIVE + 1;
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
    value = release();
    value.tag_name = "different-tag".into();
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
    value = release();
    value.draft = true;
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
    value = release();
    value.assets.remove(0);
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
    value = release();
    value.assets[1].name = ARCHIVE.into();
    assert!(asset(&value, ARCHIVE, MAX_ARCHIVE).is_err());
}

#[test]
fn requires_exact_archive_checksum_and_rejects_ambiguous_records() {
    let record = format!(
        "{}  {ARCHIVE}\r\n{}  SSMT-Runtime-x64-symbols.zip\r\n",
        "A".repeat(64),
        "b".repeat(64)
    );
    assert_eq!(checksum(record.as_bytes()).unwrap(), "a".repeat(64));
    assert!(checksum(format!("{record}{record}").as_bytes()).is_err());
    assert!(checksum(b"abc SSMT-Runtime-x64.zip").is_err());
    assert!(checksum(format!("{}  ../{ARCHIVE}", "a".repeat(64)).as_bytes()).is_err());
}

#[test]
fn rejects_traversal_extra_files_and_non_dll_archives() {
    assert!(extract_dll(&zip_files(&[("../d3d11.dll", b"test")])).is_err());
    assert!(extract_dll(&zip_files(&[("d3d11.dll", b"test"), ("Run.exe", b"test")])).is_err());
    assert!(extract_dll(&zip_files(&[("d3d11.dll", b"not a PE file")])).is_err());
}

#[test]
fn failed_staging_validation_preserves_existing_file_and_cleans_up() {
    let fixture = Fixture::new();
    let target = fixture.0.join("current.json");
    fs::write(&target, "previous good receipt").unwrap();
    assert!(atomic_write(&target, b"broken update", |_| Err(
        "validation failed".into()
    ))
    .is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "previous good receipt");
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn archive_must_match_both_size_and_sha256_before_using_valid_x64_dll() {
    let dll = dll([1, 2, 2, 24]);
    let bytes = zip_files(&[("d3d11.dll", &dll)]);
    let mut release = release();
    release.assets[0].size = bytes.len() as u64;
    assert_eq!(
        checked_dll(&release.assets[0], &hash(&bytes), &bytes).unwrap(),
        dll
    );
    assert!(checked_dll(&release.assets[0], &"0".repeat(64), &bytes).is_err());
    release.assets[0].size += 1;
    assert!(checked_dll(&release.assets[0], &hash(&bytes), &bytes).is_err());
}

#[cfg(windows)]
#[test]
fn old_bundle_never_overwrites_newer_deployed_version() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundled.dll");
    let target = fixture.0.join("target.dll");
    let cache = fixture.0.join("cache");
    fs::write(&bundle, dll([1, 2, 2, 23])).unwrap();
    fs::write(&target, dll([1, 2, 2, 24])).unwrap();
    let result = choose_source(&bundle, &target, &cache, Some("offline".into())).unwrap();
    assert_eq!(result.origin, "existing");
    assert_eq!(result.source_path, target.to_string_lossy());
    assert_eq!(result.version, "1.2.2.24");
    assert_eq!(result.warning.as_deref(), Some("offline"));
    fs::write(&bundle, dll([1, 2, 2, 25])).unwrap();
    assert_eq!(
        choose_source(&bundle, &target, &cache, None)
            .unwrap()
            .origin,
        "bundled"
    );
}

#[cfg(windows)]
#[test]
fn verified_github_cache_wins_old_bundle_and_republished_rolling_cannot_downgrade_it() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundled.dll");
    let target = fixture.0.join("target.dll");
    let cache = fixture.0.join("cache");
    fs::write(&bundle, dll([1, 2, 2, 23])).unwrap();
    let current = dll([1, 2, 2, 24]);
    publish_cache(&cache, &receipt(&current, 1), &current).unwrap();
    assert_eq!(
        choose_source(&bundle, &target, &cache, None)
            .unwrap()
            .origin,
        "github"
    );
    let old = dll([1, 2, 2, 20]);
    publish_cache(&cache, &receipt(&old, 2), &old).unwrap();
    assert_eq!(cached_runtime(&cache).unwrap().0.asset_id, 1);
    let newer = dll([1, 2, 2, 25]);
    publish_cache(&cache, &receipt(&newer, 3), &newer).unwrap();
    assert_eq!(
        choose_source(&bundle, &target, &cache, None)
            .unwrap()
            .version,
        "1.2.2.25"
    );
    fs::write(cached_runtime(&cache).unwrap().1, "corrupted cache").unwrap();
    assert_eq!(
        choose_source(&bundle, &target, &cache, None)
            .unwrap()
            .origin,
        "bundled"
    );
}

#[tokio::test]
#[cfg(windows)]
#[ignore = "访问真实 GitHub，只在显式运行时验证发布链；不加载 DLL 或启动游戏"]
async fn published_rolling_is_verified_or_falls_back_without_replacing_local_runtime() {
    let fixture = Fixture::new();
    let cache = fixture.0.join("cache");
    let bundle = fixture.0.join("bundled.dll");
    let target = fixture.0.join("target.dll");
    let local = dll([1, 2, 2, 24]);
    fs::write(&bundle, &local).unwrap();
    fs::write(&target, &local).unwrap();
    let warning = tokio::time::timeout(UPDATE_TIMEOUT, refresh_cache(&cache))
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result)
        .err();
    println!("GitHub refresh warning: {warning:?}");
    let source = choose_source(&bundle, &target, &cache, warning.clone()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), local);
    if warning.is_some() {
        assert_eq!(source.origin, "bundled");
        assert!(source.warning.is_some());
    } else {
        let (_, path) = cached_runtime(&cache).unwrap();
        println!(
            "GitHub Runtime verified: {} {:?}",
            path.display(),
            file_version(&path).unwrap()
        );
    }
}

#[cfg(windows)]
#[test]
fn zero_version_publish_is_rejected_without_damaging_previous_receipt() {
    let fixture = Fixture::new();
    let good = dll([1, 2, 2, 24]);
    publish_cache(&fixture.0, &receipt(&good, 1), &good).unwrap();
    let zero = dll([0, 0, 0, 0]);
    assert!(publish_cache(&fixture.0, &receipt(&zero, 2), &zero).is_err());
    assert_eq!(cached_runtime(&fixture.0).unwrap().0.asset_id, 1);
}

#[test]
fn core_package_cannot_replace_runtime_dlls_and_other_archive_users_keep_default_behavior() {
    use crate::utils::ssmt_compress_utils::SSMTCompressUtils;
    let fixture = Fixture::new();
    let target = fixture.0.join("game");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("d3d11.dll"), "custom runtime").unwrap();
    fs::write(target.join("d3d12.dll"), "custom dx12 runtime").unwrap();
    let archive = fixture.0.join("core.zip");
    fs::write(
        &archive,
        zip_files(&[
            ("GIMI/D3D11.DLL", b"old upstream runtime"),
            ("GIMI/d3d12.dll", b"old dx12 runtime"),
            ("GIMI/d3dx.ini", b"updated core config"),
        ]),
    )
    .unwrap();
    let mut completed = (0, 0);
    SSMTCompressUtils::extract_zip_archive_excluding(
        &archive,
        &target,
        &["d3d11.dll", "d3d12.dll"],
        |processed, total| completed = (processed, total),
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("d3d11.dll")).unwrap(),
        b"custom runtime"
    );
    assert_eq!(
        fs::read(target.join("d3d12.dll")).unwrap(),
        b"custom dx12 runtime"
    );
    assert_eq!(
        fs::read(target.join("d3dx.ini")).unwrap(),
        b"updated core config"
    );
    assert_eq!(completed.0, completed.1);
    let ordinary = fixture.0.join("ordinary");
    SSMTCompressUtils::extract_zip_archive(&archive, &ordinary, |_, _| {}).unwrap();
    assert_eq!(
        fs::read(ordinary.join("D3D11.DLL")).unwrap(),
        b"old upstream runtime"
    );
}
