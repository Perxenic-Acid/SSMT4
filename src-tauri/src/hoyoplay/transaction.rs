use super::{client, manifest};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    let reparse = meta.file_attributes() & 0x400 != 0;
    #[cfg(not(windows))]
    let reparse = false;
    meta.file_type().is_symlink() || reparse
}

pub fn safe(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = manifest::relative(name)?;
    let mut path = root.to_path_buf();
    for (index, part) in relative.components().enumerate() {
        path.push(part);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if is_link(&meta) {
                // 只允许原神根目录的双向兼容别名；目标必须是同目录下无链接的真实数据目录。
                let alias = part.as_os_str().to_string_lossy();
                let sibling = if alias.eq_ignore_ascii_case("GenshinImpact_Data") {
                    Some("YuanShen_Data")
                } else if alias.eq_ignore_ascii_case("YuanShen_Data") {
                    Some("GenshinImpact_Data")
                } else {
                    None
                };
                if let Some(sibling) = sibling.filter(|_| index == 0) {
                    let target = root.join(sibling);
                    if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir() && !is_link(&m)) {
                        let resolved = fs::canonicalize(&path).map_err(|e| e.to_string())?;
                        if resolved == fs::canonicalize(&target).map_err(|e| e.to_string())? {
                            path = resolved;
                            continue;
                        }
                    }
                }
                return Err(format!("资源路径包含链接：{}", path.display()));
            }
        }
    }
    Ok(path)
}

pub fn target_key(root: &Path, name: &str) -> Result<String, String> {
    Ok(safe(root, name)?.to_string_lossy().to_lowercase())
}

pub fn hash(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(format!("预期文件但遇到目录：{}", path.display()));
    }
    crate::utils::ssmt_file_utils::SSMTFileUtils::file_md5(path).map(Some)
}

pub fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit {
        return Err(format!("文件过大：{}", path.display()));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("读取文件时大小超出限制".into());
    }
    Ok(bytes)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let temp = path.with_extension("ssmt-writing");
    if let Some(parent) = temp.parent() {
        safe(
            parent,
            temp.file_name()
                .and_then(|s| s.to_str())
                .ok_or("临时文件名无效")?,
        )?;
    }
    let mut file = File::create(&temp).map_err(|e| format!("无法写入 {}：{e}", temp.display()))?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|e| e.to_string())?;
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temp, path).map_err(|e| format!("无法提交 {}：{e}", path.display()))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Change {
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    pub changes: Vec<Change>,
    pub config_before: serde_json::Value,
    pub config_after: serde_json::Value,
}
pub struct Workspace {
    pub root: PathBuf,
    pub work: PathBuf,
    pub stage: PathBuf,
    pub backup: PathBuf,
    _lock: File,
}

impl Workspace {
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_absolute()
            || root.parent().is_none()
            || root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("请选择明确的游戏目录，不能使用磁盘根目录或相对路径".into());
        }
        fs::create_dir_all(root).map_err(|e| format!("无法创建游戏目录：{e}"))?;
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let work = root.join(".ssmt-maintenance");
        if let Ok(meta) = fs::symlink_metadata(&work) {
            #[cfg(windows)]
            let reparse = meta.file_attributes() & 0x400 != 0;
            #[cfg(not(windows))]
            let reparse = false;
            if meta.file_type().is_symlink() || reparse {
                return Err("事务目录不能是链接".into());
            }
            let owner = fs::read_to_string(safe(&work, "owner.json")?)
                .map_err(|_| "已有同名事务目录不属于 SSMT")?;
            if serde_json::from_str::<serde_json::Value>(&owner).map_err(|e| e.to_string())?
                != serde_json::json!({"schema":1,"owner":"SSMT.HoYoPlay","root":root})
            {
                return Err("事务目录所有权不匹配".into());
            }
        } else {
            fs::create_dir(&work).map_err(|e| e.to_string())?;
            write_json(
                &work.join("owner.json"),
                &serde_json::json!({"schema":1,"owner":"SSMT.HoYoPlay","root":root}),
            )?;
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(windows)]
        options.share_mode(0);
        let lock = options
            .open(safe(&work, "lock")?)
            .map_err(|e| format!("客户端已有维护任务或事务锁无法打开：{e}"))?;
        let stage = safe(&work, "stage")?;
        let backup = safe(&work, "backup")?;
        fs::create_dir_all(&stage)
            .and_then(|_| fs::create_dir_all(&backup))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            work,
            stage,
            backup,
            _lock: lock,
        })
    }

    fn read_journal(&self, name: &str) -> Result<Journal, String> {
        let journal: Journal =
            serde_json::from_slice(&read_limited(&safe(&self.work, name)?, 64 * 1024 * 1024)?)
                .map_err(|e| format!("事务记录损坏，保留备份等待处理：{e}"))?;
        if journal.schema != 1 || journal.changes.len() > 1_000_000 {
            return Err("未知事务记录格式".into());
        }
        validate_config_fields(&journal.config_before, &journal.config_after)?;
        let mut unique = std::collections::HashSet::new();
        let mut targets = std::collections::HashSet::new();
        for change in &journal.changes {
            manifest::relative(&change.name)?;
            if !unique.insert(change.name.to_lowercase()) {
                return Err("事务记录含重复路径".into());
            }
            if !targets.insert(target_key(&self.root, &change.name)?) {
                return Err("事务记录包含指向同一文件的重复路径".into());
            }
            for hash in [&change.before, &change.after].into_iter().flatten() {
                client::validate_md5(hash)?;
            }
        }
        Ok(journal)
    }

    pub fn recover(&self, config_path: &Path) -> Result<(), String> {
        if !self.work.join("journal.json").exists() {
            return Ok(());
        }
        let journal = self.read_journal("journal.json")?;
        // 先检查全部文件，玩家后续修改不能在恢复过程中被覆盖。
        for change in &journal.changes {
            let target = safe(&self.root, &change.name)?;
            let backup = safe(&self.backup, &change.name)?;
            let actual = hash(&target)?;
            let saved = hash(&backup)?;
            if saved.is_some() && saved != change.before {
                return Err(format!("备份被修改：{}", change.name));
            }
            if actual.is_some() && actual != change.after && actual != change.before {
                return Err(format!("恢复遇到后续修改，已保留事务：{}", change.name));
            }
            if change.before.is_some() && saved.is_none() && actual != change.before {
                return Err(format!("恢复缺少原文件备份：{}", change.name));
            }
        }
        for change in journal.changes.iter().rev() {
            let target = safe(&self.root, &change.name)?;
            let backup = safe(&self.backup, &change.name)?;
            if backup.exists() {
                if target.exists() {
                    fs::remove_file(&target).map_err(|e| e.to_string())?;
                }
                fs::create_dir_all(target.parent().ok_or("目标目录无效")?)
                    .map_err(|e| e.to_string())?;
                fs::rename(backup, target).map_err(|e| e.to_string())?;
            } else if change.before.is_none() && target.exists() {
                fs::remove_file(target).map_err(|e| e.to_string())?;
            }
        }
        patch_config(config_path, &journal.config_after, &journal.config_before)?;
        fs::remove_file(self.work.join("journal.json")).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn commit(
        &self,
        changes: Vec<Change>,
        config_path: &Path,
        before: serde_json::Value,
        after: serde_json::Value,
    ) -> Result<(), String> {
        validate_config_fields(&before, &after)?;
        if self.work.join("journal.json").exists() {
            return Err("有未恢复事务，不能覆盖".into());
        }
        let mut targets = std::collections::HashSet::new();
        for change in &changes {
            if !targets.insert(target_key(&self.root, &change.name)?) {
                return Err("事务包含指向同一文件的重复路径".into());
            }
            if hash(&safe(&self.root, &change.name)?)? != change.before {
                return Err(format!("提交前文件发生变化：{}", change.name));
            }
            if let Some(expected) = &change.after {
                if hash(&safe(&self.stage, &change.name)?)?.as_ref() != Some(expected) {
                    return Err(format!("暂存文件校验失败：{}", change.name));
                }
            }
            // 上轮已完成的备份只按记录删除已知文件，不递归清理未知目录。
            let backup = safe(&self.backup, &change.name)?;
            if backup.exists() {
                return Err(format!("仍存在未处理备份：{}", change.name));
            }
        }
        let journal = Journal {
            schema: 1,
            changes,
            config_before: before,
            config_after: after,
        };
        write_json(&safe(&self.work, "journal.json")?, &journal)?;
        let result = (|| {
            for change in &journal.changes {
                let target = safe(&self.root, &change.name)?;
                let backup = safe(&self.backup, &change.name)?;
                if change.before.is_some() {
                    fs::create_dir_all(backup.parent().ok_or("备份路径无效")?)
                        .map_err(|e| e.to_string())?;
                    fs::rename(&target, &backup)
                        .map_err(|e| format!("文件可能被占用：{}：{e}", change.name))?;
                }
                if change.after.is_some() {
                    fs::create_dir_all(target.parent().ok_or("目标路径无效")?)
                        .map_err(|e| e.to_string())?;
                    fs::rename(safe(&self.stage, &change.name)?, target)
                        .map_err(|e| e.to_string())?;
                }
            }
            patch_config(config_path, &journal.config_before, &journal.config_after)?;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            return match self.recover(config_path) {
                Ok(()) => Err(format!("{error}；已恢复原文件")),
                Err(recovery) => Err(format!("{error}；恢复未完成：{recovery}")),
            };
        }
        // 先留下完成记录再清理备份，进程中断后不会把成功更新误当作失败恢复。
        fs::rename(
            self.work.join("journal.json"),
            safe(&self.work, "completed.json")?,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn cleanup_completed(&self) -> Result<(), String> {
        if !self.work.join("completed.json").exists() {
            return Ok(());
        }
        let journal = self.read_journal("completed.json")?;
        for change in &journal.changes {
            let path = safe(&self.backup, &change.name)?;
            if path.exists() {
                if hash(&path)? != change.before {
                    return Err("已完成事务的备份被修改，暂缓清理".into());
                }
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        fs::remove_file(self.work.join("completed.json")).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn patch_config(
    path: &Path,
    expected: &serde_json::Value,
    replacement: &serde_json::Value,
) -> Result<(), String> {
    validate_config_fields(expected, replacement)?;
    if path.as_os_str().is_empty() {
        return Ok(());
    } // 隔离测试不需要应用配置。
    let mut current: serde_json::Value =
        serde_json::from_slice(&read_limited(path, 8 * 1024 * 1024)?).map_err(|e| e.to_string())?;
    for (key, value) in expected.as_object().ok_or("配置事务格式无效")? {
        if current.get(key).unwrap_or(&serde_json::Value::Null) != value
            && current.get(key).unwrap_or(&serde_json::Value::Null) != &replacement[key]
        {
            return Err(format!("游戏设置中的 {key} 已被更改，保留当前设置"));
        }
    }
    for (key, value) in replacement.as_object().ok_or("配置事务格式无效")? {
        if value.is_null() {
            current.as_object_mut().ok_or("应用配置无效")?.remove(key);
        } else {
            current[key] = value.clone();
        }
    }
    write_json(path, &current)
}

fn validate_config_fields(
    before: &serde_json::Value,
    after: &serde_json::Value,
) -> Result<(), String> {
    let before = before.as_object().ok_or("配置事务格式无效")?;
    let after = after.as_object().ok_or("配置事务格式无效")?;
    if before.len() != after.len()
        || before.keys().any(|k| {
            !after.contains_key(k)
                || !["targetExePath", "hoyoplayServer", "hoyoplayAudioLanguages"]
                    .contains(&k.as_str())
        })
    {
        return Err("配置事务包含未知字段".into());
    }
    Ok(())
}
