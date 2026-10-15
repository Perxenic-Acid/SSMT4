use super::dlss5::Dlss5ManagedState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const STORE: &str = "GraphicsCold";
const JOURNAL: &str = "journal.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    relative: PathBuf,
    digest: String,
    original_digest: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    manifest_digest: String,
    entries: Vec<Entry>,
}

fn manifest_digest(state: &Dlss5ManagedState) -> Result<String, String> {
    Ok(digest(&state.manifest_path)?)
}

fn checked_journal(state: &Dlss5ManagedState, store: &Path) -> Result<Option<Journal>, String> {
    let journal = read_journal(store)?;
    if let Some(journal) = &journal {
        if journal.manifest_digest != manifest_digest(state)? {
            return Err("DLSS5 manifest changed after cold files were parked".to_string());
        }
    }
    Ok(journal)
}

fn roots(state: &Dlss5ManagedState) -> Result<(PathBuf, PathBuf), String> {
    let backup = state.manifest_path.parent().ok_or("invalid DLSS5 backup path")?;
    let game = backup.parent().ok_or("invalid DLSS5 game path")?;
    let normalized = game.to_string_lossy().to_lowercase();
    let identity = format!("{:x}", Sha256::digest(normalized.as_bytes()));
    let store = crate::config::path_manager::PathManager::ssmt_global_config_folder()
        .join(STORE).join(identity);
    Ok((game.to_path_buf(), store))
}

fn cold_file(relative: &Path) -> bool {
    relative.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| {
        ["dll", "addon64", "asi"].iter().any(|item| extension.eq_ignore_ascii_case(item))
    })
}

fn safe_relative(relative: &Path) -> bool {
    !relative.as_os_str().is_empty()
        && relative.components().all(|component| matches!(component, std::path::Component::Normal(_)))
        && !relative.to_string_lossy().contains(':')
}

fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !reparse(&meta) => Ok(true),
        Ok(_) => Err(format!("unsafe cold-file path: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

#[cfg(windows)]
fn reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse(meta: &fs::Metadata) -> bool { meta.file_type().is_symlink() }

fn checked_target(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if !safe_relative(relative) { return Err(format!("unsafe DLSS5 relative path: {}", relative.display())); }
    let mut parent = root.to_path_buf();
    for part in relative.parent().into_iter().flat_map(Path::components) {
        parent.push(part.as_os_str());
        let meta = fs::symlink_metadata(&parent).map_err(|error| error.to_string())?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(format!("unsafe DLSS5 parent directory: {}", parent.display()));
        }
    }
    Ok(root.join(relative))
}

fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn copy_atomic(source: &Path, target: &Path, expected: &str) -> Result<(), String> {
    let pending = target.with_extension("ssmt-copy-pending");
    if regular(&pending)? {
        if digest(&pending)? != expected {
            return Err(format!("unknown pending cold-file copy: {}", pending.display()));
        }
    } else {
        fs::copy(source, &pending).map_err(|error| error.to_string())?;
        OpenOptions::new().write(true).open(&pending).and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        if digest(&pending)? != expected {
            return Err(format!("cold-file copy changed: {}", pending.display()));
        }
    }
    fs::rename(&pending, target).map_err(|error| error.to_string())
}

fn read_journal(store: &Path) -> Result<Option<Journal>, String> {
    match fs::symlink_metadata(store) {
        Ok(meta) if meta.is_dir() && !reparse(&meta) => {}
        Ok(_) => return Err(format!("unsafe cold-file store: {}", store.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let file = store.join(JOURNAL);
    if !regular(&file)? { return Ok(None); }
    let metadata = fs::metadata(&file).map_err(|error| error.to_string())?;
    if metadata.len() > 1024 * 1024 { return Err("cold-file journal is too large".to_string()); }
    let journal: Journal = serde_json::from_slice(&fs::read(&file).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if journal.version != 1 || journal.manifest_digest.len() != 64
        || journal.entries.len() > 256 || journal.entries.iter().any(|entry| {
        !safe_relative(&entry.relative) || !cold_file(&entry.relative)
            || entry.digest.len() != 64 || entry.original_digest.as_ref().is_some_and(|hash| hash.len() != 64)
    }) {
        return Err("invalid SSMT cold-file journal".to_string());
    }
    Ok(Some(journal))
}

fn original_path(state: &Dlss5ManagedState, relative: &Path) -> Result<PathBuf, String> {
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&state.manifest_path)
        .map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let prefix = raw.get("backupPrefix").and_then(serde_json::Value::as_str).unwrap_or("");
    let prefix_path = Path::new(prefix);
    if !prefix.is_empty() && (prefix_path.components().count() != 2
        || prefix_path.components().next().map(|component| component.as_os_str()) != Some("originals".as_ref())
        || !safe_relative(prefix_path))
    {
        return Err("invalid DLSS5 original backup prefix".to_string());
    }
    Ok(state.manifest_path.parent().ok_or("invalid DLSS5 manifest path")?.join(prefix_path).join(relative))
}

fn link_points_to(link: &Path, source: &Path) -> Result<bool, String> {
    let target = fs::read_link(link).map_err(|error| error.to_string())?;
    if target == source { return Ok(true); }
    #[cfg(windows)]
    {
        let value = target.to_string_lossy();
        if let Some(stripped) = value.strip_prefix(r"\\?\") {
            return Ok(Path::new(stripped) == source);
        }
    }
    Ok(false)
}

/// Swapper 安装后立即封存冷加载文件。清单和原版备份仍由 Swapper 持有。
pub fn park_installation(state: &Dlss5ManagedState) -> Result<(), String> {
    let (game, store) = roots(state)?;
    if checked_journal(state, &store)?.is_none() {
        if store.exists() { return Err(format!("unowned cold-file store: {}", store.display())); }
        let mut entries = Vec::new();
        for relative in state.added_files.iter().chain(&state.replaced_files) {
            if !cold_file(relative) { continue; }
            if !safe_relative(relative) { return Err(format!("unsafe DLSS5 file: {}", relative.display())); }
            let target = checked_target(&game, relative)?;
            if !regular(&target)? { return Err(format!("missing installed DLSS5 file: {}", target.display())); }
            let original_digest = if state.replaced_files.contains(relative) {
                let original = original_path(state, relative)?;
                if !regular(&original)? { return Err(format!("missing original DLSS5 backup: {}", original.display())); }
                Some(digest(&original)?)
            } else { None };
            entries.push(Entry { relative: relative.clone(), digest: digest(&target)?, original_digest });
        }
        fs::create_dir_all(store.parent().ok_or("invalid cold-file storage path")?)
            .map_err(|error| error.to_string())?;
        fs::create_dir(&store).map_err(|error| error.to_string())?;
        let journal = Journal { version: 1, manifest_digest: manifest_digest(state)?, entries };
        let bytes = serde_json::to_vec_pretty(&journal).map_err(|error| error.to_string())?;
        let pending = store.join("journal.json.tmp");
        let mut file = OpenOptions::new().write(true).create_new(true).open(&pending)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|error| error.to_string())?;
        fs::rename(pending, store.join(JOURNAL)).map_err(|error| error.to_string())?;
    }
    park(state)
}

/// 清理只移除指向托管文件的链接，不接管用户后来写入的文件。
pub fn park(state: &Dlss5ManagedState) -> Result<(), String> {
    let (game, store) = roots(state)?;
    let Some(journal) = checked_journal(state, &store)? else { return Ok(()); };
    for entry in &journal.entries {
        let target = checked_target(&game, &entry.relative)?;
        let saved = store.join(&entry.relative);
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if meta.file_type().is_symlink() {
                if !link_points_to(&target, &saved)? {
                    return Err(format!("cold-file link changed: {}", target.display()));
                }
                fs::remove_file(&target).map_err(|error| error.to_string())?;
            } else {
                let observed = if meta.is_file() { digest(&target)? } else { String::new() };
                let saved_exists = regular(&saved)?;
                let original_matches = entry.original_digest.as_deref().is_some_and(|hash| observed == hash);
                if observed != entry.digest && !(saved_exists && original_matches) {
                    return Err(format!("cold-file target changed: {}", target.display()));
                }
            }
        }
        if !regular(&saved)? {
            if !regular(&target)? || digest(&target)? != entry.digest {
                return Err(format!("missing installed cold file: {}", saved.display()));
            }
            if let Some(parent) = saved.parent() { fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
            copy_atomic(&target, &saved, &entry.digest)?;
        } else if digest(&saved)? != entry.digest {
            return Err(format!("cold-file store changed: {}", saved.display()));
        }
        if regular(&target)? && digest(&target)? == entry.digest {
            fs::remove_file(&target).map_err(|error| error.to_string())?;
        }
        if let Some(original_digest) = &entry.original_digest {
            let original = original_path(state, &entry.relative)?;
            if digest(&original)? != *original_digest { return Err(format!("original backup changed: {}", original.display())); }
            if !regular(&target)? {
                copy_atomic(&original, &target, original_digest)?;
            } else if digest(&target)? != *original_digest {
                return Err(format!("original game file changed: {}", target.display()));
            }
        } else if regular(&target)? {
            return Err(format!("unexpected cold file remains: {}", target.display()));
        }
    }
    Ok(())
}

pub fn stage(state: &Dlss5ManagedState) -> Result<(), String> {
    if let Err(error) = stage_inner(state) {
        let recovery = park(state);
        return Err(format!("{error}; cold-file recovery: {recovery:?}"));
    }
    Ok(())
}

fn stage_inner(state: &Dlss5ManagedState) -> Result<(), String> {
    let (game, store) = roots(state)?;
    let journal = checked_journal(state, &store)?.ok_or("DLSS5 cold files have not been parked")?;
    for entry in &journal.entries {
        let target = checked_target(&game, &entry.relative)?;
        let saved = store.join(&entry.relative);
        if !regular(&saved)? || digest(&saved)? != entry.digest {
            return Err(format!("invalid cold-file store: {}", saved.display()));
        }
        if let Some(original_digest) = &entry.original_digest {
            if !regular(&target)? || digest(&target)? != *original_digest {
                return Err(format!("original game file changed: {}", target.display()));
            }
        } else if fs::symlink_metadata(&target).is_ok() {
            return Err(format!("cold-file target is occupied: {}", target.display()));
        }
    }
    for entry in journal.entries {
        let target = checked_target(&game, &entry.relative)?;
        let saved = store.join(&entry.relative);
        if entry.original_digest.is_some() {
            fs::remove_file(&target).map_err(|error| error.to_string())?;
        }
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_file(&saved, &target);
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(&saved, &target);
        if let Err(error) = result {
            return Err(format!("cannot link cold file {}: {error}", target.display()));
        }
    }
    Ok(())
}

pub fn remove_store(state: &Dlss5ManagedState) -> Result<(), String> {
    let (_, store) = roots(state)?;
    let Some(journal) = read_journal(&store)? else { return Ok(()); };
    for entry in journal.entries {
        let saved = store.join(&entry.relative);
        if regular(&saved)? {
            if digest(&saved)? != entry.digest { return Err(format!("cold-file store changed: {}", saved.display())); }
            fs::remove_file(&saved).map_err(|error| error.to_string())?;
        }
        let mut parent = saved.parent();
        while let Some(dir) = parent.filter(|dir| *dir != store) {
            let _ = fs::remove_dir(dir);
            parent = dir.parent();
        }
    }
    fs::remove_file(store.join(JOURNAL)).map_err(|error| error.to_string())?;
    fs::remove_dir(store).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn is_cold_file(relative: &Path) -> bool { cold_file(relative) }

pub fn is_parked(state: &Dlss5ManagedState, relative: &Path) -> Result<bool, String> {
    let (_, store) = roots(state)?;
    let Some(journal) = checked_journal(state, &store)? else { return Ok(false); };
    let Some(entry) = journal.entries.iter().find(|entry| entry.relative == relative) else { return Ok(false); };
    let saved = store.join(relative);
    Ok(regular(&saved)? && digest(&saved)? == entry.digest)
}

pub fn source_path(state: &Dlss5ManagedState, relative: &Path) -> Result<PathBuf, String> {
    if !is_parked(state, relative)? {
        return Err(format!("DLSS5 cold file is not parked: {}", relative.display()));
    }
    let (_, store) = roots(state)?;
    Ok(store.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::dlss5::Dlss5Route;

    fn fixture(replaced: bool) -> (PathBuf, Dlss5ManagedState) {
        let root = std::env::temp_dir().join(format!("ssmt-cold-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let backup = root.join("_DLSS5_Backup");
        fs::create_dir_all(backup.join("originals/test")).unwrap();
        fs::write(root.join("Game.exe"), b"game").unwrap();
        fs::write(root.join("plugin.dll"), b"plugin").unwrap();
        if replaced { fs::write(backup.join("originals/test/plugin.dll"), b"original").unwrap(); }
        fs::write(backup.join("manifest.json"), r#"{"backupPrefix":"originals/test"}"#).unwrap();
        let relative = PathBuf::from("plugin.dll");
        let state = Dlss5ManagedState {
            route: Dlss5Route::Feeder,
            manifest_path: backup.join("manifest.json"),
            game_executable: PathBuf::from("Game.exe"),
            game_api: "dxgi".to_string(),
            uses_reshade: true,
            uses_proxy: false,
            added_files: if replaced { Vec::new() } else { vec![relative.clone()] },
            replaced_files: if replaced { vec![relative] } else { Vec::new() },
            external_host: None,
        };
        (root, state)
    }

    #[test]
    fn added_dll_exists_only_during_session() {
        let (root, state) = fixture(false);
        park_installation(&state).unwrap();
        assert!(!root.join("plugin.dll").exists());
        assert!(is_parked(&state, Path::new("plugin.dll")).unwrap());
        stage(&state).unwrap();
        assert!(fs::symlink_metadata(root.join("plugin.dll")).unwrap().file_type().is_symlink());
        park(&state).unwrap();
        assert!(!root.join("plugin.dll").exists());
        remove_store(&state).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replaced_dll_restores_original_and_refuses_foreign_edit() {
        let (root, state) = fixture(true);
        park_installation(&state).unwrap();
        assert_eq!(fs::read(root.join("plugin.dll")).unwrap(), b"original");
        stage(&state).unwrap();
        park(&state).unwrap();
        assert_eq!(fs::read(root.join("plugin.dll")).unwrap(), b"original");
        fs::write(root.join("plugin.dll"), b"foreign").unwrap();
        assert!(stage(&state).is_err());
        assert_eq!(fs::read(root.join("plugin.dll")).unwrap(), b"foreign");
        remove_store(&state).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_reshade_can_link_archived_cold_file() {
        let (root, state) = fixture(false);
        park_installation(&state).unwrap();
        let archived = source_path(&state, Path::new("plugin.dll")).unwrap();
        stage(&state).unwrap();
        crate::plugin::managed_reshade_journal::stage(
            &root.join("Game.exe"), "[GENERAL]", "Techniques=", &[archived],
        ).unwrap();
        assert!(root.join("_SSMT_Graphics_Addons/plugin.dll").exists());
        crate::plugin::managed_reshade_journal::restore(&root.join("Game.exe")).unwrap();
        park(&state).unwrap();
        assert!(!root.join("plugin.dll").exists());
        remove_store(&state).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_swapper_manifest_cannot_reuse_old_archive() {
        let (root, state) = fixture(false);
        park_installation(&state).unwrap();
        fs::write(&state.manifest_path, r#"{"backupPrefix":"originals/other"}"#).unwrap();
        assert!(stage(&state).is_err());
        assert!(!root.join("plugin.dll").exists());
        remove_store(&state).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
