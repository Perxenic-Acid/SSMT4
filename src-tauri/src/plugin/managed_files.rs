//! 自有目录中的多文件提交。恢复前比较实际内容，避免覆盖用户后续修改。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const TRANSACTION: &str = ".ssmt-pending-files";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    name: String,
    before: Option<String>,
    after: Option<String>,
}

pub(super) fn hash(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let linked = metadata.file_type().is_symlink();
            if !metadata.is_file() || linked {
                return Err(format!("受管文件不是普通文件：{}", path.display()));
            }
            let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
            let mut digest = Sha256::new();
            std::io::copy(&mut file, &mut digest).map_err(|e| e.to_string())?;
            Ok(Some(format!("{:x}", digest.finalize())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn safe_name(name: &str) -> bool {
    super::validate_package_relative_path(name).is_ok()
        && !name
            .replace('\\', "/")
            .split('/')
            .any(|part| part.starts_with(".ssmt-"))
}

fn sync_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn remove_transaction(path: &Path) -> Result<(), String> {
    // 调用方只传入已校验根目录下的固定事务目录，不遍历外部链接。
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        hash(&entry.path())?;
        fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
    }
    fs::remove_dir(path).map_err(|e| e.to_string())
}

/// 持有期间串行执行读取、组合、提交和恢复；进程结束后系统自动释放文件锁。
pub(super) struct DirectoryTransaction {
    root: PathBuf,
    _lock: fs::File,
}

impl DirectoryTransaction {
    pub(super) fn open(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let lock_path = root.join(".ssmt-files.lock");
        if lock_path.exists() {
            hash(&lock_path)?;
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let lock = options
            .open(lock_path)
            .map_err(|e| format!("配置正在准备或无法加锁：{e}"))?;
        let transaction = Self { root, _lock: lock };
        transaction.recover()?;
        Ok(transaction)
    }

    fn target(&self, name: &str) -> Result<PathBuf, String> {
        if !safe_name(name) {
            return Err("受管文件路径无效".into());
        }
        let target = self.root.join(name);
        let parent = target.parent().ok_or("受管文件缺少父目录")?;
        if parent.canonicalize().map_err(|e| e.to_string())? != parent {
            return Err("受管文件的父目录不能包含链接".into());
        }
        Ok(target)
    }

    fn pending(&self) -> Result<PathBuf, String> {
        let path = self.root.join(TRANSACTION);
        if path.exists() && path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("事务目录不能是链接".into());
        }
        Ok(path)
    }

    fn recover(&self) -> Result<(), String> {
        let pending = self.pending()?;
        if !pending.exists() {
            return Ok(());
        }
        let journal = pending.join("journal.json");
        if !journal.exists() || pending.join("committed").exists() {
            return remove_transaction(&pending);
        }
        hash(&journal)?;
        let records: Vec<Record> =
            serde_json::from_slice(&fs::read(&journal).map_err(|e| e.to_string())?)
                .map_err(|e| format!("无法读取配置恢复记录：{e}"))?;
        // 全部检查后再恢复，遇到并发修改时保留现场。
        for (index, record) in records.iter().enumerate() {
            if !safe_name(&record.name) {
                return Err("配置恢复记录路径无效".into());
            }
            let current = hash(&self.target(&record.name)?)?;
            if current != record.before && current != record.after {
                return Err(format!(
                    "{} 在配置准备后被修改，已保留文件及恢复记录",
                    record.name
                ));
            }
            if record.before.is_some()
                && hash(&pending.join(format!("{index}.old")))? != record.before
            {
                return Err("配置备份校验失败，已保留恢复记录".into());
            }
        }
        for (index, record) in records.iter().enumerate().rev() {
            let destination = self.target(&record.name)?;
            if hash(&destination)? == record.before {
                continue;
            }
            if record.before.is_some() {
                let temporary = pending.join(format!("{index}.restore"));
                if temporary.exists() {
                    hash(&temporary)?;
                    fs::remove_file(&temporary).map_err(|e| e.to_string())?;
                }
                fs::copy(pending.join(format!("{index}.old")), &temporary)
                    .map_err(|e| e.to_string())?;
                fs::rename(temporary, destination).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(destination).map_err(|e| e.to_string())?;
            }
        }
        remove_transaction(&pending)
    }

    pub(super) fn commit(&self, changes: BTreeMap<String, Option<Vec<u8>>>) -> Result<(), String> {
        self.commit_inner(changes, None)
    }

    fn commit_inner(
        &self,
        changes: BTreeMap<String, Option<Vec<u8>>>,
        fail_after: Option<usize>,
    ) -> Result<(), String> {
        let mut names = std::collections::HashSet::new();
        for name in changes.keys() {
            if !names.insert(name.to_ascii_lowercase()) {
                return Err("受管文件路径重复".into());
            }
            if !safe_name(name) {
                return Err(format!("受管文件名无效：{name}"));
            }
        }
        let pending = self.pending()?;
        fs::create_dir(&pending).map_err(|e| e.to_string())?;
        let result = (|| {
            let mut records = Vec::new();
            for (index, (name, content)) in changes.iter().enumerate() {
                let target = self.target(name)?;
                let before = hash(&target)?;
                if before.is_some() {
                    fs::copy(&target, pending.join(format!("{index}.old")))
                        .map_err(|e| e.to_string())?;
                }
                let after = match content {
                    Some(bytes) => {
                        sync_write(&pending.join(format!("{index}.new")), bytes)?;
                        Some(format!("{:x}", Sha256::digest(bytes)))
                    }
                    None => None,
                };
                records.push(Record {
                    name: name.clone(),
                    before,
                    after,
                });
            }
            sync_write(
                &pending.join("journal.tmp"),
                &serde_json::to_vec(&records).map_err(|e| e.to_string())?,
            )?;
            fs::rename(pending.join("journal.tmp"), pending.join("journal.json"))
                .map_err(|e| e.to_string())?;
            for (index, record) in records.iter().enumerate() {
                let target = self.target(&record.name)?;
                if hash(&target)? != record.before {
                    return Err(format!("{} 在准备期间被修改", record.name));
                }
                if record.after.is_some() {
                    fs::rename(pending.join(format!("{index}.new")), target)
                        .map_err(|e| e.to_string())?;
                } else if record.before.is_some() {
                    fs::remove_file(target).map_err(|e| e.to_string())?;
                }
                if fail_after == Some(index + 1) {
                    return Err("模拟配置提交中断".into());
                }
            }
            sync_write(&pending.join("committed"), b"1")?;
            Ok(())
        })();
        match result {
            Ok(()) => remove_transaction(&pending),
            Err(error) => {
                let recovery = self.recover();
                Err(match recovery {
                    Ok(()) => error,
                    Err(restore) => format!("{error}；恢复待处理：{restore}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_group_restores_existing_and_removes_new_files() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-files-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.ini"), b"player settings").unwrap();
        let tx = DirectoryTransaction::open(&root).unwrap();
        let changes = BTreeMap::from([
            ("a.ini".into(), Some(b"new".to_vec())),
            ("b.addon64".into(), Some(b"addon".to_vec())),
        ]);
        assert!(tx.commit_inner(changes.clone(), Some(2)).is_err());
        assert_eq!(fs::read(root.join("a.ini")).unwrap(), b"player settings");
        assert!(!root.join("b.addon64").exists());
        tx.commit(changes).unwrap();
        assert_eq!(fs::read(root.join("a.ini")).unwrap(), b"new");
        drop(tx);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_recovery_preserves_later_user_edits() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-files-recover-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let pending = root.join(TRANSACTION);
        fs::create_dir(&pending).unwrap();
        fs::write(pending.join("0.old"), b"old").unwrap();
        fs::write(root.join("ReShade.ini"), b"user changed").unwrap();
        let records = vec![Record {
            name: "ReShade.ini".into(),
            before: hash(&pending.join("0.old")).unwrap(),
            after: Some(format!("{:x}", Sha256::digest(b"applied"))),
        }];
        fs::write(
            pending.join("journal.json"),
            serde_json::to_vec(&records).unwrap(),
        )
        .unwrap();
        assert!(DirectoryTransaction::open(&root).is_err());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"user changed");
        fs::write(root.join("ReShade.ini"), b"applied").unwrap();
        drop(DirectoryTransaction::open(&root).unwrap());
        assert_eq!(fs::read(root.join("ReShade.ini")).unwrap(), b"old");
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
