use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::collections::BTreeMap;

use super::managed_reshade::MANAGED_ADDON_DIR;

const BACKUP_DIR: &str = "_SSMT_Graphics_Backup";
const JOURNAL: &str = "journal.json";
const FILES: [&str; 2] = ["ReShade.ini", "ReShadePreset.ini"];
const MAX_CONFIG_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileRecord {
    name: String,
    existed: bool,
    original_hash: Option<String>,
    applied_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddonRecord {
    name: String,
    applied_hash: String,
    #[serde(default)]
    source: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    files: Vec<FileRecord>,
    #[serde(default)]
    addon_files: Vec<AddonRecord>,
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && Path::new(name).file_name().and_then(|part| part.to_str()) == Some(name)
        && !name.contains(['/', '\\', ':'])
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn link_target_matches(actual: &Path, expected: &Path) -> bool {
    if actual == expected {
        return true;
    }
    #[cfg(windows)]
    {
        let expected = expected.to_string_lossy();
        let native = if let Some(path) = expected.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{path}")
        } else if let Some(path) = expected.strip_prefix(r"\\?\") {
            path.to_string()
        } else {
            return false;
        };
        return actual == Path::new(&native);
    }
    #[cfg(not(windows))]
    false
}

fn regular_file_or_missing(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !is_reparse_point(&meta) => Ok(true),
        Ok(_) => Err(format!("unsafe graphics config path: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

#[cfg(windows)]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn game_root(game_executable: &Path) -> Result<PathBuf, String> {
    if !regular_file_or_missing(game_executable)? {
        return Err(format!(
            "game executable does not exist: {}",
            game_executable.display()
        ));
    }
    let parent = game_executable
        .parent()
        .ok_or("game executable has no parent")?;
    let meta = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if !meta.is_dir() || is_reparse_point(&meta) {
        return Err("game directory is not a regular directory".to_string());
    }
    Ok(parent.to_path_buf())
}

fn backup_root(game_root: &Path) -> Result<PathBuf, String> {
    let backup = game_root.join(BACKUP_DIR);
    match fs::symlink_metadata(&backup) {
        Ok(meta) if meta.is_dir() && !is_reparse_point(&meta) => {}
        Ok(_) => return Err(format!("unsafe graphics backup path: {}", backup.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(backup)
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|error| error.to_string())?;
    if meta.len() > MAX_CONFIG_SIZE {
        return Err(format!(
            "graphics config exceeds 16 MiB: {}",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| error.to_string())
}

fn write_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = temporary_path(target);
    if regular_file_or_missing(&temp)? {
        // This name belongs solely to the active journal. A leftover is
        // handled through recovery rather than silently overwritten.
        return Err(format!("pending graphics config write: {}", temp.display()));
    }
    write_sync(&temp, bytes)?;
    fs::rename(&temp, target)
        .map_err(|error| format!("cannot replace {}: {error}", target.display()))
}

fn temporary_path(target: &Path) -> PathBuf {
    target.with_extension("ssmt-next")
}

fn load_journal(backup: &Path) -> Result<Option<Journal>, String> {
    let path = backup.join(JOURNAL);
    if !regular_file_or_missing(&path)? {
        return Ok(None);
    }
    let journal: Journal = serde_json::from_slice(&read_limited(&path)?)
        .map_err(|error| format!("invalid graphics journal: {error}"))?;
    if journal.version != 1
        || journal.files.len() != FILES.len()
        || journal
            .files
            .iter()
            .zip(FILES)
            .any(|(row, expected)| row.name != expected)
        || journal.addon_files.len() > 128
        || journal
            .addon_files
            .iter()
            .any(|row| !safe_file_name(&row.name))
    {
        return Err("invalid graphics journal file list".to_string());
    }
    Ok(Some(journal))
}

pub fn stage(
    game_executable: &Path,
    game_ini: &str,
    preset_ini: &str,
    addon_sources: &[PathBuf],
) -> Result<(), String> {
    let root = game_root(game_executable)?;
    let backup = backup_root(&root)?;
    if load_journal(&backup)?.is_some() {
        restore(game_executable)?;
    }
    if backup.is_dir()
        && fs::read_dir(&backup)
            .map_err(|error| error.to_string())?
            .next()
            .is_some()
    {
        return Err("graphics backup directory contains unowned files".to_string());
    }
    let addon_dir = root.join(MANAGED_ADDON_DIR);
    if fs::symlink_metadata(&addon_dir).is_ok() {
        return Err(format!("managed add-on directory already exists: {}", addon_dir.display()));
    }
    let mut addons = BTreeMap::<String, (PathBuf, Vec<u8>)>::new();
    for source in addon_sources {
        let name = source.file_name().and_then(|part| part.to_str()).ok_or_else(|| {
            format!("invalid add-on source name: {}", source.display())
        })?;
        if !safe_file_name(name) || !regular_file_or_missing(source)? {
            return Err(format!("invalid add-on source: {}", source.display()));
        }
        let source = source.canonicalize().map_err(|error| error.to_string())?;
        let bytes = read_limited(&source)?;
        if let Some((_, previous)) = addons.insert(name.to_string(), (source, bytes.clone())) {
            if previous != bytes {
                return Err(format!("conflicting add-on file: {name}"));
            }
        }
    }
    if addons.is_empty() {
        return Err("managed ReShade host has no add-on files".to_string());
    }
    let content = [game_ini.as_bytes(), preset_ini.as_bytes()];
    let mut rows = Vec::new();
    for (index, name) in FILES.iter().enumerate() {
        let target = root.join(name);
        let existed = regular_file_or_missing(&target)?;
        let original = if existed {
            Some(read_limited(&target)?)
        } else {
            None
        };
        rows.push((
            index,
            FileRecord {
                name: (*name).to_string(),
                existed,
                original_hash: original.as_deref().map(hash),
                applied_hash: hash(content[index]),
            },
            original,
        ));
    }
    fs::create_dir_all(&backup).map_err(|error| error.to_string())?;
    for (index, _, original) in &rows {
        if let Some(bytes) = original {
            write_sync(&backup.join(format!("original-{index}.bin")), bytes)?;
        }
    }
    let journal = Journal {
        version: 1,
        files: rows.into_iter().map(|(_, row, _)| row).collect(),
        addon_files: addons.iter().map(|(name, (source, bytes))| AddonRecord {
            name: name.clone(),
            applied_hash: hash(bytes),
            source: Some(source.clone()),
        }).collect(),
    };
    let journal_bytes = serde_json::to_vec_pretty(&journal).map_err(|error| error.to_string())?;
    write_atomic(&backup.join(JOURNAL), &journal_bytes)?;
    for (index, name) in FILES.iter().enumerate() {
        if let Err(error) = write_atomic(&root.join(name), content[index]) {
            let recovery = restore(game_executable);
            return Err(format!("{error}; recovery: {recovery:?}"));
        }
    }
    if let Err(error) = fs::create_dir(&addon_dir).map_err(|error| error.to_string()).and_then(|_| {
        for (name, (source, _)) in &addons {
            let target = addon_dir.join(name);
            #[cfg(windows)]
            if std::os::windows::fs::symlink_file(source, &target).is_ok() {
                continue;
            }
            #[cfg(unix)]
            if std::os::unix::fs::symlink(source, &target).is_ok() {
                continue;
            }
            if fs::hard_link(source, &target).is_ok() {
                continue;
            }
            if fs::symlink_metadata(&target).is_ok() {
                return Err(format!("failed to link add-on safely: {}", target.display()));
            }
            fs::copy(source, &target).map_err(|error| error.to_string())?;
        }
        Ok(())
    }) {
        let recovery = restore(game_executable);
        return Err(format!("{error}; recovery: {recovery:?}"));
    }
    Ok(())
}

pub fn restore(game_executable: &Path) -> Result<bool, String> {
    let root = game_root(game_executable)?;
    let backup = backup_root(&root)?;
    let Some(journal) = load_journal(&backup)? else {
        return Ok(false);
    };
    // Validate all current and saved bytes before changing either file.
    let mut originals = Vec::new();
    let mut temporary_files = Vec::new();
    let mut recovered = Vec::new();
    let mut owned_addons = Vec::new();
    for (index, row) in journal.files.iter().enumerate() {
        let target = root.join(&row.name);
        let temp = temporary_path(&target);
        if regular_file_or_missing(&temp)? {
            let temp_hash = hash(&read_limited(&temp)?);
            if temp_hash != row.applied_hash && Some(&temp_hash) != row.original_hash.as_ref() {
                return Err(format!(
                    "unknown graphics config temporary file: {}",
                    temp.display()
                ));
            }
            temporary_files.push(temp);
        }
        let current = if regular_file_or_missing(&target)? {
            Some(read_limited(&target)?)
        } else {
            None
        };
        let original = if row.existed {
            let saved = backup.join(format!("original-{index}.bin"));
            if !regular_file_or_missing(&saved)? {
                return Err(format!("missing graphics backup: {}", saved.display()));
            }
            let bytes = read_limited(&saved)?;
            if Some(hash(&bytes)) != row.original_hash {
                return Err(format!(
                    "graphics backup checksum mismatch: {}",
                    saved.display()
                ));
            }
            Some(bytes)
        } else {
            None
        };
        let current_hash = current.as_deref().map(hash);
        if current_hash.as_deref() != Some(&row.applied_hash) && current_hash != row.original_hash {
            let bytes = current.ok_or_else(|| {
                format!("graphics config disappeared after SSMT staged it: {}", target.display())
            })?;
            let digest = current_hash.expect("existing config has a hash");
            let path = root.join(format!("{}.ssmt-recovered-{}.bak", row.name, &digest[..16]));
            if regular_file_or_missing(&path)? && read_limited(&path)? != bytes {
                return Err(format!(
                    "graphics recovery file already has different content: {}",
                    path.display()
                ));
            }
            recovered.push((path, bytes));
        }
        originals.push(original);
    }
    let addon_dir = root.join(MANAGED_ADDON_DIR);
    if !journal.addon_files.is_empty() {
        match fs::symlink_metadata(&addon_dir) {
            Ok(meta) if meta.is_dir() && !is_reparse_point(&meta) => {}
            Ok(_) => return Err(format!("unsafe managed add-on directory: {}", addon_dir.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    for row in &journal.addon_files {
        let target = addon_dir.join(&row.name);
        let temp = temporary_path(&target);
        if regular_file_or_missing(&temp)? {
            if hash(&read_limited(&temp)?) != row.applied_hash {
                return Err(format!("unknown managed add-on temporary file: {}", temp.display()));
            }
            temporary_files.push(temp);
        }
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let expected = row.source.as_ref().ok_or_else(|| {
                    format!("unowned managed add-on link: {}", target.display())
                })?;
                if !link_target_matches(&fs::read_link(&target).map_err(|error| error.to_string())?, expected) {
                    return Err(format!("managed add-on link target changed: {}", target.display()));
                }
                owned_addons.push((target, None));
                continue;
            }
            Ok(meta) if meta.is_file() && !is_reparse_point(&meta) => {}
            Ok(_) => return Err(format!("unsafe managed add-on file: {}", target.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        }
        let bytes = read_limited(&target)?;
        let digest = hash(&bytes);
        if digest != row.applied_hash {
            let path = root.join(format!("{}.ssmt-recovered-{}.bak", row.name, &digest[..16]));
            if regular_file_or_missing(&path)? && read_limited(&path)? != bytes {
                return Err(format!(
                    "graphics recovery file already has different content: {}",
                    path.display()
                ));
            }
            recovered.push((path, bytes.clone()));
        }
        owned_addons.push((target, Some(bytes)));
    }
    for (path, bytes) in recovered {
        if !regular_file_or_missing(&path)? {
            write_atomic(&path, &bytes)?;
        }
        eprintln!("[GraphicsStack] Preserved game-edited config at {}", path.display());
    }
    for temp in temporary_files {
        fs::remove_file(temp).map_err(|error| error.to_string())?;
    }
    for (index, name) in FILES.iter().enumerate() {
        let target = root.join(name);
        match &originals[index] {
            Some(bytes) if fs::read(&target).ok().as_deref() != Some(bytes) => {
                write_atomic(&target, bytes)?
            }
            None if regular_file_or_missing(&target)? => {
                fs::remove_file(&target).map_err(|error| error.to_string())?
            }
            _ => {}
        }
    }
    for (target, observed) in owned_addons {
        if let Some(observed) = observed {
            if !regular_file_or_missing(&target)? || read_limited(&target)? != observed {
                return Err(format!("managed add-on changed during restore: {}", target.display()));
            }
        } else {
            let row = journal.addon_files.iter().find(|row| target.file_name().and_then(|name| name.to_str()) == Some(row.name.as_str())).ok_or("missing managed add-on journal entry")?;
            if !link_target_matches(&fs::read_link(&target).map_err(|error| error.to_string())?, row.source.as_ref().ok_or("missing managed add-on source")?) {
                return Err(format!("managed add-on link target changed: {}", target.display()));
            }
        }
        fs::remove_file(&target).map_err(|error| error.to_string())?;
    }
    if !journal.addon_files.is_empty() && addon_dir.is_dir() {
        for entry in fs::read_dir(&addon_dir).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if !regular_file_or_missing(&path)? {
                continue;
            }
            let bytes = read_limited(&path)?;
            let digest = hash(&bytes);
            let name = path.file_name().and_then(|part| part.to_str()).ok_or("invalid generated add-on file")?;
            let saved = root.join(format!("{name}.ssmt-recovered-{}.bak", &digest[..16]));
            if regular_file_or_missing(&saved)? {
                if read_limited(&saved)? != bytes {
                    return Err(format!("graphics recovery file already has different content: {}", saved.display()));
                }
            } else {
                write_atomic(&saved, &bytes)?;
            }
            fs::remove_file(&path).map_err(|error| error.to_string())?;
            eprintln!("[GraphicsStack] Preserved add-on generated file at {}", saved.display());
        }
        fs::remove_dir(&addon_dir).map_err(|error| error.to_string())?;
    }
    fs::remove_file(backup.join(JOURNAL)).map_err(|error| error.to_string())?;
    for index in 0..FILES.len() {
        let file = backup.join(format!("original-{index}.bin"));
        if regular_file_or_missing(&file)? {
            fs::remove_file(file).map_err(|error| error.to_string())?;
        }
    }
    let _ = fs::remove_dir(backup);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ssmt-managed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("Game.exe");
        fs::write(&exe, b"game").unwrap();
        fs::write(root.join("test.addon64"), b"addon").unwrap();
        (root, exe)
    }

    #[test]
    fn restores_original_bytes_and_removes_only_owned_new_file() {
        let (root, exe) = fixture();
        let original = [0xff, 0xfe, 0x5b, 0x00, 0x47, 0x00];
        fs::write(root.join("ReShade.ini"), original).unwrap();
        stage(&exe, "[GENERAL]\n", "Techniques=Feed\n", &[root.join("test.addon64")]).unwrap();
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"[GENERAL]\n");
        assert!(restore(&exe).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), original);
        assert!(!root.join("ReShadePreset.ini").exists());
        assert!(!root.join(MANAGED_ADDON_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_external_edit_before_restoring_original() {
        let (root, exe) = fixture();
        fs::write(root.join("ReShade.ini"), b"original").unwrap();
        stage(&exe, "managed", "preset", &[root.join("test.addon64")]).unwrap();
        fs::write(root.join("ReShade.ini"), b"someone else edited").unwrap();
        assert!(restore(&exe).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"original");
        let digest = hash(b"someone else edited");
        let saved = root.join(format!("ReShade.ini.ssmt-recovered-{}.bak", &digest[..16]));
        assert_eq!(fs::read(saved).unwrap(), b"someone else edited");
        assert!(!root.join(BACKUP_DIR).join(JOURNAL).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovers_interrupted_atomic_write() {
        let (root, exe) = fixture();
        fs::write(root.join("ReShade.ini"), b"original").unwrap();
        stage(&exe, "managed", "preset", &[root.join("test.addon64")]).unwrap();
        fs::write(root.join("ReShade.ssmt-next"), b"managed").unwrap();
        assert!(restore(&exe).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"original");
        assert!(!root.join("ReShade.ssmt-next").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_modified_managed_addon_on_restore() {
        let (root, exe) = fixture();
        stage(&exe, "managed", "preset", &[root.join("test.addon64")]).unwrap();
        let active = root.join(MANAGED_ADDON_DIR).join("test.addon64");
        fs::remove_file(&active).unwrap();
        fs::write(&active, b"changed addon").unwrap();
        fs::write(root.join(MANAGED_ADDON_DIR).join("addon.log"), b"log data").unwrap();
        assert!(restore(&exe).unwrap());
        assert!(!root.join(MANAGED_ADDON_DIR).exists());
        let digest = hash(b"changed addon");
        let saved = root.join(format!("test.addon64.ssmt-recovered-{}.bak", &digest[..16]));
        assert_eq!(fs::read(saved).unwrap(), b"changed addon");
        assert_eq!(fs::read(root.join("test.addon64")).unwrap(), b"addon");
        let log_digest = hash(b"log data");
        let saved_log = root.join(format!("addon.log.ssmt-recovered-{}.bak", &log_digest[..16]));
        assert_eq!(fs::read(saved_log).unwrap(), b"log data");
        fs::remove_dir_all(root).unwrap();
    }
}
