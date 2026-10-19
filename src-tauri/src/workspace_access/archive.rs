use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workspace_access::models::{
    PortableWorkspaceMetadataV1, WorkspaceAccessPreflightRequest, WorkspacePreflightReport,
};
use crate::workspace_access::validate::preflight_workspace;

const ARCHIVE_VERSION: u32 = 1;
const MANIFEST_PATH: &str = "manifest.json";
const PORTABLE_METADATA_PATH: &str = "portable-workspace.json";
const PAYLOAD_PREFIX: &str = "payload/";
const MAX_ARCHIVE_UNCOMPRESSED_SIZE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_COMPRESSION_RATIO: u64 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveManifestV1 {
    pub archive_version: u32,
    pub created_at: String,
    pub generator_version: String,
    pub portable_metadata_path: String,
    pub file_count: u64,
    pub total_uncompressed_size: u64,
    pub files: Vec<ArchiveManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveManifestFile {
    pub path: String,
    pub role: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveBuildResult {
    pub archive_path: String,
    pub sha256: String,
    pub size: u64,
    pub uncompressed_size: u64,
    pub file_count: u64,
    pub manifest: ArchiveManifestV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub file_count: u64,
    pub total_uncompressed_size: u64,
    pub manifest: Option<ArchiveManifestV1>,
}

#[derive(Debug, Clone)]
struct ArchiveSourceFile {
    source_path: PathBuf,
    archive_path: String,
    size: u64,
    modified: Option<SystemTime>,
    role: String,
    sha256: String,
}

pub fn create_workspace_archive(
    request: &WorkspaceAccessPreflightRequest,
    output_path: &Path,
) -> Result<ArchiveBuildResult, String> {
    if output_path.extension().and_then(|value| value.to_str()) != Some("ssmtws") {
        return Err("ARCHIVE_OUTPUT_EXTENSION_INVALID".to_string());
    }
    if output_path.exists() {
        return Err("ARCHIVE_OUTPUT_ALREADY_EXISTS".to_string());
    }
    let report = preflight_workspace(request);
    let metadata = require_preflight_metadata(report)?;
    let root = fs::canonicalize(&request.workspace_path).map_err(|_| "WORKSPACE_NOT_ACCESSIBLE")?;
    let sources = collect_payload_files(&root, &metadata)?;
    let manifest = build_manifest(&sources);

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let write_result = write_archive(output_path, &manifest, &metadata, &sources);
    if let Err(error) = write_result {
        let _ = fs::remove_file(output_path);
        return Err(error);
    }
    if !snapshot_is_unchanged(&sources)? {
        let _ = fs::remove_file(output_path);
        return Err("WORKSPACE_CHANGED_DURING_ARCHIVE".to_string());
    }

    let validation = validate_workspace_archive(output_path);
    if !validation.valid {
        let _ = fs::remove_file(output_path);
        return Err("ARCHIVE_SELF_VALIDATION_FAILED".to_string());
    }
    let size = fs::metadata(output_path)
        .map_err(|error| error.to_string())?
        .len();
    Ok(ArchiveBuildResult {
        archive_path: output_path.to_string_lossy().to_string(),
        sha256: sha256_file(output_path)?,
        size,
        uncompressed_size: manifest.total_uncompressed_size,
        file_count: manifest.file_count,
        manifest,
    })
}

pub fn validate_workspace_archive(path: &Path) -> ArchiveValidationReport {
    let mut errors = Vec::new();
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return invalid_report(vec!["ARCHIVE_NOT_FOUND".to_string()]),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(_) => return invalid_report(vec!["ARCHIVE_NOT_ZIP".to_string()]),
    };
    if archive.len() < 2 {
        return invalid_report(vec!["ARCHIVE_ENTRIES_MISSING".to_string()]);
    }
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return invalid_report(vec!["ARCHIVE_TOO_MANY_ENTRIES".to_string()]);
    }
    let manifest: ArchiveManifestV1 = match read_zip_json(&mut archive, 0, MANIFEST_PATH) {
        Ok(manifest) => manifest,
        Err(error) => return invalid_report(vec![error]),
    };
    if manifest.archive_version != ARCHIVE_VERSION {
        errors.push("UNSUPPORTED_ARCHIVE_VERSION".to_string());
    }
    if manifest.portable_metadata_path != PORTABLE_METADATA_PATH {
        errors.push("INVALID_PORTABLE_METADATA_PATH".to_string());
    }
    match read_zip_json::<PortableWorkspaceMetadataV1>(&mut archive, 1, PORTABLE_METADATA_PATH) {
        Ok(metadata) if validate_portable_metadata(&metadata) => {}
        _ => errors.push("INVALID_PORTABLE_METADATA".to_string()),
    }

    let mut names = HashSet::new();
    let mut folded_names = HashSet::new();
    for index in 0..archive.len() {
        let file = match archive.by_index(index) {
            Ok(file) => file,
            Err(_) => {
                errors.push("ARCHIVE_ENTRY_UNREADABLE".to_string());
                continue;
            }
        };
        let name = file.name().to_string();
        let folded_name = name.to_uppercase();
        if !names.insert(name.clone())
            || !folded_names.insert(folded_name)
            || !is_safe_archive_path(&name)
            || file.is_dir()
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            errors.push("ARCHIVE_PATH_INVALID".to_string());
        }
        if file.compressed_size() == 0 && file.size() > 0
            || file.compressed_size() > 0
                && file.size() / file.compressed_size() > MAX_COMPRESSION_RATIO
        {
            errors.push("ARCHIVE_COMPRESSION_RATIO_INVALID".to_string());
        }
        if name.starts_with(PAYLOAD_PREFIX) && !is_allowed_payload_path(&name) {
            errors.push("ARCHIVE_FILE_EXTENSION_INVALID".to_string());
        }
    }

    let mut expected_paths = HashSet::new();
    let mut actual_total = 0_u64;
    for entry in &manifest.files {
        if !entry.path.starts_with(PAYLOAD_PREFIX) || !is_safe_archive_path(&entry.path) {
            errors.push("MANIFEST_PATH_INVALID".to_string());
            continue;
        }
        if !is_allowed_payload_path(&entry.path) {
            errors.push("MANIFEST_FILE_EXTENSION_INVALID".to_string());
        }
        if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            errors.push("MANIFEST_HASH_INVALID".to_string());
        }
        if !expected_paths.insert(entry.path.clone()) {
            errors.push("MANIFEST_PATH_DUPLICATE".to_string());
            continue;
        }
        let mut file = match archive.by_name(&entry.path) {
            Ok(file) => file,
            Err(_) => {
                errors.push("MANIFEST_FILE_MISSING".to_string());
                continue;
            }
        };
        if file.size() != entry.size {
            errors.push("MANIFEST_FILE_SIZE_MISMATCH".to_string());
        }
        match sha256_reader(&mut file) {
            Ok(hash) if hash == entry.sha256 => {}
            Ok(_) => errors.push("MANIFEST_FILE_HASH_MISMATCH".to_string()),
            Err(_) => errors.push("MANIFEST_FILE_READ_FAILED".to_string()),
        }
        actual_total = actual_total.saturating_add(entry.size);
    }
    if manifest.file_count != manifest.files.len() as u64 {
        errors.push("MANIFEST_FILE_COUNT_MISMATCH".to_string());
    }
    if manifest.total_uncompressed_size != actual_total {
        errors.push("MANIFEST_TOTAL_SIZE_MISMATCH".to_string());
    }
    if actual_total > MAX_ARCHIVE_UNCOMPRESSED_SIZE {
        errors.push("ARCHIVE_UNCOMPRESSED_TOO_LARGE".to_string());
    }
    if names.len() != manifest.files.len() + 2 {
        errors.push("ARCHIVE_UNDECLARED_ENTRY".to_string());
    }

    ArchiveValidationReport {
        valid: errors.is_empty(),
        errors,
        file_count: manifest.file_count,
        total_uncompressed_size: manifest.total_uncompressed_size,
        manifest: Some(manifest),
    }
}

pub fn read_workspace_archive_metadata(path: &Path) -> Result<PortableWorkspaceMetadataV1, String> {
    let validation = validate_workspace_archive(path);
    if !validation.valid {
        return Err(validation
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "ARCHIVE_INVALID".to_string()));
    }
    let file = fs::File::open(path).map_err(|_| "ARCHIVE_NOT_FOUND")?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "ARCHIVE_NOT_ZIP")?;
    let metadata = read_zip_json(&mut archive, 1, PORTABLE_METADATA_PATH)?;
    validate_portable_metadata(&metadata)
        .then_some(metadata)
        .ok_or_else(|| "INVALID_PORTABLE_METADATA".to_string())
}

pub(crate) fn validate_portable_metadata(metadata: &PortableWorkspaceMetadataV1) -> bool {
    if metadata.schema_version != 1
        || !is_safe_workspace_name(&metadata.workspace_name)
        || !is_safe_lod_name(&metadata.game_preset)
        || metadata.lods.is_empty()
        || metadata.lods.len() > 128
    {
        return false;
    }
    let mut names = HashSet::new();
    metadata.lods.iter().all(|lod| {
        is_safe_lod_name(&lod.name)
            && crate::workspace::component_aliases::valid_component_aliases(&lod.component_aliases)
            && names.insert(lod.name.to_uppercase())
            && !lod.draw_ib.is_empty()
            && lod
                .draw_ib
                .iter()
                .all(|item| is_hex_hash(&item.hash, 8, 16))
            && lod
                .skip_ib
                .iter()
                .all(|item| is_hex_hash(&item.hash, 8, 16))
            && lod
                .vs_check
                .iter()
                .all(|item| is_hex_hash(&item.hash, 8, 64))
    })
}

fn require_preflight_metadata(
    report: WorkspacePreflightReport,
) -> Result<PortableWorkspaceMetadataV1, String> {
    report.metadata.ok_or_else(|| {
        report
            .errors
            .first()
            .map(|issue| issue.describe())
            .unwrap_or_else(|| "WORKSPACE_PREFLIGHT_FAILED".to_string())
    })
}

fn collect_payload_files(
    root: &Path,
    metadata: &PortableWorkspaceMetadataV1,
) -> Result<Vec<ArchiveSourceFile>, String> {
    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    for lod in &metadata.lods {
        let lod_root = root.join(&lod.name);
        collect_directory_files(&lod_root, &lod_root, &lod.name, &mut files, &mut seen_paths)?;
    }
    files.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    Ok(files)
}

fn collect_directory_files(
    root: &Path,
    directory: &Path,
    lod_name: &str,
    files: &mut Vec<ArchiveSourceFile>,
    seen_paths: &mut HashSet<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|_| "PAYLOAD_DIRECTORY_READ_FAILED")? {
        let entry = entry.map_err(|_| "PAYLOAD_DIRECTORY_READ_FAILED")?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| "PAYLOAD_METADATA_READ_FAILED")?;
        if metadata.file_type().is_symlink() {
            return Err("PAYLOAD_SYMLINK_NOT_ALLOWED".to_string());
        }
        if metadata.is_dir() {
            collect_directory_files(root, &path, lod_name, files, seen_paths)?;
            continue;
        }
        if !metadata.is_file() {
            return Err("PAYLOAD_FILE_TYPE_INVALID".to_string());
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "PAYLOAD_PATH_INVALID")?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !is_safe_archive_path(&relative) {
            return Err("PAYLOAD_PATH_INVALID".to_string());
        }
        let archive_path = format!("{PAYLOAD_PREFIX}{lod_name}/{relative}");
        if !seen_paths.insert(archive_path.to_ascii_uppercase()) {
            return Err("PAYLOAD_PATH_CASE_COLLISION".to_string());
        }
        files.push(ArchiveSourceFile {
            source_path: path.clone(),
            archive_path,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            role: file_role(&path).to_string(),
            sha256: sha256_file(&path)?,
        });
    }
    Ok(())
}

fn build_manifest(sources: &[ArchiveSourceFile]) -> ArchiveManifestV1 {
    ArchiveManifestV1 {
        archive_version: ARCHIVE_VERSION,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        generator_version: env!("CARGO_PKG_VERSION").to_string(),
        portable_metadata_path: PORTABLE_METADATA_PATH.to_string(),
        file_count: sources.len() as u64,
        total_uncompressed_size: sources.iter().map(|source| source.size).sum(),
        files: sources
            .iter()
            .map(|source| ArchiveManifestFile {
                path: source.archive_path.clone(),
                role: source.role.clone(),
                size: source.size,
                sha256: source.sha256.clone(),
            })
            .collect(),
    }
}

fn write_archive(
    output_path: &Path,
    manifest: &ArchiveManifestV1,
    metadata: &PortableWorkspaceMetadataV1,
    sources: &[ArchiveSourceFile],
) -> Result<(), String> {
    let file = fs::File::create(output_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let stored = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true)
        .unix_permissions(0o644);
    let deflated = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true)
        .unix_permissions(0o644);
    write_zip_json(&mut zip, MANIFEST_PATH, manifest, stored)?;
    write_zip_json(&mut zip, PORTABLE_METADATA_PATH, metadata, deflated)?;
    for source in sources {
        let options = if should_store(&source.source_path) {
            stored
        } else {
            deflated
        };
        zip.start_file(&source.archive_path, options)
            .map_err(|error| error.to_string())?;
        let mut input = fs::File::open(&source.source_path).map_err(|error| error.to_string())?;
        std::io::copy(&mut input, &mut zip).map_err(|error| error.to_string())?;
    }
    zip.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn write_zip_json<T: Serialize>(
    zip: &mut zip::ZipWriter<fs::File>,
    path: &str,
    value: &T,
    options: zip::write::FileOptions,
) -> Result<(), String> {
    zip.start_file(path, options)
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut *zip, value).map_err(|error| error.to_string())?;
    Ok(())
}

fn read_zip_json<T: serde::de::DeserializeOwned>(
    archive: &mut zip::ZipArchive<fs::File>,
    index: usize,
    expected_name: &str,
) -> Result<T, String> {
    let file = archive
        .by_index(index)
        .map_err(|_| "ARCHIVE_ENTRY_UNREADABLE")?;
    if file.name() != expected_name {
        return Err("ARCHIVE_ENTRY_ORDER_INVALID".to_string());
    }
    if expected_name == MANIFEST_PATH && file.compression() != zip::CompressionMethod::Stored {
        return Err("ARCHIVE_MANIFEST_NOT_STORED".to_string());
    }
    serde_json::from_reader(file).map_err(|_| "ARCHIVE_JSON_INVALID".to_string())
}

fn snapshot_is_unchanged(sources: &[ArchiveSourceFile]) -> Result<bool, String> {
    for source in sources {
        let metadata =
            fs::metadata(&source.source_path).map_err(|_| "WORKSPACE_CHANGED_DURING_ARCHIVE")?;
        if metadata.len() != source.size || metadata.modified().ok() != source.modified {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    sha256_reader(file)
}

fn sha256_reader<R: Read>(mut reader: R) -> Result<String, String> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn should_store(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "buf" | "dds" | "ib" | "jpeg" | "jpg" | "png"
    )
}

fn file_role(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "buf" => "category-buffer",
        "ib" => "index-buffer",
        "dds" | "jpg" | "jpeg" | "png" => "texture",
        "json" => "metadata",
        "txt" => "text",
        _ => "payload",
    }
}

fn is_safe_archive_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return false;
    }
    Path::new(path)
        .components()
        .all(|component| match component {
            Component::Normal(segment) => {
                let segment = segment.to_string_lossy();
                let stem = segment.split('.').next().unwrap_or_default().to_uppercase();
                !segment.contains(':')
                    && !segment.ends_with(' ')
                    && !segment.ends_with('.')
                    && !WINDOWS_RESERVED_NAMES.contains(&stem.as_str())
            }
            _ => false,
        })
}

fn is_allowed_payload_path(path: &str) -> bool {
    path.strip_prefix(PAYLOAD_PREFIX)
        .and_then(|value| Path::new(value).extension())
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "buf" | "dds" | "ib" | "jpeg" | "jpg" | "json" | "png" | "txt"
            )
        })
        .unwrap_or(false)
}

fn is_safe_workspace_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 128
        && !value.chars().any(char::is_control)
        && !value.ends_with(' ')
        && !value.ends_with('.')
        && !value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
}

fn is_safe_lod_name(value: &str) -> bool {
    value.chars().count() <= 128
        && is_safe_workspace_name(value)
        && is_safe_archive_path(value)
        && Path::new(value).components().count() == 1
}

fn is_hex_hash(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn invalid_report(errors: Vec<String>) -> ArchiveValidationReport {
    ArchiveValidationReport {
        valid: false,
        errors,
        file_count: 0,
        total_uncompressed_size: 0,
        manifest: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_validates_a_streamed_workspace_archive() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-workspace-archive-test-{}",
            std::process::id()
        ));
        let output = root.join("output.ssmtws");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Config").join("Tabs")).unwrap();
        fs::create_dir_all(root.join("LOD0")).unwrap();
        fs::write(
            root.join("Config").join("WorkPageTabs.json"),
            r#"{"tabs":[{"id":"lod0","name":"LOD0"}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("Config").join("Tabs").join("lod0.json"),
            r#"{"modelRows":[{"drawIB":"0F8A6711","aliasName":"Face"}]}"#,
        )
        .unwrap();
        fs::write(root.join("LOD0").join("mesh.buf"), [1_u8, 2, 3, 4]).unwrap();
        fs::write(root.join("LOD0").join("notes.txt"), "portable").unwrap();

        let result = create_workspace_archive(
            &WorkspaceAccessPreflightRequest {
                workspace_path: root.to_string_lossy().to_string(),
                game_preset: "SRMI".to_string(),
                workspace_name: "Archive Test".to_string(),
            },
            &output,
        )
        .unwrap();
        assert_eq!(result.file_count, 2);
        assert_eq!(result.manifest.files[0].path, "payload/LOD0/mesh.buf");
        assert!(result.sha256.len() == 64);

        let validation = validate_workspace_archive(&output);
        assert!(validation.valid, "{:?}", validation.errors);
        let archive_bytes = fs::read(&output).unwrap();
        assert_eq!(&archive_bytes[0..4], b"PK\x03\x04");
        assert_eq!(
            u16::from_le_bytes([archive_bytes[6], archive_bytes[7]]) & 0x1,
            0
        );
        assert_eq!(u16::from_le_bytes([archive_bytes[8], archive_bytes[9]]), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streams_a_fifty_mib_payload_without_per_file_buffering() {
        assert_streamed_large_payload(50 * 1024 * 1024);
    }

    #[test]
    #[ignore = "manual 800 MiB streaming acceptance check"]
    fn streams_an_eight_hundred_mib_payload_without_per_file_buffering() {
        assert_streamed_large_payload(800 * 1024 * 1024);
    }

    fn assert_streamed_large_payload(payload_size: u64) {
        let root = std::env::temp_dir().join(format!(
            "ssmt-workspace-large-archive-test-{}-{payload_size}",
            std::process::id(),
        ));
        let output = root.join("output.ssmtws");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Config").join("Tabs")).unwrap();
        fs::create_dir_all(root.join("LOD0")).unwrap();
        fs::write(
            root.join("Config").join("WorkPageTabs.json"),
            r#"{"tabs":[{"id":"lod0","name":"LOD0"}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("Config").join("Tabs").join("lod0.json"),
            r#"{"modelRows":[{"drawIB":"0F8A6711","aliasName":"Large"}]}"#,
        )
        .unwrap();
        let payload = fs::File::create(root.join("LOD0").join("large.buf")).unwrap();
        payload.set_len(payload_size).unwrap();

        let result = create_workspace_archive(
            &WorkspaceAccessPreflightRequest {
                workspace_path: root.to_string_lossy().to_string(),
                game_preset: "SRMI".to_string(),
                workspace_name: "Large Archive Test".to_string(),
            },
            &output,
        )
        .unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.uncompressed_size, payload_size);
        assert!(result.size >= payload_size);
        assert!(validate_workspace_archive(&output).valid);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_reserved_names_in_downloaded_archives() {
        let path = std::env::temp_dir().join(format!(
            "ssmt-malicious-archive-{}.ssmtws",
            std::process::id()
        ));
        let file = fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let manifest = ArchiveManifestV1 {
            archive_version: 1,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            generator_version: "test".to_string(),
            portable_metadata_path: PORTABLE_METADATA_PATH.to_string(),
            file_count: 1,
            total_uncompressed_size: 0,
            files: vec![ArchiveManifestFile {
                path: "payload/LOD0/CON.txt".to_string(),
                role: "text".to_string(),
                size: 0,
                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
            }],
        };
        let metadata = PortableWorkspaceMetadataV1 {
            schema_version: 1,
            game_preset: "SRMI".to_string(),
            workspace_name: "Test".to_string(),
            lods: vec![],
        };
        write_zip_json(&mut zip, MANIFEST_PATH, &manifest, options).unwrap();
        write_zip_json(&mut zip, PORTABLE_METADATA_PATH, &metadata, options).unwrap();
        zip.start_file("payload/LOD0/CON.txt", options).unwrap();
        zip.finish().unwrap();
        let report = validate_workspace_archive(&path);
        assert!(!report.valid);
        assert!(report
            .errors
            .iter()
            .any(|error| error == "ARCHIVE_PATH_INVALID"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unsafe_lod_names_in_portable_metadata() {
        let metadata = PortableWorkspaceMetadataV1 {
            schema_version: 1,
            game_preset: "SRMI".to_string(),
            workspace_name: "Test".to_string(),
            lods: vec![crate::workspace_access::models::PortableLodV1 {
                component_aliases: Default::default(),
                name: "..".to_string(),
                draw_ib: vec![crate::workspace_access::models::HashAlias {
                    hash: "0f8a6711".to_string(),
                    alias: String::new(),
                }],
                skip_ib: Vec::new(),
                vs_check: Vec::new(),
            }],
        };
        assert!(!validate_portable_metadata(&metadata));
    }
}
