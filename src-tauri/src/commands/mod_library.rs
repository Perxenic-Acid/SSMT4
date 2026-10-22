use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::fs;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

macro_rules! log_scan {
    ($($arg:tt)*) => {
        eprintln!("[SCAN] {}", format!($($arg)*))
    };
}

pub struct ModLibraryWatcher(pub Mutex<Option<RecommendedWatcher>>);

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub path: String,
    pub relative_path: String,
    pub preview_images: Vec<String>,
    pub group: String,
    pub is_dir: bool,
    pub last_modified: u64,
    pub is_directory_link: bool,
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
    pub is_directory_link: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub mods: Vec<ModInfo>,
    pub groups: Vec<GroupInfo>,
}

fn upsert_group(groups: &mut Vec<GroupInfo>, group: GroupInfo) {
    // 启用与禁用目录会归一成同一个逻辑 ID；视图只保留启用版本，避免节点重复。
    let Some(existing) = groups.iter_mut().find(|item| item.id == group.id) else {
        groups.push(group);
        return;
    };

    let is_directory_link = existing.is_directory_link || group.is_directory_link;
    if group.enabled && !existing.enabled {
        let icon_path = group.icon_path.clone().or_else(|| existing.icon_path.clone());
        *existing = GroupInfo {
            icon_path,
            is_directory_link,
            ..group
        };
    } else {
        if group.enabled == existing.enabled {
            existing.mod_count = existing.mod_count.max(group.mod_count);
        }
        existing.is_directory_link = is_directory_link;
        if existing.icon_path.is_none() {
            existing.icon_path = group.icon_path;
        }
    }
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

fn modified_unix_seconds(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn normalize_group_path(value: &str) -> String {
    let normalized = normalize_slashes(value)
        .trim()
        .trim_matches('/')
        .to_string();
    if normalized == "Root" || normalized == "All" {
        String::new()
    } else {
        normalized
    }
}

fn group_cache_key(group_id: &str) -> String {
    let normalized = normalize_group_id(group_id);
    if normalized.is_empty() {
        "Root".to_string()
    } else {
        normalized
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
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

fn db_path(install_dir: &str) -> Result<PathBuf, String> {
    let dir = mods_root(install_dir).join(".ssmt");
    fs::create_dir_all(&dir).map_err(|error| {
        format!(
            "Failed to create mod library dir {}: {}",
            dir.display(),
            error
        )
    })?;
    Ok(dir.join("mod-library.sqlite"))
}

fn open_db(install_dir: &str) -> Result<Connection, String> {
    let conn = Connection::open(db_path(install_dir)?).map_err(|error| error.to_string())?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS groups (
            game TEXT NOT NULL,
            id TEXT NOT NULL,
            parent TEXT NOT NULL,
            name TEXT NOT NULL,
            icon_path TEXT,
            path TEXT NOT NULL,
            enabled INTEGER NOT NULL,
            mod_count INTEGER NOT NULL DEFAULT 0,
            is_directory_link INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (game, id)
        );

        CREATE INDEX IF NOT EXISTS idx_groups_parent ON groups(game, parent);

        CREATE TABLE IF NOT EXISTS mods (
            game TEXT NOT NULL,
            id TEXT NOT NULL,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL,
            path TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            group_path TEXT NOT NULL,
            is_dir INTEGER NOT NULL,
            last_modified INTEGER NOT NULL DEFAULT 0,
            is_directory_link INTEGER NOT NULL DEFAULT 0,
            preview_images_json TEXT NOT NULL DEFAULT '[]',
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (game, id)
        );

        CREATE INDEX IF NOT EXISTS idx_mods_group ON mods(game, group_path);
        CREATE INDEX IF NOT EXISTS idx_mods_name ON mods(game, name);

        CREATE TABLE IF NOT EXISTS scan_state (
            game TEXT NOT NULL,
            group_path TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (game, group_path)
        );
        "#,
    )
    .map_err(|error| error.to_string())?;

    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    if user_version < 2 {
        conn.execute_batch(
            r#"
            DELETE FROM mods;
            DELETE FROM groups;
            DELETE FROM scan_state;
            PRAGMA user_version = 2;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }
    if user_version < 3 {
        // New databases already include these columns; existing databases need
        // the migration before cached scan results can carry link metadata.
        let _ = conn.execute(
            "ALTER TABLE groups ADD COLUMN is_directory_link INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE mods ADD COLUMN is_directory_link INTEGER NOT NULL DEFAULT 0",
            [],
        );
        conn.execute_batch(
            r#"
            DELETE FROM mods;
            DELETE FROM groups;
            DELETE FROM scan_state;
            PRAGMA user_version = 3;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }
    if user_version < 6 {
        // Mod counts now count visible top-level entries instead of every
        // internal INI directory inside modular Mods. Rebuild once more so
        // enabled groups do not inherit counts from DISABLED_ duplicates.
        conn.execute_batch(
            r#"
            DELETE FROM mods;
            DELETE FROM groups;
            DELETE FROM scan_state;
            PRAGMA user_version = 6;
            "#,
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(conn)
}

fn strip_disabled_prefix(name: &str) -> (String, bool) {
    let upper = name.to_uppercase();
    if upper.starts_with("DISABLED_") {
        (name[9..].to_string(), true)
    } else if upper.starts_with("DISABLED") {
        (name[8..].to_string(), true)
    } else {
        (name.to_string(), false)
    }
}

fn is_directory_link(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

fn path_uses_directory_link(root: &Path, physical_group: &str) -> bool {
    if is_directory_link(root) {
        return true;
    }
    let mut current = root.to_path_buf();
    for segment in physical_group
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        current.push(segment);
        if is_directory_link(&current) {
            return true;
        }
    }
    false
}

fn is_managed_backup_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("disabled_") && lower.ends_with("_bak")
}

fn normalize_group_id(value: &str) -> String {
    normalize_group_path(value)
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let (clean, _) = strip_disabled_prefix(segment);
            if clean.is_empty() {
                segment.to_string()
            } else {
                clean
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn find_physical_segment(parent: &Path, logical_segment: &str) -> Option<String> {
    let entries = fs::read_dir(parent).ok()?;
    let mut disabled_match: Option<String> = None;

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
        if dir_name.starts_with('.') || dir_name.starts_with('$') {
            continue;
        }

        let (clean_name, disabled) = strip_disabled_prefix(&dir_name);
        if !clean_name.eq_ignore_ascii_case(logical_segment) {
            continue;
        }

        if !disabled {
            return Some(dir_name);
        }

        if disabled_match.is_none() {
            disabled_match = Some(dir_name);
        }
    }

    disabled_match
}

fn resolve_physical_group_path(root: &Path, group_id: &str) -> String {
    let logical_group = normalize_group_id(group_id);
    if logical_group.is_empty() {
        return String::new();
    }

    let mut current = root.to_path_buf();
    let mut physical_parts = Vec::new();

    for segment in logical_group
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        let physical_segment =
            find_physical_segment(&current, segment).unwrap_or_else(|| segment.to_string());
        current = current.join(&physical_segment);
        physical_parts.push(physical_segment);
    }

    physical_parts.join("/")
}

fn is_content_image_file(name_lower: &str) -> bool {
    name_lower.ends_with(".jpg")
        || name_lower.ends_with(".jpeg")
        || name_lower.ends_with(".png")
        || name_lower.ends_with(".gif")
        || name_lower.ends_with(".bmp")
        || name_lower.ends_with(".webp")
        || name_lower.ends_with(".avif")
}

fn is_standard_group_icon(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "folder.jpg" | "folder.png" | "icon.jpg" | "icon.png" | "cover.jpg" | "cover.png"
    )
}

/// Result of analyzing a directory: images, has_ini, has_subdirs, has_content_images, icon_path
struct DirAnalysis {
    images: Vec<String>,
    has_ini: bool,
    has_mod_marker: bool,
    has_subdirs: bool,
    has_content_images: bool,
    icon_path: Option<String>,
}

/// Lightweight analysis: only reads one level to determine if this directory is
/// a Mod or a group. Group entry counts are handled separately.
fn analyze_directory(path: &Path) -> DirAnalysis {
    let t0 = Instant::now();
    let mut images = Vec::new();
    let mut has_ini = false;
    let mut has_mod_marker = false;
    let mut has_subdirs = false;
    let mut has_content_images = false;
    let mut icon_path = None;
    let mut entry_count = 0u64;

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            entry_count += 1;
            let sub_path = entry.path();
            if sub_path.is_dir() {
                let sub_name = sub_path.file_name().unwrap_or_default().to_string_lossy();
                if !sub_name.starts_with('.')
                    && !sub_name.starts_with('$')
                    && !is_managed_backup_name(&sub_name)
                {
                    has_subdirs = true;
                }
                continue;
            }

            if !sub_path.is_file() {
                continue;
            }

            let Some(name) = sub_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let lower = name.to_lowercase();
            if lower.ends_with(".ini") {
                has_ini = true;
            }
            if lower == "thisisa.mod" {
                has_mod_marker = true;
            }
            if is_content_image_file(&lower) {
                let image_path = normalize_slashes(&sub_path.to_string_lossy());
                images.push(image_path.clone());
                if is_standard_group_icon(&lower) {
                    icon_path = Some(image_path);
                } else {
                    has_content_images = true;
                }
            }
        }
    }

    images.sort();
    let ms = t0.elapsed().as_millis();
    if ms > 50 {
        log_scan!(
            "analyze_directory SLOW {}ms  entries={}  has_ini={} has_subdirs={}  path={}",
            ms,
            entry_count,
            has_ini,
            has_subdirs,
            path.display()
        );
    }
    DirAnalysis {
        images,
        has_ini,
        has_mod_marker,
        has_subdirs,
        has_content_images,
        icon_path,
    }
}

fn is_leaf_mod_dir_from_analysis(analysis: &DirAnalysis) -> bool {
    // A directory is a leaf mod if it contains:
    // - a "thisisa.mod" marker file, OR
    // - a .ini file, OR
    // - no subdirectories with some content images
    analysis.has_mod_marker
        || analysis.has_ini
        || (!analysis.has_subdirs && analysis.has_content_images)
}

/// Count visible Mod entries directly below a group. A modular Mod can contain
/// many internal INI directories, but it is still one entry for its category.
fn count_group_entries(path: &Path) -> u64 {
    let mut count = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let sub_path = entry.path();
            if !sub_path.is_dir() {
                continue;
            }
            let name = sub_path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name.starts_with('$') || is_managed_backup_name(&name) {
                continue;
            }
            count += 1;
        }
    }
    count
}

fn scan_group_from_disk(install_dir: &str, group_path: &str) -> Result<ScanResult, String> {
    let t0 = Instant::now();
    let root = mods_root(install_dir);
    let logical_group = normalize_group_id(group_path);
    let physical_group = resolve_physical_group_path(&root, &logical_group);
    let target = if physical_group.is_empty() {
        root.clone()
    } else {
        root.join(&physical_group)
    };
    let parent_group_id = if logical_group.is_empty() {
        "Root".to_string()
    } else {
        logical_group.clone()
    };

    log_scan!(
        "scan_group_from_disk START  group={}  physical={}  path={}",
        if logical_group.is_empty() {
            "Root"
        } else {
            &logical_group
        },
        if physical_group.is_empty() {
            "Root"
        } else {
            &physical_group
        },
        target.display()
    );

    if !target.exists() {
        log_scan!(
            "scan_group_from_disk DONE (not found)  {}ms",
            t0.elapsed().as_millis()
        );
        return Ok(ScanResult {
            mods: Vec::new(),
            groups: Vec::new(),
        });
    }

    let parent_uses_directory_link = path_uses_directory_link(&root, &physical_group);

    let mut mods = Vec::new();
    let mut groups = Vec::new();
    let entries = fs::read_dir(&target).map_err(|error| error.to_string())?;
    let mut entry_count = 0u64;
    let mut skipped_count = 0u64;

    for entry in entries {
        // A transient filesystem error must fail the scan. Silently dropping an
        // unreadable entry can turn a healthy library into an apparently empty
        // one, which would then be persisted as a valid SQLite snapshot.
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read an entry in mod directory {}: {}",
                target.display(),
                error
            )
        })?;
        entry_count += 1;
        let path = entry.path();
        if !path.is_dir() {
            skipped_count += 1;
            continue;
        }
        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if dir_name.starts_with('.')
            || dir_name.starts_with('$')
            || is_managed_backup_name(&dir_name)
        {
            skipped_count += 1;
            continue;
        }

        let t_entry = Instant::now();
        let (clean_name, is_disabled) = strip_disabled_prefix(&dir_name);
        let enabled = !is_disabled;
        let analysis = analyze_directory(&path);
        let is_leaf_mod = is_leaf_mod_dir_from_analysis(&analysis);
        let logical_name = if clean_name.is_empty() {
            dir_name.clone()
        } else {
            clean_name.clone()
        };
        let entry_group_id = join_rel(&logical_group, &logical_name);
        let entry_physical_path = join_rel(&physical_group, &dir_name);
        let is_directory_link = parent_uses_directory_link || is_directory_link(&path);
        let entry_ms = t_entry.elapsed().as_millis();

        if is_leaf_mod {
            if entry_ms > 10 {
                log_scan!(
                    "  MOD   \"{}\"  {}ms  images={}  previews={}",
                    clean_name,
                    entry_ms,
                    analysis.images.len(),
                    analysis
                        .images
                        .iter()
                        .map(|p| std::path::Path::new(p)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            mods.push(ModInfo {
                id: entry_physical_path.clone(),
                name: clean_name,
                enabled,
                path: normalize_slashes(&path.to_string_lossy()),
                relative_path: entry_physical_path,
                preview_images: analysis.images,
                group: parent_group_id.clone(),
                is_dir: true,
                last_modified: modified_unix_seconds(&path),
                is_directory_link,
            });
        } else {
            log_scan!(
                "  GROUP \"{}\"  {}ms  has_ini={}  has_subdirs={}  has_content_images={}",
                clean_name,
                entry_ms,
                analysis.has_ini,
                analysis.has_subdirs,
                analysis.has_content_images
            );
            upsert_group(
                &mut groups,
                GroupInfo {
                    id: entry_group_id,
                    name: clean_name,
                    icon_path: analysis.icon_path,
                    path: entry_physical_path,
                    enabled,
                    mod_count: count_group_entries(&path),
                    is_directory_link,
                },
            );
        }
    }

    mods.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    groups.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    let total_ms = t0.elapsed().as_millis();
    log_scan!(
        "scan_group_from_disk DONE  {}ms  total_entries={}  skipped={}  mods={}  groups={}",
        total_ms,
        entry_count,
        skipped_count,
        mods.len(),
        groups.len()
    );
    Ok(ScanResult { mods, groups })
}

fn index_group(
    conn: &mut Connection,
    game_name: &str,
    install_dir: &str,
    group_path: &str,
) -> Result<ScanResult, String> {
    let t0 = Instant::now();
    let group_id = normalize_group_id(group_path);
    let group_key = group_cache_key(&group_id);
    let group_label = if group_id.is_empty() {
        "Root".to_string()
    } else {
        group_id.clone()
    };
    log_scan!(
        "index_group START  game={}  group={}",
        game_name,
        group_label
    );
    let result = scan_group_from_disk(install_dir, &group_id)?;
    let scan_ms = t0.elapsed().as_millis();
    let now = chrono::Utc::now().timestamp_millis();
    let tx = conn.transaction().map_err(|error| error.to_string())?;

    tx.execute(
        "DELETE FROM mods WHERE game = ?1 AND group_path = ?2",
        params![game_name, group_key.as_str()],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM groups WHERE game = ?1 AND parent = ?2",
        params![game_name, group_key.as_str()],
    )
    .map_err(|error| error.to_string())?;

    for item in &result.mods {
        let preview_json =
            serde_json::to_string(&item.preview_images).map_err(|error| error.to_string())?;
        tx.execute(
            r#"
            INSERT OR REPLACE INTO mods
              (game, id, name, enabled, path, relative_path, group_path, is_dir, last_modified, is_directory_link, preview_images_json, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                game_name,
                item.id,
                item.name,
                if item.enabled { 1 } else { 0 },
                item.path,
                item.relative_path,
                item.group,
                if item.is_dir { 1 } else { 0 },
                item.last_modified as i64,
                if item.is_directory_link { 1 } else { 0 },
                preview_json,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    }

    for item in &result.groups {
        tx.execute(
            r#"
            INSERT OR REPLACE INTO groups
              (game, id, parent, name, icon_path, path, enabled, mod_count, is_directory_link, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                game_name,
                item.id,
                group_key.as_str(),
                item.name,
                item.icon_path,
                item.path,
                if item.enabled { 1 } else { 0 },
                item.mod_count as i64,
                if item.is_directory_link { 1 } else { 0 },
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    }

    tx.execute(
        "INSERT OR REPLACE INTO scan_state (game, group_path, updated_at) VALUES (?1, ?2, ?3)",
        params![game_name, group_key.as_str(), now],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    let sql_ms = t0.elapsed().as_millis() - scan_ms;
    log_scan!(
        "index_group DONE  total={}ms  scan={}ms  sqlite={}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        scan_ms,
        sql_ms,
        result.mods.len(),
        result.groups.len()
    );
    Ok(result)
}

fn group_is_indexed(conn: &Connection, game_name: &str, group_path: &str) -> Result<bool, String> {
    let key = group_cache_key(group_path);
    let found: Option<i64> = conn
        .query_row(
            "SELECT updated_at FROM scan_state WHERE game = ?1 AND group_path = ?2",
            params![game_name, key.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if found.is_none() {
        return Ok(false);
    }

    // Cache entries created before modification-time sorting was implemented
    // contain only zero timestamps. Re-index them once so existing users get
    // working sorting without having to clear the SQLite cache manually.
    let has_missing_timestamps = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM mods WHERE game = ?1 AND group_path = ?2 AND last_modified = 0)",
            params![game_name, key.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?
        != 0;

    Ok(!has_missing_timestamps)
}

fn query_group(conn: &Connection, game_name: &str, group_path: &str) -> Result<ScanResult, String> {
    let key = group_cache_key(group_path);

    let mut mods_stmt = conn
        .prepare(
            r#"
            SELECT id, name, enabled, path, relative_path, group_path, is_dir, last_modified, is_directory_link, preview_images_json
            FROM mods
            WHERE game = ?1 AND group_path = ?2
            ORDER BY lower(name)
            "#,
        )
        .map_err(|error| error.to_string())?;
    let mods = mods_stmt
        .query_map(params![game_name, key.as_str()], |row| {
            let preview_json: String = row.get(9)?;
            let preview_images =
                serde_json::from_str::<Vec<String>>(&preview_json).unwrap_or_default();
            Ok(ModInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                path: row.get(3)?,
                relative_path: row.get(4)?,
                group: row.get(5)?,
                is_dir: row.get::<_, i64>(6)? != 0,
                last_modified: row.get::<_, i64>(7)? as u64,
                is_directory_link: row.get::<_, i64>(8)? != 0,
                preview_images,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut groups_stmt = conn
        .prepare(
            r#"
            SELECT id, name, icon_path, path, enabled, mod_count, is_directory_link
            FROM groups
            WHERE game = ?1 AND parent = ?2
            ORDER BY lower(name)
            "#,
        )
        .map_err(|error| error.to_string())?;
    let groups = groups_stmt
        .query_map(params![game_name, key.as_str()], |row| {
            Ok(GroupInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                icon_path: row.get(2)?,
                path: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                mod_count: row.get::<_, i64>(5)? as u64,
                is_directory_link: row.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(ScanResult { mods, groups })
}

fn refresh_library_recursive(
    conn: &mut Connection,
    game_name: &str,
    install_dir: &str,
    group_path: &str,
) -> Result<(), String> {
    let label = if group_path.is_empty() || group_path == "Root" {
        "Root"
    } else {
        group_path
    };
    log_scan!("refresh_library_recursive ENTER  group={}", label);
    let t0 = Instant::now();
    let result = index_group(conn, game_name, install_dir, group_path)?;
    let children = result.groups.len();
    log_scan!(
        "refresh_library_recursive indexed \"{}\"  {}ms  children={}",
        label,
        t0.elapsed().as_millis(),
        children
    );
    for group in result.groups {
        refresh_library_recursive(conn, game_name, install_dir, &group.id)?;
    }
    Ok(())
}

/// Streaming scan: emits progress events so the frontend can render incrementally.
/// Uses SQLite cache when available (fast path), otherwise streams from disk scan.
/// Emits `mod-library-scan-chunk` events with phases: "start", "chunk", "done".
#[tauri::command]
pub async fn mod_library_stream_scan(
    app: AppHandle,
    game_name: String,
    install_dir: String,
    group_path: String,
) -> Result<ScanResult, String> {
    log_scan!(
        ">>> COMMAND mod_library_stream_scan  game={}  group={}",
        game_name,
        group_path
    );
    let t0 = Instant::now();
    // Fast path: use SQLite cache if available
    let conn = open_db(&install_dir)?;
    if group_is_indexed(&conn, &game_name, &group_path)? {
        log_scan!("  CACHE HIT  querying SQLite...");
        let result = query_group(&conn, &game_name, &group_path)?;
        log_scan!(
            "<<< COMMAND mod_library_stream_scan CACHED  {}ms  mods={}  groups={}",
            t0.elapsed().as_millis(),
            result.mods.len(),
            result.groups.len()
        );
        // Emit a single "done" event so the frontend knows data is ready
        let _ = app.emit(
            "mod-library-scan-chunk",
            serde_json::json!({
                "phase": "done",
                "totalMods": result.mods.len(),
                "totalGroups": result.groups.len()
            }),
        );
        return Ok(result);
    }
    log_scan!("  CACHE MISS  starting disk scan...");
    drop(conn);

    let root = mods_root(&install_dir);
    let logical_group = normalize_group_id(&group_path);
    let physical_group = resolve_physical_group_path(&root, &logical_group);
    let group_key = group_cache_key(&logical_group);
    let target = if physical_group.is_empty() {
        root.clone()
    } else {
        root.join(&physical_group)
    };
    let parent_group_id = if logical_group.is_empty() {
        "Root".to_string()
    } else {
        logical_group.clone()
    };

    if !target.exists() {
        let _ = app.emit(
            "mod-library-scan-chunk",
            serde_json::json!({
                "phase": "done",
                "totalMods": 0,
                "totalGroups": 0
            }),
        );
        return Ok(ScanResult {
            mods: Vec::new(),
            groups: Vec::new(),
        });
    }

    let parent_uses_directory_link = path_uses_directory_link(&root, &physical_group);

    let entries: Vec<_> = fs::read_dir(&target)
        .map_err(|error| error.to_string())?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .collect();

    // Send start event with total count for progress display
    let _ = app.emit(
        "mod-library-scan-chunk",
        serde_json::json!({
            "phase": "start",
            "total": entries.len()
        }),
    );

    const CHUNK_SIZE: usize = 8;
    let mut all_mods: Vec<ModInfo> = Vec::new();
    let mut all_groups: Vec<GroupInfo> = Vec::new();

    for chunk in entries.chunks(CHUNK_SIZE) {
        let mut chunk_mods = Vec::new();
        let mut chunk_groups = Vec::new();

        for entry in chunk {
            let path = entry.path();
            let dir_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if dir_name.starts_with('.')
                || dir_name.starts_with('$')
                || is_managed_backup_name(&dir_name)
            {
                continue;
            }

            let (clean_name, is_disabled) = strip_disabled_prefix(&dir_name);
            let enabled = !is_disabled;
            let analysis = analyze_directory(&path);
            let is_leaf_mod = is_leaf_mod_dir_from_analysis(&analysis);
            let logical_name = if clean_name.is_empty() {
                dir_name.clone()
            } else {
                clean_name.clone()
            };
            let entry_group_id = join_rel(&logical_group, &logical_name);
            let entry_physical_path = join_rel(&physical_group, &dir_name);
            let is_directory_link = parent_uses_directory_link || is_directory_link(&path);

            if is_leaf_mod {
                chunk_mods.push(ModInfo {
                    id: entry_physical_path.clone(),
                    name: clean_name,
                    enabled,
                    path: normalize_slashes(&path.to_string_lossy()),
                    relative_path: entry_physical_path,
                    preview_images: analysis.images,
                    group: parent_group_id.clone(),
                    is_dir: true,
                    last_modified: modified_unix_seconds(&path),
                    is_directory_link,
                });
            } else {
                upsert_group(
                    &mut chunk_groups,
                    GroupInfo {
                        id: entry_group_id,
                        name: clean_name,
                        icon_path: analysis.icon_path,
                        path: entry_physical_path,
                        enabled,
                        mod_count: count_group_entries(&path),
                        is_directory_link,
                    },
                );
            }
        }

        if !chunk_mods.is_empty() || !chunk_groups.is_empty() {
            let _ = app.emit(
                "mod-library-scan-chunk",
                serde_json::json!({
                    "phase": "chunk",
                    "mods": chunk_mods,
                    "groups": chunk_groups,
                }),
            );
        }

        all_mods.extend(chunk_mods);
        for group in chunk_groups {
            upsert_group(&mut all_groups, group);
        }
    }

    all_mods.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    all_groups.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));

    // Index into SQLite for future fast loads
    let mut conn = open_db(&install_dir)?;
    let now = chrono::Utc::now().timestamp_millis();
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM mods WHERE game = ?1 AND group_path = ?2",
        params![game_name, group_key.as_str()],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM groups WHERE game = ?1 AND parent = ?2",
        params![game_name, group_key.as_str()],
    )
    .map_err(|error| error.to_string())?;
    for item in &all_mods {
        let preview_json =
            serde_json::to_string(&item.preview_images).map_err(|error| error.to_string())?;
        tx.execute("INSERT OR REPLACE INTO mods (game, id, name, enabled, path, relative_path, group_path, is_dir, last_modified, preview_images_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![game_name, item.id, item.name, if item.enabled { 1 } else { 0 }, item.path, item.relative_path, item.group, if item.is_dir { 1 } else { 0 }, item.last_modified as i64, preview_json, now],
        ).map_err(|error| error.to_string())?;
    }
    for item in &all_groups {
        tx.execute("INSERT OR REPLACE INTO groups (game, id, parent, name, icon_path, path, enabled, mod_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![game_name, item.id, group_key.as_str(), item.name, item.icon_path, item.path, if item.enabled { 1 } else { 0 }, item.mod_count as i64, now],
        ).map_err(|error| error.to_string())?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO scan_state (game, group_path, updated_at) VALUES (?1, ?2, ?3)",
        params![game_name, group_key.as_str(), now],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;

    let _ = app.emit(
        "mod-library-scan-chunk",
        serde_json::json!({
            "phase": "done",
            "totalMods": all_mods.len(),
            "totalGroups": all_groups.len()
        }),
    );

    log_scan!(
        "<<< COMMAND mod_library_stream_scan SCANNED  {}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        all_mods.len(),
        all_groups.len()
    );
    Ok(ScanResult {
        mods: all_mods,
        groups: all_groups,
    })
}

#[tauri::command]
pub async fn mod_library_scan_group(
    game_name: String,
    install_dir: String,
    group_path: String,
) -> Result<ScanResult, String> {
    log_scan!(
        ">>> COMMAND mod_library_scan_group  game={}  group={}",
        game_name,
        group_path
    );
    let t0 = Instant::now();
    let mut conn = open_db(&install_dir)?;
    let is_indexed = group_is_indexed(&conn, &game_name, &group_path)?;
    log_scan!(
        "  DB open {}ms  indexed={}",
        t0.elapsed().as_millis(),
        is_indexed
    );
    let result = if !is_indexed {
        let r = index_group(&mut conn, &game_name, &install_dir, &group_path)?;
        log_scan!("  (indexed fresh)");
        r
    } else {
        let r = query_group(&conn, &game_name, &group_path)?;
        log_scan!("  (from cache)");
        r
    };
    log_scan!(
        "<<< COMMAND mod_library_scan_group DONE  {}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        result.mods.len(),
        result.groups.len()
    );
    Ok(result)
}

#[tauri::command]
pub async fn mod_library_refresh_group(
    game_name: String,
    install_dir: String,
    group_path: String,
) -> Result<ScanResult, String> {
    log_scan!(
        ">>> COMMAND mod_library_refresh_group  game={}  group={}",
        game_name,
        group_path
    );
    let t0 = Instant::now();
    let mut conn = open_db(&install_dir)?;
    let result = index_group(&mut conn, &game_name, &install_dir, &group_path)?;
    log_scan!(
        "<<< COMMAND mod_library_refresh_group DONE  {}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        result.mods.len(),
        result.groups.len()
    );
    Ok(result)
}

#[tauri::command]
pub async fn mod_library_refresh_all(
    game_name: String,
    install_dir: String,
) -> Result<ScanResult, String> {
    log_scan!(">>> COMMAND mod_library_refresh_all  game={}", game_name);
    let t0 = Instant::now();
    // `open_db` creates `<Mods>/.ssmt`. Check the library root first, otherwise
    // a temporarily unavailable/wrong install path would be created as a new,
    // empty library and overwrite the last known-good index.
    let root = mods_root(&install_dir);
    if !root.is_dir() {
        return Err(format!(
            "Mod library directory is unavailable: {}",
            root.display()
        ));
    }
    let mut conn = open_db(&install_dir)?;
    conn.execute(
        "DELETE FROM mods WHERE game = ?1",
        params![game_name.as_str()],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "DELETE FROM groups WHERE game = ?1",
        params![game_name.as_str()],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "DELETE FROM scan_state WHERE game = ?1",
        params![game_name.as_str()],
    )
    .map_err(|error| error.to_string())?;
    refresh_library_recursive(&mut conn, &game_name, &install_dir, "Root")?;
    let result = mod_library_all_mods(game_name, install_dir).await?;
    log_scan!(
        "<<< COMMAND mod_library_refresh_all DONE  {}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        result.mods.len(),
        result.groups.len()
    );
    Ok(result)
}

#[tauri::command]
pub async fn mod_library_all_mods(
    game_name: String,
    install_dir: String,
) -> Result<ScanResult, String> {
    log_scan!(">>> COMMAND mod_library_all_mods  game={}", game_name);
    let t0 = Instant::now();
    let mut conn = open_db(&install_dir)?;
    if !group_is_indexed(&conn, &game_name, "Root")? {
        log_scan!("  Root not indexed, running refresh_library_recursive...");
        refresh_library_recursive(&mut conn, &game_name, &install_dir, "Root")?;
    }

    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, name, enabled, path, relative_path, group_path, is_dir, last_modified, is_directory_link, preview_images_json
            FROM mods
            WHERE game = ?1
            ORDER BY lower(name)
            "#,
        )
        .map_err(|error| error.to_string())?;
    let mods = stmt
        .query_map(params![game_name], |row| {
            let preview_json: String = row.get(9)?;
            let preview_images =
                serde_json::from_str::<Vec<String>>(&preview_json).unwrap_or_default();
            Ok(ModInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
                path: row.get(3)?,
                relative_path: row.get(4)?,
                group: row.get(5)?,
                is_dir: row.get::<_, i64>(6)? != 0,
                last_modified: row.get::<_, i64>(7)? as u64,
                is_directory_link: row.get::<_, i64>(8)? != 0,
                preview_images,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut group_stmt = conn
        .prepare(
            r#"
            SELECT id, name, icon_path, path, enabled, mod_count, is_directory_link
            FROM groups
            WHERE game = ?1
            ORDER BY lower(name)
            "#,
        )
        .map_err(|error| error.to_string())?;
    let groups = group_stmt
        .query_map(params![game_name], |row| {
            Ok(GroupInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                icon_path: row.get(2)?,
                path: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                mod_count: row.get::<_, i64>(5)? as u64,
                is_directory_link: row.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    log_scan!(
        "<<< COMMAND mod_library_all_mods DONE  {}ms  mods={}  groups={}",
        t0.elapsed().as_millis(),
        mods.len(),
        groups.len()
    );
    Ok(ScanResult { mods, groups })
}

fn mod_directory_matches_query(root: &Path, query: &str) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains(query) {
                return true;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                pending.push(path);
                continue;
            }
            if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("ini"))
                && fs::metadata(&path)
                    .map(|metadata| metadata.len() <= 8 * 1024 * 1024)
                    .unwrap_or(false)
                && fs::read(&path)
                    .map(|contents| {
                        String::from_utf8_lossy(&contents)
                            .to_lowercase()
                            .contains(query)
                    })
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn mod_library_search(
    game_name: String,
    install_dir: String,
    query: String,
) -> Result<Vec<String>, String> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }
    let result = mod_library_all_mods(game_name, install_dir).await?;
    Ok(result
        .mods
        .into_iter()
        .filter(|mod_info| {
            mod_info.name.to_lowercase().contains(&normalized_query)
                || mod_directory_matches_query(Path::new(&mod_info.path), &normalized_query)
        })
        .map(|mod_info| mod_info.relative_path)
        .collect())
}

#[tauri::command]
pub async fn watch_mod_library(
    app: AppHandle,
    state: State<'_, ModLibraryWatcher>,
    install_dir: String,
) -> Result<(), String> {
    let mods_dir = mods_root(&install_dir);
    if !mods_dir.exists() {
        return Err(format!(
            "Mods directory not found at: {}",
            mods_dir.display()
        ));
    }

    let mut watcher_guard = state.0.lock().unwrap();
    *watcher_guard = None;

    let app_handle = app.clone();
    let mods_dir_str = normalize_slashes(&mods_dir.to_string_lossy());
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let paths: Vec<String> = event
                    .paths
                    .iter()
                    .map(|path| normalize_slashes(&path.to_string_lossy()))
                    .filter(|path| {
                        // Ignore .ssmt directory changes to prevent feedback loop
                        // (SQLite writes trigger filesystem events)
                        let relative = path
                            .strip_prefix(&mods_dir_str)
                            .unwrap_or(path)
                            .trim_start_matches('/');
                        !relative.starts_with(".ssmt")
                    })
                    .collect();
                if !paths.is_empty() {
                    let _ = app_handle.emit("mod-library-files-changed", paths);
                }
            }
        },
        Config::default(),
    )
    .map_err(|error| format!("Failed to create mod library watcher: {}", error))?;

    watcher
        .watch(&mods_dir, RecursiveMode::Recursive)
        .map_err(|error| format!("Failed to watch mod library: {}", error))?;
    *watcher_guard = Some(watcher);
    Ok(())
}

#[tauri::command]
pub fn unwatch_mod_library(state: State<'_, ModLibraryWatcher>) -> Result<(), String> {
    let mut watcher_guard = state.0.lock().unwrap();
    *watcher_guard = None;
    Ok(())
}

#[tauri::command]
pub fn find_nested_ini_files(
    install_dir: String,
    group_path: String,
) -> Result<Vec<String>, String> {
    let root = mods_root(&install_dir);
    let group_dir = if group_path.is_empty() || group_path == "Root" {
        root.clone()
    } else {
        root.join(&group_path)
    };

    if !group_dir.exists() {
        return Err(format!("Directory not found: {}", group_dir.display()));
    }

    let mut ini_files: Vec<String> = Vec::new();
    let prefix = normalize_slashes(&group_dir.to_string_lossy());
    find_ini_recursive(&group_dir, &prefix, &mut ini_files)
        .map_err(|e| format!("Failed to scan: {}", e))?;

    Ok(ini_files)
}

fn find_ini_recursive(dir: &Path, prefix: &str, results: &mut Vec<String>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            find_ini_recursive(&path, prefix, results)?;
        } else if path.is_file() {
            let name_lower = name.to_lowercase();
            if name_lower.ends_with(".ini") {
                let full = normalize_slashes(&path.to_string_lossy());
                let rel = full
                    .strip_prefix(prefix)
                    .unwrap_or(&full)
                    .trim_start_matches('/')
                    .to_string();
                results.push(rel);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod group_dedupe_tests {
    use super::{count_group_entries, upsert_group, GroupInfo};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn group(id: &str, path: &str, enabled: bool, mod_count: u64) -> GroupInfo {
        GroupInfo {
            id: id.to_string(),
            name: id.rsplit('/').next().unwrap_or(id).to_string(),
            icon_path: None,
            path: path.to_string(),
            enabled,
            mod_count,
            is_directory_link: false,
        }
    }

    #[test]
    fn enabled_group_replaces_disabled_duplicate_and_keeps_count() {
        let mut groups = Vec::new();
        let mut disabled = group("Characters", "DISABLED_Characters", false, 2);
        disabled.icon_path = Some("C:/icon.png".to_string());
        upsert_group(&mut groups, disabled);
        upsert_group(
            &mut groups,
            group("Characters", "Characters", true, 5),
        );

        assert_eq!(groups.len(), 1);
        assert!(groups[0].enabled);
        assert_eq!(groups[0].path, "Characters");
        assert_eq!(groups[0].mod_count, 5);
        assert_eq!(groups[0].icon_path.as_deref(), Some("C:/icon.png"));
    }

    #[test]
    fn repeated_enabled_group_keeps_single_entry_and_largest_count() {
        let mut groups = Vec::new();
        upsert_group(&mut groups, group("Characters", "Characters", true, 1));
        upsert_group(&mut groups, group("Characters", "Characters", true, 3));

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].mod_count, 3);
    }

    #[test]
    fn enabled_empty_group_does_not_inherit_disabled_duplicate_count() {
        let mut groups = Vec::new();
        upsert_group(
            &mut groups,
            group("SSMTGeneratedMod", "DISABLED_SSMTGeneratedMod", false, 11),
        );
        upsert_group(
            &mut groups,
            group("SSMTGeneratedMod", "SSMTGeneratedMod", true, 0),
        );

        assert_eq!(groups.len(), 1);
        assert!(groups[0].enabled);
        assert_eq!(groups[0].mod_count, 0);
    }

    #[test]
    fn group_count_treats_each_top_level_entry_as_one_mod() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ssmt-mod-count-{}-{}",
            std::process::id(),
            unique
        ));
        let first_mod = root.join("FirstMod");
        let second_mod = root.join("SecondMod");
        let modular_mod = root.join("ModularMod");
        fs::create_dir_all(first_mod.join("textures")).unwrap();
        fs::create_dir_all(&second_mod).unwrap();
        fs::create_dir_all(modular_mod.join("Modules").join("PartA")).unwrap();
        fs::create_dir_all(modular_mod.join("Modules").join("PartB")).unwrap();
        fs::write(first_mod.join("mod.ini"), "[TextureOverride_Test]\n").unwrap();
        fs::write(second_mod.join("mod.ini"), "[TextureOverride_Test2]\n").unwrap();
        fs::write(
            modular_mod.join("Modules").join("PartA").join("Menu.ini"),
            "[TextureOverride_A]\n",
        )
        .unwrap();
        fs::write(
            modular_mod.join("Modules").join("PartB").join("Menu.ini"),
            "[TextureOverride_B]\n",
        )
        .unwrap();

        assert_eq!(count_group_entries(&root), 3);
        assert_eq!(count_group_entries(&root.join("EmptyGroup")), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
