use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sysinfo::Disks;

use crate::workspace_access::archive::{validate_portable_metadata, validate_workspace_archive};
use crate::workspace_access::models::PortableWorkspaceMetadataV1;

const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 1000;
pub const RECOMMENDED_DOWNLOAD_FREE_BYTES: u64 = 1024 * 1024 * 1024;

pub fn workspace_access_http_client(
    proxy_port: Option<u16>,
    timeout: Option<Duration>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(port) = proxy_port.filter(|port| *port != 0) {
        let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .map_err(|_| "WORKSPACE_PROXY_INVALID")?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|_| "WORKSPACE_HTTP_CLIENT_INIT_FAILED".to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub workspace_path: String,
    pub workspace_name: String,
    pub file_count: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceReport {
    pub available_bytes: Option<u64>,
    pub recommended_free_bytes: u64,
    pub below_recommended: bool,
}

pub fn check_disk_space(path: &Path) -> DiskSpaceReport {
    let existing_path = existing_ancestor(path).unwrap_or_else(|| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    let available_bytes = disks
        .list()
        .iter()
        .filter(|disk| existing_path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space());
    DiskSpaceReport {
        below_recommended: available_bytes
            .is_some_and(|value| value < RECOMMENDED_DOWNLOAD_FREE_BYTES),
        available_bytes,
        recommended_free_bytes: RECOMMENDED_DOWNLOAD_FREE_BYTES,
    }
}

pub fn temporary_archive_path(label: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "TEMPORARY_ARCHIVE_PATH_FAILED")?
        .as_nanos();
    let directory = std::env::temp_dir().join("SSMT").join("WorkspaceAccess");
    fs::create_dir_all(&directory).map_err(|_| "TEMPORARY_ARCHIVE_PATH_FAILED")?;
    let label: String = label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(32)
        .collect();
    Ok(directory.join(format!(
        "{}-{}-{stamp}.ssmtws",
        if label.is_empty() {
            "workspace"
        } else {
            &label
        },
        std::process::id()
    )))
}

pub async fn download_archive(
    url: &str,
    destination: &Path,
    expected_sha256: &str,
    proxy_port: Option<u16>,
) -> Result<DownloadResult, String> {
    if destination.extension().and_then(|value| value.to_str()) != Some("ssmtws") {
        return Err("DOWNLOAD_OUTPUT_EXTENSION_INVALID".to_string());
    }
    let expected = expected_sha256.trim().to_ascii_lowercase();
    if !is_sha256(&expected) {
        return Err("DOWNLOAD_SHA256_INVALID".to_string());
    }
    if destination.exists() {
        return Err("DOWNLOAD_OUTPUT_ALREADY_EXISTS".to_string());
    }
    let response = workspace_access_http_client(proxy_port, None)?
        .get(url)
        .send()
        .await
        .map_err(|_| "DOWNLOAD_REQUEST_FAILED")?;
    if !response.status().is_success() {
        return Err(format!("DOWNLOAD_HTTP_{}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE_BYTES)
    {
        return Err("DOWNLOAD_ARCHIVE_TOO_LARGE".to_string());
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| "DOWNLOAD_DIRECTORY_FAILED")?;
    let part = destination.with_extension("ssmtws.part");
    let mut file = File::create(&part).map_err(|_| "DOWNLOAD_OUTPUT_FAILED")?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "DOWNLOAD_STREAM_FAILED")?;
        size = size.saturating_add(chunk.len() as u64);
        if size > MAX_ARCHIVE_BYTES {
            let _ = fs::remove_file(&part);
            return Err("DOWNLOAD_ARCHIVE_TOO_LARGE".to_string());
        }
        digest.update(&chunk);
        file.write_all(&chunk)
            .map_err(|_| "DOWNLOAD_OUTPUT_FAILED")?;
    }
    file.flush().map_err(|_| "DOWNLOAD_OUTPUT_FAILED")?;
    let actual = format!("{:x}", digest.finalize());
    if actual != expected {
        let _ = fs::remove_file(&part);
        return Err("DOWNLOAD_SHA256_MISMATCH".to_string());
    }
    fs::rename(&part, destination).map_err(|_| "DOWNLOAD_RENAME_FAILED")?;
    Ok(DownloadResult {
        path: destination.to_string_lossy().to_string(),
        size,
        sha256: actual,
    })
}

pub fn import_archive(
    archive_path: &Path,
    workspace_base: &Path,
    requested_name: &str,
    game_preset: &str,
) -> Result<ImportResult, String> {
    let validation = validate_workspace_archive(archive_path);
    if !validation.valid {
        return Err(validation
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "ARCHIVE_INVALID".to_string()));
    }
    if validation.total_uncompressed_size > MAX_UNCOMPRESSED_BYTES {
        return Err("ARCHIVE_UNCOMPRESSED_TOO_LARGE".to_string());
    }
    let file = File::open(archive_path).map_err(|_| "ARCHIVE_NOT_FOUND")?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "ARCHIVE_NOT_ZIP")?;
    let metadata: PortableWorkspaceMetadataV1 = {
        let entry = archive
            .by_index(1)
            .map_err(|_| "ARCHIVE_METADATA_MISSING")?;
        serde_json::from_reader(entry).map_err(|_| "ARCHIVE_METADATA_INVALID")?
    };
    if !validate_portable_metadata(&metadata) || metadata.game_preset != game_preset.trim() {
        return Err("ARCHIVE_METADATA_UNSUPPORTED".to_string());
    }
    let base_name = requested_name.trim();
    if !is_safe_name(base_name) {
        return Err("IMPORT_WORKSPACE_NAME_INVALID".to_string());
    }
    fs::create_dir_all(workspace_base).map_err(|_| "IMPORT_DIRECTORY_FAILED")?;
    let final_name = unique_workspace_name(workspace_base, base_name);
    let destination = workspace_base.join(&final_name);
    let staging = create_staging(workspace_base)?;
    let result = (|| {
        let payload = staging.join("payload");
        fs::create_dir_all(&payload).map_err(|_| "IMPORT_STAGING_FAILED")?;
        let mut total = 0_u64;
        let mut files = 0_u64;
        for index in 2..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|_| "ARCHIVE_ENTRY_UNREADABLE")?;
            let name = entry.name().replace('\\', "/");
            if !name.starts_with("payload/") || !safe_archive_path(&name) {
                return Err("ARCHIVE_PATH_INVALID".to_string());
            }
            if entry.is_dir() {
                return Err("ARCHIVE_DIRECTORY_ENTRY_UNSUPPORTED".to_string());
            }
            let compressed = entry.compressed_size();
            if compressed == 0 && entry.size() > 0
                || compressed > 0 && entry.size() / compressed > MAX_COMPRESSION_RATIO
            {
                return Err("ARCHIVE_COMPRESSION_RATIO_INVALID".to_string());
            }
            total = total.saturating_add(entry.size());
            if total > MAX_UNCOMPRESSED_BYTES {
                return Err("ARCHIVE_UNCOMPRESSED_TOO_LARGE".to_string());
            }
            let target = staging.join(&name);
            let parent = target
                .parent()
                .ok_or_else(|| "ARCHIVE_PATH_INVALID".to_string())?;
            fs::create_dir_all(parent).map_err(|_| "IMPORT_STAGING_FAILED")?;
            let mut output = File::create(&target).map_err(|_| "IMPORT_OUTPUT_FAILED")?;
            std::io::copy(&mut entry, &mut output).map_err(|_| "IMPORT_OUTPUT_FAILED")?;
            files += 1;
        }
        for lod in &metadata.lods {
            let source = payload.join(&lod.name);
            if !source.is_dir() {
                return Err("ARCHIVE_LOD_MISSING".to_string());
            }
            fs::rename(&source, staging.join(&lod.name)).map_err(|_| "IMPORT_STAGING_FAILED")?;
        }
        let _ = fs::remove_dir(&payload);
        write_config(&staging, &metadata)?;
        fs::rename(&staging, &destination).map_err(|_| "IMPORT_RENAME_FAILED")?;
        Ok(ImportResult {
            workspace_path: destination.to_string_lossy().to_string(),
            workspace_name: final_name,
            file_count: files,
            total_size: total,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

pub fn import_metadata_skeleton(
    metadata: &PortableWorkspaceMetadataV1,
    workspace_base: &Path,
    requested_name: &str,
) -> Result<ImportResult, String> {
    if !validate_portable_metadata(metadata) || !is_safe_name(requested_name.trim()) {
        return Err("IMPORT_METADATA_INVALID".to_string());
    }
    fs::create_dir_all(workspace_base).map_err(|_| "IMPORT_DIRECTORY_FAILED")?;
    let final_name = unique_workspace_name(workspace_base, requested_name.trim());
    let destination = workspace_base.join(&final_name);
    let staging = create_staging(workspace_base)?;
    let result = (|| {
        write_config(&staging, metadata)?;
        fs::rename(&staging, &destination).map_err(|_| "IMPORT_RENAME_FAILED")?;
        Ok(ImportResult {
            workspace_path: destination.to_string_lossy().to_string(),
            workspace_name: final_name,
            file_count: 0,
            total_size: 0,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn write_config(root: &Path, metadata: &PortableWorkspaceMetadataV1) -> Result<(), String> {
    let config = root.join("Config").join("Tabs");
    fs::create_dir_all(&config).map_err(|_| "IMPORT_CONFIG_FAILED")?;
    let tabs: Vec<_> = metadata
        .lods
        .iter()
        .enumerate()
        .map(|(index, lod)| serde_json::json!({ "id": format!("lod-{index}"), "name": lod.name }))
        .collect();
    let active = tabs
        .first()
        .and_then(|tab| tab.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("lod-0");
    write_json(
        &root.join("Config").join("WorkPageTabs.json"),
        &serde_json::json!({ "activeTabId": active, "tabs": tabs }),
    )?;
    for (index, lod) in metadata.lods.iter().enumerate() {
        if !lod.component_aliases.is_empty() {
            let lod_path = root.join(&lod.name);
            fs::create_dir_all(&lod_path).map_err(|_| "IMPORT_CONFIG_FAILED")?;
            write_json(
                &lod_path.join(crate::workspace::component_aliases::COMPONENT_ALIASES_FILE),
                &serde_json::to_value(&lod.component_aliases)
                    .map_err(|_| "IMPORT_CONFIG_FAILED")?,
            )?;
        }
        let model_rows: Vec<_> = lod
            .draw_ib
            .iter()
            .map(|item| serde_json::json!({ "drawIB": item.hash, "aliasName": item.alias }))
            .collect();
        let skip_rows: Vec<_> = lod.skip_ib.iter().map(|item| serde_json::json!({ "skipIB": item.hash, "aliasName": item.alias, "indexCount": item.index_count.to_string(), "firstIndex": item.first_index.to_string() })).collect();
        let vs_rows: Vec<_> = lod
            .vs_check
            .iter()
            .map(|item| serde_json::json!({ "enabled": item.enabled, "hash": item.hash }))
            .collect();
        write_json(
            &config.join(format!("lod-{index}.json")),
            &serde_json::json!({ "modelRows": model_rows, "skipRows": skip_rows, "vsRows": vs_rows }),
        )?;
    }
    Ok(())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "IMPORT_CONFIG_FAILED")?;
    fs::write(path, bytes).map_err(|_| "IMPORT_CONFIG_FAILED".to_string())
}

fn create_staging(base: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "IMPORT_STAGING_FAILED")?
        .as_nanos();
    let path = base.join(format!(".ssmtws-import-{}-{stamp}", std::process::id()));
    fs::create_dir(&path).map_err(|_| "IMPORT_STAGING_FAILED")?;
    Ok(path)
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    while !current.exists() {
        current = current.parent()?.to_path_buf();
    }
    Some(current)
}

fn unique_workspace_name(base: &Path, name: &str) -> String {
    if !base.join(name).exists() {
        return name.to_string();
    }
    for index in 1..10000 {
        let candidate = format!("{name} ({index})");
        if !base.join(&candidate).exists() {
            return candidate;
        }
    }
    format!(
        "{name} ({})",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0)
    )
}

fn is_safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 128
        && !value.chars().any(char::is_control)
        && !value.ends_with(' ')
        && !value.ends_with('.')
        && !value.chars().any(|ch| matches!(ch, '/' | '\\' | ':'))
}

fn safe_archive_path(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('/') && Path::new(value).components().all(|component| matches!(component, Component::Normal(segment) if !segment.to_string_lossy().contains(':') && !segment.to_string_lossy().ends_with(' ') && !segment.to_string_lossy().ends_with('.')))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "ARCHIVE_NOT_FOUND")?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| "ARCHIVE_READ_FAILED")?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_access::archive::create_workspace_archive;
    use crate::workspace_access::models::WorkspaceAccessPreflightRequest;

    #[test]
    fn imports_into_a_unique_workspace_and_rebuilds_config() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-workspace-import-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let archive = root.join("source.ssmtws");
        let destination = root.join("WorkSpace").join("SRMI");
        fs::create_dir_all(source.join("Config").join("Tabs")).unwrap();
        fs::create_dir_all(source.join("LOD0")).unwrap();
        fs::write(
            source.join("Config").join("WorkPageTabs.json"),
            r#"{"tabs":[{"id":"lod0","name":"LOD0"}]}"#,
        )
        .unwrap();
        fs::write(
            source.join("Config").join("Tabs").join("lod0.json"),
            r#"{"modelRows":[{"drawIB":"0F8A6711","aliasName":"Face"}]}"#,
        )
        .unwrap();
        fs::write(source.join("LOD0").join("mesh.buf"), [1_u8, 2, 3, 4]).unwrap();
        fs::write(
            source.join("LOD0/ComponentAliases.json"),
            r#"{"0f8a6711-3-0":"面部.高光"}"#,
        )
        .unwrap();
        create_workspace_archive(
            &WorkspaceAccessPreflightRequest {
                workspace_path: source.to_string_lossy().to_string(),
                game_preset: "SRMI".to_string(),
                workspace_name: "Source".to_string(),
            },
            &archive,
        )
        .unwrap();
        fs::create_dir_all(destination.join("Imported")).unwrap();

        let result = import_archive(&archive, &destination, "Imported", "SRMI").unwrap();
        assert_eq!(result.workspace_name, "Imported (1)");
        assert!(Path::new(&result.workspace_path)
            .join("Config")
            .join("WorkPageTabs.json")
            .is_file());
        assert!(Path::new(&result.workspace_path)
            .join("LOD0")
            .join("mesh.buf")
            .is_file());
        let imported_aliases = crate::workspace::component_aliases::read_component_aliases(
            &Path::new(&result.workspace_path).join("LOD0"),
        )
        .unwrap();
        assert_eq!(imported_aliases["0f8a6711-3-0"], "面部.高光");
        let metadata =
            crate::workspace_access::archive::read_workspace_archive_metadata(&archive).unwrap();
        assert_eq!(metadata.lods[0].component_aliases, imported_aliases);
        let skeleton = import_metadata_skeleton(&metadata, &destination, "Only Metadata").unwrap();
        assert_eq!(
            crate::workspace::component_aliases::read_component_aliases(
                &Path::new(&skeleton.workspace_path).join("LOD0"),
            )
            .unwrap(),
            imported_aliases
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn imports_metadata_only_as_a_config_skeleton() {
        let root =
            std::env::temp_dir().join(format!("ssmt-metadata-import-test-{}", std::process::id()));
        let metadata = PortableWorkspaceMetadataV1 {
            schema_version: 1,
            game_preset: "SRMI".to_string(),
            workspace_name: "Skeleton".to_string(),
            lods: vec![crate::workspace_access::models::PortableLodV1 {
                component_aliases: Default::default(),
                name: "LOD0".to_string(),
                draw_ib: vec![crate::workspace_access::models::HashAlias {
                    hash: "0f8a6711".to_string(),
                    alias: String::new(),
                }],
                skip_ib: Vec::new(),
                vs_check: Vec::new(),
            }],
        };
        let result = import_metadata_skeleton(&metadata, &root, "Skeleton").unwrap();
        assert!(Path::new(&result.workspace_path)
            .join("Config")
            .join("Tabs")
            .join("lod-0.json")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
