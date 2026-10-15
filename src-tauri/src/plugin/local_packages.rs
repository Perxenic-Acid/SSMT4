use super::package_installer::{inspect_ssmtpkg, PackageInspection};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPluginPackage {
    pub archive_path: PathBuf,
    pub inspection: PackageInspection,
}

/// 只发现构建脚本放好的安装包；发现不安装、不启用，也不把本地作者声明视为官方来源。
pub fn discover(directory: &Path) -> Result<Vec<LocalPluginPackage>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut packages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry.file_type().map_err(|error| error.to_string())?.is_file() { continue; }
        let archive_path = entry.path();
        if !archive_path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ssmtpkg")) { continue; }
        match inspect_ssmtpkg(&archive_path) {
            Ok(inspection) => {
                packages.push(LocalPluginPackage { archive_path, inspection });
            }
            Err(error) => eprintln!("[Plugins] 跳过无效本地插件包 {}：{error}", archive_path.display()),
        }
    }
    packages.sort_by(|a, b| a.inspection.manifest.id.cmp(&b.inspection.manifest.id).then_with(|| {
        let a = semver::Version::parse(&a.inspection.manifest.version).unwrap();
        let b = semver::Version::parse(&b.inspection.manifest.version).unwrap();
        b.cmp(&a)
    }));
    packages.dedup_by(|a, b| a.inspection.manifest.id == b.inspection.manifest.id);
    Ok(packages)
}

pub fn reviewed(directory: &Path, id: &str, sha256: &str) -> Result<LocalPluginPackage, String> {
    let package = discover(directory)?.into_iter().find(|package| package.inspection.manifest.id == id)
        .ok_or_else(|| "local build package is no longer available".to_string())?;
    if package.inspection.sha256 != sha256.to_ascii_lowercase() {
        return Err("local package changed since review".into());
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn package(directory: &Path, id: &str, version: &str) {
        let path = directory.join(format!("{id}-{version}.ssmtpkg"));
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("ssmt-plugin.json", zip::write::FileOptions::default()).unwrap();
        zip.write_all(serde_json::json!({
            "schemaVersion":1,"id":id,"name":"本地构建","version":version,"author":"SSMT",
            "compatibility":{"ssmt":">=4.0.0","platforms":["windows-x64"]},"contributions":{}
        }).to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn local_discovery_is_optional_validated_and_keeps_latest() {
        let root = std::env::temp_dir().join(format!("ssmt-local-discovery-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        assert!(discover(&root).unwrap().is_empty());
        fs::create_dir(&root).unwrap();
        package(&root,"dev.ssmt.example","1.0.0");
        package(&root,"dev.ssmt.example","1.2.0");
        package(&root,"ssmt.local-example","1.0.0");
        fs::write(root.join("broken.ssmtpkg"), b"invalid").unwrap();
        fs::write(root.join("bare.dll"), b"not a package").unwrap();
        let found = discover(&root).unwrap();
        assert_eq!(found.len(),2);
        assert_eq!(found[0].inspection.manifest.version,"1.2.0");
        let original = found.iter().find(|package| package.inspection.manifest.id == "ssmt.local-example").unwrap();
        assert!(reviewed(&root,"missing",&original.inspection.sha256).is_err());
        assert!(reviewed(&root,"ssmt.local-example",&"0".repeat(64)).is_err());
        let package = reviewed(&root,"ssmt.local-example",&original.inspection.sha256).unwrap();
        let mut registry = super::super::registry::PluginRegistry::new(root.join("installed")).unwrap();
        let installed = super::super::package_installer::install_verified_ssmtpkg(&mut registry,&package.archive_path,&package.inspection.sha256).unwrap();
        assert!(!installed.enabled&&!installed.official);
        assert!(registry.generate_plugin_host_config().plugins.is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(),6);
        fs::remove_dir_all(root).unwrap();
    }
}
