//! Crash-recoverable file replacement for generic plugin launch plans.
//!
//! The journal is written before the first target mutation. Every original file is
//! copied into the transaction directory, and recovery refuses to overwrite content
//! that is neither the recorded "before" nor "after" state.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCK_FILE: &str = ".ssmt-launch-files.lock";
const TRANSACTIONS_DIR: &str = ".ssmt-launch-transactions";
const JOURNAL_FILE: &str = "journal.json";
const COMMITTED_FILE: &str = "committed";
const MAX_ACTIONS: usize = 128;

static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutation {
    /// Game-relative target path. Absolute paths and parent traversal are rejected.
    pub target: String,
    /// `None` removes the target. `Some` atomically replaces it with this file.
    pub source: Option<PathBuf>,
    /// Optional optimistic-concurrency guard for the target's current bytes.
    pub expected_before_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTransactionReport {
    pub transaction_id: String,
    pub replaced_files: Vec<String>,
    pub removed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionRecord {
    target: String,
    before_sha256: Option<String>,
    after_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionJournal {
    version: u32,
    transaction_id: String,
    records: Vec<TransactionRecord>,
}

struct DirectoryLock {
    _file: File,
}

pub fn execute(root: &Path, mutations: &[FileMutation]) -> Result<FileTransactionReport, String> {
    execute_inner(root, mutations, None, false)
}

/// Applies a launch deployment and keeps its recovery journal until explicitly restored.
pub fn apply(root: &Path, mutations: &[FileMutation]) -> Result<FileTransactionReport, String> {
    execute_inner(root, mutations, None, true)
}

fn execute_inner(
    root: &Path,
    mutations: &[FileMutation],
    fail_after: Option<usize>,
    persist: bool,
) -> Result<FileTransactionReport, String> {
    validate_mutations(mutations)?;
    let root = checked_root(root)?;
    let _lock = lock_root(&root)?;
    recover_incomplete(&root)?;
    let active_targets = active_target_set(&root)?;
    for mutation in mutations {
        if active_targets.contains(&normalized_target(&mutation.target)) {
            return Err(format!(
                "{} is already owned by an applied launch deployment",
                mutation.target
            ));
        }
    }

    let transactions = transactions_root(&root, true)?
        .ok_or_else(|| "cannot create launch transaction directory".to_string())?;
    let transaction_id = next_transaction_id();
    let pending = transactions.join(&transaction_id);
    fs::create_dir(&pending).map_err(|error| error.to_string())?;

    let result = (|| {
        let mut records = Vec::with_capacity(mutations.len());
        for (index, mutation) in mutations.iter().enumerate() {
            let target = checked_target(&root, &mutation.target)?;
            let before = hash_regular(&target)?;
            if let Some(expected) = mutation.expected_before_sha256.as_deref() {
                if before.as_deref() != Some(expected) {
                    return Err(format!(
                        "{} changed after the launch plan was prepared",
                        mutation.target
                    ));
                }
            }
            if let Some(before) = &before {
                copy_sync(&target, &pending.join(format!("{index}.old")))?;
                if hash_regular(&pending.join(format!("{index}.old")))?.as_deref() != Some(before) {
                    return Err(format!("cannot verify backup for {}", mutation.target));
                }
            }
            let after = match &mutation.source {
                Some(source) => {
                    require_regular_file(source)?;
                    if source
                        .canonicalize()
                        .ok()
                        .zip(target.canonicalize().ok())
                        .is_some_and(|(source, target)| source == target)
                    {
                        return Err(format!(
                            "launch source and target are the same file: {}",
                            mutation.target
                        ));
                    }
                    let staged = pending.join(format!("{index}.new"));
                    copy_sync(source, &staged)?;
                    let digest = hash_regular(&staged)?
                        .ok_or_else(|| "staged launch file disappeared".to_string())?;
                    Some(digest)
                }
                None => None,
            };
            records.push(TransactionRecord {
                target: mutation.target.replace('\\', "/"),
                before_sha256: before,
                after_sha256: after,
            });
        }
        let journal = TransactionJournal {
            version: 1,
            transaction_id: transaction_id.clone(),
            records,
        };
        write_journal(&pending, &journal)?;

        for (index, record) in journal.records.iter().enumerate() {
            let target = checked_target(&root, &record.target)?;
            if hash_regular(&target)? != record.before_sha256 {
                return Err(format!(
                    "{} changed while the launch transaction was being prepared",
                    record.target
                ));
            }
            match &record.after_sha256 {
                Some(_) => {
                    fs::rename(pending.join(format!("{index}.new")), &target)
                        .map_err(|error| format!("cannot replace {}: {error}", target.display()))?;
                }
                None => {
                    if target.exists() {
                        fs::remove_file(&target).map_err(|error| error.to_string())?;
                    }
                }
            }
            if fail_after == Some(index + 1) {
                return Err("simulated launch transaction failure".to_string());
            }
        }
        write_sync(&pending.join(COMMITTED_FILE), b"1")?;
        Ok(FileTransactionReport {
            transaction_id: transaction_id.clone(),
            replaced_files: journal
                .records
                .iter()
                .filter(|record| record.after_sha256.is_some())
                .map(|record| record.target.clone())
                .collect(),
            removed_files: journal
                .records
                .iter()
                .filter(|record| record.after_sha256.is_none())
                .map(|record| record.target.clone())
                .collect(),
        })
    })();

    match result {
        Ok(report) => {
            if !persist {
                remove_owned_transaction(&pending)?;
                cleanup_empty_transactions(&root)?;
            }
            Ok(report)
        }
        Err(error) => match rollback_transaction(&root, &pending) {
            Ok(()) => {
                cleanup_empty_transactions(&root)?;
                Err(error)
            }
            Err(recovery) => Err(format!("{error}; transaction recovery pending: {recovery}")),
        },
    }
}

pub fn restore_applied(root: &Path, transaction_id: &str) -> Result<bool, String> {
    if !safe_transaction_id(transaction_id) {
        return Err("invalid launch transaction id".to_string());
    }
    let root = checked_root(root)?;
    let _lock = lock_root(&root)?;
    let Some(transactions) = transactions_root(&root, false)? else {
        return Ok(false);
    };
    let pending = transactions.join(transaction_id);
    if !pending.exists() {
        return Ok(false);
    }
    let meta = fs::symlink_metadata(&pending).map_err(|error| error.to_string())?;
    if !meta.is_dir() || reparse(&meta) {
        return Err("unsafe launch transaction path".to_string());
    }
    require_regular_file(&pending.join(COMMITTED_FILE))?;
    rollback_transaction(&root, &pending)?;
    cleanup_empty_transactions(&root)?;
    Ok(true)
}

fn validate_mutations(mutations: &[FileMutation]) -> Result<(), String> {
    if mutations.is_empty() || mutations.len() > MAX_ACTIONS {
        return Err(format!(
            "launch transaction requires between 1 and {MAX_ACTIONS} mutations"
        ));
    }
    let mut targets = HashSet::new();
    for mutation in mutations {
        super::validate_package_relative_path(&mutation.target)
            .map_err(|error| error.to_string())?;
        if mutation
            .target
            .replace('\\', "/")
            .split('/')
            .any(|segment| segment.to_ascii_lowercase().starts_with(".ssmt-"))
        {
            return Err(format!("reserved launch target: {}", mutation.target));
        }
        if !targets.insert(mutation.target.replace('\\', "/").to_ascii_lowercase()) {
            return Err(format!("duplicate launch target: {}", mutation.target));
        }
        if let Some(expected) = &mutation.expected_before_sha256 {
            validate_sha256(expected)?;
        }
    }
    Ok(())
}

fn checked_root(root: &Path) -> Result<PathBuf, String> {
    let canonical = root.canonicalize().map_err(|error| error.to_string())?;
    let meta = fs::symlink_metadata(&canonical).map_err(|error| error.to_string())?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(format!(
            "launch transaction root is not a regular directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn checked_target(root: &Path, relative: &str) -> Result<PathBuf, String> {
    super::validate_package_relative_path(relative).map_err(|error| error.to_string())?;
    let target = root.join(relative);
    let mut ancestor = root.to_path_buf();
    for component in target
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .into_iter()
        .flat_map(Path::components)
    {
        ancestor.push(component.as_os_str());
        let meta = fs::symlink_metadata(&ancestor).map_err(|error| error.to_string())?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(format!(
                "launch target parent is not a regular directory: {}",
                ancestor.display()
            ));
        }
    }
    let parent = target
        .parent()
        .ok_or_else(|| "launch target has no parent".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("launch target parent is missing: {error}"))?;
    if !parent.starts_with(root) {
        return Err(format!(
            "launch target escapes its game directory: {relative}"
        ));
    }
    let meta = fs::symlink_metadata(&parent).map_err(|error| error.to_string())?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(format!(
            "launch target parent is not a regular directory: {}",
            parent.display()
        ));
    }
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if !meta.is_file() || reparse(&meta) {
            return Err(format!("unsafe launch target: {}", target.display()));
        }
    }
    Ok(target)
}

fn transactions_root(root: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    let path = root.join(TRANSACTIONS_DIR);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() && !reparse(&meta) => Ok(Some(path)),
        Ok(_) => Err(format!(
            "unsafe launch transaction directory: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            fs::create_dir(&path).map_err(|error| error.to_string())?;
            Ok(Some(path))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn lock_root(root: &Path) -> Result<DirectoryLock, String> {
    let path = root.join(LOCK_FILE);
    if path.exists() {
        require_regular_file(&path)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("launch files are busy or cannot be locked: {error}"))?;
    Ok(DirectoryLock { _file: file })
}

fn recover_incomplete(root: &Path) -> Result<(), String> {
    let Some(transactions) = transactions_root(root, false)? else {
        return Ok(());
    };
    for entry in fs::read_dir(&transactions).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(format!(
                "unexpected launch transaction entry: {}",
                entry.path().display()
            ));
        }
        if entry.path().join(COMMITTED_FILE).exists() {
            validate_committed(root, &entry.path())?;
            continue;
        }
        rollback_transaction(root, &entry.path())?;
    }
    Ok(())
}

pub fn applied_targets(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let root = checked_root(root)?;
    let Some(transactions) = transactions_root(&root, false)? else {
        return Ok(BTreeMap::new());
    };
    let mut targets = BTreeMap::new();
    for entry in fs::read_dir(&transactions).map_err(|error| error.to_string())? {
        let pending = entry.map_err(|error| error.to_string())?.path();
        if !pending.join(COMMITTED_FILE).exists() {
            continue;
        }
        validate_committed(&root, &pending)?;
        let journal = read_journal(&pending)?;
        for record in journal.records {
            targets.insert(
                normalized_target(&record.target),
                journal.transaction_id.clone(),
            );
        }
    }
    Ok(targets)
}

fn active_target_set(root: &Path) -> Result<HashSet<String>, String> {
    Ok(applied_targets(root)?.into_keys().collect())
}

fn validate_committed(root: &Path, pending: &Path) -> Result<(), String> {
    let journal = read_journal(pending)?;
    for (index, record) in journal.records.iter().enumerate() {
        let target = checked_target(root, &record.target)?;
        if hash_regular(&target)? != record.after_sha256 {
            return Err(format!(
                "{} changed after its launch deployment committed; recovery data was preserved",
                record.target
            ));
        }
        if record.before_sha256.is_some() {
            let backup = pending.join(format!("{index}.old"));
            require_regular_file(&backup)?;
            if hash_regular(&backup)? != record.before_sha256 {
                return Err(format!(
                    "launch deployment backup checksum mismatch: {}",
                    record.target
                ));
            }
        }
    }
    Ok(())
}

fn cleanup_empty_transactions(root: &Path) -> Result<(), String> {
    let Some(transactions) = transactions_root(root, false)? else {
        return Ok(());
    };
    if fs::read_dir(&transactions)
        .map_err(|error| error.to_string())?
        .next()
        .is_none()
    {
        fs::remove_dir(&transactions).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn rollback_transaction(root: &Path, pending: &Path) -> Result<(), String> {
    let journal_path = pending.join(JOURNAL_FILE);
    if !journal_path.exists() {
        return remove_owned_transaction(pending);
    }
    let journal = read_journal(pending)?;
    let mut originals = Vec::with_capacity(journal.records.len());
    for (index, record) in journal.records.iter().enumerate() {
        let target = checked_target(root, &record.target)?;
        let current = hash_regular(&target)?;
        let interrupted_replace =
            current.is_none() && record.before_sha256.is_some() && record.after_sha256.is_some();
        if current != record.before_sha256 && current != record.after_sha256 && !interrupted_replace
        {
            return Err(format!(
                "{} changed after the launch transaction; recovery data was preserved",
                record.target
            ));
        }
        let backup = pending.join(format!("{index}.old"));
        if record.before_sha256.is_some() {
            require_regular_file(&backup)?;
            if hash_regular(&backup)? != record.before_sha256 {
                return Err(format!(
                    "launch transaction backup checksum mismatch: {}",
                    record.target
                ));
            }
            originals.push(Some(backup));
        } else {
            originals.push(None);
        }
    }
    for (index, record) in journal.records.iter().enumerate().rev() {
        let target = checked_target(root, &record.target)?;
        if let Some(backup) = &originals[index] {
            let restore = pending.join(format!("{index}.restore"));
            if restore.exists() {
                require_regular_file(&restore)?;
                fs::remove_file(&restore).map_err(|error| error.to_string())?;
            }
            copy_sync(backup, &restore)?;
            fs::rename(&restore, &target).map_err(|error| {
                format!("cannot restore launch target {}: {error}", target.display())
            })?;
        } else if target.exists() {
            fs::remove_file(&target).map_err(|error| error.to_string())?;
        }
    }
    remove_owned_transaction(pending)
}

fn read_journal(pending: &Path) -> Result<TransactionJournal, String> {
    let path = pending.join(JOURNAL_FILE);
    require_regular_file(&path)?;
    let meta = fs::metadata(&path).map_err(|error| error.to_string())?;
    if meta.len() > 1024 * 1024 {
        return Err("launch transaction journal is too large".to_string());
    }
    let journal: TransactionJournal =
        serde_json::from_slice(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("invalid launch transaction journal: {error}"))?;
    if journal.version != 1
        || journal.records.is_empty()
        || journal.records.len() > MAX_ACTIONS
        || journal.records.iter().any(|record| {
            super::validate_package_relative_path(&record.target).is_err()
                || record
                    .before_sha256
                    .as_ref()
                    .is_some_and(|hash| validate_sha256(hash).is_err())
                || record
                    .after_sha256
                    .as_ref()
                    .is_some_and(|hash| validate_sha256(hash).is_err())
        })
    {
        return Err("invalid launch transaction journal".to_string());
    }
    Ok(journal)
}

fn write_journal(pending: &Path, journal: &TransactionJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal).map_err(|error| error.to_string())?;
    let temporary = pending.join("journal.tmp");
    write_sync(&temporary, &bytes)?;
    fs::rename(temporary, pending.join(JOURNAL_FILE)).map_err(|error| error.to_string())
}

fn remove_owned_transaction(pending: &Path) -> Result<(), String> {
    if !pending.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(pending).map_err(|error| error.to_string())?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(format!(
            "unsafe launch transaction path: {}",
            pending.display()
        ));
    }
    for entry in fs::read_dir(pending).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "invalid launch transaction filename".to_string())?;
        let owned = matches!(name.as_str(), JOURNAL_FILE | "journal.tmp" | COMMITTED_FILE)
            || is_numbered_transaction_file(&name);
        if !owned {
            return Err(format!(
                "unexpected file in launch transaction: {}",
                entry.path().display()
            ));
        }
        require_regular_file(&entry.path())?;
        fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
    }
    fs::remove_dir(pending).map_err(|error| error.to_string())
}

fn is_numbered_transaction_file(name: &str) -> bool {
    let Some((index, suffix)) = name.split_once('.') else {
        return false;
    };
    !index.is_empty()
        && index.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(suffix, "old" | "new" | "restore")
}

fn hash_regular(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !reparse(&meta) => {
            let mut file = File::open(path).map_err(|error| error.to_string())?;
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            Ok(Some(format!("{:x}", digest.finalize())))
        }
        Ok(_) => Err(format!("unsafe launch file: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn require_regular_file(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|error| format!("missing launch file {}: {error}", path.display()))?;
    if !meta.is_file() || reparse(&meta) {
        return Err(format!("unsafe launch file: {}", path.display()));
    }
    Ok(())
}

fn copy_sync(source: &Path, target: &Path) -> Result<(), String> {
    require_regular_file(source)?;
    if target.exists() {
        return Err(format!(
            "pending launch file already exists: {}",
            target.display()
        ));
    }
    fs::copy(source, target).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .write(true)
        .open(target)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

fn write_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("invalid SHA256: {value}"))
    }
}

fn normalized_target(target: &str) -> String {
    target.replace('\\', "/").to_ascii_lowercase()
}

fn safe_transaction_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn next_transaction_id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default(),
        TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(windows)]
fn reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ssmt-launch-tx-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn failed_multi_file_transaction_restores_originals_and_removes_new_files() {
        let root = fixture("rollback");
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(root.join("dxgi.dll"), b"original").unwrap();
        fs::write(source.join("dxgi.dll"), b"replacement").unwrap();
        fs::write(source.join("new.dll"), b"new").unwrap();
        let expected = hash_regular(&root.join("dxgi.dll")).unwrap();
        let mutations = vec![
            FileMutation {
                target: "dxgi.dll".into(),
                source: Some(source.join("dxgi.dll")),
                expected_before_sha256: expected,
            },
            FileMutation {
                target: "new.dll".into(),
                source: Some(source.join("new.dll")),
                expected_before_sha256: None,
            },
        ];
        let error = execute_inner(&root, &mutations, Some(1), false).unwrap_err();
        assert!(error.contains("simulated"));
        assert_eq!(fs::read(root.join("dxgi.dll")).unwrap(), b"original");
        assert!(!root.join("new.dll").exists());
        assert!(!root.join(TRANSACTIONS_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_transaction_replaces_and_removes_files() {
        let root = fixture("success");
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(root.join("old.dll"), b"old").unwrap();
        fs::write(root.join("proxy.dll"), b"stock").unwrap();
        fs::write(source.join("proxy.dll"), b"managed").unwrap();
        let report = execute(
            &root,
            &[
                FileMutation {
                    target: "old.dll".into(),
                    source: None,
                    expected_before_sha256: None,
                },
                FileMutation {
                    target: "proxy.dll".into(),
                    source: Some(source.join("proxy.dll")),
                    expected_before_sha256: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(report.removed_files, vec!["old.dll"]);
        assert_eq!(report.replaced_files, vec!["proxy.dll"]);
        assert!(!root.join("old.dll").exists());
        assert_eq!(fs::read(root.join("proxy.dll")).unwrap(), b"managed");
        assert!(!root.join(TRANSACTIONS_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_changed_target_before_first_mutation() {
        let root = fixture("stale");
        fs::write(root.join("proxy.dll"), b"original").unwrap();
        let mutation = FileMutation {
            target: "proxy.dll".into(),
            source: None,
            expected_before_sha256: Some("0".repeat(64)),
        };
        assert!(execute(&root, &[mutation]).is_err());
        assert_eq!(fs::read(root.join("proxy.dll")).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persistent_deployment_is_restored_after_game_exit() {
        let root = fixture("persistent");
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(root.join("dxgi.dll"), b"stock").unwrap();
        fs::write(source.join("dxgi.dll"), b"managed").unwrap();
        let report = apply(
            &root,
            &[FileMutation {
                target: "dxgi.dll".into(),
                source: Some(source.join("dxgi.dll")),
                expected_before_sha256: None,
            }],
        )
        .unwrap();
        assert_eq!(fs::read(root.join("dxgi.dll")).unwrap(), b"managed");
        assert!(root.join(TRANSACTIONS_DIR).is_dir());
        assert!(apply(
            &root,
            &[FileMutation {
                target: "dxgi.dll".into(),
                source: None,
                expected_before_sha256: None,
            }]
        )
        .is_err());
        assert!(restore_applied(&root, &report.transaction_id).unwrap());
        assert_eq!(fs::read(root.join("dxgi.dll")).unwrap(), b"stock");
        assert!(!root.join(TRANSACTIONS_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_preserves_unrelated_user_edits() {
        let root = fixture("recovery");
        fs::write(root.join("proxy.dll"), b"original").unwrap();
        let transactions = transactions_root(&root, true).unwrap().unwrap();
        let pending = transactions.join("interrupted");
        fs::create_dir(&pending).unwrap();
        copy_sync(&root.join("proxy.dll"), &pending.join("0.old")).unwrap();
        let journal = TransactionJournal {
            version: 1,
            transaction_id: "interrupted".into(),
            records: vec![TransactionRecord {
                target: "proxy.dll".into(),
                before_sha256: hash_regular(&root.join("proxy.dll")).unwrap(),
                after_sha256: Some(format!("{:x}", Sha256::digest(b"managed"))),
            }],
        };
        write_journal(&pending, &journal).unwrap();
        fs::write(root.join("proxy.dll"), b"user edit").unwrap();
        assert!(recover_incomplete(&root).is_err());
        assert_eq!(fs::read(root.join("proxy.dll")).unwrap(), b"user edit");
        assert!(pending.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
