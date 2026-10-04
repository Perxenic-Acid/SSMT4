use super::marketplace::{MarketplaceCatalog, MarketplaceEntry};
use super::package_installer::{inspect_ssmtpkg, install_verified_ssmtpkg};
use super::registry::{InstalledPlugin, PluginRegistry};
use super::resource_cache::{download_verified, MAX_ASSET_BYTES};
use serde::Deserialize;

const RELEASE_API: &str =
    "https://api.github.com/repos/Perxenic-Acid/SSMT-Native/releases/latest";
const CATALOG_NAME: &str = "ssmt-plugin-catalog.json";
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
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
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("github.com")
        && parsed.path().starts_with("/Perxenic-Acid/SSMT-Native/releases/download/")
}

async fn latest_release(client: &reqwest::Client) -> Result<Release, String> {
    let response = client.get(RELEASE_API).send().await.map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Release { assets: Vec::new() });
    }
    response.error_for_status().map_err(|error| error.to_string())?
        .json().await.map_err(|error| error.to_string())
}

async fn fetch_limited(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !official_asset_url(url) { return Err("release asset URL is not owned by SSMT-Native".into()) }
    let mut response = client.get(url).send().await.map_err(|error| error.to_string())?
        .error_for_status().map_err(|error| error.to_string())?;
    if response.content_length().is_some_and(|length| length > limit) {
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
    let catalog = MarketplaceCatalog::from_json_str(
        std::str::from_utf8(bytes).map_err(|error| error.to_string())?
    ).map_err(|error| error.to_string())?;
    for entry in &catalog.entries {
        let asset = release.assets.iter().find(|asset| asset.name == format!("{}-{}.ssmtpkg", entry.id, entry.version))
            .ok_or_else(|| format!("official package asset is missing: {} {}", entry.id, entry.version))?;
        if !official_asset_url(&asset.browser_download_url)
            || asset.browser_download_url != entry.download_url
            || asset.size != entry.package_size
            || asset.size > MAX_ASSET_BYTES
            || asset.digest.as_deref().is_some_and(|digest| digest != format!("sha256:{}", entry.sha256.to_ascii_lowercase()))
        {
            return Err(format!("official package metadata does not match release asset: {}", entry.id));
        }
    }
    Ok(catalog)
}

async fn catalog_and_release(client: &reqwest::Client) -> Result<(MarketplaceCatalog, Release), String> {
    let release = latest_release(client).await?;
    let Some(asset) = release.assets.iter().find(|asset| asset.name == CATALOG_NAME) else {
        return Ok((MarketplaceCatalog { schema_version: 1, entries: Vec::new() }, release));
    };
    if asset.size > MAX_CATALOG_BYTES { return Err("official catalog exceeds 1 MiB".into()) }
    let bytes = fetch_limited(client, &asset.browser_download_url, MAX_CATALOG_BYTES).await?;
    if bytes.len() as u64 != asset.size { return Err("official catalog size mismatch".into()) }
    let catalog = checked_catalog(&release, &bytes)?;
    Ok((catalog, release))
}

pub async fn official_catalog() -> Result<Vec<MarketplaceEntry>, String> {
    let client = client()?;
    Ok(catalog_and_release(&client).await?.0.entries)
}

pub async fn install_official(
    registry: &mut PluginRegistry,
    id: &str,
    version: &str,
    ssmt_version: &str,
) -> Result<InstalledPlugin, String> {
    let client = client()?;
    let (catalog, _) = catalog_and_release(&client).await?;
    let entry = catalog.entries.iter().find(|entry| entry.id == id && entry.version == version)
        .ok_or("official plugin is not in the current Native release catalog")?;
    if !entry.supports(ssmt_version, super::SUPPORTED_PLATFORM, None) {
        return Err("official plugin is incompatible with this SSMT version or platform".into());
    }
    let cache_root = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join("PluginAssetCache");
    let package = download_verified(&client, &entry.download_url, &entry.sha256,
        entry.package_size, &cache_root).await?;
    let inspection = inspect_ssmtpkg(&package).map_err(|error| error.to_string())?;
    if inspection.manifest.id != entry.id || inspection.manifest.version != entry.version {
        return Err("official package manifest does not match release catalog".into());
    }
    if !inspection.manifest.compatibility.supports(ssmt_version, super::SUPPORTED_PLATFORM, None) {
        return Err("official package manifest is incompatible with this SSMT version or platform".into());
    }
    let installed = install_verified_ssmtpkg(registry, &package, &entry.sha256)
        .map_err(|error| error.to_string())?;
    registry.mark_official(&installed.manifest.id, &installed.manifest.version)
        .map_err(|error| error.to_string())?;
    registry.find(id).cloned().ok_or("official plugin disappeared after installation".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_assets_outside_native_release() {
        assert!(official_asset_url("https://github.com/Perxenic-Acid/SSMT-Native/releases/download/v1/a.ssmtpkg"));
        assert!(!official_asset_url("https://example.com/a.ssmtpkg"));
        assert!(!official_asset_url("https://github.com/Perxenic-Acid/SSMT-Native.evil/releases/download/v1/a.ssmtpkg"));
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
        })).unwrap();
        let mut release = Release { assets: vec![ReleaseAsset {
            name: "ssmt.example-1.0.0.ssmtpkg".into(),
            browser_download_url: url.into(),
            size: 12,
            digest: Some(format!("sha256:{}", "a".repeat(64))),
        }] };
        assert!(checked_catalog(&release, &bytes).is_ok());
        release.assets[0].size = 13;
        assert!(checked_catalog(&release, &bytes).is_err());
    }
}
