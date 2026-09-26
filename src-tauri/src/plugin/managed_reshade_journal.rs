use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

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
struct Journal {
    version: u32,
    files: Vec<FileRecord>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    {
        return Err("invalid graphics journal file list".to_string());
    }
    Ok(Some(journal))
}

pub fn stage(game_executable: &Path, game_ini: &str, preset_ini: &str) -> Result<(), String> {
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
    };
    let journal_bytes = serde_json::to_vec_pretty(&journal).map_err(|error| error.to_string())?;
    write_atomic(&backup.join(JOURNAL), &journal_bytes)?;
    for (index, name) in FILES.iter().enumerate() {
        if let Err(error) = write_atomic(&root.join(name), content[index]) {
            let recovery = restore(game_executable);
            return Err(format!("{error}; recovery: {recovery:?}"));
        }
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
            return Err(format!(
                "graphics config changed after SSMT staged it: {}",
                target.display()
            ));
        }
        originals.push(original);
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
        (root, exe)
    }

    #[test]
    fn restores_original_bytes_and_removes_only_owned_new_file() {
        let (root, exe) = fixture();
        let original = [0xff, 0xfe, 0x5b, 0x00, 0x47, 0x00];
        fs::write(root.join("ReShade.ini"), original).unwrap();
        stage(&exe, "[GENERAL]\n", "Techniques=Feed\n").unwrap();
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"[GENERAL]\n");
        assert!(restore(&exe).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), original);
        assert!(!root.join("ReShadePreset.ini").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_external_edit() {
        let (root, exe) = fixture();
        stage(&exe, "managed", "preset").unwrap();
        fs::write(root.join("ReShade.ini"), b"someone else edited").unwrap();
        assert!(restore(&exe).unwrap_err().contains("changed after SSMT"));
        assert!(root.join(BACKUP_DIR).join(JOURNAL).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovers_interrupted_atomic_write() {
        let (root, exe) = fixture();
        fs::write(root.join("ReShade.ini"), b"original").unwrap();
        stage(&exe, "managed", "preset").unwrap();
        fs::write(root.join("ReShade.ssmt-next"), b"managed").unwrap();
        assert!(restore(&exe).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"original");
        assert!(!root.join("ReShade.ssmt-next").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
