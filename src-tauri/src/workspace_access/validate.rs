use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::workspace::submesh_json::SubMeshJson;
use crate::workspace_access::models::{
    HashAlias, PortableLodV1, PortableWorkspaceMetadataV1, SkipIbEntry, VsCheckEntry,
    WorkspaceAccessIssue, WorkspaceAccessPreflightRequest, WorkspacePreflightReport,
};

const ALLOWED_EXTENSIONS: &[&str] = &["buf", "dds", "ib", "jpeg", "jpg", "json", "png", "txt"];
const GAME_PRESETS: &[&str] = &[
    "GIMI",
    "HIMI",
    "SRMI",
    "ZZMI",
    "ZZMIDX12",
    "WWMI",
    "EFMI",
    "NTEMI",
    "GF2",
    "IdentityV",
    "AILIMIT",
    "DOAV",
    "SnowBreak",
    "YYSLS",
    "APMI",
    "Naraka",
    "NarakaM",
    "CAMI",
];
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Deserialize)]
struct WorkPageTabsIndex {
    tabs: Vec<WorkPageTab>,
}

#[derive(Debug, Deserialize)]
struct WorkPageTab {
    id: String,
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkPageTabConfig {
    #[serde(default)]
    model_rows: Vec<ModelRow>,
    #[serde(default)]
    skip_rows: Vec<SkipRow>,
    #[serde(default)]
    vs_rows: Vec<VsRow>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelRow {
    #[serde(rename = "drawIB", default)]
    draw_ib: String,
    #[serde(default)]
    alias_name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkipRow {
    #[serde(rename = "skipIB", default)]
    skip_ib: String,
    #[serde(default)]
    alias_name: String,
    #[serde(default)]
    index_count: String,
    #[serde(default)]
    first_index: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VsRow {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    hash: String,
}

fn default_true() -> bool {
    true
}

pub fn preflight_workspace(request: &WorkspaceAccessPreflightRequest) -> WorkspacePreflightReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let workspace_name = request.workspace_name.trim();
    let game_preset = request.game_preset.trim();

    validate_workspace_name(workspace_name, &mut errors);
    if !GAME_PRESETS.contains(&game_preset) {
        errors.push(issue(
            "UNSUPPORTED_GAME_PRESET",
            None,
            "The selected game preset is not supported.",
        ));
    }

    let root = PathBuf::from(&request.workspace_path);
    if !root.is_dir() {
        errors.push(issue(
            "WORKSPACE_NOT_FOUND",
            None,
            "The workspace directory does not exist.",
        ));
        return report(errors, warnings, 0, 0, None);
    }

    let root = match fs::canonicalize(&root) {
        Ok(path) => path,
        Err(error) => {
            errors.push(issue("WORKSPACE_NOT_ACCESSIBLE", None, error.to_string()));
            return report(errors, warnings, 0, 0, None);
        }
    };

    let (file_count, total_size) = scan_workspace_files(&root, &mut errors);
    let lods = read_portable_lods(&root, &mut errors, &mut warnings);
    validate_extracted_files(&root, &mut errors);

    let metadata = if errors.is_empty() {
        Some(PortableWorkspaceMetadataV1 {
            schema_version: 1,
            game_preset: game_preset.to_string(),
            workspace_name: workspace_name.to_string(),
            lods,
        })
    } else {
        None
    };

    report(errors, warnings, file_count, total_size, metadata)
}

fn report(
    errors: Vec<WorkspaceAccessIssue>,
    warnings: Vec<WorkspaceAccessIssue>,
    file_count: u64,
    total_size: u64,
    metadata: Option<PortableWorkspaceMetadataV1>,
) -> WorkspacePreflightReport {
    WorkspacePreflightReport {
        valid: errors.is_empty(),
        errors,
        warnings,
        file_count,
        total_size,
        metadata,
    }
}

fn issue(code: &str, path: Option<String>, detail: impl Into<String>) -> WorkspaceAccessIssue {
    WorkspaceAccessIssue::new(code, path, detail)
}

fn validate_workspace_name(name: &str, errors: &mut Vec<WorkspaceAccessIssue>) {
    if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        errors.push(issue(
            "INVALID_WORKSPACE_NAME",
            None,
            "Workspace names must contain 1-128 non-control characters.",
        ));
    }
}

fn scan_workspace_files(root: &Path, errors: &mut Vec<WorkspaceAccessIssue>) -> (u64, u64) {
    fn visit(
        root: &Path,
        directory: &Path,
        errors: &mut Vec<WorkspaceAccessIssue>,
        seen_paths: &mut HashSet<String>,
        file_count: &mut u64,
        total_size: &mut u64,
    ) {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(issue(
                    "DIRECTORY_READ_FAILED",
                    relative(root, directory),
                    error.to_string(),
                ));
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let relative_path = relative(root, &path);
            if let Err(code) = validate_relative_path(path.strip_prefix(root).unwrap_or(&path)) {
                errors.push(issue(
                    code,
                    relative_path,
                    "The workspace contains an unsafe path.",
                ));
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    errors.push(issue(
                        "PATH_METADATA_FAILED",
                        relative_path,
                        error.to_string(),
                    ));
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                errors.push(issue(
                    "SYMLINK_NOT_ALLOWED",
                    relative_path,
                    "Symbolic links and reparse points are not portable.",
                ));
                continue;
            }
            let fold_key = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/")
                .to_uppercase();
            if !seen_paths.insert(fold_key) {
                errors.push(issue(
                    "CASE_COLLIDING_PATH",
                    relative_path,
                    "Path conflicts after Windows case folding.",
                ));
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, errors, seen_paths, file_count, total_size);
                continue;
            }
            if !metadata.is_file() {
                errors.push(issue(
                    "UNSUPPORTED_FILE_TYPE",
                    relative_path,
                    "Only regular files are allowed.",
                ));
                continue;
            }
            *file_count += 1;
            *total_size = total_size.saturating_add(metadata.len());
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
                errors.push(issue(
                    "UNSUPPORTED_FILE_EXTENSION",
                    relative(root, &path),
                    "The file extension is not allowed in a portable workspace.",
                ));
            }
        }
    }

    let mut seen_paths = HashSet::new();
    let mut file_count = 0;
    let mut total_size = 0;
    visit(
        root,
        root,
        errors,
        &mut seen_paths,
        &mut file_count,
        &mut total_size,
    );
    (file_count, total_size)
}

fn read_portable_lods(
    root: &Path,
    errors: &mut Vec<WorkspaceAccessIssue>,
    warnings: &mut Vec<WorkspaceAccessIssue>,
) -> Vec<PortableLodV1> {
    let index_path = root.join("Config").join("WorkPageTabs.json");
    let index: WorkPageTabsIndex = match read_json::<WorkPageTabsIndex>(&index_path) {
        Ok(index) if !index.tabs.is_empty() => index,
        Ok(_) => {
            errors.push(issue(
                "NO_WORKSPACE_LODS",
                relative(root, &index_path),
                "The workspace has no WorkPage tabs.",
            ));
            return Vec::new();
        }
        Err(error) => {
            errors.push(issue(
                "INVALID_WORKPAGE_TABS",
                relative(root, &index_path),
                error,
            ));
            return Vec::new();
        }
    };

    let mut seen_lods = HashSet::new();
    let mut lods = Vec::new();
    for tab in index.tabs {
        if validate_relative_path(Path::new(&tab.name)).is_err() || tab.name.trim().is_empty() {
            errors.push(issue(
                "INVALID_LOD_NAME",
                Some(tab.name),
                "LOD names must be safe relative path segments.",
            ));
            continue;
        }
        if !seen_lods.insert(tab.name.to_uppercase()) {
            errors.push(issue(
                "DUPLICATE_LOD_NAME",
                Some(tab.name),
                "LOD names must be unique after case folding.",
            ));
            continue;
        }
        if validate_relative_path(Path::new(&tab.id)).is_err() || tab.id.trim().is_empty() {
            errors.push(issue(
                "INVALID_TAB_ID",
                Some(tab.id),
                "Tab IDs must be safe relative path segments.",
            ));
            continue;
        }
        let lod_path = root.join(&tab.name);
        if !lod_path.is_dir() {
            errors.push(issue(
                "MISSING_LOD_DIRECTORY",
                Some(tab.name.clone()),
                "The LOD directory does not exist.",
            ));
        }
        let tab_path = root
            .join("Config")
            .join("Tabs")
            .join(format!("{}.json", tab.id));
        let config: WorkPageTabConfig = match read_json::<WorkPageTabConfig>(&tab_path) {
            Ok(config) => config,
            Err(error) => {
                errors.push(issue(
                    "INVALID_TAB_CONFIG",
                    relative(root, &tab_path),
                    error,
                ));
                continue;
            }
        };
        lods.push(map_lod(&tab.name, config, errors, warnings));
    }
    lods
}

fn map_lod(
    name: &str,
    config: WorkPageTabConfig,
    errors: &mut Vec<WorkspaceAccessIssue>,
    warnings: &mut Vec<WorkspaceAccessIssue>,
) -> PortableLodV1 {
    let mut draw_ib = Vec::new();
    for row in config.model_rows {
        let hash = normalize_hash(&row.draw_ib);
        if hash.is_empty() {
            continue;
        }
        if !is_hex_hash_in_range(&hash, 8, 16) {
            errors.push(issue(
                "INVALID_DRAWIB_HASH",
                Some(name.to_string()),
                "DrawIB hashes must contain 8-16 hexadecimal characters.",
            ));
            continue;
        }
        if hash.len() != 8 {
            warnings.push(issue("DRAWIB_HASH_LENGTH_UNAUDITED", Some(name.to_string()), "This DrawIB hash is not 8 hexadecimal characters and needs preset-specific review."));
        }
        draw_ib.push(HashAlias {
            hash,
            alias: row.alias_name.trim().to_string(),
        });
    }
    if draw_ib.is_empty() {
        errors.push(issue(
            "MISSING_DRAWIB",
            Some(name.to_string()),
            "Each portable LOD needs at least one DrawIB.",
        ));
    }

    let mut skip_ib = Vec::new();
    for row in config.skip_rows {
        let hash = normalize_hash(&row.skip_ib);
        if hash.is_empty() {
            continue;
        }
        let index_count =
            parse_nonnegative(&row.index_count, "INVALID_SKIPIB_INDEX_COUNT", name, errors);
        let first_index =
            parse_nonnegative(&row.first_index, "INVALID_SKIPIB_FIRST_INDEX", name, errors);
        if !is_hex_hash_in_range(&hash, 8, 16) {
            errors.push(issue(
                "INVALID_SKIPIB_HASH",
                Some(name.to_string()),
                "SkipIB hashes must contain 8-16 hexadecimal characters.",
            ));
            continue;
        }
        if let (Some(index_count), Some(first_index)) = (index_count, first_index) {
            skip_ib.push(SkipIbEntry {
                hash,
                alias: row.alias_name.trim().to_string(),
                index_count,
                first_index,
            });
        }
    }

    let mut vs_check = Vec::new();
    for row in config.vs_rows {
        let hash = normalize_hash(&row.hash);
        if hash.is_empty() {
            continue;
        }
        if !is_hex_hash_in_range(&hash, 8, 64) {
            errors.push(issue(
                "INVALID_VSCHECK_HASH",
                Some(name.to_string()),
                "VS hashes must contain 8-64 hexadecimal characters.",
            ));
            continue;
        }
        vs_check.push(VsCheckEntry {
            enabled: row.enabled,
            hash,
        });
    }
    PortableLodV1 {
        name: name.to_string(),
        draw_ib,
        skip_ib,
        vs_check,
    }
}

fn validate_extracted_files(root: &Path, errors: &mut Vec<WorkspaceAccessIssue>) {
    fn visit(root: &Path, directory: &Path, errors: &mut Vec<WorkspaceAccessIssue>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, errors);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if extension == "dds" {
                validate_dds(&path, root, errors);
            } else if extension == "json" && !path.starts_with(root.join("Config")) {
                validate_json_and_references(&path, root, errors);
            }
        }
    }
    visit(root, root, errors);
}

fn validate_dds(path: &Path, root: &Path, errors: &mut Vec<WorkspaceAccessIssue>) {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            errors.push(issue(
                "DDS_READ_FAILED",
                relative(root, path),
                error.to_string(),
            ));
            return;
        }
    };
    let mut header = [0_u8; 148];
    let count = match file.read(&mut header) {
        Ok(count) => count,
        Err(error) => {
            errors.push(issue(
                "DDS_READ_FAILED",
                relative(root, path),
                error.to_string(),
            ));
            return;
        }
    };
    if count < 128
        || &header[0..4] != b"DDS "
        || u32::from_le_bytes(header[4..8].try_into().unwrap()) != 124
    {
        errors.push(issue(
            "INVALID_DDS_HEADER",
            relative(root, path),
            "DDS files must include a complete standard header.",
        ));
        return;
    }
    let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let width = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if height == 0 || width == 0 {
        errors.push(issue(
            "INVALID_DDS_DIMENSIONS",
            relative(root, path),
            "DDS dimensions must be non-zero.",
        ));
    }
    if &header[84..88] == b"DX10" && count < 148 {
        errors.push(issue(
            "INVALID_DDS_DX10_HEADER",
            relative(root, path),
            "DDS DX10 files need the complete extension header.",
        ));
    }
    if fs::metadata(path)
        .map(|metadata| metadata.len() <= 128)
        .unwrap_or(true)
    {
        errors.push(issue(
            "INVALID_DDS_DATA",
            relative(root, path),
            "DDS files must contain pixel data after the header.",
        ));
    }
}

fn validate_json_and_references(path: &Path, root: &Path, errors: &mut Vec<WorkspaceAccessIssue>) {
    let source: serde_json::Value = match read_json::<serde_json::Value>(path) {
        Ok(value) => value,
        Err(error) => {
            errors.push(issue("INVALID_JSON", relative(root, path), error));
            return;
        }
    };
    if source.get("IndexBufferList").is_none() {
        return;
    }
    let submesh: SubMeshJson = match serde_json::from_value(source) {
        Ok(value) => value,
        Err(error) => {
            errors.push(issue(
                "INVALID_SUBMESH_JSON",
                relative(root, path),
                error.to_string(),
            ));
            return;
        }
    };
    let parent = path.parent().unwrap_or(root);
    if submesh.vertex_offset < 0
        || submesh.vertex_count < 0
        || submesh.index_offset < 0
        || submesh.index_count < 0
    {
        errors.push(issue(
            "INVALID_SUBMESH_RANGE",
            relative(root, path),
            "Submesh vertex and index offsets/counts must be non-negative.",
        ));
    }
    for buffer in &submesh.index_buffer_list {
        let Some(buffer_path) = resolve_reference(parent, &buffer.file_name, root) else {
            errors.push(issue(
                "UNSAFE_REFERENCED_FILE",
                relative(root, path),
                "Submesh index buffer path is unsafe.",
            ));
            continue;
        };
        match fs::metadata(&buffer_path) {
            Ok(metadata) if metadata.len() > 0 => {
                let element_size = match buffer.dxgi_format.to_ascii_uppercase().as_str() {
                    "DXGI_FORMAT_R16_UINT" => Some(2),
                    "DXGI_FORMAT_R32_UINT" => Some(4),
                    _ => None,
                };
                match element_size {
                    Some(size) if metadata.len() % size == 0 => {
                        let element_count = metadata.len() / size;
                        let end = u64::try_from(submesh.index_offset).ok().and_then(|offset| {
                            u64::try_from(submesh.index_count)
                                .ok()
                                .and_then(|count| offset.checked_add(count))
                        });
                        if end.is_none_or(|value| value > element_count) {
                            errors.push(issue(
                                "INDEX_RANGE_OUT_OF_BOUNDS",
                                relative(root, &buffer_path),
                                "IndexOffset and IndexCount exceed the index buffer length.",
                            ));
                        }
                    }
                    Some(_) => errors.push(issue(
                        "INVALID_IB_LENGTH",
                        relative(root, &buffer_path),
                        "Index buffer length is not aligned to its DXGI format.",
                    )),
                    None => errors.push(issue(
                        "UNSUPPORTED_IB_FORMAT",
                        relative(root, path),
                        "Only R16_UINT and R32_UINT index buffers are portable.",
                    )),
                }
            }
            Ok(_) => errors.push(issue(
                "EMPTY_REFERENCED_FILE",
                relative(root, &buffer_path),
                "Referenced files cannot be empty.",
            )),
            Err(_) => errors.push(issue(
                "MISSING_REFERENCED_FILE",
                relative(root, path),
                format!("Referenced file does not exist: {}", buffer.file_name),
            )),
        }
    }
    for buffer in &submesh.category_buffer_list {
        validate_category_buffer(buffer, parent, root, path, errors);
    }
    if !submesh.bone_matrix_file_name.is_empty() {
        validate_required_reference(parent, &submesh.bone_matrix_file_name, root, path, errors);
    }
    for texture in submesh.texture_mark_up_info_list {
        if !texture.mark_file_name.is_empty() {
            validate_required_reference(parent, &texture.mark_file_name, root, path, errors);
        }
    }
    for diffuse_map in submesh.diffuse_map {
        if !diffuse_map.is_empty() {
            validate_required_reference(parent, &diffuse_map, root, path, errors);
        }
    }
}

fn validate_category_buffer(
    buffer: &crate::workspace::submesh_json::SubMeshCategoryBuffer,
    parent: &Path,
    root: &Path,
    source: &Path,
    errors: &mut Vec<WorkspaceAccessIssue>,
) {
    let Some(path) = resolve_reference(parent, &buffer.file_name, root) else {
        errors.push(issue(
            "UNSAFE_REFERENCED_FILE",
            relative(root, source),
            "Referenced path is unsafe.",
        ));
        return;
    };
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > 0 => (),
        Ok(_) => {
            errors.push(issue(
                "EMPTY_REFERENCED_FILE",
                relative(root, &path),
                "Referenced files cannot be empty.",
            ));
            return;
        }
        Err(_) => {
            errors.push(issue(
                "MISSING_REFERENCED_FILE",
                relative(root, source),
                format!("Referenced file does not exist: {}", buffer.file_name),
            ));
            return;
        }
    };
    if buffer.d3d11_element_list.is_empty() {
        errors.push(issue(
            "CATEGORY_LAYOUT_MISSING",
            relative(root, source),
            "Category buffers require a D3D11 element layout.",
        ));
        return;
    }
}

fn validate_required_reference(
    parent: &Path,
    name: &str,
    root: &Path,
    source: &Path,
    errors: &mut Vec<WorkspaceAccessIssue>,
) {
    let Some(path) = resolve_reference(parent, name, root) else {
        errors.push(issue(
            "UNSAFE_REFERENCED_FILE",
            relative(root, source),
            "Referenced path is unsafe.",
        ));
        return;
    };
    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > 0 => {}
        Ok(_) => errors.push(issue(
            "EMPTY_REFERENCED_FILE",
            relative(root, &path),
            "Referenced files cannot be empty.",
        )),
        Err(_) => errors.push(issue(
            "MISSING_REFERENCED_FILE",
            relative(root, source),
            format!("Referenced file does not exist: {name}"),
        )),
    }
}

fn resolve_reference(parent: &Path, name: &str, root: &Path) -> Option<PathBuf> {
    let relative = Path::new(name);
    if validate_relative_path(relative).is_err() {
        return None;
    }
    let path = parent.join(relative);
    path.starts_with(root).then_some(path)
}

fn validate_relative_path(path: &Path) -> Result<(), &'static str> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("UNSAFE_PATH");
    }
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let value = segment.to_string_lossy();
                let stem = value
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                if value.contains(':')
                    || value.ends_with(' ')
                    || value.ends_with('.')
                    || WINDOWS_RESERVED_NAMES.contains(&stem.as_str())
                {
                    return Err("UNSAFE_PATH");
                }
            }
            _ => return Err("UNSAFE_PATH"),
        }
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

fn normalize_hash(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn is_hex_hash_in_range(value: &str, min_length: usize, max_length: usize) -> bool {
    (min_length..=max_length).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_nonnegative(
    value: &str,
    code: &str,
    lod: &str,
    errors: &mut Vec<WorkspaceAccessIssue>,
) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Some(0);
    }
    match value.parse::<u64>() {
        Ok(value) => Some(value),
        Err(_) => {
            errors.push(issue(
                code,
                Some(lod.to_string()),
                "Values must be non-negative integers.",
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_relative_paths() {
        assert!(validate_relative_path(Path::new("LOD0/mesh.buf")).is_ok());
        assert!(validate_relative_path(Path::new("../mesh.buf")).is_err());
        assert!(validate_relative_path(Path::new("LOD0/CON.json")).is_err());
        assert!(validate_relative_path(Path::new("LOD0/mesh:stream.buf")).is_err());
    }

    #[test]
    fn normalizes_and_validates_hashes() {
        assert_eq!(normalize_hash(" 0F8A6711 "), "0f8a6711");
        assert!(is_hex_hash_in_range("0f8a6711", 8, 16));
        assert!(is_hex_hash_in_range("0123456789abcdef", 8, 16));
        assert!(!is_hex_hash_in_range("not-a-hash", 8, 64));
    }

    #[test]
    fn maps_workpage_configuration_without_local_paths() {
        let root =
            std::env::temp_dir().join(format!("ssmt-workspace-access-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Config").join("Tabs")).unwrap();
        fs::create_dir_all(root.join("LOD0")).unwrap();
        fs::write(
            root.join("Config").join("WorkPageTabs.json"),
            r#"{"activeTabId":"lod0","tabs":[{"id":"lod0","name":"LOD0"}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("Config").join("Tabs").join("lod0.json"),
            r#"{
                "modelRows":[{"drawIB":"0F8A6711","aliasName":"Face"}],
                "skipRows":[{"skipIB":"1234ABCD","aliasName":"Hair","indexCount":"12","firstIndex":"0"}],
                "vsRows":[{"enabled":true,"hash":"0123456789abcdef"}],
                "frameAnalysisFolderPath":"C:\\private\\capture",
                "selectedFrameAnalysis":"FrameAnalysis-1"
            }"#,
        )
        .unwrap();

        let report = preflight_workspace(&WorkspaceAccessPreflightRequest {
            workspace_path: root.to_string_lossy().to_string(),
            game_preset: "SRMI".to_string(),
            workspace_name: "Test Workspace".to_string(),
        });

        assert!(report.valid, "{:?}", report.errors);
        let metadata = report.metadata.unwrap();
        assert_eq!(metadata.lods[0].draw_ib[0].hash, "0f8a6711");
        let metadata_json = serde_json::to_string(&metadata).unwrap();
        assert!(!metadata_json.contains("frameAnalysisFolderPath"));
        assert!(!metadata_json.contains("private"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_index_ranges_without_reinterpreting_category_buffers() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-workspace-access-submesh-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let lod = root.join("LOD0");
        fs::create_dir_all(&lod).unwrap();
        fs::write(lod.join("mesh.ib"), [0_u8; 4]).unwrap();
        fs::write(lod.join("position.buf"), [0_u8; 6]).unwrap();
        let mut submesh = SubMeshJson::default();
        submesh.index_count = 3;
        submesh
            .index_buffer_list
            .push(crate::workspace::submesh_json::SubMeshIndexBuffer {
                dxgi_format: "DXGI_FORMAT_R16_UINT".to_string(),
                file_name: "mesh.ib".to_string(),
            });
        submesh
            .category_buffer_list
            .push(crate::workspace::submesh_json::SubMeshCategoryBuffer {
                file_name: "position.buf".to_string(),
                buffer_type: "Position".to_string(),
                d3d11_element_list: vec![crate::workspace::submesh_json::SubMeshD3D11Element {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: "0".to_string(),
                    format: "DXGI_FORMAT_R32_FLOAT".to_string(),
                    byte_width: "4".to_string(),
                    extract_slot: "0".to_string(),
                    extract_technique: String::new(),
                    category: "Position".to_string(),
                    draw_category: "Position".to_string(),
                }],
            });
        let source = lod.join("submesh.json");
        fs::write(&source, serde_json::to_vec(&submesh).unwrap()).unwrap();
        let mut errors = Vec::new();
        validate_json_and_references(&source, &root, &mut errors);
        assert!(errors
            .iter()
            .any(|issue| issue.code == "INDEX_RANGE_OUT_OF_BOUNDS"));
        assert!(!errors
            .iter()
            .any(|issue| issue.code == "CATEGORY_BUFFER_STRIDE_MISMATCH"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_ssmt_category_layout_without_reinterpreting_slot_or_format() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-workspace-access-category-layout-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let lod = root.join("LOD0");
        fs::create_dir_all(&lod).unwrap();
        fs::write(lod.join("mesh.ib"), [0_u8; 6]).unwrap();
        fs::write(lod.join("position.buf"), [0_u8; 12]).unwrap();

        let mut submesh = SubMeshJson::default();
        submesh.vertex_count = 1;
        submesh.index_count = 3;
        submesh
            .index_buffer_list
            .push(crate::workspace::submesh_json::SubMeshIndexBuffer {
                dxgi_format: "DXGI_FORMAT_R16_UINT".to_string(),
                file_name: "mesh.ib".to_string(),
            });
        submesh
            .category_buffer_list
            .push(crate::workspace::submesh_json::SubMeshCategoryBuffer {
                file_name: "position.buf".to_string(),
                buffer_type: "Position".to_string(),
                d3d11_element_list: vec![crate::workspace::submesh_json::SubMeshD3D11Element {
                    semantic_name: "POSITION".to_string(),
                    semantic_index: "0".to_string(),
                    format: "R32G32B32_FLOAT".to_string(),
                    byte_width: "12".to_string(),
                    extract_slot: "vb0".to_string(),
                    extract_technique: "pointlist".to_string(),
                    category: "Position".to_string(),
                    draw_category: "Position".to_string(),
                }],
            });

        let source = lod.join("submesh.json");
        fs::write(&source, serde_json::to_vec(&submesh).unwrap()).unwrap();
        let mut errors = Vec::new();
        validate_json_and_references(&source, &root, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        fs::remove_dir_all(root).unwrap();
    }
}
