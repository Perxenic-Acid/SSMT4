use super::registry::{InstalledPlugin, PluginRegistry, PluginRegistryError};
use super::{validate_package_relative_path, PluginManifest, PluginManifestError};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const MANIFEST_FILE_NAME: &str = "ssmt-plugin.json";
const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_ENTRY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum PackageInstallError {
    Io(io::Error),
    Zip(zip::result::ZipError),
    Manifest(PluginManifestError),
    Registry(PluginRegistryError),
    UnsafePath(String),
    SymlinkEntry(String),
    MissingManifest,
    InvalidArchive(String),
    ResourceLimit(String),
    DuplicateEntry(String),
}

impl std::fmt::Display for PackageInstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "package I/O error: {error}"),
            Self::Zip(error) => write!(formatter, "invalid package archive: {error}"),
            Self::Manifest(error) => write!(formatter, "package manifest error: {error}"),
            Self::Registry(error) => write!(formatter, "plugin registry error: {error}"),
            Self::UnsafePath(path) => write!(formatter, "unsafe package path: {path}"),
            Self::SymlinkEntry(path) => write!(formatter, "package contains symlink: {path}"),
            Self::MissingManifest => write!(formatter, "package manifest is missing"),
            Self::InvalidArchive(error) => write!(formatter, "invalid package archive: {error}"),
            Self::ResourceLimit(detail) => {
                write!(formatter, "package resource limit exceeded: {detail}")
            }
            Self::DuplicateEntry(path) => {
                write!(formatter, "duplicate package archive entry: {path}")
            }
        }
    }
}

impl std::error::Error for PackageInstallError {}

impl From<io::Error> for PackageInstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<zip::result::ZipError> for PackageInstallError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error)
    }
}

impl From<PluginManifestError> for PackageInstallError {
    fn from(error: PluginManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<PluginRegistryError> for PackageInstallError {
    fn from(error: PluginRegistryError) -> Self {
        Self::Registry(error)
    }
}

pub fn install_ssmtpkg(
    registry: &mut PluginRegistry,
    archive_path: &Path,
) -> Result<InstalledPlugin, PackageInstallError> {
    if !archive_path.is_file() {
        return Err(PackageInstallError::InvalidArchive(format!(
            "archive does not exist: {}",
            archive_path.display()
        )));
    }
    let temporary = TemporaryDirectory::create()?;
    extract_archive(archive_path, temporary.path())?;
    let manifest_path = temporary.path().join(MANIFEST_FILE_NAME);
    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(PackageInstallError::MissingManifest);
        }
        Err(error) => return Err(PackageInstallError::Io(error)),
    };
    PluginManifest::from_json_str(&raw)?;
    registry
        .install_directory_package(temporary.path())
        .map_err(PackageInstallError::from)
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn create() -> io::Result<Self> {
        let base = std::env::temp_dir();
        for attempt in 0..16 {
            let path = base.join(format!(
                "ssmt-package-install-{}-{}-{attempt}",
                std::process::id(),
                unique_suffix()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique package extraction directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<(), PackageInstallError> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(PackageInstallError::ResourceLimit(format!(
            "{} entries exceeds limit {MAX_ARCHIVE_ENTRIES}",
            archive.len()
        )));
    }
    let mut seen = HashMap::<String, bool>::new();
    let mut declared_total = 0u64;
    let mut extracted_total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let raw_name = entry.name().to_string();
        let relative_name = raw_name.trim_end_matches('/');
        if relative_name.is_empty() {
            continue;
        }
        let normalized = relative_name.replace('\\', "/").to_lowercase();
        let is_directory = entry.is_dir();
        if seen.contains_key(&normalized)
            || normalized
                .split('/')
                .scan(String::new(), |parent, segment| {
                    if !parent.is_empty() {
                        parent.push('/');
                    }
                    parent.push_str(segment);
                    Some(parent.clone())
                })
                .take_while(|parent| parent != &normalized)
                .any(|parent| seen.get(&parent) == Some(&false))
            || (!is_directory
                && seen
                    .keys()
                    .any(|path| path.starts_with(&format!("{normalized}/"))))
        {
            return Err(PackageInstallError::DuplicateEntry(raw_name));
        }
        seen.insert(normalized, is_directory);
        validate_package_relative_path(relative_name)
            .map_err(|_| PackageInstallError::UnsafePath(raw_name.clone()))?;
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(PackageInstallError::ResourceLimit(format!(
                "entry exceeds {MAX_ENTRY_BYTES} bytes: {raw_name}"
            )));
        }
        declared_total = declared_total.saturating_add(entry.size());
        if declared_total > MAX_ARCHIVE_BYTES {
            return Err(PackageInstallError::ResourceLimit(format!(
                "declared package size exceeds {MAX_ARCHIVE_BYTES} bytes"
            )));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(PackageInstallError::SymlinkEntry(raw_name));
        }
        let output_path = destination.join(PathBuf::from(relative_name.replace('\\', "/")));
        if is_directory {
            fs::create_dir_all(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&output_path)?;
        let copied = io::copy(&mut entry.by_ref().take(MAX_ENTRY_BYTES + 1), &mut output)?;
        if copied > MAX_ENTRY_BYTES {
            return Err(PackageInstallError::ResourceLimit(format!(
                "entry exceeds {MAX_ENTRY_BYTES} bytes: {raw_name}"
            )));
        }
        extracted_total = extracted_total.saturating_add(copied);
        if extracted_total > MAX_ARCHIVE_BYTES {
            return Err(PackageInstallError::ResourceLimit(format!(
                "extracted package size exceeds {MAX_ARCHIVE_BYTES} bytes"
            )));
        }
        if copied != entry.size() {
            return Err(PackageInstallError::InvalidArchive(format!(
                "entry size mismatch: {raw_name}"
            )));
        }
    }
    Ok(())
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::FileOptions;

    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-package-installer-{label}-{}",
            unique_suffix()
        ))
    }

    fn manifest(id: &str) -> String {
        format!(
            r#"{{
          "schemaVersion": 1, "id": "{id}", "name": "Fixture", "version": "1.0.0", "author": "Test",
          "compatibility": {{"ssmt": ">=4.0.0", "platforms": ["windows-x64"]}},
          "contributions": {{}}, "permissions": []
        }}"#
        )
    }

    fn zip_fixture(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, contents) in entries {
            archive.start_file(*name, FileOptions::default()).unwrap();
            archive.write_all(contents).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn installs_archive_atomically_and_discovers_plugin() {
        let root = root("atomic");
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("fixture.ssmtpkg");
        zip_fixture(
            &archive,
            &[
                ("ssmt-plugin.json", manifest("ssmt.archive").as_bytes()),
                ("assets/readme.txt", b"ok"),
            ],
        );
        let mut registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        let installed = install_ssmtpkg(&mut registry, &archive).unwrap();
        assert_eq!(installed.id(), "ssmt.archive");
        assert!(installed.package_root.join("assets/readme.txt").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_zip_slip_before_installing_anything() {
        let root = root("zip-slip");
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("unsafe.ssmtpkg");
        zip_fixture(
            &archive,
            &[
                ("../outside.txt", b"nope"),
                ("ssmt-plugin.json", manifest("ssmt.unsafe").as_bytes()),
            ],
        );
        let plugins_root = root.join("Plugins");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        assert!(matches!(
            install_ssmtpkg(&mut registry, &archive),
            Err(PackageInstallError::UnsafePath(_))
        ));
        assert!(!root.join("outside.txt").exists());
        assert!(registry.installed().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_archive_without_manifest() {
        let root = root("missing-manifest");
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("missing.ssmtpkg");
        zip_fixture(&archive, &[("readme.txt", b"no manifest")]);
        let mut registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        assert!(matches!(
            install_ssmtpkg(&mut registry, &archive),
            Err(PackageInstallError::MissingManifest)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_duplicate_paths_before_installing() {
        let root = root("duplicate-entry");
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("duplicate.ssmtpkg");
        zip_fixture(
            &archive,
            &[
                ("ssmt-plugin.json", manifest("ssmt.duplicate").as_bytes()),
                ("assets/readme.txt", b"first"),
                ("ASSETS/README.TXT", b"second"),
            ],
        );
        let plugins_root = root.join("Plugins");
        let mut registry = PluginRegistry::new(&plugins_root).unwrap();
        assert!(matches!(
            install_ssmtpkg(&mut registry, &archive),
            Err(PackageInstallError::DuplicateEntry(_))
        ));
        assert!(registry.installed().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_file_directory_path_collisions() {
        let root = root("path-collision");
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("collision.ssmtpkg");
        zip_fixture(
            &archive,
            &[("assets", b"file"), ("assets/readme.txt", b"child")],
        );
        let mut registry = PluginRegistry::new(root.join("Plugins")).unwrap();
        assert!(matches!(
            install_ssmtpkg(&mut registry, &archive),
            Err(PackageInstallError::DuplicateEntry(_))
        ));
        assert!(registry.installed().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
