use crate::ini_container::migoto_data_types::condition::Condition;
use crate::ini_container::migoto_data_types::domain::Key;
use crate::ini_container::migoto_ini_container::container::MigotoIniContainer;
use crate::utils::ssmt_compress_utils::{ArchivePreview, ExtractResult, SSMTCompressUtils};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncWriteExt;

// Watcher State
pub struct ModWatcher(pub Mutex<Option<RecommendedWatcher>>);

const GAMEBANANA_DOWNLOAD_CANCELLED: &str = "GameBanana download cancelled";

// A download can be cancelled from a second Tauri command while the first one
// is awaiting network chunks.  Store only active jobs so a late or stale click
// cannot cancel a later download that happens to use the same target name.
static GAMEBANANA_DOWNLOAD_CANCELLATIONS: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

fn gamebanana_download_key(game_name: &str, target_name: &str) -> String {
    format!(
        "{}\u{1f}{}",
        game_name.trim().to_ascii_lowercase(),
        target_name.trim().to_ascii_lowercase(),
    )
}

fn gamebanana_download_cancellations() -> &'static Mutex<HashMap<String, bool>> {
    GAMEBANANA_DOWNLOAD_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gamebanana_download_is_cancelled(key: &str) -> bool {
    gamebanana_download_cancellations()
        .lock()
        .map(|jobs| jobs.get(key).copied().unwrap_or(false))
        .unwrap_or(false)
}

struct GamebananaDownloadCancellationGuard {
    key: String,
}

impl GamebananaDownloadCancellationGuard {
    fn begin(game_name: &str, target_name: &str) -> Self {
        let key = gamebanana_download_key(game_name, target_name);
        if let Ok(mut jobs) = gamebanana_download_cancellations().lock() {
            jobs.insert(key.clone(), false);
        }
        Self { key }
    }
}

impl Drop for GamebananaDownloadCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut jobs) = gamebanana_download_cancellations().lock() {
            jobs.remove(&self.key);
        }
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub path: String,
    pub relative_path: String,
    pub preview_images: Vec<String>,
    pub group: String, // Changed to 'group' to match JS interface
    pub is_dir: bool,
    pub last_modified: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    pub icon_path: Option<String>,
    pub path: String,
    pub enabled: bool,
    pub mod_count: u64,
}

fn modified_unix_seconds(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn is_content_image_file(name_lower: &str) -> bool {
    matches!(name_lower, _ if name_lower.ends_with(".jpg")
        || name_lower.ends_with(".jpeg")
        || name_lower.ends_with(".png")
        || name_lower.ends_with(".gif")
        || name_lower.ends_with(".bmp")
        || name_lower.ends_with(".webp")
        || name_lower.ends_with(".avif"))
}

fn is_standard_group_icon(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "folder.jpg" | "folder.png" | "icon.jpg" | "icon.png" | "cover.jpg" | "cover.png"
    )
}

fn is_ssmt_backup_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("disabled_") && lower.ends_with("_bak")
}

fn mods_root(install_dir: &str) -> PathBuf {
    let install_path = PathBuf::from(install_dir);
    if install_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|name| name.eq_ignore_ascii_case("Mods"))
        .unwrap_or(false)
    {
        install_path
    } else {
        install_path.join("Mods")
    }
}

fn is_leaf_mod_dir(path: &Path) -> bool {
    let mut has_ini = false;
    let mut has_subdirs = false;
    let mut has_content_images = false;

    if let Ok(subs) = fs::read_dir(path) {
        for sub in subs.flatten() {
            let sub_path = sub.path();
            if sub_path.is_dir() {
                let sub_name = sub_path.file_name().unwrap_or_default().to_string_lossy();
                if !sub_name.starts_with('.')
                    && !sub_name.starts_with('$')
                    && !is_ssmt_backup_name(&sub_name)
                {
                    has_subdirs = true;
                }
            } else if sub_path.is_file() {
                if let Some(name) = sub_path.file_name().and_then(|n| n.to_str()) {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".ini") {
                        has_ini = true;
                    }
                    if is_content_image_file(&lower) && !is_standard_group_icon(&lower) {
                        has_content_images = true;
                    }
                }
            }
        }
    }

    has_ini || (!has_subdirs && has_content_images)
}

fn count_direct_leaf_mod_dirs(path: &Path) -> u64 {
    let mut count = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let sub_path = entry.path();
            if !sub_path.is_dir() {
                continue;
            }

            let sub_name = sub_path.file_name().unwrap_or_default().to_string_lossy();
            if sub_name.starts_with('.')
                || sub_name.starts_with('$')
                || is_ssmt_backup_name(&sub_name)
            {
                continue;
            }

            if is_leaf_mod_dir(&sub_path) {
                count += 1;
            }
        }
    }
    count
}

fn analyze_directory(path: &Path) -> (Vec<String>, bool, bool, bool, Option<String>) {
    let mut images = Vec::new();
    let mut has_ini = false;
    let mut has_subdirs = false;
    let mut has_content_images = false;
    let mut icon_path: Option<String> = None;

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let sub_path = entry.path();
            if sub_path.is_dir() {
                let sub_name = sub_path.file_name().unwrap_or_default().to_string_lossy();
                if !sub_name.starts_with('.')
                    && !sub_name.starts_with('$')
                    && !is_ssmt_backup_name(&sub_name)
                {
                    has_subdirs = true;
                }
                continue;
            }

            if !sub_path.is_file() {
                continue;
            }

            if let Some(name) = sub_path.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                if lower.ends_with(".ini") {
                    has_ini = true;
                }
                if is_content_image_file(&lower) {
                    let image_path = sub_path.to_string_lossy().replace("\\", "/");
                    images.push(image_path.clone());
                    if is_standard_group_icon(&lower) {
                        icon_path = Some(image_path);
                    } else {
                        has_content_images = true;
                    }
                }
            }
        }
    }

    images.sort();
    (images, has_ini, has_subdirs, has_content_images, icon_path)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub mods: Vec<ModInfo>,
    pub groups: Vec<GroupInfo>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModKeyInfo {
    pub source_ini: String,
    pub key_name: String,
    pub back_name: String,
    pub key_type: String,
    pub value_summary: String,
    pub condition_summary: String,
}

fn strip_disabled_prefix(name: &str) -> (String, bool) {
    let upper = name.to_uppercase();
    if upper.starts_with("DISABLED_") {
        (name[9..].to_string(), true)
    } else if upper.starts_with("DISABLED ") {
        (name[9..].to_string(), true)
    } else if upper.starts_with("DISABLED") {
        (name[8..].to_string(), true)
    } else {
        (name.to_string(), false)
    }
}

fn archive_file_name_is_valid(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && !trimmed.ends_with('.')
        && !trimmed.ends_with(' ')
        && !trimmed
            .chars()
            .any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

fn mod_path_segment_candidates(segment: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |value: String| {
        let key = value.to_lowercase();
        if seen.insert(key) {
            values.push(value);
        }
    };

    push(segment.to_string());
    let (clean, disabled) = strip_disabled_prefix(segment);
    if disabled {
        push(clean);
    } else {
        push(format!("DISABLED_{}", segment));
        push(format!("DISABLED {}", segment));
    }

    values
}

fn resolve_physical_install_group_path(mods_dir: &Path, logical_group_path: &Path) -> PathBuf {
    let mut current = mods_dir.to_path_buf();
    for segment in logical_group_path.components() {
        let std::path::Component::Normal(segment) = segment else {
            continue;
        };
        let requested = segment.to_string_lossy().to_string();
        let existing = mod_path_segment_candidates(&requested)
            .into_iter()
            .map(|candidate| current.join(candidate))
            .find(|candidate| candidate.is_dir());
        current = existing.unwrap_or_else(|| current.join(requested));
    }
    current
}

fn install_target_exists(
    mods_dir: &Path,
    target_group_path: &Path,
    target_name_path: &Path,
) -> bool {
    let target_parent = resolve_physical_install_group_path(mods_dir, target_group_path);
    let Some(target_name) = target_name_path
        .file_name()
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    mod_path_segment_candidates(target_name)
        .into_iter()
        .map(|candidate| target_parent.join(candidate))
        .any(|candidate| candidate.is_dir())
}

fn find_mod_dir_by_clean_tail(root: &Path, tail: &str) -> Option<PathBuf> {
    let wanted = strip_disabled_prefix(tail).0.to_lowercase();
    if wanted.is_empty() {
        return None;
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.starts_with('$') || is_ssmt_backup_name(&name) {
                continue;
            }

            let clean = strip_disabled_prefix(&name).0;
            if clean.eq_ignore_ascii_case(&wanted) && is_leaf_mod_dir(&path) {
                return Some(path);
            }

            stack.push(path);
        }
    }

    None
}

fn resolve_existing_mod_dir(install_dir: &str, mod_relative_path: &str) -> Result<PathBuf, String> {
    let root = mods_root(install_dir);
    let normalized = mod_relative_path
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        return Err("Mod relative path is empty".to_string());
    }

    let direct = root.join(normalized.replace('/', std::path::MAIN_SEPARATOR_STR));
    if direct.is_dir() {
        return Ok(direct);
    }

    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut candidates = vec![root.clone()];
    for segment in segments {
        let variants = mod_path_segment_candidates(segment);
        let mut next = Vec::new();
        for base in &candidates {
            for variant in &variants {
                next.push(base.join(variant));
            }
        }
        candidates = next;
    }

    for candidate in candidates {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }

    if let Some(tail) = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .last()
    {
        if let Some(found) = find_mod_dir_by_clean_tail(&root, tail) {
            return Ok(found);
        }
    }

    Err(format!(
        "Mod source directory not found. Tried relative path '{}' under '{}'. The Mod list may be stale; refresh Mods and try again.",
        mod_relative_path,
        root.to_string_lossy()
    ))
}

fn collect_ini_files_recursive(path: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let current_path = entry.path();
        if current_path.is_dir() {
            collect_ini_files_recursive(&current_path, output);
            continue;
        }

        let Some(name) = current_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if name.to_ascii_lowercase().ends_with(".ini") {
            output.push(current_path);
        }
    }
}

fn build_condition_summary(condition: &Condition) -> String {
    if condition.condition_expression_list.is_empty() {
        return String::new();
    }

    if condition.logic_list.is_empty() {
        let expr = &condition.condition_expression_list[0];
        return format!("{} == {}", expr.var_name, expr.var_value);
    }

    let mut segments = Vec::new();
    for (index, expr) in condition.condition_expression_list.iter().enumerate() {
        if index > 0 {
            let logic = condition
                .logic_list
                .get(index - 1)
                .cloned()
                .unwrap_or_else(|| "&&".to_string());
            segments.push(logic);
        }
        segments.push(format!("{} == {}", expr.var_name, expr.var_value));
    }

    segments.join(" ")
}

fn build_value_summary(key: &Key) -> String {
    let mut segments: Vec<String> = Vec::new();

    let mut active_entries: Vec<_> = key.active_variable_name_active_value_map.iter().collect();
    active_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (name, value) in active_entries {
        segments.push(format!("{} = {}", name, value));
    }

    let mut cycle_entries: Vec<_> = key
        .cycle_variable_name_possible_value_list_map
        .iter()
        .collect();
    cycle_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (name, values) in cycle_entries {
        segments.push(format!("{} = {}", name, values.join(", ")));
    }

    segments.join(" | ")
}

#[tauri::command]
pub async fn scan_directory(
    install_dir: String,
    relative_path: String,
) -> Result<ScanResult, String> {
    let mods_root = mods_root(&install_dir);

    // Construct target path: mods_root + relative_path (relative_path can be empty or "Root")
    // If relative_path is "Root", treat as empty
    let target_sub = if relative_path == "Root" {
        "".to_string()
    } else {
        relative_path.clone()
    };

    let target_path = mods_root.join(&target_sub);

    if !target_path.exists() {
        return Ok(ScanResult {
            mods: vec![],
            groups: vec![],
        });
    }

    let mut mods = Vec::new();
    let mut groups = Vec::new();

    let entries = fs::read_dir(&target_path).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if dir_name.starts_with('.') || dir_name.starts_with('$') || is_ssmt_backup_name(&dir_name)
        {
            continue;
        }

        let (clean_name, is_disabled) = strip_disabled_prefix(&dir_name);
        let enabled = !is_disabled;
        let (images, has_ini, has_subdirs, has_content_images, icon_path) =
            analyze_directory(&path);
        let is_leaf_mod = has_ini || (!has_subdirs && has_content_images);
        let entry_relative_path = if target_sub.is_empty() {
            dir_name.clone()
        } else {
            format!("{}/{}", target_sub, dir_name)
        };

        if is_leaf_mod {
            mods.push(ModInfo {
                id: entry_relative_path.clone(),
                name: clean_name,
                enabled,
                path: path.to_string_lossy().replace("\\", "/"),
                relative_path: entry_relative_path.clone(),
                preview_images: images,
                group: if target_sub.is_empty() {
                    "Root".to_string()
                } else {
                    target_sub.replace("\\", "/")
                },
                is_dir: true,
                last_modified: modified_unix_seconds(&path),
            });
        } else {
            let mod_count = count_direct_leaf_mod_dirs(&path);
            groups.push(GroupInfo {
                id: entry_relative_path.clone(),
                name: clean_name,
                icon_path,
                path: entry_relative_path,
                enabled,
                mod_count,
            });
        }
    }

    mods.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(ScanResult { mods, groups })
}

#[tauri::command]
pub async fn get_mod_key_list(
    install_dir: String,
    mod_relative_path: String,
) -> Result<Vec<ModKeyInfo>, String> {
    let mod_path = mods_root(&install_dir).join(&mod_relative_path);
    if !mod_path.exists() {
        return Err(format!(
            "Mod directory not found at: {}",
            mod_path.display()
        ));
    }

    let mut ini_files = Vec::new();
    collect_ini_files_recursive(&mod_path, &mut ini_files);
    ini_files.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));

    let mut result = Vec::new();
    for ini_file in ini_files {
        let container = MigotoIniContainer::from_ini_file(&ini_file)?;
        let source_ini = ini_file
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();

        for key in container.global_m_key_list {
            let value_summary = build_value_summary(&key);
            if value_summary.is_empty() {
                continue;
            }

            result.push(ModKeyInfo {
                source_ini: source_ini.clone(),
                key_name: key.key_name,
                back_name: key.back_name,
                key_type: key.key_type,
                value_summary,
                condition_summary: build_condition_summary(&key.condition),
            });
        }
    }

    result.sort_by(|left, right| {
        left.source_ini
            .cmp(&right.source_ini)
            .then_with(|| left.key_name.cmp(&right.key_name))
            .then_with(|| left.back_name.cmp(&right.back_name))
    });

    Ok(result)
}

#[tauri::command]
pub async fn watch_mods(
    app: AppHandle,
    state: State<'_, ModWatcher>,
    install_dir: String,
) -> Result<(), String> {
    let mods_dir = mods_root(&install_dir);

    if !mods_dir.exists() {
        return Err(format!("Mods directory not found at: {:?}", mods_dir));
    }

    // Stop existing watcher
    let mut watcher_guard = state.0.lock().unwrap();
    if let Some(_) = *watcher_guard {
        // Drop old watcher
        *watcher_guard = None;
    }

    let app_handle = app.clone();

    // Config: Poll every 2 seconds if native events fail, but usage of default() usually implies native.
    // For Windows, default is ReadDirectoryChangesW which is instant.
    // We can add a small delay to debounce at the source if needed, but notify v6 handles this differently.
    // We will just emit raw events and let frontend debounce.

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            match res {
                Ok(_event) => {
                    // Filter for relevant events if needed, but 'refresh' is safe for all
                    // println!("File event: {:?}", event);
                    // Send event to all windows
                    let _ = app_handle.emit("mod-filesystem-changed", ());
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        },
        Config::default(),
    )
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    // Watch recursively
    watcher
        .watch(&mods_dir, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to start watch: {}", e))?;

    // Store watcher
    *watcher_guard = Some(watcher);

    println!("[ModWatcher] Started watching: {:?}", mods_dir);

    Ok(())
}

#[tauri::command]
pub fn unwatch_mods(state: State<'_, ModWatcher>) -> Result<(), String> {
    let mut watcher_guard = state.0.lock().unwrap();
    *watcher_guard = None;
    println!("[ModWatcher] Stopped watching");
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct InstallProgressPayload {
    pub game_name: String,
    pub mod_name: String,
    pub stage: String,
    pub current: u64,
    pub total: u64,
}

fn emit_install_progress(app: &AppHandle, payload: InstallProgressPayload) {
    let _ = app.emit("mod-install-progress", payload);
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GamebananaInstallProgressPayload {
    pub game_name: String,
    pub mod_name: String,
    pub current: u64,
    pub total: u64,
}

fn emit_gamebanana_install_progress(app: &AppHandle, payload: GamebananaInstallProgressPayload) {
    let _ = app.emit("gamebanana-install-progress", payload);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModArchiveFormat {
    Zip,
    SevenZip,
    Rar,
    SystemTar,
}

fn detect_mod_archive_format(path: &Path) -> Result<ModArchiveFormat, String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "Failed to open archive {}: {}",
            path.to_string_lossy(),
            error
        )
    })?;
    let mut header = [0u8; 8];
    let read = file.read(&mut header).map_err(|error| {
        format!(
            "Failed to read archive header {}: {}",
            path.to_string_lossy(),
            error
        )
    })?;
    let header = &header[..read];

    if header.starts_with(b"PK\x03\x04")
        || header.starts_with(b"PK\x05\x06")
        || header.starts_with(b"PK\x07\x08")
    {
        return Ok(ModArchiveFormat::Zip);
    }
    if header.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Ok(ModArchiveFormat::SevenZip);
    }
    if header.starts_with(b"Rar!\x1A\x07\x00") || header.starts_with(b"Rar!\x1A\x07\x01\x00") {
        return Ok(ModArchiveFormat::Rar);
    }

    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "zip" => Ok(ModArchiveFormat::Zip),
        "7z" => Ok(ModArchiveFormat::SevenZip),
        "rar" => Ok(ModArchiveFormat::Rar),
        // tar.exe is available on supported Windows releases and covers the
        // less common tar-based uploads that GameBanana authors sometimes use.
        "tar" | "gz" | "bz2" | "xz" | "tgz" | "tbz" | "tbz2" | "txz" => {
            Ok(ModArchiveFormat::SystemTar)
        }
        _ => Err(
            "Unsupported archive format. Expected ZIP, 7z, RAR, or a tar-compatible archive."
                .to_string(),
        ),
    }
}

fn extract_with_system_tar(path: &Path, staging_dir: &Path) -> Result<ExtractResult, String> {
    let output = std::process::Command::new("tar")
        .arg("-xf")
        .arg(path)
        .arg("-C")
        .arg(staging_dir)
        .output()
        .map_err(|error| {
            format!(
                "Failed to start tar fallback for {}: {}",
                path.to_string_lossy(),
                error
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "tar fallback failed for {}: {}",
            path.to_string_lossy(),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    let total = count_files_recursive(staging_dir)?;
    Ok(ExtractResult {
        processed: total,
        total,
    })
}

fn count_files_recursive(dir: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).map_err(|e| {
            format!(
                "Failed to read directory {}: {}",
                current.to_string_lossy(),
                e
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                total += 1;
            }
        }
    }

    Ok(total)
}

fn preview_mod_folder(path: &Path) -> Result<ArchivePreview, String> {
    let mut root_dirs = Vec::new();
    let mut has_ini = false;

    let direct_entries = fs::read_dir(path)
        .map_err(|e| format!("Failed to read folder {}: {}", path.to_string_lossy(), e))?;
    for entry in direct_entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            if let Some(name) = entry_path.file_name().and_then(|s| s.to_str()) {
                root_dirs.push(name.to_string());
            }
        }
    }
    root_dirs.sort();

    let file_count = count_files_recursive(path)? as usize;

    let mut stack = vec![path.to_path_buf()];
    'outer: while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).map_err(|e| {
            format!(
                "Failed to read directory {}: {}",
                current.to_string_lossy(),
                e
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("ini"))
                .unwrap_or(false)
            {
                has_ini = true;
                break 'outer;
            }
        }
    }

    Ok(ArchivePreview {
        root_dirs,
        file_count,
        has_ini,
        format: "folder".to_string(),
    })
}

fn is_ignorable_install_entry(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };

    name == "__MACOSX" || name == ".DS_Store"
}

fn resolve_install_source_dir(source_dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(source_dir).map_err(|e| {
        format!(
            "Failed to read folder {}: {}",
            source_dir.to_string_lossy(),
            e
        )
    })?;

    let mut effective_entries = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if is_ignorable_install_entry(&path) {
            continue;
        }
        effective_entries.push(path);
    }

    if effective_entries.len() == 1 && effective_entries[0].is_dir() {
        return Ok(effective_entries[0].clone());
    }

    Ok(source_dir.to_path_buf())
}

fn copy_folder_contents_with_progress<F>(
    source_dir: &Path,
    dest_dir: &Path,
    mut on_progress: F,
) -> Result<ExtractResult, String>
where
    F: FnMut(u64, u64),
{
    let total_files = count_files_recursive(source_dir)?;
    let mut processed = 0u64;

    fn copy_recursive<F>(
        src: &Path,
        dst: &Path,
        processed: &mut u64,
        total: u64,
        on_progress: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(u64, u64),
    {
        fs::create_dir_all(dst).map_err(|e| {
            format!(
                "Failed to create directory {}: {}",
                dst.to_string_lossy(),
                e
            )
        })?;

        let entries = fs::read_dir(src)
            .map_err(|e| format!("Failed to read directory {}: {}", src.to_string_lossy(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                copy_recursive(&src_path, &dst_path, processed, total, on_progress)?;
            } else if src_path.is_file() {
                fs::copy(&src_path, &dst_path).map_err(|e| {
                    format!(
                        "Failed to copy file {} to {}: {}",
                        src_path.to_string_lossy(),
                        dst_path.to_string_lossy(),
                        e
                    )
                })?;

                *processed += 1;
                on_progress(*processed, total.max(1));
            }
        }

        Ok(())
    }

    copy_recursive(
        source_dir,
        dest_dir,
        &mut processed,
        total_files,
        &mut on_progress,
    )?;

    Ok(ExtractResult {
        processed,
        total: total_files,
    })
}

fn normalize_install_relative_path(
    value: &str,
    field_name: &str,
    allow_nested: bool,
    allow_empty: bool,
) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        if allow_empty {
            return Ok(PathBuf::new());
        }
        return Err(format!("{} cannot be empty", field_name));
    }

    let normalized = trimmed.replace('\\', "/");
    if normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
        return Err(format!("{} must be a relative path", field_name));
    }

    let mut output = PathBuf::new();
    let mut segment_count = 0usize;
    for segment in normalized.split('/') {
        let segment = segment.trim();
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains(':') {
            return Err(format!("{} contains an unsafe path segment", field_name));
        }
        segment_count += 1;
        if !allow_nested && segment_count > 1 {
            return Err(format!("{} cannot contain path separators", field_name));
        }
        output.push(segment);
    }

    if segment_count == 0 {
        if allow_empty {
            return Ok(PathBuf::new());
        }
        return Err(format!("{} cannot be empty", field_name));
    }

    Ok(output)
}

fn create_install_staging_dir(staging_parent: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(staging_parent).map_err(|e| {
        format!(
            "Failed to create install parent {}: {}",
            staging_parent.to_string_lossy(),
            e
        )
    })?;

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    for attempt in 0..20 {
        let staging_dir = staging_parent.join(format!(
            ".ssmt-installing-{}-{}-{}",
            std::process::id(),
            millis,
            attempt
        ));
        if staging_dir.exists() {
            continue;
        }
        fs::create_dir(&staging_dir).map_err(|e| {
            format!(
                "Failed to create install staging directory {}: {}",
                staging_dir.to_string_lossy(),
                e
            )
        })?;
        return Ok(staging_dir);
    }

    Err("Failed to allocate install staging directory".to_string())
}

fn remove_install_staging_dir(staging_dir: &Path) {
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(staging_dir);
    }
}

fn rename_install_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    for attempt in 0..4 {
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 3 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(Duration::from_millis(120 * (attempt + 1)));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn finalize_install_directory(staging: &Path, destination: &Path) -> Result<(), String> {
    if destination.try_exists().map_err(|e| e.to_string())? {
        return Err("MOD_IMPORT_CONFLICT".into());
    }
    match rename_install_path(staging, destination) {
        Ok(()) => return Ok(()),
        Err(error)
            if matches!(error.raw_os_error(), Some(5 | 32 | 33)) && !destination.exists() =>
        {
            // Windows 允许读取但不允许重命名被占用的目录；仅向本次独占创建的空目标复制。
            // 目标已存在时绝不借此绕过冲突确认。
            let diagnostic = super::file_locks::describe_install_access(&[staging.to_path_buf()]);
            eprintln!("[ModImport] 目录移动受阻，尝试复制回退：{error}\n{diagnostic}");
            fs::create_dir(destination).map_err(|copy_error| {
                format!(
                    "移动目录失败：{error}；创建复制目标 {} 也失败：{copy_error}",
                    destination.display()
                )
            })?;
            if let Err(copy_error) = merge_install_tree(staging, destination) {
                let diagnostic = super::file_locks::describe_install_access(&[
                    staging.to_path_buf(),
                    destination.to_path_buf(),
                ]);
                let cleanup = fs::remove_dir_all(destination);
                return Err(format!("移动目录失败：{error}；复制回退失败：{copy_error}\n{diagnostic}\n目标清理结果：{cleanup:?}"));
            }
            remove_install_staging_dir(staging);
            return Ok(());
        }
        Err(error) => Err(format!(
            "移动目录 {} -> {} 失败：{error}",
            staging.display(),
            destination.display()
        )),
    }
}

// 直接写入只在完整解压成功后执行；禁止沿目标目录中的链接写出 Mod 目录。
fn merge_install_tree(source: &Path, destination: &Path) -> Result<(), String> {
    if fs::symlink_metadata(destination)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!("不能直接写入链接路径：{}", destination.display()));
    }
    fs::create_dir_all(destination)
        .map_err(|e| format!("无法创建目录 {}: {e}", destination.display()))?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = destination.join(entry.file_name());
        if fs::symlink_metadata(&target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!("不能覆盖链接路径：{}", target.display()));
        }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            merge_install_tree(&entry.path(), &target)?;
        } else {
            #[cfg(windows)]
            if target.is_file() {
                let mut permissions = fs::metadata(&target)
                    .map_err(|e| e.to_string())?
                    .permissions();
                if permissions.readonly() {
                    permissions.set_readonly(false);
                    fs::set_permissions(&target, permissions).map_err(|e| e.to_string())?;
                }
            }
            fs::copy(entry.path(), &target).map_err(|e| {
                format!(
                    "无法写入 {}: {e}；请检查文件占用或目录权限",
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn backup_existing_install_dir(dest_dir: &Path) -> Result<PathBuf, String> {
    let parent = dest_dir
        .parent()
        .ok_or_else(|| "Install destination has no parent directory".to_string())?;
    let original_name = dest_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Install destination has an invalid directory name".to_string())?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    for attempt in 0..100 {
        // `_bak` identifies an SSMT-managed, recoverable backup; `DISABLED_`
        // is the separate 3Dmigoto-facing marker that keeps its INI files from
        // being loaded without modifying d3dx.ini.
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{}", attempt)
        };
        let backup = parent.join(format!(
            "DISABLED_{}-{}{}_bak",
            original_name, millis, suffix
        ));
        if backup.exists() {
            continue;
        }
        rename_install_path(dest_dir, &backup).map_err(|error| {
            format!(
                "Failed to move existing Mod into the 3Dmigoto-safe backup {}: {}",
                backup.to_string_lossy(),
                error,
            )
        })?;
        return Ok(backup);
    }

    Err("Failed to allocate a unique backup directory name".to_string())
}

fn is_allowed_remote_preview_url(url: &reqwest::Url, source: &str) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url
            .host_str()
            .map(|host| {
                let host = host.to_ascii_lowercase();
                match source {
                    "gamebanana" => host == "gamebanana.com" || host.ends_with(".gamebanana.com"),
                    "nexusmods" => host == "nexusmods.com" || host.ends_with(".nexusmods.com"),
                    _ => false,
                }
            })
            .unwrap_or(false)
}

fn remote_preview_extension(path: &str, content_type: Option<&str>) -> String {
    let by_path = Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .map(|extension| extension.to_ascii_lowercase());
    if let Some(extension) = by_path.as_deref() {
        if matches!(
            extension,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "avif"
        ) {
            return if extension == "jpeg" {
                "jpg".to_string()
            } else {
                extension.to_string()
            };
        }
    }

    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.contains("png") {
        return "png".to_string();
    }
    if content_type.contains("webp") {
        return "webp".to_string();
    }
    if content_type.contains("gif") {
        return "gif".to_string();
    }
    if content_type.contains("bmp") {
        return "bmp".to_string();
    }
    if content_type.contains("avif") {
        return "avif".to_string();
    }
    "jpg".to_string()
}

fn next_remote_preview_path(
    mod_dir: &Path,
    source: &str,
    ordinal: usize,
    extension: &str,
) -> PathBuf {
    let stem = format!("00_preview_{source}_{ordinal:03}");
    let initial = mod_dir.join(format!("{stem}.{extension}"));
    if !initial.exists() {
        return initial;
    }

    for attempt in 2..=1000 {
        let candidate = mod_dir.join(format!("{stem}-{attempt}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    mod_dir.join(format!(
        "{stem}-{}.{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        extension
    ))
}

async fn save_remote_previews(
    mod_dir: PathBuf,
    preview_urls: Vec<String>,
    source: &str,
) -> Result<(usize, Vec<String>), String> {
    if preview_urls.is_empty() {
        return Ok((0, Vec::new()));
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("SSMT4 {source} preview downloader"))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("Failed to configure {source} preview downloader: {error}"))?;
    let mut seen = HashSet::new();
    let mut saved_count = 0usize;
    let mut warnings = Vec::new();

    for raw_url in preview_urls {
        let preview_url = raw_url.trim();
        if preview_url.is_empty() || !seen.insert(preview_url.to_ascii_lowercase()) {
            continue;
        }

        let parsed_url = match reqwest::Url::parse(preview_url) {
            Ok(url) if is_allowed_remote_preview_url(&url, source) => url,
            Ok(url) => {
                warnings.push(format!("Skipped preview from an unsupported host: {url}"));
                continue;
            }
            Err(error) => {
                warnings.push(format!("Skipped invalid preview URL: {error}"));
                continue;
            }
        };

        let response = match client.get(parsed_url.clone()).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                warnings.push(format!(
                    "Preview download returned HTTP {}",
                    response.status()
                ));
                continue;
            }
            Err(error) => {
                warnings.push(format!("Preview download failed: {error}"));
                continue;
            }
        };

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = match response.bytes().await {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => {
                warnings.push("Preview download returned an empty image".to_string());
                continue;
            }
            Err(error) => {
                warnings.push(format!("Failed to read preview image: {error}"));
                continue;
            }
        };

        let extension = remote_preview_extension(parsed_url.path(), content_type.as_deref());
        let target = next_remote_preview_path(&mod_dir, source, saved_count + 1, &extension);
        if let Err(error) = tokio::fs::write(&target, bytes).await {
            warnings.push(format!(
                "Failed to save preview {}: {}",
                target.display(),
                error
            ));
            continue;
        }
        saved_count += 1;
    }

    Ok((saved_count, warnings))
}

fn spawn_gamebanana_preview_fetch(mod_dir: PathBuf, preview_urls: Vec<String>) {
    if preview_urls.is_empty() {
        return;
    }

    tauri::async_runtime::spawn(async move {
        match save_remote_previews(mod_dir.clone(), preview_urls, "gamebanana").await {
            Ok((saved_count, warnings)) if warnings.is_empty() => {
                println!(
                    "[GameBanana] Saved {} preview image(s) for {}",
                    saved_count,
                    mod_dir.display()
                );
            }
            Ok((saved_count, warnings)) => {
                eprintln!(
                    "[GameBanana] Saved {} preview image(s) for {} with warnings: {}",
                    saved_count,
                    mod_dir.display(),
                    warnings.join(" | ")
                );
            }
            Err(error) => {
                eprintln!(
                    "[GameBanana] Failed to save preview images for {}: {}",
                    mod_dir.display(),
                    error
                );
            }
        }
    });
}

fn ensure_gamebanana_mod_marker(mod_dir: &Path) -> Result<(), String> {
    let marker_path = mod_dir.join("thisisa.mod");
    fs::write(&marker_path, b"# SSMT GameBanana Mod Marker\n").map_err(|error| {
        format!(
            "Failed to write GameBanana Mod marker {}: {}",
            marker_path.to_string_lossy(),
            error
        )
    })
}

fn spawn_nexusmods_preview_fetch(mod_dir: PathBuf, preview_urls: Vec<String>) {
    if preview_urls.is_empty() {
        return;
    }

    tauri::async_runtime::spawn(async move {
        match save_remote_previews(mod_dir.clone(), preview_urls, "nexusmods").await {
            Ok((saved_count, warnings)) if warnings.is_empty() => {
                println!(
                    "[NexusMods] Saved {} preview image(s) for {}",
                    saved_count,
                    mod_dir.display()
                );
            }
            Ok((saved_count, warnings)) => {
                eprintln!(
                    "[NexusMods] Saved {} preview image(s) for {} with warnings: {}",
                    saved_count,
                    mod_dir.display(),
                    warnings.join(" | ")
                );
            }
            Err(error) => {
                eprintln!(
                    "[NexusMods] Failed to save preview images for {}: {}",
                    mod_dir.display(),
                    error
                );
            }
        }
    });
}

#[cfg(test)]
mod install_path_tests {
    use super::normalize_install_relative_path;
    use std::path::PathBuf;

    #[cfg(windows)]
    #[test]
    fn finalize_falls_back_to_copy_when_open_file_prevents_directory_rename() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!(
            "ssmt-locked-install-{}-{}",
            std::process::id(),
            super::SystemTime::now()
                .duration_since(super::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("mod.ini"), b"content").unwrap();
        // 允许其他读取、写入，但不共享删除权，模拟索引器或扫描器的短暂句柄。
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(source.join("mod.ini"))
            .unwrap();
        assert!(std::fs::rename(&source, &target).is_err());
        super::finalize_install_directory(&source, &target).unwrap();
        assert_eq!(std::fs::read(target.join("mod.ini")).unwrap(), b"content");
        drop(handle);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finalize_does_not_overwrite_existing_destination() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-conflict-install-{}-{}",
            std::process::id(),
            super::SystemTime::now()
                .duration_since(super::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.ini"), b"keep").unwrap();
        assert!(super::finalize_install_directory(&source, &target).is_err());
        assert_eq!(std::fs::read(target.join("keep.ini")).unwrap(), b"keep");
        assert!(source.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn merge_install_preserves_other_files_and_overwrites_matching_files() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-merge-{}-{}",
            std::process::id(),
            super::SystemTime::now()
                .duration_since(super::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("source");
        let target = root.join("target");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::create_dir_all(target.join("nested")).unwrap();
        std::fs::write(source.join("nested/mod.ini"), b"new").unwrap();
        std::fs::write(target.join("nested/mod.ini"), b"old").unwrap();
        std::fs::write(target.join("keep.ini"), b"keep").unwrap();
        #[cfg(windows)]
        {
            let mut permissions = std::fs::metadata(target.join("nested/mod.ini"))
                .unwrap()
                .permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(target.join("nested/mod.ini"), permissions).unwrap();
        }
        super::merge_install_tree(&source, &target).unwrap();
        assert_eq!(
            std::fs::read(target.join("nested/mod.ini")).unwrap(),
            b"new"
        );
        assert_eq!(std::fs::read(target.join("keep.ini")).unwrap(), b"keep");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn install_group_path_allows_nested_relative_groups() {
        let path = normalize_install_relative_path("Characters/Ayaka", "Target group", true, false)
            .expect("nested group should be accepted");

        assert_eq!(path, PathBuf::from("Characters").join("Ayaka"));
    }

    #[test]
    fn install_mod_name_rejects_nested_or_escape_paths() {
        assert!(normalize_install_relative_path("Ayaka/Alt", "Mod name", false, false).is_err());
        assert!(normalize_install_relative_path("../Alt", "Mod name", false, false).is_err());
        assert!(normalize_install_relative_path("C:/Alt", "Mod name", false, false).is_err());
    }
}

#[tauri::command]
pub async fn preview_mod_archive(
    path: String,
    password: Option<String>,
) -> Result<ArchivePreview, String> {
    let path_buf = PathBuf::from(&path);

    if path_buf.is_dir() {
        return preview_mod_folder(&path_buf);
    }

    SSMTCompressUtils::preview_archive_with_password(&path_buf, password.as_deref())
}

#[tauri::command]
pub async fn mod_install_target_exists(
    install_dir: String,
    target_name: String,
    target_group: String,
) -> Result<bool, String> {
    let target_name_path = normalize_install_relative_path(&target_name, "Mod name", false, false)?;
    let target_group_path =
        if target_group.trim().eq_ignore_ascii_case("Root") || target_group.trim().is_empty() {
            PathBuf::new()
        } else {
            normalize_install_relative_path(target_group.trim(), "Target group", true, false)?
        };
    Ok(install_target_exists(
        &mods_root(&install_dir),
        &target_group_path,
        &target_name_path,
    ))
}

#[tauri::command]
pub async fn install_mod_archive(
    app: AppHandle,
    game_name: String,
    install_dir: String,
    archive_path: String,
    target_name: String,  // User defined name for the folder
    target_group: String, // E.g. "Ayaka", or "Root"
    password: Option<String>,
    backup_existing: Option<bool>,
    conflict_action: Option<String>,
) -> Result<(), String> {
    let mods_dir = mods_root(&install_dir);
    let target_name_path = normalize_install_relative_path(&target_name, "Mod name", false, false)?;
    let target_group_trimmed = target_group.trim();
    let target_group_path =
        if target_group_trimmed.eq_ignore_ascii_case("Root") || target_group_trimmed.is_empty() {
            PathBuf::new()
        } else {
            normalize_install_relative_path(target_group_trimmed, "Target group", true, false)?
        };
    let target_parent = resolve_physical_install_group_path(&mods_dir, &target_group_path);
    let dest_dir = target_parent.join(&target_name_path);

    let action = conflict_action.as_deref().unwrap_or("ask");
    if !matches!(action, "ask" | "replace" | "merge") {
        return Err("无效的同名目标处理方式".into());
    }
    if dest_dir.exists() && action == "ask" && !backup_existing.unwrap_or(false) {
        return Err("MOD_IMPORT_CONFLICT".into());
    }

    let path_buf = PathBuf::from(&archive_path);
    if !path_buf.exists() {
        return Err(format!(
            "Source path not found: {}",
            path_buf.to_string_lossy()
        ));
    }

    let archive_format = if path_buf.is_dir() {
        None
    } else {
        Some(detect_mod_archive_format(&path_buf)?)
    };

    emit_install_progress(
        &app,
        InstallProgressPayload {
            game_name: game_name.clone(),
            mod_name: target_name.clone(),
            stage: "analyzing".to_string(),
            current: 0,
            total: 0,
        },
    );

    let staging_parent = mods_dir.join(".ssmt").join("install-staging");
    let staging_dir = create_install_staging_dir(&staging_parent)?;
    let extract_result = (|| -> Result<ExtractResult, String> {
        if path_buf.is_dir() {
            let effective_source_dir = resolve_install_source_dir(&path_buf)?;
            copy_folder_contents_with_progress(
                &effective_source_dir,
                &staging_dir,
                |current, total| {
                    emit_install_progress(
                        &app,
                        InstallProgressPayload {
                            game_name: game_name.clone(),
                            mod_name: target_name.clone(),
                            stage: "copying".to_string(),
                            current,
                            total,
                        },
                    );
                },
            )
        } else if archive_format == Some(ModArchiveFormat::Zip) {
            SSMTCompressUtils::extract_zip_archive_with_password(
                &path_buf,
                &staging_dir,
                password.as_deref(),
                |current, total| {
                    emit_install_progress(
                        &app,
                        InstallProgressPayload {
                            game_name: game_name.clone(),
                            mod_name: target_name.clone(),
                            stage: "extracting".to_string(),
                            current,
                            total,
                        },
                    );
                },
            )
        } else if archive_format == Some(ModArchiveFormat::SevenZip) {
            SSMTCompressUtils::extract_7z_archive_with_password(
                &path_buf,
                &staging_dir,
                password.as_deref(),
                |current, total| {
                    emit_install_progress(
                        &app,
                        InstallProgressPayload {
                            game_name: game_name.clone(),
                            mod_name: target_name.clone(),
                            stage: "extracting".to_string(),
                            current,
                            total,
                        },
                    );
                },
            )
        } else if archive_format == Some(ModArchiveFormat::Rar) {
            SSMTCompressUtils::extract_rar_archive_with_password(
                &path_buf,
                &staging_dir,
                password.as_deref(),
                |current, total| {
                    emit_install_progress(
                        &app,
                        InstallProgressPayload {
                            game_name: game_name.clone(),
                            mod_name: target_name.clone(),
                            stage: "extracting".to_string(),
                            current,
                            total,
                        },
                    );
                },
            )
        } else if archive_format == Some(ModArchiveFormat::SystemTar) {
            emit_install_progress(
                &app,
                InstallProgressPayload {
                    game_name: game_name.clone(),
                    mod_name: target_name.clone(),
                    stage: "extracting".to_string(),
                    current: 0,
                    total: 1,
                },
            );
            let result = extract_with_system_tar(&path_buf, &staging_dir)?;
            emit_install_progress(
                &app,
                InstallProgressPayload {
                    game_name: game_name.clone(),
                    mod_name: target_name.clone(),
                    stage: "extracting".to_string(),
                    current: result.total.max(1),
                    total: result.total.max(1),
                },
            );
            Ok(result)
        } else {
            Err("Unsupported archive format".to_string())
        }
    })();

    let extract_result = match extract_result {
        Ok(value) => value,
        Err(error) => {
            let diagnostic = if error.contains("os error 5")
                || error.contains("os error 32")
                || error.contains("拒绝访问")
                || error.to_lowercase().contains("access is denied")
            {
                super::file_locks::describe_install_access(&[path_buf.clone(), staging_dir.clone()])
            } else {
                String::new()
            };
            remove_install_staging_dir(&staging_dir);
            return Err(format!(
                "解压 {} 到 {} 失败：{error}\n{diagnostic}",
                path_buf.display(),
                staging_dir.display()
            ));
        }
    };

    fs::create_dir_all(&target_parent).map_err(|e| {
        remove_install_staging_dir(&staging_dir);
        format!(
            "Failed to create install target parent {}: {}",
            target_parent.to_string_lossy(),
            e
        )
    })?;

    if dest_dir.exists() && action == "merge" {
        let result = merge_install_tree(&staging_dir, &dest_dir);
        remove_install_staging_dir(&staging_dir);
        result.map_err(|error| format!("直接写入未完成，部分文件可能已更新：{error}"))?;
        emit_install_progress(
            &app,
            InstallProgressPayload {
                game_name,
                mod_name: target_name,
                stage: "done".into(),
                current: extract_result.total.max(1),
                total: extract_result.total.max(1),
            },
        );
        return Ok(());
    }

    let backup_path = if dest_dir.exists() {
        match backup_existing.unwrap_or(false) || action == "replace" {
            true => match backup_existing_install_dir(&dest_dir) {
                Ok(path) => Some(path),
                Err(error) => {
                    remove_install_staging_dir(&staging_dir);
                    return Err(error);
                }
            },
            false => {
                remove_install_staging_dir(&staging_dir);
                return Err("MOD_IMPORT_CONFLICT".into());
            }
        }
    } else {
        None
    };

    finalize_install_directory(&staging_dir, &dest_dir).map_err(|e| {
        let diagnostic = super::file_locks::describe_install_access(&[
            staging_dir.clone(),
            target_parent.clone(),
        ]);
        remove_install_staging_dir(&staging_dir);
        let recovery = if let Some(backup) = backup_path.as_ref() {
            match rename_install_path(backup, &dest_dir) {
                Ok(()) => "原目录已恢复。".to_string(),
                Err(restore_error) => format!(
                    "恢复原目录失败，原内容仍保留在 {}：{restore_error}",
                    backup.display()
                ),
            }
        } else {
            String::new()
        };
        format!(
            "Failed to finalize install {} -> {}: {}\n{}\n{}",
            staging_dir.to_string_lossy(),
            dest_dir.to_string_lossy(),
            e,
            diagnostic,
            recovery
        )
    })?;

    let ExtractResult {
        processed: processed_entries,
        total: total_entries,
    } = extract_result;

    // Final done event
    emit_install_progress(
        &app,
        InstallProgressPayload {
            game_name,
            mod_name: target_name,
            stage: "done".to_string(),
            current: processed_entries.max(total_entries),
            total: total_entries.max(1),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn gamebanana_download_and_install_mod(
    app: AppHandle,
    game_name: String,
    install_dir: String,
    download_url: String,
    archive_name: String,
    target_name: String,
    target_group: String,
    password: Option<String>,
    preview_urls: Option<Vec<String>>,
    expected_size_bytes: Option<u64>,
) -> Result<(), String> {
    let cancellation_guard = GamebananaDownloadCancellationGuard::begin(&game_name, &target_name);
    let cancellation_key = cancellation_guard.key.clone();
    let parsed_url = reqwest::Url::parse(download_url.trim())
        .map_err(|error| format!("Invalid GameBanana download URL: {}", error))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err("GameBanana download URL must use HTTP or HTTPS".to_string());
    }

    let download_dir = std::env::temp_dir().join("ssmt4-gamebanana-downloads");
    tokio::fs::create_dir_all(&download_dir)
        .await
        .map_err(|error| format!("Failed to create GameBanana download cache: {}", error))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let extension = Path::new(archive_name.trim())
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            value.len() <= 12 && value.chars().all(|value| value.is_ascii_alphanumeric())
        })
        .map(|value| format!(".{}", value))
        .unwrap_or_else(|| ".download".to_string());
    let archive_path = download_dir.join(format!(
        "gamebanana-{}-{}-{}{}",
        std::process::id(),
        stamp,
        target_name
            .chars()
            .filter(|value| value.is_ascii_alphanumeric())
            .take(20)
            .collect::<String>(),
        extension,
    ));

    emit_gamebanana_install_progress(
        &app,
        GamebananaInstallProgressPayload {
            game_name: game_name.clone(),
            mod_name: target_name.clone(),
            current: 0,
            total: 0,
        },
    );

    let download_result = async {
        if gamebanana_download_is_cancelled(&cancellation_key) {
            return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
        }
        let client = reqwest::Client::builder()
            .user_agent("SSMT4 GameBanana installer")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| format!("Failed to configure GameBanana downloader: {}", error))?;
        if gamebanana_download_is_cancelled(&cancellation_key) {
            return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
        }
        let mut response = client
            .get(parsed_url)
            .send()
            .await
            .map_err(|error| format!("GameBanana download request failed: {}", error))?;
        if !response.status().is_success() {
            return Err(format!(
                "GameBanana download returned HTTP {}",
                response.status()
            ));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if content_type.contains("text/html") || content_type.contains("application/json") {
            return Err(format!(
                "Mod download returned an unexpected response ({}) instead of an archive",
                content_type,
            ));
        }

        let total = response.content_length().unwrap_or(0);
        let mut output = tokio::fs::File::create(&archive_path)
            .await
            .map_err(|error| format!("Failed to create download file: {}", error))?;
        let mut downloaded = 0u64;
        let mut last_progress_at = Instant::now();
        loop {
            if gamebanana_download_is_cancelled(&cancellation_key) {
                return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
            }
            let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| format!("Failed to read GameBanana download: {}", error))?
            else {
                break;
            };
            if gamebanana_download_is_cancelled(&cancellation_key) {
                return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|error| format!("Failed to save GameBanana download: {}", error))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if last_progress_at.elapsed() >= Duration::from_millis(80) {
                emit_gamebanana_install_progress(
                    &app,
                    GamebananaInstallProgressPayload {
                        game_name: game_name.clone(),
                        mod_name: target_name.clone(),
                        current: downloaded,
                        total,
                    },
                );
                last_progress_at = Instant::now();
            }
        }
        if gamebanana_download_is_cancelled(&cancellation_key) {
            return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
        }
        output
            .flush()
            .await
            .map_err(|error| format!("Failed to finalize GameBanana download: {}", error))?;
        if let Some(expected_size) = expected_size_bytes.filter(|value| *value > 0) {
            // A CDN error page can still arrive with HTTP 200.  Prevent a few
            // kilobytes of HTML from being installed in place of a real archive.
            let minimum_expected_size = expected_size.saturating_mul(80) / 100;
            if downloaded < minimum_expected_size {
                return Err(format!(
                    "Downloaded archive is unexpectedly small ({} bytes; expected about {} bytes)",
                    downloaded, expected_size,
                ));
            }
        }
        emit_gamebanana_install_progress(
            &app,
            GamebananaInstallProgressPayload {
                game_name: game_name.clone(),
                mod_name: target_name.clone(),
                current: downloaded,
                total: total.max(downloaded),
            },
        );
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = download_result {
        let _ = tokio::fs::remove_file(&archive_path).await;
        return Err(error);
    }

    if gamebanana_download_is_cancelled(&cancellation_key) {
        let _ = tokio::fs::remove_file(&archive_path).await;
        return Err(GAMEBANANA_DOWNLOAD_CANCELLED.to_string());
    }

    let preview_target = {
        let mods_dir = mods_root(&install_dir);
        let target_name_path =
            normalize_install_relative_path(&target_name, "Mod name", false, false)?;
        let target_group_path =
            if target_group.trim().eq_ignore_ascii_case("Root") || target_group.trim().is_empty() {
                PathBuf::new()
            } else {
                normalize_install_relative_path(target_group.trim(), "Target group", true, false)?
            };
        let physical_group = resolve_physical_install_group_path(&mods_dir, &target_group_path);
        let target = physical_group.join(&target_name_path);
        println!(
            "[GameBanana] install target logical_group={} physical_group={} target={}",
            target_group,
            physical_group.display(),
            target.display(),
        );
        target
    };

    let install_result = install_mod_archive(
        app,
        game_name,
        install_dir,
        archive_path.to_string_lossy().to_string(),
        target_name,
        target_group,
        password,
        Some(true),
        None,
    )
    .await;
    let _ = tokio::fs::remove_file(&archive_path).await;
    install_result?;
    ensure_gamebanana_mod_marker(&preview_target)?;
    spawn_gamebanana_preview_fetch(preview_target, preview_urls.unwrap_or_default());
    Ok(())
}

#[tauri::command]
pub fn cancel_gamebanana_download_and_install_mod(
    game_name: String,
    target_name: String,
) -> Result<bool, String> {
    let key = gamebanana_download_key(&game_name, &target_name);
    let mut jobs = gamebanana_download_cancellations()
        .lock()
        .map_err(|_| "GameBanana download cancellation state is unavailable".to_string())?;
    let Some(cancelled) = jobs.get_mut(&key) else {
        return Ok(false);
    };
    *cancelled = true;
    Ok(true)
}

/// Nexus Mods supplies short-lived CDN download URLs.  The archive handling,
/// backup convention, and streamed installation path are shared with the
/// GameBanana installer, while the fixed source group and preview naming keep
/// the resulting local Mod tree independent at `Mods/NexusMods`.
#[tauri::command]
pub async fn nexusmods_download_and_install_mod(
    app: AppHandle,
    game_name: String,
    install_dir: String,
    download_url: String,
    archive_name: String,
    target_name: String,
    target_group: String,
    password: Option<String>,
    expected_size_bytes: Option<u64>,
    preview_urls: Option<Vec<String>>,
) -> Result<(), String> {
    if !target_group.trim().eq_ignore_ascii_case("NexusMods") {
        return Err("Nexus Mods installs must target the NexusMods group".to_string());
    }

    let preview_target = {
        let mods_dir = mods_root(&install_dir);
        let target_name_path =
            normalize_install_relative_path(&target_name, "Mod name", false, false)?;
        resolve_physical_install_group_path(&mods_dir, Path::new("NexusMods"))
            .join(target_name_path)
    };

    match gamebanana_download_and_install_mod(
        app,
        game_name,
        install_dir,
        download_url,
        archive_name,
        target_name,
        "NexusMods".to_string(),
        password,
        None,
        expected_size_bytes,
    )
    .await
    {
        Ok(()) => {
            spawn_nexusmods_preview_fetch(preview_target, preview_urls.unwrap_or_default());
            Ok(())
        }
        Err(error) if error == GAMEBANANA_DOWNLOAD_CANCELLED => {
            Err("Nexus Mods download cancelled".to_string())
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub fn cancel_nexusmods_download_and_install_mod(
    game_name: String,
    target_name: String,
) -> Result<bool, String> {
    cancel_gamebanana_download_and_install_mod(game_name, target_name)
}

#[tauri::command]
pub async fn export_mod_archive(
    install_dir: String,
    mod_relative_path: String,
    output_dir: String,
    archive_name: String,
    format: String,
    password: Option<String>,
) -> Result<String, String> {
    let archive_name = archive_name.trim();
    if !archive_file_name_is_valid(archive_name) {
        return Err("Archive name contains invalid characters".to_string());
    }

    let format = format.trim().to_lowercase();
    if !matches!(format.as_str(), "zip" | "7z" | "rar") {
        return Err(format!("Unsupported archive format: {}", format));
    }

    let source_dir = resolve_existing_mod_dir(&install_dir, &mod_relative_path)?;
    let output_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;
    let output_path = output_dir.join(format!("{}.{}", archive_name, format));

    SSMTCompressUtils::create_mod_archive(&source_dir, &output_path, &format, password.as_deref())?;

    Ok(output_path.to_string_lossy().replace('\\', "/"))
}
