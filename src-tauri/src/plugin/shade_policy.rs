//! Shade 的资源位置由包声明；共享 Host 只接收已解析的贡献。
use super::{gimi_hot_runtime::{canonical_path, checked_path, read_text, rebase_search_paths}, registry::{ExternalDependencyStatus, InstalledPlugin}};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    schema_version: u32,
    pub processes: Vec<String>,
    dependency: String,
    pub host_dll: String,
    config: String,
    preset: String,
    addons: String,
}

pub(super) struct Contribution {
    pub source: String,
    pub config: String,
    pub preset: String,
    pub addons: BTreeMap<String, PathBuf>,
}

impl Policy {
    pub fn load(plugin: &InstalledPlugin) -> Result<Self, String> {
        let root = canonical_path(&plugin.package_root).map_err(|e| e.to_string())?;
        let text = match plugin.manifest.contributions.host_extensions.iter()
            .find(|item| item.provider == "ssmt.graphics.reshade").and_then(|item| item.config.as_deref()) {
            Some(path) => read_text(&checked_path(&root, path)?)?,
            None => include_str!("shade_legacy_policy.json").to_string(),
        };
        let policy: Self = serde_json::from_str(&text).map_err(|e| format!("Shade 声明无效：{e}"))?;
        if policy.schema_version != 1 || policy.processes.is_empty() { return Err("不支持的 Shade 声明".into()); }
        super::validate_plugin_id(&policy.dependency).map_err(|e| e.to_string())?;
        for path in [&policy.host_dll, &policy.config, &policy.preset, &policy.addons].into_iter().chain(&policy.processes) {
            super::validate_package_relative_path(path).map_err(|e| e.to_string())?;
        }
        if policy.processes.iter().any(|name| name.contains(['/', '\\'])) { return Err("Shade 目标必须是文件名".into()); }
        Ok(policy)
    }

    pub fn dependency_root(&self, plugin: &InstalledPlugin) -> Result<PathBuf, String> {
        let dependency = plugin.external_dependencies.get(&self.dependency)
            .filter(|item| item.status == ExternalDependencyStatus::Ready).and_then(|item| item.path.as_ref())
            .ok_or("Shade Bridge 的资源目录尚未就绪")?;
        canonical_path(dependency).map_err(|e| e.to_string())
    }

    pub fn contribution(&self, plugin: &InstalledPlugin) -> Result<Contribution, String> {
        let root = self.dependency_root(plugin)?;
        let mut config = read_text(&checked_path(&root, &self.config)?)?;
        rebase_search_paths(&mut config, &root);
        let preset = if root.join(&self.preset).is_file() { read_text(&checked_path(&root, &self.preset)?)? } else { String::new() };
        let mut addons = BTreeMap::new();
        let directory = root.join(&self.addons);
        if directory.exists() {
            let directory = canonical_path(&directory).map_err(|e| e.to_string())?;
            if !directory.starts_with(&root) { return Err("Shade Add-on 越出资源目录".into()); }
            for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if !path.is_file() || !name.to_ascii_lowercase().ends_with(".addon64") { continue; }
                addons.insert(name.clone(), checked_path(&root, &format!("{}/{name}", self.addons))?);
            }
        }
        Ok(Contribution { source: format!("{} v{}：{}", plugin.manifest.name, plugin.version(), root.join(&self.config).display()), config, preset, addons })
    }
}
