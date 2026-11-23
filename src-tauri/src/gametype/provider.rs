use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use crate::common::d3d11_element::D3D11Element;
use crate::common::d3d11_gametype::D3D11GameType;

#[derive(Deserialize)]
struct ProviderType {
    #[serde(rename = "GameTypeName", default)]
    name: Option<String>,
    #[serde(rename = "D3D11ElementList")]
    elements: Vec<D3D11Element>,
}

fn safe_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 200 || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
        return false;
    }
    let upper = name.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") { return false; }
    for prefix in ["COM", "LPT"] {
        if let Some(suffix) = upper.strip_prefix(prefix) {
            if suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9') { return false; }
        }
    }
    true
}

fn check_entry(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(windows)]
    let reparse = { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if reparse || if directory { !metadata.is_dir() } else { !metadata.is_file() } {
        return Err(format!("unsupported provider entry: {}", path.display()));
    }
    Ok(())
}

/// 仅读取指定目录的插件 JSON，不查询或回退到内置类型。
/// 元数据检查与打开文件之间仍有 TOCTOU，解决前不得用于生产提取链。
pub fn load_provider_types(folder: &Path) -> Result<Vec<D3D11GameType>, String> {
    if !folder.is_absolute() { return Err("provider path must be absolute".into()); }
    let mut current = PathBuf::new();
    for component in folder.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => { current.push(part); check_entry(&current, true)?; }
            _ => return Err("invalid provider path".into()),
        }
    }
    check_entry(folder, true)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        check_entry(&path, false)?;
        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() { return Err("provider contains no JSON types".into()); }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let stem = path.file_stem().and_then(|s| s.to_str()).ok_or("invalid type filename")?;
        if !safe_name(stem) { return Err(format!("unsafe type filename: {stem}")); }
        let raw = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let parsed: ProviderType = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = parsed.name.as_deref().unwrap_or(stem);
        if !safe_name(name) { return Err(format!("unsafe type name: {name}")); }
        if !seen.insert(name.to_ascii_lowercase()) { return Err(format!("duplicate type name: {name}")); }
        result.push(D3D11GameType::from_parts(name, parsed.elements));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn workspace() -> PathBuf {
        let p = std::env::temp_dir().join(format!("ssmt-provider-test-{}-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&p).unwrap(); p
    }
    fn write_type(p: &Path, file: &str, name: &str) {
        fs::write(p.join(file), format!(r#"{{"GameTypeName":"{name}","D3D11ElementList":[]}}"#)).unwrap();
    }
    #[test]
    fn provider_loads_its_own_json_and_rejects_invalid_data() {
        let root = workspace();
        assert!(load_provider_types(&root).is_err());
        write_type(&root, "plugin.json", "PluginOnly");
        let types = load_provider_types(&root).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].game_type_name, "PluginOnly");
        write_type(&root, "second.json", "pluginonly");
        assert!(load_provider_types(&root).is_err());
        write_type(&root, "second.json", "../escape");
        assert!(load_provider_types(&root).is_err());
        write_type(&root, "second.json", "NUL");
        assert!(load_provider_types(&root).is_err());
        fs::write(root.join("second.json"), "{").unwrap();
        assert!(load_provider_types(&root).is_err());
        write_type(&root, "second.json", "Valid");
        write_type(&root, "bad name.json", "Other");
        assert!(load_provider_types(&root).is_err());
        assert!(load_provider_types(Path::new("relative")).is_err());
    }
    #[test]
    fn provider_rejects_reparse_directory() {
        let root = workspace();
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir(&target).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&target).status().unwrap().success());
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(load_provider_types(&link).is_err());
    }
}
