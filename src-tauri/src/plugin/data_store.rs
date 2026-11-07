//! Plugin-scoped persistent data files plus an optional launch mirror.
//!
//! Persistent files live outside the installed package so plugin upgrades do not
//! overwrite user presets. Runtime-mirrored directories are copied back into the
//! package only when a native runtime still expects package-relative files.

use super::registry::{PluginRegistry, PluginRegistryError};
use super::PluginDataDirectoryContribution;
use crate::config::path_manager::PathManager;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
const MIRROR_MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_MIRROR_FILES: usize = 100_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct ScopedDataStore {
    persistent_root: PathBuf,
    package_root: PathBuf,
    mirror_manifest: PathBuf,
    seed_from_package: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MirrorManifest {
    schema_version: u32,
    files: Vec<String>,
}

impl ScopedDataStore {
    pub fn for_plugin(plugin_id: &str, game_name: &str, data_id: &str) -> Result<Self, String> {
        let registry =
            PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
        let scoped = registry.for_game(game_name);
        let plugin = scoped
            .find(plugin_id)
            .filter(|plugin| plugin.enabled)
            .ok_or_else(|| format!("plugin is not installed or enabled: {plugin_id}"))?;
        let data = plugin
            .manifest
            .contributions
            .data_directories
            .iter()
            .find(|data| data.id == data_id)
            .ok_or_else(|| {
                format!("plugin data directory is not declared: {plugin_id}/{data_id}")
            })?;
        let persistent_root = PathManager::ssmt_global_config_folder()
            .join("PluginData")
            .join(plugin_id)
            .join(data_id);
        let package_root = plugin.package_root.join(&data.package_path);
        let mirror_manifest = PathManager::ssmt_global_config_folder()
            .join("PluginMirrorState")
            .join(plugin_id)
            .join(format!("{}.json", data_id));
        Ok(Self {
            persistent_root,
            package_root,
            mirror_manifest,
            seed_from_package: data.seed_from_package,
        })
    }

    pub fn ensure_seeded(&self) -> Result<(), String> {
        if self.persistent_root.exists() {
            require_plain_directory(&self.persistent_root)?;
            return Ok(());
        }
        if self.seed_from_package {
            require_plain_directory(&self.package_root)?;
            copy_tree(&self.package_root, &self.persistent_root)?;
        } else {
            fs::create_dir_all(&self.persistent_root).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn list(&self, relative: &str) -> Result<Vec<DataEntry>, String> {
        self.ensure_seeded()?;
        let root = checked_directory(&self.persistent_root, relative)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if reparse(&meta) {
                return Err(format!(
                    "plugin data contains a link: {}",
                    entry.path().display()
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", relative.trim_end_matches('/'), name)
            };
            entries.push(DataEntry {
                path,
                name,
                kind: if meta.is_dir() { "directory" } else { "file" }.to_string(),
                size: if meta.is_file() { meta.len() } else { 0 },
            });
        }
        entries.sort_by(|left, right| {
            (left.kind != "directory", left.path.to_ascii_lowercase())
                .cmp(&(right.kind != "directory", right.path.to_ascii_lowercase()))
        });
        Ok(entries)
    }

    pub fn read(&self, relative: &str) -> Result<String, String> {
        self.ensure_seeded()?;
        read_text_limited(&checked_file(&self.persistent_root, relative)?)
    }

    pub fn write_atomic(&self, relative: &str, content: &str) -> Result<(), String> {
        self.ensure_seeded()?;
        let target = checked_target(&self.persistent_root, relative, true)?;
        write_atomic(&target, content.as_bytes())
    }

    pub fn remove(&self, relative: &str) -> Result<bool, String> {
        self.ensure_seeded()?;
        let target = checked_target(&self.persistent_root, relative, false)?;
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_file() && !reparse(&meta) => {
                fs::remove_file(target).map_err(|error| error.to_string())?;
                Ok(true)
            }
            Ok(_) => Err(format!("plugin data path is not a file: {relative}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn publish(&self) -> Result<(), String> {
        self.ensure_seeded()?;
        require_plain_directory(&self.package_root)?;
        let current_files = collect_relative_files(&self.persistent_root)?;
        let previous = read_mirror_manifest(&self.mirror_manifest)?;
        copy_tree(&self.persistent_root, &self.package_root)?;
        let current_normalized: HashSet<String> = current_files
            .iter()
            .map(|path| path.to_ascii_lowercase())
            .collect();
        for stale in previous
            .files
            .iter()
            .filter(|path| !current_normalized.contains(&path.to_ascii_lowercase()))
        {
            remove_mirrored_file(&self.package_root, stale)?;
        }
        write_mirror_manifest(&self.mirror_manifest, current_files)
    }

    pub fn read_runtime(&self, relative: &str) -> Result<String, String> {
        read_text_limited(&checked_file(&self.package_root, relative)?)
    }
}

pub fn publish_for_game(game_name: &str) -> Result<usize, String> {
    let registry = PluginRegistry::from_default_location()
        .map_err(|error: PluginRegistryError| error.to_string())?;
    let mut published = 0usize;
    for plugin in registry
        .for_game(game_name)
        .selected_plugins()
        .filter(|plugin| plugin.enabled)
    {
        for data in plugin
            .manifest
            .contributions
            .data_directories
            .iter()
            .filter(|data| data.mirror_on_launch)
        {
            ScopedDataStore::for_contribution(plugin, data)?.publish()?;
            published += 1;
        }
    }
    Ok(published)
}

impl ScopedDataStore {
    fn for_contribution(
        plugin: &super::registry::InstalledPlugin,
        data: &PluginDataDirectoryContribution,
    ) -> Result<Self, String> {
        Ok(Self {
            persistent_root: PathManager::ssmt_global_config_folder()
                .join("PluginData")
                .join(&plugin.manifest.id)
                .join(&data.id),
            package_root: plugin.package_root.join(&data.package_path),
            mirror_manifest: PathManager::ssmt_global_config_folder()
                .join("PluginMirrorState")
                .join(&plugin.manifest.id)
                .join(format!("{}.json", data.id)),
            seed_from_package: data.seed_from_package,
        })
    }
}

fn checked_directory(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let target = checked_target(root, relative, false)?;
    let meta = fs::symlink_metadata(&target)
        .map_err(|error| format!("plugin data directory is unavailable: {error}"))?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(format!(
            "plugin data path is not a directory: {}",
            target.display()
        ));
    }
    Ok(target)
}

fn checked_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let target = checked_target(root, relative, false)?;
    let meta = fs::symlink_metadata(&target)
        .map_err(|error| format!("plugin data file is unavailable: {error}"))?;
    if !meta.is_file() || reparse(&meta) {
        return Err(format!(
            "plugin data path is not a file: {}",
            target.display()
        ));
    }
    Ok(target)
}

fn checked_target(root: &Path, relative: &str, create_parent: bool) -> Result<PathBuf, String> {
    require_plain_directory(root)?;
    if relative.is_empty() {
        return Ok(root.to_path_buf());
    }
    super::validate_package_relative_path(relative).map_err(|error| error.to_string())?;
    let target = root.join(relative);
    let parent_relative = Path::new(relative)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let mut current = root.to_path_buf();
    for component in parent_relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !reparse(&meta) => {}
            Ok(_) => {
                return Err(format!(
                    "plugin data parent is unsafe: {}",
                    current.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_parent => {
                fs::create_dir(&current).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(target)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    require_plain_directory(source)?;
    if destination.exists() {
        require_plain_directory(destination)?;
    } else {
        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    }
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source_dir, destination_dir)) = pending.pop() {
        for entry in fs::read_dir(&source_dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if reparse(&meta) {
                return Err(format!(
                    "plugin data cannot contain links: {}",
                    entry.path().display()
                ));
            }
            let target = destination_dir.join(entry.file_name());
            if meta.is_dir() {
                if target.exists() {
                    require_plain_directory(&target)?;
                } else {
                    fs::create_dir(&target).map_err(|error| error.to_string())?;
                }
                pending.push((entry.path(), target));
            } else if meta.is_file() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::copy(entry.path(), &target).map_err(|error| error.to_string())?;
            } else {
                return Err(format!(
                    "unsupported plugin data entry: {}",
                    entry.path().display()
                ));
            }
        }
    }
    Ok(())
}

fn collect_relative_files(root: &Path) -> Result<Vec<String>, String> {
    require_plain_directory(root)?;
    let mut files = BTreeSet::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if reparse(&meta) {
                return Err(format!(
                    "plugin data mirror cannot contain links: {}",
                    entry.path().display()
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_dir() {
                pending.push((entry.path(), relative));
            } else if meta.is_file() {
                super::validate_package_relative_path(&relative)
                    .map_err(|error| error.to_string())?;
                files.insert(relative);
                if files.len() > MAX_MIRROR_FILES {
                    return Err(format!(
                        "plugin data mirror exceeds {MAX_MIRROR_FILES} files"
                    ));
                }
            } else {
                return Err(format!(
                    "unsupported plugin data entry: {}",
                    entry.path().display()
                ));
            }
        }
    }
    Ok(files.into_iter().collect())
}

fn read_mirror_manifest(path: &Path) -> Result<MirrorManifest, String> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            let manifest: MirrorManifest =
                serde_json::from_str(&raw).map_err(|error| error.to_string())?;
            if manifest.schema_version != MIRROR_MANIFEST_SCHEMA_VERSION {
                return Err(format!(
                    "unsupported plugin mirror manifest schema: {}",
                    manifest.schema_version
                ));
            }
            Ok(manifest)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(MirrorManifest {
            schema_version: MIRROR_MANIFEST_SCHEMA_VERSION,
            files: Vec::new(),
        }),
        Err(error) => Err(error.to_string()),
    }
}

fn write_mirror_manifest(path: &Path, files: Vec<String>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let manifest = MirrorManifest {
        schema_version: MIRROR_MANIFEST_SCHEMA_VERSION,
        files,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    write_atomic(path, &bytes)
}

fn remove_mirrored_file(root: &Path, relative: &str) -> Result<(), String> {
    let target = checked_target(root, relative, false)?;
    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() && !reparse(&meta) => {
            fs::remove_file(target).map_err(|error| error.to_string())
        }
        Ok(_) => Err(format!(
            "plugin mirror path is not a regular file: {relative}"
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_text_limited(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|error| error.to_string())?;
    if meta.len() > MAX_READ_BYTES {
        return Err(format!(
            "plugin data file exceeds 16 MiB: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes).map_err(|error| format!("plugin data is not UTF-8 text: {error}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "plugin data file has no parent".to_string())?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("data"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    drop(file);
    replace_file(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(format!(
            "cannot replace plugin data file: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

fn require_plain_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|error| format!("plugin data directory is unlocked/unavailable: {error}"))?;
    if !meta.is_dir() || reparse(&meta) {
        return Err(format!(
            "plugin data path is not a regular directory: {}",
            path.display()
        ));
    }
    Ok(())
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

    fn fixture(label: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ssmt-plugin-data-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let persistent = root.join("persistent");
        let package = root.join("package");
        fs::create_dir_all(package.join("presets")).unwrap();
        fs::write(package.join("presets/Default.json"), b"{\"seed\":true}").unwrap();
        (persistent, package)
    }

    fn store(persistent: &Path, package: &Path) -> ScopedDataStore {
        ScopedDataStore {
            persistent_root: persistent.to_path_buf(),
            package_root: package.to_path_buf(),
            mirror_manifest: package
                .parent()
                .unwrap_or(package)
                .join("mirror-state.json"),
            seed_from_package: true,
        }
    }

    #[test]
    fn seeds_writes_and_publishes_scoped_data() {
        let (persistent, package) = fixture("roundtrip");
        let data = store(&persistent, &package);
        assert_eq!(
            data.read("presets/Default.json").unwrap(),
            "{\"seed\":true}"
        );
        data.write_atomic("runtime/config.json", "{\"revision\":1}")
            .unwrap();
        data.publish().unwrap();
        assert_eq!(
            fs::read_to_string(package.join("runtime/config.json")).unwrap(),
            "{\"revision\":1}"
        );
        assert_eq!(
            data.read_runtime("presets/Default.json").unwrap(),
            "{\"seed\":true}"
        );
        fs::remove_dir_all(persistent.parent().unwrap()).unwrap();
    }

    #[test]
    fn rejects_traversal_and_symlink_parents() {
        let (persistent, package) = fixture("paths");
        let data = store(&persistent, &package);
        data.ensure_seeded().unwrap();
        assert!(data.write_atomic("../outside.json", "{}").is_err());
        let link = persistent.join("linked");
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_dir(&package, &link);
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(&package, &link);
        if result.is_ok() {
            assert!(data.write_atomic("linked/escape.json", "{}").is_err());
        }
        fs::remove_dir_all(persistent.parent().unwrap()).unwrap();
    }

    #[test]
    fn publish_removes_files_deleted_from_the_persistent_mirror() {
        let (persistent, package) = fixture("mirror-delete");
        let data = store(&persistent, &package);
        data.ensure_seeded().unwrap();
        data.write_atomic("runtime/old.json", "old").unwrap();
        data.write_atomic("runtime/keep.json", "keep").unwrap();
        data.publish().unwrap();
        assert!(package.join("runtime/old.json").is_file());

        data.remove("runtime/old.json").unwrap();
        data.publish().unwrap();
        assert!(!package.join("runtime/old.json").exists());
        assert_eq!(data.read_runtime("runtime/keep.json").unwrap(), "keep");
        fs::remove_dir_all(persistent.parent().unwrap()).unwrap();
    }
}
