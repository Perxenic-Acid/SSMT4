use super::registry::{InstalledPlugin, PluginRegistry, PluginRegistryError};
use super::{validate_package_relative_path, PluginManifest, PluginManifestError};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const MANIFEST_FILE_NAME: &str = "ssmt-plugin.json";

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
    let temporary = std::env::temp_dir().join(format!("ssmt-package-install-{}", unique_suffix()));
    fs::create_dir_all(&temporary)?;
    if let Err(error) = extract_archive(archive_path, &temporary) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    let manifest_path = temporary.join(MANIFEST_FILE_NAME);
    let raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let _ = fs::remove_dir_all(&temporary);
            return Err(PackageInstallError::MissingManifest);
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            return Err(PackageInstallError::Io(error));
        }
    };
    PluginManifest::from_json_str(&raw)?;
    let result = registry.install_directory_package(&temporary);
    let _ = fs::remove_dir_all(&temporary);
    result.map_err(PackageInstallError::from)
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<(), PackageInstallError> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let raw_name = entry.name().to_string();
        let relative_name = raw_name.trim_end_matches('/');
        if relative_name.is_empty() {
            continue;
        }
        validate_package_relative_path(relative_name)
            .map_err(|_| PackageInstallError::UnsafePath(raw_name.clone()))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(PackageInstallError::SymlinkEntry(raw_name));
        }
        let output_path = destination.join(PathBuf::from(relative_name));
        if entry.is_dir() {
            fs::create_dir_all(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&output_path)?;
        io::copy(&mut entry, &mut output)?;
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
}
