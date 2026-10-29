use super::marketplace::{MarketplaceCatalog, MarketplaceEntry};
use super::package_installer::{inspect_ssmtpkg, install_verified_ssmtpkg};
use super::registry::{InstalledPlugin, PluginRegistry};
use super::resource_cache::{download_verified, MAX_ASSET_BYTES};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

const RELEASE_API: &str = "https://api.github.com/repos/Perxenic-Acid/SSMT-Native/releases/latest";
const CATALOG_NAME: &str = "ssmt-plugin-catalog.json";
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("SSMT4-PluginMarketplace")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())
}

fn official_asset_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("github.com")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed
            .path()
            .starts_with("/Perxenic-Acid/SSMT-Native/releases/download/")
}

fn asset_belongs_to_release(release: &Release, asset: &ReleaseAsset) -> bool {
    official_asset_url(&asset.browser_download_url)
        && asset.browser_download_url
            == format!(
                "https://github.com/Perxenic-Acid/SSMT-Native/releases/download/{}/{}",
                release.tag_name, asset.name
            )
}

async fn fetch_release(client: &reqwest::Client, api: &str) -> Result<Release, String> {
    let response = client
        .get(api)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Release {
            tag_name: String::new(),
            assets: Vec::new(),
        });
    }
    response
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_limited(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !official_asset_url(url) {
        return Err("release asset URL is not owned by SSMT-Native".into());
    }
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response.url().scheme() != "https" {
        return Err("official catalog redirected outside HTTPS".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err("release asset exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err("release asset exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn checked_catalog(release: &Release, bytes: &[u8]) -> Result<MarketplaceCatalog, String> {
    let mut document: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    // v0.1.6 的 PowerShell 打包器把未声明依赖写成 [null]；仅在官方目录兼容此空值，
    // 包内清单及依赖对象仍使用原有严格校验，不能借此跳过损坏的实际依赖。
    if let Some(entries) = document
        .get_mut("entries")
        .and_then(serde_json::Value::as_array_mut)
    {
        for entry in entries {
            if entry
                .get("externalDependencies")
                .is_some_and(|value| value == &serde_json::json!([null]))
            {
                entry["externalDependencies"] = serde_json::json!([]);
            }
        }
    }
    let catalog: MarketplaceCatalog =
        serde_json::from_value(document).map_err(|error| error.to_string())?;
    catalog.validate().map_err(|error| error.to_string())?;
    for entry in &catalog.entries {
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == format!("{}-{}.ssmtpkg", entry.id, entry.version))
            .ok_or_else(|| {
                format!(
                    "official package asset is missing: {} {}",
                    entry.id, entry.version
                )
            })?;
        if !asset_belongs_to_release(release, asset)
            || asset.browser_download_url != entry.download_url
            || asset.size != entry.package_size
            || asset.size == 0
            || asset.size > MAX_ASSET_BYTES
            || asset.digest.as_deref().is_some_and(|digest| {
                digest != format!("sha256:{}", entry.sha256.to_ascii_lowercase())
            })
        {
            return Err(format!(
                "official package metadata does not match release asset: {}",
                entry.id
            ));
        }
    }
    Ok(catalog)
}

fn verify_catalog_asset(
    release: &Release,
    asset: &ReleaseAsset,
    bytes: &[u8],
) -> Result<(), String> {
    if !asset_belongs_to_release(release, asset)
        || asset.size == 0
        || asset.size > MAX_CATALOG_BYTES
        || bytes.len() as u64 != asset.size
        || asset
            .digest
            .as_deref()
            .is_some_and(|digest| digest != format!("sha256:{:x}", Sha256::digest(bytes)))
    {
        return Err("official catalog does not match its Native release asset".into());
    }
    Ok(())
}

async fn catalog_from_release(
    client: &reqwest::Client,
    release: Release,
) -> Result<(MarketplaceCatalog, Release), String> {
    let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name == CATALOG_NAME)
    else {
        return Ok((
            MarketplaceCatalog {
                schema_version: 1,
                entries: Vec::new(),
            },
            release,
        ));
    };
    if !asset_belongs_to_release(&release, asset)
        || asset.size == 0
        || asset.size > MAX_CATALOG_BYTES
    {
        return Err("official catalog URL or size does not match its Native release".into());
    }
    let bytes = fetch_limited(client, &asset.browser_download_url, MAX_CATALOG_BYTES).await?;
    // 先对原始字节校验，再兼容目录格式，不能让兼容处理绕过发布摘要。
    verify_catalog_asset(&release, asset, &bytes)?;
    let catalog = checked_catalog(&release, &bytes)?;
    Ok((catalog, release))
}

pub async fn official_catalog() -> Result<Vec<MarketplaceEntry>, String> {
    let client = client()?;
    Ok(
        catalog_from_release(&client, fetch_release(&client, RELEASE_API).await?)
            .await?
            .0
            .entries,
    )
}

pub async fn install_official(
    registry: &mut PluginRegistry,
    id: &str,
    version: &str,
    ssmt_version: &str,
) -> Result<InstalledPlugin, String> {
    let client = client()?;
    let (catalog, _) =
        catalog_from_release(&client, fetch_release(&client, RELEASE_API).await?).await?;
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == id && entry.version == version)
        .ok_or("official plugin is not in the current Native release catalog")?;
    let cache_root = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssetCache");
    install_catalog_entry(&client, registry, entry, ssmt_version, &cache_root).await
}

async fn install_catalog_entry(
    client: &reqwest::Client,
    registry: &mut PluginRegistry,
    entry: &MarketplaceEntry,
    ssmt_version: &str,
    cache_root: &Path,
) -> Result<InstalledPlugin, String> {
    if !entry.supports(ssmt_version, super::SUPPORTED_PLATFORM, None) {
        return Err("official plugin is incompatible with this SSMT version or platform".into());
    }
    let package = download_verified(
        client,
        &entry.download_url,
        &entry.sha256,
        entry.package_size,
        cache_root,
    )
    .await?;
    let inspection = inspect_ssmtpkg(&package).map_err(|error| error.to_string())?;
    if inspection.manifest.id != entry.id || inspection.manifest.version != entry.version {
        return Err("official package manifest does not match release catalog".into());
    }
    if !inspection
        .manifest
        .compatibility
        .supports(ssmt_version, super::SUPPORTED_PLATFORM, None)
    {
        return Err(
            "official package manifest is incompatible with this SSMT version or platform".into(),
        );
    }
    let installed = install_verified_ssmtpkg(registry, &package, &entry.sha256)
        .map_err(|error| error.to_string())?;
    registry
        .mark_official(&installed.manifest.id, &installed.manifest.version)
        .map_err(|error| error.to_string())?;
    registry
        .installed()
        .iter()
        .find(|plugin| plugin.id() == entry.id && plugin.version() == entry.version)
        .cloned()
        .ok_or("official plugin disappeared after installation".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_assets_outside_native_release() {
        assert!(official_asset_url(
            "https://github.com/Perxenic-Acid/SSMT-Native/releases/download/v1/a.ssmtpkg"
        ));
        assert!(!official_asset_url("https://example.com/a.ssmtpkg"));
        assert!(!official_asset_url(
            "https://github.com/Perxenic-Acid/SSMT-Native.evil/releases/download/v1/a.ssmtpkg"
        ));
    }

    #[test]
    fn catalog_must_match_the_same_release_asset() {
        let url = "https://github.com/Perxenic-Acid/SSMT-Native/releases/download/v1/ssmt.example-1.0.0.ssmtpkg";
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "entries": [{
                "id":"ssmt.example", "name":"Example", "description":"Example",
                "author":"SSMT", "version":"1.0.0", "downloadUrl":url,
                "sha256":"a".repeat(64), "minimumSsmtVersion":">=4.0.0",
                "supportedPlatforms":["windows-x64"], "supportedGames":[],
                "packageSize":12, "permissions":[], "externalDependencies":[]
            }]
        }))
        .unwrap();
        let mut release = Release {
            tag_name: "v1".into(),
            assets: vec![ReleaseAsset {
                name: "ssmt.example-1.0.0.ssmtpkg".into(),
                browser_download_url: url.into(),
                size: 12,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
            }],
        };
        assert!(checked_catalog(&release, &bytes).is_ok());
        release.assets[0].size = 13;
        assert!(checked_catalog(&release, &bytes).is_err());
        release.assets[0].size = 12;
        release.tag_name = "v2".into();
        assert!(
            checked_catalog(&release, &bytes).is_err(),
            "同仓库的其他 Release 也不能混用"
        );
    }

    #[test]
    fn published_powershell_empty_dependencies_do_not_hide_the_whole_catalog() {
        let url = "https://github.com/Perxenic-Acid/SSMT-Native/releases/download/v0.1.6/ssmt.example-1.0.0.ssmtpkg";
        let release = Release {
            tag_name: "v0.1.6".into(),
            assets: vec![ReleaseAsset {
                name: "ssmt.example-1.0.0.ssmtpkg".into(),
                browser_download_url: url.into(),
                size: 12,
                digest: None,
            }],
        };
        let mut document = serde_json::json!({
            "schemaVersion": 1, "entries": [{
                "id":"ssmt.example", "name":"Example", "description":"Example", "author":"SSMT", "version":"1.0.0",
                "downloadUrl":url, "sha256":"a".repeat(64), "minimumSsmtVersion":">=4.0.0",
                "supportedPlatforms":["windows-x64"], "packageSize":12, "externalDependencies":[null]
            }]
        });
        let catalog = checked_catalog(&release, &serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(catalog.entries[0].external_dependencies.is_empty());
        // 仅兼容旧 PowerShell 的空数组表示，不能吞掉损坏的依赖对象或其他未知字段。
        for dependencies in [
            serde_json::json!([null, {"id":"broken"}]),
            serde_json::json!([{}]),
            serde_json::json!({}),
        ] {
            document["entries"][0]["externalDependencies"] = dependencies;
            assert!(checked_catalog(&release, &serde_json::to_vec(&document).unwrap()).is_err());
        }
    }

    #[test]
    fn catalog_bytes_are_verified_before_legacy_empty_array_compatibility() {
        let bytes = b"{\"schemaVersion\":1,\"entries\":[]}";
        let release = Release {
            tag_name: "v1".into(),
            assets: Vec::new(),
        };
        let mut asset = ReleaseAsset {
            name: CATALOG_NAME.into(),
            browser_download_url: format!(
                "https://github.com/Perxenic-Acid/SSMT-Native/releases/download/v1/{CATALOG_NAME}"
            ),
            size: bytes.len() as u64,
            digest: Some(format!("sha256:{:x}", Sha256::digest(bytes))),
        };
        verify_catalog_asset(&release, &asset, bytes).unwrap();
        let mut changed = bytes.to_vec();
        changed[2] = b'X';
        assert!(verify_catalog_asset(&release, &asset, &changed).is_err());
        asset.size += 1;
        assert!(verify_catalog_asset(&release, &asset, bytes).is_err());
        asset.size -= 1;
        asset.browser_download_url = asset.browser_download_url.replace("/v1/", "/v2/");
        assert!(verify_catalog_asset(&release, &asset, bytes).is_err());
    }

    #[tokio::test]
    #[ignore = "设置 SSMT_NATIVE_RELEASE_TEST_TAG，联网下载正式插件并在临时目录安装；不加载 DLL、不读取用户配置"]
    async fn published_release_downloads_and_installs_every_package_without_enabling_plugins() {
        let tag = std::env::var("SSMT_NATIVE_RELEASE_TEST_TAG").expect("缺少正式 Release 标签");
        assert!(
            !tag.is_empty()
                && tag
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        );
        let client = client().unwrap();
        let api =
            format!("https://api.github.com/repos/Perxenic-Acid/SSMT-Native/releases/tags/{tag}");
        let release = fetch_release(&client, &api).await.unwrap();
        assert_eq!(release.tag_name, tag);
        let (catalog, _) = catalog_from_release(&client, release).await.unwrap();
        assert!(!catalog.entries.is_empty());
        let temp_root = std::env::temp_dir().canonicalize().unwrap();
        let root = temp_root.join(format!(
            "ssmt-release-install-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut registry = PluginRegistry::new(root.join("plugins")).unwrap();
        let version =
            serde_json::from_str::<serde_json::Value>(include_str!("../../tauri.conf.json"))
                .unwrap()["version"]
                .as_str()
                .unwrap()
                .to_owned();
        // 倒序安装也必须返回请求的版本，不能把已安装的新版本误当成本次安装结果。
        for entry in catalog.entries.iter().rev() {
            let installed =
                install_catalog_entry(&client, &mut registry, entry, &version, &root.join("cache"))
                    .await
                    .unwrap();
            assert_eq!(installed.id(), entry.id);
            assert_eq!(installed.version(), entry.version);
            assert!(installed.official && !installed.enabled);
            println!("已校验并隔离安装 {} {}", entry.id, entry.version);
        }
        assert_eq!(registry.installed().len(), catalog.entries.len());
        let restarted = PluginRegistry::new(root.join("plugins")).unwrap();
        assert!(restarted
            .installed()
            .iter()
            .all(|plugin| plugin.official && !plugin.enabled));
        for preset in ["GIMI", "SRMI", "WWMI", "ZZMI", "ZZMIDX12", "EFMI"] {
            assert!(restarted
                .for_game_preset(preset, preset)
                .generate_plugin_host_config()
                .plugins
                .is_empty());
        }
        assert_eq!(
            root.canonicalize().unwrap().parent(),
            Some(temp_root.as_path())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
